mod authority;
mod capture;
mod changes;
pub use capture::RecordingSettings;
mod clock;
mod control;
#[cfg(all(feature = "managed-codex", target_os = "linux"))]
mod managed;
mod remote_effects;
mod remote_replay;
mod snapshot;
mod start;
use crate::store::Store;
use base64::{Engine, engine::general_purpose::STANDARD};
use fs2::FileExt;
#[cfg(all(feature = "managed-codex", target_os = "linux"))]
pub use managed::CodexProfile;
use qgenie_remote_adapters::{LaunchEnvironment, PreparedPty, PtyControl, Screen};
use qgenie_remote_policy::{Action, Facts, authorize};
use qgenie_remote_proto::ipc::RemoteScope;
use qgenie_remote_proto::*;
use qgenie_remote_transport::ipc::{private_directory, private_file};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct Config {
    pub state_dir: PathBuf,
    pub roots: Vec<PathBuf>,
    pub replay_events: usize,
    pub replay_bytes: usize,
    pub max_sessions: usize,
    /// Locally trusted enrollment bindings. Adding a new binding never adopts
    /// old sessions or changes an existing binding's owner.
    pub remote_bindings: Vec<RemoteScope>,
    pub recording: RecordingSettings,
    /// Host-only provider/binary configuration. Never supplied by a remote launch.
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    pub codex: Option<CodexProfile>,
    #[cfg(test)]
    start_fault: Option<StartFault>,
}
impl Config {
    pub fn new(state_dir: PathBuf, roots: Vec<PathBuf>) -> Self {
        Self {
            state_dir,
            roots,
            replay_events: 10_000,
            replay_bytes: 8 * 1024 * 1024,
            max_sessions: 64,
            remote_bindings: Vec::new(),
            recording: RecordingSettings::default(),
            #[cfg(all(feature = "managed-codex", target_os = "linux"))]
            codex: None,
            #[cfg(test)]
            start_fault: None,
        }
    }
}
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum StartFault {
    BeforeJournal,
    Accepted,
    Launching,
    NativeLaunched,
    RunningCommitted,
    BeforeAck,
}

struct Session {
    agent_state: Option<AgentState>,
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    agent_control: Option<managed::Control>,
    recording: Option<capture::Capture>,
    remote_controller: Option<control::RemoteController>,
    info: SessionInfo,
    spec: StartSpec,
    fingerprint: String,
    generation: u64,
    input_seq: u64,
    deadline: Option<Instant>,
    deadline_elapsed: Option<Duration>,
    control: Option<PtyControl>,
    screen: Option<Screen>,
    pending_lost_bytes: usize,
    scope: Option<RemoteScope>,
}
pub struct Runtime {
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    approvals: crate::NativeApprovals,
    config: Config,
    directories: crate::directory::Roots,
    machine: MachineId,
    store: Mutex<Store>,
    sessions: Mutex<BTreeMap<SessionId, Arc<Mutex<Session>>>>,
    changes: Mutex<BTreeMap<RemoteScope, tokio::sync::watch::Sender<u64>>>,
    starts: Mutex<()>,
    readers: Mutex<Vec<thread::JoinHandle<()>>>,
    snapshots: Mutex<snapshot::Snapshots>,
    snapshot_work: tokio::sync::Semaphore,
    authority: Mutex<authority::Authority>,
    recording: Mutex<capture::Worker>,
    stopping: AtomicBool,
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    agent_verified: AtomicBool,
    // Deduplication never persists a publicly enumerable keystroke hash. Old
    // generations cannot become live again after this process key is lost.
    input_key: zeroize::Zeroizing<[u8; 32]>,
    _owner_lock: std::fs::File,
}

