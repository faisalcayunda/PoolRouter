-- Baca sisa kuota untuk laporan dan `Retry-After`. MURNI BACA: tidak ada
-- INCR, HDEL, ZREM, atau ZREMRANGEBYSCORE. §5a.4, D2.10.
--
-- Menggantikan `kuota_retry_after.lua`. Namanya mengikuti apa yang
-- dikembalikan: kuota yang terbaca, bukan hanya header Retry-After, karena
-- pemanggil memakai angka yang sama untuk panel kuota dan untuk 429.
--
-- KEYS (D1.7):
--   1 pr:siap                          gerbang kesiapan
--   2 pr:attempts:{<pool>}:<periode>   counter percobaan periode ini
--   3 pr:ember:{<pool>}                hash ember 10 menit
--   4 pr:slot:{<pool>}                 semaphore konkurensi
--
-- ARGV:
--   1 run_id               dicocokkan dengan KEYS[1]
--   2 mode                 `bergulir` | `kalender`
--   3 kuota_efektif
--   4 jumlah_ember         `JUMLAH_EMBER` (145)
--   5 no_ember             `floor(requested_at_unix / 600)`
--
-- Kembalian:
--   {'belum_siap'} / {'kunci_tidak_sah'} / {<kode galat>}
--   {'ok', terpakai, attempts, slot_aktif, retry_after_detik}
--
-- `slot_aktif` hanya menghitung slot yang belum lewat skornya. Slot yang
-- kedaluwarsa tidak dihapus di sini: pembersihannya milik
-- `kuota_ambil_token.lua`, satu-satunya tempat, supaya metrik
-- `poolrouter_slot_kedaluwarsa_total` tidak menghitung dua kali (D1.9).
--
-- `retry_after_detik` hanya diisi untuk mode `bergulir`; pada mode `kalender`
-- pemanggil menghitungnya sampai `periode_selesai` (D2.10), karena skrip tidak
-- menerima zona waktu. Pada `bergulir`:
--   * ember tertua yang masih berisi: `(b_tertua + jumlah_ember) × 600 − now`,
--     yaitu saat ember itu keluar dari jendela;
--   * tanpa ember berisi: 1 detik;
--   * batas atas 87000 detik (24 jam 10 menit) supaya header tetap sah.

-- sisip: potongan/siap.lua sebagai siap
-- sisip: potongan/bersama.lua sebagai bersama

local run_id = ARGV[1]
if not siap(KEYS[1], run_id) then
  return { 'belum_siap' }
end
if not bersama.kunci_sah(KEYS[1], KEYS[2], KEYS[3], KEYS[4]) then
  return { 'kunci_tidak_sah' }
end

local a = {
  mode = ARGV[2],
  kuota_efektif = ARGV[3],
  jumlah_ember = ARGV[4],
  no_ember = ARGV[5]
}
local galat = bersama.periksa(a)
if galat then
  return { galat }
end

local now = bersama.mikro()
local terpakai = bersama.terpakai(KEYS[2], KEYS[3], a)
local attempts = tonumber(redis.call('GET', KEYS[2]) or '0')
local slot_aktif = redis.call('ZCOUNT', KEYS[4], '(' .. tostring(now), '+inf')

local retry = 0
if a.mode == 'bergulir' and terpakai >= a.kuota_efektif then
  local tertua = nil
  local semua = redis.call('HGETALL', KEYS[3])
  for i = 1, #semua, 2 do
    local n = tonumber(string.match(semua[i], bersama.POLA_FIELD_EMBER))
    if n ~= nil and bersama.di_jendela(a, n) and tonumber(semua[i + 1]) > 0 then
      if tertua == nil or n < tertua then
        tertua = n
      end
    end
  end
  if tertua == nil then
    retry = 1
  else
    local detik = math.floor(now / 1000000)
    retry = (tertua + a.jumlah_ember) * 600 - detik
    if retry > 87000 then
      retry = 87000
    elseif retry < 1 then
      retry = 1
    end
  end
end

return { 'ok', terpakai, attempts, slot_aktif, retry }
