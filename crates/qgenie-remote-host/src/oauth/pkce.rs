use super::{OAuth, Tokens, cancelled};
use crate::Error;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    time::{Instant, sleep_until, timeout},
};
use zeroize::Zeroizing;

pub struct BrowserLogin {
    oauth: OAuth,
    listener: TcpListener,
    authorization_url: String,
    redirect: String,
    state: Zeroizing<String>,
    verifier: Zeroizing<String>,
    expires: Instant,
}
impl std::fmt::Debug for BrowserLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrowserLogin([state and verifier redacted])")
    }
}
fn random() -> Zeroizing<String> {
    Zeroizing::new(URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()))
}
impl OAuth {
    pub async fn browser(&self) -> Result<BrowserLogin, Error> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| Error::Unavailable)?;
        let redirect = format!(
            "http://{}/oauth/callback",
            listener.local_addr().map_err(|_| Error::Unavailable)?
        );
        let state = random();
        let verifier = random();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = url::Url::parse(&self.config.authorization_endpoint)
            .map_err(|_| Error::Configuration)?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", self.config.client_id.as_str()),
            ("response_type", "code"),
            ("redirect_uri", &redirect),
            ("scope", &self.scopes()),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
        ]);
        Ok(BrowserLogin {
            oauth: self.clone(),
            listener,
            authorization_url: url.to_string(),
            redirect,
            state,
            verifier,
            expires: Instant::now() + Duration::from_secs(300),
        })
    }
}
impl BrowserLogin {
    /// Pass directly to the system browser. Avoid logging the authorization URL.
    pub fn authorization_url(&self) -> &str {
        &self.authorization_url
    }
    pub async fn wait(self, cancel: &mut watch::Receiver<bool>) -> Result<Tokens, Error> {
        let mut attempts = 0;
        loop {
            let accepted = tokio::select! {biased;_=cancelled(cancel)=>return Err(Error::Cancelled),_=sleep_until(self.expires)=>return Err(Error::Expired),result=self.listener.accept()=>result.map_err(|_| Error::Unavailable)?};
            attempts += 1;
            if attempts > 32 {
                return Err(Error::Denied);
            }
            let (mut stream, peer) = accepted;
            if !peer.ip().is_loopback() {
                continue;
            }
            let response = tokio::select! {biased;_=cancelled(cancel)=>return Err(Error::Cancelled),_=sleep_until(self.expires)=>return Err(Error::Expired),result=timeout(Duration::from_secs(2),callback(&mut stream,&self.redirect,&self.state))=>result};
            let code = match response {
                Ok(Ok(code)) => {
                    respond(&mut stream, true).await;
                    code
                }
                Ok(Err(CallbackError::Denied)) => {
                    respond(&mut stream, false).await;
                    return Err(Error::Denied);
                }
                _ => {
                    respond(&mut stream, false).await;
                    continue;
                }
            };
            let fields: [(&str, &str); 5] = [
                ("client_id", &self.oauth.config.client_id),
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", &self.redirect),
                ("code_verifier", &self.verifier),
            ];
            let request = self.oauth.exchange(&fields);
            return tokio::select! {biased;_=cancelled(cancel)=>Err(Error::Cancelled),_=sleep_until(self.expires)=>Err(Error::Expired),result=request=>result};
        }
    }
}
enum CallbackError {
    Invalid,
    Denied,
}
async fn callback(
    stream: &mut TcpStream,
    redirect: &str,
    state: &str,
) -> Result<Zeroizing<String>, CallbackError> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 8192 {
            return Err(CallbackError::Invalid);
        }
        bytes.push(stream.read_u8().await.map_err(|_| CallbackError::Invalid)?);
    }
    let header = std::str::from_utf8(&bytes).map_err(|_| CallbackError::Invalid)?;
    let mut lines = header.split("\r\n");
    let request = lines.next().ok_or(CallbackError::Invalid)?;
    let mut parts = request.split(' ');
    if parts.next() != Some("GET") {
        return Err(CallbackError::Invalid);
    }
    let target = parts.next().ok_or(CallbackError::Invalid)?;
    if parts.next() != Some("HTTP/1.1")
        || parts.next().is_some()
        || !target.starts_with("/oauth/callback?")
    {
        return Err(CallbackError::Invalid);
    }
    let redirect = url::Url::parse(redirect).map_err(|_| CallbackError::Invalid)?;
    let mut host = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let (key, value) = line.split_once(':').ok_or(CallbackError::Invalid)?;
        if key.eq_ignore_ascii_case("host") {
            if host.is_some() {
                return Err(CallbackError::Invalid);
            }
            host = Some(value.trim());
        }
    }
    if host
        != Some(&format!(
            "127.0.0.1:{}",
            redirect.port().ok_or(CallbackError::Invalid)?
        ))
    {
        return Err(CallbackError::Invalid);
    }
    let url = redirect.join(target).map_err(|_| CallbackError::Invalid)?;
    let mut fields = HashMap::new();
    for (key, value) in url.query_pairs() {
        if fields
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err(CallbackError::Invalid);
        }
    }
    if fields
        .get("state")
        .is_none_or(|candidate| candidate != state)
    {
        return Err(CallbackError::Invalid);
    }
    if fields.contains_key("error") {
        return Err(CallbackError::Denied);
    }
    let code = fields.remove("code").ok_or(CallbackError::Invalid)?;
    if code.is_empty()
        || code.len() > 4096
        || !code.is_ascii()
        || code
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(CallbackError::Invalid);
    }
    Ok(Zeroizing::new(code))
}
async fn respond(stream: &mut TcpStream, success: bool) {
    let (status, body) = if success {
        ("200 OK", "Sign-in received. Return to the terminal.")
    } else {
        ("400 Bad Request", "Sign-in could not be accepted.")
    };
    let message = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = timeout(Duration::from_secs(1), stream.write_all(message.as_bytes())).await;
}
