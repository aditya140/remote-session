//! Interactive local terminal preview using only the client SDK.
mod app;
mod remote;
mod remote_agent;
mod remote_approvals;
mod remote_badge;
mod remote_dialog;
mod remote_machine;
mod remote_start;
mod remote_terminal;
mod view;
pub use remote::{run_remote, run_remote_with_state};

pub use app::run;
