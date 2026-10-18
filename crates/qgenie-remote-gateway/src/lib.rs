//! Gateway authority and HTTP composition. No native execution or local IPC.
mod app;
mod approvals;
mod audit;
pub use approvals::{ApprovalChoice, ApprovalDelivery, ApprovalProjection};
mod config;
mod controllers;
pub use controllers::{ControllerProposal, ControllerRequest, ControllerScope};
mod error;
mod identity;
mod revocation;
mod routing;
pub use identity::{DirectoryConfig, DirectoryObservation, GraphDirectory};
pub use identity::{IdentityService, JwtConfig, JwtVerifier, MachinePrincipal};
pub use routing::RoutingConfig;
mod store;
mod transaction;
mod transcripts;
pub use app::{Authenticator, Gateway, Principal, RejectAuthentication};
pub use audit::{
    AuditAction, AuditEvent, AuditMetadata, AuditRecord, RecordingLifecycleAudit,
    TranscriptIndexAudit, TranscriptReadAudit,
};
pub use config::{Config, SecretDsn};
pub use error::Error;
pub use store::{SCHEMA_MAX, SCHEMA_MIN, Store};
pub use transcripts::{
    ProtectedRecording, RecordingDatabase, RecordingRedaction, RecordingRole, RecordingValidation,
    ValidatedRecording,
};
#[cfg(test)]
mod tests;
