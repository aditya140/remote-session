BEGIN IMMEDIATE;
CREATE TABLE streams (
    epoch TEXT PRIMARY KEY,
    identity TEXT NOT NULL,
    frontier BLOB NOT NULL CHECK(length(frontier)=8),
    captured_at TEXT NOT NULL,
    sealed INTEGER NOT NULL DEFAULT 0 CHECK(sealed IN (0,1))
);
CREATE TABLE reservations (
    epoch TEXT NOT NULL REFERENCES streams(epoch),
    seq BLOB NOT NULL CHECK(length(seq)=8),
    captured_at TEXT NOT NULL,
    PRIMARY KEY(epoch,seq)
);
CREATE TABLE chunks (
    position INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    epoch TEXT NOT NULL REFERENCES streams(epoch),
    metadata TEXT NOT NULL,
    bytes INTEGER NOT NULL CHECK(bytes>0)
);
CREATE TABLE gaps (
    position INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    epoch TEXT NOT NULL REFERENCES streams(epoch),
    metadata TEXT NOT NULL,
    offered INTEGER NOT NULL DEFAULT 0 CHECK(offered IN (0,1))
);
CREATE TABLE warnings (singleton INTEGER PRIMARY KEY CHECK(singleton=1),reason TEXT NOT NULL);
CREATE TABLE allocation (data BLOB);
CREATE TABLE settings (key TEXT PRIMARY KEY,value INTEGER NOT NULL);
PRAGMA user_version=1;
COMMIT;
