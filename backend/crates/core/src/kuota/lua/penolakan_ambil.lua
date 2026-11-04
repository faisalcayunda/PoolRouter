-- Limiter penolakan per client: ambil satu token GCRA untuk satu penolakan.
-- ADR-0001 rev 15 §5 (butir limiter penolakan), ADR-0002 rev 5 D6, contract
-- v0.7.4 §3.2.2a L-01…L-08.
--
-- Bucket ini hanya untuk jalur penolakan submit. Request yang diterima tidak
-- memakainya, dan kuota upstream pool tidak tersentuh (L-02). Kuncinya per
-- client dan terpisah dari bucket upstream pool, jadi satu client yang rusak
-- tidak memakan kapasitas client lain maupun kapasitas pool.
--
-- Semantik GCRA sama dengan `kuota_ambil_token.lua` (keputusan Shu D2): virtual
-- scheduling, `T` = jarak antar token, `tau = (burst - 1) * T`, kunci yang tidak
-- ada berarti bucket penuh. Burst 20 dan T = 1 detik adalah default L-03/L-04;
-- keduanya dikirim pemanggil karena platform admin bisa mengubahnya per client.
--
-- KEYS:
--   1 pr:siap                  gerbang kesiapan
--   2 pr:tolak:{<client_id>}   TAT GCRA client ini
--
-- ARGV:
--   1 run_id    dari `INFO server`, dicocokkan dengan KEYS[1]
--   2 t_mikro   jarak antar token dalam mikrodetik (default 1000000)
--   3 burst     kapasitas bucket (default 20)
--
-- Kembalian (indeks 1 = kode):
--   {'belum_siap'}                       Redis belum siap; pemanggil 503 (L-07)
--   {'kunci_tidak_sah'} / {'t_tidak_sah'} / {'burst_tidak_sah'}
--   {'lolos', sisa_token}                token diambil; respons `rejected` biasa
--   {'dibatasi', tunggu_mikro}           token habis; pemanggil 429
--                                        `penolakan_berlebih` + Retry-After
--
-- `dibatasi` tidak menulis apa pun: penolakan yang dibatasi tidak memperpanjang
-- hukuman (tidak ada TAT yang bergeser), sehingga client yang berhenti
-- mengirim pulih tepat satu token per `T`.

-- sisip: potongan/siap.lua sebagai siap
-- sisip: potongan/bersama.lua sebagai bersama

local run_id = ARGV[1]
if not siap(KEYS[1], run_id) then
  return { 'belum_siap' }
end
if not bersama.kunci_sah(KEYS[1], KEYS[2]) then
  return { 'kunci_tidak_sah' }
end

local t = bersama.bulat(ARGV[2])
if t == nil or t < 1 then
  return { 't_tidak_sah' }
end
local burst = bersama.bulat(ARGV[3])
if burst == nil or burst < 1 then
  return { 'burst_tidak_sah' }
end

local now = bersama.mikro()
local tat = tonumber(redis.call('GET', KEYS[2]) or '0')
if tat < now then
  tat = now
end

local geser = (burst - 1) * t
if tat - geser > now then
  return { 'dibatasi', (tat - geser) - now }
end

local baru = tat + t
-- TTL = sisa waktu sampai TAT lewat + 60 detik, supaya kunci tidak hilang
-- sebelum bucket penuh lagi (D2.4).
redis.call('SET', KEYS[2], tostring(baru), 'PX', math.floor((baru - now) / 1000) + 60000)

local sisa = math.floor((now - baru) / t) + burst
if sisa < 0 then
  sisa = 0
elseif sisa > burst then
  sisa = burst
end
return { 'lolos', sisa }
