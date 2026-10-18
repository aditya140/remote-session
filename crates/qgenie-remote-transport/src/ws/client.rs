use super::{Connection, Error, Limits, Metrics, connection};
use base64::{Engine, engine::general_purpose::STANDARD};
use ipnet::IpNet;
use rustls::{RootCertStore, pki_types::ServerName};
use std::{fmt, net::IpAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest,
    http::{HeaderValue, header::AUTHORIZATION},
};
use url::Url;

/// An authorization header whose Debug output cannot expose credentials.
#[derive(Clone)]
pub struct SecretHeader(HeaderValue);
impl SecretHeader {
    pub fn bearer(token: &str) -> Result<Self, Error> {
        if token.is_empty()
            || token.len() > 16 * 1024
            || token
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        {
            return Err(Error::Configuration);
        }
        let mut value =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| Error::Configuration)?;
        value.set_sensitive(true);
        Ok(Self(value))
    }
}
impl fmt::Debug for SecretHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretHeader([redacted])")
    }
}
#[derive(Clone)]
pub struct ClientConfig {
    endpoint: Url,
    proxy: Option<Url>,
    no_proxy: String,
    ca_bundle: Option<PathBuf>,
    authorization: Option<SecretHeader>,
    allow_insecure_loopback: bool,
}
impl fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientConfig")
            .field("endpoint", &"[redacted URL]")
            .field("proxy", &self.proxy.is_some())
            .field("custom_ca", &self.ca_bundle.is_some())
            .field("authorization", &self.authorization)
            .field("allow_insecure_loopback", &self.allow_insecure_loopback)
            .finish()
    }
}
impl ClientConfig {
    /// Snapshot proxy environment now; connection tasks do not mutate process configuration.
    pub fn new(endpoint: &str) -> Result<Self, Error> {
        let endpoint = Url::parse(endpoint).map_err(|_| Error::Configuration)?;
        if !["wss", "ws"].contains(&endpoint.scheme())
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(Error::Configuration);
        }
        let proxy = environment("HTTPS_PROXY", "https_proxy")
            .map(|proxy| parse_proxy(&proxy))
            .transpose()?;
        Ok(Self {
            endpoint,
            proxy,
            no_proxy: environment("NO_PROXY", "no_proxy").unwrap_or_default(),
            ca_bundle: None,
            authorization: None,
            allow_insecure_loopback: false,
        })
    }
    pub fn proxy(mut self, proxy: Option<&str>) -> Result<Self, Error> {
        self.proxy = proxy.map(parse_proxy).transpose()?;
        Ok(self)
    }
    pub fn no_proxy(mut self, rules: impl Into<String>) -> Self {
        self.no_proxy = rules.into();
        self
    }
    pub fn ca_bundle(mut self, path: impl Into<PathBuf>) -> Self {
        self.ca_bundle = Some(path.into());
        self
    }
    pub fn authorization(mut self, header: SecretHeader) -> Self {
        self.authorization = Some(header);
        self
    }
    /// Explicit development escape hatch: the actual TCP peer must also be loopback.
    pub fn allow_insecure_loopback(mut self, allow: bool) -> Self {
        self.allow_insecure_loopback = allow;
        self
    }
    pub fn bypasses_proxy(&self) -> bool {
        bypass(
            &self.no_proxy,
            host(&self.endpoint),
            self.endpoint.port_or_known_default().unwrap_or(443),
        )
    }
}
fn environment(upper: &str, lower: &str) -> Option<String> {
    [upper, lower]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
}
fn parse_proxy(text: &str) -> Result<Url, Error> {
    let proxy = Url::parse(text).map_err(|_| Error::Configuration)?;
    if !["http", "https"].contains(&proxy.scheme())
        || proxy.host_str().is_none()
        || proxy.path() != "/"
        || proxy.query().is_some()
        || proxy.fragment().is_some()
        || text.len() > 32 * 1024
    {
        return Err(Error::Configuration);
    }
    Ok(proxy)
}
fn host(url: &Url) -> &str {
    url.host_str()
        .unwrap_or("")
        .trim_start_matches('[')
        .trim_end_matches(']')
}
fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}
fn bypass(rules: &str, host: &str, port: u16) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    for entry in rules
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        if entry == "*" {
            return true;
        }
        if let Ok(network) = entry.parse::<IpNet>() {
            if host
                .parse::<IpAddr>()
                .is_ok_and(|address| network.contains(&address))
            {
                return true;
            }
            continue;
        }
        if let Ok(address) = entry.trim_matches(['[', ']']).parse::<IpAddr>() {
            if host.parse::<IpAddr>().ok() == Some(address) {
                return true;
            }
            continue;
        }
        let (rule, rule_port) = if entry.starts_with('[') {
            match entry.split_once("]:") {
                Some((host, port)) => (host.trim_start_matches('['), Some(port)),
                None => (entry, None),
            }
        } else if entry.matches(':').count() == 1 {
            match entry.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (entry, None),
            }
        } else {
            (entry, None)
        };
        if let Some(rule_port) = rule_port
            && rule_port.parse::<u16>().ok() != Some(port)
        {
            continue;
        }
        let rule = rule
            .trim_start_matches('.')
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if !rule.is_empty()
            && (host == rule
                || host
                    .strip_suffix(&rule)
                    .is_some_and(|prefix| prefix.ends_with('.')))
        {
            return true;
        }
    }
    false
}
trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
type Stream = Box<dyn Io>;

