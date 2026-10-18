#![cfg(feature = "codex")]
#[cfg(target_os = "linux")]
#[path = "support/codex_descriptor_cwd.rs"]
mod descriptor_cwd;
#[cfg(target_os = "linux")]
#[path = "support/codex_ownership.rs"]
mod ownership;
use qgenie_remote_adapters::{Codex, CodexConfig, LaunchEnvironment, NativeEvent, Provider};
use qgenie_remote_proto::{
    ApplicationOutcome,
    wire::{AgentEvent, ApprovalDecision},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};
fn config(binary: &Path, root: &Path, port: u16) -> CodexConfig {
    CodexConfig {
        kill_on_owner_thread_exit: false,
        binary: binary.into(),
        home: root.join("native-home"),
        cwd: root.into(),
        provider: Provider::new(
            format!("http://127.0.0.1:{port}/v1"),
            "fixture".into(),
            "synthetic-only".into(),
        )
        .allow_loopback_fixture(true),
        environment: LaunchEnvironment::default(),
    }
}
async fn next(adapter: &mut Codex) -> NativeEvent {
    timeout(Duration::from_secs(15), adapter.recv())
        .await
        .expect("native event timeout")
        .expect("native process ended")
}
/// The actual pinned app-server consumes local SSE; no provider credentials are used.
#[tokio::test]
#[ignore = "requires QGR_CODEX_BINARY selecting the pinned Codex binary; run with --ignored"]
async fn actual_pinned_codex_approval_stream_steer_interrupt_and_restart_resume() {
    let binary = PathBuf::from(std::env::var_os("QGR_CODEX_BINARY").expect("set QGR_CODEX_BINARY"));
    let temp = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fixture_root = root.clone();
    let (active_tx, active_rx) = tokio::sync::oneshot::channel();
    let model = tokio::spawn(async move {
        let mut active_tx = Some(active_tx);
        let mut calls = 0;
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
            let mut bytes = vec![0; length];
            stream.read_exact(&mut bytes).await.unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(
                request["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["name"] == "exec_command")
            );
            calls += 1;
            if calls == 3 {
                let _ = active_tx.take().unwrap().send(());
                let mut end = [0u8; 1];
                let _ = timeout(Duration::from_secs(15), stream.read(&mut end)).await;
                continue;
            }
            let item = if calls == 1 || calls == 4 {
                json!({"type":"function_call","id":format!("fc_{calls}"),"call_id":format!("call_{calls}"),"name":"exec_command","arguments":json!({"cmd":format!("touch {}",fixture_root.join(if calls==1 {"applied"} else {"cancelled"}).display()),"max_output_chars":1000}).to_string()})
            } else {
                json!({"type":"message","id":format!("msg_{calls}"),"role":"assistant","status":"completed","content":[{"type":"output_text","text":"native fixture complete","annotations":[]}]})
            };
            let response = format!("resp_{calls}");
            let events = [
                json!({"type":"response.created","response":{"id":response}}),
                json!({"type":"response.output_item.added","output_index":0,"item":item}),
                json!({"type":"response.output_item.done","output_index":0,"item":item}),
                json!({"type":"response.completed","response":{"id":response,"status":"completed","output":[item],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}}),
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
    let mut adapter = Codex::launch(config(&binary, &root, port)).await.unwrap();
    let thread = adapter.start_thread(&root).await.unwrap();
    adapter.prompt(&thread, "native fixture").await.unwrap();
    let mut approval_seen = false;
    let mut message_seen = false;
    loop {
        match next(&mut adapter).await {
            NativeEvent::Approval(approval) => {
                assert!(!approval_seen);
                approval_seen = true;
                assert!(matches!(
                    adapter
                        .answer(&approval, ApprovalDecision::AllowOnce)
                        .await
                        .unwrap(),
                    ApplicationOutcome::OutcomeUnknown
                ));
                assert!(matches!(
                    adapter
                        .answer(&approval, ApprovalDecision::AllowOnce)
                        .await
                        .unwrap(),
                    ApplicationOutcome::OutcomeUnknown
                ));
            }
            NativeEvent::Event {
                event: AgentEvent::MessageCompleted { text, .. },
                ..
            } => message_seen = text == "native fixture complete",
            NativeEvent::Event {
                event:
                    AgentEvent::TurnCompleted {
                        input_tokens,
                        output_tokens,
                        ..
                    },
                ..
            } => {
                assert_eq!((input_tokens.0, output_tokens.0), (2, 4));
                break;
            }
            _ => {}
        }
    }
    assert!(approval_seen && message_seen && root.join("applied").exists());
    let turn = adapter.prompt(&thread, "hold turn").await.unwrap();
    timeout(Duration::from_secs(10), active_rx)
        .await
        .unwrap()
        .unwrap();
    adapter
        .steer(&thread, &turn, "steer fixture")
        .await
        .unwrap();
    adapter.interrupt(&thread, &turn).await.unwrap();
    loop {
        if matches!(
            next(&mut adapter).await,
            NativeEvent::Event {
                event: AgentEvent::TurnCompleted { .. },
                ..
            }
        ) {
            break;
        }
    }
    let turn = adapter.prompt(&thread, "cancel approval").await.unwrap();
    let approval = loop {
        if let NativeEvent::Approval(approval) = next(&mut adapter).await {
            break approval;
        }
    };
    adapter.interrupt(&thread, &turn).await.unwrap();
    loop {
        if let NativeEvent::ApprovalResolved { native_id, .. } = next(&mut adapter).await
            && native_id == approval.native_id
        {
            break;
        }
    }
    assert!(matches!(
        adapter
            .answer(&approval, ApprovalDecision::AllowOnce)
            .await
            .unwrap(),
        ApplicationOutcome::Cancelled
    ));
    assert!(!root.join("cancelled").exists());
    let pid = adapter.native_process_id().unwrap();
    let stale_handle = adapter.handle();
    struct Expired;
    impl qgenie_remote_adapters::NativeEffectAuthorization for Expired {
        fn perform(
            &self,
            _effect: &mut dyn FnMut() -> std::io::Result<usize>,
        ) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::PermissionDenied.into())
        }
    }
    let expired = adapter.handle().authorized(std::sync::Arc::new(Expired));
    assert!(matches!(
        adapter
            .process_control()
            .terminate(std::sync::Arc::new(Expired))
            .await,
        Err(qgenie_remote_adapters::CodexError::NotSent)
    ));
    assert_eq!(
        adapter.native_process_id(),
        Some(pid),
        "expired remote termination killed the native process"
    );
    assert!(matches!(
        expired
            .prompt(&thread, "expired request must not reach native")
            .await,
        Err(qgenie_remote_adapters::CodexError::Unknown)
    ));
    assert!(adapter.closed().await.is_err());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(adapter.native_process_id(), Some(pid));
    // SAFETY: signal 0 only probes this in-memory identity of the owned child.
    assert_eq!(
        unsafe { libc::kill(pid as i32, 0) },
        0,
        "authorization denial/native IPC failure killed the owned process"
    );
    assert!(matches!(
        stale_handle.start_thread(&root).await,
        Err(qgenie_remote_adapters::CodexError::NotSent)
    ));
    assert!(
        std::fs::read_dir(root.join("native-home"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .path()
                .extension()
                .is_some_and(|extension| extension == "sock")),
        "closing IPC unlinked the live owner's endpoint"
    );
    adapter.reconnect().await.unwrap();
    assert_eq!(
        adapter.native_process_id(),
        Some(pid),
        "native IPC reconnect launched another process"
    );
    assert_eq!(adapter.resume_thread(&thread).await.unwrap(), thread);
    assert!(
        matches!(
            stale_handle.start_thread(&root).await,
            Err(qgenie_remote_adapters::CodexError::NotSent)
        ),
        "reconnect rebound an old mutation handle"
    );
    adapter.disconnect().await.unwrap();
    assert_eq!(adapter.native_process_id(), Some(pid));
    adapter.shutdown().await.unwrap();
    let resumed = Codex::launch(config(&binary, &root, port)).await.unwrap();
    assert_eq!(resumed.resume_thread(&thread).await.unwrap(), thread);
    struct Current;
    impl qgenie_remote_adapters::NativeEffectAuthorization for Current {
        fn perform(
            &self,
            effect: &mut dyn FnMut() -> std::io::Result<usize>,
        ) -> std::io::Result<usize> {
            effect()
        }
    }
    let control = resumed.process_control();
    drop(control.clone());
    assert!(resumed.native_process_id().is_some());
    control
        .terminate(std::sync::Arc::new(Current))
        .await
        .unwrap();
    timeout(Duration::from_secs(3), async {
        while resumed.native_process_id().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // The driver observes actual process exit; explicit local owner cleanup reaps it.
    let mut resumed = resumed;
    let _ = resumed.closed().await;
    resumed.shutdown().await.unwrap();
    model.abort();
    let _ = model.await;
}
