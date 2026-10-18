//! Standalone composition. The connector never owns or supervises native children.
use anyhow::{Context, Result, ensure};
use clap::{Args as ClapArgs, Subcommand};
use fs2::FileExt;
use qgenie_remote_client::Client;
use qgenie_remote_host::{
    Binding, BindingStatus, ConnectionStatus, Connector, CredentialOptions, GatewayApi, Identity,
    MachineMetadata, NativeClient, Protection, RECORDING_DISCLOSURE, RecordingConsent,
    filesystem::{Filesystem, Roots},
    oauth::{OAuth, OAuthConfig},
};
use qgenie_remote_proto::{decode_json, ipc::RemoteScope};
use qgenie_remote_sessiond::{Config, RedactionRules};
use qgenie_remote_transport::ipc::{private_directory, private_file};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::sync::watch;
mod service;

#[derive(ClapArgs)]
pub(crate) struct Args {
    #[arg(long)]
    config: PathBuf,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Resume the host using user services, or independent background processes explicitly.
    Up {
        #[arg(long)]
        no_service: bool,
    },
    /// Install or manage separate user services. Connector operations preserve sessiond.
    Service {
        #[command(subcommand)]
        command: service::Action,
    },
    /// Enroll with an injected current remote user credential; no user token is stored.
    Enroll {
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long)]
        acknowledge_recording: bool,
    },
    /// Scoped PKCE/device sign-in and enrollment; refresh tokens are discarded.
    Login {
        #[arg(long)]
        oauth_config: PathBuf,
        #[arg(long)]
        device: bool,
        #[arg(long)]
        acknowledge_recording: bool,
    },
    /// Run one machine-only outbound connector in the foreground (native owner is separate).
    Connect,
    /// Pause only this connector. Running sessions and their recording remain owned by sessiond.
    Down {
        #[arg(long)]
        keep_sessions: bool,
        #[arg(long)]
        terminate_sessions: bool,
    },
    /// Show current local enrollment, connector lock and native IPC state.
    Status,
    /// Verify native IPC, allowed roots and an actual TLS machine-authentication exchange.
    Doctor,
    /// Revoke this enrollment using an owner credential; preserves native sessions.
    Unenroll {
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long)]
        yes: bool,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub gateway: String,
    pub os_user: String,
    pub name: String,
    pub hostname: String,
    pub roots: Vec<PathBuf>,
    #[serde(default)]
    pub ca_bundle: Option<PathBuf>,
    #[serde(default)]
    pub identity_directory: Option<PathBuf>,
    #[serde(default)]
    pub allow_private_file: bool,
    #[serde(default)]
    pub redaction_file: Option<PathBuf>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub codex: Option<CodexSettings>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CodexSettings {
    binary: PathBuf,
    endpoint: String,
    model: String,
    credential_file: PathBuf,
}
pub(crate) fn read_private<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .context("cannot open private host configuration/state")?;
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions.
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= 65536,
        "host configuration/state must be an owned private regular file, at most 64 KiB"
    );
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(65537)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 65536,
        "host configuration/state exceeds its budget"
    );
    decode_json(&bytes).map_err(Into::into)
}
impl Settings {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let settings: Self = read_private(path)?;
        ensure!(
            path.is_absolute(),
            "host configuration path must be absolute"
        );
        GatewayApi::new(&settings.gateway, settings.ca_bundle.as_deref())?;
        ensure!(
            !settings.os_user.is_empty()
                && settings.os_user.len() <= 128
                && !settings.os_user.chars().any(char::is_control),
            "invalid OS user binding"
        );
        ensure!(
            !settings.name.is_empty()
                && settings.name.len() <= 256
                && !settings.name.chars().any(char::is_control)
                && !settings.hostname.is_empty()
                && settings.hostname.len() <= 256
                && !settings.hostname.chars().any(char::is_control),
            "invalid machine metadata"
        );
        ensure!(
            settings.ca_bundle.as_ref().is_none_or(|p| p.is_absolute())
                && settings
                    .identity_directory
                    .as_ref()
                    .is_none_or(|p| p.is_absolute())
                && settings
                    .redaction_file
                    .as_ref()
                    .is_none_or(|p| p.is_absolute()),
            "configured host paths must be absolute"
        );
        Roots::open(&settings.roots)?;
        if let Some(codex) = &settings.codex {
            ensure!(
                codex.binary.is_absolute() && codex.credential_file.is_absolute(),
                "Codex binary and credential paths must be absolute"
            );
            ensure!(
                codex.endpoint.starts_with("https://")
                    && codex.endpoint.len() <= 2048
                    && !codex.model.is_empty()
                    && codex.model.len() <= 128,
                "Codex requires a bounded HTTPS provider endpoint and model"
            );
            #[cfg(not(all(feature = "managed-codex", target_os = "linux")))]
            anyhow::bail!(
                "this build does not support managed Codex; enable managed-codex on Linux"
            );
        }
        Ok(settings)
    }
    fn identity_dir(&self, state: &Path) -> PathBuf {
        self.identity_directory
            .clone()
            .unwrap_or_else(|| state.join("identity"))
    }
    async fn identity(&self, state: &Path) -> Result<Identity> {
        Ok(Identity::open(
            &self.identity_dir(state),
            &self.gateway,
            &self.os_user,
            CredentialOptions {
                prefer_keyring: true,
                allow_private_file: self.allow_private_file,
            },
        )
        .await?)
    }
    fn binding(&self, state: &Path) -> Result<Binding> {
        read_private(&self.identity_dir(state).join("binding.json"))
    }
    fn scope(&self, binding: &Binding) -> RemoteScope {
        RemoteScope {
            origin: self.gateway.clone(),
            owner: binding.owner,
            machine: binding.machine,
        }
    }
    fn metadata(&self) -> MachineMetadata {
        MachineMetadata {
            hostname: self.hostname.clone(),
            name: self.name.clone(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            cli_version: env!("CARGO_PKG_VERSION").into(),
            labels: self.labels.clone(),
        }
    }
}
/// Native service reads only trusted local binding/configuration, never machine signing keys.
pub(crate) fn native_config(state: &Path, path: &Path) -> Result<Config> {
    let settings = Settings::load(path)?;
    let binding = settings.binding(state)?;
    ensure!(
        binding.status == BindingStatus::Enrolled,
        "enroll before starting the remote native owner"
    );
    let mut config = Config::new(state.to_owned(), settings.roots.clone());
    config.remote_bindings.push(settings.scope(&binding));
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    if let Some(codex) = &settings.codex {
        // The connector validates paths but never opens the provider secret.
        // Only the independent native owner injects it into its owned child.
        let credential: String = read_private(&codex.credential_file)?;
        config.codex = Some(qgenie_remote_sessiond::CodexProfile::new(
            codex.binary.clone(),
            codex.endpoint.clone(),
            codex.model.clone(),
            credential,
        ));
    }
    if let Some(path) = &settings.redaction_file {
        let rules: Vec<String> = read_private(path)?;
        config.recording.redaction =
            RedactionRules::literals(rules.into_iter().map(String::into_bytes).collect())?;
    }
    Ok(config)
}
async fn shutdown_signal() -> Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {result=tokio::signal::ctrl_c()=>result?,_=terminate.recv()=>{}}
    Ok(())
}
async fn enrollment(
    settings: &Settings,
    state: &Path,
    access: &qgenie_remote_proto::rest::AccessToken,
    acknowledged: bool,
) -> Result<()> {
    ensure!(
        acknowledged,
        "enrollment requires --acknowledge-recording after reading the recording disclosure"
    );
    let mut identity = settings.identity(state).await?;
    if identity.protection() == Protection::PrivateFile {
        eprintln!(
            "Machine key protection: private 0600 file (OS credential store unavailable or not previously selected)."
        );
    }
    let binding = GatewayApi::new(&settings.gateway, settings.ca_bundle.as_deref())?
        .enroll(
            &mut identity,
            access,
            settings.metadata(),
            RecordingConsent::acknowledged(),
        )
        .await?;
    println!("{}", serde_json::to_string_pretty(&binding)?);
    Ok(())
}
pub(crate) async fn run(args: Args, state: &Path) -> Result<()> {
    let settings = Settings::load(&args.config)?;
    private_directory(state)?;
    let api = GatewayApi::new(&settings.gateway, settings.ca_bundle.as_deref())?;
    match args.command {
        Action::Up { no_service } => {
            if no_service {
                background_up(&settings, state, &args.config).await?;
            } else {
                service::run(service::Action::Up, state, &args.config).await?;
            }
        }
        Action::Service { command } => service::run(command, state, &args.config).await?,
        Action::Enroll {
            token_file,
            acknowledge_recording,
        } => {
            eprintln!("{RECORDING_DISCLOSURE}");
            let token = super::read_remote_token(&token_file)?;
            enrollment(&settings, state, &token, acknowledge_recording).await?;
        }
        Action::Login {
            oauth_config,
            device,
            acknowledge_recording,
        } => {
            eprintln!("{RECORDING_DISCLOSURE}");
            ensure!(
                acknowledge_recording,
                "login/enrollment requires --acknowledge-recording"
            );
            let oauth = OAuth::new(OAuthConfig::from_file(&oauth_config)?)?;
            let (_stop, mut cancel) = watch::channel(false);
            let login = async {
                let mut tokens = if device {
                    let pending = oauth.device(&mut cancel).await?;
                    eprintln!(
                        "Open {} and enter {}",
                        pending.verification_uri(),
                        pending.user_code()
                    );
                    pending.wait(&mut cancel).await?
                } else {
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
                    pending.wait(&mut cancel).await?
                };
                drop(tokens.take_refresh());
                enrollment(&settings, state, &tokens.access, true).await
            };
            tokio::select! {result=login=>result?,result=shutdown_signal()=>{result?;anyhow::bail!("sign-in cancelled");}}
        }
        Action::Connect => {
            super::set_private_umask();
            let lock = private_file(&state.join("connector.lock"))?;
            lock.try_lock_exclusive()
                .context("a connector already owns this state directory")?;
            let stop_file = state.join("connector.stop");
            if stop_file.try_exists()? {
                let _ = std::fs::remove_file(&stop_file);
            }
            let identity = settings.identity(state).await?;
            let credentials = identity.machine_credentials()?;
            let scope = settings.scope(credentials.binding());
            drop(identity);
            let connector = Connector::new(
                api,
                credentials,
                Filesystem::new(Roots::open(&settings.roots)?),
                settings.ca_bundle.clone(),
                true,
            )?
            .with_native(NativeClient::new(state.to_owned(), scope)?)?;
            let (stop, mut cancel) = watch::channel(false);
            let (status, mut updates) = watch::channel(ConnectionStatus::Disconnected);
            let connect = connector.run(&mut cancel, status);
            tokio::pin!(connect);
            let mut ticks = tokio::time::interval(Duration::from_millis(250));
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let stopping = async {
                loop {
                    ticks.tick().await;
                    if stop_file.try_exists().unwrap_or(false) {
                        break;
                    }
                }
            };
            tokio::pin!(stopping);
            let signal = shutdown_signal();
            tokio::pin!(signal);
            loop {
                tokio::select! {
                    result=&mut connect=>{result?;break;},
                    changed=updates.changed()=>{if changed.is_ok(){eprintln!("Connector: {:?}",*updates.borrow());}},
                result=&mut signal=>{result?;stop.send_replace(true);tokio::time::timeout(Duration::from_secs(3),&mut connect).await.context("connector shutdown timed out")??;break;},
                    _=&mut stopping=>{stop.send_replace(true);tokio::time::timeout(Duration::from_secs(3),&mut connect).await.context("connector pause timed out")??;break;},
                }
            }
        }
        Action::Down {
            keep_sessions,
            terminate_sessions,
        } => {
            ensure!(
                keep_sessions != terminate_sessions,
                "choose --keep-sessions or --terminate-sessions explicitly"
            );
            // The control file requests cancellation of the independently locked connector;
            // no PID read from disk is ever signalled.
            let file = private_file(&state.join("connector.stop"))?;
            file.sync_all()?;
            let lock = private_file(&state.join("connector.lock"))?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                match lock.try_lock_exclusive() {
                    Ok(()) => break,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && tokio::time::Instant::now() < deadline =>
                    {
                        tokio::time::sleep(Duration::from_millis(50)).await
                    }
                    Err(error) => {
                        return Err(error).context(
                            "connector has not confirmed pause; native sessions were preserved",
                        );
                    }
                }
            }
            if terminate_sessions {
                let binding = settings.binding(state)?;
                let sessions = Client::local(state).sessions().await?;
                for session in sessions {
                    if session.machine == binding.machine
                        && matches!(
                            session.state,
                            qgenie_remote_proto::SessionState::Running
                                | qgenie_remote_proto::SessionState::Idle
                                | qgenie_remote_proto::SessionState::Accepted
                                | qgenie_remote_proto::SessionState::Launching
                        )
                    {
                        Client::local(state).terminate(session.id, true).await?;
                    }
                }
            }
            println!("Connector stopped. Native session owner remains independent.");
        }
        Action::Unenroll { token_file, yes } => {
            ensure!(yes, "unenrollment requires --yes");
            let mut identity = settings.identity(state).await?;
            api.unenroll(&mut identity, &super::read_remote_token(&token_file)?, true)
                .await?;
            println!("Enrollment revoked. Native sessions remain owned locally.");
        }
        Action::Status | Action::Doctor => {
            let doctor = matches!(args.command, Action::Doctor);
            let binding = settings.binding(state)?;
            let lock = private_file(&state.join("connector.lock"))?;
            let connector = match lock.try_lock_exclusive() {
                Ok(()) => false,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => true,
                Err(error) => return Err(error.into()),
            };
            let status = Client::local(state).status().await;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"binding":binding,"connector_process_locked":connector,"native":status.as_ref().ok(),"native_ipc_available":status.is_ok()})
                )?
            );
            if doctor {
                let native = status?;
                let expected: std::collections::BTreeSet<_> = Roots::open(&settings.roots)?
                    .paths()
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                ensure!(
                    native
                        .roots
                        .into_iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        == expected,
                    "native allowed roots differ from host configuration; defer native policy updates until an explicit idle upgrade"
                );
                let identity = settings.identity(state).await?;
                api.authenticate_machine(&identity.machine_credentials()?)
                    .await?;
                println!(
                    "PASS verified TLS machine authentication; PASS current native IPC and allowed roots. No remote mutation was issued."
                );
            }
        }
    }
    Ok(())
}

