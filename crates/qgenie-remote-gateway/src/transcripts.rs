//! Recording domain preparation. Protected persistence/read APIs are composed only with lifecycle policy.
pub(crate) const RECORDING_RELEASE_SCHEMA: i32 = 13;
pub(crate) mod api;
#[cfg(test)]
mod cancellation_tests;
pub(crate) mod database;
#[cfg(all(test, unix))]
mod durability;
pub(crate) mod holds;
pub(crate) mod ingest;
pub(crate) mod read;
mod redact;
mod retention;
pub(crate) mod service;
mod start;
mod transaction;
mod validate;

pub use database::{RecordingDatabase, RecordingRole};
pub use redact::{ProtectedRecording, RecordingRedaction};
pub use validate::{RecordingValidation, ValidatedRecording};
