#!/usr/bin/env bash
# Lane build/test PoolRouter di Docker dengan penjaga storage (SHAA-373). Lihat docs/runbook/docker-lane.md.
# Pemakaian:
#   lane.sh check               cek ruang kosong dan ambang; juga penjaga / dan target bersama (SHAA-390)
#   lane.sh cleanup <SHAA-n>    hapus container, volume, dan network berlabel shaal.issue=<SHAA-n> saja
#   lane.sh builder-prune       batasi cache build Docker (builder prune --keep-storage)
# Dilarang di skrip ini: system prune, volume prune, image prune -a, rm tanpa filter.
set -euo pipefail

MIN_FREE_ROOT_GB="${MIN_FREE_ROOT_GB:-3}"      # filesystem /var/lib/docker
MIN_FREE_DATA_GB="${MIN_FREE_DATA_GB:-1}"      # /data
BUILD_CACHE_KEEP="${BUILD_CACHE_KEEP:-3GB}"    # batas cache build
# Penjaga / (SHAA-390). Nilai dikirim backend/Makefile; default sama dengan Makefile.
SHARED_TARGET_DIR="${SHARED_TARGET_DIR:-/var/tmp/shaal-build/poolrouter-target}"
SHARED_TARGET_LIMIT_MB="${SHARED_TARGET_LIMIT_MB:-2560}"   # target bersama > ini => clean-shared
ROOT_MIN_FREE_MB="${ROOT_MIN_FREE_MB:-2048}"               # sisa bebas / untuk sistem
CLEAN_SHARED_CMD="${CLEAN_SHARED_CMD:-}"                   # perintah clean-shared (di bawah kunci heavy)

die() { echo "ERROR: $*" >&2; exit 1; }

require_issue() {
  [[ "${1:-}" =~ ^SHAA-[0-9]+$ ]] || die "argumen harus berbentuk SHAA-<n>"
}

free_gb() { df -BG --output=avail "$1" | tail -1 | tr -dc '0-9'; }
free_mb() { df -BM --output=avail "$1" | tail -1 | tr -dc '0-9'; }
size_mb() { [[ -d "$1" ]] && du -sm "$1" | cut -f1 || echo 0; }

SHARED_CARGO_HOME="${SHARED_CARGO_HOME:-/data/workspace/shaal-build/cargo-home}"
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

# GC checkout (SHAA-408): hapus node_modules dan .cache frontend di checkout Multica yang issue-nya
# sudah done/cancelled DAN HEAD-nya sudah menjadi ancestor main. Checkout issue aktif, status yang
# tidak terbaca, dan HEAD yang belum di main tidak disentuh. Dipanggil hanya saat check di bawah ambang.
WORKSPACES_ROOT="${WORKSPACES_ROOT:-/data/home/multica_workspaces}"

gc_checkouts() {
  local repo dir issue status head rc=0 removed=0
  command -v multica >/dev/null 2>&1 || { echo "gc-checkouts: CLI multica tidak ada; dilewati" >&2; return 0; }
  command -v jq >/dev/null 2>&1 || { echo "gc-checkouts: jq tidak ada; dilewati" >&2; return 0; }
  shopt -s nullglob
  for repo in "$WORKSPACES_ROOT"/*/shaa-*/workdir/PoolRouter; do
    [[ -d "$repo/frontend" && -e "$repo/.git" ]] || continue
    dir="$(basename "$(dirname "$(dirname "$repo")")")"    # shaa-<n>-<hash>
    issue="SHAA-$(sed -E 's/^shaa-([0-9]+)-.*/\1/' <<<"$dir")"
    [[ "$issue" =~ ^SHAA-[0-9]+$ ]] || continue
    status="$(multica issue get "$issue" --output json 2>/dev/null | jq -r '.status // empty' 2>/dev/null || true)"
    if [[ "$status" != "done" && "$status" != "cancelled" ]]; then
      echo "gc-checkouts: lewati $issue (status='${status:-tidak terbaca}')"; continue
    fi
    head="$(git -C "$repo" rev-parse HEAD 2>/dev/null || true)"
    if [[ -z "$head" ]] || ! git -C "$repo" merge-base --is-ancestor "$head" main 2>/dev/null; then
      echo "gc-checkouts: lewati $issue (HEAD belum di main)"; continue
    fi
    for p in frontend/node_modules frontend/.cache; do
      [[ -e "$repo/$p" ]] || continue
      if rm -rf -- "$repo/$p" 2>/dev/null; then
        echo "gc-checkouts: hapus $repo/$p ($issue, $status)"; removed=$((removed + 1))
      else
        echo "gc-checkouts: gagal hapus $repo/$p (milik root? jalankan make -C frontend clean)" >&2; rc=1
      fi
    done
  done
  echo "gc-checkouts: selesai, $removed path dihapus"
  return "$rc"
}

check() {
  local root_dir free_root free_data
  root_dir="$(docker info --format '{{.DockerRootDir}}')"
  free_root="$(free_gb "$root_dir")"
  free_data="$(free_gb /data)"
  echo "docker root: $root_dir, bebas ${free_root}G (ambang ${MIN_FREE_ROOT_GB}G)"
  echo "/data: bebas ${free_data}G (ambang ${MIN_FREE_DATA_GB}G)"
  if (( free_root < MIN_FREE_ROOT_GB )) || (( free_data < MIN_FREE_DATA_GB )); then
    echo "di bawah ambang: jalankan GC checkout yang sudah selesai"
    gc_checkouts || true
    free_root="$(free_gb "$root_dir")"
    free_data="$(free_gb /data)"
    echo "setelah GC: docker root bebas ${free_root}G, /data bebas ${free_data}G"
  fi
  if (( free_root < MIN_FREE_ROOT_GB )) || (( free_data < MIN_FREE_DATA_GB )); then
    die "ruang kosong di bawah ambang sesudah GC; run berhenti. Jangan diakali, lapor blocker."
  fi
  [[ -d "$SHARED_CARGO_HOME" ]] && check_cargo_home
  [[ -d "$(dirname "$SHARED_TARGET_DIR")" ]] && check_root
  echo "OK: ruang kosong di atas ambang"
}

cleanup() {
  local issue="$1" ids
  require_issue "$issue"
  ids="$(docker ps -aq --filter "label=shaal.issue=${issue}")"
  [[ -n "$ids" ]] && docker rm -fv $ids
  ids="$(docker volume ls -q --filter "label=shaal.issue=${issue}")"
  [[ -n "$ids" ]] && docker volume rm $ids
  ids="$(docker network ls -q --filter "label=shaal.issue=${issue}")"
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
  gc-checkouts) gc_checkouts ;;
  *) die "pakai: check | cleanup <SHAA-n> | builder-prune | gc-checkouts" ;;
esac
