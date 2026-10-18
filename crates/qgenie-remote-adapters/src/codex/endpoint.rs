use super::CodexError;
use super::authorization::{NativeStream, WriteScope};
use qgenie_remote_proto::{MAX_JSON_BYTES, SessionId};
use std::{
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt},
    },
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    net::UnixStream,
    sync::watch,
    time::{sleep, timeout},
};
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::WebSocketConfig};
pub(super) struct SocketGuard {
    files: Vec<(PathBuf, u64, bool)>,
}
#[derive(Clone)]
pub(super) struct SocketIdentity(Vec<(PathBuf, u64, bool)>);
impl SocketGuard {
    pub fn identity(&self) -> SocketIdentity {
        SocketIdentity(self.files.clone())
    }
}
impl SocketIdentity {
    fn validate(&self) -> Result<(), CodexError> {
        for (path, inode, symlink) in &self.0 {
            let metadata =
                std::fs::symlink_metadata(path).map_err(|_| CodexError::Configuration)?;
            // SAFETY: geteuid has no preconditions.
            if metadata.ino() != *inode
                || metadata.uid() != unsafe { libc::geteuid() }
                || if *symlink {
                    !metadata.file_type().is_symlink()
                } else {
                    !metadata.file_type().is_socket() || metadata.mode() & 0o077 != 0
                }
            {
                return Err(CodexError::Configuration);
            }
        }
        Ok(())
    }
}
impl Drop for SocketGuard {
    fn drop(&mut self) {
        for (path, inode, symlink) in &self.files {
            if let Ok(metadata) = std::fs::symlink_metadata(path)
                && metadata.ino() == *inode
                && (metadata.file_type().is_socket()
                    || *symlink && metadata.file_type().is_symlink())
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}
pub(super) fn socket_path(home: &Path) -> Result<PathBuf, CodexError> {
    let nonce = SessionId::new();
    let suffix = nonce.as_bytes()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let path = home.join(format!("n_{suffix}.sock"));
    if path.as_os_str().as_bytes().len() > 4096 {
        return Err(CodexError::Configuration);
    }
    Ok(path)
}
fn check(path: &Path) -> Result<SocketGuard, CodexError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| CodexError::Configuration)?;
    // SAFETY: geteuid has no arguments or preconditions.
    if !metadata.file_type().is_socket()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(CodexError::Configuration);
    }
    Ok(SocketGuard {
        files: vec![(path.into(), metadata.ino(), false)],
    })
}
// Codex 0.159.2 advertises an owned symlink to a hash-named protected socket.
// Accept only that pinned mapping, then connect to the checked physical socket directly.
fn resolve(path: &Path) -> Result<(PathBuf, SocketGuard), CodexError> {
    use sha2::{Digest, Sha256};
    let alias = std::fs::symlink_metadata(path).map_err(|_| CodexError::Configuration)?;
    // SAFETY: geteuid has no arguments or preconditions.
    let uid = unsafe { libc::geteuid() };
    if !alias.file_type().is_symlink() || alias.uid() != uid {
        return Err(CodexError::Configuration);
    }
    let canonical = std::fs::canonicalize(path.parent().ok_or(CodexError::Configuration)?)
        .map_err(|_| CodexError::Configuration)?
        .join(path.file_name().ok_or(CodexError::Configuration)?);
    let directory = std::fs::canonicalize("/tmp")
        .map_err(|_| CodexError::Configuration)?
        .join(format!("codex-daemon-{uid}"));
    let metadata = std::fs::symlink_metadata(&directory).map_err(|_| CodexError::Configuration)?;
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o777 != 0o700 {
        return Err(CodexError::Configuration);
    }
    let hash = Sha256::digest(canonical.as_os_str().as_bytes());
    let physical = directory.join(format!("{hash:x}"));
    if std::fs::read_link(path).map_err(|_| CodexError::Configuration)? != physical {
        return Err(CodexError::Configuration);
    }
    let mut guard = check(&physical)?;
    guard.files.push((path.into(), alias.ino(), true));
    Ok((physical, guard))
}
pub(super) async fn connect_owned(
    path: &Path,
    running: watch::Receiver<bool>,
    scope: WriteScope,
) -> Result<(WebSocketStream<NativeStream>, SocketGuard), CodexError> {
    timeout(Duration::from_secs(5), async {
        loop {
            if !*running.borrow() {
                return Err(CodexError::Process);
            }
            if path.exists() {
                let (_, guard) = resolve(path)?;
                let socket =
                    connect_retained(&guard.identity(), running.clone(), scope.clone()).await?;
                return Ok((socket, guard));
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| CodexError::Process)?
}
/// Reconnect only the exact endpoint already retained by this native owner.
/// Never resolve/adopt a replacement alias or silently create a new process.
pub(super) async fn connect_retained(
    identity: &SocketIdentity,
    running: watch::Receiver<bool>,
    scope: WriteScope,
) -> Result<WebSocketStream<NativeStream>, CodexError> {
    timeout(Duration::from_secs(5), async {
        if !*running.borrow() {
            return Err(CodexError::Process);
        }
        identity.validate()?;
        let stream = UnixStream::connect(&identity.0.first().ok_or(CodexError::Configuration)?.0)
            .await
            .map_err(|_| CodexError::Process)?;
        // SAFETY: geteuid has no preconditions.
        if stream.peer_cred().map_err(|_| CodexError::Process)?.uid() != unsafe { libc::geteuid() }
        {
            return Err(CodexError::Configuration);
        }
        identity.validate()?;
        if !*running.borrow() {
            return Err(CodexError::Process);
        }
        let config = WebSocketConfig::default()
            .read_buffer_size(16384)
            .write_buffer_size(16384)
            .max_write_buffer_size(2 * MAX_JSON_BYTES)
            .max_frame_size(Some(MAX_JSON_BYTES))
            .max_message_size(Some(MAX_JSON_BYTES));
        let (socket, _) = tokio_tungstenite::client_async_with_config(
            "ws://localhost/",
            NativeStream::new(stream, scope),
            Some(config),
        )
        .await
        .map_err(|_| CodexError::Protocol)?;
        identity.validate()?;
        Ok(socket)
    })
    .await
    .map_err(|_| CodexError::Process)?
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn refuses_insecure_or_symlinked_native_sockets_and_preserves_replacements() {
        let temp = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        let path = temp.path().join("native.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(check(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = temp.path().join("link.sock");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(check(&link).is_err());
        let guard = check(&path).unwrap();
        let identity = guard.identity();
        assert!(identity.validate().is_ok());
        std::fs::remove_file(&path).unwrap();
        let replacement = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(
            identity.validate().is_err(),
            "reconnect must reject a replaced native endpoint"
        );
        drop(guard);
        assert!(path.exists());
        drop((listener, replacement));
    }
}
