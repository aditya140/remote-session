use crate::Error;
use ring::aead;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

/// Explicit browser OIDC deployment configuration. All gateways share the key file.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserConfig {
    pub public_origin: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub client_id: String,
    pub scope: String,
    pub encryption_key_file: PathBuf,
    #[serde(default)]
    pub ca_bundle: Option<PathBuf>,
}
impl BrowserConfig {
    pub fn from_file(path: &Path) -> Result<Self, Error> {
        let bytes = std::fs::read(path).map_err(|_| Error::Configuration)?;
        if bytes.len() > 16384 {
            return Err(Error::Configuration);
        }
        let config: Self =
            qgenie_remote_proto::decode_json(&bytes).map_err(|_| Error::Configuration)?;
        config.validate()?;
        Ok(config)
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        for value in [
            &self.public_origin,
            &self.authorization_endpoint,
            &self.token_endpoint,
        ] {
            let u = url::Url::parse(value).map_err(|_| Error::Configuration)?;
            if u.scheme() != "https"
                || u.host_str().is_none()
                || !u.username().is_empty()
                || u.password().is_some()
                || u.fragment().is_some()
                || value.len() > 2048
            {
                return Err(Error::Configuration);
            }
        }
        let origin = url::Url::parse(&self.public_origin).map_err(|_| Error::Configuration)?;
        if origin.origin().ascii_serialization() != self.public_origin
            || self.client_id.is_empty()
            || self.client_id.len() > 256
            || self.scope.is_empty()
            || self.scope.len() > 1024
            || self.scope.chars().any(char::is_control)
        {
            return Err(Error::Configuration);
        }
        if url::Url::parse(&self.token_endpoint)
            .map_err(|_| Error::Configuration)?
            .query()
            .is_some()
        {
            return Err(Error::Configuration);
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct BrowserService {
    pub(crate) config: BrowserConfig,
    pub(crate) http: reqwest::Client,
    pub(crate) cipher: Arc<aead::LessSafeKey>,
}
impl BrowserService {
    pub fn new(config: BrowserConfig) -> Result<Self, Error> {
        config.validate()?;
        let md = std::fs::symlink_metadata(&config.encryption_key_file)
            .map_err(|_| Error::Configuration)?;
        if !md.is_file() || md.len() != 32 {
            return Err(Error::Configuration);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if md.permissions().mode() & 0o007 != 0 {
                return Err(Error::Configuration);
            }
        }
        let mut key =
            std::fs::read(&config.encryption_key_file).map_err(|_| Error::Configuration)?;
        let unbound =
            aead::UnboundKey::new(&aead::AES_256_GCM, &key).map_err(|_| Error::Configuration)?;
        use zeroize::Zeroize;
        key.zeroize();
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(8));
        if let Some(path) = &config.ca_bundle {
            let bytes = std::fs::read(path).map_err(|_| Error::Configuration)?;
            if bytes.len() > 1024 * 1024 {
                return Err(Error::Configuration);
            }
            builder = builder.add_root_certificate(
                reqwest::Certificate::from_pem(&bytes).map_err(|_| Error::Configuration)?,
            );
        }
        Ok(Self {
            config,
            http: builder.build().map_err(|_| Error::Configuration)?,
            cipher: Arc::new(aead::LessSafeKey::new(unbound)),
        })
    }
    pub(crate) fn seal(&self, plain: &str, purpose: &[u8]) -> Result<Vec<u8>, Error> {
        use rand::RngCore;
        let mut nonce = [0; 12];
        rand::rng().fill_bytes(&mut nonce);
        let mut value = plain.as_bytes().to_vec();
        self.cipher
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(purpose),
                &mut value,
            )
            .map_err(|_| Error::Configuration)?;
        let mut result = nonce.to_vec();
        result.extend(value);
        Ok(result)
    }
    pub(crate) fn open(&self, mut data: Vec<u8>, purpose: &[u8]) -> Result<String, Error> {
        if data.len() < 28 || data.len() > 65564 {
            return Err(Error::Unauthenticated);
        }
        let nonce: [u8; 12] = data[..12].try_into().map_err(|_| Error::Unauthenticated)?;
        let plain = self
            .cipher
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(purpose),
                &mut data[12..],
            )
            .map_err(|_| Error::Unauthenticated)?;
        String::from_utf8(plain.to_vec()).map_err(|_| Error::Unauthenticated)
    }
}
