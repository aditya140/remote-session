use super::*;
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use std::time::Instant;
use std::{collections::VecDeque, sync::Mutex as StdMutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TENANT: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const CLIENT: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const OBJECT: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const SECRET: &str = "synthetic-app-secret";
const TOKEN: &str = "synthetic-graph-token";

struct Reply {
    status: u16,
    body: String,
    delay: Duration,
}
impl Reply {
    fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
            delay: Duration::ZERO,
        }
    }
}
pub(crate) struct Fixture {
    _files: tempfile::TempDir,
    pub(crate) config: DirectoryConfig,
    replies: Arc<StdMutex<VecDeque<Reply>>>,
    token_calls: Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) graph_calls: Arc<std::sync::atomic::AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    pub(crate) async fn new() -> Self {
        let files = tempfile::tempdir().unwrap();
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec!["directory-ca.invalid".into()]).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = params.self_signed(&key).unwrap();
        let leaf_key = KeyPair::generate().unwrap();
        let leaf = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])
            .unwrap()
            .signed_by(&leaf_key, &ca, &key)
            .unwrap();
        let server = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone(), ca.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).into(),
        )
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!(
            "https://127.0.0.1:{}",
            listener.local_addr().unwrap().port()
        );
        let credential = files.path().join("credential");
        std::fs::write(&credential, SECRET).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let ca_file = files.path().join("ca.pem");
        std::fs::write(&ca_file, ca.pem()).unwrap();
        let config = DirectoryConfig {
            tenant: TENANT.into(),
            client_id: CLIENT.into(),
            credential_file: credential,
            token_url: Some(format!("{base}/token")),
            graph_base: Some(format!("{base}/v1.0")),
            ca_bundle: Some(ca_file),
        };
        let replies = Arc::new(StdMutex::new(VecDeque::<Reply>::new()));
        let token_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let graph_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let state = (replies.clone(), token_calls.clone(), graph_calls.clone());
        let task = tokio::spawn(async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (tcp, _) = result.unwrap();
                        let acceptor = acceptor.clone();
                        let (replies, tokens, graphs) = state.clone();
                        children.spawn(async move {
                            let Ok(mut tls) = acceptor.accept(tcp).await else { return; };
                            let mut header = Vec::new();
                            while !header.ends_with(b"\r\n\r\n") {
                                if header.len() >= 8192 { return; }
                                let Ok(byte) = tls.read_u8().await else { return; };
                                header.push(byte);
                            }
                            let header = String::from_utf8(header).unwrap();
                            let reply = if header.starts_with("POST /token ") {
                                tokens.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                let length = header.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length: ").and_then(|v| v.parse::<usize>().ok())).unwrap();
                                assert!(length < 8192);
                                let mut body = vec![0; length];
                                tls.read_exact(&mut body).await.unwrap();
                                let values: std::collections::HashMap<_,_> = url::form_urlencoded::parse(&body).into_owned().collect();
                                assert_eq!(values["grant_type"], "client_credentials");
                                assert_eq!(values["client_secret"], SECRET);
                                assert_eq!(values["client_id"], CLIENT);
                                assert!(values["scope"].ends_with("/.default"));
                                assert!(!header.to_ascii_lowercase().contains("authorization:"));
                                Reply::new(200, format!(r#"{{"access_token":"{TOKEN}","token_type":"Bearer","expires_in":300}}"#))
                            } else {
                                graphs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                let object=if header.starts_with("GET /v1.0/users/dddddddd-dddd-4ddd-8ddd-dddddddddddd?%24select=id%2CaccountEnabled ") {"dddddddd-dddd-4ddd-8ddd-dddddddddddd"} else {
                                    assert!(header.starts_with(&format!("GET /v1.0/users/{OBJECT}?%24select=id%2CaccountEnabled ")));OBJECT
                                };
                                assert!(header.to_ascii_lowercase().contains(&format!("authorization: bearer {TOKEN}")));
                                replies.lock().unwrap().pop_front().unwrap_or_else(|| Reply::new(200, format!(r#"{{"id":"{object}","accountEnabled":true}}"#)))
                            };
                            tokio::time::sleep(reply.delay).await;
                            let _ = tls.write_all(format!("HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{}", reply.status, reply.body.len(), reply.body).as_bytes()).await;
                        });
                    },
                    result = children.join_next(), if !children.is_empty() => {
                        if let Some(result) = result { result.unwrap(); }
                    }
                }
            }
        });
        Self {
            _files: files,
            config,
            replies,
            token_calls,
            graph_calls,
            task,
        }
    }
    pub(crate) fn respond(&self, status: u16, body: impl Into<String>) {
        self.replies
            .lock()
            .unwrap()
            .push_back(Reply::new(status, body));
    }
    pub(crate) fn respond_after(&self, delay: Duration, status: u16, body: impl Into<String>) {
        self.replies.lock().unwrap().push_back(Reply {
            status,
            body: body.into(),
            delay,
        });
    }
}

