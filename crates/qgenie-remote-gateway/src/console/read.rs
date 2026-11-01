use super::types::*;
use crate::{Error, Gateway};
use qgenie_remote_proto::UserId;
use sqlx::Row;
use std::collections::BTreeMap;

pub(crate) const SNAPSHOT_LIMIT: usize = 500;
pub(crate) const FRESHNESS_MS: u64 = 30_000;
pub(crate) fn now_ms() -> Result<u64, Error> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Unavailable)?
        .as_millis()
        .try_into()
        .map_err(|_| Error::Unavailable)
}
fn millis(value: i64) -> Result<u64, Error> {
    value.try_into().map_err(|_| Error::Database)
}
fn clean(value: Option<&str>, limit: usize) -> Option<String> {
    value
        .filter(|s| !s.is_empty() && s.len() <= limit && !s.chars().any(char::is_control))
        .map(str::to_owned)
}
fn timestamp(value: Option<&serde_json::Value>) -> Option<u64> {
    let value = value?;
    value.as_u64().or_else(|| {
        serde_json::from_value::<qgenie_remote_proto::wire::Timestamp>(value.clone())
            .ok()
            .map(|v| v.unix_millis())
    })
}
// The app role can read identity and session metadata. Owner filters are applied
// in SQL, before fetching. No recording data or raw launch body leaves this module.
pub(crate) async fn snapshot(gateway: &Gateway, owner: Option<UserId>) -> Result<Snapshot, Error> {
    let scope = owner.map(|id| id.to_string());
    let mut tx = gateway
        .store
        .pool
        .begin()
        .await
        .map_err(|_| Error::Database)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|_| Error::Database)?;
    let users = sqlx::query(
        "SELECT id,disabled FROM qgr.users WHERE ($1::text IS NULL OR id=$1) ORDER BY id LIMIT 501",
    )
    .bind(&scope)
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| Error::Database)?;
    let machine_rows = sqlx::query("SELECT m.id,m.owner,m.name,m.hostname,m.os,m.arch,m.cli_version,m.labels,m.disabled,floor(extract(epoch from m.last_seen)*1000)::bigint AS seen_ms,COALESCE(r.activated,false) AS activated,COALESCE((SELECT jsonb_object_agg(k,jsonb_build_object('version',left(v->>'version',64))) FROM (SELECT left(key,128) AS k,value AS v FROM jsonb_each(COALESCE(r.agents,'{}'::jsonb)) ORDER BY key LIMIT 32) inventory),'{}'::jsonb) AS agents FROM qgr.machines m LEFT JOIN qgr.machine_routes r ON r.machine=m.id WHERE ($1::text IS NULL OR m.owner=$1) ORDER BY m.id LIMIT 501")
        .bind(&scope).fetch_all(&mut *tx).await.map_err(|_| Error::Database)?;
    let session_rows = sqlx::query("SELECT s.id,s.machine,s.owner,s.state,jsonb_build_object('kind',jsonb_build_object('kind',s.body->'kind'->'kind','agent',left(s.body->'kind'->>'agent',128)),'requested_model',left(s.body->>'requested_model',128),'origin',s.body->'origin','created_at',s.body->'created_at','recording_warning',left(s.body->>'recording_warning',1024),'capabilities',jsonb_build_object('terminate',s.body->'capabilities'->'terminate')) AS body,jsonb_build_object('name',left(s.metadata->>'name',128)) AS metadata,floor(extract(epoch from s.updated_at)*1000)::bigint AS updated_ms FROM qgr.sessions s WHERE ($1::text IS NULL OR s.owner=$1) ORDER BY s.id LIMIT 501")
        .bind(&scope).fetch_all(&mut *tx).await.map_err(|_| Error::Database)?;
    tx.commit().await.map_err(|_| Error::Database)?;
    let now = now_ms()?;
    let complete = users.len() <= SNAPSHOT_LIMIT
        && machine_rows.len() <= SNAPSHOT_LIMIT
        && session_rows.len() <= SNAPSHOT_LIMIT;
    let mut machines = Vec::with_capacity(machine_rows.len().min(SNAPSHOT_LIMIT));
    for row in machine_rows.into_iter().take(SNAPSHOT_LIMIT) {
        let labels: serde_json::Value = row.get("labels");
        let class = labels
            .get("device_class")
            .and_then(|v| v.as_str())
            .filter(|v| matches!(*v, "server" | "workstation"))
            .unwrap_or("unknown");
        let seen = millis(row.get("seen_ms"))?;
        let disabled: bool = row.get("disabled");
        let activated: bool = row.get("activated");
        machines.push(Machine {
            id: row.get("id"),
            owner: row.get("owner"),
            name: row.get("name"),
            hostname: row.get("hostname"),
            os: row.get("os"),
            arch: row.get("arch"),
            cli_version: row.get("cli_version"),
            device_class: class.into(),
            classification_source: if class == "unknown" {
                "not_reported"
            } else {
                "owner_label"
            },
            connected: !disabled
                && activated
                && seen <= now
                && now.saturating_sub(seen) <= FRESHNESS_MS,
            disabled,
            last_seen_ms: seen,
            last_connected_ms: None,
            agents: row.get("agents"),
        });
    }
    let by_machine: BTreeMap<_, _> = machines.iter().map(|m| (m.id.as_str(), m)).collect();
    let mut sessions = Vec::with_capacity(session_rows.len().min(SNAPSHOT_LIMIT));
    for row in session_rows.into_iter().take(SNAPSHOT_LIMIT) {
        let body: serde_json::Value = row.get("body");
        let metadata: serde_json::Value = row.get("metadata");
        let id: String = row.get("id");
        let machine: String = row.get("machine");
        let native = by_machine.get(machine.as_str());
        let reported: String = row.get("state");
        let observed = millis(row.get("updated_ms"))?;
        let terminal = matches!(
            reported.as_str(),
            "exited" | "failed" | "lost" | "outcome_unknown"
        );
        let fresh = native.is_some_and(|m| m.connected)
            && observed <= now
            && now.saturating_sub(observed) <= FRESHNESS_MS;
        let kind = body
            .pointer("/kind/kind")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let harness = if kind == "pty" {
            "PTY shell".into()
        } else {
            clean(body.pointer("/kind/agent").and_then(|v| v.as_str()), 128)
                .unwrap_or_else(|| "Unknown harness".into())
        };
        let version = native
            .and_then(|m| m.agents.get(&harness))
            .and_then(|v| v.get("version"))
            .and_then(|v| clean(v.as_str(), 64));
        let name = clean(metadata.get("name").and_then(|v| v.as_str()), 128)
            .unwrap_or_else(|| format!("Session {id}"));
        let requested_model = clean(body.get("requested_model").and_then(|v| v.as_str()), 128);
        let configured = requested_model.is_some();
        sessions.push(Session {
            id,
            machine,
            owner: row.get("owner"),
            name,
            kind: kind.into(),
            origin: clean(body.get("origin").and_then(|v| v.as_str()), 32)
                .unwrap_or_else(|| "unknown".into()),
            harness,
            harness_version: version,
            model: None,
            requested_model,
            effective_model: None,
            model_scope: if configured {
                "session_configuration"
            } else {
                "unknown"
            },
            model_source: if configured {
                "launch_configuration"
            } else {
                "not_reported"
            },
            state: if terminal || fresh {
                reported.clone()
            } else {
                "unknown".into()
            },
            reported_state: reported,
            task_state: "unknown",
            created_at_ms: timestamp(body.get("created_at")),
            observed_at_ms: observed,
            recording_warning: clean(body.get("recording_warning").and_then(|v| v.as_str()), 1024),
            terminate: fresh
                && !terminal
                && body
                    .pointer("/capabilities/terminate")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
        });
    }
    let users = users
        .into_iter()
        .take(SNAPSHOT_LIMIT)
        .map(|row| {
            let id: String = row.get("id");
            let own: Vec<_> = machines.iter().filter(|m| m.owner == id).collect();
            let runs: Vec<_> = sessions.iter().filter(|s| s.owner == id).collect();
            User {
                display_name: format!("User {id}"),
                disabled: row.get("disabled"),
                devices: own.len(),
                connected_devices: own.iter().filter(|m| m.connected).count(),
                servers: own.iter().filter(|m| m.device_class == "server").count(),
                connected_servers: own
                    .iter()
                    .filter(|m| m.device_class == "server" && m.connected)
                    .count(),
                active_sessions: runs
                    .iter()
                    .filter(|s| matches!(s.state.as_str(), "running" | "idle"))
                    .count(),
                unknown_sessions: runs
                    .iter()
                    .filter(|s| matches!(s.state.as_str(), "unknown" | "outcome_unknown" | "lost"))
                    .count(),
                last_seen_ms: own.iter().map(|m| m.last_seen_ms).max(),
                id,
            }
        })
        .collect::<Vec<_>>();
    let coverage = Coverage {
        complete,
        users: users.len(),
        machines: machines.len(),
        sessions: sessions.len(),
        limit: SNAPSHOT_LIMIT,
        freshness_ms: FRESHNESS_MS,
    };
    Ok(Snapshot {
        users,
        machines,
        sessions,
        observed_at_ms: now,
        coverage,
    })
}

