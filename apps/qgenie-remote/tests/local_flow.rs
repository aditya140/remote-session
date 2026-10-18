use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_client::Client;
use qgenie_remote_proto::*;
use qgenie_remote_transport::ipc::{connect, read_frame, write_frame};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::time::{sleep, timeout};

struct Daemon {
    child: Child,
}
impl Daemon {
    fn start(path: &Path) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_qgenie-remote"))
            .args([
                "--state-dir",
                path.to_str().unwrap(),
                "--root",
                path.to_str().unwrap(),
                "sessiond",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        Self { child }
    }
    async fn ready(&mut self, client: &Client) {
        timeout(Duration::from_secs(5), async {
            loop {
                if client.status().await.is_ok() {
                    break;
                }
                assert!(
                    self.child.try_wait().unwrap().is_none(),
                    "daemon exited during startup"
                );
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("daemon startup timed out");
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn stop(path: &Path, terminate: bool) -> Result<(), RemoteError> {
    let mut stream = connect(&path.join("admin.sock")).await.unwrap();
    let bytes = serde_json::to_vec(&ipc::AdminRequest {
        ipc_major: IPC_MAJOR,
        command: ipc::AdminCommand::Shutdown {
            terminate_sessions: terminate,
        },
    })
    .unwrap();
    write_frame(&mut stream, &bytes).await.unwrap();
    decode_json::<ipc::AdminResponse>(&read_frame(&mut stream).await.unwrap())
        .unwrap()
        .result
}
fn shell(path: &Path, script: &str) -> StartSpec {
    StartSpec {
        key: IdempotencyKey::new(),
        command: vec!["/bin/sh".into(), "-c".into(), script.into()],
        cwd: path.display().to_string(),
        rows: 24,
        cols: 80,
    }
}
async fn output_contains(client: &Client, id: SessionId, needle: &str) -> String {
    timeout(Duration::from_secs(5), async {
        loop {
            let replay = client.replay(id, Counter(0)).await.unwrap();
            let mut output = Vec::new();
            for event in replay.events {
                if let EventBody::PtyOutput(data) = event.body {
                    output.extend(STANDARD.decode(data).unwrap());
                }
            }
            let output = String::from_utf8_lossy(&output).into_owned();
            if output.contains(needle) {
                return output;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("expected PTY output did not arrive")
}

#[tokio::test]
async fn actual_native_bridge_registers_without_adopting_local_sessions_and_fences_access() {
    use qgenie_remote_host::NativeClient;
    use qgenie_remote_proto::wire::{Envelope, Message};
    let temp = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let client = Client::local(&path);
    let mut daemon = Daemon::start(&path);
    daemon.ready(&client).await;
    let spec = shell(&path, "printf 'LOCAL_STILL_RUNNING'; read value");
    let local_key = spec.key;
    let session = client.start(spec).await.unwrap();
    output_contains(&client, session.id, "LOCAL_STILL_RUNNING").await;
    let scope = ipc::RemoteScope {
        origin: "https://gateway.example.invalid".into(),
        owner: UserId::new(),
        machine: MachineId::new(),
    };
    let native = NativeClient::new(path.clone(), scope.clone()).unwrap();
    let first = ClientId::new();
    assert!(matches!(
        native.challenge(first, Counter(1)).await,
        Err(qgenie_remote_host::Error::Rejected(RemoteError {
            code: ErrorCode::Forbidden,
            ..
        }))
    ));
    native.register_binding().await.unwrap();
    native.register_binding().await.unwrap();
    assert_eq!(client.status().await.unwrap().sessions, 1);
    assert_eq!(client.lookup_start(local_key).await.unwrap().id, session.id);
    let mut impostor = scope.clone();
    impostor.owner = UserId::new();
    assert!(matches!(
        NativeClient::new(path.clone(), impostor)
            .unwrap()
            .register_binding()
            .await,
        Err(qgenie_remote_host::Error::Rejected(RemoteError {
            code: ErrorCode::Forbidden,
            ..
        }))
    ));
    let challenge = native.challenge(first, Counter(1)).await.unwrap();
    let access = native.activate(&challenge, 10_000, false).await.unwrap();
    assert_eq!(
        native.recording_status(&access).await.unwrap().chunks,
        Counter(0)
    );
    assert!(
        native.recording_fetch(&access).await.unwrap().is_none(),
        "local preview output must not enter an enrolled recording scope"
    );
    assert!(matches!(
        native
            .controller_state(&access, ClientId::new(), session.id)
            .await,
        Err(qgenie_remote_host::Error::Rejected(RemoteError {
            code: ErrorCode::Forbidden,
            ..
        }))
    ));
    assert!(matches!(
        native
            .activate_controller(
                &access,
                ClientId::new(),
                session.id,
                qgenie_remote_host::NativeControlProposal {
                    generation: Counter(1),
                    remaining_lease_ms: 15_000,
                    takeover: false,
                    renewal: false
                }
            )
            .await,
        Err(qgenie_remote_host::Error::Rejected(RemoteError {
            code: ErrorCode::Forbidden,
            ..
        }))
    ));
    let mut list = Envelope::new(Message::SessionList);
    list.machine = Some(scope.machine);
    let response = native
        .read(&access, scope.owner, ClientId::new(), list.clone())
        .await
        .unwrap();
    assert!(matches!(response.message, Message::Sessions {sessions} if sessions.is_empty()));
    let mut lookup = Envelope::new(Message::StartLookup { key: local_key });
    lookup.machine = Some(scope.machine);
    assert!(matches!(
        native
            .read(&access, scope.owner, ClientId::new(), lookup)
            .await,
        Err(qgenie_remote_host::Error::Rejected(RemoteError {
            outcome: OperationOutcome::Unknown,
            ..
        }))
    ));
    assert!(matches!(
        native
            .read(&access, UserId::new(), ClientId::new(), list.clone())
            .await,
        Err(qgenie_remote_host::Error::Denied)
    ));
    let second = ClientId::new();
    let challenge = native.challenge(second, Counter(2)).await.unwrap();
    let replacement = native.activate(&challenge, 10_000, false).await.unwrap();
    assert!(matches!(
        native
            .read(&access, scope.owner, ClientId::new(), list.clone())
            .await,
        Err(qgenie_remote_host::Error::Rejected(RemoteError {
            code: ErrorCode::StaleGeneration,
            ..
        }))
    ));
    let challenge = native.challenge(second, Counter(2)).await.unwrap();
    let short = native.activate(&challenge, 150, true).await.unwrap();
    sleep(Duration::from_millis(180)).await;
    assert!(!short.is_fresh());
    assert!(native.recording_status(&short).await.is_err());
    assert!(native.recording_fetch(&short).await.is_err());
    assert!(matches!(
        native
            .read(&short, scope.owner, ClientId::new(), list.clone())
            .await,
        Err(qgenie_remote_host::Error::Rejected(RemoteError {
            code: ErrorCode::AccessStale,
            ..
        }))
    ));
    assert!(
        native
            .read(&replacement, scope.owner, ClientId::new(), list.clone())
            .await
            .is_ok()
    );
    // Binding/admin/native access never terminates or adopts the local child.
    assert_eq!(client.lookup_start(local_key).await.unwrap().id, session.id);
    assert!(stop(&path, false).await.is_err());
    stop(&path, true).await.unwrap();
    timeout(Duration::from_secs(3), async {
        while daemon.child.try_wait().unwrap().is_none() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mut restarted = Daemon::start(&path);
    restarted.ready(&client).await;
    native.register_binding().await.unwrap();
    assert!(replacement.is_fresh());
    assert!(matches!(
        native
            .read(&replacement, scope.owner, ClientId::new(), list.clone())
            .await,
        Err(qgenie_remote_host::Error::Rejected(RemoteError {
            code: ErrorCode::AccessStale,
            ..
        }))
    ));
    assert!(matches!(
        native.challenge(ClientId::new(), Counter(2)).await,
        Err(qgenie_remote_host::Error::Rejected(RemoteError {
            code: ErrorCode::StaleGeneration,
            ..
        }))
    ));
    let challenge = native.challenge(ClientId::new(), Counter(3)).await.unwrap();
    let new_access = native.activate(&challenge, 10_000, false).await.unwrap();
    assert!(
        native
            .read(&new_access, scope.owner, ClientId::new(), list)
            .await
            .is_ok()
    );
    assert_eq!(client.lookup_start(local_key).await.unwrap().id, session.id);
    stop(&path, false).await.unwrap();
}

#[tokio::test]
async fn actual_ipc_assembles_a_large_terminal_checkpoint() {
    let temp = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let client = Client::local(&path);
    let mut daemon = Daemon::start(&path);
    daemon.ready(&client).await;
    let mut spec = shell(&path, "printf 'checkpoint marker'; read value");
    spec.rows = 128;
    spec.cols = 512;
    let info = client.start(spec).await.unwrap();
    output_contains(&client, info.id, "checkpoint marker").await;
    let attachment = client.attach(info.id).await.unwrap();
    let data = STANDARD.decode(attachment.snapshot.data).unwrap();
    assert!(
        data.len() > 256 * 1024,
        "the SDK must assemble more than one actual IPC frame"
    );
    let parser = vt100::Parser::from_checkpoint(&data).unwrap();
    assert!(parser.screen().contents().contains("checkpoint marker"));
    assert_eq!(parser.screen().size(), (128, 512));
    stop(&path, true).await.unwrap();
    timeout(Duration::from_secs(5), async {
        while daemon.child.try_wait().unwrap().is_none() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn actual_process_preserves_sessions_deduplicates_starts_and_fences_takeover() {
    let temp = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let a = Client::local(&path);
    let b = Client::local(&path);
    let mut daemon = Daemon::start(&path);
    daemon.ready(&a).await;
    let spec = shell(
        &path,
        "printf 'READY\\n'; while IFS= read -r line; do printf 'OUT:%s\\n' \"$line\"; done",
    );
    let (first, duplicate) = tokio::join!(a.start(spec.clone()), b.start(spec.clone()));
    let session = first.unwrap();
    assert_eq!(session.id, duplicate.unwrap().id);
    assert_eq!(a.sessions().await.unwrap().len(), 1);
    output_contains(&a, session.id, "READY").await;
    let mut changed = spec.clone();
    changed.cols = 81;
    assert!(matches!(
        a.start(changed).await.unwrap_err().code,
        ErrorCode::IdempotencyConflict
    ));
    let mut old = a.take_control(session.id, false).await.unwrap();
    assert!(matches!(
        b.take_control(session.id, false).await.err().unwrap().code,
        ErrorCode::ControlHeld
    ));
    let mut new = b.take_control(session.id, true).await.unwrap();
    assert!(matches!(
        old.input(b"STALE\n").await.unwrap_err().code,
        ErrorCode::StaleGeneration
    ));
    new.input(b"hello\n").await.unwrap();
    let output = output_contains(&b, session.id, "OUT:hello").await;
    assert!(!output.contains("STALE"));
    new.resize(30, 100).await.unwrap();
    let attached = b.attach(session.id).await.unwrap();
    assert_eq!((attached.snapshot.rows, attached.snapshot.cols), (30, 100));
    assert!(
        stop(&path, false).await.is_err(),
        "shutdown must refuse active sessions"
    );
    drop(old);
    drop(a);
    let observer = Client::local(&path);
    assert!(matches!(
        observer.attach(session.id).await.unwrap().session.state,
        SessionState::Running
    ));
    assert!(observer.terminate(session.id, false).await.is_err());
    observer.terminate(session.id, true).await.unwrap();
    timeout(Duration::from_secs(3), async {
        loop {
            if matches!(
                observer.sessions().await.unwrap()[0].state,
                SessionState::Exited
            ) {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(stop(&path, false).await.is_ok());
    timeout(Duration::from_secs(3), async {
        while daemon.child.try_wait().unwrap().is_none() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mut restarted = Daemon::start(&path);
    restarted.ready(&observer).await;
    let previous = observer.start(spec).await.unwrap();
    assert_eq!(previous.id, session.id);
    assert!(matches!(previous.state, SessionState::Exited));
    stop(&path, false).await.unwrap();
}

#[tokio::test]
async fn no_echo_input_is_absent_from_replay_and_admin_schema_cannot_dispatch() {
    let temp = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let client = Client::local(&path);
    let mut daemon = Daemon::start(&path);
    daemon.ready(&client).await;
    let mut stream = connect(&path.join("dispatch.sock")).await.unwrap();
    write_frame(
        &mut stream,
        br#"{"ipc_major":1,"command":{"type":"shutdown","terminate_sessions":true}}"#,
    )
    .await
    .unwrap();
    assert!(read_frame(&mut stream).await.is_err());
    assert!(client.status().await.is_ok());
    let session = client
        .start(shell(
            &path,
            "stty -echo; printf 'PASSWORD_READY\\n'; IFS= read -r password; printf 'DONE\\n'",
        ))
        .await
        .unwrap();
    output_contains(&client, session.id, "PASSWORD_READY").await;
    let mut controller = client.take_control(session.id, false).await.unwrap();
    controller
        .input(b"test-secret-never-recorded\n")
        .await
        .unwrap();
    let output = output_contains(&client, session.id, "DONE").await;
    assert!(!output.contains("test-secret-never-recorded"));
    stop(&path, true).await.unwrap();
}
