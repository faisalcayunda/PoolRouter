#!/bin/sh
# Harness e2e `@api-asli` (ARC-251; spec ui-v2-layar AC-4): UI admin terhadap backend sungguhan.
#
#   make e2e-api-asli CONTAINER_LABEL=arctiva.issue=ARC-<n> [E2E_BUILD_IMAGES=1]
#
# Urutan: env + secret acak di direktori sementara (di luar repo) -> overlay label -> infra ->
# db-bootstrap -> migrate -> seed sintetis -> backend-webserver + frontend -> SQL fixture sesi
# (pengguna seed platform admin, MFA terverifikasi) -> Playwright @api-asli di netns container frontend
# -> pembersihan. Semua container, volume, dan network berlabel arctiva.issue=ARC-<n>; pembersihan
# hanya lewat filter label itu, termasuk saat gagal.
#
# Batas (dicatat, bukan disembunyikan):
#   - controller dan worker tidak dinaikkan: layar provider/secret hanya butuh webserver, Postgres,
#     Redis (pembatas login) dan RabbitMQ (dibaca konfigurasi webserver).
#   - mock-dukcapil tidak dinaikkan (profil `mock`).
#   - login lewat UI (password + MFA TOTP) tidak diuji di sini; pengguna uji masuk lewat baris
#     admin_session dari SQL fixture, sesuai AC-4.
#   - E2E_UPTO=bootstrap menghentikan run setelah db-bootstrap (bukti skala kecil tanpa image aplikasi).
#   - compose.yaml memasang `./../db` untuk db-bootstrap (deploy/db tidak ada); overlay harness menambalnya
#     ke backend/db. Bug di deploy/compose itu di luar ARC-251 dan butuh tiket sendiri.
#   - jangan jalankan bersamaan dengan gate berlabel ARC-251 yang sama: pembersihan menghapus semua
#     resource berlabel tiket.
#   - image backend tidak dibangun otomatis kecuali E2E_BUILD_IMAGES=1: build Rust memakan belasan menit
#     dan tidak lewat kunci build berat.
set -eu

label=${CONTAINER_LABEL:-}
case "$label" in
  arctiva.issue=ARC-[0-9]*) ;;
  *) echo "CONTAINER_LABEL wajib arctiva.issue=ARC-<n> (contoh: make e2e-api-asli CONTAINER_LABEL=arctiva.issue=ARC-251)" >&2; exit 2 ;;
