//! Per-session snapshots avoid a machine-wide pending limit. Four reads share
//! one original grant and run independently of host activation and live input.
use crate::{Error, NativeAccess, NativeClient};
use qgenie_remote_proto::{
    SessionId,
    wire::{Envelope, Message},
};
use qgenie_remote_transport::ws::{Frame, Priority, Sender};
use std::time::Duration;

pub(super) async fn poll(
    sender: Sender,
    client: NativeClient,
    access: NativeAccess,
    rotation: usize,
) -> Result<(), Error> {
    let budget = access.remaining()?.min(Duration::from_secs(8));
    tokio::time::timeout(budget, async {
        let mut request = Envelope::new(Message::SessionList);
        request.machine = Some(client.scope().machine);
        let reply = client
            .read(&access, client.scope().owner, access.connection(), request)
            .await?;
        let Message::Sessions { sessions } = reply.message else {
            return Err(Error::Protocol);
        };
        let mut sessions = sessions
            .into_iter()
            .filter(|session| !session.capabilities.approvals.is_empty())
            .map(|session| session.id)
            .collect::<Vec<_>>();
        if sessions.len() > 64 {
            return Err(Error::Protocol);
        }
        sessions.sort();
        if !sessions.is_empty() {
            let start = rotation % sessions.len();
            sessions.rotate_left(start);
        }
        let mut sessions = sessions.into_iter();
        let mut jobs = tokio::task::JoinSet::new();
        loop {
            while jobs.len() < 4 {
                let Some(session) = sessions.next() else {
                    break;
                };
                jobs.spawn(read_session(
                    sender.clone(),
                    client.clone(),
                    access.clone(),
                    session,
                ));
            }
            match jobs.join_next().await {
                Some(result) => result.map_err(|_| Error::Unavailable)??,
                None => break,
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| Error::Unavailable)?
}
async fn read_session(
    sender: Sender,
    client: NativeClient,
    access: NativeAccess,
    session: SessionId,
) -> Result<(), Error> {
    let mut request = Envelope::new(Message::ApprovalList);
    request.machine = Some(client.scope().machine);
    request.session = Some(session);
    let mut reply = client
        .read(&access, client.scope().owner, access.connection(), request)
        .await?;
    reply.re = None;
    sender
        .try_send(Priority::Event, Frame::Text(Box::new(reply)))
        .map_err(|_| Error::Unavailable)?
        .transmitted()
        .await
        .map_err(|_| Error::Unavailable)
}
