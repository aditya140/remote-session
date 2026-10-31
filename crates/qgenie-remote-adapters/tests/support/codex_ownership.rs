//! Verify actual owned tool descendants, rather than inferring cleanup from the app-server PID.
use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Current;
impl qgenie_remote_adapters::NativeEffectAuthorization for Current {
    fn perform(
        &self,
        effect: &mut dyn FnMut() -> std::io::Result<usize>,
    ) -> std::io::Result<usize> {
        effect()
    }
}

#[tokio::test]
#[ignore = "requires actual pinned QGR_CODEX_BINARY; launcher ownership qualification"]
async fn native_socket_must_belong_to_the_launched_process() {
    use std::os::{fd::FromRawFd, unix::fs::PermissionsExt};
    let binary = PathBuf::from(std::env::var_os("QGR_CODEX_BINARY").expect("set QGR_CODEX_BINARY"));
    let temp = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let quote = |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "'\\''"));
    let wrapper = root.join("launcher");
    let pid_file = root.join("forked.pid");
    std::fs::write(&wrapper, format!(
        "#!/bin/sh\nif [ \"$1\" = --version ]; then exec {} \"$@\"; fi\n{} \"$@\" &\nprintf '%s' \"$!\" > {}\nwait\n",
        quote(&binary), quote(&binary), quote(&pid_file),
    )).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = Codex::launch(config(&wrapper, &root, 9)).await;
    let rejected = matches!(
        result,
        Err(qgenie_remote_adapters::CodexError::Configuration)
    );
    let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
    // Retain an exact OS process reference for cleanup even when testing the broken code.
    // SAFETY: pidfd_open accepts a PID and zero flags. It returns an owned descriptor.
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    let descriptor = i32::try_from(descriptor)
        .ok()
        .filter(|descriptor| *descriptor >= 0)
        .map(|descriptor| {
            // SAFETY: the successful syscall returned a fresh descriptor owned here.
            unsafe { std::os::fd::OwnedFd::from_raw_fd(descriptor) }
        });
    if let Ok(adapter) = result {
        adapter.shutdown().await.unwrap();
    }
    let still_running = || {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit_once(')')
                    .map(|(_, rest)| rest.trim().starts_with(['Z', 'X']))
            })
            .is_some_and(|dead| !dead)
    };
    let stopped = timeout(Duration::from_secs(2), async {
        while still_running() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok();
    if !stopped && let Some(descriptor) = descriptor {
        use std::os::fd::AsRawFd;
        // This fixture owns the launcher and its child. Never signal a reused PID.
        // SAFETY: the descriptor is live, SIGKILL is valid, and null selects no siginfo.
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                descriptor.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
    }
    assert!(
        rejected,
        "a forked native socket was accepted as the owned child"
    );
    assert!(stopped, "rejected launcher left its native child running");

    // A wrapper that preserves the child identity remains supported.
    std::fs::write(
        &wrapper,
        format!("#!/bin/sh\nexec {} \"$@\"\n", quote(&binary)),
    )
    .unwrap();
    Codex::launch(config(&wrapper, &root, 9))
        .await
        .unwrap()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn failed_native_startup_is_reaped_before_returning_to_its_executor_owner() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    for stalled_probe in [false, true] {
        let pid_file = root.join("startup.pid");
        let wrapper = root.join("broken-native");
        let quote =
            |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "'\\''"));
        let probe = if stalled_probe {
            ""
        } else {
            "if [ \"$1\" = --version ]; then printf 'codex-cli 0.159.2\\n'; exit 0; fi\n"
        };
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\n{probe}printf '%s' \"$$\" > {}\nexec sleep 60\n",
                quote(&pid_file)
            ),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = Codex::launch(config(&wrapper, &root, 9)).await;
        assert!(result.is_err());
        let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
        // A zombie is still present under kill(pid, 0); this checks actual reap,
        // before the current-thread runtime gets another opportunity to run.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        std::fs::remove_file(pid_file).unwrap();
    }
}

