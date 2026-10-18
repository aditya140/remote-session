use crate::{Binding, BindingStatus, Error, Identity, MachineCredentials, http};
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{
    KeyId, MachineId,
    rest::{
        AccessToken, Challenge, ChallengeRequest, Enrolled, EnrollmentRequest, MachineRevoke,
        MachineToken, Proof, Revoked,
    },
    wire::Machine,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::Instant;

pub const RECORDING_DISCLOSURE: &str = "Remote sessions require recording. Transcripts and terminal output are retained for the configured period (90 days by default, with monthly purge rounding to about 120 days); legal holds can retain them longer. Authorized owner/admin access applies, admin reads are audited, and secret redaction is best effort. Recording failures produce warnings and explicit gaps. Production remote sessions require the protected recording and retention implementation.";
/// Composition displays the disclosure and obtains the user's acknowledgement.
pub struct RecordingConsent(());
impl RecordingConsent {
    pub fn acknowledged() -> Self {
        Self(())
    }
}
#[derive(Debug, Clone)]
pub struct MachineMetadata {
    pub hostname: String,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub cli_version: String,
    pub labels: BTreeMap<String, String>,
}
#[derive(Clone)]
pub struct GatewayApi {
    origin: String,
    http: reqwest::Client,
}
impl std::fmt::Debug for GatewayApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GatewayApi([configured TLS origin])")
    }
}
pub struct MachineAuthentication {
    pub token: AccessToken,
    pub expires: Instant,
}
impl std::fmt::Debug for MachineAuthentication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachineAuthentication")
            .field("token", &"[redacted]")
            .finish_non_exhaustive()
    }
}
impl GatewayApi {
    pub fn new(origin: &str, ca: Option<&Path>) -> Result<Self, Error> {
        let url = http::https(origin)?;
        if url.origin().ascii_serialization() != origin {
            return Err(Error::Configuration);
        }
        Ok(Self {
            origin: origin.into(),
            http: http::client(ca)?,
        })
    }
    pub fn origin(&self) -> &str {
        &self.origin
    }
    async fn request<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        access: Option<&AccessToken>,
        body: Option<&impl Serialize>,
    ) -> Result<T, Error> {
        self.request_limited(method, path, access, body, 65536)
            .await
    }
    pub(crate) async fn request_limited<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        access: Option<&AccessToken>,
        body: Option<&impl Serialize>,
        limit: usize,
    ) -> Result<T, Error> {
        let mut request = self.http.request(method, format!("{}{path}", self.origin));
        if let Some(access) = access {
            let mut header =
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", access.as_str()))
                    .map_err(|_| Error::Protocol)?;
            header.set_sensitive(true);
            request = request.header(reqwest::header::AUTHORIZATION, header);
        }
        if let Some(body) = body {
            let bytes = serde_json::to_vec(body).map_err(|_| Error::Protocol)?;
            if bytes.len() > limit {
                return Err(Error::Protocol);
            }
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(bytes);
        }
        let response = request.send().await.map_err(|_| Error::Unavailable)?;
        if response.status().is_success() {
            http::response(response, 1024 * 1024).await
        } else {
            let status = response.status();
            let mut failure: http_failure::Failure = match http_failure::parse(response).await {
                Ok(failure) => failure,
                Err(_) if status.is_server_error() => return Err(Error::Unavailable),
                Err(_)
                    if status == reqwest::StatusCode::UNAUTHORIZED
                        || status == reqwest::StatusCode::FORBIDDEN =>
                {
                    return Err(Error::Denied);
                }
                Err(error) => return Err(error),
            };
            failure.error.message = "gateway rejected the request".into();
            Err(Error::Rejected(failure.error))
        }
    }
    pub async fn enroll(
        &self,
        identity: &mut Identity,
        access: &AccessToken,
        metadata: MachineMetadata,
        _consent: RecordingConsent,
    ) -> Result<Binding, Error> {
        if identity.origin() != self.origin {
            return Err(Error::Configuration);
        }
        if identity.binding().is_some_and(|binding| {
            !matches!(
                binding.status,
                BindingStatus::Enrolled | BindingStatus::RecoveryPending
            )
        }) {
            return Err(Error::Revoked);
        }
        let request = EnrollmentRequest {
            installation: identity.installation(),
            key: identity.key(),
            public_key: identity.public_key().into(),
            os_user: identity.os_user().into(),
            hostname: metadata.hostname,
            name: metadata.name,
            os: metadata.os,
            arch: metadata.arch,
            cli_version: metadata.cli_version,
            labels: metadata.labels,
            recovery: identity
                .binding()
                .filter(|binding| binding.status == BindingStatus::RecoveryPending)
                .map(|binding| qgenie_remote_proto::rest::Recovery {
                    machine: binding.machine,
                    confirmed: true,
                }),
        };
        // Identity/key are already fsynced. A lost response is reconciled by a new
        // challenge using this same installation/key, never by generating another.
        let challenge: Challenge = self
            .request(
                reqwest::Method::POST,
                "/v1/enrollments/challenge",
                Some(access),
                Some(&request),
            )
            .await?;
        validate_challenge(
            &challenge,
            &self.origin,
            identity.key(),
            identity.binding().map(|binding| binding.machine),
        )?;
        let proof = Proof {
            challenge: challenge.challenge,
            signature: STANDARD
                .encode(identity.sign(&challenge.signing_bytes().map_err(|_| Error::Protocol)?)),
        };
        let enrolled: Enrolled = self
            .request(
                reqwest::Method::POST,
                "/v1/enrollments",
                Some(access),
                Some(&proof),
            )
            .await?;
        if enrolled.machine != challenge.machine || enrolled.key != identity.key() {
            return Err(Error::Protocol);
        }
        let machine: Machine = self
            .request(
                reqwest::Method::GET,
                &format!("/v1/machines/{}", enrolled.machine),
                Some(access),
                None::<&()>,
            )
            .await?;
        if machine.id != enrolled.machine {
            return Err(Error::Protocol);
        }
        let binding = Binding {
            installation: identity.installation(),
            key: identity.key(),
            machine: enrolled.machine,
            owner: machine.owner,
            status: BindingStatus::Enrolled,
        };
        identity.set_binding(binding.clone())?;
        identity.finish_recovery().await?;
        Ok(binding)
    }
    pub async fn authenticate_machine(
        &self,
        credentials: &MachineCredentials,
    ) -> Result<MachineAuthentication, Error> {
        if credentials.origin != self.origin
            || credentials.binding.status != BindingStatus::Enrolled
        {
            return Err(Error::Revoked);
        }
        let started = Instant::now();
        let challenge: Challenge = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/machines/{}/challenges", credentials.binding.machine),
                None,
                Some(&ChallengeRequest {
                    key: credentials.binding.key,
                }),
            )
            .await?;
        validate_challenge(
            &challenge,
            &self.origin,
            credentials.binding.key,
            Some(credentials.binding.machine),
        )?;
        let proof = Proof {
            challenge: challenge.challenge,
            signature: STANDARD
                .encode(credentials.sign(&challenge.signing_bytes().map_err(|_| Error::Protocol)?)),
        };
        let token: MachineToken = self
            .request(
                reqwest::Method::POST,
                "/v1/machine-tokens",
                None,
                Some(&proof),
            )
            .await?;
        let remaining = token
            .expires_at
            .unix_millis()
            .checked_sub(now_ms()?)
            .filter(|remaining| *remaining > 0 && *remaining <= 905000)
            .ok_or(Error::Protocol)?
            .min(900000);
        let expires = started + Duration::from_millis(remaining);
        if expires <= Instant::now() {
            return Err(Error::Expired);
        }
        Ok(MachineAuthentication {
            token: token.token.try_into().map_err(|_| Error::Protocol)?,
            expires,
        })
    }
    pub async fn unenroll(
        &self,
        identity: &mut Identity,
        access: &AccessToken,
        confirmed: bool,
    ) -> Result<(), Error> {
        if !confirmed {
            return Err(Error::Denied);
        }
        if identity.origin() != self.origin {
            return Err(Error::Configuration);
        }
        let mut binding = identity.binding().cloned().ok_or(Error::State)?;
        if binding.status == BindingStatus::RecoveryPending {
            return Err(Error::State);
        }
        identity.finish_recovery().await?;
        binding.status = BindingStatus::RevocationPending;
        identity.set_binding(binding.clone())?;
        let revoked: Revoked = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/machines/{}/revoke", binding.machine),
                Some(access),
                Some(&MachineRevoke { confirmed: true }),
            )
            .await?;
        if revoked.machine != binding.machine {
            return Err(Error::Protocol);
        }
        binding.status = BindingStatus::Revoked;
        identity.set_binding(binding)?;
        identity.remove_key().await
    }
}
fn validate_challenge(
    challenge: &Challenge,
    origin: &str,
    key: KeyId,
    machine: Option<MachineId>,
) -> Result<(), Error> {
    let now = now_ms()?;
    if challenge.origin != origin
        || challenge.key != key
        || machine.is_some_and(|machine| machine != challenge.machine)
        || challenge.issued_at_ms.0 > now.saturating_add(5000)
        || challenge.expires_at_ms.0 <= now
    {
        return Err(Error::Denied);
    }
    challenge.signing_bytes().map_err(|_| Error::Protocol)?;
    Ok(())
}
fn now_ms() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::State)?
        .as_millis()
        .try_into()
        .map_err(|_| Error::State)
}
mod http_failure {
    use crate::{Error, http};
    #[derive(serde::Deserialize)]
    pub(super) struct Failure {
        pub error: qgenie_remote_proto::RemoteError,
    }
    pub(super) async fn parse(response: reqwest::Response) -> Result<Failure, Error> {
        http::response(response, 65536).await
    }
}
