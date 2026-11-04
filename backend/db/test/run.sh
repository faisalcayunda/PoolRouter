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
rb_dir=$(cd "$db_dir/rollback" && pwd)

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
  for r in poolrouter_owner pr_api pr_admin_api pr_controller pr_worker pr_webhook pr_job_balikan pr_job_arsip pr_restore; do
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
# apply_migrations <database> [glob]: every migration (or those matching the glob) as one transaction, as poolrouter_owner
apply_migrations() {
  { echo "SET ROLE poolrouter_owner;"; for f in "$mig_dir"/${2:-*}.sql; do echo "-- $(basename "$f")"; cat "$f"; echo; done; } \
    | psql_su -d "$1" -1
}
apply_migrations poolrouter_test
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

# ---- rollback: F0 -> F1 -> (refused with data) -> F0 must leave the schema identical to F0; F1 must apply again afterwards
echo "==> rollback of the F1 migrations"
psql_su -d postgres -c 'CREATE DATABASE poolrouter_rb'
$ENGINE exec -e POOLROUTER_SECRETS_DIR=/tmp/sec -e PGUSER=postgres -e PGHOST=127.0.0.1 -e PGPASSWORD="$SU_PW" \
  "$NAME" sh /tmp/db/bootstrap.sh poolrouter_rb
dump_schema() { $ENGINE exec -e PGPASSWORD="$SU_PW" "$NAME" pg_dump -U postgres -h 127.0.0.1 -s --no-comments -d poolrouter_rb | grep -v -E '^.(un)?restrict '; }
rollback_all() {
  for f in 4 3 2 1; do
    { echo "SET ROLE poolrouter_owner;"; cat "$rb_dir"/2026110400000${f}_*.down.sql; } | psql_su -d poolrouter_rb -1 || return 1
  done
}
apply_migrations poolrouter_rb '20261103*'
f0_dump=$(dump_schema)
# MF-07: the F1 migration revokes the sessions of platform admins / auditors, and only theirs
psql_su -d poolrouter_rb -c "INSERT INTO admin_user (pengguna_id, username, password_hash) VALUES
    ('00000000-0000-7000-8000-0000000000b1', 'rb-admin', 'x'), ('00000000-0000-7000-8000-0000000000b2', 'rb-plain', 'x');
  INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh) VALUES ('00000000-0000-7000-8000-0000000000b1', 'platform_admin', '00000000-0000-7000-8000-0000000000b1');
  INSERT INTO admin_session (sesi_id, token_sha256, pengguna_id, kedaluwarsa_at) VALUES
    ('00000000-0000-7000-8000-0000000000c1', decode(repeat('01', 32), 'hex'), '00000000-0000-7000-8000-0000000000b1', now() + interval '8 hours'),
    ('00000000-0000-7000-8000-0000000000c2', decode(repeat('02', 32), 'hex'), '00000000-0000-7000-8000-0000000000b2', now() + interval '8 hours')"
apply_migrations poolrouter_rb '20261104*'
[ "$(psql_su -d poolrouter_rb -t -A -c "SELECT string_agg(sesi_id::text, ',') FROM admin_session")" = "00000000-0000-7000-8000-0000000000c2" ] \
  || { echo "FAIL: MF-07 session revocation" >&2; exit 1; }
echo "MF-07: sessions of the platform admin revoked, the other session kept: ok"
psql_su -d poolrouter_rb -c "DELETE FROM admin_session; DELETE FROM admin_user_peran; DELETE FROM admin_user"
# with F1 data the rollback must refuse and leave everything in place
psql_su -d poolrouter_rb -c "INSERT INTO admin_user (pengguna_id, username, password_hash) VALUES ('00000000-0000-7000-8000-0000000000a1', 'rb-user', '\$argon2id\$synthetic');
  INSERT INTO pengguna_mfa (pengguna_id, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, status)
  VALUES ('00000000-0000-7000-8000-0000000000a1', '\\xaa', '\\xbb', '\\xcc', 'kek-1', 'menunggu_konfirmasi')"
if rollback_all 2>/dev/null; then echo "FAIL: rollback ran although MFA data exists" >&2; exit 1; fi
echo "rollback refuses while F1 data exists: ok"
psql_su -d poolrouter_rb -c "DELETE FROM pengguna_mfa; DELETE FROM admin_user WHERE username = 'rb-user'"
rollback_all
f0_after=$(dump_schema)
[ "$f0_dump" = "$f0_after" ] || { echo "FAIL: schema after rollback differs from F0" >&2; printf '%s\n' "$f0_dump" > "$here/../f0.before"; printf '%s\n' "$f0_after" > "$here/../f0.after"; diff "$here/../f0.before" "$here/../f0.after" >&2; exit 1; }
echo "rollback leaves the F0 schema identical: ok"
apply_migrations poolrouter_rb '20261104*'
echo "F1 applies again after rollback: ok"
echo "DB TESTS PASSED"
