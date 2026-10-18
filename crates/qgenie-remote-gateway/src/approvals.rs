//! Durable gateway choice and delivery ledger. Native execution is never inferred from commit.
//! Callers supply live authenticated owner/host scope and directory freshness; public routing
//! and native journaling must compose these operations before advertising approval capability.
use crate::{
    AuditAction, AuditEvent, AuditMetadata, ControllerScope, Error, Store,
    controllers::lock_owned_scope,
};
use qgenie_remote_proto::{
    wire::{ApprovalDecision, ApprovalRequest, ApprovalWinner, Timestamp},
    *,
};
use sqlx::{Row, postgres::PgRow};

pub type ApprovalProjection = qgenie_remote_proto::wire::ApprovalState;
pub struct ApprovalChoice {
    pub winner: ApprovalWinner,
    pub chosen: bool,
}
#[derive(Clone)]
pub struct ApprovalDelivery {
    pub request: ApprovalRequest,
    pub winner: ApprovalWinner,
    pub claim: MessageId,
    pub attempts: u32,
}
fn db(_: sqlx::Error) -> Error {
    Error::Database
}
fn value<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, Error> {
    serde_json::to_value(value).map_err(|_| Error::Invalid)
}
fn read<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|_| Error::Database)
}
fn projection(row: PgRow) -> Result<ApprovalProjection, Error> {
    Ok(ApprovalProjection {
        machine: row
            .get::<String, _>("machine")
            .parse()
            .map_err(|_| Error::Database)?,
        request: read(row.get("request"))?,
        winner: row
            .get::<Option<serde_json::Value>, _>("winner")
            .map(read)
            .transpose()?,
        cancelled: row.get::<String, _>("state") != "pending",
        outcome: row
            .get::<Option<String>, _>("outcome")
            .map(|value| read(serde_json::Value::String(value)))
            .transpose()?,
        reason: row.get("reason"),
    })
}
async fn lock_policy(
    tx: &mut sqlx::PgConnection,
    scope: &ControllerScope,
    revision: Counter,
    kind: &str,
) -> Result<(), Error> {
    lock_owned_scope(tx, scope, true).await?;
    let current:bool=sqlx::query_scalar("SELECT (r.native_access->>'policy_revision')=$2 AND COALESCE(s.body->'capabilities'->'approvals' ? $4,false) FROM qgr.machine_routes r JOIN qgr.sessions s ON s.machine=r.machine WHERE r.machine=$1 AND s.id=$3").bind(scope.machine.to_string()).bind(revision.0.to_string()).bind(scope.session.to_string()).bind(kind).fetch_one(&mut *tx).await.map_err(db)?;
    if current {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}
impl Store {
    pub(crate) async fn approval_scope(
        &self,
        owner: UserId,
        approval: ApprovalId,
    ) -> Result<ControllerScope, Error> {
        let row=sqlx::query("SELECT a.owner,a.machine,a.session,r.generation::text AS generation,r.connection FROM qgr.approval_requests a JOIN qgr.machine_routes r ON r.machine=a.machine WHERE a.id=$1 AND a.owner=$2 AND r.activated AND r.native_access IS NOT NULL")
            .bind(approval.to_string()).bind(owner.to_string()).fetch_optional(&self.pool).await.map_err(db)?.ok_or(Error::Unavailable)?;
        approval_scope(row)
    }
    /// Only a gateway holding the current local host connection claims delivery.
    /// The later claim transaction rechecks current owner/key/access and route.
    pub(crate) async fn due_approval_scopes(
        &self,
        node: &str,
        machines: &[MachineId],
        limit: u16,
    ) -> Result<Vec<ControllerScope>, Error> {
        if machines.len() > 4096 || !(1..=32).contains(&limit) {
            return Err(Error::Invalid);
        }
        if machines.is_empty() {
            return Ok(Vec::new());
        }
        let machines = machines.iter().map(ToString::to_string).collect::<Vec<_>>();
        let rows=sqlx::query("SELECT a.owner,a.machine,a.session,r.generation::text AS generation,r.connection FROM qgr.approval_requests a JOIN qgr.approval_outbox o ON o.approval=a.id JOIN qgr.machine_routes r ON r.machine=a.machine JOIN qgr.machines m ON m.id=a.machine AND m.owner=a.owner JOIN qgr.users u ON u.id=a.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id CROSS JOIN qgr.access_state x WHERE r.node=$1 AND r.machine=ANY($2::text[]) AND r.activated AND r.native_access IS NOT NULL AND NOT(m.disabled OR u.disabled OR k.revoked OR x.disabled) AND u.directory_enabled IS DISTINCT FROM false AND o.outcome IS NULL AND o.due_at<=clock_timestamp() AND (o.claim_until IS NULL OR o.claim_until<=clock_timestamp()) GROUP BY a.owner,a.machine,a.session,r.generation,r.connection ORDER BY min(o.due_at),a.session LIMIT $3")
            .bind(node).bind(machines).bind(i64::from(limit)).fetch_all(&self.pool).await.map_err(db)?;
        rows.into_iter().map(approval_scope).collect()
    }
    /// Read-only reconciliation works when the host/client that chose the winner
    /// is offline. The caller still needs current owner/directory authentication.
    pub async fn owner_approval_status(
        &self,
        owner: UserId,
        id: ApprovalId,
    ) -> Result<ApprovalProjection, Error> {
        let mut tx = self.transaction().await?;
        Self::lock_access(&mut tx).await?;
        let row=sqlx::query("SELECT r.machine AS machine,r.body AS request,d.body AS winner,CASE WHEN r.state='pending' AND d.id IS NULL AND r.expires_at<=clock_timestamp() THEN 'timed_out' ELSE r.state END AS state,o.outcome,COALESCE(o.reason,r.reason) AS reason FROM qgr.approval_requests r JOIN qgr.machines m ON m.id=r.machine AND m.owner=r.owner JOIN qgr.users u ON u.id=r.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id LEFT JOIN qgr.approval_decisions d ON d.approval=r.id LEFT JOIN qgr.approval_outbox o ON o.approval=r.id WHERE r.id=$1 AND r.owner=$2 AND NOT(m.disabled OR u.disabled OR k.revoked) AND u.directory_enabled IS DISTINCT FROM false FOR SHARE OF r,m,u,k")
            .bind(id.to_string()).bind(owner.to_string()).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let result = projection(row)?;
        result.validate().map_err(|_| Error::Database)?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn register_approval(
        &self,
        scope: &ControllerScope,
        request: &ApprovalRequest,
    ) -> Result<(), Error> {
        self.register_approval_once(scope, request)
            .await
            .map(|_| ())
    }
    pub(crate) async fn register_approval_once(
        &self,
        scope: &ControllerScope,
        request: &ApprovalRequest,
    ) -> Result<bool, Error> {
        request.validate().map_err(|_| Error::Invalid)?;
        if request.owner != scope.owner || request.session != scope.session {
            return Err(Error::Forbidden);
        }
        let mut tx = self.transaction().await?;
        lock_policy(&mut tx, scope, request.policy_revision, &request.kind).await?;
        // Session row share locks permit multiple different requests. Serialize the
        // per-session count and native-id uniqueness without locking another owner.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,5856751401703146821))")
            .bind(scope.session.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let existing: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT body FROM qgr.approval_requests WHERE id=$1 OR (session=$2 AND native_id=$3)",
        )
        .bind(request.id.to_string())
        .bind(scope.session.to_string())
        .bind(&request.native_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        if let Some(existing) = existing {
            if existing != value(request)? {
                return Err(Error::Conflict);
            }
            tx.commit().await?;
            return Ok(false);
        }
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM qgr.approval_requests r WHERE session=$1 AND state='pending' AND NOT EXISTS(SELECT FROM qgr.approval_decisions d WHERE d.approval=r.id)").bind(scope.session.to_string()).fetch_one(&mut *tx).await.map_err(db)?;
        if count >= 32 {
            return Err(Error::Overloaded);
        }
        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM qgr.approval_requests WHERE session=$1")
                .bind(scope.session.to_string())
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        if total >= 4096 {
            return Err(Error::Overloaded);
        }
        sqlx::query("INSERT INTO qgr.approval_requests(id,owner,machine,session,native_id,request_hash,policy_revision,body,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7::numeric,$8,$9)")
            .bind(request.id.to_string()).bind(scope.owner.to_string()).bind(scope.machine.to_string()).bind(scope.session.to_string()).bind(&request.native_id).bind(&request.request_hash).bind(request.policy_revision.0.to_string()).bind(value(request)?).bind(request.expires_at.as_ref().map(|at|time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(at.unix_millis())*1_000_000)).transpose().map_err(|_|Error::Invalid)?).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await?;
        Ok(true)
    }
    pub(crate) async fn reconcile_native_approvals(
        &self,
        scope: &ControllerScope,
        present: &[ApprovalId],
    ) -> Result<Vec<ApprovalId>, Error> {
        if present.len() > 32 {
            return Err(Error::Invalid);
        }
        let mut tx = self.transaction().await?;
        lock_owned_scope(&mut tx, scope, false).await?;
        let present = present.iter().map(ToString::to_string).collect::<Vec<_>>();
        let rows:Vec<String>=sqlx::query_scalar("UPDATE qgr.approval_requests r SET state='cancelled',reason='Native request is no longer pending' WHERE r.owner=$1 AND r.machine=$2 AND r.session=$3 AND r.state='pending' AND NOT(r.id=ANY($4::text[])) AND NOT EXISTS(SELECT FROM qgr.approval_decisions d WHERE d.approval=r.id) RETURNING r.id")
            .bind(scope.owner.to_string()).bind(scope.machine.to_string()).bind(scope.session.to_string()).bind(present).fetch_all(&mut *tx).await.map_err(db)?;
        tx.commit().await?;
        rows.into_iter()
            .map(|id| id.parse().map_err(|_| Error::Database))
            .collect()
    }
    /// No lease or attachment is required. Current host policy and original request
    /// identity must match; one conditional winner/audit/outbox commit survives retries.
    pub async fn choose_approval(
        &self,
        scope: &ControllerScope,
        client: ClientId,
        id: ApprovalId,
        request_hash: &str,
        decision: ApprovalDecision,
    ) -> Result<ApprovalChoice, Error> {
        self.choose_approval_guarded(scope, client, id, request_hash, decision, || Ok(()))
            .await
    }
    pub(crate) async fn choose_approval_guarded(
        &self,
        scope: &ControllerScope,
        client: ClientId,
        id: ApprovalId,
        request_hash: &str,
        decision: ApprovalDecision,
        guard: impl Fn() -> Result<(), Error>,
    ) -> Result<ApprovalChoice, Error> {
        guard()?;
        let mut tx = self.transaction().await?;
        lock_owned_scope(&mut tx, scope, false).await?;
        let row=sqlx::query("SELECT body,state,expires_at IS NOT NULL AND expires_at<=clock_timestamp() AS expired FROM qgr.approval_requests WHERE id=$1 AND owner=$2 AND machine=$3 AND session=$4 FOR UPDATE").bind(id.to_string()).bind(scope.owner.to_string()).bind(scope.machine.to_string()).bind(scope.session.to_string()).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let request: ApprovalRequest = read(row.get("body"))?;
        guard()?;
        if request.request_hash != request_hash {
            return Err(Error::Conflict);
        }
        if let Some(winner) = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT body FROM qgr.approval_decisions WHERE approval=$1",
        )
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?
        {
            let winner = read(winner)?;
            guard()?;
            tx.commit().await?;
            return Ok(ApprovalChoice {
                winner,
                chosen: false,
            });
        }
        lock_policy(&mut tx, scope, request.policy_revision, &request.kind).await?;
        if row.get::<String, _>("state") != "pending" || row.get::<bool, _>("expired") {
            return Err(Error::Conflict);
        }
        if !request
            .allowed_decisions
            .iter()
            .any(|allowed| value(allowed).ok() == value(&decision).ok())
        {
            return Err(Error::Forbidden);
        }
        let at: time::OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        let winner = ApprovalWinner {
            approval: id,
            id: DecisionId::new(),
            actor: scope.owner,
            client,
            request_hash: request_hash.into(),
            policy_revision: request.policy_revision,
            decision,
            committed_at: Timestamp::from_unix_millis(
                (at.unix_timestamp_nanos() / 1_000_000)
                    .try_into()
                    .map_err(|_| Error::Database)?,
            )
            .map_err(|_| Error::Database)?,
        };
        guard()?;
        sqlx::query("INSERT INTO qgr.approval_decisions(approval,id,owner,client,body) VALUES($1,$2,$3,$4,$5)").bind(id.to_string()).bind(winner.id.to_string()).bind(scope.owner.to_string()).bind(client.to_string()).bind(value(&winner)?).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("INSERT INTO qgr.approval_outbox(decision,approval) VALUES($1,$2)")
            .bind(winner.id.to_string())
            .bind(id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        Self::append_audit(
            &mut tx,
            &AuditEvent {
                actor: scope.owner,
                action: AuditAction::ApprovalDecided,
                machine: Some(scope.machine),
                session: Some(scope.session),
                metadata: AuditMetadata {
                    decision: Some(winner.id),
                    client: Some(client),
                    generation: Some(scope.host_generation),
                    ..Default::default()
                },
            },
        )
        .await?;
        // Row/policy/audit waits can cross the request's expiry after the first
        // SELECT. Recheck the primary clock at the final commit boundary.
        let pending: bool = sqlx::query_scalar("SELECT expires_at IS NULL OR expires_at>clock_timestamp() FROM qgr.approval_requests WHERE id=$1")
            .bind(id.to_string()).fetch_one(&mut *tx).await.map_err(db)?;
        if !pending {
            return Err(Error::Conflict);
        }
        guard()?;
        tx.commit().await?;
        Ok(ApprovalChoice {
            winner,
            chosen: true,
        })
    }
    pub async fn approval_status(
        &self,
        scope: &ControllerScope,
        id: ApprovalId,
    ) -> Result<ApprovalProjection, Error> {
        let mut tx = self.transaction().await?;
        lock_owned_scope(&mut tx, scope, false).await?;
        let row=sqlx::query("SELECT r.machine AS machine,r.body AS request,d.body AS winner,r.state,o.outcome,COALESCE(o.reason,r.reason) AS reason FROM qgr.approval_requests r LEFT JOIN qgr.approval_decisions d ON d.approval=r.id LEFT JOIN qgr.approval_outbox o ON o.approval=r.id WHERE r.id=$1 AND r.owner=$2 AND r.machine=$3 AND r.session=$4").bind(id.to_string()).bind(scope.owner.to_string()).bind(scope.machine.to_string()).bind(scope.session.to_string()).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let result = projection(row)?;
        tx.commit().await?;
        Ok(result)
    }
    /// Native cancellation is separate from any already committed delivery result.
    pub async fn cancel_approval(
        &self,
        scope: &ControllerScope,
        id: ApprovalId,
        request_hash: &str,
        reason: &str,
    ) -> Result<(), Error> {
        if reason.is_empty() || reason.len() > 1024 {
            return Err(Error::Invalid);
        }
        let mut tx = self.transaction().await?;
        lock_owned_scope(&mut tx, scope, false).await?;
        let affected=sqlx::query("UPDATE qgr.approval_requests SET state='cancelled',reason=COALESCE(reason,$6) WHERE id=$1 AND owner=$2 AND machine=$3 AND session=$4 AND request_hash=$5").bind(id.to_string()).bind(scope.owner.to_string()).bind(scope.machine.to_string()).bind(scope.session.to_string()).bind(request_hash).bind(reason).execute(&mut *tx).await.map_err(db)?.rows_affected();
        if affected != 1 {
            return Err(Error::NotFound);
        }
        tx.commit().await
    }
    /// Claims bound concurrent workers, never native execution. Lost claims expire;
    /// the native journal must deduplicate simultaneous or redelivered decisions.
    pub async fn claim_approvals(
        &self,
        scope: &ControllerScope,
        limit: u16,
    ) -> Result<Vec<ApprovalDelivery>, Error> {
        if !(1..=32).contains(&limit) {
            return Err(Error::Invalid);
        }
        let mut tx = self.transaction().await?;
        lock_owned_scope(&mut tx, scope, false).await?;
        let rows=sqlx::query("SELECT r.machine AS machine,r.body AS request,d.body AS winner,o.decision,o.attempts FROM qgr.approval_outbox o JOIN qgr.approval_requests r ON r.id=o.approval JOIN qgr.approval_decisions d ON d.id=o.decision WHERE r.owner=$1 AND r.machine=$2 AND r.session=$3 AND o.outcome IS NULL AND o.due_at<=clock_timestamp() AND (o.claim_until IS NULL OR o.claim_until<=clock_timestamp()) ORDER BY o.due_at,o.decision LIMIT $4 FOR UPDATE OF o SKIP LOCKED").bind(scope.owner.to_string()).bind(scope.machine.to_string()).bind(scope.session.to_string()).bind(i64::from(limit)).fetch_all(&mut *tx).await.map_err(db)?;
        let mut deliveries = Vec::new();
        for row in rows {
            let claim = MessageId::new();
            sqlx::query("UPDATE qgr.approval_outbox SET claim=$2,claim_until=clock_timestamp()+interval '9 seconds',attempts=LEAST(attempts+1,1000000) WHERE decision=$1").bind(row.get::<String,_>("decision")).bind(claim.to_string()).execute(&mut *tx).await.map_err(db)?;
            deliveries.push(ApprovalDelivery {
                request: read(row.get("request"))?,
                winner: read(row.get("winner"))?,
                claim,
                attempts: (row.get::<i32, _>("attempts") as u32)
                    .saturating_add(1)
                    .min(1000000),
            });
        }
        tx.commit().await?;
        Ok(deliveries)
    }
    pub async fn retry_approval(
        &self,
        scope: &ControllerScope,
        delivery: &ApprovalDelivery,
    ) -> Result<(), Error> {
        let mut tx = self.transaction().await?;
        lock_owned_scope(&mut tx, scope, false).await?;
        if delivery.request.owner != scope.owner || delivery.request.session != scope.session {
            return Err(Error::Forbidden);
        }
        let backoff = (1u32 << delivery.attempts.min(6)).min(60);
        let changed=sqlx::query("UPDATE qgr.approval_outbox o SET claim=NULL,claim_until=NULL,due_at=clock_timestamp()+$4*interval '1 second' FROM qgr.approval_requests r WHERE o.approval=r.id AND o.decision=$1 AND o.approval=$2 AND o.claim=$3 AND o.outcome IS NULL AND r.owner=$5 AND r.machine=$6 AND r.session=$7 AND r.request_hash=$8").bind(delivery.winner.id.to_string()).bind(delivery.request.id.to_string()).bind(delivery.claim.to_string()).bind(f64::from(backoff)).bind(scope.owner.to_string()).bind(scope.machine.to_string()).bind(scope.session.to_string()).bind(&delivery.request.request_hash).execute(&mut *tx).await.map_err(db)?.rows_affected();
        if changed != 1 {
            return Err(Error::Conflict);
        }
        tx.commit().await
    }
    /// A correlated native result retires delivery. OutcomeUnknown remains explicit;
    /// a successful gateway commit or native response write is never labeled Applied.
    pub async fn acknowledge_approval(
        &self,
        scope: &ControllerScope,
        delivery: &ApprovalDelivery,
        outcome: ApplicationOutcome,
        reason: Option<&str>,
    ) -> Result<(), Error> {
        if reason.is_some_and(|reason| reason.len() > 1024) {
            return Err(Error::Invalid);
        }
        if delivery.request.owner != scope.owner
            || delivery.request.session != scope.session
            || delivery.winner.approval != delivery.request.id
        {
            return Err(Error::Forbidden);
        }
        let mut tx = self.transaction().await?;
        lock_owned_scope(&mut tx, scope, false).await?;
        let row=sqlx::query("SELECT o.outcome,o.claim,o.claim_until>clock_timestamp() AS live,r.request_hash FROM qgr.approval_outbox o JOIN qgr.approval_requests r ON r.id=o.approval WHERE o.decision=$1 AND o.approval=$2 AND r.owner=$3 AND r.machine=$4 AND r.session=$5 FOR UPDATE OF o").bind(delivery.winner.id.to_string()).bind(delivery.request.id.to_string()).bind(scope.owner.to_string()).bind(scope.machine.to_string()).bind(scope.session.to_string()).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        if row.get::<String, _>("request_hash") != delivery.request.request_hash
            || delivery.winner.request_hash != delivery.request.request_hash
        {
            return Err(Error::Conflict);
        }
        let encoded = value(&outcome)?.as_str().ok_or(Error::Invalid)?.to_owned();
        if let Some(previous) = row.get::<Option<String>, _>("outcome") {
            if previous != encoded {
                return Err(Error::Conflict);
            }
            return tx.commit().await;
        }
        if row.get::<Option<String>, _>("claim").as_deref()
            != Some(delivery.claim.to_string().as_str())
            || !row.get::<Option<bool>, _>("live").unwrap_or(false)
        {
            return Err(Error::Conflict);
        }
        sqlx::query("UPDATE qgr.approval_outbox SET outcome=$2,reason=$3,claim=NULL,claim_until=NULL WHERE decision=$1").bind(delivery.winner.id.to_string()).bind(encoded).bind(reason).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await
    }
    /// Owner-wide reads require current directory/token evidence at the caller.
    /// They require neither an online host nor a controller/writable attachment.
    pub async fn list_approvals(
        &self,
        owner: UserId,
        after: Option<ApprovalId>,
        limit: u16,
    ) -> Result<Vec<ApprovalProjection>, Error> {
        if !(1..=200).contains(&limit) {
            return Err(Error::Invalid);
        }
        let mut tx = self.transaction().await?;
        Self::lock_access(&mut tx).await?;
        let allowed:bool=sqlx::query_scalar("SELECT NOT disabled AND directory_enabled IS DISTINCT FROM false FROM qgr.users WHERE id=$1 FOR SHARE").bind(owner.to_string()).fetch_optional(&mut *tx).await.map_err(db)?.unwrap_or(false);
        if !allowed {
            return Err(Error::Forbidden);
        }
        let rows=sqlx::query("SELECT r.machine AS machine,r.body AS request,d.body AS winner,CASE WHEN r.state='pending' AND d.id IS NULL AND r.expires_at<=clock_timestamp() THEN 'timed_out' ELSE r.state END AS state,o.outcome,COALESCE(o.reason,r.reason) AS reason FROM qgr.approval_requests r JOIN qgr.machines m ON m.id=r.machine AND m.owner=r.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id LEFT JOIN qgr.approval_decisions d ON d.approval=r.id LEFT JOIN qgr.approval_outbox o ON o.approval=r.id WHERE r.owner=$1 AND NOT m.disabled AND NOT k.revoked AND ($2::text IS NULL OR r.id>$2) ORDER BY r.id LIMIT $3 FOR SHARE OF r,m,k").bind(owner.to_string()).bind(after.map(|id|id.to_string())).bind(i64::from(limit)).fetch_all(&mut *tx).await.map_err(db)?;
        let mut bytes = 0usize;
        let mut result = Vec::new();
        for row in rows {
            let projection = projection(row)?;
            bytes += serde_json::to_vec(&projection)
                .map_err(|_| Error::Database)?
                .len();
            if bytes > 512 * 1024 {
                return Err(Error::Overloaded);
            }
            result.push(projection);
        }
        tx.commit().await?;
        Ok(result)
    }
}
fn approval_scope(row: PgRow) -> Result<ControllerScope, Error> {
    Ok(ControllerScope {
        owner: row
            .get::<String, _>("owner")
            .parse()
            .map_err(|_| Error::Database)?,
        machine: row
            .get::<String, _>("machine")
            .parse()
            .map_err(|_| Error::Database)?,
        session: row
            .get::<String, _>("session")
            .parse()
            .map_err(|_| Error::Database)?,
        host_generation: Counter(
            row.get::<String, _>("generation")
                .parse()
                .map_err(|_| Error::Database)?,
        ),
        host_connection: row
            .get::<String, _>("connection")
            .parse()
            .map_err(|_| Error::Database)?,
    })
}
#[cfg(test)]
pub(crate) mod tests;
