use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn machine_changes_never_retry_and_invalid_success_remains_uncertain() {
    for scenario in 0..4 {
        let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let ca = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(ca.path(), certificate.cert.pem()).unwrap();
        let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.key_pair.serialize_der())
                .into(),
        )
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = GatewayClient::new(GatewayConfig {
            origin: format!(
                "https://localhost:{}",
                listener.local_addr().unwrap().port()
            ),
            ca_bundle: Some(ca.path().into()),
        })
        .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let machine = MachineId::new();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_rustls::TlsAcceptor::from(Arc::new(tls))
                .accept(socket)
                .await
                .unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                assert!(header.len() < 8192);
                header.push(socket.read_u8().await.unwrap());
            }
            let header = String::from_utf8(header).unwrap();
            assert!(header.starts_with(&if scenario == 3 {
                format!("POST /v1/machines/{machine}/revoke ")
            } else {
                format!("PATCH /v1/machines/{machine} ")
            }));
            let length = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            assert!(length < 1024);
            let mut body = vec![0; length];
            socket.read_exact(&mut body).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            if scenario == 3 {
                assert_eq!(body["confirmed"], true);
            } else {
                assert_eq!(body["name"], "once");
            }
            counter.fetch_add(1, Ordering::SeqCst);
            if scenario != 0 {
                let body = match scenario {
                    1 => "{invalid success".to_owned(),
                    2 => serde_json::to_string(&serde_json::json!({"id":MachineId::new(),"owner":UserId::new(),"name":"once","hostname":"fixture","os":"linux","arch":"x86_64","cli_version":"fixture","labels":{},"agents":{},"online":false,"last_seen":"2026-10-18T00:00:00Z","sessions":0})).unwrap(),
                    _ => serde_json::json!({"machine":MachineId::new()}).to_string(),
                };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                let _ = socket.shutdown().await;
            }
            // A second request is a failing retry, even after the response is lost.
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let token: AccessToken = "synthetic-machine-owner".to_owned().try_into().unwrap();
        let invalid = api
            .patch_machine(
                &token,
                machine,
                qgenie_remote_proto::rest::MachinePatch {
                    name: Some(String::new()),
                    labels: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(invalid.outcome, OperationOutcome::NotSent));
        assert!(matches!(
            api.revoke_machine(&token, machine, false)
                .await
                .unwrap_err()
                .outcome,
            OperationOutcome::NotSent
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let result = if scenario == 3 {
            api.revoke_machine(&token, machine, true).await
        } else {
            api.patch_machine(
                &token,
                machine,
                qgenie_remote_proto::rest::MachinePatch {
                    name: Some("once".into()),
                    labels: None,
                },
            )
            .await
            .map(|_| ())
        };
        assert!(matches!(
            result.unwrap_err().outcome,
            OperationOutcome::Unknown
        ));
        server.await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
