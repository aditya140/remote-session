#![cfg(all(feature = "client-ws", feature = "server-ws"))]
use futures_util::SinkExt;
use qgenie_remote_proto::{Counter, PtyFrame, PtyPayload, SessionId};
use qgenie_remote_transport::ws::{
    self, ClientConfig, Connection, Error, Frame, Limits, Priority, SecretHeader,
};
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use rustls::pki_types::PrivatePkcs8KeyDer;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::Message;

struct Certificates {
    _dir: tempfile::TempDir,
    ca: std::path::PathBuf,
    acceptor: TlsAcceptor,
}
impl Certificates {
    fn new() -> Self {
        let ca_key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec!["test-ca.invalid".into()]).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = params.self_signed(&ca_key).unwrap();
        let key = KeyPair::generate().unwrap();
        let params = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        let cert = params.signed_by(&key, &ca, &ca_key).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone(), ca.der().clone()],
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let ca_path = dir.path().join("ca.pem");
        std::fs::write(&ca_path, ca.pem()).unwrap();
        Self {
            _dir: dir,
            ca: ca_path,
            acceptor: TlsAcceptor::from(Arc::new(config)),
        }
    }
    fn client(&self, address: SocketAddr) -> ClientConfig {
        ClientConfig::new(&format!("wss://localhost:{}/socket", address.port()))
            .unwrap()
            .proxy(None)
            .unwrap()
            .no_proxy("")
            .ca_bundle(&self.ca)
    }
}
async fn server(
    cert: &Certificates,
    limits: Limits,
) -> (SocketAddr, oneshot::Receiver<Connection>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let acceptor = cert.acceptor.clone();
    let (tx, rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        if let Ok(tls) = acceptor.accept(tcp).await
            && let Ok(connection) = ws::accept(tls, limits, None).await
        {
            let _ = tx.send(connection);
        }
    });
    (address, rx, task)
}
fn output() -> Frame {
    Frame::Binary(PtyFrame {
        session: SessionId::new(),
        seq: Counter(1),
        payload: PtyPayload::Output(b"hello".to_vec()),
    })
}
async fn exchange(client: &Connection, server: &mut Connection) {
    let receipt = client
        .sender()
        .try_send(Priority::Control, output())
        .unwrap();
    timeout(Duration::from_secs(2), receipt.transmitted())
        .await
        .unwrap()
        .unwrap();
    let received = timeout(Duration::from_secs(2), server.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(received.frame,Frame::Binary(PtyFrame { payload:PtyPayload::Output(ref bytes),.. }) if bytes==b"hello")
    );
}
async fn proxy(
    target: SocketAddr,
    tls: Option<TlsAcceptor>,
    idle: Duration,
) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        if let Some(tls) = tls {
            proxy_stream(tls.accept(stream).await.unwrap(), target, idle).await;
        } else {
            proxy_stream(stream, target, idle).await;
        }
    });
    (address, task)
}
async fn proxy_stream<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    target: SocketAddr,
    idle: Duration,
) {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        assert!(header.len() < 16 * 1024);
        header.push(stream.read_u8().await.unwrap());
    }
    let header = String::from_utf8(header).unwrap();
    assert!(header.starts_with(&format!("CONNECT localhost:{} HTTP/1.1\r\n", target.port())));
    assert!(header.contains("Proxy-Authorization: Basic dXNlcjpwYXNz\r\n"));
    assert!(!header.contains("Bearer"));
    let mut upstream = TcpStream::connect(target).await.unwrap();
    stream
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await
        .unwrap();
    let mut down = [0u8; 16 * 1024];
    let mut up = [0u8; 16 * 1024];
    loop {
        let transfer = async {
            tokio::select! {
                count=stream.read(&mut down)=> {
                    let count=count.ok()?;if count==0 { return None; }
                    upstream.write_all(&down[..count]).await.ok()?;Some(())
                },
                count=upstream.read(&mut up)=> {
                    let count=count.ok()?;if count==0 { return None; }
                    stream.write_all(&up[..count]).await.ok()?;Some(())
                },
            }
        };
        if !matches!(timeout(idle, transfer).await, Ok(Some(()))) {
            break;
        }
    }
}

