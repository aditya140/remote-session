//! Read-only durable reconciliation never reanswers a native request.
use crate::LiveHandle;
use qgenie_remote_proto::{
    wire::{ApprovalDecision, ApprovalState, ApprovalWinner, Envelope, Message},
    *,
};
#[derive(Debug)]
pub struct ApprovalPage {
    pub states: Vec<ApprovalState>,
    pub next: Option<ApprovalId>,
}
impl LiveHandle {
    pub fn approvals_available(&self) -> bool {
        self.approvals
    }
    /// Commit one explicit choice; application is observed separately. Lost or
    /// cancelled replies require read-only status reconciliation, never a resend.
    pub async fn resolve_approval(
        &self,
        state: &ApprovalState,
        decision: ApprovalDecision,
    ) -> Result<ApprovalWinner, RemoteError> {
        if !self.approvals {
            return Err(RemoteError {
                code: ErrorCode::UnsupportedCapability,
                outcome: OperationOutcome::NotSent,
                message: "gateway did not negotiate durable-approvals-v1".into(),
                retryable: false,
            });
        }
        state.validate().map_err(|mut error| {
            error.outcome = OperationOutcome::NotSent;
            error
        })?;
        if state.cancelled
            || state.winner.is_some()
            || !state.request.allowed_decisions.contains(&decision)
        {
            return Err(RemoteError {
                code: ErrorCode::AlreadyResolved,
                outcome: OperationOutcome::NotSent,
                message: "approval is no longer pending or this decision was not offered".into(),
                retryable: false,
            });
        }
        let mut request = Envelope::new(Message::ApprovalResolve {
            approval: state.request.id,
            request_hash: state.request.request_hash.clone(),
            decision,
        });
        request.machine = Some(state.machine);
        request.session = Some(state.request.session);
        let mut frames = self.submit(request).await?;
        if frames.len() != 1 {
            return Err(RemoteError::unknown(
                "approval commit reply is incomplete; read its status",
            ));
        }
        match frames.remove(0).message {
            Message::ApprovalCommitted(winner)
                if winner.approval == state.request.id
                    && winner.request_hash == state.request.request_hash
                    && winner.policy_revision == state.request.policy_revision
                    && winner.actor == state.request.owner
                    && state.request.allowed_decisions.contains(&winner.decision) =>
            {
                Ok(winner)
            }
            _ => Err(RemoteError::unknown(
                "approval commit identity differs; read its status",
            )),
        }
    }
    pub async fn approval_index(
        &self,
        after: Option<ApprovalId>,
        limit: u16,
    ) -> Result<ApprovalPage, RemoteError> {
        if !self.approval_status || !(1..=200).contains(&limit) {
            return Err(RemoteError {
                code: ErrorCode::UnsupportedCapability,
                outcome: OperationOutcome::NotSent,
                message: "approval-status-v1 and a 1..=200 page limit are required".into(),
                retryable: false,
            });
        }
        let mut frames = self
            .submit(Envelope::new(Message::ApprovalIndex { after, limit }))
            .await?;
        if frames.len() != 1 {
            return Err(RemoteError::unknown(
                "approval index response is incomplete",
            ));
        }
        match frames.remove(0).message {
            Message::ApprovalPage { states, next }
                if states.len() <= usize::from(limit)
                    && states
                        .first()
                        .is_none_or(|state| after.is_none_or(|after| state.request.id > after)) =>
            {
                Ok(ApprovalPage { states, next })
            }
            _ => Err(RemoteError::unknown(
                "approval page cursor or limit differs",
            )),
        }
    }
    pub async fn approval_status(
        &self,
        approval: ApprovalId,
    ) -> Result<ApprovalState, RemoteError> {
        if !self.approval_status {
            return Err(RemoteError {
                code: ErrorCode::UnsupportedCapability,
                outcome: OperationOutcome::NotSent,
                message: "gateway did not negotiate approval-status-v1".into(),
                retryable: false,
            });
        }
        let mut frames = self
            .submit(Envelope::new(Message::ApprovalGet { approval }))
            .await?;
        if frames.len() != 1 {
            return Err(RemoteError::unknown(
                "approval status response is incomplete",
            ));
        }
        match frames.remove(0).message {
            Message::ApprovalStatus(state) if state.request.id == approval => {
                state.validate()?;
                Ok(state)
            }
            _ => Err(RemoteError::unknown("approval status identity differs")),
        }
    }
}
