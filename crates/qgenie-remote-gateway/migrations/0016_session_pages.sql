-- Bound history traversal independently from the number of completed sessions.
CREATE INDEX sessions_owner_page ON qgr.sessions(owner,machine,id);
CREATE INDEX sessions_owner_live_page ON qgr.sessions(owner,machine,id)
WHERE state IN ('accepted','launching','running','idle');
