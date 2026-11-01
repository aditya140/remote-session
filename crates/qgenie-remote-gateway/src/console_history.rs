//! Browser-readable history uses the same protected readers and committed read audits.
//! Decode permits remain with blocking work if its HTTP caller disconnects.
use crate::{Error, Gateway};
use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{
    ChunkId, Counter, MachineId, RecordedContent, RecordingEpochId, RecordingRecord, SessionId,
    decode_json,
    rest::{AdminHistoryQuery, HistoricalChunk, HistoryItem, HistoryPage, HistoryQuery},
    wire::{LossRange, RecordingCodec},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

const MAX_CHUNKS: usize = 4;
const MAX_COMPRESSED: usize = 64 * 1024;
const MAX_UNCOMPRESSED: usize = 1024 * 1024;
// At most two pages, each <=4 MiB of normalized JSONL, can be decoded at once.
static DECODERS: Semaphore = Semaphore::const_new(2);

pub(crate) fn routes() -> Router<Gateway> {
    Router::new()
        .route("/v1/console/sessions/{session}/history", post(owner))
        .route("/v1/console/admin/sessions/{session}/history", post(admin))
}

#[derive(Serialize)]
struct BrowserHistory {
    session: SessionId,
    machine: MachineId,
    stream_epoch: RecordingEpochId,
    items: Vec<BrowserItem>,
    next: Option<String>,
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BrowserItem {
    Content {
        id: ChunkId,
        from: Counter,
        to: Counter,
        redaction_saturated: bool,
        records: Vec<RecordingRecord>,
    },
    Gap {
        gap: LossRange,
    },
}

async fn owner(
    State(gateway): State<Gateway>,
    Path(session): Path<SessionId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let actor = gateway.authenticate_client(&headers).await?;
    let query: HistoryQuery = decode_json(&body).map_err(|_| Error::Invalid)?;
    query.validate().map_err(|_| Error::Invalid)?;
    let permit = DECODERS.try_acquire().map_err(|_| Error::Overloaded)?;
    let identity = gateway.identity.as_deref().ok_or(Error::Configuration)?;
    let recordings = gateway.recordings.as_ref().ok_or(Error::Configuration)?;
    // This reader enforces current owner scope through the isolated owner pool.
    // Privileged admin reads use the separately audited path below.
    let page = recordings
        .owner
        .owner_history(identity, &actor, session, query)
        .await?;
    decoded_response(page, permit).await
}
async fn admin(
    State(gateway): State<Gateway>,
    Path(session): Path<SessionId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let actor = gateway.authenticate_client(&headers).await?;
    if !actor.platform_admin {
        return Err(Error::Forbidden);
    }
    let query: AdminHistoryQuery = decode_json(&body).map_err(|_| Error::Invalid)?;
    query.history.validate().map_err(|_| Error::Invalid)?;
    let permit = DECODERS.try_acquire().map_err(|_| Error::Overloaded)?;
    let identity = gateway.identity.as_deref().ok_or(Error::Configuration)?;
    let recordings = gateway.recordings.as_ref().ok_or(Error::Configuration)?;
    // The existing reader enforces reason, held-content rules, current authority,
    // dedicated admin RLS credentials, and the atomic committed read audit.
    let page = recordings
        .admin
        .admin_history(identity, &actor, session, query)
        .await?;
    decoded_response(page, permit).await
}
async fn decoded_response(
    page: HistoryPage,
    permit: tokio::sync::SemaphorePermit<'static>,
) -> Result<Response, Error> {
    let bytes = tokio::task::spawn_blocking(move || {
        let page = decode_page(page)?;
        let bytes = serde_json::to_vec(&page).map_err(|_| Error::Invalid)?;
        Ok::<_, Error>(HistoryBytes {
            bytes,
            _permit: permit,
        })
    })
    .await
    .map_err(|_| Error::Unavailable)??;
    // The backing Bytes owner holds the budget through HTTP delivery or cancellation.
    let mut response = Bytes::from_owner(bytes).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response.headers_mut().insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}
struct HistoryBytes {
    bytes: Vec<u8>,
    _permit: tokio::sync::SemaphorePermit<'static>,
}
impl AsRef<[u8]> for HistoryBytes {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
fn decode_page(page: HistoryPage) -> Result<BrowserHistory, Error> {
    if page.items.len() > MAX_CHUNKS {
        return Err(Error::Invalid);
    }
    let mut items = Vec::with_capacity(page.items.len());
    for item in page.items {
        match item {
            HistoryItem::Content(chunk) => {
                if chunk.session != page.session
                    || chunk.machine != page.machine
                    || chunk.stream_epoch != page.stream_epoch
                {
                    return Err(Error::Invalid);
                }
                let id = chunk.id;
                let from = chunk.from;
                let to = chunk.to;
                let redaction_saturated = chunk.redaction_saturated;
                items.push(BrowserItem::Content {
                    id,
                    from,
                    to,
                    redaction_saturated,
                    records: decode_chunk(chunk)?,
                });
            }
            HistoryItem::Gap(gap) => {
                if gap.session != page.session
                    || gap.stream_epoch != page.stream_epoch
                    || gap.from.0 == 0
                    || gap.to < gap.from
                {
                    return Err(Error::Invalid);
                }
                items.push(BrowserItem::Gap { gap });
            }
        }
    }
    Ok(BrowserHistory {
        session: page.session,
        machine: page.machine,
        stream_epoch: page.stream_epoch,
        items,
        next: page.next,
    })
}
fn decode_chunk(chunk: HistoricalChunk) -> Result<Vec<RecordingRecord>, Error> {
    if !matches!(chunk.codec, RecordingCodec::ZstdJsonl)
        || chunk.uncompressed_bytes == 0
        || chunk.uncompressed_bytes as usize > MAX_UNCOMPRESSED
        || chunk.data.len() > MAX_COMPRESSED.div_ceil(3) * 4
    {
        return Err(Error::Invalid);
    }
    let bytes = STANDARD.decode(&chunk.data).map_err(|_| Error::Invalid)?;
    if bytes.len() > MAX_COMPRESSED
        || format!("{:x}", Sha256::digest(&bytes)) != chunk.stored_digest
        || zstd::zstd_safe::find_frame_compressed_size(&bytes).map_err(|_| Error::Invalid)?
            != bytes.len()
    {
        return Err(Error::Invalid);
    }
    let raw = zstd::bulk::decompress(&bytes, MAX_UNCOMPRESSED).map_err(|_| Error::Invalid)?;
    if raw.len() != chunk.uncompressed_bytes as usize || raw.last() != Some(&b'\n') {
        return Err(Error::Invalid);
    }
    let mut records = Vec::new();
    let mut previous: Option<(Counter, u32, bool)> = None;
    for line in raw[..raw.len() - 1].split(|b| *b == b'\n') {
        if line.is_empty() || line.len() > 32 * 1024 || records.len() >= 4096 {
            return Err(Error::Invalid);
        }
        let record: RecordingRecord = decode_json(line).map_err(|_| Error::Invalid)?;
        record.validate(&chunk.stream).map_err(|_| Error::Invalid)?;
        let (fragment, last) = match &record.content {
            RecordedContent::Output { fragment, last, .. }
            | RecordedContent::AgentText { fragment, last, .. } => (*fragment, *last),
            _ => (0, true),
        };
        if let Some((seq, part, complete)) = previous
            && ((complete && (seq.0.checked_add(1) != Some(record.seq.0) || fragment != 0))
                || (!complete && (record.seq != seq || part.checked_add(1) != Some(fragment))))
        {
            return Err(Error::Invalid);
        }
        previous = Some((record.seq, fragment, last));
        if record.seq < chunk.from
            || record.seq > chunk.to
            || record.captured_at < chunk.captured_from
            || record.captured_at > chunk.captured_to
        {
            return Err(Error::Invalid);
        }
        records.push(record);
    }
    if records.first().map(|r| r.seq) != Some(chunk.from)
        || records.last().map(|r| r.seq) != Some(chunk.to)
        || records.iter().map(|r| &r.captured_at).min() != Some(&chunk.captured_from)
        || records.iter().map(|r| &r.captured_at).max() != Some(&chunk.captured_to)
    {
        return Err(Error::Invalid);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qgenie_remote_proto::wire::{RecordingStream, Timestamp};
    fn chunk() -> HistoricalChunk {
        let at = Timestamp::from_unix_millis(1000).unwrap();
        let record = RecordingRecord {
            v: 1,
            seq: Counter(1),
            captured_at: at.clone(),
            elapsed_ms: Counter(1),
            content: RecordedContent::Output {
                data: STANDARD.encode(b"visible output\n"),
                fragment: 0,
                last: true,
            },
        };
        let mut raw = serde_json::to_vec(&record).unwrap();
        raw.push(b'\n');
        let compressed = zstd::bulk::compress(&raw, 1).unwrap();
        HistoricalChunk {
            id: ChunkId::new(),
            session: SessionId::new(),
            machine: MachineId::new(),
            stream: RecordingStream::Pty,
            stream_epoch: RecordingEpochId::new(),
            from: Counter(1),
            to: Counter(1),
            captured_from: at.clone(),
            captured_to: at,
            codec: RecordingCodec::ZstdJsonl,
            uncompressed_bytes: raw.len() as u32,
            stored_digest: format!("{:x}", Sha256::digest(&compressed)),
            data: STANDARD.encode(compressed),
            redaction_saturated: false,
        }
    }
    #[test]
    fn integrity_bounds_and_stream_identity_are_checked_before_rendering() {
        let good = chunk();
        let records = decode_chunk(good.clone()).unwrap();
        assert_eq!(records.len(), 1);
        for bad in [
            {
                let mut c = good.clone();
                c.stored_digest = "0".repeat(64);
                c
            },
            {
                let mut c = good.clone();
                c.uncompressed_bytes += 1;
                c
            },
            {
                let mut c = good.clone();
                c.uncompressed_bytes = (MAX_UNCOMPRESSED + 1) as u32;
                c
            },
            {
                let mut c = good.clone();
                c.data = "!".into();
                c
            },
            {
                let mut c = good.clone();
                c.stream = RecordingStream::Events;
                c
            },
            {
                let mut c = good.clone();
                c.to = Counter(2);
                c
            },
            {
                let mut c = good.clone();
                let mut bytes = STANDARD.decode(&c.data).unwrap();
                bytes.extend(bytes.clone());
                c.data = STANDARD.encode(&bytes);
                c.stored_digest = format!("{:x}", Sha256::digest(&bytes));
                c
            },
        ] {
            assert!(decode_chunk(bad).is_err());
        }
        let page = HistoryPage {
            session: SessionId::new(),
            machine: good.machine,
            stream_epoch: good.stream_epoch,
            items: vec![HistoryItem::Content(good)],
            next: None,
        };
        assert!(decode_page(page).is_err());
    }
    #[test]
    fn response_byte_budget_survives_clones_until_delivery_or_cancel() {
        static BUDGET: Semaphore = Semaphore::const_new(1);
        let payload = Bytes::from_owner(HistoryBytes {
            bytes: b"{}".to_vec(),
            _permit: BUDGET.try_acquire().unwrap(),
        });
        let queued = payload.clone();
        drop(payload);
        assert_eq!(BUDGET.available_permits(), 0);
        assert_eq!(queued.as_ref(), b"{}");
        drop(queued);
        assert_eq!(BUDGET.available_permits(), 1);
    }
    #[test]
    fn pages_preserve_loss_and_reject_scope_or_item_limit_violations() {
        use qgenie_remote_proto::{GapId, wire::LossReason};
        let c = chunk();
        let gap = LossRange {
            id: GapId::new(),
            session: c.session,
            stream: RecordingStream::Pty,
            from: Counter(2),
            to: Counter(3),
            stream_epoch: c.stream_epoch,
            captured_from: c.captured_from.clone(),
            captured_to: c.captured_to.clone(),
            reason: LossReason::CaptureBackpressure,
        };
        let page = HistoryPage {
            session: c.session,
            machine: c.machine,
            stream_epoch: c.stream_epoch,
            items: vec![
                HistoryItem::Content(c.clone()),
                HistoryItem::Gap(gap.clone()),
            ],
            next: Some("opaque-cursor".into()),
        };
        let rendered = serde_json::to_value(decode_page(page).unwrap()).unwrap();
        assert_eq!(rendered["items"][1]["kind"], "gap");
        assert_eq!(
            rendered["items"][1]["gap"]["reason"],
            "capture_backpressure"
        );
        assert_eq!(rendered["next"], "opaque-cursor");
        let oversized = HistoryPage {
            session: c.session,
            machine: c.machine,
            stream_epoch: c.stream_epoch,
            items: vec![HistoryItem::Content(c.clone()); MAX_CHUNKS + 1],
            next: None,
        };
        assert!(decode_page(oversized).is_err());
        let mut wrong = gap;
        wrong.session = SessionId::new();
        assert!(
            decode_page(HistoryPage {
                session: c.session,
                machine: c.machine,
                stream_epoch: c.stream_epoch,
                items: vec![HistoryItem::Gap(wrong)],
                next: None
            })
            .is_err()
        );
    }
    #[test]
    fn normalized_jsonl_rejects_duplicate_sequences_and_missing_newline() {
        let good = chunk();
        let raw = zstd::bulk::decompress(&STANDARD.decode(&good.data).unwrap(), MAX_UNCOMPRESSED)
            .unwrap();
        for raw in [raw[..raw.len() - 1].to_vec(), [raw.clone(), raw].concat()] {
            let mut c = good.clone();
            let bytes = zstd::bulk::compress(&raw, 1).unwrap();
            c.uncompressed_bytes = raw.len() as u32;
            c.stored_digest = format!("{:x}", Sha256::digest(&bytes));
            c.data = STANDARD.encode(bytes);
            assert!(decode_chunk(c).is_err());
        }
    }
}
