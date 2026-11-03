-- Gerbang intake: cek kuota + antrean dalam skrip yang sama.
-- ADR-0001 rev 15 §5a.1 butir 1a, 1b, dan §5.
--
-- Web UI memanggil ini sebelum menerima permintaan sync. Tiga keputusan
-- dibuat atomik terhadap `attempts_tambah.lua`, sehingga dua permintaan yang
-- bersamaan tidak bisa sama-sama melihat sisa kuota dan sama-sama lolos
-- (itulah invarian M-02 = 0 pelanggaran kuota; lihat uji properti §9).
--
-- Ketiga penolakan di bawah DILAPORKAN, bukan diputuskan, di sini: skrip hanya
-- membandingkan angka. Pemetaan ke problem code tetap satu tempat di Rust.
--
-- Fail-closed: `belum_siap` dan argumen tidak sah tidak pernah menjadi "lolos".

local siap_mod = dofile('siap.lua')
local bersama = dofile('bersama.lua')

local POLA_FIELD_EMBER = '^b(%d+)$'
local MODE_GULIR = 'bergulir'
local MODE_KALENDER = 'kalender'

-- ARGV[1] = client_id           (hanya untuk logging di pemanggil; tidak dipakai di sini)
-- ARGV[2] = run_id
-- ARGV[3] = mode jendela saat ini: 'bergulir' | 'kalender'
-- ARGV[4] = kuota_sync: '1' bila permintaan ini memakai porsi sync, '0' bila async
-- ARGV[5.. ] = argumen bersama `bersama.lua`, mulai dari kunci_attempts
if not siap_mod.siap(ARGV[2]) then
  return { 'belum_siap' }
end

local mode = ARGV[3]
if mode ~= MODE_GULIR and mode ~= MODE_KALENDER then
  return { 'mode_tidak_sah' }
end

local sync = ARGV[4]
if sync ~= '0' and sync ~= '1' then
  return { 'sync_tidak_sah' }
end
sync = sync == '1'

local a = bersama.baca(ARGV, 5)
local galat = bersama.periksa(a)
if galat then
  return { galat }
end

-- 1) `terpakai` (§5a.1 butir 1a, KP-07).
local terpakai
if mode == MODE_KALENDER then
  terpakai = tonumber(redis.call('GET', a.kunci_attempts) or '0')
else
  terpakai = 0
  local semua = redis.call('HGETALL', a.hash_ember)
  local batas = a.ember_mulai - a.jumlah_ember
  for i = 1, #semua, 2 do
    local n = tonumber(string.match(semua[i], POLA_FIELD_EMBER))
    if n ~= nil and n >= batas and n <= a.ember_mulai then
      terpakai = terpakai + tonumber(semua[i + 1])
    end
  end
end

-- 2) Kuota habis: `terpakai >= kuota_efektif`, kedua mode (§5a.1 butir 1b).
--    `terpakai` tidak boleh SAMA DENGAN, bukan hanya melebihi.
if terpakai >= a.kuota_efektif then
  return { 'kuota_habis', terpakai, a.kuota_efektif }
end

-- 3) Kapasitas antre (mode async saja; sync tidak melewati antrean, §4.1).
if not sync then
  local antre = tonumber(redis.call('GET', a.kunci_antre) or '0')
  if antre >= a.kuota_efektif then
    return { 'antrean_penuh', antre, a.kuota_efektif }
  end
end

return { 'lolos', terpakai, a.kuota_efektif }
