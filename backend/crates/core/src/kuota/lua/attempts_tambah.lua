-- Tambah percobaan: satu-satunya jalur yang menaikkan `terpakai`.
-- ADR-0001 rev 15 §5a.1 butir 1a, 1c, 1d dan §5.
--
-- Dipanggil worker (async) dan webserver (sync) SETELAH baris log percobaan
-- ter-commit dan SEBELUM panggilan upstream (§5, DQ-05). Setiap percobaan
-- menambah `attempts:{pool}:{periode_mulai}` DAN ember jendela 10 menit dalam
-- mode apa pun, termasuk percobaan yang kemudian gagal TLS/koneksi (butir 1c,
-- tanpa opsi mematikan di F0). Itu yang membuat DQ-05 = 0 per periode.
--
-- `terpakai` untuk cek kuota (§5a.1 butir 1a):
--   * `kalender`      : `attempts:{pool}:{periode_mulai}` saja.
--   * `bergulir_24j`  : jumlah ember yang masih di jendela + ember berjalan.
-- Periode transisi `kalender` (butir 1d) ditegakkan bergulir, jadi pemanggil
-- mengirim mode 'bergulir' untuk periode itu.
--
-- Cakupan ember > 24 jam (145 ember), jadi hitungan selalu berlebih, tidak
-- pernah kurang (kapasitas jenuh ±99,3% `kuota_efektif` per 24 jam).

local siap_mod = dofile('siap.lua')
local bersama = dofile('bersama.lua')

local MODE_GULIR = 'bergulir'
local MODE_KALENDER = 'kalender'
local POLA_FIELD_EMBER = '^b(%d+)$'

-- ARGV[1] = run_id
-- ARGV[2] = mode jendela saat ini: 'bergulir' | 'kalender'
-- ARGV[3.. ] = argumen bersama `bersama.lua`, mulai dari kunci_attempts
if not siap_mod.siap(ARGV[1]) then
  return { 'belum_siap' }
end

local mode = ARGV[2]
if mode ~= MODE_GULIR and mode ~= MODE_KALENDER then
  return { 'mode_tidak_sah' }
end

local a = bersama.baca(ARGV, 3)
local galat = bersama.periksa(a)
if galat then
  return { galat }
end

-- 1) Counter periode. Selalu dinaikkan, kedua mode.
local attempts = redis.call('INCR', a.kunci_attempts)

-- 2) Ember jendela 10 menit.
redis.call('HINCRBY', a.hash_ember, 'b' .. a.ember_mulai, 1)

-- Field lama dibuang supaya hash tidak tumbuh tanpa batas. Yang dipertahankan
-- `jumlah_ember` ember ke belakang ditambah ember berjalan, sama dengan yang
-- dijumlahkan butir 3. Field `terpakai` tidak cocok pola dan dipertahankan.
local batas = a.ember_mulai - a.jumlah_ember
local semua = redis.call('HGETALL', a.hash_ember)
local buang = {}
for i = 1, #semua, 2 do
  local n = tonumber(string.match(semua[i], POLA_FIELD_EMBER))
  if n ~= nil and n < batas then
    buang[#buang + 1] = semua[i]
  end
end
if #buang > 0 then
  -- Redis menjalankan Lua 5.1: `table.unpack` belum ada di sana, `unpack` global
  -- yang ada. Semua nilai adalah nama field hasil HGETALL, jadi aman sebagai
  -- argumen variadik.
  redis.call('HDEL', a.hash_ember, unpack(buang))
end

-- 3) Hitung `terpakai`.
local terpakai = attempts
if mode == MODE_GULIR then
  terpakai = 0
  semua = redis.call('HGETALL', a.hash_ember)
  for i = 1, #semua, 2 do
    local n = tonumber(string.match(semua[i], POLA_FIELD_EMBER))
    if n ~= nil and n >= batas and n <= a.ember_mulai then
      terpakai = terpakai + tonumber(semua[i + 1])
    end
  end
end

redis.call('HSET', a.hash_ember, 'terpakai', terpakai)

return { 'ok', attempts, terpakai, a.kuota_efektif }
