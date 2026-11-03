-- BERKAS HASIL RAKITAN (scripts/rakit-lua.sh), JANGAN DISUNTING.
-- Sumber: rate_limit_ambil.lua di direktori ini, dengan potongan disisipkan pada baris `-- sisip:`.
-- Bentuk inilah yang dimuat ke Redis; berkas sumber tidak boleh dimuat langsung.
-- Buat ulang: make -C backend lua-rakit. Gate: make -C backend lua-rakit-check.

-- Pemberian token GCRA + porsi sync, atomik untuk ketiga jendela.
-- ADR-0001 rev 15 §5 (butir rate limit), §5a.1 butir 1a.
--
-- Satu skrip memeriksa dan memotong TPS, TPM, dan TPH sekaligus. Skrip
-- terpisah per jendela menghasilkan urutan di mana TPS lolos lalu TPM ditolak
-- tanpa atomisitas di antara keduanya, sehingga pemanggil tidak bisa tahu
-- jendela mana yang sudah terpakai.
--
-- Token bucket GCRA, state per jendela:
--   pr:rl:{pool}:{jendela}       = TAT (theoretical arrival time, mikrodetik)
--   pr:rl:{pool}:{jendela}:sync  = TAT bucket porsi sync
-- Interval emisi T = 1e6/rate, toleransi tol = burst*T - T. Permintaan
-- diterima bila TAT - tol <= sekarang; TAT lalu max(TAT, sekarang) + T.
--
-- Porsi sync (K-10): sync memakai bucket global DAN bucket sendiri sebesar
-- share*rate; async hanya bucket global. Dispatcher menghentikan pengambilan
-- token async saat isi bucket global di bawah cadangan sync (share*burst);
-- skrip mengembalikan isi bucket, pemutusnya di Rust
-- (`kuota::porsi_sync::async_boleh_jalan`) supaya ambangnya satu tempat.
--
-- Jam: satu-satunya tempat skrip kuota membaca `TIME`, dan hanya untuk bucket
-- rate limit. Rate limit adalah properti saat token diambil, bukan properti
-- percobaan; periode dan ember kuota tetap datang dari Rust
-- (`bersama.lua`, §5a.1 butir 1d). Satu jam (Redis) untuk semua instance
-- berarti webserver dan dispatcher tidak bisa melihat waktu berbeda.
--
-- Kalau porsi sync habis, permintaan sync TIDAK turun ke porsi async: kode
-- `porsi_sync_habis`. Pemanggil sync menerjemahkannya ke `kuota_habis` sync
-- (502 `retry_habis`), bukan `penolakan_berlebih`.
--
-- Skrip tidak menulis apa pun sebelum seluruh validasi lolos dan semua
-- jendela diuji, jadi penolakan tidak memakan token jendela lain.

-- Potongan bersama disisipkan di sini oleh `scripts/rakit-lua.sh`; yang
-- dimuat ke Redis adalah `rakitan/rate_limit_ambil.lua`, bukan berkas ini. Sandbox
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

local JENIS_JENDELA = { 'tps', 'tpm', 'tph' }
local ARGV_AWAL_JENDELA = 6

-- ARGV[1] = run_id
-- ARGV[2] = kunci_pool (bagian dari kunci rate limit)
-- ARGV[3] = sync: '1' | '0'
-- ARGV[4] = jumlah jendela N (1..3)
-- ARGV[5] = share porsi sync, '0' bila tidak ada sync
-- lalu per jendela i = 1..N: ARGV[5 + 2i] = rate_i (token/detik),
--                             ARGV[6 + 2i] = burst_i (token)
if not siap_mod.siap(ARGV[1]) then
  return { 'belum_siap' }
end

local kunci_pool = ARGV[2]
if kunci_pool == nil or kunci_pool == '' then
  return { 'kunci_pool_tidak_sah' }
end

local sync = ARGV[3]
if sync ~= '0' and sync ~= '1' then
  return { 'sync_tidak_sah' }
end
sync = sync == '1'

local n = math.floor(tonumber(ARGV[4]) or -1)
if n < 1 or n > #JENIS_JENDELA then
  return { 'jumlah_jendela_tidak_sah' }
end

local share = tonumber(ARGV[5])
if share == nil or share < 0 or share >= 1 then
  return { 'share_tidak_sah' }
end

