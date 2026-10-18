//! The gateway holding a current host connection drains durable primary claims.
//! Browser lifetimes, Redis hints and request retries never own native execution.
use super::*;
use crate::{ApprovalDelivery, ControllerScope};
use qgenie_remote_proto::{
    ApplicationOutcome, ErrorCode,
    wire::{HostContext, Timestamp},
};
use std::sync::Arc;

impl Hub {
    pub(in super::super) async fn deliver_approvals(self: &Arc<Self>) -> Result<(), Error> {
        if !self.approval_status_enabled {
            return Ok(());
        }
        let machines = self
            .peers
            .lock()
            .unwrap()
            .hosts
            .values()
            .filter(|host| {
                host.native && host.approvals && !*host.stop.borrow() && host.gate.budget().is_ok()
            })
            .map(|host| host.actor.machine)
            .collect::<Vec<_>>();
        let scopes = self
            .gateway
            .store
            .due_approval_scopes(&self.node, &machines, 16)
            .await?;
        let mut jobs = tokio::task::JoinSet::new();
        for scope in scopes {
            let hub = self.clone();
            jobs.spawn(async move {
                let _ = hub.deliver_one_approval(scope).await;
            });
        }
        while jobs.join_next().await.is_some() {}
        Ok(())
    }
    async fn deliver_one_approval(&self, scope: ControllerScope) -> Result<(), Error> {
        let host = self
            .peers
            .lock()
            .unwrap()
            .hosts
            .get(&scope.machine)
            .cloned()
            .ok_or(Error::Unavailable)?;
        if !host.approvals
            || host.actor.owner != scope.owner
            || host.generation != scope.host_generation
            || host.id != scope.host_connection
            || *host.stop.borrow()
        {
            return Err(Error::Forbidden);
        }
        self.gateway
            .identity
            .as_ref()
            .ok_or(Error::Configuration)?
            .assert_directory(scope.owner)
            .await?;
        let (budget, _) = host.gate.budget()?;
        let mut access = host.grants.lock().unwrap().current()?;
        access.remaining_ms = access.remaining_ms.min(budget);
        let budget = access.remaining_ms.min(7500);
        if budget < 100 {
            return Err(Error::Unavailable);
        }
        let context = HostContext {
            actor: scope.owner,
            client: host.id,
            host_generation: host.generation,
            expires_at: Timestamp::from_unix_millis(
                super::super::now_ms()?.saturating_add(u64::from(budget)),
            )
            .map_err(|_| Error::Unavailable)?,
            access: Some(access),
        };
        let deadline = tokio::time::Instant::now() + Duration::from_millis(u64::from(budget));
        let deliveries = self.gateway.store.claim_approvals(&scope, 1).await?;
        for delivery in deliveries {
            let started = std::time::Instant::now();
            let result = self
                .native_rpc(
                    &host,
                    &context,
                    scope.session,
                    Message::ApprovalCommitted(delivery.winner.clone()),
                    deadline,
                )
                .await;
            let accepted = self.accept_approval_result(&scope, &delivery, result).await;
            self.observations.delivery(started.elapsed());
            match accepted {
                Ok(true) => {
                    let _ = self.approval_notify(&scope, delivery.request.id).await;
                }
                _ => {
                    let _ = self.gateway.store.retry_approval(&scope, &delivery).await;
                }
            }
        }
        Ok(())
    }
    async fn accept_approval_result(
        &self,
        scope: &ControllerScope,
        delivery: &ApprovalDelivery,
        result: Result<Envelope, Error>,
    ) -> Result<bool, Error> {
        let (outcome, reason) = match result?.message {
            Message::ApprovalApplication {
                approval,
                decision,
                outcome,
                reason,
            } if approval == delivery.request.id && decision == delivery.winner.id => {
                (outcome, reason)
            }
            Message::Error(error) if error.code == ErrorCode::AlreadyResolved => (
                ApplicationOutcome::OutcomeUnknown,
                Some(
                    "Native journal has an incompatible prior answer; no new answer was sent"
                        .into(),
                ),
            ),
            _ => return Ok(false),
        };
        self.gateway
            .store
            .acknowledge_approval(scope, delivery, outcome, reason.as_deref())
            .await?;
        Ok(true)
    }
}