impl Runtime {
    pub(crate) fn can_resume_history(&self) -> bool {
        #[cfg(all(feature = "managed-codex", target_os = "linux"))]
        {
            self.config.codex.is_some()
        }
        #[cfg(not(all(feature = "managed-codex", target_os = "linux")))]
        {
            false
        }
    }
    pub(crate) async fn prepare_agents(&self) -> anyhow::Result<()> {
        #[cfg(all(feature = "managed-codex", target_os = "linux"))]
        if let Some(profile) = &self.config.codex {
            qgenie_remote_adapters::Codex::verify_version(&profile.binary).await?;
            self.agent_verified.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub(crate) fn managed_enabled(&self) -> bool {
        #[cfg(all(feature = "managed-codex", target_os = "linux"))]
        {
            true
        }
        #[cfg(not(all(feature = "managed-codex", target_os = "linux")))]
        {
            false
        }
    }
    pub(crate) fn is_agent_session(&self, id: SessionId) -> bool {
        self.session(id)
            .is_ok_and(|session| session.lock().is_ok_and(|state| state.info.agent.is_some()))
    }
    /// Allow a prior owner's destructor or an inherited close-on-exec file
    /// description to finish releasing the lock. Never recover state until the
    /// exclusive lock actually succeeds, and never retry another error.
    pub(crate) fn open_after_previous_owner(config: Config) -> anyhow::Result<Arc<Self>> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match Self::open(config.clone()) {
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::WouldBlock)
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(5))
                }
                result => return result,
            }
        }
    }
    #[cfg(test)]
    fn fault(&self, point: StartFault) {
        if self.config.start_fault == Some(point) {
            std::process::exit(86);
        }
    }
    pub fn open(mut config: Config) -> anyhow::Result<Arc<Self>> {
        private_directory(&config.state_dir)?;
        let owner_lock = private_file(&config.state_dir.join("owner.lock"))?;
        owner_lock.try_lock_exclusive().map_err(|error| {
            anyhow::Error::new(error).context("another session owner holds this state directory")
        })?;
        anyhow::ensure!(
            config.max_sessions > 0 && config.max_sessions <= 64,
            "session cap must be 1..=64"
        );
        anyhow::ensure!(
            config.recording.content_bytes > 0
                && (64 * 1024..=64 * 1024 * 1024).contains(&config.recording.metadata_bytes),
            "invalid mandatory recording budgets"
        );
        anyhow::ensure!(
            (1..=10_000).contains(&config.replay_events)
                && (64 * 1024..=8 * 1024 * 1024).contains(&config.replay_bytes),
            "invalid replay limits"
        );
        let directories = crate::directory::Roots::open(&config.roots)?;
        config.roots = directories.paths();
        let store = Store::open(
            &config.state_dir.join("sessions.db"),
            config.replay_events,
            config.replay_bytes,
        )?;
        let machine = store.machine()?;
        anyhow::ensure!(
            config.remote_bindings.len() <= 64,
            "at most 64 native remote bindings"
        );
        for binding in &config.remote_bindings {
            store.bind(binding)?;
        }
        let had_recording_spool = config
            .state_dir
            .join("recordings/metadata.sqlite")
            .symlink_metadata()
            .is_ok();
        let mut sessions = BTreeMap::new();
        for mut stored in store.load(machine)? {
            if stored.scope.is_some() && !had_recording_spool {
                stored.info.recording_warning = Some(
                    "Recording spool metadata is absent; historical completeness is unknown".into(),
                );
            }
            sessions.insert(
                stored.info.id,
                Arc::new(Mutex::new(Session {
                    agent_state: None,
                    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
                    agent_control: None,
                    recording: None,
                    remote_controller: None,
                    info: stored.info,
                    spec: stored.spec,
                    fingerprint: stored.fingerprint,
                    generation: stored.generation,
                    input_seq: 0,
                    deadline: None,
                    deadline_elapsed: None,
                    control: None,
                    screen: None,
                    pending_lost_bytes: 0,
                    scope: stored.scope,
                })),
            );
        }
        let runtime = Arc::new(Self {
            #[cfg(all(feature = "managed-codex", target_os = "linux"))]
            approvals: crate::NativeApprovals::open(&config.state_dir.join("approvals"))?,
            config,
            directories,
            machine,
            store: Mutex::new(store),
            sessions: Mutex::new(sessions),
            changes: Mutex::new(BTreeMap::new()),
            starts: Mutex::new(()),
            readers: Mutex::new(Vec::new()),
            snapshots: Mutex::new(snapshot::Snapshots::default()),
            snapshot_work: tokio::sync::Semaphore::new(4),
            authority: Mutex::new(authority::Authority::default()),
            recording: Mutex::new(capture::Worker::Empty),
            stopping: AtomicBool::new(false),
            #[cfg(all(feature = "managed-codex", target_os = "linux"))]
            agent_verified: AtomicBool::new(false),
            input_key: zeroize::Zeroizing::new(rand::random()),
            _owner_lock: owner_lock,
        });
        if had_recording_spool {
            let _ = runtime.recording_worker();
        }
        Ok(runtime)
    }

    pub fn dispatch(self: &Arc<Self>, request: Request) -> Response {
        let reply = if request.v != PROTOCOL_MAJOR {
            Err(RemoteError::rejected(
                ErrorCode::UnsupportedVersion,
                "supported protocol major is 1",
            ))
        } else {
            self.execute(request.client, request.body)
        };
        Response {
            v: PROTOCOL_MAJOR,
            re: request.id,
            body: reply.unwrap_or_else(Reply::Error),
        }
    }

    fn execute(
        self: &Arc<Self>,
        client: ClientId,
        operation: Operation,
    ) -> Result<Reply, RemoteError> {
        self.execute_guarded(client, operation, None)
    }
    fn execute_guarded(
        self: &Arc<Self>,
        client: ClientId,
        operation: Operation,
        remote: Option<&remote_effects::Execution<'_>>,
    ) -> Result<Reply, RemoteError> {
        match operation {
            Operation::Status => Ok(Reply::Status(HostStatus {
                machine: self.machine,
                name: std::env::var("HOSTNAME").unwrap_or_else(|_| "Local machine".into()),
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
                version: env!("CARGO_PKG_VERSION").into(),
                roots: self
                    .config
                    .roots
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect(),
                sessions: lock(&self.sessions)?.len(),
                mode: "local preview (same OS user; no gateway)".into(),
            })),
            Operation::List => Ok(Reply::Sessions(self.list()?)),
            Operation::Start(spec) => self.start(spec).map(Reply::Started),
            Operation::LookupStart { key } => {
                let _start = lock(&self.starts)?;
                let sessions: Vec<_> = lock(&self.sessions)?.values().cloned().collect();
                for session in sessions {
                    let session = lock(&session)?;
                    if session.spec.key == key && session.scope.is_none() {
                        return Ok(Reply::Started(session.info.clone()));
                    }
                }
                Err(RemoteError::unknown(
                    "no known session for this key; no new launch was sent",
                ))
            }
            Operation::Approvals => Ok(Reply::Approvals(Vec::new())),
            Operation::Attach { session } => {
                let _capture = self.snapshot_work.try_acquire().map_err(|_| {
                    RemoteError::rejected(
                        ErrorCode::Overloaded,
                        "terminal capture capacity unavailable",
                    )
                })?;
                let session = self.session(session)?;
                let mut state = lock(&session)?;
                self.commit_gap(&mut state)?;
                expire(&mut state);
                let snapshot = self.snapshot(&state)?;
                let snapshot = lock(&self.snapshots)?.capture(
                    state.info.id,
                    client,
                    state.info.head,
                    snapshot,
                )?;
                Ok(Reply::Attached(Attachment {
                    session: state.info.clone(),
                    snapshot,
                }))
            }
            Operation::SnapshotPage {
                session,
                epoch,
                page,
            } => {
                self.session(session)?;
                lock(&self.snapshots)?
                    .page(session, client, epoch, page)
                    .map(Reply::SnapshotPage)
            }
            Operation::Replay { session, after } => {
                let session = self.session(session)?;
                let mut state = lock(&session)?;
                if state.info.agent.is_some() {
                    return Err(RemoteError::rejected(
                        ErrorCode::UnsupportedCapability,
                        "structured sessions require the negotiated agent replay contract",
                    ));
                }
                expire(&mut state);
                self.commit_gap(&mut state)?;
                renew(&mut state, client);
                let (floor, events, more) = lock(&self.store)?
                    .replay(state.info.id, after)
                    .map_err(storage_error)?;
                let truncated = after.0.checked_add(1).is_some_and(|next| floor.0 > next)
                    || after > state.info.head
                    || (after < state.info.head
                        && events
                            .first()
                            .is_none_or(|event| after.0.checked_add(1) != Some(event.seq.0)));
                Ok(Reply::Replay(ReplayBatch {
                    session: state.info.clone(),
                    floor,
                    head: state.info.head,
                    truncated,
                    events,
                    more,
                }))
            }
            Operation::TakeControl { session, takeover } => {
                let session = self.session(session)?;
                let mut state = lock(&session)?;
                expire(&mut state);
                ensure_running(&state)?;
                if let Some(lease) = &state.info.controller {
                    if lease.holder == client && state.remote_controller.is_none() {
                        renew(&mut state, client);
                        return state
                            .info
                            .controller
                            .clone()
                            .map(Reply::Control)
                            .ok_or_else(|| {
                                RemoteError::rejected(
                                    ErrorCode::StaleGeneration,
                                    "control expired during renewal",
                                )
                            });
                    }
                    if !takeover {
                        return Err(RemoteError::rejected(
                            ErrorCode::ControlHeld,
                            "another client controls this session; confirm takeover",
                        ));
                    }
                }
                let generation = state.generation.checked_add(1).ok_or_else(|| {
                    RemoteError::rejected(ErrorCode::CapacityExhausted, "generation exhausted")
                })?;
                let deadline = Instant::now() + Duration::from_secs(15);
                let elapsed_deadline = clock::elapsed()? + Duration::from_secs(15);
                let persisted = lock(&self.store).and_then(|store| {
                    store
                        .generation(state.info.id, generation)
                        .map_err(storage_error)
                });
                if let Err(error) = persisted {
                    // The commit outcome can be uncertain. Fence the prior
                    // holder and skip this candidate in memory; never revive it
                    // merely because the caller did not receive activation.
                    state.generation = generation;
                    clear_control(&mut state);
                    return Err(error);
                }
                state.generation = generation;
                state.input_seq = 0;
                clear_control(&mut state);
                let remaining = elapsed_deadline
                    .saturating_sub(clock::elapsed()?)
                    .min(deadline.saturating_duration_since(Instant::now()));
                if remaining.is_zero() {
                    return Err(RemoteError::rejected(
                        ErrorCode::StaleGeneration,
                        "control expired while persisting activation",
                    ));
                }
                let lease = Lease {
                    holder: client,
                    generation: Counter(generation),
                    expires_at_ms: Counter(now_ms() + remaining.as_millis() as u64),
                    last_input: Counter(0),
                };
                state.deadline = Some(deadline);
                state.deadline_elapsed = Some(elapsed_deadline);
                state.info.controller = Some(lease.clone());
                Ok(Reply::Control(lease))
            }
            Operation::ReleaseControl { session } => {
                let session = self.session(session)?;
                let mut state = lock(&session)?;
                if state
                    .info
                    .controller
                    .as_ref()
                    .is_some_and(|l| l.holder == client)
                {
                    clear_control(&mut state);
                }
                Ok(Reply::Ok)
            }
            Operation::Input {
                session,
                generation,
                input_seq,
                data,
            } => {
                let bytes = STANDARD.decode(data).map_err(|_| {
                    RemoteError::rejected(ErrorCode::InvalidFrame, "invalid input encoding")
                })?;
                if bytes.is_empty() || bytes.len() > MAX_PTY_BYTES {
                    return Err(RemoteError::rejected(
                        ErrorCode::FrameTooLarge,
                        "input must contain 1..=32768 bytes",
                    ));
                }
                let session = self.session(session)?;
                let mut state = lock(&session)?;
                check_control(&mut state, client, generation, Action::Input, remote)?;
                ensure_pty(&state)?;
                use hmac::{Hmac, Mac};
                let mut digest = Hmac::<sha2::Sha256>::new_from_slice(self.input_key.as_slice())
                    .map_err(|_| RemoteError::unknown("input journal key unavailable"))?;
                digest.update(b"qgenie-remote:input:v1\0");
                digest.update(state.info.id.to_string().as_bytes());
                digest.update(&generation.0.to_be_bytes());
                digest.update(&input_seq.0.to_be_bytes());
                digest.update(&bytes);
                let digest = digest
                    .finalize()
                    .into_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                if let Some(outcome) = lock(&self.store)?
                    .input_lookup(state.info.id, generation, input_seq, &digest)
                    .map_err(|_| {
                        RemoteError::rejected(
                            ErrorCode::IdempotencyConflict,
                            "input sequence conflicts with earlier bytes",
                        )
                    })?
                {
                    check_control(&mut state, client, generation, Action::Input, remote)?;
                    return Ok(Reply::InputAck { input_seq, outcome });
                }
                if state.input_seq.checked_add(1) != Some(input_seq.0) {
                    return Err(RemoteError::rejected(
                        ErrorCode::StaleGeneration,
                        "input sequence is not the next accepted value",
                    ));
                }
                lock(&self.store)?
                    .input_record(
                        state.info.id,
                        generation,
                        input_seq,
                        &digest,
                        ApplicationOutcome::OutcomeUnknown,
                    )
                    .map_err(storage_error)?;
                // A committed input intent consumes its identity even if a
                // later authority check cancels delivery before native bytes.
                state.input_seq = input_seq.0;
                if let Some(lease) = &mut state.info.controller {
                    lease.last_input = input_seq;
                }
                if let Err(error) =
                    check_control(&mut state, client, generation, Action::Input, remote)
                {
                    let _ = lock(&self.store).and_then(|store| {
                        store
                            .input_record(
                                state.info.id,
                                generation,
                                input_seq,
                                &digest,
                                ApplicationOutcome::Cancelled,
                            )
                            .map_err(storage_error)
                    });
                    return Err(error);
                }
                let deadline = state.deadline.ok_or_else(|| {
                    RemoteError::rejected(
                        ErrorCode::StaleGeneration,
                        "control deadline is unavailable",
                    )
                })?;
                let elapsed_deadline = state.deadline_elapsed.ok_or_else(|| {
                    RemoteError::rejected(
                        ErrorCode::StaleGeneration,
                        "control clock is unavailable",
                    )
                })?;
                let outcome = match remote_effects::perform(remote, || {
                    state
                        .control
                        .as_mut()
                        .unwrap()
                        .input_guarded(&bytes, || {
                            remote_effects::deadline(remote, deadline, elapsed_deadline)
                                .map_err(anyhow::Error::new)
                        })
                        .map_err(|_| RemoteError::unknown("native input delivery is uncertain"))
                }) {
                    Ok(()) => ApplicationOutcome::Applied,
                    Err(error) if matches!(error.outcome, OperationOutcome::Rejected) => {
                        ApplicationOutcome::Cancelled
                    }
                    Err(_) => ApplicationOutcome::OutcomeUnknown,
                };
                if lock(&self.store)?
                    .input_record(
                        state.info.id,
                        generation,
                        input_seq,
                        &digest,
                        outcome.clone(),
                    )
                    .is_err()
                {
                    return Ok(Reply::InputAck {
                        input_seq,
                        outcome: ApplicationOutcome::OutcomeUnknown,
                    });
                }
                if remote.is_none() {
                    renew(&mut state, client);
                }
                Ok(Reply::InputAck { input_seq, outcome })
            }
            Operation::Resize {
                session,
                generation,
                rows,
                cols,
            } => {
                validate_size(rows, cols)?;
                let session = self.session(session)?;
                let mut state = lock(&session)?;
                check_control(&mut state, client, generation, Action::Resize, remote)?;
                ensure_pty(&state)?;
                remote_effects::perform(remote, || {
                    state
                        .control
                        .as_ref()
                        .unwrap()
                        .resize(rows, cols)
                        .map_err(|_| RemoteError::unknown("PTY resize outcome is unknown"))
                })?;
                state.screen.as_mut().unwrap().resize(rows, cols)?;
                Self::record(&mut state, crate::CaptureContent::Resize { rows, cols });
                self.append(&mut state, EventBody::Resize { rows, cols })?;
                Ok(Reply::Ok)
            }
            Operation::Interrupt {
                session,
                generation,
            } => {
                let session = self.session(session)?;
                let mut state = lock(&session)?;
                check_control(&mut state, client, generation, Action::Interrupt, remote)?;
                ensure_pty(&state)?;
                let deadline = state.deadline.ok_or_else(|| {
                    RemoteError::rejected(
                        ErrorCode::StaleGeneration,
                        "control deadline is unavailable",
                    )
                })?;
                let elapsed_deadline = state.deadline_elapsed.ok_or_else(|| {
                    RemoteError::rejected(
                        ErrorCode::StaleGeneration,
                        "control clock is unavailable",
                    )
                })?;
                remote_effects::perform(remote, || {
                    state
                        .control
                        .as_mut()
                        .unwrap()
                        .input_guarded(&[3], || {
                            remote_effects::deadline(remote, deadline, elapsed_deadline)
                                .map_err(anyhow::Error::new)
                        })
                        .map_err(|_| RemoteError::unknown("interrupt outcome is unknown"))
                })?;
                Ok(Reply::Ok)
            }
            Operation::Terminate { session, confirmed } => {
                authorize(
                    Action::Terminate,
                    Facts {
                        termination_confirmed: confirmed,
                        ..local_facts()
                    },
                )?;
                let session = self.session(session)?;
                let mut state = lock(&session)?;
                ensure_running(&state)?;
                if let Some(remote) = remote {
                    remote.check_scope(&state)?;
                }
                #[cfg(all(feature = "managed-codex", target_os = "linux"))]
                if let Some(control) = state.agent_control.clone() {
                    let grant = managed::authorization(remote.map(|remote| remote.fence.clone()));
                    drop(state);
                    control.terminate(grant)?;
                    return Ok(Reply::Ok);
                }
                remote_effects::perform(remote, || {
                    state
                        .control
                        .as_mut()
                        .unwrap()
                        .terminate()
                        .map_err(|_| RemoteError::unknown("termination outcome is unknown"))
                })?;
                Ok(Reply::Ok)
            }
        }
    }

    pub fn list(&self) -> Result<Vec<SessionInfo>, RemoteError> {
        let sessions: Vec<_> = lock(&self.sessions)?.values().cloned().collect();
        let mut result = Vec::with_capacity(sessions.len());
        let mut bytes = 0;
        for session in sessions {
            let mut state = lock(&session)?;
            expire(&mut state);
            self.recording_warning(&mut state);
            bytes += serde_json::to_vec(&state.info)
                .map_err(storage_error)?
                .len();
            if bytes > 512 * 1024 {
                return Err(RemoteError::rejected(
                    ErrorCode::Overloaded,
                    "session metadata exceeds the local list limit; use a known session ID until paginated listing is available",
                ));
            }
            result.push(state.info.clone());
        }
        result.sort_by_key(|s| s.created_at_ms);
        Ok(result)
    }

    fn session(&self, id: SessionId) -> Result<Arc<Mutex<Session>>, RemoteError> {
        lock(&self.sessions)?
            .get(&id)
            .cloned()
            .ok_or_else(|| RemoteError::rejected(ErrorCode::NotFound, "session not found"))
    }

    fn output(&self, session: &Mutex<Session>, bytes: &[u8]) {
        let Ok(mut state) = session.lock() else {
            return;
        };
        Self::record(&mut state, crate::CaptureContent::Output(bytes.to_vec()));
        let _ = self.commit_gap(&mut state);
        state.screen.as_mut().unwrap().process(bytes);
        if self
            .append(&mut state, EventBody::PtyOutput(STANDARD.encode(bytes)))
            .is_err()
        {
            state.pending_lost_bytes = state.pending_lost_bytes.saturating_add(bytes.len());
            state.info.recording_warning = Some(
                "Replay persistence failed; output continues locally and a gap will be reported"
                    .into(),
            );
        }
    }
    fn commit_gap(&self, state: &mut Session) -> Result<(), RemoteError> {
        if state.pending_lost_bytes == 0 {
            return Ok(());
        }
        let next = Counter(state.info.head.0.checked_add(1).ok_or_else(|| {
            RemoteError::rejected(ErrorCode::CapacityExhausted, "output cursor exhausted")
        })?);
        self.append(
            state,
            EventBody::Gap {
                from: next,
                to: next,
                reason: format!(
                    "replay storage failure: {} unretained bytes; reattach for screen state",
                    state.pending_lost_bytes
                ),
            },
        )?;
        state.pending_lost_bytes = 0;
        Ok(())
    }
    fn append(&self, state: &mut Session, body: EventBody) -> Result<(), RemoteError> {
        let seq = state.info.head.0.checked_add(1).ok_or_else(|| {
            RemoteError::rejected(ErrorCode::CapacityExhausted, "output cursor exhausted")
        })?;
        let event = OutputEvent {
            seq: Counter(seq),
            at_ms: Counter(now_ms()),
            body,
        };
        let mut agent = state.agent_state.clone();
        if let Some(agent) = &mut agent {
            agent.push(public_event(&event)?)?;
        }
        lock(&self.store)?
            .append(state.info.id, &event, || {
                if let Some(agent) = &agent {
                    return agent.encode();
                }
                state
                    .screen
                    .as_ref()
                    .ok_or_else(|| RemoteError::unknown("no live screen"))?
                    .snapshot()
                    .map(|snapshot| snapshot.data)
            })
            .map_err(storage_error)?;
        state.agent_state = agent;
        state.info.head = Counter(seq);
        self.publish_change(state.scope.as_ref());
        Ok(())
    }

    pub fn shutdown(&self, terminate: bool) -> Result<(), RemoteError> {
        let _start = lock(&self.starts)?;
        let sessions: Vec<_> = lock(&self.sessions)?.values().cloned().collect();
        if !terminate
            && sessions
                .iter()
                .any(|s| s.lock().is_ok_and(|s| owns_process(&s)))
        {
            return Err(RemoteError::rejected(
                ErrorCode::Forbidden,
                "active sessions remain; use explicit --terminate-sessions",
            ));
        }
        self.stopping.store(true, Ordering::Release);
        if terminate {
            for session in sessions {
                // Managed owners observe `stopping` independently, including
                // during protocol loss. A closed request queue cannot block
                // this explicitly confirmed local shutdown.
                if let Some(control) = lock(&session)?.control.as_mut() {
                    control
                        .terminate()
                        .map_err(|_| RemoteError::unknown("termination outcome is unknown"))?;
                }
            }
        }
        Ok(())
    }
}
fn owns_process(state: &Session) -> bool {
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    if state.agent_control.is_some() {
        return true;
    }
    state.control.is_some()
        || matches!(
            state.info.state,
            SessionState::Accepted
                | SessionState::Launching
                | SessionState::Running
                | SessionState::Idle
        )
}
fn public_event(event: &OutputEvent) -> Result<wire::SequencedEvent, RemoteError> {
    Ok(wire::SequencedEvent {
        seq: event.seq,
        at: wire::Timestamp::from_unix_millis(event.at_ms.0)?,
        event: match &event.body {
            EventBody::Agent(event) => event.clone(),
            EventBody::PtyOutput(data) => wire::AgentEvent::PtyOutput { data: data.clone() },
            EventBody::Resize { rows, cols } => wire::AgentEvent::Resize {
                rows: *rows,
                cols: *cols,
            },
            EventBody::Exited { code } => wire::AgentEvent::Exited { code: *code },
            EventBody::Gap { from, to, reason } => wire::AgentEvent::Gap {
                from: *from,
                to: *to,
                reason: reason.clone(),
            },
        },
    })
}

