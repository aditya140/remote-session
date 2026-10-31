use super::super::tests::{call, directory, spec};
use super::*;
use crate::SpoolItem;
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::ipc::{DispatchFrame, HostActivation, RemoteContext};
use qgenie_remote_proto::{
    RecordedContent, RecordingRecord,
    wire::{CommitReceipt, LossReason, ReceiptDisposition, Timestamp},
};

#[track_caller]
fn wait(mut predicate: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(6);
    while !predicate() {
        assert!(
            Instant::now() < until,
            "native capture condition did not complete"
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn scope() -> RemoteScope {
    RemoteScope {
        origin: "https://native-recording.example.test".into(),
        machine: MachineId::new(),
        owner: UserId::new(),
    }
}
fn configured(path: &std::path::Path, scope: &RemoteScope) -> Config {
    let mut config = Config::new(path.to_owned(), vec![path.to_owned()]);
    config.remote_bindings.push(scope.clone());
    config.recording.metadata_bytes = 1024 * 1024;
    config.recording.redaction =
        RedactionRules::literals(vec![b"visible-secret".to_vec()]).unwrap();
    config
}
fn access(
    runtime: &Arc<Runtime>,
    scope: &RemoteScope,
    generation: u64,
    remaining_ms: u32,
) -> RemoteContext {
    // Domain fixture: the actual Unix test below independently negotiates read capability.
    let owned = scope.clone();
    let connection = ClientId::new();
    let DispatchFrame::AccessChallengeIssued { nonce, .. } =
        runtime.dispatch_frame(DispatchFrame::AccessChallenge {
            scope: owned.clone(),
            connection,
            host_generation: Counter(generation),
        })
    else {
        panic!("challenge");
    };
    let activation = HostActivation {
        scope: owned.clone(),
        connection,
        generation: Counter(generation),
        nonce,
        remaining_ms,
        policy_revision: Counter(1),
    };
    let reply = runtime.dispatch_frame(DispatchFrame::ActivateHost(activation));
    assert!(matches!(reply, DispatchFrame::HostActivated { .. }));
    RemoteContext {
        scope: owned,
        actor: scope.owner,
        client: ClientId::new(),
        connection,
        host_generation: Counter(generation),
        controller_generation: None,
        access_nonce: nonce,
        remaining_access_ms: remaining_ms,
        policy_revision: Counter(1),
    }
}
fn replay(runtime: &Arc<Runtime>, session: SessionId) -> Vec<u8> {
    let Reply::Replay(batch) = call(
        runtime,
        ClientId::new(),
        Operation::Replay {
            session,
            after: Counter(0),
        },
    ) else {
        panic!("replay");
    };
    let mut bytes = Vec::new();
    for event in batch.events {
        if let EventBody::PtyOutput(data) = event.body {
            bytes.extend(STANDARD.decode(data).unwrap());
        }
    }
    bytes
}
fn input(
    runtime: &Arc<Runtime>,
    client: ClientId,
    session: SessionId,
    generation: Counter,
    seq: u64,
    data: &[u8],
) {
    assert!(matches!(
        call(
            runtime,
            client,
            Operation::Input {
                session,
                generation,
                input_seq: Counter(seq),
                data: STANDARD.encode(data)
            }
        ),
        Reply::InputAck {
            outcome: ApplicationOutcome::Applied,
            ..
        }
    ));
}
fn control(runtime: &Arc<Runtime>, client: ClientId, session: SessionId) -> Counter {
    let Reply::Control(lease) = call(
        runtime,
        client,
        Operation::TakeControl {
            session,
            takeover: false,
        },
    ) else {
        panic!("control");
    };
    lease.generation
}
fn acknowledge(
    worker: &RecordingWorker,
    scope: &RemoteScope,
    batch: &qgenie_remote_proto::wire::RecordingBatch,
) {
    assert!(
        worker
            .acknowledge_scoped(
                scope.clone(),
                receipt(batch, "native-test"),
                "native-test".into()
            )
            .unwrap()
    );
}
fn receipt(batch: &qgenie_remote_proto::wire::RecordingBatch, epoch: &str) -> CommitReceipt {
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
        recovery_epoch: epoch.into(),
        disposition: ReceiptDisposition::Stored,
        committed_at: Timestamp::from_unix_millis(now_ms()).unwrap(),
    }
}

fn bind_epoch(runtime: &Arc<Runtime>, context: &RemoteContext, epoch: &str) -> ChallengeId {
    let DispatchFrame::RecordingEpochChallengeIssued { challenge } =
        runtime.dispatch_frame(DispatchFrame::RecordingEpochChallenge {
            context: context.clone(),
        })
    else {
        panic!("epoch challenge");
    };
    let authority = qgenie_remote_proto::rest::RecordingAuthority {
        challenge,
        origin: context.scope.origin.clone(),
        owner: context.scope.owner,
        machine: context.scope.machine,
        key: KeyId::new(),
        backend_epoch: epoch.into(),
    };
    assert!(
        matches!(runtime.dispatch_frame(DispatchFrame::BindRecordingEpoch{context:context.clone(),authority}),DispatchFrame::RecordingEpochBound{challenge:c} if c==challenge)
    );
    challenge
}

#[test]
fn receipt_reclamation_checks_original_authority_after_sqlite_wait_and_rejects_old_epochs() {
    use qgenie_remote_proto::ipc::RecordingReceipt;
    let temp = directory();
    let path = temp.path().canonicalize().unwrap();
    let scope = scope();
    let runtime = Runtime::open(configured(&path, &scope)).unwrap();
    runtime
        .start_scoped(spec(&path, IdempotencyKey::new()), Some(scope.clone()))
        .unwrap();
    wait(|| runtime.readers_finished());
    let worker = runtime.recording_worker().unwrap();
    wait(|| worker.status_scoped(scope.clone()).unwrap().chunks > 0);
    let Some(SpoolItem::Content(batch)) = worker.fetch_scoped(scope.clone()).unwrap() else {
        panic!("content");
    };
    let count = worker.status_scoped(scope.clone()).unwrap().chunks;
    let context = access(&runtime, &scope, 1, 150);
    let challenge = bind_epoch(&runtime, &context, "epoch-one");
    let blocker = rusqlite::Connection::open(path.join("recordings/metadata.sqlite")).unwrap();
    blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let native = runtime.clone();
    let r = RecordingReceipt::Content(receipt(&batch, "epoch-one"));
    let waiting = thread::spawn(move || native.recording_acknowledge(context, challenge, r));
    thread::sleep(Duration::from_millis(180));
    blocker.execute_batch("COMMIT").unwrap();
    assert!(waiting.join().unwrap().is_err());
    assert_eq!(worker.status_scoped(scope.clone()).unwrap().chunks, count);
    let context = access(&runtime, &scope, 2, 3000);
    let challenge = bind_epoch(&runtime, &context, "epoch-one");
    blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let native = runtime.clone();
    let r = RecordingReceipt::Content(receipt(&batch, "epoch-one"));
    let waiting = thread::spawn(move || native.recording_acknowledge(context, challenge, r));
    thread::sleep(Duration::from_millis(30));
    let current = access(&runtime, &scope, 3, 3000);
    let pin = bind_epoch(&runtime, &current, "epoch-two");
    blocker.execute_batch("COMMIT").unwrap();
    assert!(waiting.join().unwrap().is_err());
    assert!(
        runtime
            .recording_acknowledge(
                current.clone(),
                pin,
                RecordingReceipt::Content(receipt(&batch, "epoch-one"))
            )
            .is_err()
    );
    assert_eq!(worker.status_scoped(scope.clone()).unwrap().chunks, count);
    let valid = RecordingReceipt::Content(receipt(&batch, "epoch-two"));
    assert!(matches!(
        runtime.recording_acknowledge(current.clone(), pin, valid.clone()),
        Ok(DispatchFrame::RecordingAcknowledged { reclaimed: true })
    ));
    assert!(matches!(
        runtime.recording_acknowledge(current, pin, valid),
        Ok(DispatchFrame::RecordingAcknowledged { reclaimed: false })
    ));
    runtime.shutdown(false).unwrap();
}

#[test]
fn original_native_end_survives_restart_and_remote_projection_without_claiming_recovered_lifetimes()
{
    let temp = directory();
    let path = temp.path().canonicalize().unwrap();
    let scope = scope();
    let config = configured(&path, &scope);
    let runtime = Runtime::open(config.clone()).unwrap();
    let info = runtime
        .start_scoped(spec(&path, IdempotencyKey::new()), Some(scope.clone()))
        .unwrap();
    wait(|| runtime.readers_finished());
    let observed = runtime
        .list()
        .unwrap()
        .into_iter()
        .find(|s| s.id == info.id)
        .unwrap()
        .ended_at_ms
        .expect("observed exit");
    runtime.shutdown(false).unwrap();
    drop(runtime);
    let runtime = Runtime::open_after_previous_owner(config).unwrap();
    let restored = runtime
        .list()
        .unwrap()
        .into_iter()
        .find(|s| s.id == info.id)
        .unwrap();
    assert_eq!(restored.ended_at_ms, Some(observed));
    let context = access(&runtime, &scope, 1, 3000);
    let mut message =
        qgenie_remote_proto::wire::Envelope::new(qgenie_remote_proto::wire::Message::SessionList);
    message.machine = Some(scope.machine);
    let DispatchFrame::RemoteReply(reply) = runtime.dispatch_frame(DispatchFrame::Remote {
        context,
        message: Box::new(message),
    }) else {
        panic!("remote list");
    };
    let qgenie_remote_proto::wire::Message::Sessions { sessions } = reply.message else {
        panic!("sessions");
    };
    assert_eq!(
        sessions[0].ended_at.as_ref().unwrap().unix_millis(),
        observed.0
    );
    runtime.shutdown(false).unwrap();
}

#[test]
fn native_scoped_capture_survives_viewer_absence_and_records_resize_without_no_echo_input() {
    let temp = directory();
    let path = temp.path().canonicalize().unwrap();
    let scope = scope();
    let runtime = Runtime::open(configured(&path, &scope)).unwrap();
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] =
        "stty -echo; printf READY; IFS= read -r password; printf 'visible-secret DONE\\n'".into();
    let info = runtime.start_scoped(launch, Some(scope.clone())).unwrap();
    assert!(info.recording_warning.is_none());
    wait(|| replay(&runtime, info.id).windows(5).any(|w| w == b"READY"));
    let client = ClientId::new();
    let generation = control(&runtime, client, info.id);
    assert!(matches!(
        call(
            &runtime,
            client,
            Operation::Resize {
                session: info.id,
                generation,
                rows: 30,
                cols: 100
            }
        ),
        Reply::Ok
    ));
    input(
        &runtime,
        client,
        info.id,
        generation,
        1,
        b"no-echo-password-93\n",
    );
    wait(|| runtime.readers_finished());
    let worker = runtime.recording_worker().unwrap();
    wait(|| worker.status_scoped(scope.clone()).unwrap().chunks > 0);
    assert_eq!(worker.status_scoped(scope.clone()).unwrap().gaps, 0);
    let mut output = Vec::new();
    let mut resized = false;
    while let Some(item) = worker.fetch_scoped(scope.clone()).unwrap() {
        let SpoolItem::Content(batch) = item else {
            panic!("complete native EOF should not create loss");
        };
        assert_eq!(batch.session, info.id);
        let raw =
            zstd::bulk::decompress(&STANDARD.decode(&batch.data).unwrap(), 1024 * 1024).unwrap();
        for line in raw.split(|b| *b == b'\n').filter(|v| !v.is_empty()) {
            let record: RecordingRecord = serde_json::from_slice(line).unwrap();
            record.validate(&batch.stream).unwrap();
            match record.content {
                RecordedContent::Output { data, .. } => {
                    output.extend(STANDARD.decode(data).unwrap())
                }
                RecordedContent::Resize {
                    rows: 30,
                    cols: 100,
                } => resized = true,
                _ => {}
            }
        }
        acknowledge(&worker, &scope, &batch);
    }
    assert!(resized && output.windows(4).any(|w| w == b"DONE"));
    for secret in [b"visible-secret".as_slice(), b"no-echo-password-93"] {
        assert!(!output.windows(secret.len()).any(|w| w == secret));
    }
    assert!(output.windows(10).any(|w| w == b"[REDACTED]"));
    assert!(
        replay(&runtime, info.id)
            .windows(14)
            .any(|w| w == b"visible-secret")
    );
    // Unenrolled local sessions retain their original preview scope and are never adopted.
    let local = runtime.start(spec(&path, IdempotencyKey::new())).unwrap();
    wait(|| runtime.readers_finished());
    assert!(local.recording_warning.unwrap().contains("not enrolled"));
    assert!(worker.fetch_scoped(scope).unwrap().is_none());
    runtime.shutdown(false).unwrap();
    drop(worker);
    drop(runtime);
}

#[test]
fn native_input_and_output_continue_while_the_recording_database_is_locked() {
    let temp = directory();
    let path = temp.path().canonicalize().unwrap();
    let scope = scope();
    let runtime = Runtime::open(configured(&path, &scope)).unwrap();
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] = "stty -echo; printf READY; read first; printf CONTROLLED; read last".into();
    let info = runtime.start_scoped(launch, Some(scope.clone())).unwrap();
    wait(|| replay(&runtime, info.id).windows(5).any(|w| w == b"READY"));
    let worker = runtime.recording_worker().unwrap();
    let blocker = rusqlite::Connection::open(path.join("recordings/metadata.sqlite")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let client = ClientId::new();
    let generation = control(&runtime, client, info.id);
    let began = Instant::now();
    input(&runtime, client, info.id, generation, 1, b"first\n");
    wait(|| {
        replay(&runtime, info.id)
            .windows(10)
            .any(|w| w == b"CONTROLLED")
    });
    assert!(began.elapsed() < Duration::from_secs(1));
    // The lock stays held until after the independently observable native effect.
    thread::sleep(Duration::from_millis(200));
    blocker.execute_batch("COMMIT").unwrap();
    input(&runtime, client, info.id, generation, 2, b"last\n");
    wait(|| runtime.readers_finished());
    wait(|| worker.status_scoped(scope.clone()).unwrap().gaps > 0);
    let SpoolItem::Gap(gap) = worker.fetch_scoped(scope).unwrap().unwrap() else {
        panic!("storage failure needs explicit loss");
    };
    assert!(matches!(
        gap.reason,
        LossReason::RecoveryLoss | LossReason::CaptureBackpressure
    ));
    runtime.shutdown(false).unwrap();
    drop(worker);
    drop(runtime);
}

#[test]
fn failed_recording_initialization_does_not_launch_a_scoped_child_or_block_local_preview() {
    let temp = directory();
    let path = temp.path().canonicalize().unwrap();
    let scope = scope();
    std::fs::write(path.join("recordings"), b"owned test obstruction").unwrap();
    let runtime = Runtime::open(configured(&path, &scope)).unwrap();
    let launch = spec(&path, IdempotencyKey::new());
    let failed = runtime
        .start_scoped(launch.clone(), Some(scope.clone()))
        .unwrap();
    assert!(matches!(failed.state, SessionState::Failed));
    assert!(!path.join("native-marker").exists());
    assert_eq!(
        runtime.start_scoped(launch, Some(scope)).unwrap().id,
        failed.id
    );
    runtime.start(spec(&path, IdempotencyKey::new())).unwrap();
    wait(|| runtime.readers_finished());
    assert_eq!(std::fs::read(path.join("native-marker")).unwrap(), b"x");
    runtime.shutdown(false).unwrap();
}

#[test]
fn inherited_terminal_tail_records_the_platform_eof_boundary() {
    let temp = directory();
    let path = temp.path().canonicalize().unwrap();
    let scope = scope();
    let runtime = Runtime::open(configured(&path, &scope)).unwrap();
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] = "trap '' HUP; sleep 3 & printf DONE; exit 0".into();
    let began = Instant::now();
    let info = runtime.start_scoped(launch, Some(scope.clone())).unwrap();
    wait(|| runtime.readers_finished());
    assert!(began.elapsed() < Duration::from_secs(2));
    let worker = runtime.recording_worker().unwrap();
    #[cfg(target_os = "macos")]
    {
        // Darwin reports EOF when the controlling session leader exits, even
        // while the background process retains its inherited descriptor.
        wait(|| worker.status_scoped(scope.clone()).unwrap().chunks > 0);
        assert_eq!(worker.status_scoped(scope.clone()).unwrap().gaps, 0);
        let mut output = Vec::new();
        while let Some(item) = worker.fetch_scoped(scope.clone()).unwrap() {
            let SpoolItem::Content(batch) = item else {
                panic!("observed EOF needs complete content");
            };
            assert_eq!(batch.session, info.id);
            let raw = zstd::bulk::decompress(&STANDARD.decode(&batch.data).unwrap(), 1024 * 1024)
                .unwrap();
            for line in raw
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
            {
                let record: RecordingRecord = serde_json::from_slice(line).unwrap();
                record.validate(&batch.stream).unwrap();
                if let RecordedContent::Output { data, .. } = record.content {
                    output.extend(STANDARD.decode(data).unwrap());
                }
            }
            acknowledge(&worker, &scope, &batch);
        }
        assert_eq!(output, b"DONE");
        assert!(
            runtime
                .list()
                .unwrap()
                .iter()
                .find(|session| session.id == info.id)
                .unwrap()
                .recording_warning
                .is_none()
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        wait(|| worker.status_scoped(scope.clone()).unwrap().gaps > 0);
        let SpoolItem::Gap(gap) = worker.fetch_scoped(scope).unwrap().unwrap() else {
            panic!("unobserved EOF needs loss");
        };
        assert!(matches!(gap.reason, LossReason::RecoveryLoss));
        assert_eq!(gap.session, info.id);
    }
    runtime.shutdown(false).unwrap();
    drop(worker);
    drop(runtime);
}

#[test]
fn native_spool_cap_warning_survives_restart_and_does_not_fence_live_control() {
    let temp = directory();
    let path = temp.path().canonicalize().unwrap();
    let scope = scope();
    let mut config = configured(&path, &scope);
    config.recording.content_bytes = 8 * 1024;
    let runtime = Runtime::open(config.clone()).unwrap();
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] = "stty -echo; head -c 200000 /dev/urandom; printf HOLD; read done".into();
    let info = runtime.start_scoped(launch, Some(scope.clone())).unwrap();
    let worker = runtime.recording_worker().unwrap();
    wait(|| worker.warning_seen());
    assert!(worker.status_scoped(scope.clone()).unwrap().gaps > 0);
    assert!(
        runtime
            .list()
            .unwrap()
            .iter()
            .find(|v| v.id == info.id)
            .unwrap()
            .recording_warning
            .is_some()
    );
    let client = ClientId::new();
    let generation = control(&runtime, client, info.id);
    input(&runtime, client, info.id, generation, 1, b"done\n");
    wait(|| runtime.readers_finished());
    runtime.shutdown(false).unwrap();
    drop(worker);
    drop(runtime);
    let reopened = Runtime::open_after_previous_owner(config).unwrap();
    assert!(
        reopened
            .list()
            .unwrap()
            .iter()
            .find(|v| v.id == info.id)
            .unwrap()
            .recording_warning
            .is_some()
    );
    assert!(reopened.recording_worker().unwrap().warning_seen());
    reopened.shutdown(false).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_recording_ipc_requires_capability_scope_and_independent_epoch_before_reclaim() {
    use qgenie_remote_proto::ipc::RecordingUpload;
    use qgenie_remote_transport::ipc::{
        client_handshake_with_capabilities, connect, read_dispatch, write_dispatch,
    };
    let temp = directory();
    let path = temp.path().canonicalize().unwrap();
    let scope = scope();
    let runtime = Runtime::open(configured(&path, &scope)).unwrap();
    let info = runtime
        .start_scoped(spec(&path, IdempotencyKey::new()), Some(scope.clone()))
        .unwrap();
    wait(|| runtime.readers_finished());
    let worker = runtime.recording_worker().unwrap();
    wait(|| worker.status_scoped(scope.clone()).unwrap().chunks > 0);
    let context = access(&runtime, &scope, 1, 3000);
    let server = tokio::spawn(crate::server::serve_existing(runtime.clone(), path.clone()));
    let until = Instant::now() + Duration::from_secs(2);
    while !path.join("dispatch.sock").exists() {
        assert!(Instant::now() < until);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    async fn rpc(path: &std::path::Path, frame: DispatchFrame, recording: bool) -> DispatchFrame {
        let mut socket = connect(&path.join("dispatch.sock")).await.unwrap();
        let reclaim = matches!(
            &frame,
            DispatchFrame::RecordingEpochChallenge { .. }
                | DispatchFrame::BindRecordingEpoch { .. }
                | DispatchFrame::RecordingAcknowledge { .. }
        );
        let required: &[&str] = if recording && reclaim {
            &[
                "native-authority-v1",
                "recording-read-v1",
                "recording-reclaim-v1",
            ]
        } else if recording {
            &["native-authority-v1", "recording-read-v1"]
        } else {
            &["native-authority-v1"]
        };
        client_handshake_with_capabilities(&mut socket, required)
            .await
            .unwrap();
        write_dispatch(&mut socket, &frame).await.unwrap();
        read_dispatch(&mut socket).await.unwrap()
    }
    let frame = DispatchFrame::RecordingFetch {
        context: context.clone(),
        max_bytes: 64 * 1024,
    };
    assert!(matches!(
        rpc(&path, frame.clone(), false).await,
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::UnsupportedCapability,
            ..
        })
    ));
    let DispatchFrame::RecordingFetched(Some(RecordingUpload::Content(batch))) =
        rpc(&path, frame, true).await
    else {
        panic!("scoped native content");
    };
    assert_eq!(batch.session, info.id);
    assert_eq!(batch.machine, scope.machine);
    let mut foreign = context.clone();
    foreign.scope.owner = UserId::new();
    foreign.actor = foreign.scope.owner;
    assert!(matches!(
        rpc(
            &path,
            DispatchFrame::RecordingStatus { context: foreign },
            true
        )
        .await,
        DispatchFrame::Error(_)
    ));
    let receipt = CommitReceipt {
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
        recovery_epoch: "native-test".into(),
        disposition: ReceiptDisposition::Stored,
        committed_at: Timestamp::from_unix_millis(now_ms()).unwrap(),
    };
    assert!(matches!(
        rpc(&path, DispatchFrame::RecordingCommit(receipt.clone()), true).await,
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::UnsupportedCapability,
            ..
        })
    ));
    let DispatchFrame::RecordingStatusReply(status) = rpc(
        &path,
        DispatchFrame::RecordingStatus {
            context: context.clone(),
        },
        true,
    )
    .await
    else {
        panic!("status");
    };
    assert!(
        status.chunks.0 > 0,
        "a receipt body cannot authenticate its own recovery epoch"
    );
    let frame = DispatchFrame::RecordingEpochChallenge {
        context: context.clone(),
    };
    assert!(matches!(
        rpc(&path, frame.clone(), false).await,
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::UnsupportedCapability,
            ..
        })
    ));
    let DispatchFrame::RecordingEpochChallengeIssued { challenge } = rpc(&path, frame, true).await
    else {
        panic!("epoch challenge");
    };
    let authority = qgenie_remote_proto::rest::RecordingAuthority {
        challenge,
        origin: scope.origin.clone(),
        owner: scope.owner,
        machine: scope.machine,
        key: KeyId::new(),
        backend_epoch: "native-test".into(),
    };
    assert!(matches!(
        rpc(
            &path,
            DispatchFrame::BindRecordingEpoch {
                context: context.clone(),
                authority
            },
            true
        )
        .await,
        DispatchFrame::RecordingEpochBound { .. }
    ));
    assert!(matches!(
        rpc(
            &path,
            DispatchFrame::RecordingAcknowledge {
                context,
                challenge,
                receipt: qgenie_remote_proto::ipc::RecordingReceipt::Content(receipt)
            },
            true
        )
        .await,
        DispatchFrame::RecordingAcknowledged { reclaimed: true }
    ));
    runtime.shutdown(false).unwrap();
    server.abort();
    let _ = server.await;
    drop(worker);
    drop(runtime);
}

