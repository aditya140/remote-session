use super::stream::PgStream;
use crate::{Error, PgConnectOptions};
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

/// Opaque connection-specific PostgreSQL/PgBouncer cancellation capability.
/// No startup/authentication query is sent on the cancellation socket. TLS policy,
/// certificate roots, client identity and destination are inherited unchanged.
#[derive(Clone)]
pub struct PgCancelToken {
    process_id: u32,
    secret_key: u32,
    options: Arc<PgConnectOptions>,
    requested: Arc<AtomicBool>,
}
impl fmt::Debug for PgCancelToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PgCancelToken([redacted])")
    }
}
impl PgCancelToken {
    pub(super) fn new(
        process_id: u32,
        secret_key: u32,
        options: Arc<PgConnectOptions>,
        requested: Arc<AtomicBool>,
    ) -> Self {
        Self {
            process_id,
            secret_key,
            options,
            requested,
        }
    }
    /// Best-effort cancellation, not proof of rollback or transaction outcome. In particular,
    /// cancelling synchronous COMMIT can end its wait after it committed locally. Callers
    /// must enforce their own deadline and never return a durable receipt after cancellation.
    /// Keep the original frontend checked out and retire it, avoiding cancellation of a
    /// later operation on the same pooled frontend. PgBouncer routes its virtual key.
    pub async fn cancel_query(&self) -> Result<(), Error> {
        self.requested.store(true, Ordering::Release);
        let mut stream = PgStream::connect(&self.options).await?;
        let mut packet = [0u8; 16];
        packet[..4].copy_from_slice(&16u32.to_be_bytes());
        packet[4..8].copy_from_slice(&80877102u32.to_be_bytes());
        packet[8..12].copy_from_slice(&self.process_id.to_be_bytes());
        packet[12..].copy_from_slice(&self.secret_key.to_be_bytes());
        stream.write_buffer_mut().put_slice(&packet);
        stream.flush().await?;
        // CancelRequest has no response payload: the server closes this new connection
        // after processing it. Do not shut down early and race request transmission.
        match stream.read_buffered(1).await {
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(()),
            Err(error) => Err(error),
            Ok(_) => Err(Error::Protocol(
                "unexpected PostgreSQL cancellation response".into(),
            )),
        }
    }
}
