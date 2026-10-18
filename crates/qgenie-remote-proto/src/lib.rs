//! Bounded, runtime-independent contracts for the standalone remote preview.
mod agent_state;
mod binary;
pub use agent_state::AgentState;
mod codec;
mod ids;
pub mod ipc;
mod model;
mod negotiation;
mod pty_display;
mod recording;
pub use pty_display::PtyDisplayGuard;
pub use recording::{RecordedContent, RecordingChannel, RecordingRecord};
pub mod rest;
pub mod wire;

pub use binary::{PtyFrame, PtyPayload};
pub use codec::{decode_json, decode_json_bounded, fingerprint};
pub use ids::*;
pub use model::*;
pub use negotiation::{
    Hello, MachineChallenge, Version, Welcome, negotiate, validate_gateway_origin,
};

pub const PROTOCOL_MAJOR: u16 = 1;
pub const IPC_MAJOR: u16 = 1;
pub const IPC_MINOR: u16 = 1;
/// Maximum authority horizon from the original primary read/native challenge.
/// Revocation completion relies on gateway and native I/O enforcing this bound.
pub const MAX_ACCESS_LIFETIME_MS: u32 = 10_000;
const _: () = assert!(MAX_ACCESS_LIFETIME_MS <= 10_000);
pub const MAX_JSON_BYTES: usize = 1024 * 1024;
pub const MAX_PTY_BYTES: usize = 32 * 1024;

#[cfg(feature = "schema")]
pub fn public_schema() -> schemars::Schema {
    schemars::schema_for!(wire::Envelope)
}

#[cfg(feature = "schema")]
pub fn recording_schema() -> schemars::Schema {
    schemars::schema_for!(RecordingRecord)
}

#[cfg(feature = "schema")]
mod rest_schema;
#[cfg(feature = "schema")]
pub fn rest_schema() -> schemars::Schema {
    schemars::schema_for!(rest_schema::RestSchemas)
}
