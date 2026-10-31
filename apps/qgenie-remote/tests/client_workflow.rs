//! Actual CLI processes over TLS/WSS. The peer accepts only observation requests.
#[path = "support/identity.rs"]
#[allow(dead_code)]
mod identity;
use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message as WsMessage, WebSocket},
    },
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::SinkExt;
use qgenie_remote_proto::{wire::*, *};
use serde_json::Value;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Clone)]
struct Peer {
    session: Session,
    agent: AgentState,
    connections: Arc<AtomicUsize>,
    after: Arc<Mutex<Vec<Counter>>>,
    reconnect: bool,
    ui: bool,
    protocol_error: bool,
    leases: Arc<AtomicUsize>,
    drop_connection: Arc<tokio::sync::Notify>,
}
impl Peer {
    fn new(reconnect: bool) -> Self {
        let machine = MachineId::new();
        let mut agent = AgentState::default();
        for event in [
            AgentEvent::TurnStarted {
                turn: "wanted".into(),
            },
            AgentEvent::MessageCompleted {
                message: "m".into(),
                text: "readable result\n\x1b[2Jsafe".into(),
            },
            AgentEvent::TurnCompleted {
                turn: "wanted".into(),
                input_tokens: Counter(1),
                output_tokens: Counter(2),
                outcome: Some(TurnOutcome::Completed),
            },
        ] {
            agent
                .push(SequencedEvent {
                    seq: Counter(agent.seq.0 + 1),
                    at: Timestamp::from_unix_millis(1).unwrap(),
                    event,
                })
                .unwrap();
        }
        Self {
            session: Session {
                id: SessionId::new(),
                machine,
                owner: UserId::new(),
                kind: LaunchKind::Agent {
                    agent: "codex".into(),
                    prompt: None,
                    native_resume_id: None,
                },
                cwd: "/fixture".into(),
                origin: Origin::Managed,
                state: SessionState::Running,
                created_at: Timestamp::from_unix_millis(1).unwrap(),
                ended_at: None,
                head: agent.seq,
                capabilities: Capabilities::default(),
                recording_warning: None,
            },
            agent,
            connections: Default::default(),
            after: Default::default(),
            reconnect,
            ui: false,
            protocol_error: false,
            leases: Default::default(),
            drop_connection: Default::default(),
        }
    }
}
async fn send(socket: &mut WebSocket, envelope: Envelope) {
    socket
        .send(WsMessage::Text(
            serde_json::to_string(&envelope).unwrap().into(),
        ))
        .await
        .unwrap();
}
fn reply(request: &Envelope, message: Message) -> Envelope {
    let mut envelope = Envelope::new(message);
    envelope.re = Some(request.id);
    envelope.machine = request.machine;
    envelope.session = request.session;
    envelope
}
async fn endpoint(State(peer): State<Peer>, upgrade: WebSocketUpgrade) -> axum::response::Response {
    upgrade.on_upgrade(move |mut socket|async move {
        let connection=peer.connections.fetch_add(1,Ordering::SeqCst);
        let Some(Ok(WsMessage::Text(text)))=socket.recv().await else {panic!("hello expected")};
        let hello:Envelope=serde_json::from_str(&text).unwrap();
        let Message::Hello(hello)=hello.message else {panic!("hello expected")};
        let available=["routing-v1","machine-pages-v1","auth-refresh-v1","session-pages-v1","session-replay-v1","agent-events-v1","approval-status-v1","session-control-v1","durable-approvals-v1"].into_iter().map(String::from).collect();
        send(&mut socket,Envelope::new(Message::Welcome(negotiate(&hello,&[Version{major:1,minor:0}],&available).unwrap()))).await;
        let client_id=ClientId::new();
        send(&mut socket,Envelope::new(Message::ConnectionReady{client:client_id,host_generation:None,activated:false})).await;
        if peer.protocol_error {socket.send(WsMessage::Text("{}".into())).await.unwrap();return;}
        loop {
            let incoming=tokio::select! {value=socket.recv()=>value,_=peer.drop_connection.notified(),if peer.ui && connection==0=>{let _=socket.close().await;return;}};
            let Some(Ok(frame))=incoming else {break};
            let WsMessage::Text(text)=frame else {continue};
            let request:Envelope=serde_json::from_str(&text).unwrap();
            match &request.message {
                Message::SessionListPage{..} if peer.ui=>send(&mut socket,reply(&request,Message::SessionPage{sessions:vec![peer.session.clone()],next:None})).await,
                Message::LeaseRequest{takeover:false} if peer.ui=>{peer.leases.fetch_add(1,Ordering::SeqCst);send(&mut socket,reply(&request,Message::LeaseGranted(ControlLease{holder:client_id,host_generation:Counter(1),generation:Counter(1),expires_at:Timestamp::from_unix_millis(4102444800000).unwrap(),last_input:Counter(0)}))).await;},
                Message::SessionGet{session}=>{assert_eq!(*session,peer.session.id);send(&mut socket,reply(&request,Message::SessionFound(peer.session.clone()))).await;},
                Message::ApprovalIndex{..}=>send(&mut socket,reply(&request,Message::ApprovalPage{states:Vec::new(),next:None})).await,
                Message::Attach{after} if peer.reconnect=>{
                    peer.after.lock().unwrap().push(*after);
                    let seq=Counter(connection as u64+1);
                    send(&mut socket,reply(&request,Message::Replay{floor:Counter(1),head:seq,events:vec![SequencedEvent{seq,at:Timestamp::from_unix_millis(1).unwrap(),event:AgentEvent::Idle}],more:false,history_truncated:false})).await;
                    if connection==0 { let _=socket.close().await; return; }
                },
                Message::Attach{..}|Message::Capture=>{
                    if let Message::Attach{after}=&request.message {peer.after.lock().unwrap().push(*after);}
                    send(&mut socket,reply(&request,Message::Snapshot{snapshot:SnapshotPage{epoch:"fixture".into(),seq:peer.agent.seq,rows:24,cols:80,format:SnapshotFormat::AgentStateV1,page:0,total_pages:1,data:STANDARD.encode(peer.agent.encode().unwrap())}})).await;
                    send(&mut socket,reply(&request,Message::Head{floor:Counter(1),head:peer.agent.seq,last_input:Counter(0)})).await;
                },
                Message::ReplayRequest{after}=>send(&mut socket,reply(&request,Message::Replay{floor:Counter(1),head:*after,events:vec![],more:false,history_truncated:false})).await,
                Message::Detach=>{},
                _=>panic!("observation must not request control, input, start, approval choices, or termination"),
            }
        }
    })
}
async fn cli(
    server: &identity::Server,
    tls: &identity::Tls,
    peer: &Peer,
    args: &[&str],
) -> std::process::Output {
    let token = identity::private(b"synthetic-client-workflow-token");
    let root = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_qgenie-remote"));
    command
        .arg("--state-dir")
        .arg(root.path())
        .arg("remote")
        .arg("--gateway")
        .arg(&server.origin)
        .arg("--ca-bundle")
        .arg(tls.ca.path())
        .arg("--token-file")
        .arg(token.path())
        .arg(args[0])
        .arg(peer.session.machine.to_string())
        .arg(peer.session.id.to_string())
        .args(&args[1..])
        .kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(8), command.output())
        .await
        .unwrap()
        .unwrap()
}
#[tokio::test]
async fn readable_output_activity_exact_turn_and_timeout_use_only_observations() {
    let tls = identity::Tls::new();
    let peer = Peer::new(false);
    let server = tls
        .bind()
        .await
        .serve(
            Router::new()
                .route("/v1/connect/client", get(endpoint))
                .with_state(peer.clone()),
        )
        .await;
    let read = cli(&server, &tls, &peer, &["read", "--lines", "10"]).await;
    assert!(
        read.status.success(),
        "{}",
        String::from_utf8_lossy(&read.stderr)
    );
    let text = String::from_utf8(read.stdout).unwrap();
    assert!(text.contains("readable result"));
    assert!(!text.contains('\x1b'));
    let activity = cli(&server, &tls, &peer, &["activity"]).await;
    assert!(
        activity.status.success(),
        "{}",
        String::from_utf8_lossy(&activity.stderr)
    );
    let report: Value = serde_json::from_slice(&activity.stdout).unwrap();
    assert_eq!(report["lifetime"], "running");
    assert_eq!(report["activity"]["state"], "completed");
    assert_eq!(report["approval_status_complete"], true);
    let wait = cli(
        &server,
        &tls,
        &peer,
        &["wait", "--turn", "wanted", "--timeout", "2"],
    )
    .await;
    assert!(
        wait.status.success(),
        "{}",
        String::from_utf8_lossy(&wait.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&wait.stdout).unwrap()["outcome"],
        "completed"
    );
    let wait = cli(
        &server,
        &tls,
        &peer,
        &["wait", "--turn", "different", "--timeout", "1"],
    )
    .await;
    assert!(
        wait.status.success(),
        "{}",
        String::from_utf8_lossy(&wait.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&wait.stdout).unwrap()["outcome"],
        "timeout"
    );
}
#[tokio::test]
async fn event_recovery_resumes_after_the_last_observed_sequence() {
    let tls = identity::Tls::new();
    let peer = Peer::new(true);
    let server = tls
        .bind()
        .await
        .serve(
            Router::new()
                .route("/v1/connect/client", get(endpoint))
                .with_state(peer.clone()),
        )
        .await;
    let output = cli(
        &server,
        &tls,
        &peer,
        &["events", "--limit", "2", "--timeout", "5"],
    )
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["seq"], "1");
    assert_eq!(records[1]["seq"], "2");
    assert_eq!(*peer.after.lock().unwrap(), vec![Counter(0), Counter(1)]);
    assert_eq!(peer.connections.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn host_setup_validates_roots_writes_private_settings_and_does_not_replace_them() {
    use std::os::unix::fs::MetadataExt;
    let root = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let allowed = tempfile::tempdir().unwrap();
    let run = || {
        let mut c = tokio::process::Command::new(env!("CARGO_BIN_EXE_qgenie-remote"));
        c.arg("--state-dir")
            .arg(root.path())
            .args([
                "host",
                "setup",
                "--gateway",
                "https://gateway.example",
                "--allow-root",
            ])
            .arg(allowed.path());
        c
    };
    let output = run().output().await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = root.path().join("host.json");
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    let before = std::fs::read(&path).unwrap();
    assert!(!run().output().await.unwrap().status.success());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(!root.path().join("identity").exists());
}

#[tokio::test]
async fn saved_setup_device_sign_in_and_new_invocation_never_persist_user_credentials() {
    use serde_json::json;
    let tls = identity::Tls::new();
    let provider = identity::Provider::default();
    let server = tls
        .bind()
        .await
        .serve(provider.router().route(
            "/v1/machines",
            get(|headers: axum::http::HeaderMap| async move {
                assert!(
                    headers["authorization"]
                        .to_str()
                        .unwrap()
                        .starts_with("Bearer eyJ")
                );
                axum::Json(Vec::<Machine>::new())
            }),
        ))
        .await;
    let config = provider.config(&server.origin, tls.ca.path().into());
    let config=identity::private(&serde_json::to_vec(&json!({"client_id":config.client_id,"remote_scope":config.remote_scope,"authorization_endpoint":config.authorization_endpoint,"token_endpoint":config.token_endpoint,"device_endpoint":config.device_endpoint,"ca_bundle":config.ca_bundle})).unwrap());
    let root = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let mut setup = tokio::process::Command::new(env!("CARGO_BIN_EXE_qgenie-remote"));
    let output = setup
        .arg("--state-dir")
        .arg(root.path())
        .args(["remote", "setup", "--gateway"])
        .arg(&server.origin)
        .arg("--oauth-config")
        .arg(config.path())
        .arg("--ca-bundle")
        .arg(tls.ca.path())
        .arg("--device")
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..2 {
        let output = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_qgenie-remote"))
                .arg("--state-dir")
                .arg(root.path())
                .args(["remote", "machines"])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            json!([])
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("FIXTURE-ONLY"));
    }
    assert_eq!(provider.polls.load(Ordering::SeqCst), 3);
    for file in std::fs::read_dir(root.path()).unwrap() {
        let bytes = std::fs::read(file.unwrap().path()).unwrap();
        let contents = String::from_utf8_lossy(&bytes);
        assert!(!contents.contains("synthetic-refresh"));
        assert!(!contents.contains("eyJ"));
        assert!(!contents.contains("access_token"));
        assert!(!contents.contains("refresh_token"));
    }
}

