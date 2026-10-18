-- Run only with the dedicated migration credential. App/admin readers never own tables.
DO $$ BEGIN
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='qgr_app') THEN
        CREATE ROLE qgr_app NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;
    END IF;
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='qgr_audit') THEN
        CREATE ROLE qgr_audit NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;
    END IF;
END $$;
REVOKE ALL ON SCHEMA qgr FROM PUBLIC;
GRANT USAGE ON SCHEMA qgr TO qgr_app,qgr_audit;
GRANT SELECT ON qgr.schema_versions TO qgr_app,qgr_audit;
CREATE SEQUENCE qgr.audit_event_ids AS bigint;
CREATE TABLE qgr.audit_events (
    event_id bigint PRIMARY KEY,
    at timestamptz NOT NULL DEFAULT clock_timestamp(),
    actor text NOT NULL CHECK (length(actor)<=64),
    action text NOT NULL CHECK (length(action)<=64),
    machine text CHECK (length(machine)<=64),
    session text CHECK (length(session)<=64),
    metadata jsonb NOT NULL CHECK (octet_length(metadata::text)<=4096)
);
-- Assign the cursor after acquiring this short transaction lock. Thus committed
-- audit IDs are ordered: a higher committed cursor cannot skip an earlier transaction.
CREATE FUNCTION qgr.stamp_audit() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path=pg_catalog AS $$ BEGIN
    IF NEW.event_id IS NOT NULL THEN RAISE EXCEPTION 'audit cursor is database assigned'; END IF;
    PERFORM pg_advisory_xact_lock(5856751401703146817);
    NEW.event_id := nextval('qgr.audit_event_ids');
    NEW.at := clock_timestamp();
    RETURN NEW;
END $$;
CREATE TRIGGER stamp_audit BEFORE INSERT ON qgr.audit_events FOR EACH ROW EXECUTE FUNCTION qgr.stamp_audit();
CREATE FUNCTION qgr.reject_audit_rewrite() RETURNS trigger LANGUAGE plpgsql
SET search_path=pg_catalog AS $$ BEGIN RAISE EXCEPTION 'audit history is append only'; END $$;
CREATE TRIGGER immutable_audit BEFORE UPDATE OR DELETE OR TRUNCATE ON qgr.audit_events
FOR EACH STATEMENT EXECUTE FUNCTION qgr.reject_audit_rewrite();
REVOKE ALL ON qgr.audit_events FROM PUBLIC,qgr_app,qgr_audit;
REVOKE ALL ON qgr.audit_event_ids FROM PUBLIC,qgr_app,qgr_audit;
REVOKE ALL ON FUNCTION qgr.stamp_audit(),qgr.reject_audit_rewrite() FROM PUBLIC;
GRANT INSERT (actor,action,machine,session,metadata) ON qgr.audit_events TO qgr_app,qgr_audit;
GRANT SELECT ON qgr.audit_events TO qgr_audit;
