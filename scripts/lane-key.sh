#!/bin/sh
# Kunci lane (SHAA-401): <SHAA-n | local>-<8 hex sha256 path absolut checkout di host>.
# Pemakaian: lane-key.sh <CONTAINER_LABEL> <path-checkout>   -> contoh: SHAA-401-1a2b3c4d
set -eu
label=${1:-}
dir=$(cd "${2:?path checkout wajib}" && pwd -P)
issue=$(printf '%s' "$label" | sed -n 's/^shaal\.issue=\(SHAA-[0-9][0-9]*\)$/\1/p')
[ -n "$issue" ] || issue=local
hash=$(printf '%s' "$dir" | sha256sum | cut -c1-8)
printf '%s-%s\n' "$issue" "$hash"
