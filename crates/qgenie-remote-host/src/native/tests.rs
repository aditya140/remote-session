use super::*;
use qgenie_remote_proto::{
    ApplicationOutcome, IPC_MINOR, MachineId,
    wire::{Generations, SessionInput},
};
use qgenie_remote_transport::ipc::{Listener, server_handshake_with_capabilities};
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn stalled_native_reads_cannot_exhaust_controller_or_prompt_admission() {
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = directory.path().canonicalize().unwrap();
    let listener = Listener::bind(&path.join("dispatch.sock")).unwrap();
    let scope = RemoteScope {
        origin: "https://gateway.example.test".into(),
        owner: UserId::new(),
        machine: MachineId::new(),
    };
    let client = NativeClient::new(path, scope).unwrap();
    let (entered, mut reads) = tokio::sync::mpsc::channel(4);
    let (release, gate) = tokio::sync::watch::channel(false);
    let fixture = tokio::spawn(async move {
        let nonce = ChallengeId::new();
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let mut stream = listener.accept().await.unwrap();
            let entered = entered.clone();
            let mut gate = gate.clone();
            tasks.spawn(async move {
                server_handshake_with_capabilities(
                    &mut stream,
                    IPC_MINOR,
                    &[
                        "native-authority-v1",
                        "native-floor-v1",
                        "native-controller-v1",
                        "native-effects-v1",
                        "native-agent-v1",
                        "native-prompt-receipts-v1",
                    ],
                )
                .await
                .unwrap();
                let reply = match read_dispatch(&mut stream).await.unwrap() {
                    DispatchFrame::AccessChallenge { .. } => DispatchFrame::AccessChallengeIssued {
                        nonce,
                        policy_revision: Counter(1),
                        max_remaining_ms: 10000,
                    },
                    DispatchFrame::ActivateHost(activation) => DispatchFrame::HostActivated {
                        nonce,
                        generation: activation.generation,
                    },
                    DispatchFrame::QueryHostFloor { .. } => {
                        entered.send(()).await.unwrap();
                        gate.wait_for(|released| *released).await.unwrap();
                        DispatchFrame::HostFloor {
                            generation: Counter(1),
                        }
                    }
                    DispatchFrame::QueryControl { session, .. } => DispatchFrame::ControlState {
                        session,
                        floor: Counter(1),
                        holder: None,
                        remaining_ms: 0,
                        last_input: Counter(0),
                    },
                    DispatchFrame::Remote { message, .. } => {
                        let Message::Input {
                            input: SessionInput::PromptReceipt(ref text),
                            input_seq,
                            authority,
                        } = message.message
                        else {
                            panic!("expected one prompt")
                        };
                        assert_eq!(text, "one prompt");
                        let mut reply = Envelope::new(Message::InputAck {
                            generation: authority.controller.unwrap(),
                            input_seq,
                            outcome: ApplicationOutcome::Applied,
                            turn: Some("exact-turn".into()),
                        });
                        reply.machine = message.machine;
                        reply.session = message.session;
                        reply.re = Some(message.id);
                        DispatchFrame::RemoteReply(Box::new(reply))
                    }
                    other => panic!("unexpected request {other:?}"),
                };
                write_dispatch(&mut stream, &reply).await.unwrap();
            });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    });
    let challenge = client.challenge(ClientId::new(), Counter(1)).await.unwrap();
    let access = client.activate(&challenge, 10000, false).await.unwrap();
    let mut pending = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let client = client.clone();
        pending.spawn(async move { client.host_floor().await });
    }
    for _ in 0..4 {
        tokio::time::timeout(Duration::from_secs(1), reads.recv())
            .await
            .unwrap()
            .unwrap();
    }
    assert!(
        matches!(client.host_floor().await,Err(Error::Rejected(error)) if error.code==ErrorCode::Overloaded)
    );
    let session = SessionId::new();
    assert_eq!(
        client
            .controller_state(&access, ClientId::new(), session)
            .await
            .unwrap()
            .floor,
        Counter(1)
    );
    let mut request = Envelope::new(Message::Input {
        authority: Generations {
            host: Counter(1),
            controller: Some(Counter(1)),
        },
        input_seq: Counter(1),
        input: SessionInput::PromptReceipt("one prompt".into()),
    });
    request.machine = Some(client.scope.machine);
    request.session = Some(session);
    let result = client
        .effect(&access, ClientId::new(), request)
        .await
        .unwrap();
    assert!(
        matches!(result.message,Message::InputAck {turn:Some(turn),outcome:ApplicationOutcome::Applied,..} if turn=="exact-turn")
    );
    release.send_replace(true);
    while let Some(result) = pending.join_next().await {
        assert_eq!(result.unwrap().unwrap(), Counter(1));
    }
    tokio::time::timeout(Duration::from_secs(1), fixture)
        .await
        .unwrap()
        .unwrap();
}
