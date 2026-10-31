use qgenie_remote_client::{
    UserCredential, UserCredentials, WaitTarget, observation_can_reconnect,
};
use qgenie_remote_proto::{
    AgentState, Counter, ErrorCode, OperationOutcome, RemoteError, SessionState,
    wire::{AgentEvent, SequencedEvent, Timestamp, TurnOutcome},
};
use serde_json::json;
use std::time::Duration;
use tokio::time::Instant;

fn recovery() {
    let codes = [
        ErrorCode::Unauthenticated,
        ErrorCode::Forbidden,
        ErrorCode::AccessStale,
        ErrorCode::UnsupportedVersion,
        ErrorCode::UnsupportedCapability,
        ErrorCode::InvalidFrame,
        ErrorCode::FrameTooLarge,
        ErrorCode::Overloaded,
        ErrorCode::IdempotencyConflict,
        ErrorCode::StaleGeneration,
        ErrorCode::AlreadyResolved,
        ErrorCode::HistoryTruncated,
        ErrorCode::CapacityExhausted,
        ErrorCode::NotFound,
        ErrorCode::Internal,
        ErrorCode::ControlHeld,
    ];
    for code in codes {
        for outcome in [
            OperationOutcome::NotSent,
            OperationOutcome::Unknown,
            OperationOutcome::Rejected,
        ] {
            for retryable in [false, true] {
                let error = RemoteError {
                    code: code.clone(),
                    outcome: outcome.clone(),
                    retryable,
                    message: "synthetic connection failure".into(),
                };
                println!(
                    "{}",
                    json!({"kind": "recovery", "code": code, "outcome": outcome,
                        "retryable": retryable, "result": observation_can_reconnect(&error)})
                );
            }
        }
    }
}

fn credentials() {
    for mask in 0..8 {
        let publisher_live = mask & 1 != 0;
        let sign_in_required = mask & 2 != 0;
        let expired = mask & 4 != 0;
        let deadline = Instant::now()
            + if expired {
                Duration::ZERO
            } else {
                Duration::from_secs(60)
            };
        let token = "synthetic-spec-token".to_owned().try_into().unwrap();
        let (updates, credentials) =
            UserCredentials::channel(UserCredential::expiring(token, deadline));
        if sign_in_required {
            updates.require_sign_in();
        }
        let publisher = publisher_live.then_some(updates);
        let result = credentials.current();
        println!(
            "{}",
            json!({"kind": "credential", "mask": mask, "result": result.is_ok(),
                "error": result.err().map(|error| error.code)})
        );
        drop(publisher);
    }
}

fn turns() {
    // Eight symbols: two distinct turn IDs, each with four native terminal outcomes.
    // Enumerate every chronological trace of length zero through three.
    for length in 0..=3 {
        for index in 0..8_u32.pow(length) {
            let mut state = AgentState::default();
            let mut symbols = index;
            for _ in 0..length {
                let symbol = symbols % 8;
                symbols /= 8;
                let outcome = match symbol % 4 {
                    0 => Some(TurnOutcome::Completed),
                    1 => Some(TurnOutcome::Failed),
                    2 => Some(TurnOutcome::Interrupted),
                    _ => None,
                };
                state
                    .push(SequencedEvent {
                        seq: Counter(state.seq.0 + 1),
                        at: Timestamp::from_unix_millis(1).unwrap(),
                        event: AgentEvent::TurnCompleted {
                            turn: (symbol / 4).to_string(),
                            outcome,
                            input_tokens: Counter(0),
                            output_tokens: Counter(0),
                        },
                    })
                    .unwrap();
            }
            for target in 0..2 {
                let turn = target.to_string();
                let has_evidence = state.events.iter().any(|item| {
                    matches!(&item.event, AgentEvent::TurnCompleted { turn: found, .. } if found == &turn)
                });
                for scenario in 0..4 {
                    if scenario != 0 && !has_evidence {
                        continue;
                    }
                    let lifetime = match scenario {
                        1 => SessionState::Lost,
                        3 => SessionState::Exited,
                        _ => SessionState::Running,
                    };
                    state.history_truncated = scenario >= 2;
                    state.unconfirmed_tail = scenario >= 2;
                    let result = WaitTarget::Turn(turn.clone()).outcome(
                        &lifetime,
                        Some(&state),
                        usize::from(scenario != 0),
                    );
                    println!(
                        "{}",
                        json!({"kind": "turn", "length": length, "index": index,
                            "target": target, "scenario": scenario, "result": result})
                    );
                }
            }
        }
    }
}

fn main() {
    recovery();
    credentials();
    turns();
}
