//! Approval delivery uses owner access, independently of the controller lease.
use super::*;
use crate::NativeApprovalReservation;
use qgenie_remote_proto::{
    ipc::{DispatchFrame, RemoteContext},
    wire::{ApprovalRequest, ApprovalWinner, Message, Timestamp},
};

pub(super) fn approval_source(state: &Session) -> Option<crate::ApprovalSource> {
    Some(crate::ApprovalSource {
        scope: state.scope.clone()?,
        session: state.info.id,
        instance: state.info.agent.as_ref()?.native_instance,
    })
}
impl Runtime {
    pub(super) fn record_native_approval(
        &self,
        state: &Session,
        approval: NativeApproval,
    ) -> Result<(), RemoteError> {
        let Some(source) = approval_source(state) else {
            return Ok(());
        };
        if state
            .info
            .agent
            .as_ref()
            .and_then(|agent| agent.native_thread.as_deref())
            != Some(approval.thread.as_str())
        {
            return Err(RemoteError::rejected(
                ErrorCode::Forbidden,
                "approval thread differs from owned session",
            ));
        }
        let hash = fingerprint(
            &serde_json::json!({"source":source,"native_id":approval.native_id,"thread":approval.thread,"item":approval.item,"kind":approval.kind,"summary":approval.summary,"decisions":approval.allowed_decisions}),
        )?;
        let request = match self
            .approvals
            .native_request(&source, &approval.native_id)?
        {
            Some(request) => {
                if request.request_hash != hash {
                    return Err(RemoteError::rejected(
                        ErrorCode::AlreadyResolved,
                        "native approval ID changed its original body",
                    ));
                }
                request
            }
            None => {
                let mut summary = approval.summary;
                if summary.len() > 8192 {
                    let mut end = 8140;
                    while !summary.is_char_boundary(end) {
                        end -= 1;
                    }
                    summary.truncate(end);
                    summary.push_str(" [summary truncated]");
                }
                ApprovalRequest {
                    id: ApprovalId::new(),
                    session: source.session,
                    owner: source.scope.owner,
                    native_id: approval.native_id,
                    kind: approval.kind,
                    summary,
                    request_hash: hash,
                    policy_revision: Counter(1),
                    requested_at: Timestamp::from_unix_millis(now_ms())?,
                    expires_at: None,
                    allowed_decisions: approval.allowed_decisions,
                }
            }
        };
        self.approvals.record(&source, &request)
    }

    pub(in super::super) fn native_approval_rpc(
        &self,
        context: &RemoteContext,
        request: wire::Envelope,
    ) -> Result<DispatchFrame, RemoteError> {
        request.validate()?;
        if request.machine != Some(context.scope.machine)
            || request.re.is_some()
            || context.controller_generation.is_some()
        {
            return Err(RemoteError::rejected(
                ErrorCode::Forbidden,
                "approval authority differs",
            ));
        }
        let fence = lock(&self.authority)?.fence(context)?;
        let body = match request.message {
            Message::ApprovalList => {
                if let Some(session) = request.session {
                    let state = self.session(session)?;
                    if lock(&state)?.scope.as_ref() != Some(&context.scope) {
                        return Err(RemoteError::rejected(
                            ErrorCode::Forbidden,
                            "approval session scope differs",
                        ));
                    }
                }
                Message::Approvals {
                    approvals: self
                        .approvals
                        .pending_for_session(&context.scope, request.session)?,
                }
            }
            Message::ApprovalCommitted(winner) => {
                let session = request.session.ok_or_else(|| {
                    RemoteError::rejected(ErrorCode::InvalidFrame, "missing approval session")
                })?;
                let outcome =
                    self.deliver_native_approval(context, session, &winner, fence.clone())?;
                Message::ApprovalApplication {
                    approval: winner.approval,
                    decision: winner.id,
                    outcome,
                    reason: None,
                }
            }
            _ => {
                return Err(RemoteError::rejected(
                    ErrorCode::UnsupportedCapability,
                    "unsupported native approval operation",
                ));
            }
        };
        fence.check().map_err(|_| {
            RemoteError::unknown("approval reply lost original access; reconcile the same decision")
        })?;
        let mut reply = wire::Envelope::new(body);
        reply.re = Some(request.id);
        reply.machine = request.machine;
        reply.session = request.session;
        Ok(DispatchFrame::RemoteReply(Box::new(reply)))
    }

    fn deliver_native_approval(
        &self,
        context: &RemoteContext,
        session: SessionId,
        winner: &ApprovalWinner,
        fence: authority::Fence,
    ) -> Result<ApplicationOutcome, RemoteError> {
        let owned = self.session(session)?;
        let (source, control, thread, mut recording) = {
            let state = lock(&owned)?;
            if state.scope.as_ref() != Some(&context.scope) {
                return Err(RemoteError::rejected(
                    ErrorCode::Forbidden,
                    "approval owner scope differs",
                ));
            }
            let source = approval_source(&state).ok_or_else(|| {
                RemoteError::rejected(
                    ErrorCode::UnsupportedCapability,
                    "no native approval source",
                )
            })?;
            (
                source,
                state.agent_control.clone(),
                state
                    .info
                    .agent
                    .as_ref()
                    .and_then(|agent| agent.native_thread.clone()),
                state.recording.clone(),
            )
        };
        fence.check()?;
        match self
            .approvals
            .reserve(&source, winner, context.policy_revision, now_ms())?
        {
            NativeApprovalReservation::Known(outcome) => Ok(outcome),
            NativeApprovalReservation::Send(intent) => {
                if let Some(capture) = &mut recording {
                    capture_text(
                        capture,
                        RecordingChannel::ApprovalDecision,
                        &serde_json::to_vec(winner).map_err(storage_error)?,
                    );
                }
                let outcome = if let (Some(control), Some(thread)) = (control, thread) {
                    let request = intent.request();
                    let native = NativeApproval {
                        native_id: request.native_id.clone(),
                        thread,
                        item: String::new(),
                        kind: request.kind.clone(),
                        summary: request.summary.clone(),
                        allowed_decisions: request.allowed_decisions.clone(),
                    };
                    match control.request(
                        ActionRequest::Answer {
                            approval: native,
                            decision: winner.decision,
                        },
                        Arc::new(fence),
                    ) {
                        Ok(outcome) => outcome,
                        Err(error) if matches!(error.outcome, OperationOutcome::Rejected) => {
                            ApplicationOutcome::Cancelled
                        }
                        Err(_) => ApplicationOutcome::OutcomeUnknown,
                    }
                } else {
                    ApplicationOutcome::Cancelled
                };
                intent.complete(outcome.clone(), None)?;
                if let Some(capture) = &mut recording {
                    capture_text(capture,RecordingChannel::ApprovalApplication,&serde_json::to_vec(&serde_json::json!({"approval":winner.approval,"decision":winner.id,"outcome":outcome})).map_err(storage_error)?);
                }
                Ok(outcome)
            }
        }
    }
}
