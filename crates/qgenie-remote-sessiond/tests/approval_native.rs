//! Real vendor I/O plus durable native application state; no public authority is mocked as proved.
use qgenie_remote_adapters::{
    Codex, CodexConfig, LaunchEnvironment, NativeApproval, NativeEvent, Provider,
};
use qgenie_remote_proto::{
    ipc::RemoteScope,
    wire::{AgentEvent, ApprovalDecision, ApprovalRequest, ApprovalWinner, Timestamp},
    *,
};
use qgenie_remote_sessiond::{ApprovalSource, NativeApprovalReservation, NativeApprovals};
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};

fn request(source: &ApprovalSource, native: &NativeApproval) -> ApprovalRequest {
    let id = ApprovalId::new();
    ApprovalRequest {
        id,
        session: source.session,
        owner: source.scope.owner,
        native_id: native.native_id.clone(),
        kind: native.kind.clone(),
        summary: native.summary.clone(),
        request_hash: fingerprint(&(
            id,
            source,
            &native.native_id,
            &native.thread,
            &native.item,
            &native.kind,
            &native.summary,
            &native.allowed_decisions,
        ))
        .unwrap(),
        policy_revision: Counter(1),
        requested_at: Timestamp::from_unix_millis(1).unwrap(),
        expires_at: None,
        allowed_decisions: native.allowed_decisions.clone(),
    }
}
fn winner(request: &ApprovalRequest) -> ApprovalWinner {
    ApprovalWinner {
        approval: request.id,
        id: DecisionId::new(),
        actor: request.owner,
        client: ClientId::new(),
        request_hash: request.request_hash.clone(),
        policy_revision: request.policy_revision,
        decision: ApprovalDecision::AllowOnce,
        committed_at: Timestamp::from_unix_millis(2).unwrap(),
    }
}
async fn next(adapter: &mut Codex) -> NativeEvent {
    timeout(Duration::from_secs(15), adapter.recv())
        .await
        .expect("native event timeout")
        .expect("native owner ended")
}

