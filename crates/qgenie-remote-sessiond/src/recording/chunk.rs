use super::redact::{Redacted, RedactionRules, Redactor};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{
    ChunkId, Counter, MachineId, RecordedContent as Content, RecordingEpochId,
    RecordingRecord as Record, SessionId, UserId,
    wire::{RecordingBatch, RecordingCodec, RecordingStream, Timestamp},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const FRAGMENT_BYTES: usize = 16 * 1024;
const RAW_LIMIT: usize = 1024 * 1024;
const COMPRESSED_LIMIT: usize = 64 * 1024;

/// Immutable original session identity. Owners are retained locally, never accepted as ingest authority.
#[derive(Clone, Serialize, Deserialize)]
pub struct RecordingIdentity {
    pub origin: String,
    pub session: SessionId,
    pub machine: MachineId,
    pub owner: UserId,
    pub stream: RecordingStream,
    pub stream_epoch: RecordingEpochId,
}

/// Intentionally has no input/keystroke variant. Output can contain arbitrary terminal bytes.
pub enum CaptureContent {
    Output(Vec<u8>),
    AgentText {
        channel: qgenie_remote_proto::RecordingChannel,
        bytes: Vec<u8>,
    },
    Resize {
        rows: u16,
        cols: u16,
    },
}

/// Sequence must already have been durably reserved by the independent spool worker.
pub struct CapturedRecord {
    pub seq: Counter,
    pub captured_at: Timestamp,
    pub elapsed_ms: Counter,
    pub content: CaptureContent,
}

struct Pending {
    record: CapturedRecord,
    unresolved: usize,
    redacted: Vec<u8>,
}
struct Line {
    seq: Counter,
    time: Timestamp,
    data: Vec<u8>,
}

/// CPU-only, bounded encoder. Call on an owned recording worker, never a live input/output lock.
/// Timer flushes retain possible secret prefixes across chunk boundaries until resolved or EOF.
pub struct RecordingEncoder {
    identity: RecordingIdentity,
    redactor: Redactor,
    pending: VecDeque<Pending>,
    lines: Vec<Line>,
    raw_bytes: usize,
    started: Instant,
    previous: u64,
    finished: bool,
}
impl RecordingEncoder {
    pub fn new(identity: RecordingIdentity, rules: RedactionRules) -> Self {
        Self {
            identity,
            redactor: Redactor::new(rules),
            pending: VecDeque::new(),
            lines: Vec::new(),
            raw_bytes: 0,
            started: Instant::now(),
            previous: 0,
            finished: false,
        }
    }

    pub fn push(&mut self, mut record: CapturedRecord) -> Result<Vec<RecordingBatch>> {
        ensure!(!self.finished, "recording stream finished");
        ensure!(
            record.seq.0 > 0
                && (self.previous == 0 || self.previous.checked_add(1) == Some(record.seq.0)),
            "recording sequence is not contiguous"
        );
        ensure!(
            self.pending.len() < 1024,
            "recording normalization backlog full"
        );
        ensure!(
            matches!(
                (&self.identity.stream, &record.content),
                (RecordingStream::Events, CaptureContent::AgentText { .. })
                    | (
                        RecordingStream::Pty,
                        CaptureContent::Output(_) | CaptureContent::Resize { .. }
                    )
                    | (
                        RecordingStream::Stdout | RecordingStream::Stderr,
                        CaptureContent::Output(_)
                    )
            ),
            "capture content does not match its recording stream"
        );
        let bytes = match &record.content {
            CaptureContent::Output(bytes) => bytes.as_slice(),
            CaptureContent::AgentText { bytes, .. } => bytes.as_slice(),
            CaptureContent::Resize { rows, cols } => {
                ensure!(*rows > 0 && *cols > 0, "invalid recording terminal size");
                &[]
            }
        };
        ensure!(
            bytes.len() <= 32 * 1024,
            "capture event exceeds bound; fragment before admission"
        );
        let pieces = self.redactor.push(record.seq.0, bytes);
        let count = bytes.len();
        // Unredacted input belongs only to this admission, never retained in queued records.
        match &mut record.content {
            CaptureContent::Output(bytes) | CaptureContent::AgentText { bytes, .. } => {
                zeroize::Zeroize::zeroize(bytes);
                bytes.clear();
            }
            _ => {}
        }
        self.previous = record.seq.0;
        self.pending.push_back(Pending {
            record,
            unresolved: count,
            redacted: Vec::new(),
        });
        self.apply(pieces)?;
        let mut batches = Vec::new();
        self.drain_ready(&mut batches)?;
        if self.started.elapsed() >= Duration::from_secs(2) || self.raw_bytes >= COMPRESSED_LIMIT {
            let size = self.compressed_size()?;
            if self.started.elapsed() >= Duration::from_secs(2)
                || size >= COMPRESSED_LIMIT
                || self.raw_bytes >= RAW_LIMIT / 2
            {
                batches.extend(self.flush()?);
            }
        }
        Ok(batches)
    }

    pub fn flush(&mut self) -> Result<Vec<RecordingBatch>> {
        let mut batches = Vec::new();
        let lines = std::mem::take(&mut self.lines);
        self.raw_bytes = 0;
        self.started = Instant::now();
        self.encode(&lines, &mut batches)?;
        Ok(batches)
    }

    pub fn finish(&mut self) -> Result<Vec<RecordingBatch>> {
        ensure!(!self.finished, "recording stream finished");
        self.finished = true;
        let pieces = self.redactor.finish();
        self.apply(pieces)?;
        let mut batches = Vec::new();
        self.drain_ready(&mut batches)?;
        ensure!(self.pending.is_empty(), "unresolved recording prefix");
        batches.extend(self.flush()?);
        Ok(batches)
    }

    fn apply(&mut self, pieces: Vec<Redacted>) -> Result<()> {
        for piece in pieces {
            let pending = self
                .pending
                .iter_mut()
                .find(|p| p.record.seq.0 == piece.source)
                .ok_or_else(|| anyhow::anyhow!("unknown redaction source"))?;
            ensure!(
                pending.unresolved >= piece.consumed,
                "redaction accounting mismatch"
            );
            pending.unresolved -= piece.consumed;
            pending.redacted.extend(piece.output);
        }
        Ok(())
    }

    fn drain_ready(&mut self, batches: &mut Vec<RecordingBatch>) -> Result<()> {
        while self.pending.front().is_some_and(|p| p.unresolved == 0) {
            let pending = self.pending.pop_front().expect("ready record");
            let record = pending.record;
            let count = pending.redacted.len().div_ceil(FRAGMENT_BYTES).max(1);
            for index in 0..count {
                let start = (index * FRAGMENT_BYTES).min(pending.redacted.len());
                let end = ((index + 1) * FRAGMENT_BYTES).min(pending.redacted.len());
                let data = STANDARD.encode(&pending.redacted[start..end]);
                let content = match &record.content {
                    CaptureContent::Output(_) => Content::Output {
                        data,
                        fragment: index as u32,
                        last: index + 1 == count,
                    },
                    CaptureContent::AgentText { channel, .. } => Content::AgentText {
                        channel: *channel,
                        data,
                        fragment: index as u32,
                        last: index + 1 == count,
                    },
                    CaptureContent::Resize { rows, cols } => Content::Resize {
                        rows: *rows,
                        cols: *cols,
                    },
                };
                let encoded = Record {
                    v: 1,
                    seq: record.seq,
                    captured_at: record.captured_at.clone(),
                    elapsed_ms: record.elapsed_ms,
                    content,
                };
                let mut data = serde_json::to_vec(&encoded)?;
                data.push(b'\n');
                // Never make a later capture month extend earlier content's retention age.
                let month_changed = self.lines.last().is_some_and(|line| {
                    line.time.as_str()[..7] != record.captured_at.as_str()[..7]
                });
                if month_changed || self.raw_bytes + data.len() > RAW_LIMIT {
                    batches.extend(self.flush()?);
                }
                self.raw_bytes += data.len();
                self.lines.push(Line {
                    seq: record.seq,
                    time: record.captured_at.clone(),
                    data,
                });
            }
        }
        Ok(())
    }

    fn compressed_size(&self) -> Result<usize> {
        let data: Vec<_> = self
            .lines
            .iter()
            .flat_map(|l| l.data.iter().copied())
            .collect();
        Ok(zstd::bulk::compress(&data, 3)?.len())
    }
    fn encode(&self, lines: &[Line], batches: &mut Vec<RecordingBatch>) -> Result<()> {
        if lines.is_empty() {
            return Ok(());
        }
        let data: Vec<_> = lines.iter().flat_map(|l| l.data.iter().copied()).collect();
        ensure!(
            data.len() <= RAW_LIMIT,
            "recording raw chunk exceeds budget"
        );
        let compressed = zstd::bulk::compress(&data, 3)?;
        if compressed.len() > COMPRESSED_LIMIT {
            ensure!(
                lines.len() > 1,
                "recording fragment exceeds compressed budget"
            );
            let middle = lines.len() / 2;
            self.encode(&lines[..middle], batches)?;
            return self.encode(&lines[middle..], batches);
        }
        let batch = RecordingBatch {
            id: ChunkId::new(),
            session: self.identity.session,
            machine: self.identity.machine,
            stream: self.identity.stream.clone(),
            stream_epoch: self.identity.stream_epoch,
            from: lines.first().unwrap().seq,
            to: lines.last().unwrap().seq,
            captured_from: lines.iter().map(|l| &l.time).min().unwrap().clone(),
            captured_to: lines.iter().map(|l| &l.time).max().unwrap().clone(),
            codec: RecordingCodec::ZstdJsonl,
            uncompressed_bytes: data.len() as u32,
            digest: format!("{:x}", Sha256::digest(&compressed)),
            data: STANDARD.encode(compressed),
        };
        batch.validate()?;
        batches.push(batch);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
