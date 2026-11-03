#!/bin/sh
# Database tests for the F0 schema (SHAA-193): roles bootstrap, migrations from empty, role-grant matrix, constraints.
# Everything runs inside a throwaway PostgreSQL 17 container labelled shaal.issue=<SHAA_ISSUE>; only that container is removed.
#
#   sh backend/db/test/run.sh                       # podman
#   CONTAINER_ENGINE=docker sh backend/db/test/run.sh
set -eu

ENGINE=${CONTAINER_ENGINE:-podman}
ISSUE=${SHAA_ISSUE:-SHAA-193}
# Same pinned image as deploy/compose/compose.yaml (never `latest`).
IMAGE=${PG_IMAGE:-docker.io/library/postgres:17-alpine@sha256:266d3c257594e195b309ae9d1805e54c788f1d7b375f56bf6c367aac6c917e7b}
NAME="poolrouter-dbtest-$$"
here=$(cd "$(dirname "$0")" && pwd)
db_dir=$(cd "$here/.." && pwd)
mig_dir=$(cd "$db_dir/../migrations" && pwd)

cleanup() {
  $ENGINE rm -f -v "$NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

# Superuser password is random and lives only in this container's environment.
SU_PW=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
$ENGINE run -d --name "$NAME" --label "shaal.issue=$ISSUE" \
  -e POSTGRES_PASSWORD="$SU_PW" -e POSTGRES_DB=postgres \
  --tmpfs /var/lib/postgresql/data:rw "$IMAGE" >/dev/null

i=0
until $ENGINE exec "$NAME" pg_isready -U postgres -h 127.0.0.1 >/dev/null 2>&1; do
  i=$((i + 1))
  [ "$i" -le 60 ] || { echo "postgres did not become ready" >&2; exit 1; }
  sleep 1
done
# The image restarts once after init; wait until it is stable.
sleep 2
until $ENGINE exec "$NAME" pg_isready -U postgres -h 127.0.0.1 >/dev/null 2>&1; do sleep 1; done

psql_su() { $ENGINE exec -i "$NAME" psql -U postgres -h 127.0.0.1 --no-psqlrc -q -v ON_ERROR_STOP=1 "$@"; }

echo "==> secrets and role bootstrap"
$ENGINE exec "$NAME" sh -c '
  umask 077; mkdir -p /tmp/sec
  for r in poolrouter_owner pr_api pr_admin_api pr_controller pr_worker pr_webhook; do
    od -An -N18 -tx1 /dev/urandom | tr -d " \n" > /tmp/sec/$r
  done'
$ENGINE exec "$NAME" mkdir -p /tmp/db
$ENGINE cp "$db_dir/bootstrap.sql" "$NAME:/tmp/db/bootstrap.sql"
$ENGINE cp "$db_dir/bootstrap.sh" "$NAME:/tmp/db/bootstrap.sh"
# compose creates POSTGRES_DB the same way; the bootstrap then hands it to poolrouter_owner
psql_su -d postgres -c 'CREATE DATABASE poolrouter_test'
$ENGINE exec -e POOLROUTER_SECRETS_DIR=/tmp/sec -e PGUSER=postgres -e PGHOST=127.0.0.1 -e PGPASSWORD="$SU_PW" \
  "$NAME" sh /tmp/db/bootstrap.sh poolrouter_test
# the bootstrap is idempotent
$ENGINE exec -e POOLROUTER_SECRETS_DIR=/tmp/sec -e PGUSER=postgres -e PGHOST=127.0.0.1 -e PGPASSWORD="$SU_PW" \
  "$NAME" sh /tmp/db/bootstrap.sh poolrouter_test
echo "bootstrap: ok (run twice)"

echo "==> migrations from empty"
apply_migrations() {
  { echo "SET ROLE poolrouter_owner;"; for f in "$mig_dir"/*.sql; do echo "-- $(basename "$f")"; cat "$f"; echo; done; } \
    | psql_su -d poolrouter_test -1
}
apply_migrations
echo "migrations: ok"

echo "==> assertions"
for t in "$here"/0*.sql; do
  echo "--- $(basename "$t")"
  psql_su -d poolrouter_test -t -A -f - < "$t" >/dev/null
done
awk 'NF==2 {printf "INSERT INTO t.kolom_contract VALUES (\047%s\047, \047%s\047);\n", $1, $2}' "$here/kolom-contract.txt" \
  | psql_su -d poolrouter_test -f -
for t in "$here"/[1-9]*.sql; do
  echo "--- $(basename "$t")"
  psql_su -d poolrouter_test -t -A -f - < "$t" >/dev/null
done
echo "DB TESTS PASSED"
