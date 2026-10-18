use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use qgenie_remote_client::Client;
use qgenie_remote_proto::{
    IPC_MAJOR, IdempotencyKey, SessionId, StartSpec, decode_json,
    ipc::{AdminCommand, AdminRequest, AdminResponse},
};
use qgenie_remote_sessiond::Config;
use qgenie_remote_transport::ipc::{
    connect, private_directory, private_file, read_frame, write_frame,
};
use std::{
    io::Read,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
mod host;
mod remote_cli;

#[derive(Parser)]
#[command(
    version,
    about = "Standalone QGenie Remote sessions and recorded history",
    long_about = "Local terminal sessions with private same-user IPC, durable start keys and replay; remote owner metadata and protected recorded history with explicitly injected gateway credentials."
)]
struct Args {
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    root: Vec<PathBuf>,
    #[command(subcommand)]
    command: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    /// Enroll, connect, pause and diagnose the independently owned remote host.
    Host(host::Args),
    /// Open the remote TUI or scriptable owner PTY/history operations using injected credentials.
    Remote {
        #[arg(long)]
        gateway: String,
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long)]
        ca_bundle: Option<PathBuf>,
        #[command(subcommand)]
        command: Option<remote_cli::Action>,
    },
    /// Start the interactive machine/session interface.
    Tui {
        #[arg(long)]
        session: Option<SessionId>,
    },
    /// Start the independent local session daemon, if it is not running.
    Up,
    /// Run the local session owner in the foreground.
    Sessiond {
        #[arg(long)]
        host_config: Option<PathBuf>,
    },
    /// Show local machine and daemon status.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// List available machines (the local machine in this preview).
    Machines {
        #[arg(long)]
        json: bool,
    },
    /// List running and historical local sessions.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Start a command in a PTY; keep its printed key to reconcile uncertainty.
    Run {
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        key: Option<IdempotencyKey>,
        #[arg(long, default_value_t = 24)]
        rows: u16,
        #[arg(long, default_value_t = 80)]
        cols: u16,
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Open a session in observe mode; Ctrl-] c requests input control.
    Attach { session: SessionId },
    /// Look up the original start key without sending a new launch.
    Reconcile { key: IdempotencyKey },
    /// Send one explicit input, release control, and never retry it automatically.
    Send {
        session: SessionId,
        text: String,
        #[arg(long)]
        enter: bool,
        #[arg(long)]
        takeover: bool,
    },
    /// Interrupt the foreground turn without terminating the shell.
    Interrupt {
        session: SessionId,
        #[arg(long)]
        takeover: bool,
    },
    /// Terminate a session explicitly.
    Terminate {
        session: SessionId,
        #[arg(long)]
        yes: bool,
    },
    /// Stop the daemon; refuse while sessions remain unless termination is explicit.
    Stop {
        #[arg(long)]
        terminate_sessions: bool,
    },
    /// Check local IPC, roots, capabilities, and preview limits.
    Doctor,
    /// Read daemon diagnostics. PTY content is not written to this log.
    Logs,
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let args = Args::parse();
    let cwd = std::env::current_dir()?;
    let state_dir = absolute(
        args.state_dir
            .or_else(|| std::env::var_os("QGR_STATE_DIR").map(PathBuf::from))
            .unwrap_or_else(default_state_dir),
        &cwd,
    );
    let roots = if args.root.is_empty() {
        vec![cwd.clone()]
    } else {
        args.root
            .iter()
            .map(|root| absolute(root.clone(), &cwd))
            .collect()
    };
    let client = Client::local(&state_dir);
    match args.command.unwrap_or(Action::Tui { session: None }) {
        Action::Remote {
            gateway,
            token_file,
            ca_bundle,
            command,
        } => {
            let token = read_remote_token(&token_file)?;
            let config = qgenie_remote_client::GatewayConfig {
                origin: gateway,
                ca_bundle,
            };
            if let Some(command) = command {
                remote_cli::run(config, token, &state_dir, command).await?;
            } else {
                qgenie_remote_tui::run_remote_with_state(config, token, &state_dir).await?;
            }
        }
        Action::Host(args) => host::run(args, &state_dir).await?,
        Action::Sessiond { host_config } => {
            // Private umask covers SQLite sidecar creation and service logs.
            set_private_umask();
            let config = if let Some(path) = host_config {
                host::native_config(&state_dir, &path)?
            } else {
                Config::new(state_dir, roots)
            };
            qgenie_remote_sessiond::serve(config).await?;
        }
        Action::Up => {
            ensure_up(&state_dir, &roots).await?;
            println!("{}", serde_json::to_string_pretty(&client.status().await?)?);
        }
        Action::Tui { session } => {
            ensure_up(&state_dir, &roots).await?;
            qgenie_remote_tui::run(client, cwd.display().to_string(), session).await?;
        }
        Action::Attach { session } => {
            qgenie_remote_tui::run(client, cwd.display().to_string(), Some(session)).await?;
        }
        Action::Reconcile { key } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&client.lookup_start(key).await?)?
            );
        }
        Action::Status { json } | Action::Machines { json } => {
            let status = client.status().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!(
                    "{}  {} {}  {} sessions\n{}\nAllowed roots: {}",
                    status.name,
                    status.os,
                    status.arch,
                    status.sessions,
                    status.mode,
                    status.roots.join(", ")
                );
            }
        }
        Action::List { json } => {
            let sessions = client.sessions().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&sessions)?);
            } else {
                for session in sessions {
                    println!(
                        "{}  {:?}  {}  {}",
                        session.id,
                        session.state,
                        session.command.join(" "),
                        session.cwd
                    );
                }
            }
        }
        Action::Run {
            cwd: run_cwd,
            key,
            rows,
            cols,
            command,
        } => {
            ensure_up(&state_dir, &roots).await?;
            let key = key.unwrap_or_default();
            let command = if command.is_empty() {
                vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())]
            } else {
                command
            };
            let spec = StartSpec {
                key,
                command,
                cwd: absolute(run_cwd.unwrap_or(cwd.clone()), &cwd)
                    .display()
                    .to_string(),
                rows,
                cols,
            };
            // The SDK journals the identity before dispatch for CLI and TUI callers.
            eprintln!("Start key: {key}");
            let session = client.start(spec).await?;
            println!("{}", serde_json::to_string_pretty(&session)?);
        }
        Action::Send {
            session,
            text,
            enter,
            takeover,
        } => {
            let mut controller = client.take_control(session, takeover).await?;
            let mut bytes = text.into_bytes();
            if enter {
                bytes.push(b'\r');
            }
            let result = controller.input(&bytes).await;
            let _ = client.release(session).await;
            result?;
        }
        Action::Interrupt { session, takeover } => {
            let controller = client.take_control(session, takeover).await?;
            let result = controller.interrupt().await;
            let _ = client.release(session).await;
            result?;
        }
        Action::Terminate { session, yes } => {
            anyhow::ensure!(yes, "termination requires --yes");
            client.terminate(session, true).await?;
        }
        Action::Stop { terminate_sessions } => {
            let mut stream = connect(&state_dir.join("admin.sock")).await?;
            write_frame(
                &mut stream,
                &serde_json::to_vec(&AdminRequest {
                    ipc_major: IPC_MAJOR,
                    command: AdminCommand::Shutdown { terminate_sessions },
                })?,
            )
            .await?;
            let response: AdminResponse = decode_json(&read_frame(&mut stream).await?)?;
            response.result?;
        }
        Action::Doctor => {
            private_directory(&state_dir)?;
            let status = client.status().await?;
            println!(
                "PASS private same-user IPC\nPASS daemon {}\nPASS allowed roots: {}\nAvailable native runtime: shell/PTY\nMode: {}\nFor remote machine authentication and connector state, use host --config PATH doctor.",
                status.version,
                status.roots.join(", "),
                status.mode
            );
        }
        Action::Logs => {
            let log = private_file(&state_dir.join("daemon.log"))?;
            let mut contents = String::new();
            log.take(1024 * 1024).read_to_string(&mut contents)?;
            print!("{contents}");
        }
    }
    Ok(())
}

