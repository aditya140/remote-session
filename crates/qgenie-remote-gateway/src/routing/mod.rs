pub(crate) mod activation;
mod approvals;
mod authority;
mod broker;
mod config;
mod control;
mod edge;
mod hub;
mod inventory;
mod message;
mod metrics;
mod pending;
mod route;
mod socket;
mod store;
#[cfg(test)]
pub(crate) mod tests;
use crate::Error;
pub use config::RoutingConfig;
pub(crate) use edge::{client_upgrade, host_upgrade};
pub(crate) use hub::Routing;
fn now_ms() -> Result<u64, Error> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Unavailable)?
        .as_millis()
        .try_into()
        .map_err(|_| Error::Unavailable)
}
