//! Persist an immutable start identity before submitting a network mutation.
use crate::LiveHandle;
use qgenie_remote_proto::{
    wire::{Envelope, Launch, LaunchKind, Message, Session},
    *,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RemoteStartIdentity {
    pub origin: String,
    pub owner: UserId,
    pub machine: MachineId,
    pub key: IdempotencyKey,
    pub fingerprint: String,
}
impl LiveHandle {
    pub fn agents_available(&self) -> bool {
        self.agents
    }
    /// Bounded, scope-filtered journal identities for explicit recovery after client restart.
    /// Presence means submission was attempted, not that a native launch occurred.
    pub async fn start_identities(
        &self,
        state: impl AsRef<Path>,
        owner: UserId,
        machine: MachineId,
    ) -> Result<Vec<RemoteStartIdentity>, RemoteError> {
        let mut records = Vec::new();
        let mut after = None;
        loop {
            let (page, next) = self
                .start_identities_page(state.as_ref(), owner, machine, after, 256)
                .await?;
            records.extend(page);
            if records.len() > 4096 || (records.len() == 4096 && next.is_some()) {
                return Err(not_sent(
                    ErrorCode::Overloaded,
                    "use paginated start identities",
                ));
            }
            after = next;
            if after.is_none() {
                return Ok(records);
            }
        }
    }

    /// Reads a bounded page of original start identities for this gateway and owner.
    /// `limit` must be 1..=256. The next cursor is the last returned key.
    /// A record proves an attempted submission, not a completed native launch.
    /// The read remains bounded if its caller cancels during journal access.
    ///
    /// # Errors
    /// Rejects invalid bounds. Returns `NotSent` for exhausted worker capacity, an
    /// inaccessible journal, or invalid imported state. This read submits no launch.
    pub async fn start_identities_page(
        &self,
        state: impl AsRef<Path>,
        owner: UserId,
        machine: MachineId,
        after: Option<IdempotencyKey>,
        limit: u16,
    ) -> Result<(Vec<RemoteStartIdentity>, Option<IdempotencyKey>), RemoteError> {
        let state = state.as_ref().to_owned();
        let origin = self.origin.clone();
        let permit = self
            .start_work
            .clone()
            .try_acquire_owned()
            .map_err(|_| not_sent(ErrorCode::Overloaded, "start journal capacity exceeded"))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            crate::start_journal::Journal::open(&state)?
                .remote_page(&origin, owner, machine, after, limit)
        })
        .await
        .map_err(|_| not_sent(ErrorCode::Internal, "start journal worker stopped"))?
    }
    /// The caller retains `launch.key` across uncertainty and uses `lookup_start`.
    /// Reconnect never resubmits this operation. The identity journal contains no argv/prompt.
    /// Cancelling this future does not undo a persisted identity or an accepted launch.
    ///
    /// # Errors
    /// Returns `NotSent` for invalid arguments, missing capabilities, or journal failure
    /// before submission. Native rejection preserves its error code. A lost reply can
    /// return `Unknown`; reconcile with `lookup_start` using the original key.
    pub async fn start(
        &self,
        state_dir: impl AsRef<Path>,
        owner: UserId,
        machine: MachineId,
        launch: Launch,
    ) -> Result<Session, RemoteError> {
        launch.validate()?;
        if matches!(launch.kind, LaunchKind::Agent { .. }) && !self.agents {
            return Err(not_sent(
                ErrorCode::UnsupportedCapability,
                "gateway did not negotiate agent-events-v1",
            ));
        }
        let saved = RemoteStartIdentity {
            origin: self.origin.clone(),
            owner,
            machine,
            key: launch.key,
            fingerprint: launch.fingerprint()?,
        };
        let directory = state_dir.as_ref().to_owned();
        let permit = self
            .start_work
            .clone()
            .try_acquire_owned()
            .map_err(|_| not_sent(ErrorCode::Overloaded, "start journal capacity exceeded"))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            persist(&directory, &saved)
        })
        .await
        .map_err(|_| not_sent(ErrorCode::Internal, "start journal unavailable"))??;
        let mut request = Envelope::new(Message::Start(launch));
        request.machine = Some(machine);
        match self.submit(request).await?.pop().map(|v| v.message) {
            Some(Message::Started(session))
                if session.owner == owner && session.machine == machine =>
            {
                Ok(session)
            }
            _ => Err(RemoteError::unknown(
                "invalid start reply; reconcile the persisted original key",
            )),
        }
    }
}
fn not_sent(code: ErrorCode, message: &str) -> RemoteError {
    RemoteError {
        code,
        outcome: OperationOutcome::NotSent,
        message: message.into(),
        retryable: false,
    }
}
fn persist(state: &Path, saved: &RemoteStartIdentity) -> Result<(), RemoteError> {
    crate::start_journal::Journal::open(state)?.remote(saved)
}
