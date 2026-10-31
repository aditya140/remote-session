use super::*;
use crate::GatewayConfig;
use qgenie_remote_proto::wire::*;
use std::{
    future::Future,
    sync::atomic::{AtomicUsize, Ordering},
};
struct Fixture {
    api: GatewayClient,
    task: JoinHandle<()>,
    _ca: tempfile::NamedTempFile,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn fixture<F, Fut>(serve: F) -> Fixture
where
    F: FnOnce(Connection) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fixture_with_agents(serve, false).await
}
async fn fixture_with_agents<F, Fut>(serve: F, agents: bool) -> Fixture
where
    F: FnOnce(Connection) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fixture_with_capabilities(serve, agents, false).await
}
async fn fixture_with_capabilities<F, Fut>(serve: F, agents: bool, approval_status: bool) -> Fixture
where
    F: FnOnce(Connection) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fixture_with_discovery(serve, agents, approval_status, false).await
}
async fn fixture_with_discovery<F, Fut>(
    serve: F,
    agents: bool,
    approval_status: bool,
    discovery: bool,
) -> Fixture
where
    F: FnOnce(Connection) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let mut ca = tempfile::NamedTempFile::new().unwrap();
    use std::io::Write;
    ca.write_all(cert.cert.pem().as_bytes()).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into(),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!(
        "https://localhost:{}",
        listener.local_addr().unwrap().port()
    );
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let socket = tokio_rustls::TlsAcceptor::from(Arc::new(config))
            .accept(socket)
            .await
            .unwrap();
        let mut connection = ws::accept(socket, Limits::default(), None).await.unwrap();
        let request = read(&mut connection).await;
        let Message::Hello(hello) = request.message else {
            panic!("hello");
        };
        let mut available: BTreeSet<String> = [
            "routing-v1",
            "machine-pages-v1",
            "auth-refresh-v1",
            "session-control-v1",
            "session-replay-v1",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if discovery {
            available.insert("session-pages-v1".into());
            available.insert("root-discovery-v1".into());
        }
        if agents {
            available.insert("agent-events-v1".into());
        }
        if approval_status {
            available.insert("approval-status-v1".into());
            available.insert("durable-approvals-v1".into());
        }
        let welcome = negotiate(&hello, &[Version { major: 1, minor: 0 }], &available).unwrap();
        send(&connection, Envelope::new(Message::Welcome(welcome))).await;
        send(
            &connection,
            Envelope::new(Message::ConnectionReady {
                client: ClientId::new(),
                host_generation: None,
                activated: false,
            }),
        )
        .await;
        serve(connection).await;
    });
    Fixture {
        api: GatewayClient::new(GatewayConfig {
            origin,
            ca_bundle: Some(ca.path().into()),
        })
        .unwrap(),
        task,
        _ca: ca,
    }
}
fn token() -> AccessToken {
    "synthetic-live-token".to_owned().try_into().unwrap()
}

#[tokio::test]
async fn queued_command_observes_protocol_failure_before_its_send_error() {
    use tokio::io::AsyncWriteExt;
    let (mut peer, socket) = tokio::io::duplex(4096);
    let accept = tokio::spawn(ws::accept(socket, Limits::default(), None));
    peer.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n")
        .await
        .unwrap();
    let mut connection = accept.await.unwrap().unwrap();
    // A valid masked WebSocket frame with an invalid public JSON envelope.
    peer.write_all(&[0x81, 0x82, 0, 0, 0, 0, b'{', b'}'])
        .await
        .unwrap();
    assert!(matches!(
        connection.closed().await,
        Err(ws::Error::Protocol(_))
    ));
    let (commands, receive) = mpsc::channel(1);
    let (reply, response) = oneshot::channel();
    let permit = || Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
    commands
        .try_send(Command {
            one_way: false,
            envelope: Envelope::new(Message::SessionList),
            deadline: Instant::now() + Duration::from_secs(10),
            reply,
            _request: permit(),
            _bytes: permit(),
            _snapshot: None,
        })
        .unwrap();
    let (events, _events) = mpsc::channel(1);
    let (status, observed) = watch::channel(LiveStatus::Connected {
        client: ClientId::new(),
    });
    let (_stop, stopped) = watch::channel(false);
    // Both command and EOF are ready. The command branch runs first.
    run(connection, receive, events, status, stopped).await;
    assert_eq!(*observed.borrow(), LiveStatus::ProtocolError);
    let error = response.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidFrame);
    assert!(matches!(error.outcome, OperationOutcome::NotSent));
}

#[tokio::test]
async fn lost_wss_refresh_ack_allows_new_observation_but_never_resends_the_request() {
    let server = fixture(move |mut connection| async move {
        assert!(matches!(
            read(&mut connection).await.message,
            Message::Reauthenticate { .. }
        ));
        connection.close().await.unwrap();
    })
    .await;
    let (updates, credentials) =
        crate::UserCredentials::channel(crate::UserCredential::fixed(token()));
    let mut client = server
        .api
        .connect_authenticated(&credentials)
        .await
        .unwrap();
    updates.replace(crate::UserCredential::fixed(token()));
    tokio::time::timeout(Duration::from_secs(3), async {
        while client.next().await.is_some() {}
    })
    .await
    .unwrap();
    tokio::task::yield_now().await;
    assert_eq!(client.disconnect_error().code, ErrorCode::AccessStale);
    assert!(credentials.current().is_ok());
    client.close().await;
}

