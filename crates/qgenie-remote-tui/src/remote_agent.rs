//! Structured content is rendered as text widgets, never processed as terminal escapes.
use qgenie_remote_proto::{
    AgentState, RemoteError,
    wire::{AgentEvent, SequencedEvent},
};
use std::collections::BTreeMap;

pub(crate) struct AgentView {
    pub state: AgentState,
    pub lines: Vec<String>,
}
impl AgentView {
    pub fn new(state: AgentState) -> Self {
        let mut view = Self {
            state,
            lines: Vec::new(),
        };
        view.rebuild();
        view
    }
    pub fn push(&mut self, event: SequencedEvent) -> Result<(), RemoteError> {
        self.state.push(event)
    }
    pub fn rebuild(&mut self) {
        let mut rows: Vec<String> = Vec::new();
        let mut messages = BTreeMap::new();
        let mut tools = BTreeMap::new();
        if self.state.history_truncated {
            rows.push("[Earlier timeline is truncated or contains a gap. Recorded history remains available.]".into());
        }
        if self.state.unconfirmed_tail {
            rows.push("[Native output tail is unconfirmed.]".into());
        }
        for item in &self.state.events {
            match &item.event {
                AgentEvent::TurnStarted { .. } => rows.push("Turn started".into()),
                AgentEvent::MessageDelta { message, text } => {
                    let index = *messages.entry(message).or_insert_with(|| {
                        rows.push(String::new());
                        rows.len() - 1
                    });
                    rows[index].push_str(text);
                }
                AgentEvent::MessageCompleted { message, text } => {
                    let index = *messages.entry(message).or_insert_with(|| {
                        rows.push(String::new());
                        rows.len() - 1
                    });
                    rows[index] = text.clone();
                }
                AgentEvent::ToolStarted {
                    tool,
                    name,
                    summary,
                } => {
                    tools.insert(tool, rows.len());
                    rows.push(format!("Tool {name}: {summary}"));
                }
                AgentEvent::ToolOutput { tool, text } => {
                    let index = *tools.entry(tool).or_insert_with(|| {
                        rows.push("Tool output".into());
                        rows.len() - 1
                    });
                    rows[index].push('\n');
                    rows[index].push_str(text);
                }
                AgentEvent::ToolCompleted { success, .. } => rows.push(format!(
                    "Tool {}",
                    if *success { "completed" } else { "failed" }
                )),
                AgentEvent::TurnCompleted {
                    input_tokens,
                    output_tokens,
                    ..
                } => rows.push(format!(
                    "Turn complete · {} input / {} output tokens",
                    input_tokens.0, output_tokens.0
                )),
                AgentEvent::ReasoningSummary { text } => rows.push(format!("Reasoning: {text}")),
                AgentEvent::FileDiff { path, diff } => rows.push(format!("File {path}\n{diff}")),
                AgentEvent::Error { message } => rows.push(format!("Error: {message}")),
                AgentEvent::Gap { reason, .. } => rows.push(format!("[Output gap: {reason}]")),
                AgentEvent::Exited { code } => rows.push(format!("Native process exited ({code})")),
                AgentEvent::Idle => rows.push("Idle".into()),
                AgentEvent::Opaque { native_kind, .. } => {
                    rows.push(format!("Unsupported native event: {native_kind}"))
                }
                AgentEvent::PtyOutput { .. } | AgentEvent::Resize { .. } => {}
            }
        }
        self.lines = rows
            .into_iter()
            .flat_map(|text| text.split('\n').map(clean).collect::<Vec<_>>())
            .collect();
    }
}
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() || matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}') {
                '�'
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use qgenie_remote_proto::{Counter, wire::Timestamp};
    #[test]
    fn completion_replaces_streamed_prefix_and_native_text_cannot_control_the_terminal() {
        let mut view = AgentView::new(AgentState::default());
        for (index, event) in [
            AgentEvent::MessageDelta {
                message: "m".into(),
                text: "prefix".into(),
            },
            AgentEvent::MessageCompleted {
                message: "m".into(),
                text: "complete\n\u{1b}[2J\u{202e}safe".into(),
            },
            AgentEvent::ToolStarted {
                tool: "t".into(),
                name: "command".into(),
                summary: "printf x".into(),
            },
            AgentEvent::ToolOutput {
                tool: "t".into(),
                text: "tool result".into(),
            },
        ]
        .into_iter()
        .enumerate()
        {
            view.push(SequencedEvent {
                seq: Counter(index as u64 + 1),
                at: Timestamp::from_unix_millis(1).unwrap(),
                event,
            })
            .unwrap();
        }
        view.rebuild();
        let text = view.lines.join("\n");
        assert!(text.contains("complete\n"));
        assert!(!text.contains("prefix"));
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\u{202e}'));
        assert!(text.contains("Tool command: printf x\ntool result"));
    }
}
