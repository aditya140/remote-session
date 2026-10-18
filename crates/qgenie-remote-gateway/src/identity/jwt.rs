use crate::{Authenticator, Error, Principal, Store};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{AlgorithmParameters, JwkSet, KeyAlgorithm, PublicKeyUse},
};
use qgenie_remote_proto::UserId;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

#[derive(Clone, Serialize, Deserialize)]
pub struct JwtConfig {
    pub issuer: String,
    pub tenant: String,
    pub audience: String,
    pub remote_scope: String,
    pub admin_role: String,
    pub jwks_url: String,
    #[serde(default)]
    pub ca_bundle: Option<PathBuf>,
}
impl std::fmt::Debug for JwtConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JwtConfig([configured identity endpoints])")
    }
}
impl JwtConfig {
    pub fn from_file(path: &std::path::Path) -> Result<Self, Error> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| Error::Configuration)?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Configuration)?;
        if bytes.len() > 65536 {
            return Err(Error::Configuration);
        }
        let config: Self =
            qgenie_remote_proto::decode_json(&bytes).map_err(|_| Error::Configuration)?;
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<(), Error> {
        for url in [&self.issuer, &self.jwks_url] {
            let parsed = url::Url::parse(url).map_err(|_| Error::Configuration)?;
            if parsed.scheme() != "https"
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || url.len() > 2048
            {
                return Err(Error::Configuration);
            }
        }
        if self.tenant.is_empty()
            || self.tenant.len() > 64
            || self.audience.is_empty()
            || self.audience.len() > 256
            || self.remote_scope.is_empty()
            || self.remote_scope.len() > 128
            || self.remote_scope.chars().any(char::is_whitespace)
            || self.admin_role.is_empty()
            || self.admin_role.len() > 128
        {
            return Err(Error::Configuration);
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize)]
struct UserClaims {
    sub: String,
    oid: String,
    tid: String,
    scp: String,
    #[serde(default)]
    roles: Vec<String>,
    iat: u64,
    exp: u64,
}
struct Keys {
    entries: Vec<(String, DecodingKey)>,
    expires: Duration,
    refresh_at: Duration,
    attempt_after: Duration,
}
impl Keys {
    fn new(set: JwkSet) -> Result<Self, Error> {
        if set.keys.is_empty() || set.keys.len() > 32 {
            return Err(Error::Configuration);
        }
        let mut entries = Vec::new();
        for jwk in set.keys {
            let Some(kid) = &jwk.common.key_id else {
                continue;
            };
            if kid.is_empty() || kid.len() > 128 || entries.iter().any(|(known, _)| known == kid) {
                return Err(Error::Configuration);
            }
            if jwk
                .common
                .public_key_use
                .as_ref()
                .is_some_and(|usage| usage != &PublicKeyUse::Signature)
                || jwk
                    .common
                    .key_algorithm
                    .is_some_and(|algorithm| algorithm != KeyAlgorithm::RS256)
            {
                continue;
            }
            let AlgorithmParameters::RSA(parameters) = &jwk.algorithm else {
                continue;
            };
            let modulus = URL_SAFE_NO_PAD
                .decode(&parameters.n)
                .map_err(|_| Error::Configuration)?;
            let exponent = URL_SAFE_NO_PAD
                .decode(&parameters.e)
                .map_err(|_| Error::Configuration)?;
            if !(256..=1024).contains(&modulus.len()) || exponent.is_empty() || exponent.len() > 8 {
                return Err(Error::Configuration);
            }
            entries.push((
                kid.clone(),
                DecodingKey::from_jwk(&jwk).map_err(|_| Error::Configuration)?,
            ));
        }
        if entries.is_empty() {
            return Err(Error::Configuration);
        }
        let started = elapsed()?;
        Ok(Self {
            entries,
            expires: started + Duration::from_secs(600),
            refresh_at: started + Duration::from_secs(300),
            attempt_after: started + Duration::from_secs(60),
        })
    }
}
pub struct JwtVerifier {
    config: JwtConfig,
    keys: Mutex<Keys>,
    http: reqwest::Client,
    store: Store,
}
impl JwtVerifier {
    pub async fn connect(config: JwtConfig, store: Store) -> Result<Self, Error> {
        config.validate()?;
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none());
        if let Some(path) = &config.ca_bundle {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(|_| Error::Configuration)?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| Error::Configuration)?;
            if bytes.len() > 1024 * 1024 {
                return Err(Error::Configuration);
            }
            for certificate in
                reqwest::Certificate::from_pem_bundle(&bytes).map_err(|_| Error::Configuration)?
            {
                builder = builder.add_root_certificate(certificate);
            }
        }
        let http = builder.build().map_err(|_| Error::Configuration)?;
        let keys = fetch_keys(&http, &config.jwks_url).await?;
        Ok(Self {
            config,
            keys: Mutex::new(keys),
            http,
            store,
        })
    }
    #[cfg(test)]
    pub(crate) fn with_keys(config: JwtConfig, set: JwkSet, store: Store) -> Result<Self, Error> {
        config.validate()?;
        Ok(Self {
            config,
            keys: Mutex::new(Keys::new(set)?),
            http: reqwest::Client::new(),
            store,
        })
    }
    async fn claims(&self, token: &str) -> Result<UserClaims, Error> {
        unique_token(token)?;
        let header = decode_header(token).map_err(|_| Error::Unauthenticated)?;
        if header.alg != Algorithm::RS256
            || header
                .typ
                .as_deref()
                .is_some_and(|typ| !["JWT", "at+jwt"].contains(&typ))
        {
            return Err(Error::Unauthenticated);
        }
        let kid = header.kid.ok_or(Error::Unauthenticated)?;
        if kid.len() > 128 {
            return Err(Error::Unauthenticated);
        }
        let mut keys = self.keys.lock().await;
        let known = keys.entries.iter().any(|(known, _)| *known == kid);
        let now = elapsed()?;
        if now >= keys.attempt_after && (now >= keys.refresh_at || !known) {
            keys.attempt_after = now + Duration::from_secs(60);
            if let Ok(fresh) = fetch_keys(&self.http, &self.config.jwks_url).await {
                *keys = fresh;
            }
        }
        if elapsed()? >= keys.expires {
            return Err(Error::Unauthenticated);
        }
        let key = keys
            .entries
            .iter()
            .find(|(known, _)| *known == kid)
            .map(|(_, key)| key.clone())
            .ok_or(Error::Unauthenticated)?;
        drop(keys);
        let claims = validate_claims(&self.config, &key, token)?;
        Ok(claims)
    }
}
fn validate_claims(
    config: &JwtConfig,
    key: &DecodingKey,
    token: &str,
) -> Result<UserClaims, Error> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[&config.issuer]);
    validation.set_audience(&[&config.audience]);
    validation.validate_nbf = true;
    validation.leeway = 30;
    validation.set_required_spec_claims(&["iss", "aud", "exp", "nbf", "iat", "sub"]);
    let claims = decode::<UserClaims>(token, key, &validation)
        .map_err(|_| Error::Unauthenticated)?
        .claims;
    let now = unix_seconds()?;
    if sqlx::types::Uuid::parse_str(&claims.oid)
        .map_or(true, |oid| oid.hyphenated().to_string() != claims.oid)
        || claims.tid != config.tenant
        || claims.sub.is_empty()
        || claims.sub.len() > 256
        || claims.roles.len() > 64
        || !claims
            .scp
            .split_ascii_whitespace()
            .any(|scope| scope == config.remote_scope)
        || claims.iat > now.saturating_add(30)
        || claims.exp <= claims.iat
        || claims.exp - claims.iat > 24 * 60 * 60
    {
        return Err(Error::Unauthenticated);
    }
    Ok(claims)
}
impl Authenticator for JwtVerifier {
    fn authenticate<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, Error>> + Send + 'a>> {
        Box::pin(async move {
            let claims = self.claims(token).await?;
            let oid =
                sqlx::types::Uuid::parse_str(&claims.oid).map_err(|_| Error::Unauthenticated)?;
            let mut transaction = self.store.transaction().await?;
            let mut row=sqlx::query("SELECT id,disabled FROM qgr.users WHERE issuer=$1 AND tenant=$2 AND directory_object=$3").bind(&self.config.issuer).bind(&self.config.tenant).bind(oid).fetch_optional(&mut *transaction).await.map_err(|_| Error::Database)?;
            if row.is_none() {
                row=sqlx::query("UPDATE qgr.users SET directory_object=$4 WHERE issuer=$1 AND tenant=$2 AND subject=$3 AND directory_object IS NULL RETURNING id,disabled").bind(&self.config.issuer).bind(&self.config.tenant).bind(&claims.sub).bind(oid).fetch_optional(&mut *transaction).await.map_err(|_| Error::Database)?;
            }
            if row.is_none() {
                let unmapped:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM qgr.users WHERE issuer=$1 AND tenant=$2 AND directory_object IS NULL)").bind(&self.config.issuer).bind(&self.config.tenant).fetch_one(&mut *transaction).await.map_err(|_| Error::Database)?;
                if unmapped {
                    return Err(Error::Schema);
                }
                sqlx::query("INSERT INTO qgr.users(id,issuer,tenant,subject,directory_object) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING").bind(UserId::new().to_string()).bind(&self.config.issuer).bind(&self.config.tenant).bind(&claims.sub).bind(oid).execute(&mut *transaction).await.map_err(|_| Error::Database)?;
                row=sqlx::query("SELECT id,disabled FROM qgr.users WHERE issuer=$1 AND tenant=$2 AND directory_object=$3").bind(&self.config.issuer).bind(&self.config.tenant).bind(oid).fetch_optional(&mut *transaction).await.map_err(|_| Error::Database)?;
            }
            let row = row.ok_or(Error::Forbidden)?;
            transaction.commit().await?;
            if row.get::<bool, _>("disabled") {
                return Err(Error::Forbidden);
            }
            Ok(Principal {
                user: row
                    .get::<String, _>("id")
                    .parse()
                    .map_err(|_| Error::Database)?,
                platform_admin: claims.roles.contains(&self.config.admin_role),
                expires_at: claims.exp,
            })
        })
    }
}
fn elapsed() -> Result<Duration, Error> {
    qgenie_remote_transport::elapsed::now().map_err(|_| Error::Unauthenticated)
}
async fn fetch_keys(http: &reqwest::Client, url: &str) -> Result<Keys, Error> {
    let started = elapsed()?;
    let set = fetch(http, url).await?;
    if elapsed()?
        .checked_sub(started)
        .is_none_or(|age| age > Duration::from_secs(5))
    {
        return Err(Error::Unauthenticated);
    }
    let mut keys = Keys::new(set)?;
    keys.expires = started + Duration::from_secs(600);
    keys.refresh_at = started + Duration::from_secs(300);
    keys.attempt_after = started + Duration::from_secs(60);
    Ok(keys)
}
async fn fetch(http: &reqwest::Client, url: &str) -> Result<JwkSet, Error> {
    let mut response = http
        .get(url)
        .send()
        .await
        .map_err(|_| Error::Unauthenticated)?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > 256 * 1024)
    {
        return Err(Error::Unauthenticated);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unauthenticated)? {
        if bytes.len() + chunk.len() > 256 * 1024 {
            return Err(Error::Unauthenticated);
        }
        bytes.extend_from_slice(&chunk);
    }
    qgenie_remote_proto::decode_json(&bytes).map_err(|_| Error::Unauthenticated)
}
pub(super) fn unique_token(token: &str) -> Result<(), Error> {
    if token.len() > 16384 {
        return Err(Error::Unauthenticated);
    }
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(Error::Unauthenticated);
    }
    for part in &parts[..2] {
        let bytes = URL_SAFE_NO_PAD
            .decode(part)
            .map_err(|_| Error::Unauthenticated)?;
        let _: serde_json::Value =
            qgenie_remote_proto::decode_json(&bytes).map_err(|_| Error::Unauthenticated)?;
    }
    Ok(())
}
pub(crate) fn unix_seconds() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| Error::Database)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde_json::{Value, json};
    pub(crate) fn config() -> JwtConfig {
        JwtConfig {
            issuer: "https://identity.example.invalid/tenant/v2.0".into(),
            tenant: "synthetic-tenant".into(),
            audience: "api://qgenie-remote-test".into(),
            remote_scope: "Remote.Control".into(),
            admin_role: "Remote.PlatformAdmin".into(),
            jwks_url: "https://identity.example.invalid/tenant/keys".into(),
            ca_bundle: None,
        }
    }
    pub(crate) fn keys() -> JwkSet {
        serde_json::from_str(include_str!(
            "../../../../conformance/identity/synthetic-jwks.json"
        ))
        .unwrap()
    }
    pub(crate) fn claims(subject: &str) -> Value {
        let now = unix_seconds().unwrap();
        let config = config();
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(subject.as_bytes());
        let oid = sqlx::types::Uuid::from_bytes(digest[..16].try_into().unwrap()).to_string();
        json!({"oid":oid,"iss":config.issuer,"aud":config.audience,"tid":config.tenant,"sub":subject,"scp":"Remote.Control","iat":now,"nbf":now-1,"exp":now+300,"roles":[]})
    }
    pub(crate) fn sign(claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("synthetic-rsa-v1".into());
        encode(
            &header,
            claims,
            &EncodingKey::from_rsa_pem(include_bytes!(
                "../../../../conformance/identity/synthetic-rsa-private.pem"
            ))
            .unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn dedicated_remote_claims_and_keys_refuse_inference_and_forged_tokens() {
        let config = config();
        config.validate().unwrap();
        let set = keys();
        let key = DecodingKey::from_jwk(&set.keys[0]).unwrap();
        let valid = claims("owner-a");
        validate_claims(&config, &key, &sign(&valid)).unwrap();
        let now = unix_seconds().unwrap();
        for (name, value) in [
            ("aud", json!("api://inference")),
            ("iss", json!("https://other.invalid/")),
            ("tid", json!("foreign-tenant")),
            ("scp", json!("Inference.Run")),
            ("exp", json!(now - 60)),
            ("nbf", json!(now + 120)),
            ("iat", json!(now + 120)),
            ("sub", json!("")),
            ("oid", json!("not-a-directory-object")),
        ] {
            let mut invalid = valid.clone();
            invalid[name] = value;
            assert!(
                validate_claims(&config, &key, &sign(&invalid)).is_err(),
                "accepted {name}"
            );
        }
        let token = encode(
            &Header::new(Algorithm::HS256),
            &valid,
            &EncodingKey::from_secret(b"synthetic-machine-signing-key-only"),
        )
        .unwrap();
        assert!(validate_claims(&config, &key, &token).is_err());
        let duplicate = format!(
            "{}.{}.x",
            URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256"}"#),
            URL_SAFE_NO_PAD.encode(br#"{"sub":"a","sub":"b"}"#)
        );
        assert!(unique_token(&duplicate).is_err());
        let mut duplicate_set = keys();
        duplicate_set.keys.push(duplicate_set.keys[0].clone());
        assert!(Keys::new(duplicate_set).is_err());
    }

    pub(crate) async fn trusted_jwks_contract(store: Store) {
        use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
        use std::{io::Write, sync::Arc};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec!["fixture-ca.invalid".into()]).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = params.self_signed(&key).unwrap();
        let leaf_key = KeyPair::generate().unwrap();
        let params = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        let leaf = params.signed_by(&leaf_key, &ca, &key).unwrap();
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
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            for _ in 0..2 {
                let (tcp, _) = listener.accept().await.unwrap();
                if let Ok(mut tls) = acceptor.accept(tcp).await {
                    let mut header = Vec::new();
                    while !header.ends_with(b"\r\n\r\n") {
                        assert!(header.len() < 8192);
                        header.push(tls.read_u8().await.unwrap());
                    }
                    let text = String::from_utf8(header).unwrap();
                    assert!(text.starts_with("GET /keys HTTP/1.1\r\n"));
                    assert!(!text.to_ascii_lowercase().contains("authorization:"));
                    let body = include_str!("../../../../conformance/identity/synthetic-jwks.json");
                    tls.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
                }
            }
        });
        let mut config = config();
        config.jwks_url = format!("https://127.0.0.1:{port}/keys");
        assert!(matches!(
            JwtVerifier::connect(config.clone(), store.clone()).await,
            Err(Error::Unauthenticated)
        ));
        let mut ca_file = tempfile::NamedTempFile::new().unwrap();
        ca_file.write_all(ca.pem().as_bytes()).unwrap();
        config.ca_bundle = Some(ca_file.path().into());
        let verifier = JwtVerifier::connect(config, store).await.unwrap();
        let token = sign(&claims("live-jwks-owner"));
        assert!(!verifier.authenticate(&token).await.unwrap().platform_admin);
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        // Old cached keys cannot authorize indefinitely when the configured JWKS endpoint is gone.
        verifier.keys.lock().await.expires = Duration::ZERO;
        assert!(matches!(
            verifier.authenticate(&token).await,
            Err(Error::Unauthenticated)
        ));
    }
}
