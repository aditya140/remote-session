//! Native acknowledgments and the primary proposal must both agree before an
//! access nonce becomes routable. Original nonces survive every routing queue.
use super::{authority, hub::Hub};
use crate::{AuditAction, AuditEvent, AuditMetadata, Error, MachinePrincipal, Store};
use qgenie_remote_proto::{
    ChallengeId, ClientId, Counter, MachineId, MessageId, UserId,
    wire::{Envelope, HostAccess, Message},
};
use qgenie_remote_transport::ws::{Frame, Priority};
use sqlx::Row;
use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicU64},
    time::Duration,
};

struct Grant {
    access: HostAccess,
    message: MessageId,
    deadline: Duration,
    acknowledged: bool,
    sequence: u64,
}
#[derive(Default)]
pub(super) struct Grants {
    values: HashMap<ChallengeId, Grant>,
    current: Option<ChallengeId>,
    sequence: u64,
}
fn now() -> Result<Duration, Error> {
    qgenie_remote_transport::elapsed::now().map_err(|_| Error::Unavailable)
}
impl Grants {
    fn prune(&mut self, now: Duration) {
        self.values.retain(|_, grant| grant.deadline > now);
        if self
            .current
            .is_some_and(|nonce| !self.values.contains_key(&nonce))
        {
            self.current = None;
        }
    }
    pub fn current(&mut self) -> Result<HostAccess, Error> {
        self.prune(now()?);
        self.current
            .and_then(|nonce| self.values.get(&nonce))
            .filter(|grant| grant.acknowledged)
            .map(|grant| grant.access.clone())
            .ok_or(Error::Unavailable)
    }
    pub fn accepts(&mut self, access: &HostAccess) -> Result<(), Error> {
        access.validate().map_err(|_| Error::Invalid)?;
        self.prune(now()?);
        if self.values.get(&access.nonce).is_some_and(|grant| {
            let mut bounded = grant.access.clone();
            bounded.remaining_ms = access.remaining_ms;
            grant.acknowledged
                && access.remaining_ms <= grant.access.remaining_ms
                && bounded == *access
        }) {
            Ok(())
        } else {
            Err(Error::Unavailable)
        }
    }
}
impl Hub {
    pub async fn native_activation(
        &self,
        actor: &MachinePrincipal,
        generation: Counter,
        envelope: &Envelope,
    ) -> Result<(), Error> {
        if !self.native_enabled
            || envelope.machine != Some(actor.machine)
            || envelope.session.is_some()
        {
            return Err(Error::Forbidden);
        }
        let host = self
            .peers
            .lock()
            .unwrap()
            .hosts
            .get(&actor.machine)
            .cloned()
            .ok_or(Error::NotFound)?;
        if !host.native
            || host.actor.owner != actor.owner
            || host.generation != generation
            || *host.stop.borrow()
        {
            return Err(Error::Forbidden);
        }
        authority::check(
            self,
            &authority::Actor::Host(actor.clone(), Arc::new(AtomicU64::new(generation.0))),
            &host.expires,
            &host.gate,
        )
        .await?;
        let mut response = match &envelope.message {
            Message::HostAccessChallenge {
                nonce,
                host_generation,
                policy_revision,
                max_remaining_ms,
            } => {
                if *host_generation != generation || envelope.re.is_some() {
                    return Err(Error::Forbidden);
                }
                let (budget, deadline) = host.gate.budget()?;
                let access = HostAccess {
                    connection: host.id,
                    host_generation: generation,
                    nonce: *nonce,
                    remaining_ms: budget.min(*max_remaining_ms),
                    policy_revision: *policy_revision,
                };
                access.validate().map_err(|_| Error::Invalid)?;
                let mut response = Envelope::new(Message::HostAccessGranted {
                    access: access.clone(),
                });
                response.re = Some(envelope.id);
                let mut grants = host.grants.lock().unwrap();
                grants.prune(now()?);
                if grants.values.contains_key(nonce) {
                    return Err(Error::Conflict);
                }
                if grants.values.len() >= 4 {
                    return Err(Error::Overloaded);
                }
                grants.sequence = grants.sequence.checked_add(1).ok_or(Error::Overloaded)?;
                let sequence = grants.sequence;
                grants.values.insert(
                    *nonce,
                    Grant {
                        access,
                        message: response.id,
                        deadline,
                        acknowledged: false,
                        sequence,
                    },
                );
                response
            }
            Message::HostActivated {
                nonce,
                host_generation,
            } => {
                if *host_generation != generation {
                    return Err(Error::Forbidden);
                }
                let (persist, repeated) = {
                    let mut grants = host.grants.lock().unwrap();
                    grants.prune(now()?);
                    let proposed = grants.values.get(nonce).ok_or(Error::Unavailable)?;
                    if envelope.re != Some(proposed.message) {
                        return Err(Error::Forbidden);
                    }
                    let newest = grants
                        .current
                        .and_then(|current| grants.values.get(&current))
                        .filter(|current| current.sequence > proposed.sequence)
                        .unwrap_or(proposed);
                    (newest.access.clone(), proposed.acknowledged)
                };
                if !repeated {
                    self.gateway
                        .store
                        .confirm_native_access(actor, generation, host.id, &persist)
                        .await?;
                    let mut grants = host.grants.lock().unwrap();
                    grants.prune(now()?);
                    let proposed = grants.values.get_mut(nonce).ok_or(Error::Unavailable)?;
                    proposed.acknowledged = true;
                    let sequence = proposed.sequence;
                    if grants
                        .current
                        .and_then(|current| grants.values.get(&current))
                        .is_none_or(|current| current.sequence < sequence)
                    {
                        grants.current = Some(*nonce);
                    }
                }
                let mut response = Envelope::new(Message::HostActivationConfirmed {
                    nonce: *nonce,
                    host_generation: generation,
                });
                response.re = Some(envelope.id);
                response
            }
            _ => return Err(Error::Invalid),
        };
        let confirmed = matches!(response.message, Message::HostActivationConfirmed { .. });
        response.machine = Some(actor.machine);
        Self::deliver(
            &host.sender,
            &host.stop,
            Frame::Text(Box::new(response)),
            Priority::Control,
        )?;
        if confirmed {
            let _ = self.publish_hint(&host).await;
        }
        Ok(())
    }
}
impl Store {
    pub(crate) async fn confirm_native_access(
        &self,
        actor: &MachinePrincipal,
        generation: Counter,
        connection: ClientId,
        access: &HostAccess,
    ) -> Result<(), Error> {
        access.validate().map_err(|_| Error::Invalid)?;
        if access.connection != connection || access.host_generation != generation {
            return Err(Error::Forbidden);
        }
        let mut tx = self.transaction().await?;
        Self::lock_access(&mut tx).await?;
        let enabled: bool = sqlx::query_scalar("SELECT NOT(m.disabled OR u.disabled OR k.revoked) FROM qgr.machines m JOIN qgr.users u ON u.id=m.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id WHERE m.id=$1 AND m.owner=$2 AND m.active_key=$3 FOR SHARE OF m,u,k")
            .bind(actor.machine.to_string()).bind(actor.owner.to_string()).bind(actor.key.to_string()).fetch_optional(&mut *tx).await.map_err(|_| Error::Database)?.unwrap_or(false);
        if !enabled {
            return Err(Error::Forbidden);
        }
        let route = sqlx::query("SELECT generation::text,connection,activated FROM qgr.machine_routes WHERE machine=$1 FOR UPDATE")
            .bind(actor.machine.to_string()).fetch_optional(&mut *tx).await.map_err(|_| Error::Database)?.ok_or(Error::Forbidden)?;
        if route.get::<String, _>("generation") != generation.0.to_string()
            || route.get::<String, _>("connection") != connection.to_string()
        {
            return Err(Error::Forbidden);
        }
        sqlx::query(
            "UPDATE qgr.machine_routes SET activated=true,native_access=$2 WHERE machine=$1",
        )
        .bind(actor.machine.to_string())
        .bind(serde_json::to_value(access).map_err(|_| Error::Invalid)?)
        .execute(&mut *tx)
        .await
        .map_err(|_| Error::Database)?;
        if !route.get::<bool, _>("activated") {
            Self::append_audit(
                &mut tx,
                &AuditEvent {
                    actor: actor.owner,
                    action: AuditAction::HostActivated,
                    machine: Some(actor.machine),
                    session: None,
                    metadata: AuditMetadata {
                        generation: Some(generation),
                        ..Default::default()
                    },
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub(super) async fn routed_native_access(
        &self,
        owner: UserId,
        machine: MachineId,
        node: &str,
        generation: Counter,
    ) -> Result<Option<HostAccess>, Error> {
        let row = sqlx::query("SELECT r.native_access FROM qgr.machine_routes r JOIN qgr.machines m ON m.id=r.machine JOIN qgr.users u ON u.id=m.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id CROSS JOIN qgr.access_state a WHERE m.id=$1 AND m.owner=$2 AND r.node=$3 AND r.generation=$4::numeric AND NOT(m.disabled OR u.disabled OR k.revoked OR a.disabled)")
            .bind(machine.to_string()).bind(owner.to_string()).bind(node).bind(generation.0.to_string()).fetch_optional(&self.pool).await.map_err(|_| Error::Database)?.ok_or(Error::Forbidden)?;
        row.get::<Option<serde_json::Value>, _>("native_access")
            .map(|value| {
                let access: HostAccess =
                    serde_json::from_value(value).map_err(|_| Error::Database)?;
                access.validate().map_err(|_| Error::Database)?;
                if access.host_generation != generation {
                    return Err(Error::Forbidden);
                }
                Ok(access)
            })
            .transpose()
    }
}

#[cfg(test)]
pub(crate) mod tests;
