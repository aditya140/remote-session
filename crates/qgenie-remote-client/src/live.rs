//! One owned WSS connection with bounded requests/events. Reconnect is caller-owned;
//! an interrupted request is never resent by this driver.
use crate::{GatewayClient, UserCredentials};
use qgenie_remote_proto::{
    rest::AccessToken,
    wire::{DirectoryEntry, Envelope, Message, Session, StartStage},
    *,
};
use qgenie_remote_transport::ws::{
    self, ClientConfig, Connection, Frame, Limits, Priority, SecretHeader,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::JoinHandle,
    time::Instant,
};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveStatus {
    Connected { client: ClientId },
    Stopped,
    Disconnected,
    ResyncRequired,
    ProtocolError,
}
pub struct LiveEvent {
    pub frame: Frame,
    _bytes: OwnedSemaphorePermit,
}
#[derive(Clone)]
pub struct LiveHandle {
    id: ClientId,
    pub(crate) origin: String,
    pub(crate) controls: bool,
    pub(crate) pages: bool,
    pub(crate) metadata: bool,
    pub(crate) roots: bool,
    pub(crate) replays: bool,
    pub(crate) session_changes: bool,
    pub(crate) agents: bool,
    pub(crate) prompt_receipts: bool,
    pub(crate) approval_status: bool,
    pub(crate) approvals: bool,
    pub(crate) start_work: Arc<Semaphore>,
    status: watch::Receiver<LiveStatus>,
    commands: mpsc::Sender<Command>,
    requests: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
    snapshots: Arc<Semaphore>,
}
pub struct LiveClient {
    handle: LiveHandle,
    events: mpsc::Receiver<LiveEvent>,
    status: watch::Receiver<LiveStatus>,
    status_sender: watch::Sender<LiveStatus>,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
    refresh: Option<JoinHandle<()>>,
    credential_failure: Arc<Mutex<Option<RemoteError>>>,
}
struct Command {
    one_way: bool,
    envelope: Envelope,
    deadline: Instant,
    reply: oneshot::Sender<Result<Vec<Envelope>, RemoteError>>,
    _request: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
    _snapshot: Option<OwnedSemaphorePermit>,
}
struct Pending {
    machine: Option<MachineId>,
    session: Option<SessionId>,
    deadline: Instant,
    reply: oneshot::Sender<Result<Vec<Envelope>, RemoteError>>,
    frames: Vec<Envelope>,
    snapshot: Option<Snapshot>,
    attach: bool,
    _request: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
    _snapshot: Option<OwnedSemaphorePermit>,
    response_bytes: Vec<OwnedSemaphorePermit>,
}
struct Snapshot {
    epoch: String,
    seq: Counter,
    rows: u16,
    cols: u16,
    total: u16,
    next: u16,
    format: SnapshotFormat,
}
fn refused(code: ErrorCode, message: &str) -> RemoteError {
    RemoteError {
        code,
        outcome: OperationOutcome::NotSent,
        message: message.into(),
        retryable: false,
    }
}
fn protocol() -> RemoteError {
    RemoteError::rejected(
        ErrorCode::InvalidFrame,
        "invalid correlated gateway response",
    )
}
impl GatewayClient {
    /// Connects with caller-supplied credentials and renews this connection on updates.
    ///
    /// # Errors
    /// Returns authentication, transport, or negotiation errors. No application request
    /// is retried. Dropping the returned client stops its credential-update task.
    pub async fn connect_authenticated(
        &self,
        credentials: &UserCredentials,
    ) -> Result<LiveClient, RemoteError> {
        let mut updates = credentials.clone();
        let mut client = self.connect(&updates.current()?).await?;
        let handle = client.handle();
        let stop = client.stop.clone();
        let failure = client.credential_failure.clone();
        client.refresh = Some(tokio::spawn(async move {
            loop {
                let result = match updates.changed().await {
                    Ok(token) => handle.reauthenticate(token).await.map(|_| ()),
                    Err(error) => Err(error),
                };
                if let Err(error) = result {
                    *failure.lock().expect("credential failure lock poisoned") = Some(error);
                    stop.send_replace(true);
                    break;
                }
            }
        }));
        Ok(client)
    }
    pub async fn connect(&self, token: &AccessToken) -> Result<LiveClient, RemoteError> {
        let mut config = ClientConfig::new(&format!(
            "{}/v1/connect/client",
            self.config.origin.replacen("https://", "wss://", 1)
        ))
        .map_err(|_| {
            refused(
                ErrorCode::InvalidFrame,
                "invalid gateway connection configuration",
            )
        })?
        .authorization(
            SecretHeader::bearer(token.as_str())
                .map_err(|_| refused(ErrorCode::Unauthenticated, "invalid injected credential"))?,
        );
        if let Some(ca) = &self.config.ca_bundle {
            config = config.ca_bundle(ca);
        }
        let mut connection =
            ws::connect(config, Limits::default(), None)
                .await
                .map_err(|error| {
                    refused(
                        match error {
                            ws::Error::UpgradeRejected(401) => ErrorCode::Unauthenticated,
                            ws::Error::UpgradeRejected(403) => ErrorCode::Forbidden,
                            _ => ErrorCode::AccessStale,
                        },
                        "gateway WSS connection unavailable",
                    )
                })?;
        let required: BTreeSet<String> = ["routing-v1", "machine-pages-v1", "auth-refresh-v1"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let hello = Envelope::new(Message::Hello(Hello {
            versions: vec![Version { major: 1, minor: 0 }],
            required: required.clone(),
            optional: [
                "session-control-v1".into(),
                "session-pages-v1".into(),
                "session-metadata-v1".into(),
                "root-discovery-v1".into(),
                "session-replay-v1".into(),
                "session-changes-v1".into(),
                "agent-events-v1".into(),
                "prompt-receipts-v1".into(),
                "approval-status-v1".into(),
                "durable-approvals-v1".into(),
            ]
            .into(),
        }));
        let id = tokio::time::timeout(Duration::from_secs(8), async {
            connection
                .sender()
                .try_send(Priority::Control, Frame::Text(Box::new(hello)))?
                .transmitted()
                .await?;
            let first = connection.recv().await.ok_or_else(protocol)?;
            let Frame::Text(first) = &first.frame else {
                return Err(protocol());
            };
            let Message::Welcome(welcome) = &first.message else {
                return Err(refused(
                    ErrorCode::UnsupportedVersion,
                    "gateway compatibility negotiation failed",
                ));
            };
            if welcome.version.major != 1
                || welcome.version.minor != 0
                || !required.iter().all(|v| welcome.capabilities.contains(v))
            {
                return Err(refused(
                    ErrorCode::UnsupportedVersion,
                    "gateway lacks required client capabilities",
                ));
            }
            let ready = connection.recv().await.ok_or_else(protocol)?;
            match ready.frame {
                Frame::Text(ref message) => match &message.message {
                    Message::ConnectionReady {
                        client,
                        host_generation: None,
                        activated: false,
                    } => Ok((
                        *client,
                        welcome.capabilities.contains("session-control-v1"),
                        welcome.capabilities.contains("session-replay-v1"),
                        welcome.capabilities.contains("agent-events-v1"),
                        welcome.capabilities.contains("prompt-receipts-v1"),
                        welcome.capabilities.contains("approval-status-v1"),
                        welcome.capabilities.contains("durable-approvals-v1"),
                        welcome.capabilities.contains("session-changes-v1"),
                        welcome.capabilities.contains("session-pages-v1"),
                        welcome.capabilities.contains("session-metadata-v1"),
                        welcome.capabilities.contains("root-discovery-v1"),
                    )),
                    _ => Err(protocol()),
                },
                _ => Err(protocol()),
            }
        })
        .await
        .map_err(|_| {
            refused(
                ErrorCode::AccessStale,
                "gateway negotiation deadline exceeded",
            )
        })??;
        let (
            id,
            controls,
            replays,
            agents,
            prompt_receipts,
            approval_status,
            approvals,
            session_changes,
            pages,
            metadata,
            roots,
        ) = id;
        let (commands, receive) = mpsc::channel(64);
        let (events, subscription) = mpsc::channel(64);
        let (stop, cancel) = watch::channel(false);
        let (status, updates) = watch::channel(LiveStatus::Connected { client: id });
        let handle = LiveHandle {
            id,
            origin: self.config.origin.clone(),
            controls,
            pages,
            metadata,
            roots,
            replays,
            session_changes,
            agents,
            prompt_receipts,
            approval_status,
            approvals,
            start_work: Arc::new(Semaphore::new(4)),
            status: updates.clone(),
            commands,
            requests: Arc::new(Semaphore::new(128)),
            bytes: Arc::new(Semaphore::new(8 * 1024 * 1024)),
            snapshots: Arc::new(Semaphore::new(4)),
        };
        let task = tokio::spawn(run(connection, receive, events, status.clone(), cancel));
        Ok(LiveClient {
            handle,
            events: subscription,
            status: updates,
            status_sender: status,
            stop,
            task: Some(task),
            refresh: None,
            credential_failure: Arc::new(Mutex::new(None)),
        })
    }
}
impl LiveClient {
    /// Explains a closed connection without exposing credential material.
    pub fn disconnect_error(&self) -> RemoteError {
        let status = self.status.borrow().clone();
        if status == LiveStatus::ProtocolError {
            return refused(ErrorCode::InvalidFrame, "gateway protocol failed");
        }
        if let Some(error) = self
            .credential_failure
            .lock()
            .expect("credential failure lock poisoned")
            .clone()
        {
            if status == LiveStatus::Disconnected && error.code == ErrorCode::Internal {
                // A lost WSS refresh acknowledgment is safe to replace with a new
                // authenticated connection. Application actions remain uncertain.
                return refused(
                    ErrorCode::AccessStale,
                    "gateway connection lost during reauthentication",
                );
            }
            return error;
        }
        match status {
            LiveStatus::ResyncRequired => refused(
                ErrorCode::Overloaded,
                "connection consumer fell behind; install a new checkpoint",
            ),
            _ => refused(ErrorCode::AccessStale, "gateway connection lost"),
        }
    }
    pub fn handle(&self) -> LiveHandle {
        self.handle.clone()
    }
    pub fn status(&self) -> watch::Receiver<LiveStatus> {
        self.status.clone()
    }
    pub async fn next(&mut self) -> Option<LiveEvent> {
        let event = self.events.recv().await?;
        if let Frame::Text(envelope) = &event.frame
            && let Message::Error(error) = &envelope.message
            && matches!(
                error.code,
                ErrorCode::Unauthenticated | ErrorCode::Forbidden
            )
        {
            *self
                .credential_failure
                .lock()
                .expect("credential failure lock poisoned") = Some(error.clone());
        }
        Some(event)
    }
    pub async fn close(mut self) {
        if let Some(task) = self.refresh.take() {
            task.abort();
            let _ = task.await;
        }
        self.stop.send_replace(true);
        if let Some(mut task) = self.task.take()
            && tokio::time::timeout(Duration::from_secs(2), &mut task)
                .await
                .is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }
}
impl Drop for LiveClient {
    fn drop(&mut self) {
        if let Some(task) = self.refresh.take() {
            task.abort();
        }
        self.status_sender.send_replace(LiveStatus::Stopped);
        self.stop.send_replace(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
impl LiveHandle {
    pub fn id(&self) -> ClientId {
        self.id
    }
    pub async fn request(&self, envelope: Envelope) -> Result<Vec<Envelope>, RemoteError> {
        if envelope.re.is_some()
            || !matches!(
                envelope.message,
                Message::SessionList
                    | Message::SessionListPage { .. }
                    | Message::SessionGet { .. }
                    | Message::SessionResolve { .. }
                    | Message::HostGenerationQuery
                    | Message::StartLookup { .. }
                    | Message::FsList { .. }
                    | Message::FsRoots
                    | Message::Attach { .. }
                    | Message::Capture
                    | Message::ReplayRequest { .. }
                    | Message::ApprovalList
                    | Message::Reauthenticate { .. }
            )
        {
            return Err(refused(
                ErrorCode::UnsupportedCapability,
                "this SDK request path currently supports reads and explicit credential refresh",
            ));
        }
        self.submit(envelope).await
    }
    pub(crate) async fn submit(&self, envelope: Envelope) -> Result<Vec<Envelope>, RemoteError> {
        self.enqueue(envelope, false).await
    }
    pub(crate) async fn detach_stream(&self, envelope: Envelope) -> Result<(), RemoteError> {
        self.enqueue(envelope, true).await.map(|_| ())
    }
    async fn enqueue(
        &self,
        envelope: Envelope,
        one_way: bool,
    ) -> Result<Vec<Envelope>, RemoteError> {
        if !matches!(*self.status.borrow(), LiveStatus::Connected { .. }) {
            return Err(refused(
                ErrorCode::AccessStale,
                "client connection ended; reconnect explicitly",
            ));
        }
        envelope.validate()?;
        let size = envelope.encode()?.len();
        let work = self.requests.clone().try_acquire_owned().map_err(|_| {
            refused(
                ErrorCode::Overloaded,
                "client pending request budget exceeded",
            )
        })?;
        let bytes = self
            .bytes
            .clone()
            .try_acquire_many_owned(size as u32)
            .map_err(|_| refused(ErrorCode::Overloaded, "client pending byte budget exceeded"))?;
        let snapshot =
            if matches!(
                envelope.message,
                Message::Attach { .. } | Message::Capture | Message::ReplayRequest { .. }
            ) {
                Some(self.snapshots.clone().try_acquire_owned().map_err(|_| {
                    refused(ErrorCode::Overloaded, "client snapshot budget exceeded")
                })?)
            } else {
                None
            };
        let (reply, receive) = oneshot::channel();
        self.commands
            .try_send(Command {
                one_way,
                envelope,
                deadline: Instant::now() + Duration::from_secs(10),
                reply,
                _request: work,
                _bytes: bytes,
                _snapshot: snapshot,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    refused(ErrorCode::Overloaded, "client connection queue unavailable")
                }
                mpsc::error::TrySendError::Closed(_) => refused(
                    ErrorCode::AccessStale,
                    "client connection ended; reconnect explicitly",
                ),
            })?;
        receive.await.unwrap_or_else(|_| {
            Err(RemoteError::unknown(
                "connection ended before a correlated response; request was not retried",
            ))
        })
    }
    /// Reads one ID-ordered page for a machine. `limit` must be 1..=256.
    /// Use the returned cursor for the next page. Pages are not a transactional snapshot;
    /// restart traversal to include concurrent inserts before the cursor.
    ///
    /// # Errors
    /// Returns an error for invalid bounds, denied or unavailable access, or malformed
    /// metadata. An older gateway uses the bounded legacy complete-list fallback.
    pub async fn session_page(
        &self,
        machine: MachineId,
        after: Option<SessionId>,
        limit: u16,
        active_only: bool,
    ) -> Result<(Vec<Session>, Option<SessionId>), RemoteError> {
        if !(1..=256).contains(&limit) {
            return Err(refused(
                ErrorCode::InvalidFrame,
                "session page limit must be 1..=256",
            ));
        }
        if !self.pages {
            if after.is_some() {
                return Err(refused(
                    ErrorCode::UnsupportedCapability,
                    "gateway does not support session pages",
                ));
            }
            let mut sessions = self.sessions(machine).await?;
            if active_only {
                sessions.retain(|s| {
                    matches!(
                        s.state,
                        SessionState::Accepted
                            | SessionState::Launching
                            | SessionState::Running
                            | SessionState::Idle
                    )
                });
            }
            return Ok((sessions, None));
        }
        let mut request = Envelope::new(Message::SessionListPage {
            after,
            limit,
            active_only,
        });
        request.machine = Some(machine);
        match self.request(request).await?.pop().map(|v| v.message) {
            Some(Message::SessionPage { sessions, next })
                if valid_session_page(&sessions, next, machine, after, limit) =>
            {
                Ok((sessions, next))
            }
            _ => Err(protocol()),
        }
    }
    pub async fn session(&self, machine: MachineId, id: SessionId) -> Result<Session, RemoteError> {
        if !self.pages {
            return self
                .sessions(machine)
                .await?
                .into_iter()
                .find(|s| s.id == id)
                .ok_or_else(|| RemoteError::rejected(ErrorCode::NotFound, "session not found"));
        }
        let mut request = Envelope::new(Message::SessionGet { session: id });
        request.machine = Some(machine);
        match self.request(request).await?.pop().map(|v| v.message) {
            Some(Message::SessionFound(session))
                if session.id == id && session.machine == machine =>
            {
                Ok(session)
            }
            _ => Err(protocol()),
        }
    }
    pub async fn sessions(&self, machine: MachineId) -> Result<Vec<Session>, RemoteError> {
        let mut request = Envelope::new(Message::SessionList);
        request.machine = Some(machine);
        let mut replies = self.request(request).await?;
        match replies.pop().map(|v| v.message) {
            Some(Message::Sessions { sessions })
                if sessions.iter().all(|s| s.machine == machine) =>
            {
                Ok(sessions)
            }
            _ => Err(protocol()),
        }
    }
    pub async fn allowed_roots(&self, machine: MachineId) -> Result<Vec<String>, RemoteError> {
        if !self.roots {
            return Err(refused(
                ErrorCode::UnsupportedCapability,
                "gateway does not support root discovery; enter an allowed path manually",
            ));
        }
        let mut request = Envelope::new(Message::FsRoots);
        request.machine = Some(machine);
        match self.request(request).await?.pop().map(|v| v.message) {
            Some(Message::FsRootsListed { roots }) => Ok(roots),
            _ => Err(protocol()),
        }
    }
    pub async fn browse(
        &self,
        machine: MachineId,
        path: String,
        cursor: Option<String>,
        limit: u16,
    ) -> Result<(Vec<DirectoryEntry>, Option<String>), RemoteError> {
        let mut request = Envelope::new(Message::FsList {
            path,
            cursor,
            limit,
        });
        request.machine = Some(machine);
        match self.request(request).await?.pop().map(|v| v.message) {
            Some(Message::FsListed { entries, next }) => Ok((entries, next)),
            _ => Err(protocol()),
        }
    }
    pub async fn lookup_start(
        &self,
        machine: MachineId,
        key: IdempotencyKey,
    ) -> Result<(StartStage, Option<SessionId>), RemoteError> {
        let mut request = Envelope::new(Message::StartLookup { key });
        request.machine = Some(machine);
        match self.request(request).await?.pop().map(|v| v.message) {
            Some(Message::StartStatus {
                key: actual,
                stage,
                session,
            }) if actual == key => Ok((stage, session)),
            _ => Err(protocol()),
        }
    }
    pub async fn reauthenticate(
        &self,
        token: AccessToken,
    ) -> Result<qgenie_remote_proto::wire::Timestamp, RemoteError> {
        match self
            .request(Envelope::new(Message::Reauthenticate { token }))
            .await?
            .pop()
            .map(|v| v.message)
        {
            Some(Message::AuthRefreshed { expires_at }) => Ok(expires_at),
            _ => Err(protocol()),
        }
    }
}
fn valid_session_page(
    sessions: &[Session],
    next: Option<SessionId>,
    machine: MachineId,
    after: Option<SessionId>,
    limit: u16,
) -> bool {
    sessions.len() <= usize::from(limit)
        && sessions
            .iter()
            .all(|s| s.machine == machine && after.is_none_or(|id| s.id > id))
        && sessions.windows(2).all(|pair| pair[0].id < pair[1].id)
        && next.is_none_or(|id| sessions.last().is_some_and(|s| s.id == id))
}
impl Pending {
    fn accept(&mut self, envelope: Envelope, budget: &Arc<Semaphore>) -> Result<bool, RemoteError> {
        if let Message::Error(error) = &envelope.message {
            if envelope.machine.is_some_and(|v| Some(v) != self.machine)
                || envelope.session.is_some_and(|v| Some(v) != self.session)
            {
                return Err(protocol());
            }
            return Err(error.clone());
        }
        if envelope.machine != self.machine || envelope.session != self.session {
            return Err(protocol());
        }
        let done = match &envelope.message {
            Message::Snapshot { snapshot: page } => {
                if !self.attach {
                    return Err(protocol());
                }
                let progress = self.snapshot.get_or_insert_with(|| Snapshot {
                    epoch: page.epoch.clone(),
                    seq: page.seq,
                    rows: page.rows,
                    cols: page.cols,
                    total: page.total_pages,
                    next: 0,
                    format: page.format,
                });
                if page.page != progress.next
                    || page.total_pages != progress.total
                    || page.epoch != progress.epoch
                    || page.seq != progress.seq
                    || page.rows != progress.rows
                    || page.cols != progress.cols
                    || page.format != progress.format
                {
                    return Err(protocol());
                }
                progress.next += 1;
                false
            }
            Message::Head { head, .. }
                if self
                    .snapshot
                    .as_ref()
                    .is_some_and(|p| p.next == p.total && *head >= p.seq) =>
            {
                true
            }
            _ if self.snapshot.is_some() => return Err(protocol()),
            _ => true,
        };
        if self.frames.len() >= 33 {
            return Err(protocol());
        }
        self.response_bytes.push(
            budget
                .clone()
                .try_acquire_many_owned(envelope.encode()?.len() as u32)
                .map_err(|_| {
                    refused(
                        ErrorCode::Overloaded,
                        "client response assembly budget exceeded",
                    )
                })?,
        );
        self.frames.push(envelope);
        Ok(done)
    }
}
async fn run(
    mut connection: Connection,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<LiveEvent>,
    status: watch::Sender<LiveStatus>,
    mut stop: watch::Receiver<bool>,
) {
    let mut pending: HashMap<MessageId, Pending> = HashMap::new();
    let mut correlations = HashSet::new();
    let event_budget = Arc::new(Semaphore::new(8 * 1024 * 1024));
    let response_budget = Arc::new(Semaphore::new(16 * 1024 * 1024));
    let mut timer = tokio::time::interval(Duration::from_millis(100));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let ended = loop {
        tokio::select! {biased;
            _=stop.changed()=>break LiveStatus::Stopped,
            _=timer.tick()=>{
                let expired:Vec<_>=pending.iter().filter(|(_,p)|p.deadline<=Instant::now() || p.reply.is_closed()).map(|(id,_)|*id).collect();
                for id in expired {if let Some(p)=pending.remove(&id) {let _=p.reply.send(Err(RemoteError::unknown("correlated response deadline exceeded; request was not retried")));}}
            },
            command=commands.recv()=>{
                let Some(command)=command else {break LiveStatus::Stopped;};
                if command.reply.is_closed() {continue;}
                let id=command.envelope.id;
                if correlations.contains(&id) {let _=command.reply.send(Err(refused(ErrorCode::IdempotencyConflict,"request correlation was already used on this connection")));continue;}
                if correlations.len()>=65536 {let _=command.reply.send(Err(refused(ErrorCode::Overloaded,"connection correlation budget exhausted; reconnect explicitly")));continue;}
                if command.deadline<=Instant::now() {let _=command.reply.send(Err(refused(ErrorCode::AccessStale,"request expired before transmission")));continue;}
                let machine=command.envelope.machine;let session=command.envelope.session;let attach=matches!(command.envelope.message,Message::Attach{..}|Message::Capture|Message::ReplayRequest{..});
                correlations.insert(id);
                if let Err(error)=connection.sender().try_send(Priority::Control,Frame::Text(Box::new(command.envelope))) {
                    if connection.sender().is_closed() {
                        // The transport can fail before this task polls its
                        // receive branch. Publish the cause before the caller
                        // considers reconnecting after the failed submission.
                        let ended=closed_status(&mut connection).await;
                        status.send_replace(ended.clone());
                        let error=if ended==LiveStatus::ProtocolError {
                            refused(ErrorCode::InvalidFrame,"gateway protocol failed")
                        } else {error};
                        let _=command.reply.send(Err(error));
                        break ended;
                    }
                    let _=command.reply.send(Err(error));continue;
                }
                if command.one_way {let _=command.reply.send(Ok(Vec::new()));continue;}
                pending.insert(id,Pending{machine,session,deadline:command.deadline,reply:command.reply,frames:Vec::new(),snapshot:None,attach,_request:command._request,_bytes:command._bytes,_snapshot:command._snapshot,response_bytes:Vec::new()});
            },
            incoming=connection.recv()=>{
                let Some(incoming)=incoming else {
                    break closed_status(&mut connection).await;
                };
                let frame=incoming.frame.clone();
                if let Frame::Text(envelope)=&frame && let Some(re)=envelope.re {
                    if let Some(mut request)=pending.remove(&re) {
                        if request.deadline<=Instant::now() {let _=request.reply.send(Err(RemoteError::unknown("response arrived after its original request deadline")));continue;}
                        match request.accept(*envelope.clone(),&response_budget) {
                            Ok(true)=>{let _=request.reply.send(Ok(request.frames));},Ok(false)=>{pending.insert(re,request);},Err(error)=>{let _=request.reply.send(Err(error));},
                        }
                    }
                    continue; // A late duplicate cannot complete another logical request.
                }
                let encoded=match &frame {Frame::Text(v)=>v.encode(),Frame::Binary(v)=>v.encode()};
                let size=match encoded {Ok(v)=>v.len(),Err(_)=>break LiveStatus::ProtocolError};
                let Ok(bytes)=event_budget.clone().try_acquire_many_owned(size as u32) else {break LiveStatus::ResyncRequired;};
                if events.try_send(LiveEvent{frame,_bytes:bytes}).is_err() {break LiveStatus::ResyncRequired;}
            }
        }
    };
    status.send_replace(ended);
    for (_, request) in pending {
        let _ = request.reply.send(Err(RemoteError::unknown(
            "connection interrupted; request was not retried",
        )));
    }
    drop(commands);
    let _ = tokio::time::timeout(Duration::from_secs(1), connection.close()).await;
}

async fn closed_status(connection: &mut Connection) -> LiveStatus {
    match connection.closed().await {
        Err(ws::Error::Protocol(_)) => LiveStatus::ProtocolError,
        Err(ws::Error::SlowConsumer) => LiveStatus::ResyncRequired,
        _ => LiveStatus::Disconnected,
    }
}

#[cfg(test)]
mod tests;
