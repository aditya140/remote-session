use crate::*;
use serde::{Deserialize, Serialize};

macro_rules! wire {
    ($($item:item)*) => {$ (
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
        $item
    )*};
}

wire! {
    pub struct Request {
        pub v: u16,
        pub id: RequestId,
        pub client: ClientId,
        pub body: Operation,
    }
    pub struct Response {
        pub v: u16,
        pub re: RequestId,
        pub body: Reply,
    }
    #[serde(tag = "type", content = "body", rename_all = "snake_case")]
    pub enum Operation {
        Status,
        List,
        ListPage { after: Option<SessionId>, limit: u16, active_only: bool },
        Start(StartSpec),
        LookupStart { key: IdempotencyKey },
        Attach { session: SessionId },
        SnapshotPage { session: SessionId, epoch: MessageId, page: u16 },
        Replay { session: SessionId, after: Counter },
        TakeControl { session: SessionId, takeover: bool },
        ReleaseControl { session: SessionId },
        Input { session: SessionId, generation: Counter, input_seq: Counter, data: String },
        Resize { session: SessionId, generation: Counter, rows: u16, cols: u16 },
        Interrupt { session: SessionId, generation: Counter },
        Terminate { session: SessionId, confirmed: bool },
        Approvals,
    }
    #[serde(tag = "type", content = "body", rename_all = "snake_case")]
    pub enum Reply {
        Status(HostStatus),
        Sessions(Vec<SessionInfo>),
        SessionPage { sessions: Vec<SessionInfo>, next: Option<SessionId> },
        Started(SessionInfo),
        Attached(Attachment),
        SnapshotPage(Snapshot),
        Replay(ReplayBatch),
        Control(Lease),
        InputAck { input_seq: Counter, outcome: ApplicationOutcome },
        Approvals(Vec<Approval>),
        Ok,
        Error(RemoteError),
    }
    pub struct StartSpec {
        pub key: IdempotencyKey,
        pub command: Vec<String>,
        pub cwd: String,
        pub rows: u16,
        pub cols: u16,
    }
    pub struct HostStatus {
        pub machine: MachineId,
        pub name: String,
        pub os: String,
        pub arch: String,
        pub version: String,
        pub roots: Vec<String>,
        pub sessions: usize,
        pub mode: String,
    }
    #[serde(rename_all = "snake_case")]
    pub enum SessionState { Accepted, Launching, Running, Idle, Exited, Failed, Lost, OutcomeUnknown }
    pub struct ManagedSessionInfo {
        pub agent: String,
        pub native_instance: MessageId,
        pub native_thread: Option<String>,
        /// Canonical privately owned native history directory. Older/new roots use self.id.
        #[serde(default,skip_serializing_if="Option::is_none")]
        pub native_home: Option<SessionId>,
    }
    pub struct SessionInfo {
        pub id: SessionId,
        pub machine: MachineId,
        pub command: Vec<String>,
        pub cwd: String,
        pub state: SessionState,
        pub created_at_ms: Counter,
        /// Native-observed process end; absent when owner recovery cannot establish it.
        #[serde(default,skip_serializing_if="Option::is_none")]
        pub ended_at_ms: Option<Counter>,
        pub head: Counter,
        pub controller: Option<Lease>,
        pub exit_code: Option<u32>,
        pub recording_warning: Option<String>,
        #[serde(default,skip_serializing_if="Option::is_none")]
        pub agent: Option<ManagedSessionInfo>,
    }
    pub struct Lease {
        pub holder: ClientId,
        pub generation: Counter,
        pub expires_at_ms: Counter,
        pub last_input: Counter,
    }
    pub struct Snapshot {
        pub seq: Counter,
        pub rows: u16,
        pub cols: u16,
        pub format: SnapshotFormat,
        pub epoch: MessageId,
        pub page: u16,
        pub total_pages: u16,
        pub data: String,
    }
    #[serde(rename_all="snake_case")]
    #[derive(Copy,PartialEq,Eq)]
    pub enum SnapshotFormat { Vt100StateV1,AgentStateV1 }
    pub struct Attachment { pub session: SessionInfo, pub snapshot: Snapshot }
    pub struct OutputEvent { pub seq: Counter, pub at_ms: Counter, pub body: EventBody }
    #[serde(tag = "kind", content = "data", rename_all = "snake_case")]
    pub enum EventBody {
        Agent(crate::wire::AgentEvent),
        PtyOutput(String),
        Resize { rows: u16, cols: u16 },
        Exited { code: u32 },
        Gap { from: Counter, to: Counter, reason: String },
    }
    pub struct ReplayBatch {
        pub session: SessionInfo,
        pub floor: Counter,
        pub head: Counter,
        pub truncated: bool,
        pub events: Vec<OutputEvent>,
        pub more: bool,
    }
    pub struct Approval {
        pub id: ApprovalId,
        pub session: SessionId,
        pub summary: String,
        pub decision: Option<DecisionId>,
        pub application: Option<ApplicationOutcome>,
    }
    #[serde(rename_all = "snake_case")]
    pub enum ApplicationOutcome { Applied, AlreadyApplied, Cancelled, OutcomeUnknown }
    #[serde(rename_all = "snake_case")]
    pub enum OperationOutcome { NotSent, Unknown, Rejected }
    #[serde(rename_all = "snake_case")]
    #[derive(PartialEq,Eq)]
    pub enum ErrorCode {
        Unauthenticated, Forbidden, AccessStale, UnsupportedVersion, UnsupportedCapability,
        InvalidFrame, FrameTooLarge, Overloaded, IdempotencyConflict, AmbiguousName, StaleGeneration,
        AlreadyResolved, HistoryTruncated, CapacityExhausted, NotFound, Internal, ControlHeld,
    }
    pub struct RemoteError {
        pub code: ErrorCode,
        pub outcome: OperationOutcome,
        pub message: String,
        pub retryable: bool,
    }
}

impl RemoteError {
    pub fn rejected(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            outcome: OperationOutcome::Rejected,
            retryable: false,
        }
    }
    pub fn unknown(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Internal,
            message: message.into(),
            outcome: OperationOutcome::Unknown,
            retryable: false,
        }
    }
}

impl ManagedSessionInfo {
    pub fn validate(&self) -> Result<(), RemoteError> {
        if self.agent.is_empty()
            || self.agent.len() > 64
            || !self
                .agent
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || self.native_thread.as_ref().is_some_and(|thread| {
                thread.is_empty() || thread.len() > 512 || thread.chars().any(char::is_control)
            })
        {
            return Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "invalid managed native identity",
            ));
        }
        Ok(())
    }
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for RemoteError {}

pub fn validate_size(rows: u16, cols: u16) -> Result<(), RemoteError> {
    if !(1..=4096).contains(&rows) || !(1..=4096).contains(&cols) {
        return Err(RemoteError::rejected(
            ErrorCode::InvalidFrame,
            "terminal dimensions must be 1..=4096",
        ));
    }
    // A preview must not allocate millions of terminal cells from one request.
    if u32::from(rows) * u32::from(cols) > 262_144 {
        return Err(RemoteError::rejected(
            ErrorCode::CapacityExhausted,
            "terminal cell budget exceeded",
        ));
    }
    Ok(())
}
