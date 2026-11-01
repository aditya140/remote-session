use super::*;
use axum::http::HeaderValue;

#[test]
fn cookie_parsing_rejects_duplicates_and_nonopaque_values() {
    let id = random();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("other=ignored; {SESSION_COOKIE}={id}")).unwrap(),
    );
    assert_eq!(cookie(&headers, SESSION_COOKIE), Some(id.clone()));
    headers.append(
        header::COOKIE,
        HeaderValue::from_str(&format!("{SESSION_COOKIE}={id}")).unwrap(),
    );
    assert_eq!(cookie(&headers, SESSION_COOKIE), None);
    headers.clear();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_static("__Host-qgr_session=../../token"),
    );
    assert!(cookie(&headers, SESSION_COOKIE).is_none());
}
#[test]
fn origin_and_csrf_require_the_exact_browser_context() {
    let token = random();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("https://console.example.test"),
    );
    headers.insert("x-csrf-token", HeaderValue::from_str(&token).unwrap());
    assert!(same_origin(&headers, "https://console.example.test"));
    assert!(csrf_valid(&headers, &token));
    assert!(!same_origin(&headers, "https://other.example.test"));
    assert!(!csrf_valid(&headers, &random()));
    headers.insert(header::ORIGIN, HeaderValue::from_static("null"));
    assert!(!same_origin(&headers, "https://console.example.test"));
    headers.remove("x-csrf-token");
    assert!(!csrf_valid(&headers, &token));
}
#[test]
fn browser_credentials_are_not_used_for_host_or_recording_ingest() {
    for path in [
        "/v1/connect/host",
        "/v1/enrollments",
        "/v1/machine-tokens",
        "/v1/recordings/chunks",
        "/v1/recordings/gaps",
    ] {
        assert!(!protected_path(path), "{path}");
    }
    for path in [
        "/v1/me",
        "/v1/admin/users",
        "/v1/me/sessions",
        "/v1/connect/client",
        "/v1/sessions/s_example/history",
    ] {
        assert!(protected_path(path), "{path}");
    }
    assert!(mutating(&Method::GET, "/v1/connect/client"));
    assert!(mutating(&Method::POST, "/v1/sessions/s_example/history"));
    assert!(!mutating(&Method::GET, "/v1/me"));
}
#[test]
fn browser_secrets_use_authenticated_encryption_and_separate_purposes() {
    use std::io::Write;
    let mut key = tempfile::NamedTempFile::new().unwrap();
    key.write_all(&[42; 32]).unwrap();
    let config = BrowserConfig {
        public_origin: "https://console.example.test".into(),
        authorization_endpoint: "https://login.example.test/authorize".into(),
        token_endpoint: "https://login.example.test/token".into(),
        client_id: "console".into(),
        scope: "openid remote".into(),
        encryption_key_file: key.path().into(),
        ca_bundle: None,
    };
    let service = BrowserService::new(config).unwrap();
    let sealed = service.seal("secret bearer", b"browser-token").unwrap();
    assert!(!sealed.windows(6).any(|s| s == b"secret"));
    assert_eq!(
        service.open(sealed.clone(), b"browser-token").unwrap(),
        "secret bearer"
    );
    assert!(service.open(sealed.clone(), b"browser-pkce").is_err());
    let mut changed = sealed;
    changed[15] ^= 1;
    assert!(service.open(changed, b"browser-token").is_err());
}

