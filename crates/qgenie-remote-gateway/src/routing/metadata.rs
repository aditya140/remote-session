//! PostgreSQL is the sole authority for user-assigned session names and labels.
use super::{
    broker::user_channel,
    hub::Hub,
    message::{Body, Data, Packet},
    now_ms,
};
use crate::{AuditAction, AuditEvent, AuditMetadata, Error, Store};
use qgenie_remote_proto::{
    Counter, MachineId, MessageId, SessionId, UserId,
    wire::{Envelope, Message, Session, SessionMetadata, SessionMetadataChange},
};
use qgenie_remote_transport::ws::Priority;
use sqlx::Row;
use std::collections::BTreeMap;

impl Store {
    pub(super) async fn update_session_metadata(
        &self,
        owner: UserId,
        machine: MachineId,
        session: SessionId,
        change: &SessionMetadataChange,
        operation: MessageId,
    ) -> Result<Session, Error> {
        let mut transaction = self.transaction().await?;
        Self::lock_access(&mut transaction).await?;
        let allowed: bool = sqlx::query_scalar("SELECT NOT(m.disabled OR u.disabled OR k.revoked) FROM qgr.machines m JOIN qgr.users u ON u.id=m.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id WHERE m.id=$1 AND m.owner=$2 AND u.directory_object IS NOT NULL FOR SHARE OF m,u,k")
            .bind(machine.to_string()).bind(owner.to_string()).fetch_optional(&mut *transaction).await.map_err(|_|Error::Database)?.unwrap_or(false);
        if !allowed {
            return Err(Error::Forbidden);
        }
        let row = sqlx::query("SELECT body,metadata FROM qgr.sessions WHERE owner=$1 AND machine=$2 AND id=$3 FOR UPDATE")
            .bind(owner.to_string()).bind(machine.to_string()).bind(session.to_string()).fetch_optional(&mut *transaction).await.map_err(|_|Error::Database)?.ok_or(Error::NotFound)?;
        let mut metadata: SessionMetadata =
            serde_json::from_value(row.get("metadata")).map_err(|_| Error::Database)?;
        metadata.apply(change).map_err(|_| Error::Invalid)?;
        sqlx::query("UPDATE qgr.sessions SET metadata=$4 WHERE owner=$1 AND machine=$2 AND id=$3")
            .bind(owner.to_string())
            .bind(machine.to_string())
            .bind(session.to_string())
            .bind(serde_json::to_value(&metadata).map_err(|_| Error::Invalid)?)
            .execute(&mut *transaction)
            .await
            .map_err(|_| Error::Database)?;
        Self::append_audit(
            &mut transaction,
            &AuditEvent {
                actor: owner,
                action: AuditAction::SessionMetadataChanged,
                machine: Some(machine),
                session: Some(session),
                metadata: AuditMetadata {
                    operation: Some(operation),
                    ..Default::default()
                },
            },
        )
        .await?;
        let mut result: Session =
            serde_json::from_value(row.get("body")).map_err(|_| Error::Database)?;
        result.metadata = metadata;
        transaction.commit().await?;
        Ok(result)
    }

    pub(super) async fn resolve_session_name(
        &self,
        owner: UserId,
        machine: MachineId,
        name: &str,
    ) -> Result<Session, Error> {
        SessionMetadata::validate_name(name).map_err(|_| Error::Invalid)?;
        let rows = sqlx::query("SELECT body || jsonb_build_object('metadata',metadata) AS body FROM qgr.sessions WHERE owner=$1 AND machine=$2 AND metadata->>'name'=$3 ORDER BY id LIMIT 2")
            .bind(owner.to_string()).bind(machine.to_string()).bind(name).fetch_all(&self.pool).await.map_err(|_|Error::Database)?;
        match rows.len() {
            0 => Err(Error::NotFound),
            1 => serde_json::from_value(rows[0].get("body")).map_err(|_| Error::Database),
            _ => Err(Error::AmbiguousName),
        }
    }

    /// One bounded lookup per frame. Connector-supplied metadata is never trusted.
    pub(super) async fn decorate_session_metadata(
        &self,
        owner: UserId,
        machine: MachineId,
        message: &mut Message,
    ) -> Result<(), Error> {
        let sessions: &mut [Session] = match message {
            Message::Sessions { sessions }
            | Message::SessionUpdates { sessions }
            | Message::SessionPage { sessions, .. } => sessions,
            Message::SessionFound(session) | Message::Started(session) => {
                std::slice::from_mut(session)
            }
            _ => return Ok(()),
        };
        if sessions.is_empty() {
            return Ok(());
        }
        if sessions.len() > 1024 {
            return Err(Error::Overloaded);
        }
        let ids: Vec<String> = sessions
            .iter()
            .map(|session| session.id.to_string())
            .collect();
        let rows = sqlx::query("SELECT id,metadata FROM qgr.sessions WHERE owner=$1 AND machine=$2 AND id=ANY($3) LIMIT 1024")
            .bind(owner.to_string()).bind(machine.to_string()).bind(ids).fetch_all(&self.pool).await.map_err(|_|Error::Database)?;
        let mut metadata = BTreeMap::new();
        for row in rows {
            metadata.insert(
                row.get::<String, _>("id"),
                serde_json::from_value::<SessionMetadata>(row.get("metadata"))
                    .map_err(|_| Error::Database)?,
            );
        }
        for session in sessions {
            session.metadata = metadata.remove(&session.id.to_string()).unwrap_or_default();
        }
        Ok(())
    }
}

