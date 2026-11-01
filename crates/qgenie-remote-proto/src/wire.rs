//! Public network messages. Local dispatch/admin schemas are independently versioned.
use crate::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

macro_rules! types {
    ($($item:item)*) => {$ (
        #[derive(Debug,Clone,Serialize,Deserialize)]
        #[cfg_attr(feature="schema",derive(schemars::JsonSchema))]
        $item
    )*};
}

impl HostAccess {
    pub fn validate(&self) -> Result<(), RemoteError> {
        if self.host_generation.0 == 0
            || self.policy_revision.0 == 0
            || !(1..=MAX_ACCESS_LIFETIME_MS).contains(&self.remaining_ms)
        {
            return Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "invalid native access grant",
            ));
        }
        Ok(())
    }
}

/// Canonical UTC timestamp with exactly millisecond precision.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Timestamp(String);
impl TryFrom<String> for Timestamp {
    type Error = String;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        let format = time::format_description::parse_borrowed::<2>(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z",
        )
        .map_err(|_| "timestamp format")?;
        let parsed = time::PrimitiveDateTime::parse(&text, &format)
            .map_err(|_| "expected UTC timestamp with milliseconds")?;
        if parsed.assume_utc().unix_timestamp() < 0 {
            return Err("timestamp predates Unix epoch".into());
        }
        if parsed.format(&format).map_err(|_| "timestamp format")? != text {
            return Err("noncanonical timestamp".into());
        }
        Ok(Self(text))
    }
}
impl From<Timestamp> for String {
    fn from(timestamp: Timestamp) -> Self {
        timestamp.0
    }
}
impl Timestamp {
    pub fn from_unix_millis(millis: u64) -> Result<Self, RemoteError> {
        let datetime =
            time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000)
                .map_err(|_| invalid("timestamp out of range"))?;
        let format = time::format_description::parse_borrowed::<2>(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z",
        )
        .map_err(|_| invalid("timestamp format"))?;
        datetime
            .format(&format)
            .map_err(|_| invalid("timestamp format"))?
            .try_into()
            .map_err(|_| invalid("timestamp out of range"))
    }
    pub fn unix_millis(&self) -> u64 {
        let format = time::format_description::parse_borrowed::<2>(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z",
        )
        .expect("fixed timestamp format");
        let parsed = time::PrimitiveDateTime::parse(&self.0, &format).expect("validated timestamp");
        (parsed.assume_utc().unix_timestamp_nanos() / 1_000_000) as u64
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

types! {
    pub struct Envelope {
        pub v: u16,
        pub id: MessageId,
        #[serde(default,skip_serializing_if="Option::is_none")]
        pub re: Option<MessageId>,
        #[serde(default,skip_serializing_if="Option::is_none")]
        pub request: Option<RequestId>,
        #[serde(default,skip_serializing_if="Option::is_none")]
        pub machine: Option<MachineId>,
        #[serde(default,skip_serializing_if="Option::is_none")]
        pub session: Option<SessionId>,
        #[serde(default,skip_serializing_if="Option::is_none")]
        pub seq: Option<Counter>,
        #[serde(flatten)]
        pub message: Message,
    }

    #[serde(tag="type",content="body")]
    pub enum Message {
        #[serde(rename="hello")] Hello(Hello),
        #[serde(rename="welcome")] Welcome(Welcome),
        #[serde(rename="auth.refresh")] Reauthenticate { token:crate::rest::AccessToken },
        #[serde(rename="auth.refreshed")] AuthRefreshed { expires_at:Timestamp },
        #[serde(rename="connection.ready")] ConnectionReady { client:ClientId,host_generation:Option<Counter>,activated:bool },
        #[serde(rename="host.dispatch")] HostDispatch {context:HostContext,message:Box<Envelope>},
        #[serde(rename="host.access.challenge")] HostAccessChallenge {nonce:ChallengeId,host_generation:Counter,policy_revision:Counter,max_remaining_ms:u32},
        #[serde(rename="host.access.granted")] HostAccessGranted {access:HostAccess},
        #[serde(rename="host.activated")] HostActivated {nonce:ChallengeId,host_generation:Counter},
        #[serde(rename="host.activation.confirmed")] HostActivationConfirmed {nonce:ChallengeId,host_generation:Counter},
        #[serde(rename="host.control.request")] HostControlRequest(HostControlRequest),
        #[serde(rename="host.control.response")] HostControlResponse(HostControlResponse),
        #[serde(rename="goodbye")] Goodbye { code: u16,reason: String },
        #[serde(rename="machine.list")] MachineList,
        #[serde(rename="machine.snapshot")] Machines { machines: Vec<Machine> },
        #[serde(rename="machine.page")] MachinePage { epoch:String,page:u16,total_pages:u16,machines:Vec<Machine> },
        #[serde(rename="machine.rename")] Rename { name: String },
        #[serde(rename="machine.label")] Labels { labels: BTreeMap<String,String> },
        #[serde(rename="machine.revoke")] Revoke,
        #[serde(rename="machine.updated")] MachineUpdated(Machine),
        #[serde(rename="machine.presence")] Presence { online: bool,last_seen: Timestamp },
        #[serde(rename="inventory.update")] Inventory { agents: BTreeMap<String,AgentInventory> },
        #[serde(rename="fs.roots")] FsRoots,
        #[serde(rename="fs.roots.listed")] FsRootsListed { roots:Vec<String> },
        #[serde(rename="fs.list")] FsList { path: String,cursor: Option<String>,limit: u16 },
        #[serde(rename="fs.listed")] FsListed { entries: Vec<DirectoryEntry>,next: Option<String> },
        #[serde(rename="session.list")] SessionList,
        #[serde(rename="session.list.page")] SessionListPage { after:Option<SessionId>,limit:u16,active_only:bool },
        #[serde(rename="session.list.paged")] SessionPage { sessions:Vec<Session>,next:Option<SessionId> },
        #[serde(rename="session.get")] SessionGet { session:SessionId },
        #[serde(rename="session.found")] SessionFound(Session),
        #[serde(rename="session.metadata.update")] SessionMetadataUpdate { change: SessionMetadataChange },
        #[serde(rename="session.resolve")] SessionResolve { name: String },
        #[serde(rename="session.updates")] SessionUpdates { sessions: Vec<Session> },
        #[serde(rename="session.snapshot_list")] Sessions { sessions: Vec<Session> },
        #[serde(rename="session.start")] Start(Launch),
        #[serde(rename="session.start.lookup")] StartLookup { key: IdempotencyKey },
        #[serde(rename="session.started")] Started(Session),
        #[serde(rename="session.start.status")] StartStatus { key: IdempotencyKey,stage: StartStage,session: Option<SessionId> },
        #[serde(rename="session.attach")] Attach { after: Counter },
        #[serde(rename="session.capture")] Capture,
        #[serde(rename="session.replay.request")] ReplayRequest {after:Counter},
        #[serde(rename="session.host.query")] HostGenerationQuery,
        #[serde(rename="session.host")] HostGeneration {generation:Counter},
        #[serde(rename="session.detach")] Detach,
        #[serde(rename="session.replay")] Replay { floor: Counter,head: Counter,events: Vec<SequencedEvent>,more: bool,history_truncated: bool },
        #[serde(rename="session.head")] Head { floor: Counter,head: Counter,last_input: Counter },
        #[serde(rename="session.snapshot")] Snapshot { snapshot: SnapshotPage },
        #[serde(rename="session.lease.request")] LeaseRequest { takeover: bool },
        #[serde(rename="session.lease.granted")] LeaseGranted(ControlLease),
        #[serde(rename="session.lease.renew")] LeaseRenew { generation: Counter },
        #[serde(rename="session.lease.release")] LeaseRelease { generation: Counter },
        #[serde(rename="session.lease.revoked")] LeaseRevoked { generation: Counter,reason: String },
        #[serde(rename="session.input")] Input { authority: Generations,input_seq: Counter,input: SessionInput },
        #[serde(rename="session.input.ack")] InputAck { generation: Counter,input_seq: Counter,outcome: ApplicationOutcome, #[serde(default,skip_serializing_if="Option::is_none")] turn: Option<String> },
        #[serde(rename="session.resize")] Resize { authority: Generations,rows: u16,cols: u16 },
        #[serde(rename="session.interrupt")] Interrupt { authority: Generations },
        #[serde(rename="session.terminate")] Terminate { host_generation: Counter,confirmed: bool,external_confirmed: bool },
        #[serde(rename="session.action.result")] ActionResult { action: SessionAction,outcome: ApplicationOutcome },
        #[serde(rename="session.resume")] Resume { historical: SessionId,launch: Launch },
        #[serde(rename="session.event")] Event(SequencedEvent),
        #[serde(rename="session.exited")] Exited { code: u32 },
        #[serde(rename="approval.list")] ApprovalList,
        #[serde(rename="approval.get")] ApprovalGet { approval: ApprovalId },
        #[serde(rename="approval.status")] ApprovalStatus(ApprovalState),
        #[serde(rename="approval.index")] ApprovalIndex { after: Option<ApprovalId>,limit: u16 },
        #[serde(rename="approval.page")] ApprovalPage { states: Vec<ApprovalState>,next: Option<ApprovalId> },
        #[serde(rename="approval.pending")] Approvals { approvals: Vec<ApprovalRequest> },
        #[serde(rename="approval.requested")] ApprovalRequested(ApprovalRequest),
        #[serde(rename="approval.resolve")] ApprovalResolve { approval: ApprovalId,request_hash: String,decision: ApprovalDecision },
        #[serde(rename="approval.committed")] ApprovalCommitted(ApprovalWinner),
        #[serde(rename="approval.application")] ApprovalApplication { approval: ApprovalId,decision: DecisionId,outcome: ApplicationOutcome,reason: Option<String> },
        #[serde(rename="approval.cancelled")] ApprovalCancelled { approval: ApprovalId,reason: String },
        #[serde(rename="recording.ingest")] RecordingIngest(RecordingBatch),
        #[serde(rename="recording.receipt")] RecordingReceipt(CommitReceipt),
        #[serde(rename="recording.gap")] RecordingGap(LossRange),
        #[serde(rename="recording.gap_receipt")] RecordingGapReceipt(GapCommitReceipt),
        #[serde(rename="recording.warning")] RecordingWarning { reason: String,pending_bytes: Counter },
        #[serde(rename="error")] Error(RemoteError),
    }

    pub struct Machine {
        pub id: MachineId,pub owner: UserId,pub name: String,pub hostname: String,
        pub os: String,pub arch: String,pub cli_version: String,pub labels: BTreeMap<String,String>,
        pub agents: BTreeMap<String,AgentInventory>,pub online: bool,pub last_seen: Timestamp,pub sessions: u32,
    }
    pub struct HostContext {
        pub actor:UserId,pub client:ClientId,pub host_generation:Counter,pub expires_at:Timestamp,
        #[serde(default,skip_serializing_if="Option::is_none")] pub access:Option<HostAccess>,
    }
    #[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
    pub enum HostControlRequest {
        Query,
        Activate {generation:Counter,remaining_ms:u32,takeover:bool,renewal:bool},
        Release {generation:Counter},
    }
    #[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
    pub enum HostControlResponse {
        State {floor:Counter,holder:Option<ClientId>,remaining_ms:u32,last_input:Counter},
        Activated {holder:ClientId,generation:Counter,remaining_ms:u32,last_input:Counter},
        Released {generation:Counter},
    }
    #[derive(PartialEq,Eq)]
    #[serde(deny_unknown_fields)]
    pub struct HostAccess {
        pub connection:ClientId,pub host_generation:Counter,pub nonce:ChallengeId,
        pub remaining_ms:u32,pub policy_revision:Counter,
    }
    pub struct AgentInventory { pub version: String,pub capabilities: Capabilities }
    #[derive(Default)]
    #[serde(default)]
    pub struct Capabilities {
        pub input: bool,pub steer: bool,pub interrupt: bool,pub resize: bool,pub terminate: bool,
        pub resume: bool,pub approvals: BTreeSet<String>,pub external_control: bool,
    }
    pub struct DirectoryEntry { pub name: String,pub path: String,pub directory: bool }
    #[serde(rename_all="snake_case")]
    pub enum Origin { Managed,External }
    pub struct Session {
        /// Original native launch configuration; the effective runtime model remains unknown.
        #[serde(default,skip_serializing_if="Option::is_none",deserialize_with="crate::model::deserialize_requested_model")]
        pub requested_model: Option<String>,
        pub id: SessionId,pub machine: MachineId,pub owner: UserId,pub kind: LaunchKind,
        #[serde(default,skip_serializing_if="SessionMetadata::is_empty")]
        pub metadata: SessionMetadata,
        pub cwd: String,pub origin: Origin,pub state: SessionState,pub created_at: Timestamp,
        /// Original native observation, never gateway projection/upload time.
        #[serde(default,skip_serializing_if="Option::is_none")]
        pub ended_at: Option<Timestamp>,
        pub head: Counter,pub capabilities: Capabilities,pub recording_warning: Option<String>,
    }
    #[derive(Default,PartialEq,Eq)]
    #[serde(deny_unknown_fields)]
    pub struct SessionMetadata {
        #[serde(default)] pub name: Option<String>,
        #[serde(default)] pub labels: BTreeMap<String,String>,
    }
    #[serde(tag="kind",content="data",rename_all="snake_case",deny_unknown_fields)]
    pub enum SessionMetadataChange { Rename(Option<String>), Label { key: String,value: Option<String> } }
    pub struct Launch {
        pub key: IdempotencyKey,pub kind: LaunchKind,pub cwd: String,
        pub rows: u16,pub cols: u16,
    }
    #[serde(tag="kind",rename_all="snake_case")]
    pub enum LaunchKind { Pty { command: Vec<String> },Agent { agent: String,prompt: Option<String>,native_resume_id: Option<String> } }
    #[serde(rename_all="snake_case")]
    pub enum StartStage { Received,Accepted,Launching,Running,Failed,OutcomeUnknown,Lost }
    pub struct Generations { pub host: Counter,pub controller: Option<Counter> }
    pub struct ControlLease {
        pub holder: ClientId,pub host_generation: Counter,pub generation: Counter,
        pub expires_at: Timestamp,pub last_input: Counter,
    }
    #[serde(rename_all="snake_case")]
    pub enum SessionAction { Resize,Interrupt,Terminate }
    #[serde(tag="kind",content="data",rename_all="snake_case")]
    pub enum SessionInput { Pty(String),Prompt(String),PromptReceipt(String),Steer(String) }
    pub struct InputReceipt { pub outcome: ApplicationOutcome, pub turn: Option<String> }
    pub struct SnapshotPage {
        pub epoch: String,pub seq: Counter,pub page: u16,pub total_pages: u16,
        pub rows: u16,pub cols: u16,pub format: crate::SnapshotFormat,pub data: String,
    }
    pub struct SequencedEvent { pub seq: Counter,pub at: Timestamp,pub event: AgentEvent }
    #[serde(tag="kind",content="data",rename_all="snake_case")]
    pub enum AgentEvent {
        TurnStarted { turn: String },
        MessageDelta { message: String,text: String },
        MessageCompleted { message: String,text: String },
        ReasoningSummary { text: String },
        ToolStarted { tool: String,name: String,summary: String },
        ToolOutput { tool: String,text: String },
        ToolCompleted { tool: String,success: bool },
        FileDiff { path: String,diff: String },
        TurnCompleted {
            turn: String,input_tokens: Counter,output_tokens: Counter,
            /// Older peers and retained history can omit the final native outcome.
            #[serde(default,skip_serializing_if="Option::is_none")]
            outcome: Option<TurnOutcome>,
        },
        Idle,
        Error { message: String },
        PtyOutput { data: String },
        Resize { rows: u16,cols: u16 },
        Exited { code: u32 },
        Gap { from: Counter,to: Counter,reason: String },
        Opaque { native_kind: String,payload: serde_json::Value },
    }
    #[derive(Copy,PartialEq,Eq)]
    #[serde(rename_all="snake_case")]
    pub enum TurnOutcome { Completed,Failed,Interrupted }
    #[derive(PartialEq, Eq)]
    pub struct ApprovalRequest {
        pub id: ApprovalId,pub session: SessionId,pub owner: UserId,pub native_id: String,
        pub kind: String,pub summary: String,pub request_hash: String,pub policy_revision: Counter,
        pub requested_at: Timestamp,pub expires_at: Option<Timestamp>,pub allowed_decisions: Vec<ApprovalDecision>,
    }
    #[serde(rename_all="snake_case")]
    #[derive(Copy, PartialEq, Eq)]
    pub enum ApprovalDecision { AllowOnce,AllowSession,Deny }
    #[derive(PartialEq, Eq)]
    pub struct ApprovalWinner {
        pub approval: ApprovalId,pub id: DecisionId,pub actor: UserId,pub client: ClientId,
        pub request_hash: String,pub policy_revision: Counter,pub decision: ApprovalDecision,
        pub committed_at: Timestamp,
    }
    pub struct ApprovalState {
        pub machine: MachineId,pub request: ApprovalRequest,pub winner: Option<ApprovalWinner>,
        pub cancelled: bool,pub outcome: Option<ApplicationOutcome>,pub reason: Option<String>,
    }
    #[serde(rename_all="snake_case")]
    pub enum RecordingStream { Events,Pty,Stdout,Stderr }
    #[serde(rename_all="snake_case")]
    pub enum RecordingCodec { ZstdJsonl,ZstdAsciicast }
    pub struct RecordingBatch {
        pub id: ChunkId,pub session: SessionId,pub machine: MachineId,pub stream: RecordingStream,
        pub stream_epoch: RecordingEpochId,
        pub from: Counter,pub to: Counter,pub captured_from: Timestamp,pub captured_to: Timestamp,
        pub codec: RecordingCodec,pub uncompressed_bytes: u32,pub digest: String,pub data: String,
    }
    #[serde(rename_all="snake_case")]
    pub enum ReceiptDisposition { Stored,Expired }
    pub struct CommitReceipt {
        pub id: ReceiptId,pub chunk: ChunkId,pub session: SessionId,pub machine: MachineId,
        pub stream_epoch: RecordingEpochId,pub captured_from: Timestamp,pub captured_to: Timestamp,
        pub from: Counter,pub to: Counter,pub digest: String,pub recovery_epoch: String,
        pub disposition: ReceiptDisposition,pub committed_at: Timestamp,
    }
    #[serde(rename_all="snake_case")]
    pub enum LossReason { SpoolCap,DiskFailure,RecoveryLoss,ReplayTruncated,CaptureBackpressure,Unknown }
    pub struct LossRange {
        pub id: GapId,pub session: SessionId,pub stream: RecordingStream,pub from: Counter,pub to: Counter,
        pub stream_epoch: RecordingEpochId,
        pub captured_from: Timestamp,pub captured_to: Timestamp,pub reason: LossReason,
    }
    pub struct GapCommitReceipt {
        pub id: ReceiptId,pub gap: GapId,pub session: SessionId,pub machine: MachineId,
        pub stream_epoch: RecordingEpochId,pub from: Counter,pub to: Counter,
        pub captured_from: Timestamp,pub captured_to: Timestamp,pub digest: String,
        pub recovery_epoch: String,pub disposition: ReceiptDisposition,pub committed_at: Timestamp,
    }
}

impl Envelope {
    pub fn new(message: Message) -> Self {
        Self {
            v: PROTOCOL_MAJOR,
            id: MessageId::new(),
            re: None,
            request: None,
            machine: None,
            session: None,
            seq: None,
            message,
        }
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, RemoteError> {
        let frame: Self = decode_json(bytes)?;
        frame.validate()?;
        Ok(frame)
    }
    pub fn encode(&self) -> Result<Vec<u8>, RemoteError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| invalid("envelope encoding failed"))?;
        if bytes.len() > MAX_JSON_BYTES {
            return Err(RemoteError::rejected(
                ErrorCode::FrameTooLarge,
                "JSON frame exceeds 1 MiB",
            ));
        }
        Ok(bytes)
    }
    pub fn validate(&self) -> Result<(), RemoteError> {
        if self.v != PROTOCOL_MAJOR {
            return Err(RemoteError::rejected(
                ErrorCode::UnsupportedVersion,
                "unsupported protocol major",
            ));
        }
        if self.seq.is_some_and(|seq| seq.0 == 0) {
            return Err(invalid("event sequence must be positive"));
        }
        let requires_session = matches!(
            &self.message,
            Message::Attach { .. }
                | Message::Detach
                | Message::Replay { .. }
                | Message::Head { .. }
                | Message::Snapshot { .. }
                | Message::LeaseRequest { .. }
                | Message::LeaseGranted(_)
                | Message::LeaseRenew { .. }
                | Message::LeaseRelease { .. }
                | Message::LeaseRevoked { .. }
                | Message::Input { .. }
                | Message::InputAck { .. }
                | Message::SessionMetadataUpdate { .. }
                | Message::Resize { .. }
                | Message::Interrupt { .. }
                | Message::Terminate { .. }
                | Message::ActionResult { .. }
                | Message::Capture
                | Message::HostGenerationQuery
                | Message::HostGeneration { .. }
                | Message::ReplayRequest { .. }
                | Message::HostControlRequest(_)
                | Message::HostControlResponse(_)
                | Message::Event(_)
                | Message::Exited { .. }
        );
        if requires_session && self.session.is_none() {
            return Err(invalid("session target is missing"));
        }
        match &self.message {
            Message::HostControlRequest(_) if self.machine.is_none() || self.re.is_some() => {
                return Err(invalid("invalid native control request target"));
            }
            Message::HostControlResponse(_)
            | Message::ActionResult { .. }
            | Message::HostGeneration { .. }
                if self.machine.is_none() || self.re.is_none() =>
            {
                return Err(invalid("invalid native action reply target"));
            }
            Message::HostDispatch { context, message } => {
                if let Some(access) = &context.access {
                    access.validate()?;
                    if access.host_generation != context.host_generation {
                        return Err(invalid("access generation differs from dispatch"));
                    }
                }
                if context.host_generation.0 == 0
                    || self.machine.is_none()
                    || self.machine != message.machine
                    || matches!(
                        message.message,
                        Message::HostDispatch { .. } | Message::Reauthenticate { .. }
                    )
                {
                    return Err(invalid("invalid host dispatch context"));
                }
                message.validate()?;
            }
            Message::HostAccessGranted { access } => {
                if self.machine.is_none() || self.session.is_some() || self.re.is_none() {
                    return Err(invalid("invalid native grant target/correlation"));
                }
                access.validate()?;
            }
            Message::HostAccessChallenge {
                host_generation,
                policy_revision,
                max_remaining_ms,
                ..
            } => {
                if self.re.is_some()
                    || self.machine.is_none()
                    || self.session.is_some()
                    || host_generation.0 == 0
                    || policy_revision.0 == 0
                    || !(1..=MAX_ACCESS_LIFETIME_MS).contains(max_remaining_ms)
                {
                    return Err(invalid("invalid native access challenge"));
                }
            }
            Message::HostActivated {
                host_generation, ..
            }
            | Message::HostActivationConfirmed {
                host_generation, ..
            } => {
                if self.re.is_none()
                    || self.machine.is_none()
                    || self.session.is_some()
                    || host_generation.0 == 0
                {
                    return Err(invalid("invalid native activation acknowledgment"));
                }
            }
            Message::MachinePage {
                epoch,
                page,
                total_pages,
                machines,
            } if epoch.is_empty()
                || epoch.len() > 128
                || *total_pages == 0
                || *total_pages > 1024
                || page >= total_pages
                || machines.len() > 64 =>
            {
                return Err(invalid("invalid machine page"));
            }
            Message::Hello(hello) => {
                if hello.versions.is_empty()
                    || hello.versions.len() > 16
                    || hello.required.len() + hello.optional.len() > 64
                    || hello
                        .required
                        .iter()
                        .chain(&hello.optional)
                        .any(|c| c.is_empty() || c.len() > 64)
                {
                    return Err(invalid("invalid hello limits"));
                }
            }
            Message::Welcome(welcome) => {
                if welcome.version.major != PROTOCOL_MAJOR || welcome.capabilities.len() > 64 {
                    return Err(invalid("invalid welcome"));
                }
            }
            Message::Rename { name } if name.is_empty() || name.len() > 256 => {
                return Err(invalid("invalid machine name"));
            }
            Message::Labels { labels }
                if labels.len() > 64
                    || labels
                        .iter()
                        .any(|(k, v)| k.is_empty() || k.len() > 64 || v.len() > 256) =>
            {
                return Err(invalid("invalid labels"));
            }
            Message::FsRootsListed { roots }
                if roots.is_empty()
                    || roots.len() > 16
                    || roots.iter().any(|root| {
                        !root.starts_with('/')
                            || root.len() > 4096
                            || root.chars().any(char::is_control)
                    }) =>
            {
                return Err(invalid("invalid allowed roots"));
            }
            Message::SessionMetadataUpdate { change } => {
                if self.machine.is_none() || self.re.is_some() {
                    return Err(invalid("invalid session metadata target"));
                }
                let mut metadata = SessionMetadata::default();
                metadata.apply(change)?;
            }
            Message::SessionResolve { name } => {
                if self.machine.is_none() || self.session.is_some() || self.re.is_some() {
                    return Err(invalid("invalid session name lookup target"));
                }
                SessionMetadata::validate_name(name)?;
            }
            Message::SessionListPage { limit, .. } if !(1..=256).contains(limit) => {
                return Err(invalid("invalid session page limit"));
            }
            Message::SessionPage { sessions, .. } | Message::SessionUpdates { sessions }
                if sessions.len() > 256 =>
            {
                return Err(invalid("session page count exceeds limit"));
            }
            Message::FsList { path, limit, .. }
                if path.is_empty()
                    || path.len() > 4096
                    || path.contains('\0')
                    || *limit == 0
                    || *limit > 1024 =>
            {
                return Err(invalid("invalid directory request"));
            }
            Message::Start(launch) | Message::Resume { launch, .. } => launch.validate()?,
            Message::Input {
                authority,
                input_seq,
                input,
            } => {
                authority.validate(true)?;
                if input_seq.0 == 0 {
                    return Err(invalid("input sequence must be positive"));
                }
                match input {
                    SessionInput::Pty(data) => {
                        decode_base64(data, MAX_PTY_BYTES)?;
                    }
                    SessionInput::Prompt(text)
                    | SessionInput::PromptReceipt(text)
                    | SessionInput::Steer(text)
                        if text.len() > 256 * 1024 =>
                    {
                        return Err(invalid("prompt limit exceeded"));
                    }
                    _ => {}
                }
            }
            Message::Resize {
                authority,
                rows,
                cols,
            } => {
                authority.validate(true)?;
                validate_size(*rows, *cols)?;
            }
            Message::Interrupt { authority } => authority.validate(true)?,
            Message::HostGeneration { generation } if generation.0 == 0 => {
                return Err(invalid("host generation must be positive"));
            }
            Message::HostControlRequest(HostControlRequest::Activate {
                generation,
                remaining_ms,
                ..
            }) if generation.0 == 0 || !(1..=15_000).contains(remaining_ms) => {
                return Err(invalid("invalid native controller activation"));
            }
            Message::HostControlRequest(HostControlRequest::Release { generation })
                if generation.0 == 0 =>
            {
                return Err(invalid("invalid native controller release"));
            }
            Message::HostControlResponse(HostControlResponse::State {
                floor,
                holder,
                remaining_ms,
                last_input,
            }) if *remaining_ms > 15_000
                || holder.is_some() != (*remaining_ms > 0)
                || (holder.is_some() && floor.0 == 0)
                || (holder.is_none() && last_input.0 != 0) =>
            {
                return Err(invalid("invalid native controller state"));
            }
            Message::HostControlResponse(HostControlResponse::Activated {
                generation,
                remaining_ms,
                ..
            }) if generation.0 == 0 || !(1..=15_000).contains(remaining_ms) => {
                return Err(invalid("invalid native controller acknowledgment"));
            }
            Message::HostControlResponse(HostControlResponse::Released { generation })
                if generation.0 == 0 =>
            {
                return Err(invalid("invalid native controller release"));
            }
            Message::Terminate {
                host_generation, ..
            } if host_generation.0 == 0 => return Err(invalid("host generation must be positive")),
            Message::Snapshot { snapshot } => {
                validate_size(snapshot.rows, snapshot.cols)?;
                if snapshot.total_pages == 0
                    || snapshot.total_pages > 32
                    || snapshot.page >= snapshot.total_pages
                    || snapshot.epoch.len() > 128
                {
                    return Err(invalid("invalid snapshot page"));
                }
                decode_base64(&snapshot.data, 256 * 1024)?;
            }
            Message::RecordingIngest(batch) => batch.validate()?,
            Message::Event(event) => {
                if event.seq.0 == 0 || self.seq != Some(event.seq) {
                    return Err(invalid("event cursor does not match envelope"));
                }
            }
            Message::Replay {
                floor,
                head,
                events,
                ..
            } => {
                if events.len() > 512 || (*floor > *head && head.0.checked_add(1) != Some(floor.0))
                {
                    return Err(invalid("invalid replay bounds"));
                }
                let mut previous = 0;
                for event in events {
                    if event.seq.0 == 0
                        || event.seq.0 <= previous
                        || event.seq > *head
                        || event.seq < *floor
                    {
                        return Err(invalid("invalid replay event order"));
                    }
                    previous = event.seq.0;
                }
            }
            Message::RecordingGap(gap) => {
                gap.validate()?;
            }
            Message::RecordingReceipt(receipt) => validate_receipt(
                receipt.from,
                receipt.to,
                &receipt.captured_from,
                &receipt.captured_to,
                &receipt.digest,
                &receipt.recovery_epoch,
            )?,
            Message::RecordingGapReceipt(receipt) => validate_receipt(
                receipt.from,
                receipt.to,
                &receipt.captured_from,
                &receipt.captured_to,
                &receipt.digest,
                &receipt.recovery_epoch,
            )?,
            Message::ApprovalRequested(request) => {
                request.validate()?;
                if self
                    .session
                    .is_some_and(|session| session != request.session)
                {
                    return Err(invalid("approval session differs from envelope"));
                }
            }
            Message::Approvals { approvals } => {
                if approvals.len() > 200 {
                    return Err(invalid("approval page exceeds limit"));
                }
                for request in approvals {
                    request.validate()?;
                }
            }
            Message::ApprovalCommitted(winner) => winner.validate()?,
            Message::ApprovalStatus(state) => {
                state.validate()?;
                if self.re.is_none()
                    || self.machine.is_some_and(|machine| machine != state.machine)
                    || self
                        .session
                        .is_some_and(|session| session != state.request.session)
                {
                    return Err(invalid("approval status target/correlation differs"));
                }
            }
            Message::ApprovalIndex { limit, .. } if !(1..=200).contains(limit) => {
                return Err(invalid("approval page limit must be 1..=200"));
            }
            Message::ApprovalPage { states, next } => {
                if self.re.is_none()
                    || self.machine.is_some()
                    || self.session.is_some()
                    || states.len() > 200
                    || next.is_some_and(|next| {
                        states.last().is_none_or(|state| state.request.id != next)
                    })
                {
                    return Err(invalid("invalid approval page/cursor"));
                }
                let mut previous = None;
                for state in states {
                    state.validate()?;
                    if previous.is_some_and(|id| state.request.id <= id) {
                        return Err(invalid("approval page is not ordered"));
                    }
                    previous = Some(state.request.id);
                }
                if serde_json::to_vec(states)
                    .map_err(|_| invalid("approval page encoding failed"))?
                    .len()
                    > 512 * 1024
                {
                    return Err(invalid("approval page byte limit exceeded"));
                }
            }
            Message::ApprovalResolve { request_hash, .. } => {
                validate_digest(request_hash)?;
                if self.machine.is_none() || self.session.is_none() || self.re.is_some() {
                    return Err(invalid("approval decision target/correlation differs"));
                }
            }
            Message::ApprovalApplication { reason, .. }
                if reason.as_ref().is_some_and(|reason| reason.len() > 1024) =>
            {
                return Err(invalid("approval application reason exceeds limit"));
            }
            _ => {}
        }
        match &self.message {
            Message::RecordingIngest(batch)
                if self.machine.is_some_and(|id| id != batch.machine)
                    || self.session.is_some_and(|id| id != batch.session) =>
            {
                return Err(invalid("recording target mismatch"));
            }
            Message::RecordingGap(gap)
                if self.machine.is_none() || self.session != Some(gap.session) =>
            {
                return Err(invalid("recording gap target mismatch"));
            }
            Message::RecordingReceipt(receipt)
                if self.machine != Some(receipt.machine)
                    || self.session != Some(receipt.session) =>
            {
                return Err(invalid("recording receipt target mismatch"));
            }
            Message::RecordingGapReceipt(receipt)
                if self.machine != Some(receipt.machine)
                    || self.session != Some(receipt.session) =>
            {
                return Err(invalid("recording gap receipt target mismatch"));
            }
            Message::Started(session)
                if self.machine.is_some_and(|id| id != session.machine)
                    || self.session.is_some_and(|id| id != session.id) =>
            {
                return Err(invalid("session target mismatch"));
            }
            _ => {}
        }
        Ok(())
    }
}
impl Launch {
    pub fn validate(&self) -> Result<(), RemoteError> {
        validate_size(self.rows, self.cols)?;
        if self.cwd.is_empty() || self.cwd.len() > 4096 || self.cwd.contains('\0') {
            return Err(invalid("invalid launch directory"));
        }
        match &self.kind {
            LaunchKind::Pty { command }
                if command.is_empty()
                    || command.len() > 128
                    || command.iter().any(|s| s.contains('\0'))
                    || command.iter().map(String::len).sum::<usize>() > 65536 =>
            {
                Err(invalid("invalid command"))
            }
            LaunchKind::Agent {
                agent,
                prompt,
                native_resume_id,
            } if agent.is_empty()
                || agent.len() > 64
                || prompt.as_ref().is_some_and(|p| p.len() > 256 * 1024)
                || native_resume_id.as_ref().is_some_and(|id| id.len() > 1024) =>
            {
                Err(invalid("invalid agent launch"))
            }
            _ => Ok(()),
        }
    }
    pub fn fingerprint(&self) -> Result<String, RemoteError> {
        self.validate()?;
        let mut value =
            serde_json::to_value(self).map_err(|_| invalid("launch encoding failed"))?;
        value.as_object_mut().expect("launch object").remove("key");
        fingerprint(&value)
    }
}
impl Generations {
    pub fn validate(&self, controller: bool) -> Result<(), RemoteError> {
        if self.host.0 == 0 || (controller && !self.controller.is_some_and(|g| g.0 > 0)) {
            return Err(invalid("mutation generation is missing"));
        }
        Ok(())
    }
}
impl RecordingBatch {
    pub fn validate(&self) -> Result<(), RemoteError> {
        self.validate_metadata()?;
        decode_base64(&self.data, 64 * 1024)?;
        Ok(())
    }
    /// The spool/receipt ledger stores immutable metadata separately from compressed bytes.
    pub fn validate_metadata(&self) -> Result<(), RemoteError> {
        validate_range(self.from, self.to)?;
        validate_digest(&self.digest)?;
        if self.captured_from > self.captured_to
            || self.captured_from.as_str()[..7] != self.captured_to.as_str()[..7]
            || self.uncompressed_bytes == 0
            || self.uncompressed_bytes > MAX_JSON_BYTES as u32
        {
            return Err(invalid("invalid recording bounds"));
        }
        Ok(())
    }
}
impl LossRange {
    pub fn validate(&self) -> Result<(), RemoteError> {
        validate_range(self.from, self.to)?;
        if self.captured_from > self.captured_to {
            return Err(invalid("invalid gap capture range"));
        }
        Ok(())
    }
    /// SHA-256 input for gap receipts: canonical sorted-key JSON of this complete immutable range.
    /// Content receipts instead hash exact compressed bytes, before base64 transport encoding.
    pub fn receipt_digest(&self) -> Result<String, RemoteError> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| invalid("invalid gap metadata"))?;
        crate::fingerprint(&value)
    }
}
fn validate_receipt(
    from: Counter,
    to: Counter,
    start: &Timestamp,
    end: &Timestamp,
    digest: &str,
    epoch: &str,
) -> Result<(), RemoteError> {
    validate_range(from, to)?;
    validate_digest(digest)?;
    if start > end
        || epoch.is_empty()
        || epoch.len() > 128
        || !epoch
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(invalid("invalid recording receipt"));
    }
    Ok(())
}
impl ApprovalRequest {
    pub fn validate(&self) -> Result<(), RemoteError> {
        validate_digest(&self.request_hash)?;
        if self.policy_revision.0 == 0
            || self.native_id.is_empty()
            || self.native_id.len() > 512
            || self.native_id.chars().any(char::is_control)
            || self.kind.is_empty()
            || self.kind.len() > 64
            || !self
                .kind
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            || self.summary.len() > 8192
            || self.allowed_decisions.is_empty()
            || self.allowed_decisions.len() > 3
            || self
                .allowed_decisions
                .iter()
                .enumerate()
                .any(|(index, decision)| self.allowed_decisions[..index].contains(decision))
            || self
                .expires_at
                .as_ref()
                .is_some_and(|at| at.unix_millis() <= self.requested_at.unix_millis())
            || serde_json::to_vec(self)
                .map_err(|_| invalid("invalid approval"))?
                .len()
                > 16384
        {
            return Err(invalid("invalid approval request bounds or decision menu"));
        }
        Ok(())
    }
}
impl ApprovalWinner {
    pub fn validate(&self) -> Result<(), RemoteError> {
        validate_digest(&self.request_hash)?;
        if self.policy_revision.0 == 0 {
            return Err(invalid("approval policy revision must be positive"));
        }
        Ok(())
    }
}
impl ApprovalState {
    pub fn validate(&self) -> Result<(), RemoteError> {
        self.request.validate()?;
        if let Some(winner) = &self.winner {
            winner.validate()?;
            if winner.approval != self.request.id
                || winner.actor != self.request.owner
                || winner.request_hash != self.request.request_hash
                || winner.policy_revision != self.request.policy_revision
                || !self.request.allowed_decisions.contains(&winner.decision)
            {
                return Err(invalid("approval status winner differs from request"));
            }
        }
        if (self.outcome.is_some() && self.winner.is_none())
            || self
                .reason
                .as_ref()
                .is_some_and(|reason| reason.len() > 1024)
        {
            return Err(invalid("invalid approval application status"));
        }
        Ok(())
    }
}
pub fn validate_digest(digest: &str) -> Result<(), RemoteError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("expected SHA-256 hex digest"));
    }
    Ok(())
}
fn validate_range(from: Counter, to: Counter) -> Result<(), RemoteError> {
    if from.0 == 0 || from > to {
        return Err(invalid("invalid sequence range"));
    }
    Ok(())
}
pub fn decode_base64(data: &str, limit: usize) -> Result<Vec<u8>, RemoteError> {
    if data.len() > limit.div_ceil(3) * 4 {
        return Err(RemoteError::rejected(
            ErrorCode::FrameTooLarge,
            "encoded payload exceeds its limit",
        ));
    }
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| invalid("invalid base64 payload"))?;
    if bytes.is_empty() || bytes.len() > limit || STANDARD.encode(&bytes) != data {
        return Err(invalid("invalid or noncanonical payload"));
    }
    Ok(bytes)
}
fn invalid(message: &str) -> RemoteError {
    RemoteError::rejected(ErrorCode::InvalidFrame, message)
}

