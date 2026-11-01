use anyhow::{Context, Result};
use qgenie_remote_proto::ipc::RemoteScope;
use qgenie_remote_proto::*;
use qgenie_remote_transport::ipc::private_file;
use rusqlite::{Connection, OptionalExtension, params};
use std::{path::Path, time::Duration};

pub(crate) struct Store {
    connection: Connection,
    max_events: usize,
    max_bytes: usize,
}
pub(crate) struct StoredSession {
    pub info: SessionInfo,
    pub spec: StartSpec,
    pub fingerprint: String,
    pub generation: u64,
    pub scope: Option<RemoteScope>,
}

impl Store {
    pub fn open(path: &Path, max_events: usize, max_bytes: usize) -> Result<Self> {
        private_file(path)?;
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
        )?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        anyhow::ensure!(version <= 8, "unsupported session schema");
        if version == 0 {
            connection.execute_batch(include_str!("../migrations/0001_sessions.sql"))?;
        }
        if version < 2 {
            connection.execute_batch(include_str!("../migrations/0002_checkpoints.sql"))?;
        }
        if version < 3 {
            connection.execute_batch(include_str!("../migrations/0003_ownership.sql"))?;
        }
        if version < 4 {
            connection.execute_batch(include_str!("../migrations/0004_native_authority.sql"))?;
        }
        if version < 5 {
            connection.execute_batch(include_str!("../migrations/0005_native_end.sql"))?;
        }
        if version < 6 {
            connection.execute_batch(include_str!("../migrations/0006_managed_identity.sql"))?;
        }
        if version < 7 {
            connection.execute_batch(include_str!("../migrations/0007_session_pages.sql"))?;
        }
        if version < 8 {
            connection.execute_batch(include_str!("../migrations/0008_prompt_receipts.sql"))?;
        }
        // SQLite WAL/SHM inherit the private DB permissions in the private state directory.
        Ok(Self {
            connection,
            max_events,
            max_bytes,
        })
    }
    pub fn machine(&self) -> Result<MachineId> {
        let existing: Option<String> = self
            .connection
            .query_row("SELECT id FROM machine WHERE singleton=1", [], |r| r.get(0))
            .optional()?;
        if let Some(id) = existing {
            return id.parse().map_err(anyhow::Error::msg);
        }
        let id = MachineId::new();
        self.connection
            .execute("INSERT INTO machine VALUES(1,?1)", [id.to_string()])?;
        Ok(id)
    }
    pub fn recover(&self) -> Result<()> {
        self.connection.execute("UPDATE sessions SET state=CASE state WHEN '\"running\"' THEN '\"lost\"' WHEN '\"idle\"' THEN '\"lost\"' ELSE '\"outcome_unknown\"' END WHERE state IN ('\"running\"','\"idle\"','\"accepted\"','\"launching\"')", [])?;
        Ok(())
    }
    #[cfg(test)]
    pub fn load(&self, machine: MachineId) -> Result<Vec<StoredSession>> {
        self.recover()?;
        self.read_where(machine, "1 ORDER BY created_ms", [])
    }
    pub fn get(&self, machine: MachineId, id: SessionId) -> Result<Option<StoredSession>> {
        Ok(self.read_where(machine, "id=?1", [id.to_string()])?.pop())
    }
    pub fn lookup(
        &self,
        scope: Option<&RemoteScope>,
        key: IdempotencyKey,
    ) -> Result<Option<SessionId>> {
        let key = match scope {
            None => key.to_string(),
            Some(scope) => format!(
                "remote:v1:{}:{}",
                fingerprint(&serde_json::to_value(scope)?)?,
                key
            ),
        };
        self.connection
            .query_row(
                "SELECT id FROM sessions WHERE idempotency_key=?1",
                [key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|id| id.parse().map_err(anyhow::Error::msg))
            .transpose()
    }
    pub fn count(&self) -> Result<usize> {
        Ok(self
            .connection
            .query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))?)
    }
    pub fn page_ids(
        &self,
        scope: Option<&RemoteScope>,
        after: Option<SessionId>,
        limit: u16,
        active_only: bool,
    ) -> Result<Vec<SessionId>> {
        let scoped = scope.is_some();
        let scope = scope.map(serde_json::to_string).transpose()?;
        // Only fixed SQL fragments vary; scope and cursor remain bound parameters.
        let scope_filter = if scoped {
            "remote_scope=?1"
        } else {
            "?1 IS NULL"
        };
        let live_filter = if active_only {
            "AND state IN ('\"accepted\"','\"launching\"','\"running\"','\"idle\"')"
        } else {
            ""
        };
        let mut query = self.connection.prepare(&format!("SELECT id FROM sessions WHERE {scope_filter} AND id>?2 {live_filter} ORDER BY id LIMIT ?3"))?;
        query
            .query_map(
                params![
                    scope,
                    after.map(|id| id.to_string()).unwrap_or_default(),
                    u32::from(limit) + 1
                ],
                |row| row.get::<_, String>(0),
            )?
            .map(|id| id?.parse().map_err(anyhow::Error::msg))
            .collect()
    }
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    pub fn history_homes(
        &self,
        scope: Option<&RemoteScope>,
        agent: &str,
        thread: &str,
    ) -> Result<Vec<SessionId>> {
        let scope = scope.map(serde_json::to_string).transpose()?;
        let mut query=self.connection.prepare("SELECT DISTINCT coalesce(json_extract(agent,'$.native_home'),id) FROM sessions WHERE agent IS NOT NULL AND remote_scope IS ?1 AND state='\"exited\"' AND ended_ms IS NOT NULL AND json_extract(agent,'$.agent')=?2 AND json_extract(agent,'$.native_thread')=?3 LIMIT 2")?;
        query
            .query_map(params![scope, agent, thread], |row| row.get::<_, String>(0))?
            .map(|id| id?.parse().map_err(anyhow::Error::msg))
            .collect()
    }
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    pub fn history_uncertain(&self, home: SessionId) -> Result<bool> {
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE agent IS NOT NULL AND coalesce(json_extract(agent,'$.native_home'),id)=?1 AND state NOT IN ('\"exited\"','\"failed\"'))", [home.to_string()], |row|row.get(0))?)
    }
    fn read_where(
        &self,
        machine: MachineId,
        condition: &str,
        parameters: impl rusqlite::Params,
    ) -> Result<Vec<StoredSession>> {
        let mut query=self.connection.prepare(&format!("SELECT id,spec,fingerprint,state,created_ms,head,generation,exit_code,remote_scope,ended_ms,agent,recording_warning FROM sessions WHERE {condition}"))?;
        let rows = query.query_map(parameters, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Vec<u8>>(5)?,
                row.get::<_, Vec<u8>>(6)?,
                row.get::<_, Option<u32>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
            ))
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            let (
                id,
                spec,
                digest,
                state,
                created,
                head,
                generation,
                exit_code,
                scope,
                ended,
                agent,
                recording_warning,
            ) = row?;
            let scope: Option<RemoteScope> =
                scope.map(|text| serde_json::from_str(&text)).transpose()?;
            let agent: Option<ManagedSessionInfo> =
                agent.map(|body| serde_json::from_str(&body)).transpose()?;
            if let Some(agent) = &agent {
                agent.validate()?;
            }
            if let Some(scope) = &scope {
                scope.validate()?;
                anyhow::ensure!(
                    self.bound(scope)?,
                    "persisted session has no immutable owner binding"
                );
            }
            let spec: StartSpec = serde_json::from_str(&spec)?;
            let state: SessionState = serde_json::from_str(&state)?;
            let id: SessionId = id.parse().map_err(anyhow::Error::msg)?;
            sessions.push(StoredSession {
                info: SessionInfo {
                    id,
                    machine:scope.as_ref().map_or(machine,|scope|scope.machine),
                    command: spec.command.clone(),
                    cwd: spec.cwd.clone(),
                    state: state.clone(),
                    created_at_ms: Counter(created.parse()?),
                    ended_at_ms: ended.map(|v|v.parse().map(Counter)).transpose()?,
                    head: Counter(counter(&head)?),
                    controller: None,
                    exit_code,
                    recording_warning: recording_warning.or_else(|| if scope.is_some() {
                        matches!(state,SessionState::Lost|SessionState::OutcomeUnknown).then(||"Native owner restarted; recording spool reconciles the uncertain output tail".into())
                    } else {
                        Some("Local preview replay only; this session is not enrolled for remote recording".into())
                    }),
                    agent,
                },
                spec,
                fingerprint: digest,
                generation: counter(&generation)?,
                scope,
            });
        }
        Ok(sessions)
    }
    pub fn insert(
        &self,
        info: &SessionInfo,
        spec: &StartSpec,
        digest: &str,
        scope: Option<&RemoteScope>,
    ) -> Result<()> {
        if let Some(agent) = &info.agent {
            agent.validate()?;
        }
        let key = match scope {
            None => spec.key.to_string(),
            Some(scope) => format!(
                "remote:v1:{}:{}",
                fingerprint(&serde_json::to_value(scope)?)?,
                spec.key
            ),
        };
        self.connection.execute("INSERT INTO sessions(id,idempotency_key,fingerprint,spec,state,created_ms,head,generation,remote_scope,agent) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![info.id.to_string(),key,digest,serde_json::to_string(spec)?,serde_json::to_string(&info.state)?,info.created_at_ms.0.to_string(),0u64.to_be_bytes().as_slice(),0u64.to_be_bytes().as_slice(),scope.map(serde_json::to_string).transpose()?,info.agent.as_ref().map(serde_json::to_string).transpose()?])?;
        Ok(())
    }
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    pub fn managed_identity(&self, id: SessionId, agent: &ManagedSessionInfo) -> Result<()> {
        agent.validate()?;
        let changed = self.connection.execute(
            "UPDATE sessions SET agent=?1 WHERE id=?2 AND agent IS NOT NULL AND json_extract(agent,'$.native_thread') IS NULL AND json_extract(agent,'$.native_instance')=?3",
            params![serde_json::to_string(agent)?, id.to_string(), agent.native_instance.to_string()],
        )?;
        anyhow::ensure!(
            changed == 1,
            "managed identity already bound or unavailable"
        );
        Ok(())
    }
    pub fn bound(&self, scope: &RemoteScope) -> Result<bool> {
        scope.validate()?;
        Ok(self.binding_owner(scope)? == Some(scope.owner))
    }
    pub fn binding_owner(
        &self,
        scope: &RemoteScope,
    ) -> Result<Option<qgenie_remote_proto::UserId>> {
        let owner: Option<String> = self
            .connection
            .query_row(
                "SELECT owner FROM remote_bindings WHERE origin=?1 AND machine=?2",
                params![scope.origin, scope.machine.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        owner
            .map(|value| value.parse().map_err(anyhow::Error::msg))
            .transpose()
    }
    pub fn bind(&self, scope: &RemoteScope) -> Result<()> {
        scope.validate()?;
        let count: i64 =
            self.connection
                .query_row("SELECT count(*) FROM remote_bindings", [], |row| row.get(0))?;
        if count >= 64 {
            anyhow::ensure!(self.bound(scope)?, "native owner binding capacity reached");
            return Ok(());
        }
        self.connection.execute(
            "INSERT INTO remote_bindings(origin,machine,owner) VALUES(?1,?2,?3) ON CONFLICT(origin,machine) DO NOTHING",
            params![
                scope.origin,
                scope.machine.to_string(),
                scope.owner.to_string()
            ],
        )?;
        anyhow::ensure!(
            self.bound(scope)?,
            "native machine binding cannot change owner"
        );
        Ok(())
    }
    pub fn state(&self, id: SessionId, state: &SessionState, code: Option<u32>) -> Result<()> {
        self.connection.execute(
            "UPDATE sessions SET state=?1,exit_code=?2 WHERE id=?3",
            params![serde_json::to_string(state)?, code, id.to_string()],
        )?;
        Ok(())
    }
    pub fn finished(&self, info: &SessionInfo) -> Result<()> {
        self.connection.execute(
            "UPDATE sessions SET state=?1,exit_code=?2,ended_ms=coalesce(ended_ms,?3),recording_warning=coalesce(?4,recording_warning) WHERE id=?5",
            params![
                serde_json::to_string(&info.state)?,
                info.exit_code,
                info.ended_at_ms.map(|v| v.0.to_string()),
                info.recording_warning,
                info.id.to_string()
            ],
        )?;
        Ok(())
    }
    pub fn host_floor(&self, scope: &RemoteScope) -> Result<(Counter, Option<ClientId>)> {
        let (floor,connection):(Vec<u8>,Option<String>)=self.connection.query_row(
            "SELECT host_floor,connection FROM remote_bindings WHERE origin=?1 AND machine=?2 AND owner=?3",
            params![scope.origin,scope.machine.to_string(),scope.owner.to_string()],|row|Ok((row.get(0)?,row.get(1)?)),
        )?;
        Ok((
            Counter(counter(&floor)?),
            connection
                .map(|value| value.parse().map_err(anyhow::Error::msg))
                .transpose()?,
        ))
    }
    pub fn activate_host_floor(
        &mut self,
        scope: &RemoteScope,
        generation: Counter,
        connection: ClientId,
    ) -> Result<bool> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let updated=transaction.execute(
            "UPDATE remote_bindings SET host_floor=?4,connection=?5 WHERE origin=?1 AND machine=?2 AND owner=?3 AND (host_floor<?4 OR (host_floor=?4 AND connection=?5))",
            params![scope.origin,scope.machine.to_string(),scope.owner.to_string(),generation.0.to_be_bytes().as_slice(),connection.to_string()],
        )?;
        transaction.commit()?;
        Ok(updated == 1)
    }
    pub fn generation(&self, id: SessionId, generation: u64) -> Result<()> {
        self.connection.execute(
            "UPDATE sessions SET generation=?1 WHERE id=?2",
            params![generation.to_be_bytes().as_slice(), id.to_string()],
        )?;
        Ok(())
    }
    pub fn append(
        &mut self,
        id: SessionId,
        event: &OutputEvent,
        checkpoint: impl FnOnce() -> Result<Vec<u8>, RemoteError>,
    ) -> Result<()> {
        let encoded = serde_json::to_string(event)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO replay VALUES(?1,?2,?3,?4)",
            params![
                id.to_string(),
                event.seq.0.to_be_bytes().as_slice(),
                encoded,
                encoded.len()
            ],
        )?;
        tx.execute(
            "UPDATE sessions SET head=?1 WHERE id=?2",
            params![event.seq.0.to_be_bytes().as_slice(), id.to_string()],
        )?;
        let mut query =
            tx.prepare("SELECT seq,bytes FROM replay WHERE session_id=?1 ORDER BY seq DESC")?;
        let mut rows = query.query([id.to_string()])?;
        let mut count = 0;
        let mut total = 0;
        let mut floor = None;
        while let Some(row) = rows.next()? {
            count += 1;
            total += row.get::<_, usize>(1)?;
            if count > self.max_events || total > self.max_bytes {
                break;
            }
            floor = Some(row.get::<_, Vec<u8>>(0)?);
        }
        drop(rows);
        drop(query);
        let checkpoint_seq: Option<Vec<u8>> = tx
            .query_row(
                "SELECT seq FROM terminal_checkpoints WHERE session_id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let checkpoint_seq = checkpoint_seq
            .as_deref()
            .map(counter)
            .transpose()?
            .unwrap_or(0);
        let floor_seq = floor
            .as_deref()
            .map(counter)
            .transpose()?
            .unwrap_or(event.seq.0);
        // Commit a complete checkpoint in this same transaction before pruning
        // its required suffix. Periodic checkpoints bound normal restart work.
        if (event.seq.0.is_multiple_of(64)
            || floor_seq > checkpoint_seq.saturating_add(1)
            || matches!(event.body, EventBody::Gap { .. }))
            && let Ok(data) = checkpoint()
        {
            tx.execute(
                "INSERT INTO terminal_checkpoints VALUES(?1,?2,?3) ON CONFLICT(session_id) DO UPDATE SET seq=excluded.seq,data=excluded.data",
                params![id.to_string(), event.seq.0.to_be_bytes().as_slice(), data],
            )?;
        }
        if let Some(floor) = floor {
            tx.execute(
                "DELETE FROM replay WHERE session_id=?1 AND seq<?2",
                params![id.to_string(), floor],
            )?;
        } else {
            tx.execute("DELETE FROM replay WHERE session_id=?1", [id.to_string()])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn replay(
        &self,
        id: SessionId,
        after: Counter,
    ) -> Result<(Counter, Vec<OutputEvent>, bool)> {
        let floor: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT seq FROM replay WHERE session_id=?1 ORDER BY seq LIMIT 1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let mut query = self.connection.prepare(
            "SELECT event FROM replay WHERE session_id=?1 AND seq>?2 ORDER BY seq LIMIT 65",
        )?;
        let encoded = query
            .query_map(
                params![id.to_string(), after.0.to_be_bytes().as_slice()],
                |r| r.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut bytes = 0;
        let mut events = Vec::new();
        let mut more = false;
        for text in encoded {
            if events.len() >= 64 || bytes + text.len() > 512 * 1024 {
                more = true;
                break;
            }
            bytes += text.len();
            events.push(serde_json::from_str(&text)?);
        }
        Ok((
            Counter(floor.as_deref().map(counter).transpose()?.unwrap_or(0)),
            events,
            more,
        ))
    }
    pub fn input_lookup(
        &self,
        id: SessionId,
        generation: Counter,
        seq: Counter,
        digest: &str,
    ) -> Result<Option<ApplicationOutcome>> {
        Ok(self
            .input_receipt(id, generation, seq, digest)?
            .map(|receipt| receipt.outcome))
    }

    pub fn input_record(
        &self,
        id: SessionId,
        generation: Counter,
        seq: Counter,
        digest: &str,
        outcome: ApplicationOutcome,
    ) -> Result<()> {
        self.record_input_receipt(
            id,
            generation,
            seq,
            digest,
            wire::InputReceipt {
                outcome,
                turn: None,
            },
        )
    }
    pub fn input_receipt(
        &self,
        id: SessionId,
        generation: Counter,
        seq: Counter,
        digest: &str,
    ) -> Result<Option<wire::InputReceipt>> {
        let found: Option<(String, String, Option<String>)> = self.connection.query_row(
            "SELECT digest,outcome,turn FROM input_ack WHERE session_id=?1 AND generation=?2 AND seq=?3",
            params![id.to_string(), generation.0.to_be_bytes().as_slice(), seq.0.to_be_bytes().as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
        found
            .map(|(existing, outcome, turn)| {
                anyhow::ensure!(
                    existing == digest,
                    "input sequence conflicts with earlier bytes"
                );
                Ok(wire::InputReceipt {
                    outcome: serde_json::from_str(&outcome)?,
                    turn,
                })
            })
            .transpose()
    }
    pub fn record_input_receipt(
        &self,
        id: SessionId,
        generation: Counter,
        seq: Counter,
        digest: &str,
        receipt: wire::InputReceipt,
    ) -> Result<()> {
        self.connection.execute("INSERT INTO input_ack(session_id,generation,seq,digest,outcome,turn) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(session_id,generation,seq) DO UPDATE SET outcome=excluded.outcome,turn=excluded.turn",params![id.to_string(),generation.0.to_be_bytes().as_slice(),seq.0.to_be_bytes().as_slice(),digest,serde_json::to_string(&receipt.outcome)?,receipt.turn])?;
        self.connection.execute("DELETE FROM input_ack WHERE session_id=?1 AND (generation<>?2 OR seq NOT IN (SELECT seq FROM input_ack WHERE session_id=?1 AND generation=?2 ORDER BY seq DESC LIMIT 128))",params![id.to_string(),generation.0.to_be_bytes().as_slice()])?;
        Ok(())
    }
    pub fn checkpoint(&self, id: SessionId) -> Result<Option<(Counter, Vec<u8>)>> {
        let found: Option<(Vec<u8>, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT seq,data FROM terminal_checkpoints WHERE session_id=?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        found
            .map(|(seq, data)| Ok((Counter(counter(&seq)?), data)))
            .transpose()
    }
    pub fn snapshot_tail(&self, id: SessionId, after: Counter) -> Result<Vec<OutputEvent>> {
        let mut query = self
            .connection
            .prepare("SELECT event FROM replay WHERE session_id=?1 AND seq>?2 ORDER BY seq")?;
        query
            .query_map(
                params![id.to_string(), after.0.to_be_bytes().as_slice()],
                |r| r.get::<_, String>(0),
            )?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect()
    }
}
fn counter(bytes: &[u8]) -> Result<u64> {
    Ok(u64::from_be_bytes(
        bytes.try_into().context("invalid persisted counter")?,
    ))
}

#[cfg(test)]
mod managed_identity_tests {
    use super::*;
    #[test]
    fn managed_identity_recovery_preserves_native_source_and_scoped_start_key() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("sessions.db");
        let mut store = Store::open(&path, 64, 65536).unwrap();
        let machine = store.machine().unwrap();
        let scope = RemoteScope {
            origin: "https://first.example".into(),
            owner: UserId::new(),
            machine: MachineId::new(),
        };
        let other = RemoteScope {
            origin: "https://second.example".into(),
            owner: UserId::new(),
            machine: scope.machine,
        };
        store.bind(&scope).unwrap();
        store.bind(&other).unwrap();
        let spec = StartSpec {
            key: IdempotencyKey::new(),
            command: vec!["codex".into()],
            cwd: root.path().to_str().unwrap().into(),
            rows: 24,
            cols: 80,
        };
        let info = SessionInfo {
            id: SessionId::new(),
            machine: scope.machine,
            command: spec.command.clone(),
            cwd: spec.cwd.clone(),
            state: SessionState::Running,
            created_at_ms: Counter(1),
            ended_at_ms: None,
            head: Counter(0),
            controller: None,
            exit_code: None,
            recording_warning: None,
            agent: Some(ManagedSessionInfo {
                requested_model: Some("configured-model".into()),
                native_home: None,
                agent: "codex".into(),
                native_instance: MessageId::new(),
                native_thread: Some("owned-native-thread".into()),
            }),
        };
        store
            .insert(&info, &spec, "managed-fingerprint", Some(&scope))
            .unwrap();
        let mut second = info.clone();
        second.id = SessionId::new();
        store
            .insert(&second, &spec, "other-fingerprint", Some(&other))
            .unwrap();
        let mut duplicate = info.clone();
        duplicate.id = SessionId::new();
        assert!(
            store
                .insert(&duplicate, &spec, "changed-fingerprint", Some(&scope))
                .is_err()
        );
        store
            .append(
                info.id,
                &OutputEvent {
                    seq: Counter(1),
                    at_ms: Counter(2),
                    body: EventBody::Agent(qgenie_remote_proto::wire::AgentEvent::Idle),
                },
                || {
                    Err(RemoteError::unknown(
                        "structured session has no terminal checkpoint",
                    ))
                },
            )
            .unwrap();
        store
            .input_record(
                info.id,
                Counter(1),
                Counter(1),
                "unknown-digest",
                ApplicationOutcome::OutcomeUnknown,
            )
            .unwrap();
        store
            .record_input_receipt(
                info.id,
                Counter(1),
                Counter(2),
                "receipt-digest",
                wire::InputReceipt {
                    outcome: ApplicationOutcome::Applied,
                    turn: Some("exact-native-turn".into()),
                },
            )
            .unwrap();
        drop(store);
        let store = Store::open(&path, 64, 65536).unwrap();
        let unknown = store
            .input_receipt(info.id, Counter(1), Counter(1), "unknown-digest")
            .unwrap()
            .unwrap();
        assert!(matches!(
            unknown.outcome,
            ApplicationOutcome::OutcomeUnknown
        ));
        assert!(unknown.turn.is_none());
        let receipt = store
            .input_receipt(info.id, Counter(1), Counter(2), "receipt-digest")
            .unwrap()
            .unwrap();
        assert!(matches!(receipt.outcome, ApplicationOutcome::Applied));
        assert_eq!(receipt.turn.as_deref(), Some("exact-native-turn"));
        assert!(
            store
                .input_receipt(info.id, Counter(1), Counter(2), "different-digest")
                .is_err()
        );
        let rows = store.load(machine).unwrap();
        assert_eq!(rows.len(), 2);
        let recovered = rows.iter().find(|row| row.info.id == info.id).unwrap();
        assert!(matches!(recovered.info.state, SessionState::Lost));
        assert_eq!(recovered.spec.key, spec.key);
        assert_eq!(recovered.scope.as_ref(), Some(&scope));
        let identity = recovered.info.agent.as_ref().unwrap();
        assert_eq!(
            identity.requested_model.as_deref(),
            Some("configured-model")
        );
        assert_eq!(
            identity.native_instance,
            info.agent.unwrap().native_instance
        );
        assert_eq!(
            identity.native_thread.as_deref(),
            Some("owned-native-thread")
        );
        let (_, events, _) = store.replay(recovered.info.id, Counter(0)).unwrap();
        assert!(matches!(
            events[0].body,
            EventBody::Agent(qgenie_remote_proto::wire::AgentEvent::Idle)
        ));
        assert!(
            store
                .bind(&RemoteScope {
                    owner: UserId::new(),
                    ..scope
                })
                .is_err()
        );
    }
}
