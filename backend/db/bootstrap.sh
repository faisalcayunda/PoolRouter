#!/bin/sh
# Create or update the PoolRouter database roles (SHAA-193). Run as a PostgreSQL superuser.
#
#   POOLROUTER_SECRETS_DIR=/path/to/secrets PGHOST=... PGUSER=... PGPASSWORD=... \
#     sh backend/db/bootstrap.sh poolrouter
#
# The secrets directory holds one file per role (poolrouter_owner, pr_api, pr_admin_api,
# pr_controller, pr_worker, pr_webhook). Values are never printed or put on a command line.
set -eu

dbname="${1:?usage: bootstrap.sh <database>}"
: "${POOLROUTER_SECRETS_DIR:?set POOLROUTER_SECRETS_DIR}"

for f in poolrouter_owner pr_api pr_admin_api pr_controller pr_worker pr_webhook; do
  [ -s "$POOLROUTER_SECRETS_DIR/$f" ] || { echo "missing or empty secret file: $f" >&2; exit 2; }
done

here=$(cd "$(dirname "$0")" && pwd)
export POOLROUTER_SECRETS_DIR
exec psql --no-psqlrc --quiet -v ON_ERROR_STOP=1 -v dbname="$dbname" -d postgres -f "$here/bootstrap.sql"
