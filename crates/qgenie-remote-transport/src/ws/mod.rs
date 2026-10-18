//! Bounded WebSocket connections. This module performs no product authorization.
#[cfg(feature = "client-ws")]
mod client;
mod config;
mod connection;
mod error;
mod queues;

#[cfg(feature = "client-ws")]
pub use client::{ClientConfig, SecretHeader, connect};
pub use config::{Backoff, Limits, Priority, SharedBudget};
#[cfg(feature = "server-ws")]
pub use connection::accept;
pub use connection::{
    Connection, ConnectionCancellation, Frame, Metrics, ReceivedFrame, Sender, Transmission,
    TransportEvent,
};
pub use error::{CloseReason, Error};

// Framework adapters can use these types without selecting a second codec version.
pub use tokio_tungstenite::tungstenite::{
    Message as SocketMessage,
    protocol::{CloseFrame as SocketCloseFrame, frame::coding::CloseCode as SocketCloseCode},
};
