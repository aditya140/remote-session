//! Private native application journal. Reserving an answer poisons it before vendor I/O.
//! The runtime must bind sources to its owned adapter and recheck original access/current
//! policy at actual native delivery. This component grants no network authority by itself.
use fs2::FileExt;
use qgenie_remote_proto::{
    ipc::RemoteScope,
    wire::{ApprovalRequest, ApprovalWinner},
    *,
};
use qgenie_remote_transport::ipc::{private_directory, private_file};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalSource {
    pub scope: RemoteScope,
    pub session: SessionId,
    pub instance: MessageId,
}
struct Inner {
    connection: Mutex<Connection>,
    _owner: std::fs::File,
}
#[derive(Clone)]
pub struct NativeApprovals {
    inner: Arc<Inner>,
}
pub enum NativeApprovalReservation {
    Send(Box<NativeApprovalIntent>),
    Known(ApplicationOutcome),
}
/// Keep this value until the native attempt finishes. Dropping/cancelling it leaves
/// a durable offered tombstone; another delivery returns OutcomeUnknown, never Send.
pub struct NativeApprovalIntent {
    journal: NativeApprovals,
    source: ApprovalSource,
    approval: ApprovalId,
    winner: String,
    request: ApprovalRequest,
}
impl NativeApprovalIntent {
    pub fn request(&self) -> &ApprovalRequest {
        &self.request
    }
    /// Call only with actual adapter evidence. A successful native write generally
    /// proves OutcomeUnknown; it does not prove Applied or AlreadyApplied.
    pub fn complete(
        self,
        outcome: ApplicationOutcome,
        reason: Option<&str>,
    ) -> Result<(), RemoteError> {
        if reason.is_some_and(|s| s.len() > 1024) {
            return Err(invalid());
        }
        let mut connection = self
            .journal
            .inner
            .connection
            .lock()
            .map_err(|_| storage())?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| storage())?;
        checked_source(&tx, &self.source)?;
        let encoded = serde_json::to_string(&outcome).map_err(|_| invalid())?;
        let changed=tx.execute("UPDATE requests SET state='done',outcome=?4,reason=?5 WHERE id=?1 AND source=?2 AND winner=?3 AND state='offered'",params![self.approval.to_string(),serde_json::to_string(&self.source).map_err(|_|invalid())?,self.winner,encoded,reason]).map_err(|_|storage())?;
        if changed != 1 {
            let previous: Option<String> = tx
                .query_row(
                    "SELECT outcome FROM requests WHERE id=?1 AND winner=?2",
                    params![self.approval.to_string(), self.winner],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| storage())?
                .flatten();
            if previous.as_deref() != Some(&encoded) {
                return Err(conflict());
            }
        }
        tx.commit().map_err(|_| storage())
    }
}
fn invalid() -> RemoteError {
    RemoteError::rejected(
        ErrorCode::InvalidFrame,
        "invalid native approval identity or bounds",
    )
}
fn conflict() -> RemoteError {
    RemoteError::rejected(
        ErrorCode::AlreadyResolved,
        "native approval already has a different immutable decision",
    )
}
fn forbidden() -> RemoteError {
    RemoteError::rejected(
        ErrorCode::Forbidden,
        "native approval source or owner differs",
    )
}
fn storage() -> RemoteError {
    RemoteError::unknown(
        "native approval journal unavailable; no answer may be resent without journal reconciliation",
    )
}
fn checked_source(connection: &Connection, source: &ApprovalSource) -> Result<bool, RemoteError> {
    source.scope.validate()?;
    let row: Option<(String, bool)> = connection
        .query_row(
            "SELECT body,active FROM sources WHERE session=?1",
            [source.session.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|_| storage())?;
    let Some((body, active)) = row else {
        return Err(forbidden());
    };
    if body != serde_json::to_string(source).map_err(|_| invalid())? {
        return Err(forbidden());
    }
    Ok(active)
}
impl NativeApprovals {
    /// Reuse the first immutable request body when the exact owned native source
    /// re-emits an ID. Reconnect never creates a new approval or extends its time.
    pub fn native_request(
        &self,
        source: &ApprovalSource,
        native_id: &str,
    ) -> Result<Option<ApprovalRequest>, RemoteError> {
        let connection = self.inner.connection.lock().map_err(|_| storage())?;
        checked_source(&connection, source)?;
        let body: Option<String> = connection
            .query_row(
                "SELECT body FROM requests WHERE session=?1 AND source=?2 AND native_id=?3",
                params![
                    source.session.to_string(),
                    serde_json::to_string(source).map_err(|_| invalid())?,
                    native_id
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| storage())?;
        body.map(|body| serde_json::from_str(&body).map_err(|_| storage()))
            .transpose()
    }
    pub fn pending(&self, scope: &RemoteScope) -> Result<Vec<ApprovalRequest>, RemoteError> {
        self.pending_for_session(scope, None)
    }
    pub fn pending_for_session(
        &self,
        scope: &RemoteScope,
        session: Option<SessionId>,
    ) -> Result<Vec<ApprovalRequest>, RemoteError> {
        scope.validate()?;
        let connection = self.inner.connection.lock().map_err(|_| storage())?;
        let mut query = connection.prepare("SELECT r.body FROM requests r JOIN sources s ON r.session=s.session WHERE s.active=1 AND r.state='pending' AND json_extract(r.source,'$.scope.origin')=?1 AND json_extract(r.source,'$.scope.machine')=?2 AND json_extract(r.source,'$.scope.owner')=?3 AND (?4 IS NULL OR r.session=?4) ORDER BY r.id LIMIT 201").map_err(|_| storage())?;
        let rows = query
            .query_map(
                params![
                    scope.origin,
                    scope.machine.to_string(),
                    scope.owner.to_string(),
                    session.map(|session| session.to_string())
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| storage())?;
        let mut pending = Vec::new();
        let mut bytes = 0;
        for body in rows {
            let body = body.map_err(|_| storage())?;
            bytes += body.len();
            if pending.len() >= 200 || bytes > 512 * 1024 {
                return Err(RemoteError::rejected(
                    ErrorCode::Overloaded,
                    "pending native approvals require pagination",
                ));
            }
            pending.push(serde_json::from_str(&body).map_err(|_| storage())?);
        }
        Ok(pending)
    }
    pub fn deactivate_source(&self, source: &ApprovalSource) -> Result<(), RemoteError> {
        let mut connection = self.inner.connection.lock().map_err(|_| storage())?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| storage())?;
        checked_source(&tx, source)?;
        let source = serde_json::to_string(source).map_err(|_| invalid())?;
        tx.execute("UPDATE requests SET outcome=CASE WHEN state='pending' THEN '\"cancelled\"' ELSE '\"outcome_unknown\"' END,state=CASE WHEN state='pending' THEN 'cancelled' ELSE 'done' END WHERE source=?1 AND state IN ('pending','offered')", [&source]).map_err(|_| storage())?;
        tx.execute("UPDATE sources SET active=0 WHERE body=?1", [source])
            .map_err(|_| storage())?;
        tx.commit().map_err(|_| storage())
    }
    pub fn open(directory: &Path) -> anyhow::Result<Self> {
        private_directory(directory)?;
        let owner = private_file(&directory.join("approvals.lock"))?;
        owner.try_lock_exclusive()?;
        let path = directory.join("approvals.sqlite");
        private_file(&path)?;
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA max_page_count=16384;")?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        anyhow::ensure!(version <= 1, "unsupported native approval schema");
        if version == 0 {
            connection.execute_batch(include_str!("../migrations/approvals_0001.sql"))?;
        }
        // No native ownership is adopted across journal-owner restart. Pending
        // requests are cancelled; possibly offered answers remain uncertain.
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE sources SET active=0", [])?;
        tx.execute("UPDATE requests SET state='cancelled',outcome='\"cancelled\"',reason='native approval owner restarted before delivery' WHERE state='pending'",[])?;
        tx.execute("UPDATE requests SET state='done',outcome='\"outcome_unknown\"',reason='native approval owner restarted after reservation' WHERE state='offered'",[])?;
        tx.commit()?;
        Ok(Self {
            inner: Arc::new(Inner {
                connection: Mutex::new(connection),
                _owner: owner,
            }),
        })
    }
    /// Trusted native ownership registration, not an externally callable adoption API.
    pub fn register_source(&self, source: &ApprovalSource) -> Result<(), RemoteError> {
        source.scope.validate()?;
        let mut connection = self.inner.connection.lock().map_err(|_| storage())?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| storage())?;
        let owner: Option<String> = tx
            .query_row(
                "SELECT owner FROM bindings WHERE origin=?1 AND machine=?2",
                params![source.scope.origin, source.scope.machine.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| storage())?;
        if owner
            .as_deref()
            .is_some_and(|owner| owner != source.scope.owner.to_string())
        {
            return Err(forbidden());
        }
        tx.execute(
            "INSERT OR IGNORE INTO bindings(origin,machine,owner) VALUES(?1,?2,?3)",
            params![
                source.scope.origin,
                source.scope.machine.to_string(),
                source.scope.owner.to_string()
            ],
        )
        .map_err(|_| storage())?;
        let existing: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sources WHERE session=?1)",
                [source.session.to_string()],
                |r| r.get(0),
            )
            .map_err(|_| storage())?;
        if existing {
            if !checked_source(&tx, source)? {
                return Err(forbidden());
            }
            return tx.commit().map_err(|_| storage());
        }
        let count: u32 = tx
            .query_row("SELECT count(*) FROM sources", [], |r| r.get(0))
            .map_err(|_| storage())?;
        if count >= 4096 {
            return Err(RemoteError::rejected(
                ErrorCode::CapacityExhausted,
                "native approval source registry is full",
            ));
        }
        tx.execute(
            "INSERT INTO sources(session,body,active) VALUES(?1,?2,1)",
            params![
                source.session.to_string(),
                serde_json::to_string(source).map_err(|_| invalid())?
            ],
        )
        .map_err(|_| storage())?;
        tx.commit().map_err(|_| storage())
    }
    pub fn record(
        &self,
        source: &ApprovalSource,
        request: &ApprovalRequest,
    ) -> Result<(), RemoteError> {
        request.validate()?;
        if request.session != source.session || request.owner != source.scope.owner {
            return Err(forbidden());
        }
        let mut connection = self.inner.connection.lock().map_err(|_| storage())?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| storage())?;
        if !checked_source(&tx, source)? {
            return Err(forbidden());
        }
        let body = serde_json::to_string(request).map_err(|_| invalid())?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT body FROM requests WHERE id=?1 OR (session=?2 AND native_id=?3)",
                params![
                    request.id.to_string(),
                    source.session.to_string(),
                    request.native_id
                ],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| storage())?;
        if let Some(previous) = existing {
            if previous != body {
                return Err(conflict());
            }
            return tx.commit().map_err(|_| storage());
        }
        let (count, pending): (u32, u32) = tx
            .query_row(
                "SELECT count(*),COALESCE(sum(state='pending'),0) FROM requests WHERE session=?1",
                [source.session.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| storage())?;
        if count >= 4096 || pending >= 32 {
            return Err(RemoteError::rejected(
                ErrorCode::CapacityExhausted,
                "native approval journal capacity reached",
            ));
        }
        tx.execute("INSERT INTO requests(id,session,source,native_id,body,state) VALUES(?1,?2,?3,?4,?5,'pending')",params![request.id.to_string(),source.session.to_string(),serde_json::to_string(source).map_err(|_|invalid())?,request.native_id,body]).map_err(|_|storage())?;
        tx.commit().map_err(|_| storage())
    }
    /// The caller rechecks its original native access and current host policy after
    /// any SQLite wait and again immediately before handing Send to the adapter.
    pub fn reserve(
        &self,
        source: &ApprovalSource,
        winner: &ApprovalWinner,
        current_policy: Counter,
        now_ms: u64,
    ) -> Result<NativeApprovalReservation, RemoteError> {
        winner.validate()?;
        if winner.actor != source.scope.owner {
            return Err(forbidden());
        }
        let mut connection = self.inner.connection.lock().map_err(|_| storage())?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| storage())?;
        let active = checked_source(&tx, source)?;
        let row: Option<(String, String, Option<String>, Option<String>)> = tx
            .query_row(
                "SELECT body,state,winner,outcome FROM requests WHERE id=?1 AND source=?2",
                params![
                    winner.approval.to_string(),
                    serde_json::to_string(source).map_err(|_| invalid())?
                ],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(|_| storage())?;
        let Some((body, state, previous, outcome)) = row else {
            return Err(RemoteError::rejected(
                ErrorCode::NotFound,
                "native approval request is absent",
            ));
        };
        let request: ApprovalRequest = decode_json(body.as_bytes())?;
        if winner.request_hash != request.request_hash
            || winner.policy_revision != request.policy_revision
        {
            return Err(conflict());
        }
        let encoded = serde_json::to_string(winner).map_err(|_| invalid())?;
        if let Some(previous) = previous {
            if previous != encoded {
                return Err(conflict());
            }
            let outcome = outcome
                .map(|s| decode_json(s.as_bytes()))
                .transpose()?
                .unwrap_or(ApplicationOutcome::OutcomeUnknown);
            tx.commit().map_err(|_| storage())?;
            return Ok(NativeApprovalReservation::Known(outcome));
        }
        let cancelled = !active
            || state != "pending"
            || current_policy != request.policy_revision
            || !request.allowed_decisions.contains(&winner.decision)
            || request
                .expires_at
                .as_ref()
                .is_some_and(|at| now_ms >= at.unix_millis());
        let outcome = cancelled.then_some("\"cancelled\"");
        tx.execute(
            "UPDATE requests SET winner=?2,state=?3,outcome=?4 WHERE id=?1",
            params![
                winner.approval.to_string(),
                encoded,
                if cancelled { "cancelled" } else { "offered" },
                outcome
            ],
        )
        .map_err(|_| storage())?;
        tx.commit().map_err(|_| storage())?;
        if cancelled {
            Ok(NativeApprovalReservation::Known(
                ApplicationOutcome::Cancelled,
            ))
        } else {
            Ok(NativeApprovalReservation::Send(Box::new(
                NativeApprovalIntent {
                    journal: self.clone(),
                    source: source.clone(),
                    approval: winner.approval,
                    winner: encoded,
                    request,
                },
            )))
        }
    }
    /// A native resolution without an execution proof cannot establish Applied.
    pub fn resolved(&self, source: &ApprovalSource, id: ApprovalId) -> Result<(), RemoteError> {
        let mut connection = self.inner.connection.lock().map_err(|_| storage())?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| storage())?;
        checked_source(&tx, source)?;
        let changed=tx.execute("UPDATE requests SET outcome=CASE WHEN state='pending' THEN '\"cancelled\"' ELSE '\"outcome_unknown\"' END,state=CASE WHEN state='pending' THEN 'cancelled' ELSE 'done' END WHERE id=?1 AND source=?2 AND state IN ('pending','offered')",params![id.to_string(),serde_json::to_string(source).map_err(|_|invalid())?]).map_err(|_|storage())?;
        if changed > 1 {
            return Err(storage());
        }
        tx.commit().map_err(|_| storage())
    }
}
#[cfg(test)]
mod tests;
