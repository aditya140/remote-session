use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{
    MachineId, ReceiptId, RecordedContent, RecordingRecord, SessionId, UserId,
    wire::{LossReason, ReceiptDisposition},
};
use std::{fs, os::unix::fs::PermissionsExt};

fn time(n: u64) -> Timestamp {
    Timestamp::from_unix_millis(n * 1000).unwrap()
}
fn tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
fn config(dir: &std::path::Path) -> SpoolConfig {
    let mut c = SpoolConfig::new(dir.to_owned());
    c.metadata_bytes = 1024 * 1024;
    c
}
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
fn receipt(b: &qgenie_remote_proto::wire::RecordingBatch) -> CommitReceipt {
    CommitReceipt {
        id: ReceiptId::new(),
        chunk: b.id,
        session: b.session,
        machine: b.machine,
        stream_epoch: b.stream_epoch,
        captured_from: b.captured_from.clone(),
        captured_to: b.captured_to.clone(),
        from: b.from,
        to: b.to,
        digest: b.digest.clone(),
        recovery_epoch: "test-1".into(),
        disposition: ReceiptDisposition::Stored,
        committed_at: time(999),
    }
}
fn wait_until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn scoped_worker_rpcs_do_not_upload_or_ack_another_enrollment() {
    let dir = tempdir();
    let worker = RecordingWorker::spawn(config(dir.path()), time(1)).unwrap();
    let first = identity();
    let mut second = first.clone();
    second.owner = UserId::new();
    second.session = SessionId::new();
    second.stream_epoch = RecordingEpochId::new();
    for value in [&first, &second] {
        let capture = worker
            .register(
                value.clone(),
                RedactionRules::literals(Vec::new()).unwrap(),
                time(1),
            )
            .unwrap();
        assert!(capture.capture(
            time(2),
            Counter(1),
            CaptureContent::Output(b"scoped fixture".to_vec())
        ));
        capture.finish();
    }
    wait_until(|| worker.status().unwrap().chunks == 2);
    let first_scope = RemoteScope {
        origin: first.origin.clone(),
        machine: first.machine,
        owner: first.owner,
    };
    let second_scope = RemoteScope {
        origin: second.origin.clone(),
        machine: second.machine,
        owner: second.owner,
    };
    let SpoolItem::Content(batch) = worker.fetch_scoped(first_scope.clone()).unwrap().unwrap()
    else {
        panic!("content");
    };
    assert_eq!(batch.session, first.session);
    assert!(
        !worker
            .acknowledge_scoped(second_scope.clone(), receipt(&batch), "test-1".into())
            .unwrap()
    );
    assert!(
        worker
            .acknowledge_scoped(first_scope.clone(), receipt(&batch), "test-1".into())
            .unwrap()
    );
    assert!(worker.fetch_scoped(first_scope.clone()).unwrap().is_none());
    assert_eq!(worker.status_scoped(first_scope).unwrap().chunks, 0);
    assert_eq!(worker.status_scoped(second_scope).unwrap().chunks, 1);
    worker.shutdown().unwrap();
}

