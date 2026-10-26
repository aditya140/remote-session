use super::{Error, NativeAccess, NativeClient};
use qgenie_remote_proto::wire::{Envelope, Message, Session};

// The native owner admits 64 live processes. Allow some turnover during traversal,
// while bounding both retained metadata and reads under the original access grant.
const MAX_ACTIVE_TRAVERSAL: usize = 128;
const MAX_ACTIVE_METADATA_BYTES: usize = 8 * 1024 * 1024;

impl NativeClient {
    pub(crate) async fn active_session_pages(
        &self,
        access: &NativeAccess,
    ) -> Result<Vec<Vec<Session>>, Error> {
        let mut pages = Vec::new();
        let mut after = None;
        let mut count = 0;
        let mut bytes = 0;
        loop {
            let mut request = Envelope::new(Message::SessionListPage {
                after,
                limit: 128,
                active_only: true,
            });
            request.machine = Some(self.scope().machine);
            let reply = self
                .read(access, self.scope().owner, access.connection(), request)
                .await?;
            let (sessions, next) = match reply.message {
                Message::SessionPage { sessions, next } => (sessions, next),
                Message::Error(error) => return Err(Error::Rejected(error)),
                _ => return Err(Error::Protocol),
            };
            count += sessions.len();
            bytes += serde_json::to_vec(&sessions)
                .map_err(|_| Error::Protocol)?
                .len();
            if count > MAX_ACTIVE_TRAVERSAL
                || bytes > MAX_ACTIVE_METADATA_BYTES
                || !sessions.iter().all(|session| {
                    session.owner == self.scope().owner
                        && session.machine == self.scope().machine
                        && after.is_none_or(|id| session.id > id)
                })
                || !sessions.windows(2).all(|pair| pair[0].id < pair[1].id)
                || next.is_some_and(|id| sessions.last().is_none_or(|session| session.id != id))
            {
                return Err(Error::Protocol);
            }
            pages.push(sessions);
            after = next;
            if after.is_none() {
                return Ok(pages);
            }
        }
    }
}

#[cfg(test)]
mod tests;
