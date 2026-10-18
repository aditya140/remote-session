use axum::{
    Json, Router,
    extract::{Form, State},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use qgenie_remote_gateway::{Config, SecretDsn, Store};
use qgenie_remote_host::oauth::OAuthConfig;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Write,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{net::TcpListener, task::JoinHandle};

pub fn private(bytes: &[u8]) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(bytes).unwrap();
    file
}
pub struct Tls {
    pub ca: tempfile::NamedTempFile,
    acceptor: tokio_rustls::TlsAcceptor,
}
impl Tls {
    pub fn new() -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert.cert.der().clone()], key.into())
            .unwrap();
        Self {
            ca: private(cert.cert.pem().as_bytes()),
            acceptor: tokio_rustls::TlsAcceptor::from(Arc::new(config)),
        }
    }
    pub async fn bind(&self) -> Edge {
        Edge {
            listener: TcpListener::bind("127.0.0.1:0").await.unwrap(),
            acceptor: self.acceptor.clone(),
        }
    }
}
pub struct Edge {
    listener: TcpListener,
    acceptor: tokio_rustls::TlsAcceptor,
}
pub struct Server {
    pub origin: String,
    tasks: Vec<JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Edge {
    pub fn origin(&self) -> String {
        format!(
            "https://localhost:{}",
            self.listener.local_addr().unwrap().port()
        )
    }
    pub async fn serve(self, router: Router) -> Server {
        let origin = self.origin();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let proxy = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {result=self.listener.accept(),if tasks.len()<64=> {let (tcp,_)=result.unwrap();let acceptor=self.acceptor.clone();tasks.spawn(async move {let Ok(mut tls)=acceptor.accept(tcp).await else{return;};let mut upstream=tokio::net::TcpStream::connect(address).await.unwrap();let _=tokio::io::copy_bidirectional(&mut tls,&mut upstream).await;});},_=tasks.join_next(),if !tasks.is_empty()=>{}}
            }
        });
        Server {
            origin,
            tasks: vec![server, proxy],
        }
    }
}
pub struct Database {
    pub config: Config,
    pub admin: sqlx::PgPool,
    parent: sqlx::PgPool,
    name: String,
}
impl Database {
    pub async fn create() -> Self {
        let path =
            std::env::var_os("QGR_PG_TEST_URL_FILE").expect("set disposable QGR_PG_TEST_URL_FILE");
        SecretDsn::from_file(std::path::Path::new(&path)).unwrap();
        let mut url = url::Url::parse(std::fs::read_to_string(path).unwrap().trim()).unwrap();
        let parent = sqlx::PgPool::connect(url.as_str()).await.unwrap();
        let name = format!("qgr_host_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&parent)
            .await
            .unwrap();
        url.set_path(&name);
        let file = private(url.as_str().as_bytes());
        Store::migrate(&SecretDsn::from_file(file.path()).unwrap())
            .await
            .unwrap();
        let admin = sqlx::PgPool::connect(url.as_str()).await.unwrap();
        sqlx::raw_sql("DO $$ BEGIN IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='qgr_host_fixture_app') THEN CREATE ROLE qgr_host_fixture_app LOGIN PASSWORD 'synthetic-host-fixture'; END IF; END $$; GRANT qgr_app TO qgr_host_fixture_app;").execute(&admin).await.unwrap();
        url.set_username("qgr_host_fixture_app").unwrap();
        url.set_password(Some("synthetic-host-fixture")).unwrap();
        let file = private(url.as_str().as_bytes());
        let config = Config {
            node_id: "host-fixture".into(),
            listen: "127.0.0.1:8081".parse::<SocketAddr>().unwrap(),
            health_listen: "127.0.0.1:9091".parse().unwrap(),
            database: SecretDsn::from_file(file.path()).unwrap(),
            audit_database: None,
            pool_size: 4,
        };
        Self {
            config,
            admin,
            parent,
            name,
        }
    }
    pub async fn destroy(self) {
        self.admin.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE {} WITH (FORCE)",
            self.name
        )))
        .execute(&self.parent)
        .await
        .unwrap();
        self.parent.close().await;
    }
}
pub const ISSUER: &str = "https://login.example.invalid/11111111-1111-4111-8111-111111111111/v2.0";
pub const TENANT: &str = "11111111-1111-4111-8111-111111111111";
pub const AUDIENCE: &str = "api://remote-fixture";
pub const SCOPE: &str = "api://remote-fixture/Remote.Control";
#[derive(Clone)]
pub struct Provider {
    pub polls: Arc<AtomicUsize>,
    pub proof: Arc<Mutex<Option<(String, String)>>>,
    pub wrong_scope: Arc<AtomicUsize>,
    pub mode: Arc<AtomicUsize>,
    pub times: Arc<Mutex<Vec<tokio::time::Instant>>>,
    pub lifetime: Arc<AtomicUsize>,
}
impl Default for Provider {
    fn default() -> Self {
        Self {
            polls: Default::default(),
            proof: Default::default(),
            wrong_scope: Default::default(),
            mode: Default::default(),
            times: Default::default(),
            lifetime: Arc::new(AtomicUsize::new(30)),
        }
    }
}
impl Provider {
    pub fn router(&self) -> Router {
        Router::new()
            .route(
                "/keys",
                get(|| async {
                    axum::Json(
                        serde_json::from_str::<Value>(include_str!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/../../conformance/identity/synthetic-jwks.json"
                        )))
                        .unwrap(),
                    )
                }),
            )
            .route("/device", post(device))
            .route("/token", post(token))
            .route("/graph-token", post(graph_token))
            .route("/v1.0/users/{object}", get(graph_user))
            .with_state(self.clone())
    }
    pub fn config(&self, origin: &str, ca: PathBuf) -> OAuthConfig {
        OAuthConfig {
            client_id: "standalone-fixture".into(),
            remote_scope: SCOPE.into(),
            authorization_endpoint: format!("{origin}/authorize"),
            token_endpoint: format!("{origin}/token"),
            device_endpoint: format!("{origin}/device"),
            ca_bundle: Some(ca),
        }
    }
}
async fn graph_token(Form(form): Form<HashMap<String, String>>) -> Json<Value> {
    assert_eq!(form["grant_type"], "client_credentials");
    assert_eq!(form["client_id"], "33333333-3333-4333-8333-333333333333");
    assert_eq!(form["client_secret"], "synthetic-graph-application-secret");
    assert!(form["scope"].ends_with("/.default"));
    Json(json!({"access_token":"synthetic-graph-token","token_type":"Bearer","expires_in":300}))
}
async fn graph_user(
    axum::extract::Path(object): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> Json<Value> {
    assert_eq!(object, "22222222-2222-4222-8222-222222222222");
    assert_eq!(headers["authorization"], "Bearer synthetic-graph-token");
    Json(json!({"id":object,"accountEnabled":true}))
}
async fn device(
    State(provider): State<Provider>,
    Form(form): Form<HashMap<String, String>>,
) -> Json<Value> {
    assert_eq!(form["client_id"], "standalone-fixture");
    assert!(
        form["scope"]
            .split_ascii_whitespace()
            .any(|scope| scope == SCOPE)
    );
    Json(
        json!({"device_code":"synthetic-device-code","user_code":"FIXTURE-ONLY","verification_uri":"https://login.example.invalid/device","expires_in":provider.lifetime.load(Ordering::SeqCst),"interval":1}),
    )
}
async fn token(
    State(provider): State<Provider>,
    Form(form): Form<HashMap<String, String>>,
) -> (axum::http::StatusCode, Json<Value>) {
    assert_eq!(form["client_id"], "standalone-fixture");
    assert!(!form.contains_key("client_secret"));
    match form["grant_type"].as_str() {
        "urn:ietf:params:oauth:grant-type:device_code" => {
            provider
                .times
                .lock()
                .unwrap()
                .push(tokio::time::Instant::now());
            assert_eq!(form["device_code"], "synthetic-device-code");
            let first = provider.polls.fetch_add(1, Ordering::SeqCst) == 0;
            if provider.mode.load(Ordering::SeqCst) == 1 && first {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(json!({"error":"slow_down"})),
                );
            }
            if provider.mode.load(Ordering::SeqCst) == 3 {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(json!({"error":"authorization_declined"})),
                );
            }
            if first || provider.mode.load(Ordering::SeqCst) == 2 {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(json!({"error":"authorization_pending"})),
                );
            }
        }
        "authorization_code" => {
            assert_eq!(form["code"], "synthetic-auth-code");
            let (challenge, redirect) = provider.proof.lock().unwrap().clone().unwrap();
            assert_eq!(
                URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())),
                challenge
            );
            assert_eq!(form["redirect_uri"], redirect);
        }
        _ => panic!("unexpected grant"),
    }
    let access = fixture_access_token(false);
    (
        axum::http::StatusCode::OK,
        Json(
            json!({"access_token":access,"refresh_token":"synthetic-refresh-do-not-deploy","token_type":"Bearer","expires_in":3600,"scope":if provider.wrong_scope.load(Ordering::SeqCst)>0 {"inference"}else{SCOPE}}),
        ),
    )
}
pub fn admin_access_token() -> String {
    fixture_access_token(true)
}
fn fixture_access_token(admin: bool) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = json!({"iss":ISSUER,"aud":AUDIENCE,"tid":TENANT,"oid":"22222222-2222-4222-8222-222222222222","sub":"host-fixture-subject","iat":now,"nbf":now-1,"exp":now+3600,"scp":"Remote.Control","roles":if admin {vec!["Remote.Admin"]}else{vec![]}});
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("synthetic-rsa-v1".into());
    let key = EncodingKey::from_rsa_pem(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/identity/synthetic-rsa-private.pem"
    )))
    .unwrap();
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}
