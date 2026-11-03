#!/bin/sh
# Runs inside the toolchain container (or natively in CI) from `make oasdiff-breaking`
# (backend/Makefile, SHAA-279 / SHAA-278). Arguments: WORK_DIR OASDIFF_VERSION SHA256_arm64 SHA256_amd64.
# Baseline files are already extracted to $WORK_DIR/.cache/oasdiff/base-<name>.yaml by the Makefile;
# a missing base file means the contract is new in the baseline (warning was printed by the Makefile).
# Policy (ADR-0003 section 4.2 item 3):
#   downstream-api.yaml: breaking change fails the gate (--fail-on ERR).
#   admin-api.yaml: report only; breaking change becomes a ::warning::, exit 0.
set -eu
work=$1; version=$2; sha_arm64=$3; sha_amd64=$4
bin="$work/.cache/oasdiff/oasdiff"

if [ ! -x "$bin" ] || ! "$bin" --version 2>/dev/null | grep -q "$version"; then
  arch=$(uname -m)
  case "$arch" in
    aarch64) urlarch=arm64; sha=$sha_arm64 ;;
    x86_64) urlarch=amd64; sha=$sha_amd64 ;;
    *) echo "oasdiff: unsupported arch $arch" >&2; exit 1 ;;
  esac
  t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
  url="https://github.com/oasdiff/oasdiff/releases/download/v${version}/oasdiff_${version}_linux_${urlarch}.tar.gz"
  curl -fsSL -o "$t/o.tar.gz" "$url"
  echo "$sha  $t/o.tar.gz" | sha256sum -c -
  tar -xzf "$t/o.tar.gz" -C "$t" oasdiff
  mv "$t/oasdiff" "$bin"
fi

rc=0
for name in downstream-api admin-api; do
  base="$work/.cache/oasdiff/base-$name.yaml"
  cur="$work/openapi/$name.yaml"
  if [ ! -f "$base" ]; then
    echo "oasdiff: $name.yaml skipped (not in baseline)"
    continue
  fi
  case "$name" in
    downstream-api)
      echo "oasdiff: compare base-$name.yaml -> openapi/$name.yaml (fail-on ERR)"
      if ! "$bin" breaking --fail-on ERR "$base" "$cur"; then
        echo "oasdiff: breaking change in $name.yaml against baseline" >&2
        rc=1
      fi
      ;;
    admin-api)
      echo "oasdiff: compare base-$name.yaml -> openapi/$name.yaml (report only)"
      out=$("$bin" breaking "$base" "$cur") || { echo "oasdiff: oasdiff failed on $name.yaml" >&2; rc=1; continue; }
      if [ -n "$out" ]; then
        printf '%s\n' "$out"
        summary=$(printf '%s\n' "$out" | head -n 20 | sed -e 's/%/%25/g' -e 's/\r//g' | awk 'BEGIN{ORS="%0A"} {print}')
        echo "::warning title=oasdiff admin-api breaking::breaking change in $name.yaml (report only; reviewer verifies the frontend is fixed in the same commit)%0A$summary"
      fi
      ;;
  esac
done
exit "$rc"
