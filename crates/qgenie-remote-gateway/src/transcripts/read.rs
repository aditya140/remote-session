mod index;
use super::transaction::RecordingTransaction as Transaction;
use super::{
    RecordingDatabase, RecordingRole,
    ingest::{AccessWindow, db},
};
use crate::{
    AuditAction, AuditEvent, AuditMetadata, Error, IdentityService, Principal, Store,
    TranscriptReadAudit,
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use qgenie_remote_proto::{
    ChunkId, Counter, GapId, MachineId, RecordingEpochId, SessionId, UserId,
    rest::{AdminHistoryQuery, HistoricalChunk, HistoryItem, HistoryPage, HistoryQuery},
    wire::{LossRange, RecordingBatch},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Row, postgres::PgRow};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Position {
    seq: Counter,
    fragment: u32,
    kind: u8,
    id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    session: SessionId,
    stream_epoch: RecordingEpochId,
    backend_epoch: String,
    held: bool,
    case_hash: Option<String>,
    position: Position,
}
struct Scope {
    machine: MachineId,
    owner: UserId,
    epoch: String,
}
struct Admin {
    reason: String,
    case_ref: Option<String>,
    held: bool,
}

impl RecordingDatabase {
    /// Owner path uses the isolated owner pool even when this user also has an admin role.
    pub async fn owner_history(
        &self,
        identity: &IdentityService,
        actor: &Principal,
        session: SessionId,
        query: HistoryQuery,
    ) -> Result<HistoryPage, Error> {
        self.history(identity, actor, session, query, None, false)
            .await
    }
    pub async fn admin_history(
        &self,
        identity: &IdentityService,
        actor: &Principal,
        session: SessionId,
        query: AdminHistoryQuery,
    ) -> Result<HistoryPage, Error> {
        if !actor.platform_admin {
            return Err(Error::Forbidden);
        }
        let admin = admin_context(query.reason, query.case_ref, query.include_held)?;
        self.history(identity, actor, session, query.history, Some(admin), false)
            .await
    }
    async fn begin_read(
        &self,
        identity: &IdentityService,
        actor: &Principal,
        session: SessionId,
        admin: Option<&Admin>,
        synthetic: bool,
    ) -> Result<(Transaction, Scope, AccessWindow), Error> {
        if (admin.is_some() && (self.role != RecordingRole::AdminRead || !actor.platform_admin))
            || (admin.is_none() && self.role != RecordingRole::OwnerRead)
        {
            return Err(Error::Forbidden);
        }
        if !synthetic && identity.directory.is_none() {
            return Err(Error::Configuration);
        }
        if !synthetic {
            let version: Option<i32> =
                sqlx::query_scalar("SELECT max(version) FROM qgr.schema_versions")
                    .fetch_one(&self.pool)
                    .await
                    .map_err(db)?;
            let version = version.ok_or(Error::Schema)?;
            if version > crate::SCHEMA_MAX {
                return Err(Error::Schema);
            }
            if version < super::RECORDING_RELEASE_SCHEMA {
                return Err(Error::Unavailable);
            }
        }
        let access = AccessWindow::user(actor.expires_at)?;
        if admin.is_some() {
            identity.assert_admin(actor.user).await?;
        } else {
            identity.assert_directory(actor.user).await?;
        }
        current(identity, actor, &access)?;
        let mut tx = self.transaction().await?;
        sqlx::raw_sql("SET LOCAL transaction_timeout='3s'; SET LOCAL statement_timeout='2s'; SET LOCAL lock_timeout='2s';").execute(&mut *tx).await.map_err(db)?;
        let row = if let Some(admin) = &admin {
            sqlx::query("SELECT * FROM qgr_recordings.admin_read_scope($1,$2,$3,$4,$5,$6)")
                .bind(actor.user.to_string())
                .bind(session.to_string())
                .bind(i64::try_from(actor.expires_at).map_err(|_| Error::Unauthenticated)?)
                .bind(&admin.reason)
                .bind(&admin.case_ref)
                .bind(admin.held)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?
        } else {
            sqlx::query("SELECT * FROM qgr_recordings.owner_read_scope($1,$2,$3)")
                .bind(actor.user.to_string())
                .bind(session.to_string())
                .bind(i64::try_from(actor.expires_at).map_err(|_| Error::Unauthenticated)?)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?
        }
        .ok_or(Error::Unavailable)?;
        if admin.is_some() && !synthetic {
            let durable: bool =
                sqlx::query_scalar("SELECT qgr_recordings.audit_durability_ready()")
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db)?;
            if !durable {
                return Err(Error::Unavailable);
            }
            sqlx::query("SET LOCAL synchronous_commit='remote_apply'")
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        let scope = Scope {
            machine: row
                .get::<String, _>("machine_id")
                .parse()
                .map_err(|_| Error::Database)?,
            owner: row
                .get::<String, _>("owner_id")
                .parse()
                .map_err(|_| Error::Database)?,
            epoch: row.get("recovery_epoch"),
        };
        Ok((tx, scope, access))
    }
    async fn history(
        &self,
        identity: &IdentityService,
        actor: &Principal,
        session: SessionId,
        query: HistoryQuery,
        admin: Option<Admin>,
        synthetic: bool,
    ) -> Result<HistoryPage, Error> {
        query.validate().map_err(|_| Error::Invalid)?;
        let (mut tx, scope, access) = self
            .begin_read(identity, actor, session, admin.as_ref(), synthetic)
            .await?;
        let case_hash = admin
            .as_ref()
            .and_then(|a| a.case_ref.as_ref())
            .map(qgenie_remote_proto::fingerprint)
            .transpose()
            .map_err(|_| Error::Invalid)?;
        let held = admin.as_ref().is_some_and(|a| a.held);
        let after = if let Some(token) = &query.after {
            let cursor = decode_cursor(token)?;
            if cursor.session != session
                || cursor.stream_epoch != query.stream_epoch
                || cursor.held != held
                || cursor.case_hash != case_hash
            {
                return Err(Error::Invalid);
            }
            if cursor.backend_epoch != scope.epoch {
                return Err(Error::HistoryTruncated);
            }
            cursor.position
        } else {
            Position {
                seq: Counter(0),
                fragment: 0,
                kind: 0,
                id: String::new(),
            }
        };
        let mut candidates = BTreeMap::new();
        // Case reads prefer an already preserved copy when the same immutable ID exists in
        // both stores. Each source is bounded to page+one lookahead; no unbounded buffering.
        for preserved in if held { vec![true, false] } else { vec![false] } {
            for row in content_rows(&mut tx, &scope, session, &query, &after, preserved).await? {
                let (position, item) = content(&row, &scope, session, query.stream_epoch)?;
                candidates.entry(position).or_insert(item);
            }
            for row in gap_rows(&mut tx, &scope, session, &query, &after, preserved).await? {
                let gap: LossRange =
                    serde_json::from_value(row.get("body")).map_err(|_| Error::Database)?;
                gap.validate().map_err(|_| Error::Database)?;
                let id: String = row.get("id");
                if gap.id.to_string() != id
                    || gap.session != session
                    || gap.stream_epoch != query.stream_epoch
                {
                    return Err(Error::Database);
                }
                candidates
                    .entry(Position {
                        seq: gap.from,
                        fragment: 0,
                        kind: 0,
                        id,
                    })
                    .or_insert(HistoryItem::Gap(gap));
            }
        }
        let more = candidates.len() > usize::from(query.limit);
        let selected: Vec<_> = candidates
            .into_iter()
            .take(usize::from(query.limit))
            .collect();
        let next = if more {
            selected
                .last()
                .map(|(position, _)| {
                    encode_cursor(Cursor {
                        v: 1,
                        session,
                        stream_epoch: query.stream_epoch,
                        backend_epoch: scope.epoch.clone(),
                        held,
                        case_hash,
                        position: position.clone(),
                    })
                })
                .transpose()?
        } else {
            None
        };
        let items: Vec<_> = selected.into_iter().map(|(_, item)| item).collect();
        if let Some(admin) = admin {
            let mut content = Vec::new();
            let mut gaps = Vec::new();
            for item in &items {
                match item {
                    HistoryItem::Content(c) => content.push(c.id),
                    HistoryItem::Gap(g) => gaps.push(g.id),
                }
            }
            Store::append_audit(
                &mut tx,
                &AuditEvent {
                    actor: actor.user,
                    action: AuditAction::TranscriptRead,
                    machine: Some(scope.machine),
                    session: Some(session),
                    metadata: AuditMetadata {
                        subject: Some(scope.owner),
                        transcript: Some(TranscriptReadAudit {
                            reason: admin.reason,
                            case_ref: admin.case_ref,
                            stream_epoch: query.stream_epoch,
                            after: query.after.clone(),
                            limit: query.limit,
                            include_held: held,
                            content,
                            gaps,
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
        Ok(HistoryPage {
            session,
            machine: scope.machine,
            stream_epoch: query.stream_epoch,
            items,
            next,
        })
    }
}

fn current(
    identity: &IdentityService,
    actor: &Principal,
    access: &AccessWindow,
) -> Result<(), Error> {
    access.check()?;
    if let Some(directory) = &identity.directory {
        directory.cached(actor.user)?;
    }
    if crate::identity::jwt::unix_seconds()? >= actor.expires_at {
        return Err(Error::Unauthenticated);
    }
    Ok(())
}
fn admin_context(reason: String, case_ref: Option<String>, held: bool) -> Result<Admin, Error> {
    let reason = reason.trim().to_owned();
    let case_ref = case_ref.map(|v| v.trim().to_owned());
    if reason.is_empty()
        || reason.len() > 512
        || reason.contains('\0')
        || case_ref
            .as_ref()
            .is_some_and(|v| v.is_empty() || v.len() > 256 || v.contains('\0'))
        || (held && case_ref.is_none())
    {
        return Err(Error::Invalid);
    }
    Ok(Admin {
        reason,
        case_ref,
        held,
    })
}
fn encode_cursor(cursor: Cursor) -> Result<String, Error> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).map_err(|_| Error::Invalid)?))
}
fn decode_cursor(token: &str) -> Result<Cursor, Error> {
    let bytes = URL_SAFE_NO_PAD.decode(token).map_err(|_| Error::Invalid)?;
    let cursor: Cursor = qgenie_remote_proto::decode_json(&bytes).map_err(|_| Error::Invalid)?;
    if cursor.v != 1
        || cursor.position.seq.0 == 0
        || cursor.position.fragment > 32
        || cursor.position.kind > 1
        || cursor.backend_epoch.is_empty()
        || cursor.backend_epoch.len() > 128
        || encode_cursor(Cursor {
            v: cursor.v,
            session: cursor.session,
            stream_epoch: cursor.stream_epoch,
            backend_epoch: cursor.backend_epoch.clone(),
            held: cursor.held,
            case_hash: cursor.case_hash.clone(),
            position: cursor.position.clone(),
        })? != token
    {
        return Err(Error::Invalid);
    }
    if cursor.position.kind == 0 {
        if cursor.position.fragment != 0 || cursor.position.id.parse::<GapId>().is_err() {
            return Err(Error::Invalid);
        }
    } else if cursor.position.id.parse::<ChunkId>().is_err() {
        return Err(Error::Invalid);
    }
    Ok(cursor)
}
async fn content_rows(
    tx: &mut Transaction,
    scope: &Scope,
    session: SessionId,
    query: &HistoryQuery,
    after: &Position,
    held: bool,
) -> Result<Vec<PgRow>, Error> {
    let table = if held { "held_content" } else { "content" };
    // Identifier comes only from the closed choice above; all request values are bound.
    let sql = format!(
        "WITH candidate AS (SELECT c.id,c.payload,c.stored_digest,r.immutable,r.immutable->'batch'->>'from' AS seq,coalesce((SELECT min(f.fragment) FROM qgr_recordings.fragments f WHERE f.machine=c.machine AND f.session=c.session AND f.stream_epoch=$3 AND f.chunk=c.id AND f.seq=(r.immutable->'batch'->>'from')::numeric),-1) AS part FROM qgr_recordings.{table} c JOIN qgr_recordings.receipts r ON r.machine=c.machine AND r.session=c.session AND r.id=c.id AND r.kind='content' WHERE c.machine=$1 AND c.session=$2 AND r.immutable->'batch'->>'stream_epoch'=$3) SELECT * FROM candidate WHERE (seq::numeric,part,1,id)>($4::numeric,$5,$6,$7) ORDER BY seq::numeric,part,id LIMIT $8"
    );
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(scope.machine.to_string())
        .bind(session.to_string())
        .bind(query.stream_epoch.to_string())
        .bind(after.seq.0.to_string())
        .bind(i32::try_from(after.fragment).map_err(|_| Error::Invalid)?)
        .bind(i32::from(after.kind))
        .bind(&after.id)
        .bind(i64::from(query.limit) + 1)
        .fetch_all(&mut **tx)
        .await
        .map_err(db)
}
async fn gap_rows(
    tx: &mut Transaction,
    scope: &Scope,
    session: SessionId,
    query: &HistoryQuery,
    after: &Position,
    held: bool,
) -> Result<Vec<PgRow>, Error> {
    let table = if held { "held_gaps" } else { "gaps" };
    let sql = format!(
        "SELECT g.id,g.body FROM qgr_recordings.{table} g WHERE g.machine=$1 AND g.session=$2 AND g.body->>'stream_epoch'=$3 AND ((g.body->>'from')::numeric,0,0,g.id)>($4::numeric,$5,$6,$7) ORDER BY (g.body->>'from')::numeric,g.id LIMIT $8"
    );
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(scope.machine.to_string())
        .bind(session.to_string())
        .bind(query.stream_epoch.to_string())
        .bind(after.seq.0.to_string())
        .bind(i32::try_from(after.fragment).map_err(|_| Error::Invalid)?)
        .bind(i32::from(after.kind))
        .bind(&after.id)
        .bind(i64::from(query.limit) + 1)
        .fetch_all(&mut **tx)
        .await
        .map_err(db)
}
fn content(
    row: &PgRow,
    scope: &Scope,
    session: SessionId,
    epoch: RecordingEpochId,
) -> Result<(Position, HistoryItem), Error> {
    let immutable: Value = row.get("immutable");
    let batch: RecordingBatch =
        serde_json::from_value(immutable.get("batch").cloned().ok_or(Error::Database)?)
            .map_err(|_| Error::Database)?;
    batch.validate_metadata().map_err(|_| Error::Database)?;
    let id: String = row.get("id");
    if batch.id.to_string() != id
        || batch.machine != scope.machine
        || batch.session != session
        || batch.stream_epoch != epoch
    {
        return Err(Error::Database);
    }
    let stored = immutable.get("stored").ok_or(Error::Database)?;
    let size = stored
        .get("uncompressed_bytes")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0 && *v <= 1024 * 1024)
        .ok_or(Error::Database)?;
    let payload: Vec<u8> = row.get("payload");
    let digest: String = row.get("stored_digest");
    qgenie_remote_proto::wire::validate_digest(&digest).map_err(|_| Error::Database)?;
    if payload.is_empty()
        || payload.len() > 64 * 1024
        || format!("{:x}", Sha256::digest(&payload)) != digest
        || zstd::zstd_safe::find_frame_compressed_size(&payload).map_err(|_| Error::Database)?
            != payload.len()
        || zstd::zstd_safe::get_frame_content_size(&payload).map_err(|_| Error::Database)?
            != Some(size)
    {
        return Err(Error::Database);
    }
    let fragment: u32 = row
        .get::<i32, _>("part")
        .try_into()
        .map_err(|_| Error::Database)?;
    if fragment > 32 {
        return Err(Error::Database);
    }
    let position = Position {
        seq: batch.from,
        fragment,
        kind: 1,
        id,
    };
    let item = HistoricalChunk {
        id: batch.id,
        session,
        machine: scope.machine,
        stream: batch.stream,
        stream_epoch: epoch,
        from: batch.from,
        to: batch.to,
        captured_from: batch.captured_from,
        captured_to: batch.captured_to,
        codec: batch.codec,
        uncompressed_bytes: size as u32,
        stored_digest: digest,
        data: STANDARD.encode(payload),
        redaction_saturated: stored
            .get("redaction_saturated")
            .and_then(Value::as_bool)
            .ok_or(Error::Database)?,
    };
    Ok((position, HistoryItem::Content(item)))
}

#[cfg(test)]
pub(crate) mod tests;
