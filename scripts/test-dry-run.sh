#!/bin/sh
# Uji regresi ARC-398: `make -n` pada gate resmi tidak boleh memanggil docker atau podman.
# Stub pencatat di PATH menulis setiap panggilan ke log; log harus kosong dan exit harus 0.
# Tidak butuh Docker sungguhan. Jalankan: make test-dry-run (dari akar repo).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/bin"
log="$tmp/stub.log"
: > "$log"
for e in docker podman; do
	printf '#!/bin/sh\necho "%s $*" >> "%s"\nexit 0\n' "$e" "$log" > "$tmp/bin/$e"
	chmod +x "$tmp/bin/$e"
done
fail=0
run() {
	name=$1; shift
	set +e
	(cd "$root" && PATH="$tmp/bin:$PATH" make -n "$@" > "$tmp/out" 2>&1)
	rc=$?
	set -e
	if [ "$rc" -ne 0 ] || [ -s "$log" ]; then
		echo "GAGAL: $name exit=$rc"; echo "--- log stub:"; cat "$log"; echo "--- output make:"; tail -20 "$tmp/out"
		fail=1; : > "$log"
	else
		echo "lulus: $name exit=0, log stub kosong"
	fi
}
V="RUNNER=docker FRONTEND_RUNNER=docker CONTAINER_LABEL=arctiva.issue=ARC-398"
# shellcheck disable=SC2086
run "setup test lint" setup test lint $V
# shellcheck disable=SC2086
run "backend uji-redis check-disk" -C backend uji-redis $V
# push-main sekarang ada sebagai penjaga: ia menolak dengan exit 2 dan menjelaskan alur PR (AC-3),
# tanpa menyentuh engine. `make -n` pada target ini juga tidak boleh memanggil docker/podman.
set +e
(cd "$root" && PATH="$tmp/bin:$PATH" make -n push-main > "$tmp/out" 2>&1)
rc=$?
set -e
if [ "$rc" -ne 0 ] || [ -s "$log" ]; then
	echo "GAGAL: make -n push-main exit=$rc atau memanggil engine"; tail -20 "$tmp/out"; fail=1; : > "$log"
else
	echo "lulus: make -n push-main exit=0, log stub kosong"
fi
set +e
(cd "$root" && PATH="$tmp/bin:$PATH" make -s push-main > "$tmp/out" 2>&1)
rc=$?
set -e
if [ "$rc" -eq 2 ] && grep -q "PR ke main" "$tmp/out" && [ ! -s "$log" ]; then
	echo "lulus: make push-main menolak dengan pesan alur PR (exit=2), log stub kosong"
else
	echo "GAGAL: make push-main exit=$rc, pesan/engine tidak sesuai"; tail -20 "$tmp/out"; fail=1; : > "$log"
fi
exit "$fail"