#[tokio::test]
async fn injected_rotation_reauthenticates_and_sign_in_failure_stops_the_connection() {
    let (seen, received) = oneshot::channel();
    let server = fixture(move |mut connection| async move {
        let request = read(&mut connection).await;
        let Message::Reauthenticate { token } = &request.message else {
            panic!("only reauthentication is expected")
        };
        assert_eq!(token.as_str(), "synthetic-rotated-token");
        send(
            &connection,
            response(
                &request,
                Message::AuthRefreshed {
                    expires_at: Timestamp::from_unix_millis(4102444800000).unwrap(),
                },
            ),
        )
        .await;
        seen.send(()).unwrap();
        assert!(
            connection.recv().await.is_none(),
            "rotation and failure must not send an application action"
        );
    })
    .await;
    let (updates, credentials) =
        crate::UserCredentials::channel(crate::UserCredential::fixed(token()));
    let mut client = server
        .api
        .connect_authenticated(&credentials)
        .await
        .unwrap();
    updates.replace(crate::UserCredential::fixed(
        "synthetic-rotated-token".to_owned().try_into().unwrap(),
    ));
    received.await.unwrap();
    updates.require_sign_in();
    tokio::time::timeout(Duration::from_secs(3), async {
        while client.next().await.is_some() {}
    })
    .await
    .unwrap();
    assert_eq!(client.disconnect_error().code, ErrorCode::Unauthenticated);
    assert!(!crate::observation_can_reconnect(
        &client.disconnect_error()
    ));
    client.close().await;
}

#[tokio::test]
async fn rejected_rotation_is_not_retried_and_cannot_restart_observation() {
    let server = fixture(move |mut connection| async move {
        let request = read(&mut connection).await;
        assert!(matches!(request.message, Message::Reauthenticate { .. }));
        send(
            &connection,
            response(
                &request,
                Message::Error(RemoteError::rejected(
                    ErrorCode::Forbidden,
                    "access revoked",
                )),
            ),
        )
        .await;
        assert!(connection.recv().await.is_none());
    })
    .await;
    let (updates, credentials) =
        crate::UserCredentials::channel(crate::UserCredential::fixed(token()));
    let mut client = server
        .api
        .connect_authenticated(&credentials)
        .await
        .unwrap();
    updates.replace(crate::UserCredential::fixed(token()));
    tokio::time::timeout(Duration::from_secs(3), async {
        while client.next().await.is_some() {}
    })
    .await
    .unwrap();
    assert_eq!(client.disconnect_error().code, ErrorCode::Forbidden);
    client.close().await;
}
async fn read(connection: &mut Connection) -> Envelope {
    let frame = tokio::time::timeout(Duration::from_secs(3), connection.recv())
        .await
        .unwrap()
        .unwrap();
    match &frame.frame {
        Frame::Text(envelope) => *envelope.clone(),
        _ => panic!("JSON request"),
    }
}
async fn send(connection: &Connection, envelope: Envelope) {
    connection
        .sender()
        .try_send(Priority::Event, Frame::Text(Box::new(envelope)))
        .unwrap()
        .transmitted()
        .await
        .unwrap();
}
fn response(request: &Envelope, message: Message) -> Envelope {
    let mut response = Envelope::new(message);
    response.re = Some(request.id);
    response.machine = request.machine;
    response.session = request.session;
    response
}
fn session(machine: MachineId) -> Session {
    Session {
        metadata: Default::default(),
        id: SessionId::new(),
        machine,
        owner: UserId::new(),
        kind: LaunchKind::Pty {
            command: vec!["fixture".into()],
        },
        cwd: "/fixture".into(),
        origin: Origin::Managed,
        state: SessionState::Running,
        created_at: Timestamp::from_unix_millis(1).unwrap(),
        ended_at: None,
        head: Counter(0),
        capabilities: Capabilities::default(),
        recording_warning: None,
    }
}

#[tokio::test]
async fn negotiated_roots_and_pages_reject_repeated_cursors() {
    let machine = MachineId::new();
    let value = session(machine);
    let id = value.id;
    let server = fixture_with_discovery(
        move |mut connection| async move {
            let roots = read(&mut connection).await;
            assert!(matches!(roots.message, Message::FsRoots));
            let mut reply = Envelope::new(Message::FsRootsListed {
                roots: vec!["/projects".into(), "/scratch".into()],
            });
            reply.re = Some(roots.id);
            reply.machine = Some(machine);
            send(&connection, reply).await;
            for _ in 0..2 {
                let request = read(&mut connection).await;
                assert!(matches!(
                    request.message,
                    Message::SessionListPage { limit: 1, .. }
                ));
                let mut reply = Envelope::new(Message::SessionPage {
                    sessions: vec![value.clone()],
                    next: Some(id),
                });
                reply.re = Some(request.id);
                reply.machine = Some(machine);
                send(&connection, reply).await;
            }
            std::future::pending::<()>().await;
        },
        false,
        false,
        true,
    )
    .await;
    let live = server.api.connect(&token()).await.unwrap();
    let handle = live.handle();
    assert_eq!(
        handle.allowed_roots(machine).await.unwrap(),
        vec!["/projects", "/scratch"]
    );
    let (page, next) = handle.session_page(machine, None, 1, false).await.unwrap();
    assert_eq!(page[0].id, id);
    assert_eq!(next, Some(id));
    assert!(handle.session_page(machine, next, 1, false).await.is_err());
    live.close().await;
}

