#!/bin/sh
set -eu
umask 077
export PGPASSFILE=/run/qgr/replication.pgpass
if [ ! -s "$PGDATA/PG_VERSION" ]; then
    pg_basebackup --host=postgres --username=qgr_replication --pgdata="$PGDATA" \
        --wal-method=stream --write-recovery-conf --no-password --checkpoint=fast
    # This belongs only to the owned development volume. It is never a restore
    # policy or a production promotion decision.
    printf "primary_conninfo = 'host=postgres port=5432 user=qgr_replication application_name=qgr_development_standby passfile=/run/qgr/replication.pgpass sslmode=disable'\n" >> "$PGDATA/postgresql.auto.conf"
fi
exec postgres -c hot_standby=on -c max_connections=192
