use super::*;
pub(super) fn directory() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
pub(super) fn call(runtime: &Arc<Runtime>, client: ClientId, body: Operation) -> Reply {
    runtime
        .dispatch(Request {
            v: 1,
            id: RequestId::new(),
            client,
            body,
        })
        .body
}
pub(super) fn spec(path: &std::path::Path, key: IdempotencyKey) -> StartSpec {
    let marker = path
        .join("native-marker")
        .display()
        .to_string()
        .replace('\'', "'\"'\"'");
    StartSpec {
        key,
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            format!("printf x >> '{marker}'"),
        ],
        cwd: path.display().to_string(),
        rows: 24,
        cols: 80,
    }
}
#[test]
fn concurrent_duplicate_starts_have_one_native_effect_and_lost_observers_do_not_kill() {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let runtime = Runtime::open(Config::new(path.clone(), vec![path.clone()])).unwrap();
    let launch = spec(&path, IdempotencyKey::new());
    let barrier = Arc::new(std::sync::Barrier::new(16));
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let runtime = runtime.clone();
        let barrier = barrier.clone();
        let launch = launch.clone();
        tasks.push(thread::spawn(move || {
            barrier.wait();
            let Reply::Started(info) = call(&runtime, ClientId::new(), Operation::Start(launch))
            else {
                panic!("start failed");
            };
            info.id
        }));
    }
    let ids = tasks
        .into_iter()
        .map(|task| task.join().unwrap())
        .collect::<Vec<_>>();
    assert!(ids.iter().all(|id| *id == ids[0]));
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.join("native-marker").exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(std::fs::read(path.join("native-marker")).unwrap(), b"x");
    let mut launch = spec(&path, IdempotencyKey::new());
    let finish = path.join("finish-child");
    let quoted_finish = finish.display().to_string().replace('\'', "'\"'\"'");
    launch.command[2] = format!(
        "while [ ! -f '{quoted_finish}' ]; do sleep 0.01; done; {}",
        launch.command[2]
    );
    call(&runtime, ClientId::new(), Operation::Start(launch));
    let weak = Arc::downgrade(&runtime);
    drop(runtime);
    assert!(Runtime::open(Config::new(path.clone(), vec![path.clone()])).is_err());
    std::fs::write(finish, b"release").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while weak.upgrade().is_some() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(std::fs::read(path.join("native-marker")).unwrap(), b"xx");
    Runtime::open_after_previous_owner(Config::new(path.clone(), vec![path])).unwrap();
}
#[test]
fn lease_expiring_during_sqlite_intent_cannot_apply_input() {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let runtime = Runtime::open(Config::new(path.clone(), vec![path.clone()])).unwrap();
    let client = ClientId::new();
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] = format!("read value; {}", launch.command[2]);
    let Reply::Started(info) = call(&runtime, client, Operation::Start(launch)) else {
        panic!("start failed");
    };
    let Reply::Control(lease) = call(
        &runtime,
        client,
        Operation::TakeControl {
            session: info.id,
            takeover: false,
        },
    ) else {
        panic!("control failed");
    };
    lock(&runtime.session(info.id).unwrap()).unwrap().deadline =
        Some(Instant::now() + Duration::from_millis(500));
    let blocker = rusqlite::Connection::open(path.join("sessions.db")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let owned = runtime.clone();
    let task = thread::spawn(move || {
        call(
            &owned,
            client,
            Operation::Input {
                session: info.id,
                generation: lease.generation,
                input_seq: Counter(1),
                data: STANDARD.encode(b"must not execute\n"),
            },
        )
    });
    thread::sleep(Duration::from_millis(750));
    blocker.execute_batch("COMMIT").unwrap();
    assert!(matches!(
        task.join().unwrap(),
        Reply::Error(RemoteError {
            code: ErrorCode::StaleGeneration,
            ..
        })
    ));
    let outcome: String = blocker
        .query_row(
            "SELECT outcome FROM input_ack WHERE session_id=?1",
            [info.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(matches!(
        serde_json::from_str::<ApplicationOutcome>(&outcome).unwrap(),
        ApplicationOutcome::Cancelled
    ));
    assert!(!path.join("native-marker").exists());
    runtime.shutdown(true).unwrap();
}
#[test]
fn failed_control_activation_cannot_leave_the_previous_holder_live() {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let runtime = Runtime::open(Config::new(path.clone(), vec![path.clone()])).unwrap();
    let old = ClientId::new();
    let next = ClientId::new();
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] = "read value; printf %s \"$value\" > native-marker".into();
    let Reply::Started(info) = call(&runtime, old, Operation::Start(launch)) else {
        panic!("start");
    };
    let Reply::Control(lease) = call(
        &runtime,
        old,
        Operation::TakeControl {
            session: info.id,
            takeover: false,
        },
    ) else {
        panic!("control");
    };
    let blocker = rusqlite::Connection::open(path.join("sessions.db")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(matches!(
        call(
            &runtime,
            next,
            Operation::TakeControl {
                session: info.id,
                takeover: true
            }
        ),
        Reply::Error(_)
    ));
    blocker.execute_batch("COMMIT").unwrap();
    assert!(matches!(
        call(
            &runtime,
            old,
            Operation::Input {
                session: info.id,
                generation: lease.generation,
                input_seq: Counter(1),
                data: STANDARD.encode(b"denied\n")
            }
        ),
        Reply::Error(RemoteError {
            code: ErrorCode::StaleGeneration,
            ..
        })
    ));
    let Reply::Control(fresh) = call(
        &runtime,
        next,
        Operation::TakeControl {
            session: info.id,
            takeover: false,
        },
    ) else {
        panic!("fresh control");
    };
    assert!(fresh.generation.0 > lease.generation.0 + 1);
    assert!(matches!(
        call(
            &runtime,
            next,
            Operation::Input {
                session: info.id,
                generation: fresh.generation,
                input_seq: Counter(1),
                data: STANDARD.encode(b"allowed\n")
            }
        ),
        Reply::InputAck {
            outcome: ApplicationOutcome::Applied,
            ..
        }
    ));
    let deadline = Instant::now() + Duration::from_secs(3);
    while !runtime.readers_finished() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        std::fs::read(path.join("native-marker")).unwrap(),
        b"allowed"
    );
}
#[test]
#[ignore = "isolated crash helper; invoked only by the boundary proof"]
fn start_crash_fixture_child() {
    let path = PathBuf::from(std::env::var_os("QGR_START_CRASH_STATE").expect("fixture path"));
    let key = std::env::var("QGR_START_CRASH_KEY")
        .unwrap()
        .parse()
        .unwrap();
    let phase = std::env::var("QGR_START_CRASH_PHASE").unwrap();
    let fault = match phase.as_str() {
        "before_journal" => StartFault::BeforeJournal,
        "accepted" => StartFault::Accepted,
        "launching" => StartFault::Launching,
        "native" => StartFault::NativeLaunched,
        "running" => StartFault::RunningCommitted,
        "ack" => StartFault::BeforeAck,
        _ => panic!("invalid fixture phase"),
    };
    let mut config = Config::new(path.clone(), vec![path.clone()]);
    config.start_fault = Some(fault);
    let runtime = Runtime::open(config).unwrap();
    call(
        &runtime,
        ClientId::new(),
        Operation::Start(spec(&path, key)),
    );
    panic!("crash boundary was not reached");
}
#[test]
fn real_process_crashes_at_start_boundaries_preserve_keys_and_explicit_uncertainty() {
    for phase in [
        "before_journal",
        "accepted",
        "launching",
        "native",
        "running",
        "ack",
    ] {
        let temp = directory();
        let path = std::fs::canonicalize(temp.path()).unwrap();
        let key = IdempotencyKey::new();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::tests::start_crash_fixture_child",
                "--ignored",
            ])
            .env("QGR_START_CRASH_STATE", &path)
            .env("QGR_START_CRASH_KEY", key.to_string())
            .env("QGR_START_CRASH_PHASE", phase)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86), "{phase}");
        let runtime = Runtime::open(Config::new(path.clone(), vec![path.clone()])).unwrap();
        let reply = call(&runtime, ClientId::new(), Operation::LookupStart { key });
        if phase == "before_journal" {
            assert!(matches!(reply, Reply::Error(_)));
            assert!(!path.join("native-marker").exists());
            continue;
        }
        let Reply::Started(info) = reply else {
            panic!("durable key missing at {phase}");
        };
        assert!(matches!(
            info.state,
            SessionState::OutcomeUnknown | SessionState::Lost
        ));
        let Reply::Started(repeated) = call(
            &runtime,
            ClientId::new(),
            Operation::Start(spec(&path, key)),
        ) else {
            panic!("same-key lookup failed");
        };
        assert_eq!(info.id, repeated.id);
        thread::sleep(Duration::from_millis(100));
        let count = std::fs::read(path.join("native-marker")).unwrap_or_default();
        assert!(count.len() <= 1, "duplicate native effect at {phase}");
        if matches!(phase, "accepted" | "launching") {
            assert!(count.is_empty());
        }
        // A separately requested replacement is a new native lifetime. The
        // original unknown/lost record must remain available under its old ID.
        let fresh_key = IdempotencyKey::new();
        let Reply::Started(fresh) = call(
            &runtime,
            ClientId::new(),
            Operation::Start(spec(&path, fresh_key)),
        ) else {
            panic!("explicit fresh launch failed after {phase}");
        };
        assert_ne!(fresh.id, info.id);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let sessions = runtime.list().unwrap();
            assert!(sessions.iter().any(|s| s.id == info.id
                && matches!(s.state, SessionState::Lost | SessionState::OutcomeUnknown)));
            if sessions
                .iter()
                .any(|s| s.id == fresh.id && matches!(s.state, SessionState::Exited))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "fresh native process did not finish"
            );
            thread::sleep(Duration::from_millis(10));
        }
        runtime.shutdown(false).unwrap();
    }
}

