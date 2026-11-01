//! An owned native process and drain outlive RPCs, clients and connectors.
//! SQLite replay has a separate bounded worker; it cannot block the native drain.
use super::*;
use qgenie_remote_adapters::{
    Codex, CodexConfig, CodexError, CodexHandle, NativeApproval, NativeEffectAuthorization,
    NativeEvent, Provider,
};
use qgenie_remote_proto::wire::{AgentEvent, InputReceipt, Launch, LaunchKind};
use std::sync::{atomic::AtomicUsize, mpsc as channel};
mod approvals;
mod effects;
mod owner;
mod replay;
use approvals::approval_source;

/// Trusted local configuration; credentials are injected only into the native child.
#[derive(Clone)]
pub struct CodexProfile {
    pub binary: PathBuf,
    pub provider: Provider,
}
impl CodexProfile {
    /// Only trusted host composition supplies provider credentials; they never
    /// appear in a remote launch, inventory, or start-key fingerprint.
    pub fn new(binary: PathBuf, endpoint: String, model: String, credential: String) -> Self {
        Self {
            binary,
            provider: Provider::new(endpoint, model, credential),
        }
    }
}

#[derive(Clone)]
pub(super) struct Control {
    commands: tokio::sync::mpsc::Sender<Command>,
    turn: Arc<Mutex<Option<String>>>,
}
enum ActionRequest {
    Terminate,
    Prompt {
        thread: String,
        text: String,
    },
    Steer {
        thread: String,
        turn: String,
        text: String,
    },
    Interrupt {
        thread: String,
        turn: String,
    },
    Answer {
        approval: NativeApproval,
        decision: wire::ApprovalDecision,
    },
}
struct Command {
    action: ActionRequest,
    authorization: Arc<dyn NativeEffectAuthorization>,
    reply: channel::SyncSender<Result<InputReceipt, RemoteError>>,
}
impl Control {
    pub fn terminate(
        &self,
        authorization: Arc<dyn NativeEffectAuthorization>,
    ) -> Result<(), RemoteError> {
        self.request(ActionRequest::Terminate, authorization)
            .map(|_| ())
    }
    fn request(
        &self,
        action: ActionRequest,
        authorization: Arc<dyn NativeEffectAuthorization>,
    ) -> Result<ApplicationOutcome, RemoteError> {
        self.request_receipt(action, authorization)
            .map(|receipt| receipt.outcome)
    }
    fn request_receipt(
        &self,
        action: ActionRequest,
        authorization: Arc<dyn NativeEffectAuthorization>,
    ) -> Result<InputReceipt, RemoteError> {
        let (reply, result) = channel::sync_channel(1);
        self.commands
            .try_send(Command {
                action,
                authorization,
                reply,
            })
            .map_err(|_| {
                RemoteError::rejected(
                    ErrorCode::Overloaded,
                    "native owner queue unavailable; no command sent",
                )
            })?;
        result
            .recv_timeout(Duration::from_secs(3))
            .unwrap_or_else(|_| {
                Err(RemoteError::unknown(
                    "native command reply lost; do not repeat the command",
                ))
            })
    }
}

struct Local;
impl NativeEffectAuthorization for Local {
    fn perform(
        &self,
        effect: &mut dyn FnMut() -> std::io::Result<usize>,
    ) -> std::io::Result<usize> {
        effect()
    }
}
pub(super) fn authorization(fence: Option<authority::Fence>) -> Arc<dyn NativeEffectAuthorization> {
    fence.map_or_else(
        || Arc::new(Local) as Arc<dyn NativeEffectAuthorization>,
        |fence| Arc::new(fence),
    )
}

struct LaunchOwned {
    resume_thread: Option<String>,
    native: CodexConfig,
    directory: std::fs::File,
    initial_prompt: Option<String>,
    authorization: Arc<dyn NativeEffectAuthorization>,
    recording: Option<capture::Capture>,
}
enum Persist {
    Event(AgentEvent),
    Approval(NativeApproval),
    Resolved(String),
    End(qgenie_remote_adapters::CodexExit),
}
#[derive(Default)]
struct DrainStatus {
    lost: AtomicUsize,
    disconnected: AtomicBool,
}
struct OwnerChannels {
    commands: tokio::sync::mpsc::Receiver<Command>,
    turn: Arc<Mutex<Option<String>>>,
    events: channel::SyncSender<Persist>,
    lost: Arc<DrainStatus>,
    ack: channel::SyncSender<Result<SessionInfo, RemoteError>>,
}
struct OwnerReservation(std::sync::Weak<Mutex<Session>>);
impl Drop for OwnerReservation {
    fn drop(&mut self) {
        if let Some(session) = self.0.upgrade()
            && let Ok(mut state) = session.lock()
        {
            state.agent_control = None;
            if matches!(state.info.state, SessionState::Running | SessionState::Idle) {
                state.info.state = SessionState::OutcomeUnknown;
            }
        }
    }
}

