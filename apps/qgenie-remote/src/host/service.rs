//! Service managers supervise two independent jobs; native upgrades require an idle shutdown.
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use fs2::FileExt;
use qgenie_remote_client::Client;
use qgenie_remote_proto::{
    IPC_MAJOR, RequestId, decode_json,
    ipc::{AdminCommand, AdminRequest, AdminResponse},
};
use qgenie_remote_transport::ipc::{
    connect, private_directory, private_file, read_frame, write_frame,
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

const NATIVE: &str = "qgenie-remote-sessiond";
const CONNECTOR: &str = "qgenie-remote-connector";
#[derive(Subcommand)]
pub(super) enum Action {
    /// Generate private user units/plists and pin this executable. Does not start/restart jobs.
    Install {
        #[arg(long)]
        directory: Option<PathBuf>,
    },
    /// Idempotently start native ownership and resume/restart only the connector.
    Up,
    /// Stop only the connector job, preserving native processes and capture.
    Pause,
    /// Upgrade sessiond only after its atomic administrative shutdown confirms it is idle.
    UpgradeNative {
        #[arg(long)]
        yes: bool,
    },
    /// Ask the OS user-service manager for both jobs' actual state.
    Status,
}
fn directory() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is required for user services")?;
    ensure!(home.is_absolute(), "HOME must be absolute");
    Ok(if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("systemd/user")
    })
}
fn checked_path(path: &Path) -> Result<&str> {
    let value = path.to_str().context("service paths must be UTF-8")?;
    ensure!(
        path.is_absolute() && value.len() <= 4096 && !value.chars().any(char::is_control),
        "service paths must be bounded absolute paths without control characters"
    );
    Ok(value)
}
fn unit_arg(path: &Path) -> Result<String> {
    Ok(format!(
        "\"{}\"",
        checked_path(path)?
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    ))
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn definitions(binary: &Path, state: &Path, config: &Path) -> Result<[(String, String); 2]> {
    let mut definitions = Vec::new();
    for (name, native) in [(NATIVE, true), (CONNECTOR, false)] {
        let mut arguments = vec![
            binary.to_owned(),
            PathBuf::from("--state-dir"),
            state.to_owned(),
        ];
        if native {
            arguments.extend([
                PathBuf::from("sessiond"),
                PathBuf::from("--host-config"),
                config.to_owned(),
            ]);
        } else {
            arguments.extend([
                PathBuf::from("host"),
                PathBuf::from("--config"),
                config.to_owned(),
                PathBuf::from("connect"),
            ]);
        }
        checked_path(binary)?;
        checked_path(state)?;
        checked_path(config)?;
        if cfg!(target_os = "macos") {
            let args = arguments
                .iter()
                .map(|arg| {
                    Ok(format!(
                        "<string>{}</string>",
                        xml(arg.to_str().context("invalid service argument")?)
                    ))
                })
                .collect::<Result<Vec<_>>>()?
                .join("");
            let log = state.join(format!("{name}.log"));
            private_file(&log)?;
            definitions.push((format!("com.qgenie.{name}.plist"),format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>com.qgenie.{name}</string><key>ProgramArguments</key><array>{args}</array><key>RunAtLoad</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict><key>ThrottleInterval</key><integer>3</integer><key>Umask</key><integer>63</integer><key>StandardOutPath</key><string>{}</string><key>StandardErrorPath</key><string>{}</string></dict></plist>\n",xml(checked_path(&log)?),xml(checked_path(&log)?))));
        } else {
            let args = arguments
                .iter()
                .map(|arg| {
                    if arg.is_absolute() {
                        unit_arg(arg)
                    } else {
                        Ok(arg.to_str().context("invalid service argument")?.to_owned())
                    }
                })
                .collect::<Result<Vec<_>>>()?
                .join(" ");
            definitions.push((format!("{name}.service"),format!("[Unit]\nDescription=QGenie Remote {}\nAfter=network-online.target{}\n\n[Service]\nType=simple\nExecStart={args}\nRestart=on-failure\nRestartSec=3\nKillMode=control-group\nTimeoutStopSec=10\nUMask=0077\n{}\n[Install]\nWantedBy=default.target\n",if native {"native session owner"}else{"outbound connector"},if native {""}else{" qgenie-remote-sessiond.service"},if native {""}else{"NoNewPrivileges=true\n"})));
        }
    }
    definitions
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid service definitions"))
}
fn digest_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    ensure!(
        file.metadata()?.len() <= 1024 * 1024 * 1024,
        "executable exceeds pinning budget"
    );
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0usize;
    loop {
        let bytes = file.read(&mut buffer)?;
        if bytes == 0 {
            break;
        }
        total += bytes;
        ensure!(
            total <= 1024 * 1024 * 1024,
            "executable changed beyond budget"
        );
        hash.update(&buffer[..bytes]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn pin_executable(source: &Path, destination: &Path, expected: &str) -> Result<()> {
    let parent = destination
        .parent()
        .context("pinned executable has no parent")?;
    let temporary = parent.join(format!(".{}", RequestId::new()));
    let result = (|| -> Result<()> {
        let mut source = std::fs::File::open(source)?;
        let mut target = private_file(&temporary)?;
        let copied = std::io::copy(
            &mut Read::by_ref(&mut source).take(1024 * 1024 * 1024 + 1),
            &mut target,
        )?;
        ensure!(
            copied <= 1024 * 1024 * 1024 && digest_file(&temporary)? == expected,
            "executable changed while pinning"
        );
        target.set_permissions(std::fs::Permissions::from_mode(0o700))?;
        target.sync_all()?;
        std::fs::rename(&temporary, destination)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
fn atomic(path: &Path, bytes: &[u8], executable: bool) -> Result<()> {
    let parent = path.parent().context("service path has no parent")?;
    if path.try_exists()? {
        let metadata = std::fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_file()
                && metadata.uid() == qgenie_remote_transport::ipc::current_uid()
                && metadata.mode() & 0o077 == 0,
            "existing service file must be owned, private and regular"
        );
    }
    let temporary = parent.join(format!(".{}", RequestId::new()));
    let result = (|| -> Result<()> {
        let mut file = private_file(&temporary)?;
        file.write_all(bytes)?;
        if executable {
            file.set_permissions(std::fs::Permissions::from_mode(0o700))?;
        }
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
async fn install(state: &Path, config: &Path, units: &Path, owner_sealed: bool) -> Result<bool> {
    ensure!(units.is_absolute(), "service directory must be absolute");
    super::native_config(state, config)?;
    private_directory(state)?;
    let lock = private_file(&state.join("services.lock"))?;
    lock.try_lock_exclusive()
        .context("another service update is active")?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(units)?;
    let metadata = std::fs::symlink_metadata(units)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == qgenie_remote_transport::ipc::current_uid()
            && metadata.mode() & 0o022 == 0,
        "unsafe user service directory"
    );
    let versions = state.join("versions");
    private_directory(&versions)?;
    let executable = std::env::current_exe()?;
    let hash = digest_file(&executable)?;
    let pinned = versions.join(format!("qgenie-remote-{hash}"));
    if !pinned.try_exists()? {
        pin_executable(&executable, &pinned, &hash)?;
    } else {
        ensure!(
            digest_file(&pinned)? == hash,
            "pinned executable has changed"
        );
    }
    // Native service configuration is frozen with the version. Updating the source
    // config cannot silently change a still-running owner's next restart arguments.
    let bytes = std::fs::read(config)?;
    ensure!(bytes.len() <= 65536, "service configuration exceeds budget");
    let frozen = versions.join(format!("host-{:x}.json", Sha256::digest(&bytes)));
    atomic(&frozen, &bytes, false)?;
    let definitions = definitions(&pinned, state, &frozen)?;
    let active = Client::local(state)
        .sessions()
        .await
        .map(|sessions| {
            sessions.iter().any(|s| {
                matches!(
                    s.state,
                    qgenie_remote_proto::SessionState::Accepted
                        | qgenie_remote_proto::SessionState::Launching
                        | qgenie_remote_proto::SessionState::Running
                        | qgenie_remote_proto::SessionState::Idle
                )
            })
        })
        .unwrap_or(true);
    let mut deferred = false;
    for (index, (name, body)) in definitions.into_iter().enumerate() {
        let path = units.join(name);
        let previous = std::fs::read(&path).ok();
        if index == 0
            && !owner_sealed
            && active
            && previous
                .as_deref()
                .is_some_and(|previous| previous != body.as_bytes())
        {
            deferred = true;
            continue;
        }
        atomic(&path, body.as_bytes(), false)?;
    }
    println!(
        "Service definitions: {}\nPinned executable: {}\nNative update deferred: {deferred}",
        units.display(),
        pinned.display()
    );
    Ok(deferred)
}
async fn command(program: &str, args: &[String]) -> Result<()> {
    let status = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new(program)
            .args(args)
            .kill_on_drop(true)
            .status(),
    )
    .await
    .context("user service manager timed out")??;
    ensure!(
        status.success(),
        "user service manager failed; verify the active user login/service-manager environment"
    );
    Ok(())
}
async fn manager_available() -> Result<()> {
    let (program, args) = if cfg!(target_os = "macos") {
        (
            "launchctl",
            vec![
                "print".into(),
                format!("gui/{}", qgenie_remote_transport::ipc::current_uid()),
            ],
        )
    } else {
        (
            "systemctl",
            vec![
                "--user".into(),
                "show".into(),
                "--property=Version".into(),
                "--value".into(),
            ],
        )
    };
    let status = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(program)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await
    .context("user service manager unavailable; use explicit --no-service process mode")??;
    ensure!(
        status.success(),
        "user service manager unavailable; use explicit --no-service process mode"
    );
    Ok(())
}
async fn manage(verb: &str, name: &str, units: &Path) -> Result<()> {
    if cfg!(target_os = "macos") {
        let domain = format!("gui/{}", qgenie_remote_transport::ipc::current_uid());
        let label = format!("com.qgenie.{name}");
        let args = match verb {
            "start" => vec![
                "bootstrap".into(),
                domain,
                checked_path(&units.join(format!("{label}.plist")))?.into(),
            ],
            "stop" => vec!["bootout".into(), format!("{domain}/{label}")],
            "status" => vec!["print".into(), format!("{domain}/{label}")],
            _ => anyhow::bail!("unsupported launchd operation"),
        };
        command("launchctl", &args).await
    } else {
        command(
            "systemctl",
            &["--user".into(), verb.into(), format!("{name}.service")],
        )
        .await
    }
}
async fn idle_shutdown(state: &Path) -> Result<()> {
    let mut stream = connect(&state.join("admin.sock")).await?;
    write_frame(
        &mut stream,
        &serde_json::to_vec(&AdminRequest {
            ipc_major: IPC_MAJOR,
            command: AdminCommand::Shutdown {
                terminate_sessions: false,
            },
        })?,
    )
    .await?;
    let response: AdminResponse = decode_json(&read_frame(&mut stream).await?)?;
    response.result?;
    Ok(())
}
pub(super) async fn run(action: Action, state: &Path, config: &Path) -> Result<()> {
    match action {
        Action::Install { directory: custom } => {
            install(
                state,
                config,
                &custom.map(Ok).unwrap_or_else(directory)?,
                false,
            )
            .await?;
        }
        Action::Up => {
            manager_available().await?;
            let units = directory()?;
            install(state, config, &units, false).await?;
            if cfg!(target_os = "linux") {
                command("systemctl", &["--user".into(), "daemon-reload".into()]).await?;
                command(
                    "systemctl",
                    &[
                        "--user".into(),
                        "enable".into(),
                        format!("{NATIVE}.service"),
                        format!("{CONNECTOR}.service"),
                    ],
                )
                .await?;
                manage("start", NATIVE, &units).await?;
                manage("restart", CONNECTOR, &units).await?;
            } else {
                // Starting an already loaded native job must never use kickstart -k.
                let domain = format!(
                    "gui/{}/com.qgenie.{NATIVE}",
                    qgenie_remote_transport::ipc::current_uid()
                );
                let loaded = tokio::process::Command::new("launchctl")
                    .args(["print", &domain])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await?
                    .success();
                if !loaded {
                    manage("start", NATIVE, &units).await?;
                }
                let _ = manage("stop", CONNECTOR, &units).await;
                manage("start", CONNECTOR, &units).await?;
            }
        }
        Action::Pause => manage("stop", CONNECTOR, &directory()?).await?,
        Action::Status => {
            let units = directory()?;
            manage("status", NATIVE, &units).await?;
            manage("status", CONNECTOR, &units).await?;
        }
        Action::UpgradeNative { yes } => {
            ensure!(
                yes,
                "native-owner upgrade requires --yes and no active sessions"
            );
            manager_available().await?;
            // The native starts mutex checks and seals idle shutdown atomically.
            // A racing accepted launch refuses this request before any service stop.
            idle_shutdown(state).await.context(
                "native owner did not confirm idle shutdown; no service stop was issued",
            )?;
            let units = directory()?;
            manage("stop", NATIVE, &units).await?;
            install(state, config, &units, true).await?;
            if cfg!(target_os = "linux") {
                command("systemctl", &["--user".into(), "daemon-reload".into()]).await?;
            }
            manage("start", NATIVE, &units).await?;
        }
    }
    Ok(())
}
