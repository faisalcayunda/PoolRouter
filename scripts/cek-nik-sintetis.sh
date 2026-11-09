#!/bin/sh
# Cek prefiks NIK sintetis pada data uji beban (ARC-237, aturan repo: NIK hanya sintetis dan berawalan 00).
#
# Yang dijaga di direktori perf/ (generator k6, mock Dukcapil, skrip jalan uji):
#   1. setiap deretan tepat 16 digit berawalan 00;
#   2. generator k6 (`syntheticNik`) memakai konstanta NIK_PREFIKS yang bernilai '00'.
#
# Self-test lebih dulu: direktori palsu dengan prefiks salah HARUS gagal di kedua cek, supaya cek yang
# mati ketahuan. Tidak butuh Docker. Jalankan: make test-nik (dari akar repo atau backend/).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)

# cek_dir <dir>: cetak pelanggaran ke stdout; exit 1 bila ada.
cek_dir() {
	dir=$1
	hasil=0
	# Deretan 16 digit yang berdiri sendiri (tidak bagian dari deretan digit lebih panjang) dan tidak berawalan 00.
	salah=$(grep -rnIE '(^|[^0-9])[0-9]{16}([^0-9]|$)' "$dir" 2>/dev/null \
		| grep -vE '(^|[^0-9])00[0-9]{14}([^0-9]|$)' || true)
	if [ -n "$salah" ]; then
		echo "NIK 16 digit tidak berawalan 00:"
		echo "$salah"
		hasil=1
	fi
	for js in "$dir"/k6/*.js; do
		[ -f "$js" ] || continue
		grep -q "syntheticNik" "$js" || continue
		if ! grep -qE "^const NIK_PREFIKS = '00';" "$js"; then
			echo "$js: NIK_PREFIKS harus bernilai '00'"
			hasil=1
		fi
		if ! awk '/function syntheticNik/{d=1} d&&/NIK_PREFIKS/{ok=1} d&&/^}/{exit} END{exit !ok}' "$js"; then
			echo "$js: syntheticNik tidak memakai NIK_PREFIKS"
			hasil=1
		fi
	done
	return "$hasil"
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/k6"
cat > "$tmp/k6/bad.js" <<'JS'
const NIK_PREFIKS = '3201';
function syntheticNik() {
  let s = '3201';
  return s;
}
const contoh = '3201010101010001';
JS
if out=$(cek_dir "$tmp" 2>&1); then
	echo "GAGAL self-test: fixture NIK berawalan salah lolos cek"
	exit 1
fi
for frasa in "tidak berawalan 00" "NIK_PREFIKS harus" "tidak memakai NIK_PREFIKS"; do
	printf '%s' "$out" | grep -q "$frasa" || { echo "GAGAL self-test: cek '$frasa' tidak terpicu"; exit 1; }
done
echo "lulus: self-test (fixture berawalan salah ditolak oleh ketiga cek)"

if out=$(cek_dir "$root/perf" 2>&1); then
	echo "lulus: NIK di perf/ berawalan 00"
else
	echo "GAGAL: $out"
	exit 1
fi