#[test]
fn durable_checkpoint_survives_pruning_and_missing_history_fails_explicitly() {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let mut config = Config::new(path.clone(), vec![path.clone()]);
    config.replay_events = 2;
    let runtime = Runtime::open(config.clone()).unwrap();
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] = "read value".into();
    let Reply::Started(info) = call(&runtime, ClientId::new(), Operation::Start(launch)) else {
        panic!("start");
    };
    let session = runtime.session(info.id).unwrap();
    for bytes in [
        b"main\x1b[?1049h".as_slice(),
        b"alternate",
        b"\xe2\x9c",
        b"\x93\x1b[?1049l tail",
    ] {
        runtime.output(&session, bytes);
    }
    let before = runtime.snapshot(&lock(&session).unwrap()).unwrap().data;
    runtime.shutdown(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !runtime.readers_finished() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        lock(&session).unwrap().screen.is_none(),
        "finished sessions do not retain native grids"
    );
    drop(runtime);
    let runtime = Runtime::open_after_previous_owner(config.clone()).unwrap();
    let state = runtime.session(info.id).unwrap();
    assert!(
        lock(&state).unwrap().screen.is_none(),
        "historical grids load only for capture"
    );
    assert!(
        runtime.snapshot(&lock(&state).unwrap()).unwrap().data == before,
        "checkpoint and tail must reconstruct all hidden state"
    );
    let store = lock(&runtime.store).unwrap();
    let (floor, events, _) = store.replay(info.id, Counter(0)).unwrap();
    assert!(floor.0 > 1 && events.len() <= 2);
    assert!(store.checkpoint(info.id).unwrap().is_some());
    drop(store);
    drop(runtime);
    let connection = rusqlite::Connection::open(path.join("sessions.db")).unwrap();
    connection
        .execute("DELETE FROM terminal_checkpoints", [])
        .unwrap();
    drop(connection);
    let runtime = Runtime::open(config).unwrap();
    let state = runtime.session(info.id).unwrap();
    assert!(matches!(
        runtime.snapshot(&lock(&state).unwrap()).unwrap_err().code,
        ErrorCode::HistoryTruncated
    ));
}

