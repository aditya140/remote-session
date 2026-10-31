use qgenie_remote_policy::{Action, Facts, authorize};

fn main() {
    let actions = [
        Action::Observe,
        Action::Start,
        Action::Browse,
        Action::Control,
        Action::Input,
        Action::Resize,
        Action::Interrupt,
        Action::Approve,
        Action::Terminate,
    ];
    for action in actions {
        for mask in 0..2048 {
            let bit = |i| mask & (1 << i) != 0;
            let facts = Facts {
                owner: bit(0),
                access_fresh: bit(1),
                revoked: bit(2),
                same_os_user: bit(3),
                controller: bit(4),
                within_roots: bit(5),
                capability: bit(6),
                host_allows: bit(7),
                external: bit(8),
                termination_confirmed: bit(9),
                external_termination_confirmed: bit(10),
            };
            match authorize(action, facts) {
                Ok(()) => println!("{action:?},{mask},Allow,success,false"),
                Err(error) => println!(
                    "{action:?},{mask},{:?},{:?},{}",
                    error.code, error.outcome, error.retryable
                ),
            }
        }
    }
}
