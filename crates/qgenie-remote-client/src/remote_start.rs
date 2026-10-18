//! Persist an immutable start identity before submitting a network mutation.
use crate::{LiveHandle, private_directory, private_file};
use fs2::FileExt;
use qgenie_remote_proto::{
    wire::{Envelope, Launch, LaunchKind, Message, Session},
    *,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
};

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
        let state = state.as_ref().to_owned();
        let origin = self.origin.clone();
        let permit = self
            .start_work
            .clone()
            .try_acquire_owned()
            .map_err(|_| not_sent(ErrorCode::Overloaded, "start journal capacity exceeded"))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            (|| -> std::io::Result<Vec<RemoteStartIdentity>> {
                private_directory(&state)?;
                let lock = private_file(&state.join("remote-starts.lock"))?;
                FileExt::try_lock_shared(&lock)?;
                let directory = state.join("remote-starts");
                private_directory(&directory)?;
                let prefix = format!(
                    "{}-",
                    fingerprint(&(&origin, owner, machine)).map_err(std::io::Error::other)?
                );
                let mut records = Vec::new();
                for (count, entry) in std::fs::read_dir(&directory)?.enumerate() {
                    if count >= 4096 {
                        return Err(std::io::Error::other("start journal capacity exceeded"));
                    }
                    let entry = entry?;
                    let name = entry.file_name();
                    let Some(name) = name
                        .to_str()
                        .filter(|name| name.starts_with(&prefix) && name.ends_with(".json"))
                    else {
                        continue;
                    };
                    let mut bytes = Vec::new();
                    private_file(&entry.path())?
                        .take(4097)
                        .read_to_end(&mut bytes)?;
                    let saved: RemoteStartIdentity =
                        decode_json(&bytes).map_err(std::io::Error::other)?;
                    if bytes.len() > 4096
                        || saved.origin != origin
                        || saved.owner != owner
                        || saved.machine != machine
                        || name != format!("{prefix}{}.json", saved.key)
                        || saved.fingerprint.len() != 64
                        || !saved
                            .fingerprint
                            .bytes()
                            .all(|byte| byte.is_ascii_hexdigit())
                    {
                        return Err(std::io::Error::other("invalid start journal identity"));
                    }
                    records.push(saved);
                }
                records.sort_by_key(|saved| saved.key.to_string());
                Ok(records)
            })()
            .map_err(|_| {
                not_sent(
                    ErrorCode::Internal,
                    "start journal is unavailable or invalid",
                )
            })
        })
        .await
        .map_err(|_| not_sent(ErrorCode::Internal, "start journal worker stopped"))?
    }
    /// The caller retains `launch.key` across uncertainty and uses `lookup_start`.
    /// Reconnect never resubmits this operation. Journal files contain no argv/prompt.
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
    let result = (|| -> std::io::Result<()> {
        private_directory(state)?;
        let lock = private_file(&state.join("remote-starts.lock"))?;
        lock.try_lock_exclusive()?;
        let directory = state.join("remote-starts");
        private_directory(&directory)?;
        let scope = fingerprint(&(&saved.origin, saved.owner, saved.machine))
            .map_err(std::io::Error::other)?;
        let path = directory.join(format!("{scope}-{}.json", saved.key));
        if !path.exists() && std::fs::read_dir(&directory)?.count() >= 4096 {
            return Err(std::io::Error::other("start journal capacity exceeded"));
        }
        let mut file = private_file(&path)?;
        if file.metadata()?.len() > 0 {
            let mut bytes = Vec::new();
            (&mut file).take(4097).read_to_end(&mut bytes)?;
            if bytes.len() > 4096 {
                return Err(std::io::Error::other("invalid start journal"));
            }
            let previous: RemoteStartIdentity =
                decode_json(&bytes).map_err(std::io::Error::other)?;
            if previous != *saved {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "start identity conflicts",
                ));
            }
        } else {
            file.write_all(&serde_json::to_vec(saved)?)?;
            file.sync_all()?;
            std::fs::File::open(&directory)?.sync_all()?;
            std::fs::File::open(state)?.sync_all()?;
        }
        Ok(())
    })();
    result.map_err(|e| {
        not_sent(
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                ErrorCode::IdempotencyConflict
            } else {
                ErrorCode::Internal
            },
            "start identity cannot be persisted or conflicts; no request was sent",
        )
    })
}
