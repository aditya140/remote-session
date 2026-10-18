use super::*;
use crate::runtime::tests::{call, directory, spec};
use qgenie_remote_proto::ipc::HostActivation;

fn fixture() -> (tempfile::TempDir, Arc<Runtime>, RemoteScope, SessionId) {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let scope = RemoteScope {
        origin: "https://gateway.example.invalid".into(),
        owner: UserId::new(),
        machine: MachineId::new(),
    };
    let mut config = Config::new(path.clone(), vec![path.clone()]);
    config.remote_bindings = vec![scope.clone()];
    let runtime = Runtime::open(config).unwrap();
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] = "read value; printf %s \"$value\" > native-marker".into();
    let session = runtime
        .start_scoped(launch, Some(scope.clone()))
        .unwrap()
        .id;
    (temp, runtime, scope, session)
}
fn access(
    runtime: &Arc<Runtime>,
    scope: &RemoteScope,
    connection: ClientId,
    host: u64,
) -> RemoteContext {
    let DispatchFrame::AccessChallengeIssued { nonce, .. } =
        runtime.dispatch_frame(DispatchFrame::AccessChallenge {
            scope: scope.clone(),
            connection,
            host_generation: Counter(host),
        })
    else {
        panic!("challenge");
    };
    let activation = HostActivation {
        scope: scope.clone(),
        connection,
        generation: Counter(host),
        nonce,
        remaining_ms: 10_000,
        policy_revision: Counter(1),
    };
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateHost(activation)),
        DispatchFrame::HostActivated { .. }
    ));
    RemoteContext {
        actor: scope.owner,
        client: ClientId::new(),
        scope: scope.clone(),
        connection,
        host_generation: Counter(host),
        controller_generation: None,
        access_nonce: nonce,
        remaining_access_ms: 10_000,
        policy_revision: Counter(1),
    }
}
fn proposal(
    context: &RemoteContext,
    session: SessionId,
    holder: ClientId,
    generation: u64,
    takeover: bool,
) -> ControlActivation {
    let mut context = context.clone();
    context.client = holder;
    context.controller_generation = Some(Counter(generation));
    ControlActivation {
        context,
        session,
        holder,
        generation: Counter(generation),
        remaining_lease_ms: 15_000,
        takeover,
        renewal: false,
    }
}
fn activated(frame: DispatchFrame, generation: u64) -> u32 {
    let DispatchFrame::ControlActivated {
        generation: actual,
        remaining_ms,
        ..
    } = frame
    else {
        panic!("control activation: {frame:?}");
    };
    assert_eq!(actual, Counter(generation));
    assert!(remaining_ms > 0 && remaining_ms <= 15_000);
    remaining_ms
}
#[test]
fn durable_controller_takeover_reconciliation_host_replacement_and_no_revival() {
    let (_temp, runtime, scope, session) = fixture();
    let config = runtime.config.clone();
    let host = ClientId::new();
    let first = access(&runtime, &scope, host, 1);
    let a = ClientId::new();
    let b = ClientId::new();
    let one = proposal(&first, session, a, 1, false);
    activated(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(one.clone())),
        1,
    );
    let before = runtime.session(session).unwrap().lock().unwrap().deadline;
    activated(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(one.clone())),
        1,
    );
    assert_eq!(
        runtime.session(session).unwrap().lock().unwrap().deadline,
        before
    );
    let mut two = proposal(&first, session, b, 2, false);
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(two.clone())),
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::ControlHeld,
            ..
        })
    ));
    two.takeover = true;
    activated(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(two.clone())),
        2,
    );
    assert!(matches!(
        call(
            &runtime,
            a,
            Operation::Input {
                session,
                generation: Counter(1),
                input_seq: Counter(1),
                data: STANDARD.encode(b"denied\n")
            }
        ),
        Reply::Error(RemoteError {
            code: ErrorCode::StaleGeneration,
            ..
        })
    ));
    let mut old = two.clone();
    old.renewal = true;
    let replacement = access(&runtime, &scope, ClientId::new(), 2);
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(old)),
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::StaleGeneration,
            ..
        })
    ));
    let rebound = proposal(&replacement, session, b, 2, false);
    let before = runtime.session(session).unwrap().lock().unwrap().deadline;
    activated(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(rebound.clone())),
        2,
    );
    assert_eq!(
        runtime.session(session).unwrap().lock().unwrap().deadline,
        before
    );
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::DeactivateControl {
            context: rebound.context.clone(),
            session,
            generation: Counter(2)
        }),
        DispatchFrame::ControlDeactivated { .. }
    ));
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(rebound)),
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::StaleGeneration,
            ..
        })
    ));
    let max = proposal(&replacement, session, b, u64::MAX, false);
    activated(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(max.clone())),
        u64::MAX,
    );
    assert!(
        matches!(runtime.dispatch_frame(DispatchFrame::QueryControl {context:replacement.clone(),session}),DispatchFrame::ControlState {floor:Counter(u64::MAX),holder:Some(holder),..} if holder==b)
    );
    let local = ClientId::new();
    assert!(matches!(
        call(
            &runtime,
            local,
            Operation::TakeControl {
                session,
                takeover: true
            }
        ),
        Reply::Error(RemoteError {
            code: ErrorCode::CapacityExhausted,
            ..
        })
    ));
    // Even the same client ID cannot route a remote lease through local IPC.
    assert!(matches!(
        call(
            &runtime,
            b,
            Operation::Input {
                session,
                generation: Counter(u64::MAX),
                input_seq: Counter(1),
                data: STANDARD.encode(b"denied\n")
            }
        ),
        Reply::Error(_)
    ));
    let mut context = replacement.clone();
    context.client = b;
    context.controller_generation = Some(Counter(u64::MAX));
    assert!(
        matches!(effect(&runtime,&context,session,qgenie_remote_proto::wire::Message::Input{
        authority:qgenie_remote_proto::wire::Generations{host:context.host_generation,controller:context.controller_generation},input_seq:Counter(1),input:qgenie_remote_proto::wire::SessionInput::Pty(STANDARD.encode(b"allowed\n"))
    }),DispatchFrame::RemoteReply(reply) if matches!(reply.message,qgenie_remote_proto::wire::Message::InputAck{outcome:ApplicationOutcome::Applied,..}))
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while !runtime.readers_finished() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        std::fs::read(runtime.config.state_dir.join("native-marker")).unwrap(),
        b"allowed"
    );
    drop(runtime);
    let reopened = Runtime::open_after_previous_owner(config).unwrap();
    let access = access(&reopened, &scope, ClientId::new(), 3);
    assert!(matches!(
        reopened.dispatch_frame(DispatchFrame::QueryControl {
            context: access,
            session
        }),
        DispatchFrame::ControlState {
            floor: Counter(u64::MAX),
            holder: None,
            remaining_ms: 0,
            ..
        }
    ));
}

