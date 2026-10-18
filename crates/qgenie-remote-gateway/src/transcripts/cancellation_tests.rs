use super::{RecordingDatabase, RecordingRole};
use crate::SecretDsn;
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct TlsProxy {
    files: tempfile::TempDir,
    ca: String,
    task: tokio::task::JoinHandle<()>,
    port: u16,
}
impl Drop for TlsProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl TlsProxy {
    async fn start(binding: &SecretDsn) -> Self {
        let files = tempfile::tempdir().unwrap();
        let key = KeyPair::generate().unwrap();
        let mut params =
            CertificateParams::new(vec!["recording-cancel-ca.invalid".into()]).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = params.self_signed(&key).unwrap();
        let leaf_key = KeyPair::generate().unwrap();
        let leaf = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])
            .unwrap()
            .signed_by(&leaf_key, &ca, &key)
            .unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone(), ca.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).into(),
        )
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let url = url::Url::parse(&binding.0).unwrap();
        let host = url.host_str().unwrap().to_owned();
        let target = url.port().unwrap_or(5432);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let budget = Arc::new(tokio::sync::Semaphore::new(8));
            let mut peers = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted=listener.accept()=>{
                        let (mut socket,_)=accepted.unwrap();let Ok(permit)=budget.clone().try_acquire_owned() else {continue;};let acceptor=acceptor.clone();let host=host.clone();
                        peers.spawn(async move {
                            let _permit=permit;let mut request=[0u8;8];
                            socket.read_exact(&mut request).await.unwrap();assert_eq!(request,[0,0,0,8,4,210,22,47],"normal and cancellation connections must negotiate TLS");
                            socket.write_all(b"S").await.unwrap();
                            let Ok(mut tls)=acceptor.accept(socket).await else {return;};
                            let mut backend=tokio::net::TcpStream::connect((host.as_str(),target)).await.unwrap();
                            let _=tokio::io::copy_bidirectional(&mut tls,&mut backend).await;
                        });
                    }
                    Some(result)=peers.join_next(),if !peers.is_empty()=>{result.unwrap();}
                }
            }
        });
        Self {
            files,
            ca: ca.pem(),
            task,
            port,
        }
    }
    fn certificate(&self) -> std::path::PathBuf {
        self.files.path().join("ca.pem")
    }
    fn binding(&self, original: &SecretDsn) -> SecretDsn {
        std::fs::write(self.certificate(), &self.ca).unwrap();
        let mut url = url::Url::parse(&original.0).unwrap();
        url.set_host(Some("127.0.0.1")).unwrap();
        url.set_port(Some(self.port)).unwrap();
        url.set_query(None);
        url.query_pairs_mut()
            .append_pair("sslmode", "verify-full")
            .append_pair("sslrootcert", self.certificate().to_str().unwrap());
        SecretDsn(url.to_string())
    }
}
async fn exercise(database: &RecordingDatabase, certificate: Option<(&Path, &str)>) {
    let mut connection = database.pool.acquire().await.unwrap();
    sqlx::raw_sql("SET statement_timeout='0'; SET transaction_timeout='0'")
        .execute(&mut *connection)
        .await
        .unwrap();
    let cancel = connection.cancel_token();
    assert_eq!(format!("{cancel:?}"), "PgCancelToken([redacted])");
    let late = cancel.clone();
    let sleeping = tokio::spawn(async move {
        sqlx::query("SELECT pg_sleep(30)")
            .execute(&mut *connection)
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    if let Some((path, _)) = certificate {
        let other_key = KeyPair::generate().unwrap();
        let mut params =
            CertificateParams::new(vec!["untrusted-cancel-ca.invalid".into()]).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        std::fs::write(path, params.self_signed(&other_key).unwrap().pem()).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), cancel.cancel_query())
                .await
                .unwrap()
                .is_err(),
            "cancellation must verify the same TLS roots, without plaintext fallback"
        );
    }
    if let Some((path, pem)) = certificate {
        std::fs::write(path, pem).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(2), cancel.cancel_query())
        .await
        .unwrap()
        .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(2), sleeping)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("57014")
    );
    // Pool retirement closes the original frontend before reuse. A delayed copy of its key
    // cannot cancel the next request, including a new PgBouncer frontend on the same backend.
    let mut replacement = database.pool.acquire().await.unwrap();
    let next = tokio::spawn(async move {
        sqlx::query("SELECT pg_sleep(0.2)")
            .execute(&mut *replacement)
            .await
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    tokio::time::timeout(Duration::from_secs(2), late.cancel_query())
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), next)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
}
pub(crate) async fn database_contract(database: &RecordingDatabase, binding: &SecretDsn) {
    exercise(database, None).await;
    let proxy = TlsProxy::start(binding).await;
    let encrypted =
        RecordingDatabase::connect(&proxy.binding(binding), RecordingRole::OwnerRead, 1)
            .await
            .unwrap();
    exercise(&encrypted, Some((proxy.certificate().as_path(), &proxy.ca))).await;
    encrypted.close().await;
}
