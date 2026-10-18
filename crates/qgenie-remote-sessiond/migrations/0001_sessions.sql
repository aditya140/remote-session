CREATE TABLE IF NOT EXISTS machine (singleton INTEGER PRIMARY KEY CHECK(singleton=1), id TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL,
    spec TEXT NOT NULL,
    state TEXT NOT NULL,
    created_ms TEXT NOT NULL,
    head BLOB NOT NULL,
    generation BLOB NOT NULL,
    exit_code INTEGER
);
CREATE TABLE IF NOT EXISTS replay (
    session_id TEXT NOT NULL REFERENCES sessions(id),
    seq BLOB NOT NULL,
    event TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    PRIMARY KEY(session_id,seq)
);
CREATE TABLE IF NOT EXISTS input_ack (
    session_id TEXT NOT NULL REFERENCES sessions(id),
    generation BLOB NOT NULL,
    seq BLOB NOT NULL,
    digest TEXT NOT NULL,
    outcome TEXT NOT NULL,
    PRIMARY KEY(session_id,generation,seq)
);
PRAGMA user_version=1;
