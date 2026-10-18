//! Bounded transport primitives. No operation retries or authorization policy.
#[cfg(feature = "elapsed-clock")]
pub mod elapsed;
#[cfg(all(unix, feature = "local-ipc"))]
pub mod ipc;
#[cfg(feature = "ws")]
pub mod ws;
