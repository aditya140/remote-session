//! SDK for the same-user local preview. Connection retries never resend mutations.
mod gateway;
mod live;
mod observe;
mod output;
pub use observe::{ObservedOutput, RemoteObserver};
mod remote_approvals;
pub use remote_approvals::ApprovalPage;
mod remote_control;
mod remote_replay;
pub use output::{OutputCursor, OutputItem, OutputUpdate, ReadPlan};
pub use remote_replay::{RemoteAttachment, RemoteReplay, RemoteSnapshot};
mod remote_start;
use base64::{Engine, engine::general_purpose::STANDARD};
use fs2::FileExt;
pub use gateway::{GatewayClient, GatewayConfig};
pub use live::{LiveClient, LiveEvent, LiveHandle, LiveStatus};
use qgenie_remote_proto::ipc::DispatchFrame;
use qgenie_remote_proto::*;
use qgenie_remote_transport::ipc::{
    client_handshake, connect, private_directory, private_file, read_dispatch, write_dispatch,
};
pub use qgenie_remote_transport::ws::Frame as LiveFrame;
pub use remote_control::RemoteController;
pub use remote_start::RemoteStartIdentity;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct Client {
    path: PathBuf,
    id: ClientId,
    pending: Arc<Semaphore>,
    journal_work: Arc<Semaphore>,
    snapshot_work: Arc<Semaphore>,
}
impl Client {
    pub fn local(state_dir: impl AsRef<Path>) -> Self {
        Self {
            path: state_dir.as_ref().join("dispatch.sock"),
            id: ClientId::new(),
            pending: Arc::new(Semaphore::new(128)),
            journal_work: Arc::new(Semaphore::new(16)),
            snapshot_work: Arc::new(Semaphore::new(4)),
        }
    }
    pub fn id(&self) -> ClientId {
        self.id
    }
    pub async fn request(&self, body: Operation) -> Result<Reply, RemoteError> {
        let _permit = self.pending.try_acquire().map_err(|_| {
            failure(
                ErrorCode::Overloaded,
                OperationOutcome::NotSent,
                "too many pending requests",
            )
        })?;
        let request = Request {
            v: PROTOCOL_MAJOR,
            id: RequestId::new(),
            client: self.id,
            body,
        };
        let mut stream = connect(&self.path).await.map_err(|_| {
            failure(
                ErrorCode::Internal,
                OperationOutcome::NotSent,
                "session daemon is unavailable; run qgenie-remote up",
            )
        })?;
        client_handshake(&mut stream).await.map_err(|_| {
            failure(
                ErrorCode::UnsupportedVersion,
                OperationOutcome::NotSent,
                "IPC negotiation failed; no operation was sent",
            )
        })?;
        write_dispatch(&mut stream, &DispatchFrame::Local(request.clone()))
            .await
            .map_err(|_| {
                RemoteError::unknown(
                    "request transmission interrupted; reconcile its original identity",
                )
            })?;
        let frame = read_dispatch(&mut stream).await.map_err(|_| {
            RemoteError::unknown("reply lost; do not resend input or allocate a new start key")
        })?;
        let DispatchFrame::LocalReply(response) = frame else {
            return Err(RemoteError::unknown(
                "invalid reply; operation outcome is unknown",
            ));
        };
        if response.v != PROTOCOL_MAJOR || response.re != request.id {
            return Err(RemoteError::unknown(
                "reply version or correlation does not match",
            ));
        }
        match response.body {
            Reply::Error(error) => Err(error),
            reply => Ok(reply),
        }
    }
    pub async fn status(&self) -> Result<HostStatus, RemoteError> {
        match self.request(Operation::Status).await? {
            Reply::Status(status) => Ok(status),
            _ => Err(unexpected()),
        }
    }
    pub async fn sessions(&self) -> Result<Vec<SessionInfo>, RemoteError> {
        match self.request(Operation::List).await? {
            Reply::Sessions(sessions) => Ok(sessions),
            _ => Err(unexpected()),
        }
    }
    pub async fn start(&self, spec: StartSpec) -> Result<SessionInfo, RemoteError> {
        let permit = self.journal_work.try_acquire().map_err(|_| {
            failure(
                ErrorCode::Overloaded,
                OperationOutcome::NotSent,
                "start journal worker limit reached",
            )
        })?;
        let state_dir = self.path.parent().unwrap().to_owned();
        let saved = spec.clone();
        tokio::task::spawn_blocking(move || persist_start(&state_dir, &saved))
            .await
            .map_err(|_| {
                failure(
                    ErrorCode::Internal,
                    OperationOutcome::NotSent,
                    "start journal worker failed",
                )
            })??;
        drop(permit);
        let key = spec.key;
        let reply = match self.request(Operation::Start(spec)).await {
            Ok(reply) => reply,
            Err(mut error) => {
                if matches!(error.outcome, OperationOutcome::Unknown) {
                    if let Ok(session) = self.lookup_start(key).await {
                        return Ok(session);
                    }
                    error.message = format!("{}; original start key: {key}", error.message);
                }
                return Err(error);
            }
        };
        match reply {
            Reply::Started(session) => Ok(session),
            _ => Err(unexpected()),
        }
    }
    pub async fn lookup_start(&self, key: IdempotencyKey) -> Result<SessionInfo, RemoteError> {
        match self.request(Operation::LookupStart { key }).await? {
            Reply::Started(session) => Ok(session),
            _ => Err(unexpected()),
        }
    }
    pub async fn attach(&self, session: SessionId) -> Result<Attachment, RemoteError> {
        let _capture = self.snapshot_work.try_acquire().map_err(|_| {
            RemoteError::rejected(
                ErrorCode::Overloaded,
                "snapshot assembly capacity unavailable",
            )
        })?;
        let Reply::Attached(mut attachment) = self.request(Operation::Attach { session }).await?
        else {
            return Err(unexpected());
        };
        let first = &attachment.snapshot;
        if attachment.session.id != session
            || first.page != 0
            || first.total_pages == 0
            || first.total_pages > 32
            || first.seq != attachment.session.head
        {
            return Err(unexpected());
        }
        validate_size(first.rows, first.cols)?;
        let decode = |data: &str| STANDARD.decode(data).map_err(|_| unexpected());
        let mut data = decode(&first.data)?;
        if data.len() > 256 * 1024 {
            return Err(unexpected());
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        for page in 1..first.total_pages {
            let Reply::SnapshotPage(next) = tokio::time::timeout_at(
                deadline,
                self.request(Operation::SnapshotPage {
                    session,
                    epoch: first.epoch,
                    page,
                }),
            )
            .await
            .map_err(|_| {
                RemoteError::rejected(ErrorCode::HistoryTruncated, "snapshot assembly timed out")
            })??
            else {
                return Err(unexpected());
            };
            if next.epoch != first.epoch
                || next.seq != first.seq
                || next.page != page
                || next.total_pages != first.total_pages
                || next.rows != first.rows
                || next.cols != first.cols
                || std::mem::discriminant(&next.format) != std::mem::discriminant(&first.format)
            {
                return Err(unexpected());
            }
            let chunk = decode(&next.data)?;
            if chunk.len() > 256 * 1024 || data.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err(unexpected());
            }
            data.extend(chunk);
        }
        attachment.snapshot.data = STANDARD.encode(data);
        attachment.snapshot.total_pages = 1;
        Ok(attachment)
    }
    pub async fn replay(
        &self,
        session: SessionId,
        after: Counter,
    ) -> Result<ReplayBatch, RemoteError> {
        match self.request(Operation::Replay { session, after }).await? {
            Reply::Replay(batch) => Ok(batch),
            _ => Err(unexpected()),
        }
    }
    pub async fn take_control(
        &self,
        session: SessionId,
        takeover: bool,
    ) -> Result<Controller, RemoteError> {
        match self
            .request(Operation::TakeControl { session, takeover })
            .await?
        {
            Reply::Control(lease) => Ok(Controller {
                client: self.clone(),
                session,
                generation: lease.generation,
                next: lease.last_input.0.checked_add(1).ok_or_else(|| {
                    RemoteError::rejected(ErrorCode::CapacityExhausted, "input sequence exhausted")
                })?,
                uncertain: false,
            }),
            _ => Err(unexpected()),
        }
    }
    pub async fn release(&self, session: SessionId) -> Result<(), RemoteError> {
        self.request(Operation::ReleaseControl { session })
            .await
            .map(|_| ())
    }
    pub async fn terminate(&self, session: SessionId, confirmed: bool) -> Result<(), RemoteError> {
        self.request(Operation::Terminate { session, confirmed })
            .await
            .map(|_| ())
    }
    pub async fn approvals(&self) -> Result<Vec<Approval>, RemoteError> {
        match self.request(Operation::Approvals).await? {
            Reply::Approvals(approvals) => Ok(approvals),
            _ => Err(unexpected()),
        }
    }
}

