use super::*;
use qgenie_remote_proto::ipc::HostActivation;
use qgenie_remote_transport::ipc::read_dispatch;

fn fixture() -> (tempfile::TempDir, Arc<Runtime>, RemoteContext) {
    let directory = crate::runtime::tests::directory();
    let path = std::fs::canonicalize(directory.path()).unwrap();
    let scope = RemoteScope {
        origin: "https://changes.example.invalid".into(),
        owner: UserId::new(),
        machine: MachineId::new(),
    };
    let mut config = Config::new(path.clone(), vec![path]);
    config.remote_bindings.push(scope.clone());
    let runtime = Runtime::open(config).unwrap();
    let connection = ClientId::new();
    let DispatchFrame::AccessChallengeIssued { nonce, .. } =
        runtime.dispatch_frame(DispatchFrame::AccessChallenge {
            scope: scope.clone(),
            connection,
            host_generation: Counter(1),
        })
    else {
        panic!("native challenge");
    };
    assert!(matches!(
        runtime.dispatch_frame(DispatchFrame::ActivateHost(HostActivation {
            scope: scope.clone(),
            connection,
            generation: Counter(1),
            nonce,
            remaining_ms: 10000,
            policy_revision: Counter(1),
        })),
        DispatchFrame::HostActivated { .. }
    ));
    let context = RemoteContext {
        actor: scope.owner,
        client: ClientId::new(),
        scope,
        connection,
        host_generation: Counter(1),
        controller_generation: None,
        access_nonce: nonce,
        remaining_access_ms: 10000,
        policy_revision: Counter(1),
    };
    (directory, runtime, context)
}
#[tokio::test]
async fn changes_are_scoped_coalesced_and_cancel_without_a_native_action() {
    let (_directory, runtime, context) = fixture();
    let (mut server, mut peer) = UnixStream::pair().unwrap();
    let native = runtime.clone();
    let original = context.clone();
    let task = tokio::spawn(async move { native.serve_changes(&mut server, original).await });
    assert!(matches!(
        read_dispatch(&mut peer).await.unwrap(),
        DispatchFrame::ChangesReady
    ));
    let mut foreign = context.scope.clone();
    foreign.owner = UserId::new();
    runtime.publish_change(Some(&foreign));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), read_dispatch(&mut peer))
            .await
            .is_err()
    );
    for _ in 0..10_000 {
        runtime.publish_change(Some(&context.scope));
    }
    assert!(matches!(
        read_dispatch(&mut peer).await.unwrap(),
        DispatchFrame::NativeChanged
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), read_dispatch(&mut peer))
            .await
            .is_err()
    );
    assert_eq!(lock(&runtime.changes).unwrap().len(), 1);
    drop(peer);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(runtime.list().unwrap().is_empty());
}
#[tokio::test]
async fn blocked_notification_never_uses_a_later_access_budget() {
    let (_directory, runtime, mut context) = fixture();
    context.remaining_access_ms = 100;
    let fence = lock(&runtime.authority).unwrap().fence(&context).unwrap();
    let (server, peer) = UnixStream::pair().unwrap();
    server.writable().await.unwrap();
    let mut filled = 0;
    loop {
        match server.try_write(&[b'x'; 8192]) {
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            result => panic!("could not fill owned fixture socket: {result:?}"),
        }
    }
    assert!(filled > 0);
    let task =
        tokio::spawn(
            async move { write_update(&server, &fence, DispatchFrame::NativeChanged).await },
        );
    tokio::time::sleep(Duration::from_millis(150)).await;
    context.remaining_access_ms = 2000;
    assert!(
        lock(&runtime.authority)
            .unwrap()
            .fence(&context)
            .unwrap()
            .check()
            .is_ok()
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let mut received = Vec::new();
    let mut bytes = [0; 8192];
    peer.readable().await.unwrap();
    loop {
        match peer.try_read(&mut bytes) {
            Ok(0) => break,
            Ok(count) => received.extend_from_slice(&bytes[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            result => panic!("owned fixture read: {result:?}"),
        }
    }
    assert_eq!(received, vec![b'x'; filled]);
}
#[tokio::test]
async fn foreign_owner_and_idle_expiry_cannot_retain_a_subscription() {
    let (_directory, runtime, mut context) = fixture();
    let (mut server, mut peer) = UnixStream::pair().unwrap();
    let mut foreign = context.clone();
    foreign.actor = UserId::new();
    assert!(runtime.serve_changes(&mut server, foreign).await.is_err());
    context.remaining_access_ms = 150;
    let native = runtime.clone();
    let task = tokio::spawn(async move { native.serve_changes(&mut server, context).await });
    assert!(matches!(
        read_dispatch(&mut peer).await.unwrap(),
        DispatchFrame::ChangesReady
    ));
    assert!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(read_dispatch(&mut peer).await.is_err());
}
