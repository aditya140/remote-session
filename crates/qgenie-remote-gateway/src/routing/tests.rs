//! Actual PostgreSQL/Redis/TLS/WebSocket component proof. Host messages are explicit
//! fixtures; native execution is proved separately by the adapter/runtime tests.
use super::*;
use crate::identity::jwt::tests as jwt;
use crate::{Authenticator, IdentityService, JwtVerifier, Store};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use qgenie_remote_proto::{
    Counter, Hello, InstallationId, KeyId, SessionId, Version,
    rest::{EnrollmentRequest, Proof},
    wire::*,
};
use qgenie_remote_transport::ws::{
    self, ClientConfig, Connection, Frame, Limits, Priority, SecretHeader,
};
use std::{collections::BTreeMap, io::Write, sync::Arc, time::Duration};
use tokio::{net::TcpListener, task::JoinHandle};

fn private(bytes: &[u8]) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(bytes).unwrap();
    file
}
fn proof(key: &SigningKey, challenge: &qgenie_remote_proto::rest::Challenge) -> Proof {
    Proof {
        challenge: challenge.challenge,
        signature: STANDARD.encode(key.sign(&challenge.signing_bytes().unwrap()).to_bytes()),
    }
}
macro_rules! send {
    ($connection:expr,$frame:expr $(,)?) => {
        send_at($connection, $frame, concat!(file!(), ":", line!()))
    };
}
async fn send_at(connection: &Connection, envelope: Envelope, location: &str) {
    connection
        .sender()
        .try_send(Priority::Control, Frame::Text(Box::new(envelope)))
        .unwrap_or_else(|error| panic!("send failed at {location}: {error:?}"))
        .transmitted()
        .await
        .unwrap_or_else(|error| panic!("send failed at {location}: {error:?}"));
}
macro_rules! receive {
    ($connection:expr,$filter:expr) => {
        receive_at($connection, $filter, concat!(file!(), ":", line!()))
    };
}
async fn receive_at(
    connection: &mut Connection,
    filter: impl Fn(&Envelope) -> bool,
    location: &str,
) -> Envelope {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let item = connection
                .recv()
                .await
                .unwrap_or_else(|| panic!("socket unexpectedly closed at {location}"));
            if let Frame::Text(frame) = &item.frame
                && filter(frame)
            {
                return *frame.clone();
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("expected frame deadline at {location}"))
}
struct Server {
    port: u16,
    child: tokio::process::Child,
    proxy: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.proxy.abort();
        let _ = self.child.start_kill();
    }
}
impl Server {
    async fn shutdown(&mut self) {
        let id = self.child.id().unwrap();
        assert!(
            tokio::process::Command::new("kill")
                .args(["-TERM", &id.to_string()])
                .status()
                .await
                .unwrap()
                .success()
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(8), self.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }
}
struct Bindings<'a> {
    database: &'a std::path::Path,
    identity: &'a std::path::Path,
    machine_key: &'a std::path::Path,
    redis: &'a std::path::Path,
    routing_key: &'a std::path::Path,
}
async fn server(
    node: &str,
    bindings: &Bindings<'_>,
    acceptor: tokio_rustls::TlsAcceptor,
) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    let health = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let health_addr = health.local_addr().unwrap();
    drop(listener);
    drop(health);
    let binary =
        std::env::var_os("QGR_GATEWAY_BINARY").expect("build gateway and set QGR_GATEWAY_BINARY");
    let mut child = tokio::process::Command::new(binary)
        .arg("serve")
        .arg("--database-url-file")
        .arg(bindings.database)
        .arg("--identity-config-file")
        .arg(bindings.identity)
        .arg("--machine-key-file")
        .arg(bindings.machine_key)
        .arg("--redis-url-file")
        .arg(bindings.redis)
        .arg("--routing-key-file")
        .arg(bindings.routing_key)
        .arg("--node-id")
        .arg(node)
        .arg("--origin")
        .arg("https://gateway.example.invalid")
        .arg("--listen")
        .arg(upstream_addr.to_string())
        .arg("--health-listen")
        .arg(health_addr.to_string())
        .env("HTTPS_PROXY", "")
        .env("HTTP_PROXY", "")
        .env("NO_PROXY", "localhost,127.0.0.1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            assert!(
                child.try_wait().unwrap().is_none(),
                "gateway exited before ready"
            );
            if tokio::net::TcpStream::connect(upstream_addr).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let proxy = tokio::spawn(async move {
        let mut children = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                result=listener.accept(),if children.len()<128=> {let (stream,_)=result.unwrap();let acceptor=acceptor.clone();children.spawn(async move {let Ok(mut tls)=acceptor.accept(stream).await else{return;};let mut upstream=tokio::net::TcpStream::connect(upstream_addr).await.unwrap();let _=tokio::io::copy_bidirectional(&mut tls,&mut upstream).await;});},
                _=children.join_next(),if !children.is_empty()=>{}
            }
        }
    });
    Server { port, child, proxy }
}
struct Issuer {
    port: u16,
    task: JoinHandle<()>,
}
impl Drop for Issuer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn issuer(acceptor: tokio_rustls::TlsAcceptor) -> Issuer {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                result=listener.accept(),if tasks.len()<16=> {let (tcp,_)=result.unwrap();let acceptor=acceptor.clone();tasks.spawn(async move {let Ok(mut tls)=acceptor.accept(tcp).await else{return;};let mut header=Vec::new();while !header.ends_with(b"\r\n\r\n") {assert!(header.len()<8192);header.push(tls.read_u8().await.unwrap());}assert!(!String::from_utf8_lossy(&header).to_ascii_lowercase().contains("authorization:"));let body=include_str!("../../../../conformance/identity/synthetic-jwks.json");tls.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();});},
                _=tasks.join_next(),if !tasks.is_empty()=>{}
            }
        }
    });
    Issuer { port, task }
}
fn tls() -> (tokio_rustls::TlsAcceptor, tempfile::NamedTempFile) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert.der().clone()], key.into())
        .unwrap();
    (
        tokio_rustls::TlsAcceptor::from(Arc::new(config)),
        private(cert.cert.pem().as_bytes()),
    )
}
async fn connect(server: &Server, ca: &std::path::Path, role: &str, token: &str) -> Connection {
    let config = ClientConfig::new(&format!(
        "wss://localhost:{}/v1/connect/{role}",
        server.port
    ))
    .unwrap()
    .proxy(None)
    .unwrap()
    .ca_bundle(ca)
    .authorization(SecretHeader::bearer(token).unwrap());
    let mut connection = ws::connect(config, Limits::default(), None).await.unwrap();
    send!(
        &connection,
        Envelope::new(Message::Hello(Hello {
            versions: vec![Version { major: 1, minor: 0 }],
            required: [
                "routing-v1".into(),
                "machine-pages-v1".into(),
                "host-dispatch-v1".into()
            ]
            .into(),
            optional: ["auth-refresh-v1".into()].into(),
        })),
    )
    .await;
    // Do not discard unexpected frames: real clients require this exact prefix,
    // including when the owner has concurrent host inventory or approval events.
    let welcome = receive!(&mut connection, |_| true).await;
    assert!(matches!(welcome.message, Message::Welcome(_)));
    let ready = receive!(&mut connection, |_| true).await;
    assert!(matches!(ready.message, Message::ConnectionReady { .. }));
    connection
}
macro_rules! receive_host {
    ($connection:expr,$filter:expr) => {
        receive_host_at($connection, $filter, concat!(file!(), ":", line!()))
    };
}
async fn receive_host_at(
    connection: &mut Connection,
    filter: impl Fn(&Envelope) -> bool,
    location: &str,
) -> Envelope {
    let envelope = receive_at(
        connection,
        |envelope| match &envelope.message {
            Message::HostDispatch { message, .. } => filter(message),
            _ => filter(envelope),
        },
        location,
    )
    .await;
    if let Message::HostDispatch { context, message } = envelope.message {
        assert_eq!(envelope.machine, message.machine);
        assert!(context.host_generation.0 > 0);
        *message
    } else {
        envelope
    }
}
async fn closed(connection: &mut Connection) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while connection.recv().await.is_some() {}
    })
    .await
    .expect("peer must be disconnected");
}