pub struct Controller {
    client: Client,
    session: SessionId,
    generation: Counter,
    next: u64,
    uncertain: bool,
}
impl Controller {
    pub fn session(&self) -> SessionId {
        self.session
    }
    pub fn generation(&self) -> Counter {
        self.generation
    }
    pub async fn input(&mut self, bytes: &[u8]) -> Result<(), RemoteError> {
        if self.uncertain {
            return Err(RemoteError::unknown(
                "previous input outcome is unknown; release and reacquire control before new input",
            ));
        }
        // Poison before awaiting: cancellation cannot make a later call reuse
        // this input identity or silently resend bytes.
        self.uncertain = true;
        let seq = Counter(self.next);
        let reply = self
            .client
            .request(Operation::Input {
                session: self.session,
                generation: self.generation,
                input_seq: seq,
                data: STANDARD.encode(bytes),
            })
            .await?;
        match reply {
            Reply::InputAck {
                input_seq,
                outcome: ApplicationOutcome::Applied | ApplicationOutcome::AlreadyApplied,
            } if input_seq == seq => {
                self.next = self.next.checked_add(1).ok_or_else(|| {
                    RemoteError::rejected(ErrorCode::CapacityExhausted, "input sequence exhausted")
                })?;
                self.uncertain = false;
                Ok(())
            }
            _ => Err(RemoteError::unknown(
                "input application is uncertain; bytes will not be resent",
            )),
        }
    }
    pub async fn resize(&self, rows: u16, cols: u16) -> Result<(), RemoteError> {
        self.client
            .request(Operation::Resize {
                session: self.session,
                generation: self.generation,
                rows,
                cols,
            })
            .await
            .map(|_| ())
    }
    pub async fn interrupt(&self) -> Result<(), RemoteError> {
        self.client
            .request(Operation::Interrupt {
                session: self.session,
                generation: self.generation,
            })
            .await
            .map(|_| ())
    }
}
fn failure(code: ErrorCode, outcome: OperationOutcome, message: &str) -> RemoteError {
    RemoteError {
        code,
        outcome,
        message: message.into(),
        retryable: false,
    }
}
fn unexpected() -> RemoteError {
    RemoteError::unknown("unexpected response type")
}