fn read_remote_token(path: &Path) -> Result<qgenie_remote_proto::rest::AccessToken> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .context("cannot open injected gateway credential")?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= 16386,
        "gateway credential must be a private regular file owned by this user, at most 16 KiB"
    );
    let mut token = String::new();
    std::io::Read::by_ref(&mut file)
        .take(16387)
        .read_to_string(&mut token)
        .context("cannot read injected gateway credential")?;
    anyhow::ensure!(
        token.len() <= 16386,
        "gateway credential exceeds its budget"
    );
    token
        .trim_end_matches(['\r', '\n'])
        .to_owned()
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid injected gateway credential"))
}

fn default_state_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join(".local/share")
        });
    base.join("qgenie-remote")
}
fn absolute(path: PathBuf, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}
fn set_private_umask() {
    // Called before database creation in this single-purpose daemon process.
    // SAFETY: umask accepts a permission bitmask and does not dereference pointers.
    unsafe {
        libc::umask(0o077);
    }
}

async fn ensure_up(state_dir: &Path, roots: &[PathBuf]) -> Result<()> {
    let client = Client::local(state_dir);
    if client.status().await.is_ok() {
        return Ok(());
    }
    private_directory(state_dir)?;
    let log = private_file(&state_dir.join("daemon.log"))?;
    let mut command = Command::new(std::env::current_exe()?);
    command.arg("--state-dir").arg(state_dir);
    for root in roots {
        command.arg("--root").arg(root);
    }
    command
        .arg("sessiond")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .process_group(0);
    let mut child = command.spawn().context("cannot launch session daemon")?;
    for _ in 0..100 {
        if client.status().await.is_ok() {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            bail!("session daemon exited ({status}); inspect qgenie-remote logs");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("session daemon did not become ready; inspect qgenie-remote logs")
}
