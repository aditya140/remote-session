//! Native execution is isolated from gateway and client dependencies.
#[cfg(feature = "pty")]
mod pty;
#[cfg(feature = "pty")]
pub use pty::{NativeExit, PreparedPty, PtyControl, PtyProcess, PtyReader, spawn};
#[cfg(feature = "pty")]
mod screen;
#[cfg(feature = "pty")]
pub use screen::{Screen, ScreenSnapshot};

mod launch;
pub use launch::LaunchEnvironment;
#[cfg(feature = "codex")]
mod codex;
#[cfg(feature = "codex")]
pub use codex::{
    Codex, CodexConfig, CodexError, CodexExit, CodexHandle, CodexProcessControl, NativeApproval,
    NativeEffectAuthorization, NativeEvent, PINNED_CODEX_VERSION, Provider,
};
