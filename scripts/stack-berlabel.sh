#!/bin/sh
# Pustaka bersama: stack Compose berlabel arctiva.issue=ARC-<n> (ADR-0001 Addendum A).
# Dipakai oleh scripts/e2e-api-asli.sh (ARC-251) dan scripts/smoke.sh (ARC-378) lewat `. `.
# Bukan skrip yang dijalankan sendiri: tanpa `set -eu` di sini supaya variabel errexit/nounset
# pemanggil tidak berubah; pemanggil wajib `set -eu` sendiri sebelum source.
#
# Variabel yang wajib sudah diisi pemanggil sebelum memakai fungsi mana pun:
#   label   arctiva.issue=ARC-<n> (sudah divalidasi oleh sb_init_label)
#   ENGINE  podman atau docker
#   MAKE    perintah make (biasanya ${MAKE:-make})
#   repo    akar repo (absolut)
#
# Variabel yang diisi oleh fungsi di bawah, untuk dipakai pemanggil sesudahnya:
#   issue, issue_lc   dari sb_init_label
#   tmp, secrets, envf, overlay   dari sb_init_tmp
#   project           dari sb_init_tmp (poolrouter-<prefix>-<issue_lc>)

# sb_init_label <CONTAINER_LABEL> <contoh-pesan-error>
# Validasi label arctiva.issue=ARC-<n>; set $label, $issue, $issue_lc.
sb_init_label() {
  label=${1:-}
  contoh=${2:-arctiva.issue=ARC-1}
  case "$label" in
    arctiva.issue=ARC-[0-9]*) ;;
    *) echo "CONTAINER_LABEL wajib arctiva.issue=ARC-<n> (contoh: $contoh)" >&2; exit 2 ;;
  esac
  issue=${label#arctiva.issue=}
  issue_lc=$(echo "$issue" | tr 'A-Z' 'a-z')
}

# sb_init_tmp <prefix-project> <prefix-tmpdir>
# Buat direktori sementara (di luar repo, chmod 700) dan $project; set $tmp, $secrets, $envf,
# $overlay, $project.
sb_init_tmp() {
  project_prefix=${1:?sb_init_tmp butuh prefix project}
  tmp_prefix=${2:?sb_init_tmp butuh prefix tmpdir}
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/$tmp_prefix.XXXXXX")
  chmod 700 "$tmp"
  secrets="$tmp/secrets"
  envf="$tmp/dev.env"
  overlay="$tmp/labels.yaml"
  project="$project_prefix-$issue_lc"
}

