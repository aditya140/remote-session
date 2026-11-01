use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Me {
    pub user: String,
    pub platform_admin: bool,
    pub expires_at: u64,
    pub capabilities: BTreeMap<&'static str, bool>,
}
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Coverage {
    pub complete: bool,
    pub users: usize,
    pub machines: usize,
    pub sessions: usize,
    pub limit: usize,
    pub freshness_ms: u64,
}
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Snapshot {
    pub users: Vec<User>,
    pub machines: Vec<Machine>,
    pub sessions: Vec<Session>,
    pub observed_at_ms: u64,
    pub coverage: Coverage,
}
#[derive(Debug, Clone, Serialize)]
pub(crate) struct User {
    pub id: String,
    pub display_name: String,
    pub disabled: bool,
    pub devices: usize,
    pub connected_devices: usize,
    pub servers: usize,
    pub connected_servers: usize,
    pub active_sessions: usize,
    pub unknown_sessions: usize,
    pub last_seen_ms: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Machine {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub cli_version: String,
    pub device_class: String,
    pub classification_source: &'static str,
    pub connected: bool,
    pub disabled: bool,
    pub last_seen_ms: u64,
    pub last_connected_ms: Option<u64>,
    pub agents: serde_json::Value,
}
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Session {
    pub id: String,
    pub machine: String,
    pub owner: String,
    pub name: String,
    pub kind: String,
    pub origin: String,
    pub harness: String,
    pub harness_version: Option<String>,
    pub model: Option<String>,
    pub requested_model: Option<String>,
    pub effective_model: Option<String>,
    pub model_scope: &'static str,
    pub model_source: &'static str,
    pub state: String,
    pub reported_state: String,
    pub task_state: &'static str,
    pub created_at_ms: Option<u64>,
    pub observed_at_ms: u64,
    pub recording_warning: Option<String>,
    pub terminate: bool,
}
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Page<T: Serialize> {
    pub items: Vec<T>,
    pub next: Option<String>,
    pub observed_at_ms: u64,
    pub coverage: Coverage,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Query {
    #[serde(default = "default_limit")]
    pub limit: usize,
    pub cursor: Option<String>,
    pub owner: Option<qgenie_remote_proto::UserId>,
    pub machine: Option<qgenie_remote_proto::MachineId>,
}
fn default_limit() -> usize {
    25
}
impl Query {
    pub fn validate(&self) -> Result<(), crate::Error> {
        if !(1..=100).contains(&self.limit)
            || self.cursor.as_ref().is_some_and(|c| {
                c.len() > 64
                    || c.is_empty()
                    || !c
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
        {
            return Err(crate::Error::Invalid);
        }
        Ok(())
    }
}
