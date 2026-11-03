#!/bin/sh
# compose-lint (V1): ADR-0001 Addendum A §5 (D3), §3.3, §7 (D6), §8 (V1). Kontrak env v1 §3 (owner).
#
# Dipanggil dari `make compose-lint`. Variabel dari Makefile:
#   COMPOSE_CMD   perintah compose lengkap, termasuk prefix env provider
#   COMPOSE_DIR   direktori file Compose (default deploy/compose)
#
# Urutan:
#   1. Self-test: setiap fixture pelanggaran HARUS gagal DENGAN label yang tertulis di baris
#      `# expect: <label>` fixture itu. Fixture hanya memuat satu pelanggaran, supaya aturan yang
#      mati ketahuan.
#   2. Render `compose config` untuk dev, staging, prod (semua profile) dari file env contoh.
#   3. Cek file sumber (YAML) dan setiap Containerfile dengan .dockerignore pasangannya.
# Exit 0 hanya jika semua cek lolos.
set -eu

COMPOSE_DIR=${COMPOSE_DIR:-deploy/compose}
COMPOSE_CMD=${COMPOSE_CMD:?COMPOSE_CMD belum diisi (lewat make compose-lint)}
LINT_DIR="$COMPOSE_DIR/lint"
RENDER_DIR="$LINT_DIR/.render"
FAILS=0

# Service yang boleh memakai cap_add (kontrak B2). Daftar eksplisit, bukan pengecualian global.
CAP_ADD_ALLOWED="postgres redis"

