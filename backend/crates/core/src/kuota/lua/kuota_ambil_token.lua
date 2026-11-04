-- Gerbang M-02: periksa kuota + jendela rate limit, lalu pesan slot, dalam
-- SATU EVAL. ADR-0001 rev 15 §5a.1 butir 3 dan 6, §5a.4 (timeout slot).
--
-- Inilah satu-satunya tempat percobaan boleh dimulai. `kuota_intake.lua`
-- hanya kontrol penerimaan, `attempts_tambah.lua` hanya menaikkan counter
-- sesudah baris log commit. Karena pemeriksaan `terpakai + slot_aktif` dan
-- `ZADD` slot berada di skrip yang sama, dua pemberian token yang bersamaan
-- tidak bisa sama-sama melihat sisa kuota yang sama: yang kalah melihat slot
-- milik pemenang (keputusan Shu SHAA-195 butir 1, opsi C).
--
-- Menggantikan `rate_limit_ambil.lua`: jendela GCRA dan pemesanan slot tidak
-- boleh dipisah, karena slot adalah bukti bahwa jendela sudah diperiksa.
--
-- KEYS (D1.7, urutannya tetap):
--   1 pr:siap                            gerbang kesiapan
--   2 pr:attempts:{<pool>}:<periode>     counter percobaan periode ini
--   3 pr:ember:{<pool>}                  hash ember 10 menit (mode bergulir)
--   4 pr:slot:{<pool>}                   semaphore konkurensi, skor = kedaluwarsa
--   5 pr:retry:{<pool>}                  hash anggaran retry: p<menit>, r<menit>
--   6, 8, …  pr:rl:{<pool>}:<jenis>      TAT GCRA jendela ke-1..N
--   7, 9, …  pr:rl:{<pool>}:<jenis>:sync porsi sync jendela yang sama
--
-- ARGV:
--   1  run_id              dari `INFO server`, dicocokkan dengan KEYS[1]
--   2  request_id          UUID percobaan; member slot `<request_id>:<n>`
--   3  nomor_percobaan     1 untuk panggilan pertama, >1 untuk retry
--   4  mode                `bergulir` | `kalender`
--   5  sync                `1` permintaan sync, `0` async
--   6  kuota_efektif       `floor(kuota_harian × (1 − margin))`
--   7  jumlah_ember        `JUMLAH_EMBER` (145)
--   8  no_ember            `floor(requested_at_unix / 600)`
--   9  maks_konkurensi     batas slot aktif bersamaan (§5a.1 butir 5)
--   10 ttl_slot_mikro      `timeout_total_upstream + 60 s`, dalam mikrodetik
--   11 rasio_retry         `retry_rasio` pool, `0`..`1` (ADR-0001 §5a.3; default 0,10)
--   12 retry_min           `retry_min_per_jam` pool, bilangan bulat >= 0 (default 20)
--   13 porsi_sync          pecahan kapasitas untuk sync, `0`..`1`
--   14 jumlah_jendela      N, banyaknya pasangan jendela di KEYS/ARGV
--   15… (t_mikro, burst)   per jendela: jarak antar token dan kapasitas bucket
--
-- Kembalian (indeks 1 = kode):
--   {'belum_siap'}
--   {'kunci_tidak_sah'} / {'jendela_tidak_sah'} / {<kode galat>}
--   {'kuota_habis', terpakai, slot_aktif, kedaluwarsa, 0, isi, isi_porsi}
--   {'konkurensi_penuh', …}
--   {'retry_budget_habis', …}
--   {'rate_limit', …, tunggu_mikro, …}   `tunggu_mikro` dari TAT yang menolak
--   {'slot_ganda', …}                    member sudah ada; pemanggil rusak
--   {'ok', terpakai, slot_aktif, kedaluwarsa, 0, isi, isi_porsi}
--
-- `kedaluwarsa` adalah jumlah slot yang dibersihkan langkah ini. Itu
-- satu-satunya tempat slot dihapus karena waktu (`ZREMRANGEBYSCORE`), dan
-- pemanggil menambahkannya ke `poolrouter_slot_kedaluwarsa_total` (§5a.1
-- butir 7). Slot yang pemiliknya mati karena itu tetap terhitung sampai
-- skornya lewat (D1.9), sehingga kegagalan tidak pernah membuka kuota.
--
-- `isi` dan `isi_porsi` adalah sisa token minimum di antara semua jendela:
-- `isi` untuk bucket global, `isi_porsi` untuk porsi sync. Async memakai
-- `isi` untuk memutuskan `async_boleh_jalan` (K-10); sync memakai
-- `isi_porsi`. Rumusnya `clamp(floor((now − TAT)/T) + burst, 0, burst)`, jadi
-- bucket penuh melaporkan `burst`, bukan 1 (keputusan Shu D2.5).

