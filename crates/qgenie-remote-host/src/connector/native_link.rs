//! WSS/native activation exchange. Pending grants become usable only after both
//! native persistence and the gateway's current-proposal confirmation.
use super::send;
use crate::{Error, NativeAccess, NativeChallenge, NativeClient};
use qgenie_remote_proto::{
    ChallengeId, ClientId, Counter, ErrorCode, MessageId, RemoteError,
    wire::{Envelope, HostContext, Message},
};
use qgenie_remote_transport::ws::{Connection, Frame, Priority};
use std::{collections::HashMap, time::Duration};
use tokio::time::Instant;

enum Pending {
    Grant {
        challenge: NativeChallenge,
        request: MessageId,
        renewal: bool,
        deadline: Instant,
    },
    Confirmation {
        access: NativeAccess,
        request: MessageId,
        deadline: Instant,
    },
}
pub(super) struct NativeLink {
    changes: bool,
    agents: bool,
    approvals: bool,
    client: NativeClient,
    connection: ClientId,
    generation: Counter,
    grants: HashMap<ChallengeId, NativeAccess>,
    current: Option<ChallengeId>,
    pending: Option<Pending>,
    next: Instant,
}
impl NativeLink {
    pub async fn connect(
        client: NativeClient,
        connection: ClientId,
        generation: Counter,
        socket: &mut Connection,
        approvals: bool,
        agents: bool,
        changes: bool,
    ) -> Result<Self, Error> {
        let mut link = Self {
            changes,
            agents,
            approvals,
            client,
            connection,
            generation,
            grants: HashMap::new(),
            current: None,
            pending: None,
            next: Instant::now(),
        };
        tokio::time::timeout(Duration::from_secs(8), async {
            link.begin(socket, false).await?;
            while link.current.is_none() {
                let frame = socket.recv().await.ok_or(Error::Unavailable)?;
                let Frame::Text(envelope) = &frame.frame else {
                    return Err(Error::Protocol);
                };
                link.frame(socket, envelope).await?;
            }
            Ok(())
        })
        .await
        .map_err(|_| Error::Unavailable)??;
        Ok(link)
    }
    pub fn deadline(&self) -> Instant {
        match &self.pending {
            Some(Pending::Grant { deadline, .. } | Pending::Confirmation { deadline, .. }) => {
                *deadline
            }
            None => self.next,
        }
    }
    pub async fn tick(&mut self, socket: &Connection) -> Result<(), Error> {
        if self.pending.is_some() {
            return Err(Error::Unavailable);
        }
        self.begin(socket, true).await
    }
    async fn begin(&mut self, socket: &Connection, renewal: bool) -> Result<(), Error> {
        let deadline = Instant::now() + Duration::from_secs(8);
        let challenge = self
            .client
            .challenge(self.connection, self.generation)
            .await?;
        let mut request = Envelope::new(Message::HostAccessChallenge {
            nonce: challenge.nonce(),
            host_generation: self.generation,
            policy_revision: challenge.policy_revision(),
            max_remaining_ms: challenge.max_remaining_ms(),
        });
        request.machine = Some(self.client.scope().machine);
        self.pending = Some(Pending::Grant {
            challenge,
            request: request.id,
            renewal,
            deadline,
        });
        send(socket, request, Priority::Control).await
    }
    pub async fn frame(&mut self, socket: &Connection, envelope: &Envelope) -> Result<bool, Error> {
        if envelope.machine != Some(self.client.scope().machine) || envelope.session.is_some() {
            return Err(Error::Protocol);
        }
        let pending = self.pending.take().ok_or(Error::Protocol)?;
        match (&envelope.message, pending) {
            (
                Message::HostAccessGranted { access },
                Pending::Grant {
                    challenge,
                    request,
                    renewal,
                    deadline,
                },
            ) => {
                if Instant::now() >= deadline
                    || envelope.re != Some(request)
                    || access.connection != self.connection
                    || access.host_generation != self.generation
                    || access.nonce != challenge.nonce()
                    || access.policy_revision != challenge.policy_revision()
                {
                    return Err(Error::Protocol);
                }
                access.validate().map_err(Error::Rejected)?;
                let native = self
                    .client
                    .activate(&challenge, access.remaining_ms, renewal)
                    .await?;
                let mut ack = Envelope::new(Message::HostActivated {
                    nonce: native.nonce(),
                    host_generation: self.generation,
                });
                ack.re = Some(envelope.id);
                ack.machine = envelope.machine;
                self.pending = Some(Pending::Confirmation {
                    access: native,
                    request: ack.id,
                    deadline,
                });
                send(socket, ack, Priority::Control).await?;
                Ok(false)
            }
            (
                Message::HostActivationConfirmed {
                    nonce,
                    host_generation,
                },
                Pending::Confirmation {
                    access,
                    request,
                    deadline,
                },
            ) => {
                if Instant::now() >= deadline
                    || envelope.re != Some(request)
                    || *nonce != access.nonce()
                    || *host_generation != self.generation
                    || !access.is_fresh()
                {
                    return Err(stale());
                }
                let left = access.remaining()?;
                if left < Duration::from_millis(200) {
                    return Err(stale());
                }
                self.next = Instant::now() + (left / 2).min(Duration::from_secs(3));
                self.grants.retain(|_, grant| grant.is_fresh());
                if self.grants.len() >= 4 && !self.grants.contains_key(nonce) {
                    return Err(Error::Unavailable);
                }
                self.grants.insert(*nonce, access);
                self.current = Some(*nonce);
                Ok(true)
            }
            _ => Err(Error::Protocol),
        }
    }
    pub fn access(&self, context: &HostContext) -> Result<(NativeClient, NativeAccess), Error> {
        let bounded = context.access.as_ref().ok_or_else(stale)?;
        if context.host_generation != self.generation
            || bounded.host_generation != self.generation
            || bounded.connection != self.connection
        {
            return Err(stale());
        }
        let access = self.grants.get(&bounded.nonce).ok_or_else(stale)?;
        if bounded.policy_revision != access.policy_revision() {
            return Err(stale());
        }
        Ok((self.client.clone(), access.restrict(bounded.remaining_ms)?))
    }
    pub fn recording_access(&self) -> Option<(NativeClient, NativeAccess)> {
        let access = self.current.and_then(|nonce| self.grants.get(&nonce))?;
        (access.remaining().ok()? > Duration::from_secs(1))
            .then(|| (self.client.clone(), access.clone()))
    }
    pub fn changes_available(&self) -> bool {
        self.changes
    }
    pub async fn inventory(&self) -> Result<Envelope, Error> {
        let access = self
            .current
            .and_then(|nonce| self.grants.get(&nonce))
            .ok_or_else(stale)?;
        let mut message = Envelope::new(Message::SessionList);
        message.machine = Some(self.client.scope().machine);
        let mut response = self
            .client
            .read(access, self.client.scope().owner, self.connection, message)
            .await?;
        response.re = None;
        Ok(response)
    }
    pub fn approval_access(&self) -> Option<(NativeClient, NativeAccess)> {
        if self.approvals {
            self.recording_access()
        } else {
            None
        }
    }
    pub async fn agent_inventory(&self) -> Result<Option<Envelope>, Error> {
        if !self.agents {
            return Ok(None);
        }
        let access = self
            .current
            .and_then(|nonce| self.grants.get(&nonce))
            .ok_or_else(stale)?;
        let agents = self.client.agent_inventory(access).await?;
        let mut envelope = Envelope::new(Message::Inventory { agents });
        envelope.machine = Some(self.client.scope().machine);
        Ok(Some(envelope))
    }
}
fn stale() -> Error {
    Error::Rejected(RemoteError::rejected(
        ErrorCode::AccessStale,
        "original native access identity is stale",
    ))
}
