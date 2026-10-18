use super::*;
use qgenie_remote_proto::{ChunkId, KeyId, MachineId, SessionId, UserId};

pub(crate) async fn database_contract(admin: &PgPool, migration: &SecretDsn, authority: &Store) {
    // This fixture seeds synthetic rows as the migration identity to test actual application
    // role boundaries. It does not claim an authenticated/validated ingest API exists yet.
    sqlx::raw_sql("DO $$ BEGIN
      IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='qgr_test_record_ingest') THEN CREATE ROLE qgr_test_record_ingest LOGIN PASSWORD 'test-only'; END IF;
      IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='qgr_test_history_owner') THEN CREATE ROLE qgr_test_history_owner LOGIN PASSWORD 'test-only'; END IF;
      IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='qgr_test_history_admin') THEN CREATE ROLE qgr_test_history_admin LOGIN PASSWORD 'test-only'; END IF;
      IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='qgr_test_record_lifecycle') THEN CREATE ROLE qgr_test_record_lifecycle LOGIN PASSWORD 'test-only'; END IF;
    END $$;
    GRANT qgr_record_ingest TO qgr_test_record_ingest; GRANT qgr_history_owner TO qgr_test_history_owner; GRANT qgr_history_admin TO qgr_test_history_admin; GRANT qgr_record_lifecycle TO qgr_test_record_lifecycle;
    SELECT qgr_recordings.ensure_month('2026-10-01');").execute(admin).await.unwrap();
    let mut fixtures = Vec::new();
    for _ in 0..2 {
        let owner = UserId::new();
        let machine = MachineId::new();
        let session = SessionId::new();
        let chunk = ChunkId::new();
        sqlx::query("INSERT INTO qgr.users(id,issuer,tenant,subject) VALUES($1,'synthetic-recording-fixture','test',$1)").bind(owner.to_string()).execute(admin).await.unwrap();
        sqlx::query("INSERT INTO qgr.machines(id,owner,installation,os_user,active_key,hostname,name,os,arch,cli_version) VALUES($1,$2,$1,'fixture',$3,'fixture','fixture','linux','test','test')").bind(machine.to_string()).bind(owner.to_string()).bind(KeyId::new().to_string()).execute(admin).await.unwrap();
        sqlx::query(
            "INSERT INTO qgr.sessions(id,machine,owner,body,state) VALUES($1,$2,$3,'{}','running')",
        )
        .bind(session.to_string())
        .bind(machine.to_string())
        .bind(owner.to_string())
        .execute(admin)
        .await
        .unwrap();
        sqlx::query("INSERT INTO qgr_recordings.receipts(machine,session,owner,kind,id,immutable,receipt,captured_from,captured_to) VALUES($1,$2,$3,'content',$4,'{}','{}','2026-10-17T00:00:00Z','2026-10-17T00:00:00Z')").bind(machine.to_string()).bind(session.to_string()).bind(owner.to_string()).bind(chunk.to_string()).execute(admin).await.unwrap();
        for table in ["content", "held_content"] {
            let sql = match table {
                "content" => {
                    "INSERT INTO qgr_recordings.content(machine,session,owner,id,captured_from,captured_to,payload,stored_digest) VALUES($1,$2,$3,$4,'2026-10-17T00:00:00Z','2026-10-17T00:00:00Z',$5,$6)"
                }
                _ => {
                    "INSERT INTO qgr_recordings.held_content(machine,session,owner,id,captured_from,captured_to,payload,stored_digest) VALUES($1,$2,$3,$4,'2026-10-17T00:00:00Z','2026-10-17T00:00:00Z',$5,$6)"
                }
            };
            sqlx::query(sql)
                .bind(machine.to_string())
                .bind(session.to_string())
                .bind(owner.to_string())
                .bind(chunk.to_string())
                .bind(vec![1u8])
                .bind("00".repeat(32))
                .execute(admin)
                .await
                .unwrap();
        }
        fixtures.push((owner, machine, session, chunk));
    }
    sqlx::query("INSERT INTO qgr_recordings.holds(id,owner,revision,active,case_ref,reason,actor) VALUES('hold_00000000-0000-4000-8000-000000000001',$1,1,true,'fixture-case','synthetic RLS fixture',$1)").bind(fixtures[0].0.to_string()).execute(admin).await.unwrap();
    sqlx::query("INSERT INTO qgr_recordings.held_cases(machine,session,id,hold,case_ref) VALUES($1,$2,$3,'hold_00000000-0000-4000-8000-000000000001','fixture-case')").bind(fixtures[0].1.to_string()).bind(fixtures[0].2.to_string()).bind(fixtures[0].3.to_string()).execute(admin).await.unwrap();
    let mut databases = Vec::new();
    for (name, role) in [
        ("qgr_test_record_ingest", RecordingRole::Ingest),
        ("qgr_test_history_owner", RecordingRole::OwnerRead),
        ("qgr_test_history_admin", RecordingRole::AdminRead),
        ("qgr_test_record_lifecycle", RecordingRole::Lifecycle),
    ] {
        let mut url = url::Url::parse(&migration.0).unwrap();
        url.set_username(name).unwrap();
        url.set_password(Some("test-only")).unwrap();
        let secret = crate::tests::recording_secret(url.as_str());
        let database = RecordingDatabase::connect(&secret, role, 1).await.unwrap();
        database.ready().await.unwrap();
        assert_eq!(database.role(), role);
        assert!(matches!(
            RecordingDatabase::connect(
                &secret,
                if role == RecordingRole::Ingest {
                    RecordingRole::OwnerRead
                } else {
                    RecordingRole::Ingest
                },
                1
            )
            .await,
            Err(Error::Role)
        ));
        databases.push((database, secret));
    }
    assert!(matches!(
        RecordingDatabase::connect(migration, RecordingRole::Lifecycle, 1).await,
        Err(Error::Role)
    ));
    let ingest = &databases[0].0;
    let owner = &databases[1].0;
    let history_admin = &databases[2].0;
    let lifecycle = &databases[3].0;
    for database in [&owner, &history_admin] {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.content")
            .fetch_one(&database.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert!(
            sqlx::query("SELECT * FROM qgr_recordings.content_202610")
                .execute(&database.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM qgr_recordings.receipts")
                .execute(&database.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("ALTER TABLE qgr_recordings.content DISABLE ROW LEVEL SECURITY")
                .execute(&database.pool)
                .await
                .is_err()
        );
    }
    for database in [ingest, lifecycle] {
        assert!(
            sqlx::query("SELECT * FROM qgr_recordings.content")
                .execute(&database.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("SELECT * FROM qgr_recordings.held_content")
                .execute(&database.pool)
                .await
                .is_err()
        );
    }
    assert!(
        sqlx::query("SELECT * FROM qgr_recordings.content")
            .execute(&authority.pool)
            .await
            .is_err()
    );
    for (user, _, _, _) in &fixtures {
        let mut tx = owner.pool.begin().await.unwrap();
        set_transaction(&mut tx).await;
        sqlx::query("SELECT set_config('qgr.owner',$1,true)")
            .bind(user.to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        let rows: Vec<String> = sqlx::query_scalar("SELECT owner FROM qgr_recordings.content")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
        assert_eq!(rows, vec![user.to_string()]);
        let rows: Vec<String> = sqlx::query_scalar("SELECT owner FROM qgr_recordings.receipts")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
        assert_eq!(rows, vec![user.to_string()]);
        tx.commit().await.unwrap();
        // Same physical size-one pool, new request, absent context: previous owner cannot leak.
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.content")
            .fetch_one(&owner.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert!(
            sqlx::query("SELECT * FROM qgr_recordings.held_content")
                .execute(&owner.pool)
                .await
                .is_err()
        );
    }
    let mut tx = ingest.pool.begin().await.unwrap();
    set_transaction(&mut tx).await;
    sqlx::query("SELECT set_config('qgr.owner',$1,true),set_config('qgr.machine',$2,true)")
        .bind(fixtures[0].0.to_string())
        .bind(fixtures[1].1.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.receipts")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 0);
    tx.rollback().await.unwrap();
    let mut tx = history_admin.pool.begin().await.unwrap();
    set_transaction(&mut tx).await;
    sqlx::query("SELECT set_config('qgr.actor',$1,true),set_config('qgr.reason',' ',true)")
        .bind(fixtures[0].0.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.content")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 0);
    sqlx::query("SELECT set_config('qgr.reason','synthetic case investigation',true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.content")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 2);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.held_content")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 0);
    sqlx::query("SELECT set_config('qgr.case','fixture-case',true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.held_content")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 1);
    tx.rollback().await.unwrap();
    // This proves RLS/case membership, not lifecycle state or audit-before-release; no API releases bytes.
    super::super::cancellation_tests::database_contract(owner, &databases[1].1).await;
    sqlx::raw_sql("GRANT qgr_history_owner TO qgr_test_record_ingest")
        .execute(admin)
        .await
        .unwrap();
    assert!(matches!(
        RecordingDatabase::connect(&databases[0].1, RecordingRole::Ingest, 1).await,
        Err(Error::Role)
    ));
    sqlx::raw_sql("REVOKE qgr_history_owner FROM qgr_test_record_ingest")
        .execute(admin)
        .await
        .unwrap();
    ingest_scope_contract(admin, ingest, &fixtures[0]).await;
    for (database, _) in databases {
        database.close().await;
    }
}

async fn ingest_scope_contract(
    admin: &PgPool,
    ingest: &RecordingDatabase,
    fixture: &(UserId, MachineId, SessionId, ChunkId),
) {
    let (owner, machine, session, _) = *fixture;
    let key: String = sqlx::query_scalar("SELECT active_key FROM qgr.machines WHERE id=$1")
        .bind(machine.to_string())
        .fetch_one(admin)
        .await
        .unwrap();
    sqlx::query("INSERT INTO qgr.machine_keys(id,machine,public_key) VALUES($1,$2,$3)")
        .bind(&key)
        .bind(machine.to_string())
        .bind(rand::random::<[u8; 32]>().to_vec())
        .execute(admin)
        .await
        .unwrap();
    sqlx::query("UPDATE qgr.sessions SET body=jsonb_build_object('created_at','2026-10-17T00:00:00.000Z') WHERE id=$1").bind(session.to_string()).execute(admin).await.unwrap();
    for (candidate_owner, candidate_key, expires) in [
        (UserId::new(), key.clone(), 4_102_444_800_i64),
        (owner, KeyId::new().to_string(), 4_102_444_800),
        (owner, key.clone(), 1),
    ] {
        let error = sqlx::query("SELECT * FROM qgr_recordings.ingest_scope($1,$2,$3,$4,$5)")
            .bind(machine.to_string())
            .bind(candidate_key)
            .bind(session.to_string())
            .bind(candidate_owner.to_string())
            .bind(expires)
            .fetch_one(&ingest.pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("42501")
        );
    }
    let mut previous: time::OffsetDateTime =
        sqlx::query_scalar("SELECT touched_at FROM qgr_recordings.commit_barrier")
            .fetch_one(admin)
            .await
            .unwrap();
    for _ in 0..2 {
        let mut tx = ingest.pool.begin().await.unwrap();
        set_transaction(&mut tx).await;
        let row = sqlx::query("SELECT * FROM qgr_recordings.ingest_scope($1,$2,$3,$4,4102444800)")
            .bind(machine.to_string())
            .bind(&key)
            .bind(session.to_string())
            .bind(owner.to_string())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("owner_id"), owner.to_string());
        assert_eq!(row.get::<String, _>("durability"), "unqualified");
        assert!(!row.get::<bool, _>("lifecycle_ready"));
        assert!(row.get::<bool, _>("recovery_pending"));
        assert!(!row.get::<bool, _>("synchronous_ready"));
        let actual: time::OffsetDateTime = row.get("created_at");
        assert_eq!(actual.year(), 2026);
        assert_eq!(actual.month(), time::Month::October);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.receipts")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(count, 1);
        tx.commit().await.unwrap();
        let touched: time::OffsetDateTime =
            sqlx::query_scalar("SELECT touched_at FROM qgr_recordings.commit_barrier")
                .fetch_one(admin)
                .await
                .unwrap();
        assert!(touched > previous);
        previous = touched;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.receipts")
            .fetch_one(&ingest.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    // These are local PostgreSQL write barriers, not synchronous-standby qualification.
    assert!(
        sqlx::query("SELECT * FROM qgr_recordings.state")
            .execute(&ingest.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE qgr_recordings.commit_barrier SET touched_at=clock_timestamp()")
            .execute(&ingest.pool)
            .await
            .is_err()
    );
}

async fn set_transaction(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) {
    sqlx::query("SELECT set_config('qgr.transaction',pg_current_xact_id()::text,true)")
        .execute(&mut **tx)
        .await
        .unwrap();
}