#[test]
fn snapshot_pages_keep_a_fixed_cursor_and_reconcile_an_idle_lost_tail() {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let runtime = Runtime::open(Config::new(path.clone(), vec![path.clone()])).unwrap();
    let client = ClientId::new();
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] = "read value".into();
    launch.rows = 128;
    launch.cols = 512;
    let Reply::Started(info) = call(&runtime, client, Operation::Start(launch)) else {
        panic!("start");
    };
    let session = runtime.session(info.id).unwrap();
    runtime.output(&session, b"before snapshot");
    let Reply::Attached(attachment) =
        call(&runtime, client, Operation::Attach { session: info.id })
    else {
        panic!("attach");
    };
    let first = attachment.snapshot;
    assert!(first.total_pages > 1);
    runtime.output(&session, b" final frame");
    let mut data = STANDARD.decode(&first.data).unwrap();
    for page in 1..first.total_pages {
        assert!(matches!(
            call(
                &runtime,
                ClientId::new(),
                Operation::SnapshotPage {
                    session: info.id,
                    epoch: first.epoch,
                    page
                }
            ),
            Reply::Error(_)
        ));
        let Reply::SnapshotPage(next) = call(
            &runtime,
            client,
            Operation::SnapshotPage {
                session: info.id,
                epoch: first.epoch,
                page,
            },
        ) else {
            panic!("page");
        };
        assert_eq!(next.seq, first.seq);
        data.extend(STANDARD.decode(next.data).unwrap());
    }
    let mut restored = Screen::from_checkpoint(&data).unwrap();
    // Simulate losing the final event and receiving no later activity. A head
    // read/replay with the last applied cursor still exposes that event.
    let Reply::Replay(dropped) = call(
        &runtime,
        client,
        Operation::Replay {
            session: info.id,
            after: first.seq,
        },
    ) else {
        panic!("tail");
    };
    assert!(dropped.head > first.seq);
    let Reply::Replay(reconciled) = call(
        &runtime,
        client,
        Operation::Replay {
            session: info.id,
            after: first.seq,
        },
    ) else {
        panic!("reconcile");
    };
    let mut cursor = first.seq;
    for event in reconciled.events {
        assert_eq!(cursor.0.checked_add(1), Some(event.seq.0));
        apply_screen(&mut restored, &event.body).unwrap();
        cursor = event.seq;
    }
    assert_eq!(cursor, reconciled.head);
    assert!(
        restored.snapshot().unwrap().data
            == runtime.snapshot(&lock(&session).unwrap()).unwrap().data
    );
    runtime.shutdown(true).unwrap();
}

