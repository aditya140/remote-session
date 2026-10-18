//! A schema catalog, not an HTTP payload wrapper.
#[derive(schemars::JsonSchema)]
#[allow(dead_code)]
pub struct RestSchemas {
    enrollment: crate::rest::EnrollmentRequest,
    proof: crate::rest::Proof,
    challenge_request: crate::rest::ChallengeRequest,
    challenge: crate::rest::Challenge,
    enrolled: crate::rest::Enrolled,
    machine_patch: crate::rest::MachinePatch,
    machine_revoke: crate::rest::MachineRevoke,
    revoked: crate::rest::Revoked,
    revoke_access: crate::rest::RevokeAccess,
    access_revocation: crate::rest::AccessRevocation,
    history_query: crate::rest::HistoryQuery,
    recording_authority_request: crate::rest::RecordingAuthorityRequest,
    recording_authority: crate::rest::RecordingAuthority,
    history_page: crate::rest::HistoryPage,
    history_streams_query: crate::rest::HistoryStreamsQuery,
    history_streams_page: crate::rest::HistoryStreamsPage,
    admin_history_streams_query: crate::rest::AdminHistoryStreamsQuery,
    admin_history_query: crate::rest::AdminHistoryQuery,
    hold_change: crate::rest::HoldChange,
    hold_commit: crate::rest::HoldCommit,
    lifecycle_progress: crate::rest::LifecycleProgress,
    machine_token: crate::rest::MachineToken,
    machines: Vec<crate::wire::Machine>,
    error: crate::RemoteError,
}