impl Runtime {
    #[cfg(test)]
    fn managed_fault(&self, point: StartFault) {
        if self.config.start_fault != Some(point) {
            return;
        }
        if let Some(root) = std::env::var_os("QGR_MANAGED_FAULT_BARRIER") {
            let root = PathBuf::from(root);
            std::fs::write(root.join("fault.pending"), b"ready").unwrap();
            std::fs::rename(root.join("fault.pending"), root.join("fault.ready")).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !root.join("fault.release").exists() {
                assert!(
                    Instant::now() < deadline,
                    "managed fault controller unavailable"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
        self.fault(point);
    }
    /// Same-user local entry. Network dispatch must supply its original host fence.
    pub fn start_managed(self: &Arc<Self>, launch: Launch) -> Result<SessionInfo, RemoteError> {
        self.start_agent(launch, None, None)
    }
    /// This entry shares the PTY start lock, scoped key namespace and durable store.
    /// It is private IPC composition, never authority to configure a provider remotely.
    pub(super) fn start_agent(
        self: &Arc<Self>,
        launch: Launch,
        scope: Option<RemoteScope>,
        fence: Option<authority::Fence>,
    ) -> Result<SessionInfo, RemoteError> {
        let _start = lock(&self.starts)?;
        if let Some(fence) = &fence {
            fence.check()?;
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err(RemoteError::rejected(
                ErrorCode::Forbidden,
                "session daemon is stopping",
            ));
        }
        if let Some(scope) = &scope {
            scope.validate()?;
            if !lock(&self.store)?.bound(scope).map_err(storage_error)? {
                return Err(RemoteError::rejected(
                    ErrorCode::Forbidden,
                    "native scope is not registered",
                ));
            }
        }
        let mut parameters = serde_json::to_value(&launch).map_err(storage_error)?;
        parameters.as_object_mut().unwrap().remove("key");
        let digest = fingerprint(&parameters)?;
        if let Some(session) = self.lookup_start(scope.as_ref(), launch.key)? {
            let state = lock(&session)?;
            if state.fingerprint != digest {
                return Err(RemoteError::rejected(
                    ErrorCode::IdempotencyConflict,
                    "start key has different parameters",
                ));
            }
            return Ok(state.info.clone());
        }
        self.reap_sessions()?;
        let sessions = lock(&self.sessions)?.values().cloned().collect::<Vec<_>>();
        let LaunchKind::Agent {
            agent,
            prompt,
            native_resume_id,
        } = launch.kind
        else {
            return Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "expected an agent launch",
            ));
        };
        if agent != "codex" {
            return Err(RemoteError::rejected(
                ErrorCode::UnsupportedCapability,
                "this host only supports managed Codex",
            ));
        }
        let native_home = if let Some(thread) = &native_resume_id {
            let homes = lock(&self.store)?
                .history_homes(scope.as_ref(), &agent, thread)
                .map_err(storage_error)?;
            if homes.len() != 1 {
                return Err(RemoteError::rejected(
                    ErrorCode::NotFound,
                    "no confirmed ended native history in this owner scope",
                ));
            }
            let home = *homes.first().unwrap();
            let root = self.session(home)?;
            let state = lock(&root)?;
            let canonical = state.scope == scope
                && matches!(state.info.state, SessionState::Exited)
                && state.info.ended_at_ms.is_some()
                && state.info.agent.as_ref().is_some_and(|identity| {
                    identity.agent == agent
                        && identity.native_home.is_none()
                        && identity.native_thread.as_ref() == Some(thread)
                });
            drop(state);
            if !canonical {
                return Err(RemoteError::rejected(
                    ErrorCode::Forbidden,
                    "native history root differs from this owner and thread",
                ));
            }
            if lock(&self.store)?
                .history_uncertain(home)
                .map_err(storage_error)?
                || sessions.iter().any(|session| {
                    session.lock().is_ok_and(|state| {
                        owns_process(&state)
                            && state.info.agent.as_ref().is_some_and(|identity| {
                                identity.native_home.unwrap_or(state.info.id) == home
                            })
                    })
                })
            {
                return Err(RemoteError::rejected(
                    ErrorCode::Forbidden,
                    "native history has an active or unconfirmed owner",
                ));
            }
            Some(home)
        } else {
            None
        };
        validate_size(launch.rows, launch.cols)?;
        if prompt
            .as_ref()
            .is_some_and(|text| text.is_empty() || text.len() > 32768 || text.contains('\0'))
        {
            return Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "invalid initial prompt",
            ));
        }
        let profile = self.config.codex.clone().ok_or_else(|| {
            RemoteError::rejected(
                ErrorCode::UnsupportedCapability,
                "managed Codex is not configured on this host",
            )
        })?;
        if sessions
            .iter()
            .filter(|session| session.lock().is_ok_and(|state| owns_process(&state)))
            .count()
            >= self.config.max_sessions
        {
            return Err(RemoteError::rejected(
                ErrorCode::CapacityExhausted,
                "session registry capacity reached",
            ));
        }
        let directory = self
            .directories
            .resolve(std::path::Path::new(&launch.cwd))
            .map_err(|_| {
                RemoteError::rejected(ErrorCode::Forbidden, "working directory is unavailable")
            })?;
        let info = SessionInfo {
            id: SessionId::new(),
            machine: scope.as_ref().map_or(self.machine, |scope| scope.machine),
            command: vec!["codex".into()],
            cwd: directory.path.display().to_string(),
            state: SessionState::Accepted,
            created_at_ms: Counter(now_ms()),
            ended_at_ms: None,
            head: Counter(0),
            controller: None,
            exit_code: None,
            recording_warning: scope
                .is_none()
                .then(|| "Local preview replay only; no enrolled recording".into()),
            agent: Some(ManagedSessionInfo {
                requested_model: profile.provider.requested_model().map(str::to_owned),
                native_home,
                agent,
                native_instance: MessageId::new(),
                native_thread: None,
            }),
        };
        // The compatibility launch spec contains neither prompt nor provider credentials.
        let spec = StartSpec {
            key: launch.key,
            command: info.command.clone(),
            cwd: info.cwd.clone(),
            rows: launch.rows,
            cols: launch.cols,
        };
        let state = Arc::new(Mutex::new(Session {
            agent_state: Some(AgentState {
                history_truncated: native_resume_id.is_some(),
                ..AgentState::default()
            }),
            agent_control: None,
            recording: None,
            remote_controller: None,
            info: info.clone(),
            spec: spec.clone(),
            fingerprint: digest.clone(),
            generation: 0,
            input_seq: 0,
            deadline: None,
            deadline_elapsed: None,
            control: None,
            screen: None,
            pending_lost_bytes: 0,
            scope: scope.clone(),
        }));
        let (commands, command_rx) = tokio::sync::mpsc::channel(4);
        let turn = Arc::new(Mutex::new(None));
        lock(&state)?.agent_control = Some(Control {
            commands,
            turn: turn.clone(),
        });
        let (events, receiver) = channel::sync_channel(64);
        let lost = Arc::new(DrainStatus::default());
        let (handoff, owned_launch) = channel::sync_channel::<LaunchOwned>(1);
        let (ready, reservation) = channel::sync_channel(1);
        let (ack, result) = channel::sync_channel(1);
        let native_runtime = self.clone();
        let native_state = state.clone();
        let native_lost = lost.clone();
        let owner = thread::Builder::new()
            .name(format!("codex-{}", info.id))
            .spawn(move || {
                let _reservation = OwnerReservation(Arc::downgrade(&native_state));
                let executor = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let _ = ready.send(executor.is_ok());
                if let Ok(executor) = executor
                    && let Ok(launch) = owned_launch.recv()
                {
                    executor.block_on(native_runtime.own_agent(
                        native_state,
                        launch,
                        OwnerChannels {
                            commands: command_rx,
                            turn,
                            events,
                            lost: native_lost,
                            ack,
                        },
                    ));
                }
            })
            .map_err(|_| {
                RemoteError::rejected(
                    ErrorCode::CapacityExhausted,
                    "native supervisor capacity unavailable",
                )
            })?;
        lock(&self.readers)?.push(owner);
        if !reservation.recv().unwrap_or(false) {
            return Err(RemoteError::rejected(
                ErrorCode::CapacityExhausted,
                "native executor unavailable",
            ));
        }
        let persistence_runtime = self.clone();
        let persistence_state = state.clone();
        let persistence = thread::Builder::new()
            .name(format!("agent-replay-{}", info.id))
            .spawn(move || persistence_runtime.persist_agent(persistence_state, receiver, lost))
            .map_err(|_| {
                RemoteError::rejected(ErrorCode::CapacityExhausted, "replay worker unavailable")
            })?;
        lock(&self.readers)?.push(persistence);
        if let Some(fence) = &fence {
            fence.check()?;
        }
        #[cfg(test)]
        self.managed_fault(StartFault::BeforeJournal);
        lock(&self.store)?
            .insert(&info, &spec, &digest, scope.as_ref())
            .map_err(storage_error)?;
        #[cfg(test)]
        self.managed_fault(StartFault::Accepted);
        lock(&self.sessions)?.insert(info.id, state.clone());
        self.publish_change(scope.as_ref());
        if let Some(scope) = &scope {
            let source = crate::ApprovalSource {
                scope: scope.clone(),
                session: info.id,
                instance: info.agent.as_ref().unwrap().native_instance,
            };
            if self.approvals.register_source(&source).is_err() {
                return self.fail_agent_start(
                    &state,
                    "Approval journal could not initialize; no native child was started",
                );
            }
        }
        let recording = if let Some(scope) = &scope {
            match self.begin_capture(&info, scope, launch.rows, launch.cols) {
                Ok(capture) => Some(capture),
                Err(_) => {
                    return self.fail_agent_start(
                        &state,
                        "Recording could not initialize; no native child was started",
                    );
                }
            }
        } else {
            None
        };
        let mut recording_guard =
            capture::StartGuard::new(recording.as_ref().map(|capture| capture.source.clone()));
        lock(&state)?.recording = recording.clone();
        if fence.as_ref().is_some_and(|fence| fence.check().is_err()) {
            return self.fail_agent_start(
                &state,
                "Original launch authority expired; no native child was started",
            );
        }
        lock(&self.store)?
            .state(info.id, &SessionState::Launching, None)
            .map_err(storage_error)?;
        lock(&state)?.info.state = SessionState::Launching;
        #[cfg(test)]
        self.managed_fault(StartFault::Launching);
        let native = CodexConfig {
            kill_on_owner_thread_exit: true,
            binary: profile.binary,
            home: self
                .config
                .state_dir
                .join("agents")
                .join(native_home.unwrap_or(info.id).to_string()),
            cwd: directory.path,
            provider: profile.provider,
            environment: LaunchEnvironment::default(),
        };
        handoff
            .send(LaunchOwned {
                resume_thread: native_resume_id,
                native,
                directory: directory.file,
                initial_prompt: prompt,
                authorization: authorization(fence),
                recording,
            })
            .map_err(|_| {
                RemoteError::unknown("accepted agent lost its owner; reconcile the original key")
            })?;
        recording_guard.handed_off();
        result
            .recv_timeout(Duration::from_secs(3))
            .unwrap_or_else(|_| {
                Err(RemoteError::unknown(
                    "native launch reply unavailable; reconcile the original key",
                ))
            })
    }

    fn fail_agent_start(
        &self,
        state: &Mutex<Session>,
        message: &str,
    ) -> Result<SessionInfo, RemoteError> {
        let mut state = lock(state)?;
        state.agent_control = None;
        state.info.state = SessionState::Failed;
        state.info.recording_warning = Some(message.into());
        lock(&self.store)?
            .state(state.info.id, &state.info.state, None)
            .map_err(storage_error)?;
        Ok(state.info.clone())
    }
}

