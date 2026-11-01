use super::*;
use axum::{
    Json, Router,
    extract::{Query, Request, State},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;

pub fn routes() -> Router<Gateway> {
    Router::new()
        .route("/console/auth/login", get(login))
        .route("/console/auth/callback", get(callback))
        .route("/console/auth/session", get(session))
        .route("/console/auth/logout", post(logout))
}
fn private(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}
fn set_cookie(response: &mut Response, name: &str, value: &str, seconds: i64) -> Result<(), Error> {
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{name}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={seconds}"
        ))
        .map_err(|_| Error::Configuration)?,
    );
    Ok(())
}
/// Installed on the public application router. No cookie authentication on host APIs.
pub async fn middleware(
    State(gateway): State<Gateway>,
    mut request: Request,
    next: Next,
) -> Response {
    if !protected_path(request.uri().path())
        || request.headers().contains_key(header::AUTHORIZATION)
        || cookie(request.headers(), SESSION_COOKIE).is_none()
    {
        return private(next.run(request).await);
    }
    let result = async {
        let service = gateway.browser.as_ref().ok_or(Error::Unauthenticated)?;
        let value = lookup(&gateway, request.headers()).await?;
        let path = request.uri().path();
        if mutating(request.method(), path)
            && !same_origin(request.headers(), &service.config.public_origin)
        {
            return Err(Error::Forbidden);
        }
        if !matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS
        ) && !csrf_valid(request.headers(), &value.csrf)
        {
            return Err(Error::Forbidden);
        }
        let mut header = HeaderValue::from_str(&format!("Bearer {}", value.token.as_str()))
            .map_err(|_| Error::Unauthenticated)?;
        header.set_sensitive(true);
        request.headers_mut().insert(header::AUTHORIZATION, header);
        Ok::<_, Error>(())
    }
    .await;
    match result {
        Ok(()) => private(next.run(request).await),
        Err(error) => private(error.into_response()),
    }
}
async fn login(State(gateway): State<Gateway>) -> Result<Response, Error> {
    let service = gateway.browser.as_ref().ok_or(Error::NotFound)?;
    let current = now()?;
    let state = random();
    let browser = random();
    let verifier = random();
    let cipher = service.seal(&verifier, b"browser-pkce")?;
    let mut tx = gateway.store.transaction().await?;
    // Serialize bounded capacity checks across gateway processes; no I/O under a Rust lock.
    sqlx::query("SELECT pg_advisory_xact_lock(717412559)")
        .execute(&mut *tx)
        .await
        .map_err(|_| Error::Database)?;
    sqlx::query("DELETE FROM qgr.browser_login_states WHERE state_hash IN (SELECT state_hash FROM qgr.browser_login_states WHERE expires_at<=$1 LIMIT 1024)").bind(current).execute(&mut *tx).await.map_err(|_|Error::Database)?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr.browser_login_states")
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| Error::Database)?;
    if count >= 4096 {
        return Err(Error::Overloaded);
    }
    sqlx::query("INSERT INTO qgr.browser_login_states(state_hash,browser_hash,encrypted_verifier,expires_at) VALUES($1,$2,$3,$4)").bind(hash(&state)).bind(hash(&browser)).bind(cipher).bind(current+300).execute(&mut *tx).await.map_err(|_|Error::Database)?;
    tx.commit().await?;
    let mut url = url::Url::parse(&service.config.authorization_endpoint)
        .map_err(|_| Error::Configuration)?;
    url.query_pairs_mut()
        .append_pair("client_id", &service.config.client_id)
        .append_pair("response_type", "code")
        .append_pair(
            "redirect_uri",
            &format!("{}/console/auth/callback", service.config.public_origin),
        )
        .append_pair("scope", &service.config.scope)
        .append_pair("state", &state)
        .append_pair(
            "code_challenge",
            &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        )
        .append_pair("code_challenge_method", "S256");
    let mut response = private(Redirect::to(url.as_str()).into_response());
    set_cookie(&mut response, LOGIN_COOKIE, &browser, 300)?;
    Ok(response)
}
#[derive(Deserialize)]
pub(super) struct Callback {
    #[serde(default)]
    pub(super) code: Option<String>,
    #[serde(default)]
    pub(super) state: Option<String>,
}
#[derive(Deserialize)]
struct Token {
    access_token: String,
}
pub(super) async fn callback(
    State(gateway): State<Gateway>,
    headers: HeaderMap,
    Query(query): Query<Callback>,
) -> Result<Response, Error> {
    let service = gateway.browser.as_ref().ok_or(Error::NotFound)?;
    let code = query
        .code
        .filter(|v| !v.is_empty() && v.len() <= 4096)
        .ok_or(Error::Unauthenticated)?;
    let state = query
        .state
        .filter(|v| {
            v.len() == 43
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
        .ok_or(Error::Unauthenticated)?;
    let browser = cookie(&headers, LOGIN_COOKIE).ok_or(Error::Unauthenticated)?;
    // DELETE RETURNING makes successful state consumption atomic across all nodes.
    let row=sqlx::query("DELETE FROM qgr.browser_login_states WHERE state_hash=$1 AND browser_hash=$2 AND expires_at>$3 RETURNING encrypted_verifier")
        .bind(hash(&state)).bind(hash(&browser)).bind(now()?).fetch_optional(&gateway.store.pool).await.map_err(|_|Error::Database)?.ok_or(Error::Unauthenticated)?;
    let verifier =
        zeroize::Zeroizing::new(service.open(row.get("encrypted_verifier"), b"browser-pkce")?);
    let redirect = format!("{}/console/auth/callback", service.config.public_origin);
    let mut received = service
        .http
        .post(&service.config.token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", service.config.client_id.as_str()),
            ("code", code.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|_| Error::Unavailable)?;
    if !received.status().is_success() {
        return Err(Error::Unauthenticated);
    }
    let mut body = zeroize::Zeroizing::new(Vec::new());
    while let Some(chunk) = received.chunk().await.map_err(|_| Error::Unavailable)? {
        if body.len() + chunk.len() > 65536 {
            return Err(Error::Unauthenticated);
        }
        body.extend_from_slice(&chunk);
    }
    let token: Token = serde_json::from_slice(&body).map_err(|_| Error::Unauthenticated)?;
    let token = zeroize::Zeroizing::new(token.access_token);
    if token.is_empty() || token.len() > 32768 || token.chars().any(char::is_control) {
        return Err(Error::Unauthenticated);
    }
    let mut bearer = HeaderMap::new();
    bearer.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", token.as_str()))
            .map_err(|_| Error::Unauthenticated)?,
    );
    let actor = gateway.principal(&bearer).await?;
    let current = now()?;
    let expires = (actor.expires_at.min(i64::MAX as u64) as i64).min(current + 3600);
    if expires <= current {
        return Err(Error::Unauthenticated);
    }
    let id = random();
    let csrf = random();
    let encrypted = service.seal(&token, b"browser-token")?;
    let mut tx = gateway.store.transaction().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(717412560)")
        .execute(&mut *tx)
        .await
        .map_err(|_| Error::Database)?;
    sqlx::query("DELETE FROM qgr.browser_sessions WHERE id_hash IN (SELECT id_hash FROM qgr.browser_sessions WHERE expires_at<=$1 LIMIT 1024)").bind(current).execute(&mut *tx).await.map_err(|_|Error::Database)?;
    if let Some(old) = cookie(&headers, SESSION_COOKIE) {
        sqlx::query("DELETE FROM qgr.browser_sessions WHERE id_hash=$1")
            .bind(hash(&old))
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::Database)?;
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr.browser_sessions")
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| Error::Database)?;
    if count >= 16384 {
        return Err(Error::Overloaded);
    }
    sqlx::query("INSERT INTO qgr.browser_sessions(id_hash,encrypted_token,csrf,owner,expires_at,created_at) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(hash(&id)).bind(encrypted).bind(csrf).bind(actor.user.to_string()).bind(expires).bind(current).execute(&mut *tx).await.map_err(|_|Error::Database)?;
    tx.commit().await?;
    let mut response = private(Redirect::to("/console/").into_response());
    set_cookie(&mut response, SESSION_COOKIE, &id, expires - current)?;
    set_cookie(&mut response, LOGIN_COOKIE, "", 0)?;
    Ok(response)
}
async fn session(State(gateway): State<Gateway>, headers: HeaderMap) -> Result<Response, Error> {
    if cookie(&headers, SESSION_COOKIE).is_none() {
        return Ok(private(
            Json(serde_json::json!({"authenticated":false})).into_response(),
        ));
    }
    let value = match lookup(&gateway, &headers).await {
        Ok(value) => value,
        Err(Error::Unauthenticated) => {
            return Ok(private(
                Json(serde_json::json!({"authenticated":false})).into_response(),
            ));
        }
        Err(e) => return Err(e),
    };
    let actor = gateway.principal(&headers).await?;
    if actor.user.to_string() != value.owner {
        return Err(Error::Unauthenticated);
    }
    Ok(private(Json(serde_json::json!({"authenticated":true,"user":actor.user,"platform_admin":actor.platform_admin,"csrf_token":value.csrf,"expires_at":value.expires_at})).into_response()))
}
async fn logout(State(gateway): State<Gateway>, headers: HeaderMap) -> Result<Response, Error> {
    let service = gateway.browser.as_ref().ok_or(Error::Unauthenticated)?;
    let value = lookup(&gateway, &headers).await?;
    if !same_origin(&headers, &service.config.public_origin) || !csrf_valid(&headers, &value.csrf) {
        return Err(Error::Forbidden);
    }
    let id = cookie(&headers, SESSION_COOKIE).ok_or(Error::Unauthenticated)?;
    sqlx::query("DELETE FROM qgr.browser_sessions WHERE id_hash=$1")
        .bind(hash(&id))
        .execute(&gateway.store.pool)
        .await
        .map_err(|_| Error::Database)?;
    let mut response = private(StatusCode::NO_CONTENT.into_response());
    set_cookie(&mut response, SESSION_COOKIE, "", 0)?;
    Ok(response)
}
