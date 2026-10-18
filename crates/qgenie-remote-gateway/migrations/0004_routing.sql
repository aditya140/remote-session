ALTER TABLE qgr.machines ADD CONSTRAINT machine_owner_identity UNIQUE(id,owner);
CREATE TABLE qgr.machine_routes (
    machine text PRIMARY KEY REFERENCES qgr.machines(id),
    generation numeric(20,0) NOT NULL CHECK(generation BETWEEN 1 AND 18446744073709551615),
    node text NOT NULL CHECK(length(node)<=128),
    connection text NOT NULL CHECK(length(connection)<=64),
    activated boolean NOT NULL DEFAULT false,
    agents jsonb NOT NULL DEFAULT '{}' CHECK(octet_length(agents::text)<=65536),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE qgr.sessions (
    id text PRIMARY KEY CHECK(length(id)<=64),
    machine text NOT NULL,
    owner text NOT NULL,
    body jsonb NOT NULL CHECK(octet_length(body::text)<=262144),
    state text NOT NULL CHECK(length(state)<=32),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY(machine,owner) REFERENCES qgr.machines(id,owner)
);
CREATE INDEX sessions_machine ON qgr.sessions(machine,state);
GRANT SELECT,INSERT,UPDATE ON qgr.machine_routes,qgr.sessions TO qgr_app;
