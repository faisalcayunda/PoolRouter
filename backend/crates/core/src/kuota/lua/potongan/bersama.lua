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
