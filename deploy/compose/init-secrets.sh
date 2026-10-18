#!/bin/sh
set -eu
umask 077
# Runs only in the development bootstrap container with no network. Dedicated
# output volumes prevent an edge/identity process from reading database credentials.
for qgr_scope in gateway edge identity redis bouncer migrate jobs postgres; do
    qgr_target="/generated/$qgr_scope"
    mkdir -p "$qgr_target"
    for qgr_file in "/input/$qgr_scope/"*; do
        [ -f "$qgr_file" ] || continue
        qgr_name=${qgr_file##*/}
        if [ -e "$qgr_target/$qgr_name" ]; then
            cmp -s "$qgr_file" "$qgr_target/$qgr_name" || {
                echo 'Existing development secrets differ; refusing an implicit credential rotation.' >&2
                exit 1
            }
        else
            cp "$qgr_file" "$qgr_target/$qgr_name"
        fi
        chmod 0400 "$qgr_target/$qgr_name"
    done
    qgr_owner=10001
    [ "$qgr_scope" != postgres ] || qgr_owner=70
    chown -R "$qgr_owner:$qgr_owner" "$qgr_target"
    chmod 0700 "$qgr_target"
done
for qgr_data in /pg-primary /pg-standby; do
    chown 70:70 "$qgr_data"
    chmod 0700 "$qgr_data"
done
