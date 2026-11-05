#!/usr/bin/env bash
# Lane build/test PoolRouter di Docker dengan penjaga storage (SHAA-373). Lihat docs/runbook/docker-lane.md.
# Pemakaian:
#   lane.sh check               cek ruang kosong dan ambang; juga penjaga / dan target bersama (SHAA-390)
#   lane.sh cleanup <ARC-n>    hapus container, volume, dan network berlabel arctiva.issue=<ARC-n> saja
#   lane.sh builder-prune       batasi cache build Docker (builder prune --keep-storage)
# Dilarang di skrip ini: system prune, volume prune, image prune -a, rm tanpa filter.
set -euo pipefail

MIN_FREE_ROOT_GB="${MIN_FREE_ROOT_GB:-3}"      # filesystem /var/lib/docker
MIN_FREE_DATA_GB="${MIN_FREE_DATA_GB:-1}"      # /data
BUILD_CACHE_KEEP="${BUILD_CACHE_KEEP:-3GB}"    # batas cache build
# Penjaga / (SHAA-390). Nilai dikirim backend/Makefile; default sama dengan Makefile.
SHARED_TARGET_DIR="${SHARED_TARGET_DIR:-/var/tmp/poolrouter-build/poolrouter-target}"
SHARED_TARGET_LIMIT_MB="${SHARED_TARGET_LIMIT_MB:-2560}"   # target bersama > ini => clean-shared
ROOT_MIN_FREE_MB="${ROOT_MIN_FREE_MB:-2048}"               # sisa bebas / untuk sistem
CLEAN_SHARED_CMD="${CLEAN_SHARED_CMD:-}"                   # perintah clean-shared (di bawah kunci heavy)

die() { echo "ERROR: $*" >&2; exit 1; }

require_issue() {
  [[ "${1:-}" =~ ^ARC-[0-9]+$ ]] || die "argumen harus berbentuk ARC-<n>"
}

# data_mount: titik mount disk kerja bersama. /data bila ada (host produksi), selain itu akar
# SHARED_CARGO_HOME terdekat yang sudah ada (mis. /tmp/... di host tanpa /data).
data_mount() {
  if [[ -d /data ]]; then echo /data; return; fi
  local p="$SHARED_CARGO_HOME"
  while [[ -n "$p" && "$p" != "/" && ! -d "$p" ]]; do p="$(dirname "$p")"; done
  [[ -d "$p" ]] || p=/
  echo "$p"
}

free_gb() { df -BG --output=avail "$1" | tail -1 | tr -dc '0-9'; }
free_mb() { df -BM --output=avail "$1" | tail -1 | tr -dc '0-9'; }
size_mb() { [[ -d "$1" ]] && du -sm "$1" | cut -f1 || echo 0; }

SHARED_CARGO_HOME="${SHARED_CARGO_HOME:-/data/workspace/poolrouter-build/cargo-home}"
SHARED_CARGO_HOME_LIMIT_MB="${SHARED_CARGO_HOME_LIMIT_MB:-2560}"   # cargo-home + rustup bersama (SHAA-394)

# Penjaga cargo-home bersama (SHAA-394): melapor ukuran; di atas batas => berhenti, TIDAK menghapus apa pun.
check_cargo_home() {
  local csize
  csize="$(size_mb "$SHARED_CARGO_HOME")"
  echo "cargo-home bersama $SHARED_CARGO_HOME: ${csize}M (batas ${SHARED_CARGO_HOME_LIMIT_MB}M)"
  (( csize <= SHARED_CARGO_HOME_LIMIT_MB )) || die "cargo-home bersama ${csize}M melebihi batas; jalankan 'make -C backend clean-cache-shared RUNNER=docker CONTAINER_LABEL=...' atau naikkan SHARED_CARGO_HOME_LIMIT_MB dengan keputusan tertulis."
}

# Penjaga /: (1) target bersama > batas => cargo clean di bawah kunci (exit 75 bila kunci dipegang);
# (2) bebas / harus >= ROOT_MIN_FREE_MB + sisa ruang tumbuh target sampai batasnya. Di bawah itu: exit non-nol.
check_root() {
  local tsize free need grow
  tsize="$(size_mb "$SHARED_TARGET_DIR")"
  echo "target bersama $SHARED_TARGET_DIR: ${tsize}M (batas ${SHARED_TARGET_LIMIT_MB}M)"
  if (( tsize > SHARED_TARGET_LIMIT_MB )); then
    [[ -n "$CLEAN_SHARED_CMD" ]] || die "target bersama ${tsize}M melebihi batas dan CLEAN_SHARED_CMD kosong"
    echo "target bersama melebihi batas; clean-shared di bawah kunci"
    local rc=0
    sh -c "$CLEAN_SHARED_CMD" || rc=$?
    if (( rc != 0 )); then
      echo "ERROR: clean-shared gagal (exit $rc); kunci heavy dipegang build lain bila sub-exit 75" >&2
      exit "$rc"
    fi
    tsize="$(size_mb "$SHARED_TARGET_DIR")"
    echo "target bersama sesudah clean: ${tsize}M"
  fi
  free="$(free_mb /)"
  grow=$(( SHARED_TARGET_LIMIT_MB - tsize )); (( grow < 0 )) && grow=0
  need=$(( ROOT_MIN_FREE_MB + grow ))
  echo "/: bebas ${free}M, perlu ${need}M (${ROOT_MIN_FREE_MB}M sistem + ${grow}M tumbuh target)"
  (( free >= need )) || die "bebas / di bawah ambang; run berhenti. Jangan diakali, lapor blocker."
}

check() {
  local root_dir free_root free_data
  root_dir="$(docker info --format '{{.DockerRootDir}}')"
  free_root="$(free_gb "$root_dir")"
  # Disk data dibaca dari mount CARGO_HOME bersama, bukan hardcode /data: host satu-disk,
  # CI, dan sesi agen tidak punya /data. Default produksi tetap /data/workspace/...
  free_data="$(free_gb "$(data_mount)")"
  echo "docker root: $root_dir, bebas ${free_root}G (ambang ${MIN_FREE_ROOT_GB}G)"
  echo "data ($(data_mount)): bebas ${free_data}G (ambang ${MIN_FREE_DATA_GB}G)"
  if (( free_root < MIN_FREE_ROOT_GB )) || (( free_data < MIN_FREE_DATA_GB )); then
    die "ruang kosong di bawah ambang; run berhenti. Jangan diakali, lapor blocker."
  fi
  [[ -d "$SHARED_CARGO_HOME" ]] && check_cargo_home
  [[ -d "$(dirname "$SHARED_TARGET_DIR")" ]] && check_root
  echo "OK: ruang kosong di atas ambang"
}

cleanup() {
  local issue="$1" ids
  require_issue "$issue"
  ids="$(docker ps -aq --filter "label=arctiva.issue=${issue}")"
  [[ -n "$ids" ]] && docker rm -fv $ids
  ids="$(docker volume ls -q --filter "label=arctiva.issue=${issue}")"
  [[ -n "$ids" ]] && docker volume rm $ids
  ids="$(docker network ls -q --filter "label=arctiva.issue=${issue}")"
  [[ -n "$ids" ]] && docker network rm $ids
  echo "cleanup ${issue} selesai (hanya resource berlabel ${issue})"
}

builder_prune() {
  docker builder prune --keep-storage "$BUILD_CACHE_KEEP" --force
}

case "${1:-}" in
  check) check ;;
  cleanup) cleanup "${2:-}" ;;
  builder-prune) builder_prune ;;
  *) die "pakai: check | cleanup <ARC-n> | builder-prune" ;;
esac
