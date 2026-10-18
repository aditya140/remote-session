//! Local session authority, independent of a client or network connector.
mod approvals;
mod directory;
pub use approvals::{
    ApprovalSource, NativeApprovalIntent, NativeApprovalReservation, NativeApprovals,
};
mod recording;
mod runtime;
mod server;
mod store;

pub use recording::{
    CaptureContent, CapturedRecord, RecordingCapture, RecordingEncoder, RecordingIdentity,
    RecordingSpool, RecordingWorker, RedactionRules, SpoolConfig, SpoolItem, SpoolStatus,
};
#[cfg(all(feature = "managed-codex", target_os = "linux"))]
pub use runtime::CodexProfile;
pub use runtime::{Config, RecordingSettings, Runtime};
pub use server::{serve, serve_runtime};
