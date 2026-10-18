//! Native controller floors and original deadlines for the public gateway lease path.
use super::*;
use qgenie_remote_proto::ipc::{ControlActivation, DispatchFrame, RemoteContext};

#[derive(Clone, Copy)]
pub(super) struct RemoteController {
    pub host: Counter,
    pub connection: ClientId,
}
impl Runtime {
    pub(super) fn remote_control_state(
        &self,
        context: &RemoteContext,
        session: SessionId,
    ) -> Result<DispatchFrame, RemoteError> {
        let authority = lock(&self.authority)?;
        authority.validate(context, Instant::now())?;
        let owned = self.session(session)?;
        let mut state = lock(&owned)?;
        check_scope(&state, context)?;
        expire(&mut state);
        let remaining = remaining(&state)?;
        if remaining == 0 {
            clear_control(&mut state);
        }
        authority.validate(context, Instant::now())?;
        Ok(DispatchFrame::ControlState {
            session,
            floor: Counter(state.generation),
            holder: state.info.controller.as_ref().map(|lease| lease.holder),
            remaining_ms: remaining,
            last_input: state
                .info
                .controller
                .as_ref()
                .map(|lease| lease.last_input)
                .unwrap_or(Counter(0)),
        })
    }
    pub(super) fn activate_remote_control(
        &self,
        request: ControlActivation,
    ) -> Result<DispatchFrame, RemoteError> {
        let context = &request.context;
        if context.client != request.holder
            || context.controller_generation != Some(request.generation)
            || request.generation.0 == 0
        {
            return Err(stale());
        }
        let authority = lock(&self.authority)?;
        let (deadline, elapsed_deadline) =
            authority.control_deadlines(context, request.remaining_lease_ms)?;
        let owned = self.session(request.session)?;
        let mut state = lock(&owned)?;
        check_scope(&state, context)?;
        ensure_running(&state)?;
        expire(&mut state);
        if request.generation.0 < state.generation {
            return Err(stale());
        }
        if request.generation.0 == state.generation {
            if state.remote_controller.is_none()
                || !state.info.controller.as_ref().is_some_and(|lease| {
                    lease.holder == request.holder && lease.generation == request.generation
                })
            {
                return Err(stale());
            }
            // A live in-process duplicate can reconcile an ACK. Restart/expiry
            // clears this state, so an old generation cannot regain its HMAC key.
            if request.renewal && elapsed_deadline > state.deadline_elapsed.ok_or_else(stale)? {
                // Keep one original clock pair across suspend intervals.
                state.deadline = Some(deadline);
                state.deadline_elapsed = Some(elapsed_deadline);
            }
        } else {
            if request.renewal {
                return Err(stale());
            }
            if state
                .info
                .controller
                .as_ref()
                .is_some_and(|lease| lease.holder != request.holder)
                && !request.takeover
            {
                return Err(RemoteError::rejected(
                    ErrorCode::ControlHeld,
                    "native controller exists; confirm takeover",
                ));
            }
            let persisted = lock(&self.store).and_then(|store| {
                store
                    .generation(state.info.id, request.generation.0)
                    .map_err(storage_error)
            });
            state.generation = request.generation.0;
            clear_control(&mut state);
            persisted?;
            state.input_seq = 0;
            // Old authority is already fenced even if persistence consumed the
            // new grant. Never acknowledge a fresh lease from receipt time.
            authority.validate(context, Instant::now())?;
            if deadline <= Instant::now() || elapsed_deadline <= clock::elapsed()? {
                return Err(stale());
            }
            state.deadline = Some(deadline);
            state.deadline_elapsed = Some(elapsed_deadline);
            state.info.controller = Some(Lease {
                holder: request.holder,
                generation: request.generation,
                expires_at_ms: Counter(0),
                last_input: Counter(0),
            });
        }
        if let Err(error) = authority.validate(context, Instant::now()) {
            clear_control(&mut state);
            return Err(error);
        }
        expire(&mut state);
        let left = remaining(&state)?;
        if left == 0 {
            clear_control(&mut state);
            return Err(stale());
        }
        let lease = state.info.controller.as_mut().ok_or_else(stale)?;
        lease.expires_at_ms = Counter(now_ms().saturating_add(u64::from(left)));
        let last_input = lease.last_input;
        state.remote_controller = Some(RemoteController {
            host: context.host_generation,
            connection: context.connection,
        });
        Ok(DispatchFrame::ControlActivated {
            session: request.session,
            holder: request.holder,
            generation: request.generation,
            remaining_ms: left,
            last_input,
        })
    }
    pub(super) fn release_remote_control(
        &self,
        context: &RemoteContext,
        session: SessionId,
        generation: Counter,
    ) -> Result<DispatchFrame, RemoteError> {
        let authority = lock(&self.authority)?;
        authority.validate(context, Instant::now())?;
        if context.controller_generation != Some(generation) {
            return Err(stale());
        }
        let owned = self.session(session)?;
        let mut state = lock(&owned)?;
        check_scope(&state, context)?;
        expire(&mut state);
        if state.generation != generation.0 {
            return Err(stale());
        }
        if let Some(lease) = &state.info.controller {
            let origin = state.remote_controller.ok_or_else(stale)?;
            if lease.holder != context.client
                || origin.host != context.host_generation
                || origin.connection != context.connection
            {
                return Err(stale());
            }
        }
        authority.validate(context, Instant::now())?;
        clear_control(&mut state);
        Ok(DispatchFrame::ControlDeactivated {
            session,
            generation,
        })
    }
}
fn check_scope(state: &Session, context: &RemoteContext) -> Result<(), RemoteError> {
    if state.scope.as_ref() != Some(&context.scope) {
        return Err(RemoteError::rejected(
            ErrorCode::Forbidden,
            "native session ownership scope differs",
        ));
    }
    authorize(
        Action::Control,
        Facts {
            owner: context.actor == context.scope.owner,
            access_fresh: true,
            within_roots: false,
            ..local_facts()
        },
    )
}
fn remaining(state: &Session) -> Result<u32, RemoteError> {
    if state.info.controller.is_none() {
        return Ok(0);
    }
    let left = state
        .deadline
        .ok_or_else(stale)?
        .saturating_duration_since(Instant::now())
        .min(
            state
                .deadline_elapsed
                .ok_or_else(stale)?
                .saturating_sub(clock::elapsed()?),
        );
    Ok(u32::try_from(left.as_millis())
        .unwrap_or(15_000)
        .min(15_000))
}
fn stale() -> RemoteError {
    RemoteError::rejected(
        ErrorCode::StaleGeneration,
        "native controller generation or lifetime is stale",
    )
}

#[cfg(test)]
mod tests;
