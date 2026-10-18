use crate::Error;
use serde::de::DeserializeOwned;
use std::{io::Read, path::Path, time::Duration};

pub(crate) fn https(text: &str) -> Result<url::Url, Error> {
    let url = url::Url::parse(text).map_err(|_| Error::Configuration)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
        || text.len() > 2048
    {
        return Err(Error::Configuration);
    }
    Ok(url)
}
pub(crate) fn client(ca: Option<&Path>) -> Result<reqwest::Client, Error> {
    let mut builder = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(4);
    if let Some(path) = ca {
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
    builder.build().map_err(|_| Error::Configuration)
}
pub(crate) async fn response<T: DeserializeOwned>(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<T, Error> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(Error::Protocol);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unavailable)? {
        if bytes.len() + chunk.len() > limit {
            return Err(Error::Protocol);
        }
        bytes.extend_from_slice(&chunk);
    }
    qgenie_remote_proto::decode_json(&bytes).map_err(|_| Error::Protocol)
}
