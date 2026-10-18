use super::*;
use crate::runtime::tests::{directory, spec};
fn fixture() -> (tempfile::TempDir, Arc<Runtime>, RemoteScope) {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let scope = RemoteScope {
        origin: "https://gateway.example.test".into(),
        owner: UserId::new(),
        machine: MachineId::new(),
    };
    let mut config = Config::new(path.clone(), vec![path]);
    config.remote_bindings = vec![scope.clone()];
    (temp, Runtime::open(config).unwrap(), scope)
}
fn issued(frame: DispatchFrame) -> ChallengeId {
    let DispatchFrame::AccessChallengeIssued { nonce, .. } = frame else {
        panic!("challenge rejected: {frame:?}");
    };
    nonce
}
fn context(request: &HostActivation) -> RemoteContext {
    RemoteContext {
        actor: request.scope.owner,
        client: ClientId::new(),
        scope: request.scope.clone(),
        connection: request.connection,
        host_generation: request.generation,
        controller_generation: None,
        access_nonce: request.nonce,
        remaining_access_ms: request.remaining_ms,
        policy_revision: request.policy_revision,
    }
}
fn proposed(
    scope: RemoteScope,
    connection: ClientId,
    generation: u64,
    nonce: ChallengeId,
    remaining_ms: u32,
) -> HostActivation {
    HostActivation {
        scope,
        connection,
        generation: Counter(generation),
        nonce,
        remaining_ms,
        policy_revision: Counter(1),
    }
}

#[test]
fn replacement_fences_pending_effects_and_waits_for_an_admitted_os_call_without_a_store_lock() {
    let (_temp, runtime, scope) = fixture();
    let connection = ClientId::new();
    let nonce = issued(runtime.dispatch_frame(DispatchFrame::AccessChallenge {
        scope: scope.clone(),
        connection,
        host_generation: Counter(1),
    }));
    let first = proposed(scope.clone(), connection, 1, nonce, 5000);
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateHost(first.clone())),
        DispatchFrame::HostActivated { .. }
    ));
    let fence = lock(&runtime.authority)
        .unwrap()
        .fence(&context(&first))
        .unwrap();
    let stale_nonce = issued(runtime.dispatch_frame(DispatchFrame::AccessChallenge {
        scope: scope.clone(),
        connection,
        host_generation: Counter(1),
    }));
    let next_connection = ClientId::new();
    let nonce = issued(runtime.dispatch_frame(DispatchFrame::AccessChallenge {
        scope: scope.clone(),
        connection: next_connection,
        host_generation: Counter(2),
    }));
    let next = proposed(scope.clone(), next_connection, 2, nonce, 5000);
    let (entered, started) = std::sync::mpsc::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    let old = fence.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let effect_done = finished.clone();
    let effect = thread::spawn(move || {
        old.perform(|| {
            entered.send(()).unwrap();
            paused.recv().unwrap();
            effect_done.store(true, Ordering::Release);
            Ok(())
        })
    });
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    let native = runtime.clone();
    let proposal = next.clone();
    let (ack, acknowledged) = std::sync::mpsc::channel();
    let replacing = thread::spawn(move || {
        ack.send(native.dispatch_frame(DispatchFrame::ActivateHost(proposal)))
            .unwrap();
    });
    let until = Instant::now() + Duration::from_secs(1);
    while fence.check().is_ok() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(matches!(
        acknowledged.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert!(
        runtime.store.try_lock().is_ok(),
        "replacement must not hold the native database while waiting for an OS call"
    );
    resume.send(()).unwrap();
    effect.join().unwrap().unwrap();
    assert!(matches!(
        acknowledged.recv_timeout(Duration::from_secs(2)).unwrap(),
        DispatchFrame::HostActivated { .. }
    ));
    replacing.join().unwrap();
    assert!(finished.load(Ordering::Acquire));
    assert!(fence.perform(|| Ok(())).is_err());
    let current = lock(&runtime.authority)
        .unwrap()
        .fence(&context(&next))
        .unwrap();
    current.perform(|| Ok(())).unwrap();
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateHost(proposed(
            scope,
            connection,
            1,
            stale_nonce,
            5000
        ))),
        DispatchFrame::Error(_)
    ));
    current.perform(|| Ok(())).unwrap();
    runtime.shutdown(false).unwrap();
}

