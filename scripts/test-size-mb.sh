#!/usr/bin/env bash
# Uji regresi size_mb (SHAA-405): berkas hilang saat dipindai => satu angka, exit 0; direktori tak terbaca => gagal jelas.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=docker-lane.sh
source "$here/docker-lane.sh"
set +e
tmp="$(mktemp -d)"; trap 'chmod -R u+rwx "$tmp" 2>/dev/null; rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

# 1) direktori tidak ada => 0
[[ "$(size_mb "$tmp/none")" == 0 ]] || fail "direktori tidak ada harus 0"

# 2) du dipaksa keluar 1 karena berkas hilang (stub du di PATH) => tepat satu angka, exit 0
mkdir "$tmp/bin" "$tmp/d"
cat > "$tmp/bin/du" <<'STUB'
#!/bin/sh
echo "du: cannot access '$2/deps/x.o': No such file or directory" >&2
printf '42\t%s\n' "$2"
exit 1
STUB
chmod +x "$tmp/bin/du"
out="$(PATH="$tmp/bin:$PATH" size_mb "$tmp/d")"; rc=$?
[[ $rc -eq 0 && "$out" == 42 ]] || fail "stub du exit 1 (hilang): rc=$rc out='$out'"

# 3) du asli vs loop penghapus: setiap panggilan tepat satu bilangan bulat, exit 0
mkdir "$tmp/race"
( while :; do for i in $(seq 1 300); do : > "$tmp/race/f$i"; done; rm -f "$tmp/race"/f*; done ) &
bg=$!
for _ in $(seq 1 40); do
  out="$(size_mb "$tmp/race")"; rc=$?
  [[ $rc -eq 0 && "$out" =~ ^[0-9]+$ ]] || { kill $bg; fail "race: rc=$rc out='$out'"; }
done
kill $bg; wait $bg 2>/dev/null

# 4) direktori tak terbaca => gagal dengan pesan jelas (dilewati bila root, izin tidak berlaku)
if [[ "$(id -u)" != 0 ]]; then
  mkdir -p "$tmp/ro/sub"; : > "$tmp/ro/sub/f"; chmod 000 "$tmp/ro/sub"
  msg="$(size_mb "$tmp/ro" 2>&1)"; rc=$?
  [[ $rc -ne 0 && "$msg" == *"du gagal mengukur"* ]] || fail "tak terbaca harus gagal jelas: rc=$rc msg='$msg'"
  # dan lewat check_cargo_home/check_root: assignment gagal => set -e menghentikan
  ( set -e; SHARED_TARGET_DIR="$tmp/ro" check_root ) >/dev/null 2>&1 && fail "check_root harus gagal"
fi
echo "OK: size_mb"
