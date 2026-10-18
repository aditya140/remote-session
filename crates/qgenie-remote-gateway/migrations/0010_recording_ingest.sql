-- One PostgreSQL lifecycle lock coordinates ingest, holds and partition retirement.
CREATE TABLE qgr_recordings.state (
    singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton),
    recovery_epoch text NOT NULL DEFAULT 'uninitialized' CHECK(recovery_epoch ~ '^[A-Za-z0-9_-]{1,128}$'),
    durability text NOT NULL DEFAULT 'unqualified' CHECK(durability IN ('unqualified','synthetic_single_server','synchronous_standby')),
    lifecycle_ready boolean NOT NULL DEFAULT false,
    recovery_pending boolean NOT NULL DEFAULT true,
    retention_days integer NOT NULL DEFAULT 90 CHECK(retention_days BETWEEN 1 AND 36500),
    retry_not_before timestamptz NOT NULL DEFAULT '1970-01-01T00:00:00Z'
);
INSERT INTO qgr_recordings.state(singleton) VALUES(true);
ALTER TABLE qgr_recordings.state ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.state FORCE ROW LEVEL SECURITY;
CREATE POLICY state_internal ON qgr_recordings.state TO CURRENT_USER USING(true) WITH CHECK(true);
CREATE TABLE qgr_recordings.commit_barrier(singleton boolean PRIMARY KEY CHECK(singleton),touched_at timestamptz NOT NULL);
INSERT INTO qgr_recordings.commit_barrier VALUES(true,clock_timestamp());
REVOKE ALL ON qgr_recordings.commit_barrier FROM PUBLIC;
CREATE TABLE qgr_recordings.partitions (
    month date PRIMARY KEY,
    name text NOT NULL UNIQUE CHECK(name ~ '^content_[0-9]{6}$'),
    state text NOT NULL CHECK(state IN ('active','retiring','retired'))
);
ALTER TABLE qgr_recordings.partitions ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.partitions FORCE ROW LEVEL SECURITY;
CREATE POLICY partition_internal ON qgr_recordings.partitions TO CURRENT_USER USING(true) WITH CHECK(true);

CREATE TABLE qgr_recordings.receipt_epochs (
    machine text NOT NULL,session text NOT NULL,owner text NOT NULL,
    kind text NOT NULL CHECK(kind IN ('content','gap')),id text NOT NULL,
    recovery_epoch text NOT NULL CHECK(recovery_epoch ~ '^[A-Za-z0-9_-]{1,128}$'),
    receipt jsonb NOT NULL CHECK(octet_length(receipt::text)<=8192),
    PRIMARY KEY(machine,session,kind,id,recovery_epoch),
    FOREIGN KEY(machine,session,kind,id) REFERENCES qgr_recordings.receipts(machine,session,kind,id),
    FOREIGN KEY(session,machine,owner) REFERENCES qgr.sessions(id,machine,owner)
);
ALTER TABLE qgr_recordings.receipt_epochs ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.receipt_epochs FORCE ROW LEVEL SECURITY;
CREATE POLICY epoch_internal ON qgr_recordings.receipt_epochs TO CURRENT_USER USING(true) WITH CHECK(true);
CREATE POLICY epoch_ingest ON qgr_recordings.receipt_epochs TO qgr_record_ingest
    USING(machine=nullif(current_setting('qgr.machine',true),'') AND owner=nullif(current_setting('qgr.owner',true),''))
    WITH CHECK(machine=nullif(current_setting('qgr.machine',true),'') AND owner=nullif(current_setting('qgr.owner',true),''));
GRANT SELECT,INSERT ON qgr_recordings.receipt_epochs TO qgr_record_ingest;

CREATE TABLE qgr_recordings.streams (
    machine text NOT NULL,session text NOT NULL,owner text NOT NULL,
    stream_epoch text NOT NULL CHECK(length(stream_epoch)<=64),
    stream text NOT NULL CHECK(stream IN ('events','pty','stdout','stderr')),
    PRIMARY KEY(machine,stream_epoch),
    UNIQUE(machine,session,stream_epoch,stream),
    FOREIGN KEY(session,machine,owner) REFERENCES qgr.sessions(id,machine,owner)
);
ALTER TABLE qgr_recordings.streams ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.streams FORCE ROW LEVEL SECURITY;
CREATE POLICY stream_internal ON qgr_recordings.streams TO CURRENT_USER USING(true) WITH CHECK(true);
CREATE POLICY stream_ingest ON qgr_recordings.streams TO qgr_record_ingest
    USING(machine=nullif(current_setting('qgr.machine',true),'') AND owner=nullif(current_setting('qgr.owner',true),''))
    WITH CHECK(machine=nullif(current_setting('qgr.machine',true),'') AND owner=nullif(current_setting('qgr.owner',true),''));
