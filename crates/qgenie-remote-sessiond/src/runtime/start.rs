use super::*;
impl Runtime {
    pub(super) fn start(self: &Arc<Self>, spec: StartSpec) -> Result<SessionInfo, RemoteError> {
        self.start_scoped(spec, None)
    }
    // Trusted local/domain entry. Network starts use start_guarded with their original fence.
    pub(super) fn start_scoped(
        self: &Arc<Self>,
        spec: StartSpec,
        scope: Option<RemoteScope>,
    ) -> Result<SessionInfo, RemoteError> {
        self.start_guarded(spec, scope, None)
    }
    pub(super) fn start_guarded(
        self: &Arc<Self>,
        spec: StartSpec,
        scope: Option<RemoteScope>,
        fence: Option<authority::Fence>,
    ) -> Result<SessionInfo, RemoteError> {
        let _start = lock(&self.starts)?;
        if let Some(fence) = &fence {
            fence.check()?;
        }
        if let Some(scope) = &scope {
            scope.validate()?;
            if !lock(&self.store)?.bound(scope).map_err(storage_error)? {
                return Err(RemoteError::rejected(
                    ErrorCode::Forbidden,
                    "native owner binding is not registered",
                ));
            }
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err(RemoteError::rejected(
                ErrorCode::Forbidden,
                "session daemon is stopping",
            ));
        }
        let mut parameters = serde_json::to_value(&spec).map_err(storage_error)?;
        parameters.as_object_mut().unwrap().remove("key");
        let digest = fingerprint(&parameters)?;
        if let Some(session) = self.lookup_start(scope.as_ref(), spec.key)? {
            let state = lock(&session)?;
            if state.fingerprint != digest {
                return Err(RemoteError::rejected(
                    ErrorCode::IdempotencyConflict,
                    "start key has different parameters",
                ));
            }
            return Ok(state.info.clone());
        }
        self.reap_sessions()?;
        let sessions: Vec<_> = lock(&self.sessions)?.values().cloned().collect();
        validate_size(spec.rows, spec.cols)?;
        if spec.command.is_empty()
            || spec.command.len() > 128
            || spec.command.iter().any(|s| s.contains('\0'))
            || spec.command.iter().map(String::len).sum::<usize>() > 65536
        {
            return Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "invalid command or arguments",
            ));
        }
        let directory = self
            .directories
            .resolve(std::path::Path::new(&spec.cwd))
            .map_err(|_| {
                RemoteError::rejected(ErrorCode::Forbidden, "working directory is unavailable")
            })?;
        let cwd = directory.path;
        authorize(
            Action::Start,
            Facts {
                within_roots: true,
                ..local_facts()
            },
        )?;
        let active = sessions
            .iter()
            .filter(|s| s.lock().is_ok_and(|s| owns_process(&s)))
            .count();
        if active >= self.config.max_sessions {
            return Err(RemoteError::rejected(
                ErrorCode::CapacityExhausted,
                "session registry capacity reached",
            ));
        }
        let mut launch = spec.clone();
        launch.cwd = cwd.display().to_string();
        let prepared = PreparedPty::with_directory(
            launch.clone(),
            LaunchEnvironment::default(),
            directory.file,
        )
        .map_err(|_| {
            RemoteError::rejected(ErrorCode::InvalidFrame, "invalid PTY launch parameters")
        })?;
        let info = SessionInfo {
            agent: None,
            id: SessionId::new(),
            machine: scope.as_ref().map_or(self.machine, |scope| scope.machine),
            command: spec.command.clone(),
            cwd: cwd.display().to_string(),
            state: SessionState::Accepted,
            created_at_ms: Counter(now_ms()),
            ended_at_ms: None,
            head: Counter(0),
            controller: None,
            exit_code: None,
            recording_warning: Some(if scope.is_some() {
                "Recording initialization pending".into()
            } else {
                "Local preview replay only; this session is not enrolled for remote recording"
                    .into()
            }),
        };
        let state = Arc::new(Mutex::new(Session {
            agent_state: None,
            #[cfg(all(feature = "managed-codex", target_os = "linux"))]
            agent_control: None,
            recording: None,
            remote_controller: None,
            info: info.clone(),
            spec: spec.clone(),
            fingerprint: digest.clone(),
            generation: 0,
            input_seq: 0,
            deadline: None,
            deadline_elapsed: None,
            control: None,
            screen: Some(Screen::new(launch.rows, launch.cols)?),
            pending_lost_bytes: 0,
            scope: scope.clone(),
        }));
        // Reserve the owner before durable acceptance. The native launch itself
        // runs in that owner, so cancelling the RPC or failing to create a worker
        // cannot strand a newly created child without an output drain.
        let (launch_tx, launch_rx) = std::sync::mpsc::sync_channel::<PreparedPty>(1);
        let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(1);
        let runtime = self.clone();
        let owned = state.clone();
        let source = fence.clone();
        let reader = thread::Builder::new()
            .name(format!("pty-{}", info.id))
            .spawn(move || {
                if let Ok(prepared) = launch_rx.recv() {
                    runtime.supervise_start(owned, prepared, ack_tx, source);
                }
            })
            .map_err(|_| {
                RemoteError::rejected(
                    ErrorCode::CapacityExhausted,
                    "output supervisor capacity unavailable; no launch accepted",
                )
            })?;
        lock(&self.readers)?.push(reader);
        #[cfg(test)]
        self.fault(StartFault::BeforeJournal);
        if let Some(fence) = &fence {
            fence.check()?;
        }
        lock(&self.store)?
            .insert(&info, &spec, &digest, scope.as_ref())
            .map_err(storage_error)?;
        #[cfg(test)]
        self.fault(StartFault::Accepted);
        lock(&self.sessions)?.insert(info.id, state.clone());
        self.publish_change(scope.as_ref());
        if fence.as_ref().is_some_and(|fence| fence.check().is_err()) {
            return self.fail_unlaunched(
                &state,
                "Source authority expired before native launch; no child was started",
            );
        }
        let mut recording_guard = capture::StartGuard::new(None);
        if let Some(scope) = &scope {
            match self.begin_capture(&info, scope, launch.rows, launch.cols) {
                Ok(recording) => {
                    recording_guard = capture::StartGuard::new(Some(recording.source.clone()));
                    let mut state = lock(&state)?;
                    state.recording = Some(recording);
                    state.info.recording_warning = None;
                }
                Err(_) => {
                    let mut state = lock(&state)?;
                    state.info.state = SessionState::Failed;
                    state.info.recording_warning = Some(
                        "Recording could not initialize before launch; no native child was started"
                            .into(),
                    );
                    lock(&self.store)?
                        .state(info.id, &state.info.state, None)
                        .map_err(storage_error)?;
                    return Ok(state.info.clone());
                }
            }
        }
        if fence.as_ref().is_some_and(|fence| fence.check().is_err()) {
            return self.fail_unlaunched(
                &state,
                "Source authority expired during recording initialization; no child was started",
            );
        }
        lock(&self.store)?
            .state(info.id, &SessionState::Launching, None)
            .map_err(storage_error)?;
        lock(&state)?.info.state = SessionState::Launching;
        #[cfg(test)]
        self.fault(StartFault::Launching);
        launch_tx.send(prepared).map_err(|_| {
            RemoteError::unknown("accepted launch lost its supervisor; reconcile the original key")
        })?;
        recording_guard.handed_off();
        ack_rx
            .recv_timeout(Duration::from_secs(3))
            .unwrap_or_else(|_| {
                Err(RemoteError::unknown(
                    "native launch acknowledgment unavailable; reconcile the original key",
                ))
            })
    }

    fn supervise_start(
        self: Arc<Self>,
        state: Arc<Mutex<Session>>,
        prepared: PreparedPty,
        ack: std::sync::mpsc::SyncSender<Result<SessionInfo, RemoteError>>,
        fence: Option<authority::Fence>,
    ) {
        let recording = match lock(&state) {
            Ok(state) => capture::NativeGuard::new(&state),
            Err(error) => {
                let _ = ack.send(Err(error));
                return;
            }
        };
        if self.stopping.load(Ordering::Acquire) {
            let result = (|| {
                let mut state = lock(&state)?;
                state.info.state = SessionState::Failed;
                lock(&self.store)?
                    .state(state.info.id, &state.info.state, None)
                    .map_err(storage_error)?;
                Ok(state.info.clone())
            })();
            let _ = ack.send(result);
            recording.complete();
            return;
        }
        let launch = || {
            prepared.launch().map_err(|_| {
                RemoteError::unknown("native launch failed; reconcile its original key")
            })
        };
        let launched = match &fence {
            Some(fence) => fence.perform(launch),
            None => launch(),
        };
        let mut process = match launched {
            Ok(process) => process,
            Err(_) => {
                let result = (|| {
                    let mut state = lock(&state)?;
                    state.info.state = SessionState::Failed;
                    lock(&self.store)?
                        .state(state.info.id, &state.info.state, None)
                        .map_err(storage_error)?;
                    Ok(state.info.clone())
                })();
                let _ = ack.send(result);
                recording.complete();
                return;
            }
        };
        #[cfg(test)]
        self.fault(StartFault::NativeLaunched);
        // A confirmed shutdown can race an already executing native spawn. Its
        // owner still drains/reaps the process and applies that explicit stop.
        if self.stopping.load(Ordering::Acquire) {
            let _ = process.control.terminate();
        }
        let result = match lock(&state) {
            Ok(mut state) => {
                state.control = Some(process.control);
                state.info.state = SessionState::Running;
                match lock(&self.store).and_then(|store| {
                    store
                        .state(state.info.id, &state.info.state, None)
                        .map_err(storage_error)
                }) {
                    Ok(()) => {
                        #[cfg(test)]
                        self.fault(StartFault::RunningCommitted);
                        Ok(state.info.clone())
                    }
                    Err(_) => {
                        state.info.recording_warning=Some("Launch occurred but its running-state commit failed; reconcile the original key".into());
                        Err(RemoteError::unknown(
                            "session launched but its state commit failed; reconcile the original key",
                        ))
                    }
                }
            }
            Err(error) => Err(error),
        };
        // A lost caller/acknowledgment never drops the native owner.
        #[cfg(test)]
        self.fault(StartFault::BeforeAck);
        let _ = ack.send(result);
        let mut bytes = [0u8; 4096];
        let mut primary_exit = None;
        let mut closed_early = false;
        let mut complete_eof = false;
        loop {
            if primary_exit.is_none()
                && let Ok(Some(status)) = process.child.try_wait()
            {
                primary_exit = Some((status.exit_code(), Instant::now(), capture::wall_now()));
            }
            if primary_exit
                .as_ref()
                .is_some_and(|(_, at, _)| at.elapsed() >= Duration::from_millis(500))
            {
                closed_early = true;
                break;
            }
            match process.reader.try_read(&mut bytes) {
                Ok(0) => {
                    complete_eof = true;
                    break;
                }
                Ok(length) => self.output(&state, &bytes[..length]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(_) => break,
            }
        }
        let (code, ended) = match primary_exit {
            Some((code, _, at)) => (code, at),
            None => match process.child.wait() {
                Ok(status) => (status.exit_code(), capture::wall_now()),
                Err(_) => (255, None),
            },
        };
        if complete_eof && !closed_early {
            recording.complete();
        }
        if let Ok(mut state) = state.lock() {
            if state.scope.is_some() && (!complete_eof || closed_early) {
                state.info.recording_warning=Some("Native output ended without complete EOF; recording tail has conservative loss".into());
            }
            let _ = self.commit_gap(&mut state);
            if closed_early && let Some(next) = state.info.head.0.checked_add(1).map(Counter) {
                let _ = self.append(&mut state,EventBody::Gap {from:next,to:next,reason:"primary process exited; an inherited terminal descriptor remained open beyond the bounded drain".into()});
            }
            let _ = self.append(&mut state, EventBody::Exited { code });
            state.info.state = if state.pending_lost_bytes > 0 {
                SessionState::Lost
            } else {
                SessionState::Exited
            };
            state.info.exit_code = Some(code);
            state.info.ended_at_ms = ended
                .map(|at| Counter(at.unix_millis()))
                .filter(|at| *at >= state.info.created_at_ms);
            if state.scope.is_some() && state.info.ended_at_ms.is_none() {
                state.info.recording_warning=Some("Native exit time is unknown; recording history cannot certify the session lifetime".into());
            }
            clear_control(&mut state);
            state.control = None;
            state.screen = None;
            if self
                .store
                .lock()
                .map_or(true, |store| store.finished(&state.info).is_err())
            {
                state.info.recording_warning = Some(
                    "Native end metadata could not be persisted; reconcile recording history"
                        .into(),
                );
            }
        }
    }
    fn fail_unlaunched(
        &self,
        state: &Arc<Mutex<Session>>,
        reason: &str,
    ) -> Result<SessionInfo, RemoteError> {
        let mut state = lock(state)?;
        state.info.state = SessionState::Failed;
        state.info.recording_warning = Some(reason.into());
        lock(&self.store)?
            .state(state.info.id, &state.info.state, None)
            .map_err(storage_error)?;
        Ok(state.info.clone())
    }
}
