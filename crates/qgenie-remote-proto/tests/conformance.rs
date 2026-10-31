use qgenie_remote_proto::*;

#[test]
fn start_fixture_preserves_ids_and_accepts_additive_fields() {
    let text = include_str!("../../../conformance/json/start.json");
    let request: Request = decode_json(text.as_bytes()).unwrap();
    let encoded = serde_json::to_value(&request).unwrap();
    let original: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(encoded, original);
    let mut future = original;
    future["future_field"] = serde_json::json!({"ignored":true});
    let decoded: Request = decode_json(&serde_json::to_vec(&future).unwrap()).unwrap();
    assert_eq!(request.id, decoded.id);
}

#[test]
fn binary_output_fixture_has_fixed_endianness_and_exact_roundtrip() {
    let hex = include_str!("../../../conformance/binary/pty-output.hex").trim();
    let bytes: Vec<_> = hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    let frame = PtyFrame::decode(&bytes).unwrap();
    assert_eq!(frame.seq, Counter(u64::MAX));
    assert_eq!(
        frame.session.to_string(),
        "s_550e8400-e29b-41d4-a716-446655440000"
    );
    assert_eq!(frame.encode().unwrap(), bytes);
}

#[cfg(feature = "schema")]
#[test]
fn public_schema_does_not_export_admin_shutdown() {
    let text = serde_json::to_string(&public_schema()).unwrap();
    assert!(!text.contains("terminate_sessions"));
    assert!(!text.contains("AdminRequest"));
    assert!(!text.contains("register_remote_binding"));
}

#[test]
fn public_network_fixtures_roundtrip_and_keep_full_precision() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/json");
    let mut count = 0;
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if !path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("network-")
        {
            continue;
        }
        let text = std::fs::read(&path).unwrap();
        let envelope = wire::Envelope::decode(&text)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let roundtrip: serde_json::Value =
            serde_json::from_slice(&envelope.encode().unwrap()).unwrap();
        assert_eq!(
            roundtrip,
            serde_json::from_slice::<serde_json::Value>(&text).unwrap(),
            "{}",
            path.display()
        );
        count += 1;
    }
    assert_eq!(count, 32);
}

#[test]
fn gap_receipts_have_distinct_typed_identity_canonical_digest_and_exact_targets() {
    let gap = wire::Envelope::decode(include_bytes!("../../../conformance/json/network-gap.json"))
        .unwrap();
    let wire::Message::RecordingGap(gap) = gap.message else {
        panic!("gap");
    };
    let mut receipt = wire::Envelope::decode(include_bytes!(
        "../../../conformance/json/network-gap-receipt.json"
    ))
    .unwrap();
    let wire::Message::RecordingGapReceipt(body) = &receipt.message else {
        panic!("receipt");
    };
    assert_eq!(body.digest, gap.receipt_digest().unwrap());
    assert_eq!(body.gap, gap.id);
    receipt.machine = Some(MachineId::new());
    assert!(receipt.validate().is_err());
    let mut json: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../conformance/json/network-gap-receipt.json"
    ))
    .unwrap();
    json["body"]["gap"] = serde_json::json!(ChunkId::new());
    assert!(wire::Envelope::decode(&serde_json::to_vec(&json).unwrap()).is_err());
}

#[test]
fn normalized_recording_fixture_is_bounded_and_cannot_record_raw_input() {
    let raw = include_bytes!("../../../conformance/recording/event.jsonl");
    let record: RecordingRecord = decode_json(raw).unwrap();
    record.validate(&wire::RecordingStream::Events).unwrap();
    assert_eq!(record.seq, Counter(9_007_199_254_740_993));
    assert!(record.validate(&wire::RecordingStream::Pty).is_err());
    let mut value: serde_json::Value = serde_json::from_slice(raw).unwrap();
    value["content"]["type"] = serde_json::json!("input");
    assert!(decode_json::<RecordingRecord>(&serde_json::to_vec(&value).unwrap()).is_err());
    let mut record = record;
    record.content = RecordedContent::Output {
        data: "aGk=".into(),
        fragment: 33,
        last: true,
    };
    assert!(record.validate(&wire::RecordingStream::Pty).is_err());
}

