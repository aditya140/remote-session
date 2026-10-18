use super::*;

#[test]
fn renewal_cannot_rebind_an_old_queued_nonce_or_increase_its_budget() {
    let mut grants = Grants::default();
    let old = HostAccess {
        connection: ClientId::new(),
        host_generation: Counter(u64::MAX),
        nonce: ChallengeId::new(),
        remaining_ms: 10_000,
        policy_revision: Counter(1),
    };
    let mut fresh = old.clone();
    fresh.nonce = ChallengeId::new();
    for (sequence, access) in [old.clone(), fresh.clone()].into_iter().enumerate() {
        grants.values.insert(
            access.nonce,
            Grant {
                access,
                message: MessageId::new(),
                deadline: now().unwrap() + Duration::from_secs(10),
                acknowledged: true,
                sequence: sequence as u64,
            },
        );
    }
    grants.current = Some(fresh.nonce);
    let mut queued = old.clone();
    queued.remaining_ms = 1000;
    grants.accepts(&queued).unwrap();
    grants.values.get_mut(&old.nonce).unwrap().deadline = Duration::ZERO;
    assert!(grants.accepts(&queued).is_err());
    assert_eq!(grants.current().unwrap().nonce, fresh.nonce);
    queued.nonce = fresh.nonce;
    queued.remaining_ms = 10_001;
    assert!(grants.accepts(&queued).is_err());
    queued = fresh.clone();
    queued.connection = ClientId::new();
    assert!(grants.accepts(&queued).is_err());
    grants.values.get_mut(&fresh.nonce).unwrap().acknowledged = false;
    assert!(grants.accepts(&fresh).is_err());
}

/// PostgreSQL proposal half of activation; native SQLite acknowledgments are
/// covered separately until the full connector handshake is composed.
pub(crate) async fn database_contract(store: Store, admin: &sqlx::PgPool) {
    use crate::{
        Authenticator, IdentityService, JwtVerifier,
        identity::{
            jwt::tests as jwt,
            tests::{proof, request},
        },
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    let verifier = JwtVerifier::with_keys(jwt::config(), jwt::keys(), store.clone()).unwrap();
    let owner = verifier
        .authenticate(&jwt::sign(&jwt::claims("native-activation-store-owner")))
        .await
        .unwrap()
        .user;
    let key_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key_file.path(), STANDARD.encode([66; 32])).unwrap();
    let identity = IdentityService::from_file(
        store.clone(),
        "https://gateway.example.invalid".into(),
        key_file.path(),
    )
    .await
    .unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let request = request(&key);
    let challenge = identity
        .enrollment_challenge(owner, request.clone())
        .await
        .unwrap();
    let enrolled = identity
        .enroll(owner, proof(&key, &challenge))
        .await
        .unwrap();
    let challenge = identity
        .machine_challenge(enrolled.machine, request.key)
        .await
        .unwrap();
    let token = identity
        .machine_token(proof(&key, &challenge))
        .await
        .unwrap();
    let actor = identity.authenticate_machine(&token.token).await.unwrap();
    let first = ClientId::new();
    let generation = store
        .connect_host(&actor, "native-store-node", first)
        .await
        .unwrap();
    let access = HostAccess {
        connection: first,
        host_generation: generation,
        nonce: ChallengeId::new(),
        remaining_ms: 10_000,
        policy_revision: Counter(1),
    };
    let second = ClientId::new();
    let replacement = store
        .connect_host(&actor, "native-store-node", second)
        .await
        .unwrap();
    assert!(matches!(
        store
            .confirm_native_access(&actor, generation, first, &access)
            .await,
        Err(Error::Forbidden)
    ));
    let mut access = HostAccess {
        connection: second,
        host_generation: replacement,
        ..access
    };
    store
        .confirm_native_access(&actor, replacement, second, &access)
        .await
        .unwrap();
    assert_eq!(
        store
            .routed_native_access(owner, actor.machine, "native-store-node", replacement)
            .await
            .unwrap(),
        Some(access.clone())
    );
    assert!(
        store
            .routed_native_access(
                UserId::new(),
                actor.machine,
                "native-store-node",
                replacement
            )
            .await
            .is_err()
    );
    access.nonce = ChallengeId::new();
    store
        .confirm_native_access(&actor, replacement, second, &access)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM qgr.audit_events WHERE machine=$1 AND action='host_activated'",
    )
    .bind(actor.machine.to_string())
    .fetch_one(admin)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let third = ClientId::new();
    let advanced = store
        .connect_host(&actor, "native-store-node", third)
        .await
        .unwrap();
    assert!(
        store
            .routed_native_access(owner, actor.machine, "native-store-node", advanced)
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        store
            .confirm_native_access(&actor, replacement, second, &access)
            .await,
        Err(Error::Forbidden)
    ));
    let recovered = store
        .connect_host_with_floor(
            &actor,
            "native-store-node",
            ClientId::new(),
            Counter(u64::MAX - 2),
        )
        .await
        .unwrap();
    assert_eq!(recovered, Counter(u64::MAX - 1));
    assert_eq!(
        store
            .connect_host_with_floor(&actor, "native-store-node", ClientId::new(), Counter(0))
            .await
            .unwrap(),
        Counter(u64::MAX)
    );
    assert!(matches!(
        store
            .connect_host_with_floor(
                &actor,
                "native-store-node",
                ClientId::new(),
                Counter(u64::MAX)
            )
            .await,
        Err(Error::Overloaded)
    ));
    sqlx::query("UPDATE qgr.users SET disabled=true WHERE id=$1")
        .bind(owner.to_string())
        .execute(admin)
        .await
        .unwrap();
    access.connection = third;
    access.host_generation = advanced;
    assert!(matches!(
        store
            .confirm_native_access(&actor, advanced, third, &access)
            .await,
        Err(Error::Forbidden)
    ));
}