fn persist_start(state_dir: &Path, spec: &StartSpec) -> Result<(), RemoteError> {
    fn persist(state_dir: &Path, spec: &StartSpec) -> std::io::Result<()> {
        private_directory(state_dir)?;
        let journal_lock = private_file(&state_dir.join("starts.lock"))?;
        journal_lock.lock_exclusive()?;
        let directory = state_dir.join("starts");
        private_directory(&directory)?;
        let path = directory.join(format!("{}.json", spec.key));
        if !path.exists() && std::fs::read_dir(&directory)?.count() >= 4096 {
            return Err(std::io::Error::other("start journal capacity reached"));
        }
        let mut file = private_file(&path)?;
        if file.metadata()?.len() > 0 {
            let mut previous = Vec::new();
            file.take(MAX_JSON_BYTES as u64 + 1)
                .read_to_end(&mut previous)?;
            let saved: StartSpec = decode_json(&previous).map_err(std::io::Error::other)?;
            if fingerprint(&saved).map_err(std::io::Error::other)?
                != fingerprint(spec).map_err(std::io::Error::other)?
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "start key has different parameters",
                ));
            }
        } else {
            file.write_all(&serde_json::to_vec(spec)?)?;
            file.sync_all()?;
            std::fs::File::open(&directory)?.sync_all()?;
        }
        Ok(())
    }
    persist(state_dir, spec).map_err(|error| {
        failure(
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                ErrorCode::IdempotencyConflict
            } else {
                ErrorCode::Internal
            },
            OperationOutcome::NotSent,
            "start identity cannot be persisted or conflicts; no request was sent",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qgenie_remote_transport::ipc::{Listener, server_handshake};
    async fn receive(stream: &mut tokio::net::UnixStream) -> Request {
        server_handshake(stream).await.unwrap();
        let DispatchFrame::Local(request) = read_dispatch(stream).await.unwrap() else {
            panic!("expected local request");
        };
        request
    }
    #[tokio::test]
    async fn lost_mutation_reply_is_unknown_without_a_retry() {
        let temp = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        let listener = Listener::bind(&temp.path().join("dispatch.sock")).unwrap();
        let peer = tokio::spawn(async move {
            let mut stream = listener.accept().await.unwrap();
            let _: Request = receive(&mut stream).await;
            drop(stream);
        });
        let client = Client::local(temp.path());
        let error = client.request(Operation::List).await.unwrap_err();
        assert!(matches!(error.outcome, OperationOutcome::Unknown));
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn lost_start_reply_uses_read_only_lookup_and_preserves_original_key() {
        let temp = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        let listener = Listener::bind(&temp.path().join("dispatch.sock")).unwrap();
        let spec = StartSpec {
            key: IdempotencyKey::new(),
            command: vec!["/bin/sh".into()],
            cwd: "/tmp".into(),
            rows: 24,
            cols: 80,
        };
        let key = spec.key;
        let id = SessionId::new();
        let peer = tokio::spawn(async move {
            let mut stream = listener.accept().await.unwrap();
            let first: Request = receive(&mut stream).await;
            assert!(
                matches!(first.body,Operation::Start(StartSpec { key:received,.. }) if received==key)
            );
            drop(stream);
            let mut stream = listener.accept().await.unwrap();
            let lookup: Request = receive(&mut stream).await;
            assert!(matches!(lookup.body,Operation::LookupStart { key:received } if received==key));
            let session = SessionInfo {
                agent: None,
                id,
                machine: MachineId::new(),
                command: vec!["/bin/sh".into()],
                cwd: "/tmp".into(),
                state: SessionState::Running,
                created_at_ms: Counter(1),
                ended_at_ms: None,
                head: Counter(0),
                controller: None,
                exit_code: None,
                recording_warning: None,
            };
            let response = Response {
                v: 1,
                re: lookup.id,
                body: Reply::Started(session),
            };
            write_dispatch(&mut stream, &DispatchFrame::LocalReply(response))
                .await
                .unwrap();
        });
        let client = Client::local(temp.path());
        assert_eq!(client.start(spec).await.unwrap().id, id);
        assert!(
            temp.path()
                .join("starts")
                .join(format!("{key}.json"))
                .exists()
        );
        peer.await.unwrap();
    }
}
