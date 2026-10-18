CREATE TABLE qgr.users (
    id text PRIMARY KEY CHECK(length(id)<=64),
    issuer text NOT NULL CHECK(length(issuer)<=2048),
    tenant text NOT NULL CHECK(length(tenant)<=64),
    subject text NOT NULL CHECK(length(subject)<=256),
    disabled boolean NOT NULL DEFAULT false,
    directory_checked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(issuer,tenant,subject)
);
CREATE TABLE qgr.access_state (
    singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton),
    disabled boolean NOT NULL DEFAULT false,
    revision numeric(20,0) NOT NULL DEFAULT 1 CHECK(revision BETWEEN 1 AND 18446744073709551615)
);
INSERT INTO qgr.access_state(singleton) VALUES(true);
CREATE TABLE qgr.machines (
    id text PRIMARY KEY CHECK(length(id)<=64),
    owner text NOT NULL REFERENCES qgr.users(id),
    installation text NOT NULL CHECK(length(installation)<=64),
    os_user text NOT NULL CHECK(length(os_user) BETWEEN 1 AND 128),
    active_key text NOT NULL CHECK(length(active_key)<=64),
    hostname text NOT NULL CHECK(length(hostname) BETWEEN 1 AND 253),
    name text NOT NULL CHECK(length(name) BETWEEN 1 AND 128),
    os text NOT NULL CHECK(length(os)<=32),
    arch text NOT NULL CHECK(length(arch)<=32),
    cli_version text NOT NULL CHECK(length(cli_version)<=64),
    labels jsonb NOT NULL DEFAULT '{}',
    disabled boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    last_seen timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(owner,installation)
);
CREATE TABLE qgr.machine_keys (
    id text PRIMARY KEY CHECK(length(id)<=64),
    machine text NOT NULL REFERENCES qgr.machines(id),
    public_key bytea NOT NULL UNIQUE CHECK(octet_length(public_key)=32),
    revoked boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE qgr.machine_challenges (
    id text PRIMARY KEY CHECK(length(id)<=64),
    machine text NOT NULL CHECK(length(machine)<=64),
    key text NOT NULL CHECK(length(key)<=64),
    owner text NOT NULL REFERENCES qgr.users(id),
    kind text NOT NULL CHECK(kind IN ('enroll','recover','auth')),
    payload jsonb NOT NULL CHECK(octet_length(payload::text)<=16384),
    issued_at_ms bigint NOT NULL CHECK(issued_at_ms>=0),
    expires_at_ms bigint NOT NULL CHECK(expires_at_ms>issued_at_ms AND expires_at_ms-issued_at_ms<=60000),
    consumed boolean NOT NULL DEFAULT false
);
CREATE INDEX challenge_expiry ON qgr.machine_challenges(expires_at_ms);
CREATE INDEX challenge_per_owner ON qgr.machine_challenges(owner,expires_at_ms);
CREATE INDEX challenge_per_machine ON qgr.machine_challenges(machine,expires_at_ms);
GRANT SELECT,INSERT,UPDATE ON qgr.users,qgr.machines,qgr.machine_keys TO qgr_app;
GRANT SELECT,UPDATE ON qgr.access_state TO qgr_app;
GRANT SELECT,INSERT,UPDATE,DELETE ON qgr.machine_challenges TO qgr_app;
