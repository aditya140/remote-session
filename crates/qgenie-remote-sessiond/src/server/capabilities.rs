//! Native IPC admission. Authorization and lifecycle checks remain in Runtime.
use crate::Runtime;
use qgenie_remote_proto::{
    ErrorCode, RemoteError,
    ipc::DispatchFrame,
    wire::{LaunchKind, Message, SessionInput},
};

pub(super) fn offered(managed: bool) -> Vec<&'static str> {
    let mut capabilities = vec![
        "local-v1",
        "native-authority-v1",
        "native-controller-v1",
        "recording-read-v1",
        "recording-reclaim-v1",
        "native-floor-v1",
        "native-start-v1",
        "native-effects-v1",
        "native-replay-v1",
        "native-changes-v1",
        "native-pages-v1",
    ];
    if managed {
        capabilities.extend(["native-agent-v1", "native-approvals-v1"]);
    }
    capabilities
}

pub(super) fn recording(frame: &DispatchFrame) -> bool {
    matches!(
        frame,
        DispatchFrame::RecordingFetch { .. }
            | DispatchFrame::RecordingStatus { .. }
            | DispatchFrame::RecordingEpochChallenge { .. }
            | DispatchFrame::BindRecordingEpoch { .. }
            | DispatchFrame::RecordingAcknowledge { .. }
    )
}

pub(super) fn check(
    runtime: &Runtime,
    frame: &DispatchFrame,
    negotiated: &[String],
) -> Result<(), RemoteError> {
    let require = |required: &[&str]| {
        if required
            .iter()
            .all(|name| negotiated.iter().any(|capability| capability == name))
        {
            Ok(())
        } else {
            Err(RemoteError::rejected(
                ErrorCode::UnsupportedCapability,
                "required native capability was not negotiated",
            ))
        }
    };
    // Minor-zero local peers predate capability negotiation.
    if matches!(frame, DispatchFrame::Local(_)) {
        return Ok(());
    }
    require(&["native-authority-v1"])?;
    match frame {
        DispatchFrame::WatchChanges { .. } => require(&["native-changes-v1"]),
        DispatchFrame::AgentInventory { .. } => require(&["native-agent-v1"]),
        DispatchFrame::QueryHostFloor { .. } => require(&["native-floor-v1"]),
        DispatchFrame::QueryControl { .. }
        | DispatchFrame::ActivateControl(_)
        | DispatchFrame::DeactivateControl { .. } => require(&["native-controller-v1"]),
        DispatchFrame::RecordingFetch { .. } | DispatchFrame::RecordingStatus { .. } => {
            require(&["recording-read-v1"])
        }
        DispatchFrame::RecordingEpochChallenge { .. }
        | DispatchFrame::BindRecordingEpoch { .. }
        | DispatchFrame::RecordingAcknowledge { .. } => {
            require(&["recording-read-v1", "recording-reclaim-v1"])
        }
        DispatchFrame::ReadHead { .. } => require(&["native-replay-v1"]),
        DispatchFrame::ReadSnapshot { session, .. } => {
            require(&["native-replay-v1"])?;
            if runtime.is_agent_session(*session) {
                require(&["native-agent-v1"])?;
            }
            Ok(())
        }
        DispatchFrame::Remote { message, .. } => match &message.message {
            Message::Start(launch) => {
                require(&["native-start-v1"])?;
                if matches!(launch.kind, LaunchKind::Agent { .. }) {
                    require(&["native-agent-v1"])?;
                }
                Ok(())
            }
            Message::Input { input, .. } => {
                require(&["native-effects-v1", "native-controller-v1"])?;
                if matches!(input, SessionInput::Prompt(_) | SessionInput::Steer(_)) {
                    require(&["native-agent-v1"])?;
                }
                Ok(())
            }
            Message::Resize { .. } | Message::Interrupt { .. } | Message::Terminate { .. } => {
                require(&["native-effects-v1", "native-controller-v1"])
            }
            Message::Attach { .. } => {
                require(&["native-replay-v1"])?;
                if message
                    .session
                    .is_some_and(|session| runtime.is_agent_session(session))
                {
                    require(&["native-agent-v1"])?;
                }
                Ok(())
            }
            Message::SessionListPage { .. } | Message::SessionGet { .. } => {
                require(&["native-pages-v1"])
            }
            Message::ApprovalList | Message::ApprovalCommitted(_) => {
                require(&["native-approvals-v1"])
            }
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}
