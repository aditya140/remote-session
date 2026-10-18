//! Primary controller proposal ledger. Callers authenticate/bind a live client,
//! verify directory/credential freshness and validate a scoped native snapshot.
//! These projections alone never authorize input; native ACK and access budgets
//! must also be bound before queueing. Public WSS composition lives in routing/control.
use crate::{AuditAction, AuditEvent, AuditMetadata, Error, Store};
use qgenie_remote_proto::{
    ClientId, Counter, MachineId, MessageId, SessionId, UserId, wire::Timestamp,
};
use sqlx::{Row, postgres::PgRow};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ControllerScope {
    pub owner: UserId,
    pub machine: MachineId,
    pub session: SessionId,
    pub host_generation: Counter,
    pub host_connection: ClientId,
}
#[derive(Debug, Clone)]
pub struct ControllerRequest {
    pub scope: ControllerScope,
    pub holder: ClientId,
    pub request: MessageId,
    pub native_floor: Counter,
    pub native_holder: Option<ClientId>,
    pub takeover: bool,
}
/// Metadata and a process-local primary deadline, not a native mutation permit.
#[derive(Debug, Clone)]
pub struct ControllerProposal {
    pub scope: ControllerScope,
    pub holder: ClientId,
    pub request: MessageId,
    pub activation: MessageId,
    pub generation: Counter,
    pub expires_at: Timestamp,
    pub native_acknowledged: bool,
    deadline: Duration,
    started: Duration,
}
impl ControllerProposal {
    pub fn remaining_budget(&self) -> Result<u32, Error> {
        let left = self
            .deadline
            .checked_sub(elapsed()?)
            .ok_or(Error::StaleGeneration)?;
        let ms: u32 = left
            .as_millis()
            .try_into()
            .map_err(|_| Error::Unavailable)?;
        if !(1..=15_000).contains(&ms) {
            return Err(Error::StaleGeneration);
        }
        Ok(ms)
    }
}
fn elapsed() -> Result<Duration, Error> {
    qgenie_remote_transport::elapsed::now().map_err(|_| Error::Unavailable)
}
fn db(_: sqlx::Error) -> Error {
    Error::Database
}
impl Store {
    pub(crate) async fn audit_session_action(
        &self,
        scope: &ControllerScope,
        client: ClientId,
        request: MessageId,
        action: AuditAction,
    ) -> Result<(), Error> {
        let mut tx = self.transaction().await?;
        lock_owned_scope(&mut tx, scope, false).await?;
        Self::append_audit(
            &mut tx,
            &AuditEvent {
                actor: scope.owner,
                action,
                machine: Some(scope.machine),
                session: Some(scope.session),
                metadata: AuditMetadata {
                    operation: Some(request),
                    client: Some(client),
                    generation: Some(scope.host_generation),
                    ..Default::default()
                },
            },
        )
        .await?;
        tx.commit().await
    }
    /// Load the current holder's exact CAS stage. `None` reconciles an already
    /// released generation only for its immutable original holder.
    pub(crate) async fn current_controller(
        &self,
        scope: &ControllerScope,
        holder: ClientId,
        generation: Counter,
        running: bool,
    ) -> Result<Option<ControllerProposal>, Error> {
        let started = elapsed()?;
        let mut tx = self.transaction().await?;
        lock_owned_scope(&mut tx, scope, running).await?;
        let row = lease(&mut tx, scope.session).await?;
        if number(&row, "gen")? != generation.0
            || number(&row, "host")? != scope.host_generation.0
            || row.get::<String, _>("host_connection") != scope.host_connection.to_string()
        {
            return Err(Error::StaleGeneration);
        }
        if row.get::<Option<String>, _>("holder").is_none() {
            let original:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM qgr.control_requests WHERE owner=$1 AND machine=$2 AND session=$3 AND holder=$4 AND generation=$5::numeric)")
                .bind(scope.owner.to_string()).bind(scope.machine.to_string()).bind(scope.session.to_string()).bind(holder.to_string()).bind(generation.0.to_string()).fetch_one(&mut *tx).await.map_err(db)?;
            if !original {
                return Err(Error::StaleGeneration);
            }
            tx.commit().await?;
            return Ok(None);
        }
        let proposal = projection(&row, started)?;
        if proposal.holder != holder {
            return Err(Error::StaleGeneration);
        }
        tx.commit().await?;
        Ok(Some(proposal))
    }
    /// Same actor/holder/request reconciles the original proposal even after a
    /// takeover; it cannot reserve a new generation or extend its old deadline.
    pub async fn propose_controller(
        &self,
        request: ControllerRequest,
    ) -> Result<ControllerProposal, Error> {
        let started = elapsed()?;
        let mut tx = self.transaction().await?;
        lock_scope(&mut tx, &request.scope).await?;
        sqlx::query("INSERT INTO qgr.control_leases(session,owner,machine,host_generation,host_connection) VALUES($1,$2,$3,$4::numeric,$5) ON CONFLICT(session) DO NOTHING")
            .bind(request.scope.session.to_string()).bind(request.scope.owner.to_string()).bind(request.scope.machine.to_string()).bind(request.scope.host_generation.0.to_string()).bind(request.scope.host_connection.to_string()).execute(&mut *tx).await.map_err(db)?;
        let current = lease(&mut tx, request.scope.session).await?;
        if let Some(original)=sqlx::query("SELECT *,generation::text AS gen,host_generation::text AS host,LEAST(15000,GREATEST(0,FLOOR(EXTRACT(epoch FROM (expires_at-clock_timestamp()))*1000)))::bigint AS remaining FROM qgr.control_requests WHERE owner=$1 AND holder=$2 AND request=$3")
            .bind(request.scope.owner.to_string()).bind(request.holder.to_string()).bind(request.request.to_string()).fetch_optional(&mut *tx).await.map_err(db)? {
            if original.get::<String,_>("session")!=request.scope.session.to_string() || original.get::<String,_>("machine")!=request.scope.machine.to_string() || original.get::<bool,_>("takeover")!=request.takeover {return Err(Error::Conflict);}
            let mut proposal=original_projection(&original,started)?;
            if current.get::<Option<String>,_>("request").as_deref()==Some(request.request.to_string().as_str()) && current.get::<Option<String>,_>("holder").as_deref()==Some(request.holder.to_string().as_str()) {
                proposal=projection(&current,started)?;
            } else {proposal.deadline=started;proposal.native_acknowledged=false;}
            if proposal.scope.host_generation != request.scope.host_generation || proposal.scope.host_connection != request.scope.host_connection {
                proposal.deadline=started;proposal.native_acknowledged=false;
            }
            tx.commit().await?;
            return Ok(proposal);
        }
        let floor = number(&current, "gen")?;
        let live = current.get::<i64, _>("remaining") > 0;
        if live
            && current
                .get::<Option<String>, _>("holder")
                .is_some_and(|holder| holder != request.holder.to_string())
            && !request.takeover
        {
            return Err(Error::ControlHeld);
        }
        if request
            .native_holder
            .is_some_and(|holder| holder != request.holder)
            && !request.takeover
        {
            return Err(Error::ControlHeld);
        }
        let generation = Counter(
            floor
                .max(request.native_floor.0)
                .checked_add(1)
                .ok_or(Error::Overloaded)?,
        );
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM qgr.control_requests WHERE session=$1")
                .bind(request.scope.session.to_string())
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        if count >= 4096 {
            return Err(Error::Overloaded);
        }
        let row=sqlx::query("INSERT INTO qgr.control_requests(owner,holder,request,session,machine,host_generation,host_connection,generation,takeover,expires_at) VALUES($1,$2,$3,$4,$5,$6::numeric,$7,$8::numeric,$9,clock_timestamp()+interval '15 seconds') RETURNING expires_at")
            .bind(request.scope.owner.to_string()).bind(request.holder.to_string()).bind(request.request.to_string()).bind(request.scope.session.to_string()).bind(request.scope.machine.to_string()).bind(request.scope.host_generation.0.to_string()).bind(request.scope.host_connection.to_string()).bind(generation.0.to_string()).bind(request.takeover).fetch_one(&mut *tx).await.map_err(db)?;
        let expires: time::OffsetDateTime = row.get("expires_at");
        sqlx::query("UPDATE qgr.control_leases SET holder=$2,request=$3,activation=$3,generation=$4::numeric,expires_at=$5,host_generation=$6::numeric,host_connection=$7,activated=false,audited=false WHERE session=$1")
            .bind(request.scope.session.to_string()).bind(request.holder.to_string()).bind(request.request.to_string()).bind(generation.0.to_string()).bind(expires).bind(request.scope.host_generation.0.to_string()).bind(request.scope.host_connection.to_string()).execute(&mut *tx).await.map_err(db)?;
        let proposal = projection(&lease(&mut tx, request.scope.session).await?, started)?;
        tx.commit().await?;
        Ok(proposal)
    }
    /// Call only after an exact, fresh native controller ACK. Its measured
    /// remaining lifetime is anchored to this proposal's earlier query start.
    pub async fn confirm_controller(
        &self,
        proposal: &ControllerProposal,
        native_remaining_ms: u32,
    ) -> Result<ControllerProposal, Error> {
        if !(1..=15_000).contains(&native_remaining_ms) {
            return Err(Error::Invalid);
        }
        proposal.remaining_budget()?;
        let started = elapsed()?;
        let mut tx = self.transaction().await?;
        lock_scope(&mut tx, &proposal.scope).await?;
        let row = lease(&mut tx, proposal.scope.session).await?;
        matches_proposal(&row, proposal)?;
        if row.get::<i64, _>("remaining") == 0 {
            return Err(Error::StaleGeneration);
        }
        let native_deadline =
            proposal.started + Duration::from_millis(u64::from(native_remaining_ms));
        if native_deadline <= elapsed()? {
            return Err(Error::StaleGeneration);
        }
        if !row.get::<bool, _>("audited") {
            Self::append_audit(
                &mut tx,
                &AuditEvent {
                    actor: proposal.scope.owner,
                    action: AuditAction::ControllerGranted,
                    machine: Some(proposal.scope.machine),
                    session: Some(proposal.scope.session),
                    metadata: AuditMetadata {
                        generation: Some(proposal.generation),
                        control_request: Some(proposal.request),
                        client: Some(proposal.holder),
                        ..Default::default()
                    },
                },
            )
            .await?;
        }
        // Persist the earlier native boundary too. A duplicate activation using a
        // newer access nonce must not regain lifetime discarded by the first ACK.
        // BEGIN's server timestamp predates this elapsed-budget computation, so
        // this mapping subtracts database waits instead of adding them to the lease.
        let native_left = native_deadline
            .checked_sub(elapsed()?)
            .ok_or(Error::StaleGeneration)?;
        let native_left: i64 = native_left
            .as_millis()
            .try_into()
            .map_err(|_| Error::Unavailable)?;
        if native_left == 0 {
            return Err(Error::StaleGeneration);
        }
        sqlx::query("UPDATE qgr.control_leases SET activated=true,audited=true,expires_at=LEAST(expires_at,transaction_timestamp()+$2*interval '1 millisecond') WHERE session=$1")
            .bind(proposal.scope.session.to_string())
            .bind(native_left)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let mut confirmed = projection(&lease(&mut tx, proposal.scope.session).await?, started)?;
        confirmed.deadline = confirmed
            .deadline
            .min(proposal.deadline)
            .min(native_deadline);
        tx.commit().await?;
        confirmed.remaining_budget()?;
        Ok(confirmed)
    }
    /// Renewals are a CAS chain of activation IDs. Retrying the current ID is
    /// idempotent; an older stage cannot extend a later stage's deadline.
    pub async fn renew_controller(
        &self,
        previous: &ControllerProposal,
        request: MessageId,
        native_floor: Counter,
        native_holder: Option<ClientId>,
    ) -> Result<ControllerProposal, Error> {
        if native_floor != previous.generation || native_holder != Some(previous.holder) {
            return Err(Error::StaleGeneration);
        }
        let started = elapsed()?;
        let mut tx = self.transaction().await?;
        lock_scope(&mut tx, &previous.scope).await?;
        let row = lease(&mut tx, previous.scope.session).await?;
        if row.get::<Option<String>, _>("activation").as_deref()
            == Some(request.to_string().as_str())
        {
            let mut same = previous.clone();
            same.activation = request;
            matches_proposal(&row, &same)?;
            let result = projection(&row, started)?;
            tx.commit().await?;
            return Ok(result);
        }
        matches_proposal(&row, previous)?;
        if !row.get::<bool, _>("activated") || row.get::<i64, _>("remaining") == 0 {
            return Err(Error::StaleGeneration);
        }
        sqlx::query("UPDATE qgr.control_leases SET activation=$2,activated=false,expires_at=clock_timestamp()+interval '15 seconds' WHERE session=$1").bind(previous.scope.session.to_string()).bind(request.to_string()).execute(&mut *tx).await.map_err(db)?;
        let result = projection(&lease(&mut tx, previous.scope.session).await?, started)?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn release_controller(&self, proposal: &ControllerProposal) -> Result<(), Error> {
        let mut tx = self.transaction().await?;
        lock_owned_scope(&mut tx, &proposal.scope, false).await?;
        let row = lease(&mut tx, proposal.scope.session).await?;
        if number(&row, "gen")? == proposal.generation.0
            && row.get::<Option<String>, _>("holder").is_none()
        {
            return Ok(());
        }
        matches_proposal(&row, proposal)?;
        sqlx::query("UPDATE qgr.control_leases SET holder=NULL,activated=false,expires_at=clock_timestamp() WHERE session=$1").bind(proposal.scope.session.to_string()).execute(&mut *tx).await.map_err(db)?;
        Self::append_audit(
            &mut tx,
            &AuditEvent {
                actor: proposal.scope.owner,
                action: AuditAction::ControllerReleased,
                machine: Some(proposal.scope.machine),
                session: Some(proposal.scope.session),
                metadata: AuditMetadata {
                    generation: Some(proposal.generation),
                    client: Some(proposal.holder),
                    ..Default::default()
                },
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
async fn lock_scope(tx: &mut sqlx::PgConnection, scope: &ControllerScope) -> Result<(), Error> {
    lock_owned_scope(tx, scope, true).await
}
pub(crate) async fn lock_owned_scope(
    tx: &mut sqlx::PgConnection,
    scope: &ControllerScope,
    running: bool,
) -> Result<(), Error> {
    Store::lock_access(tx).await?;
    let allowed:bool=sqlx::query_scalar("SELECT NOT(m.disabled OR u.disabled OR k.revoked) AND u.directory_enabled IS DISTINCT FROM false AND r.generation=$4::numeric AND r.connection=$5 AND r.activated AND r.native_access IS NOT NULL AND (NOT $6::boolean OR s.state IN ('running','idle')) FROM qgr.sessions s JOIN qgr.machines m ON m.id=s.machine AND m.owner=s.owner JOIN qgr.users u ON u.id=m.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id JOIN qgr.machine_routes r ON r.machine=m.id WHERE s.id=$1 AND s.machine=$2 AND s.owner=$3 FOR SHARE OF s,m,u,k,r")
        .bind(scope.session.to_string()).bind(scope.machine.to_string()).bind(scope.owner.to_string()).bind(scope.host_generation.0.to_string()).bind(scope.host_connection.to_string()).bind(running).fetch_optional(&mut *tx).await.map_err(db)?.unwrap_or(false);
    if allowed {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}
async fn lease(tx: &mut sqlx::PgConnection, session: SessionId) -> Result<PgRow, Error> {
    sqlx::query("SELECT *,generation::text AS gen,host_generation::text AS host,LEAST(15000,GREATEST(0,FLOOR(EXTRACT(epoch FROM (expires_at-clock_timestamp()))*1000)))::bigint AS remaining FROM qgr.control_leases WHERE session=$1 FOR UPDATE")
        .bind(session.to_string()).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::StaleGeneration)
}
fn number(row: &PgRow, key: &str) -> Result<u64, Error> {
    row.get::<String, _>(key)
        .parse()
        .map_err(|_| Error::Database)
}
fn matches_proposal(row: &PgRow, proposal: &ControllerProposal) -> Result<(), Error> {
    if number(row, "gen")? != proposal.generation.0
        || number(row, "host")? != proposal.scope.host_generation.0
        || row.get::<String, _>("host_connection") != proposal.scope.host_connection.to_string()
        || row.get::<Option<String>, _>("holder").as_deref()
            != Some(proposal.holder.to_string().as_str())
        || row.get::<Option<String>, _>("request").as_deref()
            != Some(proposal.request.to_string().as_str())
        || row.get::<Option<String>, _>("activation").as_deref()
            != Some(proposal.activation.to_string().as_str())
    {
        return Err(Error::StaleGeneration);
    }
    Ok(())
}
fn original_projection(row: &PgRow, started: Duration) -> Result<ControllerProposal, Error> {
    let expires: time::OffsetDateTime = row.get("expires_at");
    let request: MessageId = row
        .get::<String, _>("request")
        .parse()
        .map_err(|_| Error::Database)?;
    Ok(ControllerProposal {
        scope: ControllerScope {
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
            host_generation: Counter(number(row, "host")?),
            host_connection: row
                .get::<String, _>("host_connection")
                .parse()
                .map_err(|_| Error::Database)?,
        },
        holder: row
            .get::<String, _>("holder")
            .parse()
            .map_err(|_| Error::Database)?,
        request,
        activation: request,
        generation: Counter(number(row, "gen")?),
        expires_at: Timestamp::from_unix_millis(
            (expires.unix_timestamp_nanos() / 1_000_000)
                .try_into()
                .map_err(|_| Error::Database)?,
        )
        .map_err(|_| Error::Database)?,
        native_acknowledged: false,
        deadline: started + Duration::from_millis(row.get::<i64, _>("remaining") as u64),
        started,
    })
}
fn projection(row: &PgRow, started: Duration) -> Result<ControllerProposal, Error> {
    let mut proposal = original_projection(row, started)?;
    proposal.activation = row
        .get::<Option<String>, _>("activation")
        .ok_or(Error::StaleGeneration)?
        .parse()
        .map_err(|_| Error::Database)?;
    proposal.native_acknowledged = row.get("activated");
    Ok(proposal)
}

#[cfg(test)]
pub(crate) mod tests;
