-- Potongan gerbang intake beralokasi: syarat kedua sesudah cek F0
-- (ADR-0006 rev 1 §10; spec `f1-prioritas-alokasi` AC-8).
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya pada baris penanda `-- sisip: potongan/intake_alokasi.lua sebagai
-- intake_alokasi`; hasilnya ada di `rakitan/`, dan itu bentuk yang dimuat
-- Redis.
--
-- Kenapa dipotong. Keputusannya tetap satu: `intake.gerbang_intake` memutuskan
-- `kuota_habis` dan `antrean_penuh` F0, lalu `intake_alokasi` menambah satu
-- syarat lewat `o.saring`, yang dipanggil gerbang F0 SESUDAH semua ceknya dan
-- SEBELUM `INCR antre`. Karena kaitnya di dalam satu EVAL, penolakan tidak
-- meninggalkan counter antre yang naik.
--
-- `ruang_intake(d)` dihitung di Rust (`core::kuota::rencana::ruang_intake`);
-- skrip ini menerimanya sebagai ARGV[8]. Intake adalah estimasi penerimaan,
-- sama seperti di F0; penegakan atomiknya di dispatch. Karena sync tidak masuk
-- antrean, ia juga tidak diperiksa terhadap `antre_d` (alasan yang sama dengan
-- pengecualian sync di `potongan/intake.lua`).
--
-- Pemakaian `o`:
-- `o.a`            hasil `bersama.periksa`
-- `o.sync`         '0' | '1'
-- `o.ruang_intake` ruang rute ini (`>= 0`) sebagai teks
-- `KEYS[5]`        `pr:antre:dp:{<pool>}:<downstream_pool_id>`

-- Kembaliannya adalah tabel kembalian skrip apa adanya; penolakan memakai kode
-- `kuota_habis` (429) yang sudah ada, bukan kode baru.
local function gerbang_intake_alokasi(KEYS, o)
  local ruang = tonumber(o.ruang_intake)
  if ruang == nil or ruang < 0 then
    return { 'ruang_intake_tidak_sah' }
  end

  return intake.gerbang_intake(KEYS, {
    a = o.a,
    sync = o.sync,
    saring = function(terpakai, antre)
      if o.sync == '1' then
        return nil
      end
      local antre_d = tonumber(redis.call('GET', KEYS[5]) or '0')
      if antre_d == nil or antre_d < 0 then
        antre_d = 0
      end
      if antre_d >= ruang then
        return { 'kuota_habis', terpakai, antre }
      end
      redis.call('INCR', KEYS[5])
      return nil
    end,
  })
end

return {
  gerbang_intake_alokasi = gerbang_intake_alokasi
}
