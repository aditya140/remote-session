use super::{admin, private, read, types::Snapshot};
use crate::{AuditAction, AuditEvent, AuditMetadata, Error, Gateway, Store};
use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeriesQuery {
    #[serde(default = "default_hours")]
    hours: u16,
    #[serde(default = "default_buckets")]
    buckets: u16,
}
fn default_hours() -> u16 {
    12
}
fn default_buckets() -> u16 {
    12
}
fn counts(s: &Snapshot) -> Value {
    let active: Vec<_> = s
        .sessions
        .iter()
        .filter(|s| matches!(s.state.as_str(), "running" | "idle"))
        .collect();
    let mut harnesses: BTreeMap<&str, usize> = BTreeMap::new();
    for session in &active {
        *harnesses.entry(&session.harness).or_default() += 1;
    }
    let agents = active.iter().filter(|s| s.kind == "agent").count();
    let configured = active
        .iter()
        .filter(|s| s.kind == "agent" && s.requested_model.is_some())
        .count();
    json!({"observed_at_ms":s.observed_at_ms,"coverage":s.coverage,"users":s.users.len(),"enrolled_devices":s.machines.len(),"connected_devices":s.machines.iter().filter(|m|m.connected).count(),"enrolled_servers":s.machines.iter().filter(|m|m.device_class=="server").count(),"confirmed_active_sessions":active.len(),"unknown_sessions":s.sessions.iter().filter(|s|matches!(s.state.as_str(),"unknown"|"lost"|"outcome_unknown")).count(),"active_agent_processes":agents,"harnesses":harnesses,"models":{"reported":0,"unknown":agents,"configured_sessions":configured,"effective_status":"not_collected","status":if configured>0{"configuration_only"}else{"not_collected"}},"usage":{"tokens":null,"reported_sessions":0,"status":"not_collected"},"recording_warnings":s.sessions.iter().filter(|s|s.recording_warning.is_some()).count(),"aggregation":"bounded_inventory_projection"})
}
pub(crate) async fn summary(
    State(g): State<Gateway>,
    headers: HeaderMap,
) -> Result<Response, Error> {
    admin(&g, &headers).await?;
    let s = read::snapshot(&g, None).await?;
    Ok(private(counts(&s)))
}
pub(crate) async fn series(
    State(g): State<Gateway>,
    headers: HeaderMap,
    Query(q): Query<SeriesQuery>,
) -> Result<Response, Error> {
    let actor = admin(&g, &headers).await?;
    if q.hours == 0 || q.hours > 24 || q.buckets == 0 || q.buckets > 120 {
        return Err(Error::Invalid);
    }
    let now = read::now_ms()?;
    let start = now - u64::from(q.hours) * 3_600_000;
    let Some(audit) = g.audit.as_ref() else {
        return Ok(private(
            json!({"available":false,"reason":"audit_reader_unavailable","items":[],"observed_at_ms":now}),
        ));
    };
    let mut transaction = audit.transaction().await?;
    Store::append_audit(
        &mut transaction,
        &AuditEvent {
            actor: actor.user,
            action: AuditAction::AuditRead,
            machine: None,
            session: None,
            metadata: AuditMetadata::default(),
        },
    )
    .await?;
    let rows=sqlx::query("SELECT event_id,floor(extract(epoch from at)*1000)::bigint AS at_ms FROM qgr.audit_events WHERE action='session_started' AND at>=to_timestamp($1::double precision/1000) AND at<to_timestamp(($2::double precision+1)/1000) ORDER BY event_id DESC LIMIT 10001")
        .bind(start as f64).bind(now as f64).fetch_all(&mut *transaction).await.map_err(|_|Error::Database)?;
    let complete = rows.len() <= 10_000;
    let width = (now - start).div_ceil(u64::from(q.buckets));
    let mut bins = vec![0u64; usize::from(q.buckets)];
    for row in rows.into_iter().take(10_000) {
        let at: u64 = row
            .get::<i64, _>("at_ms")
            .try_into()
            .map_err(|_| Error::Database)?;
        let index = ((at - start) / width).min(u64::from(q.buckets) - 1) as usize;
        bins[index] += 1;
    }
    transaction.commit().await?;
    let items:Vec<_>=bins.into_iter().enumerate().map(|(i,count)|json!({"start_ms":start+i as u64*width,"end_ms":(start+(i as u64+1)*width).min(now),"value":count})).collect();
    Ok(private(
        json!({"available":true,"kind":"accepted_session_start_events","unit":"events","scope":"all_users","start_ms":start,"end_ms":now,"complete":complete,"max_events":10000,"items":items,"observed_at_ms":now,"baseline":{"status":"insufficient_baseline"}}),
    ))
}
pub(crate) async fn runs(State(g): State<Gateway>, headers: HeaderMap) -> Result<Response, Error> {
    admin(&g, &headers).await?;
    Ok(private(
        json!({"items":[],"available":false,"reason":"scheduled_rule_evaluation_not_configured","observed_at_ms":read::now_ms()?}),
    ))
}
pub(crate) async fn signals(
    State(g): State<Gateway>,
    headers: HeaderMap,
) -> Result<Response, Error> {
    let actor = admin(&g, &headers).await?;
    let s = read::snapshot(&g, None).await?;
    let mut items:Vec<_>=s.machines.iter().filter(|m|!m.connected&&!m.disabled).take(100).map(|m|json!({"id":format!("coverage:{}",m.id),"kind":"stale_observation","severity":"coverage","subject":m.owner,"machine":m.id,"title":"Native observation is unavailable","evidence":{"last_seen_ms":m.last_seen_ms,"freshness_ms":read::FRESHNESS_MS},"explanation":"The host may be disconnected, paused, or unreachable. This is not evidence of malicious activity.","automatic_action":false})).collect();
    let (bursts, audit_complete) = burst_signals(&g, actor.user, s.observed_at_ms).await?;
    items.extend(bursts);
    Ok(private(
        json!({"items":items,"observed_at_ms":s.observed_at_ms,"coverage":s.coverage,"audit_coverage_complete":audit_complete,"baseline":{"status":"insufficient_baseline"},"evaluation":"on_demand_metadata_rules","security_anomaly_detection":false,"rule_versions":["stale-observation-v1","enrollment-burst-v1","session-start-burst-v1"]}),
    ))
}

