-- Gerbang M-02 versi F1: tahap global F0 lebih dulu, lalu tahap anggaran
-- (prioritas, reserved/shareable, peminjaman) lewat hook sesudah semua gates
-- F0 dan sebelum tulis. ADR-0006 rev 1 §10; spec f1-prioritas-alokasi AC-5,
-- AC-6.
--
-- Tahap globalnya BUKAN salinan: `potongan/kerangka.lua` + `potongan/
-- ambil_token.lua` dipakai bersama F0, jadi setiap penolakan global
-- (`belum_siap`, `kuota_habis`, `konkurensi_penuh`, `retry_budget_habis`,
-- `rate_limit`) tidak bisa menyimpang dari F0 tanpa mengubah F0 juga. Uji
-- pasangan di `uji_lua.py` menjaga itu.
--
-- `opsi` adalah satu-satunya pembeda: hook `pilih_sumber` dan
-- `kunci_slot_sumber`. KEYS dan ARGV sama persis dengan `kuota_ambil_token.lua`
-- (tabel lengkapnya di kepala berkas itu); didefinisikan sekali di
-- `potongan/kerangka.lua` supaya kedua skrip tidak bisa berbeda. Kontrak hook
-- ada di kepala `potongan/ambil_token.lua`.
--
-- KEYS tambahan sesudah kunci F0 (lihat `potongan/alokasi_anggaran.lua`):
--   +1 `pr:anggaran:{<pool>}`      hash `<sumber>:<ember>` → cacah commit
--   +2 `pr:slot_sumber:{<pool>}`   hash member → sumber
--
-- ARGV tambahan sesudah `14 + 2n` milik F0:
--   m                       jumlah sumber pada `urutan_sumber`
--   pasangan `sumber, batas` (m pasangan), urut prioritas
--   ember_detik             detik per ember (600)

-- sisip: potongan/siap.lua sebagai siap
-- sisip: potongan/bersama.lua sebagai bersama
-- sisip: potongan/ambil_token.lua sebagai ambil_token
-- sisip: potongan/kerangka.lua sebagai kerangka
-- sisip: potongan/alokasi_anggaran.lua sebagai alokasi_anggaran

local run_id = ARGV[1]
if not siap(KEYS[1], run_id) then
  return { 'belum_siap' }
end

local n = bersama.bulat(ARGV[14])
if n == nil or n < 1 or #KEYS ~= 5 + 2 * n + 2 then
  return { 'kunci_tidak_sah' }
end

-- `ekor` = ARGV pertama milik tahap anggaran, sesudah pasangan jendela.
local ekor = 14 + 2 * n
local jumlah_sumber = bersama.bulat(ARGV[ekor + 1])
if jumlah_sumber == nil or jumlah_sumber < 1 then
  return { 'anggaran_tidak_sah' }
end
local anggota = {}
for i = 1, jumlah_sumber do
  local sumber = ARGV[ekor + 2 * i]
  local batas = bersama.bulat(ARGV[ekor + 2 * i + 1])
  if sumber == nil or sumber == '' or batas == nil or batas < 0 then
    return { 'anggaran_tidak_sah' }
  end
  anggota[i] = { sumber = sumber, batas = batas }
end
local ember_detik = bersama.bulat(ARGV[ekor + 2 * jumlah_sumber + 2])
if ember_detik == nil or ember_detik < 1 then
  return { 'anggaran_tidak_sah' }
end

local am = alokasi_anggaran.buat(
  anggota,
  KEYS[6 + 2 * n],
  KEYS[7 + 2 * n],
  ember_detik
)
return kerangka.jalankan(KEYS, ARGV, {
  pilih_sumber = am.pilih_sumber,
  kunci_slot_sumber = am.kunci_slot_sumber,
  jumlah_kunci_tambahan = 2
})