/// One attempt only. Reconnection and operation reconciliation belong to the caller.
pub async fn connect(
    config: ClientConfig,
    limits: Limits,
    metrics: Option<Metrics>,
) -> Result<Connection, Error> {
    limits.validate()?;
    if config.endpoint.scheme() == "ws" && !config.allow_insecure_loopback {
        return Err(Error::Configuration);
    }
    let tls_config = config.clone();
    let tls = tokio::task::spawn_blocking(move || trust(&tls_config))
        .await
        .map_err(|_| Error::Task)??;
    timeout(
        Duration::from_secs(10),
        connect_inner(config, limits, metrics, tls),
    )
    .await
    .map_err(|_| Error::Network)?
}
fn trust(config: &ClientConfig) -> Result<Arc<rustls::ClientConfig>, Error> {
    let mut roots = RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    for certificate in native.certs {
        let _ = roots.add(certificate);
    }
    if let Some(path) = &config.ca_bundle {
        let file = std::fs::File::open(path).map_err(|_| Error::Configuration)?;
        if file.metadata().map_err(|_| Error::Configuration)?.len() > 16 * 1024 * 1024 {
            return Err(Error::Configuration);
        }
        let certificates = rustls_pemfile::certs(&mut std::io::BufReader::new(file))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::Configuration)?;
        if certificates.is_empty() {
            return Err(Error::Configuration);
        }
        for certificate in certificates {
            roots.add(certificate).map_err(|_| Error::Configuration)?;
        }
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| Error::Configuration)?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Arc::new(config))
}
async fn tls(
    stream: Stream,
    host: &str,
    config: Arc<rustls::ClientConfig>,
) -> Result<Stream, Error> {
    let name = ServerName::try_from(host.to_owned()).map_err(|_| Error::Configuration)?;
    Ok(Box::new(
        TlsConnector::from(config)
            .connect(name, stream)
            .await
            .map_err(|_| Error::Tls)?,
    ))
}
async fn connect_inner(
    config: ClientConfig,
    limits: Limits,
    metrics: Option<Metrics>,
    trust: Arc<rustls::ClientConfig>,
) -> Result<Connection, Error> {
    let endpoint_host = host(&config.endpoint);
    let endpoint_port = config
        .endpoint
        .port_or_known_default()
        .ok_or(Error::Configuration)?;
    let proxy = config.proxy.as_ref().filter(|_| !config.bypasses_proxy());
    // Plaintext is confined to direct loopback development, including DNS resolution.
    if config.endpoint.scheme() == "ws" && proxy.is_some() {
        return Err(Error::Configuration);
    }
    let (connect_host, connect_port) = proxy.map_or((endpoint_host, endpoint_port), |proxy| {
        (host(proxy), proxy.port_or_known_default().unwrap_or(8080))
    });
    let tcp = TcpStream::connect((connect_host, connect_port))
        .await
        .map_err(|_| Error::Network)?;
    tcp.set_nodelay(true).map_err(|_| Error::Network)?;
    if config.endpoint.scheme() == "ws"
        && !tcp
            .peer_addr()
            .map_err(|_| Error::Network)?
            .ip()
            .is_loopback()
    {
        return Err(Error::Configuration);
    }
    let mut stream: Stream = Box::new(tcp);
    if let Some(proxy) = proxy {
        if proxy.scheme() == "https" {
            stream = tls(stream, host(proxy), trust.clone()).await?;
        }
        tunnel(&mut stream, proxy, &authority(endpoint_host, endpoint_port)).await?;
    }
    if config.endpoint.scheme() == "wss" {
        stream = tls(stream, endpoint_host, trust).await?;
    }
    let mut request = config
        .endpoint
        .as_str()
        .into_client_request()
        .map_err(|_| Error::Configuration)?;
    if let Some(header) = config.authorization {
        request.headers_mut().insert(AUTHORIZATION, header.0);
    }
    let (socket, _) =
        tokio_tungstenite::client_async_with_config(request, stream, Some(connection::config()))
            .await
            .map_err(connection::ws_error)?;
    Ok(connection::start(socket, limits, metrics))
}
async fn tunnel(stream: &mut Stream, proxy: &Url, authority: &str) -> Result<(), Error> {
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if !proxy.username().is_empty() || proxy.password().is_some() {
        let user = percent_encoding::percent_decode_str(proxy.username())
            .decode_utf8()
            .map_err(|_| Error::Configuration)?;
        let password = percent_encoding::percent_decode_str(proxy.password().unwrap_or(""))
            .decode_utf8()
            .map_err(|_| Error::Configuration)?;
        let encoded = STANDARD.encode(format!("{user}:{password}"));
        request.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|_| Error::Network)?;
    stream.flush().await.map_err(|_| Error::Network)?;
    let mut response = Vec::with_capacity(1024);
    loop {
        if response.len() >= 16 * 1024 {
            return Err(Error::ProxyProtocol);
        }
        response.push(stream.read_u8().await.map_err(|_| Error::ProxyProtocol)?);
        if response.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let text = std::str::from_utf8(&response).map_err(|_| Error::ProxyProtocol)?;
    let mut line = text
        .lines()
        .next()
        .ok_or(Error::ProxyProtocol)?
        .split_whitespace();
    if !matches!(line.next(), Some("HTTP/1.0" | "HTTP/1.1")) {
        return Err(Error::ProxyProtocol);
    }
    let status = line
        .next()
        .ok_or(Error::ProxyProtocol)?
        .parse::<u16>()
        .map_err(|_| Error::ProxyProtocol)?;
    if status != 200 {
        return Err(Error::ProxyRejected(status));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bypass_rules_have_domain_boundaries_and_ports() {
        assert!(bypass(".example.com", "api.example.com", 443));
        assert!(!bypass("example.com", "badexample.com", 443));
        assert!(bypass("example.com:8443", "example.com", 8443));
        assert!(!bypass("example.com:8443", "example.com", 443));
        assert!(bypass("10.0.0.0/8", "10.2.3.4", 443));
        assert!(bypass("[::1]:8443", "::1", 8443));
        assert!(bypass("::1", "::1", 443));
        assert!(!bypass("127.0.0.1", "localhost", 443));
        assert!(bypass("*", "anything.invalid", 443));
        let config = ClientConfig::new("wss://localhost/?sensitive=never-log")
            .unwrap()
            .proxy(Some("http://user:secret@localhost:8080"))
            .unwrap()
            .authorization(SecretHeader::bearer("never-log").unwrap());
        let debug = format!("{config:?}");
        for secret in ["secret", "never-log", "sensitive", "user:"] {
            assert!(!debug.contains(secret));
        }
    }
}