async fn background_up(settings: &Settings, state: &Path, config: &Path) -> Result<()> {
    use std::{
        io::{Seek, SeekFrom},
        os::unix::process::CommandExt,
    };
    let identity = settings.identity(state).await?;
    let binding = identity
        .binding()
        .cloned()
        .context("enroll before starting the host")?;
    ensure!(
        binding.status == BindingStatus::Enrolled,
        "enrollment is not active"
    );
    drop(identity);
    let executable = std::env::current_exe()?;
    let client = Client::local(state);
    let spawn = |role: &str| -> Result<std::process::Child> {
        let mut log = private_file(&state.join(format!("{role}.log")))?;
        log.seek(SeekFrom::End(0))?;
        let mut command = std::process::Command::new(&executable);
        command.arg("--state-dir").arg(state);
        if role == "sessiond" {
            command.arg("sessiond").arg("--host-config").arg(config);
        } else {
            command
                .arg("host")
                .arg("--config")
                .arg(config)
                .arg("connect");
        }
        Ok(command
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .process_group(0)
            .spawn()?)
    };
    if client.status().await.is_err() {
        let mut child = spawn("sessiond")?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if client.status().await.is_ok() {
                break;
            }
            ensure!(
                child.try_wait()?.is_none(),
                "native owner exited; inspect sessiond.log"
            );
            ensure!(
                tokio::time::Instant::now() < deadline,
                "native owner startup timed out; inspect sessiond.log"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    NativeClient::new(state.to_owned(), settings.scope(&binding))?
        .register_binding()
        .await?;
    let lock = private_file(&state.join("connector.lock"))?;
    match lock.try_lock_exclusive() {
        Ok(()) => {
            FileExt::unlock(&lock)?;
            let mut child = spawn("connector")?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                match lock.try_lock_exclusive() {
                    Ok(()) => FileExt::unlock(&lock)?,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(error.into()),
                }
                ensure!(
                    child.try_wait()?.is_none(),
                    "connector exited; inspect connector.log"
                );
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "connector startup timed out; inspect connector.log"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(error) => return Err(error.into()),
    }
    println!(
        "Independent native owner and connector are running without a service manager. Use host status/doctor to verify connection authority. Logout/reboot supervision is unavailable in this mode."
    );
    Ok(())
}
