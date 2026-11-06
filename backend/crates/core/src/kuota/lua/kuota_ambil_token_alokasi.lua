-- Gerbang M-02 versi F1: tahap global F0 lebih dulu, lalu tahap anggaran
-- (prioritas, reserved/shareable, peminjaman) lewat hook sesudah semua gates
-- F0 dan sebelum tulis. ADR-0006 rev 1 §10; spec f1-prioritas-alokasi AC-5.
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
-- ada di kepala `potongan/ambil_token.lua`. Pada tiket T2a (ARC-169) hook
-- belum dipasang.
--
-- sisip: potongan/siap.lua sebagai siap
-- sisip: potongan/bersama.lua sebagai bersama
-- sisip: potongan/ambil_token.lua sebagai ambil_token
-- sisip: potongan/kerangka.lua sebagai kerangka

-- sisip: potongan/alokasi_anggaran.lua sebagai alokasi_anggaran
local opsi = {
  pilih_sumber = alokasi_anggaran.pilih_sumber,
  kunci_slot_sumber = alokasi_anggaran.kunci_slot_sumber,
}
return kerangka.jalankan(KEYS, ARGV, opsi)