impl Runtime {
    pub(crate) fn readers_finished(&self) -> bool {
        self.readers
            .lock()
            .is_ok_and(|readers| readers.iter().all(thread::JoinHandle::is_finished))
    }
    pub(crate) fn state_dir(&self) -> &std::path::Path {
        &self.config.state_dir
    }
}

fn check_control(
    state: &mut Session,
    client: ClientId,
    generation: Counter,
    action: Action,
    remote: Option<&remote_effects::Execution<'_>>,
) -> Result<(), RemoteError> {
    if let Some(remote) = remote {
        remote.check_controller(state, generation)?;
    } else if state.remote_controller.is_some() {
        return Err(RemoteError::rejected(
            ErrorCode::StaleGeneration,
            "remote control requires original native authority",
        ));
    }
    expire(state);
    ensure_running(state)?;
    let controller = state
        .info
        .controller
        .as_ref()
        .is_some_and(|l| l.holder == client && l.generation == generation);
    authorize(
        action,
        Facts {
            controller,
            ..local_facts()
        },
    )
}
fn ensure_running(state: &Session) -> Result<(), RemoteError> {
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    if state.agent_control.is_some() {
        return Ok(());
    }
    ensure_pty(state)
}
fn ensure_pty(state: &Session) -> Result<(), RemoteError> {
    if state.control.is_none() {
        return Err(RemoteError::rejected(
            ErrorCode::UnsupportedCapability,
            "session has no running PTY",
        ));
    }
    Ok(())
}
fn expire(state: &mut Session) {
    if state.info.controller.is_none()
        || state
            .deadline
            .is_none_or(|deadline| deadline <= Instant::now())
        || state
            .deadline_elapsed
            .zip(clock::elapsed().ok())
            .is_none_or(|(deadline, now)| deadline <= now)
    {
        clear_control(state);
    }
}
fn clear_control(state: &mut Session) {
    state.remote_controller = None;
    state.info.controller = None;
    state.deadline = None;
    state.deadline_elapsed = None;
}
fn input_deadline(deadline: Instant, elapsed_deadline: Duration) -> Result<Instant, RemoteError> {
    let now = clock::elapsed()?;
    if now >= elapsed_deadline {
        return Err(RemoteError::rejected(
            ErrorCode::StaleGeneration,
            "continuous control deadline elapsed",
        ));
    }
    Ok(deadline.min(Instant::now() + (elapsed_deadline - now)))
}
fn renew(state: &mut Session, client: ClientId) {
    if state.remote_controller.is_some() {
        return;
    }
    expire(state);
    let Ok(elapsed) = clock::elapsed() else {
        clear_control(state);
        return;
    };
    if let Some(lease) = &mut state.info.controller
        && lease.holder == client
    {
        lease.expires_at_ms = Counter(now_ms() + 15_000);
        state.deadline = Some(Instant::now() + Duration::from_secs(15));
        state.deadline_elapsed = Some(elapsed + Duration::from_secs(15));
    }
}
fn local_facts() -> Facts {
    Facts {
        owner: true,
        access_fresh: true,
        revoked: false,
        same_os_user: true,
        controller: false,
        within_roots: true,
        capability: true,
        host_allows: true,
        external: false,
        termination_confirmed: false,
        external_termination_confirmed: false,
    }
}
fn apply_screen(parser: &mut Screen, event: &EventBody) -> Result<(), RemoteError> {
    match event {
        EventBody::PtyOutput(data) => {
            let bytes = STANDARD.decode(data).map_err(|_| {
                RemoteError::rejected(
                    ErrorCode::HistoryTruncated,
                    "persisted PTY bytes are invalid",
                )
            })?;
            if bytes.len() > MAX_PTY_BYTES {
                return Err(RemoteError::rejected(
                    ErrorCode::HistoryTruncated,
                    "persisted PTY bytes exceed their bound",
                ));
            }
            parser.process(&bytes);
        }
        EventBody::Resize { rows, cols } => {
            parser.resize(*rows, *cols)?;
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, RemoteError> {
    mutex
        .lock()
        .map_err(|_| RemoteError::unknown("local state is unavailable"))
}
fn storage_error(_: impl std::fmt::Display) -> RemoteError {
    RemoteError::unknown("local persistence failed; reconcile the original operation")
}

#[cfg(test)]
mod ownership_tests;
#[cfg(test)]
mod tests;
