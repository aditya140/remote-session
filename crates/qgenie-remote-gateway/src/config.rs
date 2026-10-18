use crate::Error;
use std::{fmt, io::Read, net::SocketAddr, path::Path};
/// Loaded at the composition boundary. Never includes the DSN in diagnostics.
#[derive(Clone)]
pub struct SecretDsn(pub(crate) String);
impl fmt::Debug for SecretDsn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretDsn([redacted])")
    }
}
impl SecretDsn {
    pub fn from_file(path: &Path) -> Result<Self, Error> {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| Error::Configuration)?;
        if !metadata.is_file() || metadata.len() > 16384 {
            return Err(Error::Configuration);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o007 != 0 {
                return Err(Error::Configuration);
            }
        }
        let mut value = String::new();
        std::fs::File::open(path)
            .map_err(|_| Error::Configuration)?
            .take(16385)
            .read_to_string(&mut value)
            .map_err(|_| Error::Configuration)?;
        Self::parse(value.trim_end_matches(['\n', '\r']).to_owned())
    }
    fn parse(value: String) -> Result<Self, Error> {
        let url = url::Url::parse(&value).map_err(|_| Error::Configuration)?;
        if !["postgres", "postgresql"].contains(&url.scheme())
            || url.host_str().is_none()
            || url.username().is_empty()
            || url.path().len() < 2
            || value.len() > 16384
            || value.contains(['\n', '\r', '\0'])
        {
            return Err(Error::Configuration);
        }
        Ok(Self(value))
    }
}
#[derive(Debug, Clone)]
pub struct Config {
    pub node_id: String,
    pub listen: SocketAddr,
    pub health_listen: SocketAddr,
    pub database: SecretDsn,
    pub audit_database: Option<SecretDsn>,
    pub pool_size: u32,
}
impl Config {
    pub fn validate(&self) -> Result<(), Error> {
        if self.node_id.is_empty()
            || self.node_id.len() > 64
            || !self
                .node_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || self.listen == self.health_listen
            || self.pool_size == 0
            || self.pool_size > 128
        {
            return Err(Error::Configuration);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_files_and_debug_are_bounded_and_redacted() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "postgres://user:never-log@localhost/gateway").unwrap();
        let secret = SecretDsn::from_file(file.path()).unwrap();
        assert!(!format!("{secret:?}").contains("never-log"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o440)).unwrap();
            assert!(SecretDsn::from_file(file.path()).is_ok());
            std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(SecretDsn::from_file(file.path()).is_err());
        }
        assert!(SecretDsn::parse("https://example.com/password".into()).is_err());
    }
}
