use super::*;
use qgenie_remote_proto::rest::{
    AdminHistoryStreamsQuery, HistoryStream, HistoryStreamsPage, HistoryStreamsQuery,
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexCursor {
    v: u8,
    session: SessionId,
    epoch: String,
    held: bool,
    case_hash: Option<String>,
    after: RecordingEpochId,
}
impl RecordingDatabase {
    pub async fn owner_history_streams(
        &self,
        identity: &IdentityService,
        actor: &Principal,
        session: SessionId,
        query: HistoryStreamsQuery,
    ) -> Result<HistoryStreamsPage, Error> {
        self.history_index(identity, actor, session, query, None)
            .await
    }
    pub async fn admin_history_streams(
        &self,
        identity: &IdentityService,
        actor: &Principal,
        session: SessionId,
        query: AdminHistoryStreamsQuery,
    ) -> Result<HistoryStreamsPage, Error> {
        if !actor.platform_admin {
            return Err(Error::Forbidden);
        }
        let admin = admin_context(query.reason, query.case_ref, query.include_held)?;
        self.history_index(identity, actor, session, query.history, Some(admin))
            .await
    }
    async fn history_index(
        &self,
        identity: &IdentityService,
        actor: &Principal,
        session: SessionId,
        query: HistoryStreamsQuery,
        admin: Option<Admin>,
    ) -> Result<HistoryStreamsPage, Error> {
        query.validate().map_err(|_| Error::Invalid)?;
        let (mut tx, scope, access) = self
            .begin_read(identity, actor, session, admin.as_ref(), false)
            .await?;
        let held = admin.as_ref().is_some_and(|v| v.held);
        let case_hash = admin
            .as_ref()
            .and_then(|v| v.case_ref.as_ref())
            .map(qgenie_remote_proto::fingerprint)
            .transpose()
            .map_err(|_| Error::Invalid)?;
        let after = if let Some(cursor) = &query.after {
            let bytes = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| Error::Invalid)?;
            let cursor: IndexCursor =
                qgenie_remote_proto::decode_json(&bytes).map_err(|_| Error::Invalid)?;
            if cursor.v != 1
                || cursor.session != session
                || cursor.held != held
                || cursor.case_hash != case_hash
            {
                return Err(Error::Invalid);
            }
            if cursor.epoch != scope.epoch {
                return Err(Error::HistoryTruncated);
            }
            cursor.after.to_string()
        } else {
            String::new()
        };
        // Only fixed service-selected table names enter SQL; RLS is established by the
        // same current primary context as content pages. Owner SQL never names held data.
        let tables = if held {
            vec![("content", "gaps"), ("held_content", "held_gaps")]
        } else {
            vec![("content", "gaps")]
        };
        let mut sources = Vec::new();
        for (content, gaps) in tables {
            sources.push(format!("SELECT r.immutable->'batch'->>'stream_epoch' AS epoch,r.immutable->'batch'->'stream' AS stream FROM qgr_recordings.{content} c JOIN qgr_recordings.receipts r ON r.machine=c.machine AND r.session=c.session AND r.id=c.id AND r.kind='content' WHERE c.machine=$1 AND c.session=$2"));
            sources.push(format!("SELECT g.body->>'stream_epoch' AS epoch,g.body->'stream' AS stream FROM qgr_recordings.{gaps} g WHERE g.machine=$1 AND g.session=$2"));
        }
        let sql = format!(
            "WITH visible AS ({}) SELECT DISTINCT epoch,stream FROM visible WHERE epoch>$3 ORDER BY epoch,stream LIMIT $4",
            sources.join(" UNION ALL ")
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(scope.machine.to_string())
            .bind(session.to_string())
            .bind(after)
            .bind(i64::from(query.limit) + 1)
            .fetch_all(&mut *tx)
            .await
            .map_err(db)?;
        let more = rows.len() > usize::from(query.limit);
        let mut streams = Vec::new();
        for row in rows.into_iter().take(usize::from(query.limit)) {
            streams.push(HistoryStream {
                stream_epoch: row
                    .try_get::<String, _>("epoch")
                    .map_err(db)?
                    .parse()
                    .map_err(|_| Error::Database)?,
                stream: serde_json::from_value(row.try_get("stream").map_err(db)?)
                    .map_err(|_| Error::Database)?,
            });
        }
        let next = if more {
            streams
                .last()
                .map(|last| {
                    serde_json::to_vec(&IndexCursor {
                        v: 1,
                        session,
                        epoch: scope.epoch.clone(),
                        held,
                        case_hash: case_hash.clone(),
                        after: last.stream_epoch,
                    })
                    .map(|v| URL_SAFE_NO_PAD.encode(v))
                    .map_err(|_| Error::Database)
                })
                .transpose()?
        } else {
            None
        };
        if let Some(admin) = admin {
            Store::append_audit(
                &mut tx,
                &AuditEvent {
                    actor: actor.user,
                    action: AuditAction::TranscriptRead,
                    machine: Some(scope.machine),
                    session: Some(session),
                    metadata: AuditMetadata {
                        subject: Some(scope.owner),
                        transcript_index: Some(crate::audit::TranscriptIndexAudit {
                            reason: admin.reason,
                            case_ref: admin.case_ref,
                            after: query.after,
                            limit: query.limit,
                            include_held: held,
                            streams: streams.iter().map(|s| s.stream_epoch).collect(),
                        }),
                        ..Default::default()
                    },
                },
            )
            .await?;
        }
        current(identity, actor, &access)?;
        tx.commit().await?;
        current(identity, actor, &access)?;
        Ok(HistoryStreamsPage {
            session,
            machine: scope.machine,
            streams,
            next,
        })
    }
}