impl SessionMetadata {
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.labels.is_empty()
    }
    pub fn validate_name(name: &str) -> Result<(), RemoteError> {
        if name.is_empty()
            || name.len() > 128
            || name.trim() != name
            || name.chars().any(char::is_control)
        {
            return Err(invalid(
                "session name must be 1..=128 bytes without edge whitespace or controls",
            ));
        }
        Ok(())
    }
    pub fn apply(&mut self, change: &SessionMetadataChange) -> Result<(), RemoteError> {
        match change {
            SessionMetadataChange::Rename(name) => {
                if let Some(name) = name {
                    Self::validate_name(name)?;
                }
                self.name = name.clone();
            }
            SessionMetadataChange::Label { key, value } => {
                if key.is_empty()
                    || key.len() > 64
                    || !key
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
                    || value.as_ref().is_some_and(|value| {
                        value.len() > 128 || value.chars().any(char::is_control)
                    })
                {
                    return Err(invalid("invalid session label"));
                }
                if let Some(value) = value {
                    if self.labels.len() >= 16 && !self.labels.contains_key(key) {
                        return Err(invalid("at most 16 session labels"));
                    }
                    self.labels.insert(key.clone(), value.clone());
                } else {
                    self.labels.remove(key);
                }
            }
        }
        Ok(())
    }
    pub fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.name
            .as_ref()
            .is_some_and(|name| name.to_lowercase().contains(&query))
            || self.labels.iter().any(|(key, value)| {
                key.to_lowercase().contains(&query) || value.to_lowercase().contains(&query)
            })
    }
}
