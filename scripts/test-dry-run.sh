#!/bin/sh
# Uji regresi SHAA-398: `make -n` pada gate resmi tidak boleh memanggil docker atau podman.
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
V="RUNNER=docker FRONTEND_RUNNER=docker CONTAINER_LABEL=shaal.issue=SHAA-398"
# shellcheck disable=SC2086
run "setup test lint" setup test lint $V
# shellcheck disable=SC2086
run "backend uji-redis check-disk" -C backend uji-redis $V
# shellcheck disable=SC2086
run "push-main" push-main $V
exit "$fail"
