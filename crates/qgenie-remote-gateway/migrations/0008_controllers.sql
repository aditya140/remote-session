ALTER TABLE qgr.sessions ADD CONSTRAINT session_owner_machine_identity UNIQUE(id,machine,owner);
CREATE TABLE qgr.control_requests (
    owner text NOT NULL,
    holder text NOT NULL CHECK(length(holder)<=64),
    request text NOT NULL CHECK(length(request)<=64),
    session text NOT NULL,
    machine text NOT NULL,
    host_generation numeric(20,0) NOT NULL CHECK(host_generation BETWEEN 1 AND 18446744073709551615),
    host_connection text NOT NULL CHECK(length(host_connection)<=64),
    generation numeric(20,0) NOT NULL CHECK(generation BETWEEN 1 AND 18446744073709551615),
    takeover boolean NOT NULL,
    expires_at timestamptz NOT NULL,
    PRIMARY KEY(owner,holder,request),
    UNIQUE(session,generation),
    FOREIGN KEY(session,machine,owner) REFERENCES qgr.sessions(id,machine,owner)
);
CREATE TABLE qgr.control_leases (
    session text PRIMARY KEY,
    owner text NOT NULL,
    machine text NOT NULL,
    holder text CHECK(length(holder)<=64),
    request text CHECK(length(request)<=64),
    activation text CHECK(length(activation)<=64),
    host_generation numeric(20,0) NOT NULL CHECK(host_generation BETWEEN 1 AND 18446744073709551615),
    host_connection text NOT NULL CHECK(length(host_connection)<=64),
    generation numeric(20,0) NOT NULL DEFAULT 0 CHECK(generation BETWEEN 0 AND 18446744073709551615),
    expires_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    activated boolean NOT NULL DEFAULT false,
    audited boolean NOT NULL DEFAULT false,
    CHECK(holder IS NULL OR (request IS NOT NULL AND activation IS NOT NULL AND generation>0)),
    FOREIGN KEY(session,machine,owner) REFERENCES qgr.sessions(id,machine,owner),
    FOREIGN KEY(owner,holder,request) REFERENCES qgr.control_requests(owner,holder,request)
);
GRANT SELECT,INSERT ON qgr.control_requests TO qgr_app;
GRANT SELECT,INSERT,UPDATE ON qgr.control_leases TO qgr_app;
