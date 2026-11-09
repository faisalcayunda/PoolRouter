-- Ubah counter antrean pool: koreksi `antre` di luar intake dan klaim slot
-- (ARC-72, spec f0-controller-worker AC-25, keputusan CW3).
--
-- Dipanggil controller SESUDAH commit PostgreSQL:
--   * delta -1: request keluar dari `queued`/`dispatched` tanpa lewat
--     `slot_klaim.lua` dengan `dari_antre = 1` (pembatalan client, `expired`,
--     K-20 lewat gerbang atau penyapu, `failed` saat menunggu retry);
--   * delta +1: reconciler mengembalikan request `in_flight` ke `queued`
--     (worker mati), karena klaim slot sebelumnya sudah mengurangi `antre`.
--
-- Counter tidak pernah turun di bawah 0. Bila Redis belum siap, skrip menjawab
-- `belum_siap` dan tidak mengubah apa pun: rebuild berikutnya menghitung ulang
-- `antre` dari PostgreSQL, jadi koreksi yang terlewat tidak menumpuk.
--
-- KEYS (D1.7):
--   1 pr:siap               gerbang kesiapan
--   2 pr:antre:{<pool>}     jumlah request async yang belum diklaim worker
--
-- ARGV:
--   1 run_id               dicocokkan dengan KEYS[1]
--   2 delta                bilangan bulat `-1` atau `1`
--
-- Kembalian:
--   {'belum_siap'} / {'kunci_tidak_sah'} / {'delta_tidak_sah'}
--   {'ok', antre}          nilai counter sesudah perubahan

-- sisip: potongan/siap.lua sebagai siap
-- sisip: potongan/bersama.lua sebagai bersama

local run_id = ARGV[1]
if not siap(KEYS[1], run_id) then
  return { 'belum_siap' }
end
if not bersama.kunci_sah(KEYS[1], KEYS[2]) then
  return { 'kunci_tidak_sah' }
end

local delta = ARGV[2]
if delta ~= '-1' and delta ~= '1' then
  return { 'delta_tidak_sah' }
end

local antre = tonumber(redis.call('GET', KEYS[2]) or '0') or 0
if delta == '1' then
  antre = redis.call('INCR', KEYS[2])
elseif antre > 0 then
  antre = redis.call('DECR', KEYS[2])
else
  antre = 0
end
return { 'ok', antre }
