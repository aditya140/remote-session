use super::*;
use qgenie_remote_proto::wire::{Envelope, Message};
fn batch() -> RecordingBatch {
    let Envelope {
        message: Message::RecordingIngest(batch),
        ..
    } = Envelope::decode(include_bytes!(
        "../../../../../conformance/json/network-recording.json"
    ))
    .unwrap()
    else {
        panic!("recording");
    };
    batch
}
fn time(text: &str) -> Timestamp {
    text.to_owned().try_into().unwrap()
}
fn rewrite(batch: &mut RecordingBatch, raw: &[u8]) {
    let compressed = zstd::bulk::compress(raw, 3).unwrap();
    batch.uncompressed_bytes = raw.len() as u32;
    batch.digest = format!("{:x}", Sha256::digest(&compressed));
    batch.data = STANDARD.encode(compressed);
}

#[tokio::test]
async fn normalized_content_hash_and_original_lifetime_bounds_are_checked_before_storage() {
    let validation = RecordingValidation::default();
    let batch = batch();
    let start = time("2026-10-17T00:00:00.000Z");
    let now = time("2026-10-17T00:00:01.000Z");
    let result = validation
        .prepare(batch.clone(), now.clone(), start.clone(), None)
        .await
        .unwrap();
    assert_eq!(result.records().len(), 1);
    assert_eq!(result.batch().captured_from, start);
    let mut forged = batch.clone();
    forged.digest = "00".repeat(32);
    assert!(
        validation
            .prepare(forged, now.clone(), start.clone(), None)
            .await
            .is_err()
    );
    assert!(
        validation
            .prepare(
                batch.clone(),
                time("2026-10-16T23:54:59.999Z"),
                start.clone(),
                None
            )
            .await
            .is_err()
    );
    assert!(
        validation
            .prepare(
                batch.clone(),
                now.clone(),
                time("2026-10-17T00:05:00.001Z"),
                None
            )
            .await
            .is_err()
    );
    assert!(
        validation
            .prepare(
                batch.clone(),
                now.clone(),
                time("2026-10-16T00:00:00.000Z"),
                Some(time("2026-10-16T23:54:59.999Z"))
            )
            .await
            .is_err()
    );
    let delayed = validation
        .prepare(batch, time("2027-02-17T00:00:00.000Z"), start.clone(), None)
        .await
        .unwrap();
    assert_eq!(delayed.batch().captured_from, start);
}

#[tokio::test]
async fn decompression_bombs_duplicate_fields_forged_order_and_missing_fragments_fail() {
    let validation = RecordingValidation::default();
    let now = time("2026-10-17T00:00:01.000Z");
    let start = time("2026-10-17T00:00:00.000Z");
    let mut bomb = batch();
    let compressed = zstd::bulk::compress(&vec![b'a'; 1024 * 1024 + 1], 3).unwrap();
    bomb.data = STANDARD.encode(&compressed);
    bomb.digest = format!("{:x}", Sha256::digest(compressed));
    bomb.uncompressed_bytes = 1024 * 1024;
    assert!(
        validation
            .prepare(bomb, now.clone(), start.clone(), None)
            .await
            .is_err()
    );
    let raw = include_bytes!("../../../../../conformance/recording/event.jsonl");
    for mutation in [
        String::from_utf8(raw.to_vec())
            .unwrap()
            .replace("\"v\":1", "\"v\":1,\"v\":1"),
        String::from_utf8(raw.to_vec())
            .unwrap()
            .replace("agent_text", "input"),
    ] {
        let mut b = batch();
        rewrite(&mut b, mutation.as_bytes());
        assert!(
            validation
                .prepare(b, now.clone(), start.clone(), None)
                .await
                .is_err()
        );
    }
    let mut b = batch();
    let two = [raw.as_slice(), raw.as_slice()].concat();
    rewrite(&mut b, &two);
    assert!(
        validation
            .prepare(b, now.clone(), start.clone(), None)
            .await
            .is_err()
    );
    let mut b = batch();
    let mut first: RecordingRecord = serde_json::from_slice(raw).unwrap();
    let mut second = first.clone();
    if let RecordedContent::AgentText { last, .. } = &mut first.content {
        *last = false;
    }
    if let RecordedContent::AgentText { fragment, .. } = &mut second.content {
        *fragment = 2;
    }
    let raw = format!(
        "{}\n{}\n",
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    rewrite(&mut b, raw.as_bytes());
    assert!(validation.prepare(b, now, start, None).await.is_err());
}

#[tokio::test]
async fn cpu_preparation_refuses_work_when_all_real_permits_are_held() {
    let validation = RecordingValidation::default();
    let permits = validation
        .work
        .clone()
        .acquire_many_owned(16)
        .await
        .unwrap();
    assert!(matches!(
        validation
            .prepare(
                batch(),
                time("2026-10-17T00:00:01.000Z"),
                time("2026-10-17T00:00:00.000Z"),
                None
            )
            .await,
        Err(Error::Overloaded)
    ));
    drop(permits);
    validation
        .prepare(
            batch(),
            time("2026-10-17T00:00:01.000Z"),
            time("2026-10-17T00:00:00.000Z"),
            None,
        )
        .await
        .unwrap();
}
