use super::*;
fn output_peer(reconnect: bool, chunks: &[&[u8]]) -> Peer {
    let mut peer = Peer::new(reconnect);
    peer.session.kind = LaunchKind::Pty {
        command: vec!["fixture".into()],
    };
    peer.output = Some(
        chunks
            .iter()
            .enumerate()
            .map(|(i, bytes)| SequencedEvent {
                seq: Counter(i as u64 + 1),
                at: Timestamp::from_unix_millis(1).unwrap(),
                event: AgentEvent::PtyOutput {
                    data: STANDARD.encode(bytes),
                },
            })
            .collect(),
    );
    peer
}
async fn serve(tls: &identity::Tls, peer: &Peer) -> identity::Server {
    tls.bind()
        .await
        .serve(
            Router::new()
                .route("/v1/connect/client", get(endpoint))
                .with_state(peer.clone()),
        )
        .await
}
#[tokio::test]
async fn fresh_output_ignores_old_text_and_resumes_partial_match_after_disconnect() {
    let tls = identity::Tls::new();
    let peer = output_peer(true, &[b"ready", b"re\x1b[3", b"1mady\x1b[0m"]);
    let server = serve(&tls, &peer).await;
    let result = cli(
        &server,
        &tls,
        &peer,
        &[
            "wait-output",
            "--after",
            "1",
            "--literal",
            "ready",
            "--timeout",
            "5",
        ],
    )
    .await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(result["outcome"], "matched");
    assert_eq!(result["observed_through"], "3");
    assert_eq!(*peer.after.lock().unwrap(), vec![Counter(0), Counter(2)]);
    assert_eq!(peer.leases.load(Ordering::SeqCst), 0);
    assert_eq!(peer.inputs.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn old_output_times_out_and_gaps_cannot_match_a_marker() {
    let tls = identity::Tls::new();
    for gap in [false, true] {
        let mut peer = output_peer(false, &[b"ready"]);
        if gap {
            peer.output.as_mut().unwrap().push(SequencedEvent {
                seq: Counter(2),
                at: Timestamp::from_unix_millis(1).unwrap(),
                event: AgentEvent::Gap {
                    from: Counter(2),
                    to: Counter(2),
                    reason: "fixture loss".into(),
                },
            });
        }
        let server = serve(&tls, &peer).await;
        let result = cli(
            &server,
            &tls,
            &peer,
            &[
                "wait-output",
                "--after",
                "1",
                "--literal",
                "ready",
                "--timeout",
                "1",
            ],
        )
        .await;
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(
            result["outcome"],
            if gap {
                "history_unavailable"
            } else {
                "timeout"
            }
        );
    }
}
fn failed_turn(peer: &mut Peer) {
    peer.agent
        .push(SequencedEvent {
            seq: Counter(peer.agent.seq.0 + 1),
            at: Timestamp::from_unix_millis(1).unwrap(),
            event: AgentEvent::TurnCompleted {
                turn: "failed".into(),
                input_tokens: Counter(0),
                output_tokens: Counter(0),
                outcome: Some(TurnOutcome::Failed),
            },
        })
        .unwrap();
}
fn command(
    server: &identity::Server,
    tls: &identity::Tls,
    token: &std::path::Path,
    root: &std::path::Path,
    peer: &Peer,
    mode: &str,
    timeout: &str,
) -> tokio::process::Command {
    let target =
        |condition: &str| format!("{}/{}/{}", peer.session.machine, peer.session.id, condition);
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_qgenie-remote"));
    command
        .arg("--state-dir")
        .arg(root)
        .args(["remote", "--gateway"])
        .arg(&server.origin)
        .arg("--ca-bundle")
        .arg(tls.ca.path())
        .arg("--token-file")
        .arg(token)
        .args([
            "wait-many",
            "--mode",
            mode,
            "--target",
            &target("turn:wanted"),
            "--target",
            &target("turn:failed"),
            "--target",
            &target("exit"),
            "--timeout",
            timeout,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    command
}
#[tokio::test]
async fn any_all_waits_keep_fast_completion_failure_and_timeout_separate() {
    let tls = identity::Tls::new();
    let mut peer = Peer::new(false);
    failed_turn(&mut peer);
    let server = serve(&tls, &peer).await;
    let token = identity::private(b"synthetic-multi-wait-token");
    let root = tempfile::tempdir().unwrap();
    for mode in ["any", "all"] {
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            command(&server, &tls, token.path(), root.path(), &peer, mode, "1").output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let value: Value = serde_json::from_slice(&result.stdout).unwrap();
        let targets = value["targets"].as_array().unwrap();
        assert_eq!(targets.len(), 3);
        if mode == "all" {
            assert_eq!(value["reason"], "timeout");
            assert_eq!(targets[0]["outcome"], "completed");
            assert_eq!(targets[1]["outcome"], "failed");
            assert_eq!(targets[2]["outcome"], "timeout");
        } else {
            assert_eq!(value["reason"], "any_resolved");
            assert!(targets.iter().any(|result|result["outcome"]=="completed"||result["outcome"]=="failed"));
            assert!(targets[2]["outcome"].is_null());
        }
        assert_eq!(targets[0]["target"]["condition"]["turn"], "wanted");
    }
    assert_eq!(peer.inputs.load(Ordering::SeqCst), 0);
    assert_eq!(peer.leases.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn cancelled_many_wait_retains_resolved_targets_and_does_not_control_sessions() {
    let tls = identity::Tls::new();
    let mut peer = Peer::new(false);
    failed_turn(&mut peer);
    let server = serve(&tls, &peer).await;
    let token = identity::private(b"synthetic-multi-wait-token");
    let root = tempfile::tempdir().unwrap();
    let child = command(&server, &tls, token.path(), root.path(), &peer, "all", "30")
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while peer.connections.load(Ordering::SeqCst) < 4 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::process::Command::new("kill")
            .arg("-TERM")
            .arg(child.id().unwrap().to_string())
            .status()
            .await
            .unwrap()
            .success()
    );
    let result = tokio::time::timeout(Duration::from_secs(3), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["reason"], "cancelled");
    assert_eq!(value["targets"][0]["outcome"], "completed");
    assert_eq!(value["targets"][1]["outcome"], "failed");
    assert_eq!(value["targets"][2]["outcome"], "cancelled");
    assert_eq!(peer.inputs.load(Ordering::SeqCst), 0);
    assert_eq!(peer.leases.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn duplicate_many_targets_fail_before_opening_an_observation_connection() {
    let tls = identity::Tls::new();
    let peer = Peer::new(false);
    let server = serve(&tls, &peer).await;
    let token = identity::private(b"synthetic-multi-wait-token");
    let root = tempfile::tempdir().unwrap();
    let mut cmd = command(&server, &tls, token.path(), root.path(), &peer, "all", "1");
    cmd.arg("--target").arg(format!(
        "{}/{}/turn:wanted",
        peer.session.machine, peer.session.id
    ));
    let result = cmd.output().await.unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("duplicate wait target"));
    assert_eq!(peer.connections.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_checkpoint_cannot_satisfy_a_fresh_output_wait() {
    let tls = identity::Tls::new();
    let mut peer = Peer::new(false);
    peer.session.kind = LaunchKind::Pty {
        command: vec!["fixture".into()],
    };
    let server = serve(&tls, &peer).await;
    let result = cli(
        &server,
        &tls,
        &peer,
        &[
            "wait-output",
            "--after",
            "0",
            "--literal",
            "readable result",
            "--timeout",
            "1",
        ],
    )
    .await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["outcome"], "history_unavailable");
}
