-- Pembacaan kuota tanpa mutasi, untuk `Retry-After` dan kuota tracker admin.
-- ADR-0001 rev 15 §5a.1 butir 1d, §5a.7.
--
-- Skrip ini MURNI BACA: tidak ada menulis, tidak ada `TIME`. Waktu untuk
-- memutuskan sisa jendela bergulir datang dari `ARGV[3] = now_mikro`, dihitung
-- Rust dari jam yang sama dengan `requested_at` (§5a.1 butir 1d). Kalau skrip
-- ini membaca jam sendiri, `Retry-After` dan hitungan admter bisa memakai
-- dasar waktu berbeda dari yang dipakai menulis ember.
--
-- Kapan dipanggil (bukan saat intake): setelah pemberian token, untuk mengisi
-- header `Retry-After` dan metrik. Karena itu ia tidak boleh mengubah apa pun.

local siap_mod = dofile('siap.lua')

local POLA_FIELD_EMBER = '^b(%d+)$'
local MODE_GULIR = 'bergulir'
local MODE_KALENDER = 'kalender'
local JEDA_EMBER_MIKRO = 600 * 1000000
local JENDELA_MIKRO = 24 * 3600 * 1000000

-- KEYS[1] = kunci attempts periode
-- KEYS[2] = hash ember jendela
-- ARGV[1] = run_id
-- ARGV[2] = mode jendela saat ini: 'bergulir' | 'kalender'
-- ARGV[3] = now_mikro
-- ARGV[4] = jumlah_ember
-- ARGV[5] = kuota_efektif
-- ARGV[6] = ember_mulai
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

local jumlah_ember = math.floor(tonumber(ARGV[4]) or -1)
local kuota = math.floor(tonumber(ARGV[5]) or -1)
local ember_mulai = math.floor(tonumber(ARGV[6]) or -1)
if jumlah_ember < 1 or kuota <= 0 or ember_mulai < 0 then
  return { 'argumen_tidak_sah' }
end

local attempts = tonumber(redis.call('GET', KEYS[1]) or '0') or 0

-- Jendela bergulir 24 jam atas ember 10 menit. Ember k mencakup
-- [k*10 menit, (k+1)*10 menit). Sebuah percobaan tetap dihitung selama
-- `now - 24 jam < batas_atas embernya`, yaitu untuk ember k dengan
-- (k+1)*10 menit > now - 24 jam. Ini lebih konservatif daripada pembacaan
-- `attempts_tambah.lua`/`kuota_intake.lua` (yang memakai seluruh 145 ember
-- `b{ember_mulai-144}..b{ember_mulai}`); selisihnya hanya ember tertua yang
-- sudah lewat 24 jam, dan pada `terpakai >= kuota_efektif` itu tidak bisa
-- mengubah penolakan.
local awal_jendela = now - JENDELA_MIKRO
local batas_bawah_ember = math.floor(awal_jendela / JEDA_EMBER_MIKRO)
local terpakai = attempts
local ember_tertua = nil
if mode == MODE_GULIR then
  terpakai = 0
  local semua = redis.call('HGETALL', KEYS[2])
  for i = 1, #semua, 2 do
    local n = tonumber(string.match(semua[i], POLA_FIELD_EMBER))
    if n ~= nil and n >= batas_bawah_ember then
      terpakai = terpakai + tonumber(semua[i + 1])
      if ember_tertua == nil or n < ember_tertua then
        ember_tertua = n
      end
    end
  end
end

-- `Retry-After` jendela bergulir = saat percobaan tertua yang masih dihitung
-- keluar dari jendela: awal embernya + 24 jam - now. Tanpa isi ember, tidak
-- ada yang perlu ditunggu.
local lepas = 0
if ember_tertua ~= nil then
  lepas = ember_tertua * JEDA_EMBER_MIKRO + JENDELA_MIKRO - now
  if lepas < 0 then
    lepas = 0
  end
end

return { 'ok', terpakai, attempts, kuota, math.floor(lepas / 1000000) }
