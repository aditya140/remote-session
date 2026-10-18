//! Fixed native captures retain their original authority and response byte permits.
use super::*;
use qgenie_remote_proto::MessageId;
use qgenie_remote_proto::wire::{Message, SnapshotPage};

pub struct NativeAttachment {
    pub(crate) frames: Vec<Envelope>,
    _work: tokio::sync::OwnedSemaphorePermit,
    _bytes: Vec<tokio::sync::OwnedSemaphorePermit>,
}
impl NativeAttachment {
    pub fn frames(&self) -> &[Envelope] {
        &self.frames
    }
}
fn overloaded() -> Error {
    Error::Rejected(RemoteError::rejected(
        ErrorCode::Overloaded,
        "native replay budget exceeded",
    ))
}
impl NativeAttachment {
    fn push_page(
        &mut self,
        client: &NativeClient,
        request: &Envelope,
        snapshot: SnapshotPage,
    ) -> Result<(), Error> {
        let mut page = Envelope::new(Message::Snapshot { snapshot });
        page.re = Some(request.id);
        page.machine = request.machine;
        page.session = request.session;
        self.push(client, page)
    }
    fn push(&mut self, client: &NativeClient, frame: Envelope) -> Result<(), Error> {
        let size = frame.encode().map_err(Error::Rejected)?.len();
        self._bytes.push(
            client
                .replay_bytes
                .clone()
                .try_acquire_many_owned(size.try_into().map_err(|_| overloaded())?)
                .map_err(|_| overloaded())?,
        );
        self.frames.push(frame);
        Ok(())
    }
}
impl NativeClient {
    pub async fn replay(
        &self,
        access: &NativeAccess,
        client: ClientId,
        session: qgenie_remote_proto::SessionId,
        after: Counter,
    ) -> Result<Envelope, Error> {
        let context = self.context(access, client, None)?;
        let mut message = Envelope::new(Message::Attach { after });
        message.machine = Some(self.scope.machine);
        message.session = Some(session);
        let id = message.id;
        let response = self
            .rpc(DispatchFrame::Remote {
                context,
                message: Box::new(message),
            })
            .await?;
        access.challenge.check(access.remaining_ms)?;
        match response {
            DispatchFrame::RemoteReply(reply)
                if reply.re == Some(id)
                    && reply.machine == Some(self.scope.machine)
                    && reply.session == Some(session)
                    && matches!(reply.message, Message::Replay { .. }) =>
            {
                reply.validate().map_err(Error::Rejected)?;
                Ok(*reply)
            }
            _ => Err(Error::Protocol),
        }
    }
    async fn snapshot_page(
        &self,
        access: &NativeAccess,
        client: ClientId,
        session: qgenie_remote_proto::SessionId,
        epoch: Option<MessageId>,
        page: u16,
    ) -> Result<SnapshotPage, Error> {
        let context = self.context(access, client, None)?;
        let response = self
            .rpc(DispatchFrame::ReadSnapshot {
                context,
                session,
                epoch,
                page,
            })
            .await?;
        access.challenge.check(access.remaining_ms)?;
        match response {
            DispatchFrame::SnapshotRead {
                session: actual,
                snapshot,
            } if actual == session && snapshot.page == page => {
                let mut test = Envelope::new(Message::Snapshot {
                    snapshot: snapshot.clone(),
                });
                test.session = Some(session);
                test.machine = Some(self.scope.machine);
                test.validate().map_err(Error::Rejected)?;
                if epoch.is_some_and(|epoch| epoch.to_string() != snapshot.epoch)
                    || snapshot.epoch.is_empty()
                    || snapshot.epoch.len() > 128
                {
                    return Err(Error::Protocol);
                }
                Ok(snapshot)
            }
            _ => Err(Error::Protocol),
        }
    }
    /// A single bounded attachment either returns contiguous replay, or one
    /// immutable checkpoint plus a final head. Partial captures never escape.
    pub async fn attachment(
        &self,
        access: &NativeAccess,
        client: ClientId,
        request: Envelope,
    ) -> Result<NativeAttachment, Error> {
        request.validate().map_err(Error::Rejected)?;
        let after = match request.message {
            Message::Attach { after } | Message::ReplayRequest { after } => Some(after),
            Message::Capture => None,
            _ => return Err(Error::Denied),
        };
        if request.machine != Some(self.scope.machine) || request.re.is_some() {
            return Err(Error::Denied);
        }
        let session = request.session.ok_or(Error::Denied)?;
        let permit = self
            .replay_work
            .clone()
            .try_acquire_owned()
            .map_err(|_| overloaded())?;
        tokio::time::timeout(access.remaining()?, async {
            let mut result = NativeAttachment {
                frames: Vec::new(),
                _work: permit,
                _bytes: Vec::new(),
            };
            if let Some(after) = after {
                let mut replay = self.replay(access, client, session, after).await?;
                if matches!(
                    &replay.message,
                    Message::Replay {
                        history_truncated: false,
                        ..
                    }
                ) {
                    replay.re = Some(request.id);
                    result.push(self, replay)?;
                    return Ok(result);
                }
            }
            let first = self.snapshot_page(access, client, session, None, 0).await?;
            let epoch: MessageId = first.epoch.parse().map_err(|_| Error::Protocol)?;
            let shape = (
                first.seq,
                first.total_pages,
                first.rows,
                first.cols,
                first.format,
            );
            result.push_page(self, &request, first)?;
            for page in 1..shape.1 {
                let next = self
                    .snapshot_page(access, client, session, Some(epoch), page)
                    .await?;
                if (
                    next.seq,
                    next.total_pages,
                    next.rows,
                    next.cols,
                    next.format,
                ) != shape
                {
                    return Err(Error::Protocol);
                }
                result.push_page(self, &request, next)?;
            }
            // Re-read the actual head after capture. If its required suffix was
            // already pruned/lost, require a new attachment rather than hiding it.
            let head = self.replay(access, client, session, shape.0).await?;
            let Message::Replay {
                floor,
                head,
                history_truncated: false,
                ..
            } = head.message
            else {
                return Err(Error::Rejected(RemoteError::rejected(
                    ErrorCode::HistoryTruncated,
                    "native tail changed beyond this checkpoint; attach again",
                )));
            };
            let control = self.controller_state(access, client, session).await?;
            let mut final_head = Envelope::new(Message::Head {
                floor,
                head,
                last_input: control.last_input,
            });
            final_head.re = Some(request.id);
            final_head.machine = request.machine;
            final_head.session = request.session;
            result.push(self, final_head)?;
            access.challenge.check(access.remaining_ms)?;
            Ok(result)
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }
}