#[test]
fn bounded_handoff_preserves_redaction_capture_time_and_final_output_after_viewer_drop() {
    let dir = tempdir();
    let worker = RecordingWorker::spawn(config(dir.path()), time(1)).unwrap();
    let identity = identity();
    let capture = worker
        .register(
            identity.clone(),
            RedactionRules::literals(vec![b"secret-value".to_vec()]).unwrap(),
            time(1),
        )
        .unwrap();
    let viewer = capture.clone();
    assert!(viewer.capture(
        time(2),
        Counter(2),
        CaptureContent::Output(b"before secret-".to_vec())
    ));
    drop(viewer);
    assert!(capture.capture(
        time(3),
        Counter(3),
        CaptureContent::Resize { rows: 25, cols: 90 }
    ));
    assert!(capture.capture(
        time(4),
        Counter(4),
        CaptureContent::Output(b"value after\xff".to_vec())
    ));
    capture.finish();
    wait_until(|| worker.status().is_ok_and(|s| s.chunks > 0));
    let mut records = Vec::new();
    while let Some(item) = worker.fetch().unwrap() {
        let SpoolItem::Content(batch) = item else {
            panic!("no loss");
        };
        let raw =
            zstd::bulk::decompress(&STANDARD.decode(&batch.data).unwrap(), 1024 * 1024).unwrap();
        records.extend(
            raw.split(|b| *b == b'\n')
                .filter(|l| !l.is_empty())
                .map(|l| serde_json::from_slice::<RecordingRecord>(l).unwrap()),
        );
        let ack = receipt(&batch);
        assert!(worker.acknowledge(ack.clone(), "test-1".into()).unwrap());
        assert!(!worker.acknowledge(ack, "test-1".into()).unwrap());
    }
    assert_eq!(
        records
            .iter()
            .map(|r| (r.seq.0, r.captured_at.unix_millis()))
            .collect::<Vec<_>>(),
        vec![(1, 2000), (2, 3000), (3, 4000)]
    );
    let output: Vec<_> = records
        .into_iter()
        .flat_map(|r| match r.content {
            RecordedContent::Output { data, .. } => STANDARD.decode(data).unwrap(),
            _ => vec![],
        })
        .collect();
    assert_eq!(output, b"before [REDACTED] after\xff");
    worker.shutdown().unwrap();
    let mut spool = RecordingSpool::open(config(dir.path()), time(10)).unwrap();
    assert!(spool.fetch().unwrap().is_none());
}

#[test]
fn sqlite_stall_cannot_block_capture_admission_and_overflow_survives_recovery() {
    let dir = tempdir();
    let worker = RecordingWorker::spawn(config(dir.path()), time(1)).unwrap();
    let identity = identity();
    let capture = worker
        .register(
            identity.clone(),
            RedactionRules::literals(vec![]).unwrap(),
            time(1),
        )
        .unwrap();
    let database = rusqlite::Connection::open(dir.path().join("metadata.sqlite")).unwrap();
    database.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let started = Instant::now();
    let mut rejected = 0;
    for n in 0..1000 {
        if !capture.capture(
            time(2),
            Counter(n),
            CaptureContent::Output(vec![b'x'; 1024]),
        ) {
            rejected += 1;
        }
    }
    assert!(rejected > 0);
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(capture.backpressure_seen());
    wait_until(|| worker.durability_unknown());
    database.execute_batch("ROLLBACK").unwrap();
    drop(database);
    wait_until(|| !worker.durability_unknown());
    assert!(capture.capture(
        time(5),
        Counter(1001),
        CaptureContent::Output(b"after-recovery".to_vec())
    ));
    capture.finish();
    wait_until(|| worker.status().is_ok_and(|s| s.gaps > 0 && s.chunks > 0));
    let mut saw_gap = false;
    let mut saw_output = false;
    while let Some(item) = worker.fetch().unwrap() {
        match item {
            SpoolItem::Gap(gap) => {
                assert!(matches!(
                    gap.reason,
                    LossReason::RecoveryLoss | LossReason::CaptureBackpressure
                ));
                saw_gap = true;
                let receipt = GapCommitReceipt {
                    id: ReceiptId::new(),
                    gap: gap.id,
                    session: gap.session,
                    machine: identity.machine,
                    stream_epoch: gap.stream_epoch,
                    from: gap.from,
                    to: gap.to,
                    captured_from: gap.captured_from.clone(),
                    captured_to: gap.captured_to.clone(),
                    digest: gap.receipt_digest().unwrap(),
                    recovery_epoch: "test-1".into(),
                    disposition: ReceiptDisposition::Stored,
                    committed_at: time(999),
                };
                worker.acknowledge_gap(receipt, "test-1".into()).unwrap();
            }
            SpoolItem::Content(batch) => {
                let raw =
                    zstd::bulk::decompress(&STANDARD.decode(&batch.data).unwrap(), 1024 * 1024)
                        .unwrap();
                for line in raw.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                    let record: RecordingRecord = serde_json::from_slice(line).unwrap();
                    if let RecordedContent::Output { data, .. } = record.content {
                        saw_output |= STANDARD.decode(data).unwrap() == b"after-recovery";
                    }
                }
                worker
                    .acknowledge(receipt(&batch), "test-1".into())
                    .unwrap();
            }
        }
    }
    assert!(saw_gap && saw_output);
    worker.shutdown().unwrap();
}

