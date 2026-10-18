//! Actual native owner SIGKILL qualification; fixture cleanup uses an owned
//! helper Child and a kernel pidfd, never a PID-file signaling authority.
use super::*;
use std::os::{
    fd::{AsRawFd, FromRawFd, OwnedFd},
    unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
};

fn configuration(root: &std::path::Path) -> Config {
    let mut config = Config::new(root.join("state"), vec![root.into()]);
    config.codex = Some(CodexProfile::new(
        root.join("native-wrapper"),
        "https://synthetic.invalid/v1".into(),
        "fixture".into(),
        "synthetic-only".into(),
    ));
    config
}
fn launch(root: &std::path::Path, key: IdempotencyKey) -> Launch {
    Launch {
        key,
        kind: LaunchKind::Agent {
            agent: "codex".into(),
            prompt: None,
            native_resume_id: None,
        },
        cwd: root.to_str().unwrap().into(),
        rows: 24,
        cols: 80,
    }
}
#[test]
#[ignore = "isolated managed owner crash helper; invoked only by its parent proof"]
fn managed_crash_fixture_child() {
    let root = PathBuf::from(std::env::var_os("QGR_MANAGED_CRASH_ROOT").unwrap());
    let key = std::env::var("QGR_MANAGED_CRASH_KEY")
        .unwrap()
        .parse()
        .unwrap();
    let runtime = Runtime::open(configuration(&root)).unwrap();
    let runtime = if let Ok(point) = std::env::var("QGR_MANAGED_FAULT_POINT") {
        drop(runtime);
        let mut config = configuration(&root);
        config.start_fault = Some(match point.as_str() {
            "before_journal" => StartFault::BeforeJournal,
            "accepted" => StartFault::Accepted,
            "launching" => StartFault::Launching,
            "native_launched" => StartFault::NativeLaunched,
            "running_committed" => StartFault::RunningCommitted,
            "before_ack" => StartFault::BeforeAck,
            _ => panic!("invalid managed fault fixture point"),
        });
        Runtime::open_after_previous_owner(config).unwrap()
    } else {
        runtime
    };
    let info = runtime.start_managed(launch(&root, key)).unwrap();
    assert!(info.agent.as_ref().unwrap().native_thread.is_some());
    std::fs::write(
        root.join("ready.pending"),
        serde_json::to_vec(&info).unwrap(),
    )
    .unwrap();
    std::fs::rename(root.join("ready.pending"), root.join("ready.json")).unwrap();
    loop {
        thread::park();
    }
}
struct Helper(std::process::Child);
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Native(OwnedFd);
impl Drop for Native {
    fn drop(&mut self) {
        // SAFETY: this pidfd was opened and parent-validated while our exact
        // helper owned the native child. It cannot target a reused PID.
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.0.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            );
        }
    }
}
fn observe_native(helper: &Helper, root: &std::path::Path) -> Native {
    let pid: i32 = std::fs::read_to_string(root.join("native.pid"))
        .unwrap()
        .parse()
        .unwrap();
    // SAFETY: successful pidfd_open transfers an owned, reuse-safe descriptor.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    assert!(fd >= 0, "owned native pidfd unavailable");
    let observed = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    assert!(status.lines().any(|line| {
        line.strip_prefix("PPid:")
            .is_some_and(|value| value.trim() == helper.0.id().to_string())
    }));
    Native(observed)
}
fn observed_exit(native: &Native) {
    let mut ended = libc::pollfd {
        fd: native.0.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: a single initialized pollfd remains valid for the bounded wait.
    assert_eq!(
        unsafe { libc::poll(&mut ended, 1, 2000) },
        1,
        "native survived owner crash"
    );
    assert_ne!(ended.revents & libc::POLLIN, 0);
}
#[test]
#[ignore = "requires pinned QGR_CODEX_BINARY; six actual managed start crash boundaries"]
fn actual_managed_start_crash_boundaries_preserve_keys_without_relaunch() {
    let binary = PathBuf::from(std::env::var_os("QGR_CODEX_BINARY").expect("set pinned binary"));
    for (point, accepted, spawned) in [
        ("before_journal", false, false),
        ("accepted", true, false),
        ("launching", true, false),
        ("native_launched", true, true),
        ("running_committed", true, true),
        ("before_ack", true, true),
    ] {
        let directory = super::super::tests::directory();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let quote = |path: &std::path::Path| {
            format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
        };
        let wrapper = root.join("native-wrapper");
        std::fs::write(&wrapper,format!("#!/bin/sh\nif [ \"$1\" != --version ]; then printf '%s' \"$$\" > {}; printf x >> {}; fi\nexec {} \"$@\"\n",quote(&root.join("native.pid")),quote(&root.join("spawns")),quote(&binary))).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let key = IdempotencyKey::new();
        let mut helper = Helper(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "runtime::managed::crash_tests::managed_crash_fixture_child",
                ])
                .env("QGR_MANAGED_CRASH_ROOT", &root)
                .env("QGR_MANAGED_CRASH_KEY", key.to_string())
                .env("QGR_MANAGED_FAULT_POINT", point)
                .env("QGR_MANAGED_FAULT_BARRIER", &root)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.join("fault.ready").exists() {
            assert!(
                helper.0.try_wait().unwrap().is_none(),
                "managed helper exited before {point}"
            );
            assert!(
                Instant::now() < deadline,
                "managed fault point {point} not reached"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let native = spawned.then(|| observe_native(&helper, &root));
        let sockets = std::fs::read_dir(root.join("state/agents"))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .flat_map(|home| std::fs::read_dir(home.path()).into_iter().flatten())
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let target = std::fs::read_link(entry.path()).ok()?;
                let metadata = std::fs::symlink_metadata(&target).ok()?;
                metadata
                    .file_type()
                    .is_socket()
                    .then_some((target, metadata.ino()))
            })
            .collect::<Vec<_>>();
        std::fs::write(root.join("fault.release"), b"release").unwrap();
        assert_eq!(
            helper.0.wait().unwrap().code(),
            Some(86),
            "wrong managed crash at {point}"
        );
        if let Some(native) = &native {
            observed_exit(native);
        }
        let recovered = Runtime::open_after_previous_owner(configuration(&root)).unwrap();
        if accepted {
            let sessions = recovered.list().unwrap();
            assert_eq!(sessions.len(), 1, "{point}");
            let result = recovered.start_managed(launch(&root, key)).unwrap();
            assert_eq!(result.id, sessions[0].id);
            assert!(
                matches!(
                    result.state,
                    SessionState::Lost | SessionState::OutcomeUnknown
                ),
                "{point}: {:?}",
                result.state
            );
        } else {
            assert!(recovered.list().unwrap().is_empty());
            let reply = super::super::tests::call(
                &recovered,
                ClientId::new(),
                Operation::LookupStart { key },
            );
            assert!(matches!(
                reply,
                Reply::Error(RemoteError {
                    outcome: OperationOutcome::Unknown,
                    ..
                })
            ));
        }
        assert_eq!(
            std::fs::read(root.join("spawns")).unwrap_or_default(),
            if spawned { b"x".as_slice() } else { b"" },
            "duplicate native launch at {point}"
        );
        recovered.shutdown(false).unwrap();
        for (path, inode) in sockets {
            if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.ino() == inode) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}
#[test]
#[ignore = "requires actual pinned QGR_CODEX_BINARY; Linux native owner SIGKILL proof"]
fn actual_managed_owner_crash_fences_history_without_relaunch() {
    let binary = PathBuf::from(std::env::var_os("QGR_CODEX_BINARY").expect("set pinned binary"));
    let temp = super::super::tests::directory();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let quote =
        |path: &std::path::Path| format!("'{}'", path.display().to_string().replace('\'', "'\\''"));
    let wrapper = root.join("native-wrapper");
    std::fs::write(&wrapper,format!("#!/bin/sh\nif [ \"$1\" != --version ]; then printf '%s' \"$$\" > {}; printf x >> {}; fi\nexec {} \"$@\"\n",quote(&root.join("native.pid")),quote(&root.join("spawns")),quote(&binary))).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let key = IdempotencyKey::new();
    let mut helper = Helper(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "runtime::managed::crash_tests::managed_crash_fixture_child",
                "--nocapture",
            ])
            .env("QGR_MANAGED_CRASH_ROOT", &root)
            .env("QGR_MANAGED_CRASH_KEY", key.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.join("ready.json").exists() {
        assert!(helper.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let info: SessionInfo =
        serde_json::from_slice(&std::fs::read(root.join("ready.json")).unwrap()).unwrap();
    let pid: i32 = std::fs::read_to_string(root.join("native.pid"))
        .unwrap()
        .parse()
        .unwrap();
    // SAFETY: pidfd_open returns an owned FD or -1, tested before construction.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    assert!(
        fd >= 0,
        "pidfd observation unavailable: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: successful pidfd_open transferred this descriptor to us.
    let observed = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    assert!(status.lines().any(|line| {
        line.strip_prefix("PPid:")
            .is_some_and(|value| value.trim() == helper.0.id().to_string())
    }));
    let native = Native(observed);
    let home = root.join("state/agents").join(info.id.to_string());
    let sockets = std::fs::read_dir(&home)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()? != "sock" {
                return None;
            }
            let target = std::fs::read_link(path).ok()?;
            let metadata = std::fs::symlink_metadata(&target).ok()?;
            Some((target, metadata.ino()))
        })
        .collect::<Vec<_>>();
    helper.0.kill().unwrap();
    helper.0.wait().unwrap();
    let mut ended = libc::pollfd {
        fd: native.0.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one initialized pollfd remains valid for this bounded observation.
    assert_eq!(
        unsafe { libc::poll(&mut ended, 1, 2000) },
        1,
        "native process survived its owner SIGKILL"
    );
    assert_ne!(ended.revents & libc::POLLIN, 0);
    let recovered = Runtime::open_after_previous_owner(configuration(&root)).unwrap();
    let replay = recovered.start_managed(launch(&root, key)).unwrap();
    assert_eq!(replay.id, info.id);
    assert!(matches!(
        replay.state,
        SessionState::Lost | SessionState::OutcomeUnknown
    ));
    assert_eq!(
        std::fs::read(root.join("spawns")).unwrap(),
        b"x",
        "same key restarted an unconfirmed owner"
    );
    recovered.shutdown(false).unwrap();
    for (path, inode) in sockets {
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.ino() == inode) {
            let _ = std::fs::remove_file(path);
        }
    }
}