#[tokio::test]
async fn old_gateway_keeps_manual_roots_and_explicit_complete_list_fallback() {
    let machine = MachineId::new();
    let value = session(machine);
    let server = fixture(move |mut connection| async move {
        let request = read(&mut connection).await;
        assert!(matches!(request.message, Message::SessionList));
        let mut reply = Envelope::new(Message::Sessions {
            sessions: vec![value],
        });
        reply.re = Some(request.id);
        reply.machine = Some(machine);
        send(&connection, reply).await;
        std::future::pending::<()>().await;
    })
    .await;
    let live = server.api.connect(&token()).await.unwrap();
    let handle = live.handle();
    for limit in [0, 257] {
        let error = handle
            .session_page(machine, None, limit, false)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidFrame);
        assert!(matches!(error.outcome, OperationOutcome::NotSent));
    }
    assert_eq!(
        handle.allowed_roots(machine).await.unwrap_err().code,
        ErrorCode::UnsupportedCapability
    );
    let (page, next) = handle.session_page(machine, None, 64, false).await.unwrap();
    assert_eq!(page.len(), 1);
    assert!(next.is_none());
    live.close().await;
}
#[tokio::test]
async fn reverse_responses_duplicates_and_reused_correlation_cannot_cross_requests() {
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let fixture = fixture(move |mut connection| async move {
        let first = read(&mut connection).await;
        let second = read(&mut connection).await;
        observed.fetch_add(2, Ordering::SeqCst);
        let reply = response(
            &second,
            Message::Sessions {
                sessions: vec![session(second.machine.unwrap())],
            },
        );
        send(&connection, reply.clone()).await;
        send(&connection, reply).await;
        send(
            &connection,
            response(
                &first,
                Message::Sessions {
                    sessions: vec![session(first.machine.unwrap())],
                },
            ),
        )
        .await;
        let third = read(&mut connection).await;
        observed.fetch_add(1, Ordering::SeqCst);
        send(
            &connection,
            response(
                &third,
                Message::Sessions {
                    sessions: Vec::new(),
                },
            ),
        )
        .await;
        while connection.recv().await.is_some() {}
    })
    .await;
    let mut client = fixture.api.connect(&token()).await.unwrap();
    let handle = client.handle();
    let a = MachineId::new();
    let b = MachineId::new();
    let (first, second) = tokio::join!(handle.sessions(a), handle.sessions(b));
    assert_eq!(first.unwrap()[0].machine, a);
    assert_eq!(second.unwrap()[0].machine, b);
    assert!(matches!(
        client.events.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    let mut request = Envelope::new(Message::SessionList);
    request.machine = Some(a);
    handle.request(request.clone()).await.unwrap();
    assert_eq!(
        handle.request(request).await.unwrap_err().code,
        ErrorCode::IdempotencyConflict
    );
    assert_eq!(count.load(Ordering::SeqCst), 3);
    client.close().await;
}
#[tokio::test]
async fn dropped_reply_is_unknown_without_resend_and_owner_drop_stops_handles() {
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let fixture = fixture(move |mut connection| async move {
        read(&mut connection).await;
        observed.fetch_add(1, Ordering::SeqCst);
        drop(connection);
    })
    .await;
    let client = fixture.api.connect(&token()).await.unwrap();
    let handle = client.handle();
    let status = client.status();
    let error = handle.sessions(MachineId::new()).await.unwrap_err();
    assert!(matches!(error.outcome, OperationOutcome::Unknown));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    drop(client);
    assert_eq!(*status.borrow(), LiveStatus::Stopped);
    assert!(matches!(
        handle.sessions(MachineId::new()).await.unwrap_err().outcome,
        OperationOutcome::NotSent
    ));
}
#[tokio::test]
async fn out_of_order_snapshot_is_rejected_without_exposing_partial_pages() {
    let fixture = fixture(move |mut connection| async move {
        let request = read(&mut connection).await;
        send(
            &connection,
            response(
                &request,
                Message::Snapshot {
                    snapshot: SnapshotPage {
                        epoch: "fixture".into(),
                        seq: Counter(5),
                        page: 1,
                        total_pages: 2,
                        rows: 24,
                        cols: 80,
                        format: SnapshotFormat::Vt100StateV1,
                        data: "AA==".into(),
                    },
                },
            ),
        )
        .await;
        while connection.recv().await.is_some() {}
    })
    .await;
    let client = fixture.api.connect(&token()).await.unwrap();
    let mut request = Envelope::new(Message::Attach { after: Counter(0) });
    request.machine = Some(MachineId::new());
    request.session = Some(SessionId::new());
    assert_eq!(
        client.handle().request(request).await.unwrap_err().code,
        ErrorCode::InvalidFrame
    );
    client.close().await;
}
#[tokio::test]
async fn slow_event_subscriber_is_disconnected_with_explicit_resync_state() {
    let fixture = fixture(move |connection| async move {
        for _ in 0..65 {
            send(
                &connection,
                Envelope::new(Message::RecordingWarning {
                    reason: "fixture".into(),
                    pending_bytes: Counter(0),
                }),
            )
            .await;
        }
        std::future::pending::<()>().await;
    })
    .await;
    let client = fixture.api.connect(&token()).await.unwrap();
    let mut status = client.status();
    tokio::time::timeout(Duration::from_secs(3), async {
        while *status.borrow() != LiveStatus::ResyncRequired {
            status.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(client.events.len(), 64);
    client.close().await;
}

#[tokio::test]
async fn lost_start_reply_keeps_original_scoped_key_without_resending() {
    for agent in [false, true] {
        let journal = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        let location = journal.path().to_owned();
        let machine = MachineId::new();
        let owner = UserId::new();
        let key = IdempotencyKey::new();
        let seen = Arc::new(AtomicUsize::new(0));
        let requests = seen.clone();
        let server = fixture_with_agents(
            move |mut connection| async move {
                let request = read(&mut connection).await;
                let Message::Start(launch) = request.message else {
                    panic!("persisted start")
                };
                assert_eq!(launch.key, key);
                assert_eq!(request.machine, Some(machine));
                let db =
                    rusqlite::Connection::open(location.join("start-identities.sqlite")).unwrap();
                let bytes: String = db
                    .query_row(
                        "SELECT body FROM identities WHERE key=?1",
                        [key.to_string()],
                        |row| row.get(0),
                    )
                    .unwrap();
                let saved: serde_json::Value = serde_json::from_str(&bytes).unwrap();
                assert_eq!(saved["key"], key.to_string());
                assert_eq!(saved["owner"], owner.to_string());
                assert_eq!(saved["machine"], machine.to_string());
                assert!(!bytes.contains("private-argv"));
                requests.fetch_add(1, Ordering::SeqCst);
                let _ = connection.close().await; // Native may have accepted; lose its reply.
            },
            agent,
        )
        .await;
        let client = server.api.connect(&token()).await.unwrap();
        let handle = client.handle();
        let launch = Launch {
            key,
            kind: if agent {
                LaunchKind::Agent {
                    agent: "codex".into(),
                    prompt: Some("private-argv".into()),
                    native_resume_id: None,
                }
            } else {
                LaunchKind::Pty {
                    command: vec!["/bin/sh".into(), "private-argv".into()],
                }
            },
            cwd: "/fixture".into(),
            rows: 24,
            cols: 80,
        };
        let mut generic = Envelope::new(Message::Start(launch.clone()));
        generic.machine = Some(machine);
        assert!(matches!(
            handle.request(generic).await.unwrap_err().outcome,
            OperationOutcome::NotSent
        ));
        let result = handle
            .start(journal.path(), owner, machine, launch.clone())
            .await
            .unwrap_err();
        assert!(matches!(result.outcome, OperationOutcome::Unknown));
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        let mut changed = launch;
        changed.cols = 81;
        let conflict = handle
            .start(journal.path(), owner, machine, changed)
            .await
            .unwrap_err();
        assert_eq!(conflict.code, ErrorCode::IdempotencyConflict);
        assert!(matches!(conflict.outcome, OperationOutcome::NotSent));
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        client.close().await;
        let records = handle
            .start_identities(journal.path(), owner, machine)
            .await
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].key, key);
        assert!(
            handle
                .start_identities(journal.path(), UserId::new(), machine)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            handle
                .start_identities(journal.path(), owner, MachineId::new())
                .await
                .unwrap()
                .is_empty()
        );
    }
}
#[tokio::test]
async fn remote_input_loss_and_cancellation_poison_control_without_resending() {
    for kind in 0..3 {
        for cancel in [false, true] {
            let machine = MachineId::new();
            let session = SessionId::new();
            let (holder_send, holder) = oneshot::channel();
            let (received_send, received) = oneshot::channel();
            let (finish_send, finish) = oneshot::channel();
            let count = Arc::new(AtomicUsize::new(0));
            let seen = count.clone();
            let server = fixture_with_agents(
                move |mut connection| async move {
                    let holder = holder.await.unwrap();
                    let request = read(&mut connection).await;
                    assert!(matches!(
                        request.message,
                        Message::LeaseRequest { takeover: false }
                    ));
                    send(
                        &connection,
                        response(
                            &request,
                            Message::LeaseGranted(ControlLease {
                                holder,
                                host_generation: Counter(1),
                                generation: Counter(2),
                                expires_at: Timestamp::from_unix_millis(4102444800000).unwrap(),
                                last_input: Counter(0),
                            }),
                        ),
                    )
                    .await;
                    let input = read(&mut connection).await;
                    assert!(matches!(
                        input.message,
                        Message::Input {
                            input_seq: Counter(1),
                            ..
                        }
                    ));
                    seen.fetch_add(1, Ordering::SeqCst);
                    received_send.send(()).unwrap();
                    if let Message::Input { input, .. } = input.message {
                        assert!(matches!(
                            (kind, input),
                            (0, SessionInput::Pty(_))
                                | (1, SessionInput::Prompt(_))
                                | (2, SessionInput::Steer(_))
                        ));
                    }
                    finish.await.unwrap();
                    let _ = connection.close().await;
                },
                kind != 0,
            )
            .await;
            let client = server.api.connect(&token()).await.unwrap();
            let handle = client.handle();
            holder_send.send(handle.id()).unwrap();
            let mut control = handle.take_control(machine, session, false).await.unwrap();
            if cancel {
                let mut sending = Box::pin(send_input_kind(
                    &mut control,
                    kind,
                    "one irreversible input",
                ));
                tokio::select! {result=&mut sending=>panic!("unexpected input reply: {result:?}"),_=received=>{}}
                drop(sending);
                finish_send.send(()).unwrap();
            } else {
                let (result, ()) = tokio::join!(
                    send_input_kind(&mut control, kind, "one irreversible input"),
                    async {
                        received.await.unwrap();
                        finish_send.send(()).unwrap();
                    }
                );
                assert!(matches!(
                    result.unwrap_err().outcome,
                    OperationOutcome::Unknown
                ));
            }
            assert!(control.input_uncertain());
            assert!(matches!(
                send_input_kind(&mut control, kind, "must not resend")
                    .await
                    .unwrap_err()
                    .outcome,
                OperationOutcome::NotSent
            ));
            assert_eq!(count.load(Ordering::SeqCst), 1);
            client.close().await;
        }
    }
}
async fn send_input_kind(
    control: &mut crate::RemoteController,
    kind: u8,
    text: &str,
) -> Result<(), RemoteError> {
    match kind {
        0 => control.input(text.as_bytes()).await,
        1 => control.prompt(text).await,
        _ => control.steer(text).await,
    }
}

#[tokio::test]
async fn unnegotiated_agent_input_sends_no_frame_and_does_not_poison_the_lease() {
    let (holder_send, holder) = oneshot::channel();
    let (done, observed) = oneshot::channel();
    let server = fixture(move |mut connection| async move {
        let holder = holder.await.unwrap();
        let request = read(&mut connection).await;
        send(&connection, response(&request, Message::LeaseGranted(ControlLease { holder, host_generation: Counter(1), generation: Counter(2), expires_at: Timestamp::from_unix_millis(4102444800000).unwrap(), last_input: Counter(0) }))).await;
        let mut inputs = 0;
        while let Some(frame) = connection.recv().await {
            assert!(!matches!(&frame.frame,Frame::Text(envelope) if matches!(envelope.message,Message::Start(_))),"unnegotiated agent start reached the gateway");
            if matches!(&frame.frame, Frame::Text(envelope) if matches!(envelope.message, Message::Input { .. })) { inputs += 1; }
        }
        let _ = done.send(inputs);
    }).await;
    let client = server.api.connect(&token()).await.unwrap();
    let handle = client.handle();
    holder_send.send(handle.id()).unwrap();
    let mut control = handle
        .take_control(MachineId::new(), SessionId::new(), false)
        .await
        .unwrap();
    for kind in [1, 2] {
        assert!(matches!(
            send_input_kind(&mut control, kind, "must not send")
                .await
                .unwrap_err()
                .outcome,
            OperationOutcome::NotSent
        ));
    }
    assert!(!control.input_uncertain());
    let journal = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let result = handle
        .start(
            journal.path(),
            UserId::new(),
            MachineId::new(),
            Launch {
                key: IdempotencyKey::new(),
                kind: LaunchKind::Agent {
                    agent: "codex".into(),
                    prompt: None,
                    native_resume_id: None,
                },
                cwd: "/fixture".into(),
                rows: 24,
                cols: 80,
            },
        )
        .await;
    assert!(matches!(
        result.unwrap_err().outcome,
        OperationOutcome::NotSent
    ));
    client.close().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), observed)
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn approval_status_is_read_only_negotiated_and_bound_to_its_original_identity() {
    for (capability, wrong_identity) in [(true, false), (true, true), (false, false)] {
        let approval = ApprovalId::new();
        let (done, observed) = oneshot::channel();
        let server = fixture_with_capabilities(
            move |mut connection| async move {
                let mut reads = 0;
                while let Some(frame) = connection.recv().await {
                    let Frame::Text(request) = &frame.frame else {
                        panic!("expected typed read");
                    };
                    let Message::ApprovalGet {
                        approval: requested,
                    } = request.message
                    else {
                        panic!("status must not resolve or redeliver an approval");
                    };
                    assert_eq!(requested, approval);
                    assert!(request.machine.is_none() && request.session.is_none());
                    reads += 1;
                    let owner = UserId::new();
                    let id = if wrong_identity {
                        ApprovalId::new()
                    } else {
                        approval
                    };
                    let request_body = ApprovalRequest {
                        id,
                        session: SessionId::new(),
                        owner,
                        native_id: "fixture".into(),
                        kind: "command".into(),
                        summary: "synthetic approval".into(),
                        request_hash: "a".repeat(64),
                        policy_revision: Counter(1),
                        requested_at: Timestamp::from_unix_millis(1).unwrap(),
                        expires_at: None,
                        allowed_decisions: vec![ApprovalDecision::AllowOnce],
                    };
                    let winner = ApprovalWinner {
                        approval: id,
                        id: DecisionId::new(),
                        actor: owner,
                        client: ClientId::new(),
                        request_hash: request_body.request_hash.clone(),
                        policy_revision: Counter(1),
                        decision: ApprovalDecision::AllowOnce,
                        committed_at: Timestamp::from_unix_millis(2).unwrap(),
                    };
                    send(
                        &connection,
                        response(
                            request,
                            Message::ApprovalStatus(ApprovalState {
                                machine: MachineId::new(),
                                request: request_body,
                                winner: Some(winner),
                                cancelled: false,
                                outcome: Some(ApplicationOutcome::OutcomeUnknown),
                                reason: None,
                            }),
                        ),
                    )
                    .await;
                }
                let _ = done.send(reads);
            },
            false,
            capability,
        )
        .await;
        let client = server.api.connect(&token()).await.unwrap();
        let result = client.handle().approval_status(approval).await;
        if !capability {
            assert!(matches!(
                result.unwrap_err().outcome,
                OperationOutcome::NotSent
            ));
        } else if wrong_identity {
            assert!(matches!(
                result.unwrap_err().outcome,
                OperationOutcome::Unknown
            ));
        } else {
            assert!(matches!(
                result.unwrap().outcome,
                Some(ApplicationOutcome::OutcomeUnknown)
            ));
        }
        client.close().await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), observed)
                .await
                .unwrap()
                .unwrap(),
            u32::from(capability)
        );
    }
}

