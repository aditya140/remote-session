use super::*;

fn identity() -> RecordingIdentity {
    RecordingIdentity {
        origin: "https://gateway.example.test".into(),
        session: SessionId::new(),
        machine: MachineId::new(),
        owner: UserId::new(),
        stream: RecordingStream::Pty,
        stream_epoch: RecordingEpochId::new(),
    }
}
fn record(seq: u64, content: CaptureContent) -> CapturedRecord {
    CapturedRecord {
        seq: Counter(seq),
        captured_at: Timestamp::from_unix_millis(seq * 1000).unwrap(),
        elapsed_ms: Counter(seq * 100),
        content,
    }
}
fn records(batches: &[RecordingBatch]) -> Vec<Record> {
    let mut records = Vec::new();
    for batch in batches {
        batch.validate().unwrap();
        let compressed = STANDARD.decode(&batch.data).unwrap();
        assert_eq!(batch.digest, format!("{:x}", Sha256::digest(&compressed)));
        let raw = zstd::bulk::decompress(&compressed, RAW_LIMIT).unwrap();
        assert_eq!(raw.len(), batch.uncompressed_bytes as usize);
        for line in raw.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            records.push(serde_json::from_slice(line).unwrap());
        }
    }
    records
}

#[test]
fn timer_flush_cannot_reveal_a_secret_across_output_and_resize_boundaries() {
    for cut in 0..=b"super-secret".len() {
        let mut encoder = RecordingEncoder::new(
            identity(),
            RedactionRules::literals(vec![b"super-secret".to_vec()]).unwrap(),
        );
        let mut batches = encoder
            .push(record(
                1,
                CaptureContent::Output([b"before ".as_slice(), &b"super-secret"[..cut]].concat()),
            ))
            .unwrap();
        batches.extend(encoder.flush().unwrap());
        batches.extend(
            encoder
                .push(record(2, CaptureContent::Resize { rows: 24, cols: 80 }))
                .unwrap(),
        );
        batches.extend(encoder.flush().unwrap());
        batches.extend(
            encoder
                .push(record(
                    3,
                    CaptureContent::Output([&b"super-secret"[cut..], b" after\xff"].concat()),
                ))
                .unwrap(),
        );
        batches.extend(encoder.finish().unwrap());
        let records = records(&batches);
        assert_eq!(
            records.iter().map(|r| r.seq.0).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(matches!(
            records[1].content,
            Content::Resize { rows: 24, cols: 80 }
        ));
        let output: Vec<_> = records
            .into_iter()
            .flat_map(|r| match r.content {
                Content::Output { data, .. } => STANDARD.decode(data).unwrap(),
                _ => vec![],
            })
            .collect();
        assert_eq!(output, b"before [REDACTED] after\xff", "cut {cut}");
    }
}

#[test]
fn incompressible_content_and_redaction_expansion_preserve_fragments_times_and_binary() {
    let mut encoder = RecordingEncoder::new(
        identity(),
        RedactionRules::literals(vec![b"!".to_vec()]).unwrap(),
    );
    let mut batches = Vec::new();
    let mut expected = Vec::new();
    for seq in 1..=40 {
        let bytes = if seq == 1 {
            vec![b'!'; 32 * 1024]
        } else {
            (0..32 * 1024).map(|_| rand::random::<u8>()).collect()
        };
        for &byte in &bytes {
            if byte == b'!' {
                expected.extend_from_slice(b"[REDACTED]");
            } else {
                expected.push(byte);
            }
        }
        batches.extend(
            encoder
                .push(record(seq, CaptureContent::Output(bytes)))
                .unwrap(),
        );
    }
    batches.extend(encoder.finish().unwrap());
    assert!(batches.len() > 10);
    let mut actual = Vec::new();
    let mut previous = (1, 0);
    for r in records(&batches) {
        assert_eq!(r.captured_at.unix_millis(), r.seq.0 * 1000);
        assert_eq!(r.elapsed_ms.0, r.seq.0 * 100);
        let Content::Output {
            data,
            fragment,
            last,
        } = r.content
        else {
            panic!("output");
        };
        assert_eq!((r.seq.0, fragment), previous);
        actual.extend(STANDARD.decode(data).unwrap());
        previous = if last {
            (r.seq.0 + 1, 0)
        } else {
            (r.seq.0, fragment + 1)
        };
    }
    assert_eq!(actual, expected);
    assert_eq!(previous, (41, 0));
}

#[test]
fn malformed_admission_and_full_u64_boundaries_do_not_advance_the_encoder() {
    let mut encoder = RecordingEncoder::new(identity(), RedactionRules::literals(vec![]).unwrap());
    let mut r = record(1, CaptureContent::Output(vec![0; 32769]));
    assert!(encoder.push(r).is_err());
    r = record(1, CaptureContent::Output(vec![0; 32768]));
    r.seq = Counter(u64::MAX);
    encoder.push(r).unwrap();
    assert!(
        encoder
            .push(record(1, CaptureContent::Output(vec![])))
            .is_err()
    );
    assert_eq!(encoder.finish().unwrap()[0].to, Counter(u64::MAX));
    assert!(encoder.finish().is_err());
}

#[test]
fn network_recording_fixture_hash_and_normalized_content_match_the_shared_contract() {
    let frame = qgenie_remote_proto::wire::Envelope::decode(include_bytes!(
        "../../../../../conformance/json/network-recording.json"
    ))
    .unwrap();
    let qgenie_remote_proto::wire::Message::RecordingIngest(batch) = frame.message else {
        panic!("recording");
    };
    let compressed = STANDARD.decode(&batch.data).unwrap();
    assert_eq!(batch.digest, format!("{:x}", Sha256::digest(&compressed)));
    let raw = zstd::bulk::decompress(&compressed, RAW_LIMIT).unwrap();
    assert_eq!(
        raw,
        include_bytes!("../../../../../conformance/recording/event.jsonl")
    );
    for line in raw.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        qgenie_remote_proto::decode_json::<qgenie_remote_proto::RecordingRecord>(line)
            .unwrap()
            .validate(&batch.stream)
            .unwrap();
    }
}

#[test]
fn utc_month_boundaries_and_clock_regression_do_not_extend_older_content_retention() {
    let mut encoder = RecordingEncoder::new(identity(), RedactionRules::literals(vec![]).unwrap());
    let times = [
        "2026-01-31T23:59:59.999Z",
        "2026-02-01T00:00:00.001Z",
        "2026-01-31T23:59:59.998Z",
    ];
    let mut batches = Vec::new();
    for (index, time) in times.iter().enumerate() {
        let mut r = record(index as u64 + 1, CaptureContent::Output(b"output".to_vec()));
        r.captured_at = (*time).to_owned().try_into().unwrap();
        batches.extend(encoder.push(r).unwrap());
    }
    batches.extend(encoder.finish().unwrap());
    assert_eq!(batches.len(), 3);
    for (batch, time) in batches.iter().zip(times) {
        assert_eq!(batch.captured_from.as_str(), time);
        assert_eq!(batch.captured_to.as_str(), time);
    }
}
