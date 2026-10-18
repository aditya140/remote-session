//! Local administrative messages. Deliberately excluded from public schemas.
use crate::RemoteError;
use serde::{Deserialize, Serialize};

/// Immutable native ownership. Legacy/local sessions have no remote scope and
/// cannot become remotely visible merely because a connector is enrolled.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteScope {
    pub origin: String,
    pub owner: crate::UserId,
    pub machine: crate::MachineId,
}
impl RemoteScope {
    pub fn validate(&self) -> Result<(), RemoteError> {
        crate::validate_gateway_origin(&self.origin)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdminRequest {
    pub ipc_major: u16,
    pub command: AdminCommand,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminCommand {
    Shutdown {
        terminate_sessions: bool,
    },
    /// Trusted local enrollment composition only; never a remotely routed op.
    RegisterRemoteBinding {
        scope: RemoteScope,
    },
}
#[derive(Debug, Serialize, Deserialize)]
pub struct AdminResponse {
    pub result: Result<(), RemoteError>,
}

/// Independently versioned host/session-owner data protocol. This enum cannot
/// represent administrative shutdown or service/update operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum DispatchFrame {
    Hello {
        major: u16,
        minor: u16,
        capabilities: Vec<String>,
    },
    Welcome {
        major: u16,
        minor: u16,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        capabilities: Vec<String>,
    },
    Local(crate::Request),
    LocalReply(crate::Response),
    Remote {
        context: RemoteContext,
        message: Box<crate::wire::Envelope>,
    },
    RemoteReply(Box<crate::wire::Envelope>),
    AccessChallenge {
        scope: RemoteScope,
        connection: crate::ClientId,
        host_generation: crate::Counter,
    },
    QueryHostFloor {
        scope: RemoteScope,
    },
    AgentInventory {
        context: RemoteContext,
    },
    AgentInventoryReply {
        agents: std::collections::BTreeMap<String, crate::wire::AgentInventory>,
    },
    /// Coalesced read-only notifications, bound to the original host access.
    WatchChanges {
        context: RemoteContext,
    },
    ChangesReady,
    NativeChanged,
    ReadHead {
        context: RemoteContext,
        session: crate::SessionId,
    },
    HeadRead {
        session: crate::SessionId,
        floor: crate::Counter,
        head: crate::Counter,
        last_input: crate::Counter,
    },
    HostFloor {
        generation: crate::Counter,
    },
    ReadSnapshot {
        context: RemoteContext,
        session: crate::SessionId,
        epoch: Option<crate::MessageId>,
        page: u16,
    },
    SnapshotRead {
        session: crate::SessionId,
        snapshot: crate::wire::SnapshotPage,
    },
    AccessChallengeIssued {
        nonce: crate::ChallengeId,
        policy_revision: crate::Counter,
        max_remaining_ms: u32,
    },
    ActivateHost(HostActivation),
    HostActivated {
        nonce: crate::ChallengeId,
        generation: crate::Counter,
    },
    QueryControl {
        context: RemoteContext,
        session: crate::SessionId,
    },
    ControlState {
        session: crate::SessionId,
        floor: crate::Counter,
        holder: Option<crate::ClientId>,
        remaining_ms: u32,
        last_input: crate::Counter,
    },
    ActivateControl(ControlActivation),
    ControlActivated {
        session: crate::SessionId,
        holder: crate::ClientId,
        generation: crate::Counter,
        remaining_ms: u32,
        last_input: crate::Counter,
    },
    DeactivateControl {
        context: RemoteContext,
        session: crate::SessionId,
        generation: crate::Counter,
    },
    ControlDeactivated {
        session: crate::SessionId,
        generation: crate::Counter,
    },
    AccessRenewal(HostActivation),
    RecordingFetch {
        context: RemoteContext,
        max_bytes: u32,
    },
    RecordingFetched(Option<RecordingUpload>),
    RecordingStatus {
        context: RemoteContext,
    },
    RecordingStatusReply(RecordingStatus),
    RecordingEpochChallenge {
        context: RemoteContext,
    },
    RecordingEpochChallengeIssued {
        challenge: crate::ChallengeId,
    },
    BindRecordingEpoch {
        context: RemoteContext,
        authority: crate::rest::RecordingAuthority,
    },
    RecordingEpochBound {
        challenge: crate::ChallengeId,
    },
    RecordingAcknowledge {
        context: RemoteContext,
        challenge: crate::ChallengeId,
        receipt: RecordingReceipt,
    },
    RecordingAcknowledged {
        reclaimed: bool,
    },
    RecordingCommit(crate::wire::CommitReceipt),
    RecordingGapCommit(crate::wire::GapCommitReceipt),
    Error(RemoteError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "snake_case")]
pub enum RecordingUpload {
    Content(crate::wire::RecordingBatch),
    Gap(crate::wire::LossRange),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "snake_case")]
pub enum RecordingReceipt {
    Content(crate::wire::CommitReceipt),
    Gap(crate::wire::GapCommitReceipt),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStatus {
    pub pending_bytes: crate::Counter,
    pub chunks: crate::Counter,
    pub gaps: crate::Counter,
    pub warning: Option<String>,
    pub durability_unknown: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostActivation {
    pub scope: RemoteScope,
    pub connection: crate::ClientId,
    pub generation: crate::Counter,
    pub nonce: crate::ChallengeId,
    pub remaining_ms: u32,
    pub policy_revision: crate::Counter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlActivation {
    pub context: RemoteContext,
    pub session: crate::SessionId,
    pub holder: crate::ClientId,
    pub generation: crate::Counter,
    pub remaining_lease_ms: u32,
    pub takeover: bool,
    pub renewal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteContext {
    pub actor: crate::UserId,
    pub client: crate::ClientId,
    pub scope: RemoteScope,
    pub connection: crate::ClientId,
    pub host_generation: crate::Counter,
    pub controller_generation: Option<crate::Counter>,
    pub access_nonce: crate::ChallengeId,
    pub remaining_access_ms: u32,
    pub policy_revision: crate::Counter,
}

impl DispatchFrame {
    pub fn validate_hello(&self) -> Result<(), RemoteError> {
        match self {
            Self::Hello {
                major,
                minor: _,
                capabilities,
            } if *major == crate::IPC_MAJOR
                && capabilities.len() <= 64
                && capabilities
                    .iter()
                    .all(|cap| !cap.is_empty() && cap.len() <= 64) =>
            {
                Ok(())
            }
            Self::Hello { major, .. } if *major != crate::IPC_MAJOR => Err(RemoteError::rejected(
                crate::ErrorCode::UnsupportedVersion,
                "unsupported IPC major",
            )),
            _ => Err(RemoteError::rejected(
                crate::ErrorCode::InvalidFrame,
                "expected bounded IPC hello",
            )),
        }
    }
}
