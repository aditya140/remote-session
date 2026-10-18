//! Durable replay and approval work runs outside the native drain.
use super::*;
impl Runtime {
    pub(super) fn persist_agent(
        self: Arc<Self>,
        state: Arc<Mutex<Session>>,
        events: channel::Receiver<Persist>,
        lost: Arc<DrainStatus>,
    ) {
        loop {
            let event = match events.recv_timeout(Duration::from_millis(50)) {
                Ok(event) => Some(event),
                Err(channel::RecvTimeoutError::Timeout)
                    if lost.disconnected.load(Ordering::Acquire)
                        || lost.lost.load(Ordering::Relaxed) > 0 =>
                {
                    None
                }
                Err(channel::RecvTimeoutError::Timeout) => continue,
                Err(channel::RecvTimeoutError::Disconnected) => break,
            };
            let Ok(mut state) = state.lock() else {
                return;
            };
            state.pending_lost_bytes = state
                .pending_lost_bytes
                .saturating_add(lost.lost.swap(0, Ordering::AcqRel));
            if lost.disconnected.swap(false, Ordering::AcqRel) {
                state.info.state = SessionState::OutcomeUnknown;
                state.info.recording_warning = Some("Native protocol disconnected; process remains owned and output continuity is unknown".into());
                state.pending_lost_bytes = state.pending_lost_bytes.saturating_add(1);
            }
            let _ = self.commit_gap(&mut state);
            let Some(event) = event else {
                continue;
            };
            match event {
                Persist::Approval(approval) => {
                    if self.record_native_approval(&state, approval).is_err() {
                        state.info.recording_warning = Some(
                            "Native approval could not be journaled; no answer will be sent".into(),
                        );
                    }
                }
                Persist::Resolved(native_id) => {
                    if let Some(source) = approval_source(&state)
                        && let Ok(Some(request)) =
                            self.approvals.native_request(&source, &native_id)
                    {
                        let _ = self.approvals.resolved(&source, request.id);
                    }
                }
                Persist::Event(event) => {
                    match &event {
                        AgentEvent::TurnStarted { .. } => state.info.state = SessionState::Running,
                        AgentEvent::TurnCompleted { .. } | AgentEvent::Idle => {
                            state.info.state = SessionState::Idle
                        }
                        _ => {}
                    }
                    if self.append(&mut state, EventBody::Agent(event)).is_err() {
                        state.pending_lost_bytes = state.pending_lost_bytes.saturating_add(1);
                    }
                }
                Persist::End(exit) => {
                    if let Some(source) = approval_source(&state) {
                        let _ = self.approvals.deactivate_source(&source);
                    }
                    let _ = self.append(
                        &mut state,
                        EventBody::Agent(AgentEvent::Exited { code: exit.code }),
                    );
                    state.info.state = SessionState::Exited;
                    state.info.exit_code = Some(exit.code);
                    state.info.ended_at_ms = exit
                        .observed_at_ms
                        .filter(|at| *at >= state.info.created_at_ms);
                    state.agent_control = None;
                    clear_control(&mut state);
                    if lock(&self.store).is_ok_and(|store| store.finished(&state.info).is_err()) {
                        state.info.recording_warning =
                            Some("Native end metadata could not be committed".into());
                    }
                    break;
                }
            }
        }
    }
}