fn capture_text(capture: &mut capture::Capture, channel: RecordingChannel, bytes: &[u8]) {
    for part in bytes.chunks(32768) {
        capture.push(crate::CaptureContent::AgentText {
            channel,
            bytes: part.to_vec(),
        });
    }
}
fn event_channel(event: &AgentEvent) -> RecordingChannel {
    match event {
        AgentEvent::MessageDelta { .. } | AgentEvent::MessageCompleted { .. } => {
            RecordingChannel::MessageDelta
        }
        AgentEvent::ToolOutput { .. } => RecordingChannel::ToolOutput,
        AgentEvent::ToolStarted { .. } | AgentEvent::ToolCompleted { .. } => {
            RecordingChannel::ToolStatus
        }
        AgentEvent::TurnCompleted { .. } => RecordingChannel::Usage,
        AgentEvent::Error { .. } | AgentEvent::Gap { .. } => RecordingChannel::Warning,
        _ => RecordingChannel::Native,
    }
}
fn native_error(error: CodexError) -> RemoteError {
    match error {
        CodexError::NotSent => {
            RemoteError::rejected(ErrorCode::AccessStale, "native effect was not sent")
        }
        _ => RemoteError::unknown("native effect outcome is unknown; do not resend"),
    }
}

#[cfg(test)]
mod crash_tests;
#[cfg(test)]
mod tests;
