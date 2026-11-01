//! Same-origin browser authentication. It supplies no native or admin authority.
mod config;
mod routes;
#[cfg(test)]
pub(crate) mod tests;
use crate::{Error, Gateway};
use axum::http::{HeaderMap, Method, header};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
pub use config::{BrowserConfig, BrowserService};
pub use routes::{middleware, routes};
use sha2::{Digest, Sha256};
use sqlx::Row;

const SESSION_COOKIE: &str = "__Host-qgr_session";
const LOGIN_COOKIE: &str = "__Host-qgr_login";
fn now() -> Result<i64, Error> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Configuration)?
        .as_secs() as i64)
}
fn random() -> String {
    use rand::RngCore;
    let mut bytes = [0; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let values: Vec<_> = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|p| p.trim().split_once('='))
        .filter(|(k, _)| *k == name)
        .map(|(_, v)| v)
        .collect();
    if values.len() != 1 {
        return None;
    }
    let v = values[0];
    if v.len() != 43
        || !v
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    Some(v.to_owned())
}
fn same_origin(headers: &HeaderMap, origin: &str) -> bool {
    headers.get_all(header::ORIGIN).iter().count() == 1
        && headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) == Some(origin)
}
fn csrf_valid(headers: &HeaderMap, expected: &str) -> bool {
    if headers.get_all("x-csrf-token").iter().count() != 1 {
        return false;
    }
    let Some(value) = headers.get("x-csrf-token").and_then(|v| v.to_str().ok()) else {
        return false;
    };
    use hmac::Mac;
    let Ok(mut mac) = hmac::Hmac::<Sha256>::new_from_slice(expected.as_bytes()) else {
        return false;
    };
    mac.update(value.as_bytes());
    let actual = mac.finalize().into_bytes();
    let Ok(mut check) = hmac::Hmac::<Sha256>::new_from_slice(expected.as_bytes()) else {
        return false;
    };
    check.update(expected.as_bytes());
    check.verify_slice(&actual).is_ok()
}
fn protected_path(path: &str) -> bool {
    path == "/console/auth/session"
        || path == "/console/auth/logout"
        || ((path.starts_with("/v1/console/sessions/")
            || path.starts_with("/v1/console/admin/sessions/"))
            && path.ends_with("/history"))
        || path == "/v1/me"
        || path.starts_with("/v1/me/")
        || path.starts_with("/v1/admin/")
        || path == "/v1/machines"
        || (path.starts_with("/v1/machines/")
            && (path.matches('/').count() == 3 || path.ends_with("/revoke")))
        || path.starts_with("/v1/sessions/")
        || path == "/v1/connect/client"
}
fn mutating(method: &Method, path: &str) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) || path == "/v1/connect/client"
}
pub(crate) struct Session {
    pub token: zeroize::Zeroizing<String>,
    pub csrf: String,
    pub owner: String,
    pub expires_at: i64,
}
async fn lookup(gateway: &Gateway, headers: &HeaderMap) -> Result<Session, Error> {
    let service = gateway.browser.as_ref().ok_or(Error::Unauthenticated)?;
    let id = cookie(headers, SESSION_COOKIE).ok_or(Error::Unauthenticated)?;
    let row=sqlx::query("SELECT encrypted_token,csrf,owner,expires_at FROM qgr.browser_sessions WHERE id_hash=$1 AND expires_at>$2")
        .bind(hash(&id)).bind(now()?).fetch_optional(&gateway.store.pool).await.map_err(|_|Error::Database)?.ok_or(Error::Unauthenticated)?;
    Ok(Session {
        token: zeroize::Zeroizing::new(service.open(row.get("encrypted_token"), b"browser-token")?),
        csrf: row.get("csrf"),
        owner: row.get("owner"),
        expires_at: row.get("expires_at"),
    })
}

/// Private identity binding for a browser socket. No raw token crosses this interface.
pub(crate) struct ConnectionSession {
    pub expires_at: u64,
    id_hash: String,
    owner: String,
    store: crate::Store,
}
pub(crate) async fn connection_session(
    gateway: &Gateway,
    headers: &HeaderMap,
    owner: qgenie_remote_proto::UserId,
) -> Result<Option<ConnectionSession>, Error> {
    let Some(id) = cookie(headers, SESSION_COOKIE) else {
        return Ok(None);
    };
    let service = gateway.browser.as_ref().ok_or(Error::Unauthenticated)?;
    if !same_origin(headers, &service.config.public_origin) {
        return Err(Error::Forbidden);
    }
    let value = lookup(gateway, headers).await?;
    if value.owner != owner.to_string() {
        return Err(Error::Unauthenticated);
    }
    Ok(Some(ConnectionSession {
        expires_at: value.expires_at as u64,
        id_hash: hash(&id),
        owner: value.owner,
        store: gateway.store.clone(),
    }))
}
impl ConnectionSession {
    pub async fn current(&self) -> Result<(), Error> {
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM qgr.browser_sessions WHERE id_hash=$1 AND owner=$2 AND expires_at>$3)").bind(&self.id_hash).bind(&self.owner).bind(now()?).fetch_one(&self.store.pool).await.map_err(|_|Error::Database)?;
        if valid {
            Ok(())
        } else {
            Err(Error::Unauthenticated)
        }
    }
}