#[tokio::test]
async fn verified_tls_direct_connect_proxy_https_proxy_and_bypass() {
    let cert = Certificates::new();
    for scheme in ["direct", "http", "https", "bypass"] {
        let (address, rx, server_task) = server(&cert, Limits::default()).await;
        let mut config = cert
            .client(address)
            .authorization(SecretHeader::bearer("synthetic-token").unwrap());
        let mut proxy_task = None;
        if matches!(scheme, "http" | "https") {
            let (proxy_address, task) = proxy(
                address,
                if scheme == "https" {
                    Some(cert.acceptor.clone())
                } else {
                    None
                },
                Duration::from_secs(60),
            )
            .await;
            proxy_task = Some(task);
            config = config
                .proxy(Some(&format!(
                    "{scheme}://user:pass@localhost:{}",
                    proxy_address.port()
                )))
                .unwrap();
        } else if scheme == "bypass" {
            config = config
                .proxy(Some("http://127.0.0.1:1"))
                .unwrap()
                .no_proxy("localhost");
        }
        let client = ws::connect(config, Limits::default(), None).await.unwrap();
        let mut remote = rx.await.unwrap();
        exchange(&client, &mut remote).await;
        assert_eq!(client.close().await.unwrap().code, 1000);
        assert_eq!(
            timeout(Duration::from_secs(2), remote.closed())
                .await
                .unwrap()
                .unwrap()
                .code,
            1000
        );
        server_task.await.unwrap();
        if let Some(task) = proxy_task {
            timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap();
        }
    }
}
#[tokio::test]
async fn untrusted_ca_and_plaintext_are_rejected() {
    let cert = Certificates::new();
    let (address, _rx, task) = server(&cert, Limits::default()).await;
    let config = ClientConfig::new(&format!("wss://localhost:{}", address.port()))
        .unwrap()
        .proxy(None)
        .unwrap();
    assert!(matches!(
        ws::connect(config, Limits::default(), None).await,
        Err(Error::Tls)
    ));
    task.await.unwrap();
    let config = ClientConfig::new("ws://127.0.0.1:1")
        .unwrap()
        .proxy(None)
        .unwrap();
    assert!(matches!(
        ws::connect(config, Limits::default(), None).await,
        Err(Error::Configuration)
    ));
}
#[tokio::test]
async fn slow_observer_disconnects_without_stalling_a_second_connection() {
    let cert = Certificates::new();
    let limits = Limits {
        incoming_messages: 1,
        incoming_bytes: 100,
        ..Limits::default()
    };
    let (address, rx, _) = server(&cert, limits).await;
    let client = ws::connect(cert.client(address), Limits::default(), None)
        .await
        .unwrap();
    let mut slow = rx.await.unwrap();
    for _ in 0..3 {
        let _ = client
            .sender()
            .try_send(Priority::Bulk, output())
            .unwrap()
            .transmitted()
            .await;
    }
    assert!(matches!(
        timeout(Duration::from_secs(2), slow.closed())
            .await
            .unwrap(),
        Err(Error::SlowConsumer)
    ));
    let (address, rx, _) = server(&cert, Limits::default()).await;
    let second = ws::connect(cert.client(address), Limits::default(), None)
        .await
        .unwrap();
    let mut remote = rx.await.unwrap();
    exchange(&second, &mut remote).await;
    second.close().await.unwrap();
}
#[tokio::test]
async fn malformed_oversized_dropped_pong_and_owner_drop_are_bounded() {
    for payload in [
        Message::Text("{bad-json".into()),
        Message::Binary(vec![0; 1024 * 1024 + 1].into()),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut remote = ws::accept(tcp, Limits::default(), None).await.unwrap();
            timeout(Duration::from_secs(2), remote.closed())
                .await
                .unwrap()
        });
        let (mut raw, _) = tokio_tungstenite::client_async(
            format!("ws://{address}"),
            TcpStream::connect(address).await.unwrap(),
        )
        .await
        .unwrap();
        let _ = raw.send(payload).await;
        assert!(matches!(task.await.unwrap(), Err(Error::Protocol(_))));
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let raw = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        tokio_tungstenite::accept_async(tcp).await.unwrap()
    });
    let config = ClientConfig::new(&format!("ws://{address}"))
        .unwrap()
        .proxy(None)
        .unwrap()
        .allow_insecure_loopback(true);
    let limits = Limits {
        heartbeat_interval: Duration::from_millis(60),
        pong_timeout: Duration::from_millis(20),
        ..Limits::default()
    };
    let mut client = ws::connect(config, limits, None).await.unwrap();
    let mut raw = raw.await.unwrap();
    // No peer reads, so tungstenite cannot answer Ping.
    assert!(matches!(
        timeout(Duration::from_secs(2), client.closed())
            .await
            .unwrap(),
        Err(Error::HeartbeatTimeout)
    ));
    assert!(
        timeout(Duration::from_secs(2), raw.close(None))
            .await
            .is_ok()
    );
    let cert = Certificates::new();
    let (address, rx, _) = server(&cert, Limits::default()).await;
    let client = ws::connect(cert.client(address), Limits::default(), None)
        .await
        .unwrap();
    let mut remote = rx.await.unwrap();
    let orphan_sender = client.sender();
    drop(client);
    let _ = timeout(Duration::from_secs(2), remote.closed())
        .await
        .unwrap();
    assert!(orphan_sender.try_send(Priority::Control, output()).is_err());
}
#[tokio::test]
async fn heartbeats_keep_a_sixty_second_idle_proxy_alive() {
    let cert = Certificates::new();
    let (address, rx, _) = server(&cert, Limits::default()).await;
    let (proxy_address, task) = proxy(address, None, Duration::from_secs(60)).await;
    let config = cert
        .client(address)
        .proxy(Some(&format!(
            "http://user:pass@localhost:{}",
            proxy_address.port()
        )))
        .unwrap();
    let client = ws::connect(config, Limits::default(), None).await.unwrap();
    let mut remote = rx.await.unwrap();
    sleep(Duration::from_secs(65)).await;
    exchange(&client, &mut remote).await;
    client.close().await.unwrap();
    remote.closed().await.unwrap();
    timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn rejected_or_oversized_proxy_responses_are_sanitized() {
    for (reply, expected) in [
        (
            b"HTTP/1.1 407 Authentication required\r\nSecret: never-log\r\n\r\n".to_vec(),
            407,
        ),
        (vec![b'x'; 16 * 1024 + 1], 0),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(stream.read_u8().await.unwrap());
            }
            stream.write_all(&reply).await.unwrap();
        });
        let config = ClientConfig::new("wss://localhost:443")
            .unwrap()
            .proxy(Some(&format!("http://{address}")))
            .unwrap()
            .no_proxy("");
        let error = ws::connect(config, Limits::default(), None)
            .await
            .err()
            .unwrap();
        if expected == 407 {
            assert!(matches!(error, Error::ProxyRejected(407)));
        } else {
            assert!(matches!(error, Error::ProxyProtocol));
        }
        assert!(!format!("{error:?} {error}").contains("never-log"));
        task.await.unwrap();
    }
}