local jendela = {}
for i = 1, n do
  local rate = tonumber(ARGV[ARGV_AWAL_JENDELA + 2 * (i - 1)])
  local burst = tonumber(ARGV[ARGV_AWAL_JENDELA + 2 * (i - 1) + 1])
  if rate == nil or rate <= 0 or burst == nil or burst < 1 then
    return { 'parameter_rate_tidak_sah' }
  end
  local t = 1000000 / rate
  local j = {
    jenis = JENIS_JENDELA[i],
    rate = rate,
    burst = burst,
    kunci = 'pr:rl:' .. kunci_pool .. ':' .. JENIS_JENDELA[i],
    t = t,
    tol = burst * t - t
  }
  if sync and share > 0 then
    local burst_sync = math.max(burst * share, 1)
    j.kunci_sync = j.kunci .. ':sync'
    j.rate_sync = rate * share
    j.burst_sync = burst_sync
    j.t_sync = 1000000 / j.rate_sync
    j.tol_sync = burst_sync * j.t_sync - j.t_sync
  end
  jendela[i] = j
end

-- Waktu Redis dalam mikrodetik; satu panggilan untuk semua jendela.
local waktu = redis.call('TIME')
local sekarang = tonumber(waktu[1]) * 1000000 + tonumber(waktu[2])

-- Isi bucket pada `saat` untuk TAT `tat`, dibatasi `burst`. Hanya laporan:
-- keputusan penolakan dan `Retry-After` dihitung di Rust (§5a.7).
local function isi(tat, t, burst, saat)
  local tersedia = math.floor((saat + t - tat) / t)
  if tersedia < 0 then
    tersedia = 0
  elseif tersedia > burst then
    tersedia = burst
  end
  return tersedia
end

-- TAT yang tersimpan; kunci tidak ada berarti bucket penuh (`sekarang`).
local function tat_tersimpan(kunci)
  local tat = tonumber(redis.call('GET', kunci) or '0')
  if tat == nil or tat <= 0 then
    return sekarang
  end
  return tat
end

-- Tahap 1: uji tanpa menulis apa pun.
local jendela_ditolak = ''
local porsi_ditolak = false
local isi_global = {}
local isi_porsi = {}
for i = 1, n do
  local j = jendela[i]
  j.tat = tat_tersimpan(j.kunci)
  local boleh_global = j.tat - j.tol <= sekarang
  local boleh_porsi
  if j.kunci_sync then
    j.tat_sync = tat_tersimpan(j.kunci_sync)
    boleh_porsi = j.tat_sync - j.tol_sync <= sekarang
    isi_porsi[i] = isi(j.tat_sync, j.t_sync, j.burst_sync, sekarang)
  else
    -- Pool tanpa porsi sync: sync tidak boleh memakai token sama sekali.
    boleh_porsi = not sync
    isi_porsi[i] = isi(j.tat, j.t, j.burst, sekarang)
  end
  isi_global[i] = isi(j.tat, j.t, j.burst, sekarang)
  if not boleh_global and jendela_ditolak == '' then
    jendela_ditolak = j.jenis
  end
  if not boleh_porsi then
    porsi_ditolak = true
  end
end

local kode = 'lolos'
if porsi_ditolak then
  kode = 'porsi_sync_habis'
elseif jendela_ditolak ~= '' then
  kode = 'rate_limit'
end

-- Tahap 2: potong token bila lolos. Global dulu, lalu bucket porsi sync.
if kode == 'lolos' then
  for i = 1, n do
    local j = jendela[i]
    local tat_baru = math.max(j.tat, sekarang) + j.t
    -- TTL = waktu bucket terisi penuh lagi + 60 detik sisa pembulatan.
    local ttl_ms = (math.ceil(j.tol / 1000000) + 60) * 1000
    redis.call('SET', j.kunci, string.format('%d', math.floor(tat_baru)), 'PX', ttl_ms)
    isi_global[i] = isi(tat_baru, j.t, j.burst, sekarang)
    if j.kunci_sync then
      local tat_porsi = math.max(j.tat_sync, sekarang) + j.t_sync
      local ttl_porsi = (math.ceil(j.tol_sync / 1000000) + 60) * 1000
      redis.call('SET', j.kunci_sync, string.format('%d', math.floor(tat_porsi)), 'PX', ttl_porsi)
      isi_porsi[i] = isi(tat_porsi, j.t_sync, j.burst_sync, sekarang)
    end
  end
end

-- Laporan: `jenis:isi_global:isi_porsi` per jendela, dipisah koma.
local laporan = {}
for i = 1, n do
  laporan[#laporan + 1] = string.format('%s:%d:%d', jendela[i].jenis, isi_global[i], isi_porsi[i])
end

return { kode, table.concat(laporan, ','), jendela_ditolak }
