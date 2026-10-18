//! Normalized redacted recording lines, independent of the compression implementation.
use crate::{
    Counter, ErrorCode, RemoteError,
    wire::{RecordingStream, Timestamp},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RecordingRecord {
    pub v: u16,
    pub seq: Counter,
    pub captured_at: Timestamp,
    pub elapsed_ms: Counter,
    pub content: RecordedContent,
}
/// Fixed metadata labels. Vendor/user text belongs in the redacted data field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RecordingChannel {
    MessageDelta,
    UserMessage,
    ToolOutput,
    ToolStatus,
    ApprovalRequest,
    ApprovalDecision,
    ApprovalApplication,
    Usage,
    Warning,
    Native,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordedContent {
    Output {
        data: String,
        fragment: u32,
        last: bool,
    },
    AgentText {
        channel: RecordingChannel,
        data: String,
        fragment: u32,
        last: bool,
    },
    Resize {
        rows: u16,
        cols: u16,
    },
}
impl RecordingRecord {
    pub fn validate(&self, stream: &RecordingStream) -> Result<(), RemoteError> {
        let invalid = || {
            RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "invalid normalized recording record",
            )
        };
        if self.v != 1 || self.seq.0 == 0 {
            return Err(invalid());
        }
        let (data, fragment) = match (&self.content, stream) {
            (
                RecordedContent::Output { data, fragment, .. },
                RecordingStream::Pty | RecordingStream::Stdout | RecordingStream::Stderr,
            ) => (data, fragment),
            (RecordedContent::AgentText { data, fragment, .. }, RecordingStream::Events) => {
                (data, fragment)
            }
            (RecordedContent::Resize { rows, cols }, RecordingStream::Pty)
                if *rows > 0 && *cols > 0 =>
            {
                return Ok(());
            }
            _ => return Err(invalid()),
        };
        if *fragment > 32 || data.len() > (16 * 1024_usize).div_ceil(3) * 4 {
            return Err(invalid());
        }
        let bytes = STANDARD.decode(data).map_err(|_| invalid())?;
        if bytes.len() > 16 * 1024 || STANDARD.encode(bytes) != *data {
            return Err(invalid());
        }
        Ok(())
    }
}
