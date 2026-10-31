//! Public agent launch and approval delivery with an actual pinned native process.
use super::{fixture::private, metadata};
use qgenie_remote_client::{GatewayClient, LiveHandle};
use qgenie_remote_host::{
    Connector, CredentialOptions, GatewayApi, Identity, NativeClient, RecordingConsent,
};
use qgenie_remote_proto::{ipc::RemoteScope, rest::AccessToken, wire::*, *};
use qgenie_remote_sessiond::{CodexProfile, Config, Runtime};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Owner(Arc<Runtime>);
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.0.shutdown(true);
    }
}

pub(super) async fn prove(
    api: GatewayApi,
    sdk: GatewayClient,
    token: &AccessToken,
    origin: &str,
    ca: &Path,
    admin: &sqlx::PgPool,
) {
    let binary =
        std::env::var_os("QGR_CODEX_BINARY").expect("managed proof requires pinned binary");
    let root = tempfile::tempdir().unwrap();
    let root_path = std::fs::canonicalize(root.path()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
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
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            assert!(length < 2 * 1024 * 1024);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            assert!(!String::from_utf8_lossy(&body).contains("UNSENT_AGENT_READ_ONLY"));
            let resumed = String::from_utf8_lossy(&body).contains("RESUME_CWD_PROOF");
            if resumed {
                assert!(
                    String::from_utf8_lossy(&body).contains("UI_PROMPT_ONLY"),
                    "native history lost its earlier context"
                );
            }
            let item = if resumed
                && !String::from_utf8_lossy(&body).contains("function_call_output")
            {
                serde_json::json!({"type":"function_call","id":"fc_resume","call_id":"call_resume","name":"exec_command","arguments":serde_json::json!({"cmd":"printf resumed > resume-relative-marker","max_output_chars":1000}).to_string()})
            } else if String::from_utf8_lossy(&body).contains("function_call_output")
                || String::from_utf8_lossy(&body).contains("UI_PROMPT_ONLY")
            {
                serde_json::json!({"type":"message","id":"msg_public","role":"assistant","status":"completed","content":[{"type":"output_text","text":"public approved response","annotations":[]}]})
            } else {
                serde_json::json!({"type":"function_call","id":"fc_public","call_id":"call_public","name":"exec_command","arguments":serde_json::json!({"cmd":"printf x >> public-approved-marker","max_output_chars":1000}).to_string()})
            };
            let events = [
                serde_json::json!({"type":"response.created","response":{"id":"resp_public"}}),
                serde_json::json!({"type":"response.output_item.added","output_index":0,"item":item}),
                serde_json::json!({"type":"response.output_item.done","output_index":0,"item":item}),
                serde_json::json!({"type":"response.completed","response":{"id":"resp_public","status":"completed","output":[item],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
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
    let mut identity = Identity::open(
        &root_path.join("identity"),
        origin,
        "managed-public-fixture",
        CredentialOptions {
            prefer_keyring: false,
            allow_private_file: true,
        },
    )
    .await
    .unwrap();
    let binding = api
        .enroll(
            &mut identity,
            token,
            metadata("managed-public-fixture"),
            RecordingConsent::acknowledged(),
        )
        .await
        .unwrap();
    let scope = RemoteScope {
        origin: origin.into(),
        owner: binding.owner,
        machine: binding.machine,
    };
    let state = root_path.join("state");
    let mut config = Config::new(state.clone(), vec![root_path.clone()]);
    config.remote_bindings.push(scope.clone());
    config.codex = Some(CodexProfile {
        binary: binary.into(),
        provider: qgenie_remote_adapters::Provider::new(
            format!("http://127.0.0.1:{port}/v1"),
            "fixture".into(),
            "synthetic-only".into(),
        )
        .allow_loopback_fixture(true),
    });
    let runtime = tokio::task::spawn_blocking(move || Runtime::open(config))
        .await
        .unwrap()
        .unwrap();
    let owner = Owner(runtime.clone());
    let server = tokio::spawn(qgenie_remote_sessiond::serve_runtime(runtime));
    let local = qgenie_remote_client::Client::local(&state);
    tokio::time::timeout(Duration::from_secs(5), async {
        while local.status().await.is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let proxy_directory = root_path.join("native-fault-proxy");
    let drops = super::approval_drops::Proxy::start(&proxy_directory, &state);
    let native = NativeClient::new(proxy_directory, scope).unwrap();
    let connector = Connector::new(
        api.clone(),
        identity.machine_credentials().unwrap(),
        qgenie_remote_host::filesystem::Filesystem::new(
            qgenie_remote_host::filesystem::Roots::open(std::slice::from_ref(&root_path)).unwrap(),
        ),
        Some(ca.into()),
        true,
    )
    .unwrap()
    .with_native(native.clone())
    .unwrap();
    let (stop, mut cancel) = tokio::sync::watch::channel(false);
    let (status, mut updates) =
        tokio::sync::watch::channel(qgenie_remote_host::ConnectionStatus::Disconnected);
    let host = tokio::spawn(async move { connector.run(&mut cancel, status).await });
    tokio::time::timeout(Duration::from_secs(10), async {
        while !matches!(
            *updates.borrow(),
            qgenie_remote_host::ConnectionStatus::Connected {
                activated: true,
                ..
            }
        ) {
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let mut first = Some(sdk.connect(token).await.unwrap());
    let mut second = Some(sdk.connect(token).await.unwrap());
    let handle = first.as_ref().unwrap().handle();
    let mut observation = None;
    assert!(handle.approvals_available());
    let starts = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let machines = sdk.machines(token).await.unwrap();
            if let Some(machine) = machines.iter().find(|machine| {
                machine.id == binding.machine && machine.agents.contains_key("codex")
            }) {
                assert_eq!(machine.agents["codex"].version, "0.159.2");
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let mut launched = Vec::new();
    for index in 0..2 {
        let session = handle
            .start(
                starts.path(),
                binding.owner,
                binding.machine,
                Launch {
                    key: IdempotencyKey::new(),
                    kind: LaunchKind::Agent {
                        agent: "codex".into(),
                        prompt: Some("Run the controlled public approval fixture.".into()),
                        native_resume_id: None,
                    },
                    cwd: root_path.to_str().unwrap().into(),
                    rows: 24,
                    cols: 80,
                },
            )
            .await
            .unwrap();
        if index == 0 {
            observation = Some(
                first
                    .take()
                    .unwrap()
                    .into_observer(binding.machine, session.id),
            );
        }
        let state = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let page = handle.approval_index(None, 200).await.unwrap();
                if let Some(state) = page.states.into_iter().find(|state| {
                    state.request.session == session.id
                        && state.winner.is_none()
                        && !state.cancelled
                }) {
                    break state;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        if index == 0 {
            let other = second.as_ref().unwrap().handle();
            let (a, b) = tokio::join!(
                handle.resolve_approval(&state, ApprovalDecision::AllowOnce),
                other.resolve_approval(&state, ApprovalDecision::AllowOnce)
            );
            assert_eq!(a.unwrap().id, b.unwrap().id);
            second.take().unwrap().close().await;
        } else {
            let token_file = private(token.as_str().as_bytes());
            let ui = tokio::process::Command::new("python3")
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/e2e/remote_approval_tui_smoke.py"
                ))
                .arg(env!("CARGO_BIN_EXE_qgenie-remote"))
                .arg(sdk.origin())
                .arg(token_file.path())
                .arg(ca)
                .arg(state.request.id.to_string())
                .arg(binding.machine.to_string())
                .arg(session.id.to_string())
                .output()
                .await
                .unwrap();
            assert!(
                ui.status.success(),
                "{}",
                String::from_utf8_lossy(&ui.stderr)
            );
        }
        await_application(&handle, state.request.id).await;
        if index == 0 {
            assert!(
                drops.complete(),
                "independent native delivery/ACK drops were not exercised"
            );
            let attempts: i32 =
                sqlx::query_scalar("SELECT attempts FROM qgr.approval_outbox WHERE approval=$1")
                    .bind(state.request.id.to_string())
                    .fetch_one(admin)
                    .await
                    .unwrap();
            assert!(
                attempts >= 3,
                "durable outbox did not retry both distinct losses"
            );
            tokio::time::timeout(Duration::from_secs(10),async {
                loop {
                    let observed=observation.as_mut().unwrap().next().await.unwrap().unwrap();
                    let found=observed.update.events.iter().any(|event|matches!(&event.event,AgentEvent::MessageCompleted{text,..} if text.contains("public approved response"))) || observed.update.snapshot.as_ref().is_some_and(|snapshot|snapshot.agent_state().unwrap().events.iter().any(|event|matches!(&event.event,AgentEvent::MessageCompleted{text,..} if text.contains("public approved response"))));
                    if found {break;}
                }
            }).await.unwrap();
        }
        let marker = root_path.join("public-approved-marker");
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if std::fs::read(&marker).ok().as_deref()
                    == Some(if index == 0 { b"x" } else { b"xx" })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if index != 0 {
            handle
                .attach(binding.machine, session.id, Counter(0))
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                let qgenie_remote_client::RemoteAttachment::Snapshot(snapshot)=handle.capture(binding.machine,session.id).await.unwrap() else {panic!("explicit capture must produce a checkpoint");};
                let state=snapshot.agent_state().unwrap();
                if state.events.iter().any(|event|matches!(&event.event,AgentEvent::MessageCompleted{text,..} if text.contains("public approved response"))) {break;}
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            }).await.unwrap();
            handle.detach(binding.machine, session.id).await.unwrap();
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM qgr.audit_events WHERE action='approval_decided' AND session=$1",
        )
        .bind(session.id.to_string())
        .fetch_one(admin)
        .await
        .unwrap();
        assert_eq!(count, 1);
        launched.push((session.id, state.request.id));
    }
    let controllers: i64 =
        sqlx::query_scalar("SELECT count(*) FROM qgr.control_leases WHERE machine=$1")
            .bind(binding.machine.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert_eq!(controllers, 0, "approval took a controller lease");
    let token_file = private(token.as_str().as_bytes());
    let receipt_cli = tokio::process::Command::new(env!("CARGO_BIN_EXE_qgenie-remote"))
        .arg("--state-dir")
        .arg(starts.path())
        .args(["remote", "--gateway"])
        .arg(sdk.origin())
        .arg("--token-file")
        .arg(token_file.path())
        .arg("--ca-bundle")
        .arg(ca)
        .arg("send")
        .arg(binding.machine.to_string())
        .arg(launched[0].0.to_string())
        .args(["RECEIPT_PROOF", "--prompt", "--wait", "--timeout", "15"])
        .kill_on_drop(true)
        .output();
    let receipt_cli = tokio::time::timeout(Duration::from_secs(25), receipt_cli)
        .await
        .unwrap()
        .unwrap();
    assert!(
        receipt_cli.status.success(),
        "{}",
        String::from_utf8_lossy(&receipt_cli.stderr)
    );
    let values: Vec<serde_json::Value> = serde_json::Deserializer::from_slice(&receipt_cli.stdout)
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(values.len(), 2);
    assert!(
        values[0]["turn"]
            .as_str()
            .is_some_and(|turn| !turn.is_empty())
    );
    assert_eq!(values[0]["control_released"], true);
    assert_eq!(values[1]["outcome"], "completed");
    let ui = tokio::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/e2e/remote_agent_tui_smoke.py"
        ))
        .arg(env!("CARGO_BIN_EXE_qgenie-remote"))
        .arg(sdk.origin())
        .arg(token_file.path())
        .arg(ca)
        .output()
        .await
        .unwrap();
    assert!(
        ui.status.success(),
        "{}",
        String::from_utf8_lossy(&ui.stderr)
    );
    let historical = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(session) = handle
                .sessions(binding.machine)
                .await
                .unwrap()
                .into_iter()
                .find(|session| session.capabilities.resume)
            {
                break session;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let LaunchKind::Agent {
        native_resume_id: Some(thread),
        ..
    } = historical.kind
    else {
        panic!("ended native thread missing");
    };
    let original_home = local
        .sessions()
        .await
        .unwrap()
        .into_iter()
        .find(|session| session.id == historical.id)
        .unwrap()
        .agent
        .unwrap()
        .native_home
        .unwrap_or(historical.id);
    let resumed_cwd = root_path.join("resumed-directory");
    std::fs::create_dir(&resumed_cwd).unwrap();
    let resumed = handle
        .start(
            starts.path(),
            binding.owner,
            binding.machine,
            Launch {
                key: IdempotencyKey::new(),
                kind: LaunchKind::Agent {
                    agent: "codex".into(),
                    prompt: Some("RESUME_CWD_PROOF".into()),
                    native_resume_id: Some(thread),
                },
                cwd: resumed_cwd.to_str().unwrap().into(),
                rows: 24,
                cols: 80,
            },
        )
        .await
        .unwrap();
    assert_ne!(resumed.id, historical.id);
    let approval = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(state) = handle
                .approval_index(None, 200)
                .await
                .unwrap()
                .states
                .into_iter()
                .find(|state| {
                    state.request.session == resumed.id
                        && state.winner.is_none()
                        && !state.cancelled
                })
            {
                break state;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    handle
        .resolve_approval(&approval, ApprovalDecision::AllowOnce)
        .await
        .unwrap();
    await_application(&handle, approval.request.id).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !resumed_cwd.join("resume-relative-marker").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !root_path.join("resume-relative-marker").exists(),
        "resumed native tool used the historical cwd descriptor"
    );
    let native_resumed = local
        .sessions()
        .await
        .unwrap()
        .into_iter()
        .find(|session| session.id == resumed.id)
        .unwrap();
    assert_eq!(
        native_resumed.agent.unwrap().native_home,
        Some(original_home)
    );
    handle
        .terminate_current(binding.machine, resumed.id, true, false)
        .await
        .unwrap();
    launched.push((resumed.id, approval.request.id));
    stop.send_replace(true);
    host.await.unwrap().unwrap();
    let reconciler = sdk.connect(token).await.unwrap();
    let status_reader = reconciler.handle();
    for (_, approval) in &launched {
        assert!(
            status_reader
                .approval_status(*approval)
                .await
                .unwrap()
                .winner
                .is_some(),
            "offline host prevented reconciliation"
        );
    }
    reconciler.close().await;
    let mut admin_stream = qgenie_remote_transport::ipc::connect(&state.join("admin.sock"))
        .await
        .unwrap();
    qgenie_remote_transport::ipc::write_frame(
        &mut admin_stream,
        &serde_json::to_vec(&qgenie_remote_proto::ipc::AdminRequest {
            ipc_major: IPC_MAJOR,
            command: qgenie_remote_proto::ipc::AdminCommand::Shutdown {
                terminate_sessions: true,
            },
        })
        .unwrap(),
    )
    .await
    .unwrap();
    let response = qgenie_remote_transport::ipc::read_frame(&mut admin_stream)
        .await
        .unwrap();
    qgenie_remote_proto::decode_json::<qgenie_remote_proto::ipc::AdminResponse>(&response)
        .unwrap()
        .result
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(owner);
    if let Some(first) = first {
        first.close().await;
    }
    if let Some(second) = second {
        second.close().await;
    }
    if let Some(observation) = observation {
        observation.close().await;
    }
    model.abort();
    eprintln!(
        "Actual pinned native approval → connector → two gateways → SDK/TUI choice → durable outbox → native application passed"
    );
}
async fn await_application(handle: &LiveHandle, id: ApprovalId) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if handle.approval_status(id).await.unwrap().outcome.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
}
