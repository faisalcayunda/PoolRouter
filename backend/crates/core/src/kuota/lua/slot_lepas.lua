-- Lepas slot: sesudah baris log commit (pemenang CAS), saat commit
-- `dispatched` gagal (dispatcher), atau sesudah reconciler commit. D1.5.
--
-- Ini satu-satunya skrip yang menghapus member slot di luar pembersihan
-- berbasis waktu. Ketiga pemanggil itu sudah memastikan tidak ada percobaan
-- yang masih memakai slot tersebut; pihak lain yang memanggil skrip ini akan
-- melepas kapasitas untuk percobaan yang belum tentu terjadi, jadi jangan
-- dipakai untuk "membersihkan" slot yang tampak yatim (D1.9).
--
-- KEYS (D1.7):
--   1 pr:siap               gerbang kesiapan
--   2 pr:slot:{<pool>}      semaphore konkurensi
--
-- ARGV:
--   1 run_id               dicocokkan dengan KEYS[1]
--   2 member               `<request_id>:<nomor_percobaan>`
--   3 asal                 `kas` | `dispatcher` | `reconciler`, untuk audit log
--                          pemanggil; tidak memengaruhi keputusan
--
-- Kembalian:
--   {'belum_siap'} / {'kunci_tidak_sah'} / {'member_tidak_sah'}
--   {'ok', dihapus}        `dihapus` 0 berarti slot sudah tidak ada

-- sisip: potongan/siap.lua sebagai siap
-- sisip: potongan/bersama.lua sebagai bersama

local run_id = ARGV[1]
if not siap(KEYS[1], run_id) then
  return { 'belum_siap' }
end
if not bersama.kunci_sah(KEYS[1], KEYS[2]) then
  return { 'kunci_tidak_sah' }
end

local member = ARGV[2]
if member == nil or member == '' then
  return { 'member_tidak_sah' }
end

local dihapus = redis.call('ZREM', KEYS[2], member)
return { 'ok', dihapus }