#[test]
fn idle_storage_recovery_records_a_gap_before_exposing_the_current_screen() {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let runtime = Runtime::open(Config::new(path.clone(), vec![path.clone()])).unwrap();
    let client = ClientId::new();
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] = "read value".into();
    let Reply::Started(info) = call(&runtime, client, Operation::Start(launch)) else {
        panic!("start");
    };
    let session = runtime.session(info.id).unwrap();
    let blocker = rusqlite::Connection::open(path.join("sessions.db")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    runtime.output(&session, b"output during storage outage");
    assert!(lock(&session).unwrap().pending_lost_bytes > 0);
    blocker.execute_batch("COMMIT").unwrap();
    let Reply::Attached(attachment) =
        call(&runtime, client, Operation::Attach { session: info.id })
    else {
        panic!("idle recovery");
    };
    assert_eq!(lock(&session).unwrap().pending_lost_bytes, 0);
    assert_eq!(attachment.snapshot.seq, Counter(1));
    let Reply::Replay(replay) = call(
        &runtime,
        client,
        Operation::Replay {
            session: info.id,
            after: Counter(0),
        },
    ) else {
        panic!("gap");
    };
    assert!(matches!(
        &replay.events[..],
        [OutputEvent {
            body: EventBody::Gap { .. },
            ..
        }]
    ));
    let stored = lock(&runtime.store)
        .unwrap()
        .checkpoint(info.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.0, attachment.snapshot.seq);
    assert!(stored.1 == STANDARD.decode(attachment.snapshot.data).unwrap());
    runtime.shutdown(true).unwrap();
}

