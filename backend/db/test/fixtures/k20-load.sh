#!/bin/sh
# Rebuild the synthetic K-20 test dataset in a throwaway PostgreSQL 17 container (SHAA-204).
#
#   sh backend/db/test/fixtures/k20-load.sh
#
# Steps: start container -> bootstrap roles -> migrations -> 05_fixtures.sql -> k20.sql -> argon2 hashes -> verify.
# Requires POOLROUTER_TEST_PASSWORD in the environment (name only is passed to the container).
# The container is labelled shaal.issue=SHAA-204 and removed on exit. Nothing is installed on the host.
set -eu

# Password for the synthetic K-20 admin logins (ui-null, ui-en). Passed by env NAME only; never
# written to argv, logs, or the repo. Set it temporarily in the shell for the run.
if [ -z "${POOLROUTER_TEST_PASSWORD:-}" ]; then
  echo "POOLROUTER_TEST_PASSWORD is not set; refusing to load K-20 fixtures" >&2
  exit 2
fi
ARGON2_APK=argon2=20190702-r5

ENGINE=${CONTAINER_ENGINE:-podman}
ISSUE=SHAA-204
IMAGE=${PG_IMAGE:-docker.io/library/postgres:17-alpine@sha256:266d3c257594e195b309ae9d1805e54c788f1d7b375f56bf6c367aac6c917e7b}
NAME="poolrouter-k20-$$"
here=$(cd "$(dirname "$0")" && pwd)
test_dir=$(cd "$here/.." && pwd)
db_dir=$(cd "$test_dir/.." && pwd)
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

{ echo "SET ROLE poolrouter_owner;"; for f in "$mig_dir"/*.sql; do echo "-- $(basename "$f")"; cat "$f"; echo; done; } \
  | psql_su -d poolrouter_test -1
echo "migrations: ok"

psql_su -d poolrouter_test -f - < "$test_dir/05_fixtures.sql"
echo "fixtures 05: ok"
psql_su -d poolrouter_test -f - < "$here/k20.sql"
echo "fixtures k20: ok"

# Replace the placeholder hashes of the two K-20 UI logins with real argon2id hashes made inside
# the throwaway container. The password comes from the env var name only (-e NAME, no value in argv).
$ENGINE exec "$NAME" apk add --no-cache "$ARGON2_APK" >/dev/null
hash=$($ENGINE exec -e POOLROUTER_TEST_PASSWORD "$NAME" sh -c '
  salt=$(od -An -N16 -tx1 /dev/urandom | tr -d " \n")
  printf "%s" "$POOLROUTER_TEST_PASSWORD" | argon2 "$salt" -id -e')
case "$hash" in '$argon2id$'*) ;; *) echo "argon2 hash generation failed" >&2; exit 1 ;; esac
printf "UPDATE admin_user SET password_hash = '%s' WHERE username IN ('ui-null','ui-en');\n" "$hash" \
  | psql_su -d poolrouter_test -f -
echo "admin hashes: ok"

echo "==> verify"
psql_su -d poolrouter_test -t -A -f - <<'SQL'
DO $$
DECLARE n integer;
BEGIN
  SELECT count(*) INTO n FROM client WHERE nama IN ('client-c','client-e','client-d','client-u','client-p','client-kedaluwarsa','client-bersama','client-x');
  IF n <> 8 THEN RAISE EXCEPTION 'client count % <> 8', n; END IF;
  SELECT count(*) INTO n FROM client_endpoint ce JOIN client c USING (client_id) WHERE c.nama IN ('client-c','client-e','client-d','client-u','client-p','client-kedaluwarsa','client-bersama','client-x');
  IF n < 8 THEN RAISE EXCEPTION 'route count % < 8', n; END IF;
  SELECT count(*) INTO n FROM client_kredensial WHERE client_id = '10000000-0000-7000-8000-000000000001' AND status = 'aktif';
  IF n <> 2 THEN RAISE EXCEPTION 'client-x active credentials % <> 2 (K1, K2)', n; END IF;
  SELECT count(*) INTO n FROM client_kredensial WHERE kedaluwarsa_at < now();
  IF n <> 1 THEN RAISE EXCEPTION 'expired credentials % <> 1', n; END IF;
  SELECT count(*) INTO n FROM request WHERE status = 'queued' AND referensi_client LIKE 'SINTETIS-K20-7-%';
  IF n <> 5 THEN RAISE EXCEPTION 'queued K20-7 requests % <> 5', n; END IF;
  SELECT count(*) INTO n FROM upstream_pool WHERE provider_id = '20000000-0000-7000-8000-000000000018';
  IF n <> 2 THEN RAISE EXCEPTION 'shared provider pools % <> 2', n; END IF;
  SELECT count(*) INTO n FROM upstream_pool WHERE batas_percobaan <> 3 AND nama LIKE 'up-%' AND nama NOT IN ('up-1');
  IF n <> 0 THEN RAISE EXCEPTION 'K-20 pools with batas_percobaan <> 3: %', n; END IF;
  SELECT count(*) INTO n FROM admin_user WHERE bahasa_ui IS NULL AND username = 'ui-null';
  IF n <> 1 THEN RAISE EXCEPTION 'ui-null user missing'; END IF;
  SELECT count(*) INTO n FROM admin_user WHERE username IN ('ui-null','ui-en') AND password_hash LIKE '$argon2id$synthetic%';
  IF n <> 0 THEN RAISE EXCEPTION 'placeholder hash left on K-20 login users'; END IF;
  SELECT count(*) INTO n FROM client_webhook WHERE webhook_url !~ '^https://hook\.example\.test/';
  IF n <> 0 THEN RAISE EXCEPTION 'non-synthetic webhook host found: %', n; END IF;
  RAISE NOTICE 'K-20 fixture verify: ok';
END $$;
SELECT 'K-20 verify: ok';
SQL

echo "K-20 FIXTURES LOADED AND VERIFIED"
