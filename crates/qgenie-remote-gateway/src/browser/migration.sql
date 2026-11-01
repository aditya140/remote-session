-- Browser sessions are shared across gateways. Tokens and PKCE verifiers are AEAD
-- encrypted by a private key outside PostgreSQL. Cookie values are stored hashed.
CREATE TABLE qgr.browser_sessions (
    id_hash text PRIMARY KEY CHECK(length(id_hash)=64),
    encrypted_token bytea NOT NULL CHECK(octet_length(encrypted_token) BETWEEN 29 AND 65564),
    csrf text NOT NULL CHECK(length(csrf)=43),
    owner text NOT NULL REFERENCES qgr.users(id),
    expires_at bigint NOT NULL CHECK(expires_at>0),
    created_at bigint NOT NULL CHECK(created_at>0)
);
CREATE INDEX browser_sessions_expiry ON qgr.browser_sessions(expires_at);
CREATE TABLE qgr.browser_login_states (
    state_hash text PRIMARY KEY CHECK(length(state_hash)=64),
    browser_hash text NOT NULL CHECK(length(browser_hash)=64),
    encrypted_verifier bytea NOT NULL CHECK(octet_length(encrypted_verifier) BETWEEN 29 AND 1024),
    expires_at bigint NOT NULL CHECK(expires_at>0)
);
CREATE INDEX browser_login_states_expiry ON qgr.browser_login_states(expires_at);
GRANT SELECT,INSERT,DELETE ON qgr.browser_sessions,qgr.browser_login_states TO qgr_app;

-- Console metadata reads use the existing append-only audit authority. These
-- indexes add no new reader privileges and avoid searching unrelated history.
CREATE INDEX console_audit_activation_lookup
    ON qgr.audit_events(machine,event_id DESC) INCLUDE(at)
    WHERE action='host_activated';
CREATE INDEX console_audit_activity_window
    ON qgr.audit_events(action,at DESC,event_id DESC)
    WHERE action IN ('machine_enrolled','session_started');
