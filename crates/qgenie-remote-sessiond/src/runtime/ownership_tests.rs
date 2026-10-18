use super::*;
use tests::{call, directory, spec};

#[test]
fn remote_start_keys_and_immutable_owners_survive_reenrollment_and_restart() {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let owner = UserId::new();
    let machine = MachineId::new();
    let a = RemoteScope {
        origin: "https://a.example.test".into(),
        owner,
        machine,
    };
    let mut b = a.clone();
    b.origin = "https://b.example.test".into();
    let mut c = a.clone();
    c.machine = MachineId::new();
    let mut config = Config::new(path.clone(), vec![path.clone()]);
    config.remote_bindings = vec![a.clone(), b.clone(), c.clone()];
    let runtime = Runtime::open(config.clone()).unwrap();
    let launch = spec(&path, IdempotencyKey::new());
    let local = runtime.start(launch.clone()).unwrap();
    let first = runtime
        .start_scoped(launch.clone(), Some(a.clone()))
        .unwrap();
    let second = runtime
        .start_scoped(launch.clone(), Some(b.clone()))
        .unwrap();
    let third = runtime
        .start_scoped(launch.clone(), Some(c.clone()))
        .unwrap();
    assert_eq!(first.machine, machine);
    assert_eq!(third.machine, c.machine);
    let ids = std::collections::BTreeSet::from([local.id, first.id, second.id, third.id]);
    assert_eq!(ids.len(), 4);
    assert_eq!(
        runtime
            .start_scoped(launch.clone(), Some(a.clone()))
            .unwrap()
            .id,
        first.id
    );
    let mut changed = launch.clone();
    changed.cols += 1;
    assert!(matches!(
        runtime
            .start_scoped(changed, Some(a.clone()))
            .unwrap_err()
            .code,
        ErrorCode::IdempotencyConflict
    ));
    let mut impostor = a.clone();
    impostor.owner = UserId::new();
    assert!(
        runtime
            .start_scoped(launch.clone(), Some(impostor.clone()))
            .is_err()
    );
    let Reply::Started(found) = call(
        &runtime,
        ClientId::new(),
        Operation::LookupStart { key: launch.key },
    ) else {
        panic!("local lookup");
    };
    assert_eq!(
        found.id, local.id,
        "local lookup cannot return an equal remote key"
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while !runtime.readers_finished() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(std::fs::read(path.join("native-marker")).unwrap(), b"xxxx");
    drop(runtime);
    let reopened = Runtime::open_after_previous_owner(config.clone()).unwrap();
    assert_eq!(
        reopened
            .start_scoped(launch.clone(), Some(a.clone()))
            .unwrap()
            .id,
        first.id
    );
    assert_eq!(
        lock(&reopened.session(local.id).unwrap()).unwrap().scope,
        None,
        "local history is never adopted"
    );
    assert_eq!(
        lock(&reopened.session(first.id).unwrap()).unwrap().scope,
        Some(a)
    );
    assert_eq!(
        lock(&reopened.session(second.id).unwrap()).unwrap().scope,
        Some(b)
    );
    assert_eq!(std::fs::read(path.join("native-marker")).unwrap(), b"xxxx");
    drop(reopened);
    config.remote_bindings = vec![impostor];
    assert!(
        Runtime::open(config).is_err(),
        "durable machine binding cannot change owner"
    );
}

#[test]
fn legacy_local_keys_upgrade_without_becoming_remote_sessions() {
    let temp = directory();
    let path = std::fs::canonicalize(temp.path()).unwrap();
    let database = path.join("sessions.db");
    private_file(&database).unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(include_str!("../../migrations/0001_sessions.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../../migrations/0002_checkpoints.sql"))
        .unwrap();
    let id = SessionId::new();
    let launch = spec(&path, IdempotencyKey::new());
    let mut parameters = serde_json::to_value(&launch).unwrap();
    parameters.as_object_mut().unwrap().remove("key");
    let digest = fingerprint(&parameters).unwrap();
    connection
        .execute(
            "INSERT INTO sessions VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![
                id.to_string(),
                launch.key.to_string(),
                digest,
                serde_json::to_string(&launch).unwrap(),
                serde_json::to_string(&SessionState::Exited).unwrap(),
                "1",
                0u64.to_be_bytes().as_slice(),
                0u64.to_be_bytes().as_slice(),
                0
            ],
        )
        .unwrap();
    drop(connection);
    let scope = RemoteScope {
        origin: "https://gateway.example.test".into(),
        owner: UserId::new(),
        machine: MachineId::new(),
    };
    let mut config = Config::new(path.clone(), vec![path]);
    config.remote_bindings = vec![scope];
    let runtime = Runtime::open(config).unwrap();
    let Reply::Started(found) = call(
        &runtime,
        ClientId::new(),
        Operation::LookupStart { key: launch.key },
    ) else {
        panic!("legacy key");
    };
    assert_eq!(found.id, id);
    assert!(lock(&runtime.session(id).unwrap()).unwrap().scope.is_none());
    assert_eq!(runtime.start(launch).unwrap().id, id); // original fingerprint and key survive
}
