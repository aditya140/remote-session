//! Real CLI invocations prove that profile management works before authentication.
use std::{os::unix::fs::PermissionsExt, process::Command};
fn run(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_qgenie-remote"))
        .arg("--state-dir")
        .arg(root)
        .arg("remote")
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn profiles_persist_private_shell_choices_and_reject_scope_changes_and_credential_imports() {
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let machine = qgenie_remote_proto::MachineId::new().to_string();
    let saved = run(
        root.path(),
        &[
            "--gateway",
            "https://example.invalid",
            "profile",
            "save",
            "dev-server",
            &machine,
            "--cwd",
            "/projects",
            "--",
            "/bin/bash",
            "-l",
        ],
    );
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    let path = root.path().join("launch-profiles/dev-server.json");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let original = std::fs::read(&path).unwrap();
    let shown = run(root.path(), &["profile", "show", "dev-server"]);
    assert!(shown.status.success());
    let profile: serde_json::Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(profile["machine"], machine);
    assert_eq!(
        profile["kind"]["argv"],
        serde_json::json!(["/bin/bash", "-l"])
    );
    assert!(!String::from_utf8_lossy(&original).contains("token"));
    let mismatch = run(
        root.path(),
        &[
            "--gateway",
            "https://other.invalid",
            "profile",
            "run",
            "dev-server",
        ],
    );
    assert!(!mismatch.status.success());
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains("different gateway"));
    let mut invalid = profile;
    invalid["access_token"] = serde_json::json!("synthetic-do-not-store");
    std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
    assert!(
        !run(root.path(), &["profile", "show", "dev-server"])
            .status
            .success()
    );
    std::fs::write(&path, &original).unwrap();
    assert!(
        !run(
            root.path(),
            &[
                "--gateway",
                "https://example.invalid",
                "profile",
                "save",
                "dev-server",
                &machine,
                "--cwd",
                "/projects"
            ]
        )
        .status
        .success(),
        "replacement must be explicit"
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(
        !run(root.path(), &["profile", "show", "../outside"])
            .status
            .success()
    );
    let listed = run(root.path(), &["profile", "list"]);
    assert_eq!(
        serde_json::from_slice::<Vec<String>>(&listed.stdout).unwrap(),
        vec!["dev-server"]
    );
    assert!(
        run(root.path(), &["profile", "remove", "dev-server"])
            .status
            .success()
    );
    assert!(!path.exists());
    assert!(
        !root.path().join("start-identities.sqlite").exists(),
        "local profile operations must not submit work"
    );
}

#[test]
fn profiles_validate_agent_tasks_and_bound_imports() {
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let machine = qgenie_remote_proto::MachineId::new().to_string();
    let saved = run(
        root.path(),
        &[
            "--gateway",
            "https://example.invalid",
            "profile",
            "save",
            "read-only-reviewer",
            &machine,
            "--cwd",
            "/projects",
            "--agent",
            "codex",
            "--task",
            "Review only by convention; this grants no permissions.",
        ],
    );
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    let path = root.path().join("launch-profiles/read-only-reviewer.json");
    let mut profile: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(profile["kind"]["agent"], "codex");
    profile["kind"]["refresh_token"] = serde_json::json!("synthetic");
    std::fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();
    assert!(
        !run(root.path(), &["profile", "show", "read-only-reviewer"])
            .status
            .success()
    );
    std::fs::write(&path, vec![b' '; 65537]).unwrap();
    assert!(
        !run(root.path(), &["profile", "run", "read-only-reviewer"])
            .status
            .success()
    );
    assert!(
        run(root.path(), &["profile", "remove", "read-only-reviewer"])
            .status
            .success(),
        "invalid profiles remain removable"
    );
}
