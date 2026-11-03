#!/bin/sh
# rakit-lua.sh — menyusun skrip Lua kuota dari berkas sumber + potongan bersama.
# SHAA-195.
#
# Satu-satunya penyusun, dipakai tiga tempat sekaligus:
#   * build     : `make -C backend lua-rakit`  (menulis <lua>/rakitan/)
#   * gate      : `make -C backend lua-rakit-check` (merakit ke direktori
#                 sementara lalu membandingkan dengan rakitan/ yang di-commit)
#   * harness   : `uji_lua.py` (merakit dengan skrip ini juga, lalu memastikan
#                 hasilnya byte-identik dengan rakitan/ yang dimuat ke Redis)
#
# Alasan keberadaannya: sandbox Redis tidak punya `dofile`/`require`, jadi
# potongan `potongan/*.lua` harus disisipkan saat build, bukan dimuat saat
# jalan (review 778f9fd, temuan blocking 2). Aturan penanda ada di
# scripts/rakit-lua.awk.
set -eu

ENTRYPOINT="kuota_intake.lua kuota_ambil_token.lua slot_klaim.lua slot_lepas.lua attempts_tambah.lua kuota_baca.lua"

pemakaian() {
  cat <<'TEKS'
Pakai: rakit-lua.sh [--lua DIR] [--keluaran DIR]

  --lua DIR        direktori sumber: berisi potongan/ dan skrip entrypoint.
                   Default: <repo>/backend/crates/core/src/kuota/lua
  --keluaran DIR   direktori hasil. Default: <lua>/rakitan
TEKS
}

akar=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
lua="$akar/backend/crates/core/src/kuota/lua"
keluaran=''

while [ $# -gt 0 ]; do
  case "$1" in
    --lua)
      [ $# -ge 2 ] || { echo "rakit-lua: --lua butuh nilai" >&2; exit 2; }
      lua=$2
      shift 2
      ;;
    --keluaran)
      [ $# -ge 2 ] || { echo "rakit-lua: --keluaran butuh nilai" >&2; exit 2; }
      keluaran=$2
      shift 2
      ;;
    -h|--help)
      pemakaian
      exit 0
      ;;
    *)
      echo "rakit-lua: argumen tidak dikenal: $1" >&2
      pemakaian >&2
      exit 2
      ;;
  esac
done

[ -d "$lua" ] || { echo "rakit-lua: direktori sumber tidak ada: $lua" >&2; exit 2; }
[ -d "$lua/potongan" ] || { echo "rakit-lua: potongan/ tidak ada di $lua" >&2; exit 2; }
if [ -z "$keluaran" ]; then
  keluaran="$lua/rakitan"
fi
mkdir -p "$keluaran"

for nama in $ENTRYPOINT; do
  masuk="$lua/$nama"
  [ -f "$masuk" ] || { echo "rakit-lua: skrip sumber tidak ada: $masuk" >&2; exit 1; }
  hasil="$keluaran/$nama"
  sementara="$hasil.sementara.$$"
  if ! RAKIT_LUA_DIR="$lua" RAKIT_NAMA="$nama" \
    awk -f "$akar/scripts/rakit-lua.awk" "$masuk" > "$sementara"; then
    rm -f "$sementara"
    exit 1
  fi
  # Penjaga bentuk produksi: hasil rakitan tidak boleh memuat pemuatan berkas
  # (ditolak sandbox Redis) atau penanda yang belum disisipkan.
  if grep -q -e 'dofile[[:space:]]*(' -e 'require[[:space:]]*(' \
    -e 'loadstring[[:space:]]*(' -e '^-- sisip:' "$sementara"; then
    echo "rakit-lua: hasil rakitan $nama masih memuat pemuatan berkas atau penanda" >&2
    rm -f "$sementara"
    exit 1
  fi
  mv "$sementara" "$hasil"
done

echo "rakit-lua: hasil di $keluaran"
