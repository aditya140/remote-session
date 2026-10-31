use super::*;
use crate::{CaptureContent, CapturedRecord, RecordingEncoder, RedactionRules};
use qgenie_remote_proto::{MachineId, ReceiptId, RecordingEpochId, SessionId, UserId};
use std::{os::unix::fs::PermissionsExt, path::Path};
fn tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn time(n: u64) -> Timestamp {
    Timestamp::from_unix_millis(n * 1000).unwrap()
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
fn config(dir: &Path) -> SpoolConfig {
    let mut c = SpoolConfig::new(dir.to_owned());
    c.metadata_bytes = 1024 * 1024;
    c
}
fn capture(
    spool: &mut RecordingSpool,
    identity: &RecordingIdentity,
    n: u64,
) -> Vec<RecordingBatch> {
    let seq = spool.reserve(identity, time(n)).unwrap();
    let mut encoder = RecordingEncoder::new(
        identity.clone(),
        RedactionRules::literals(vec![b"fake-password".to_vec()]).unwrap(),
    );
    let bytes: Vec<_> = (0..8192).map(|_| rand::random::<u8>()).collect();
    let mut batches = encoder
        .push(CapturedRecord {
            seq,
            captured_at: time(n),
            elapsed_ms: Counter(n),
            content: CaptureContent::Output(bytes),
        })
        .unwrap();
    batches.extend(encoder.finish().unwrap());
    batches
}
fn receipt(batch: &RecordingBatch) -> CommitReceipt {
    CommitReceipt {
        id: ReceiptId::new(),
        chunk: batch.id,
        session: batch.session,
        machine: batch.machine,
        stream_epoch: batch.stream_epoch,
        captured_from: batch.captured_from.clone(),
        captured_to: batch.captured_to.clone(),
        from: batch.from,
        to: batch.to,
        digest: batch.digest.clone(),
        recovery_epoch: "test-epoch-1".into(),
        disposition: ReceiptDisposition::Stored,
        committed_at: time(999),
    }
}

#[test]
fn reclaimed_stream_tombstones_do_not_exhaust_live_capacity() {
    let dir = tempdir();
    let mut spool = RecordingSpool::open(SpoolConfig::new(dir.path().to_owned()), time(1)).unwrap();
    let first = identity();
    // Seed durable, fully reclaimed history in one transaction. Keep its epoch identities.
    let tx = spool.db.transaction().unwrap();
    for n in 0..4100 {
        let value = if n == 0 { first.clone() } else { identity() };
        tx.execute(
            "INSERT INTO streams VALUES(?1,?2,?3,?4,1)",
            params![
                value.stream_epoch.to_string(),
                serde_json::to_string(&value).unwrap(),
                0u64.to_be_bytes().as_slice(),
                time(1).as_str()
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let next = identity();
    spool.register(&next, time(2)).unwrap();
    spool.reserve(&next, time(2)).unwrap();
    assert!(spool.reserve(&first, time(2)).is_err());
    let changed = RecordingIdentity {
        session: SessionId::new(),
        ..first
    };
    assert!(spool.register(&changed, time(2)).is_err());
}
fn gap_receipt(gap: &LossRange, machine: MachineId) -> GapCommitReceipt {
    GapCommitReceipt {
        id: ReceiptId::new(),
        gap: gap.id,
        session: gap.session,
        machine,
        stream_epoch: gap.stream_epoch,
        from: gap.from,
        to: gap.to,
        captured_from: gap.captured_from.clone(),
        captured_to: gap.captured_to.clone(),
        digest: gap.receipt_digest().unwrap(),
        recovery_epoch: "test-epoch-1".into(),
        disposition: ReceiptDisposition::Stored,
        committed_at: time(999),
    }
}

#[test]
fn fetch_status_and_reclamation_use_original_origin_owner_and_machine_scope() {
    let dir = tempdir();
    let original = identity();
    let scope = RemoteScope {
        origin: original.origin.clone(),
        machine: original.machine,
        owner: original.owner,
    };
    let mut spool = RecordingSpool::open(config(dir.path()), time(1)).unwrap();
    let mut identities = Vec::new();
    let mut chunks = Vec::new();
    for n in 0..4 {
        let mut value = original.clone();
        value.session = SessionId::new();
        value.stream_epoch = RecordingEpochId::new();
        match n {
            1 => value.owner = UserId::new(),
            2 => value.origin = "https://different.example.test".into(),
            3 => value.machine = MachineId::new(),
            _ => {}
        }
        spool.register(&value, time(1)).unwrap();
        let batches = capture(&mut spool, &value, 2);
        spool.commit(&value, &batches).unwrap();
        if n == 1 {
            spool.capture_loss(&value, 2, time(3), time(4)).unwrap();
        }
        spool.seal(&value).unwrap();
        chunks.push(batches[0].clone());
        identities.push(value);
    }
    let foreign_gap: LossRange = serde_json::from_str(
        &spool
            .db
            .query_row::<String, _, _>("SELECT metadata FROM gaps", [], |r| r.get(0))
            .unwrap(),
    )
    .unwrap();
    let SpoolItem::Content(own) = spool.fetch_scoped(&scope).unwrap().unwrap() else {
        panic!("another binding's priority gap leaked");
    };
    assert_eq!(own.id, chunks[0].id);
    let offered: bool = spool
        .db
        .query_row("SELECT offered FROM gaps", [], |r| r.get(0))
        .unwrap();
    assert!(!offered);
    let status = spool.status_scoped(&scope).unwrap();
    assert_eq!(status.chunks, 1);
    assert_eq!(status.gaps, 0);
    for chunk in &chunks[1..] {
        assert!(
            !spool
                .acknowledge_scoped(&scope, &receipt(chunk), "test-epoch-1")
                .unwrap()
        );
    }
    assert!(
        !spool
            .acknowledge_gap_scoped(
                &scope,
                &gap_receipt(&foreign_gap, identities[1].machine),
                "test-epoch-1"
            )
            .unwrap()
    );
    assert_eq!(spool.status().unwrap().chunks, 4);
    assert_eq!(spool.status().unwrap().gaps, 1);
    assert!(
        spool
            .acknowledge_scoped(&scope, &receipt(&own), "restored-epoch")
            .is_err()
    );
    assert!(
        spool
            .acknowledge_scoped(&scope, &receipt(&own), "test-epoch-1")
            .unwrap()
    );
    assert!(spool.fetch_scoped(&scope).unwrap().is_none());
    assert_eq!(spool.status_scoped(&scope).unwrap().pending_bytes, 0);
    for value in &identities[1..] {
        let scoped = RemoteScope {
            origin: value.origin.clone(),
            machine: value.machine,
            owner: value.owner,
        };
        let item = spool.fetch_scoped(&scoped).unwrap().unwrap();
        match item {
            SpoolItem::Content(batch) => assert_eq!(batch.session, value.session),
            SpoolItem::Gap(gap) => assert_eq!(gap.session, value.session),
        }
    }
    let mut invalid = scope;
    invalid.origin = "http://invalid.example.test".into();
    assert!(spool.fetch_scoped(&invalid).is_err());
}

#[test]
fn private_persistent_chunks_require_exact_authenticated_receipts_and_survive_lost_ack() {
    let dir = tempdir();
    let identity = identity();
    let mut spool = RecordingSpool::open(config(dir.path()), time(1)).unwrap();
    assert!(RecordingSpool::open(config(dir.path()), time(1)).is_err());
    spool.register(&identity, time(1)).unwrap();
    let batches = capture(&mut spool, &identity, 2);
    spool.commit(&identity, &batches).unwrap();
    spool.seal(&identity).unwrap();
    let expected = serde_json::to_vec(&batches[0]).unwrap();
    let SpoolItem::Content(batch) = spool.fetch().unwrap().unwrap() else {
        panic!("content");
    };
    assert_eq!(serde_json::to_vec(&batch).unwrap(), expected);
    let mut ack = receipt(&batch);
    ack.digest = "00".repeat(32);
    assert!(spool.acknowledge(&ack, "test-epoch-1").is_err());
    let ack = receipt(&batch);
    assert!(spool.acknowledge(&ack, "restored-epoch-2").is_err());
    drop(spool);
    let mut spool = RecordingSpool::open(config(dir.path()), time(100)).unwrap();
    let SpoolItem::Content(retry) = spool.fetch().unwrap().unwrap() else {
        panic!("retry");
    };
    assert_eq!(serde_json::to_vec(&retry).unwrap(), expected);
    assert!(spool.acknowledge(&ack, "test-epoch-1").unwrap());
    assert!(!spool.acknowledge(&ack, "test-epoch-1").unwrap());
    assert!(spool.fetch().unwrap().is_none());
    assert_eq!(spool.status().unwrap().pending_bytes, 0);
}

#[test]
fn cap_commits_coalesced_loss_before_reclaim_and_offered_gap_ids_never_change() {
    let dir = tempdir();
    let identity = identity();
    let mut cfg = config(dir.path());
    cfg.content_bytes = 12 * 1024;
    let mut spool = RecordingSpool::open(cfg, time(1)).unwrap();
    spool.register(&identity, time(1)).unwrap();
    for n in 1..=8 {
        let batches = capture(&mut spool, &identity, n);
        spool.commit(&identity, &batches).unwrap();
    }
    assert!(spool.status().unwrap().pending_bytes <= 12 * 1024);
    assert_eq!(spool.status().unwrap().gaps, 1);
    let SpoolItem::Gap(first) = spool.fetch().unwrap().unwrap() else {
        panic!("gap");
    };
    assert_eq!((first.from, first.to), (Counter(1), Counter(7)));
    let batches = capture(&mut spool, &identity, 9);
    spool.commit(&identity, &batches).unwrap();
    let SpoolItem::Gap(retry) = spool.fetch().unwrap().unwrap() else {
        panic!("gap retry");
    };
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&retry).unwrap()
    );
    assert_eq!(spool.status().unwrap().gaps, 2);
    let mut ack = gap_receipt(&first, identity.machine);
    ack.machine = MachineId::new();
    assert!(spool.acknowledge_gap(&ack, "test-epoch-1").is_err());
    let ack = gap_receipt(&first, identity.machine);
    assert!(spool.acknowledge_gap(&ack, "test-epoch-1").unwrap());
    assert!(!spool.acknowledge_gap(&ack, "test-epoch-1").unwrap());
    drop(spool);
    let mut spool = RecordingSpool::open(config(dir.path()), time(10)).unwrap();
    assert!(spool.status().unwrap().warning.is_some());
    let SpoolItem::Gap(second) = spool.fetch().unwrap().unwrap() else {
        panic!("remaining loss");
    };
    assert_eq!((second.from, second.to), (Counter(8), Counter(8)));
}

#[test]
fn disk_write_failure_is_visible_and_recovery_does_not_reuse_reserved_sequences() {
    let dir = tempdir();
    let identity = identity();
    let mut spool = RecordingSpool::open(config(dir.path()), time(1)).unwrap();
    spool.register(&identity, time(1)).unwrap();
    assert_eq!(spool.reserve(&identity, time(2)).unwrap(), Counter(1));
    spool.db.execute_batch("PRAGMA query_only=ON").unwrap();
    assert!(spool.reserve(&identity, time(3)).is_err());
    assert!(spool.durability_unknown());
    spool.db.execute_batch("PRAGMA query_only=OFF").unwrap();
    spool.reconcile(time(4)).unwrap();
    assert!(!spool.durability_unknown());
    let SpoolItem::Gap(gap) = spool.fetch().unwrap().unwrap() else {
        panic!("uncertainty");
    };
    assert_eq!((gap.from, gap.to), (Counter(1), Counter(2)));
    assert_eq!(gap.captured_from, time(2));
    assert_eq!(gap.captured_to, time(4));
    assert_eq!(spool.reserve(&identity, time(5)).unwrap(), Counter(3));
}

#[test]
fn immutable_identity_forged_batches_and_capture_times_are_rejected_before_disk_mutation() {
    let dir = tempdir();
    let identity = identity();
    let mut spool = RecordingSpool::open(config(dir.path()), time(1)).unwrap();
    spool.register(&identity, time(1)).unwrap();
    let mut other = identity.clone();
    other.owner = UserId::new();
    assert!(spool.register(&other, time(1)).is_err());
    assert!(spool.reserve(&other, time(2)).is_err());
    let batches = capture(&mut spool, &identity, 2);
    let mut forged = batches.clone();
    forged[0].captured_from = time(3);
    assert!(spool.commit(&identity, &forged).is_err());
    let mut forged = batches.clone();
    forged[0].digest = "00".repeat(32);
    assert!(spool.commit(&identity, &forged).is_err());
    let mut forged = batches.clone();
    forged[0].stream_epoch = RecordingEpochId::new();
    assert!(spool.commit(&identity, &forged).is_err());
    assert_eq!(spool.status().unwrap().chunks, 0);
    spool.commit(&identity, &batches).unwrap();
}

#[test]
fn real_process_exit_after_sequence_commit_leaves_a_durable_loss_range() {
    let dir = tempdir();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "recording::spool::tests::recording_crash_helper",
            "--ignored",
        ])
        .env("QGR_RECORDING_CRASH_DIR", dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(result.code(), Some(77));
    let mut spool = RecordingSpool::open(config(dir.path()), time(10)).unwrap();
    let SpoolItem::Gap(gap) = spool.fetch().unwrap().unwrap() else {
        panic!("durable loss");
    };
    assert_eq!((gap.from, gap.to), (Counter(1), Counter(2)));
    let text: String = spool
        .db
        .query_row("SELECT identity FROM streams", [], |r| r.get(0))
        .unwrap();
    let identity = serde_json::from_str(&text).unwrap();
    assert_eq!(spool.reserve(&identity, time(11)).unwrap(), Counter(3));
}

#[test]
#[ignore = "isolated crash helper; invoked by real process proof"]
fn recording_crash_helper() {
    let Some(dir) = std::env::var_os("QGR_RECORDING_CRASH_DIR") else {
        return;
    };
    let identity = identity();
    let mut spool = RecordingSpool::open(config(Path::new(&dir)), time(1)).unwrap();
    spool.register(&identity, time(1)).unwrap();
    spool.reserve(&identity, time(2)).unwrap();
    std::process::exit(77);
}

#[test]
fn missing_or_corrupt_chunk_becomes_durable_loss_instead_of_blocking_upload_forever() {
    for corrupt in [false, true] {
        let dir = tempdir();
        let identity = identity();
        let mut spool = RecordingSpool::open(config(dir.path()), time(1)).unwrap();
        spool.register(&identity, time(1)).unwrap();
        let batches = capture(&mut spool, &identity, 2);
        spool.commit(&identity, &batches).unwrap();
        spool.seal(&identity).unwrap();
        let path = spool.content_path(batches[0].id);
        if corrupt {
            fs::write(path, b"corrupt").unwrap();
        } else {
            fs::remove_file(path).unwrap();
        }
        assert!(spool.fetch().is_err());
        assert!(spool.durability_unknown());
        let SpoolItem::Gap(gap) = spool.fetch().unwrap().unwrap() else {
            panic!("loss");
        };
        assert!(matches!(gap.reason, LossReason::DiskFailure));
        assert_eq!((gap.from, gap.to), (batches[0].from, batches[0].to));
        drop(spool);
        let mut spool = RecordingSpool::open(config(dir.path()), time(10)).unwrap();
        let SpoolItem::Gap(retry) = spool.fetch().unwrap().unwrap() else {
            panic!("durable loss");
        };
        assert_eq!(gap.id, retry.id);
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires explicitly isolated <=16 MiB tmpfs; tests/e2e/recording_disk_full.sh"]
fn actual_enospc_recovers_with_durable_uncertainty_and_live_pty_control() {
    use qgenie_remote_proto::{ClientId, Operation, Reply};
    use std::os::unix::ffi::OsStrExt;
    let Some(path) = std::env::var_os("QGR_RECORDING_DISK_FULL_DIR") else {
        return;
    };
    let path = PathBuf::from(path);
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    let mut stats = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: live C path and output storage; result is initialized only on success.
    assert_eq!(
        unsafe { libc::statfs(c_path.as_ptr(), stats.as_mut_ptr()) },
        0
    );
    // SAFETY: statfs succeeded above.
    let stats = unsafe { stats.assume_init() };
    assert_eq!(stats.f_type, libc::TMPFS_MAGIC);
    assert!(stats.f_blocks as u128 * stats.f_bsize as u128 <= 16 * 1024 * 1024);
    let native = tempdir();
    let root = fs::canonicalize(native.path()).unwrap();
    let runtime =
        crate::Runtime::open(crate::Config::new(root.clone(), vec![root.clone()])).unwrap();
    let client = ClientId::new();
    let call = |runtime: &std::sync::Arc<crate::Runtime>, client, body| {
        runtime
            .dispatch(qgenie_remote_proto::Request {
                v: qgenie_remote_proto::PROTOCOL_MAJOR,
                id: qgenie_remote_proto::RequestId::new(),
                client,
                body,
            })
            .body
    };
    let mut launch = qgenie_remote_proto::StartSpec {
        key: qgenie_remote_proto::IdempotencyKey::new(),
        command: vec![],
        cwd: root.to_str().unwrap().into(),
        rows: 24,
        cols: 80,
    };
    launch.command = vec![
        "/bin/sh".into(),
        "-c".into(),
        "read value; printf 'CONTROL-STILL-WORKS:%s\\n' \"$value\"".into(),
    ];
    let Reply::Started(session) = call(&runtime, client, Operation::Start(launch)) else {
        panic!("native start");
    };
    let identity = identity();
    let mut spool = RecordingSpool::open(config(&path), time(1)).unwrap();
    spool.register(&identity, time(1)).unwrap();
    spool.reserve(&identity, time(2)).unwrap();
    let mut filler = File::create(path.join("filler")).unwrap();
    let block = vec![0u8; 128 * 1024];
    let mut filled = 0;
    loop {
        match filler.write(&block) {
            Ok(size) => {
                assert!(size > 0);
                filled += size;
                assert!(filled <= 16 * 1024 * 1024);
            }
            Err(e) => {
                assert_eq!(e.raw_os_error(), Some(libc::ENOSPC));
                break;
            }
        }
    }
    assert!(spool.reserve(&identity, time(3)).is_err());
    assert!(spool.durability_unknown());
    let Reply::Control(lease) = call(
        &runtime,
        client,
        Operation::TakeControl {
            session: session.id,
            takeover: false,
        },
    ) else {
        panic!("control during ENOSPC");
    };
    assert!(matches!(
        call(
            &runtime,
            client,
            Operation::Input {
                session: session.id,
                generation: lease.generation,
                input_seq: Counter(1),
                data: STANDARD.encode(b"ok\n")
            }
        ),
        Reply::InputAck {
            outcome: qgenie_remote_proto::ApplicationOutcome::Applied,
            ..
        }
    ));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(std::time::Instant::now() < deadline);
        let sessions = runtime.list().unwrap();
        if sessions.iter().any(|s| {
            s.id == session.id && matches!(s.state, qgenie_remote_proto::SessionState::Exited)
        }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let Reply::Replay(page) = call(
        &runtime,
        client,
        Operation::Replay {
            session: session.id,
            after: Counter(0),
        },
    ) else {
        panic!("native replay");
    };
    let output: Vec<_> = page
        .events
        .into_iter()
        .flat_map(|e| match e.body {
            qgenie_remote_proto::EventBody::PtyOutput(data) => STANDARD.decode(data).unwrap(),
            _ => vec![],
        })
        .collect();
    assert!(
        output
            .windows(b"CONTROL-STILL-WORKS:ok".len())
            .any(|w| w == b"CONTROL-STILL-WORKS:ok")
    );
    drop(filler);
    fs::remove_file(path.join("filler")).unwrap();
    spool.reconcile(time(4)).unwrap();
    assert!(!spool.durability_unknown());
    let SpoolItem::Gap(gap) = spool.fetch().unwrap().unwrap() else {
        panic!("explicit uncertainty");
    };
    assert_eq!((gap.from, gap.to), (Counter(1), Counter(2)));
    assert!(spool.status().unwrap().warning.is_some());
    runtime.shutdown(true).unwrap();
}