GRANT SELECT,INSERT ON qgr_recordings.streams TO qgr_record_ingest;
CREATE TABLE qgr_recordings.fragments (
    machine text NOT NULL,session text NOT NULL,owner text NOT NULL,
    stream_epoch text NOT NULL CHECK(length(stream_epoch)<=64),
    stream text NOT NULL CHECK(stream IN ('events','pty','stdout','stderr')),
    seq numeric(20,0) NOT NULL CHECK(seq BETWEEN 1 AND 18446744073709551615),
    fragment integer NOT NULL CHECK(fragment BETWEEN 0 AND 32),last boolean NOT NULL,
    kind text NOT NULL DEFAULT 'content' CHECK(kind='content'),chunk text NOT NULL,
    PRIMARY KEY(machine,session,stream_epoch,seq,fragment),
    FOREIGN KEY(machine,session,stream_epoch,stream) REFERENCES qgr_recordings.streams(machine,session,stream_epoch,stream),
    FOREIGN KEY(machine,session,kind,chunk) REFERENCES qgr_recordings.receipts(machine,session,kind,id),
    FOREIGN KEY(session,machine,owner) REFERENCES qgr.sessions(id,machine,owner)
);
ALTER TABLE qgr_recordings.fragments ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.fragments FORCE ROW LEVEL SECURITY;
CREATE POLICY fragment_internal ON qgr_recordings.fragments TO CURRENT_USER USING(true) WITH CHECK(true);
CREATE POLICY fragment_ingest ON qgr_recordings.fragments TO qgr_record_ingest
    USING(machine=nullif(current_setting('qgr.machine',true),'') AND owner=nullif(current_setting('qgr.owner',true),''))
    WITH CHECK(machine=nullif(current_setting('qgr.machine',true),'') AND owner=nullif(current_setting('qgr.owner',true),''));
GRANT SELECT,INSERT ON qgr_recordings.fragments TO qgr_record_ingest;

CREATE TABLE qgr_recordings.gaps (
    machine text NOT NULL,session text NOT NULL,owner text NOT NULL,
    kind text NOT NULL DEFAULT 'gap' CHECK(kind='gap'),id text NOT NULL,
    captured_from timestamptz NOT NULL,captured_to timestamptz NOT NULL CHECK(captured_to>=captured_from),
    body jsonb NOT NULL CHECK(octet_length(body::text)<=8192),
    PRIMARY KEY(machine,session,id),
    FOREIGN KEY(machine,session,kind,id) REFERENCES qgr_recordings.receipts(machine,session,kind,id),
    FOREIGN KEY(session,machine,owner) REFERENCES qgr.sessions(id,machine,owner)
);
ALTER TABLE qgr_recordings.gaps ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.gaps FORCE ROW LEVEL SECURITY;
CREATE POLICY gap_internal ON qgr_recordings.gaps TO CURRENT_USER USING(true) WITH CHECK(true);
CREATE POLICY gap_ingest ON qgr_recordings.gaps FOR INSERT TO qgr_record_ingest
    WITH CHECK(machine=nullif(current_setting('qgr.machine',true),'') AND owner=nullif(current_setting('qgr.owner',true),''));
CREATE POLICY gap_owner ON qgr_recordings.gaps FOR SELECT TO qgr_history_owner USING(owner=nullif(current_setting('qgr.owner',true),''));
CREATE POLICY gap_admin ON qgr_recordings.gaps FOR SELECT TO qgr_history_admin USING(nullif(current_setting('qgr.actor',true),'') IS NOT NULL AND nullif(btrim(current_setting('qgr.reason',true)),'') IS NOT NULL);
GRANT INSERT ON qgr_recordings.gaps TO qgr_record_ingest;
GRANT SELECT ON qgr_recordings.gaps TO qgr_history_owner,qgr_history_admin;

