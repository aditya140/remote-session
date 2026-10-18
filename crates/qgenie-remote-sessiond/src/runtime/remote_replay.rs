//! Native replay/checkpoint reads release the authority mutex over SQLite work.
use super::*;
use qgenie_remote_proto::{
    ipc::{DispatchFrame, RemoteContext},
    wire::{self, Message},
};
#[cfg(test)]
mod tests;

fn scope(state: &Session, context: &RemoteContext) -> Result<(), RemoteError> {
    if state.scope.as_ref() != Some(&context.scope) {
        return Err(RemoteError::rejected(
            ErrorCode::Forbidden,
            "native replay scope differs",
        ));
    }
    authorize(
        Action::Observe,
        Facts {
            owner: context.actor == context.scope.owner,
            access_fresh: true,
            within_roots: false,
            ..local_facts()
        },
    )
}
impl Runtime {
    pub(super) fn remote_head(
        &self,
        context: &RemoteContext,
        session: SessionId,
    ) -> Result<DispatchFrame, RemoteError> {
        let fence = lock(&self.authority)?.fence(context)?;
        let owned = self.session(session)?;
        let state = lock(&owned)?;
        scope(&state, context)?;
        fence.check()?;
        let (floor, _, _) = lock(&self.store)?
            .replay(session, state.info.head)
            .map_err(storage_error)?;
        fence.check()?;
        Ok(DispatchFrame::HeadRead {
            session,
            floor,
            head: state.info.head,
            last_input: Counter(state.input_seq),
        })
    }
    pub(super) fn remote_replay(
        &self,
        context: &RemoteContext,
        request: wire::Envelope,
    ) -> Result<DispatchFrame, RemoteError> {
        request.validate()?;
        if request.machine != Some(context.scope.machine) || request.re.is_some() {
            return Err(RemoteError::rejected(
                ErrorCode::Forbidden,
                "native replay target differs",
            ));
        }
        let session = request.session.ok_or_else(|| {
            RemoteError::rejected(ErrorCode::InvalidFrame, "missing native replay session")
        })?;
        let Message::Attach { after } = request.message else {
            return Err(RemoteError::rejected(
                ErrorCode::UnsupportedCapability,
                "native replay operation unsupported",
            ));
        };
        let fence = lock(&self.authority)?.fence(context)?;
        let owned = self.session(session)?;
        let mut state = lock(&owned)?;
        scope(&state, context)?;
        fence.check()?;
        self.commit_gap(&mut state)?;
        let (floor, events, more) = lock(&self.store)?
            .replay(session, after)
            .map_err(storage_error)?;
        let truncated = matches!(
            state.info.state,
            SessionState::Lost | SessionState::OutcomeUnknown
        ) || after.0.checked_add(1).is_some_and(|next| floor.0 > next)
            || after > state.info.head
            || (after < state.info.head
                && events
                    .first()
                    .is_none_or(|event| after.0.checked_add(1) != Some(event.seq.0)));
        let events = events
            .into_iter()
            .map(|event| public_event(&event))
            .collect::<Result<Vec<_>, RemoteError>>()?;
        fence.check()?;
        let mut reply = wire::Envelope::new(Message::Replay {
            floor,
            head: state.info.head,
            events,
            more,
            history_truncated: truncated,
        });
        reply.re = Some(request.id);
        reply.machine = request.machine;
        reply.session = Some(session);
        reply.encode()?;
        Ok(DispatchFrame::RemoteReply(Box::new(reply)))
    }
    pub(super) fn remote_snapshot(
        &self,
        context: &RemoteContext,
        session: SessionId,
        epoch: Option<MessageId>,
        page: u16,
    ) -> Result<DispatchFrame, RemoteError> {
        let fence = lock(&self.authority)?.fence(context)?;
        let _permit = self.snapshot_work.try_acquire().map_err(|_| {
            RemoteError::rejected(ErrorCode::Overloaded, "native checkpoint work unavailable")
        })?;
        let owned = self.session(session)?;
        let mut state = lock(&owned)?;
        scope(&state, context)?;
        fence.check()?;
        let snapshot = if let Some(epoch) = epoch {
            lock(&self.snapshots)?.page(session, context.client, epoch, page)?
        } else {
            if page != 0 {
                return Err(RemoteError::rejected(
                    ErrorCode::InvalidFrame,
                    "native capture must begin at page zero",
                ));
            }
            self.commit_gap(&mut state)?;
            let snapshot = self.snapshot(&state)?;
            fence.check()?;
            lock(&self.snapshots)?.capture(session, context.client, state.info.head, snapshot)?
        };
        fence.check()?;
        Ok(DispatchFrame::SnapshotRead {
            session,
            snapshot: wire::SnapshotPage {
                epoch: snapshot.epoch.to_string(),
                seq: snapshot.seq,
                page: snapshot.page,
                total_pages: snapshot.total_pages,
                rows: snapshot.rows,
                cols: snapshot.cols,
                format: snapshot.format,
                data: snapshot.data,
            },
        })
    }
}