#[test]
fn real_no_echo_pty_records_output_without_password_input() {
    let dir = tempdir();
    let worker = RecordingWorker::spawn(config(dir.path()), time(1)).unwrap();
    let capture = worker
        .register(
            identity(),
            RedactionRules::literals(vec![]).unwrap(),
            time(1),
        )
        .unwrap();
    let spec = qgenie_remote_proto::StartSpec {
        key: qgenie_remote_proto::IdempotencyKey::new(),
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            "stty -echo; printf 'READY\\n'; IFS= read -r secret; printf 'DONE\\n'; stty echo"
                .into(),
        ],
        cwd: dir.path().to_str().unwrap().into(),
        rows: 24,
        cols: 80,
    };
    let mut process = qgenie_remote_adapters::spawn(&spec).unwrap();
    let mut output = Vec::new();
    let mut sent = false;
    let mut buffer = [0u8; 4096];
    let mut number = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline);
        match process.reader.try_read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                number += 1;
                assert!(capture.capture(
                    time(number + 1),
                    Counter(number),
                    CaptureContent::Output(buffer[..size].to_vec())
                ));
                output.extend_from_slice(&buffer[..size]);
                if !sent && output.windows(5).any(|w| w == b"READY") {
                    process.control.input(b"no-echo-password-93\n").unwrap();
                    sent = true;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
            Err(e) => panic!("{e}"),
        }
    }
    assert!(sent && process.child.wait().unwrap().success());
    capture.finish();
    wait_until(|| worker.status().is_ok_and(|s| s.chunks > 0));
    let mut recorded = Vec::new();
    while let Some(item) = worker.fetch().unwrap() {
        let SpoolItem::Content(batch) = item else {
            panic!("unexpected recording loss");
        };
        let raw =
            zstd::bulk::decompress(&STANDARD.decode(&batch.data).unwrap(), 1024 * 1024).unwrap();
        for line in raw.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let record: RecordingRecord = serde_json::from_slice(line).unwrap();
            record.validate(&batch.stream).unwrap();
            if let RecordedContent::Output { data, .. } = record.content {
                recorded.extend(STANDARD.decode(data).unwrap());
            }
        }
        worker
            .acknowledge(receipt(&batch), "test-1".into())
            .unwrap();
    }
    assert_eq!(recorded, output);
    assert!(recorded.windows(4).any(|w| w == b"DONE"));
    assert!(
        !recorded
            .windows(b"no-echo-password-93".len())
            .any(|w| w == b"no-echo-password-93")
    );
    worker.shutdown().unwrap();
}

#[test]
fn owner_stop_without_native_eof_drains_admissions_and_keeps_the_tail_uncertain() {
    let dir = tempdir();
    let worker = RecordingWorker::spawn(config(dir.path()), time(1)).unwrap();
    let identity = identity();
    let capture = worker
        .register(identity, RedactionRules::literals(vec![]).unwrap(), time(1))
        .unwrap();
    for n in 0..10 {
        assert!(capture.capture(
            time(2),
            Counter(n),
            CaptureContent::Output(b"final-output".to_vec())
        ));
    }
    worker.shutdown().unwrap();
    assert!(!capture.capture(
        time(3),
        Counter(11),
        CaptureContent::Output(b"too-late".to_vec())
    ));
    let mut spool = RecordingSpool::open(config(dir.path()), time(4)).unwrap();
    let Some(SpoolItem::Gap(gap)) = spool.fetch().unwrap() else {
        panic!("native EOF was never established");
    };
    assert!(matches!(gap.reason, LossReason::RecoveryLoss));
    assert_eq!((gap.from, gap.to), (Counter(11), Counter(11)));
    assert!(spool.status().unwrap().chunks > 0);
}
