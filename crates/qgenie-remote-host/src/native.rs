//! Same-user IPC bridge. This owns no native processes and never maps a public
//! operation to administrative IPC. Sealed access retains its original nonce.
use crate::Error;
use qgenie_remote_proto::{
    ChallengeId, ClientId, Counter, ErrorCode, IPC_MAJOR, RemoteError, SessionId, UserId,
    ipc::{
        AdminCommand, AdminRequest, AdminResponse, ControlActivation, DispatchFrame,
        HostActivation, RemoteContext, RemoteScope,
    },
    wire::{Envelope, Message},
};
use qgenie_remote_transport::ipc::{
    client_handshake_with_options, connect, read_dispatch, read_frame, write_dispatch, write_frame,
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
mod replay;
pub use replay::NativeAttachment;
mod changes;
mod inventory;
pub use changes::NativeChanges;

#[derive(Clone)]
pub struct NativeClient {
    directory: PathBuf,
    scope: RemoteScope,
    work: Arc<Semaphore>,
    change_work: Arc<Semaphore>,
    recording_work: Arc<Semaphore>,
    replay_work: Arc<Semaphore>,
    replay_bytes: Arc<Semaphore>,
}
#[derive(Debug, Clone)]
pub struct NativeControlState {
    pub floor: Counter,
    pub holder: Option<ClientId>,
    pub remaining_ms: u32,
    pub last_input: Counter,
}
#[derive(Debug, Clone)]
pub struct NativeControlProposal {
    pub generation: Counter,
    pub remaining_lease_ms: u32,
    pub takeover: bool,
    pub renewal: bool,
}
#[derive(Debug, Clone)]
pub struct NativeControllerLease {
    pub holder: ClientId,
    pub generation: Counter,
    /// Measured before the native ACK. Callers must subtract their entire
    /// round-trip age before deriving an authority deadline from this value.
    pub remaining_ms: u32,
    pub last_input: Counter,
}
/// Native-issued challenge plus the connector's earlier, conservative clock.
/// No public constructor can replace a delayed request's original issue time.
#[derive(Clone)]
pub struct NativeChallenge {
    nonce: ChallengeId,
    policy_revision: Counter,
    max_remaining_ms: u32,
    scope: RemoteScope,
    connection: ClientId,
    generation: Counter,
    issued: Instant,
    issued_elapsed: Duration,
}
/// One acknowledged native access identity. Renewal creates a separate value;
/// an old queued dispatch keeps this value and its original nonce/deadline.
#[derive(Clone)]
pub struct NativeAccess {
    challenge: NativeChallenge,
    remaining_ms: u32,
}
impl NativeAccess {
    /// Narrow a queued dispatch's budget without changing its nonce or issue
    /// time. A later renewal cannot extend this value.
    pub fn restrict(&self, remaining_ms: u32) -> Result<Self, Error> {
        if remaining_ms > self.remaining_ms {
            return Err(Error::Denied);
        }
        self.challenge.check(remaining_ms)?;
        Ok(Self {
            challenge: self.challenge.clone(),
            remaining_ms,
        })
    }
    pub fn nonce(&self) -> ChallengeId {
        self.challenge.nonce
    }
    pub fn generation(&self) -> Counter {
        self.challenge.generation
    }
    pub fn connection(&self) -> ClientId {
        self.challenge.connection
    }
    /// Original delegated budget; never a new lifetime from this call's time.
    pub fn remaining_ms(&self) -> u32 {
        self.remaining_ms
    }
    pub fn policy_revision(&self) -> Counter {
        self.challenge.policy_revision
    }
    pub fn is_fresh(&self) -> bool {
        self.challenge.check(self.remaining_ms).is_ok()
    }
    pub fn remaining(&self) -> Result<Duration, Error> {
        self.challenge.check(self.remaining_ms)?;
        let used = elapsed()?
            .checked_sub(self.challenge.issued_elapsed)
            .ok_or_else(stale)?
            .max(self.challenge.issued.elapsed());
        Duration::from_millis(u64::from(self.remaining_ms))
            .checked_sub(used)
            .filter(|left| !left.is_zero())
            .ok_or_else(stale)
    }
}
impl NativeChallenge {
    pub fn nonce(&self) -> ChallengeId {
        self.nonce
    }
    pub fn policy_revision(&self) -> Counter {
        self.policy_revision
    }
    pub fn max_remaining_ms(&self) -> u32 {
        self.max_remaining_ms
    }
    fn check(&self, remaining_ms: u32) -> Result<(), Error> {
        let elapsed = elapsed()?
            .checked_sub(self.issued_elapsed)
            .ok_or_else(stale)?;
        let budget = Duration::from_millis(u64::from(remaining_ms));
        if remaining_ms == 0
            || remaining_ms > self.max_remaining_ms
            || remaining_ms > qgenie_remote_proto::MAX_ACCESS_LIFETIME_MS
            || self.issued.elapsed() >= budget
            || elapsed >= budget
        {
            return Err(stale());
        }
        Ok(())
    }
}
impl NativeClient {
    pub fn new(directory: PathBuf, scope: RemoteScope) -> Result<Self, Error> {
        scope.validate().map_err(Error::Rejected)?;
        if !directory.is_absolute() {
            return Err(Error::Configuration);
        }
        Ok(Self {
            directory,
            scope,
            work: Arc::new(Semaphore::new(4)),
            change_work: Arc::new(Semaphore::new(1)),
            recording_work: Arc::new(Semaphore::new(1)),
            replay_work: Arc::new(Semaphore::new(2)),
            replay_bytes: Arc::new(Semaphore::new(16 * 1024 * 1024)),
        })
    }
    pub fn scope(&self) -> &RemoteScope {
        &self.scope
    }
    /// Same-user capability discovery has no operation frame or native effect.
    pub async fn capabilities(&self) -> Result<Vec<String>, Error> {
        let _permit = self.permit()?;
        tokio::time::timeout(Duration::from_secs(3), async {
            let mut stream = connect(&self.directory.join("dispatch.sock"))
                .await
                .map_err(|_| Error::Unavailable)?;
            client_handshake_with_options(
                &mut stream,
                &["native-authority-v1"],
                &[
                    "native-agent-v1",
                    "native-prompt-receipts-v1",
                    "native-approvals-v1",
                    "native-changes-v1",
                ],
            )
            .await
            .map_err(|_| Error::Unavailable)
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }
    pub async fn agent_inventory(
        &self,
        access: &NativeAccess,
    ) -> Result<std::collections::BTreeMap<String, qgenie_remote_proto::wire::AgentInventory>, Error>
    {
        let context = self.context(access, access.connection(), None)?;
        match self.rpc(DispatchFrame::AgentInventory { context }).await? {
            DispatchFrame::AgentInventoryReply { agents } if agents.len() <= 64 => {
                access.challenge.check(access.remaining_ms)?;
                let envelope = Envelope::new(Message::Inventory {
                    agents: agents.clone(),
                });
                envelope.validate().map_err(Error::Rejected)?;
                Ok(agents)
            }
            _ => Err(Error::Protocol),
        }
    }
    /// Trusted outbound bridge only. This retains the original nonce through native
    /// journaling/launch and never maps the request to administrative/local IPC.
    pub async fn start(
        &self,
        access: &NativeAccess,
        client: ClientId,
        launch: qgenie_remote_proto::wire::Launch,
    ) -> Result<qgenie_remote_proto::wire::Session, Error> {
        launch.validate().map_err(Error::Rejected)?;
        let context = self.context(access, client, None)?;
        let mut message = Envelope::new(Message::Start(launch));
        message.machine = Some(self.scope.machine);
        let id = message.id;
        let response = self
            .rpc(DispatchFrame::Remote {
                context,
                message: Box::new(message),
            })
            .await?;
        access.challenge.check(access.remaining_ms).map_err(|_| {
            Error::Rejected(RemoteError::unknown(
                "native start reply exceeded its original authority; reconcile the persisted key",
            ))
        })?;
        match response {
            DispatchFrame::RemoteReply(envelope)
                if envelope.re == Some(id)
                    && envelope.machine == Some(self.scope.machine)
                    && envelope.session.is_none() =>
            {
                match envelope.message {
                    Message::Started(session)
                        if session.machine == self.scope.machine
                            && session.owner == self.scope.owner =>
                    {
                        Ok(session)
                    }
                    _ => Err(Error::Protocol),
                }
            }
            _ => Err(Error::Protocol),
        }
    }
    /// Effects use original sealed access plus the exact activated controller.
    pub async fn effect(
        &self,
        access: &NativeAccess,
        client: ClientId,
        request: Envelope,
    ) -> Result<Envelope, Error> {
        request.validate().map_err(Error::Rejected)?;
        if request.machine != Some(self.scope.machine)
            || request.session.is_none()
            || request.re.is_some()
        {
            return Err(Error::Denied);
        }
        let controller = match &request.message {
            Message::Input { authority, .. }
            | Message::Resize { authority, .. }
            | Message::Interrupt { authority }
                if authority.host == access.generation() =>
            {
                authority.controller
            }
            Message::Terminate {
                host_generation, ..
            } if *host_generation == access.generation() => None,
            Message::ApprovalCommitted(_) => None,
            _ => return Err(Error::Denied),
        };
        let context = self.context(access, client, controller)?;
        let id = request.id;
        let session = request.session;
        let response = self
            .rpc(DispatchFrame::Remote {
                context,
                message: Box::new(request),
            })
            .await?;
        access.challenge.check(access.remaining_ms).map_err(|_| {
            Error::Rejected(RemoteError::unknown(
                "native effect reply exceeded its original authority; do not resend input",
            ))
        })?;
        match response {
            DispatchFrame::RemoteReply(reply)
                if reply.re == Some(id)
                    && reply.machine == Some(self.scope.machine)
                    && reply.session == session =>
            {
                Ok(*reply)
            }
            _ => Err(Error::Protocol),
        }
    }
    /// Reconcile a restored primary before it allocates a new host generation.
    /// This reads the durable native floor; it never resets or reactivates it.
    pub async fn host_floor(&self) -> Result<Counter, Error> {
        match self
            .rpc(DispatchFrame::QueryHostFloor {
                scope: self.scope.clone(),
            })
            .await?
        {
            DispatchFrame::HostFloor { generation } => Ok(generation),
            _ => Err(Error::Protocol),
        }
    }
    /// Read-only spool access. Use the authenticated uploader for epoch-bound reclamation.
    pub async fn recording_fetch(
        &self,
        access: &NativeAccess,
    ) -> Result<Option<qgenie_remote_proto::ipc::RecordingUpload>, Error> {
        let context = self.context(access, access.connection(), None)?;
        let response = self
            .rpc(DispatchFrame::RecordingFetch {
                context,
                max_bytes: 64 * 1024,
            })
            .await?;
        access.challenge.check(access.remaining_ms)?;
        let DispatchFrame::RecordingFetched(item) = response else {
            return Err(Error::Protocol);
        };
        if let Some(item) = &item {
            match item {
                qgenie_remote_proto::ipc::RecordingUpload::Content(batch) => {
                    batch.validate().map_err(|_| Error::Protocol)?;
                    if batch.machine != self.scope.machine {
                        return Err(Error::Protocol);
                    }
                }
                qgenie_remote_proto::ipc::RecordingUpload::Gap(gap) => {
                    gap.validate().map_err(|_| Error::Protocol)?
                }
            }
        }
        Ok(item)
    }
    pub async fn recording_status(
        &self,
        access: &NativeAccess,
    ) -> Result<qgenie_remote_proto::ipc::RecordingStatus, Error> {
        let context = self.context(access, access.connection(), None)?;
        let response = self.rpc(DispatchFrame::RecordingStatus { context }).await?;
        access.challenge.check(access.remaining_ms)?;
        match response {
            DispatchFrame::RecordingStatusReply(status) => Ok(status),
            _ => Err(Error::Protocol),
        }
    }
    /// Call only after trusted local enrollment accepts its immutable identity.
    /// Acknowledgment proves the binding is durable, not that a host is active.
    pub async fn register_binding(&self) -> Result<(), Error> {
        let _permit = self.permit()?;
        tokio::time::timeout(Duration::from_secs(3), async {
            let mut stream = connect(&self.directory.join("admin.sock"))
                .await
                .map_err(|_| Error::Unavailable)?;
            let request = AdminRequest {
                ipc_major: IPC_MAJOR,
                command: AdminCommand::RegisterRemoteBinding {
                    scope: self.scope.clone(),
                },
            };
            write_frame(
                &mut stream,
                &serde_json::to_vec(&request).map_err(|_| Error::Protocol)?,
            )
            .await
            .map_err(|_| Error::Unavailable)?;
            let bytes = read_frame(&mut stream)
                .await
                .map_err(|_| Error::Unavailable)?;
            let response: AdminResponse =
                qgenie_remote_proto::decode_json(&bytes).map_err(|_| Error::Protocol)?;
            response.result.map_err(Error::Rejected)
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }
    pub async fn challenge(
        &self,
        connection: ClientId,
        generation: Counter,
    ) -> Result<NativeChallenge, Error> {
        if generation.0 == 0 {
            return Err(Error::Protocol);
        }
        let issued = Instant::now();
        let issued_elapsed = elapsed()?;
        match self
            .rpc(DispatchFrame::AccessChallenge {
                scope: self.scope.clone(),
                connection,
                host_generation: generation,
            })
            .await?
        {
            DispatchFrame::AccessChallengeIssued {
                nonce,
                policy_revision,
                max_remaining_ms,
            } if (1..=qgenie_remote_proto::MAX_ACCESS_LIFETIME_MS).contains(&max_remaining_ms)
                && policy_revision.0 != 0 =>
            {
                let challenge = NativeChallenge {
                    nonce,
                    policy_revision,
                    max_remaining_ms,
                    scope: self.scope.clone(),
                    connection,
                    generation,
                    issued,
                    issued_elapsed,
                };
                challenge.check(max_remaining_ms)?;
                Ok(challenge)
            }
            _ => Err(Error::Protocol),
        }
    }
    /// A timeout can still leave the native generation floor committed. Never
    /// infer rollback or reset a generation from a missing acknowledgment.
    pub async fn activate(
        &self,
        challenge: &NativeChallenge,
        remaining_ms: u32,
        renewal: bool,
    ) -> Result<NativeAccess, Error> {
        if challenge.scope != self.scope {
            return Err(Error::Denied);
        }
        challenge.check(remaining_ms)?;
        let activation = HostActivation {
            scope: self.scope.clone(),
            connection: challenge.connection,
            generation: challenge.generation,
            nonce: challenge.nonce,
            remaining_ms,
            policy_revision: challenge.policy_revision,
        };
        let frame = if renewal {
            DispatchFrame::AccessRenewal(activation)
        } else {
            DispatchFrame::ActivateHost(activation)
        };
        match self.rpc(frame).await? {
            DispatchFrame::HostActivated { nonce, generation }
                if nonce == challenge.nonce && generation == challenge.generation =>
            {
                challenge.check(remaining_ms)?;
                Ok(NativeAccess {
                    challenge: challenge.clone(),
                    remaining_ms,
                })
            }
            _ => Err(Error::Protocol),
        }
    }
    pub async fn read(
        &self,
        access: &NativeAccess,
        actor: UserId,
        client: ClientId,
        message: Envelope,
    ) -> Result<Envelope, Error> {
        if access.challenge.scope != self.scope
            || actor != self.scope.owner
            || message.machine != Some(self.scope.machine)
        {
            return Err(Error::Denied);
        }
        if !matches!(
            message.message,
            Message::SessionList
                | Message::SessionListPage { .. }
                | Message::SessionGet { .. }
                | Message::StartLookup { .. }
                | Message::ApprovalList
        ) {
            return Err(Error::Rejected(RemoteError::rejected(
                ErrorCode::UnsupportedCapability,
                "unsupported native read",
            )));
        }
        access.challenge.check(access.remaining_ms)?;
        let request = message.id;
        let expected_session = message.session;
        let approval_list = matches!(message.message, Message::ApprovalList);
        let lookup = match &message.message {
            Message::StartLookup { key } => Some(*key),
            _ => None,
        };
        let context = RemoteContext {
            actor,
            client,
            scope: self.scope.clone(),
            connection: access.connection(),
            host_generation: access.generation(),
            controller_generation: None,
            access_nonce: access.nonce(),
            remaining_access_ms: access.remaining_ms(),
            policy_revision: access.policy_revision(),
        };
        match self
            .rpc(DispatchFrame::Remote {
                context,
                message: Box::new(message),
            })
            .await?
        {
            DispatchFrame::RemoteReply(response)
                if response.re == Some(request)
                    && response.machine == Some(self.scope.machine)
                    && response.session == expected_session =>
            {
                response.validate().map_err(Error::Rejected)?;
                let valid = match (&response.message, lookup) {
                    (
                        Message::Sessions { .. }
                        | Message::SessionPage { .. }
                        | Message::SessionFound(_),
                        None,
                    ) if !approval_list => true,
                    (Message::Approvals { .. }, None) if approval_list => true,
                    (Message::Error(_), _) => true,
                    (Message::StartStatus { key, .. }, Some(expected)) => *key == expected,
                    _ => false,
                };
                if !valid {
                    return Err(Error::Protocol);
                }
                access.challenge.check(access.remaining_ms)?;
                Ok(*response)
            }
            _ => Err(Error::Protocol),
        }
    }
    pub async fn controller_state(
        &self,
        access: &NativeAccess,
        client: ClientId,
        session: SessionId,
    ) -> Result<NativeControlState, Error> {
        let context = self.context(access, client, None)?;
        match self
            .rpc_with(DispatchFrame::QueryControl { context, session }, true)
            .await?
        {
            DispatchFrame::ControlState {
                session: actual,
                floor,
                holder,
                remaining_ms,
                last_input,
            } if actual == session && remaining_ms <= 15_000 => {
                if holder.is_some() != (remaining_ms > 0)
                    || (holder.is_some() && floor.0 == 0)
                    || (holder.is_none() && last_input.0 != 0)
                {
                    return Err(Error::Protocol);
                }
                access.challenge.check(access.remaining_ms)?;
                Ok(NativeControlState {
                    floor,
                    holder,
                    remaining_ms,
                    last_input,
                })
            }
            _ => Err(Error::Protocol),
        }
    }
    pub async fn activate_controller(
        &self,
        access: &NativeAccess,
        client: ClientId,
        session: SessionId,
        proposal: NativeControlProposal,
    ) -> Result<NativeControllerLease, Error> {
        let context = self.context(access, client, Some(proposal.generation))?;
        let request = ControlActivation {
            context,
            session,
            holder: client,
            generation: proposal.generation,
            remaining_lease_ms: proposal.remaining_lease_ms,
            takeover: proposal.takeover,
            renewal: proposal.renewal,
        };
        match self
            .rpc_with(DispatchFrame::ActivateControl(request), true)
            .await?
        {
            DispatchFrame::ControlActivated {
                session: actual,
                holder,
                generation,
                remaining_ms,
                last_input,
            } if actual == session
                && holder == client
                && generation == proposal.generation
                && (1..=15_000).contains(&remaining_ms) =>
            {
                access.challenge.check(access.remaining_ms)?;
                Ok(NativeControllerLease {
                    holder,
                    generation,
                    remaining_ms,
                    last_input,
                })
            }
            _ => Err(Error::Protocol),
        }
    }
    pub async fn release_controller(
        &self,
        access: &NativeAccess,
        client: ClientId,
        session: SessionId,
        generation: Counter,
    ) -> Result<(), Error> {
        let context = self.context(access, client, Some(generation))?;
        match self
            .rpc_with(
                DispatchFrame::DeactivateControl {
                    context,
                    session,
                    generation,
                },
                true,
            )
            .await?
        {
            DispatchFrame::ControlDeactivated {
                session: actual,
                generation: actual_generation,
            } if actual == session && actual_generation == generation => Ok(()),
            _ => Err(Error::Protocol),
        }
    }
    pub(crate) fn context(
        &self,
        access: &NativeAccess,
        client: ClientId,
        generation: Option<Counter>,
    ) -> Result<RemoteContext, Error> {
        if access.challenge.scope != self.scope {
            return Err(Error::Denied);
        }
        access.challenge.check(access.remaining_ms)?;
        Ok(RemoteContext {
            actor: self.scope.owner,
            client,
            scope: self.scope.clone(),
            connection: access.connection(),
            host_generation: access.generation(),
            controller_generation: generation,
            access_nonce: access.nonce(),
            remaining_access_ms: access.remaining_ms(),
            policy_revision: access.policy_revision(),
        })
    }
    fn permit(&self) -> Result<tokio::sync::OwnedSemaphorePermit, Error> {
        self.work.clone().try_acquire_owned().map_err(|_| {
            Error::Rejected(RemoteError::rejected(
                ErrorCode::Overloaded,
                "native IPC work capacity exceeded",
            ))
        })
    }
    pub(crate) async fn rpc(&self, request: DispatchFrame) -> Result<DispatchFrame, Error> {
        self.rpc_with(request, false).await
    }
    async fn rpc_with(
        &self,
        request: DispatchFrame,
        controller: bool,
    ) -> Result<DispatchFrame, Error> {
        let start = matches!(&request,DispatchFrame::Remote{message,..} if matches!(message.message,Message::Start(_)));
        let agent = matches!(&request,DispatchFrame::Remote{message,..} if matches!(message.message,Message::Start(qgenie_remote_proto::wire::Launch{kind:qgenie_remote_proto::wire::LaunchKind::Agent{..},..})|Message::Input{input:qgenie_remote_proto::wire::SessionInput::Prompt(_)|qgenie_remote_proto::wire::SessionInput::PromptReceipt(_)|qgenie_remote_proto::wire::SessionInput::Steer(_),..}));
        let agent = agent || matches!(request, DispatchFrame::AgentInventory { .. });
        let approval = matches!(&request,DispatchFrame::Remote{message,..} if matches!(message.message,Message::ApprovalList|Message::ApprovalCommitted(_)));
        let effect = matches!(&request,DispatchFrame::Remote{message,..} if matches!(message.message,Message::Input{..}|Message::Resize{..}|Message::Interrupt{..}|Message::Terminate{..}));
        let replay = matches!(
            &request,
            DispatchFrame::ReadSnapshot { .. } | DispatchFrame::ReadHead { .. }
        ) || matches!(&request,DispatchFrame::Remote{message,..} if matches!(message.message,Message::Attach{..}));
        let reclaim = matches!(
            &request,
            DispatchFrame::RecordingEpochChallenge { .. }
                | DispatchFrame::BindRecordingEpoch { .. }
                | DispatchFrame::RecordingAcknowledge { .. }
        );
        let recording = reclaim
            || matches!(
                &request,
                DispatchFrame::RecordingFetch { .. } | DispatchFrame::RecordingStatus { .. }
            );
        let _recording_permit = if recording {
            Some(
                self.recording_work
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| {
                        Error::Rejected(RemoteError::rejected(
                            ErrorCode::Overloaded,
                            "native recording read capacity exceeded",
                        ))
                    })?,
            )
        } else {
            None
        };
        let _permit = self.permit()?;
        tokio::time::timeout(Duration::from_secs(3), async {
            let mut stream = connect(&self.directory.join("dispatch.sock"))
                .await
                .map_err(|_| Error::Unavailable)?;
            // No operation is sent until the independent native capability ACK.
            let required: &[&str] = if start {
                &["native-authority-v1", "native-start-v1"]
            } else if replay {
                &["native-authority-v1", "native-replay-v1"]
            } else if effect {
                &[
                    "native-authority-v1",
                    "native-controller-v1",
                    "native-effects-v1",
                ]
            } else if matches!(request, DispatchFrame::QueryHostFloor { .. }) {
                &["native-authority-v1", "native-floor-v1"]
            } else if reclaim {
                &[
                    "native-authority-v1",
                    "recording-read-v1",
                    "recording-reclaim-v1",
                ]
            } else if recording {
                &["native-authority-v1", "recording-read-v1"]
            } else if controller {
                &["native-authority-v1", "native-controller-v1"]
            } else {
                &["native-authority-v1"]
            };
            let mut required = required.to_vec();
            if matches!(&request,DispatchFrame::Remote{message,..} if matches!(message.message,Message::SessionListPage{..}|Message::SessionGet{..})) {required.push("native-pages-v1");}
            if matches!(&request, DispatchFrame::Remote {message,..} if matches!(message.message, Message::Input { input: qgenie_remote_proto::wire::SessionInput::PromptReceipt(_), .. })) { required.push("native-prompt-receipts-v1"); }
            if agent {
                required.push("native-agent-v1");
            }
            if approval {
                required.push("native-approvals-v1");
            }
            client_handshake_with_options(
                &mut stream,
                &required,
                &["native-agent-v1", "native-approvals-v1"],
            )
            .await
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::InvalidData {
                    Error::Rejected(RemoteError {
                        code: ErrorCode::UnsupportedCapability,
                        outcome: qgenie_remote_proto::OperationOutcome::NotSent,
                        message:
                            "native IPC lacks required action capabilities; no operation was sent"
                                .into(),
                        retryable: false,
                    })
                } else {
                    Error::Unavailable
                }
            })?;
            write_dispatch(&mut stream, &request)
                .await
                .map_err(|_| Error::Unavailable)?;
            match read_dispatch(&mut stream)
                .await
                .map_err(|_| Error::Unavailable)?
            {
                DispatchFrame::Error(error) => Err(Error::Rejected(error)),
                response => Ok(response),
            }
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }
}
fn elapsed() -> Result<Duration, Error> {
    qgenie_remote_transport::elapsed::now().map_err(|_| stale())
}
fn stale() -> Error {
    Error::Rejected(RemoteError::rejected(
        ErrorCode::AccessStale,
        "original native access budget expired",
    ))
}