#[tokio::test]
async fn verified_https_property_reads_token_cache_and_failure_evidence() {
    let fixture = Fixture::new().await;
    let mut untrusted = fixture.config.clone();
    untrusted.ca_bundle = None;
    assert!(matches!(
        GraphDirectory::connect(untrusted)
            .unwrap()
            .observe(OBJECT)
            .await,
        Err(Error::Unavailable)
    ));
    let graph = GraphDirectory::connect(fixture.config.clone()).unwrap();
    assert_eq!(graph.tenant(), TENANT);
    let before = qgenie_remote_transport::elapsed::now().unwrap();
    let enabled = graph.observe(OBJECT).await.unwrap();
    assert!(enabled.enabled && enabled.started_elapsed >= before);
    fixture.respond(
        200,
        format!(r#"{{"id":"{OBJECT}","accountEnabled":false}}"#),
    );
    assert!(!graph.observe(OBJECT).await.unwrap().enabled);
    fixture.respond(404, "{}");
    assert!(!graph.observe(OBJECT).await.unwrap().enabled);
    for (status, body) in [
        (200, format!(r#"{{"id":"{OBJECT}"}}"#)),
        (200, format!(r#"{{"id":"{CLIENT}","accountEnabled":true}}"#)),
        (
            200,
            format!(r#"{{"id":"{OBJECT}","accountEnabled":true,"accountEnabled":false}}"#),
        ),
        (200, "x".repeat(32769)),
        (403, "{}".into()),
        (429, "{}".into()),
        (503, "{}".into()),
        (302, "{}".into()),
    ] {
        fixture.respond(status, body);
        assert!(matches!(
            graph.observe(OBJECT).await,
            Err(Error::Unavailable)
        ));
    }
    use std::sync::atomic::Ordering::SeqCst;
    assert_eq!(fixture.token_calls.load(SeqCst), 1);
    fixture.respond(401, "{}");
    assert!(graph.observe(OBJECT).await.is_err());
    assert!(graph.observe(OBJECT).await.unwrap().enabled);
    assert_eq!(fixture.token_calls.load(SeqCst), 2);
    assert!(!format!("{graph:?} {:?}", fixture.config).contains(SECRET));
    assert!(matches!(
        graph.observe("not-an-oid").await,
        Err(Error::Invalid)
    ));
    assert!(!fixture.task.is_finished());
}

#[tokio::test]
async fn total_deadline_and_bounded_provider_work() {
    let fixture = Fixture::new().await;
    let graph = GraphDirectory::connect(fixture.config.clone()).unwrap();
    fixture.replies.lock().unwrap().push_back(Reply {
        status: 200,
        body: format!(r#"{{"id":"{OBJECT}","accountEnabled":true}}"#),
        delay: Duration::from_secs(6),
    });
    let started = Instant::now();
    assert!(matches!(
        graph.observe(OBJECT).await,
        Err(Error::Unavailable)
    ));
    assert!(started.elapsed() < Duration::from_millis(5500));
    let permit = graph.work.clone().acquire_many_owned(16).await.unwrap();
    assert!(matches!(
        graph.observe(OBJECT).await,
        Err(Error::Overloaded)
    ));
    drop(permit);
    assert_eq!(
        fixture
            .graph_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn configuration_and_credential_dac_are_enforced() {
    let fixture = Fixture::new().await;
    let mut config = fixture.config.clone();
    config.token_url = Some("http://localhost/token".into());
    assert!(matches!(
        GraphDirectory::connect(config),
        Err(Error::Configuration)
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = &fixture.config.credential_file;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            GraphDirectory::connect(fixture.config.clone()),
            Err(Error::Configuration)
        ));
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = fixture._files.path().join("link");
        symlink(path, &link).unwrap();
        config = fixture.config.clone();
        config.credential_file = link;
        assert!(matches!(
            GraphDirectory::connect(config),
            Err(Error::Configuration)
        ));
    }
}
