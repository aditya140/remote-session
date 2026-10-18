//! Microsoft Graph accountEnabled evidence. Tokens authenticate the read; they
//! never substitute for the directory property or reach a host/native process.
use crate::Error;
use qgenie_remote_proto::{decode_json, rest::AccessToken};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore};
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryConfig {
    pub tenant: String,
    pub client_id: String,
    pub credential_file: PathBuf,
    /// Explicit HTTPS overrides support sovereign clouds and synthetic fixtures.
    #[serde(default)]
    pub token_url: Option<String>,
    #[serde(default)]
    pub graph_base: Option<String>,
    #[serde(default)]
    pub ca_bundle: Option<PathBuf>,
}
impl std::fmt::Debug for DirectoryConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DirectoryConfig([configured application identity])")
    }
}
impl DirectoryConfig {
    pub fn from_file(path: &Path) -> Result<Self, Error> {
        let bytes = read_file(path, 64 * 1024, false)?;
        let value: Self = decode_json(&bytes).map_err(|_| Error::Configuration)?;
        value.endpoints()?;
        Ok(value)
    }
    fn endpoints(&self) -> Result<(url::Url, url::Url, String), Error> {
        canonical_id(&self.tenant)?;
        canonical_id(&self.client_id)?;
        let token = https(self.token_url.as_deref().unwrap_or(&format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            self.tenant
        )))?;
        let graph = https(
            self.graph_base
                .as_deref()
                .unwrap_or("https://graph.microsoft.com/v1.0"),
        )?;
        if graph.path().ends_with('/') {
            return Err(Error::Configuration);
        }
        let scope = format!("{}/.default", graph.origin().ascii_serialization());
        Ok((token, graph, scope))
    }
}
fn canonical_id(value: &str) -> Result<(), Error> {
    if sqlx::types::Uuid::parse_str(value).map_or(true, |id| id.hyphenated().to_string() != value) {
        return Err(Error::Configuration);
    }
    Ok(())
}
fn https(value: &str) -> Result<url::Url, Error> {
    let url = url::Url::parse(value).map_err(|_| Error::Configuration)?;
    if value.len() > 2048
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Configuration);
    }
    Ok(url)
}
fn read_file(path: &Path, limit: u64, secret: bool) -> Result<Vec<u8>, Error> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| Error::Configuration)?;
    let metadata = file.metadata().map_err(|_| Error::Configuration)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(Error::Configuration);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // The credential, trust roots and endpoint configuration must all come
        // from the service user/root, with no foreign writable substitution.
        if metadata.permissions().mode() & if secret { 0o037 } else { 0o022 } != 0
            || ![0, unsafe { libc::geteuid() }].contains(&metadata.uid())
        {
            return Err(Error::Configuration);
        }
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Configuration)?;
    if bytes.len() as u64 > limit {
        return Err(Error::Configuration);
    }
    Ok(bytes)
}
struct Token {
    value: AccessToken,
    expires: Duration,
}
/// One verification result, anchored before network/credential work begins.
#[derive(Debug, Clone, Copy)]
pub struct DirectoryObservation {
    pub enabled: bool,
    pub started_elapsed: Duration,
}
/// Bounded, HTTPS-only Graph client. No refresh worker or detached runtime.
pub struct GraphDirectory {
    tenant: String,
    client_id: String,
    secret: Zeroizing<String>,
    token_url: url::Url,
    graph: url::Url,
    scope: String,
    http: reqwest::Client,
    token: Mutex<Option<Token>>,
    work: Arc<Semaphore>,
}
impl std::fmt::Debug for GraphDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GraphDirectory([configured; credentials redacted])")
    }
}
impl GraphDirectory {
    pub fn connect(config: DirectoryConfig) -> Result<Self, Error> {
        let (token_url, graph, scope) = config.endpoints()?;
        let bytes = read_file(&config.credential_file, 8192, true)?;
        let mut secret =
            Zeroizing::new(String::from_utf8(bytes).map_err(|_| Error::Configuration)?);
        let length = secret.trim_end_matches(['\n', '\r']).len();
        secret.truncate(length);
        if secret.is_empty() || secret.chars().any(char::is_control) {
            return Err(Error::Configuration);
        }
        let mut http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none());
        if let Some(path) = config.ca_bundle {
            for certificate in
                reqwest::Certificate::from_pem_bundle(&read_file(&path, 1024 * 1024, false)?)
                    .map_err(|_| Error::Configuration)?
            {
                http = http.add_root_certificate(certificate);
            }
        }
        Ok(Self {
            tenant: config.tenant,
            client_id: config.client_id,
            secret,
            token_url,
            graph,
            scope,
            http: http.build().map_err(|_| Error::Configuration)?,
            token: Mutex::new(None),
            work: Arc::new(Semaphore::new(16)),
        })
    }
    pub fn tenant(&self) -> &str {
        &self.tenant
    }
    /// Errors provide no enabled-state evidence. One total five-second budget
    /// includes token-lock wait, token acquisition, response body and Graph GET.
    pub async fn observe(&self, object: &str) -> Result<DirectoryObservation, Error> {
        canonical_id(object).map_err(|_| Error::Invalid)?;
        let _permit = self
            .work
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Overloaded)?;
        let started_elapsed =
            qgenie_remote_transport::elapsed::now().map_err(|_| Error::Unavailable)?;
        let enabled = tokio::time::timeout(Duration::from_secs(5), self.account_enabled(object))
            .await
            .map_err(|_| Error::Unavailable)??;
        // A suspended verifier cannot return a fresh result from an old request.
        if qgenie_remote_transport::elapsed::now()
            .map_err(|_| Error::Unavailable)?
            .checked_sub(started_elapsed)
            .is_none_or(|age| age > Duration::from_secs(5))
        {
            return Err(Error::Unavailable);
        }
        Ok(DirectoryObservation {
            enabled,
            started_elapsed,
        })
    }
    async fn token(&self) -> Result<AccessToken, Error> {
        let mut slot = self.token.lock().await;
        if let Some(token) = slot.as_ref()
            && token.expires
                > qgenie_remote_transport::elapsed::now().map_err(|_| Error::Unavailable)?
                    + Duration::from_secs(30)
        {
            return Ok(token.value.clone());
        }
        let started = qgenie_remote_transport::elapsed::now().map_err(|_| Error::Unavailable)?;
        let response = self
            .http
            .post(self.token_url.clone())
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.secret.as_str()),
                ("scope", self.scope.as_str()),
            ])
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        if !response.status().is_success() {
            return Err(Error::Unavailable);
        }
        #[derive(Deserialize)]
        struct Response {
            access_token: AccessToken,
            token_type: String,
            expires_in: u64,
        }
        let response: Response =
            decode_json(&body(response, 32 * 1024).await?).map_err(|_| Error::Unavailable)?;
        if !response.token_type.eq_ignore_ascii_case("bearer")
            || !(1..=3600).contains(&response.expires_in)
        {
            return Err(Error::Unavailable);
        }
        let value = response.access_token;
        *slot = Some(Token {
            value: value.clone(),
            expires: started + Duration::from_secs(response.expires_in),
        });
        Ok(value)
    }
    async fn account_enabled(&self, object: &str) -> Result<bool, Error> {
        let token = self.token().await?;
        let mut url = self.graph.clone();
        url.set_path(&format!("{}/users/{object}", self.graph.path()));
        url.query_pairs_mut()
            .append_pair("$select", "id,accountEnabled");
        let response = self
            .http
            .get(url)
            .bearer_auth(token.as_str())
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.token.lock().await.take();
            return Err(Error::Unavailable);
        }
        if !response.status().is_success() {
            return Err(Error::Unavailable);
        }
        #[derive(Deserialize)]
        struct User {
            id: String,
            #[serde(rename = "accountEnabled")]
            enabled: bool,
        }
        let user: User =
            decode_json(&body(response, 32 * 1024).await?).map_err(|_| Error::Unavailable)?;
        if user.id != object {
            return Err(Error::Unavailable);
        }
        Ok(user.enabled)
    }
}
async fn body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, Error> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(Error::Unavailable);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unavailable)? {
        if bytes.len() + chunk.len() > limit {
            return Err(Error::Unavailable);
        }
        bytes.extend(chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
pub(crate) mod tests;
