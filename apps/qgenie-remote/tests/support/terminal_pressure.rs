//! Missing composed cases: two real PTYs, a stalled observer, and protected tail.
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_client::{Client, GatewayClient, LiveHandle};
use qgenie_remote_proto::{rest::*, wire::*, *};
use std::{
    path::Path,
    time::{Duration, Instant},
};

async fn output(local: &Client, session: SessionId) -> (Vec<u8>, Option<Counter>, Option<Counter>) {
    let mut after = Counter(0);
    let mut bytes = Vec::new();
    let mut final_seq = None;
    let mut exit = None;
    loop {
        let page = local.replay(session, after).await.unwrap();
        assert!(
            !page.truncated,
            "fixture output must fit the retained replay"
        );
        for event in page.events {
            assert!(event.seq > after);
            after = event.seq;
            match event.body {
                EventBody::PtyOutput(data) => {
                    assert!(exit.is_none(), "output appeared after native completion");
                    bytes.extend(STANDARD.decode(data).unwrap());
                    if bytes
                        .windows(b"PRESSURE_FINAL".len())
                        .any(|v| v == b"PRESSURE_FINAL")
                    {
                        final_seq = Some(event.seq);
                    }
                }
                EventBody::Exited { .. } => exit = Some(event.seq),
                EventBody::Gap { .. } => panic!("unexpected fixture replay loss"),
                _ => {}
            }
        }
        if !page.more {
            break;
        }
    }
    (bytes, final_seq, exit)
}

pub async fn prove(
    sdk: &GatewayClient,
    token: &AccessToken,
    owner: UserId,
    machine: MachineId,
    root: &Path,
    handle: &LiveHandle,
    local: &Client,
) {
    let started_at = Instant::now();
    tokio::time::timeout(Duration::from_secs(35), async {
        let journal = tempfile::Builder::new().permissions(
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700)
        ).tempdir().unwrap();
        let launch = |script: &str| Launch { key: IdempotencyKey::new(), kind: LaunchKind::Pty {
            command: vec!["/bin/sh".into(), "-c".into(), script.into()],
        }, cwd: root.to_str().unwrap().into(), rows: 24, cols: 80 };
        let noisy = handle.start(journal.path(), owner, machine, launch(
            "stty -echo; printf 'PRESSURE_READY\\n'; read go; i=0; while [ $i -lt 80 ]; do printf '%04096d\\n' $i; i=$((i+1)); sleep 0.05; done; printf 'PRESSURE_FINAL\\n'"
        )).await.unwrap();
        let healthy = handle.start(journal.path(), owner, machine, launch(
            "stty -echo; printf 'HEALTHY_READY\\n'; while IFS= read -r value; do printf 'HEALTHY:%s\\n' \"$value\"; done"
        )).await.unwrap();
        assert_ne!(noisy.id, healthy.id, "distinct native processes need distinct lifetime IDs");
        let mut stalled = sdk.observe_session(token, machine, noisy.id).await.unwrap();
        stalled.next().await.unwrap().unwrap();
        let mut noisy_control = handle.take_control(machine, noisy.id, false).await.unwrap();
        let mut healthy_control = handle.take_control(machine, healthy.id, false).await.unwrap();
        noisy_control.input(b"go\n").await.unwrap();
        // Never read the first observer again. Its queue has eight entries.
        let deadline = Instant::now() + Duration::from_secs(6);
        while stalled.failure().is_none() {
            assert!(Instant::now() < deadline, "stalled consumer did not reach its bounded limit");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(stalled.failure().unwrap().code, ErrorCode::Overloaded);
        healthy_control.resize(29, 101).await.unwrap();
        healthy_control.input(b"progress-under-pressure\n").await.unwrap();
        loop {
            let (bytes, _, _) = output(local, healthy.id).await;
            if bytes.windows(b"HEALTHY:progress-under-pressure".len()).any(|v| v == b"HEALTHY:progress-under-pressure") { break; }
            assert!(Instant::now() < deadline, "unrelated native session stopped making progress");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let snapshot = local.attach(healthy.id).await.unwrap().snapshot;
        assert_eq!((snapshot.rows, snapshot.cols), (29, 101));
        let mut replacement = sdk.observe_session(token, machine, noisy.id).await.unwrap();
        replacement.next().await.unwrap().unwrap();
        stalled.close().await; // A late old close cannot close this new connection.
        let mut saw_exit = false;
        while !saw_exit {
            let update = replacement.next().await.unwrap().unwrap();
            saw_exit = update.update.events.iter().any(|event| matches!(event.event, AgentEvent::Exited { .. }));
        }
        replacement.close().await;
        let (bytes, final_seq, exit_seq) = output(local, noisy.id).await;
        assert!(bytes.windows(b"PRESSURE_FINAL".len()).any(|v| v == b"PRESSURE_FINAL"));
        assert!(final_seq.unwrap() < exit_seq.unwrap(), "native final output must precede completion");
        assert!(local.sessions().await.unwrap().iter().any(|s| s.id == healthy.id && matches!(s.state, SessionState::Running)));
        // Read the authenticated history path, including every page. Its records
        // must contain the final marker in this no-loss fixture.
        loop {
            let streams = sdk.history_streams(token, noisy.id, HistoryStreamsQuery { after: None, limit: 4 }).await.unwrap();
            assert!(streams.next.is_none());
            let mut recorded = Vec::new();
            let mut explicit_gap = false;
            for stream in streams.streams {
                let mut after = None;
                for page_number in 0..16 {
                    let page = sdk.history(token, noisy.id, HistoryQuery { stream_epoch: stream.stream_epoch, after, limit: 4 }).await.unwrap();
                    for item in page.items {
                        match item {
                            HistoryItem::Gap(_) => explicit_gap = true,
                            HistoryItem::Content(chunk) => for record in sdk.decode_history(chunk).await.unwrap() {
                                if let RecordedContent::Output { data, .. } = record.content { recorded.extend(STANDARD.decode(data).unwrap()); }
                            },
                        }
                    }
                    after = page.next;
                    if after.is_none() { break; }
                    assert!(page_number < 15, "bounded fixture history page limit");
                }
            }
            if recorded.windows(b"PRESSURE_FINAL".len()).any(|v| v == b"PRESSURE_FINAL") { break; }
            assert!(!explicit_gap, "normal pressure fixture unexpectedly lost recording");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        healthy_control.release().await.unwrap();
        noisy_control.release().await.unwrap();
        handle.terminate_current(machine, healthy.id, true, false).await.unwrap();
    }).await.expect("two-session pressure proof exceeded its owned deadline");
    println!(
        "Two real PTYs, bounded stalled consumer, replacement after old close, final output order and protected tail passed in {:?}",
        started_at.elapsed()
    );
}
