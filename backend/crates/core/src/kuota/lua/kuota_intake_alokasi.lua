-- Gerbang intake beralokasi: cek intake F0 lebih dulu, lalu syarat `antre_d`
-- rute ini (ADR-0006 rev 1 §10; spec `f1-prioritas-alokasi` AC-8).
--
-- Skrip ini hanya dipakai untuk upstream pool yang punya rute beralokasi.
-- Upstream pool tanpa alokasi tetap memakai `kuota_intake.lua`; pemilihan
-- skripnya dilakukan sekali per panggilan di lapisan `KuotaStore`.
--
-- KEYS (D1.7, urutannya tetap):
--   1 pr:siap                          gerbang kesiapan
--   2 pr:attempts:{<pool>}:<periode>   counter percobaan periode ini
--   3 pr:ember:{<pool>}                hash ember 10 menit (mode bergulir)
--   4 pr:antre:{<pool>}                jumlah permintaan async yang diterima,
--                                      belum dipanggil
--   5 pr:antre:dp:{<pool>}:<d>         jumlah permintaan async rute `d` yang
--                                      diterima, belum dipanggil
--
-- ARGV: 1–7 sama dengan `kuota_intake.lua`, ditambah
--   8 ruang_intake        `rencana::ruang_intake(d)` sebagai teks, `>= 0`
--
-- Kembalian: sama dengan `kuota_intake.lua`, ditambah
--   {'kuota_habis', terpakai, antre}    juga saat `antre_d >= ruang_intake`
--   {'ruang_intake_tidak_sah'}          ARGV[8] bukan bilangan `>= 0`
--
-- Kedua counter antre dinaikkan di EVAL yang sama, jadi penolakan tidak
-- meninggalkan antrean yang naik (AC-8).

-- sisip: potongan/siap.lua sebagai siap
-- sisip: potongan/bersama.lua sebagai bersama
-- sisip: potongan/intake.lua sebagai intake
-- sisip: potongan/intake_alokasi.lua sebagai intake_alokasi

local run_id = ARGV[1]
if not siap(KEYS[1], run_id) then
  return { 'belum_siap' }
end
if not bersama.kunci_sah(KEYS[1], KEYS[2], KEYS[3], KEYS[4], KEYS[5]) then
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

return intake_alokasi.gerbang_intake_alokasi(KEYS, {
  sync = sync,
  a = a,
  ruang_intake = ARGV[8],
})
