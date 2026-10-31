use super::*;
use crate::{
    Authenticator, IdentityService, JwtVerifier,
    identity::{
        jwt::tests as jwt,
        tests::{proof, request},
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::SessionState;
use qgenie_remote_proto::wire::{Capabilities, HostAccess, LaunchKind, Origin, Session};

pub(crate) async fn database_contract(store: Store, admin: &sqlx::PgPool) {
    let verifier = JwtVerifier::with_keys(jwt::config(), jwt::keys(), store.clone()).unwrap();
    let owner = verifier
        .authenticate(&jwt::sign(&jwt::claims("controller-owner")))
        .await
        .unwrap()
        .user;
    let key_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key_file.path(), STANDARD.encode([67; 32])).unwrap();
    let identity = IdentityService::from_file(
        store.clone(),
        "https://gateway.example.invalid".into(),
        key_file.path(),
    )
    .await
    .unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let enrollment = request(&key);
    let challenge = identity
        .enrollment_challenge(owner, enrollment.clone())
        .await
        .unwrap();
    let enrolled = identity
        .enroll(owner, proof(&key, &challenge))
        .await
        .unwrap();
    let challenge = identity
        .machine_challenge(enrolled.machine, enrollment.key)
        .await
        .unwrap();
    let token = identity
        .machine_token(proof(&key, &challenge))
        .await
        .unwrap();
    let actor = identity.authenticate_machine(&token.token).await.unwrap();
    let connection = ClientId::new();
    let generation = store
        .connect_host(&actor, "controller-store", connection)
        .await
        .unwrap();
    let mut scope = ControllerScope {
        owner,
        machine: actor.machine,
        session: SessionId::new(),
        host_generation: generation,
        host_connection: connection,
    };
    let session = Session {
        metadata: Default::default(),
        id: scope.session,
        machine: actor.machine,
        owner,
        kind: LaunchKind::Pty {
            command: vec!["synthetic-controller-fixture".into()],
        },
        cwd: "/outside/new-session-roots".into(),
        origin: Origin::Managed,
        state: SessionState::Running,
        created_at: Timestamp::from_unix_millis(1).unwrap(),
        ended_at: None,
        head: Counter(0),
        capabilities: Capabilities {
            approvals: ["command".into()].into(),
            input: true,
            resize: true,
            interrupt: true,
            ..Default::default()
        },
        recording_warning: None,
    };
    store.project(&actor, generation, &session).await.unwrap();
    let a = ClientId::new();
    let b = ClientId::new();
    let first = ControllerRequest {
        scope: scope.clone(),
        holder: a,
        request: MessageId::new(),
        native_floor: Counter(0),
        native_holder: None,
        takeover: false,
    };
    assert!(matches!(
        store.propose_controller(first.clone()).await,
        Err(Error::Forbidden)
    ));
    let access = HostAccess {
        connection,
        host_generation: generation,
        nonce: qgenie_remote_proto::ChallengeId::new(),
        remaining_ms: 10_000,
        policy_revision: Counter(1),
    };
    store
        .confirm_native_access(&actor, generation, connection, &access)
        .await
        .unwrap();
    crate::approvals::tests::database_contract(store.clone(), admin, scope.clone()).await;
    let (one, two) = tokio::join!(
        store.propose_controller(first.clone()),
        store.propose_controller(first.clone())
    );
    let one = one.unwrap();
    let two = two.unwrap();
    assert_eq!(one.generation, Counter(1));
    assert_eq!(one.generation, two.generation);
    assert_eq!(one.expires_at, two.expires_at);
    assert!(!one.native_acknowledged);
    let mut changed = first.clone();
    changed.takeover = true;
    assert!(matches!(
        store.propose_controller(changed).await,
        Err(Error::Conflict)
    ));
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert!(matches!(
        store.confirm_controller(&one, 1).await,
        Err(Error::StaleGeneration)
    ));
    let active = store.confirm_controller(&one, 9_000).await.unwrap();
    assert!(active.native_acknowledged);
    assert!(active.deadline <= one.deadline);
    let repeated = store.confirm_controller(&one, 15_000).await.unwrap();
    assert!(
        repeated.expires_at <= active.expires_at,
        "duplicate confirmation cannot extend the first native boundary"
    );
    let mut next = ControllerRequest {
        scope: scope.clone(),
        holder: b,
        request: MessageId::new(),
        native_floor: Counter(1),
        native_holder: Some(a),
        takeover: false,
    };
    assert!(matches!(
        store.propose_controller(next.clone()).await,
        Err(Error::ControlHeld)
    ));
    next.takeover = true;
    let pending = store.propose_controller(next.clone()).await.unwrap();
    assert_eq!(pending.generation, Counter(2));
    assert!(matches!(
        store.confirm_controller(&one, 15_000).await,
        Err(Error::StaleGeneration)
    ));
    let obsolete = store.propose_controller(first.clone()).await.unwrap();
    assert_eq!(obsolete.generation, Counter(1));
    assert!(obsolete.remaining_budget().is_err());
    assert!(!obsolete.native_acknowledged);
    let active = store.confirm_controller(&pending, 15_000).await.unwrap();
    let renew_id = MessageId::new();
    let renewal = store
        .renew_controller(&active, renew_id, Counter(2), Some(b))
        .await
        .unwrap();
    assert!(!renewal.native_acknowledged);
    let repeated = store
        .renew_controller(&active, renew_id, Counter(2), Some(b))
        .await
        .unwrap();
    assert_eq!(renewal.expires_at, repeated.expires_at);
    assert!(matches!(
        store.confirm_controller(&active, 15_000).await,
        Err(Error::StaleGeneration)
    ));
    let renewed = store.confirm_controller(&renewal, 15_000).await.unwrap();
    assert!(matches!(
        store
            .renew_controller(&active, MessageId::new(), Counter(2), Some(b))
            .await,
        Err(Error::StaleGeneration)
    ));
    assert!(matches!(
        store
            .renew_controller(&renewed, MessageId::new(), Counter(2), None)
            .await,
        Err(Error::StaleGeneration)
    ));
    let grants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM qgr.audit_events WHERE session=$1 AND action='controller_granted'",
    )
    .bind(scope.session.to_string())
    .fetch_one(admin)
    .await
    .unwrap();
    assert_eq!(grants, 2);
    store.release_controller(&renewed).await.unwrap();
    store.release_controller(&renewed).await.unwrap();
    assert!(matches!(
        store
            .renew_controller(&renewed, MessageId::new(), Counter(2), Some(b))
            .await,
        Err(Error::StaleGeneration)
    ));
    let mut ahead = ControllerRequest {
        scope: scope.clone(),
        holder: a,
        request: MessageId::new(),
        native_floor: Counter(100),
        native_holder: Some(ClientId::new()),
        takeover: false,
    };
    assert!(matches!(
        store.propose_controller(ahead.clone()).await,
        Err(Error::ControlHeld)
    ));
    ahead.takeover = true;
    let advanced = store.propose_controller(ahead.clone()).await.unwrap();
    assert_eq!(advanced.generation, Counter(101));
    let new_connection = ClientId::new();
    let new_generation = store
        .connect_host(&actor, "controller-store", new_connection)
        .await
        .unwrap();
    assert!(matches!(
        store.confirm_controller(&advanced, 15_000).await,
        Err(Error::Forbidden)
    ));
    scope.host_generation = new_generation;
    scope.host_connection = new_connection;
    let new_access = HostAccess {
        connection: new_connection,
        host_generation: new_generation,
        ..access
    };
    store
        .confirm_native_access(&actor, new_generation, new_connection, &new_access)
        .await
        .unwrap();
    ahead.scope = scope.clone();
    let old = store.propose_controller(ahead.clone()).await.unwrap();
    assert!(old.remaining_budget().is_err());
    ahead.request = MessageId::new();
    ahead.native_floor = Counter(u64::MAX - 1);
    ahead.native_holder = None;
    let max = store.propose_controller(ahead.clone()).await.unwrap();
    assert_eq!(max.generation, Counter(u64::MAX));
    let max = store.confirm_controller(&max, 15_000).await.unwrap();
    assert!(max.native_acknowledged);
    ahead.request = MessageId::new();
    ahead.native_floor = Counter(u64::MAX);
    assert!(matches!(
        store.propose_controller(ahead.clone()).await,
        Err(Error::Overloaded)
    ));
    let stored: String =
        sqlx::query_scalar("SELECT generation::text FROM qgr.control_leases WHERE session=$1")
            .bind(scope.session.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert_eq!(stored, u64::MAX.to_string());
    let mut foreign = ahead.clone();
    foreign.scope.owner = UserId::new();
    assert!(matches!(
        store.propose_controller(foreign).await,
        Err(Error::Forbidden)
    ));
    sqlx::query("UPDATE qgr.users SET disabled=true WHERE id=$1")
        .bind(owner.to_string())
        .execute(admin)
        .await
        .unwrap();
    assert!(matches!(
        store.confirm_controller(&max, 15_000).await,
        Err(Error::Forbidden)
    ));
}
