#!/bin/sh
# Harness `make smoke` (ARC-378; spec KB products/poolrouter/specs/smoke-test/spec.md bagian 3).
#
#   make smoke CONTAINER_LABEL=arctiva.issue=ARC-<n> [CONTAINER_ENGINE=docker]
#
# Urutan: validasi label/env -> secret + env (scripts/stack-berlabel.sh) -> overlay label +
# overlay mock-tls -> infra -> db-bootstrap -> migrate -> seed sintetis (pengguna + dukcapil) ->
# profil mock + backend-worker -> sidecar mock-tls (CA per run) -> fixture SQL sesi/MFA -> G0
# (proses) -> Playwright `@smoke` (project api, lalu ui) -> smoke-report.json -> pembersihan.
#
# Semua resource berlabel arctiva.issue=ARC-<n>; pembersihan hanya lewat filter label, termasuk
# saat gagal (scripts/stack-berlabel.sh: sb_cleanup_trap, exit non-nol bila CLEANUP left_* != 0).
#
# Batas (dicatat, bukan disembunyikan):
#   - make smoke boleh merah di main sampai S3 (ARC-379) merge: smoke-cakupan gagal sengaja karena
#     scripts/smoke/cakupan-rute.mjs (milik S3) belum ada (spec bagian 4). Langkah ini (smoke.sh)
#     tidak dipanggil sampai smoke-cakupan lulus (lihat target `smoke` di Makefile).
#   - hanya POOLROUTER_ENV=dev didukung (fixture SQL dan backend-cli seed menolak lingkungan lain).
#   - image backend/frontend/mock-dukcapil harus sudah ada atau SMOKE_BUILD_IMAGES=1.
set -eu

ENGINE=${CONTAINER_ENGINE:-podman}
case "$ENGINE" in podman|docker) ;; *) echo "CONTAINER_ENGINE harus podman atau docker" >&2; exit 2 ;; esac
tag=${IMAGE_TAG:-dev}
MAKE=${MAKE:-make}
port=${SMOKE_HTTP_PORT:-18378}
repo=$(cd "$(dirname "$0")/.." && pwd -P)
cd "$repo"

. "$repo/scripts/stack-berlabel.sh"
sb_init_label "${CONTAINER_LABEL:-}" "make smoke CONTAINER_LABEL=arctiva.issue=ARC-376"
sb_init_tmp poolrouter-smoke poolrouter-smoke

trap sb_cleanup_trap EXIT INT TERM

# --- Image -------------------------------------------------------------------------------------
have_image() { $ENGINE image exists "$1"; }
be_img="localhost/poolrouter-backend:$tag"
fe_img="localhost/poolrouter-frontend:$tag"
mock_img="localhost/poolrouter-mock-dukcapil:$tag"
if [ -n "${SMOKE_BUILD_IMAGES:-}" ]; then
  $MAKE image CONTAINER_ENGINE="$ENGINE" IMAGE_TAG="$tag" JOBS="${JOBS:-2}"
  $MAKE image-frontend CONTAINER_ENGINE="$ENGINE" IMAGE_TAG="$tag"
  $ENGINE build -f perf/mock-dukcapil/Containerfile -t "$mock_img" perf/mock-dukcapil
fi
for i in "$be_img" "$fe_img" "$mock_img"; do
  have_image "$i" || { echo "image $i tidak ada; jalankan make image image-frontend dan build perf/mock-dukcapil, atau beri SMOKE_BUILD_IMAGES=1" >&2; exit 3; }
done

# --- Secret acak dan env (di luar repo, tidak dicetak) ------------------------------------------
sb_gen_secrets
sb_render_env "$be_img" "$fe_img" "$port" \
  -e "s#^POOLROUTER_MOCK_DUKCAPIL_IMAGE=.*#POOLROUTER_MOCK_DUKCAPIL_IMAGE=$mock_img#"

# --- CA dan sertifikat TLS mock per run (Q1), direktori di luar repo, chmod 700 -----------------
tls="$tmp/tls"
mkdir -p "$tls"; chmod 700 "$tls"
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -keyout "$tls/key.pem" -out "$tls/cert.pem" \
  -subj "/CN=dukcapil-sintetis.example.test" \
  -addext "subjectAltName=DNS:dukcapil-sintetis.example.test" >/dev/null 2>&1
