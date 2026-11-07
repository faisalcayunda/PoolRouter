-- Potongan gerbang intake: keputusan penerimaan sesudah argumen sah
-- (ADR-0001 rev 15 §5a.1 butir 2, K-12; ADR-0006 rev 1 §10).
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya pada baris penanda `-- sisip: potongan/intake.lua sebagai intake`;
-- hasilnya ada di `rakitan/`, dan itu bentuk yang dimuat Redis.
--
-- Kenapa dipotong. Intake adalah tempat F1 menambahkan `ruang_intake()` dan
-- peminjaman (PA4, AC-3); kerangka pembacaan ARGV-nya tidak berubah. Memisah
-- keputusan dari bentuk panggilan membuat perubahan itu satu berkas, dan
-- membuat kontrak F0 tetap terlihat utuh di `kuota_intake.lua`.
--
-- Pemakaian di skrip:
--
--   return gerbang_intake(KEYS, {
--     sync = sync, a = a, kunci_antre = KEYS[4]
--   })
--
-- Kembaliannya adalah tabel kembalian skrip F0 apa adanya.

-- Pemakaian `o`:
-- `o.a`          hasil `bersama.periksa` (mode, kuota_efektif, jumlah_ember, no_ember)
-- `o.sync`       '0' | '1'
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

local function gerbang_intake(KEYS, o)
  local terpakai = bersama.terpakai(KEYS[2], KEYS[3], o.a)
  local antre = tonumber(redis.call('GET', KEYS[4]) or '0')
  if antre < 0 then
    antre = 0
  end

  if terpakai >= o.a.kuota_efektif then
    return { 'kuota_habis', terpakai, antre }
  end
  if o.sync == '1' and terpakai + antre >= o.a.kuota_efektif then
    return { 'kuota_habis', terpakai, antre }
  end
  if terpakai + antre >= o.a.kuota_efektif then
    return { 'antrean_penuh', terpakai, antre }
  end

  if o.sync == '0' then
    antre = redis.call('INCR', KEYS[4])
  end

  return { 'lolos', terpakai, antre }
end

return {
  gerbang_intake = gerbang_intake
}
