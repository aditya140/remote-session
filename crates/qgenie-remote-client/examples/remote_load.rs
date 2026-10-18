//! Explicit synthetic load only. Credentials enter through owned private files.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_client::{GatewayClient, GatewayConfig, LiveHandle, RemoteAttachment};
use qgenie_remote_proto::{
    Counter, IdempotencyKey, MachineId, SessionId,
    rest::AccessToken,
    wire::{AgentEvent, Launch, LaunchKind},
};
use serde::Deserialize;
use serde_json::json;
use std::{io::Read, path::PathBuf, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle, time::Instant};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    acknowledge_synthetic_load: bool,
    gateway: String,
    ca_bundle: PathBuf,
    token_file: PathBuf,
    state_directory: PathBuf,
    machine: MachineId,
    cwd: String,
    sockets: usize,
    terminals: usize,
    samples_per_terminal: usize,
    burst_bytes: usize,
}
fn private(path: &std::path::Path) -> Result<Vec<u8>> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc_flags())
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == current_uid()
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= 65536,
        "load input must be an owned private regular file <=64 KiB"
    );
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "load input grew beyond limit");
    Ok(bytes)
}
// The example uses the same supported Unix platforms as the native owner.
fn libc_flags() -> i32 {
    libc::O_NOFOLLOW | libc::O_CLOEXEC
}
fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}
struct Owner {
    handle: LiveHandle,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}