chmod 444 "$tls"/*.pem

# --- Overlay label + mock-tls: semua service, volume, dan network ------------------------------
sb_build_overlay postgres redis rabbitmq migrate backend-worker backend-cli frontend mock-dukcapil mock-tls -- backend-controller backend-webserver
smoke_overlay="$tmp/smoke.yaml"
POOLROUTER_SMOKE_TLS_CERT_DIR=$tls POOLROUTER_SMOKE_TLS_SCRIPT_DIR="$repo/scripts/smoke" \
  $ENGINE compose -f "$repo/scripts/smoke/compose.smoke.yaml" config >/dev/null # validasi awal, gagal cepat
cp "$repo/scripts/smoke/compose.smoke.yaml" "$smoke_overlay"

# sb_mk/sb_infra_up dkk memakai $overlay tunggal (stack-berlabel.sh); gabung overlay label dan
# overlay mock-tls jadi satu daftar EXTRA_COMPOSE_FILES lewat variabel overlay2.
overlay2=$overlay
overlay="$overlay2 $smoke_overlay"
sb_mk() {
  $MAKE "$@" ENV=dev CONTAINER_ENGINE="$ENGINE" ENV_FILE="$envf" PROJECT="$project" \
    EXTRA_COMPOSE_FILES="$overlay2 $smoke_overlay" \
    POOLROUTER_SMOKE_TLS_CERT_DIR="$tls" POOLROUTER_SMOKE_TLS_SCRIPT_DIR="$repo/scripts/smoke"
}

# --- Stack ---------------------------------------------------------------------------------------
echo "== infra"
sb_infra_up postgres redis rabbitmq
echo "== db-bootstrap (role per komponen)"
sb_db_bootstrap 10
echo "== migrate"
sb_migrate
echo "== seed sintetis (pengguna + dukcapil)"
sb_seed
echo "== controller (rebuild Redis: webserver baru siap setelah pr:siap ditulis controller)"
sb_mk compose-up COMPOSE_UP_SERVICES="backend-controller"
echo "== webserver, worker, frontend, mock-dukcapil, mock-tls"
sb_mk compose-up COMPOSE_UP_SERVICES="backend-webserver backend-worker frontend mock-dukcapil mock-tls" COMPOSE_PROFILES="bundled-infra mock"

# --- Fixture SQL: admin_session per peran + kode_daftar_mfa (Q4) --------------------------------
pwenv="$tmp/playwright.env"
umask 077
: > "$pwenv"
gen_sesi() {
  # gen_sesi <peran-env-var>: token sesi 43 karakter, hash SHA-256 ditulis ke fixture SQL, token
  # mentah ke $pwenv. Dipakai untuk enam peran seed.
  openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\n'
}
fixture_sql="$tmp/fixture.sql"
cp "$repo/scripts/smoke/fixture.sql" "$fixture_sql"
for var_placeholder in \
  SMOKE_TOKEN_PLATFORM_ADMIN:__HASH_PLATFORM_ADMIN__ \
  SMOKE_TOKEN_PENYETUJU:__HASH_PENYETUJU__ \
  SMOKE_TOKEN_OPERATOR:__HASH_OPERATOR__ \
  SMOKE_TOKEN_AUDITOR:__HASH_AUDITOR__ \
  SMOKE_TOKEN_CLIENT_OWNER:__HASH_CLIENT_OWNER__ \
  SMOKE_TOKEN_ADMIN_AUDITOR:__HASH_ADMIN_AUDITOR__; do
  var=${var_placeholder%%:*}; placeholder=${var_placeholder#*:}
  token=$(gen_sesi)
  [ "${#token}" = 43 ] || { echo "token sesi harus 43 karakter" >&2; exit 4; }
  hash=$(printf %s "$token" | openssl dgst -sha256 -hex | sed 's/^.*= //')
  sed -i "s/$placeholder/$hash/" "$fixture_sql"
  echo "E2E_${var}=$token" >> "$pwenv"
done
# kode_daftar_mfa (Q4): kode acak 128-bit hex, hash SHA-256 ke fixture, kode mentah ke env.
kode_daftar=$(openssl rand -hex 16)
hash_kode=$(printf %s "$kode_daftar" | openssl dgst -sha256 -hex | sed 's/^.*= //')
sed -i "s/__HASH_KODE_DAFTAR_MFA__/$hash_kode/" "$fixture_sql"
echo "E2E_KODE_DAFTAR_MFA=$kode_daftar" >> "$pwenv"
# Playwright jalan di netns container frontend (lihat blok "Playwright @smoke" di bawah): origin
# nginx internal 8080, bukan $port (port host yang dipublikasikan, beda nomor).
echo "E2E_BASE_URL=http://127.0.0.1:8080" >> "$pwenv"
# Password seed sama untuk keenam pengguna seed (backend-cli seed pengguna, backend/README.md);
# dipakai G1 untuk login + MFA sungguhan sekali lewat HTTP (spec Testing Decisions).
printf 'E2E_SEED_PASSWORD=%s\n' "$(cat "$secrets/poolrouter_seed_password")" >> "$pwenv"

n=$(sb_psql < "$fixture_sql" | grep -c '^1$' || true)
[ "$n" = 7 ] || { echo "fixture SQL smoke gagal: diharapkan 7 baris RETURNING 1 (6 admin_session + 1 kode_daftar_mfa), ada $n (pengguna seed tidak ada?)" >&2; exit 4; }

# --- G0: proses (spec bagian 2, inventaris.md "Proses") -----------------------------------------
proc_log="$tmp/proc.txt"
: > "$proc_log"
catat_proc() {
  id=$1; shift
  t0=$(date +%s)
  if "$@" >"$tmp/proc-out.txt" 2>&1; then
    detik=$(($(date +%s) - t0))
    echo "id=$id status=lulus detik=$detik" >> "$proc_log"
  else
    detik=$(($(date +%s) - t0))
    pesan=$(tr '\n' ' ' < "$tmp/proc-out.txt" | cut -c1-300)
    echo "id=$id status=gagal detik=$detik pesan=$pesan" >> "$proc_log"
  fi
}
cid() { $ENGINE ps -q --filter "label=$label" --filter "label=com.docker.compose.service=$1"; }
webserver_c=$(cid backend-webserver); controller_c=$(cid backend-controller); worker_c=$(cid backend-worker)
frontend_c=$(cid frontend)
[ -n "$frontend_c" ] || { echo "container frontend tidak ditemukan" >&2; exit 4; }

catat_proc proc:webserver sh -c "$ENGINE exec '$webserver_c' webserver healthcheck && $ENGINE exec '$webserver_c' sh -c 'exec 3<>/dev/tcp/127.0.0.1/9090; printf \"GET /metrics HTTP/1.0\r\n\r\n\" >&3; read -r l <&3; case \"\$l\" in *\" 200 \"*) ;; *) exit 1;; esac'"
catat_proc proc:controller "$ENGINE" exec "$controller_c" controller healthcheck
catat_proc proc:worker "$ENGINE" exec "$worker_c" worker liveness
catat_proc proc:backend-cli sh -c "
  $ENGINE compose -f '$repo/deploy/compose/compose.yaml' -f '$repo/deploy/compose/compose.dev.yaml' --project-name '$project' --env-file '$envf' --profile cli run --rm backend-cli bantuan &&
  $ENGINE compose -f '$repo/deploy/compose/compose.yaml' -f '$repo/deploy/compose/compose.dev.yaml' --project-name '$project' --env-file '$envf' --profile cli run --rm backend-cli seed pengguna &&
  $ENGINE compose -f '$repo/deploy/compose/compose.yaml' -f '$repo/deploy/compose/compose.dev.yaml' --project-name '$project' --env-file '$envf' --profile cli run --rm backend-cli seed dukcapil &&
  $ENGINE compose -f '$repo/deploy/compose/compose.yaml' -f '$repo/deploy/compose/compose.dev.yaml' --project-name '$project' --env-file '$envf' --profile cli run --rm backend-cli daftar | grep -c . | grep -qE '^[6-9]|^[1-9][0-9]+$'
"

# --- Playwright @smoke (project api, lalu ui) -----------------------------------------------------
# Jalan di netns container frontend (seperti scripts/e2e-api-asli.sh/ARC-251): origin
# 127.0.0.1:8080 sama dengan POOLROUTER_UI_ORIGIN (cookie __Host- Secure diterima), dan
# backend-webserver:8081 (downstream API, grup klien G3) hanya lewat network compose internal,
# tidak dipublikasikan ke host (spec Testing Decisions).
echo "== playwright @smoke"
# $tmp (di luar repo) tidak ter-mount di container Playwright (RUN_PW_OPTS hanya mount repo);
# laporan JSON harus ditulis di bawah frontend/.cache (di repo, git- dan lint-ignored), dibaca
# host sesudahnya lewat path absolut yang sama.
mkdir -p "$repo/frontend/.cache"
json_out_rel=".cache/smoke-playwright-report.json"
json_out="$repo/frontend/$json_out_rel"
rm -f "$json_out"
pw_rc=0
$MAKE -C frontend e2e-smoke FRONTEND_RUNNER="$ENGINE" CONTAINER_LABEL="$label" \
  E2E_NETNS="$frontend_c" E2E_ENV_FILE="$pwenv" PLAYWRIGHT_JSON_OUTPUT_NAME="$json_out_rel" || pw_rc=$?

# --- Laporan -------------------------------------------------------------------------------------
sha=$(git -C "$repo" rev-parse HEAD 2>/dev/null || echo unknown)
laporan_rc=0
node "$repo/scripts/smoke/laporan.mjs" "$json_out" "$proc_log" "$sha" "$repo/smoke-report.json" || laporan_rc=$?
rm -f "$json_out" 2>/dev/null || true

[ "$pw_rc" = 0 ] && [ "$laporan_rc" = 0 ]
