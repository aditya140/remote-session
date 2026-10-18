use axum::extract::ws::{CloseFrame, Message as AxumMessage, WebSocket};
use futures_util::{Sink, Stream};
use qgenie_remote_proto::ErrorCode;
use qgenie_remote_transport::ws::{Error, SocketCloseFrame, SocketMessage};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
/// One codec adaptation; queueing, limits, heartbeat, and task ownership stay in transport.
pub(super) struct Socket(pub WebSocket, pub std::sync::Arc<super::authority::Gate>);
impl Stream for Socket {
    type Item = Result<SocketMessage, Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.1.assert().is_err() {
            return Poll::Ready(Some(Err(Error::Protocol(ErrorCode::AccessStale))));
        }
        Pin::new(&mut self.0).poll_next(cx).map(|item| {
            item.map(|result| {
                result
                    .map(from_axum)
                    .map_err(|_| Error::Protocol(ErrorCode::InvalidFrame))
            })
        })
    }
}
impl Sink<SocketMessage> for Socket {
    type Error = Error;
    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        if self.1.assert().is_err() {
            return Poll::Ready(Err(Error::Protocol(ErrorCode::AccessStale)));
        }
        Pin::new(&mut self.0)
            .poll_ready(cx)
            .map_err(|_| Error::Network)
    }
    fn start_send(mut self: Pin<&mut Self>, message: SocketMessage) -> Result<(), Error> {
        self.1
            .assert()
            .map_err(|_| Error::Protocol(ErrorCode::AccessStale))?;
        let message = match message {
            SocketMessage::Text(text) => AxumMessage::Text(text.as_str().into()),
            SocketMessage::Binary(bytes) => AxumMessage::Binary(bytes),
            SocketMessage::Ping(bytes) => AxumMessage::Ping(bytes),
            SocketMessage::Pong(bytes) => AxumMessage::Pong(bytes),
            SocketMessage::Close(frame) => AxumMessage::Close(frame.map(|frame| CloseFrame {
                code: frame.code.into(),
                reason: frame.reason.as_str().into(),
            })),
            SocketMessage::Frame(_) => return Err(Error::Protocol(ErrorCode::InvalidFrame)),
        };
        Pin::new(&mut self.0)
            .start_send(message)
            .map_err(|_| Error::Network)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        if self.1.assert().is_err() {
            return Poll::Ready(Err(Error::Protocol(ErrorCode::AccessStale)));
        }
        Pin::new(&mut self.0)
            .poll_flush(cx)
            .map_err(|_| Error::Network)
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Pin::new(&mut self.0)
            .poll_close(cx)
            .map_err(|_| Error::Network)
    }
}
fn from_axum(message: AxumMessage) -> SocketMessage {
    match message {
        AxumMessage::Text(text) => SocketMessage::Text(text.as_str().into()),
        AxumMessage::Binary(bytes) => SocketMessage::Binary(bytes),
        AxumMessage::Ping(bytes) => SocketMessage::Ping(bytes),
        AxumMessage::Pong(bytes) => SocketMessage::Pong(bytes),
        AxumMessage::Close(frame) => SocketMessage::Close(frame.map(|frame| SocketCloseFrame {
            code: frame.code.into(),
            reason: frame.reason.as_str().into(),
        })),
    }
}
