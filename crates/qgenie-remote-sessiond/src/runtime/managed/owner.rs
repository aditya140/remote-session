//! Native process ownership and nonblocking event admission.
use super::*;
impl Runtime {
    pub(super) async fn own_agent(
        self: Arc<Self>,
        state: Arc<Mutex<Session>>,
        mut launch: LaunchOwned,
        channels: OwnerChannels,
    ) {
        let OwnerChannels {
            mut commands,
            turn,
            events,
            lost,
            ack,
        } = channels;
        let mut recording_guard = capture::StartGuard::new(
            launch
                .recording
                .as_ref()
                .map(|capture| capture.source.clone()),
        );
        use std::os::fd::AsRawFd;
        let thread_cwd = PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            launch.directory.as_raw_fd()
        ));
        let mut native = match Codex::launch_at_authorized(
            launch.native,
            launch.directory,
            launch.authorization.clone(),
        )
        .await
        {
            Ok(native) => native,
            Err(_) => {
                let _ =
                    ack.send(self.fail_agent_start(
                        &state,
                        "Native startup failed; reconcile the accepted key",
                    ));
                return;
            }
        };
        // From this point, only this independent owner can drop the native process.
        #[cfg(test)]
        self.managed_fault(StartFault::NativeLaunched);
        recording_guard.handed_off();
        let native_recording = capture::NativeGuard::from_capture(launch.recording.as_ref());
        let request = native.handle().authorized(launch.authorization);
        if let Some(prompt) = &launch.initial_prompt
            && let Some(capture) = &mut launch.recording
        {
            capture.push(crate::CaptureContent::AgentText {
                channel: RecordingChannel::UserMessage,
                bytes: prompt.as_bytes().to_vec(),
            });
        }
        let initialization = self.clone();
        let initializing_state = state.clone();
        let init = tokio::spawn(async move {
            initialization
                .initialize_agent(
                    initializing_state,
                    request,
                    thread_cwd,
                    launch.initial_prompt,
                    launch.resume_thread,
                    ack,
                )
                .await
        });
        let mut connected = true;
        let mut ticker = tokio::time::interval(Duration::from_millis(20));
        loop {
            tokio::select! {
                command = commands.recv() => {
                    if let Some(command) = command {
                        if let ActionRequest::Prompt { text, .. } | ActionRequest::Steer { text, .. } = &command.action
                            && let Some(capture) = &mut launch.recording {
                            capture_text(capture, RecordingChannel::UserMessage, text.as_bytes());
                        }
                        let control = native.process_control();
                        let request = native.handle().authorized(command.authorization.clone());
                        tokio::spawn(async move {
                            let result = match command.action {
                                ActionRequest::Terminate => control.terminate(command.authorization).await.map(|_| InputReceipt { outcome: ApplicationOutcome::Applied, turn: None }),
                                ActionRequest::Prompt { thread, text } => request.prompt(&thread, &text).await.map(|turn| InputReceipt { outcome: ApplicationOutcome::Applied, turn: Some(turn) }),
                                ActionRequest::Steer { thread, turn, text } => request.steer(&thread, &turn, &text).await.map(|_| InputReceipt { outcome: ApplicationOutcome::Applied, turn: None }),
                                ActionRequest::Interrupt { thread, turn } => request.interrupt(&thread, &turn).await.map(|_| InputReceipt { outcome: ApplicationOutcome::Applied, turn: None }),
                                ActionRequest::Answer { approval, decision } => request.answer(&approval, decision).await.map(|outcome| InputReceipt { outcome, turn: None }),
                            }.map_err(native_error);
                            let _ = command.reply.send(result);
                        });
                    }
                },
                event = native.recv(), if connected => {
                    match event {
                        Some(NativeEvent::Event { event, .. }) => {
                            if let Ok(mut active) = turn.lock() {
                                match &event {
                                    AgentEvent::TurnStarted { turn } => *active = Some(turn.clone()),
                                    AgentEvent::TurnCompleted { turn, .. } if active.as_ref() == Some(turn) => *active = None,
                                    _ => {},
                                }
                            }
                            if let Some(capture) = &mut launch.recording {
                                capture_text(capture, event_channel(&event), &serde_json::to_vec(&event).unwrap_or_default());
                            }
                            let size = serde_json::to_vec(&event).map_or(usize::MAX, |event| event.len());
                            if size > 32768 || events.try_send(Persist::Event(event)).is_err() { lost.lost.fetch_add(size.min(1024 * 1024), Ordering::Relaxed); }
                        },
                        Some(NativeEvent::Approval(approval)) => {
                            if let Some(capture) = &mut launch.recording { capture_text(capture, RecordingChannel::ApprovalRequest, approval.summary.as_bytes()); }
                            if events.try_send(Persist::Approval(approval)).is_err() { lost.lost.fetch_add(1, Ordering::Relaxed); }
                        },
                        Some(NativeEvent::ApprovalResolved { native_id, .. }) => { if events.try_send(Persist::Resolved(native_id)).is_err() { lost.lost.fetch_add(1, Ordering::Relaxed); } },
                        None => {
                            connected = false;
                            lost.disconnected.store(true, Ordering::Release);
                            if let Some(capture) = &mut launch.recording { capture.source.finish_uncertain(capture::wall_now().unwrap_or_else(|| capture.last_at())); }
                        },
                    }
                },
                _ = ticker.tick() => {
                    if self.stopping.load(Ordering::Acquire) {
                        let _ = native.process_control().terminate(Arc::new(Local)).await;
                    }
                    if let Some(exit) = native.native_exit() {
                        // End metadata is reliable even if replay storage is stalled.
                        // A blocking final enqueue happens only after native reap.
                        let _ = events.send(Persist::End(exit));
                        break;
                    }
                },
            }
        }
        if connected {
            native_recording.complete();
        }
        init.abort();
        let _ = native.shutdown().await;
    }

    async fn initialize_agent(
        self: Arc<Self>,
        state: Arc<Mutex<Session>>,
        native: CodexHandle,
        thread_cwd: PathBuf,
        prompt: Option<String>,
        resume: Option<String>,
        ack: channel::SyncSender<Result<SessionInfo, RemoteError>>,
    ) {
        // ProcessOwner retains the exact launch descriptor until native reap.
        let thread = match resume {
            Some(thread) => native.resume_thread_at(&thread, &thread_cwd).await,
            None => native.start_thread(&thread_cwd).await,
        };
        let runtime = self.clone();
        let saved = state.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut state = lock(&saved)?;
            match thread {
                Ok(thread) => {
                    let mut identity = state.info.agent.clone().unwrap();
                    identity.native_thread = Some(thread.clone());
                    lock(&runtime.store)?
                        .managed_identity(state.info.id, &identity)
                        .map_err(storage_error)?;
                    state.info.agent = Some(identity);
                    state.info.state = SessionState::Idle;
                    lock(&runtime.store)?
                        .state(state.info.id, &state.info.state, None)
                        .map_err(storage_error)?;
                    runtime.publish_change(state.scope.as_ref());
                    Ok((thread, state.info.clone()))
                }
                Err(error) => {
                    state.info.state = SessionState::OutcomeUnknown;
                    let _ = lock(&runtime.store)?.state(state.info.id, &state.info.state, None);
                    Err(native_error(error))
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            Err(RemoteError::unknown(
                "native thread identity commit unavailable",
            ))
        });
        match result {
            Ok((thread, info)) => {
                #[cfg(test)]
                self.managed_fault(StartFault::RunningCommitted);
                // Commit native correlation before any initial prompt is transmitted.
                if let Some(prompt) = prompt
                    && native.prompt(&thread, &prompt).await.is_err()
                {
                    let _ = ack.send(Err(RemoteError::unknown(
                        "initial prompt outcome unknown; never resend it",
                    )));
                    return;
                }
                #[cfg(test)]
                self.managed_fault(StartFault::BeforeAck);
                let _ = ack.send(Ok(info));
            }
            Err(error) => {
                let _ = ack.send(Err(error));
            }
        }
    }
}
