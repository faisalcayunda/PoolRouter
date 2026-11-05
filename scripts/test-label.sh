#!/bin/sh
# Uji regresi kontrak label gate Arctiva (AC-2, ARC-2).
#
# `make -C backend check-label` dan `make -C frontend check-label` harus:
#   - lolos (exit 0) untuk CONTAINER_LABEL=arctiva.issue=ARC-<n>, dan
#   - menolak (exit 2) semua label lain, termasuk label Shaal shaal.issue=SHAA-<n>.
#
# Tidak butuh Docker atau podman: check-label hanya memeriksa string label.
# Jalankan: make test-label (dari akar repo).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
fail=0

# check <harapan-exit> <stack> <runner-var> <runner> <label>
check() {
	want=$1; stack=$2; rvar=$3; runner=$4; label=$5
	set +e
	(cd "$root" && make -s -C "$stack" check-label "$rvar=$runner" CONTAINER_LABEL="$label" >/dev/null 2>&1)
	got=$?
	set -e
	if [ "$got" -eq "$want" ]; then
		echo "lulus: $stack $rvar=$runner CONTAINER_LABEL=$label exit=$got"
	else
		echo "GAGAL: $stack $rvar=$runner CONTAINER_LABEL=$label exit=$got (harap $want)"
		fail=1
	fi
}

for stack in backend frontend; do
	if [ "$stack" = backend ]; then rvar=RUNNER; runner=docker; else rvar=FRONTEND_RUNNER; runner=docker; fi
	check 0 "$stack" "$rvar" "$runner" "arctiva.issue=ARC-123"
	check 0 "$stack" "$rvar" "$runner" "arctiva.issue=ARC-2"
	check 2 "$stack" "$rvar" "$runner" "shaal.issue=SHAA-123"
	check 2 "$stack" "$rvar" "$runner" "arctiva.issue=SHAA-123"
	check 2 "$stack" "$rvar" "$runner" "arctiva.issue=123"
	check 2 "$stack" "$rvar" "$runner" "shaal.issue=SHAA-123"
	check 2 "$stack" "$rvar" "$runner" ""
done

exit "$fail"
