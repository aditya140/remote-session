use crate::Error;
use base64::{Engine, engine::general_purpose::STANDARD};
use redis::{Client, IntoConnectionInfo, ProtocolVersion, TlsCertificates};
use std::{io::Read, path::Path};
/// Backend credentials are private; diagnostics never include the Redis URL or signing key.
pub struct RoutingConfig {
    pub(super) node: String,
    pub(super) client: Client,
    pub(super) key: [u8; 32],
}
impl std::fmt::Debug for RoutingConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutingConfig")
            .field("node", &self.node)
            .finish_non_exhaustive()
    }
}
impl RoutingConfig {
    pub fn from_files(
        node: String,
        url_file: &Path,
        key_file: &Path,
        ca_file: Option<&Path>,
    ) -> Result<Self, Error> {
        if node.is_empty()
            || node.len() > 64
            || !node
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(Error::Configuration);
        }
        let uri = String::from_utf8(secret(url_file, 16384)?).map_err(|_| Error::Configuration)?;
        let url = url::Url::parse(uri.trim()).map_err(|_| Error::Configuration)?;
        if !["redis", "rediss"].contains(&url.scheme())
            || url.host_str().is_none()
            || url.fragment().is_some()
        {
            return Err(Error::Configuration);
        }
        if url.scheme() == "redis"
            && !url.host_str().is_some_and(|host| {
                host == "localhost"
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|address| address.is_loopback())
            })
        {
            return Err(Error::Configuration);
        }
        let mut info = uri
            .trim()
            .into_connection_info()
            .map_err(|_| Error::Configuration)?;
        info = info.clone().set_redis_settings(
            info.redis_settings()
                .clone()
                .set_protocol(ProtocolVersion::RESP3),
        );
        let client = if let Some(path) = ca_file {
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(|_| Error::Configuration)?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| Error::Configuration)?;
            if bytes.len() > 1024 * 1024 || url.scheme() != "rediss" {
                return Err(Error::Configuration);
            }
            Client::build_with_tls(
                info,
                TlsCertificates {
                    client_tls: None,
                    root_cert: Some(bytes),
                },
            )
            .map_err(|_| Error::Configuration)?
        } else {
            Client::open(info).map_err(|_| Error::Configuration)?
        };
        let encoded =
            String::from_utf8(secret(key_file, 256)?).map_err(|_| Error::Configuration)?;
        let key = STANDARD
            .decode(encoded.trim())
            .map_err(|_| Error::Configuration)?
            .try_into()
            .map_err(|_| Error::Configuration)?;
        Ok(Self { node, client, key })
    }
}
fn secret(path: &Path, limit: u64) -> Result<Vec<u8>, Error> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| Error::Configuration)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(Error::Configuration);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o007 != 0 {
            return Err(Error::Configuration);
        }
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| Error::Configuration)?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Configuration)?;
    if bytes.len() as u64 > limit {
        return Err(Error::Configuration);
    }
    Ok(bytes)
}
