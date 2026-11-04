#!/usr/bin/env bash
# Lane build/test PoolRouter di Docker dengan penjaga storage (SHAA-373). Lihat docs/runbook/docker-lane.md.
# Pemakaian:
#   lane.sh check               cek ruang kosong dan ambang, tanpa mengubah apa pun
#   lane.sh cleanup <SHAA-n>    hapus container, volume, dan network berlabel shaal.issue=<SHAA-n> saja
#   lane.sh builder-prune       batasi cache build Docker (builder prune --keep-storage)
# Dilarang di skrip ini: system prune, volume prune, image prune -a, rm tanpa filter.
set -euo pipefail

MIN_FREE_ROOT_GB="${MIN_FREE_ROOT_GB:-3}"      # filesystem /var/lib/docker
MIN_FREE_DATA_GB="${MIN_FREE_DATA_GB:-1}"      # /data
BUILD_CACHE_KEEP="${BUILD_CACHE_KEEP:-3GB}"    # batas cache build

die() { echo "ERROR: $*" >&2; exit 1; }

require_issue() {
  [[ "${1:-}" =~ ^SHAA-[0-9]+$ ]] || die "argumen harus berbentuk SHAA-<n>"
}

free_gb() { df -BG --output=avail "$1" | tail -1 | tr -dc '0-9'; }

check() {
  local root_dir free_root free_data
  root_dir="$(docker info --format '{{.DockerRootDir}}')"
  free_root="$(free_gb "$root_dir")"
  free_data="$(free_gb /data)"
  echo "docker root: $root_dir, bebas ${free_root}G (ambang ${MIN_FREE_ROOT_GB}G)"
  echo "/data: bebas ${free_data}G (ambang ${MIN_FREE_DATA_GB}G)"
  if (( free_root < MIN_FREE_ROOT_GB )) || (( free_data < MIN_FREE_DATA_GB )); then
    die "ruang kosong di bawah ambang; run berhenti. Jangan diakali, lapor blocker."
  fi
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
  *) die "pakai: check | cleanup <SHAA-n> | builder-prune" ;;
esac
