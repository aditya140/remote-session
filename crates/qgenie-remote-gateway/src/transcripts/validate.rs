use crate::Error;
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{
    Counter, RecordedContent, RecordingRecord,
    wire::{RecordingBatch, RecordingCodec, Timestamp},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Bounded CPU preparation, not an ingest authorization or reclaimable receipt.
#[derive(Clone)]
pub struct RecordingValidation {
    work: Arc<tokio::sync::Semaphore>,
}
pub struct ValidatedRecording {
    pub(crate) batch: RecordingBatch,
    pub(crate) records: Vec<RecordingRecord>,
}
impl Default for RecordingValidation {
    fn default() -> Self {
        Self {
            work: Arc::new(tokio::sync::Semaphore::new(16)),
        }
    }
}
impl ValidatedRecording {
    pub fn batch(&self) -> &RecordingBatch {
        &self.batch
    }
    pub fn records(&self) -> &[RecordingRecord] {
        &self.records
    }
}
impl RecordingValidation {
    /// Original content time is never replaced with upload time. SQL must additionally bind the
    /// authenticated session/machine/owner and verify its immutable lifetime on every ingest.
    pub async fn prepare(
        &self,
        batch: RecordingBatch,
        now: Timestamp,
        session_started: Timestamp,
        session_ended: Option<Timestamp>,
    ) -> Result<ValidatedRecording, Error> {
        batch.validate().map_err(|_| Error::Invalid)?;
        if !matches!(batch.codec, RecordingCodec::ZstdJsonl) {
            return Err(Error::Invalid);
        }
        let permit = self
            .work
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Overloaded)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            decode(batch, now, session_started, session_ended)
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }
}

fn decode(
    batch: RecordingBatch,
    now: Timestamp,
    started: Timestamp,
    ended: Option<Timestamp>,
) -> Result<ValidatedRecording, Error> {
    let invalid = || Error::Invalid;
    // Five-minute clock skew is explicit. Delayed old batches remain old, including after retry.
    let first = batch.captured_from.unix_millis();
    let last = batch.captured_to.unix_millis();
    if last > now.unix_millis().saturating_add(300_000)
        || first < started.unix_millis().saturating_sub(300_000)
        || ended.is_some_and(|end| last > end.unix_millis().saturating_add(300_000))
    {
        return Err(invalid());
    }
    let compressed = STANDARD.decode(&batch.data).map_err(|_| invalid())?;
    if format!("{:x}", Sha256::digest(&compressed)) != batch.digest {
        return Err(invalid());
    }
    if zstd::zstd_safe::find_frame_compressed_size(&compressed).map_err(|_| invalid())?
        != compressed.len()
    {
        return Err(invalid());
    }
    let raw = zstd::bulk::decompress(&compressed, 1024 * 1024).map_err(|_| invalid())?;
    if raw.len() != batch.uncompressed_bytes as usize || raw.last() != Some(&b'\n') {
        return Err(invalid());
    }
    let mut records: Vec<RecordingRecord> = Vec::new();
    let mut previous: Option<(Counter, u32, bool)> = None;
    for line in raw[..raw.len() - 1].split(|b| *b == b'\n') {
        if line.is_empty() || line.len() > 32 * 1024 || records.len() >= 4096 {
            return Err(invalid());
        }
        let record: qgenie_remote_proto::RecordingRecord =
            qgenie_remote_proto::decode_json(line).map_err(|_| invalid())?;
        record.validate(&batch.stream).map_err(|_| invalid())?;
        let (fragment, complete) = match &record.content {
            RecordedContent::Output { fragment, last, .. }
            | RecordedContent::AgentText { fragment, last, .. } => (*fragment, *last),
            RecordedContent::Resize { .. } => (0, true),
        };
        if let Some((seq, part, last)) = previous {
            if last {
                if seq.0.checked_add(1) != Some(record.seq.0) || fragment != 0 {
                    return Err(invalid());
                }
            } else if record.seq != seq || part.checked_add(1) != Some(fragment) {
                return Err(invalid());
            }
        }
        // A chunk may start/end inside one identified record. The ingest stream ledger must
        // verify those cross-batch fragments; silently accepting them as full records is unsafe.
        previous = Some((record.seq, fragment, complete));
        records.push(record);
    }
    if records.first().map(|r| r.seq) != Some(batch.from)
        || records.last().map(|r| r.seq) != Some(batch.to)
        || records.iter().map(|r| &r.captured_at).min() != Some(&batch.captured_from)
        || records.iter().map(|r| &r.captured_at).max() != Some(&batch.captured_to)
    {
        return Err(invalid());
    }
    Ok(ValidatedRecording { batch, records })
}

#[cfg(test)]
mod tests;