#[test]
fn native_start_retains_scope_key_and_original_fence_through_start_queue_wait() {
    let (temp, runtime, scope) = fixture();
    let path = temp.path().canonicalize().unwrap();
    let connection = ClientId::new();
    let nonce = issued(runtime.dispatch_frame(DispatchFrame::AccessChallenge {
        scope: scope.clone(),
        connection,
        host_generation: Counter(1),
    }));
    let request = proposed(scope.clone(), connection, 1, nonce, 5000);
    runtime.dispatch_frame(DispatchFrame::ActivateHost(request.clone()));
    let access = context(&request);
    let launch = spec(&path, IdempotencyKey::new());
    let mut message = wire::Envelope::new(wire::Message::Start(wire::Launch {
        key: launch.key,
        kind: wire::LaunchKind::Pty {
            command: launch.command.clone(),
        },
        cwd: launch.cwd.clone(),
        rows: launch.rows,
        cols: launch.cols,
    }));
    message.machine = Some(scope.machine);
    let first = runtime.dispatch_frame(DispatchFrame::Remote {
        context: access.clone(),
        message: Box::new(message.clone()),
    });
    let DispatchFrame::RemoteReply(first) = first else {
        panic!("native start");
    };
    let wire::Message::Started(first) = first.message else {
        panic!("start receipt");
    };
    let duplicate = runtime.dispatch_frame(DispatchFrame::Remote {
        context: access,
        message: Box::new(message),
    });
    let DispatchFrame::RemoteReply(duplicate) = duplicate else {
        panic!("duplicate start");
    };
    let wire::Message::Started(duplicate) = duplicate.message else {
        panic!("receipt");
    };
    assert_eq!(first.id, duplicate.id);
    assert_eq!(runtime.list().unwrap().len(), 1);
    let nonce = issued(runtime.dispatch_frame(DispatchFrame::AccessChallenge {
        scope: scope.clone(),
        connection,
        host_generation: Counter(1),
    }));
    let short = proposed(scope.clone(), connection, 1, nonce, 100);
    runtime.dispatch_frame(DispatchFrame::AccessRenewal(short.clone()));
    let fence = lock(&runtime.authority)
        .unwrap()
        .fence(&context(&short))
        .unwrap();
    let barrier = runtime.starts.lock().unwrap();
    let native = runtime.clone();
    let blocked = spec(&path, IdempotencyKey::new());
    let waiting = thread::spawn(move || native.start_guarded(blocked, Some(scope), Some(fence)));
    thread::sleep(Duration::from_millis(130));
    drop(barrier);
    assert!(waiting.join().unwrap().is_err());
    assert_eq!(runtime.list().unwrap().len(), 1);
    runtime.shutdown(true).unwrap();
}

#[test]
fn sqlite_acceptance_past_original_access_persists_failed_key_without_launch_and_survives_restart()
{
    let (temp, runtime, scope) = fixture();
    let path = temp.path().canonicalize().unwrap();
    let config = runtime.config.clone();
    let connection = ClientId::new();
    let nonce = issued(runtime.dispatch_frame(DispatchFrame::AccessChallenge {
        scope: scope.clone(),
        connection,
        host_generation: Counter(1),
    }));
    let activation = proposed(scope.clone(), connection, 1, nonce, 1000);
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateHost(activation.clone())),
        DispatchFrame::HostActivated { .. }
    ));
    let launch = spec(&path, IdempotencyKey::new());
    let key = launch.key;
    let mut message = wire::Envelope::new(wire::Message::Start(wire::Launch {
        key,
        kind: wire::LaunchKind::Pty {
            command: launch.command.clone(),
        },
        cwd: launch.cwd,
        rows: 24,
        cols: 80,
    }));
    message.machine = Some(scope.machine);
    let blocker = rusqlite::Connection::open(path.join("sessions.db")).unwrap();
    blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let native = runtime.clone();
    let source = context(&activation);
    let waiting = thread::spawn(move || {
        native.dispatch_frame(DispatchFrame::Remote {
            context: source,
            message: Box::new(message),
        })
    });
    thread::sleep(Duration::from_millis(1200));
    blocker.execute_batch("COMMIT").unwrap();
    assert!(matches!(
        waiting.join().unwrap(),
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::AccessStale,
            ..
        })
    ));
    let sessions = runtime.list().unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(matches!(sessions[0].state, SessionState::Failed));
    let failed = sessions[0].id;
    assert!(!path.join("native-marker").exists());
    runtime.shutdown(false).unwrap();
    drop(runtime);
    let runtime = Runtime::open_after_previous_owner(config).unwrap();
    let connection = ClientId::new();
    let nonce = issued(runtime.dispatch_frame(DispatchFrame::AccessChallenge {
        scope: scope.clone(),
        connection,
        host_generation: Counter(2),
    }));
    let activation = proposed(scope.clone(), connection, 2, nonce, 5000);
    runtime.dispatch_frame(DispatchFrame::ActivateHost(activation.clone()));
    let mut lookup = wire::Envelope::new(wire::Message::StartLookup { key });
    lookup.machine = Some(scope.machine);
    let DispatchFrame::RemoteReply(reply) = runtime.dispatch_frame(DispatchFrame::Remote {
        context: context(&activation),
        message: Box::new(lookup),
    }) else {
        panic!("lookup");
    };
    assert!(
        matches!(reply.message,wire::Message::StartStatus{stage:wire::StartStage::Failed,session:Some(id),..} if id==failed)
    );
    assert!(!path.join("native-marker").exists());
    runtime.shutdown(false).unwrap();
}