esac
issue=${label#arctiva.issue=}
issue_lc=$(echo "$issue" | tr 'A-Z' 'a-z')
MAKE=${MAKE:-make}
ENGINE=podman
tag=${IMAGE_TAG:-dev}
project="poolrouter-e2e-$issue_lc"
port=${E2E_HTTP_PORT:-18251}
repo=$(cd "$(dirname "$0")/.." && pwd -P)
cd "$repo"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/poolrouter-e2e.XXXXXX")
chmod 700 "$tmp"
secrets="$tmp/secrets"
envf="$tmp/dev.env"
overlay="$tmp/labels.yaml"
pwenv="$tmp/playwright.env"

cleanup() {
  rc=$?
  if [ "$rc" != 0 ]; then
    echo "== DIAG (rc=$rc): container berlabel $issue" >&2
    $ENGINE ps -a --filter "label=$label" --format '{{.Names}} {{.Status}}' >&2 || true
    for c in $($ENGINE ps -a --filter "label=$label" --filter "label=com.docker.compose.service=backend-webserver" -q); do
      $ENGINE logs --tail 40 "$c" >&2 2>&1 || true
    done
  fi
  # Resource berlabel milik tiket ini saja (CLAUDE.md: tanpa prune global).
  $ENGINE rm -f -v --filter "label=$label" >/dev/null 2>&1 || true
  for v in $($ENGINE volume ls --filter "label=$label" -q); do $ENGINE volume rm -f "$v" >/dev/null 2>&1 || true; done
  for n in $($ENGINE network ls --filter "label=$label" -q); do $ENGINE network rm -f "$n" >/dev/null 2>&1 || true; done
  rm -rf -- "$tmp"
  left=$($ENGINE ps -a --filter "label=$label" -q | wc -l | tr -d ' ')
  lv=$($ENGINE volume ls --filter "label=$label" -q | wc -l | tr -d ' ')
  ln=$($ENGINE network ls --filter "label=$label" -q | wc -l | tr -d ' ')
  echo "CLEANUP left_containers=$left left_volumes=$lv left_networks=$ln"
  [ "$left" = 0 ] && [ "$lv" = 0 ] && [ "$ln" = 0 ] || rc=1
  exit "$rc"
}
trap cleanup EXIT INT TERM

# --- Image -------------------------------------------------------------------------------------
have_image() { $ENGINE image exists "$1"; }
be_img="localhost/poolrouter-backend:$tag"
fe_img="localhost/poolrouter-frontend:$tag"
if [ -n "${E2E_BUILD_IMAGES:-}" ]; then
  $MAKE image CONTAINER_ENGINE=$ENGINE IMAGE_TAG="$tag" JOBS="${JOBS:-2}"
  $MAKE image-frontend CONTAINER_ENGINE=$ENGINE IMAGE_TAG="$tag"
fi
# E2E_UPTO=bootstrap: bukti skala kecil tanpa image aplikasi (infra, secret, label, db-bootstrap, pembersihan).
for i in "$be_img" "$fe_img"; do
  [ "${E2E_UPTO:-}" = bootstrap ] && break
  have_image "$i" || { echo "image $i tidak ada; jalankan make image image-frontend atau beri E2E_BUILD_IMAGES=1" >&2; exit 3; }
done

# --- Secret acak dan env (di luar repo, tidak dicetak) ---------------------------------------
mkdir -p "$secrets"; chmod 700 "$secrets"
rnd() { openssl rand -hex 24; }
for f in db_password redis_password rabbitmq_password poolrouter_owner pr_api pr_admin_api pr_controller \
         pr_webhook pr_worker pr_job_balikan pr_job_arsip pr_restore pr_blend \
         poolrouter_nik_hmac_key poolrouter_idempotency_hmac_key poolrouter_seed_password; do
  rnd > "$secrets/$f"
done
openssl rand -hex 32 > "$secrets/poolrouter_kek"
openssl genpkey -algorithm ed25519 > "$secrets/poolrouter_jwt_signing_key"
chmod 444 "$secrets"/*

sed -e "s#^POOLROUTER_SECRETS_DIR=.*#POOLROUTER_SECRETS_DIR=$secrets#" \
    -e "s#^POOLROUTER_UI_ORIGIN=.*#POOLROUTER_UI_ORIGIN=http://127.0.0.1:8080#" \
    -e "s#^POOLROUTER_HTTP_PORT=.*#POOLROUTER_HTTP_PORT=$port#" \
    -e "s#^POOLROUTER_BACKEND_IMAGE=.*#POOLROUTER_BACKEND_IMAGE=$be_img#" \
    -e "s#^POOLROUTER_FRONTEND_IMAGE=.*#POOLROUTER_FRONTEND_IMAGE=$fe_img#" \
    deploy/compose/env/dev.env.example > "$envf"

# --- Overlay label: semua service (termasuk run --rm), volume, dan network ---------------------
{
  echo "services:"
  for s in postgres redis rabbitmq migrate backend-webserver backend-controller backend-worker backend-cli frontend mock-dukcapil; do
    echo "  $s: {labels: {arctiva.issue: $issue}}"
  done
  # db-bootstrap memasang ./../db (deploy/db, tidak ada); skripnya ada di backend/db.
  echo "  db-bootstrap:"
  echo "    labels: {arctiva.issue: $issue}"
  echo "    volumes: !override [\"$repo/backend/db:/db:ro\"]"
  echo "volumes:"
  for v in pg_data redis_data rabbitmq_data; do echo "  $v: {labels: {arctiva.issue: $issue}}"; done
  echo "networks:"
  echo "  default: {labels: {arctiva.issue: $issue}}"
} > "$overlay"

# --- Stack -------------------------------------------------------------------------------------
mk() {
  $MAKE "$@" ENV=dev CONTAINER_ENGINE=$ENGINE ENV_FILE="$envf" PROJECT="$project" EXTRA_COMPOSE_FILES="$overlay"
}
echo "== infra"
mk compose-up COMPOSE_UP_SERVICES="postgres redis rabbitmq"
echo "== db-bootstrap (role per komponen)"
# `compose run` lewat provider yang sama dengan Makefile; awalan diambil dari make -n compose-seed (runbook).
compose_cmd=$(mk -n compose-down | sed -n 's/^\(.*\) --profile bundled-infra down.*$/\1/p' | tail -n 1)
[ -n "$compose_cmd" ] || { echo "gagal menurunkan awalan compose dari make -n" >&2; exit 4; }
eval "$compose_cmd --profile bundled-infra --profile migrate run --rm db-bootstrap"
pg=$($ENGINE ps -q --filter "label=$label" --filter "label=com.docker.compose.service=postgres")
roles=$($ENGINE exec "$pg" sh -c 'psql -U "$POSTGRES_USER" -d postgres -tAc "select count(*) from pg_roles where rolname = '"'"'poolrouter_owner'"'"' or rolname like '"'"'pr\_%'"'"'"')
[ "$roles" = 10 ] || { echo "db-bootstrap: 10 role diharapkan, ada $roles" >&2; exit 4; }
echo "db-bootstrap: $roles role dibuat"
[ "${E2E_UPTO:-}" = bootstrap ] && { echo "E2E_UPTO=bootstrap: berhenti sebelum migrate"; exit 0; }
echo "== migrate"
mk compose-migrate
echo "== seed sintetis"
mk compose-seed
echo "== webserver + frontend"
mk compose-up COMPOSE_UP_SERVICES="backend-webserver frontend"

# --- SQL fixture sesi (AC-4): pengguna seed platform admin dengan MFA terverifikasi -------------
token=$(openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\n')
[ "${#token}" = 43 ] || { echo "token sesi harus 43 karakter" >&2; exit 4; }
hash=$(printf %s "$token" | openssl dgst -sha256 -hex | sed 's/^.*= //')
fe=$($ENGINE ps -q --filter "label=$label" --filter "label=com.docker.compose.service=frontend")
[ -n "$pg" ] && [ -n "$fe" ] || { echo "container postgres atau frontend tidak ditemukan" >&2; exit 4; }
n=$(printf "%s\n" "INSERT INTO admin_session (sesi_id, token_sha256, pengguna_id, kedaluwarsa_at, user_agent, mfa_terverifikasi_at)
 SELECT gen_random_uuid(), decode('$hash','hex'), pengguna_id, now() + interval '8 hours', 'e2e-api-asli', now()
 FROM admin_user WHERE username = 'seed-platform-admin' RETURNING 1;" \
  | $ENGINE exec -i "$pg" sh -c 'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -v ON_ERROR_STOP=1 -tA' | grep -c '^1$' || true)
[ "$n" = 1 ] || { echo "SQL fixture sesi gagal (pengguna seed-platform-admin tidak ada?)" >&2; exit 4; }

umask 077
{ echo "E2E_BASE_URL=http://127.0.0.1:8080"; echo "E2E_SESSION_TOKEN=$token"; } > "$pwenv"

# --- Suite ---------------------------------------------------------------------------------------
echo "== playwright @api-asli"
$MAKE -C frontend e2e-api-asli FRONTEND_RUNNER=$ENGINE CONTAINER_LABEL="$label" E2E_NETNS="$fe" E2E_ENV_FILE="$pwenv"