pub(crate) async fn routing_contract(store: Store, admin: &sqlx::PgPool, database: &crate::Config) {
    let redis_file = std::env::var_os("QGR_REDIS_TEST_URL_FILE")
        .expect("set QGR_REDIS_TEST_URL_FILE to a disposable Redis service");
    let verifier =
        Arc::new(JwtVerifier::with_keys(jwt::config(), jwt::keys(), store.clone()).unwrap());
    let owner_jwt = jwt::sign(&jwt::claims("routing-owner"));
    let foreign_jwt = jwt::sign(&jwt::claims("routing-foreign"));
    let owner = verifier.authenticate(&owner_jwt).await.unwrap().user;
    let key_file = private(STANDARD.encode([71; 32]).as_bytes());
    let service = Arc::new(
        IdentityService::from_file(
            store.clone(),
            "https://gateway.example.invalid".into(),
            key_file.path(),
        )
        .await
        .unwrap(),
    );
    let key = SigningKey::from_bytes(&rand::random());
    let request = EnrollmentRequest {
        installation: InstallationId::new(),
        key: KeyId::new(),
        public_key: STANDARD.encode(key.verifying_key().as_bytes()),
        os_user: "routing-fixture".into(),
        hostname: "routing-host".into(),
        name: "routing host".into(),
        os: "linux".into(),
        arch: "x86_64".into(),
        cli_version: "0.1.0".into(),
        labels: BTreeMap::new(),
        recovery: None,
    };
    let challenge = service
        .enrollment_challenge(owner, request.clone())
        .await
        .unwrap();
    let enrolled = service
        .enroll(owner, proof(&key, &challenge))
        .await
        .unwrap();
    let challenge = service
        .machine_challenge(enrolled.machine, request.key)
        .await
        .unwrap();
    let token = service
        .machine_token(proof(&key, &challenge))
        .await
        .unwrap();
    let routing_key = private(STANDARD.encode([81; 32]).as_bytes());
    let routing_config = |node: &str| {
        RoutingConfig::from_files(
            node.into(),
            std::path::Path::new(&redis_file),
            routing_key.path(),
            None,
        )
        .unwrap()
    };
    presence_refresh_cleanup(&routing_config("presence-fixture")).await;
    let (acceptor, ca) = tls();
    let issuer = issuer(acceptor.clone()).await;
    let mut identity = jwt::config();
    identity.jwks_url = format!("https://localhost:{}/keys", issuer.port);
    identity.ca_bundle = Some(ca.path().into());
    let identity_file=private(serde_json::to_vec(&serde_json::json!({"issuer":identity.issuer,"tenant":identity.tenant,"audience":identity.audience,"remote_scope":identity.remote_scope,"admin_role":identity.admin_role,"jwks_url":identity.jwks_url,"ca_bundle":identity.ca_bundle})).unwrap().as_slice());
    let database_file = private(database.database.0.as_bytes());
    let bindings = Bindings {
        database: database_file.path(),
        identity: identity_file.path(),
        machine_key: key_file.path(),
        redis: std::path::Path::new(&redis_file),
        routing_key: routing_key.path(),
    };
    let mut server_a = server("test_a", &bindings, acceptor.clone()).await;
    let mut server_b = server("test_b", &bindings, acceptor).await;
    let config = |role: &str, token: &str| {
        ClientConfig::new(&format!(
            "wss://localhost:{}/v1/connect/{role}",
            server_b.port
        ))
        .unwrap()
        .proxy(None)
        .unwrap()
        .ca_bundle(ca.path())
        .authorization(SecretHeader::bearer(token).unwrap())
    };
    assert!(matches!(
        ws::connect(config("host", &owner_jwt), Limits::default(), None).await,
        Err(ws::Error::UpgradeRejected(401))
    ));
    assert!(matches!(
        ws::connect(
            config("client", token.token.as_str()),
            Limits::default(),
            None
        )
        .await,
        Err(ws::Error::UpgradeRejected(401))
    ));
    let mut client = connect(&server_b, ca.path(), "client", &owner_jwt).await;
    let initial = receive!(&mut client, |envelope| {
        matches!(envelope.message, Message::MachinePage { .. })
    })
    .await;
    assert!(
        matches!(initial.message,Message::MachinePage {machines,..} if machines.iter().any(|machine| machine.id==enrolled.machine && !machine.online))
    );
    let mut host = connect(&server_a, ca.path(), "host", token.token.as_str()).await;
    let online=receive!(&mut client,|envelope| matches!(&envelope.message,Message::MachineUpdated(machine) if machine.id==enrolled.machine && machine.online)).await;
    assert!(online.machine == Some(enrolled.machine));
    let session = Session {
        metadata: Default::default(),
        id: SessionId::new(),
        machine: enrolled.machine,
        owner,
        kind: LaunchKind::Pty {
            command: vec!["sh".into()],
        },
        cwd: "/fixture".into(),
        origin: Origin::Managed,
        state: qgenie_remote_proto::SessionState::Running,
        created_at: Timestamp::from_unix_millis(now_ms().unwrap()).unwrap(),
        ended_at: None,
        head: Counter(0),
        capabilities: Capabilities::default(),
        recording_warning: None,
    };
    let mut inventory = Envelope::new(Message::Sessions {
        sessions: vec![session.clone()],
    });
    inventory.machine = Some(enrolled.machine);
    send!(&host, inventory).await;
    receive!(&mut client,|envelope| matches!(&envelope.message,Message::MachineUpdated(machine) if machine.id==enrolled.machine && machine.sessions==1)).await;
    let metadata_actor = service.authenticate_machine(&token.token).await.unwrap();
    let metadata_generation: String =
        sqlx::query_scalar("SELECT generation::text FROM qgr.machine_routes WHERE machine=$1")
            .bind(enrolled.machine.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    super::metadata::tests::contract(
        &store,
        &metadata_actor,
        Counter(metadata_generation.parse().unwrap()),
        &session,
        admin,
    )
    .await;
    // The host's node has no client subscription yet. Its first client must not
    // receive owner-wide session events while waiting for Redis subscription.
    let publish = async {
        for _ in 0..100 {
            let mut update = Envelope::new(Message::Sessions {
                sessions: vec![session.clone()],
            });
            update.machine = Some(enrolled.machine);
            send!(&host, update).await;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    let reconnect = async {
        for _ in 0..12 {
            let connection = connect(&server_a, ca.path(), "client", &owner_jwt).await;
            connection.close().await.unwrap();
        }
    };
    tokio::join!(publish, reconnect);
    // Page a large ended history through the application role without hiding live work.
    let mut seeded = Vec::new();
    let mut tx = admin.begin().await.unwrap();
    for _ in 0..4100 {
        let mut old = session.clone();
        old.id = SessionId::new();
        old.state = qgenie_remote_proto::SessionState::Exited;
        old.ended_at = Some(old.created_at.clone());
        sqlx::query(
            "INSERT INTO qgr.sessions(id,machine,owner,body,state) VALUES($1,$2,$3,$4,'exited')",
        )
        .bind(old.id.to_string())
        .bind(old.machine.to_string())
        .bind(old.owner.to_string())
        .bind(serde_json::to_value(&old).unwrap())
        .execute(&mut *tx)
        .await
        .unwrap();
        seeded.push(old.id.to_string());
    }
    tx.commit().await.unwrap();
    let mut after = None;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let (page, next) = store
            .session_page(owner, enrolled.machine, after, 127, false)
            .await
            .unwrap();
        assert!(page.len() <= 127);
        assert!(page.windows(2).all(|pair| pair[0].id < pair[1].id));
        for item in &page {
            assert!(seen.insert(item.id));
        }
        if next.is_none() {
            break;
        }
        assert_eq!(next, page.last().map(|item| item.id));
        after = next;
    }
    assert_eq!(seen.len(), 4101);
    let (live, next) = store
        .session_page(owner, enrolled.machine, None, 64, true)
        .await
        .unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].id, session.id);
    assert!(next.is_none());
    assert!(
        store
            .session_page(
                qgenie_remote_proto::UserId::new(),
                enrolled.machine,
                None,
                64,
                false
            )
            .await
            .unwrap()
            .0
            .is_empty()
    );
    let mut request = Envelope::new(Message::SessionListPage {
        after: None,
        limit: 64,
        active_only: true,
    });
    request.machine = Some(enrolled.machine);
    let request_id = request.id;
    send!(&client, request).await;
    assert!(
        matches!(receive!(&mut client,|reply|reply.re==Some(request_id)).await.message,Message::SessionPage{sessions,next:None} if sessions.len()==1 && sessions[0].id==session.id)
    );
    sqlx::query("DELETE FROM qgr.sessions WHERE id=ANY($1)")
        .bind(seeded)
        .execute(admin)
        .await
        .unwrap();
    // Route a read across nodes. Affinity is absent; authenticated ownership determines admission.
    let mut listing = Envelope::new(Message::FsList {
        path: "/fixture".into(),
        cursor: None,
        limit: 10,
    });
    listing.machine = Some(enrolled.machine);
    let request_id = listing.id;
    send!(&client, listing).await;
    let forwarded = receive_host!(&mut host, |envelope| envelope.id == request_id).await;
    assert!(matches!(forwarded.message, Message::FsList { .. }));
    let mut reply = Envelope::new(Message::FsListed {
        entries: vec![],
        next: None,
    });
    reply.machine = Some(enrolled.machine);
    reply.re = Some(request_id);
    send!(&host, reply).await;
    receive!(&mut client, |envelope| {
        envelope.re == Some(request_id) && matches!(envelope.message, Message::FsListed { .. })
    })
    .await;
    // Explicitly refused operations never reach the host before task 017 activation.
    let mut mutation = Envelope::new(Message::LeaseRequest { takeover: false });
    mutation.machine = Some(enrolled.machine);
    mutation.session = Some(session.id);
    let id = mutation.id;
    send!(&client, mutation).await;
    assert!(
        matches!(receive!(&mut client,|envelope| envelope.re==Some(id)).await.message,Message::Error(error) if matches!(error.outcome,qgenie_remote_proto::OperationOutcome::NotSent))
    );
    let mut foreign = connect(&server_b, ca.path(), "client", &foreign_jwt).await;
    let mut stolen = Envelope::new(Message::FsList {
        path: "/fixture".into(),
        cursor: None,
        limit: 1,
    });
    stolen.machine = Some(enrolled.machine);
    let id = stolen.id;
    send!(&foreign, stolen).await;
    assert!(
        matches!(receive!(&mut foreign,|envelope| envelope.re==Some(id)).await.message,Message::Error(error) if matches!(error.code,qgenie_remote_proto::ErrorCode::Forbidden))
    );
    // Attach and fan out sequenced output only to this session's authorized observers.
    let mut attach = Envelope::new(Message::Attach { after: Counter(0) });
    attach.machine = Some(enrolled.machine);
    attach.session = Some(session.id);
    let id = attach.id;
    send!(&client, attach).await;
    receive_host!(&mut host, |envelope| envelope.id == id).await;
    // One attachment is a paged epoch followed by its post-capture head. Both
    // gateways must keep correlation until completion, rather than dropping it
    // after the first page.
    let checkpoint = include_bytes!("../../../../conformance/terminal/vt100-state-v1.bin");
    for (page, chunk) in checkpoint.chunks(checkpoint.len().div_ceil(2)).enumerate() {
        let mut snapshot = Envelope::new(Message::Snapshot {
            snapshot: SnapshotPage {
                epoch: "cross-node-capture".into(),
                seq: Counter(0),
                page: page as u16,
                total_pages: 2,
                rows: 24,
                cols: 80,
                format: qgenie_remote_proto::SnapshotFormat::Vt100StateV1,
                data: STANDARD.encode(chunk),
            },
        });
        snapshot.machine = Some(enrolled.machine);
        snapshot.session = Some(session.id);
        snapshot.re = Some(id);
        send!(&host, snapshot).await;
        receive!(&mut client,|envelope|envelope.re==Some(id) && matches!(&envelope.message,Message::Snapshot {snapshot} if snapshot.page==page as u16)).await;
    }
    let mut head = Envelope::new(Message::Head {
        floor: Counter(1),
        head: Counter(0),
        last_input: Counter(0),
    });
    head.machine = Some(enrolled.machine);
    head.session = Some(session.id);
    head.re = Some(id);
    send!(&host, head.clone()).await;
    receive!(&mut client, |envelope| envelope.re == Some(id)).await;
    send!(&host, head).await; // a late duplicate read response must not disconnect the host
    let frame = qgenie_remote_proto::PtyFrame {
        session: session.id,
        seq: Counter(1),
        payload: qgenie_remote_proto::PtyPayload::Output(b"actual cross-node delivery".to_vec()),
    };
    host.sender()
        .try_send(Priority::Bulk, Frame::Binary(frame))
        .unwrap()
        .transmitted()
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let received = client.recv().await.unwrap();
            if let Frame::Binary(frame) = &received.frame {
                assert_eq!(frame.session, session.id);
                assert_eq!(frame.seq, Counter(1));
                break;
            }
        }
    })
    .await
    .unwrap();
    // A full-size public JSON frame still fits the signed internal container.
    let mut event = Envelope::new(Message::Event(SequencedEvent {
        seq: Counter(2),
        at: Timestamp::from_unix_millis(now_ms().unwrap()).unwrap(),
        event: AgentEvent::ToolOutput {
            tool: "fixture".into(),
            text: String::new(),
        },
    }));
    event.machine = Some(enrolled.machine);
    event.session = Some(session.id);
    event.seq = Some(Counter(2));
    let padding = qgenie_remote_proto::MAX_JSON_BYTES - event.encode().unwrap().len();
    if let Message::Event(SequencedEvent {
        event: AgentEvent::ToolOutput { text, .. },
        ..
    }) = &mut event.message
    {
        *text = "x".repeat(padding);
    }
    assert_eq!(
        event.encode().unwrap().len(),
        qgenie_remote_proto::MAX_JSON_BYTES
    );
    send!(&host, event).await;
    receive!(&mut client, |envelope| envelope.seq == Some(Counter(2))).await;
    // The observer intentionally stops consuming application frames. Its bounded
    // queue must close independently while the active reader receives every frame.
    let mut slow = connect(&server_b, ca.path(), "client", &owner_jwt).await;
    let mut attach = Envelope::new(Message::Attach { after: Counter(2) });
    attach.machine = Some(enrolled.machine);
    attach.session = Some(session.id);
    let id = attach.id;
    send!(&slow, attach).await;
    receive_host!(&mut host, |envelope| envelope.id == id).await;
    let mut reply = Envelope::new(Message::Head {
        floor: Counter(1),
        head: Counter(2),
        last_input: Counter(0),
    });
    reply.machine = Some(enrolled.machine);
    reply.session = Some(session.id);
    reply.re = Some(id);
    send!(&host, reply).await;
    receive!(&mut slow, |envelope| envelope.re == Some(id)).await;
    let writing = async {
        for seq in 3..=130 {
            let frame = qgenie_remote_proto::PtyFrame {
                session: session.id,
                seq: Counter(seq),
                payload: qgenie_remote_proto::PtyPayload::Output(vec![b'x'; 32768]),
            };
            host.sender()
                .try_send(Priority::Bulk, Frame::Binary(frame))
                .unwrap()
                .transmitted()
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    let reading = async {
        let mut next = 3;
        while next <= 130 {
            let frame = client.recv().await.unwrap();
            if let Frame::Binary(frame) = &frame.frame {
                assert_eq!(frame.seq, Counter(next));
                next += 1;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(12), async {
        tokio::join!(writing, reading);
    })
    .await
    .unwrap();
    assert!(slow.closed().await.is_err());
    // In-place credential refresh cannot change an owner or host generation.
    send!(
        &client,
        Envelope::new(Message::Reauthenticate {
            token: owner_jwt.clone().try_into().unwrap(),
        }),
    )
    .await;
    receive!(&mut client, |envelope| {
        matches!(envelope.message, Message::AuthRefreshed { .. })
    })
    .await;
    send!(
        &host,
        Envelope::new(Message::Reauthenticate {
            token: token.token.clone().try_into().unwrap(),
        }),
    )
    .await;
    receive_host!(&mut host, |envelope| {
        matches!(envelope.message, Message::AuthRefreshed { .. })
    })
    .await;
    // Kill publishers while retaining subscribers. A source-side publish loss
    // must explicitly resync observers on another node after the broker rebuilds.
    let config = routing_config("publisher-fixture");
    let mut control = config
        .client
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let _: u64 = redis::cmd("CLIENT")
        .arg("KILL")
        .arg("TYPE")
        .arg("NORMAL")
        .arg("SKIPME")
        .arg("YES")
        .query_async(&mut control)
        .await
        .unwrap();
    let output = qgenie_remote_proto::PtyFrame {
        session: session.id,
        seq: Counter(131),
        payload: qgenie_remote_proto::PtyPayload::Output(b"delivery must be reconciled".to_vec()),
    };
    host.sender()
        .try_send(Priority::Bulk, Frame::Binary(output))
        .unwrap()
        .transmitted()
        .await
        .unwrap();
    closed(&mut client).await;
    let client = connect(&server_b, ca.path(), "client", &owner_jwt).await;
    // Replacement on another node advances the durable generation and closes the old socket.
    let mut replacement = connect(&server_b, ca.path(), "host", token.token.as_str()).await;
    closed(&mut host).await;
    let generation: String =
        sqlx::query_scalar("SELECT generation::text FROM qgr.machine_routes WHERE machine=$1")
            .bind(enrolled.machine.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert_eq!(generation, "2");
    let _ = client.close().await;
    // Expose local registration before the replacement's route hint is ready.
    // Bound the injected write pause below the broker's two-second budget.
    let _: () = redis::pipe()
        .atomic()
        .cmd("DEL")
        .arg(super::broker::hint_key(enrolled.machine))
        .ignore()
        .cmd("CLIENT")
        .arg("PAUSE")
        .arg(750)
        .arg("WRITE")
        .ignore()
        .query_async(&mut control)
        .await
        .unwrap();
    let mut client = connect(&server_a, ca.path(), "client", &owner_jwt).await;
    // ConnectionReady admits a local peer; Redis discovery can still be healing.
    // Wait for public presence before issuing this one cross-node request.
    receive!(&mut client, |envelope| match &envelope.message {
        Message::MachinePage { machines, .. } => machines
            .iter()
            .any(|machine| machine.id == enrolled.machine && machine.online),
        Message::MachineUpdated(machine) => machine.id == enrolled.machine && machine.online,
        _ => false,
    })
    .await;
    let mut attach = Envelope::new(Message::Attach {
        after: Counter(131),
    });
    attach.machine = Some(enrolled.machine);
    attach.session = Some(session.id);
    let id = attach.id;
    send!(&client, attach).await;
    tokio::select! {
        _ = receive_host!(&mut replacement, |envelope| envelope.id == id) => (),
        reply = receive!(&mut client, |envelope| envelope.re == Some(id)) => {
            panic!("attach returned before host dispatch: {:?}", reply.message);
        }
    }
    let mut reply = Envelope::new(Message::Head {
        floor: Counter(1),
        head: Counter(131),
        last_input: Counter(0),
    });
    reply.machine = Some(enrolled.machine);
    reply.session = Some(session.id);
    reply.re = Some(id);
    send!(&replacement, reply).await;
    receive!(&mut client, |envelope| envelope.re == Some(id)).await;
    // Drop Redis pub/sub sockets. Local peers remain owned; remote clients must resync.
    let config = routing_config("fixture");
    let mut redis = config
        .client
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    // Redis 8 pauses CLIENT UNPAUSE on this connection too. The fixture control
    // request may wait for the finite pause; production routing keeps its 2s budget.
    redis.set_response_timeout(Duration::from_secs(15));
    let _: u64 = redis::cmd("CLIENT")
        .arg("KILL")
        .arg("TYPE")
        .arg("PUBSUB")
        .query_async(&mut redis)
        .await
        .unwrap();
    closed(&mut client).await;
    // Reconnect is only discovery/subscription repair; no previous request is retransmitted.
    redis::cmd("CLIENT")
        .arg("PAUSE")
        .arg(12000)
        .arg("ALL")
        .exec_async(&mut redis)
        .await
        .unwrap();
    let mut client = connect(&server_b, ca.path(), "client", &owner_jwt).await;
    let mut list = Envelope::new(Message::SessionList);
    list.machine = Some(enrolled.machine);
    let id = list.id;
    send!(&client, list).await;
    let response = receive!(&mut client, |envelope| envelope.re == Some(id)).await;
    assert!(
        matches!(response.message,Message::Sessions {sessions} if sessions.iter().any(|value| value.id==session.id))
    );
    let mut local = Envelope::new(Message::FsList {
        path: "/fixture".into(),
        cursor: None,
        limit: 1,
    });
    local.machine = Some(enrolled.machine);
    let id = local.id;
    send!(&client, local).await;
    receive_host!(&mut replacement, |envelope| envelope.id == id).await;
    let mut reply = Envelope::new(Message::FsListed {
        entries: vec![],
        next: None,
    });
    reply.machine = Some(enrolled.machine);
    reply.re = Some(id);
    send!(&replacement, reply).await;
    receive!(&mut client, |envelope| {
        envelope.re == Some(id) && matches!(envelope.message, Message::FsListed { .. })
    })
    .await;
    // Redis cannot extend or supply access authority; this read used live PostgreSQL checks.
    redis::cmd("CLIENT")
        .arg("UNPAUSE")
        .exec_async(&mut redis)
        .await
        .unwrap();
    let mut recovered = connect(&server_a, ca.path(), "client", &owner_jwt).await;
    receive!(&mut recovered, |envelope| match &envelope.message {
        Message::MachinePage { machines, .. } => machines
            .iter()
            .any(|machine| machine.id == enrolled.machine && machine.online),
        Message::MachineUpdated(machine) => machine.id == enrolled.machine && machine.online,
        _ => false,
    })
    .await;
    let mut read = Envelope::new(Message::FsList {
        path: "/fixture".into(),
        cursor: None,
        limit: 1,
    });
    read.machine = Some(enrolled.machine);
    let id = read.id;
    send!(&recovered, read).await;
    receive_host!(&mut replacement, |envelope| envelope.id == id).await;
    let mut reply = Envelope::new(Message::FsListed {
        entries: vec![],
        next: None,
    });
    reply.machine = Some(enrolled.machine);
    reply.re = Some(id);
    send!(&replacement, reply).await;
    receive!(&mut recovered, |envelope| envelope.re == Some(id)).await;
    // Suppress every Redis hint throughout the revocation bound. Independent
    // connection watches must close both nodes and hosts without the broker.
    redis::cmd("CLIENT")
        .arg("PAUSE")
        .arg(16000)
        .arg("ALL")
        .exec_async(&mut redis)
        .await
        .unwrap();
    let mut admin_claims = jwt::claims("routing-revocation-admin");
    admin_claims["roles"] = serde_json::json!(["Remote.PlatformAdmin"]);
    let admin_token = jwt::sign(&admin_claims);
    let http = reqwest::Client::builder()
        .add_root_certificate(
            reqwest::Certificate::from_pem(&std::fs::read(ca.path()).unwrap()).unwrap(),
        )
        .no_proxy()
        .build()
        .unwrap();
    let request = qgenie_remote_proto::rest::RevokeAccess {
        request: qgenie_remote_proto::RequestId::new(),
        target: qgenie_remote_proto::rest::RevocationTarget::User { user: owner },
        confirmed: true,
    };
    let response = http
        .post(format!(
            "https://localhost:{}/v1/admin/revocations",
            server_a.port
        ))
        .bearer_auth(&admin_token)
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let result: qgenie_remote_proto::rest::AccessRevocation = response.json().await.unwrap();
    assert!(matches!(
        result.propagation,
        qgenie_remote_proto::rest::RevocationPropagation::Pending
    ));
    let committed = std::time::Instant::now();
    closed(&mut client).await;
    closed(&mut recovered).await;
    closed(&mut replacement).await;
    assert!(committed.elapsed() < Duration::from_secs(10));
    assert!(
        ws::connect(
            config_for(&server_b, ca.path(), &owner_jwt),
            Limits::default(),
            None
        )
        .await
        .is_err()
    );
    let status_url = format!(
        "https://localhost:{}/v1/admin/revocations/{}",
        server_b.port, request.request
    );
    // Another OS process cannot use the commit's UTC age or another node's
    // timer. Its first post-commit observation starts its own conservative wait.
    let pending: qgenie_remote_proto::rest::AccessRevocation = http
        .get(&status_url)
        .bearer_auth(&admin_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let observed = std::time::Instant::now();
    assert!(matches!(
        pending.propagation,
        qgenie_remote_proto::rest::RevocationPropagation::Pending
    ));
    tokio::time::sleep_until(
        (observed + Duration::from_millis(qgenie_remote_proto::MAX_ACCESS_LIFETIME_MS as u64 + 50))
            .into(),
    )
    .await;
    let completed: qgenie_remote_proto::rest::AccessRevocation = http
        .get(&status_url)
        .bearer_auth(&admin_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(matches!(
        completed.propagation,
        qgenie_remote_proto::rest::RevocationPropagation::Completed
    ));
    assert_eq!(completed.request, result.request);
    assert_eq!(completed.revision, result.revision);
    assert_eq!(completed.target, result.target);
    assert_eq!(completed.committed_at, result.committed_at);
    let retried: qgenie_remote_proto::rest::AccessRevocation = http
        .post(format!(
            "https://localhost:{}/v1/admin/revocations",
            server_a.port
        ))
        .bearer_auth(&admin_token)
        .json(&request)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(matches!(
        retried.propagation,
        qgenie_remote_proto::rest::RevocationPropagation::Completed
    ));
    assert_eq!(retried.revision, result.revision);
    // A cached completion cannot replace the mandatory current primary read.
    let mut status_lock = admin.begin().await.unwrap();
    sqlx::query("LOCK TABLE qgr.access_revocations IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *status_lock)
        .await
        .unwrap();
    let unavailable = http
        .get(&status_url)
        .bearer_auth(&admin_token)
        .timeout(Duration::from_secs(6))
        .send()
        .await
        .unwrap();
    assert_eq!(
        unavailable.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE
    );
    status_lock.rollback().await.unwrap();
    // Fixture control command waits for the finite pause; it cannot unblock
    // enforcement early or make the ten-second assertion pass through hints.
    redis::cmd("CLIENT")
        .arg("UNPAUSE")
        .exec_async(&mut redis)
        .await
        .unwrap();
    sqlx::query("UPDATE qgr.users SET disabled=false WHERE id=$1")
        .bind(owner.to_string())
        .execute(admin)
        .await
        .unwrap();
    let mut presence = connect(&server_b, ca.path(), "client", &owner_jwt).await;
    let unclean = connect(&server_a, ca.path(), "host", token.token.as_str()).await;
    receive!(&mut presence,|envelope| matches!(&envelope.message,Message::MachineUpdated(machine) if machine.id==enrolled.machine && machine.online)).await;
    drop(unclean);
    receive!(&mut presence,|envelope| matches!(&envelope.message,Message::MachineUpdated(machine) if machine.id==enrolled.machine && !machine.online)).await;
    // Stall the primary authority while one connection's handler is also
    // blocked in dispatch. The separate monitor must stop the real driver,
    // even though that handler and the inventory worker cannot finish a read.
    let mut stalled_client = connect(&server_a, ca.path(), "client", &owner_jwt).await;
    let mut stalled_host = connect(&server_b, ca.path(), "host", token.token.as_str()).await;
    let mut authority_lock = admin.begin().await.unwrap();
    sqlx::query("LOCK TABLE qgr.access_state IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *authority_lock)
        .await
        .unwrap();
    let mut blocked = Envelope::new(Message::FsList {
        path: "/fixture".into(),
        cursor: None,
        limit: 1,
    });
    blocked.machine = Some(enrolled.machine);
    send!(&stalled_client, blocked).await;
    let unavailable = std::time::Instant::now();
    tokio::join!(
        closed(&mut stalled_client),
        closed(&mut stalled_host),
        closed(&mut presence)
    );
    assert!(unavailable.elapsed() < Duration::from_secs(10));
    authority_lock.rollback().await.unwrap();
    let actor = service
        .authenticate_machine(token.token.as_str())
        .await
        .unwrap();
    assert!(matches!(
        store.project(&actor, Counter(1), &session).await,
        Err(crate::Error::Forbidden)
    ));
    sqlx::query("UPDATE qgr.machine_routes SET generation=18446744073709551614 WHERE machine=$1")
        .bind(enrolled.machine.to_string())
        .execute(admin)
        .await
        .unwrap();
    assert_eq!(
        store
            .connect_host(
                &actor,
                "counter-fixture",
                qgenie_remote_proto::ClientId::new()
            )
            .await
            .unwrap(),
        Counter(u64::MAX)
    );
    assert!(matches!(
        store
            .connect_host(
                &actor,
                "counter-fixture",
                qgenie_remote_proto::ClientId::new()
            )
            .await,
        Err(crate::Error::Overloaded)
    ));
    server_a.shutdown().await;
    server_b.shutdown().await;
    drop(foreign);
    drop(server_a);
    drop(server_b);
}

async fn presence_refresh_cleanup(config: &RoutingConfig) {
    let broker = broker::Broker::new(config);
    let (sender, _inbox) = tokio::sync::mpsc::channel(1);
    let _subscriber = broker.connect(sender, &[]).await.unwrap();
    let owner = qgenie_remote_proto::UserId::new();
    let machine = qgenie_remote_proto::MachineId::new();
    let first = qgenie_remote_proto::ClientId::new();
    // Every refresh has different signed bytes. Disconnect must clear the
    // whole connection generation, including a refresh that just completed.
    for _ in 0..2 {
        broker
            .set_hint(&config.node, owner, machine, first, Counter(1), None)
            .await
            .unwrap();
    }
    assert!(broker.hint(machine).await.unwrap().is_some());
    broker.clear_hint(machine, Counter(1)).await.unwrap();
    assert!(broker.hint(machine).await.unwrap().is_none());
    let replacement = qgenie_remote_proto::ClientId::new();
    broker
        .set_hint(&config.node, owner, machine, replacement, Counter(2), None)
        .await
        .unwrap();
    // A late old refresh and its cleanup cannot replace or erase the successor.
    broker
        .set_hint(&config.node, owner, machine, first, Counter(1), None)
        .await
        .unwrap();
    broker.clear_hint(machine, Counter(1)).await.unwrap();
    let hint = broker.hint(machine).await.unwrap().unwrap();
    assert!(
        matches!(hint.body, message::Body::Hint { connection, generation: Counter(2), .. } if connection == replacement)
    );
    broker.clear_hint(machine, Counter(2)).await.unwrap();
    broker.disconnect().await;
}
fn config_for(server: &Server, ca: &std::path::Path, token: &str) -> ClientConfig {
    ClientConfig::new(&format!(
        "wss://localhost:{}/v1/connect/client",
        server.port
    ))
    .unwrap()
    .proxy(None)
    .unwrap()
    .ca_bundle(ca)
    .authorization(SecretHeader::bearer(token).unwrap())
}