impl Hub {
    pub(super) async fn metadata_emit(
        &self,
        owner: UserId,
        machine: MachineId,
        session: SessionId,
    ) -> Result<(), Error> {
        self.gateway
            .store
            .route_access(owner, machine, Some(session))
            .await?;
        let session = self
            .gateway
            .store
            .session_get(owner, machine, session)
            .await?;
        let mut envelope = Envelope::new(Message::SessionUpdates {
            sessions: vec![session],
        });
        envelope.machine = Some(machine);
        self.fanout(owner, machine, None, Data::Json(Box::new(envelope)))
            .await
    }
    pub(super) async fn metadata_changed(
        &self,
        owner: UserId,
        machine: MachineId,
        session: SessionId,
    ) {
        // Publication can fail after the durable commit. A later read remains authoritative.
        let _ = self.metadata_emit(owner, machine, session).await;
        let packet = Packet {
            id: MessageId::new(),
            source: self.node.clone(),
            destination: user_channel(owner),
            at_ms: Counter(now_ms().unwrap_or(0)),
            body: Body::SessionMetadataChanged {
                owner,
                machine,
                session,
            },
        };
        let _ = self.broker.publish(&packet, Priority::Event).await;
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(in crate::routing) async fn contract(
        store: &Store,
        actor: &crate::MachinePrincipal,
        generation: Counter,
        session: &Session,
        admin: &sqlx::PgPool,
    ) {
        let owner = session.owner;
        let machine = session.machine;
        let change = SessionMetadataChange::Rename(Some("review-api".into()));
        let renamed = store
            .update_session_metadata(owner, machine, session.id, &change, MessageId::new())
            .await
            .unwrap();
        assert_eq!(renamed.id, session.id);
        assert_eq!(
            serde_json::to_value(&renamed.state).unwrap(),
            serde_json::to_value(&session.state).unwrap()
        );
        assert_eq!(renamed.created_at, session.created_at);
        assert_eq!(renamed.head, session.head);
        let a = SessionMetadataChange::Label {
            key: "team".into(),
            value: Some("api".into()),
        };
        let b = SessionMetadataChange::Label {
            key: "task".into(),
            value: Some("review".into()),
        };
        let (a, b) = tokio::join!(
            store.update_session_metadata(owner, machine, session.id, &a, MessageId::new()),
            store.update_session_metadata(owner, machine, session.id, &b, MessageId::new())
        );
        a.unwrap();
        b.unwrap();
        assert!(matches!(
            store
                .update_session_metadata(
                    UserId::new(),
                    machine,
                    session.id,
                    &change,
                    MessageId::new()
                )
                .await,
            Err(Error::Forbidden)
        ));
        assert!(matches!(
            store
                .update_session_metadata(
                    owner,
                    MachineId::new(),
                    session.id,
                    &change,
                    MessageId::new()
                )
                .await,
            Err(Error::Forbidden)
        ));
        assert!(matches!(
            store
                .update_session_metadata(
                    owner,
                    machine,
                    SessionId::new(),
                    &change,
                    MessageId::new()
                )
                .await,
            Err(Error::NotFound)
        ));
        // A later host update cannot erase or forge user-assigned metadata.
        let mut forged = session.clone();
        forged.metadata.name = Some("host-forgery".into());
        store.project(actor, generation, &forged).await.unwrap();
        let found = store
            .resolve_session_name(owner, machine, "review-api")
            .await
            .unwrap();
        assert_eq!(found.id, session.id);
        assert_eq!(found.metadata.labels.len(), 2);
        assert!(matches!(
            store
                .resolve_session_name(owner, machine, "host-forgery")
                .await,
            Err(Error::NotFound)
        ));
        assert!(matches!(
            store
                .resolve_session_name(UserId::new(), machine, "review-api")
                .await,
            Err(Error::NotFound)
        ));
        let duplicate = SessionId::new();
        let mut body = serde_json::to_value(session).unwrap();
        body["id"] = serde_json::json!(duplicate);
        sqlx::query("INSERT INTO qgr.sessions(id,machine,owner,body,state,metadata) SELECT $2,machine,owner,$3,state,metadata FROM qgr.sessions WHERE id=$1")
            .bind(session.id.to_string()).bind(duplicate.to_string()).bind(body).execute(admin).await.unwrap();
        assert!(matches!(
            store
                .resolve_session_name(owner, machine, "review-api")
                .await,
            Err(Error::AmbiguousName)
        ));
        sqlx::query("DELETE FROM qgr.sessions WHERE id=$1")
            .bind(duplicate.to_string())
            .execute(admin)
            .await
            .unwrap();
        let (page, _) = store
            .session_page(owner, machine, None, 64, false)
            .await
            .unwrap();
        assert_eq!(
            page.iter()
                .find(|item| item.id == session.id)
                .unwrap()
                .metadata,
            found.metadata
        );
        let mut reply = Message::SessionUpdates {
            sessions: vec![forged],
        };
        store
            .decorate_session_metadata(owner, machine, &mut reply)
            .await
            .unwrap();
        assert!(
            matches!(reply,Message::SessionUpdates{sessions} if sessions[0].metadata==found.metadata)
        );
        let audited: i64=sqlx::query_scalar("SELECT count(*) FROM qgr.audit_events WHERE session=$1 AND action='session_metadata_changed'").bind(session.id.to_string()).fetch_one(admin).await.unwrap();
        assert_eq!(
            audited, 3,
            "failed metadata updates must not commit audit rows"
        );
    }
}
