use qgenie_remote_proto::*;
use qgenie_remote_sessiond::{Config, Runtime};
use std::{
    thread,
    time::{Duration, Instant},
};

fn call(runtime: &std::sync::Arc<Runtime>, operation: Operation) -> Reply {
    runtime
        .dispatch(Request {
            v: 1,
            id: RequestId::new(),
            client: ClientId::new(),
            body: operation,
        })
        .body
}

#[test]
fn bounded_replay_reports_truncation_and_restores_current_screen() {
    let temp = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let mut config = Config::new(path.clone(), vec![path.clone()]);
    config.replay_events = 2;
    let runtime = Runtime::open(config).unwrap();
    let spec = StartSpec {
        key: IdempotencyKey::new(),
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            "for i in 1 2 3 4 5; do printf 'line%s\\n' \"$i\"; sleep 0.03; done".into(),
        ],
        cwd: path.display().to_string(),
        rows: 24,
        cols: 80,
    };
    let Reply::Started(info) = call(&runtime, Operation::Start(spec)) else {
        panic!("start failed")
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let Reply::Replay(batch) = call(
            &runtime,
            Operation::Replay {
                session: info.id,
                after: Counter(0),
            },
        ) else {
            panic!("replay failed")
        };
        if matches!(batch.session.state, SessionState::Exited) {
            assert!(batch.truncated);
            assert!(batch.events.len() <= 2);
            assert!(batch.floor.0 > 1);
            let Reply::Attached(snapshot) = call(&runtime, Operation::Attach { session: info.id })
            else {
                panic!("snapshot failed")
            };
            assert_eq!(snapshot.snapshot.seq, batch.head);
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    runtime.shutdown(false).unwrap();
}
