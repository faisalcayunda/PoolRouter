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
# Bagian stack berlabel (label, secret, env, overlay, infra, db-bootstrap, migrate, seed, pembersihan)
# ada di scripts/stack-berlabel.sh (ARC-378), dipakai bersama scripts/smoke.sh.
#
# Batas (dicatat, bukan disembunyikan):
#   - controller dinaikkan karena readiness webserver menunggu pr:siap yang ditulis controller saat
#     rebuild Redis; worker tidak dinaikkan (layar provider/secret tidak butuh worker).
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

ENGINE=podman
tag=${IMAGE_TAG:-dev}
MAKE=${MAKE:-make}
port=${E2E_HTTP_PORT:-18251}
repo=$(cd "$(dirname "$0")/.." && pwd -P)
cd "$repo"

. "$repo/scripts/stack-berlabel.sh"
sb_init_label "${CONTAINER_LABEL:-}" "make e2e-api-asli CONTAINER_LABEL=arctiva.issue=ARC-251"
sb_init_tmp poolrouter-e2e poolrouter-e2e

trap sb_cleanup_trap EXIT INT TERM

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
sb_gen_secrets
sb_render_env "$be_img" "$fe_img" "$port"

# --- Overlay label: semua service (termasuk run --rm), volume, dan network ---------------------
sb_build_overlay postgres redis rabbitmq migrate backend-worker backend-cli frontend mock-dukcapil -- backend-controller backend-webserver

# --- Stack -------------------------------------------------------------------------------------
echo "== infra"
sb_infra_up postgres redis rabbitmq
echo "== db-bootstrap (role per komponen)"
sb_db_bootstrap 10
[ "${E2E_UPTO:-}" = bootstrap ] && { echo "E2E_UPTO=bootstrap: berhenti sebelum migrate"; exit 0; }
echo "== migrate"
sb_migrate
echo "== seed sintetis"
sb_seed
echo "== controller (rebuild Redis: webserver baru siap setelah pr:siap ditulis controller)"
sb_mk compose-up COMPOSE_UP_SERVICES="backend-controller"
echo "== webserver + frontend"
sb_mk compose-up COMPOSE_UP_SERVICES="backend-webserver frontend"

# --- SQL fixture sesi (AC-4): pengguna seed platform admin dengan MFA terverifikasi -------------
token=$(openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\n')
[ "${#token}" = 43 ] || { echo "token sesi harus 43 karakter" >&2; exit 4; }
hash=$(printf %s "$token" | openssl dgst -sha256 -hex | sed 's/^.*= //')
fe=$($ENGINE ps -q --filter "label=$label" --filter "label=com.docker.compose.service=frontend")
[ -n "$pg" ] && [ -n "$fe" ] || { echo "container postgres atau frontend tidak ditemukan" >&2; exit 4; }
n=$(printf "%s\n" "INSERT INTO admin_session (sesi_id, token_sha256, pengguna_id, kedaluwarsa_at, user_agent, mfa_terverifikasi_at)
 SELECT gen_random_uuid(), decode('$hash','hex'), pengguna_id, now() + interval '8 hours', 'e2e-api-asli', now()
 FROM admin_user WHERE username = 'seed-platform-admin' RETURNING 1;" \
  | sb_psql | grep -c '^1$' || true)
[ "$n" = 1 ] || { echo "SQL fixture sesi gagal (pengguna seed-platform-admin tidak ada?)" >&2; exit 4; }

pwenv="$tmp/playwright.env"
umask 077
{ echo "E2E_BASE_URL=http://127.0.0.1:8080"; echo "E2E_SESSION_TOKEN=$token"; } > "$pwenv"

# --- Suite ---------------------------------------------------------------------------------------
echo "== playwright @api-asli"
$MAKE -C frontend e2e-api-asli FRONTEND_RUNNER=$ENGINE CONTAINER_LABEL="$label" E2E_NETNS="$fe" E2E_ENV_FILE="$pwenv"