#[test]
fn primary_exit_is_reaped_even_if_a_descendant_holds_the_terminal_open() {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let runtime = Runtime::open(Config::new(path.clone(), vec![path.clone()])).unwrap();
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] = "trap '' HUP; sleep 2 & exit 7".into();
    let Reply::Started(info) = call(&runtime, ClientId::new(), Operation::Start(launch)) else {
        panic!("start");
    };
    let deadline = Instant::now() + Duration::from_millis(1500);
    loop {
        let state = runtime.session(info.id).unwrap();
        let state = lock(&state).unwrap();
        if matches!(state.info.state, SessionState::Exited) {
            assert_eq!(state.info.exit_code, Some(7));
            let ended = state
                .info
                .ended_at_ms
                .expect("native-observed end must survive the inherited descriptor drain");
            assert!(ended >= state.info.created_at_ms && ended.0 <= now_ms());
            assert!(state.control.is_none());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "primary exit must not wait for descendant EOF"
        );
        drop(state);
        thread::sleep(Duration::from_millis(5));
    }
    runtime.shutdown(false).unwrap();
}

#[test]
fn continuous_clock_expiry_fences_a_lease_even_when_its_awake_deadline_is_future() {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let runtime = Runtime::open(Config::new(path.clone(), vec![path.clone()])).unwrap();
    let client = ClientId::new();
    let marker = path
        .join("native-marker")
        .display()
        .to_string()
        .replace('\'', "'\"'\"'");
    let mut launch = spec(&path, IdempotencyKey::new());
    launch.command[2] = format!("IFS= read -r value; printf '%s' \"$value\" > '{marker}'");
    let Reply::Started(info) = call(&runtime, client, Operation::Start(launch)) else {
        panic!("start");
    };
    let Reply::Control(old) = call(
        &runtime,
        client,
        Operation::TakeControl {
            session: info.id,
            takeover: false,
        },
    ) else {
        panic!("control");
    };
    {
        let state = runtime.session(info.id).unwrap();
        let mut state = lock(&state).unwrap();
        state.deadline = Some(Instant::now() + Duration::from_secs(3600));
        state.deadline_elapsed = Some(clock::elapsed().unwrap() - Duration::from_nanos(1));
    }
    assert!(matches!(
        call(
            &runtime,
            client,
            Operation::Input {
                session: info.id,
                generation: old.generation,
                input_seq: Counter(1),
                data: STANDARD.encode(b"denied\n")
            }
        ),
        Reply::Error(RemoteError {
            code: ErrorCode::StaleGeneration,
            ..
        })
    ));
    let Reply::Control(fresh) = call(
        &runtime,
        client,
        Operation::TakeControl {
            session: info.id,
            takeover: false,
        },
    ) else {
        panic!("fresh control");
    };
    assert!(fresh.generation > old.generation);
    assert!(matches!(
        call(
            &runtime,
            client,
            Operation::Input {
                session: info.id,
                generation: fresh.generation,
                input_seq: Counter(1),
                data: STANDARD.encode(b"allowed\n")
            }
        ),
        Reply::InputAck {
            outcome: ApplicationOutcome::Applied,
            ..
        }
    ));
    let deadline = Instant::now() + Duration::from_secs(3);
    while !runtime.readers_finished() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        std::fs::read(path.join("native-marker")).unwrap(),
        b"allowed"
    );
}