#[test]
fn original_nonce_deadline_replay_renewal_replacement_and_full_u64_floor() {
    let (_temp, runtime, scope) = fixture();
    let config = runtime.config.clone();
    let now = Instant::now();
    let connection = ClientId::new();
    let mut authority = lock(&runtime.authority).unwrap();
    let mut store = lock(&runtime.store).unwrap();
    let nonce = issued(
        authority
            .challenge(&store, scope.clone(), connection, Counter(1), now)
            .unwrap(),
    );
    let request = proposed(scope.clone(), connection, 1, nonce, 5000);
    authority
        .activate(&mut store, &request, false, || now + Duration::from_secs(3))
        .unwrap();
    let original = context(&request);
    let elapsed_expiry = authority
        .active
        .get(&scope)
        .unwrap()
        .grants
        .get(&nonce)
        .unwrap()
        .expires_elapsed;
    assert!(
        authority
            .validate_at(
                &original,
                now + Duration::from_secs(4),
                elapsed_expiry + Duration::from_nanos(1)
            )
            .is_err(),
        "suspended time expires access even if the awake-time clock has not reached its deadline"
    );
    assert_eq!(
        authority
            .validate(&original, now + Duration::from_secs(4))
            .unwrap(),
        now + Duration::from_secs(5)
    );
    authority
        .activate(&mut store, &request, false, || now + Duration::from_secs(4))
        .unwrap(); // lost ACK, same deadline
    assert!(
        authority
            .validate(&original, now + Duration::from_secs(5))
            .is_err()
    );
    let mut altered = request.clone();
    altered.remaining_ms = 6000;
    assert!(
        authority
            .activate(&mut store, &altered, false, || now + Duration::from_secs(4))
            .is_err()
    );
    let nonce = issued(
        authority
            .challenge(
                &store,
                scope.clone(),
                connection,
                Counter(1),
                now + Duration::from_secs(4),
            )
            .unwrap(),
    );
    let renewed = proposed(scope.clone(), connection, 1, nonce, 5000);
    authority
        .activate(&mut store, &renewed, true, || now + Duration::from_secs(4))
        .unwrap();
    assert!(
        authority
            .validate(&original, now + Duration::from_secs(6))
            .is_err(),
        "new access cannot renew an old queued request"
    );
    assert!(
        authority
            .validate(&context(&renewed), now + Duration::from_secs(6))
            .is_ok()
    );
    let replacement = ClientId::new();
    let nonce = issued(
        authority
            .challenge(
                &store,
                scope.clone(),
                replacement,
                Counter(2),
                now + Duration::from_secs(6),
            )
            .unwrap(),
    );
    let replaced = proposed(scope.clone(), replacement, 2, nonce, 5000);
    authority
        .activate(&mut store, &replaced, false, || {
            now + Duration::from_secs(6)
        })
        .unwrap();
    assert!(matches!(
        authority
            .validate(&context(&renewed), now + Duration::from_secs(6))
            .unwrap_err()
            .code,
        ErrorCode::StaleGeneration
    ));
    assert!(
        authority
            .challenge(
                &store,
                scope.clone(),
                connection,
                Counter(2),
                now + Duration::from_secs(6)
            )
            .is_err()
    );
    let nonce = issued(
        authority
            .challenge(
                &store,
                scope.clone(),
                replacement,
                Counter(u64::MAX),
                now + Duration::from_secs(7),
            )
            .unwrap(),
    );
    let maximum = proposed(scope.clone(), replacement, u64::MAX, nonce, 5000);
    authority
        .activate(&mut store, &maximum, false, || now + Duration::from_secs(7))
        .unwrap();
    assert_eq!(
        store.host_floor(&scope).unwrap(),
        (Counter(u64::MAX), Some(replacement))
    );
    assert!(
        authority
            .challenge(
                &store,
                scope.clone(),
                replacement,
                Counter(u64::MAX - 1),
                now + Duration::from_secs(7)
            )
            .is_err()
    );
    drop(store);
    drop(authority);
    drop(runtime);
    let reopened = Runtime::open_after_previous_owner(config).unwrap();
    assert_eq!(
        lock(&reopened.store).unwrap().host_floor(&scope).unwrap().0,
        Counter(u64::MAX)
    );
    assert!(
        lock(&reopened.authority)
            .unwrap()
            .validate(&context(&maximum), Instant::now())
            .is_err(),
        "restart never restores access grants"
    );
}

