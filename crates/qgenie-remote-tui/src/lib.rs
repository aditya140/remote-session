//! Local and remote terminal and agent views backed by the client SDK.
mod app;
mod display;
mod remote;
mod remote_agent;
mod remote_read;
pub use remote_read::render_remote_snapshot;
mod remote_approvals;
mod remote_badge;
mod remote_dialog;
mod remote_machine;
mod remote_start;
mod remote_terminal;
mod view;
pub use remote::{run_remote, run_remote_authenticated, run_remote_with_state};

pub use app::run;