pub(crate) fn paginate<T: serde::Serialize + Clone>(
    mut values: Vec<T>,
    query: &Query,
    id: impl Fn(&T) -> &str,
    observed_at_ms: u64,
    coverage: Coverage,
) -> Page<T> {
    if let Some(after) = &query.cursor {
        values.retain(|v| id(v) > after.as_str());
    }
    let next = if values.len() > query.limit {
        values.truncate(query.limit);
        values.last().map(|v| id(v).to_owned())
    } else {
        None
    };
    Page {
        items: values,
        next,
        observed_at_ms,
        coverage,
    }
}

// Connection timestamps come only from committed successful activation audits.
// The machine set is already owner-scoped before entering the audit reader pool.
pub(crate) async fn last_connections(
    gateway: &Gateway,
    actor: UserId,
    machines: &mut [Machine],
) -> Result<(), Error> {
    if machines.is_empty() {
        return Ok(());
    }
    let Some(audit) = gateway.audit.as_ref() else {
        return Ok(());
    };
    let ids: Vec<_> = machines.iter().map(|m| m.id.clone()).collect();
    let mut tx = audit.transaction().await?;
    crate::Store::append_audit(
        &mut tx,
        &crate::AuditEvent {
            actor,
            action: crate::AuditAction::AuditRead,
            machine: None,
            session: None,
            metadata: crate::AuditMetadata::default(),
        },
    )
    .await?;
    let rows=sqlx::query("SELECT requested.machine,floor(extract(epoch from latest.at)*1000)::bigint AS at_ms FROM unnest($1::text[]) AS requested(machine) CROSS JOIN LATERAL (SELECT at FROM qgr.audit_events WHERE action='host_activated' AND machine=requested.machine ORDER BY event_id DESC LIMIT 1) latest")
        .bind(ids).fetch_all(&mut *tx).await.map_err(|_|Error::Database)?;
    tx.commit().await?;
    let values: BTreeMap<_, _> = rows
        .into_iter()
        .map(|row| (row.get::<String, _>("machine"), row.get::<i64, _>("at_ms")))
        .collect();
    for machine in machines {
        machine.last_connected_ms = values.get(&machine.id).map(|v| millis(*v)).transpose()?;
    }
    Ok(())
}
