-- Native request identity is immutable. One winner, audit and outbox commit together.
CREATE TABLE qgr.approval_requests (
    id text PRIMARY KEY CHECK(length(id)<=64),
    owner text NOT NULL,
    machine text NOT NULL,
    session text NOT NULL REFERENCES qgr.sessions(id),
    native_id text NOT NULL CHECK(octet_length(native_id) BETWEEN 1 AND 512),
    request_hash text NOT NULL CHECK(request_hash ~ '^[0-9a-f]{64}$'),
    policy_revision numeric(20,0) NOT NULL CHECK(policy_revision BETWEEN 1 AND 18446744073709551615),
    body jsonb NOT NULL CHECK(octet_length(body::text)<=16384),
    expires_at timestamptz,
    state text NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','cancelled','timed_out')),
    reason text CHECK(octet_length(reason)<=1024),
    FOREIGN KEY(machine,owner) REFERENCES qgr.machines(id,owner),
    UNIQUE(session,native_id)
);
CREATE INDEX approval_owner_pending ON qgr.approval_requests(owner,state,id);
CREATE TABLE qgr.approval_decisions (
    approval text PRIMARY KEY REFERENCES qgr.approval_requests(id),
    id text NOT NULL UNIQUE CHECK(length(id)<=64),
    owner text NOT NULL,
    client text NOT NULL CHECK(length(client)<=64),
    body jsonb NOT NULL CHECK(octet_length(body::text)<=2048),
    committed_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE qgr.approval_outbox (
    decision text PRIMARY KEY REFERENCES qgr.approval_decisions(id),
    approval text NOT NULL UNIQUE REFERENCES qgr.approval_requests(id),
    due_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    claim text CHECK(length(claim)<=64),
    claim_until timestamptz,
    attempts integer NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 1000000),
    outcome text CHECK(outcome IN ('applied','already_applied','cancelled','outcome_unknown')),
    reason text CHECK(octet_length(reason)<=1024),
    CHECK((claim IS NULL)=(claim_until IS NULL))
);
CREATE INDEX approval_delivery_due ON qgr.approval_outbox(due_at,claim_until) WHERE outcome IS NULL;
REVOKE ALL ON qgr.approval_requests,qgr.approval_decisions,qgr.approval_outbox FROM PUBLIC,qgr_audit;
GRANT SELECT,INSERT,UPDATE ON qgr.approval_requests,qgr.approval_outbox TO qgr_app;
-- Winners are immutable even to the runtime identity.
GRANT SELECT,INSERT ON qgr.approval_decisions TO qgr_app;
