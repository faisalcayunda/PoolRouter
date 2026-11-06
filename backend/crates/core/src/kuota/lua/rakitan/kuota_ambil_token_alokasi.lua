-- BERKAS HASIL RAKITAN (scripts/rakit-lua.sh), JANGAN DISUNTING.
-- Sumber: kuota_ambil_token_alokasi.lua di direktori ini, dengan potongan disisipkan pada baris `-- sisip:`.
-- Bentuk inilah yang dimuat ke Redis; berkas sumber tidak boleh dimuat langsung.
-- Buat ulang: make -C backend lua-rakit. Gate: make -C backend lua-rakit-check.

-- Gerbang M-02 versi F1: tahap global F0 lebih dulu, lalu tahap anggaran
-- (prioritas, reserved/shareable, peminjaman) lewat hook sesudah semua gates
-- F0 dan sebelum tulis. ADR-0006 rev 1 §10; spec f1-prioritas-alokasi AC-5.
--
-- Tahap globalnya BUKAN salinan: `potongan/kerangka.lua` + `potongan/
-- ambil_token.lua` dipakai bersama F0, jadi setiap penolakan global
-- (`belum_siap`, `kuota_habis`, `konkurensi_penuh`, `retry_budget_habis`,
-- `rate_limit`) tidak bisa menyimpang dari F0 tanpa mengubah F0 juga. Uji
-- pasangan di `uji_lua.py` menjaga itu.
--
-- `opsi` adalah satu-satunya pembeda: hook `pilih_sumber` dan
-- `kunci_slot_sumber`. KEYS dan ARGV sama persis dengan `kuota_ambil_token.lua`
-- (tabel lengkapnya di kepala berkas itu); didefinisikan sekali di
-- `potongan/kerangka.lua` supaya kedua skrip tidak bisa berbeda. Kontrak hook
-- ada di kepala `potongan/ambil_token.lua`. Pada tiket T2a (ARC-169) hook
-- belum dipasang.
--
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
local ambil_token = (function()
-- Potongan pemberian token: gerbang M-02 sesudah argumen sah
-- (ADR-0001 rev 15 §5a.1 butir 3, 5, 6, 7 dan §5a.3; ADR-0006 rev 1 §10).
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya pada baris penanda `-- sisip: potongan/ambil_token.lua sebagai
-- ambil_token`; hasilnya ada di `rakitan/`. Sandbox Redis tidak punya
-- `dofile`, jadi berkas ini tidak pernah dimuat sendiri.
--
-- Kenapa dipotong. Langkah kuota, konkurensi, anggaran retry dan GCRA adalah
-- bagian yang akan berubah saat F1 menambah prioritas dan peminjaman; skripnya
-- sendiri tidak. Memisahkannya membuat perubahan itu terlihat di satu tempat
-- dan membuat `kuota_ambil_token.lua` tinggal berisi urutan langkah (PA4).
--
-- Kontrak tidak berubah: potongan ini membaca KEYS dan ARGV yang sama dengan
-- skrip F0, mengembalikan tabel yang sama, dan tidak menambah kunci apa pun.
-- `kuota_ambil_token.lua` tetap yang menetapkan urutan KEYS; potongan ini
-- menerimanya sebagai argumen `k` dan `o`.
--
-- Yang di luar potongan: kesiapan Redis, validasi `request_id`, `nomor`,
-- `maks`, `ttl`, `rasio_retry`, `retry_min`, `porsi`, jumlah jendela dan
-- pembacaan `waktu beri` — semuanya tetap di skrip pemanggil, karena itu
-- urusan bentuk panggilan, bukan isi keputusan. Potongan ini dimulai sesudah
-- semuanya dinyatakan sah.
--
-- Pemakaian di skrip:
--
--   local status = ambil_token(KEYS, {
--     a = a, sync = sync, maks = maks, ttl = ttl,
--     rasio_retry = rasio_retry, retry_min = retry_min, porsi = porsi,
--     jendela = jendela, nomor = nomor, request_id = request_id,
--     sekarang = fungsi_mikro
--   })
--   if status.kode ~= 'ok' then return status.tabel end
--
-- Rancangan `o`:

-- `o.a`          hasil `bersama.periksa` (mode, kuota_efektif, jumlah_ember, no_ember)
-- `o.sync`       '0' | '1'
-- `o.maks`       batas slot aktif
-- `o.ttl`        masa hidup slot, mikrodetik
-- `o.rasio_retry`, `o.retry_min`  anggaran retry (ADR-0001 §5a.3)
-- `o.porsi`      pecahan kapasitas untuk sync, 0..1
-- `o.jendela`    daftar `{ t, burst, kunci, kunci_sync }`
-- `o.nomor`      nomor percobaan
-- `o.request_id` UUID percobaan
-- `o.sekarang`   fungsi pengembali mikrodetik (Redis TIME)
-- `o.pilih_sumber`     hook F1 (opsional): dipanggil sesudah semua gates F0,
--                      sebelum tulis; kembalikan nil atau `{ kode, tabel }`
-- `o.kunci_slot_sumber` hook F1 (opsional): dipanggil sesudah slot ditulis
--
-- Kembalian: `{ kode = ..., tabel = <bentuk kembalian skrip F0> }`. `kode`
-- adalah `tabel[1]`, disediakan supaya pemanggil bisa memeriksa tanpa
-- mengindeks tabel.

local function ambil_token(KEYS, o)
  -- Langkah 3: bersihkan slot yang skornya sudah lewat, lalu hitung yang aktif.
  local now = o.sekarang()
  local kedaluwarsa = redis.call('ZREMRANGEBYSCORE', KEYS[4], '-inf', now)
  local slot_aktif = redis.call('ZCARD', KEYS[4])

  -- Langkah 4: gerbang kuota. `slot_aktif` ikut dihitung, karena setiap slot
  -- adalah percobaan yang akan terjadi walau belum di-INCR.
  local terpakai = bersama.terpakai(KEYS[2], KEYS[3], o.a)
  if terpakai + slot_aktif >= o.a.kuota_efektif then
    return { kode = 'kuota_habis', tabel = { 'kuota_habis', terpakai, slot_aktif, kedaluwarsa, 0, 0, 0 } }
  end

  -- Langkah 5: gerbang konkurensi upstream.
  if slot_aktif >= o.maks then
    return { kode = 'konkurensi_penuh', tabel = { 'konkurensi_penuh', terpakai, slot_aktif, kedaluwarsa, 0, 0, 0 } }
  end

  -- Langkah 6: anggaran retry, jendela bergulir 1 jam dari 60 bucket menit
  -- (ADR-0001 §5a.3). Percobaan ulang (`nomor > 1`) hanya diberi token bila
  -- `retry_jendela < max(retry_min, rasio_retry × percobaan_jendela)`.
  -- `percobaan_jendela` menghitung SEMUA token yang diberikan dalam jendela
  -- (percobaan pertama dan retry), `retry_jendela` hanya retry. Pembacaan
  -- murni; penulisannya di langkah 8, bersama pemberian token.
  local menit = math.floor(now / 60000000)
  local MENIT_JENDELA_RETRY = 60
  if o.nomor > 1 then
    local batas_bawah = menit - (MENIT_JENDELA_RETRY - 1)
    local semua = redis.call('HGETALL', KEYS[5])
    local percobaan_jendela = 0
    local retry_jendela = 0
    for i = 1, #semua, 2 do
      local jenis, m = string.match(semua[i], '^([pr])(%d+)$')
      m = tonumber(m)
      if jenis ~= nil and m >= batas_bawah and m <= menit then
        local nilai = tonumber(semua[i + 1])
        if jenis == 'p' then
          percobaan_jendela = percobaan_jendela + nilai
        else
          retry_jendela = retry_jendela + nilai
        end
      end
    end
    local anggaran = math.max(o.retry_min, o.rasio_retry * percobaan_jendela)
    if retry_jendela >= anggaran then
      return { kode = 'retry_budget_habis', tabel = { 'retry_budget_habis', terpakai, slot_aktif, kedaluwarsa, 0, 0, 0 } }
    end
  end

  -- Langkah 7: semua jendela GCRA, tanpa menulis apa pun. Kalau ada yang
  -- menolak, skrip berhenti sebagai penolakan murni: tidak ada TAT yang
  -- tersimpan dan tidak ada slot yang dipesan. Waktu tunggu diambil yang
  -- terbesar, karena semua jendela harus terlewati sebelum panggilan
  -- berikutnya diizinkan.
  if o.sync == '1' and o.porsi == 0 then
    return { kode = 'porsi_sync_habis', tabel = { 'porsi_sync_habis', terpakai, slot_aktif, kedaluwarsa, 0, 0, 0 } }
  end

  local tunggu = 0
  local isi = math.huge
  local isi_porsi = math.huge

  local function periksa_jendela(kunci, burst, t)
    local tat = tonumber(redis.call('GET', kunci) or '0')
    if tat < now then
      tat = now
    end
    local geser = (burst - 1) * t
    if tat - geser > now then
      local sisa = (tat - geser) - now
      if sisa > tunggu then
        tunggu = sisa
      end
    end
    local sisa_token = math.floor((now - tat) / t) + burst
    if sisa_token < 0 then
      sisa_token = 0
    elseif sisa_token > burst then
      sisa_token = burst
    end
    return tat, sisa_token
  end

  for i = 1, #o.jendela do
    local w = o.jendela[i]
    local _, sisa = periksa_jendela(w.kunci, w.burst, w.t)
    if sisa < isi then
      isi = sisa
    end
    if o.sync == '1' then
      local burst_porsi = math.floor(w.burst * o.porsi)
      if burst_porsi < 1 then
        burst_porsi = 1
      end
      local _, sisa_porsi = periksa_jendela(w.kunci_sync, burst_porsi, w.t)
      if sisa_porsi < isi_porsi then
        isi_porsi = sisa_porsi
      end
    end
  end

  if isi == math.huge then
    isi = 0
  end
  if isi_porsi == math.huge then
    isi_porsi = 0
  end

  if tunggu > 0 then
    return { kode = 'rate_limit', tabel = { 'rate_limit', terpakai, slot_aktif, kedaluwarsa, tunggu, isi, isi_porsi } }
  end

  -- Tahap anggaran F1 (hook, AC-5/AC-6): sesudah SEMUA gates F0 dan sebelum
  -- tulis. Hook menolak dengan `{ kode, tabel }` seperti rejection global;
  -- saat itu belum ada satu pun kunci yang berubah. `opsi` nil = F0 apa adanya.
  if o.pilih_sumber then
    local tolak = o.pilih_sumber(KEYS, o, terpakai, slot_aktif)
    if tolak then
      return tolak
    end
  end

  -- Langkah 8: tulis. TAT baru = TAT lama + T, dan TTL-nya `TAT_baru − now +
  -- 60 s`, bukan `TAT − now`: dengan TTL lama, kunci bisa hilang sebelum TAT
  -- terlewati sehingga satu menit pertama sesudahnya menerima burst penuh
  -- (keputusan Shu D2.9).
  for i = 1, #o.jendela do
    local w = o.jendela[i]
    local pasangan = { { w.kunci, w.burst } }
    if o.sync == '1' then
      local burst_porsi = math.floor(w.burst * o.porsi)
      if burst_porsi < 1 then
        burst_porsi = 1
      end
      pasangan[#pasangan + 1] = { w.kunci_sync, burst_porsi }
    end
    for j = 1, #pasangan do
      local kunci = pasangan[j][1]
      local tat = tonumber(redis.call('GET', kunci) or '0')
      if tat < now then
        tat = now
      end
      local baru = tat + w.t
      redis.call('SET', kunci, tostring(baru), 'PX', math.floor((baru - now) / 1000) + 60000)
    end
  end

  -- Slot: member `<request_id>:<nomor_percobaan>`, skor `waktu_beri + ttl_slot`.
  -- `NX` membuat pemberian ulang untuk percobaan yang sama gagal keras
  -- (`slot_ganda`) alih-alih memperpanjang diam-diam: itu selalu bug pemanggil.
  local member = o.request_id .. ':' .. tostring(o.nomor)
  local ditambah = redis.call('ZADD', KEYS[4], 'NX', now + o.ttl, member)
  if ditambah == 0 then
    return { kode = 'slot_ganda', tabel = { 'slot_ganda', terpakai, slot_aktif, kedaluwarsa, 0, isi, isi_porsi } }
  end

  -- Sumber slot (hook T2c): dicatat sesudah slot benar-benar milik pemanggil,
  -- supaya `slot_klaim.lua`/`slot_lepas.lua` bisa mengembalikan bebannya.
  if o.kunci_slot_sumber then
    o.kunci_slot_sumber(KEYS, o, member)
  end

  -- Anggaran retry: catat percobaan (`p<menit>`) dan, bila retry, `r<menit>`.
  -- Bucket yang lebih tua dari jendela dipangkas di sini, satu-satunya tempat
  -- hash ini ditulis, supaya hash tidak tumbuh tanpa batas. TTL 2 jam menjaga
  -- hash dari pool yang berhenti dipakai.
  redis.call('HINCRBY', KEYS[5], 'p' .. menit, 1)
  if o.nomor > 1 then
    redis.call('HINCRBY', KEYS[5], 'r' .. menit, 1)
  end
  local batas_pangkas = menit - (MENIT_JENDELA_RETRY - 1)
  local lama = redis.call('HKEYS', KEYS[5])
  for i = 1, #lama do
    local m = tonumber(string.match(lama[i], '^[pr](%d+)$'))
    if m == nil or m < batas_pangkas then
      redis.call('HDEL', KEYS[5], lama[i])
    end
  end
  redis.call('PEXPIRE', KEYS[5], 7200000)

  return { kode = 'ok', tabel = { 'ok', terpakai, slot_aktif + 1, kedaluwarsa, 0, isi, isi_porsi } }
end

return {
  ambil_token = ambil_token
}
end)()
local kerangka = (function()
-- Potongan kerangka: bentuk panggilan gerbang M-02 yang dipakai bersama
-- `kuota_ambil_token.lua` (F0) dan `kuota_ambil_token_alokasi.lua` (F1).
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya pada baris penanda `-- sisip: ... sebagai kerangka`; hasilnya ada di
-- `rakitan/`. Sandbox Redis tidak punya `dofile`, jadi berkas ini tidak pernah
-- dimuat sendiri.
--
-- Kenapa dipotong. Validasi ARGV dan urutan gates F0 hanya boleh punya satu
-- salinan; skrip alokasi F1 membedakan diri lewat `opsi`, bukan lewat salinan
-- validasi. `opsi` berisi hook opsional untuk T2b/T2c:
--   * `pilih_sumber(KEYS, o)` → nil atau `{ kode = '...', tabel = { ... } }`,
--     dipanggil di `potongan/ambil_token.lua` sesudah semua gates F0 dan
--     sebelum tulis;
--   * `kunci_slot_sumber(KEYS, o, member)` → mencatat sumber slot yang baru
--     ditulis.
-- `opsi` nil berarti perilaku F0 apa adanya (AC-5).

local function kerangka(KEYS, ARGV, opsi)
  local run_id = ARGV[1]
  if not siap(KEYS[1], run_id) then
    return { 'belum_siap' }
  end

  local request_id = ARGV[2]
  if request_id == nil or request_id == '' or string.find(request_id, ':', 1, true) then
    return { 'request_id_tidak_sah' }
  end

  local nomor = bersama.bulat(ARGV[3])
  if nomor == nil or nomor < 1 then
    return { 'nomor_percobaan_tidak_sah' }
  end

  local a = {
    mode = ARGV[4],
    kuota_efektif = ARGV[6],
    jumlah_ember = ARGV[7],
    no_ember = ARGV[8]
  }
  local galat = bersama.periksa(a)
  if galat then
    return { galat }
  end

  local sync = ARGV[5]
  if sync ~= '0' and sync ~= '1' then
    return { 'sync_tidak_sah' }
  end

  local maks = bersama.bulat(ARGV[9])
  if maks == nil or maks < 1 then
    return { 'maks_konkurensi_tidak_sah' }
  end

  local ttl = bersama.bulat(ARGV[10])
  if ttl == nil or ttl < 1 then
    return { 'ttl_slot_tidak_sah' }
  end

  -- Anggaran retry (ADR-0001 §5a.3): `rasio_retry` 0..1 dan `retry_min` bulat >= 0.
  -- Tidak ada nilai "tanpa anggaran": `0` dan `0` berarti tidak ada retry sama
  -- sekali, karena syaratnya `retry_jendela < max(retry_min, rasio × percobaan)`.
  local rasio_retry = tonumber(ARGV[11])
  if rasio_retry == nil or rasio_retry ~= rasio_retry or rasio_retry < 0 or rasio_retry > 1 then
    return { 'rasio_retry_tidak_sah' }
  end
  local retry_min = bersama.bulat(ARGV[12])
  if retry_min == nil or retry_min < 0 then
    return { 'retry_min_tidak_sah' }
  end

  local porsi = tonumber(ARGV[13])
  if porsi == nil or porsi < 0 or porsi > 1 then
    return { 'porsi_sync_tidak_sah' }
  end

  local n = bersama.bulat(ARGV[14])
  if n == nil or n < 1 then
    return { 'jumlah_jendela_tidak_sah' }
  end
  if #KEYS ~= 5 + 2 * n then
    return { 'kunci_tidak_sah' }
  end
  if not bersama.kunci_sah(KEYS[1], KEYS[2], KEYS[3], KEYS[4], KEYS[5]) then
    return { 'kunci_tidak_sah' }
  end

  local jendela = {}
  for i = 1, n do
    local t = bersama.bulat(ARGV[14 + 2 * i - 1])
    local burst = bersama.bulat(ARGV[14 + 2 * i])
    if t == nil or t < 1 or burst == nil or burst < 1 then
      return { 'jendela_tidak_sah' }
    end
    local kunci = KEYS[5 + 2 * i - 1]
    local kunci_sync = KEYS[5 + 2 * i]
    if not bersama.kunci_sah(kunci, kunci_sync) then
      return { 'kunci_tidak_sah' }
    end
    jendela[i] = { t = t, burst = burst, kunci = kunci, kunci_sync = kunci_sync }
  end

  local o = {
    a = a,
    sync = sync,
    maks = maks,
    ttl = ttl,
    rasio_retry = rasio_retry,
    retry_min = retry_min,
    porsi = porsi,
    jendela = jendela,
    nomor = nomor,
    request_id = request_id,
    sekarang = bersama.mikro
  }
  if opsi ~= nil then
    for k, v in pairs(opsi) do
      o[k] = v
    end
  end

  local hasil = ambil_token.ambil_token(KEYS, o)
  return hasil.tabel
end

return { jalankan = kerangka }
end)()

-- shortcut: tahap anggaran (pilih_sumber/kunci_slot_sumber) belum diisi;
-- skrip ini masih berperilaku sama dengan F0. Upgrade di T2b (ARC-170).
return kerangka.jalankan(KEYS, ARGV, nil)
