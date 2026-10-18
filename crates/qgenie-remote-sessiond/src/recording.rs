//! Independent recording preparation. Runtime capture and durable upload are composed separately.
mod chunk;
mod redact;
mod spool;
mod worker;

pub use chunk::{CaptureContent, CapturedRecord, RecordingEncoder, RecordingIdentity};
pub use redact::RedactionRules;
pub use spool::{RecordingSpool, SpoolConfig, SpoolItem, SpoolStatus};
pub use worker::{RecordingCapture, RecordingWorker};
