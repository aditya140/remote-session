use super::*;
use qgenie_remote_proto::{
    Counter, RecordingRecord,
    wire::{Envelope, Message, RecordingStream, Timestamp},
};

fn fixture(bytes: &[&[u8]]) -> ValidatedRecording {
    let Envelope {
        message: Message::RecordingIngest(mut batch),
        ..
    } = Envelope::decode(include_bytes!(
        "../../../../../conformance/json/network-recording.json"
    ))
    .unwrap()
    else {
        panic!("fixture");
    };
    batch.stream = RecordingStream::Pty;
    let records: Vec<_> = bytes
        .iter()
        .enumerate()
        .map(|(n, bytes)| RecordingRecord {
            v: 1,
            seq: Counter(n as u64 + 1),
            captured_at: Timestamp::from_unix_millis(1000).unwrap(),
            elapsed_ms: Counter(0),
            content: RecordedContent::Output {
                data: STANDARD.encode(bytes),
                fragment: 0,
                last: true,
            },
        })
        .collect();
    let mut raw = Vec::new();
    for record in &records {
        serde_json::to_writer(&mut raw, record).unwrap();
        raw.push(b'\n');
    }
    let compressed = zstd::bulk::compress(&raw, 3).unwrap();
    batch.from = Counter(1);
    batch.to = Counter(records.len() as u64);
    batch.captured_from = Timestamp::from_unix_millis(1000).unwrap();
    batch.captured_to = batch.captured_from.clone();
    batch.digest = format!("{:x}", Sha256::digest(&compressed));
    batch.data = STANDARD.encode(compressed);
    batch.uncompressed_bytes = raw.len() as u32;
    ValidatedRecording { batch, records }
}
fn output(recording: &ProtectedRecording) -> Vec<u8> {
    let raw = zstd::bulk::decompress(&recording.payload, 1024 * 1024).unwrap();
    assert_eq!(raw.len(), recording.uncompressed_bytes as usize);
    assert_eq!(
        format!("{:x}", Sha256::digest(&recording.payload)),
        recording.stored_digest
    );
    raw.split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .flat_map(|line| {
            let record: RecordingRecord = serde_json::from_slice(line).unwrap();
            match record.content {
                RecordedContent::Output { data, .. } => STANDARD.decode(data).unwrap(),
                _ => vec![],
            }
        })
        .collect()
}
#[tokio::test]
async fn known_literals_are_masked_across_fields_and_all_immutable_chunk_edges() {
    let policy = RecordingRedaction::literals(vec![b"private-secret".to_vec()]).unwrap();
    for cut in 1..b"private-secret".len() {
        let left = [b"before ".as_slice(), &b"private-secret"[..cut]].concat();
        let right = [&b"private-secret"[cut..], b" after\xff"].concat();
        let together = policy.protect(fixture(&[&left, &right])).await.unwrap();
        assert_eq!(output(&together), b"before ************** after\xff");
        let a = fixture(&[&left]);
        let digest = a.batch.digest.clone();
        let a = policy.protect(a).await.unwrap();
        assert_eq!(a.batch.digest, digest);
        let b = policy.protect(fixture(&[&right])).await.unwrap();
        assert_eq!(
            [output(&a), output(&b)].concat(),
            b"before ************** after\xff"
        );
    }
}
#[tokio::test]
async fn overlapping_literal_work_is_bounded_by_explicit_full_masking() {
    let policy = RecordingRedaction::literals((1..=100).map(|n| vec![b'a'; n]).collect()).unwrap();
    let full = vec![b'a'; 16 * 1024];
    let protected = policy.protect(fixture(&[&full])).await.unwrap();
    assert!(protected.redaction_saturated());
    assert_eq!(output(&protected), vec![b'*'; full.len()]);
    assert_eq!(
        protected.original_metadata().stream_epoch,
        fixture(&[b""]).batch.stream_epoch
    ); // stable fixture epoch
    assert!(protected.original_metadata().data.is_empty());
}
#[tokio::test]
async fn empty_policy_preserves_binary_but_does_not_bypass_actual_cpu_limits() {
    let policy = RecordingRedaction::literals(vec![]).unwrap();
    assert_eq!(
        output(
            &policy
                .protect(fixture(&[b"\xff\0 arbitrary"]))
                .await
                .unwrap()
        ),
        b"\xff\0 arbitrary"
    );
    let permits = policy.work.clone().acquire_many_owned(16).await.unwrap();
    assert!(matches!(
        policy.protect(fixture(&[b"data"])).await,
        Err(Error::Overloaded)
    ));
    drop(permits);
    assert!(RecordingRedaction::literals(vec![vec![]]).is_err());
    assert!(RecordingRedaction::literals(vec![vec![1; 257]]).is_err());
}

#[tokio::test]
async fn validated_upload_is_sealed_with_redacted_payload_and_original_receipt_identity() {
    let original = fixture(&[b"before private-secret after"]);
    let batch = original.batch;
    let digest = batch.digest.clone();
    let validated = crate::RecordingValidation::default()
        .prepare(
            batch,
            Timestamp::from_unix_millis(1000).unwrap(),
            Timestamp::from_unix_millis(1000).unwrap(),
            None,
        )
        .await
        .unwrap();
    let protected = RecordingRedaction::literals(vec![b"private-secret".to_vec()])
        .unwrap()
        .protect(validated)
        .await
        .unwrap();
    assert_eq!(output(&protected), b"before ************** after");
    assert_eq!(protected.original_metadata().digest, digest);
    assert_ne!(protected.stored_digest(), digest);
    assert!(protected.original_metadata().data.is_empty());
}
