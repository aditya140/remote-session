use qgenie_remote_proto::ErrorCode;

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("invalid transport configuration")]
    Configuration,
    #[error("network connection failed")]
    Network,
    #[error("TLS certificate verification or negotiation failed")]
    Tls,
    #[error("proxy CONNECT was rejected with HTTP status {0}")]
    ProxyRejected(u16),
    #[error("invalid or oversized proxy response")]
    ProxyProtocol,
    #[error("WebSocket upgrade was rejected with HTTP status {0}")]
    UpgradeRejected(u16),
    #[error("WebSocket protocol or frame validation failed ({0:?})")]
    Protocol(ErrorCode),
    #[error("connection heartbeat timed out")]
    HeartbeatTimeout,
    #[error("slow receiver exceeded its bounded queue; resynchronize")]
    SlowConsumer,
    #[error("network write deadline exceeded")]
    WriteTimeout,
    #[error("connection owner task failed")]
    Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseReason {
    pub code: u16,
    pub remote: bool,
}