#[test]
fn sqlite_delay_commits_the_floor_but_cannot_activate_expired_access() {
    let (_temp, runtime, scope, session) = fixture();
    let host = ClientId::new();
    let first = access(&runtime, &scope, host, 1);
    let a = ClientId::new();
    let b = ClientId::new();
    activated(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(proposal(
            &first, session, a, 1, false,
        ))),
        1,
    );
    let mut recent = access(&runtime, &scope, host, 1);
    recent.remaining_access_ms = 100;
    let blocker = rusqlite::Connection::open(runtime.config.state_dir.join("sessions.db")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let owned = runtime.clone();
    let two = proposal(&recent, session, b, 2, true);
    let task = thread::spawn(move || owned.dispatch_frame(DispatchFrame::ActivateControl(two)));
    thread::sleep(Duration::from_millis(250));
    blocker.execute_batch("COMMIT").unwrap();
    assert!(matches!(
        task.join().unwrap(),
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::AccessStale,
            ..
        })
    ));
    let state = runtime.session(session).unwrap();
    assert_eq!(state.lock().unwrap().generation, 2);
    assert!(state.lock().unwrap().info.controller.is_none());
    let floor: Vec<u8> = blocker
        .query_row(
            "SELECT generation FROM sessions WHERE id=?1",
            [session.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(u64::from_be_bytes(floor.try_into().unwrap()), 2);
    assert!(!runtime.config.state_dir.join("native-marker").exists());
    let fresh = access(&runtime, &scope, host, 1);
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(proposal(
            &fresh, session, b, 2, false
        ))),
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::StaleGeneration,
            ..
        })
    ));
    activated(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(proposal(
            &fresh, session, b, 3, false,
        ))),
        3,
    );
    runtime.shutdown(true).unwrap();
}

