//! Historical metadata stays in SQLite. Only execution owners need resident state.
use super::*;

impl From<crate::store::StoredSession> for Session {
    fn from(stored: crate::store::StoredSession) -> Self {
        Self {
            agent_state: None,
            #[cfg(all(feature = "managed-codex", target_os = "linux"))]
            agent_control: None,
            recording: None,
            remote_controller: None,
            info: stored.info,
            spec: stored.spec,
            fingerprint: stored.fingerprint,
            generation: stored.generation,
            input_seq: 0,
            deadline: None,
            deadline_elapsed: None,
            control: None,
            screen: None,
            pending_lost_bytes: 0,
            scope: stored.scope,
        }
    }
}

impl Runtime {
    pub(super) fn lookup_start(
        &self,
        scope: Option<&RemoteScope>,
        key: IdempotencyKey,
    ) -> Result<Option<Arc<Mutex<Session>>>, RemoteError> {
        let id = lock(&self.store)?
            .lookup(scope, key)
            .map_err(storage_error)?;
        id.map(|id| self.session(id)).transpose()
    }

    // The start lock serializes reclamation with new ownership. A caller/reader
    // retaining an Arc keeps its state resident until a later pass.
    pub(super) fn reap_sessions(&self) -> Result<(), RemoteError> {
        {
            let mut readers = lock(&self.readers)?;
            let mut pending = Vec::new();
            for reader in readers.drain(..) {
                if reader.is_finished() {
                    let _ = reader.join();
                } else {
                    pending.push(reader);
                }
            }
            *readers = pending;
        }
        let candidates = lock(&self.sessions)?.values().cloned().collect::<Vec<_>>();
        for session in candidates {
            if Arc::strong_count(&session) != 2 {
                continue;
            }
            let Ok(state) = session.try_lock() else {
                continue;
            };
            if owns_process(&state) {
                continue;
            }
            lock(&self.store)?
                .finished(&state.info)
                .map_err(storage_error)?;
            let mut sessions = lock(&self.sessions)?;
            if Arc::strong_count(&session) == 2 {
                sessions.remove(&state.info.id);
            }
        }
        Ok(())
    }

