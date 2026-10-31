//! Standalone scoped OAuth interface authorized by ADR 0007. Tokens returned here
//! are short-lived composition inputs; the machine connector never receives a
//! user refresh credential.
mod device;
mod pkce;
use crate::{Error, http};
pub use device::DeviceLogin;
pub use pkce::BrowserLogin;
use qgenie_remote_proto::rest::AccessToken;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{sync::watch, time::Instant};
use zeroize::Zeroizing;

#[derive(Clone, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub remote_scope: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub device_endpoint: String,
    /// Send the PKCE extension required by some device authorization providers.
    /// Disabled by default to preserve existing provider requests.
    #[serde(default)]
    pub device_pkce: bool,
    #[serde(default)]
    pub ca_bundle: Option<PathBuf>,
}
impl std::fmt::Debug for OAuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OAuthConfig([configured endpoints])")
    }
}
impl OAuthConfig {
    pub fn from_file(path: &Path) -> Result<Self, Error> {
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
        if self.client_id.is_empty()
            || self.client_id.len() > 128
            || self
                .client_id
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
            || !self.remote_scope.ends_with("/Remote.Control")
            || self.remote_scope.len() > 512
            || self
                .remote_scope
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        {
            return Err(Error::Configuration);
        }
        let auth = http::https(&self.authorization_endpoint)?;
        let token = http::https(&self.token_endpoint)?;
        let device = http::https(&self.device_endpoint)?;
        if auth.origin() != token.origin() || auth.origin() != device.origin() {
            return Err(Error::Configuration);
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct OAuth {
    config: OAuthConfig,
    http: reqwest::Client,
}
impl OAuth {
    pub fn new(config: OAuthConfig) -> Result<Self, Error> {
        config.validate()?;
        let http = http::client(config.ca_bundle.as_deref())?;
        Ok(Self { config, http })
    }
    fn scopes(&self) -> String {
        format!("openid offline_access {}", self.config.remote_scope)
    }
    async fn exchange(&self, fields: &[(&str, &str)]) -> Result<Tokens, Error> {
        let started = Instant::now();
        let response = self
            .http
            .post(&self.config.token_endpoint)
            .form(fields)
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        if !response.status().is_success() {
            return Err(Error::Denied);
        }
        let mut response: TokenResponse = http::response(response, 65536).await?;
        // OAuth permits omission when a refresh keeps the originally requested scope.
        if response.scope.is_none() && fields.contains(&("grant_type", "refresh_token")) {
            response.scope = Some(self.config.remote_scope.clone());
        }
        self.tokens(response, started)
    }

    /// Exchanges one client-owned refresh credential without automatic retries.
    ///
    /// The caller serializes exchanges and replaces a returned rotated credential.
    /// On an uncertain exchange, sign in again instead of resending the old token.
    /// Machine enrollment and connector state must never receive this credential.
    ///
    /// # Errors
    /// Returns a configuration error for invalid input, denied for provider refusal,
    /// or unavailable/protocol when the exchange outcome cannot be established.
    pub async fn refresh(&self, refresh: &str) -> Result<Tokens, Error> {
        if refresh.is_empty() || refresh.len() > 16384 || refresh.chars().any(char::is_control) {
            return Err(Error::Configuration);
        }
        self.exchange(&[
            ("grant_type", "refresh_token"),
            ("client_id", &self.config.client_id),
            ("refresh_token", refresh),
            ("scope", &self.scopes()),
        ])
        .await
    }
    fn tokens(&self, response: TokenResponse, started: Instant) -> Result<Tokens, Error> {
        if !response
            .token_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("Bearer"))
            || response.expires_in == 0
            || response.expires_in > 86400
            || !response.scope.as_deref().is_some_and(|scope| {
                scope
                    .split_ascii_whitespace()
                    .any(|scope| scope == self.config.remote_scope)
            })
            || response
                .refresh_token
                .as_ref()
                .is_some_and(|token| token.is_empty() || token.len() > 16384)
        {
            return Err(Error::Protocol);
        }
        let access = response
            .access_token
            .try_into()
            .map_err(|_| Error::Protocol)?;
        let expires = started + Duration::from_secs(response.expires_in);
        if expires <= Instant::now() {
            return Err(Error::Expired);
        }
        Ok(Tokens {
            access,
            refresh: response.refresh_token.map(Zeroizing::new),
            expires,
        })
    }
}
pub struct Tokens {
    pub access: AccessToken,
    refresh: Option<Zeroizing<String>>,
    pub expires: Instant,
}
impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access", &"[redacted]")
            .field("refresh", &"[redacted]")
            .finish_non_exhaustive()
    }
}
impl Tokens {
    /// Only interactive client composition may move this into its own protected
    /// credential store. Enrollment and connector APIs accept the access token only.
    pub fn take_refresh(&mut self) -> Option<Zeroizing<String>> {
        self.refresh.take()
    }
}
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: Option<String>,
    expires_in: u64,
    scope: Option<String>,
}
pub(super) async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow() {
            return;
        }
        if cancel.changed().await.is_err() {
            return;
        }
    }
}