#[test]
fn stalled_sqlite_activation_cannot_restart_an_expired_budget_or_restore_the_old_host() {
    let (temp, runtime, scope) = fixture();
    let connection = ClientId::new();
    let nonce = issued(runtime.dispatch_frame(DispatchFrame::AccessChallenge {
        scope: scope.clone(),
        connection,
        host_generation: Counter(1),
    }));
    let first = proposed(scope.clone(), connection, 1, nonce, 10_000);
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateHost(first.clone())),
        DispatchFrame::HostActivated { .. }
    ));
    let replacement = ClientId::new();
    let blocker = rusqlite::Connection::open(temp.path().join("sessions.db")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let owned = runtime.clone();
    let owned_scope = scope.clone();
    let (entered, waiting) = std::sync::mpsc::channel();
    let task = thread::spawn(move || {
        let mut authority = lock(&owned.authority).unwrap();
        let mut store = lock(&owned.store).unwrap();
        let nonce = issued(
            authority
                .challenge(
                    &store,
                    owned_scope.clone(),
                    replacement,
                    Counter(2),
                    Instant::now(),
                )
                .unwrap(),
        );
        let request = proposed(owned_scope, replacement, 2, nonce, 100);
        authority.activate(&mut store, &request, false, || {
            let now = Instant::now();
            let _ = entered.send(());
            now
        })
    });
    waiting.recv_timeout(Duration::from_secs(2)).unwrap();
    thread::sleep(Duration::from_millis(250));
    blocker.execute_batch("COMMIT").unwrap();
    assert!(matches!(
        task.join().unwrap(),
        Err(RemoteError {
            code: ErrorCode::AccessStale,
            ..
        })
    ));
    assert_eq!(
        lock(&runtime.store).unwrap().host_floor(&scope).unwrap().0,
        Counter(2)
    );
    assert!(
        lock(&runtime.authority)
            .unwrap()
            .validate(&context(&first), Instant::now())
            .is_err(),
        "the committed higher floor fences the old host even without a fresh ACK"
    );
}

