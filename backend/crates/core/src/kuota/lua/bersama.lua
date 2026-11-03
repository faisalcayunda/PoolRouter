-- Invarian ARGV bersama untuk skrip kuota (ADR-0001 rev 15 §5a.1 butir 1d).
--
-- §5a.1 butir 1d: periode dan indeks ember 10 menit dihitung SEKALI di Rust
-- dari `requested_at`, lalu dikirim sebagai argumen. Skrip tidak boleh
-- membaca jam (`TIME`) untuk menentukan kunci. Kalau skrip yang menentukan
-- kunci, percobaan dengan `requested_at` 23:59:59,9 yang di-INCR pada
-- 00:00:00,1 masuk ke periode berbeda dengan baris lognya, dan DQ-05 menjadi
-- salah positif di sekitar jam reset.
--
-- Argumen ini dipakai keempat skrip keadaan (`kuota_intake.lua`,
-- `kuota_ambil_token.lua`, `attempts_tambah.lua`, `kuota_retry_after.lua`),
-- supaya tidak ada satu jalur pun yang menyimpang. Komentar `ARGV` di setiap
-- berkas menyebut prefiks yang ditetapkan di sini lalu posisi lokalnya.
--
-- Prefiks boleh berbeda per skrip karena `run_id` tidak selalu di ARGV[1]
-- (`kuota_intake.lua` menaruh `client_id` lebih dulu). Nilainya tetap urutan
-- yang sama, jadi tiap skrip cukup menyebut geserannya.

local ARGV_PREFIKS = 4

-- Membaca argumen bersama tanpa validasi tipe. Skrip yang membutuhkannya
-- (semua kecuali `kuota_retry_after.lua`, yang murni baca) memvalidasi lebih
-- dulu lewat `periksa`. `geser` = indeks ARGV elemen pertama setelah prefiks
-- skrip pemanggil, yaitu elemen pertama blok bersama (`kunci_attempts`); tanpa
-- `geser`, blok bersama mulai sesudah prefiks terpanjang.
--
-- `run_id` TIDAK termasuk blok bersama: `siap.lua` sudah membacanya dari posisi
-- prefiks masing-masing skrip (`ARGV[1]` atau `ARGV[2]`), dan tidak ada skrip
-- yang memakai salinannya. Menyertakannya di sini membuat blok bersama menjadi
-- tujuh argumen, sehingga argumen pertama bergeser satu dan argumen terakhir
-- menjadi nil.
local function baca(ARGV, geser)
  local p = geser or (ARGV_PREFIKS + 1)
  local a = {}
  a.kunci_attempts = ARGV[p]
  a.kunci_antre = ARGV[p + 1]
  a.kuota_efektif = ARGV[p + 2]
  a.hash_ember = ARGV[p + 3]
  a.jumlah_ember = ARGV[p + 4]
  a.ember_mulai = ARGV[p + 5]
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

  local mulai = bulat(a.ember_mulai)
  if mulai == nil or mulai < 0 then
    return 'ember_mulai_tidak_sah'
  end
  a.ember_mulai = mulai

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

return {
  ARGV_PREFIKS = ARGV_PREFIKS,
  baca = baca,
  periksa = periksa
}
