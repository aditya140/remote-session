//! One original access value owns a private coalesced notification stream.
use super::*;
use tokio::net::UnixStream;

pub struct NativeChanges {
    stream: UnixStream,
    access: NativeAccess,
    _work: tokio::sync::OwnedSemaphorePermit,
}
impl NativeChanges {
    pub async fn changed(&mut self) -> Result<(), Error> {
        let remaining = self.access.remaining()?;
        let frame = tokio::time::timeout(remaining, read_dispatch(&mut self.stream))
            .await
            .map_err(|_| Error::Expired)?
            .map_err(|_| Error::Unavailable)?;
        self.access.challenge.check(self.access.remaining_ms)?;
        match frame {
            DispatchFrame::NativeChanged => Ok(()),
            DispatchFrame::Error(error) => Err(Error::Rejected(error)),
            _ => Err(Error::Protocol),
        }
    }
}
impl NativeClient {
    pub async fn output_head(
        &self,
        access: &NativeAccess,
        session: qgenie_remote_proto::SessionId,
    ) -> Result<Envelope, Error> {
        let context = self.context(access, access.connection(), None)?;
        match self
            .rpc(DispatchFrame::ReadHead { context, session })
            .await?
        {
            DispatchFrame::HeadRead {
                session: actual,
                floor,
                head,
                last_input,
            } if actual == session => {
                access.challenge.check(access.remaining_ms)?;
                let mut envelope = Envelope::new(Message::Head {
                    floor,
                    head,
                    last_input,
                });
                envelope.machine = Some(self.scope.machine);
                envelope.session = Some(session);
                envelope.validate().map_err(Error::Rejected)?;
                Ok(envelope)
            }
            DispatchFrame::Error(error) => Err(Error::Rejected(error)),
            _ => Err(Error::Protocol),
        }
    }
    pub async fn watch_changes(&self, access: &NativeAccess) -> Result<NativeChanges, Error> {
        let context = self.context(access, access.connection(), None)?;
        let permit = self
            .change_work
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Unavailable)?;
        let stream = tokio::time::timeout(Duration::from_secs(3), async {
            let mut stream = connect(&self.directory.join("dispatch.sock"))
                .await
                .map_err(|_| Error::Unavailable)?;
            client_handshake_with_options(
                &mut stream,
                &["native-authority-v1", "native-changes-v1"],
                &[],
            )
            .await
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::InvalidData {
                    Error::Rejected(RemoteError::rejected(
                        ErrorCode::UnsupportedCapability,
                        "native changes capability is unavailable",
                    ))
                } else {
                    Error::Unavailable
                }
            })?;
            write_dispatch(&mut stream, &DispatchFrame::WatchChanges { context })
                .await
                .map_err(|_| Error::Unavailable)?;
            match read_dispatch(&mut stream)
                .await
                .map_err(|_| Error::Unavailable)?
            {
                DispatchFrame::ChangesReady => Ok(stream),
                DispatchFrame::Error(error) => Err(Error::Rejected(error)),
                _ => Err(Error::Protocol),
            }
        })
        .await
        .map_err(|_| Error::Unavailable)??;
        access.challenge.check(access.remaining_ms)?;
        Ok(NativeChanges {
            stream,
            access: access.clone(),
            _work: permit,
        })
    }
}
