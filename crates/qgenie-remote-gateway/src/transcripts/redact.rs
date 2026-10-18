use super::ValidatedRecording;
use crate::Error;
use aho_corasick::AhoCorasick;
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{RecordedContent, wire::RecordingBatch};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use zeroize::{Zeroize, Zeroizing};

/// Standalone literal writer policy, not verified corporate redactor compatibility.
/// Receipts identify uploaded bytes; stored redacted bytes have their own hash.
#[derive(Clone)]
pub struct RecordingRedaction {
    patterns: Arc<Vec<Zeroizing<Vec<u8>>>>,
    matcher: Arc<AhoCorasick>,
    work: Arc<tokio::sync::Semaphore>,
}
pub struct ProtectedRecording {
    pub(super) batch: RecordingBatch,
    pub(super) payload: Vec<u8>,
    pub(super) stored_digest: String,
    pub(super) redaction_saturated: bool,
    pub(super) uncompressed_bytes: u32,
    pub(super) fragments: serde_json::Value,
}
impl ProtectedRecording {
    /// Original immutable metadata/digest. Unprotected upload bytes have been cleared.
    pub fn original_metadata(&self) -> &RecordingBatch {
        &self.batch
    }
    pub fn stored_digest(&self) -> &str {
        &self.stored_digest
    }
    pub fn redaction_saturated(&self) -> bool {
        self.redaction_saturated
    }
    pub fn stored_uncompressed_bytes(&self) -> u32 {
        self.uncompressed_bytes
    }
    pub fn stored_compressed_bytes(&self) -> usize {
        self.payload.len()
    }
}
impl RecordingRedaction {
    /// Explicit bounded sample literal policy. Never log or serialize its contents.
    pub fn from_file(path: &std::path::Path) -> Result<Self, Error> {
        use std::io::Read;
        let metadata = std::fs::symlink_metadata(path).map_err(|_| Error::Configuration)?;
        if !metadata.is_file() || metadata.len() > 32 * 1024 {
            return Err(Error::Configuration);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o007 != 0 {
                return Err(Error::Configuration);
            }
        }
        let mut bytes = Zeroizing::new(Vec::new());
        std::fs::File::open(path)
            .map_err(|_| Error::Configuration)?
            .take(32 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Configuration)?;
        if bytes.len() > 32 * 1024 {
            return Err(Error::Configuration);
        }
        let patterns: Vec<String> =
            qgenie_remote_proto::decode_json(&bytes).map_err(|_| Error::Configuration)?;
        Self::literals(patterns.into_iter().map(String::into_bytes).collect())
    }
    pub fn literals(patterns: Vec<Vec<u8>>) -> Result<Self, Error> {
        let mut patterns: Vec<_> = patterns.into_iter().map(Zeroizing::new).collect();
        if patterns.len() > 128
            || patterns.iter().any(|p| p.is_empty() || p.len() > 256)
            || patterns.iter().map(|p| p.len()).sum::<usize>() > 8192
        {
            return Err(Error::Configuration);
        }
        patterns.sort_by(|a, b| a.as_slice().cmp(b.as_slice()));
        patterns.dedup_by(|a, b| a.as_slice() == b.as_slice());
        let matcher = AhoCorasick::new(patterns.iter().map(|p| p.as_slice()))
            .map_err(|_| Error::Configuration)?;
        Ok(Self {
            patterns: Arc::new(patterns),
            matcher: Arc::new(matcher),
            work: Arc::new(tokio::sync::Semaphore::new(16)),
        })
    }
    /// Mask possible chunk-edge prefixes/suffixes without waiting for another upload. This
    /// backstop complements continuous host redaction without rewriting immutable chunks.
    pub async fn protect(
        &self,
        validated: ValidatedRecording,
    ) -> Result<ProtectedRecording, Error> {
        let permit = self
            .work
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Overloaded)?;
        let policy = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            policy.apply(validated)
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }
    fn apply(&self, mut validated: ValidatedRecording) -> Result<ProtectedRecording, Error> {
        let fragments: Vec<_> = validated
            .records
            .iter()
            .map(|record| {
                let (part, last) = match &record.content {
                    RecordedContent::Output { fragment, last, .. }
                    | RecordedContent::AgentText { fragment, last, .. } => (*fragment, *last),
                    _ => (0, true),
                };
                serde_json::json!({"seq":record.seq,"fragment":part,"last":last})
            })
            .collect();
        let mut bytes = Zeroizing::new(Vec::new());
        let mut lengths = Vec::new();
        for record in &validated.records {
            let data = match &record.content {
                RecordedContent::Output { data, .. } | RecordedContent::AgentText { data, .. } => {
                    Some(data)
                }
                _ => None,
            };
            if let Some(data) = data {
                let content = STANDARD.decode(data).map_err(|_| Error::Invalid)?;
                lengths.push(content.len());
                bytes.extend(content);
            }
        }
        let mut masked = vec![false; bytes.len()];
        let mut work = 0usize;
        let mut saturated = false;
        for found in self.matcher.find_overlapping_iter(bytes.as_slice()) {
            work = work.saturating_add(found.len());
            if work > 4 * 1024 * 1024 {
                masked.fill(true);
                saturated = true;
                break;
            }
            masked[found.start()..found.end()].fill(true);
        }
        for pattern in self.patterns.iter() {
            for size in 1..pattern.len().min(bytes.len() + 1) {
                if bytes[..size] == pattern[pattern.len() - size..] {
                    masked[..size].fill(true);
                }
                if bytes[bytes.len() - size..] == pattern[..size] {
                    masked[bytes.len() - size..].fill(true);
                }
            }
        }
        for (index, mask) in masked.into_iter().enumerate() {
            if mask {
                bytes[index] = b'*';
            }
        }
        let mut offset = 0;
        let mut lengths = lengths.into_iter();
        for record in &mut validated.records {
            let data = match &mut record.content {
                RecordedContent::Output { data, .. } | RecordedContent::AgentText { data, .. } => {
                    Some(data)
                }
                _ => None,
            };
            if let Some(data) = data {
                let count = lengths.next().ok_or(Error::Invalid)?;
                *data = STANDARD.encode(&bytes[offset..offset + count]);
                offset += count;
            }
        }
        let mut raw = Vec::new();
        for record in validated.records {
            serde_json::to_writer(&mut raw, &record).map_err(|_| Error::Invalid)?;
            raw.push(b'\n');
        }
        if raw.len() > 1024 * 1024 {
            return Err(Error::Invalid);
        }
        let payload = zstd::bulk::compress(&raw, 3).map_err(|_| Error::Unavailable)?;
        if payload.len() > 64 * 1024 {
            return Err(Error::Overloaded);
        }
        let stored_digest = format!("{:x}", Sha256::digest(&payload));
        validated.batch.data.zeroize();
        validated.batch.data.clear();
        Ok(ProtectedRecording {
            batch: validated.batch,
            payload,
            stored_digest,
            redaction_saturated: saturated,
            uncompressed_bytes: raw.len() as u32,
            fragments: serde_json::Value::Array(fragments),
        })
    }
}

#[cfg(test)]
mod tests;