#[tokio::test]
async fn approval_pages_advance_explicitly_and_refuse_a_repeated_cursor() {
    for repeated in [false, true] {
        let Message::ApprovalStatus(state) = Envelope::decode(include_bytes!(
            "../../../../conformance/json/network-approval-status.json"
        ))
        .unwrap()
        .message
        else {
            panic!("status fixture");
        };
        let id = state.request.id;
        let (done, observed) = oneshot::channel();
        let server = fixture_with_capabilities(move |mut connection| async move {
            let mut reads = 0;
            while let Some(frame) = connection.recv().await {
                let Frame::Text(request) = &frame.frame else { panic!("index request"); };
                assert!(matches!(request.message, Message::ApprovalIndex { limit: 1, .. }));
                reads += 1;
                let message = if reads == 1 { Message::ApprovalPage { states: vec![state.clone()], next: Some(id) } } else {
                    assert!(matches!(request.message, Message::ApprovalIndex { after: Some(after), .. } if after == id));
                    Message::ApprovalPage { states: Vec::new(), next: None }
                };
                send(&connection, response(request,message)).await;
            }
            let _ = done.send(reads);
        },false,true).await;
        let client = server.api.connect(&token()).await.unwrap();
        let handle = client.handle();
        let result = handle.approval_index(repeated.then_some(id), 1).await;
        if repeated {
            assert!(matches!(
                result.unwrap_err().outcome,
                OperationOutcome::Unknown
            ));
        } else {
            let page = result.unwrap();
            assert_eq!(page.states[0].request.id, id);
            let last = handle.approval_index(page.next, 1).await.unwrap();
            assert!(last.states.is_empty() && last.next.is_none());
        }
        client.close().await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), observed)
                .await
                .unwrap()
                .unwrap(),
            if repeated { 1 } else { 2 }
        );
    }
}

