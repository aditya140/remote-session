use super::*;
use qgenie_remote_proto::{
    ChallengeId, ClientId, Counter, IPC_MINOR, MachineId, SessionId, SessionState, UserId,
    ipc::{DispatchFrame, RemoteScope},
    wire::{Capabilities, LaunchKind, Origin, Timestamp},
};
use qgenie_remote_transport::ipc::{
    Listener, read_dispatch, server_handshake_with_capabilities, write_dispatch,
};
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn active_inventory_follows_byte_limited_pages_without_merging_wire_frames() {
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
    let mut sessions = (0..18)
        .map(|_| Session {
            requested_model: None,
            metadata: Default::default(),
            id: SessionId::new(),
            machine: scope.machine,
            owner: scope.owner,
            kind: LaunchKind::Pty {
                command: vec!["x".repeat(63000)],
            },
            cwd: "/work".into(),
            origin: Origin::Managed,
            state: SessionState::Running,
            created_at: Timestamp::from_unix_millis(1).unwrap(),
            ended_at: None,
            head: Counter(0),
            capabilities: Capabilities::default(),
            recording_warning: None,
        })
        .collect::<Vec<_>>();
    sessions.sort_by_key(|session| session.id);
    let expected: Vec<_> = sessions.iter().map(|session| session.id).collect();
    let fixture = tokio::spawn(async move {
        let nonce = ChallengeId::new();
        for _ in 0..5 {
            let mut stream = listener.accept().await.unwrap();
            server_handshake_with_capabilities(
                &mut stream,
                IPC_MINOR,
                &["native-authority-v1", "native-pages-v1"],
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
                DispatchFrame::Remote { message, .. } => {
                    let Message::SessionListPage {
                        after,
                        limit: 128,
                        active_only: true,
                    } = message.message
                    else {
                        panic!("expected active page")
                    };
                    let mut remaining = sessions
                        .iter()
                        .filter(|session| after.is_none_or(|id| session.id > id));
                    let page: Vec<_> = remaining.by_ref().take(7).cloned().collect();
                    let next = remaining.next().map(|_| page.last().unwrap().id);
                    let mut reply = Envelope::new(Message::SessionPage {
                        sessions: page,
                        next,
                    });
                    reply.re = Some(message.id);
                    reply.machine = message.machine;
                    DispatchFrame::RemoteReply(Box::new(reply))
                }
                other => panic!("unexpected native request {other:?}"),
            };
            write_dispatch(&mut stream, &reply).await.unwrap();
        }
    });
    let client = NativeClient::new(path, scope).unwrap();
    let challenge = client.challenge(ClientId::new(), Counter(1)).await.unwrap();
    let access = client.activate(&challenge, 10000, false).await.unwrap();
    let pages = client
        .active_session_pages(&access)
        .await
        .expect("active metadata can exceed one byte-limited page");
    assert_eq!(pages.len(), 3);
    assert_eq!(
        pages
            .iter()
            .flatten()
            .map(|session| session.id)
            .collect::<Vec<_>>(),
        expected
    );
    assert!(serde_json::to_vec(&pages).unwrap().len() > qgenie_remote_proto::MAX_JSON_BYTES);
    for sessions in pages {
        let mut update = Envelope::new(Message::SessionUpdates { sessions });
        update.machine = Some(client.scope().machine);
        update
            .validate()
            .expect("each original page remains a valid update");
        assert!(serde_json::to_vec(&update).unwrap().len() <= qgenie_remote_proto::MAX_JSON_BYTES);
    }
    fixture.await.unwrap();
}

#[tokio::test]
async fn active_inventory_rejects_foreign_scope_and_nonadvancing_pages() {
    for scenario in [
        "foreign owner",
        "empty continuation",
        "repeated cursor",
        "reversed IDs",
    ] {
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
        let mut ids = [SessionId::new(), SessionId::new()];
        ids.sort();
        let session = Session {
            requested_model: None,
            metadata: Default::default(),
            id: ids[0],
            machine: scope.machine,
            owner: scope.owner,
            kind: LaunchKind::Pty {
                command: vec!["/bin/sh".into()],
            },
            cwd: "/work".into(),
            origin: Origin::Managed,
            state: SessionState::Running,
            created_at: Timestamp::from_unix_millis(1).unwrap(),
            ended_at: None,
            head: Counter(0),
            capabilities: Capabilities::default(),
            recording_warning: None,
        };
        let fixture = tokio::spawn(async move {
            let requests = if scenario == "repeated cursor" { 2 } else { 1 };
            for index in 0..requests {
                let mut stream = listener.accept().await.unwrap();
                server_handshake_with_capabilities(
                    &mut stream,
                    IPC_MINOR,
                    &["native-authority-v1", "native-pages-v1"],
                )
                .await
                .unwrap();
                let DispatchFrame::Remote { message, .. } =
                    read_dispatch(&mut stream).await.unwrap()
                else {
                    panic!("expected session page request")
                };
                let mut sessions = vec![session.clone()];
                let next = match scenario {
                    "foreign owner" => {
                        sessions[0].owner = UserId::new();
                        None
                    }
                    "empty continuation" => {
                        sessions.clear();
                        Some(ids[0])
                    }
                    "repeated cursor" => (index == 0).then_some(ids[0]),
                    "reversed IDs" => {
                        let mut later = session.clone();
                        later.id = ids[1];
                        sessions.insert(0, later);
                        None
                    }
                    _ => unreachable!(),
                };
                let mut reply = Envelope::new(Message::SessionPage { sessions, next });
                reply.re = Some(message.id);
                reply.machine = message.machine;
                write_dispatch(&mut stream, &DispatchFrame::RemoteReply(Box::new(reply)))
                    .await
                    .unwrap();
            }
        });
        let access = NativeAccess {
            challenge: super::super::NativeChallenge {
                nonce: ChallengeId::new(),
                policy_revision: Counter(1),
                max_remaining_ms: 10000,
                scope: scope.clone(),
                connection: ClientId::new(),
                generation: Counter(1),
                issued: std::time::Instant::now(),
                issued_elapsed: super::super::elapsed().unwrap(),
            },
            remaining_ms: 10000,
        };
        let client = NativeClient::new(path, scope).unwrap();
        assert!(
            matches!(
                client.active_session_pages(&access).await,
                Err(Error::Protocol)
            ),
            "accepted {scenario}"
        );
        fixture.await.unwrap();
    }
}
