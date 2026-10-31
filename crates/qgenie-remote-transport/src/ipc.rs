use fs2::FileExt;
use qgenie_remote_proto::MAX_JSON_BYTES;
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    time::{Duration, timeout},
};

pub fn current_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and does not dereference pointers.
    unsafe { libc::geteuid() }
}

pub fn private_directory(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(denied("state directory must be absolute"));
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.uid() != current_uid() || metadata.mode() & 0o077 != 0 {
        return Err(denied(
            "state directory must be an owned, private directory (0700)",
        ));
    }
    // Resolve system-owned aliases such as macOS /var -> /private/var.
    // The final directory itself must still be a real private directory.
    let canonical = fs::canonicalize(path)?;
    for ancestor in canonical.ancestors().skip(1) {
        let metadata = fs::symlink_metadata(ancestor)?;
        if metadata.file_type().is_symlink() {
            return Err(denied("symlink in state directory ancestors"));
        }
        if metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0 {
            return Err(denied("state directory has an unsafe writable ancestor"));
        }
    }
    Ok(())
}

pub fn private_file(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != current_uid() || metadata.mode() & 0o077 != 0 {
        return Err(denied("state file must be owned and private (0600)"));
    }
    Ok(file)
}

pub struct Listener {
    listener: UnixListener,
    path: PathBuf,
    inode: u64,
    _lock: File,
}
impl Listener {
    pub fn bind(path: &Path) -> io::Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| denied("socket has no parent"))?;
        private_directory(parent)?;
        if path.as_os_str().as_bytes().len() > 100 {
            return Err(denied("socket path exceeds portable limit of 100 bytes"));
        }
        let lock = private_file(&path.with_extension("lock"))?;
        lock.try_lock_exclusive().map_err(|_| {
            io::Error::new(
                io::ErrorKind::AddrInUse,
                "another session daemon owns this endpoint",
            )
        })?;
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if !metadata.file_type().is_socket() || metadata.uid() != current_uid() {
                return Err(denied("unsafe existing socket"));
            }
            refuse_live_socket(path)?;
            fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        let inode = fs::symlink_metadata(path)?.ino();
        Ok(Self {
            listener,
            path: path.to_owned(),
            inode,
            _lock: lock,
        })
    }
    pub async fn accept(&self) -> io::Result<UnixStream> {
        let (stream, _) = self.listener.accept().await?;
        if stream.peer_cred()?.uid() != current_uid() {
            return Err(denied("foreign-user IPC peer"));
        }
        Ok(stream)
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.ino() == self.inode
            && metadata.file_type().is_socket()
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub async fn connect(path: &Path) -> io::Result<UnixStream> {
    private_directory(
        path.parent()
            .ok_or_else(|| denied("socket has no parent"))?,
    )?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != current_uid()
        || metadata.mode() & 0o077 != 0
    {
        return Err(denied("unsafe IPC socket permissions"));
    }
    let stream = timeout(Duration::from_secs(5), UnixStream::connect(path)).await??;
    if stream.peer_cred()?.uid() != current_uid() {
        return Err(denied("foreign-user IPC server"));
    }
    Ok(stream)
}

pub async fn read_frame(stream: &mut UnixStream) -> io::Result<Vec<u8>> {
    timeout(Duration::from_secs(10), async {
        let length = stream.read_u32().await? as usize;
        if length == 0 || length > MAX_JSON_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid IPC frame length",
            ));
        }
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes).await?;
        Ok(bytes)
    })
    .await?
}
pub async fn write_frame(stream: &mut UnixStream, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > MAX_JSON_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid IPC frame length",
        ));
    }
    timeout(Duration::from_secs(10), async {
        stream.write_u32(bytes.len() as u32).await?;
        stream.write_all(bytes).await?;
        stream.flush().await
    })
    .await?
}
fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn refuse_live_socket(path: &Path) -> io::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // SAFETY: socket has no borrowed arguments; the returned descriptor is owned below.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: raw is a newly created descriptor with exclusive ownership.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: fcntl is applied to a live, owned socket descriptor.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a zeroed sockaddr_un is valid once family and a bounded NUL-terminated path are supplied.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as _;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = std::mem::size_of::<libc::sockaddr_un>() as u8;
    }
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= address.sun_path.len() {
        return Err(denied("socket path too long"));
    }
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as _;
    }
    // SAFETY: address has the matching family/size and remains alive throughout connect.
    let connected = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of::<libc::sockaddr_un>() as _,
        )
    };
    if connected == 0 {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "existing live IPC socket",
        ));
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ECONNREFUSED) | Some(libc::ENOENT) => Ok(()),
        _ => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "existing IPC socket cannot be proven stale",
        )),
    }
}

