CREATE TABLE IF NOT EXISTS terminal_checkpoints (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id),
    seq BLOB NOT NULL CHECK(length(seq)=8),
    data BLOB NOT NULL CHECK(length(data)<=8388608)
);
PRAGMA user_version=2;
