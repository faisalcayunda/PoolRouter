-- Gerbang intake: kontrol penerimaan + `INCR antre` dalam satu EVAL.
-- ADR-0001 rev 15 §5a.1 butir 2 (K-12) dan §5.
--
-- Ini BUKAN gerbang M-02. Intake hanya memutuskan apakah sebuah permintaan
-- diterima; permintaan yang diterima belum tentu dipanggil, dan retry masuk
-- lagi ke antrean. Bukti M-02 ada di `kuota_ambil_token.lua`, yang memeriksa
-- `terpakai + slot_aktif` lalu memesan slot dalam satu EVAL (keputusan Shu
-- SHAA-195 butir 1, opsi C). `uji_lua.py` dulu memakai intake sebagai gerbang
-- M-02, dan itu yang melaporkan 41–42 percobaan pada kuota 37.
--
-- KEYS (D1.7, urutannya tetap):
--   1 pr:siap                          gerbang kesiapan
--   2 pr:attempts:{<pool>}:<periode>   counter percobaan periode ini
--   3 pr:ember:{<pool>}                hash ember 10 menit (mode bergulir)
--   4 pr:antre:{<pool>}                jumlah permintaan async yang diterima,
--                                      belum dipanggil
--
-- ARGV:
--   1 run_id              dari `INFO server`, dicocokkan dengan KEYS[1]
--   2 mode                `bergulir` | `kalender`
--   3 sync                `1` permintaan sync, `0` async. Sync tidak masuk
--                         antrean, jadi ia juga tidak menaikkan counter.
--   4 client_id           hanya untuk telemetri, tidak memengaruhi keputusan
--   5 kuota_efektif       `floor(kuota_harian × (1 − margin))`
--   6 jumlah_ember        `JUMLAH_EMBER` (145)
--   7 no_ember            `floor(requested_at_unix / 600)`, dihitung di Rust
--
-- Kembalian (indeks 1 = kode):
--   {'belum_siap'}                          Redis belum siap untuk run_id
--   {'kunci_tidak_sah'} / {<kode galat>}    argumen tidak sah
--   {'kuota_habis', terpakai, antre}        `terpakai >= kuota_efektif`
--   {'antrean_penuh', terpakai, antre}      `terpakai + antre >= kuota_efektif`
--   {'lolos', terpakai, antre}              diterima; async sudah di-INCR
--
-- Cek antrean memakai `terpakai + antre`, bukan `antre` saja: yang perlu
-- dijaga adalah jumlah percobaan yang akan terjadi, bukan panjang antrean.
-- Dengan begitu `INCR antre` tidak bisa membuat antrean melampaui
-- `kuota_efektif − terpakai` (D1.6).
--
-- Permintaan SYNC tidak diperiksa terhadap antrean: ia tidak masuk antrean
-- sama sekali, jadi panjang antrean tidak mengatakan apa pun tentang
-- kapasitas yang tersisa untuknya. Yang membatasinya kelak adalah slot di
-- `kuota_ambil_token.lua`, yang menghitung `terpakai + slot_aktif`.
--
-- Kedua penolakan tetap dijawab 429 oleh pemanggil, tetapi kodenya dibedakan
-- supaya metrik dan jalur retry bisa membedakan "kuota habis" dari "antrean
-- penuh" (§5a.1 butir 2).

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
  kuota_efektif = ARGV[5],
  jumlah_ember = ARGV[6],
  no_ember = ARGV[7]
}
local galat = bersama.periksa(a)
if galat then
  return { galat }
end

local sync = ARGV[3]
if sync ~= '0' and sync ~= '1' then
  return { 'sync_tidak_sah' }
end

local terpakai = bersama.terpakai(KEYS[2], KEYS[3], a)
local antre = tonumber(redis.call('GET', KEYS[4]) or '0')
if antre < 0 then
  antre = 0
end

if terpakai >= a.kuota_efektif then
  return { 'kuota_habis', terpakai, antre }
end
if sync == '1' and terpakai + antre >= a.kuota_efektif then
  return { 'kuota_habis', terpakai, antre }
end
if terpakai + antre >= a.kuota_efektif then
  return { 'antrean_penuh', terpakai, antre }
end

if sync == '0' then
  antre = redis.call('INCR', KEYS[4])
end

return { 'lolos', terpakai, antre }
