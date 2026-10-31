#[cfg(all(feature = "managed-codex", target_os = "linux"))]
#[path = "support/approval_drops.rs"]
mod approval_drops;
#[path = "support/identity.rs"]
mod fixture;
#[cfg(all(feature = "managed-codex", target_os = "linux"))]
#[path = "support/managed.rs"]
mod managed;
#[path = "support/recording.rs"]
mod recording;
#[path = "support/terminal_pressure.rs"]
mod terminal_pressure;
use axum::{
    body::Body,
    extract::{Request, State},
    middleware::{self, Next},
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use fixture::*;
use qgenie_remote_gateway::{Gateway, JwtConfig, RejectAuthentication};
use qgenie_remote_host::{
    BindingStatus, CredentialOptions, GatewayApi, Identity, MachineMetadata, Protection,
    RecordingConsent, oauth::OAuth,
};
use qgenie_remote_proto::{
    Counter,
    wire::{Envelope, Message},
};
use qgenie_remote_transport::ws::{
    self, ClientConfig, Connection, Frame, Limits, Priority, SecretHeader,
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
};

async fn callback(url: &str, state: &str) -> String {
    let uri = url::Url::parse(url).unwrap();
    let address = format!("127.0.0.1:{}", uri.port().unwrap());
    let mut stream = tokio::net::TcpStream::connect(&address).await.unwrap();
    stream.write_all(format!("GET /oauth/callback?code=synthetic-auth-code&state={state} HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    String::from_utf8(bytes).unwrap()
}
async fn lose_enrollment_reply(
    State(drop): State<Arc<AtomicBool>>,
    request: Request,
    next: Next,
) -> Response {
    let lose = request.method() == axum::http::Method::POST
        && request.uri().path() == "/v1/enrollments"
        && drop.swap(false, Ordering::SeqCst);
    let response = next.run(request).await;
    if lose {
        assert!(response.status().is_success());
        Response::new(Body::from_stream(futures_util::stream::once(async {
            Err::<Vec<u8>, _>(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "synthetic lost acknowledgement",
            ))
        })))
    } else {
        response
    }
}
fn metadata(hostname: &str) -> MachineMetadata {
    MachineMetadata {
        hostname: hostname.into(),
        name: "host fixture".into(),
        os: "linux".into(),
        arch: "x86_64".into(),
        cli_version: "0.1.0".into(),
        labels: Default::default(),
    }
}

#[tokio::test]
#[ignore = "requires disposable QGR_PG_TEST_URL_FILE; synthetic OAuth/HTTPS and real gateway"]
async fn scoped_pkce_device_enrollment_lost_ack_credentials_and_unenrollment() {
    let database = Database::create().await;
    let tls = Tls::new();
    let provider = Provider::default();
    let issuer = tls.bind().await.serve(provider.router()).await;
    let oauth = OAuth::new(provider.config(&issuer.origin, tls.ca.path().into())).unwrap();
    let (cancel, mut cancellation) = watch::channel(false);
    let device = oauth.device(&mut cancellation).await.unwrap();
    assert_eq!(device.user_code(), "FIXTURE-ONLY");
    assert!(!format!("{device:?}").contains("synthetic-device-code"));
    let tokens = device.wait(&mut cancellation).await.unwrap();
    assert!(!format!("{tokens:?}").contains("synthetic-refresh-do-not-deploy"));
    let access = tokens.access.clone();
    drop(tokens);
    let browser = oauth.browser().await.unwrap();
    let parsed = url::Url::parse(browser.authorization_url()).unwrap();
    let query = parsed.query_pairs().into_owned().collect::<HashMap<_, _>>();
    assert_eq!(query["code_challenge_method"], "S256");
    assert!(
        query["scope"]
            .split_ascii_whitespace()
            .any(|scope| scope == SCOPE)
    );
    *provider.proof.lock().unwrap() = Some((
        query["code_challenge"].clone(),
        query["redirect_uri"].clone(),
    ));
    let mut receiver = cancellation.clone();
    let wait = tokio::spawn(async move { browser.wait(&mut receiver).await });
    assert!(
        callback(&query["redirect_uri"], "wrong-state")
            .await
            .starts_with("HTTP/1.1 400")
    );
    assert!(
        callback(
            &query["redirect_uri"],
            &format!("wrong&state={}", query["state"])
        )
        .await
        .starts_with("HTTP/1.1 400")
    );
    assert!(
        callback(&query["redirect_uri"], &query["state"])
            .await
            .starts_with("HTTP/1.1 200")
    );
    let browser_tokens = wait.await.unwrap().unwrap();
    assert!(!browser_tokens.access.as_str().is_empty());
    cancel.send_replace(true);
    assert!(matches!(
        oauth.device(&mut cancellation).await,
        Err(qgenie_remote_host::Error::Cancelled)
    ));
    cancel.send_replace(false);
    provider.wrong_scope.store(1, Ordering::SeqCst);
    assert!(matches!(
        oauth
            .device(&mut cancellation)
            .await
            .unwrap()
            .wait(&mut cancellation)
            .await,
        Err(qgenie_remote_host::Error::Protocol)
    ));
    provider.wrong_scope.store(0, Ordering::SeqCst);
    let gateway_edge = tls.bind().await;
    let origin = gateway_edge.origin();
    let key = private(STANDARD.encode([77; 32]).as_bytes());
    let identity = JwtConfig {
        issuer: ISSUER.into(),
        tenant: TENANT.into(),
        audience: AUDIENCE.into(),
        remote_scope: "Remote.Control".into(),
        admin_role: "Remote.Admin".into(),
        jwks_url: format!("{}/keys", issuer.origin),
        ca_bundle: Some(tls.ca.path().into()),
    };
    let gateway = Gateway::connect(&database.config, Arc::new(RejectAuthentication))
        .await
        .unwrap()
        .enable_identity(identity, origin.clone(), key.path())
        .await
        .unwrap();
    let lose = Arc::new(AtomicBool::new(true));
    let server = gateway_edge
        .serve(
            gateway
                .router()
                .layer(middleware::from_fn_with_state(lose, lose_enrollment_reply)),
        )
        .await;
    let api = GatewayApi::new(&origin, Some(tls.ca.path())).unwrap();
    let directory = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let options = CredentialOptions {
        prefer_keyring: false,
        allow_private_file: true,
    };
    let mut identity = Identity::open(directory.path(), &origin, "uid-fixture", options)
        .await
        .unwrap();
    let installation = identity.installation();
    let public = identity.public_key().to_string();
    assert_eq!(identity.protection(), Protection::PrivateFile);
    let result = api
        .enroll(
            &mut identity,
            &access,
            metadata("original-host"),
            RecordingConsent::acknowledged(),
        )
        .await;
    assert!(matches!(
        result,
        Err(qgenie_remote_host::Error::Unavailable) | Err(qgenie_remote_host::Error::Protocol)
    ));
    assert!(identity.binding().is_none());
    drop(identity);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr.machines")
        .fetch_one(&database.admin)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let mut identity = Identity::open(directory.path(), &origin, "uid-fixture", options)
        .await
        .unwrap();
    assert_eq!(identity.installation(), installation);
    assert_eq!(identity.public_key(), public);
    let binding = api
        .enroll(
            &mut identity,
            &access,
            metadata("renamed-host"),
            RecordingConsent::acknowledged(),
        )
        .await
        .unwrap();
    let repeated = api
        .enroll(
            &mut identity,
            &access,
            metadata("another-hostname"),
            RecordingConsent::acknowledged(),
        )
        .await
        .unwrap();
    assert_eq!(binding.machine, repeated.machine);
    assert_eq!(binding.owner, repeated.owner);
    let machine = identity.machine_credentials().unwrap();
    let auth = api.authenticate_machine(&machine).await.unwrap();
    assert!(auth.expires > tokio::time::Instant::now());
    assert!(auth.expires <= tokio::time::Instant::now() + Duration::from_secs(900));
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        "authorization",
        format!("Bearer {}", auth.token.as_str()).parse().unwrap(),
    );
    assert_eq!(
        gateway.authenticate_host(&headers).await.unwrap().machine,
        binding.machine
    );
    let seed = std::fs::read(directory.path().join("machine.key")).unwrap();
    drop(identity);
    std::fs::remove_file(directory.path().join("machine.key")).unwrap();
    assert!(matches!(
        Identity::open(directory.path(), &origin, "uid-fixture", options).await,
        Err(qgenie_remote_host::Error::KeyLost)
    ));
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.path().join("machine.key"))
        .unwrap();
    std::io::Write::write_all(&mut file, &seed).unwrap();
    std::fs::set_permissions(
        directory.path().join("machine.key"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(
        Identity::open(directory.path(), &origin, "uid-fixture", options)
            .await
            .is_err()
    );
    std::fs::set_permissions(
        directory.path().join("machine.key"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(matches!(
        Identity::recover_key(directory.path(), &origin, "uid-fixture", options, false).await,
        Err(qgenie_remote_host::Error::Denied)
    ));
    let recovered = Identity::recover_key(directory.path(), &origin, "uid-fixture", options, true)
        .await
        .unwrap();
    assert_eq!(recovered.installation(), installation);
    assert_ne!(recovered.public_key(), public);
    let replacement = recovered.public_key().to_string();
    assert_eq!(
        recovered.binding().unwrap().status,
        BindingStatus::RecoveryPending
    );
    drop(recovered);
    let mut identity = Identity::open(directory.path(), &origin, "uid-fixture", options)
        .await
        .unwrap();
    assert_eq!(identity.public_key(), replacement);
    let recovered = api
        .enroll(
            &mut identity,
            &access,
            metadata("recovered-host"),
            RecordingConsent::acknowledged(),
        )
        .await
        .unwrap();
    assert_eq!(recovered.machine, binding.machine);
    assert_eq!(recovered.owner, binding.owner);
    assert!(api.authenticate_machine(&machine).await.is_err());
    let machine = identity.machine_credentials().unwrap();
    assert!(api.authenticate_machine(&machine).await.is_ok());
    assert!(!format!("{identity:?}").contains(&STANDARD.encode(&seed)));
    for entry in std::fs::read_dir(directory.path()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains(access.as_str()));
        assert!(!text.contains("synthetic-refresh-do-not-deploy"));
    }
    assert!(matches!(
        api.unenroll(&mut identity, &access, false).await,
        Err(qgenie_remote_host::Error::Denied)
    ));
    api.unenroll(&mut identity, &access, true).await.unwrap();
    assert_eq!(identity.binding().unwrap().status, BindingStatus::Revoked);
    assert!(api.authenticate_machine(&machine).await.is_err());
    drop(identity);
    assert!(matches!(
        Identity::open(directory.path(), &origin, "uid-fixture", options).await,
        Err(qgenie_remote_host::Error::Revoked)
    ));
    drop(server);
    gateway.shutdown().await;
    drop(issuer);
    database.destroy().await;
}

#[tokio::test]
async fn device_pkce_extension_binds_polling_to_original_verifier() {
    for enabled in [false, true] {
        let tls = Tls::new();
        let provider = Provider::default();
        let issuer = tls.bind().await.serve(provider.router()).await;
        let mut config = provider.config(&issuer.origin, tls.ca.path().into());
        config.device_pkce = enabled;
        let oauth = OAuth::new(config).unwrap();
        let (_cancel, mut cancellation) = watch::channel(false);
        let login = oauth.device(&mut cancellation).await.unwrap();
        assert_eq!(provider.device_challenge.lock().unwrap().is_some(), enabled);
        login.wait(&mut cancellation).await.unwrap();
        assert_eq!(provider.polls.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn device_provider_slow_down_cancellation_expiry_and_tls_are_enforced() {
    let tls = Tls::new();
    let provider = Provider::default();
    let issuer = tls.bind().await.serve(provider.router()).await;
    let config = provider.config(&issuer.origin, tls.ca.path().into());
    let oauth = OAuth::new(config.clone()).unwrap();
    let (cancel, mut cancellation) = watch::channel(false);
    let mut untrusted = config;
    untrusted.ca_bundle = None;
    assert!(matches!(
        OAuth::new(untrusted)
            .unwrap()
            .device(&mut cancellation)
            .await,
        Err(qgenie_remote_host::Error::Unavailable)
    ));
    provider.mode.store(1, Ordering::SeqCst);
    let login = oauth.device(&mut cancellation).await.unwrap();
    tokio::time::timeout(Duration::from_secs(12), login.wait(&mut cancellation))
        .await
        .unwrap()
        .unwrap();
    let times = provider.times.lock().unwrap().clone();
    assert_eq!(times.len(), 2);
    assert!(times[1].duration_since(times[0]) >= Duration::from_secs(6));
    provider.polls.store(0, Ordering::SeqCst);
    provider.mode.store(0, Ordering::SeqCst);
    let login = oauth.device(&mut cancellation).await.unwrap();
    let mut receiver = cancellation.clone();
    let waiter = tokio::spawn(async move { login.wait(&mut receiver).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while provider.polls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    cancel.send_replace(true);
    assert!(matches!(
        tokio::time::timeout(Duration::from_millis(500), waiter)
            .await
            .unwrap()
            .unwrap(),
        Err(qgenie_remote_host::Error::Cancelled)
    ));
    cancel.send_replace(false);
    provider.polls.store(0, Ordering::SeqCst);
    provider.mode.store(2, Ordering::SeqCst);
    provider.lifetime.store(2, Ordering::SeqCst);
    assert!(matches!(
        oauth
            .device(&mut cancellation)
            .await
            .unwrap()
            .wait(&mut cancellation)
            .await,
        Err(qgenie_remote_host::Error::Expired)
    ));
    provider.mode.store(3, Ordering::SeqCst);
    provider.lifetime.store(30, Ordering::SeqCst);
    assert!(matches!(
        oauth
            .device(&mut cancellation)
            .await
            .unwrap()
            .wait(&mut cancellation)
            .await,
        Err(qgenie_remote_host::Error::Denied)
    ));
}

async fn send_frame(connection: &Connection, envelope: Envelope) {
    connection
        .sender()
        .try_send(Priority::Control, Frame::Text(Box::new(envelope)))
        .unwrap()
        .transmitted()
        .await
        .unwrap();
}
async fn receive_frame(
    connection: &mut Connection,
    filter: impl Fn(&Envelope) -> bool,
) -> Envelope {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let received = connection.recv().await.expect("connection closed");
            if let Frame::Text(envelope) = &received.frame
                && filter(envelope)
            {
                return *envelope.clone();
            }
        }
    })
    .await
    .unwrap()
}
async fn socket(origin: &str, ca: &std::path::Path, role: &str, token: &str) -> Connection {
    socket_with_native(origin, ca, role, token, false).await
}
async fn socket_with_native(
    origin: &str,
    ca: &std::path::Path,
    role: &str,
    token: &str,
    native: bool,
) -> Connection {
    let endpoint = format!(
        "{}/v1/connect/{role}",
        origin.replacen("https://", "wss://", 1)
    );
    let config = ClientConfig::new(&endpoint)
        .unwrap()
        .proxy(None)
        .unwrap()
        .ca_bundle(ca)
        .authorization(SecretHeader::bearer(token).unwrap());
    let mut connection = ws::connect(config, Limits::default(), None).await.unwrap();
    let mut required: std::collections::BTreeSet<String> = [
        "routing-v1",
        "machine-pages-v1",
        "host-dispatch-v1",
        "auth-refresh-v1",
    ]
    .map(str::to_owned)
    .into();
    if native {
        required.insert("host-native-authority-v1".into());
    }
    send_frame(
        &connection,
        Envelope::new(Message::Hello(qgenie_remote_proto::Hello {
            versions: vec![qgenie_remote_proto::Version { major: 1, minor: 0 }],
            required,
            optional: Default::default(),
        })),
    )
    .await;
    receive_frame(&mut connection, |envelope| {
        matches!(envelope.message, Message::ConnectionReady { .. })
    })
    .await;
    connection
}
#[derive(Clone)]
struct TokenClock(Arc<std::sync::atomic::AtomicUsize>);
async fn short_token(State(clock): State<TokenClock>, request: Request, next: Next) -> Response {
    let token = request.uri().path() == "/v1/machine-tokens";
    let response = next.run(request).await;
    if !token || !response.status().is_success() {
        return response;
    }
    clock.0.fetch_add(1, Ordering::SeqCst);
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    value["expires_at"] = serde_json::to_value(
        qgenie_remote_proto::wire::Timestamp::from_unix_millis(now + 15000).unwrap(),
    )
    .unwrap();
    axum::response::IntoResponse::into_response(axum::Json(value))
}
#[tokio::test]
#[ignore = "requires disposable QGR_PG_TEST_URL_FILE and QGR_REDIS_TEST_URL_FILE"]
async fn actual_connector_enforces_roots_refreshes_in_place_and_survives_replacement() {
    connector_contract(false).await;
}
#[tokio::test]
#[ignore = "requires disposable QGR_PG_TEST_URL_FILE and QGR_REDIS_TEST_URL_FILE"]
async fn actual_native_connector_activates_original_nonces_across_gateways_and_survives_revocation()
{
    connector_contract(true).await;
}
async fn connector_contract(native_mode: bool) {
    use qgenie_remote_host::{
        ConnectionStatus, Connector,
        filesystem::{Filesystem, Roots},
    };
    let database = Database::create().await;
    let tls = Tls::new();
    let provider = Provider::default();
    let issuer = tls.bind().await.serve(provider.router()).await;
    let oauth = OAuth::new(provider.config(&issuer.origin, tls.ca.path().into())).unwrap();
    let (_login_cancel, mut login_cancellation) = watch::channel(false);
    let tokens = oauth
        .device(&mut login_cancellation)
        .await
        .unwrap()
        .wait(&mut login_cancellation)
        .await
        .unwrap();
    let edge = tls.bind().await;
    let origin = edge.origin();
    let key = private(STANDARD.encode([88; 32]).as_bytes());
    let graph_secret = private(b"synthetic-graph-application-secret");
    let routing_key = private(STANDARD.encode([89; 32]).as_bytes());
    let redis = std::env::var_os("QGR_REDIS_TEST_URL_FILE").expect("set disposable Redis binding");
    let mut gateway = Gateway::connect(&database.config, Arc::new(RejectAuthentication))
        .await
        .unwrap()
        .enable_identity(
            JwtConfig {
                issuer: ISSUER.into(),
                tenant: TENANT.into(),
                audience: AUDIENCE.into(),
                remote_scope: "Remote.Control".into(),
                admin_role: "Remote.Admin".into(),
                jwks_url: format!("{}/keys", issuer.origin),
                ca_bundle: Some(tls.ca.path().into()),
            },
            origin.clone(),
            key.path(),
        )
        .await
        .unwrap();
    if native_mode {
        gateway = gateway
            .enable_directory(qgenie_remote_gateway::DirectoryConfig {
                tenant: TENANT.into(),
                client_id: "33333333-3333-4333-8333-333333333333".into(),
                credential_file: graph_secret.path().into(),
                token_url: Some(format!("{}/graph-token", issuer.origin)),
                graph_base: Some(format!("{}/v1.0", issuer.origin)),
                ca_bundle: Some(tls.ca.path().into()),
            })
            .await
            .unwrap();
    }
    let peer = if native_mode {
        Some(
            gateway
                .clone()
                .enable_routing(
                    qgenie_remote_gateway::RoutingConfig::from_files(
                        "connector-peer-fixture".into(),
                        std::path::Path::new(&redis),
                        routing_key.path(),
                        None,
                    )
                    .unwrap(),
                )
                .await
                .unwrap(),
        )
    } else {
        None
    };
    let gateway = gateway
        .enable_routing(
            qgenie_remote_gateway::RoutingConfig::from_files(
                "connector-fixture".into(),
                std::path::Path::new(&redis),
                routing_key.path(),
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let clock = TokenClock(Arc::new(std::sync::atomic::AtomicUsize::new(0)));
    let server = edge
        .serve(
            gateway
                .router()
                .layer(middleware::from_fn_with_state(clock.clone(), short_token)),
        )
        .await;
    let peer_server = if let Some(peer) = &peer {
        Some(tls.bind().await.serve(peer.router()).await)
    } else {
        None
    };
    let api = GatewayApi::new(&origin, Some(tls.ca.path())).unwrap();
    let directory = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let mut identity = Identity::open(
        directory.path(),
        &origin,
        "connector-fixture",
        CredentialOptions {
            prefer_keyring: false,
            allow_private_file: true,
        },
    )
    .await
    .unwrap();
    let binding = api
        .enroll(
            &mut identity,
            &tokens.access,
            metadata("connector-host"),
            RecordingConsent::acknowledged(),
        )
        .await
        .unwrap();
    let files = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let allowed = files.path().join("allowed");
    let outside = files.path().join("outside");
    std::fs::create_dir(&allowed).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(allowed.join("inside.txt"), b"synthetic fixture").unwrap();
    std::os::unix::fs::symlink(&outside, allowed.join("escape")).unwrap();
    let native_state = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let native_path = std::fs::canonicalize(native_state.path()).unwrap();
    let local = qgenie_remote_client::Client::local(&native_path);
    let mut daemon = if native_mode {
        Some(NativeDaemon::start(&native_path, &allowed))
    } else {
        None
    };
    let mut legacy = None;
    let native = if let Some(daemon) = &mut daemon {
        daemon.ready(&local).await;
        let local_key = qgenie_remote_proto::IdempotencyKey::new();
        let local_session = local
            .start(qgenie_remote_proto::StartSpec {
                key: local_key,
                command: vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    "printf 'local-private'; read value".into(),
                ],
                cwd: allowed.to_str().unwrap().into(),
                rows: 24,
                cols: 80,
            })
            .await
            .unwrap();
        legacy = Some((local_key, local_session));
        let native = qgenie_remote_host::NativeClient::new(
            native_path.clone(),
            qgenie_remote_proto::ipc::RemoteScope {
                origin: origin.clone(),
                owner: binding.owner,
                machine: binding.machine,
            },
        )
        .unwrap();
        native.register_binding().await.unwrap();
        Some(native)
    } else {
        None
    };
    let mut connector = Connector::new(
        api.clone(),
        identity.machine_credentials().unwrap(),
        Filesystem::new(Roots::open(std::slice::from_ref(&allowed)).unwrap()),
        Some(tls.ca.path().into()),
        true,
    )
    .unwrap();
    if let Some(native) = &native {
        connector = connector.with_native(native.clone()).unwrap();
    }
    let (stop, mut cancellation) = watch::channel(false);
    let (status, mut updates) = watch::channel(ConnectionStatus::Disconnected);
    let task = tokio::spawn(async move { connector.run(&mut cancellation, status).await });
    tokio::time::timeout(Duration::from_secs(8), async {
        while !matches!(*updates.borrow(), ConnectionStatus::Connected { .. }) {
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(
        matches!(*updates.borrow(),ConnectionStatus::Connected {activated,..} if activated==native_mode)
    );
    let client_origin = peer_server
        .as_ref()
        .map(|server| server.origin.as_str())
        .unwrap_or(&origin);
    let mut client = socket(
        client_origin,
        tls.ca.path(),
        "client",
        tokens.access.as_str(),
    )
    .await;
    let sdk = qgenie_remote_client::GatewayClient::new(qgenie_remote_client::GatewayConfig {
        origin: client_origin.into(),
        ca_bundle: Some(tls.ca.path().into()),
    })
    .unwrap();
    let mut observer = sdk.connect(&tokens.access).await.unwrap();
    let initial = tokio::time::timeout(Duration::from_secs(5), observer.next())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(&initial.frame,qgenie_remote_client::LiveFrame::Text(envelope) if matches!(&envelope.message,Message::MachinePage{machines,..} if machines.iter().any(|m|m.id==binding.machine)))
    );
    assert!(
        observer
            .handle()
            .sessions(binding.machine)
            .await
            .unwrap()
            .is_empty()
    );
    let (entries, _) = observer
        .handle()
        .browse(binding.machine, allowed.to_str().unwrap().into(), None, 10)
        .await
        .unwrap();
    assert!(entries.iter().any(|entry| entry.name == "inside.txt"));
    observer
        .handle()
        .reauthenticate(tokens.access.clone())
        .await
        .unwrap();
    let first_nonce: Option<String> = if native_mode {
        Some(
            sqlx::query_scalar(
                "SELECT native_access->>'nonce' FROM qgr.machine_routes WHERE machine=$1",
            )
            .bind(binding.machine.to_string())
            .fetch_one(&database.admin)
            .await
            .unwrap(),
        )
    } else {
        None
    };
    if let Some((key, legacy)) = &legacy {
        let mut lookup = Envelope::new(Message::StartLookup { key: *key });
        lookup.machine = Some(binding.machine);
        let id = lookup.id;
        send_frame(&client, lookup).await;
        assert!(
            matches!(receive_frame(&mut client,|reply| reply.re==Some(id)).await.message, Message::Error(error) if matches!(error.outcome,qgenie_remote_proto::OperationOutcome::Unknown))
        );
        assert_eq!(local.lookup_start(*key).await.unwrap().id, legacy.id);
    }
    let mut roots = Envelope::new(Message::FsRoots);
    roots.machine = Some(binding.machine);
    let id = roots.id;
    send_frame(&client, roots).await;
    assert!(
        matches!(receive_frame(&mut client, |reply| reply.re == Some(id)).await.message,
        Message::FsRootsListed { roots } if roots == vec![allowed.to_str().unwrap().to_owned()])
    );
    for (path, permitted) in [
        (&allowed, true),
        (&outside, false),
        (&allowed.join("escape"), false),
    ] {
        let mut request = Envelope::new(Message::FsList {
            path: path.to_str().unwrap().into(),
            cursor: None,
            limit: 10,
        });
        request.machine = Some(binding.machine);
        let id = request.id;
        send_frame(&client, request).await;
        let reply = receive_frame(&mut client, |envelope| envelope.re == Some(id)).await;
        if permitted {
            assert!(
                matches!(reply.message,Message::FsListed {entries,..} if entries.iter().any(|entry| entry.name=="inside.txt"))
            );
        } else {
            assert!(
                matches!(reply.message,Message::Error(error) if matches!(error.code,qgenie_remote_proto::ErrorCode::Forbidden))
            );
        }
    }
    // A short DTO deadline exercises real WSS auth.refresh without waiting fifteen
    // minutes. The actual signed JWT is still verified by the gateway.
    tokio::time::timeout(Duration::from_secs(12), async {
        while clock.0.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let generation: String =
        sqlx::query_scalar("SELECT generation::text FROM qgr.machine_routes WHERE machine=$1")
            .bind(binding.machine.to_string())
            .fetch_one(&database.admin)
            .await
            .unwrap();
    assert_eq!(generation, "1");
    if let Some(first_nonce) = first_nonce {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let current: String = sqlx::query_scalar(
                    "SELECT native_access->>'nonce' FROM qgr.machine_routes WHERE machine=$1",
                )
                .bind(binding.machine.to_string())
                .fetch_one(&database.admin)
                .await
                .unwrap();
                if current != first_nonce {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
    let authentication = api
        .authenticate_machine(&identity.machine_credentials().unwrap())
        .await
        .unwrap();
    let replacement = socket_with_native(
        &origin,
        tls.ca.path(),
        "host",
        authentication.token.as_str(),
        native_mode,
    )
    .await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if matches!(
                *updates.borrow(),
                ConnectionStatus::Connected {
                    generation: Counter(3),
                    ..
                }
            ) {
                break;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    drop(replacement);
    let mut request = Envelope::new(Message::FsList {
        path: allowed.to_str().unwrap().into(),
        cursor: None,
        limit: 10,
    });
    request.machine = Some(binding.machine);
    let id = request.id;
    send_frame(&client, request).await;
    assert!(matches!(
        receive_frame(&mut client, |envelope| envelope.re == Some(id))
            .await
            .message,
        Message::FsListed { .. }
    ));
    if let Some(native) = &native {
        assert_eq!(native.host_floor().await.unwrap(), Counter(3));
        assert!(matches!(
            native
                .challenge(qgenie_remote_proto::ClientId::new(), Counter(2))
                .await,
            Err(qgenie_remote_host::Error::Rejected(
                qgenie_remote_proto::RemoteError {
                    code: qgenie_remote_proto::ErrorCode::StaleGeneration,
                    ..
                }
            ))
        ));
        // Simulate restoring an older authority projection while native state survives.
        // Reconnect must allocate above the durable native floor in one exchange.
        sqlx::query("UPDATE qgr.machine_routes SET generation=1,activated=false,native_access=NULL WHERE machine=$1")
            .bind(binding.machine.to_string()).execute(&database.admin).await.unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if matches!(
                    *updates.borrow(),
                    ConnectionStatus::Connected {
                        generation: Counter(4),
                        activated: true
                    }
                ) {
                    break;
                }
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(native.host_floor().await.unwrap(), Counter(4));
        let http = reqwest::Client::builder()
            .no_proxy()
            .add_root_certificate(
                reqwest::Certificate::from_pem(&std::fs::read(tls.ca.path()).unwrap()).unwrap(),
            )
            .build()
            .unwrap();
        let admin = admin_access_token();
        let request = qgenie_remote_proto::rest::RevokeAccess {
            request: qgenie_remote_proto::RequestId::new(),
            target: qgenie_remote_proto::rest::RevocationTarget::Machine {
                machine: binding.machine,
            },
            confirmed: true,
        };
        let revoked = std::time::Instant::now();
        let result: qgenie_remote_proto::rest::AccessRevocation = http
            .post(format!("{origin}/v1/admin/revocations"))
            .bearer_auth(&admin)
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
            result.propagation,
            qgenie_remote_proto::rest::RevocationPropagation::Pending
        ));
        tokio::time::timeout(Duration::from_secs(10), async {
            while !matches!(*updates.borrow(), ConnectionStatus::Denied) {
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(revoked.elapsed() < Duration::from_secs(10));
        let status = format!(
            "{}/v1/admin/revocations/{}",
            peer_server.as_ref().unwrap().origin,
            request.request
        );
        let pending: qgenie_remote_proto::rest::AccessRevocation = http
            .get(&status)
            .bearer_auth(&admin)
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
            (observed
                + Duration::from_millis(qgenie_remote_proto::MAX_ACCESS_LIFETIME_MS as u64 + 50))
            .into(),
        )
        .await;
        let completed: qgenie_remote_proto::rest::AccessRevocation = http
            .get(&status)
            .bearer_auth(&admin)
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
        assert_eq!(completed.revision, result.revision);
        let (key, session) = legacy.as_ref().unwrap();
        assert!(matches!(
            local.lookup_start(*key).await.unwrap().state,
            qgenie_remote_proto::SessionState::Running | qgenie_remote_proto::SessionState::Idle
        ));
        assert_eq!(local.lookup_start(*key).await.unwrap().id, session.id);
    }
    stop.send_replace(true);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(*updates.borrow(), ConnectionStatus::Stopped);
    if let Some(daemon) = &mut daemon {
        daemon.stop(&native_path).await;
    }
    drop(client);
    observer.close().await;
    drop(peer_server);
    if let Some(peer) = peer {
        peer.shutdown().await;
    }
    drop(server);
    gateway.shutdown().await;
    drop(issuer);
    database.destroy().await;
}

struct NativeDaemon(std::process::Child);
fn host_binary() -> std::path::PathBuf {
    std::env::var_os("QGR_HOST_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_qgenie-remote").into())
}
impl NativeDaemon {
    fn start(state: &std::path::Path, root: &std::path::Path) -> Self {
        Self(
            std::process::Command::new(host_binary())
                .arg("--state-dir")
                .arg(state)
                .arg("--root")
                .arg(root)
                .arg("sessiond")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .unwrap(),
        )
    }
    async fn ready(&mut self, client: &qgenie_remote_client::Client) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while client.status().await.is_err() {
                assert!(self.0.try_wait().unwrap().is_none());
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn stop(&mut self, state: &std::path::Path) {
        use qgenie_remote_proto::ipc::{AdminCommand, AdminRequest, AdminResponse};
        use qgenie_remote_transport::ipc::{connect, read_frame, write_frame};
        let mut stream = connect(&state.join("admin.sock")).await.unwrap();
        write_frame(
            &mut stream,
            &serde_json::to_vec(&AdminRequest {
                ipc_major: qgenie_remote_proto::IPC_MAJOR,
                command: AdminCommand::Shutdown {
                    terminate_sessions: true,
                },
            })
            .unwrap(),
        )
        .await
        .unwrap();
        qgenie_remote_proto::decode_json::<AdminResponse>(&read_frame(&mut stream).await.unwrap())
            .unwrap()
            .result
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.0.try_wait().unwrap().is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
impl Drop for NativeDaemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