// Transparent fixed thresholds are review signals, not inferred maliciousness.
// Deduplicate original resource identities rather than counting retransmissions.
pub(crate) async fn burst_signals(
    g: &Gateway,
    actor: qgenie_remote_proto::UserId,
    now: u64,
) -> Result<(Vec<Value>, bool), Error> {
    let Some(audit) = g.audit.as_ref() else {
        return Ok((Vec::new(), false));
    };
    let start = now.saturating_sub(15 * 60 * 1000);
    let mut tx = audit.transaction().await?;
    Store::append_audit(
        &mut tx,
        &AuditEvent {
            actor,
            action: AuditAction::AuditRead,
            machine: None,
            session: None,
            metadata: AuditMetadata::default(),
        },
    )
    .await?;
    let rows=sqlx::query("SELECT event_id,actor,action,machine,session FROM qgr.audit_events WHERE action IN ('machine_enrolled','session_started') AND at>=to_timestamp($1::double precision/1000) AND at<to_timestamp(($2::double precision+1)/1000) ORDER BY event_id DESC LIMIT 10001")
        .bind(start as f64).bind(now as f64).fetch_all(&mut *tx).await.map_err(|_|Error::Database)?;
    let complete = rows.len() <= 10000;
    tx.commit().await?;
    let mut groups: BTreeMap<(String, String), (BTreeSet<String>, Vec<String>)> = BTreeMap::new();
    for row in rows.into_iter().take(10000) {
        let action: String = row.get("action");
        let resource: Option<String> = if action == "session_started" {
            row.get("session")
        } else {
            row.get("machine")
        };
        let Some(resource) = resource else { continue };
        let group = groups.entry((row.get("actor"), action)).or_default();
        if group.0.insert(resource) && group.1.len() < 20 {
            group.1.push(row.get::<i64, _>("event_id").to_string());
        }
    }
    let mut signals = Vec::new();
    for ((subject, action), (resources, event_ids)) in groups {
        let (threshold, title, kind) = if action == "session_started" {
            (20, "Elevated native session starts", "session_start_burst")
        } else {
            (3, "Device enrollment burst", "device_enrollment_burst")
        };
        if resources.len() < threshold {
            continue;
        }
        signals.push(json!({"id":format!("{kind}:{subject}:{}",start/(15*60*1000)),"kind":kind,"severity":"review","subject":subject,"title":title,"evidence":{"window_start_ms":start,"window_end_ms":now,"distinct_resources":resources.len(),"threshold":threshold,"event_ids":event_ids,"complete":complete},"explanation":"A fixed review threshold was reached in 15 minutes. New projects or parallel testing can explain this activity. This is not evidence of misuse.","automatic_action":false}));
        if signals.len() == 100 {
            break;
        }
    }
    Ok((signals, complete))
}