#[tokio::test]
async fn approval_choice_needs_no_lease_and_lost_replies_never_retry() {
    for scenario in 0..4 {
        let Message::ApprovalStatus(mut state) = Envelope::decode(include_bytes!(
            "../../../../conformance/json/network-approval-status.json"
        ))
        .unwrap()
        .message
        else {
            panic!("approval fixture");
        };
        let winner = state.winner.take().unwrap();
        state.outcome = None;
        state.cancelled = false;
        let mut unoffered = state.clone();
        unoffered.request.allowed_decisions = vec![ApprovalDecision::Deny];
        let original = state.clone();
        let (done, observed) = oneshot::channel();
        let server = fixture_with_capabilities(
            move |mut connection| async move {
                let mut choices = 0;
                while let Some(frame) = connection.recv().await {
                    let Frame::Text(request) = &frame.frame else {
                        panic!("approval request");
                    };
                    assert_eq!(request.machine, Some(original.machine));
                    assert_eq!(request.session, Some(original.request.session));
                    let Message::ApprovalResolve {
                        approval,
                        request_hash,
                        decision,
                    } = &request.message
                    else {
                        panic!("a choice must not acquire a controller lease");
                    };
                    assert_eq!(*approval, original.request.id);
                    assert_eq!(*request_hash, original.request.request_hash);
                    assert!(original.request.allowed_decisions.contains(decision));
                    choices += 1;
                    if scenario == 2 {
                        let _ = connection.close().await;
                        break;
                    }
                    send(
                        &connection,
                        response(request, Message::ApprovalCommitted(winner.clone())),
                    )
                    .await;
                }
                let _ = done.send(choices);
            },
            false,
            scenario != 0,
        )
        .await;
        let client = server.api.connect(&token()).await.unwrap();
        let result = client
            .handle()
            .resolve_approval(
                if scenario == 1 { &unoffered } else { &state },
                ApprovalDecision::AllowOnce,
            )
            .await;
        match scenario {
            0 | 1 => assert!(matches!(
                result.unwrap_err().outcome,
                OperationOutcome::NotSent
            )),
            2 => assert!(matches!(
                result.unwrap_err().outcome,
                OperationOutcome::Unknown
            )),
            _ => assert_eq!(result.unwrap().approval, state.request.id),
        }
        client.close().await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), observed)
                .await
                .unwrap()
                .unwrap(),
            usize::from(scenario >= 2)
        );
    }
}