#[tokio::test]
async fn real_ipc_controller_capability_is_required_and_ack_follows_the_floor_commit() {
    use qgenie_remote_proto::ipc::{AdminCommand, AdminRequest, AdminResponse};
    use qgenie_remote_transport::ipc::{
        client_handshake_with_capabilities, connect, read_dispatch, read_frame, write_dispatch,
        write_frame,
    };
    let (_temp, runtime, scope, session) = fixture();
    let context = access(&runtime, &scope, ClientId::new(), 1);
    let holder = ClientId::new();
    let path = runtime.config.state_dir.clone();
    struct Service(Option<tokio::task::JoinHandle<anyhow::Result<()>>>);
    impl Drop for Service {
        fn drop(&mut self) {
            if let Some(task) = &self.0 {
                task.abort();
            }
        }
    }
    let mut service = Service(Some(tokio::spawn(crate::server::serve_existing(
        runtime.clone(),
        path.clone(),
    ))));
    tokio::time::timeout(Duration::from_secs(3), async {
        while connect(&path.join("dispatch.sock")).await.is_err() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    async fn rpc(path: &std::path::Path, controller: bool, frame: DispatchFrame) -> DispatchFrame {
        let mut stream = connect(&path.join("dispatch.sock")).await.unwrap();
        let caps: &[&str] = if controller {
            &["native-authority-v1", "native-controller-v1"]
        } else {
            &["native-authority-v1"]
        };
        client_handshake_with_capabilities(&mut stream, caps)
            .await
            .unwrap();
        write_dispatch(&mut stream, &frame).await.unwrap();
        read_dispatch(&mut stream).await.unwrap()
    }
    let proposal = proposal(&context, session, holder, 1, false);
    assert!(matches!(
        rpc(
            &path,
            false,
            DispatchFrame::ActivateControl(proposal.clone())
        )
        .await,
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::UnsupportedCapability,
            ..
        })
    ));
    assert!(matches!(
        rpc(
            &path,
            true,
            DispatchFrame::QueryControl {
                context: context.clone(),
                session
            }
        )
        .await,
        DispatchFrame::ControlState {
            floor: Counter(0),
            holder: None,
            ..
        }
    ));
    activated(
        rpc(
            &path,
            true,
            DispatchFrame::ActivateControl(proposal.clone()),
        )
        .await,
        1,
    );
    let db = rusqlite::Connection::open(path.join("sessions.db")).unwrap();
    let bytes: Vec<u8> = db
        .query_row(
            "SELECT generation FROM sessions WHERE id=?1",
            [session.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(u64::from_be_bytes(bytes.try_into().unwrap()), 1);
    let mut message =
        qgenie_remote_proto::wire::Envelope::new(input(&proposal.context, 1, b"allowed\n"));
    message.machine = Some(scope.machine);
    message.session = Some(session);
    let request = DispatchFrame::Remote {
        context: proposal.context.clone(),
        message: Box::new(message),
    };
    assert!(
        matches!(
            rpc(&path, true, request.clone()).await,
            DispatchFrame::Error(RemoteError {
                code: ErrorCode::UnsupportedCapability,
                ..
            })
        ),
        "controller negotiation alone cannot execute an effect"
    );
    let mut socket = connect(&path.join("dispatch.sock")).await.unwrap();
    client_handshake_with_capabilities(
        &mut socket,
        &[
            "native-authority-v1",
            "native-controller-v1",
            "native-effects-v1",
        ],
    )
    .await
    .unwrap();
    write_dispatch(&mut socket, &request).await.unwrap();
    assert!(
        matches!(read_dispatch(&mut socket).await.unwrap(),DispatchFrame::RemoteReply(reply) if matches!(reply.message,qgenie_remote_proto::wire::Message::InputAck{outcome:ApplicationOutcome::Applied,..}))
    );
    assert!(matches!(
        rpc(
            &path,
            true,
            DispatchFrame::DeactivateControl {
                context: proposal.context,
                session,
                generation: Counter(1)
            }
        )
        .await,
        DispatchFrame::ControlDeactivated { .. }
    ));
    let mut admin = connect(&path.join("admin.sock")).await.unwrap();
    write_frame(
        &mut admin,
        &serde_json::to_vec(&AdminRequest {
            ipc_major: IPC_MAJOR,
            command: AdminCommand::Shutdown {
                terminate_sessions: true,
            },
        })
        .unwrap(),
    )
    .await
    .unwrap();
    decode_json::<AdminResponse>(&read_frame(&mut admin).await.unwrap())
        .unwrap()
        .result
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), service.0.take().unwrap())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

fn effect(
    runtime: &Arc<Runtime>,
    context: &RemoteContext,
    session: SessionId,
    body: qgenie_remote_proto::wire::Message,
) -> DispatchFrame {
    let mut message = qgenie_remote_proto::wire::Envelope::new(body);
    message.machine = Some(context.scope.machine);
    message.session = Some(session);
    runtime.dispatch_frame(DispatchFrame::Remote {
        context: context.clone(),
        message: Box::new(message),
    })
}
fn input(
    context: &RemoteContext,
    sequence: u64,
    bytes: &[u8],
) -> qgenie_remote_proto::wire::Message {
    qgenie_remote_proto::wire::Message::Input {
        authority: qgenie_remote_proto::wire::Generations {
            host: context.host_generation,
            controller: context.controller_generation,
        },
        input_seq: Counter(sequence),
        input: qgenie_remote_proto::wire::SessionInput::Pty(STANDARD.encode(bytes)),
    }
}
#[test]
fn original_remote_input_fences_host_takeover_and_deduplicates_native_delivery() {
    let (_temp, runtime, scope, session) = fixture();
    let first = access(&runtime, &scope, ClientId::new(), 1);
    let a = ClientId::new();
    let one = proposal(&first, session, a, 1, false);
    let old = one.context.clone();
    activated(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(one)),
        1,
    );
    let fresh = access(&runtime, &scope, ClientId::new(), 2);
    let b = ClientId::new();
    let two = proposal(&fresh, session, b, 2, true);
    let current = two.context.clone();
    activated(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(two)),
        2,
    );
    assert!(matches!(
        effect(&runtime, &old, session, input(&old, 1, b"stale-host\n")),
        DispatchFrame::Error(_)
    ));
    let mut previous = fresh.clone();
    previous.client = a;
    previous.controller_generation = Some(Counter(1));
    assert!(matches!(
        effect(
            &runtime,
            &previous,
            session,
            input(&previous, 1, b"stale-controller\n")
        ),
        DispatchFrame::Error(_)
    ));
    for _ in 0..2 {
        assert!(
            matches!(effect(&runtime,&current,session,input(&current,1,b"allowed")),DispatchFrame::RemoteReply(reply) if matches!(reply.message,qgenie_remote_proto::wire::Message::InputAck{outcome:ApplicationOutcome::Applied,..}))
        );
    }
    let mut changed = input(&current, 1, b"changed");
    assert!(matches!(
        effect(&runtime, &current, session, changed.clone()),
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::IdempotencyConflict,
            ..
        })
    ));
    changed = input(&current, 2, b"\n");
    assert!(matches!(
        effect(&runtime, &current, session, changed),
        DispatchFrame::RemoteReply(_)
    ));
    let until = Instant::now() + Duration::from_secs(3);
    while !runtime.readers_finished() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        std::fs::read(runtime.config.state_dir.join("native-marker")).unwrap(),
        b"allowed"
    );
}
#[test]
fn original_access_expiring_during_remote_input_intent_never_writes_native_bytes() {
    let (_temp, runtime, scope, session) = fixture();
    let host = ClientId::new();
    let context = access(&runtime, &scope, host, 1);
    let holder = ClientId::new();
    let active = proposal(&context, session, holder, 1, false);
    let mut context = active.context.clone();
    activated(
        runtime.dispatch_frame(DispatchFrame::ActivateControl(active)),
        1,
    );
    context.remaining_access_ms = 100;
    let blocker = rusqlite::Connection::open(runtime.config.state_dir.join("sessions.db")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let owned = runtime.clone();
    let request = context.clone();
    let task = thread::spawn(move || {
        effect(
            &owned,
            &request,
            session,
            input(&request, 1, b"must-not-apply\n"),
        )
    });
    thread::sleep(Duration::from_millis(250));
    blocker.execute_batch("COMMIT").unwrap();
    assert!(matches!(
        task.join().unwrap(),
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::AccessStale,
            ..
        })
    ));
    assert!(!runtime.config.state_dir.join("native-marker").exists());
    let stored: String = blocker
        .query_row(
            "SELECT outcome FROM input_ack WHERE session_id=?1",
            [session.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(matches!(
        serde_json::from_str::<ApplicationOutcome>(&stored).unwrap(),
        ApplicationOutcome::Cancelled
    ));
    runtime.shutdown(true).unwrap();
}
