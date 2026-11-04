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