async fn observer_baseline(connection: &mut Connection) {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let request = read(connection).await;
    assert!(matches!(
        request.message,
        Message::Attach { after: Counter(0) }
    ));
    send(
        connection,
        response(
            &request,
            Message::Replay {
                floor: Counter(1),
                head: Counter(0),
                events: Vec::new(),
                more: false,
                history_truncated: false,
            },
        ),
    )
    .await;
    let capture = read(connection).await;
    assert!(matches!(capture.message, Message::Capture));
    let state = AgentState::default().encode().unwrap();
    send(
        connection,
        response(
            &capture,
            Message::Snapshot {
                snapshot: SnapshotPage {
                    epoch: "observer-baseline".into(),
                    seq: Counter(0),
                    page: 0,
                    total_pages: 1,
                    rows: 24,
                    cols: 80,
                    format: SnapshotFormat::AgentStateV1,
                    data: STANDARD.encode(state),
                },
            },
        ),
    )
    .await;
    send(
        connection,
        response(
            &capture,
            Message::Head {
                floor: Counter(1),
                head: Counter(0),
                last_input: Counter(0),
            },
        ),
    )
    .await;
}
#[tokio::test]
async fn owned_observer_installs_a_checkpoint_then_releases_the_final_reply_before_disconnect() {
    let machine = MachineId::new();
    let session = SessionId::new();
    let server = fixture_with_agents(
        move |mut connection| async move {
            observer_baseline(&mut connection).await;
            let request = read(&mut connection).await;
            assert!(matches!(
                request.message,
                Message::ReplayRequest { after: Counter(0) }
            ));
            send(
                &connection,
                response(
                    &request,
                    Message::Replay {
                        floor: Counter(1),
                        head: Counter(1),
                        events: vec![SequencedEvent {
                            seq: Counter(1),
                            at: Timestamp::from_unix_millis(1).unwrap(),
                            event: AgentEvent::MessageCompleted {
                                message: "owned".into(),
                                text: "read-only output".into(),
                            },
                        }],
                        more: false,
                        history_truncated: false,
                    },
                ),
            )
            .await;
            let _ = connection.close().await;
        },
        true,
    )
    .await;
    let mut observer = server
        .api
        .observe_session(&token(), machine, session)
        .await
        .unwrap();
    let snapshot = tokio::time::timeout(Duration::from_secs(3), observer.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(snapshot.update.snapshot.is_some());
    drop(snapshot);
    let output = tokio::time::timeout(Duration::from_secs(3), observer.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(&output.update.events[0].event,AgentEvent::MessageCompleted{text,..} if text=="read-only output")
    );
    assert_eq!(output.update.events[0].seq, Counter(1));
    drop(output);
    let error = tokio::time::timeout(Duration::from_secs(3), observer.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AccessStale);
    observer.close().await;
}
#[tokio::test]
async fn stalled_output_subscriber_has_a_bounded_queue_and_a_recoverable_final_error() {
    let machine = MachineId::new();
    let session = SessionId::new();
    let (release, wait) = oneshot::channel();
    let server = fixture_with_agents(
        move |mut connection| async move {
            observer_baseline(&mut connection).await;
            wait.await.unwrap();
            for seq in 1..32 {
                let mut event = Envelope::new(Message::Event(SequencedEvent {
                    seq: Counter(seq),
                    at: Timestamp::from_unix_millis(seq).unwrap(),
                    event: AgentEvent::MessageDelta {
                        message: "owned".into(),
                        text: "observed output".into(),
                    },
                }));
                event.machine = Some(machine);
                event.session = Some(session);
                event.seq = Some(Counter(seq));
                let Ok(receipt) = connection
                    .sender()
                    .try_send(Priority::Event, Frame::Text(Box::new(event)))
                else {
                    break;
                };
                if receipt.transmitted().await.is_err() {
                    break;
                }
            }
            while connection.recv().await.is_some() {}
        },
        true,
    )
    .await;
    let mut observer = server
        .api
        .observe_session(&token(), machine, session)
        .await
        .unwrap();
    drop(observer.next().await.unwrap().unwrap());
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while observer.failure().is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(observer.failure().unwrap().code, ErrorCode::Overloaded);
    let mut count = 0;
    loop {
        match observer.next().await {
            Some(Ok(_)) => count += 1,
            Some(Err(error)) => {
                assert_eq!(error.code, ErrorCode::Overloaded);
                break;
            }
            None => panic!("subscriber overflow lost its final error"),
        }
    }
    assert!(count <= 8);
    observer.close().await;
}
#[tokio::test]
async fn cancelling_an_output_observer_closes_its_original_read_without_resending() {
    let (sent, seen) = oneshot::channel();
    let (done, closed) = oneshot::channel();
    let server = fixture_with_agents(
        move |mut connection| async move {
            let request = read(&mut connection).await;
            assert!(matches!(request.message, Message::Attach { .. }));
            sent.send(()).unwrap();
            let mut more = 0;
            while connection.recv().await.is_some() {
                more += 1;
            }
            let _ = done.send(more);
        },
        true,
    )
    .await;
    let observer = server
        .api
        .observe_session(&token(), MachineId::new(), SessionId::new())
        .await
        .unwrap();
    seen.await.unwrap();
    drop(observer);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), closed)
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn explicit_token_refresh_does_not_restore_uncertain_input_or_resend_its_bytes() {
    let machine = MachineId::new();
    let session = SessionId::new();
    let (holder_send, holder) = oneshot::channel();
    let (seen, received) = oneshot::channel();
    let (done, closed) = oneshot::channel();
    let server=fixture(move|mut connection|async move {
        let lease=read(&mut connection).await;
        send(&connection,response(&lease,Message::LeaseGranted(ControlLease{holder:holder.await.unwrap(),host_generation:Counter(1),generation:Counter(1),expires_at:Timestamp::from_unix_millis(4102444800000).unwrap(),last_input:Counter(0)}))).await;
        let input=read(&mut connection).await;assert!(matches!(input.message,Message::Input{input_seq:Counter(1),..}));seen.send(()).unwrap();
        let refresh=read(&mut connection).await;assert!(matches!(refresh.message,Message::Reauthenticate{..}));
        send(&connection,response(&refresh,Message::AuthRefreshed{expires_at:Timestamp::from_unix_millis(4102444800000).unwrap()})).await;
        send(&connection,response(&input,Message::Error(RemoteError::unknown("original input lost its access before acknowledgment")))).await;
        let mut repeated=0;while let Some(frame)=connection.recv().await {if matches!(&frame.frame,Frame::Text(request) if matches!(request.message,Message::Input{..})){repeated+=1;}}
        let _=done.send(repeated);
    }).await;
    let client = server.api.connect(&token()).await.unwrap();
    let handle = client.handle();
    holder_send.send(handle.id()).unwrap();
    let mut controller = handle.take_control(machine, session, false).await.unwrap();
    let (result, refresh) = tokio::join!(controller.input(b"must never repeat"), async {
        received.await.unwrap();
        handle
            .reauthenticate("synthetic-new-token".to_owned().try_into().unwrap())
            .await
    });
    refresh.unwrap();
    assert!(matches!(
        result.unwrap_err().outcome,
        OperationOutcome::Unknown
    ));
    assert!(controller.input_uncertain());
    assert!(matches!(
        controller
            .input(b"must never repeat")
            .await
            .unwrap_err()
            .outcome,
        OperationOutcome::NotSent
    ));
    client.close().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), closed)
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn session_metadata_old_peer_refuses_before_sending() {
    let mut fixture = fixture(move |mut connection| async move {
        assert!(
            connection.recv().await.is_none(),
            "metadata must not reach an old peer"
        );
    })
    .await;
    let client = fixture.api.connect(&token()).await.unwrap();
    let machine = MachineId::new();
    let session = SessionId::new();
    for error in [
        client
            .handle()
            .resolve_session_name(machine, "review-api")
            .await
            .unwrap_err(),
        client
            .handle()
            .update_session_metadata(
                machine,
                session,
                SessionMetadataChange::Rename(Some("review-api".into())),
            )
            .await
            .unwrap_err(),
    ] {
        assert_eq!(error.code, ErrorCode::UnsupportedCapability);
        assert!(matches!(error.outcome, OperationOutcome::NotSent));
    }
    let original = session_fixture_without_metadata(machine, session);
    let decoded: Session = serde_json::from_value(original).unwrap();
    assert!(decoded.metadata.is_empty());
    assert_eq!(decoded.id, session);
    client.close().await;
    tokio::time::timeout(Duration::from_secs(2), &mut fixture.task)
        .await
        .unwrap()
        .unwrap();
}

fn session_fixture_without_metadata(machine: MachineId, id: SessionId) -> serde_json::Value {
    let mut value = serde_json::to_value(session(machine)).unwrap();
    value["id"] = serde_json::json!(id);
    value.as_object_mut().unwrap().remove("metadata");
    value
}

mod work_inbox;
