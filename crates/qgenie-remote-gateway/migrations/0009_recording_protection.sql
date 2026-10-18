-- Protected storage foundation. No application role owns/bypasses these tables.
DO $$ BEGIN
    IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='qgr_record_ingest') THEN CREATE ROLE qgr_record_ingest NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; END IF;
    IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='qgr_history_owner') THEN CREATE ROLE qgr_history_owner NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; END IF;
    IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='qgr_history_admin') THEN CREATE ROLE qgr_history_admin NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; END IF;
    IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='qgr_record_lifecycle') THEN CREATE ROLE qgr_record_lifecycle NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; END IF;
END $$;
CREATE SCHEMA qgr_recordings;
REVOKE ALL ON SCHEMA qgr_recordings FROM PUBLIC;
GRANT USAGE ON SCHEMA qgr_recordings,qgr TO qgr_record_ingest,qgr_history_owner,qgr_history_admin,qgr_record_lifecycle;
GRANT SELECT ON qgr.schema_versions TO qgr_record_ingest,qgr_history_owner,qgr_history_admin,qgr_record_lifecycle;

CREATE TABLE qgr_recordings.receipts (
    machine text NOT NULL,
    session text NOT NULL,
    owner text NOT NULL,
    kind text NOT NULL CHECK(kind IN ('content','gap')),
    id text NOT NULL CHECK(length(id)<=64),
    immutable jsonb NOT NULL CHECK(octet_length(immutable::text)<=8192),
    receipt jsonb NOT NULL CHECK(octet_length(receipt::text)<=8192),
    captured_from timestamptz NOT NULL,
    captured_to timestamptz NOT NULL CHECK(captured_to>=captured_from),
    committed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(machine,session,kind,id),
    FOREIGN KEY(session,machine,owner) REFERENCES qgr.sessions(id,machine,owner)
);
ALTER TABLE qgr_recordings.receipts ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.receipts FORCE ROW LEVEL SECURITY;
CREATE POLICY receipt_internal ON qgr_recordings.receipts TO CURRENT_USER USING(true) WITH CHECK(true);
CREATE POLICY receipt_ingest ON qgr_recordings.receipts TO qgr_record_ingest
    USING(machine=nullif(current_setting('qgr.machine',true),'') AND owner=nullif(current_setting('qgr.owner',true),''))
    WITH CHECK(machine=nullif(current_setting('qgr.machine',true),'') AND owner=nullif(current_setting('qgr.owner',true),''));
CREATE POLICY receipt_owner ON qgr_recordings.receipts FOR SELECT TO qgr_history_owner
    USING(owner=nullif(current_setting('qgr.owner',true),''));
-- Admin content/metadata access is additionally audited before API release. Context alone is
-- not authentication: only the isolated admin service identity receives this role.
CREATE POLICY receipt_admin ON qgr_recordings.receipts FOR SELECT TO qgr_history_admin
    USING(nullif(current_setting('qgr.actor',true),'') IS NOT NULL AND nullif(btrim(current_setting('qgr.reason',true)),'') IS NOT NULL);
GRANT SELECT,INSERT ON qgr_recordings.receipts TO qgr_record_ingest;
GRANT SELECT ON qgr_recordings.receipts TO qgr_history_owner,qgr_history_admin;

CREATE TABLE qgr_recordings.content (
    machine text NOT NULL,
    session text NOT NULL,
    owner text NOT NULL,
    id text NOT NULL CHECK(length(id)<=64),
    captured_from timestamptz NOT NULL,
    captured_to timestamptz NOT NULL CHECK(captured_to>=captured_from),
    payload bytea NOT NULL CHECK(octet_length(payload) BETWEEN 1 AND 65536),
    stored_digest text NOT NULL CHECK(stored_digest ~ '^[0-9a-f]{64}$'),
    PRIMARY KEY(captured_from,machine,session,id),
    FOREIGN KEY(session,machine,owner) REFERENCES qgr.sessions(id,machine,owner)
) PARTITION BY RANGE(captured_from);
ALTER TABLE qgr_recordings.content ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.content FORCE ROW LEVEL SECURITY;
CREATE POLICY content_internal ON qgr_recordings.content TO CURRENT_USER USING(true) WITH CHECK(true);
CREATE POLICY content_ingest ON qgr_recordings.content FOR INSERT TO qgr_record_ingest
    WITH CHECK(machine=nullif(current_setting('qgr.machine',true),'') AND owner=nullif(current_setting('qgr.owner',true),''));