impl Owner {
    async fn connect(api: &GatewayClient, token: &AccessToken) -> Result<Self> {
        let mut client = api.connect(token).await?;
        let handle = client.handle();
        let (stop, mut stopping) = oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stopping => break,
                    event = client.next() => if event.is_none() { break; },
                }
            }
            client.close().await;
        });
        Ok(Self {
            handle,
            stop: Some(stop),
            task: Some(task),
        })
    }
    async fn close(mut self) {
        self.stop.take();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.stop.take();
    }
}
struct Reader {
    after: Counter,
    attached: bool,
    tail: Vec<u8>,
    bytes: u64,
}
impl Reader {
    async fn until(
        &mut self,
        handle: &LiveHandle,
        machine: MachineId,
        session: SessionId,
        marker: &[u8],
    ) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            ensure!(
                Instant::now() < deadline,
                "native output deadline elapsed; input was not resent"
            );
            let page = if self.attached {
                handle.replay(machine, session, self.after).await?
            } else {
                handle.attach(machine, session, self.after).await?
            };
            self.attached = true;
            let RemoteAttachment::Replay(page) = page else {
                bail!(
                    "load reader requires lossless replay; checkpoint/gap makes this sample invalid"
                );
            };
            ensure!(
                !page.history_truncated,
                "native replay was truncated during load"
            );
            for event in page.events {
                ensure!(event.seq.0 == self.after.0 + 1, "noncontiguous load output");
                self.after = event.seq;
                match event.event {
                    AgentEvent::PtyOutput { data } => {
                        let bytes = STANDARD.decode(data)?;
                        self.bytes += bytes.len() as u64;
                        self.tail.extend(bytes);
                        if self
                            .tail
                            .windows(marker.len())
                            .any(|candidate| candidate == marker)
                        {
                            self.tail.clear();
                            return Ok(());
                        }
                        let remove = self.tail.len().saturating_sub(marker.len());
                        self.tail.drain(..remove);
                    }
                    AgentEvent::Gap { .. } => {
                        bail!("native output gap invalidated the load sample")
                    }
                    _ => {}
                }
            }
            if !page.more {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    }
}
fn quantile(sorted: &[f64], fraction: f64) -> f64 {
    sorted[((sorted.len() as f64 * fraction).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1)]
}
async fn measure_session(
    handle: LiveHandle,
    machine: MachineId,
    session: SessionId,
    mut reader: Reader,
    index: usize,
    count: usize,
) -> Result<(Vec<f64>, u64)> {
    let mut control = handle.take_control(machine, session, false).await?;
    let mut renewed = Instant::now();
    let mut samples = Vec::with_capacity(count);
    for sample in 0..count {
        if renewed.elapsed() >= Duration::from_secs(5) {
            control.renew().await?;
            renewed = Instant::now();
        }
        let marker = format!("{index}_{sample}");
        let submitted = Instant::now();
        control.input(format!("{marker}\r").as_bytes()).await?;
        reader
            .until(
                &handle,
                machine,
                session,
                format!("QGR_ECHO_{marker}\r\n").as_bytes(),
            )
            .await?;
        samples.push(submitted.elapsed().as_secs_f64() * 1000.0);
    }
    control.release().await?;
    Ok((samples, reader.bytes))
}
async fn workload(
    config: &Config,
    owners: &[Owner],
    known: &mut Vec<SessionId>,
) -> Result<serde_json::Value> {
    let handle = &owners[0].handle;
    let api = GatewayClient::new(GatewayConfig {
        origin: config.gateway.clone(),
        ca_bundle: Some(config.ca_bundle.clone()),
    })?;
    let token: AccessToken = String::from_utf8(private(&config.token_file)?)?
        .trim()
        .to_owned()
        .try_into()
        .map_err(anyhow::Error::msg)?;
    let owner = api
        .machines(&token)
        .await?
        .into_iter()
        .find(|m| m.id == config.machine)
        .context("load machine absent from current owner inventory")?
        .owner;
    let mut samples = Vec::new();
    let mut output_bytes = 0u64;
    let mut sessions = Vec::new();
    // Concurrent native sessions are retained, but startup is deliberately paced.
    for index in 0..config.terminals {
        let key = IdempotencyKey::default();
        let shell = format!(
            "stty -echo; head -c {} /dev/zero | tr '\\000' x; printf '\\nQGR_READY_{index}\\n'; while IFS= read -r line; do printf 'QGR_ECHO_%s\\n' \"$line\"; done",
            config.burst_bytes
        );
        let session = handle
            .start(
                &config.state_directory,
                owner,
                config.machine,
                Launch {
                    key,
                    kind: LaunchKind::Pty {
                        command: vec!["/bin/sh".into(), "-c".into(), shell],
                    },
                    cwd: config.cwd.clone(),
                    rows: 24,
                    cols: 80,
                },
            )
            .await?;
        known.push(session.id);
        // Same identity through the live path must reconcile the original session.
        let (_, looked_up) = handle.lookup_start(config.machine, key).await?;
        ensure!(
            looked_up == Some(session.id),
            "load start identity did not reconcile"
        );
        let mut reader = Reader {
            after: Counter(0),
            attached: false,
            tail: Vec::new(),
            bytes: 0,
        };
        reader
            .until(
                handle,
                config.machine,
                session.id,
                format!("QGR_READY_{index}").as_bytes(),
            )
            .await?;
        sessions.push((session.id, reader));
    }
    let started = Instant::now();
    let mut sessions = sessions.into_iter().enumerate();
    let mut workers = tokio::task::JoinSet::new();
    loop {
        while workers.len() < 32 {
            let Some((index, (session, reader))) = sessions.next() else {
                break;
            };
            workers.spawn(measure_session(
                handle.clone(),
                config.machine,
                session,
                reader,
                index,
                config.samples_per_terminal,
            ));
        }
        let Some(result) = workers.join_next().await else {
            break;
        };
        let (measured, bytes) = result??;
        samples.extend(measured);
        output_bytes += bytes;
    }
    let elapsed = started.elapsed().as_secs_f64();
    samples.sort_by(f64::total_cmp);
    Ok(
        json!({"measurement":"persistent_sdk_input_to_replay_observation_ms", "samples":samples.len(), "p50_ms":quantile(&samples,0.5), "p95_ms":quantile(&samples,0.95), "p99_ms":quantile(&samples,0.99), "max_ms":samples.last(), "sampling_seconds":elapsed, "output_bytes":output_bytes, "input_retries":0, "known_sessions":known.len(), "same_key_lookup_matches":known.len(), "simultaneous_echo_workers":config.terminals.min(32), "poll_interval_ms":1, "baseline_subtracted":false, "prerequisite":"synthetic protected deployment and current owner token", "beta_target_qualification":false}),
    )
}
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .context("usage: remote_load /absolute/private-load-config.json")?;
    let config: Config = serde_json::from_slice(&private(&PathBuf::from(path))?)?;
    ensure!(
        config.acknowledge_synthetic_load
            && (1..=200000).contains(&config.sockets)
            && (1..=2000).contains(&config.terminals)
            && config.terminals <= config.sockets
            && (1..=100000).contains(&config.samples_per_terminal)
            && config.terminals * config.samples_per_terminal <= 2000000
            && config.burst_bytes <= 51200,
        "invalid or unacknowledged load bounds"
    );
    let token: AccessToken = String::from_utf8(private(&config.token_file)?)?
        .trim()
        .to_owned()
        .try_into()
        .map_err(anyhow::Error::msg)?;
    let api = GatewayClient::new(GatewayConfig {
        origin: config.gateway.clone(),
        ca_bundle: Some(config.ca_bundle.clone()),
    })?;
    let mut owners = Vec::new();
    let admission_started = Instant::now();
    for _ in 0..config.sockets {
        owners.push(Owner::connect(&api, &token).await?);
    }
    let admission_seconds = admission_started.elapsed().as_secs_f64();
    let mut known = Vec::new();
    let result = tokio::select! {
        result = workload(&config, &owners, &mut known) => result,
        _ = tokio::signal::ctrl_c() => Err(anyhow::anyhow!("load interrupted; known sessions are being terminated")),
    };
    let mut cleanup_failures = 0;
    for session in known {
        if owners[0]
            .handle
            .terminate_current(config.machine, session, true, false)
            .await
            .is_err()
        {
            cleanup_failures += 1;
        }
    }
    for owner in owners {
        owner.close().await;
    }
    let mut report = result?;
    report["sockets_established"] = json!(config.sockets);
    report["connection_admission_seconds"] = json!(admission_seconds);
    report["cleanup_failures"] = json!(cleanup_failures);
    println!("{}", serde_json::to_string_pretty(&report)?);
    ensure!(
        cleanup_failures == 0,
        "native cleanup uncertain; use original persisted keys/session identities"
    );
    Ok(())
}