#[tokio::test]
#[ignore = "requires actual pinned QGR_CODEX_BINARY; native journal/vendor application proof"]
async fn actual_pinned_codex_lost_application_ack_and_cancelled_delivery_are_durable() {
    let binary = PathBuf::from(std::env::var_os("QGR_CODEX_BINARY").expect("set QGR_CODEX_BINARY"));
    let temp = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut model = tokio::task::JoinSet::new();
    let fixture = root.clone();
    model.spawn(async move {
        let mut call=0;loop {
            let (mut stream,_)=listener.accept().await.unwrap();let mut header=Vec::new();while !header.ends_with(b"\r\n\r\n"){assert!(header.len()<32768);header.push(stream.read_u8().await.unwrap());}
            let header=String::from_utf8(header).unwrap();let length=header.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|n|n.trim().parse::<usize>().unwrap())).unwrap();assert!(length<2*1024*1024);let mut body=vec![0;length];stream.read_exact(&mut body).await.unwrap();
            call+=1;assert!(call<=4,"unexpected native model loop");
            let item=if matches!(call,1|3){let command=if call==1{format!("printf x >> {}",fixture.join("applied_once").display())}else{format!("touch {}",fixture.join("cancelled").display())};json!({"type":"function_call","id":format!("fc_{call}"),"call_id":format!("call_{call}"),"name":"exec_command","arguments":json!({"cmd":command,"max_output_chars":1000}).to_string()})}else{json!({"type":"message","id":format!("msg_{call}"),"role":"assistant","status":"completed","content":[{"type":"output_text","text":"native journal fixture complete","annotations":[]}]})};
            let response=format!("journal_{call}");let events=[json!({"type":"response.created","response":{"id":response}}),json!({"type":"response.output_item.added","output_index":0,"item":item}),json!({"type":"response.output_item.done","output_index":0,"item":item}),json!({"type":"response.completed","response":{"id":response,"status":"completed","output":[item],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}})];let mut output="HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n".to_owned();for event in events{output.push_str(&format!("event: {}\ndata: {}\n\n",event["type"].as_str().unwrap(),event));}let _=stream.write_all(output.as_bytes()).await;
        }
    });
    let mut adapter = Codex::launch(CodexConfig {
        kill_on_owner_thread_exit: false,
        binary,
        home: root.join("codex-home"),
        cwd: root.clone(),
        provider: Provider::new(
            format!("http://127.0.0.1:{port}/v1"),
            "fixture".into(),
            "synthetic-only".into(),
        )
        .allow_loopback_fixture(true),
        environment: LaunchEnvironment::default(),
    })
    .await
    .unwrap();
    let handle = adapter.handle();
    let thread = handle.start_thread(&root).await.unwrap();
    let source = ApprovalSource {
        scope: RemoteScope {
            origin: "https://gateway.example.invalid".into(),
            owner: UserId::new(),
            machine: MachineId::new(),
        },
        session: SessionId::new(),
        instance: MessageId::new(),
    };
    let journal_path = root.join("native-journal");
    let journal = NativeApprovals::open(&journal_path).unwrap();
    journal.register_source(&source).unwrap();
    handle
        .prompt(&thread, "exercise the synthetic native journal effect")
        .await
        .unwrap();
    let mut applied = None;
    let mut writes = 0;
    loop {
        match next(&mut adapter).await {
            NativeEvent::Approval(native) => {
                let request = request(&source, &native);
                journal.record(&source, &request).unwrap();
                let winner = winner(&request);
                let NativeApprovalReservation::Send(intent) =
                    journal.reserve(&source, &winner, Counter(1), 2).unwrap()
                else {
                    panic!("first native delivery")
                };
                writes += 1;
                let outcome = handle.answer(&native, winner.decision).await.unwrap();
                assert!(matches!(outcome, ApplicationOutcome::OutcomeUnknown));
                // Lose application recording/acknowledgment after the actual vendor write.
                drop(intent);
                assert!(matches!(
                    journal.reserve(&source, &winner, Counter(1), 3).unwrap(),
                    NativeApprovalReservation::Known(ApplicationOutcome::OutcomeUnknown)
                ));
                applied = Some(winner);
            }
            NativeEvent::Event {
                event: AgentEvent::TurnCompleted { .. },
                ..
            } => break,
            _ => {}
        }
    }
    assert_eq!(writes, 1);
    assert_eq!(std::fs::read(root.join("applied_once")).unwrap(), b"x");
    let turn = handle
        .prompt(&thread, "exercise cancellation before delivery")
        .await
        .unwrap();
    let native = loop {
        if let NativeEvent::Approval(native) = next(&mut adapter).await {
            break native;
        }
    };
    let request = request(&source, &native);
    journal.record(&source, &request).unwrap();
    let cancelled = winner(&request);
    handle.interrupt(&thread, &turn).await.unwrap();
    loop {
        if let NativeEvent::ApprovalResolved { native_id, .. } = next(&mut adapter).await
            && native_id == native.native_id
        {
            journal.resolved(&source, request.id).unwrap();
            break;
        }
    }
    assert!(matches!(
        journal.reserve(&source, &cancelled, Counter(1), 3).unwrap(),
        NativeApprovalReservation::Known(ApplicationOutcome::Cancelled)
    ));
    assert!(!root.join("cancelled").exists());
    adapter.shutdown().await.unwrap();
    model.abort_all();
    while model.join_next().await.is_some() {}
    drop(journal);
    let journal = NativeApprovals::open(&journal_path).unwrap();
    assert!(matches!(
        journal
            .reserve(&source, &applied.unwrap(), Counter(1), 4)
            .unwrap(),
        NativeApprovalReservation::Known(ApplicationOutcome::OutcomeUnknown)
    ));
    assert!(matches!(
        journal.reserve(&source, &cancelled, Counter(1), 4).unwrap(),
        NativeApprovalReservation::Known(ApplicationOutcome::Cancelled)
    ));
}
