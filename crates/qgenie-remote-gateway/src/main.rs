use clap::{Args, Parser, Subcommand};
mod jobs;
use qgenie_remote_gateway::{
    Config, DirectoryConfig, Gateway, JwtConfig, RecordingDatabase, RecordingRole,
    RejectAuthentication, RoutingConfig, SecretDsn, Store,
};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Container health probe. Only this process namespace's loopback endpoint is accepted.
    Probe {
        #[arg(long, default_value = "127.0.0.1:9091")]
        address: SocketAddr,
        #[arg(long)]
        live: bool,
    },
    Migrate {
        #[arg(long)]
        database_url_file: PathBuf,
    },
    Serve(Box<ServeArgs>),
    Jobs {
        #[arg(long)]
        database_url_file: PathBuf,
        #[arg(long, default_value = "scheduled retention")]
        reason: String,
        #[arg(long,default_value_t=32,value_parser=clap::value_parser!(u16).range(1..=32))]
        batch_size: u16,
        #[arg(long,default_value_t=1024,value_parser=clap::value_parser!(u16).range(1..=4096))]
        max_steps: u16,
        /// Repeat bounded runs while this process owns scheduling; omission runs once.
        #[arg(long,value_parser=clap::value_parser!(u32).range(60..=86400))]
        interval_seconds: Option<u32>,
    },
}
#[derive(Args)]
struct ServeArgs {
    #[arg(long)]
    database_url_file: PathBuf,
    #[arg(long)]
    audit_database_url_file: Option<PathBuf>,
    #[arg(long, env = "QGR_NODE_ID", default_value = "node-1")]
    node_id: String,
    #[arg(long, requires = "machine_key_file")]
    identity_config_file: Option<PathBuf>,
    #[arg(long, requires = "identity_config_file")]
    directory_config_file: Option<PathBuf>,
    #[arg(long, requires = "directory_config_file")]
    recording_lifecycle_database_url_file: Option<PathBuf>,
    #[arg(long, requires_all=["directory_config_file","recording_owner_database_url_file","recording_admin_database_url_file","recording_redaction_file"])]
    recording_ingest_database_url_file: Option<PathBuf>,
    #[arg(long, requires = "recording_ingest_database_url_file")]
    recording_owner_database_url_file: Option<PathBuf>,
    #[arg(long, requires = "recording_ingest_database_url_file")]
    recording_admin_database_url_file: Option<PathBuf>,
    #[arg(long, requires = "recording_ingest_database_url_file")]
    recording_redaction_file: Option<PathBuf>,
    #[arg(long, requires = "identity_config_file")]
    machine_key_file: Option<PathBuf>,
    #[arg(long, requires_all=["routing_key_file","identity_config_file"])]
    redis_url_file: Option<PathBuf>,
    #[arg(long, requires = "redis_url_file")]
    routing_key_file: Option<PathBuf>,
    #[arg(long, requires = "redis_url_file")]
    redis_ca_file: Option<PathBuf>,
    #[arg(long, default_value = "https://qgenie-remote.qualcomm.com")]
    origin: String,
    #[arg(long, default_value = "127.0.0.1:8081")]
    listen: SocketAddr,
    #[arg(long, default_value = "127.0.0.1:9091")]
    health_listen: SocketAddr,
}
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    match Cli::parse().command {
        Command::Probe { address, live } => {
            if !address.ip().is_loopback() {
                return Err(qgenie_remote_gateway::Error::Configuration.into());
            }
            let response = reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(3))
                .build()?
                .get(format!(
                    "http://{address}/{}",
                    if live { "live" } else { "ready" }
                ))
                .send()
                .await?;
            if !response.status().is_success() {
                return Err(qgenie_remote_gateway::Error::Unavailable.into());
            }
        }
        Command::Migrate { database_url_file } => {
            Store::migrate(&SecretDsn::from_file(&database_url_file)?).await?
        }
        Command::Jobs {
            database_url_file,
            reason,
            batch_size,
            max_steps,
            interval_seconds,
        } => {
            let database = RecordingDatabase::connect(
                &SecretDsn::from_file(&database_url_file)?,
                RecordingRole::Lifecycle,
                2,
            )
            .await?;
            let result: Result<(), Box<dyn std::error::Error>> = tokio::select! {
                result=async {loop {jobs::retention(&database,&reason,batch_size,max_steps).await?;match interval_seconds {Some(seconds)=>tokio::time::sleep(std::time::Duration::from_secs(u64::from(seconds))).await,None=>break,}}Ok::<_,qgenie_remote_gateway::Error>(())}=>result.map_err(Into::into),
                signal=shutdown_signal()=>match signal {Ok(())=>Err(qgenie_remote_gateway::Error::Draining.into()),Err(error)=>Err(error.into())},
            };
            database.close().await;
            result?;
        }
        Command::Serve(args) => {
            let ServeArgs {
                database_url_file,
                audit_database_url_file,
                node_id,
                identity_config_file,
                directory_config_file,
                recording_lifecycle_database_url_file,
                recording_ingest_database_url_file,
                recording_owner_database_url_file,
                recording_admin_database_url_file,
                recording_redaction_file,
                machine_key_file,
                redis_url_file,
                routing_key_file,
                redis_ca_file,
                origin,
                listen,
                health_listen,
            } = *args;
            let config = Config {
                node_id,
                listen,
                health_listen,
                database: SecretDsn::from_file(&database_url_file)?,
                audit_database: audit_database_url_file
                    .as_deref()
                    .map(SecretDsn::from_file)
                    .transpose()?,
                pool_size: 16,
            };
            let mut gateway = Gateway::connect(&config, Arc::new(RejectAuthentication)).await?;
            if let Some(path) = identity_config_file {
                gateway = gateway
                    .enable_identity(
                        JwtConfig::from_file(&path)?,
                        origin,
                        machine_key_file
                            .as_deref()
                            .ok_or(qgenie_remote_gateway::Error::Configuration)?,
                    )
                    .await?;
            }
            if let Some(path) = directory_config_file {
                gateway = gateway
                    .enable_directory(DirectoryConfig::from_file(&path)?)
                    .await?;
            }
            if let Some(path) = recording_lifecycle_database_url_file {
                gateway = gateway
                    .enable_recording_lifecycle(&SecretDsn::from_file(&path)?)
                    .await?;
            }
            if let Some(path) = recording_ingest_database_url_file {
                gateway = gateway
                    .enable_recordings(
                        &SecretDsn::from_file(&path)?,
                        &SecretDsn::from_file(
                            recording_owner_database_url_file
                                .as_deref()
                                .ok_or(qgenie_remote_gateway::Error::Configuration)?,
                        )?,
                        &SecretDsn::from_file(
                            recording_admin_database_url_file
                                .as_deref()
                                .ok_or(qgenie_remote_gateway::Error::Configuration)?,
                        )?,
                        qgenie_remote_gateway::RecordingRedaction::from_file(
                            recording_redaction_file
                                .as_deref()
                                .ok_or(qgenie_remote_gateway::Error::Configuration)?,
                        )?,
                    )
                    .await?;
            }
            if let Some(path) = redis_url_file {
                gateway = gateway
                    .enable_routing(RoutingConfig::from_files(
                        config.node_id.clone(),
                        &path,
                        routing_key_file
                            .as_deref()
                            .ok_or(qgenie_remote_gateway::Error::Configuration)?,
                        redis_ca_file.as_deref(),
                    )?)
                    .await?;
            }
            let public = tokio::net::TcpListener::bind(config.listen).await?;
            let health = tokio::net::TcpListener::bind(config.health_listen).await?;
            let (stop, receiver) = tokio::sync::watch::channel(false);
            let mut health_stop = receiver.clone();
            let mut public_stop = receiver;
            let public_server =
                axum::serve(public, gateway.router()).with_graceful_shutdown(async move {
                    let _ = public_stop.changed().await;
                });
            let health_server = axum::serve(health, gateway.health_router())
                .with_graceful_shutdown(async move {
                    let _ = health_stop.changed().await;
                });
            let signal = async {
                shutdown_signal().await?;
                gateway.drain();
                let _ = stop.send(true);
                Ok::<_, std::io::Error>(())
            };
            tokio::try_join!(public_server, health_server, signal)?;
            gateway.shutdown().await;
        }
    }
    Ok(())
}

async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! { result=tokio::signal::ctrl_c()=>result,_=terminate.recv()=>Ok(()) }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}