CREATE POLICY content_owner ON qgr_recordings.content FOR SELECT TO qgr_history_owner
    USING(owner=nullif(current_setting('qgr.owner',true),''));
CREATE POLICY content_admin ON qgr_recordings.content FOR SELECT TO qgr_history_admin
    USING(nullif(current_setting('qgr.actor',true),'') IS NOT NULL AND nullif(btrim(current_setting('qgr.reason',true)),'') IS NOT NULL);
GRANT INSERT ON qgr_recordings.content TO qgr_record_ingest;
GRANT SELECT ON qgr_recordings.content TO qgr_history_owner,qgr_history_admin;

CREATE TABLE qgr_recordings.held_content (
    machine text NOT NULL,
    session text NOT NULL,
    owner text NOT NULL,
    id text NOT NULL CHECK(length(id)<=64),
    captured_from timestamptz NOT NULL,
    captured_to timestamptz NOT NULL CHECK(captured_to>=captured_from),
    payload bytea NOT NULL CHECK(octet_length(payload) BETWEEN 1 AND 65536),
    stored_digest text NOT NULL CHECK(stored_digest ~ '^[0-9a-f]{64}$'),
    PRIMARY KEY(machine,session,id),
    FOREIGN KEY(session,machine,owner) REFERENCES qgr.sessions(id,machine,owner)
);
ALTER TABLE qgr_recordings.held_content ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.held_content FORCE ROW LEVEL SECURITY;
CREATE POLICY held_internal ON qgr_recordings.held_content TO CURRENT_USER USING(true) WITH CHECK(true);
CREATE TABLE qgr_recordings.held_cases (
    machine text NOT NULL,
    session text NOT NULL,
    id text NOT NULL,
    hold text NOT NULL CHECK(length(hold) BETWEEN 1 AND 64),
    case_ref text NOT NULL CHECK(length(btrim(case_ref)) BETWEEN 1 AND 256),
    PRIMARY KEY(machine,session,id,hold),
    FOREIGN KEY(machine,session,id) REFERENCES qgr_recordings.held_content(machine,session,id)
);
ALTER TABLE qgr_recordings.held_cases ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.held_cases FORCE ROW LEVEL SECURITY;
CREATE POLICY coverage_internal ON qgr_recordings.held_cases TO CURRENT_USER USING(true) WITH CHECK(true);
CREATE FUNCTION qgr_recordings.held_case_allowed(machine_id text,session_id text,chunk_id text,case_reference text)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT EXISTS(SELECT 1 FROM qgr_recordings.held_cases c WHERE c.machine=machine_id AND c.session=session_id AND c.id=chunk_id AND c.case_ref=case_reference)
$$;
REVOKE ALL ON FUNCTION qgr_recordings.held_case_allowed(text,text,text,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.held_case_allowed(text,text,text,text) TO qgr_history_admin;
-- Only lifecycle functions may establish active case coverage. An arbitrary nonempty case
-- cannot authorize a held row; task 022 updates coverage atomically with hold changes.
CREATE POLICY held_admin ON qgr_recordings.held_content FOR SELECT TO qgr_history_admin
    USING(nullif(current_setting('qgr.actor',true),'') IS NOT NULL AND nullif(btrim(current_setting('qgr.reason',true)),'') IS NOT NULL AND qgr_recordings.held_case_allowed(machine,session,id,nullif(btrim(current_setting('qgr.case',true)),'')));
GRANT SELECT ON qgr_recordings.held_content TO qgr_history_admin;
GRANT INSERT(actor,action,machine,session,metadata) ON qgr.audit_events TO qgr_history_admin,qgr_record_lifecycle;
-- No default partition, public functions, table-owner memberships or lifecycle direct DML.
-- Task 022 owns coordinated partition/hold operations before any real-content release.