-- sisip: potongan/siap.lua sebagai siap
-- sisip: potongan/bersama.lua sebagai bersama

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

-- Langkah 3: bersihkan slot yang skornya sudah lewat, lalu hitung yang aktif.
local now = bersama.mikro()
local kedaluwarsa = redis.call('ZREMRANGEBYSCORE', KEYS[4], '-inf', now)
local slot_aktif = redis.call('ZCARD', KEYS[4])

-- Langkah 4: gerbang kuota. `slot_aktif` ikut dihitung, karena setiap slot
-- adalah percobaan yang akan terjadi walau belum di-INCR.
local terpakai = bersama.terpakai(KEYS[2], KEYS[3], a)
if terpakai + slot_aktif >= a.kuota_efektif then
  return { 'kuota_habis', terpakai, slot_aktif, kedaluwarsa, 0, 0, 0 }
end

-- Langkah 5: gerbang konkurensi upstream.
if slot_aktif >= maks then
  return { 'konkurensi_penuh', terpakai, slot_aktif, kedaluwarsa, 0, 0, 0 }
end

-- Langkah 6: anggaran retry, jendela bergulir 1 jam dari 60 bucket menit
-- (ADR-0001 §5a.3). Percobaan ulang (`nomor > 1`) hanya diberi token bila
-- `retry_jendela < max(retry_min, rasio_retry × percobaan_jendela)`.
-- `percobaan_jendela` menghitung SEMUA token yang diberikan dalam jendela
-- (percobaan pertama dan retry), `retry_jendela` hanya retry. Pembacaan murni;
-- penulisannya di langkah 8, bersama pemberian token.
local menit = math.floor(now / 60000000)
local MENIT_JENDELA_RETRY = 60
if nomor > 1 then
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
  local anggaran = math.max(retry_min, rasio_retry * percobaan_jendela)
  if retry_jendela >= anggaran then
    return { 'retry_budget_habis', terpakai, slot_aktif, kedaluwarsa, 0, 0, 0 }
  end
end

-- Langkah 7: semua jendela GCRA, tanpa menulis apa pun. Kalau ada yang
-- menolak, skrip berhenti sebagai penolakan murni: tidak ada TAT yang
-- tersimpan dan tidak ada slot yang dipesan. Waktu tunggu diambil yang
-- terbesar, karena semua jendela harus terlewati sebelum panggilan berikutnya
-- diizinkan.
if sync == '1' and porsi == 0 then
  return { 'porsi_sync_habis', terpakai, slot_aktif, kedaluwarsa, 0, 0, 0 }
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

for i = 1, n do
  local w = jendela[i]
  local _, sisa = periksa_jendela(w.kunci, w.burst, w.t)
  if sisa < isi then
    isi = sisa
  end
  if sync == '1' then
    local burst_porsi = math.floor(w.burst * porsi)
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
  return { 'rate_limit', terpakai, slot_aktif, kedaluwarsa, tunggu, isi, isi_porsi }
end

-- Langkah 8: tulis. TAT baru = TAT lama + T, dan TTL-nya `TAT_baru − now +
-- 60 s`, bukan `TAT − now`: dengan TTL lama, kunci bisa hilang sebelum TAT
-- terlewati sehingga satu menit pertama sesudahnya menerima burst penuh
-- (keputusan Shu D2.9).
for i = 1, n do
  local w = jendela[i]
  local pasangan = { { w.kunci, w.burst } }
  if sync == '1' then
    local burst_porsi = math.floor(w.burst * porsi)
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
local member = request_id .. ':' .. tostring(nomor)
local ditambah = redis.call('ZADD', KEYS[4], 'NX', now + ttl, member)
if ditambah == 0 then
  return { 'slot_ganda', terpakai, slot_aktif, kedaluwarsa, 0, isi, isi_porsi }
end

-- Anggaran retry: catat percobaan (`p<menit>`) dan, bila retry, `r<menit>`.
-- Bucket yang lebih tua dari jendela dipangkas di sini, satu-satunya tempat
-- hash ini ditulis, supaya hash tidak tumbuh tanpa batas. TTL 2 jam menjaga
-- hash dari pool yang berhenti dipakai.
redis.call('HINCRBY', KEYS[5], 'p' .. menit, 1)
if nomor > 1 then
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

return { 'ok', terpakai, slot_aktif + 1, kedaluwarsa, 0, isi, isi_porsi }