fail() { echo "FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

# Aturan YAML: label<TAB>regex ERE, dicocokkan per baris.
# volume-U-option: opsi `U` pada token volume, di posisi mana pun (`:U`, `:U,ro`, `:ro,U`, `:z,U`).
YAML_RULES='container_name	^[[:space:]]*container_name:
userns_mode	^[[:space:]]*userns_mode:
keep-id	keep-id
volume-U-option	:([^[:space:]"'"'"',]*,)*U([,"'"'"'[:space:]]|$)
idmap	:idmap
x-podman	x-podman
privileged	^[[:space:]]*privileged:[[:space:]]*true
devices	^[[:space:]]*devices:
network_mode_host	network_mode:[[:space:]]*host
ipc_host	ipc:[[:space:]]*host
pid_host	pid:[[:space:]]*host
links	^[[:space:]]*links:
extra_hosts	^[[:space:]]*extra_hosts:|host\.docker\.internal|host\.containers\.internal|host-gateway
platform	^[[:space:]]*platform:
develop	^[[:space:]]*develop:
watch	^[[:space:]]*watch:
include	^include:|^[[:space:]]+include:
extends	^[[:space:]]*extends:
swarm_deploy	^[[:space:]]*(mode|placement|update_config|rollback_config):
image_latest	image:.*:latest([[:space:]"'"'"'@]|$)'

# Cek satu file YAML atau hasil render. Argumen: file, lalu mode (source|dev|staging|prod).
check_yaml() {
  file=$1; mode=$2
  echo "$YAML_RULES" | while IFS='	' read -r label re; do
    [ -n "$label" ] || continue
    if grep -nE -- "$re" "$file" >/dev/null 2>&1; then
      echo "pelanggaran [$label] di $file: $(grep -nE -- "$re" "$file" | head -n 1)" >&2
      echo x >> "$RENDER_DIR/.violations"
    fi
  done
  # Driver logging selain json-file (ADR-0001 Addendum §5).
  if grep -nE '^[[:space:]]*driver:' "$file" | grep -v 'json-file' | grep -q .; then
    echo "pelanggaran [logging-driver] di $file" >&2; echo x >> "$RENDER_DIR/.violations"
  fi
  # Image wajib berdigest di semua mode (§3.3, D6). Source: image dari variabel (${...}) diperiksa
  # saat render. Dev: image lokal `localhost/` diizinkan (build lokal). Staging dan prod: tidak.
  case "$mode" in
    source) bad=$(grep -nE '^[[:space:]]*image:' "$file" | grep -vE '@sha256:[0-9a-f]{64}' | grep -v '\${' || true) ;;
    dev)    bad=$(grep -nE '^[[:space:]]*image:' "$file" | grep -vE '@sha256:[0-9a-f]{64}' | grep -v 'localhost/' || true) ;;
    *)      bad=$(grep -nE '^[[:space:]]*image:' "$file" | grep -vE '@sha256:[0-9a-f]{64}' || true) ;;
  esac
  if [ -n "$bad" ]; then
    echo "pelanggaran [image-tanpa-digest] di $file: $(echo "$bad" | head -n 1)" >&2; echo x >> "$RENDER_DIR/.violations"
  fi
  case "$mode" in
    staging|prod)
      if grep -nE '^[[:space:]]*build:' "$file" | grep -q .; then
        echo "pelanggaran [build-di-$mode] di $file" >&2; echo x >> "$RENDER_DIR/.violations"
      fi
      ;;
  esac
  if [ "$mode" = prod ]; then
    if grep -nE 'mock-dukcapil|POOLROUTER_SEED_SYNTHETIC' "$file" | grep -q .; then
      echo "pelanggaran [mock-atau-seed-di-prod] di $file" >&2; echo x >> "$RENDER_DIR/.violations"
    fi
  fi
  # Aturan per service (source dan render): cap_add hanya untuk CAP_ADD_ALLOWED; secret
  # poolrouter_owner hanya untuk service migrate (kontrak env v1 §1.5, V1).
  awk -v allow="$CAP_ADD_ALLOWED" '
    /^[^[:space:]#]/ { insvc = ($0 ~ /^services:/); next }
    insvc && /^  [A-Za-z0-9_-]+:[[:space:]]*$/ { svc = $0; sub(/^  /, "", svc); sub(/:.*/, "", svc); next }
    insvc && /^    cap_add:/ && index(" " allow " ", " " svc " ") == 0 { print "cap_add-di-" svc }
    insvc && /poolrouter_owner/ && svc != "migrate" { print "owner-secret-di-" svc }
  ' "$file" > "$RENDER_DIR/.svc" || true
  if [ -s "$RENDER_DIR/.svc" ]; then
    while read -r hit; do echo "pelanggaran [service] di $file: $hit" >&2; done < "$RENDER_DIR/.svc"
    echo x >> "$RENDER_DIR/.violations"
  fi
}

