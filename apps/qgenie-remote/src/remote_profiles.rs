//! Private local launch settings. They grant no permissions and contain no auth fields.
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use fs2::FileExt;
use qgenie_remote_client::{GatewayClient, GatewayConfig};
use qgenie_remote_proto::{
    IdempotencyKey, MachineId,
    wire::{Launch, LaunchKind},
};
use qgenie_remote_transport::ipc::{private_directory, private_file};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Subcommand)]
pub(crate) enum Action {
    /// Save a local profile. --gateway binds it to one service; no sign-in is needed.
    Save {
        name: String,
        machine: MachineId,
        #[arg(long)]
        cwd: String,
        #[arg(long, conflicts_with = "command")]
        agent: Option<String>,
        #[arg(long, requires = "agent")]
        task: Option<String>,
        #[arg(long)]
        replace: bool,
        #[arg(last = true, conflicts_with = "agent")]
        command: Vec<String>,
    },
    List,
    Show {
        name: String,
    },
    Remove {
        name: String,
    },
    /// Launch once. Retain the printed key if the result is uncertain.
    Run {
        name: String,
        #[arg(long)]
        key: Option<IdempotencyKey>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    version: u8,
    gateway: String,
    machine: MachineId,
    cwd: String,
    kind: Kind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Kind {
    Shell { argv: Vec<String> },
    Agent { agent: String, task: Option<String> },
}
pub(crate) struct Prepared {
    pub gateway: String,
    pub action: super::remote_cli::Action,
}
fn filename(name: &str) -> Result<String> {
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)),
        "profile name must be 1..=64 ASCII letters, digits, hyphens, or underscores"
    );
    Ok(format!("{name}.json"))
}
impl Profile {
    fn launch(&self, key: IdempotencyKey) -> Result<Launch> {
        ensure!(self.version == 1, "unsupported launch profile version");
        let origin = GatewayClient::new(GatewayConfig {
            origin: self.gateway.clone(),
            ca_bundle: None,
        })?;
        ensure!(
            origin.origin() == self.gateway,
            "profile requires a canonical HTTPS gateway origin"
        );
        let kind = match &self.kind {
            Kind::Shell { argv } => LaunchKind::Pty {
                command: argv.clone(),
            },
            Kind::Agent { agent, task } => LaunchKind::Agent {
                agent: agent.clone(),
                prompt: task.clone(),
                native_resume_id: None,
            },
        };
        let launch = Launch {
            key,
            kind,
            cwd: self.cwd.clone(),
            rows: 24,
            cols: 80,
        };
        launch.validate()?;
        Ok(launch)
    }
}
fn names(directory: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for (index, entry) in std::fs::read_dir(directory)?.enumerate() {
        ensure!(index < 256, "profile directory has too many entries");
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("invalid profile filename"))?;
        if let Some(name) = name.strip_suffix(".json") {
            filename(name)?;
            names.push(name.to_owned());
        }
    }
    ensure!(names.len() <= 64, "at most 64 launch profiles");
    names.sort();
    Ok(names)
}
fn load(directory: &Path, name: &str) -> Result<Profile> {
    let profile: Profile = super::host::read_private(&directory.join(filename(name)?))?;
    profile.launch(IdempotencyKey::new())?;
    Ok(profile)
}

/// Resolve a local command before sign-in. A run returns the existing launch command.
pub(crate) fn prepare(
    action: Action,
    state: &Path,
    gateway: Option<&str>,
) -> Result<Option<Prepared>> {
    private_directory(state)?;
    let directory = state.join("launch-profiles");
    private_directory(&directory)?;
    let lock = private_file(&directory.join("profiles.lock"))?;
    lock.try_lock_exclusive()
        .context("another command is updating launch profiles")?;
    match action {
        Action::Save {
            name,
            machine,
            cwd,
            agent,
            task,
            replace,
            command,
        } => {
            let filename = filename(&name)?;
            let gateway =
                gateway.context("profile save requires --gateway; it does not save credentials")?;
            let gateway = GatewayClient::new(GatewayConfig {
                origin: gateway.into(),
                ca_bundle: None,
            })?
            .origin()
            .to_owned();
            let kind = agent
                .map(|agent| Kind::Agent { agent, task })
                .unwrap_or_else(|| Kind::Shell {
                    argv: if command.is_empty() {
                        vec!["/bin/sh".into()]
                    } else {
                        command
                    },
                });
            let profile = Profile {
                version: 1,
                gateway,
                machine,
                cwd,
                kind,
            };
            profile.launch(IdempotencyKey::new())?;
            let existing = names(&directory)?;
            ensure!(
                existing.len() < 64 || existing.contains(&name),
                "at most 64 launch profiles"
            );
            super::remote_access::save_settings(&directory, &filename, &profile, replace)?;
            println!("{}", serde_json::json!({"profile":name,"saved":true}));
        }
        Action::List => println!("{}", serde_json::to_string(&names(&directory)?)?),
        Action::Show { name } => println!(
            "{}",
            serde_json::to_string_pretty(&load(&directory, &name)?)?
        ),
        Action::Remove { name } => {
            std::fs::remove_file(directory.join(filename(&name)?))?;
            std::fs::File::open(&directory)?.sync_all()?;
            println!(
                "{}",
                serde_json::json!({"profile":name,"removed":true,"native_sessions":"retained"})
            );
        }
        Action::Run { name, key } => {
            let profile = load(&directory, &name)?;
            if let Some(gateway) = gateway {
                ensure!(
                    gateway == profile.gateway,
                    "profile belongs to a different gateway"
                );
            }
            let (command, agent, task) = match profile.kind {
                Kind::Shell { argv } => (argv, None, None),
                Kind::Agent { agent, task } => (Vec::new(), Some(agent), task),
            };
            return Ok(Some(Prepared {
                gateway: profile.gateway,
                action: super::remote_cli::Action::Run {
                    machine: profile.machine,
                    cwd: profile.cwd,
                    key,
                    rows: 24,
                    cols: 80,
                    command,
                    agent,
                    task,
                    resume_id: None,
                },
            }));
        }
    }
    Ok(None)
}
