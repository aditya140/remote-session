//! Deterministic action decisions. Trusted facts are supplied by the caller.
use qgenie_remote_proto::{ErrorCode, RemoteError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Observe,
    Start,
    Browse,
    Control,
    Input,
    Resize,
    Interrupt,
    Approve,
    Terminate,
}
#[derive(Debug, Clone, Copy)]
pub struct Facts {
    pub owner: bool,
    pub access_fresh: bool,
    pub revoked: bool,
    pub same_os_user: bool,
    pub controller: bool,
    pub within_roots: bool,
    pub capability: bool,
    pub host_allows: bool,
    pub external: bool,
    pub termination_confirmed: bool,
    pub external_termination_confirmed: bool,
}

pub fn authorize(action: Action, facts: Facts) -> Result<(), RemoteError> {
    if !facts.owner || facts.revoked || !facts.same_os_user {
        return Err(deny("owner access required"));
    }
    if !facts.access_fresh {
        return Err(RemoteError::rejected(
            ErrorCode::AccessStale,
            "access facts are stale",
        ));
    }
    if !facts.host_allows {
        return Err(deny("host policy denies this action"));
    }
    if !facts.capability {
        return Err(RemoteError::rejected(
            ErrorCode::UnsupportedCapability,
            "adapter does not support this action",
        ));
    }
    if matches!(action, Action::Start | Action::Browse) && !facts.within_roots {
        return Err(deny("path is outside allowed roots"));
    }
    if matches!(action, Action::Input | Action::Resize | Action::Interrupt) && !facts.controller {
        return Err(RemoteError::rejected(
            ErrorCode::StaleGeneration,
            "active control lease required",
        ));
    }
    if action == Action::Terminate && !facts.termination_confirmed {
        return Err(deny(if facts.external {
            "external session termination requires explicit confirmation"
        } else {
            "session termination requires confirmation"
        }));
    }
    if action == Action::Terminate && facts.external && !facts.external_termination_confirmed {
        return Err(deny(
            "external session termination requires explicit external-session confirmation",
        ));
    }
    Ok(())
}
fn deny(message: &str) -> RemoteError {
    RemoteError::rejected(ErrorCode::Forbidden, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner() -> Facts {
        Facts {
            owner: true,
            access_fresh: true,
            revoked: false,
            same_os_user: true,
            controller: false,
            within_roots: false,
            capability: true,
            host_allows: true,
            external: false,
            termination_confirmed: false,
            external_termination_confirmed: false,
        }
    }
    #[test]
    fn owner_approval_is_independent_of_control_and_existing_cwd() {
        assert!(authorize(Action::Approve, owner()).is_ok());
        assert!(authorize(Action::Control, owner()).is_ok());
        assert!(authorize(Action::Input, owner()).is_err());
        assert!(
            authorize(
                Action::Input,
                Facts {
                    controller: true,
                    ..owner()
                }
            )
            .is_ok()
        );
        assert!(
            authorize(
                Action::Terminate,
                Facts {
                    external: true,
                    termination_confirmed: true,
                    ..owner()
                }
            )
            .is_err()
        );
        assert!(
            authorize(
                Action::Terminate,
                Facts {
                    external: true,
                    termination_confirmed: true,
                    external_termination_confirmed: true,
                    ..owner()
                }
            )
            .is_ok()
        );
        assert!(authorize(Action::Start, owner()).is_err());
        assert!(authorize(Action::Browse, owner()).is_err());
    }
    #[test]
    fn denies_stale_revoked_foreign_and_unsupported_facts() {
        for facts in [
            Facts {
                owner: false,
                ..owner()
            },
            Facts {
                revoked: true,
                ..owner()
            },
            Facts {
                access_fresh: false,
                ..owner()
            },
            Facts {
                same_os_user: false,
                ..owner()
            },
            Facts {
                capability: false,
                ..owner()
            },
            Facts {
                host_allows: false,
                ..owner()
            },
        ] {
            assert!(authorize(Action::Approve, facts).is_err());
        }
        assert!(authorize(Action::Terminate, owner()).is_err());
        assert!(
            authorize(
                Action::Terminate,
                Facts {
                    termination_confirmed: true,
                    ..owner()
                }
            )
            .is_ok()
        );
    }
}
