-- BERKAS HASIL RAKITAN (scripts/rakit-lua.sh), JANGAN DISUNTING.
-- Sumber: slot_klaim.lua di direktori ini, dengan potongan disisipkan pada baris `-- sisip:`.
-- Bentuk inilah yang dimuat ke Redis; berkas sumber tidak boleh dimuat langsung.
-- Buat ulang: make -C backend lua-rakit. Gate: make -C backend lua-rakit-check.

-- Worker: perpanjang slot miliknya dan keluarkan satu dari antrean.
-- ADR-0001 rev 15 §5a.1 butir 5 dan 6, D1.5.
--
-- Slot hanya boleh dilepas tiga pemilik: pemenang CAS `in_flight` sesudah log
-- lengkap, dispatcher bila commit `dispatched` gagal, dan reconciler sesudah
-- transaksinya commit. Worker yang mengklaim slot memperpanjangnya, bukan
-- melepasnya. Karena itu skrip ini tidak menghapus member: slot yang sudah
-- lewat skornya dijawab `slot_kedaluwarsa` dan baru dibersihkan
-- `kuota_ambil_token.lua` (D1.9).
--
-- KEYS (D1.7):
--   1 pr:siap               gerbang kesiapan
--   2 pr:slot:{<pool>}      semaphore konkurensi
--   3 pr:antre:{<pool>}     jumlah permintaan async yang belum dipanggil
--
-- ARGV:
--   1 run_id               dicocokkan dengan KEYS[1]
--   2 member               `<request_id>:<nomor_percobaan>` (D1.2)
--   3 ttl_slot_mikro       `timeout_total_upstream + 60 s`
--   4 dari_antre           `1` kurangi antrean, `0` jangan (rekonsiliasi)
--
-- Kembalian:
--   {'belum_siap'} / {'kunci_tidak_sah'} / {<kode galat>}
--   {'slot_tidak_ada'}                 member tidak pernah dipesan
--   {'slot_kedaluwarsa'}               skor sudah lewat; tanpa perpanjangan
--   {'ok', skor_lama, antre}           slot diperpanjang ke `now + ttl`

