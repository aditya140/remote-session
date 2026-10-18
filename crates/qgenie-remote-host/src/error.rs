#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("invalid host or identity-provider configuration")]
    Configuration,
    #[error("request failed; delivery or token exchange may be uncertain")]
    Unavailable,
    #[error("authentication denied or expired")]
    Denied,
    #[error("sign-in cancelled")]
    Cancelled,
    #[error("sign-in expired")]
    Expired,
    #[error("credential store unavailable")]
    CredentialStore,
    #[error("machine key missing or changed; explicit recovery is required")]
    KeyLost,
    #[error("private enrollment state is invalid or inaccessible")]
    State,
    #[error("another enrollment operation owns the state directory")]
    Busy,
    #[error("enrolled identity was revoked; explicit recovery is required")]
    Revoked,
    #[error("gateway rejected the operation")]
    Rejected(qgenie_remote_proto::RemoteError),
    #[error("invalid or oversized service response")]
    Protocol,
}
