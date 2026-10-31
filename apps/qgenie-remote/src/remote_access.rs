//! Saved connection settings and client-owned, in-memory OAuth credentials.
use anyhow::{Context, Result, ensure};
use clap::{Args as ClapArgs, Subcommand};
use qgenie_remote_client::{GatewayClient, GatewayConfig, UserCredential, UserCredentials};
use qgenie_remote_host::oauth::{OAuth, OAuthConfig, Tokens};
use qgenie_remote_transport::ipc::{private_directory, private_file};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, IsTerminal, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle, time::Instant};

#[derive(ClapArgs)]
pub(crate) struct Args {
    #[arg(long, global = true)]
    gateway: Option<String>,
    #[arg(long, global = true, conflicts_with = "oauth_config")]
    token_file: Option<PathBuf>,
    #[arg(long, global = true)]
    oauth_config: Option<PathBuf>,
    #[arg(long, global = true)]
    ca_bundle: Option<PathBuf>,
    /// Use a device code instead of opening a browser.
    #[arg(long, global = true)]
    device: bool,
    #[command(subcommand)]
    command: Option<Action>,
}

#[derive(Subcommand)]
enum Action {
    /// Save connection settings, then use `remote` to sign in and open the TUI.
    Setup,
    #[command(flatten)]
    Operation(super::remote_cli::Action),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    gateway: String,
    ca_bundle: Option<PathBuf>,
    oauth_config: Option<PathBuf>,
    #[serde(default)]
    device: bool,
}

pub(crate) fn save_settings<T: Serialize>(
    state: &Path,
    name: &str,
    value: &T,
    replace_existing: bool,
) -> Result<()> {
    use fs2::FileExt;
    private_directory(state)?;
    let lock = private_file(&state.join("settings.lock"))?;
    lock.try_lock_exclusive()
        .context("another setup is writing settings")?;
    let pending = state.join(format!(
        "settings-{}.pending",
        qgenie_remote_proto::RequestId::new()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&pending)?;
        let bytes = serde_json::to_vec_pretty(value)?;
        ensure!(bytes.len() <= 65536, "settings exceed 64 KiB");
        file.write_all(&bytes)?;
        file.sync_all()?;
        if replace_existing {
            std::fs::rename(&pending, state.join(name))?;
        } else {
            // Atomic no-replace publication also handles a concurrent setup or a dangling symlink.
            std::fs::hard_link(&pending, state.join(name))?;
            std::fs::remove_file(&pending)?;
        }
        std::fs::File::open(state)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&pending);
    }
    result
}