/// Real PostgreSQL contract, called by the disposable gateway authority fixture.
pub(crate) async fn database_contract(gateway: &Gateway) {
    use axum::{
        Router,
        body::Body,
        extract::State,
        http::{Request, StatusCode},
        routing::get,
    };
    use std::{io::Write, sync::Arc};
    use tower::ServiceExt;
    let owner: qgenie_remote_proto::UserId =
        sqlx::query_scalar::<_, String>("SELECT id FROM qgr.users ORDER BY id LIMIT 1")
            .fetch_one(&gateway.store.pool)
            .await
            .unwrap()
            .parse()
            .unwrap();
    let mut key = tempfile::NamedTempFile::new().unwrap();
    key.write_all(&[93; 32]).unwrap();
    let config = BrowserConfig {
        public_origin: "https://console.example.test".into(),
        authorization_endpoint: "https://login.example.test/authorize".into(),
        token_endpoint: "https://login.example.test/token".into(),
        client_id: "console".into(),
        scope: "openid remote".into(),
        encryption_key_file: key.path().into(),
        ca_bundle: None,
    };
    let mut node_a = gateway.clone();
    node_a.browser = Some(Arc::new(BrowserService::new(config.clone()).unwrap()));
    let mut node_b = gateway.clone();
    node_b.browser = Some(Arc::new(BrowserService::new(config).unwrap()));
    let id = random();
    let csrf = random();
    let service = node_a.browser.as_ref().unwrap();
    let current = now().unwrap();
    sqlx::query("INSERT INTO qgr.browser_sessions(id_hash,encrypted_token,csrf,owner,expires_at,created_at) VALUES($1,$2,$3,$4,$5,$6)").bind(hash(&id)).bind(service.seal("server-held",b"browser-token").unwrap()).bind(&csrf).bind(owner.to_string()).bind(current+300).bind(current).execute(&gateway.store.pool).await.unwrap();
    async fn authorized(headers: HeaderMap) -> StatusCode {
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            == Some("Bearer server-held")
        {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::UNAUTHORIZED
        }
    }
    let router = Router::new()
        .route("/v1/me", get(authorized).post(authorized))
        .route("/v1/connect/client", get(authorized))
        .route("/v1/connect/host", get(authorized))
        .layer(axum::middleware::from_fn_with_state(
            node_b.clone(),
            middleware,
        ));
    let cookie = format!("{SESSION_COOKIE}={id}");
    let request = |method: &str, path: &str, origin: Option<&str>, token: Option<&str>| {
        let mut r = Request::builder()
            .method(method)
            .uri(path)
            .header(header::COOKIE, &cookie);
        if let Some(v) = origin {
            r = r.header(header::ORIGIN, v)
        }
        if let Some(v) = token {
            r = r.header("x-csrf-token", v)
        }
        r.body(Body::empty()).unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(request("GET", "/v1/me", None, None))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT,
        "a separate gateway must decrypt the shared PostgreSQL session"
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                "POST",
                "/v1/me",
                Some("https://other.example.test"),
                Some(&csrf)
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                "POST",
                "/v1/me",
                Some("https://console.example.test"),
                None
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                "POST",
                "/v1/me",
                Some("https://console.example.test"),
                Some(&csrf)
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                "GET",
                "/v1/connect/client",
                Some("https://other.example.test"),
                None
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                "GET",
                "/v1/connect/client",
                Some("https://console.example.test"),
                None
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                "GET",
                "/v1/connect/host",
                Some("https://console.example.test"),
                None
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("DELETE FROM qgr.browser_sessions WHERE id_hash=$1")
        .bind(hash(&id))
        .execute(&gateway.store.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO qgr.browser_sessions(id_hash,encrypted_token,csrf,owner,expires_at,created_at) VALUES($1,$2,$3,$4,$5,$6)").bind(hash(&id)).bind(service.seal("server-held",b"browser-token").unwrap()).bind(&csrf).bind(owner.to_string()).bind(current-1).bind(current-300).execute(&gateway.store.pool).await.unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(request("GET", "/v1/me", None, None))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let state = random();
    let browser = random();
    sqlx::query("INSERT INTO qgr.browser_login_states(state_hash,browser_hash,encrypted_verifier,expires_at) VALUES($1,$2,$3,$4)").bind(hash(&state)).bind(hash(&browser)).bind(service.seal(&random(),b"browser-pkce").unwrap()).bind(current+300).execute(&gateway.store.pool).await.unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("{LOGIN_COOKIE}={browser}")).unwrap(),
    );
    // Missing/wrong browser binding must not consume another browser's login state.
    assert!(
        routes::callback(
            State(node_b.clone()),
            HeaderMap::new(),
            axum::extract::Query(routes::Callback {
                code: Some("code".into()),
                state: Some(state.clone())
            })
        )
        .await
        .is_err()
    );
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM qgr.browser_login_states WHERE state_hash=$1")
            .bind(hash(&state))
            .fetch_one(&gateway.store.pool)
            .await
            .unwrap();
    assert_eq!(remaining, 1);
    // Consume the one-time state without making an external token request.
    let take = || {
        sqlx::query("DELETE FROM qgr.browser_login_states WHERE state_hash=$1 AND browser_hash=$2 AND expires_at>$3 RETURNING encrypted_verifier").bind(hash(&state)).bind(hash(&browser)).bind(current).fetch_optional(&gateway.store.pool)
    };
    assert!(take().await.unwrap().is_some());
    assert!(take().await.unwrap().is_none());
    sqlx::query("DELETE FROM qgr.browser_sessions WHERE id_hash=$1")
        .bind(hash(&id))
        .execute(&gateway.store.pool)
        .await
        .unwrap();
}
