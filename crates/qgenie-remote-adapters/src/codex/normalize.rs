use super::{CodexError, NativeApproval, NativeEvent};
use qgenie_remote_proto::{
    Counter,
    wire::{AgentEvent, ApprovalDecision},
};
use serde_json::Value;
pub(super) fn text(value: &Value, key: &str) -> Result<String, CodexError> {
    let text = value[key].as_str().ok_or(CodexError::Protocol)?;
    if text.len() > 256 {
        return Err(CodexError::Protocol);
    }
    Ok(text.into())
}
pub(super) fn approval(
    method: &str,
    message: &Value,
) -> Result<Option<NativeApproval>, CodexError> {
    if method != "item/commandExecution/requestApproval" {
        return Ok(None);
    }
    let params = &message["params"];
    let mut allowed = Vec::new();
    if let Some(decisions) = params["availableDecisions"].as_array() {
        for decision in decisions {
            match decision.as_str() {
                Some("accept") => allowed.push(ApprovalDecision::AllowOnce),
                Some("acceptForSession") => allowed.push(ApprovalDecision::AllowSession),
                Some("cancel") => allowed.push(ApprovalDecision::Deny),
                _ => {}
            }
        }
    }
    if allowed.is_empty() {
        return Err(CodexError::Unsupported);
    }
    let id = &message["id"];
    if !(id.is_i64() || id.as_str().is_some_and(|id| id.len() <= 256)) {
        return Err(CodexError::Protocol);
    }
    let summary = params["command"].as_str().ok_or(CodexError::Protocol)?;
    if summary.len() > 64 * 1024 {
        return Err(CodexError::Protocol);
    }
    Ok(Some(NativeApproval {
        native_id: id.to_string(),
        thread: text(params, "threadId")?,
        item: text(params, "itemId")?,
        kind: "command".into(),
        summary: summary.into(),
        allowed_decisions: allowed,
    }))
}
pub(super) fn event(method: &str, params: &Value) -> Result<Option<NativeEvent>, CodexError> {
    let event = match method {
        "turn/started" => AgentEvent::TurnStarted {
            turn: text(&params["turn"], "id")?,
        },
        "turn/completed" => AgentEvent::TurnCompleted {
            turn: text(&params["turn"], "id")?,
            input_tokens: Counter(0),
            output_tokens: Counter(0),
        },
        "item/agentMessage/delta" => AgentEvent::MessageDelta {
            message: text(params, "itemId")?,
            text: params["delta"].as_str().ok_or(CodexError::Protocol)?.into(),
        },
        "item/completed" if params["item"]["type"] == "agentMessage" => {
            AgentEvent::MessageCompleted {
                message: text(&params["item"], "id")?,
                text: params["item"]["text"]
                    .as_str()
                    .ok_or(CodexError::Protocol)?
                    .into(),
            }
        }
        "item/started" if params["item"]["type"] == "commandExecution" => AgentEvent::ToolStarted {
            tool: text(&params["item"], "id")?,
            name: "command".into(),
            summary: params["item"]["command"]
                .as_str()
                .unwrap_or("command")
                .into(),
        },
        "item/commandExecution/outputDelta" => AgentEvent::ToolOutput {
            tool: text(params, "itemId")?,
            text: params["delta"].as_str().ok_or(CodexError::Protocol)?.into(),
        },
        "item/completed" if params["item"]["type"] == "commandExecution" => {
            AgentEvent::ToolCompleted {
                tool: text(&params["item"], "id")?,
                success: params["item"]["status"] == "completed" && params["item"]["exitCode"] == 0,
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(NativeEvent::Event {
        thread: text(params, "threadId")?,
        event,
    }))
}
