//! One immutable upload at a time; no input capture and no live-I/O dependency.
use crate::{
    Error, GatewayApi, MachineAuthentication, MachineCredentials, NativeAccess, NativeClient,
};
use qgenie_remote_proto::{
    ipc::{DispatchFrame, RecordingReceipt, RecordingUpload},
    rest::{RecordingAuthority, RecordingAuthorityRequest},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadProgress {
    Idle,
    Reclaimed,
    AlreadyReclaimed,
}
impl NativeClient {
    /// Only this HTTPS composition can bind an independently obtained epoch and submit
    /// its corresponding receipt. No public method accepts a caller-supplied epoch.
    pub async fn upload_recording(
        &self,
        api: &GatewayApi,
        credentials: &MachineCredentials,
        authentication: &MachineAuthentication,
        access: &NativeAccess,
    ) -> Result<UploadProgress, Error> {
        if api.origin() != self.scope().origin
            || credentials.origin != self.scope().origin
            || credentials.binding.owner != self.scope().owner
            || credentials.binding.machine != self.scope().machine
        {
            return Err(Error::Configuration);
        }
        let remaining = access.remaining()?;
        tokio::time::timeout(remaining, async {
            if tokio::time::Instant::now() >= authentication.expires {
                return Err(Error::Expired);
            }
            let Some(upload) = self.recording_fetch(access).await? else {
                return Ok(UploadProgress::Idle);
            };
            let context = self.context(access, access.connection(), None)?;
            let DispatchFrame::RecordingEpochChallengeIssued { challenge } = self
                .rpc(DispatchFrame::RecordingEpochChallenge {
                    context: context.clone(),
                })
                .await?
            else {
                return Err(Error::Protocol);
            };
            let session = match &upload {
                RecordingUpload::Content(v) => v.session,
                RecordingUpload::Gap(v) => v.session,
            };
            let authority: RecordingAuthority = api
                .request_limited(
                    reqwest::Method::POST,
                    &format!("/v1/recordings/sessions/{session}/authority"),
                    Some(&authentication.token),
                    Some(&RecordingAuthorityRequest { challenge }),
                    65536,
                )
                .await?;
            // TLS, origin, current machine/key/owner and the native-issued challenge are
            // independent of the later receipt. A late exchange retains the old grant.
            if authority.challenge != challenge
                || authority.origin != self.scope().origin
                || authority.owner != self.scope().owner
                || authority.machine != self.scope().machine
                || authority.key != credentials.binding.key
                || authority.backend_epoch.is_empty()
                || authority.backend_epoch.len() > 128
            {
                return Err(Error::Protocol);
            }
            access.remaining()?;
            let epoch = authority.backend_epoch.clone();
            match self
                .rpc(DispatchFrame::BindRecordingEpoch {
                    context: context.clone(),
                    authority,
                })
                .await?
            {
                DispatchFrame::RecordingEpochBound { challenge: bound } if bound == challenge => {}
                _ => return Err(Error::Protocol),
            }
            let receipt = match upload {
                RecordingUpload::Content(batch) => RecordingReceipt::Content(
                    api.request_limited(
                        reqwest::Method::POST,
                        "/v1/recordings/chunks",
                        Some(&authentication.token),
                        Some(&batch),
                        128 * 1024,
                    )
                    .await?,
                ),
                RecordingUpload::Gap(gap) => RecordingReceipt::Gap(
                    api.request_limited(
                        reqwest::Method::POST,
                        "/v1/recordings/gaps",
                        Some(&authentication.token),
                        Some(&gap),
                        65536,
                    )
                    .await?,
                ),
            };
            let (machine, received_epoch) = match &receipt {
                RecordingReceipt::Content(r) => (r.machine, &r.recovery_epoch),
                RecordingReceipt::Gap(r) => (r.machine, &r.recovery_epoch),
            };
            if machine != self.scope().machine || received_epoch != &epoch {
                return Err(Error::Protocol);
            }
            access.remaining()?;
            match self
                .rpc(DispatchFrame::RecordingAcknowledge {
                    context,
                    challenge,
                    receipt,
                })
                .await?
            {
                DispatchFrame::RecordingAcknowledged { reclaimed: true } => {
                    Ok(UploadProgress::Reclaimed)
                }
                DispatchFrame::RecordingAcknowledged { reclaimed: false } => {
                    Ok(UploadProgress::AlreadyReclaimed)
                }
                _ => Err(Error::Protocol),
            }
        })
        .await
        .map_err(|_| Error::Expired)?
    }
}
