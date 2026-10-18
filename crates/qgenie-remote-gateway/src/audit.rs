use crate::{Error, Store};
use qgenie_remote_proto::{
    Counter, DecisionId, MachineId, RequestId, SessionId, UserId, wire::Timestamp,
};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, PgConnection, Row};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    MachineEnrolled,
    HostConnected,
    HostActivated,
    SessionStarted,
    SessionAttached,
    SessionTerminationRequested,
    MachineKeyRecovered,
    MachineMetadataChanged,
    MachineRevoked,
    UserRevoked,
    GlobalRevoked,
    ControllerGranted,
    ControllerReleased,
    ApprovalDecided,
    AuditRead,
    TranscriptRead,
    HoldChanged,
    RetentionDeleted,
    RecordingMaintenance,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<qgenie_remote_proto::MessageId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording_lifecycle: Option<RecordingLifecycleAudit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<TranscriptReadAudit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_index: Option<TranscriptIndexAudit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_request: Option<qgenie_remote_proto::MessageId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<qgenie_remote_proto::ClientId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<UserId>,
    pub request: Option<RequestId>,
    pub decision: Option<DecisionId>,
    pub generation: Option<Counter>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingLifecycleAudit {
    pub reason: String,
    pub hold: Option<qgenie_remote_proto::rest::HoldChange>,
    pub progress: Option<qgenie_remote_proto::rest::LifecycleProgress>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptReadAudit {
    pub reason: String,
    pub case_ref: Option<String>,
    pub stream_epoch: qgenie_remote_proto::RecordingEpochId,
    pub after: Option<String>,
    pub limit: u16,
    pub include_held: bool,
    pub content: Vec<qgenie_remote_proto::ChunkId>,
    pub gaps: Vec<qgenie_remote_proto::GapId>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptIndexAudit {
    pub reason: String,
    pub case_ref: Option<String>,
    pub after: Option<String>,
    pub limit: u16,
    pub include_held: bool,
    pub streams: Vec<qgenie_remote_proto::RecordingEpochId>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub actor: UserId,
    pub action: AuditAction,
    pub machine: Option<MachineId>,
    pub session: Option<SessionId>,
    pub metadata: AuditMetadata,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub id: Counter,
    pub at: Timestamp,
    #[serde(flatten)]
    pub event: AuditEvent,
}
impl Store {
    /// Write within the domain transaction; neither operation can commit alone.
    pub(crate) async fn append_audit(
        transaction: &mut PgConnection,
        event: &AuditEvent,
    ) -> Result<(), Error> {
        if !transaction.is_in_transaction() {
            return Err(Error::Database);
        }
        sqlx::query("INSERT INTO qgr.audit_events(actor,action,machine,session,metadata) VALUES($1,$2,$3,$4,$5)")
            .bind(event.actor.to_string()).bind(serde_json::to_value(&event.action).map_err(|_| Error::Invalid)?.as_str().ok_or(Error::Invalid)?)
            .bind(event.machine.map(|id| id.to_string())).bind(event.session.map(|id| id.to_string())).bind(serde_json::to_value(&event.metadata).map_err(|_| Error::Invalid)?)
            .execute(&mut *transaction).await.map_err(|_| Error::Database)?;
        Ok(())
    }
    pub async fn read_audit(
        &self,
        actor: UserId,
        after: Counter,
        limit: u16,
    ) -> Result<Vec<AuditRecord>, Error> {
        if limit == 0 || limit > 200 || after.0 > i64::MAX as u64 {
            return Err(Error::Invalid);
        }
        let mut transaction = self.transaction().await?;
        Self::append_audit(
            &mut transaction,
            &AuditEvent {
                actor,
                action: AuditAction::AuditRead,
                machine: None,
                session: None,
                metadata: AuditMetadata::default(),
            },
        )
        .await?;
        let rows=sqlx::query("SELECT event_id,at,actor,action,machine,session,metadata FROM qgr.audit_events WHERE event_id>$1 ORDER BY event_id LIMIT $2").bind(after.0 as i64).bind(i64::from(limit)).fetch_all(&mut *transaction).await.map_err(|_| Error::Database)?;
        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let at: time::OffsetDateTime = row.get("at");
            records.push(AuditRecord {
                id: Counter(row.get::<i64, _>("event_id") as u64),
                at: Timestamp::from_unix_millis(
                    (at.unix_timestamp_nanos() / 1_000_000)
                        .try_into()
                        .map_err(|_| Error::Database)?,
                )
                .map_err(|_| Error::Database)?,
                event: AuditEvent {
                    actor: row
                        .get::<String, _>("actor")
                        .parse()
                        .map_err(|_| Error::Database)?,
                    action: serde_json::from_value(serde_json::Value::String(row.get("action")))
                        .map_err(|_| Error::Database)?,
                    machine: row
                        .get::<Option<String>, _>("machine")
                        .map(|id| id.parse())
                        .transpose()
                        .map_err(|_| Error::Database)?,
                    session: row
                        .get::<Option<String>, _>("session")
                        .map(|id| id.parse())
                        .transpose()
                        .map_err(|_| Error::Database)?,
                    metadata: serde_json::from_value(row.get("metadata"))
                        .map_err(|_| Error::Database)?,
                },
            });
        }
        transaction.commit().await?;
        Ok(records)
    }
}
