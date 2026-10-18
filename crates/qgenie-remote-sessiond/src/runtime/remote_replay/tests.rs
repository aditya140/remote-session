use super::*;
use crate::runtime::tests::{call, directory, spec};
use qgenie_remote_proto::ipc::{ControlActivation, HostActivation};
fn fixture() -> (tempfile::TempDir, Arc<Runtime>, RemoteContext, SessionId) {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let scope = RemoteScope {
        origin: "https://replay.example.invalid".into(),
        owner: UserId::new(),
        machine: MachineId::new(),
    };
    let mut config = Config::new(path.clone(), vec![path.clone()]);
    config.remote_bindings = vec![scope.clone()];
    let runtime = Runtime::open(config).unwrap();
    let connection = ClientId::new();
    let DispatchFrame::AccessChallengeIssued { nonce, .. } =
        runtime.dispatch_frame(DispatchFrame::AccessChallenge {
            scope: scope.clone(),
            connection,
            host_generation: Counter(1),
        })
    else {
        panic!("challenge")
    };
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateHost(HostActivation {
            scope: scope.clone(),
            connection,
            generation: Counter(1),
            nonce,
            remaining_ms: 10000,
            policy_revision: Counter(1)
        })),
        DispatchFrame::HostActivated { .. }
    ));
    let context = RemoteContext {
        actor: scope.owner,
        client: ClientId::new(),
        scope,
        connection,
        host_generation: Counter(1),
        controller_generation: None,
        access_nonce: nonce,
        remaining_access_ms: 10000,
        policy_revision: Counter(1),
    };
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] = "read value".into();
    launch.rows = 128;
    launch.cols = 512;
    let session = runtime
        .start_scoped(launch, Some(context.scope.clone()))
        .unwrap()
        .id;
    (temp, runtime, context, session)
}
fn replay(
    runtime: &Arc<Runtime>,
    context: &RemoteContext,
    session: SessionId,
    after: Counter,
) -> DispatchFrame {
    let mut request = wire::Envelope::new(Message::Attach { after });
    request.machine = Some(context.scope.machine);
    request.session = Some(session);
    runtime.dispatch_frame(DispatchFrame::Remote {
        context: context.clone(),
        message: Box::new(request),
    })
}
#[test]
fn remote_checkpoint_pages_keep_one_capture_and_original_owner_scope() {
    let (_temp, runtime, context, session) = fixture();
    let owned = runtime.session(session).unwrap();
    runtime.output(&owned, b"before checkpoint");
    let expected = runtime.snapshot(&owned.lock().unwrap()).unwrap().data;
    let DispatchFrame::SnapshotRead {
        snapshot: first, ..
    } = runtime.dispatch_frame(DispatchFrame::ReadSnapshot {
        context: context.clone(),
        session,
        epoch: None,
        page: 0,
    })
    else {
        panic!("checkpoint")
    };
    assert!(first.total_pages > 1);
    let epoch: MessageId = first.epoch.parse().unwrap();
    let mut bytes = STANDARD.decode(&first.data).unwrap();
    runtime.output(&owned, b" after checkpoint");
    let mut foreign = context.clone();
    foreign.actor = UserId::new();
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ReadSnapshot {
            context: foreign,
            session,
            epoch: Some(epoch),
            page: 1
        }),
        DispatchFrame::Error(_)
    ));
    for page in 1..first.total_pages {
        let DispatchFrame::SnapshotRead { snapshot, .. } =
            runtime.dispatch_frame(DispatchFrame::ReadSnapshot {
                context: context.clone(),
                session,
                epoch: Some(epoch),
                page,
            })
        else {
            panic!("fixed page")
        };
        assert_eq!(snapshot.seq, first.seq);
        assert_eq!(snapshot.epoch, first.epoch);
        bytes.extend(STANDARD.decode(snapshot.data).unwrap());
    }
    assert!(
        bytes == expected,
        "capture pages changed after native output"
    );
    assert!(bytes != runtime.snapshot(&owned.lock().unwrap()).unwrap().data);
    Screen::from_checkpoint(&bytes).unwrap();
    let DispatchFrame::RemoteReply(reply) = replay(&runtime, &context, session, first.seq) else {
        panic!("tail")
    };
    assert!(
        matches!(reply.message,Message::Replay{history_truncated:false,events,..} if events.iter().any(|e|matches!(&e.event,wire::AgentEvent::PtyOutput{data} if STANDARD.decode(data).unwrap().windows(16).any(|v|v==b"after checkpoint"))))
    );
    runtime.shutdown(true).unwrap();
}
#[test]
fn sqlite_gap_commit_cannot_renew_original_replay_access() {
    let (_temp, runtime, mut context, session) = fixture();
    context.remaining_access_ms = 1000;
    runtime
        .session(session)
        .unwrap()
        .lock()
        .unwrap()
        .pending_lost_bytes = 32;
    let blocker = rusqlite::Connection::open(runtime.config.state_dir.join("sessions.db")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let current = runtime.clone();
    let request = context.clone();
    let task = thread::spawn(move || replay(&current, &request, session, Counter(0)));
    let until = Instant::now() + Duration::from_secs(1);
    while runtime.store.try_lock().is_ok() {
        assert!(
            Instant::now() < until,
            "replay did not enter its SQLite gap write"
        );
        thread::sleep(Duration::from_millis(1));
    }
    thread::sleep(Duration::from_millis(1200));
    blocker.execute_batch("COMMIT").unwrap();
    assert!(matches!(
        task.join().unwrap(),
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::AccessStale,
            ..
        })
    ));
    runtime.shutdown(true).unwrap();
}
#[test]
fn local_and_remote_replay_do_not_renew_a_remote_controller() {
    let (_temp, runtime, context, session) = fixture();
    let mut active = context.clone();
    active.controller_generation = Some(Counter(1));
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(ControlActivation {
            context: active,
            session,
            holder: context.client,
            generation: Counter(1),
            remaining_lease_ms: 15000,
            takeover: false,
            renewal: false
        })),
        DispatchFrame::ControlActivated { .. }
    ));
    let owned = runtime.session(session).unwrap();
    let deadline = owned.lock().unwrap().deadline;
    thread::sleep(Duration::from_millis(10));
    assert!(matches!(
        replay(&runtime, &context, session, Counter(0)),
        DispatchFrame::RemoteReply(_)
    ));
    assert!(matches!(
        call(
            &runtime,
            context.client,
            Operation::Replay {
                session,
                after: Counter(0)
            }
        ),
        Reply::Replay(_)
    ));
    assert_eq!(owned.lock().unwrap().deadline, deadline);
    runtime.shutdown(true).unwrap();
}
