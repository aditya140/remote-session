//! Development-only synthetic identity. This binary is never included in the gateway image.
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Form, Path as WebPath, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use clap::Parser;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const TENANT: &str = "11111111-1111-4111-8111-111111111111";
const OID: &str = "22222222-2222-4222-8222-222222222222";
const CLIENT: &str = "qgr-development-fixture-client";
const AUDIENCE: &str = "api://qgr-development-fixture";
const SCOPE: &str = "api://qgr-development-fixture/Remote.Control";
#[derive(Parser)]
struct Args {
    /// Required isolation acknowledgment. No actual identity provider or user authentication exists.
    #[arg(long)]
    fixture_only: bool,
    #[arg(long, default_value = "0.0.0.0:8080")]
    listen: std::net::SocketAddr,
    #[arg(long)]
    public_origin: String,
    #[arg(long)]
    graph_secret_file: PathBuf,
}
struct Login {
    deadline: Instant,
    challenge: Option<String>,
    redirect: Option<String>,
    polls: u8,
}
struct App {
    origin: String,
    graph_secret: String,
    logins: Mutex<HashMap<String, Login>>,
}
fn random() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn failure() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":"invalid_development_fixture_request"})),
    )
        .into_response()
}
fn token(app: &App) -> Value {
    let issued = now();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("synthetic-rsa-v1".into());
    let key = EncodingKey::from_rsa_pem(include_bytes!(
        "../../../../conformance/identity/synthetic-rsa-private.pem"
    ))
    .expect("checked synthetic test key");
    let token=jsonwebtoken::encode(&header,&json!({"iss":format!("{}/{TENANT}/v2.0",app.origin),"aud":AUDIENCE,"tid":TENANT,"oid":OID,"sub":"qgr-development-fixture-subject","iat":issued,"nbf":issued.saturating_sub(1),"exp":issued+900,"scp":"Remote.Control"}),&key).expect("synthetic JWT");
    json!({"access_token":token,"token_type":"Bearer","expires_in":900,"scope":SCOPE})
}
async fn keys() -> Json<Value> {
    Json(
        serde_json::from_slice(include_bytes!(
            "../../../../conformance/identity/synthetic-jwks.json"
        ))
        .expect("checked synthetic JWK"),
    )
}
async fn device(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if form.get("client_id").map(String::as_str) != Some(CLIENT)
        || !form
            .get("scope")
            .is_some_and(|scope| scope.split_ascii_whitespace().any(|scope| scope == SCOPE))
    {
        return failure();
    }
    let mut logins = app.logins.lock().unwrap();
    logins.retain(|_, login| login.deadline > Instant::now());
    if logins.len() >= 128 {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let code = random();
    logins.insert(
        code.clone(),
        Login {
            deadline: Instant::now() + Duration::from_secs(120),
            challenge: None,
            redirect: None,
            polls: 0,
        },
    );
    Json(json!({"device_code":code,"user_code":"DEVELOPMENT-ONLY","verification_uri":format!("{}/fixture",app.origin),"expires_in":120,"interval":1})).into_response()
}
async fn authorize(
    State(app): State<Arc<App>>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if query.get("client_id").map(String::as_str) != Some(CLIENT)
        || query.get("code_challenge_method").map(String::as_str) != Some("S256")
        || !query
            .get("scope")
            .is_some_and(|scope| scope.split_ascii_whitespace().any(|scope| scope == SCOPE))
    {
        return failure();
    }
    let Some(redirect) = query.get("redirect_uri") else {
        return failure();
    };
    let Ok(mut url) = url::Url::parse(redirect) else {
        return failure();
    };
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/oauth/callback"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return failure();
    }
    let (Some(state), Some(challenge)) = (query.get("state"), query.get("code_challenge")) else {
        return failure();
    };
    if state.len() > 128 || !(16..=128).contains(&state.len()) || challenge.len() != 43 {
        return failure();
    }
    let mut logins = app.logins.lock().unwrap();
    logins.retain(|_, login| login.deadline > Instant::now());
    if logins.len() >= 128 {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let code = random();
    logins.insert(
        code.clone(),
        Login {
            deadline: Instant::now() + Duration::from_secs(120),
            challenge: Some(challenge.clone()),
            redirect: Some(redirect.clone()),
            polls: 0,
        },
    );
    url.query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", state);
    Redirect::temporary(url.as_str()).into_response()
}
async fn exchange(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if form.get("grant_type").map(String::as_str) == Some("client_credentials") {
        if form.get("client_id").map(String::as_str) != Some("33333333-3333-4333-8333-333333333333")
            || form.get("client_secret") != Some(&app.graph_secret)
            || form.get("scope").map(String::as_str) != Some("https://edge:9443/.default")
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        return Json(json!({"access_token":"synthetic-development-graph-token","token_type":"Bearer","expires_in":300})).into_response();
    }
    if form.get("client_id").map(String::as_str) != Some(CLIENT) {
        return failure();
    }
    // Test tools may explicitly request a current owner token; it is still a fixed
    // synthetic identity, scoped to this distinct development issuer/audience.
    if form.get("grant_type").map(String::as_str) == Some("urn:qgenie:development-fixture") {
        return Json(token(&app)).into_response();
    }
    let device = form.get("grant_type").map(String::as_str)
        == Some("urn:ietf:params:oauth:grant-type:device_code");
    if !device && form.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return failure();
    }
    let Some(code) = form.get(if device { "device_code" } else { "code" }) else {
        return failure();
    };
    let mut logins = app.logins.lock().unwrap();
    let Some(login) = logins.get_mut(code) else {
        return failure();
    };
    if Instant::now() >= login.deadline {
        return failure();
    }
    if device {
        if login.challenge.is_some() {
            return failure();
        }
        login.polls += 1;
        if login.polls == 1 {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"authorization_pending"})),
            )
                .into_response();
        }
    } else {
        let Some(verifier) = form.get("code_verifier") else {
            return failure();
        };
        if login.challenge.as_deref()
            != Some(&URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())))
            || form.get("redirect_uri") != login.redirect.as_ref()
        {
            return failure();
        }
    }
    logins.remove(code);
    Json(token(&app)).into_response()
}
async fn graph(WebPath(oid): WebPath<String>, headers: HeaderMap) -> Response {
    if headers.get("authorization").and_then(|h| h.to_str().ok())
        != Some("Bearer synthetic-development-graph-token")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if oid != OID {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(json!({"id":OID,"accountEnabled":true})).into_response()
}
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.fixture_only,
        "this separate test binary requires --fixture-only and must never serve real identities"
    );
    let origin = url::Url::parse(&args.public_origin)?;
    ensure!(
        origin.scheme() == "https"
            && origin.origin().ascii_serialization() == args.public_origin
            && origin.host_str() == Some("localhost"),
        "fixture origin must be an explicit HTTPS localhost development endpoint"
    );
    let secret = std::fs::read_to_string(args.graph_secret_file)
        .context("unreadable synthetic application secret")?;
    ensure!(
        (32..=128).contains(&secret.trim().len()),
        "invalid synthetic application secret"
    );
    let app = Arc::new(App {
        origin: args.public_origin,
        graph_secret: secret.trim().into(),
        logins: Mutex::new(HashMap::new()),
    });
    let router=Router::new().route("/live",get(||async{"development fixture only"})).route("/fixture",get(||async{"Synthetic development fixture: no actual sign-in, users or tenant. Do not use production credentials."})).route("/keys",get(keys)).route("/authorize",get(authorize)).route("/device",post(device)).route("/token",post(exchange)).route("/v1.0/users/{oid}",get(graph)).layer(DefaultBodyLimit::max(65536)).layer(tower::limit::ConcurrencyLimitLayer::new(32)).with_state(app);
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    eprintln!(
        "Synthetic development identity fixture running; never use this binary with real identities."
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let mut signal =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("signal");
            tokio::select! {_=tokio::signal::ctrl_c()=>{},_=signal.recv()=>{}}
        })
        .await?;
    Ok(())
}
