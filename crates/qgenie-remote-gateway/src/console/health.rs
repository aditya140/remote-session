//! Sanitized observations from the accepting gateway process. These are not a
//! fleet-wide scrape or proof of synchronous recording commit availability.
use super::{admin, private, read};
use crate::{Error, Gateway};
use axum::{extract::State, http::HeaderMap, response::Response};
use serde_json::{Value, json};
pub(crate) async fn get(State(g): State<Gateway>, headers: HeaderMap) -> Result<Response, Error> {
    admin(&g, &headers).await?;
    Ok(private(sample(&g).await?))
}
pub(crate) async fn sample(g: &Gateway) -> Result<Value, Error> {
    g.ready().await?;
    let broker = match &g.routing {
        Some(r) => Some(r.broker_available().await),
        None => None,
    };
    let connections = g.routing.as_ref().map(|r| r.counts());
    let directory = g.identity.as_ref().and_then(|i| i.directory.as_ref());
    let directory_status = match directory {
        None => json!({"configured":false,"status":"not_configured"}),
        Some(d) => match d.metrics() {
            Ok(m) => {
                json!({"configured":true,"status":if m.stale==0{"observed_healthy"}else{"degraded"},"active_subjects":m.active,"stale_subjects":m.stale,"max_evidence_age_seconds":m.max_age_seconds})
            }
            Err(_) => json!({"configured":true,"status":"unavailable"}),
        },
    };
    let degraded = broker == Some(false)
        || directory_status["status"] == "degraded"
        || directory_status["status"] == "unavailable";
    Ok(
        json!({"observed_at_ms":read::now_ms()?,"scope":"accepting_gateway_process","status":if degraded{"degraded"}else{"ready"},"components":{
        "database":{"status":"ready","source":"schema_compatibility_probe"},
        "routing":{"configured":broker.is_some(),"status":match broker{Some(true)=>"observed_healthy",Some(false)=>"degraded",None=>"not_configured"},"source":"bounded_broker_state"},
        "identity":{"configured":g.identity.is_some(),"status":if g.identity.is_some(){"configured"}else{"not_configured"}},
        "directory":directory_status,
        "recording":{"configured":g.recordings.is_some(),"status":if g.recordings.is_some(){"configured"}else{"not_configured"},"commit_durability":"not_probed"},
        "audit_reader":{"configured":g.audit.is_some(),"status":if g.audit.is_some(){"configured"}else{"not_configured"}},
        "recording_lifecycle":{"configured":g.recording_lifecycle.is_some(),"status":if g.recording_lifecycle.is_some(){"configured"}else{"not_configured"}}
    },"connections":connections.map(|(clients,hosts,pending,cross_node)|json!({"clients":clients,"hosts":hosts,"pending":pending,"cross_node_messages":cross_node})),"metrics_endpoint_public":false}),
    )
}
