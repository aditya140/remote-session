//! Plain, bounded output shared by the terminal client and scriptable reads.
use crate::{display::multiline, remote_agent::AgentView};
use qgenie_remote_client::RemoteSnapshot;
use qgenie_remote_proto::{ErrorCode, RemoteError, SnapshotFormat};

/// Render a native checkpoint without issuing terminal input or executing escapes.
/// Output is limited to the final `lines` (1..=1000) and 64 KiB, with an omission marker.
pub fn render_remote_snapshot(
    snapshot: &RemoteSnapshot,
    lines: usize,
) -> Result<String, RemoteError> {
    if !(1..=1000).contains(&lines) {
        return Err(RemoteError::rejected(
            ErrorCode::InvalidFrame,
            "read lines must be 1..=1000",
        ));
    }
    let (text, warning) = match snapshot.format {
        SnapshotFormat::AgentStateV1 => {
            let state = snapshot.agent_state()?;
            let warning = state.history_truncated || state.unconfirmed_tail;
            (AgentView::new(state).lines.join("\n"), warning)
        }
        SnapshotFormat::Vt100StateV1 => {
            let parser = vt100::Parser::from_checkpoint(&snapshot.data).map_err(|_| {
                RemoteError::rejected(ErrorCode::HistoryTruncated, "invalid terminal checkpoint")
            })?;
            if parser.screen().size() != (snapshot.rows, snapshot.cols) {
                return Err(RemoteError::rejected(
                    ErrorCode::InvalidFrame,
                    "terminal checkpoint dimensions differ",
                ));
            }
            (multiline(&parser.screen().contents()), false)
        }
    };
    let mut selected = Vec::new();
    let mut bytes = 0;
    let mut omitted = false;
    for line in text.lines().rev() {
        if selected.len() == lines {
            omitted = true;
            break;
        }
        let available = (65536usize - 160).saturating_sub(bytes + 1);
        if line.len() > available {
            let mut start = line.len() - available;
            while !line.is_char_boundary(start) {
                start += 1;
            }
            selected.push(&line[start..]);
            omitted = true;
            break;
        }
        selected.push(line);
        bytes += line.len() + 1;
    }
    selected.reverse();
    let mut rendered = String::new();
    if warning {
        rendered.push_str("[History is truncated or the native tail is unconfirmed.]\n");
    }
    if omitted {
        rendered.push_str("[Earlier text omitted by the read limit.]\n");
    }
    rendered.push_str(&selected.join("\n"));
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qgenie_remote_proto::{
        AgentState, Counter,
        wire::{AgentEvent, SequencedEvent, Timestamp},
    };
    #[test]
    fn output_is_bounded_and_cannot_emit_terminal_controls() {
        let mut state = AgentState::default();
        state
            .push(SequencedEvent {
                seq: Counter(1),
                at: Timestamp::from_unix_millis(1).unwrap(),
                event: AgentEvent::MessageCompleted {
                    message: "m".into(),
                    text: format!("{}\n\x1b[2Jsafe\u{202e}", "a\n".repeat(2000)),
                },
            })
            .unwrap();
        let mut snapshot = RemoteSnapshot {
            requested_after: None,
            epoch: "e".into(),
            seq: Counter(1),
            rows: 24,
            cols: 80,
            format: SnapshotFormat::AgentStateV1,
            data: state.encode().unwrap(),
            floor: Counter(1),
            head: Counter(1),
            last_input: Counter(0),
        };
        let text = render_remote_snapshot(&snapshot, 10).unwrap();
        assert!(text.starts_with("[Earlier text omitted"));
        assert!(text.contains("safe"));
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('\u{202e}'));
        assert!(text.len() <= 65536);
        assert!(render_remote_snapshot(&snapshot, 1001).is_err());
        state.events[0].event = AgentEvent::MessageCompleted {
            message: "m".into(),
            text: format!("{}END", "界".repeat(30000)),
        };
        snapshot.data = state.encode().unwrap();
        let text = render_remote_snapshot(&snapshot, 10).unwrap();
        assert!(text.ends_with("END"));
        assert!(text.len() <= 65536);
    }
}