    pub(super) fn local_page(
        &self,
        after: Option<SessionId>,
        limit: u16,
        active_only: bool,
    ) -> Result<(Vec<SessionInfo>, Option<SessionId>), RemoteError> {
        let ids = self.page_ids(None, after, limit, active_only)?;
        let mut sessions = Vec::new();
        let mut bytes = 2; // JSON array brackets; each later record also needs a comma.
        for id in ids {
            let session = self.session(id)?;
            let mut state = lock(&session)?;
            expire(&mut state);
            self.recording_warning(&mut state);
            bytes += serde_json::to_vec(&state.info)
                .map_err(storage_error)?
                .len()
                + usize::from(!sessions.is_empty());
            if sessions.len() >= usize::from(limit) || bytes > 512 * 1024 {
                let last = sessions
                    .last()
                    .map(|info: &SessionInfo| info.id)
                    .ok_or_else(|| {
                        RemoteError::rejected(
                            ErrorCode::Overloaded,
                            "session metadata exceeds page budget",
                        )
                    })?;
                return Ok((sessions, Some(last)));
            }
            sessions.push(state.info.clone());
        }
        Ok((sessions, None))
    }
    pub(super) fn page_ids(
        &self,
        scope: Option<&RemoteScope>,
        after: Option<SessionId>,
        limit: u16,
        active_only: bool,
    ) -> Result<Vec<SessionId>, RemoteError> {
        if !(1..=256).contains(&limit) {
            return Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "session page limit must be 1..=256",
            ));
        }
        lock(&self.store)?
            .page_ids(scope, after, limit, active_only)
            .map_err(storage_error)
    }
    pub(super) fn remote_session(
        &self,
        scope: &RemoteScope,
        id: SessionId,
    ) -> Result<wire::Session, RemoteError> {
        let session = self.session(id)?;
        let mut state = lock(&session)?;
        if state.scope.as_ref() != Some(scope) {
            return Err(RemoteError::rejected(
                ErrorCode::NotFound,
                "session not found in this scope",
            ));
        }
        expire(&mut state);
        self.recording_warning(&mut state);
        let info = &state.info;
        Ok(wire::Session {
            requested_model: info
                .agent
                .as_ref()
                .and_then(|agent| agent.requested_model.clone()),
            metadata: Default::default(),
            id: info.id,
            machine: info.machine,
            owner: scope.owner,
            kind: authority::native_kind(info),
            cwd: info.cwd.clone(),
            origin: wire::Origin::Managed,
            state: info.state.clone(),
            created_at: wire::Timestamp::from_unix_millis(info.created_at_ms.0)?,
            ended_at: info
                .ended_at_ms
                .map(|at| wire::Timestamp::from_unix_millis(at.0))
                .transpose()?,
            head: info.head,
            capabilities: authority::session_capabilities(&state, self.can_resume_history()),
            recording_warning: info.recording_warning.clone(),
        })
    }
    pub(super) fn remote_list(
        &self,
        scope: &RemoteScope,
    ) -> Result<Vec<wire::Session>, RemoteError> {
        let mut sessions = Vec::new();
        let mut after = None;
        let mut bytes = 2; // JSON array brackets; each later record also needs a comma.
        loop {
            let (page, next) = self.remote_page(scope, after, 256, false)?;
            for info in page {
                bytes += serde_json::to_vec(&info).map_err(storage_error)?.len()
                    + usize::from(!sessions.is_empty());
                if sessions.len() >= 1024 || bytes > 512 * 1024 {
                    return Err(RemoteError::rejected(
                        ErrorCode::Overloaded,
                        "session metadata requires paginated listing",
                    ));
                }
                sessions.push(info);
            }
            after = next;
            if after.is_none() {
                break;
            }
        }
        Ok(sessions)
    }
    pub(super) fn remote_page(
        &self,
        scope: &RemoteScope,
        after: Option<SessionId>,
        limit: u16,
        active_only: bool,
    ) -> Result<(Vec<wire::Session>, Option<SessionId>), RemoteError> {
        let ids = self.page_ids(Some(scope), after, limit, active_only)?;
        let mut sessions = Vec::new();
        let mut bytes = 2; // JSON array brackets; each later record also needs a comma.
        for id in ids {
            let session = self.remote_session(scope, id)?;
            bytes += serde_json::to_vec(&session).map_err(storage_error)?.len()
                + usize::from(!sessions.is_empty());
            if sessions.len() >= usize::from(limit) || bytes > 512 * 1024 {
                let last = sessions
                    .last()
                    .map(|s: &wire::Session| s.id)
                    .ok_or_else(|| {
                        RemoteError::rejected(
                            ErrorCode::Overloaded,
                            "session metadata exceeds page budget",
                        )
                    })?;
                return Ok((sessions, Some(last)));
            }
            sessions.push(session);
        }
        Ok((sessions, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_history_is_paged_without_resident_owners_or_relaunching_old_keys() {
        let dir = super::super::tests::directory();
        let config = Config::new(dir.path().to_owned(), vec![dir.path().to_owned()]);
        let runtime = Runtime::open(config.clone()).unwrap();
        let scope = RemoteScope {
            origin: "https://history.example".into(),
            owner: UserId::new(),
            machine: MachineId::new(),
        };
        lock(&runtime.store).unwrap().bind(&scope).unwrap();
        let mut original = None;
        for n in 0..4100 {
            let mut spec = super::super::tests::spec(dir.path(), IdempotencyKey::new());
            if n < 16 {
                spec.command.push(String::new());
            }
            let mut parameters = serde_json::to_value(&spec).unwrap();
            parameters.as_object_mut().unwrap().remove("key");
            let remote = (n >= 16 && n % 2 == 1).then_some(&scope);
            let mut info = SessionInfo {
                id: if n < 16 {
                    SessionId::from_bytes((n as u128 + 1).to_be_bytes())
                } else {
                    SessionId::new()
                },
                machine: remote.map_or(runtime.machine, |s| s.machine),
                command: spec.command.clone(),
                cwd: spec.cwd.clone(),
                state: SessionState::Exited,
                created_at_ms: Counter(n),
                ended_at_ms: Some(Counter(n + 1)),
                head: Counter(0),
                controller: None,
                exit_code: Some(0),
                recording_warning: Some("recording tail is uncertain".into()),
                agent: None,
            };
            if n < 16 {
                // Eight records alone fill the budget. Their JSON array also
                // needs brackets and separators, independent of path length.
                let padding = 64 * 1024 - serde_json::to_vec(&info).unwrap().len();
                *spec.command.last_mut().unwrap() = "x".repeat(padding);
                info.command = spec.command.clone();
                parameters = serde_json::to_value(&spec).unwrap();
                parameters.as_object_mut().unwrap().remove("key");
            }
            let store = lock(&runtime.store).unwrap();
            store
                .insert(&info, &spec, &fingerprint(&parameters).unwrap(), remote)
                .unwrap();
            store.finished(&info).unwrap();
            if n == 0 {
                original = Some((spec, info));
            }
        }
        drop(runtime);
        let runtime = Runtime::open(config).unwrap();
        assert!(lock(&runtime.sessions).unwrap().is_empty());
        let finished = thread::spawn(|| {});
        while !finished.is_finished() {
            thread::yield_now();
        }
        lock(&runtime.readers).unwrap().push(finished);
        runtime.reap_sessions().unwrap();
        assert!(lock(&runtime.readers).unwrap().is_empty());
        let mut after = None;
        let mut ids = std::collections::BTreeSet::new();
        loop {
            let (page, next) = runtime.local_page(after, 113, false).unwrap();
            assert!(page.len() <= 113);
            assert!(serde_json::to_vec(&page).unwrap().len() <= 512 * 1024);
            if after.is_none() {
                assert!(page.len() < 16, "byte budget must cut the first page");
            }
            assert!(page.windows(2).all(|pair| pair[0].id < pair[1].id));
            for info in &page {
                assert!(ids.insert(info.id));
            }
            if next.is_none() {
                break;
            }
            assert_eq!(next, page.last().map(|info| info.id));
            after = next;
        }
        assert_eq!(ids.len(), 4100);
        assert!(runtime.local_page(None, 64, true).unwrap().0.is_empty());
        let (page, next) = runtime.remote_page(&scope, None, 64, false).unwrap();
        assert!(!page.is_empty() && page.len() <= 64);
        assert!(next.is_some());
        assert!(
            page.iter()
                .all(|s| s.owner == scope.owner && s.machine == scope.machine)
        );
        let foreign = RemoteScope {
            owner: UserId::new(),
            ..scope.clone()
        };
        assert!(
            runtime
                .remote_page(&foreign, None, 64, false)
                .unwrap()
                .0
                .is_empty()
        );
        assert_eq!(
            runtime
                .remote_session(&foreign, page[0].id)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        assert!(lock(&runtime.sessions).unwrap().is_empty());
        let (spec, info) = original.unwrap();
        let reconciled = runtime.start(spec.clone()).unwrap();
        assert_eq!(reconciled.id, info.id);
        assert_eq!(
            reconciled.recording_warning.as_deref(),
            Some("recording tail is uncertain")
        );
        let mut conflict = spec;
        conflict.cols += 1;
        assert_eq!(
            runtime.start(conflict).unwrap_err().code,
            ErrorCode::IdempotencyConflict
        );
        assert!(!dir.path().join("native-marker").exists());
        let mut launch = super::super::tests::spec(dir.path(), IdempotencyKey::new());
        launch.command[2] = format!(
            "while [ ! -f finish-live ]; do sleep 0.01; done; {}",
            launch.command[2]
        );
        let started = runtime.start(launch).unwrap();
        let (live, next) = runtime.local_page(None, 64, true).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, started.id);
        assert!(next.is_none());
        std::fs::write(dir.path().join("finish-live"), b"done").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state = runtime.session(started.id).unwrap();
            if matches!(lock(&state).unwrap().info.state, SessionState::Exited) {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            std::fs::read(dir.path().join("native-marker")).unwrap(),
            b"x"
        );
        runtime.reap_sessions().unwrap();
        assert!(lock(&runtime.sessions).unwrap().len() <= 1);
        assert_eq!(lock(&runtime.store).unwrap().count().unwrap(), 4101);
    }
}
