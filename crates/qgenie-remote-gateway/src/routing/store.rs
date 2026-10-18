use crate::{AuditAction, AuditEvent, AuditMetadata, Error, MachinePrincipal, Store};
use qgenie_remote_proto::{
    ClientId, Counter, MachineId, SessionId, UserId,
    wire::{LaunchKind, Session},
};
use sqlx::Row;
impl Store {
    async fn lock_host(
        transaction: &mut sqlx::PgConnection,
        actor: &MachinePrincipal,
        generation: Counter,
    ) -> Result<(), Error> {
        Self::lock_access(transaction).await?;
        let allowed:bool=sqlx::query_scalar("SELECT NOT(m.disabled OR u.disabled OR k.revoked OR a.disabled) AND r.generation=$4::numeric FROM qgr.machines m JOIN qgr.users u ON u.id=m.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id JOIN qgr.machine_routes r ON r.machine=m.id CROSS JOIN qgr.access_state a WHERE m.id=$1 AND m.owner=$2 AND m.active_key=$3 AND u.directory_object IS NOT NULL FOR SHARE OF m,u,k,r,a").bind(actor.machine.to_string()).bind(actor.owner.to_string()).bind(actor.key.to_string()).bind(generation.0.to_string()).fetch_optional(&mut *transaction).await.map_err(|_| Error::Database)?.unwrap_or(false);
        if allowed {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }

    pub(super) async fn route_matches(
        &self,
        machine: MachineId,
        node: &str,
        generation: Counter,
    ) -> Result<bool, Error> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM qgr.machine_routes r JOIN qgr.machines m ON m.id=r.machine JOIN qgr.users u ON u.id=m.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id CROSS JOIN qgr.access_state a WHERE r.machine=$1 AND r.node=$2 AND r.generation=$3::numeric AND NOT(m.disabled OR u.disabled OR k.revoked OR a.disabled))").bind(machine.to_string()).bind(node).bind(generation.0.to_string()).fetch_one(&self.pool).await.map_err(|_| Error::Database)
    }
    pub(super) async fn inventory(
        &self,
        actor: &MachinePrincipal,
        generation: Counter,
        agents: &std::collections::BTreeMap<String, qgenie_remote_proto::wire::AgentInventory>,
    ) -> Result<(), Error> {
        let body = serde_json::to_value(agents).map_err(|_| Error::Invalid)?;
        if agents.len() > 64
            || agents
                .iter()
                .any(|(key, value)| key.is_empty() || key.len() > 64 || value.version.len() > 128)
            || serde_json::to_vec(&body).map_err(|_| Error::Invalid)?.len() > 65536
        {
            return Err(Error::Invalid);
        }
        let mut transaction = self.transaction().await?;
        Self::lock_host(&mut transaction, actor, generation).await?;
        let result = sqlx::query(
            "UPDATE qgr.machine_routes SET agents=$3 WHERE machine=$1 AND generation=$2::numeric",
        )
        .bind(actor.machine.to_string())
        .bind(generation.0.to_string())
        .bind(body)
        .execute(&mut *transaction)
        .await
        .map_err(|_| Error::Database)?;
        if result.rows_affected() != 1 {
            return Err(Error::Forbidden);
        }
        transaction.commit().await?;
        Ok(())
    }
    pub(super) async fn supports_agent(
        &self,
        machine: MachineId,
        generation: Counter,
        agent: &str,
    ) -> Result<bool, Error> {
        sqlx::query_scalar("SELECT EXISTS(SELECT FROM qgr.machine_routes WHERE machine=$1 AND generation=$2::numeric AND activated AND native_access IS NOT NULL AND agents ? $3)")
            .bind(machine.to_string()).bind(generation.0.to_string()).bind(agent).fetch_one(&self.pool).await.map_err(|_|Error::Database)
    }
    pub(super) async fn project_exit(
        &self,
        actor: &MachinePrincipal,
        generation: Counter,
        session: SessionId,
    ) -> Result<(), Error> {
        let mut transaction = self.transaction().await?;
        Self::lock_host(&mut transaction, actor, generation).await?;
        let result=sqlx::query("UPDATE qgr.sessions SET state='exited',body=jsonb_set(body,'{state}','\"exited\"'),updated_at=CASE WHEN state IN ('exited','failed','lost','outcome_unknown') THEN updated_at ELSE clock_timestamp() END WHERE id=$1 AND machine=$2 AND owner=$3").bind(session.to_string()).bind(actor.machine.to_string()).bind(actor.owner.to_string()).execute(&mut *transaction).await.map_err(|_| Error::Database)?;
        if result.rows_affected() != 1 {
            return Err(Error::NotFound);
        }
        transaction.commit().await?;
        Ok(())
    }
    pub(super) async fn enrich_machines(
        &self,
        owner: UserId,
        machines: &mut [qgenie_remote_proto::wire::Machine],
    ) -> Result<(), Error> {
        let rows=sqlx::query("SELECT m.id,r.agents,(SELECT count(*) FROM qgr.sessions s WHERE s.machine=m.id AND s.owner=m.owner AND s.state IN ('accepted','launching','running','idle')) AS sessions FROM qgr.machines m LEFT JOIN qgr.machine_routes r ON r.machine=m.id WHERE m.owner=$1 AND NOT m.disabled LIMIT 1025").bind(owner.to_string()).fetch_all(&self.pool).await.map_err(|_| Error::Database)?;
        let mut values = std::collections::HashMap::new();
        for row in rows {
            values.insert(row.get::<String, _>("id"), row);
        }
        for machine in machines {
            if let Some(row) = values.get(&machine.id.to_string()) {
                if let Some(agents) = row.get::<Option<serde_json::Value>, _>("agents") {
                    machine.agents = serde_json::from_value(agents).map_err(|_| Error::Database)?;
                }
                machine.sessions =
                    u32::try_from(row.get::<i64, _>("sessions")).map_err(|_| Error::Overloaded)?;
            }
        }
        Ok(())
    }
    /// Reserve a durable host generation. Native activation is a separate task 017 ack.
    #[cfg(test)]
    pub(crate) async fn connect_host(
        &self,
        actor: &MachinePrincipal,
        node: &str,
        connection: ClientId,
    ) -> Result<Counter, Error> {
        self.connect_host_with_floor(actor, node, connection, Counter(0))
            .await
    }
    pub(crate) async fn connect_host_with_floor(
        &self,
        actor: &MachinePrincipal,
        node: &str,
        connection: ClientId,
        native_floor: Counter,
    ) -> Result<Counter, Error> {
        if native_floor.0 == u64::MAX {
            return Err(Error::Overloaded);
        }
        let mut transaction = self.transaction().await?;
        Self::lock_access(&mut transaction).await?;
        let allowed:bool=sqlx::query_scalar("SELECT NOT(m.disabled OR u.disabled OR a.disabled OR k.revoked) FROM qgr.machines m JOIN qgr.users u ON u.id=m.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id CROSS JOIN qgr.access_state a WHERE m.id=$1 AND m.owner=$2 AND m.active_key=$3 AND u.directory_object IS NOT NULL FOR SHARE OF m,u,k,a").bind(actor.machine.to_string()).bind(actor.owner.to_string()).bind(actor.key.to_string()).fetch_optional(&mut *transaction).await.map_err(|_| Error::Database)?.unwrap_or(false);
        if !allowed {
            return Err(Error::Forbidden);
        }
        let generation:Option<String>=sqlx::query_scalar("INSERT INTO qgr.machine_routes(machine,generation,node,connection) VALUES($1,($4::numeric+1),$2,$3) ON CONFLICT(machine) DO UPDATE SET generation=GREATEST(qgr.machine_routes.generation,$4::numeric)+1,node=EXCLUDED.node,connection=EXCLUDED.connection,activated=false,native_access=NULL,agents='{}',updated_at=clock_timestamp() WHERE qgr.machine_routes.generation<18446744073709551615 RETURNING generation::text").bind(actor.machine.to_string()).bind(node).bind(connection.to_string()).bind(native_floor.0.to_string()).fetch_optional(&mut *transaction).await.map_err(|_| Error::Database)?;
        let generation = Counter(
            generation
                .ok_or(Error::Overloaded)?
                .parse()
                .map_err(|_| Error::Database)?,
        );
        Self::append_audit(
            &mut transaction,
            &AuditEvent {
                actor: actor.owner,
                action: AuditAction::HostConnected,
                machine: Some(actor.machine),
                session: None,
                metadata: AuditMetadata {
                    generation: Some(generation),
                    ..AuditMetadata::default()
                },
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(generation)
    }
    pub(super) async fn host_current(
        &self,
        actor: &MachinePrincipal,
        generation: Counter,
    ) -> Result<bool, Error> {
        sqlx::query_scalar("SELECT NOT(m.disabled OR u.disabled OR a.disabled OR k.revoked) AND r.generation=$4::numeric FROM qgr.machines m JOIN qgr.users u ON u.id=m.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id CROSS JOIN qgr.access_state a JOIN qgr.machine_routes r ON r.machine=m.id WHERE m.id=$1 AND m.owner=$2 AND m.active_key=$3 AND u.directory_object IS NOT NULL").bind(actor.machine.to_string()).bind(actor.owner.to_string()).bind(actor.key.to_string()).bind(generation.0.to_string()).fetch_optional(&self.pool).await.map(|allowed| allowed.unwrap_or(false)).map_err(|_| Error::Database)
    }
    pub(super) async fn route_access(
        &self,
        owner: UserId,
        machine: MachineId,
        session: Option<SessionId>,
    ) -> Result<(), Error> {
        let allowed:bool=sqlx::query_scalar("SELECT NOT(m.disabled OR u.disabled OR a.disabled) FROM qgr.machines m JOIN qgr.users u ON u.id=m.owner CROSS JOIN qgr.access_state a WHERE m.id=$1 AND m.owner=$2").bind(machine.to_string()).bind(owner.to_string()).fetch_optional(&self.pool).await.map_err(|_| Error::Database)?.unwrap_or(false);
        if !allowed {
            return Err(Error::Forbidden);
        }
        if let Some(session) = session {
            let owned: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM qgr.sessions WHERE id=$1 AND machine=$2 AND owner=$3)",
            )
            .bind(session.to_string())
            .bind(machine.to_string())
            .bind(owner.to_string())
            .fetch_one(&self.pool)
            .await
            .map_err(|_| Error::Database)?;
            if !owned {
                return Err(Error::NotFound);
            }
        }
        Ok(())
    }
    pub(crate) async fn project(
        &self,
        actor: &MachinePrincipal,
        generation: Counter,
        session: &Session,
    ) -> Result<(), Error> {
        if session.owner != actor.owner || session.machine != actor.machine {
            return Err(Error::Forbidden);
        }
        if session.ended_at.as_ref().is_some_and(|ended| {
            ended.unix_millis() < session.created_at.unix_millis()
                || !matches!(
                    session.state,
                    qgenie_remote_proto::SessionState::Exited
                        | qgenie_remote_proto::SessionState::Failed
                        | qgenie_remote_proto::SessionState::Lost
                        | qgenie_remote_proto::SessionState::OutcomeUnknown
                )
        }) {
            return Err(Error::Invalid);
        }
        let mut metadata = session.clone();
        match &mut metadata.kind {
            LaunchKind::Agent { prompt, .. } => *prompt = None,
            LaunchKind::Pty { command } => command.truncate(1),
        }
        let body = serde_json::to_value(&metadata).map_err(|_| Error::Invalid)?;
        if serde_json::to_vec(&body).map_err(|_| Error::Invalid)?.len() > 262144 {
            return Err(Error::Invalid);
        }
        let state = serde_json::to_value(&session.state).map_err(|_| Error::Invalid)?;
        let mut transaction = self.transaction().await?;
        Self::lock_host(&mut transaction, actor, generation).await?;
        let inserted = sqlx::query("INSERT INTO qgr.sessions(id,machine,owner,body,state) VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO NOTHING")
            .bind(session.id.to_string()).bind(actor.machine.to_string()).bind(actor.owner.to_string()).bind(&body).bind(state.as_str().ok_or(Error::Invalid)?)
            .execute(&mut *transaction).await.map_err(|_| Error::Database)?.rows_affected() == 1;
        if inserted {
            Self::append_audit(
                &mut transaction,
                &AuditEvent {
                    actor: actor.owner,
                    action: AuditAction::SessionStarted,
                    machine: Some(actor.machine),
                    session: Some(session.id),
                    metadata: AuditMetadata {
                        generation: Some(generation),
                        ..Default::default()
                    },
                },
            )
            .await?;
        } else {
            let updated = sqlx::query("UPDATE qgr.sessions SET body=$4,state=$5,updated_at=CASE WHEN state IN ('exited','failed','lost','outcome_unknown') THEN updated_at ELSE clock_timestamp() END WHERE id=$1 AND machine=$2 AND owner=$3 AND (body->'created_at')=($4::jsonb->'created_at') AND (body->>'ended_at' IS NULL OR body->'ended_at'=$4::jsonb->'ended_at') AND NOT(state IN ('exited','failed','lost','outcome_unknown') AND $5 NOT IN ('exited','failed','lost','outcome_unknown'))")
                .bind(session.id.to_string()).bind(actor.machine.to_string()).bind(actor.owner.to_string()).bind(body).bind(state.as_str().ok_or(Error::Invalid)?)
                .execute(&mut *transaction).await.map_err(|_| Error::Database)?;
            if updated.rows_affected() == 0 {
                return Err(Error::Forbidden);
            }
        }
        transaction.commit().await?;
        Ok(())
    }
    pub(super) async fn session_projection(
        &self,
        owner: UserId,
        machine: MachineId,
    ) -> Result<Vec<Session>, Error> {
        let rows = sqlx::query(
            "SELECT CASE WHEN bytes<=524288 AND total<=1024 THEN body ELSE NULL END AS body FROM (SELECT id,body,sum(octet_length(body::text)) OVER () AS bytes,count(*) OVER () AS total FROM qgr.sessions WHERE owner=$1 AND machine=$2) bounded ORDER BY id LIMIT 1025",
        )
        .bind(owner.to_string())
        .bind(machine.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|_| Error::Database)?;
        if rows.len() > 1024
            || rows
                .iter()
                .any(|row| row.get::<Option<serde_json::Value>, _>("body").is_none())
        {
            return Err(Error::Overloaded);
        }
        rows.into_iter()
            .map(|row| serde_json::from_value(row.get("body")).map_err(|_| Error::Database))
            .collect()
    }
    pub(super) async fn seen(&self, machine: MachineId, generation: Counter) -> Result<(), Error> {
        let mut tx = self.transaction().await?;
        sqlx::query("UPDATE qgr.machines SET last_seen=clock_timestamp() WHERE id=$1 AND EXISTS(SELECT 1 FROM qgr.machine_routes WHERE machine=$1 AND generation=$2::numeric)").bind(machine.to_string()).bind(generation.0.to_string()).execute(&mut *tx).await.map_err(|_| Error::Database)?;
        tx.commit().await?;
        Ok(())
    }
}