#[test]
fn network_validation_rejects_missing_fences_targets_and_changed_cursors() {
    use wire::*;
    let mut input = Envelope::new(Message::Input {
        authority: Generations {
            host: Counter(1),
            controller: None,
        },
        input_seq: Counter(1),
        input: SessionInput::Pty("aGk=".into()),
    });
    assert!(input.validate().is_err());
    input.session = Some(SessionId::new());
    assert!(input.validate().is_err());
    if let Message::Input { authority, .. } = &mut input.message {
        authority.controller = Some(Counter(2));
    }
    assert!(input.validate().is_ok());
    input.v = 2;
    assert!(matches!(
        input.validate().unwrap_err().code,
        ErrorCode::UnsupportedVersion
    ));
    let mut replay = Envelope::decode(include_bytes!(
        "../../../conformance/json/network-replay.json"
    ))
    .unwrap();
    if let Message::Replay { events, .. } = &mut replay.message {
        events.swap(0, 1);
    }
    assert!(replay.validate().is_err());
}

#[test]
fn canonical_timestamps_and_launch_fingerprints_preserve_meaning() {
    let instant = wire::Timestamp::from_unix_millis(1_761_076_800_001).unwrap();
    assert_eq!(instant.unix_millis(), 1_761_076_800_001);
    for invalid in [
        "2026-10-17T00:00:00Z",
        "2026-10-17T00:00:00.000+00:00",
        "2026-13-17T00:00:00.000Z",
        "1969-12-31T23:59:59.000Z",
    ] {
        assert!(wire::Timestamp::try_from(invalid.to_owned()).is_err());
    }
    let envelope = wire::Envelope::decode(include_bytes!(
        "../../../conformance/json/network-start.json"
    ))
    .unwrap();
    let wire::Message::Start(mut launch) = envelope.message else {
        unreachable!()
    };
    let fingerprint = launch.fingerprint().unwrap();
    launch.key = IdempotencyKey::new();
    assert_eq!(fingerprint, launch.fingerprint().unwrap());
    launch.cols += 1;
    assert_ne!(fingerprint, launch.fingerprint().unwrap());
}

#[test]
fn independent_dispatch_protocol_cannot_deserialize_shutdown() {
    assert!(
        decode_json::<ipc::DispatchFrame>(br#"{"type":"register_remote_binding","body":{}}"#)
            .is_err()
    );
    let frame = ipc::DispatchFrame::Hello {
        major: IPC_MAJOR,
        minor: 0,
        capabilities: vec!["local".into()],
    };
    assert!(frame.validate_hello().is_ok());
    assert!(
        decode_json::<ipc::DispatchFrame>(
            br#"{"type":"shutdown","body":{"terminate_sessions":true}}"#
        )
        .is_err()
    );
    let unsupported = ipc::DispatchFrame::Hello {
        major: 2,
        minor: 0,
        capabilities: vec![],
    };
    assert!(matches!(
        unsupported.validate_hello().unwrap_err().code,
        ErrorCode::UnsupportedVersion
    ));
}

#[test]
fn native_control_messages_require_scoped_bounds_and_correlation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/json");
    for (name, changes) in [
        (
            "native-control-activate",
            vec![
                ("remaining_ms", serde_json::json!(15001)),
                ("generation", serde_json::json!("0")),
            ],
        ),
        (
            "native-control-state",
            vec![
                ("remaining_ms", serde_json::json!(0)),
                ("floor", serde_json::json!("0")),
                ("remaining_ms", serde_json::json!(15001)),
            ],
        ),
    ] {
        let original: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("network-{name}.json"))).unwrap(),
        )
        .unwrap();
        for (key, value) in changes {
            let mut invalid = original.clone();
            invalid["body"][key] = value;
            assert!(wire::Envelope::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let mut invalid = original;
        invalid.as_object_mut().unwrap().remove("machine");
        assert!(wire::Envelope::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
}

#[test]
fn input_acknowledgments_decode_legacy_and_receipt_shapes() {
    use qgenie_remote_proto::wire::Message;
    let legacy = serde_json::json!({"type":"session.input.ack","body":{"generation":"1","input_seq":"2","outcome":"applied"}});
    let decoded: Message = serde_json::from_value(legacy.clone()).unwrap();
    assert!(matches!(&decoded, Message::InputAck { turn: None, .. }));
    assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    let mut receipt = legacy;
    receipt["body"]["turn"] = serde_json::json!("native-turn");
    let decoded: Message = serde_json::from_value(receipt).unwrap();
    assert!(matches!(decoded, Message::InputAck { turn: Some(turn), .. } if turn == "native-turn"));
}