#[tokio::test]
async fn real_private_ipc_activates_reads_fences_replacement_and_forgets_grants_on_restart() {
    use qgenie_remote_proto::ipc::{AdminCommand, AdminRequest, AdminResponse};
    use qgenie_remote_transport::ipc::{
        client_handshake_with_capabilities, connect, read_dispatch, read_frame, write_dispatch,
        write_frame,
    };
    struct Service(Option<tokio::task::JoinHandle<anyhow::Result<()>>>);
    impl Drop for Service {
        fn drop(&mut self) {
            if let Some(task) = &self.0 {
                task.abort();
            }
        }
    }
    async fn rpc(path: &std::path::Path, frame: DispatchFrame) -> DispatchFrame {
        tokio::time::timeout(Duration::from_secs(3), async {
            let mut stream = connect(&path.join("dispatch.sock")).await.unwrap();
            client_handshake_with_capabilities(&mut stream, &["native-authority-v1"])
                .await
                .unwrap();
            write_dispatch(&mut stream, &frame).await.unwrap();
            read_dispatch(&mut stream).await.unwrap()
        })
        .await
        .unwrap()
    }
    async fn stop(path: &std::path::Path, mut service: Service) {
        let mut stream = connect(&path.join("admin.sock")).await.unwrap();
        write_frame(
            &mut stream,
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
        let result: AdminResponse = decode_json(&read_frame(&mut stream).await.unwrap()).unwrap();
        assert!(result.result.is_ok());
        let mut task = service.0.take().unwrap();
        match tokio::time::timeout(Duration::from_secs(3), &mut task).await {
            Ok(result) => result.unwrap().unwrap(),
            Err(_) => {
                task.abort();
                let _ = task.await;
                panic!("native IPC service did not stop");
            }
        }
    }
    let (_temp, runtime, scope) = fixture();
    let config = runtime.config.clone();
    let path = config.state_dir.clone();
    let owned = runtime
        .start_scoped(spec(&path, IdempotencyKey::new()), Some(scope.clone()))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !runtime.readers_finished() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    drop(runtime);
    let connection = ClientId::new();
    let mut old_context = None;
    for cycle in 0..2 {
        let service = Service(Some(tokio::spawn(crate::serve(config.clone()))));
        tokio::time::timeout(Duration::from_secs(3), async {
            while connect(&path.join("dispatch.sock")).await.is_err() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        // A legacy handshake cannot send native authority operations even
        // though the same daemon continues to serve local minor-zero clients.
        let mut legacy = connect(&path.join("dispatch.sock")).await.unwrap();
        qgenie_remote_transport::ipc::client_handshake(&mut legacy)
            .await
            .unwrap();
        write_dispatch(
            &mut legacy,
            &DispatchFrame::AccessChallenge {
                scope: scope.clone(),
                connection,
                host_generation: Counter(cycle + 1),
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            read_dispatch(&mut legacy).await.unwrap(),
            DispatchFrame::Error(RemoteError {
                code: ErrorCode::UnsupportedCapability,
                ..
            })
        ));
        let message = || {
            let mut message = wire::Envelope::new(wire::Message::SessionList);
            message.machine = Some(scope.machine);
            Box::new(message)
        };
        if let Some(context) = old_context.clone() {
            assert!(matches!(
                rpc(
                    &path,
                    DispatchFrame::Remote {
                        context,
                        message: message()
                    }
                )
                .await,
                DispatchFrame::Error(RemoteError {
                    code: ErrorCode::AccessStale,
                    ..
                })
            ));
        }
        let generation = Counter(cycle + 1);
        let nonce = issued(
            rpc(
                &path,
                DispatchFrame::AccessChallenge {
                    scope: scope.clone(),
                    connection,
                    host_generation: generation,
                },
            )
            .await,
        );
        let request = proposed(scope.clone(), connection, generation.0, nonce, 10_000);
        let context = context(&request);
        for _ in 0..2 {
            assert!(
                matches!(rpc(&path,DispatchFrame::ActivateHost(request.clone())).await,DispatchFrame::HostActivated {generation:received,..} if received==generation)
            );
        }
        // Native read authority alone cannot activate or inspect controllers.
        assert!(matches!(
            rpc(
                &path,
                DispatchFrame::QueryControl {
                    context: context.clone(),
                    session: owned.id
                }
            )
            .await,
            DispatchFrame::Error(RemoteError {
                code: ErrorCode::UnsupportedCapability,
                ..
            })
        ));
        let mut controller = connect(&path.join("dispatch.sock")).await.unwrap();
        client_handshake_with_capabilities(
            &mut controller,
            &["native-authority-v1", "native-controller-v1"],
        )
        .await
        .unwrap();
        write_dispatch(
            &mut controller,
            &DispatchFrame::QueryControl {
                context: context.clone(),
                session: owned.id,
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(read_dispatch(&mut controller).await.unwrap(),DispatchFrame::ControlState {session,floor:Counter(0),holder:None,remaining_ms:0,..} if session==owned.id)
        );
        let DispatchFrame::RemoteReply(reply) = rpc(
            &path,
            DispatchFrame::Remote {
                context: context.clone(),
                message: message(),
            },
        )
        .await
        else {
            panic!("native IPC read");
        };
        assert!(
            matches!(&reply.message,wire::Message::Sessions {sessions} if sessions.len()==1 && sessions[0].id==owned.id)
        );
        if cycle == 1 {
            assert!(matches!(
                rpc(
                    &path,
                    DispatchFrame::AccessChallenge {
                        scope: scope.clone(),
                        connection,
                        host_generation: Counter(1)
                    }
                )
                .await,
                DispatchFrame::Error(RemoteError {
                    code: ErrorCode::StaleGeneration,
                    ..
                })
            ));
        }
        old_context = Some(context);
        stop(&path, service).await;
    }
}

#[test]
fn bounded_challenges_scope_filtered_reads_and_unknown_mutations() {
    let (_temp, runtime, scope) = fixture();
    let connection = ClientId::new();
    let now = Instant::now();
    {
        let mut authority = lock(&runtime.authority).unwrap();
        let store = lock(&runtime.store).unwrap();
        for _ in 0..4 {
            authority
                .challenge(&store, scope.clone(), connection, Counter(1), now)
                .unwrap();
        }
        assert!(matches!(
            authority
                .challenge(&store, scope.clone(), connection, Counter(1), now)
                .unwrap_err()
                .code,
            ErrorCode::Overloaded
        ));
        authority.prune(now + MAX_LIFETIME, clock::elapsed().unwrap());
    }
    let path = runtime.config.state_dir.clone();
    let launch = spec(&path, IdempotencyKey::new());
    let local = runtime.start(launch.clone()).unwrap();
    let owned = runtime
        .start_scoped(launch.clone(), Some(scope.clone()))
        .unwrap();
    let nonce = issued(runtime.dispatch_frame(DispatchFrame::AccessChallenge {
        scope: scope.clone(),
        connection,
        host_generation: Counter(1),
    }));
    let request = proposed(scope.clone(), connection, 1, nonce, 10_000);
    let context = context(&request);
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateHost(request)),
        DispatchFrame::HostActivated { .. }
    ));
    let read = |context: RemoteContext, message: wire::Message| {
        let mut envelope = wire::Envelope::new(message);
        envelope.machine = Some(scope.machine);
        runtime.dispatch_frame(DispatchFrame::Remote {
            context,
            message: Box::new(envelope),
        })
    };
    let DispatchFrame::RemoteReply(reply) = read(context.clone(), wire::Message::SessionList)
    else {
        panic!("list");
    };
    assert!(
        matches!(&reply.message,wire::Message::Sessions {sessions} if sessions.len()==1 && sessions[0].id==owned.id && sessions[0].id!=local.id)
    );
    let DispatchFrame::RemoteReply(reply) = read(
        context.clone(),
        wire::Message::StartLookup { key: launch.key },
    ) else {
        panic!("lookup");
    };
    assert!(
        matches!(reply.message,wire::Message::StartStatus {session:Some(id),..} if id==owned.id)
    );
    let mut foreign = context.clone();
    foreign.actor = UserId::new();
    assert!(matches!(
        read(foreign, wire::Message::SessionList),
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::Forbidden,
            ..
        })
    ));
    let mutation = wire::Message::Resume {
        historical: SessionId::new(),
        launch: wire::Launch {
            key: IdempotencyKey::new(),
            kind: wire::LaunchKind::Pty {
                command: launch.command.clone(),
            },
            cwd: launch.cwd.clone(),
            rows: 24,
            cols: 80,
        },
    };
    assert!(matches!(
        read(context, mutation),
        DispatchFrame::Error(RemoteError {
            code: ErrorCode::UnsupportedCapability,
            ..
        })
    ));
    let deadline = Instant::now() + Duration::from_secs(3);
    while !runtime.readers_finished() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        std::fs::read(path.join("native-marker")).unwrap(),
        b"xx",
        "no native effect for the remote mutation"
    );
}