local siap = (function()
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
end)()
local bersama = (function()
-- Potongan bersama: validasi argumen, jam Redis, dan jendela bergulir
-- (ADR-0001 rev 15 §5a.1 butir 1d).
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya ke setiap skrip pada baris penanda `-- sisip: potongan/bersama.lua
-- sebagai bersama`; hasilnya ada di `rakitan/`. Sandbox Redis tidak punya
-- `dofile`, jadi berkas ini tidak pernah dimuat sendiri (review 778f9fd,
-- temuan blocking 2).
--
-- KUNCI LEWAT `KEYS`, bukan ARGV (D1.7). Setiap skrip menetapkan urutan
-- KEYS-nya di kepala berkas; potongan ini hanya menerima kunci yang sudah
-- dipilih pemanggil, sehingga tidak ada lagi `geser` ARGV yang harus dijaga
-- saat prefiks berubah.
--
-- §5a.1 butir 1d: periode dan nomor ember 10 menit dihitung SEKALI di Rust
-- dari `requested_at`, lalu dikirim sebagai argumen. Skrip tidak boleh
-- membaca jam untuk menentukan kunci. Kalau skrip yang menentukan kunci,
-- percobaan dengan `requested_at` 23:59:59,9 yang di-INCR pada 00:00:00,1
-- masuk ke periode berbeda dengan baris lognya, dan DQ-05 menjadi salah
-- positif di sekitar jam reset.
--
-- SATUAN `no_ember`. `no_ember` adalah NOMOR ember (`floor(unix/600)`), bukan
-- detik epoch dan bukan indeks cincin `nomor mod jumlah_ember` (D2.8). Jendela
-- bergulir adalah `no_ember - (jumlah_ember - 1) ..= no_ember`, yaitu
-- `jumlah_ember` ember dihitung dalam satuan ember. Mencampur satuan di sini
-- pernah membuat jendela menyempit dari 145 ember menjadi 145 detik (review
-- 778f9fd, temuan blocking 1). `batas_jendela` dan `di_jendela` di bawah
-- satu-satunya tempat aturan itu ditulis; skrip tidak boleh mengurangkan
-- sendiri.
--
-- JAM. Skrip memakai `redis.call('TIME')` untuk semua keputusan yang
-- bergantung pada waktu berjalan (skor slot, TAT GCRA), karena semuanya
-- dibandingkan dengan struktur data Redis yang sama dan Redis hanya punya satu
-- jam per instans. Asumsinya host disinkronkan NTP (addendum D3).

-- Bilangan bulat tanpa awalan nol (`-0` ditolak, `0` diterima).
local POLA_BULAT = '^%-?%d+$'

-- Nama field ember: `b<nomor ember>`.
local POLA_FIELD_EMBER = '^b(%d+)$'

local MODE = { bergulir = true, kalender = true }

-- Jam Redis dalam mikrodetik, satuan yang dipakai semua skor dan TAT.
local function mikro()
  -- `redis.call` mengembalikan SATU tabel, bukan beberapa nilai, jadi hasilnya
  -- dibaca per indeks.
  local t = redis.call('TIME')
  return tonumber(t[1]) * 1000000 + tonumber(t[2])
end

-- Bilangan bulat dari string, atau nil. `tonumber` tidak dipakai langsung
-- karena menerima `1.5`, `1e3`, dan spasi; semuanya harus ditolak.
local function bulat(nilai)
  if type(nilai) ~= 'string' then
    return nil
  end
  if not string.match(nilai, POLA_BULAT) then
    return nil
  end
  local n = tonumber(nilai)
  if n == nil or n ~= math.floor(n) then
    return nil
  end
  return n
end

-- Validasi bersama. Pemanggil Rust bergantung pada himpunan kode galat, bukan
-- pada urutannya. `kuota_efektif` harus bulat > 0, karena
-- `floor(kuota_harian × (1 − margin))` dengan `margin < 1` selalu > 0; nilai
-- tidak sah berarti bug konfigurasi, bukan kuota habis, jadi dilaporkan
-- sebagai galat, bukan penolakan.
--
-- `a` diisi pemanggil: `mode`, `kuota_efektif`, `jumlah_ember`, `no_ember`.
-- Field yang sah ditulis balik sebagai angka.
local function periksa(a)
  if MODE[a.mode] == nil then
    return 'mode_tidak_sah'
  end

  local kuota = bulat(a.kuota_efektif)
  if kuota == nil or kuota <= 0 then
    return 'kuota_efektif_tidak_sah'
  end
  a.kuota_efektif = kuota

  local ember = bulat(a.jumlah_ember)
  if ember == nil or ember < 1 then
    return 'jumlah_ember_tidak_sah'
  end
  a.jumlah_ember = ember

  local nomor = bulat(a.no_ember)
  if nomor == nil or nomor < 0 then
    return 'no_ember_tidak_sah'
  end
  a.no_ember = nomor

  return nil
end

-- Benar bila setiap kunci yang dikirim ada dan berbentuk string tidak kosong.
local function kunci_sah(...)
  local n = select('#', ...)
  if n == 0 then
    return false
  end
  for i = 1, n do
    local k = select(i, ...)
    if type(k) ~= 'string' or k == '' then
      return false
    end
  end
  return true
end

-- Batas bawah jendela bergulir dalam satuan ember: `jumlah_ember` ember
-- terakhir, termasuk ember berjalan (§5a.1 butir 1d). Dengan
-- `jumlah_ember = 145` batas ini berjarak tepat 24 jam dari awal ember
-- berjalan, jadi jendela tidak pernah lebih pendek dari 24 jam.
local function batas_jendela(a)
  return a.no_ember - (a.jumlah_ember - 1)
end

-- Benar bila field ember bernomor `n` dihitung pada jendela ini. Field dengan
-- nomor di luar rentang (lebih tua dari jendela, atau dari masa depan karena
-- jam pemanggil berbeda) tidak dihitung.
local function di_jendela(a, n)
  return n >= batas_jendela(a) and n <= a.no_ember
end

-- `terpakai` menurut §5a.1 butir 1b. `kalender`: counter per periode.
-- `bergulir`: jumlah ember di jendela. Keduanya angka, tidak pernah negatif.
local function terpakai(kunci_attempts, kunci_ember, a)
  if a.mode == 'kalender' then
    return tonumber(redis.call('GET', kunci_attempts) or '0')
  end
  local semua = redis.call('HGETALL', kunci_ember)
  local jumlah = 0
  for i = 1, #semua, 2 do
    local n = tonumber(string.match(semua[i], POLA_FIELD_EMBER))
    if n ~= nil and di_jendela(a, n) then
      jumlah = jumlah + tonumber(semua[i + 1])
    end
  end
  return jumlah
end

return {
  POLA_FIELD_EMBER = POLA_FIELD_EMBER,
  bulat = bulat,
  mikro = mikro,
  periksa = periksa,
  kunci_sah = kunci_sah,
  batas_jendela = batas_jendela,
  di_jendela = di_jendela,
  terpakai = terpakai
}
end)()

local run_id = ARGV[1]
if not siap(KEYS[1], run_id) then
  return { 'belum_siap' }
end
if not bersama.kunci_sah(KEYS[1], KEYS[2], KEYS[3]) then
  return { 'kunci_tidak_sah' }
end

local member = ARGV[2]
if member == nil or member == '' then
  return { 'member_tidak_sah' }
end

local ttl = bersama.bulat(ARGV[3])
if ttl == nil or ttl < 1 then
  return { 'ttl_slot_tidak_sah' }
end

local dari_antre = ARGV[4]
if dari_antre ~= '0' and dari_antre ~= '1' then
  return { 'dari_antre_tidak_sah' }
end

local now = bersama.mikro()
local skor = redis.call('ZSCORE', KEYS[2], member)
if not skor then
  return { 'slot_tidak_ada' }
end
if tonumber(skor) <= now then
  return { 'slot_kedaluwarsa' }
end

redis.call('ZADD', KEYS[2], 'XX', now + ttl, member)

local antre = tonumber(redis.call('GET', KEYS[3]) or '0')
if dari_antre == '1' and antre > 0 then
  antre = redis.call('DECR', KEYS[3])
end

return { 'ok', skor, antre }