# sb_cleanup_trap: dipanggil dari `trap` pemanggil (EXIT INT TERM). Memakai $label, $ENGINE,
# $tmp, $project yang sudah diisi. Membersihkan HANYA resource berlabel $label (CLAUDE.md:
# tanpa prune global), lalu gagal (exit non-nol) bila ada sisa.
sb_cleanup_trap() {
  rc=$?
  if [ "$rc" != 0 ]; then
    echo "== DIAG (rc=$rc): container berlabel $issue" >&2
    $ENGINE ps -a --filter "label=$label" --format '{{.Names}} {{.Status}}' >&2 || true
    pgd=$($ENGINE ps -q --filter "label=$label" --filter "label=com.docker.compose.service=postgres" | head -n 1)
    for svc in backend-controller backend-webserver; do
      [ -n "$pgd" ] && { echo "-- readyz $svc" >&2; $ENGINE exec "$pgd" wget -q -O - -T 3 "http://$project-$svc-1:9090/readyz" >&2 2>&1 || true; echo >&2; }
    done
    for svc in backend-controller backend-webserver; do
      for c in $($ENGINE ps -a --filter "label=$label" --filter "label=com.docker.compose.service=$svc" -q); do
        echo "-- health $svc" >&2
        $ENGINE inspect --format '{{json .State.Health}}' "$c" >&2 2>&1 || true
        echo "-- logs $svc" >&2
        $ENGINE logs --tail 40 "$c" >&2 2>&1 || true
      done
    done
  fi
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

# sb_gen_secrets: bangkitkan secret acak (openssl) di $secrets (chmod 700/444), tidak dicetak.
sb_gen_secrets() {
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
}

# sb_render_env <be_img> <fe_img> <http_port>: render $envf dari dev.env.example dengan
# POOLROUTER_SECRETS_DIR, POOLROUTER_UI_ORIGIN, port, dan image. Argumen tambahan (opsional)
# adalah baris sed `-e '...'` tambahan diteruskan apa adanya untuk override lanjutan pemanggil.
sb_render_env() {
  be_img=$1; fe_img=$2; http_port=$3; shift 3
  sed -e "s#^POOLROUTER_SECRETS_DIR=.*#POOLROUTER_SECRETS_DIR=$secrets#" \
      -e "s#^POOLROUTER_UI_ORIGIN=.*#POOLROUTER_UI_ORIGIN=http://127.0.0.1:8080#" \
      -e "s#^POOLROUTER_HTTP_PORT=.*#POOLROUTER_HTTP_PORT=$http_port#" \
      -e "s#^POOLROUTER_BACKEND_IMAGE=.*#POOLROUTER_BACKEND_IMAGE=$be_img#" \
      -e "s#^POOLROUTER_FRONTEND_IMAGE=.*#POOLROUTER_FRONTEND_IMAGE=$fe_img#" \
      "$@" \
      "$repo/deploy/compose/env/dev.env.example" > "$envf"
}

# sb_build_overlay <services-label-polos...> -- <services-probe-dev-tcp...>
# Tulis $overlay: label arctiva.issue pada service di argumen pertama (sebelum `--`), ditambah
# healthcheck /dev/tcp (ganti probe bawaan yang panik tanpa ca-certificates) pada service di
# argumen kedua (sesudah `--`), plus db-bootstrap (volume ./../db -> backend/db), volume, network.
sb_build_overlay() {
  plain=""; probe_services=""; sisi=plain
  for a in "$@"; do
    if [ "$a" = "--" ]; then sisi=probe; continue; fi
    if [ "$sisi" = plain ]; then plain="$plain $a"; else probe_services="$probe_services $a"; fi
  done
  {
    echo "services:"
    for s in $plain; do echo "  $s: {labels: {arctiva.issue: $issue}}"; done
    probe='exec 3<>/dev/tcp/127.0.0.1/9090; printf "GET /readyz HTTP/1.0\r\n\r\n" >&3; read -r l <&3; case "$$l" in *" 200 "*) ;; *) exit 1;; esac'
    for s in $probe_services; do
      echo "  $s:"
      echo "    labels: {arctiva.issue: $issue}"
      echo "    healthcheck:"
      printf '%s\n' "      test: !override [\"CMD\", \"bash\", \"-c\", '$probe']"
    done
    echo "  db-bootstrap:"
    echo "    labels: {arctiva.issue: $issue}"
    echo "    volumes: !override [\"$repo/backend/db:/db:ro\"]"
    echo "volumes:"
    for v in pg_data redis_data rabbitmq_data; do echo "  $v: {labels: {arctiva.issue: $issue}}"; done
    echo "networks:"
    echo "  default: {labels: {arctiva.issue: $issue}}"
  } > "$overlay"
}

# sb_mk <target...>: wrapper $MAKE dengan ENV/CONTAINER_ENGINE/ENV_FILE/PROJECT/EXTRA_COMPOSE_FILES terisi.
sb_mk() {
  $MAKE "$@" ENV=dev CONTAINER_ENGINE="$ENGINE" ENV_FILE="$envf" PROJECT="$project" EXTRA_COMPOSE_FILES="$overlay"
}

# sb_infra_up <services...>: `compose-up` untuk service infra (postgres redis rabbitmq, ...).
sb_infra_up() {
  sb_mk compose-up COMPOSE_UP_SERVICES="$*"
}

# sb_db_bootstrap <jumlah-role-diharapkan>: jalankan db-bootstrap lewat profil migrate, lalu
# pastikan role per komponen sudah dibuat. Set $pg (id container postgres berlabel).
sb_db_bootstrap() {
  expected=$1
  compose_cmd=$(sb_mk -n compose-down | sed -n 's/^\(.*\) --profile bundled-infra down.*$/\1/p' | tail -n 1)
  [ -n "$compose_cmd" ] || { echo "gagal menurunkan awalan compose dari make -n" >&2; exit 4; }
  eval "$compose_cmd --profile bundled-infra --profile migrate run --rm db-bootstrap"
  pg=$($ENGINE ps -q --filter "label=$label" --filter "label=com.docker.compose.service=postgres")
  roles=$($ENGINE exec "$pg" sh -c 'psql -U "$POSTGRES_USER" -d postgres -tAc "select count(*) from pg_roles where rolname = '"'"'poolrouter_owner'"'"' or rolname like '"'"'pr\_%'"'"'"')
  [ "$roles" = "$expected" ] || { echo "db-bootstrap: $expected role diharapkan, ada $roles" >&2; exit 4; }
  echo "db-bootstrap: $roles role dibuat"
}

# sb_migrate: terapkan backend/migrations/*.sql lewat psql (image tidak punya biner `migrate`;
# lihat catatan di scripts/e2e-api-asli.sh). Butuh $pg dari sb_db_bootstrap.
sb_migrate() {
  pgdb=$(sed -n 's/^POSTGRES_DB=//p' "$envf" | tail -n 1)
  psql_pg() { $ENGINE exec -i "$pg" sh -c 'exec psql -v ON_ERROR_STOP=1 -q -U "$POSTGRES_USER" "$@"' sh -d "$pgdb" "$@"; }
  terapkan_tx() { { echo "SET ROLE poolrouter_owner;"; for m; do echo "-- $(basename "$m")"; cat "$m"; echo; done; } | psql_pg -1; }
  set --
  for f in "$repo"/backend/migrations/*.sql; do
    if head -n 1 "$f" | grep -qx -- '-- no-transaction'; then
      [ $# -eq 0 ] || { terapkan_tx "$@"; set --; }
      { echo "SET ROLE poolrouter_owner;"; cat "$f"; } | psql_pg
    else
      set -- "$@" "$f"
    fi
  done
  [ $# -eq 0 ] || terapkan_tx "$@"
  echo "migrate: ok"
}

# sb_seed: `make compose-seed` (seed pengguna + seed dukcapil, idempoten, ENV=dev).
sb_seed() {
  sb_mk compose-seed
}

# sb_psql <sql lewat stdin>: jalankan SQL terhadap $pg sebagai POSTGRES_USER/-DB dari $envf
# (dipakai untuk fixture sintetis: sesi admin_session, kode_daftar_mfa, dsb).
sb_psql() {
  $ENGINE exec -i "$pg" sh -c 'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -v ON_ERROR_STOP=1 -tA'
}
