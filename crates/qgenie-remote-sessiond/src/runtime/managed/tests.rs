use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires actual pinned QGR_CODEX_BINARY; composed managed-runtime proof"]
async fn actual_managed_owner_deduplicates_records_replays_and_survives_caller_loss() {
    let binary = PathBuf::from(std::env::var_os("QGR_CODEX_BINARY").expect("set pinned binary"));
    let temp = super::super::tests::directory();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let marker = root.join("approved-marker");
    let command = format!("printf x >> '{}'", marker.display());
    let model = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                assert!(header.len() < 32768);
                header.push(stream.read_u8().await.unwrap());
            }
            let header = String::from_utf8(header).unwrap();
            let length = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|length| length.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            assert!(length < 2 * 1024 * 1024);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            let call = observed.fetch_add(1, Ordering::SeqCst);
            let item = if call == 0 {
                serde_json::json!({"type":"function_call","id":"fc_owned","call_id":"call_owned","name":"exec_command","arguments":serde_json::json!({"cmd":command,"max_output_chars":1000}).to_string()})
            } else {
                serde_json::json!({"type":"message","id":"msg_owned","role":"assistant","status":"completed","content":[{"type":"output_text","text":"owned response PRIVATE_LITERAL","annotations":[]}]})
            };
            let events = [
                serde_json::json!({"type":"response.created","response":{"id":"resp_owned"}}),
                serde_json::json!({"type":"response.output_item.added","output_index":0,"item":item}),
                serde_json::json!({"type":"response.output_item.done","output_index":0,"item":item}),
                serde_json::json!({"type":"response.completed","response":{"id":"resp_owned","status":"completed","output":[item],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}}),
            ];
            let mut output = String::from(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            );
            for event in events {
                output.push_str(&format!(
                    "event: {}\ndata: {}\n\n",
                    event["type"].as_str().unwrap(),
                    event
                ));
            }
            let _ = stream.write_all(output.as_bytes()).await;
        }
    });
    let scope = RemoteScope {
        origin: "https://managed.example".into(),
        owner: UserId::new(),
        machine: MachineId::new(),
    };
    let mut config = Config::new(root.join("state"), vec![root.clone()]);
    config.replay_events = 2;
    config.remote_bindings.push(scope.clone());
    config.recording.redaction =
        crate::RedactionRules::literals(vec![b"PRIVATE_LITERAL".to_vec()]).unwrap();
    config.codex = Some(CodexProfile {
        binary,
        provider: Provider::new(
            format!("http://127.0.0.1:{port}/v1"),
            "fixture".into(),
            "synthetic-only".into(),
        )
        .allow_loopback_fixture(true),
    });
    let runtime = Runtime::open(config.clone()).unwrap();
    use qgenie_remote_proto::ipc::{DispatchFrame, HostActivation, RemoteContext};
    let connection = ClientId::new();
    let DispatchFrame::AccessChallengeIssued { nonce, .. } =
        runtime.dispatch_frame(DispatchFrame::AccessChallenge {
            scope: scope.clone(),
            connection,
            host_generation: Counter(1),
        })
    else {
        panic!("challenge rejected");
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
        scope: scope.clone(),
        actor: scope.owner,
        client: ClientId::new(),
        connection,
        host_generation: Counter(1),
        controller_generation: None,
        access_nonce: nonce,
        remaining_access_ms: 10000,
        policy_revision: Counter(1),
    };
    let launch = Launch {
        key: IdempotencyKey::new(),
        kind: LaunchKind::Agent {
            agent: "codex".into(),
            prompt: Some("respond once".into()),
            native_resume_id: None,
        },
        cwd: root.display().to_string(),
        rows: 24,
        cols: 80,
    };
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let joins = (0..8)
        .map(|_| {
            let runtime = runtime.clone();
            let launch = launch.clone();
            let scope = scope.clone();
            let barrier = barrier.clone();
            let context = context.clone();
            thread::spawn(move || {
                barrier.wait();
                let fence = lock(&runtime.authority).unwrap().fence(&context).unwrap();
                runtime
                    .start_agent(launch, Some(scope), Some(fence))
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let sessions = joins
        .into_iter()
        .map(|join| join.join().unwrap())
        .collect::<Vec<_>>();
    let id = sessions[0].id;
    assert!(
        sessions
            .iter()
            .all(|info| info.id == id && info.agent.as_ref().unwrap().native_thread.is_some())
    );
    assert!(matches!(
        runtime.shutdown(false).unwrap_err().code,
        ErrorCode::Forbidden
    ));
    let mut changed = launch.clone();
    changed.kind = LaunchKind::Agent {
        agent: "codex".into(),
        prompt: Some("different".into()),
        native_resume_id: None,
    };
    assert!(matches!(
        runtime
            .start_agent(changed, Some(scope.clone()), None)
            .unwrap_err()
            .code,
        ErrorCode::IdempotencyConflict
    ));
    let deadline = Instant::now() + Duration::from_secs(10);
    let approval = loop {
        if let Some(approval) = runtime
            .approvals
            .pending(&scope)
            .unwrap()
            .into_iter()
            .next()
        {
            break approval;
        }
        assert!(
            Instant::now() < deadline,
            "native approval was not journaled"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(!marker.exists());
    assert!(
        runtime.list().unwrap()[0].controller.is_none(),
        "approval must not require a controller"
    );
    let winner = wire::ApprovalWinner {
        approval: approval.id,
        id: DecisionId::new(),
        actor: scope.owner,
        client: context.client,
        request_hash: approval.request_hash.clone(),
        policy_revision: Counter(1),
        decision: wire::ApprovalDecision::AllowOnce,
        committed_at: wire::Timestamp::from_unix_millis(now_ms()).unwrap(),
    };
    let mut delivery = wire::Envelope::new(wire::Message::ApprovalCommitted(winner.clone()));
    delivery.machine = Some(scope.machine);
    delivery.session = Some(id);
    // Discard the first application reply, then reconcile the original winner.
    let _ = runtime.native_approval_rpc(&context, delivery).unwrap();
    let mut duplicate = wire::Envelope::new(wire::Message::ApprovalCommitted(winner));
    duplicate.machine = Some(scope.machine);
    duplicate.session = Some(id);
    let DispatchFrame::RemoteReply(reply) =
        runtime.native_approval_rpc(&context, duplicate).unwrap()
    else {
        panic!("approval reconciliation rejected");
    };
    assert!(matches!(
        reply.message,
        wire::Message::ApprovalApplication {
            outcome: ApplicationOutcome::OutcomeUnknown,
            ..
        }
    ));
    assert!(matches!(
        runtime
            .start_scoped(
                StartSpec {
                    key: launch.key,
                    command: vec!["/bin/true".into()],
                    cwd: launch.cwd.clone(),
                    rows: 24,
                    cols: 80
                },
                Some(scope.clone())
            )
            .unwrap_err()
            .code,
        ErrorCode::IdempotencyConflict
    ));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (_, events, _) = lock(&runtime.store)
            .unwrap()
            .replay(id, Counter(0))
            .unwrap();
        if events.iter().any(|event| {
            matches!(
                event.body,
                EventBody::Agent(AgentEvent::TurnCompleted { .. })
            )
        }) {
            assert!(events.iter().any(|event| matches!(&event.body, EventBody::Agent(AgentEvent::MessageCompleted { text, .. }) if text.contains("owned response"))));
            break;
        }
        assert!(Instant::now() < deadline, "native completion timeout");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "duplicate start must never replay initial prompt"
    );
    let saved_thread = runtime.list().unwrap()[0]
        .agent
        .as_ref()
        .unwrap()
        .native_thread
        .clone();
    let client = ClientId::new();
    let Reply::Control(lease) = super::super::tests::call(
        &runtime,
        client,
        Operation::TakeControl {
            session: id,
            takeover: false,
        },
    ) else {
        panic!("agent control rejected");
    };
    let input = qgenie_remote_proto::wire::SessionInput::PromptReceipt("second prompt".into());
    let receipt = runtime
        .agent_input(
            client,
            id,
            lease.generation,
            Counter(1),
            input.clone(),
            None,
        )
        .unwrap();
    assert!(matches!(receipt.outcome, ApplicationOutcome::Applied));
    let turn = receipt
        .turn
        .expect("native request must return its exact turn");
    let duplicate = runtime
        .agent_input(client, id, lease.generation, Counter(1), input, None)
        .unwrap();
    assert!(matches!(duplicate.outcome, ApplicationOutcome::Applied));
    assert_eq!(duplicate.turn.as_ref(), Some(&turn));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (_, events, _) = lock(&runtime.store)
            .unwrap()
            .replay(id, Counter(0))
            .unwrap();
        if events.iter().any(|event| matches!(&event.body, EventBody::Agent(AgentEvent::TurnCompleted { turn: completed, .. }) if completed == &turn)) { break; }
        assert!(
            Instant::now() < deadline,
            "receipt must identify the completed native turn"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while calls.load(Ordering::SeqCst) < 3 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        std::fs::read(&marker).unwrap(),
        b"x",
        "approval retry must not execute a second native answer"
    );
    assert!(matches!(
        runtime
            .agent_input(
                client,
                id,
                lease.generation,
                Counter(1),
                qgenie_remote_proto::wire::SessionInput::Prompt("changed bytes".into()),
                None
            )
            .unwrap_err()
            .code,
        ErrorCode::IdempotencyConflict
    ));
    // Capture a grant, then replace the controller before its actual OS effect.
    let (_, _, grant) = runtime
        .agent_action(
            client,
            id,
            lease.generation,
            Some(qgenie_remote_proto::wire::SessionInput::Prompt(
                "must not send".into(),
            )),
            None,
        )
        .unwrap();
    {
        let owned = runtime.session(id).unwrap();
        let _busy = owned.lock().unwrap();
        assert_eq!(
            grant
                .perform(&mut || panic!("busy state admitted an OS write"))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    assert!(matches!(
        super::super::tests::call(
            &runtime,
            ClientId::new(),
            Operation::TakeControl {
                session: id,
                takeover: true
            }
        ),
        Reply::Control(_)
    ));
    let mut delivered = false;
    assert_eq!(
        grant
            .perform(&mut || {
                delivered = true;
                Ok(1)
            })
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(!delivered);
    // Fetch before forced termination introduces its honest unknown-EOF gap.
    let worker = runtime.recording_worker().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(crate::SpoolItem::Content(batch)) = worker.fetch_scoped(scope.clone()).unwrap()
        {
            assert!(matches!(
                batch.stream,
                qgenie_remote_proto::wire::RecordingStream::Events
            ));
            let compressed = STANDARD.decode(batch.data).unwrap();
            let normalized = zstd::stream::decode_all(compressed.as_slice()).unwrap();
            let mut captured = Vec::new();
            let mut decision_recorded = false;
            let mut application_recorded = false;
            for line in normalized
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
            {
                let record: RecordingRecord = serde_json::from_slice(line).unwrap();
                if let RecordedContent::AgentText { data, channel, .. } = record.content {
                    decision_recorded |= channel == RecordingChannel::ApprovalDecision;
                    application_recorded |= channel == RecordingChannel::ApprovalApplication;
                    captured.extend(STANDARD.decode(data).unwrap());
                }
            }
            let captured = String::from_utf8(captured).unwrap();
            assert!(captured.contains("owned response"));
            assert!(!captured.contains("PRIVATE_LITERAL"));
            assert!(
                decision_recorded && application_recorded,
                "native answer did not record its decision and application separately"
            );
            break;
        }
        assert!(Instant::now() < deadline, "structured capture absent");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(worker);
    let weak = Arc::downgrade(&runtime);
    drop(runtime);
    assert!(
        Runtime::open(config.clone()).is_err(),
        "caller loss must not release native ownership"
    );
    let runtime = weak.upgrade().unwrap();
    let Reply::Ok = super::super::tests::call(
        &runtime,
        ClientId::new(),
        Operation::Terminate {
            session: id,
            confirmed: true,
        },
    ) else {
        panic!("explicit termination rejected");
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while !runtime.readers_finished() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let info = runtime.list().unwrap().remove(0);
    assert!(matches!(info.state, SessionState::Exited));
    assert!(info.ended_at_ms.is_some());
    drop(runtime);
    let recovered = Runtime::open_after_previous_owner(config).unwrap();
    let info = recovered.list().unwrap().remove(0);
    let original_instance = info.agent.as_ref().unwrap().native_instance;
    let snapshot = recovered
        .snapshot(&lock(recovered.session(id).unwrap().as_ref()).unwrap())
        .unwrap();
    assert_eq!(snapshot.format, SnapshotFormat::AgentStateV1);
    let view = AgentState::decode(&snapshot.data, info.head).unwrap();
    assert!(view.events.iter().any(|event| matches!(&event.event, AgentEvent::MessageCompleted { text, .. } if text.contains("owned response"))), "structured checkpoint must survive replay pruning and restart");
    assert_eq!(info.agent.unwrap().native_thread, saved_thread);
    assert!(matches!(info.state, SessionState::Exited));
    let resume = Launch {
        key: IdempotencyKey::new(),
        kind: LaunchKind::Agent {
            agent: "codex".into(),
            prompt: None,
            native_resume_id: saved_thread.clone(),
        },
        cwd: root.to_str().unwrap().into(),
        rows: 24,
        cols: 80,
    };
    let resumed = recovered
        .start_agent(resume.clone(), Some(scope.clone()), None)
        .unwrap();
    assert_ne!(resumed.id, id);
    let identity = resumed.agent.as_ref().unwrap();
    assert_eq!(identity.native_home, Some(id));
    assert_eq!(identity.native_thread, saved_thread);
    assert_ne!(identity.native_instance, original_instance);
    let mut concurrent = resume;
    concurrent.key = IdempotencyKey::new();
    assert_eq!(
        recovered
            .start_agent(concurrent, Some(scope), None)
            .unwrap_err()
            .code,
        ErrorCode::Forbidden,
        "history resumed into two live owners"
    );
    assert!(matches!(
        super::super::tests::call(
            &recovered,
            ClientId::new(),
            Operation::Terminate {
                session: resumed.id,
                confirmed: true
            }
        ),
        Reply::Ok
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !recovered.readers_finished() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    recovered.shutdown(false).unwrap();
    model.abort();
}