#[test]
fn recording_io_wait_cannot_extend_access_or_hold_the_authority_lock_through_replacement() {
    let temp = directory();
    let path = temp.path().canonicalize().unwrap();
    let scope = scope();
    let runtime = Runtime::open(configured(&path, &scope)).unwrap();
    runtime.recording_worker().unwrap();
    let context = access(&runtime, &scope, 1, 50);
    let blocker = rusqlite::Connection::open(path.join("recordings/metadata.sqlite")).unwrap();
    blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    assert!(matches!(
        runtime.recording_read(&context, None),
        Err(RemoteError {
            code: ErrorCode::AccessStale,
            ..
        })
    ));
    blocker.execute_batch("COMMIT").unwrap();
    let context = access(&runtime, &scope, 2, 3000);
    blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let native = runtime.clone();
    let waiting = thread::spawn(move || native.recording_read(&context, None));
    thread::sleep(Duration::from_millis(20));
    let renewed = access(&runtime, &scope, 3, 3000);
    blocker.execute_batch("COMMIT").unwrap();
    assert!(matches!(
        waiting.join().unwrap(),
        Err(RemoteError {
            code: ErrorCode::StaleGeneration,
            ..
        })
    ));
    assert!(matches!(
        runtime.recording_read(&renewed, None),
        Ok(DispatchFrame::RecordingStatusReply(_))
    ));
    runtime.shutdown(false).unwrap();
}
