use super::*;
use qgenie_remote_proto::{
    ChallengeId, ClientId, Counter, IdempotencyKey, MachineId, Operation, Reply, Request,
    RequestId, SessionId, UserId,
    ipc::{HostActivation, RemoteContext, RemoteScope},
    wire::{Envelope, Generations, Launch, LaunchKind, Message, SessionInput},
};
use qgenie_remote_transport::ipc::client_handshake_with_options;
use std::os::unix::fs::PermissionsExt;

fn fixture() -> (tempfile::TempDir, Arc<Runtime>, RemoteContext) {
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = directory.path().canonicalize().unwrap();
    let scope = RemoteScope {
        origin: "https://gateway.example.test".into(),
        owner: UserId::new(),
        machine: MachineId::new(),
    };
    let mut config = Config::new(path.clone(), vec![path]);
    config.remote_bindings.push(scope.clone());
    let runtime = Runtime::open(config).unwrap();
    let context = RemoteContext {
        actor: scope.owner,
        client: ClientId::new(),
        scope,
        connection: ClientId::new(),
        host_generation: Counter(1),
        controller_generation: Some(Counter(1)),
        access_nonce: ChallengeId::new(),
        remaining_access_ms: 1000,
        policy_revision: Counter(1),
    };
    (directory, runtime, context)
}

async fn exchange(
    runtime: Arc<Runtime>,
    frame: DispatchFrame,
    capabilities: &[&str],
) -> DispatchFrame {
    let (server, mut peer) = UnixStream::pair().unwrap();
    let permits = Arc::new(Semaphore::new(1));
    let task = tokio::spawn(serve_data(
        server,
        runtime,
        permits.try_acquire_owned().unwrap(),
        Arc::new(Semaphore::new(4)),
        Arc::new(Semaphore::new(4)),
    ));
    client_handshake_with_options(&mut peer, &[], capabilities)
        .await
        .unwrap();
    write_dispatch(&mut peer, &frame).await.unwrap();
    let response = read_dispatch(&mut peer).await.unwrap();
    task.await.unwrap().unwrap();
    response
}

#[tokio::test]
async fn native_ipc_rejects_missing_capabilities_before_authority_or_effects() {
    let (_directory, runtime, context) = fixture();
    let session = SessionId::new();
    let remote = |message| {
        let mut envelope = Envelope::new(message);
        envelope.machine = Some(context.scope.machine);
        envelope.session = Some(session);
        DispatchFrame::Remote {
            context: context.clone(),
            message: Box::new(envelope),
        }
    };
    let input = remote(Message::Input {
        authority: Generations {
            host: Counter(1),
            controller: Some(Counter(1)),
        },
        input_seq: Counter(1),
        input: SessionInput::Pty("eA==".into()),
    });
    let shell = Launch {
        key: IdempotencyKey::new(),
        kind: LaunchKind::Pty {
            command: vec!["/bin/true".into()],
        },
        cwd: runtime.state_dir().display().to_string(),
        rows: 24,
        cols: 80,
    };
    let agent = Launch {
        kind: LaunchKind::Agent {
            agent: "codex".into(),
            prompt: None,
            native_resume_id: None,
        },
        ..shell.clone()
    };
    let cases = [
        (remote(Message::Start(shell)), "native-start-v1"),
        (remote(Message::Start(agent)), "native-agent-v1"),
        (
            remote(Message::Input {
                authority: Generations {
                    host: Counter(1),
                    controller: Some(Counter(1)),
                },
                input_seq: Counter(1),
                input: SessionInput::Prompt("fixture".into()),
            }),
            "native-agent-v1",
        ),
        (
            DispatchFrame::QueryHostFloor {
                scope: context.scope.clone(),
            },
            "native-authority-v1",
        ),
        (
            DispatchFrame::QueryHostFloor {
                scope: context.scope.clone(),
            },
            "native-floor-v1",
        ),
        (
            DispatchFrame::QueryControl {
                context: context.clone(),
                session,
            },
            "native-controller-v1",
        ),
        (
            DispatchFrame::ReadHead {
                context: context.clone(),
                session,
            },
            "native-replay-v1",
        ),
        (
            DispatchFrame::ReadSnapshot {
                context: context.clone(),
                session,
                epoch: None,
                page: 0,
            },
            "native-replay-v1",
        ),
        (
            DispatchFrame::RecordingStatus {
                context: context.clone(),
            },
            "recording-read-v1",
        ),
        (
            DispatchFrame::RecordingEpochChallenge {
                context: context.clone(),
            },
            "recording-read-v1",
        ),
        (
            DispatchFrame::RecordingEpochChallenge {
                context: context.clone(),
            },
            "recording-reclaim-v1",
        ),
        (
            remote(Message::SessionListPage {
                after: None,
                limit: 64,
                active_only: true,
            }),
            "native-pages-v1",
        ),
        (remote(Message::SessionGet { session }), "native-pages-v1"),
        (input.clone(), "native-effects-v1"),
        (input, "native-controller-v1"),
        (
            DispatchFrame::AgentInventory {
                context: context.clone(),
            },
            "native-agent-v1",
        ),
        (remote(Message::ApprovalList), "native-approvals-v1"),
    ];
    for (frame, missing) in cases {
        let offered = capabilities::offered(runtime.managed_enabled());
        let requested: Vec<_> = offered
            .into_iter()
            .filter(|name| *name != missing)
            .collect();
        let response = exchange(runtime.clone(), frame, &requested).await;
        assert!(
            matches!(
                response,
                DispatchFrame::Error(RemoteError {
                    code: ErrorCode::UnsupportedCapability,
                    ..
                })
            ),
            "missing {missing} reached dispatch: {response:?}"
        );
    }
    let response = exchange(
        runtime.clone(),
        DispatchFrame::QueryHostFloor {
            scope: context.scope,
        },
        &["native-authority-v1", "native-floor-v1"],
    )
    .await;
    assert!(matches!(
        response,
        DispatchFrame::HostFloor {
            generation: Counter(0)
        }
    ));
    let response = exchange(
        runtime,
        DispatchFrame::Local(Request {
            v: 1,
            id: RequestId::new(),
            client: ClientId::new(),
            body: Operation::Status,
        }),
        &[],
    )
    .await;
    assert!(
        matches!(response, DispatchFrame::LocalReply(response) if matches!(response.body, Reply::Status(_)))
    );
}