#[tokio::test]
#[ignore = "requires actual pinned QGR_CODEX_BINARY; native original-grant spawn qualification"]
async fn actual_native_spawn_rechecks_authority_after_version_probe() {
    use std::os::unix::fs::PermissionsExt;
    let binary = PathBuf::from(std::env::var_os("QGR_CODEX_BINARY").expect("set QGR_CODEX_BINARY"));
    let temp = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let wrapper = root.join("native-wrapper");
    let quote = |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "'\\''"));
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ \"$1\" != --version ]; then printf spawned > {}; fi\nexec {} \"$@\"\n",
            quote(&root.join("native_spawned")),
            quote(&binary)
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    struct Expires(AtomicUsize);
    impl qgenie_remote_adapters::NativeEffectAuthorization for Expires {
        fn perform(
            &self,
            effect: &mut dyn FnMut() -> std::io::Result<usize>,
        ) -> std::io::Result<usize> {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                effect()
            } else {
                Err(std::io::ErrorKind::PermissionDenied.into())
            }
        }
    }
    let authorization = Arc::new(Expires(AtomicUsize::new(0)));
    let mut configuration = config(&wrapper, &root, 9);
    configuration.cwd = root.clone();
    let directory = std::fs::File::open(&root).unwrap();
    let result = Codex::launch_at_authorized(configuration, directory, authorization.clone()).await;
    assert!(matches!(
        result,
        Err(qgenie_remote_adapters::CodexError::NotSent)
    ));
    assert_eq!(authorization.0.load(Ordering::SeqCst), 2);
    assert!(
        !root.join("native_spawned").exists(),
        "native startup executed after its original grant expired"
    );
}

#[tokio::test]
#[ignore = "requires actual pinned QGR_CODEX_BINARY; native process-tree qualification"]
async fn actual_owned_codex_termination_does_not_leave_an_approved_tool_descendant_running() {
    owned_tool_cleanup(true).await;
}

#[tokio::test]
#[ignore = "requires actual pinned QGR_CODEX_BINARY; sandboxed native process-tree qualification"]
async fn actual_owned_codex_termination_also_cleans_sandboxed_tools() {
    owned_tool_cleanup(false).await;
}

async fn owned_tool_cleanup(unsandboxed: bool) {
    struct ReleaseTool(PathBuf);
    impl Drop for ReleaseTool {
        fn drop(&mut self) {
            // Also release the fixture if an earlier assertion fails.
            let _ = std::fs::write(&self.0, b"release");
        }
    }
    let binary = PathBuf::from(std::env::var_os("QGR_CODEX_BINARY").expect("set QGR_CODEX_BINARY"));
    let temp = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let release = ReleaseTool(root.join("release_tool"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let model = tokio::spawn(async move {
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
                    .map(|v| v.trim().parse::<usize>().unwrap())
            })
            .unwrap();
        assert!(length < 2 * 1024 * 1024);
        let mut request = vec![0; length];
        stream.read_exact(&mut request).await.unwrap();
        let mut arguments = json!({"cmd":"(printf ready > tool_live; while [ ! -e release_tool ]; do sleep 0.01; done; printf escaped > escaped_descendant) & wait","max_output_chars":1000});
        if unsandboxed {
            // Exercise the approved tool's own session even on machines where
            // the ordinary sandbox happens to contain its descendants.
            arguments["sandbox_permissions"] = json!("require_escalated");
            arguments["justification"] = json!("Controlled process ownership regression");
        }
        let item = json!({"type":"function_call","id":"fc_tree","call_id":"call_tree","name":"exec_command","arguments":arguments.to_string()});
        let events = [
            json!({"type":"response.created","response":{"id":"tree"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":item}),
            json!({"type":"response.output_item.done","output_index":0,"item":item}),
            json!({"type":"response.completed","response":{"id":"tree","status":"completed","output":[item],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
        ];
        let mut output =
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
                .to_owned();
        for event in events {
            output.push_str(&format!(
                "event: {}\ndata: {}\n\n",
                event["type"].as_str().unwrap(),
                event
            ));
        }
        stream.write_all(output.as_bytes()).await.unwrap();
    });
    let mut adapter = Codex::launch(config(&binary, &root, port)).await.unwrap();
    let thread = adapter.start_thread(&root).await.unwrap();
    adapter
        .prompt(&thread, "Run the controlled owned process-tree fixture.")
        .await
        .unwrap();
    let approval = loop {
        if let NativeEvent::Approval(approval) = next(&mut adapter).await {
            break approval;
        }
    };
    adapter
        .answer(&approval, ApprovalDecision::AllowOnce)
        .await
        .unwrap();
    timeout(Duration::from_secs(10), async {
        while !root.join("tool_live").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    // A slow runner may pause longer than the old one-second marker timer.
    // The descendant must wait for our explicit post-termination release instead.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(!root.join("escaped_descendant").exists());
    if unsandboxed {
        // Process ownership must still work after native protocol closure.
        adapter.disconnect().await.unwrap();
    }
    adapter
        .process_control()
        .terminate(Arc::new(Current))
        .await
        .unwrap();
    let _ = adapter.closed().await;
    adapter.shutdown().await.unwrap();
    model.await.unwrap();
    std::fs::write(&release.0, b"release").unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        !root.join("escaped_descendant").exists(),
        "owned app-server termination left an approved tool descendant alive"
    );
}
