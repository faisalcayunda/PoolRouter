-- BERKAS HASIL RAKITAN (scripts/rakit-lua.sh), JANGAN DISUNTING.
-- Sumber: kuota_retry_after.lua di direktori ini, dengan potongan disisipkan pada baris `-- sisip:`.
-- Bentuk inilah yang dimuat ke Redis; berkas sumber tidak boleh dimuat langsung.
-- Buat ulang: make -C backend lua-rakit. Gate: make -C backend lua-rakit-check.

-- Pembacaan kuota tanpa mutasi, untuk `Retry-After` dan kuota tracker admin.
-- ADR-0001 rev 15 §5a.1 butir 1d, §5a.7.
--
-- Skrip ini MURNI BACA: tidak ada menulis, tidak ada `TIME`. Waktu sekarang
-- untuk menghitung sisa jendela datang dari `ARGV[3] = now_mikro`, dihitung
-- Rust dari jam yang sama dengan `requested_at` (§5a.1 butir 1d). Kalau skrip
-- ini membaca jam sendiri, `Retry-After` dan hitungan admin bisa memakai dasar
-- waktu berbeda dari yang dipakai menulis ember.
--
-- Jendela bergulir memakai aturan yang SAMA dengan jalur tulis
-- (`bersama.di_jendela`): `jumlah_ember` ember terakhir termasuk ember
-- berjalan, dalam satuan nomor ember. Sebelumnya skrip ini menurunkan batas
-- dari `now_mikro`, sehingga `terpakai` di sini bisa berbeda dari `terpakai`
-- yang dipakai intake untuk data yang sama; dua aturan jendela adalah sumber
-- selisih yang tidak perlu (review 778f9fd, temuan blocking 1).
--
-- `now_mikro` tetap dipakai, tetapi hanya untuk `lepas` (sisa waktu sampai slot
-- tertua bebas), bukan untuk memilih ember mana yang dihitung.

-- Potongan bersama disisipkan di sini oleh `scripts/rakit-lua.sh`; yang
-- dimuat ke Redis adalah `rakitan/kuota_retry_after.lua`, bukan berkas ini. Sandbox
-- Redis tidak punya `dofile` (review 778f9fd, temuan blocking 2).
local siap_mod = (function()
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
-- Karena disisipkan, isi berkas ini hanya boleh berisi fungsi dan konstanta,
-- tanpa efek samping dan tanpa bergantung pada ARGV global.

local KUNCI_SIAP = 'pr:siap'

-- Mengembalikan `true` bila Redis siap untuk run_id ini, `false` bila belum.
local function siap(run_id)
  if run_id == nil or run_id == '' then
    return false
  end
  local nilai = redis.call('GET', KUNCI_SIAP)
  if not nilai then
    return false
  end
  return nilai == run_id
end

return {
  KUNCI_SIAP = KUNCI_SIAP,
  siap = siap
}
end)()
local bersama = (function()
-- Potongan bersama: invarian ARGV dan jendela bergulir skrip kuota
-- (ADR-0001 rev 15 §5a.1 butir 1d).
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya ke setiap skrip pada baris penanda `-- sisip: potongan/bersama.lua
-- sebagai bersama`; hasilnya ada di `rakitan/`. Sandbox Redis tidak punya
-- `dofile`, jadi berkas ini tidak pernah dimuat sendiri (review 778f9fd,
-- temuan blocking 2).
--
-- §5a.1 butir 1d: periode dan nomor ember 10 menit dihitung SEKALI di Rust
-- dari `requested_at`, lalu dikirim sebagai argumen. Skrip tidak boleh
-- membaca jam (`TIME`) untuk menentukan kunci. Kalau skrip yang menentukan
-- kunci, percobaan dengan `requested_at` 23:59:59,9 yang di-INCR pada
-- 00:00:00,1 masuk ke periode berbeda dengan baris lognya, dan DQ-05 menjadi
-- salah positif di sekitar jam reset.
--
-- Argumen ini dipakai keempat skrip keadaan (`kuota_intake.lua`,
-- `rate_limit_ambil.lua`, `attempts_tambah.lua`, `kuota_retry_after.lua`),
-- supaya tidak ada satu jalur pun yang menyimpang. Komentar `ARGV` di setiap
-- berkas menyebut prefiks yang ditetapkan di sini lalu posisi lokalnya.
--
-- Prefiks boleh berbeda per skrip karena `run_id` (dan `client_id` pada
-- intake) tidak selalu di ARGV[1]. Nilainya tetap urutan yang sama, jadi tiap
-- skrip membaca `geser` = posisi `kunci_attempts`.
--
-- `run_id` TIDAK termasuk blok bersama: `siap.lua` sudah membacanya dari posisi
-- prefiks masing-masing skrip (`ARGV[1]` atau `ARGV[2]`), dan tidak ada skrip
-- yang memakai salinannya. Menyertakannya di sini membuat blok bersama menjadi
-- tujuh argumen, sehingga argumen pertama bergeser satu dan argumen terakhir
-- menjadi nil.
--
-- SATUAN `no_ember`. `no_ember` adalah NOMOR ember (`epoch / 600`), bukan detik
-- epoch dan bukan indeks cincin `nomor mod jumlah_ember`. Jendela bergulir
-- adalah `no_ember - (jumlah_ember - 1) ..= no_ember`, yaitu `jumlah_ember`
-- ember dihitung dalam satuan ember. Mencampur satuan di sini pernah membuat
-- jendela menyempit dari 145 ember menjadi 145 detik (review 778f9fd, temuan
-- blocking 1). `batas_jendela` dan `di_jendela` di bawah satu-satunya tempat
-- aturan itu ditulis; skrip tidak boleh mengurangkan sendiri.

local ARGV_PREFIKS = 4

-- Membaca argumen bersama tanpa validasi tipe. Skrip yang membutuhkannya
-- memvalidasi lebih dulu lewat `periksa`. `geser` = indeks ARGV elemen pertama
-- blok bersama (`kunci_attempts`); tanpa `geser`, blok bersama mulai sesudah
-- prefiks terpanjang.
local function baca(ARGV, geser)
  local p = geser or (ARGV_PREFIKS + 1)
  local a = {}
  a.kunci_attempts = ARGV[p]
  a.kunci_antre = ARGV[p + 1]
  a.kuota_efektif = ARGV[p + 2]
  a.hash_ember = ARGV[p + 3]
  a.jumlah_ember = ARGV[p + 4]
  a.no_ember = ARGV[p + 5]
  return a
end

-- Validasi bersama. Pemanggil Rust bergantung pada himpunan kode galat, bukan
-- pada urutannya. `kuota_efektif` harus bulat > 0, karena
-- `floor(kuota_harian × (1 − margin))` dengan `margin < 1` selalu > 0; nilai
-- tidak sah berarti bug konfigurasi, bukan kuota habis, jadi dilaporkan
-- sebagai galat, bukan penolakan.
--
-- `tonumber` di bawah mengubah field; jangan sampai dipakai untuk validasi,
-- karena hasilnya selalu angka (atau nil). Validasi tipe lewat GSUB + pola.

-- Bilangan bulat tanpa awalan nol (`-0` ditolak, `0` diterima).
local POLA_BULAT = '^%-?%d+$'
local POLA_BULAT_POSITIF = '^%d+$'

local function bulat(nilai)
  if type(nilai) ~= 'string' then
    return nil
  end
  local teks = tostring(nilai)
  if not string.match(teks, POLA_BULAT) then
    return nil
  end
  local n = tonumber(teks)
  if n == nil or n ~= math.floor(n) then
    return nil
  end
  return n
end

-- Nama field ember: `b<nomor ember>`.
local POLA_FIELD_EMBER = '^b(%d+)$'

local function periksa(a)
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

  if type(a.kunci_attempts) ~= 'string' or a.kunci_attempts == '' then
    return 'kunci_attempts_tidak_sah'
  end
  if type(a.kunci_antre) ~= 'string' or a.kunci_antre == '' then
    return 'kunci_antre_tidak_sah'
  end
  if type(a.hash_ember) ~= 'string' or a.hash_ember == '' then
    return 'hash_ember_tidak_sah'
  end
  return nil
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

return {
  ARGV_PREFIKS = ARGV_PREFIKS,
  POLA_FIELD_EMBER = POLA_FIELD_EMBER,
  baca = baca,
  periksa = periksa,
  batas_jendela = batas_jendela,
  di_jendela = di_jendela
}
end)()