/// A fresh socket must negotiate before any local or remote operation is sent.
pub async fn client_handshake(stream: &mut UnixStream) -> io::Result<()> {
    use qgenie_remote_proto::{IPC_MAJOR, ipc::DispatchFrame};
    write_dispatch(
        stream,
        &DispatchFrame::Hello {
            major: IPC_MAJOR,
            minor: 0,
            capabilities: vec!["local-v1".into()],
        },
    )
    .await?;
    match read_dispatch(stream).await? {
        DispatchFrame::Welcome {
            major: IPC_MAJOR,
            minor: 0,
            ..
        } => Ok(()),
        DispatchFrame::Error(error) => Err(io::Error::new(io::ErrorKind::InvalidData, error)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incompatible IPC welcome",
        )),
    }
}
pub async fn server_handshake(stream: &mut UnixStream) -> io::Result<()> {
    server_handshake_with_capabilities(stream, 0, &["local-v1"])
        .await
        .map(|_| ())
}
/// Negotiate only features owned by the serving component. Legacy minor-zero
/// peers retain the exact 1.0 welcome, including its omitted capability field.
pub async fn server_handshake_with_capabilities(
    stream: &mut UnixStream,
    minor: u16,
    supported: &[&str],
) -> io::Result<Vec<String>> {
    use qgenie_remote_proto::{IPC_MAJOR, ipc::DispatchFrame};
    let hello = read_dispatch(stream).await?;
    if let Err(error) = hello.validate_hello() {
        let _ = write_dispatch(stream, &DispatchFrame::Error(error.clone())).await;
        return Err(io::Error::new(io::ErrorKind::InvalidData, error));
    }
    let DispatchFrame::Hello {
        minor: peer_minor,
        capabilities: requested,
        ..
    } = hello
    else {
        unreachable!()
    };
    let minor = minor.min(peer_minor);
    let capabilities = if minor == 0 {
        Vec::new()
    } else {
        requested
            .into_iter()
            .filter(|cap| supported.contains(&cap.as_str()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    };
    write_dispatch(
        stream,
        &DispatchFrame::Welcome {
            major: IPC_MAJOR,
            minor,
            capabilities: capabilities.clone(),
        },
    )
    .await?;
    Ok(capabilities)
}
/// A required component feature must be acknowledged before sending any RPC.
pub async fn client_handshake_with_capabilities(
    stream: &mut UnixStream,
    required: &[&str],
) -> io::Result<()> {
    client_handshake_with_options(stream, required, &[])
        .await
        .map(|_| ())
}
/// Optional typed formats can be negotiated without breaking older PTY owners.
/// Only the acknowledged requested intersection may be used on this connection.
pub async fn client_handshake_with_options(
    stream: &mut UnixStream,
    required: &[&str],
    optional: &[&str],
) -> io::Result<Vec<String>> {
    use qgenie_remote_proto::{IPC_MAJOR, IPC_MINOR, ipc::DispatchFrame};
    let hello = DispatchFrame::Hello {
        major: IPC_MAJOR,
        minor: IPC_MINOR,
        capabilities: required
            .iter()
            .chain(optional)
            .map(|cap| (*cap).to_owned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
    };
    hello
        .validate_hello()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    write_dispatch(stream, &hello).await?;
    match read_dispatch(stream).await? {
        DispatchFrame::Welcome {
            major: IPC_MAJOR,
            minor: IPC_MINOR,
            capabilities,
        } if capabilities.len() <= 64
            && capabilities.iter().all(|cap| {
                required.contains(&cap.as_str()) || optional.contains(&cap.as_str())
            })
            && capabilities
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == capabilities.len()
            && required
                .iter()
                .all(|cap| capabilities.iter().any(|offered| offered == cap)) =>
        {
            Ok(capabilities)
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "required native IPC capabilities were not negotiated; no RPC sent",
        )),
    }
}
pub async fn read_dispatch(
    stream: &mut UnixStream,
) -> io::Result<qgenie_remote_proto::ipc::DispatchFrame> {
    qgenie_remote_proto::decode_json(&read_frame(stream).await?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
pub async fn write_dispatch(
    stream: &mut UnixStream,
    frame: &qgenie_remote_proto::ipc::DispatchFrame,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(frame).map_err(io::Error::other)?;
    write_frame(stream, &bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn roundtrip_private_peer_and_exclusive_listener() {
        let temp = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        let path = temp.path().join("s.sock");
        let listener = Listener::bind(&path).unwrap();
        assert!(Listener::bind(&path).is_err());
        let peer = tokio::spawn(async move {
            let mut stream = listener.accept().await.unwrap();
            assert_eq!(read_frame(&mut stream).await.unwrap(), b"hello");
            write_frame(&mut stream, b"world").await.unwrap();
        });
        let mut stream = connect(&path).await.unwrap();
        write_frame(&mut stream, b"hello").await.unwrap();
        assert_eq!(read_frame(&mut stream).await.unwrap(), b"world");
        peer.await.unwrap();
    }
    #[tokio::test]
    async fn negotiation_versions_stale_socket_and_interrupted_reads() {
        use qgenie_remote_proto::ipc::DispatchFrame;
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let peer = tokio::spawn(async move {
            server_handshake(&mut server).await.unwrap();
        });
        client_handshake(&mut client).await.unwrap();
        peer.await.unwrap();
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let peer = tokio::spawn(async move {
            assert!(server_handshake(&mut server).await.is_err());
        });
        write_dispatch(
            &mut client,
            &DispatchFrame::Hello {
                major: 2,
                minor: 0,
                capabilities: vec![],
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            read_dispatch(&mut client).await.unwrap(),
            DispatchFrame::Error(_)
        ));
        peer.await.unwrap();
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client.write_all(&[0, 0]).await.unwrap();
        drop(client);
        assert_eq!(
            read_frame(&mut server).await.unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        let temp = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        let path = temp.path().join("stale.sock");
        let live = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(Listener::bind(&path).is_err());
        assert!(path.exists());
        drop(live);
        let recovered = Listener::bind(&path).unwrap();
        drop(recovered);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn required_native_features_are_acknowledged_before_any_rpc() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let peer = tokio::spawn(async move {
            server_handshake(&mut server).await.unwrap();
            assert_eq!(
                read_frame(&mut server).await.unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
        });
        assert!(
            client_handshake_with_capabilities(&mut client, &["native-authority-v1"])
                .await
                .is_err()
        );
        drop(client);
        peer.await.unwrap();
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let peer = tokio::spawn(async move {
            let chosen = server_handshake_with_capabilities(
                &mut server,
                qgenie_remote_proto::IPC_MINOR,
                &["local-v1", "native-authority-v1"],
            )
            .await
            .unwrap();
            assert_eq!(chosen, vec!["native-authority-v1"]);
        });
        client_handshake_with_capabilities(&mut client, &["native-authority-v1"])
            .await
            .unwrap();
        peer.await.unwrap();
    }
    #[tokio::test]
    async fn optional_formats_preserve_pty_compatibility_and_unrequested_formats_fail() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let peer = tokio::spawn(async move {
            let caps = server_handshake_with_capabilities(
                &mut server,
                qgenie_remote_proto::IPC_MINOR,
                &["native-replay-v1"],
            )
            .await
            .unwrap();
            assert_eq!(caps, ["native-replay-v1"]);
        });
        assert_eq!(
            client_handshake_with_options(&mut client, &["native-replay-v1"], &["native-agent-v1"])
                .await
                .unwrap(),
            ["native-replay-v1"]
        );
        peer.await.unwrap();
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let peer = tokio::spawn(async move {
            read_dispatch(&mut server).await.unwrap();
            write_dispatch(
                &mut server,
                &qgenie_remote_proto::ipc::DispatchFrame::Welcome {
                    major: qgenie_remote_proto::IPC_MAJOR,
                    minor: qgenie_remote_proto::IPC_MINOR,
                    capabilities: vec!["native-replay-v1".into(), "unrequested-format".into()],
                },
            )
            .await
            .unwrap();
        });
        assert!(
            client_handshake_with_options(&mut client, &["native-replay-v1"], &["native-agent-v1"])
                .await
                .is_err()
        );
        peer.await.unwrap();
    }
    #[tokio::test]
    async fn rejects_oversized_length_before_payload_allocation() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_u32((MAX_JSON_BYTES + 1) as u32).await.unwrap();
        assert!(read_frame(&mut reader).await.is_err());
    }
    #[test]
    fn rejects_insecure_directory_and_symlink_file() {
        let temp = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_directory(temp.path()).is_err());
        let path = temp.path().join("link");
        std::os::unix::fs::symlink("/etc/passwd", &path).unwrap();
        assert!(private_file(&path).is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "helper for the isolated root/foreign-UID proof; never run directly"]
    fn foreign_uid_fixture_child() {
        use std::io::{Read, Write};
        assert_eq!(current_uid(), 1001);
        let path = std::env::var_os("QGR_IPC_TEST_SOCKET").expect("fixture socket");
        let mode = std::env::var("QGR_IPC_FOREIGN_MODE").expect("fixture mode");
        if mode == "server" {
            let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
            println!("fixture-ready");
            std::io::stdout().flush().unwrap();
            let _ = listener.accept().unwrap();
            return;
        }
        let stream = std::os::unix::net::UnixStream::connect(path);
        if mode == "filesystem" {
            assert_eq!(stream.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            return;
        }
        assert_eq!(mode, "peer");
        let mut stream = stream.unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        // The server can reject the foreign UID before this first write.
        if let Err(error) = stream.write_all(b"fixture") {
            assert!(
                matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe
                ),
                "unexpected foreign peer write error: {error}"
            );
            return;
        }
        let mut byte = [0];
        match stream.read(&mut byte) {
            Ok(0) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe
                ) => {}
            other => panic!("foreign peer unexpectedly received data: {other:?}"),
        }
    }
    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "run only inside disposable root container with QGR_ALLOW_ROOT_IPC_PROOF=yes"]
    async fn actual_foreign_uid_is_denied_by_filesystem_and_both_peer_checks() {
        use tokio::io::AsyncBufReadExt;
        assert_eq!(current_uid(), 0);
        assert_eq!(
            std::env::var("QGR_ALLOW_ROOT_IPC_PROOF").as_deref(),
            Ok("yes")
        );
        fn child(path: &Path, mode: &str) -> tokio::process::Command {
            let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "ipc::tests::foreign_uid_fixture_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("QGR_IPC_TEST_SOCKET", path)
                .env("QGR_IPC_FOREIGN_MODE", mode)
                .uid(1001)
                .gid(1001)
                .kill_on_drop(true);
            command
        }
        let temp = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        let path = temp.path().join("s.sock");
        let listener = Listener::bind(&path).unwrap();
        assert!(
            timeout(Duration::from_secs(5), child(&path, "filesystem").status())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        // Weaken only this disposable fixture's DAC to reach the independent
        // SO_PEERCRED check. Production permissions are covered above.
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o711)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
        let mut peer = child(&path, "peer").spawn().unwrap();
        let result = timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert!(
            timeout(Duration::from_secs(5), peer.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        let server_dir = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        fs::set_permissions(server_dir.path(), fs::Permissions::from_mode(0o777)).unwrap();
        let server_path = server_dir.path().join("server.sock");
        let mut server = child(&server_path, "server")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = tokio::io::BufReader::new(server.stdout.take().unwrap()).lines();
        timeout(Duration::from_secs(5), async {
            loop {
                let line = output
                    .next_line()
                    .await
                    .unwrap()
                    .expect("server exited before ready");
                if line == "fixture-ready" {
                    break;
                }
            }
        })
        .await
        .unwrap();
        // Make inode ownership and mode pass the client-side file checks while
        // retaining the actual foreign credentials of the listening process.
        std::os::unix::fs::chown(&server_path, Some(0), Some(0)).unwrap();
        fs::set_permissions(&server_path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(server_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            connect(&server_path).await.unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(
            timeout(Duration::from_secs(5), server.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }
}
