#!/bin/sh
# Loads the K-20 fixture (k20.sql, SHAA-255) into a throwaway PostgreSQL 17 container and verifies it.
# Order: bootstrap roles -> migrations from empty -> 05_fixtures.sql -> k20.sql -> k20_verify.sql.
# The container is labelled shaal.issue=<SHAA_ISSUE> and removed on exit.
#
#   SHAA_ISSUE=SHAA-255 sh backend/db/test/k20_load.sh
#   CONTAINER_ENGINE=docker SHAA_ISSUE=SHAA-255 sh backend/db/test/k20_load.sh
set -eu

ENGINE=${CONTAINER_ENGINE:-podman}
ISSUE=${SHAA_ISSUE:?set SHAA_ISSUE to the issue this run belongs to}
IMAGE=${PG_IMAGE:-docker.io/library/postgres:17-alpine@sha256:266d3c257594e195b309ae9d1805e54c788f1d7b375f56bf6c367aac6c917e7b}
NAME="poolrouter-k20-$$"
here=$(cd "$(dirname "$0")" && pwd)
db_dir=$(cd "$here/.." && pwd)
mig_dir=$(cd "$db_dir/../migrations" && pwd)

cleanup() {
  $ENGINE rm -f -v "$NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

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
psql_su -d postgres -c 'CREATE DATABASE poolrouter_test'
$ENGINE exec -e POOLROUTER_SECRETS_DIR=/tmp/sec -e PGUSER=postgres -e PGHOST=127.0.0.1 -e PGPASSWORD="$SU_PW" \
  "$NAME" sh /tmp/db/bootstrap.sh poolrouter_test
echo "bootstrap: ok"

echo "==> migrations from empty"
{ echo "SET ROLE poolrouter_owner;"; for f in "$mig_dir"/*.sql; do echo "-- $(basename "$f")"; cat "$f"; echo; done; } \
  | psql_su -d poolrouter_test -1
echo "migrations: ok"

echo "==> base fixtures (05_fixtures.sql)"
psql_su -d poolrouter_test -f - < "$here/05_fixtures.sql" >/dev/null
echo "base fixtures: ok"

echo "==> K-20 fixture (k20.sql)"
psql_su -d poolrouter_test -f - < "$here/k20.sql" >/dev/null
echo "k20 fixture: loaded"

echo "==> K-20 verification (k20_verify.sql)"
psql_su -d poolrouter_test -f - < "$here/k20_verify.sql"
echo "K-20 FIXTURE VERIFIED"
