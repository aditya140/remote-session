use super::{OAuth, TokenResponse, Tokens, cancelled};
use crate::{Error, http};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::{
    sync::watch,
    time::{Instant, sleep_until},
};
use zeroize::Zeroizing;

pub struct DeviceLogin {
    oauth: OAuth,
    device_code: Zeroizing<String>,
    verifier: Option<Zeroizing<String>>,
    user_code: String,
    verification_uri: String,
    interval: Duration,
    expires: Instant,
}
impl std::fmt::Debug for DeviceLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeviceLogin([codes redacted])")
    }
}
#[derive(Deserialize)]
struct DeviceResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
}
fn default_interval() -> u64 {
    5
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Poll {
    Token(TokenResponse),
    Error { error: String },
}
impl OAuth {
    pub async fn device(&self, cancel: &mut watch::Receiver<bool>) -> Result<DeviceLogin, Error> {
        let started = Instant::now();
        let verifier = self.config.device_pkce.then(super::pkce::random);
        let challenge = verifier
            .as_ref()
            .map(|value| URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes())));
        let scopes = self.scopes();
        let mut fields = vec![
            ("client_id", self.config.client_id.as_str()),
            ("scope", scopes.as_str()),
        ];
        if let Some(challenge) = &challenge {
            fields.extend([
                ("code_challenge_method", "S256"),
                ("code_challenge", challenge),
            ]);
        }
        let request = async {
            let response = self
                .http
                .post(&self.config.device_endpoint)
                .form(&fields)
                .send()
                .await
                .map_err(|_| Error::Unavailable)?;
            if !response.status().is_success() {
                return Err(Error::Denied);
            }
            http::response::<DeviceResponse>(response, 16384).await
        };
        let reply = tokio::select! {biased;_=cancelled(cancel)=>return Err(Error::Cancelled),result=request=>result?};
        if reply.device_code.is_empty()
            || reply.device_code.len() > 8192
            || reply.user_code.is_empty()
            || reply.user_code.len() > 64
            || reply.user_code.chars().any(char::is_control)
            || reply.expires_in == 0
            || reply.expires_in > 1800
            || !(1..=60).contains(&reply.interval)
        {
            return Err(Error::Protocol);
        }
        http::https(&reply.verification_uri).map_err(|_| Error::Protocol)?;
        let expires = started + Duration::from_secs(reply.expires_in);
        if expires <= Instant::now() {
            return Err(Error::Expired);
        }
        Ok(DeviceLogin {
            oauth: self.clone(),
            device_code: Zeroizing::new(reply.device_code),
            verifier,
            user_code: reply.user_code,
            verification_uri: reply.verification_uri,
            interval: Duration::from_secs(reply.interval),
            expires,
        })
    }
}
impl DeviceLogin {
    pub fn user_code(&self) -> &str {
        &self.user_code
    }
    pub fn verification_uri(&self) -> &str {
        &self.verification_uri
    }
    pub fn remaining(&self) -> Duration {
        self.expires.saturating_duration_since(Instant::now())
    }
    pub async fn wait(mut self, cancel: &mut watch::Receiver<bool>) -> Result<Tokens, Error> {
        loop {
            let next = Instant::now() + self.interval;
            tokio::select! {biased;_=cancelled(cancel)=>return Err(Error::Cancelled),_=sleep_until(self.expires)=>return Err(Error::Expired),_=sleep_until(next)=>{}}
            let started = Instant::now();
            let mut fields = vec![
                ("client_id", self.oauth.config.client_id.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", self.device_code.as_str()),
            ];
            if let Some(verifier) = &self.verifier {
                fields.push(("code_verifier", verifier.as_str()));
            }
            let request = self
                .oauth
                .http
                .post(&self.oauth.config.token_endpoint)
                .form(&fields)
                .send();
            let response = tokio::select! {biased;_=cancelled(cancel)=>return Err(Error::Cancelled),_=sleep_until(self.expires)=>return Err(Error::Expired),result=request=>result.map_err(|_| Error::Unavailable)?};
            if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let delay = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(5);
                self.interval =
                    (self.interval + Duration::from_secs(5)).max(Duration::from_secs(delay));
                if self.interval > self.remaining() {
                    return Err(Error::Expired);
                }
                continue;
            }
            if response.status().is_server_error() {
                return Err(Error::Unavailable);
            }
            let success = response.status().is_success();
            let reply = tokio::select! {biased;_=cancelled(cancel)=>return Err(Error::Cancelled),_=sleep_until(self.expires)=>return Err(Error::Expired),result=http::response::<Poll>(response,65536)=>result?};
            match reply {
                Poll::Token(token) if success => return self.oauth.tokens(token, started),
                Poll::Error { error } if error == "authorization_pending" => {}
                Poll::Error { error } if error == "slow_down" => {
                    self.interval += Duration::from_secs(5)
                }
                Poll::Error { error } if error == "expired_token" => return Err(Error::Expired),
                _ => return Err(Error::Denied),
            }
        }
    }
}