#[tokio::test]
async fn wait_reports_lost_lifetime_and_cannot_treat_truncated_history_as_a_failed_turn() {
    for (lifetime, truncated, target, expected) in [
        (SessionState::Lost, false, "wanted", "lost"),
        (SessionState::Exited, true, "missing", "history_unavailable"),
        (SessionState::Exited, false, "missing", "ended_before_turn"),
    ] {
        let tls = identity::Tls::new();
        let mut peer = Peer::new(false);
        peer.session.state = lifetime;
        peer.agent.history_truncated = truncated;
        let server = tls
            .bind()
            .await
            .serve(
                Router::new()
                    .route("/v1/connect/client", get(endpoint))
                    .with_state(peer.clone()),
            )
            .await;
        let output = cli(
            &server,
            &tls,
            &peer,
            &["wait", "--turn", target, "--timeout", "2"],
        )
        .await;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["outcome"],
            expected
        );
    }
}

#[tokio::test]
async fn actual_tui_recovers_output_discards_control_and_restores_terminal_on_sigterm() {
    let tls = identity::Tls::new();
    let mut peer = Peer::new(false);
    peer.ui = true;
    peer.session.capabilities.input = true;
    peer.session.recording_warning = Some("fixture recording warning".into());
    async fn machines(State(peer): State<Peer>) -> axum::Json<Vec<Machine>> {
        axum::Json(vec![Machine {
            id: peer.session.machine,
            owner: peer.session.owner,
            name: "Fixture".into(),
            hostname: "fixture".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            cli_version: "fixture".into(),
            labels: Default::default(),
            agents: Default::default(),
            online: true,
            last_seen: Timestamp::from_unix_millis(1).unwrap(),
            sessions: 1,
        }])
    }
    let server = tls
        .bind()
        .await
        .serve(
            Router::new()
                .route("/v1/connect/client", get(endpoint))
                .route("/v1/machines", get(machines))
                .route(
                    "/drop",
                    axum::routing::post(|State(peer): State<Peer>| async move {
                        peer.drop_connection.notify_one();
                    }),
                )
                .with_state(peer.clone()),
        )
        .await;
    let token = identity::private(b"synthetic-ui-token");
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/e2e/remote_recovery_tui_smoke.py"
            ))
            .arg(env!("CARGO_BIN_EXE_qgenie-remote"))
            .arg(&server.origin)
            .arg(token.path())
            .arg(tls.ca.path())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(peer.connections.load(Ordering::SeqCst), 2);
    assert_eq!(
        peer.leases.load(Ordering::SeqCst),
        1,
        "control must be requested only by the user's F2"
    );
    assert_eq!(*peer.after.lock().unwrap(), vec![Counter(0), Counter(3)]);
}

#[tokio::test]
async fn malformed_gateway_frames_stop_observation_without_reconnect() {
    let tls = identity::Tls::new();
    let mut peer = Peer::new(false);
    peer.protocol_error = true;
    let server = tls
        .bind()
        .await
        .serve(
            Router::new()
                .route("/v1/connect/client", get(endpoint))
                .with_state(peer.clone()),
        )
        .await;
    let output = cli(&server, &tls, &peer, &["events", "--timeout", "3"]).await;
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("protocol"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        peer.connections.load(Ordering::SeqCst),
        1,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