#[tokio::test]
async fn lost_admin_reply_still_commits_shutdown() {
    let (_directory, runtime, _) = fixture();
    let (server, mut peer) = UnixStream::pair().unwrap();
    let permits = Arc::new(Semaphore::new(1));
    let (stop_tx, mut stop_rx) = mpsc::channel(1);
    write_frame(
        &mut peer,
        &serde_json::to_vec(&AdminRequest {
            ipc_major: IPC_MAJOR,
            command: AdminCommand::Shutdown {
                terminate_sessions: false,
            },
        })
        .unwrap(),
    )
    .await
    .unwrap();
    drop(peer);
    let result = serve_admin(
        server,
        runtime,
        permits.clone().try_acquire_owned().unwrap(),
        stop_tx,
    )
    .await;
    assert!(result.is_err(), "the reply must fail on the closed socket");
    assert_eq!(stop_rx.recv().await, Some(()));
    assert_eq!(permits.available_permits(), 1);
}

#[tokio::test]
async fn cancelled_admin_keeps_capacity_until_its_database_write_finishes() {
    let (directory, runtime, mut context) = fixture();
    context.scope.machine = MachineId::new();
    let db = rusqlite::Connection::open(directory.path().join("sessions.db")).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (server, mut peer) = UnixStream::pair().unwrap();
    let permits = Arc::new(Semaphore::new(1));
    let (stop_tx, _stop_rx) = mpsc::channel(1);
    let task = tokio::spawn(serve_admin(
        server,
        runtime.clone(),
        permits.clone().try_acquire_owned().unwrap(),
        stop_tx,
    ));
    write_frame(
        &mut peer,
        &serde_json::to_vec(&AdminRequest {
            ipc_major: IPC_MAJOR,
            command: AdminCommand::RegisterRemoteBinding {
                scope: context.scope.clone(),
            },
        })
        .unwrap(),
    )
    .await
    .unwrap();
    // Let the decoded request reach the SQLite write, which cannot commit yet.
    sleep(Duration::from_millis(50)).await;
    assert!(!task.is_finished());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        permits.available_permits(),
        0,
        "cancelled work still owns admission"
    );
    db.execute_batch("ROLLBACK").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while permits.available_permits() == 0 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let response = runtime.dispatch_frame(DispatchFrame::QueryHostFloor {
        scope: context.scope,
    });
    assert!(
        matches!(response, DispatchFrame::HostFloor { .. }),
        "committed registration survives caller loss: {response:?}"
    );
}

#[tokio::test]
async fn cancelled_data_request_keeps_capacity_until_activation_commits() {
    let (directory, runtime, context) = fixture();
    let DispatchFrame::AccessChallengeIssued { nonce, .. } =
        runtime.dispatch_frame(DispatchFrame::AccessChallenge {
            scope: context.scope.clone(),
            connection: context.connection,
            host_generation: Counter(1),
        })
    else {
        panic!("fixture challenge failed")
    };
    let db = rusqlite::Connection::open(directory.path().join("sessions.db")).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (server, mut peer) = UnixStream::pair().unwrap();
    let permits = Arc::new(Semaphore::new(1));
    let task = tokio::spawn(serve_data(
        server,
        runtime.clone(),
        permits.clone().try_acquire_owned().unwrap(),
        Arc::new(Semaphore::new(4)),
        Arc::new(Semaphore::new(4)),
    ));
    client_handshake_with_options(&mut peer, &["native-authority-v1"], &[])
        .await
        .unwrap();
    write_dispatch(
        &mut peer,
        &DispatchFrame::ActivateHost(HostActivation {
            scope: context.scope.clone(),
            connection: context.connection,
            generation: Counter(1),
            nonce,
            remaining_ms: 1000,
            policy_revision: Counter(1),
        }),
    )
    .await
    .unwrap();
    sleep(Duration::from_millis(50)).await;
    assert!(!task.is_finished());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        permits.available_permits(),
        0,
        "cancelled activation still owns admission"
    );
    db.execute_batch("ROLLBACK").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while permits.available_permits() == 0 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::QueryHostFloor {
            scope: context.scope
        }),
        DispatchFrame::HostFloor {
            generation: Counter(1)
        }
    ));
}
