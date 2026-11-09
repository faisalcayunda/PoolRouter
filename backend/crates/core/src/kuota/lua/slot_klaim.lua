-- Worker: perpanjang slot miliknya dan keluarkan satu dari antrean.
-- ADR-0001 rev 15 §5a.1 butir 5 dan 6, D1.5.
--
-- Slot hanya boleh dilepas tiga pemilik: pemenang CAS `in_flight` sesudah log
-- lengkap, dispatcher bila commit `dispatched` gagal, dan reconciler sesudah
-- transaksinya commit. Worker yang mengklaim slot memperpanjangnya, bukan
-- melepasnya. Karena itu skrip ini tidak menghapus member: slot yang sudah
-- lewat skornya dijawab `slot_kedaluwarsa` dan baru dibersihkan
-- `kuota_ambil_token.lua` (D1.9).
--
-- KEYS (D1.7):
--   1 pr:siap               gerbang kesiapan
--   2 pr:slot:{<pool>}      semaphore konkurensi
--   3 pr:antre:{<pool>}     jumlah permintaan async yang belum dipanggil
--   4 pr:slot_sumber:{<pool>} slot sumber anggaran (AC-7)
--
-- ARGV:
--   1 run_id               dicocokkan dengan KEYS[1]
--   2 member               `<request_id>:<nomor_percobaan>` (D1.2)
--   3 ttl_slot_mikro       `timeout_total_upstream + 60 s`
--   4 dari_antre           `1` kurangi antrean, `0` jangan (rekonsiliasi)
--
-- Kembalian:
--   {'belum_siap'} / {'kunci_tidak_sah'} / {<kode galat>}
--   {'slot_tidak_ada'}                 member tidak pernah dipesan
--   {'slot_kedaluwarsa'}               skor sudah lewat; tanpa perpanjangan
--   {'ok', skor_lama, antre, sumber}   slot diperpanjang ke `now + ttl`, sumber dari pr:slot_sumber atau nil

-- sisip: potongan/siap.lua sebagai siap
-- sisip: potongan/bersama.lua sebagai bersama

local run_id = ARGV[1]
if not siap(KEYS[1], run_id) then
  return { 'belum_siap' }
end
if not bersama.kunci_sah(KEYS[1], KEYS[2], KEYS[3], KEYS[4]) then
  return { 'kunci_tidak_sah' }
end

local member = ARGV[2]
if member == nil or member == '' then
  return { 'member_tidak_sah' }
end

local ttl = bersama.bulat(ARGV[3])
if ttl == nil or ttl < 1 then
  return { 'ttl_slot_tidak_sah' }
end

local dari_antre = ARGV[4]
if dari_antre ~= '0' and dari_antre ~= '1' then
  return { 'dari_antre_tidak_sah' }
end

local now = bersama.mikro()
local skor = redis.call('ZSCORE', KEYS[2], member)
if not skor then
  return { 'slot_tidak_ada' }
end
if tonumber(skor) <= now then
  return { 'slot_kedaluwarsa' }
end

redis.call('ZADD', KEYS[2], 'XX', now + ttl, member)

local antre = tonumber(redis.call('GET', KEYS[3]) or '0')
if dari_antre == '1' and antre > 0 then
  antre = redis.call('DECR', KEYS[3])
end

local sumber = redis.call('HGET', KEYS[4], member)
return { 'ok', skor, antre, sumber or '' }
