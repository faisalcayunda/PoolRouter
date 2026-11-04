# rakit-lua.awk — menyusun satu skrip Lua kuota dari berkas sumber + potongan.
#
# Dipanggil oleh `scripts/rakit-lua.sh` (SHAA-195). Satu-satunya penyusun yang
# dipakai build (`make -C backend lua-rakit`), gate (`lua-rakit-check`), dan
# harness uji (`uji_lua.py`), supaya bentuk yang diuji sama dengan bentuk yang
# dimuat ke Redis. Sandbox Redis tidak punya `dofile`, jadi potongan harus
# disisipkan apa adanya, bukan dimuat saat jalan (review 778f9fd, temuan
# blocking 2).
#
# Lingkungan: RAKIT_LUA_DIR = direktori sumber (berisi `potongan/`),
#             RAKIT_NAMA    = nama berkas sumber, untuk kepala berkas hasil.
#
# Penanda di berkas sumber, di awal baris:
#   -- sisip: potongan/NAMA.lua sebagai NAMA_VARIABEL
# menjadi:
#   local NAMA_VARIABEL = (function()
#   <isi potongan apa adanya>
#   end)()

BEGIN {
  RAKIT_LUA_DIR = ENVIRON["RAKIT_LUA_DIR"]
  RAKIT_NAMA = ENVIRON["RAKIT_NAMA"]
  if (RAKIT_LUA_DIR == "" || RAKIT_NAMA == "") {
    print "rakit-lua.awk: RAKIT_LUA_DIR dan RAKIT_NAMA harus diisi" > "/dev/stderr"
    exit 2
  }
}

function gagal(pesan) {
  printf "rakit-lua: %s: %s\n", RAKIT_NAMA, pesan > "/dev/stderr"
  exit 1
}

function kepala() {
  print "-- BERKAS HASIL RAKITAN (scripts/rakit-lua.sh), JANGAN DISUNTING."
  print "-- Sumber: " RAKIT_NAMA " di direktori ini, dengan potongan disisipkan pada baris `-- sisip:`."
  print "-- Bentuk inilah yang dimuat ke Redis; berkas sumber tidak boleh dimuat langsung."
  print "-- Buat ulang: make -C backend lua-rakit. Gate: make -C backend lua-rakit-check."
}

function sisipkan(berkas, nama,   jalur, baris, jumlah, i) {
  jalur = RAKIT_LUA_DIR "/" berkas
  jumlah = 0
  while ((getline baris < jalur) > 0) {
    if (index(baris, "-- sisip:") == 1) {
      close(jalur)
      gagal("potongan " berkas " memuat penanda sisip sendiri")
    }
    jumlah++
    potongan[jumlah] = baris
  }
  close(jalur)
  if (jumlah == 0) {
    gagal("potongan kosong atau tidak ada: " berkas)
  }
  if (nama !~ /^[A-Za-z_][A-Za-z0-9_]*$/) {
    gagal("nama variabel tidak sah: " nama)
  }
  print "local " nama " = (function()"
  for (i = 1; i <= jumlah; i++) {
    print potongan[i]
  }
  print "end)()"
}

FNR == 1 {
  kepala()
  print ""
}

index($0, "-- sisip:") == 1 {
  sisa = substr($0, length("-- sisip:") + 1)
  sub(/^[ \t]+/, "", sisa)
  sub(/[ \t]+$/, "", sisa)
  pisah = index(sisa, " sebagai ")
  if (pisah == 0) {
    gagal("penanda tanpa `sebagai`: " $0)
  }
  berkas = substr(sisa, 1, pisah - 1)
  nama = substr(sisa, pisah + length(" sebagai "))
  if (berkas !~ /^potongan\/[a-z0-9_]+\.lua$/) {
    gagal("berkas potongan tidak sah: " berkas)
  }
  sisipkan(berkas, nama)
  next
}

{
  print
}
