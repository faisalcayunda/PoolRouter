-- BERKAS HASIL RAKITAN (scripts/rakit-lua.sh), JANGAN DISUNTING.
-- Sumber: kuota_intake_alokasi.lua di direktori ini, dengan potongan disisipkan pada baris `-- sisip:`.
-- Bentuk inilah yang dimuat ke Redis; berkas sumber tidak boleh dimuat langsung.
-- Buat ulang: make -C backend lua-rakit. Gate: make -C backend lua-rakit-check.

-- Gerbang intake beralokasi: cek intake F0 lebih dulu, lalu syarat `antre_d`
-- rute ini (ADR-0006 rev 1 §10; spec `f1-prioritas-alokasi` AC-8).
--
-- Skrip ini hanya dipakai untuk upstream pool yang punya rute beralokasi.
-- Upstream pool tanpa alokasi tetap memakai `kuota_intake.lua`; pemilihan
-- skripnya dilakukan sekali per panggilan di lapisan `KuotaStore`.
--
-- KEYS (D1.7, urutannya tetap):
--   1 pr:siap                          gerbang kesiapan
--   2 pr:attempts:{<pool>}:<periode>   counter percobaan periode ini
--   3 pr:ember:{<pool>}                hash ember 10 menit (mode bergulir)
--   4 pr:antre:{<pool>}                jumlah permintaan async yang diterima,
--                                      belum dipanggil
--   5 pr:antre:dp:{<pool>}:<d>         jumlah permintaan async rute `d` yang
--                                      diterima, belum dipanggil
--
-- ARGV: 1–7 sama dengan `kuota_intake.lua`, ditambah
--   8 ruang_intake        `rencana::ruang_intake(d)` sebagai teks, `>= 0`
--
-- Kembalian: sama dengan `kuota_intake.lua`, ditambah
--   {'kuota_habis', terpakai, antre}    juga saat `antre_d >= ruang_intake`
--   {'ruang_intake_tidak_sah'}          ARGV[8] bukan bilangan `>= 0`
--
-- Kedua counter antre dinaikkan di EVAL yang sama, jadi penolakan tidak
-- meninggalkan antrean yang naik (AC-8).

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
local intake = (function()
-- Potongan gerbang intake: keputusan penerimaan sesudah argumen sah
-- (ADR-0001 rev 15 §5a.1 butir 2, K-12; ADR-0006 rev 1 §10).
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya pada baris penanda `-- sisip: potongan/intake.lua sebagai intake`;
-- hasilnya ada di `rakitan/`, dan itu bentuk yang dimuat Redis.
--
-- Kenapa dipotong. Intake adalah tempat F1 menambahkan `ruang_intake()` dan
-- peminjaman (PA4, AC-3); kerangka pembacaan ARGV-nya tidak berubah. Memisah
-- keputusan dari bentuk panggilan membuat perubahan itu satu berkas, dan
-- membuat kontrak F0 tetap terlihat utuh di `kuota_intake.lua`.
--
-- Pemakaian di skrip:
--
--   return gerbang_intake(KEYS, {
--     sync = sync, a = a, kunci_antre = KEYS[4]
--   })
--
-- Kembaliannya adalah tabel kembalian skrip F0 apa adanya.

-- Pemakaian `o`:
-- `o.a`          hasil `bersama.periksa` (mode, kuota_efektif, jumlah_ember, no_ember)
-- `o.sync`       '0' | '1'
-- `o.saring`     opsional; `function(terpakai, antre)` yang mengembalikan
--                tabel penolakan atau nil. Dipanggil sesudah cek F0 dan
--                sebelum `INCR antre` (dipakai `potongan/intake_alokasi.lua`)
--
-- Cek antrean memakai `terpakai + antre`, bukan `antre` saja: yang perlu
-- dijaga adalah jumlah percobaan yang akan terjadi, bukan panjang antrean.
-- Dengan begitu `INCR antre` tidak bisa membuat antrean melampaui
-- `kuota_efektif − terpakai` (D1.6).
--
-- Permintaan SYNC tidak diperiksa terhadap antrean: ia tidak masuk antrean
-- sama sekali, jadi panjang antrean tidak mengatakan apa pun tentang
-- kapasitas yang tersisa untuknya. Yang membatasinya kelak adalah slot di
-- `kuota_ambil_token.lua`, yang menghitung `terpakai + slot_aktif`.
--
-- Kedua penolakan tetap dijawab 429 oleh pemanggil, tetapi kodenya dibedakan
-- supaya metrik dan jalur retry bisa membedakan "kuota habis" dari "antrean
-- penuh" (§5a.1 butir 2).

local function gerbang_intake(KEYS, o)
  local terpakai = bersama.terpakai(KEYS[2], KEYS[3], o.a)
  local antre = tonumber(redis.call('GET', KEYS[4]) or '0')
  if antre < 0 then
    antre = 0
  end

  if terpakai >= o.a.kuota_efektif then
    return { 'kuota_habis', terpakai, antre }
  end
  if o.sync == '1' and terpakai + antre >= o.a.kuota_efektif then
    return { 'kuota_habis', terpakai, antre }
  end
  if terpakai + antre >= o.a.kuota_efektif then
    return { 'antrean_penuh', terpakai, antre }
  end

  -- Kait F1 (PA4, AC-8): syarat tambahan sesudah cek F0 dan SEBELUM
  -- `INCR antre`, supaya penolakan tidak menaikkan antrean. `potongan/
  -- intake_alokasi.lua` yang mengisi kait ini; skrip F0 tidak.
  if o.saring then
    local tolak = o.saring(terpakai, antre)
    if tolak then
      return tolak
    end
  end

  if o.sync == '0' then
    antre = redis.call('INCR', KEYS[4])
  end

  return { 'lolos', terpakai, antre }
end

return {
  gerbang_intake = gerbang_intake
}
end)()
local intake_alokasi = (function()
-- Potongan gerbang intake beralokasi: syarat kedua sesudah cek F0
-- (ADR-0006 rev 1 §10; spec `f1-prioritas-alokasi` AC-8).
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya pada baris penanda `-- sisip: potongan/intake_alokasi.lua sebagai
-- intake_alokasi`; hasilnya ada di `rakitan/`, dan itu bentuk yang dimuat
-- Redis.
--
-- Kenapa dipotong. Keputusannya tetap satu: `intake.gerbang_intake` memutuskan
-- `kuota_habis` dan `antrean_penuh` F0, lalu `intake_alokasi` menambah satu
-- syarat lewat `o.saring`, yang dipanggil gerbang F0 SESUDAH semua ceknya dan
-- SEBELUM `INCR antre`. Karena kaitnya di dalam satu EVAL, penolakan tidak
-- meninggalkan counter antre yang naik.
--
-- `ruang_intake(d)` dihitung di Rust (`core::kuota::rencana::ruang_intake`);
-- skrip ini menerimanya sebagai ARGV[8]. Intake adalah estimasi penerimaan,
-- sama seperti di F0; penegakan atomiknya di dispatch. Karena sync tidak masuk
-- antrean, ia juga tidak diperiksa terhadap `antre_d` (alasan yang sama dengan
-- pengecualian sync di `potongan/intake.lua`).
--
-- Pemakaian `o`:
-- `o.a`            hasil `bersama.periksa`
-- `o.sync`         '0' | '1'
-- `o.ruang_intake` ruang rute ini (`>= 0`) sebagai teks
-- `KEYS[5]`        `pr:antre:dp:{<pool>}:<downstream_pool_id>`

-- Kembaliannya adalah tabel kembalian skrip apa adanya; penolakan memakai kode
-- `kuota_habis` (429) yang sudah ada, bukan kode baru.
local function gerbang_intake_alokasi(KEYS, o)
  local ruang = tonumber(o.ruang_intake)
  if ruang == nil or ruang < 0 then
    return { 'ruang_intake_tidak_sah' }
  end

  return intake.gerbang_intake(KEYS, {
    a = o.a,
    sync = o.sync,
    saring = function(terpakai, antre)
      if o.sync == '1' then
        return nil
      end
      local antre_d = tonumber(redis.call('GET', KEYS[5]) or '0')
      if antre_d == nil or antre_d < 0 then
        antre_d = 0
      end
      if antre_d >= ruang then
        return { 'kuota_habis', terpakai, antre }
      end
      redis.call('INCR', KEYS[5])
      return nil
    end,
  })
end

return {
  gerbang_intake_alokasi = gerbang_intake_alokasi
}
end)()

local run_id = ARGV[1]
if not siap(KEYS[1], run_id) then
  return { 'belum_siap' }
end
if not bersama.kunci_sah(KEYS[1], KEYS[2], KEYS[3], KEYS[4], KEYS[5]) then
  return { 'kunci_tidak_sah' }
end

local a = {
  mode = ARGV[2],
  kuota_efektif = ARGV[5],
  jumlah_ember = ARGV[6],
  no_ember = ARGV[7]
}
local galat = bersama.periksa(a)
if galat then
  return { galat }
end

local sync = ARGV[3]
if sync ~= '0' and sync ~= '1' then
  return { 'sync_tidak_sah' }
end

return intake_alokasi.gerbang_intake_alokasi(KEYS, {
  sync = sync,
  a = a,
  ruang_intake = ARGV[8],
})