CREATE FUNCTION qgr_recordings.ingest_scope(machine_id text,key_id text,session_id text,claimed_owner text,token_expiry bigint)
RETURNS TABLE(owner_id text,created_at timestamptz,ended_upper timestamptz,recovery_epoch text,durability text,lifecycle_ready boolean,recovery_pending boolean,retention_days integer,retry_not_before timestamptz,synchronous_ready boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE globally_disabled boolean; session_body jsonb; session_state text; last_projection timestamptz;
BEGIN
    SELECT a.disabled INTO STRICT globally_disabled FROM qgr.access_state a FOR SHARE;
    IF globally_disabled OR token_expiry<=extract(epoch FROM clock_timestamp()) THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    -- Global -> user -> machine -> key -> session. Never invert primary domain lock order.
    PERFORM 1 FROM qgr.users u WHERE u.id=claimed_owner AND NOT u.disabled AND u.directory_enabled IS DISTINCT FROM false FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    PERFORM 1 FROM qgr.machines m WHERE m.id=machine_id AND m.owner=claimed_owner AND m.active_key=key_id AND NOT m.disabled FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    PERFORM 1 FROM qgr.machine_keys k WHERE k.id=key_id AND k.machine=machine_id AND NOT k.revoked FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    SELECT s.body,s.state,s.updated_at INTO session_body,session_state,last_projection FROM qgr.sessions s
      WHERE s.id=session_id AND s.machine=machine_id AND s.owner=claimed_owner FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    IF session_body->>'created_at' IS NULL THEN RAISE EXCEPTION 'missing original session lifetime' USING ERRCODE='22023'; END IF;
    PERFORM pg_advisory_xact_lock(5856751401703146820);
    PERFORM set_config('qgr.machine',machine_id,true),set_config('qgr.owner',claimed_owner,true);
    -- Even a duplicate writes: a new remote-apply commit must cover a prior receipt whose
    -- original COMMIT wait may have been interrupted after local commit.
    UPDATE qgr_recordings.commit_barrier SET touched_at=clock_timestamp() WHERE singleton;
    RETURN QUERY SELECT claimed_owner,(session_body->>'created_at')::timestamptz,
      CASE WHEN session_state IN ('exited','failed','lost','outcome_unknown') THEN last_projection ELSE NULL::timestamptz END,
      c.recovery_epoch,c.durability,c.lifecycle_ready,c.recovery_pending,c.retention_days,c.retry_not_before,
      current_setting('synchronous_standby_names')<>'' AND EXISTS(SELECT 1 FROM pg_stat_replication r WHERE r.state='streaming' AND r.sync_state IN ('sync','quorum'))
    FROM qgr_recordings.state c WHERE c.singleton FOR SHARE OF c;
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.ingest_scope(text,text,text,text,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.ingest_scope(text,text,text,text,bigint) TO qgr_record_ingest;

CREATE FUNCTION qgr_recordings.ensure_month(content_month date) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE partition_name text; phase text; next_month date;
BEGIN
    PERFORM pg_advisory_xact_lock(5856751401703146820);
    IF content_month<>date_trunc('month',content_month::timestamp)::date OR content_month<'1970-01-01'::date
       OR content_month>date_trunc('month',(clock_timestamp() AT TIME ZONE 'UTC')+interval '5 minutes')::date THEN RAISE EXCEPTION 'invalid content month' USING ERRCODE='22023'; END IF;
    SELECT p.state INTO phase FROM qgr_recordings.partitions p WHERE p.month=content_month;
    IF FOUND THEN RETURN phase='active'; END IF;
    partition_name:='content_'||to_char(content_month,'YYYYMM');next_month:=(content_month+interval '1 month')::date;
    EXECUTE format('CREATE TABLE qgr_recordings.%I PARTITION OF qgr_recordings.content FOR VALUES FROM (%L) TO (%L)',partition_name,content_month::text||'T00:00:00Z',next_month::text||'T00:00:00Z');
    INSERT INTO qgr_recordings.partitions VALUES(content_month,partition_name,'active');
    RETURN true;
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.ensure_month(date) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.ensure_month(date) TO qgr_record_ingest;