# Aturan Containerfile (ADR-0001 Addendum §3.3). Argumen: file.
check_containerfile() {
  file=$1
  bad=$(awk '
    BEGIN { last_user = "" }
    /^[[:space:]]*#[[:space:]]*syntax=/ { print "syntax-directive " NR }
    /^[[:space:]]*HEALTHCHECK/ { print "healthcheck " NR }
    /^[[:space:]]*SHELL/ { print "shell " NR }
    /^[[:space:]]*ONBUILD/ { print "onbuild " NR }
    /^[[:space:]]*COPY[[:space:]]/ && /--(link|parents|exclude)/ { print "copy-buildkit-flag " NR }
    /^[[:space:]]*ADD[[:space:]]+(https?:\/\/|git@)/ { print "add-remote " NR }
    /^[[:space:]]*RUN[[:space:]]+--(network|security)/ { print "run-buildkit-flag " NR }
    /^[[:space:]]*(RUN|COPY).*<</ { print "heredoc " NR }
    /^[[:space:]]*FROM[[:space:]]/ {
      i = 2; while (i <= NF && $i ~ /^--/) i++   # lewati flag seperti --platform=
      img = $i; alias_ = ""
      if (toupper($(i + 1)) == "AS") alias_ = $(i + 2)
      if (img != "scratch" && index(img, "@sha256:") == 0 && !(img in stages)) print "from-tanpa-digest " NR
      if (alias_ != "") stages[alias_] = 1
    }
    /^[[:space:]]*USER[[:space:]]/ { last_user = $2 }
    END {
      if (last_user != "10001:10001") print "user-akhir-bukan-10001:10001 (" last_user ")"
    }
  ' "$file")
  if [ -n "$bad" ]; then
    echo "pelanggaran Containerfile di $file:" >&2
    echo "$bad" | sed 's/^/  /' >&2
    echo x >> "$RENDER_DIR/.violations"
  fi
  dockerignore="$(dirname "$file")/.dockerignore"
  if [ ! -f "$dockerignore" ]; then
    echo "pelanggaran [dockerignore-hilang] untuk $file" >&2; echo x >> "$RENDER_DIR/.violations"
  fi
}

violations() { [ -f "$RENDER_DIR/.violations" ] && wc -l < "$RENDER_DIR/.violations" | tr -d ' ' || echo 0; }
reset_violations() { rm -f "$RENDER_DIR/.violations"; }

# Self-test: fixture pelanggaran harus gagal dengan label `# expect:` miliknya.
# Argumen: file, mode, nama fungsi cek.
expect_fail() {
  f=$1; mode=$2; fn=$3
  exp=$(sed -n 's/^# expect: //p' "$f" | head -n 1)
  [ -n "$exp" ] || { fail "fixture $f tidak punya baris '# expect:'"; return; }
  msg=$($fn "$f" "$mode" 2>&1 || true)
  case "$msg" in
    *"$exp"*) echo "  OK terdeteksi [$exp]: $f" ;;
    *) fail "fixture $f tidak memicu '$exp' (linter tidak mendeteksi)" ;;
  esac
}

mkdir -p "$RENDER_DIR"
reset_violations

echo "== 1. self-test: setiap fixture pelanggaran harus gagal dengan labelnya =="
for f in "$LINT_DIR"/fixtures/bad-*.yaml; do expect_fail "$f" source check_yaml; done
for f in "$LINT_DIR"/fixtures/render-bad-*.yaml; do expect_fail "$f" prod check_yaml; done
for f in "$LINT_DIR"/fixtures/containerfile/bad-*.Containerfile; do expect_fail "$f" "" check_containerfile; done
# Kontrol negatif: fixture bersih harus lolos.
msg=$(check_containerfile "$LINT_DIR/fixtures/containerfile/good.Containerfile" 2>&1 || true)
[ -z "$msg" ] && echo "  OK fixture bersih lolos" || fail "fixture good.Containerfile gagal: $msg"
reset_violations

echo "== 2. render compose config (dev, staging, prod; semua profile) =="
for env in dev staging prod; do
  out="$RENDER_DIR/$env.yaml"
  # Stdout saja yang disimpan; banner provider ada di stderr.
  if ! eval "$COMPOSE_CMD -p lint-$env -f $COMPOSE_DIR/compose.yaml -f $COMPOSE_DIR/compose.$env.yaml --env-file $COMPOSE_DIR/env/$env.env.example --profile '*' config" > "$out" 2>"$RENDER_DIR/$env.err"; then
    fail "compose config gagal untuk $env (lihat $RENDER_DIR/$env.err)"; continue
  fi
  check_yaml "$out" "$env"
  echo "  render $env: $(grep -c '^  [a-z0-9-]*:$' "$out") service, cek selesai"
done

echo "== 3. file sumber dan Containerfile =="
for f in "$COMPOSE_DIR"/compose*.yaml; do check_yaml "$f" source; done
find . -path ./.git -prune -o \( -name Containerfile -o -name '*.Containerfile' \) -print \
  | grep -v "^./$LINT_DIR/fixtures" | while read -r cf; do
    check_containerfile "$cf"
  done

rc=$(violations)
reset_violations
if [ "$rc" -gt 0 ] || [ "$FAILS" -gt 0 ]; then
  echo "compose-lint: GAGAL ($rc pelanggaran, $FAILS kegagalan self-test)"
  exit 1
fi
echo "compose-lint: LULUS"
