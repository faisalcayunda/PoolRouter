#!/bin/sh
# Cek prefiks NIK sintetis pada data uji beban (ARC-237, aturan repo: NIK hanya sintetis dan berawalan 00).
#
# Yang dijaga di direktori perf/ (generator k6, mock Dukcapil, skrip jalan uji):
#   1. setiap deretan tepat 16 digit berawalan 00 (dicek per kecocokan, bukan per baris);
#   2. generator k6 (`syntheticNik`) memakai konstanta NIK_PREFIKS yang bernilai '00'.
#
# Fail-closed: perf/k6 yang hilang, nol berkas generator, atau grep yang error (exit > 1) dihitung gagal,
# tidak pernah lolos diam-diam.
#
# Self-test lebih dulu: fixture buruk HARUS gagal pada tiap cek dan fixture baik HARUS lulus, supaya cek
# yang mati ketahuan. Tidak butuh Docker. Jalankan: make test-nik (dari akar repo atau backend/).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# cek_dir <dir>: cetak pelanggaran ke stdout; exit 1 bila ada.
cek_dir() {
	dir=$1
	hasil=0
	if [ ! -d "$dir/k6" ]; then
		echo "$dir/k6 tidak ada: generator NIK tidak bisa diperiksa"
		return 1
	fi
	# Semua deretan 16+ digit per kecocokan (-o), format "berkas:baris:digit". Hanya yang tepat 16 digit dan
	# tidak berawalan 00 yang salah. grep exit 1 (tidak ada kecocokan) wajar; exit > 1 (error baca) gagal.
	cocok="$tmp/cocok"
	rc=0
	grep -rnIoE '[0-9]{16,}' "$dir" >"$cocok" || rc=$?
	if [ "$rc" -gt 1 ]; then
		echo "grep gagal (exit $rc) saat memindai $dir"
		return 1
	fi
	salah=$(awk -F: '{ d = $NF } length(d) == 16 && substr(d, 1, 2) != "00"' "$cocok")
	if [ -n "$salah" ]; then
		echo "NIK 16 digit tidak berawalan 00:"
		echo "$salah"
		hasil=1
	fi
	generator=0
	for js in "$dir"/k6/*.js; do
		[ -f "$js" ] || continue
		rc=0
		grep -q "syntheticNik" "$js" || rc=$?
		if [ "$rc" -gt 1 ]; then
			echo "$js: grep gagal (exit $rc)"
			hasil=1
			continue
		fi
		[ "$rc" -eq 0 ] || continue
		generator=$((generator + 1))
		if ! grep -qE "^const NIK_PREFIKS = '00';" "$js"; then
			echo "$js: NIK_PREFIKS harus bernilai '00'"
			hasil=1
		fi
		if ! awk '/function syntheticNik/{d=1} d&&/NIK_PREFIKS/{ok=1} d&&/^}/{exit} END{exit !ok}' "$js"; then
			echo "$js: syntheticNik tidak memakai NIK_PREFIKS"
			hasil=1
		fi
	done
	if [ "$generator" -eq 0 ]; then
		echo "$dir/k6: tidak ada generator syntheticNik, cek prefiks tidak berarti"
		hasil=1
	fi
	return "$hasil"
}

# buat_fixture <nama> : fixture baik di $tmp/<nama>/k6/gen.js; mengubah isinya dilakukan pemanggil.
buat_fixture() {
	mkdir -p "$tmp/$1/k6"
	cat > "$tmp/$1/k6/gen.js" <<'JS'
const NIK_PREFIKS = '00';
function syntheticNik() {
  let s = NIK_PREFIKS;
  return s;
}
JS
}

# harus_lulus <nama>: fixture baik harus lulus (kontrol positif, supaya cek tidak selalu gagal).
harus_lulus() {
	if ! out=$(cek_dir "$tmp/$1" 2>&1); then
		echo "GAGAL self-test: fixture baik '$1' ditolak: $out"
		exit 1
	fi
}

# harus_gagal <nama> <frasa>...: fixture harus ditolak dan keluarannya memuat tiap frasa.
harus_gagal() {
	nama=$1
	shift
	if out=$(cek_dir "$tmp/$nama" 2>&1); then
		echo "GAGAL self-test: fixture '$nama' lolos cek"
		exit 1
	fi
	for frasa in "$@"; do
		printf '%s' "$out" | grep -qF "$frasa" || { echo "GAGAL self-test: fixture '$nama': cek '$frasa' tidak terpicu"; exit 1; }
	done
}

# Kontrol positif.
buat_fixture baik
echo "const contoh = '0012345678901234';" > "$tmp/baik/data.txt"
harus_lulus baik

# Awalan salah pada literal dan pada generator (nilai jelas palsu).
mkdir -p "$tmp/awalan/k6"
cat > "$tmp/awalan/k6/bad.js" <<'JS'
const NIK_PREFIKS = '99';
function syntheticNik() {
  let s = '99';
  return s;
}
const contoh = '9900000000000000';
JS
harus_gagal awalan "tidak berawalan 00" "NIK_PREFIKS harus" "tidak memakai NIK_PREFIKS"

# Celah 1: direktori k6 hilang.
mkdir -p "$tmp/tanpa-k6"
echo "ok" > "$tmp/tanpa-k6/catatan.txt"
harus_gagal tanpa-k6 "k6 tidak ada"

# Celah 2: generator tanpa identifier syntheticNik (nol berkas yang diperiksa).
mkdir -p "$tmp/tanpa-generator/k6"
echo "const x = 1;" > "$tmp/tanpa-generator/k6/lain.js"
harus_gagal tanpa-generator "tidak ada generator syntheticNik"

# Celah 3: satu baris berisi NIK 00 dan angka 16 digit non-00.
buat_fixture campur
echo "const dua = ['0012345678901234', '9900000000000000'];" > "$tmp/campur/data.js"
harus_gagal campur "tidak berawalan 00" "9900000000000000"

# Celah 4: grep error (berkas tidak terbaca) tidak boleh dianggap bersih. Dilewati bila root (chmod tak berlaku).
buat_fixture tak-terbaca
echo "x" > "$tmp/tak-terbaca/rahasia.txt"
chmod 000 "$tmp/tak-terbaca/rahasia.txt"
if [ -r "$tmp/tak-terbaca/rahasia.txt" ]; then
	echo "lewati: kasus berkas tak terbaca (proses bisa membaca berkas mode 000)"
else
	harus_gagal tak-terbaca "grep gagal"
fi
echo "lulus: self-test (celah awalan, k6 hilang, tanpa generator, baris campuran, grep error)"

if out=$(cek_dir "$root/perf" 2>&1); then
	echo "lulus: NIK di perf/ berawalan 00"
else
	echo "GAGAL: $out"
	exit 1
fi
