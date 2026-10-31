//! Caller-owned credential updates. The SDK never signs in or stores a credential.
use qgenie_remote_proto::{ErrorCode, RemoteError, rest::AccessToken};
use tokio::{sync::watch, time::Instant};

/// An injected user credential with an optional conservative expiry deadline.
#[derive(Clone, Debug)]
pub struct UserCredential {
    token: AccessToken,
    expires: Option<Instant>,
}
impl UserCredential {
    /// Supplies a token whose expiry is enforced by the remote gateway.
    pub fn fixed(token: AccessToken) -> Self {
        Self {
            token,
            expires: None,
        }
    }
    /// Supplies a token whose local validity also ends at the given deadline.
    pub fn expiring(token: AccessToken, expires: Instant) -> Self {
        Self {
            token,
            expires: Some(expires),
        }
    }
}

/// Publishes replacement credentials from the application's authentication owner.
#[derive(Clone, Debug)]
pub struct CredentialUpdates(watch::Sender<Result<UserCredential, RemoteError>>);
impl CredentialUpdates {
    /// Replaces a credential after a successful exchange. No request is retried here.
    pub fn replace(&self, credential: UserCredential) {
        let _ = self.0.send_replace(Ok(credential));
    }
    /// Stops authenticated work after sign-in, refresh, or credential ownership fails.
    pub fn require_sign_in(&self) {
        let _ = self.0.send_replace(Err(sign_in()));
    }
}

/// Shared access to current credentials supplied by an application.
#[derive(Clone, Debug)]
pub struct UserCredentials {
    updates: watch::Receiver<Result<UserCredential, RemoteError>>,
    // Fixed tokens do not have an external refresh owner whose loss ends validity.
    fixed: Option<CredentialUpdates>,
}
fn sign_in() -> RemoteError {
    RemoteError::rejected(
        ErrorCode::Unauthenticated,
        "sign in again to continue remote access",
    )
}
impl UserCredentials {
    /// Uses an injected token without storing it or refreshing it.
    pub fn fixed(token: AccessToken) -> Self {
        let (updates, mut credentials) = Self::channel(UserCredential::fixed(token));
        credentials.fixed = Some(updates);
        credentials
    }
    /// Creates a credential channel owned by the application's refresh task.
    pub fn channel(initial: UserCredential) -> (CredentialUpdates, Self) {
        let (sender, updates) = watch::channel(Ok(initial));
        (
            CredentialUpdates(sender),
            Self {
                updates,
                fixed: None,
            },
        )
    }
    /// Reads a valid token without performing network or credential-store operations.
    ///
    /// # Errors
    /// Returns unauthenticated after expiry, an explicit sign-in requirement, or loss
    /// of the application's credential publisher.
    pub fn current(&self) -> Result<AccessToken, RemoteError> {
        if self.fixed.is_none() && self.updates.has_changed().is_err() {
            return Err(sign_in());
        }
        let value = self.updates.borrow();
        let credential = value.as_ref().map_err(Clone::clone)?;
        if credential
            .expires
            .is_some_and(|expiry| expiry <= Instant::now())
        {
            return Err(sign_in());
        }
        Ok(credential.token.clone())
    }
    /// Waits for a replacement token, expiry, or the need to sign in again.
    ///
    /// # Errors
    /// Returns unauthenticated on expiry or loss of credential authority. Cancellation
    /// performs no authentication exchange and does not affect native sessions.
    pub async fn changed(&mut self) -> Result<AccessToken, RemoteError> {
        let expiry = self
            .updates
            .borrow()
            .as_ref()
            .map_err(Clone::clone)?
            .expires;
        tokio::select! {
            result = self.updates.changed() => { result.map_err(|_| sign_in())?; }
            _ = async {
                match expiry {
                    Some(expiry) => tokio::time::sleep_until(expiry).await,
                    None => std::future::pending().await,
                }
            } => {}
        }
        self.current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(value: &str) -> AccessToken {
        value.to_owned().try_into().unwrap()
    }

    #[tokio::test]
    async fn rotation_expiry_and_owner_loss_never_return_an_invalid_credential() {
        let (updates, mut credentials) = UserCredentials::channel(UserCredential::expiring(
            token("first-synthetic"),
            Instant::now() + std::time::Duration::from_secs(60),
        ));
        updates.replace(UserCredential::fixed(token("rotated-synthetic")));
        assert_eq!(
            credentials.changed().await.unwrap().as_str(),
            "rotated-synthetic"
        );
        assert!(!format!("{credentials:?}").contains("rotated-synthetic"));
        updates.replace(UserCredential::expiring(
            token("expired-synthetic"),
            Instant::now(),
        ));
        assert_eq!(
            credentials.current().unwrap_err().code,
            ErrorCode::Unauthenticated
        );
        updates.replace(UserCredential::fixed(token("last-synthetic")));
        drop(updates);
        assert!(credentials.current().is_err());
    }

    #[tokio::test]
    async fn sign_in_failure_is_shared_and_fixed_injection_keeps_its_owner() {
        let fixed = UserCredentials::fixed(token("fixed-synthetic"));
        assert!(fixed.current().is_ok());
        let (updates, mut credentials) =
            UserCredentials::channel(UserCredential::fixed(token("test")));
        let copy = credentials.clone();
        updates.require_sign_in();
        assert!(copy.current().is_err());
        assert!(credentials.changed().await.is_err());
    }
}
