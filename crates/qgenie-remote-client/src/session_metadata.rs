//! User metadata is a gateway operation and never acquires native control.
use crate::LiveHandle;
use qgenie_remote_proto::{
    ErrorCode, MachineId, OperationOutcome, RemoteError, SessionId,
    wire::{Envelope, Message, Session, SessionMetadata, SessionMetadataChange},
};

impl LiveHandle {
    fn metadata_capability(&self) -> Result<(), RemoteError> {
        if self.metadata {
            Ok(())
        } else {
            Err(RemoteError {
                code: ErrorCode::UnsupportedCapability,
                outcome: OperationOutcome::NotSent,
                message: "gateway did not negotiate session-metadata-v1".into(),
                retryable: false,
            })
        }
    }
    /// Commit one bounded metadata change. A lost reply requires a read, not a new launch.
    pub async fn update_session_metadata(
        &self,
        machine: MachineId,
        session: SessionId,
        change: SessionMetadataChange,
    ) -> Result<Session, RemoteError> {
        self.metadata_capability()?;
        let mut metadata = SessionMetadata::default();
        metadata.apply(&change)?;
        let mut request = Envelope::new(Message::SessionMetadataUpdate { change });
        request.machine = Some(machine);
        request.session = Some(session);
        match self.submit(request).await?.pop().map(|reply| reply.message) {
            Some(Message::SessionFound(updated))
                if updated.id == session && updated.machine == machine =>
            {
                Ok(updated)
            }
            _ => Err(RemoteError::unknown(
                "metadata reply unavailable; read the session to reconcile",
            )),
        }
    }
    /// Exact name lookup within the authenticated gateway owner and selected machine.
    /// Duplicate names are rejected. The caller uses the returned stable session ID.
    pub async fn resolve_session_name(
        &self,
        machine: MachineId,
        name: &str,
    ) -> Result<Session, RemoteError> {
        self.metadata_capability()?;
        SessionMetadata::validate_name(name)?;
        let mut request = Envelope::new(Message::SessionResolve { name: name.into() });
        request.machine = Some(machine);
        match self
            .request(request)
            .await?
            .pop()
            .map(|reply| reply.message)
        {
            Some(Message::SessionFound(session))
                if session.machine == machine && session.metadata.name.as_deref() == Some(name) =>
            {
                Ok(session)
            }
            _ => Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "invalid session name result",
            )),
        }
    }
}
