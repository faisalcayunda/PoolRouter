-- Potongan bersama: gerbang kesiapan Redis (ADR-0001 rev 15 §5a.2 butir 2).
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya ke setiap skrip pada baris penanda `-- sisip: potongan/siap.lua
-- sebagai ...`; hasilnya ada di `rakitan/` dan itulah bentuk yang dimuat ke
-- Redis. Sandbox Redis tidak punya `dofile`, jadi berkas ini tidak pernah
-- dimuat sendiri (review 778f9fd, temuan blocking 2).
--
-- Setiap skrip kuota memanggil ini lebih dulu. Kunci `pr:siap` ditulis
-- controller sebagai langkah terakhir rebuild, berisi `run_id` Redis yang
-- sedang hidup. Pemanggil mengirim `run_id` yang dibacanya dari `INFO server`
-- saat (re)connect sebagai ARGV prefiks. Kalau nilai berbeda, Redis sudah
-- restart atau replika sudah dipromosikan, sehingga counter di memori belum
-- tentu mencerminkan PostgreSQL. Pemanggil memperlakukan `belum_siap` sebagai
-- "Redis tidak tersedia" (fail-closed, §5), bukan sebagai "kuota tersedia".
--
-- Kuncinya lewat `KEYS`, bukan konstanta di sini: aturan D1.7 menaruh
-- `pr:siap` sebagai `KEYS[1]` di setiap skrip, sehingga hash tag pool (bila
-- kelak dipakai) tetap satu jalur dengan kunci lain di skrip yang sama.
--
-- Karena disisipkan, isi berkas ini hanya boleh berisi fungsi dan konstanta,
-- tanpa efek samping dan tanpa bergantung pada ARGV global.

-- Mengembalikan `true` bila Redis siap untuk run_id ini, `false` bila belum.
local function siap(kunci, run_id)
  if kunci == nil or kunci == '' then
    return false
  end
  if run_id == nil or run_id == '' then
    return false
  end
  local nilai = redis.call('GET', kunci)
  if not nilai then
    return false
  end
  return nilai == run_id
end

-- Potongan ini disisipkan sebagai `local siap = (function() … end)()`, jadi
-- yang dikembalikan adalah fungsinya sendiri, bukan tabel.
return siap
