BEGIN IMMEDIATE;
CREATE TABLE bindings(origin TEXT NOT NULL,machine TEXT NOT NULL,owner TEXT NOT NULL,PRIMARY KEY(origin,machine));
CREATE TABLE sources(session TEXT PRIMARY KEY,body TEXT NOT NULL CHECK(length(body)<=8192),active INTEGER NOT NULL CHECK(active IN (0,1)));
CREATE TABLE requests(
    id TEXT PRIMARY KEY,session TEXT NOT NULL REFERENCES sources(session),
    source TEXT NOT NULL CHECK(length(source)<=8192),native_id TEXT NOT NULL CHECK(length(native_id)<=512),
    body TEXT NOT NULL CHECK(length(CAST(body AS BLOB))<=16384),
    state TEXT NOT NULL CHECK(state IN ('pending','offered','done','cancelled')),
    winner TEXT CHECK(length(winner)<=2048),outcome TEXT CHECK(outcome IN ('"applied"','"already_applied"','"cancelled"','"outcome_unknown"')),
    reason TEXT CHECK(length(reason)<=1024),UNIQUE(session,native_id)
);
PRAGMA user_version=1;
COMMIT;