local MODE_GULIR = 'bergulir'
local MODE_KALENDER = 'kalender'
local JEDA_EMBER_MIKRO = 600 * 1000000

-- KEYS tidak dipakai: kunci datang di blok ARGV bersama, sama seperti ketiga
-- skrip lain (numkeys 0).
-- ARGV[1] = run_id
-- ARGV[2] = mode jendela saat ini: 'bergulir' | 'kalender'
-- ARGV[3] = now_mikro
-- ARGV[4.. ] = argumen bersama `bersama.lua`, mulai dari kunci_attempts
--              (`no_ember` = nomor ember epoch/600, bukan detik epoch)
if not siap_mod.siap(ARGV[1]) then
  return { 'belum_siap' }
end

local mode = ARGV[2]
if mode ~= MODE_GULIR and mode ~= MODE_KALENDER then
  return { 'mode_tidak_sah' }
end

local now = math.floor(tonumber(ARGV[3]) or -1)
if now < 0 then
  return { 'now_tidak_sah' }
end

local a = bersama.baca(ARGV, 4)
local galat = bersama.periksa(a)
if galat then
  return { galat }
end

local attempts = tonumber(redis.call('GET', a.kunci_attempts) or '0') or 0

local terpakai = attempts
local ember_tertua = nil
if mode == MODE_GULIR then
  terpakai = 0
  local semua = redis.call('HGETALL', a.hash_ember)
  for i = 1, #semua, 2 do
    local n = tonumber(string.match(semua[i], bersama.POLA_FIELD_EMBER))
    if n ~= nil and bersama.di_jendela(a, n) then
      terpakai = terpakai + tonumber(semua[i + 1])
      if ember_tertua == nil or n < ember_tertua then
        ember_tertua = n
      end
    end
  end
end

-- `Retry-After` jendela bergulir = saat percobaan tertua yang masih dihitung
-- keluar dari jendela. Ember `ember_tertua` masih dihitung selama
-- `no_ember <= ember_tertua + jumlah_ember - 1`, jadi slotnya bebas pada
-- `(ember_tertua + jumlah_ember) * 10 menit`. Tanpa isi ember, tidak ada yang
-- perlu ditunggu.
local lepas = 0
if ember_tertua ~= nil then
  lepas = (ember_tertua + a.jumlah_ember) * JEDA_EMBER_MIKRO - now
  if lepas < 0 then
    lepas = 0
  end
end

return { 'ok', terpakai, attempts, a.kuota_efektif, math.floor(lepas / 1000000) }