fn prompt(label: &str) -> Result<String> {
    ensure!(
        std::io::stdin().is_terminal(),
        "{label} is required; supply its command option in scripts"
    );
    eprint!("{label}: ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().take(4097).read_line(&mut answer)?;
    ensure!(answer.len() <= 4096, "setup input exceeds its limit");
    Ok(answer.trim().to_owned())
}

fn load_profile(args: &Args, state: &Path) -> Result<Profile> {
    let path = state.join("remote.json");
    let saved: Option<Profile> = if path.try_exists()? {
        private_directory(state)?;
        Some(super::host::read_private(&path)?)
    } else {
        None
    };
    // An explicit different gateway starts a fresh binding. Never reuse its old OAuth/CA selectors.
    let saved = saved.filter(|saved| {
        args.gateway
            .as_ref()
            .is_none_or(|origin| origin == &saved.gateway)
    });
    let setup = matches!(args.command, Some(Action::Setup));
    let gateway = match args
        .gateway
        .clone()
        .or_else(|| saved.as_ref().map(|p| p.gateway.clone()))
    {
        Some(origin) => origin,
        None if setup => prompt("Gateway HTTPS origin")?,
        None => anyhow::bail!("run `qgenie-remote remote setup` or supply --gateway"),
    };
    let mut oauth_config = args
        .oauth_config
        .clone()
        .or_else(|| saved.as_ref().and_then(|p| p.oauth_config.clone()));
    if setup && oauth_config.is_none() && args.token_file.is_none() {
        oauth_config = Some(PathBuf::from(prompt("OAuth configuration file")?));
    }
    let ca_bundle = args
        .ca_bundle
        .clone()
        .or_else(|| saved.as_ref().and_then(|p| p.ca_bundle.clone()));
    let cwd = std::env::current_dir()?;
    let profile = Profile {
        gateway,
        ca_bundle: ca_bundle.map(|path| super::absolute(path, &cwd)),
        oauth_config: oauth_config.map(|path| super::absolute(path, &cwd)),
        device: args.device || saved.as_ref().is_some_and(|p| p.device),
    };
    GatewayClient::new(profile.gateway_config())?;
    Ok(profile)
}
impl Profile {
    fn gateway_config(&self) -> GatewayConfig {
        GatewayConfig {
            origin: self.gateway.clone(),
            ca_bundle: self.ca_bundle.clone(),
        }
    }
    fn oauth(&self) -> Result<OAuth> {
        let path = self
            .oauth_config
            .as_ref()
            .context("run `remote setup` with --oauth-config, or supply --token-file")?;
        let config: OAuthConfig = super::host::read_private(path)?;
        OAuth::new(config).map_err(Into::into)
    }
}

pub(crate) async fn sign_in(oauth: &OAuth, device: bool) -> Result<Tokens> {
    let (_stop, mut cancel) = watch::channel(false);
    if device {
        let pending = oauth.device(&mut cancel).await?;
        eprintln!(
            "Open {} and enter {}",
            pending.verification_uri(),
            pending.user_code()
        );
        return pending.wait(&mut cancel).await.map_err(Into::into);
    }
    let pending = oauth.browser().await?;
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let status = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new(opener)
            .arg(pending.authorization_url())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await
    .context("browser opener timed out; use --device")??;
    ensure!(status.success(), "cannot open browser; use --device");
    pending.wait(&mut cancel).await.map_err(Into::into)
}

/// The owning command ends refresh on cancellation, normal exit, or error.
struct Login {
    credentials: UserCredentials,
    task: Option<JoinHandle<()>>,
}
impl Drop for Login {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
impl Login {
    fn refreshing(oauth: OAuth, mut tokens: Tokens) -> Self {
        let (updates, credentials) = UserCredentials::channel(UserCredential::expiring(
            tokens.access.clone(),
            tokens.expires,
        ));
        let task = tokio::spawn(async move {
            let mut refresh = tokens.take_refresh();
            loop {
                let lifetime = tokens.expires.saturating_duration_since(Instant::now());
                if refresh.is_none() {
                    tokio::time::sleep_until(tokens.expires).await;
                    updates.require_sign_in();
                    return;
                }
                let margin = (lifetime / 5).min(Duration::from_secs(60));
                tokio::time::sleep_until(tokens.expires - margin).await;
                let Some(token) = refresh.as_ref() else {
                    return;
                };
                match oauth.refresh(token).await {
                    Ok(mut next) => {
                        if let Some(rotated) = next.take_refresh() {
                            refresh = Some(rotated);
                        }
                        updates
                            .replace(UserCredential::expiring(next.access.clone(), next.expires));
                        tokens = next;
                    }
                    Err(_) => {
                        updates.require_sign_in();
                        return;
                    }
                }
            }
        });
        Self {
            credentials,
            task: Some(task),
        }
    }
}

pub(crate) async fn run(args: Args, state: &Path) -> Result<()> {
    let profile = load_profile(&args, state)?;
    if matches!(args.command, Some(Action::Setup)) {
        ensure!(
            args.token_file.is_none(),
            "setup saves connection settings only; use --oauth-config"
        );
        profile.oauth()?;
        save_settings(state, "remote.json", &profile, true)?;
        println!(
            "Remote setup saved. Run `qgenie-remote remote` to sign in. User credentials are kept only while the client runs."
        );
        return Ok(());
    }
    let authenticate = async {
        if let Some(path) = args.token_file {
            Ok(Login {
                credentials: UserCredentials::fixed(super::read_remote_token(&path)?),
                task: None,
            })
        } else {
            ensure!(
                profile.device || std::io::stdin().is_terminal(),
                "interactive sign-in requires a terminal; use --device or --token-file"
            );
            let oauth = profile.oauth()?;
            let tokens = sign_in(&oauth, profile.device).await?;
            Ok::<_, anyhow::Error>(Login::refreshing(oauth, tokens))
        }
    };
    let login = tokio::select! { result = authenticate => result?, result = super::host::shutdown_signal() => { result?; anyhow::bail!("sign-in cancelled") } };
    let config = profile.gateway_config();
    match args.command {
        Some(Action::Operation(command)) => {
            let work = super::remote_cli::run_authenticated(
                config,
                login.credentials.clone(),
                state,
                command,
            );
            tokio::select! { result = work => result, result = super::host::shutdown_signal() => { result?; anyhow::bail!("remote command cancelled; native sessions continue") } }
        }
        // The TUI owns keyboard/signal handling and terminal restoration.
        None => {
            let view = qgenie_remote_tui::run_remote_authenticated(
                config,
                login.credentials.clone(),
                state,
            );
            tokio::select! { result = view => result, result = super::host::shutdown_signal() => result }
        }
        Some(Action::Setup) => unreachable!("setup returns before authentication"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[tokio::test]
    async fn refresh_rotates_in_memory_and_a_failed_exchange_is_not_retried() {
        use axum::{Form, Json, Router, extract::State, http::StatusCode, routing::post};
        use serde_json::json;
        use std::{
            collections::HashMap,
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
            },
        };
        let count = Arc::new(AtomicUsize::new(0));
        async fn exchange(
            State(count): State<Arc<AtomicUsize>>,
            Form(fields): Form<HashMap<String, String>>,
        ) -> (StatusCode, Json<serde_json::Value>) {
            assert_eq!(fields["grant_type"], "refresh_token");
            let n = count.fetch_add(1, Ordering::SeqCst);
            assert_eq!(fields["refresh_token"], ["seed", "first", "second"][n]);
            if n == 2 {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error":"invalid_grant"})),
                );
            }
            // Unchanged scope may be omitted from refresh responses.
            (
                StatusCode::OK,
                Json(
                    json!({"access_token":format!("synthetic-{n}"),"refresh_token":(["first","second"][n]),"token_type":"Bearer","expires_in":1}),
                ),
            )
        }
        let tls = crate::test_identity::Tls::new();
        let server = tls
            .bind()
            .await
            .serve(
                Router::new()
                    .route("/token", post(exchange))
                    .with_state(count.clone()),
            )
            .await;
        let oauth = OAuth::new(OAuthConfig {
            client_id: "client".into(),
            remote_scope: "api://test/Remote.Control".into(),
            authorization_endpoint: format!("{}/authorize", server.origin),
            token_endpoint: format!("{}/token", server.origin),
            device_endpoint: format!("{}/device", server.origin),
            device_pkce: false,
            ca_bundle: Some(tls.ca.path().into()),
        })
        .unwrap();
        let tokens = oauth.refresh("seed").await.unwrap();
        let login = Login::refreshing(oauth, tokens);
        let mut credentials = login.credentials.clone();
        assert_eq!(credentials.current().unwrap().as_str(), "synthetic-0");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), credentials.changed())
                .await
                .unwrap()
                .unwrap()
                .as_str(),
            "synthetic-1"
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), credentials.changed())
                .await
                .unwrap()
                .unwrap_err()
                .code,
            qgenie_remote_proto::ErrorCode::Unauthenticated
        );
        tokio::time::sleep(Duration::from_millis(900)).await;
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert!(!format!("{credentials:?}").contains("synthetic-"));
    }
    #[test]
    fn gateway_override_does_not_reuse_saved_identity_and_settings_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let saved = Profile {
            gateway: "https://first.example".into(),
            ca_bundle: None,
            oauth_config: Some("/private/first.json".into()),
            device: true,
        };
        save_settings(root.path(), "remote.json", &saved, true).unwrap();
        let args = super::super::Args::try_parse_from([
            "qgenie-remote",
            "remote",
            "--gateway",
            "https://second.example",
            "--token-file",
            "/private/token",
            "machines",
        ])
        .unwrap();
        let super::super::Action::Remote(args) = args.command.unwrap() else {
            panic!("remote")
        };
        let resolved = load_profile(&args, root.path()).unwrap();
        assert!(resolved.oauth_config.is_none());
        assert!(!resolved.device);
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(root.path().join("remote.json"))
                .unwrap()
                .mode()
                & 0o777,
            0o600
        );
    }
}
