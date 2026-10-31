//! Bounded connection recovery. This module never receives an application operation.
use crate::{GatewayClient, LiveClient, UserCredentials};
use qgenie_remote_proto::{ErrorCode, RemoteError};
use std::time::Duration;

/// Only temporary connection/capacity loss is eligible. Denied access needs user action.
pub fn observation_can_reconnect(error: &RemoteError) -> bool {
    matches!(error.code, ErrorCode::AccessStale | ErrorCode::Overloaded)
}
impl GatewayClient {
    /// Open a fresh observation connection with current credentials and bounded backoff.
    /// The caller must discard old controller state and reattach its retained output cursor.
    /// Cancellation drops the connection attempt. No input, approval, or start is resent.
    pub async fn reconnect_observation(
        &self,
        credentials: &UserCredentials,
    ) -> Result<LiveClient, RemoteError> {
        for attempt in 0..6 {
            credentials.current()?;
            tokio::time::sleep(Duration::from_millis(250 << attempt)).await;
            match self.connect_authenticated(credentials).await {
                Ok(client) => return Ok(client),
                Err(error) if attempt == 5 || !observation_can_reconnect(&error) => {
                    return Err(error);
                }
                Err(_) => {}
            }
        }
        unreachable!("the final connection attempt returns its result")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn access_denial_and_protocol_errors_never_enter_recovery() {
        for code in [
            ErrorCode::Unauthenticated,
            ErrorCode::Forbidden,
            ErrorCode::InvalidFrame,
            ErrorCode::UnsupportedVersion,
            ErrorCode::HistoryTruncated,
        ] {
            assert!(!observation_can_reconnect(&RemoteError::rejected(
                code, "test"
            )));
        }
        assert!(observation_can_reconnect(&RemoteError::rejected(
            ErrorCode::AccessStale,
            "test"
        )));
    }
}
