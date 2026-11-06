-- Potongan tahap anggaran (AC-6): pilih sumber pertama dari urutan_sumber
-- yang masih punya ruang (u_x < a_x), catat sumber di pr:slot_sumber, dan
-- tolak anggaran_habis dengan retry_after dari ember tertua bila tidak ada.
--
-- BERKAS POTONGAN, bukan skrip. Disisipkan via `-- sisip:` ke
-- kuota_ambil_token_alokasi.lua. Sandbox Redis tidak punya dofile/require.
--
-- Hook yang diekspos:
--   pilih_sumber(KEYS, o, terpakai, slot_aktif) -> nil atau { kode, tabel }
--   kunci_slot_sumber(KEYS, o, member) -> nil
--
-- Konfigurasi anggaran dibawa lewat `o`:
--   o.urutan_sumber = daftar sumber (teks) berurutan dari anggaran sendiri,
--                     sisa (bila boleh_pinjam), lalu shareable lain.
--   o.anggaran[x]   = a_x (batas anggaran sumber x).
--   o.kunci_anggaran = prefix kunci Redis `pr:anggaran:{pool}` (hash ember).
--   o.kunci_slot_sumber_hash = kunci `pr:slot_sumber:{pool}`.
--   o.ember_detik = durasi ember (detik), untuk retry_after.
-- Bila konfigurasi tidak ada (upstream pool tanpa alokasi), hook tidak
-- menolak (perilaku F0 apa adanya).

local function u_sumber(KEYS, o, sumber)
  -- u_x = beban anggaran x dalam jendela + slot aktif bersumber x.
  local hash = o.kunci_anggaran
  local beban = 0
  if hash then
    local semua = redis.call('HGETALL', hash)
    for i = 1, #semua, 2 do
      local s, ember = string.match(semua[i], '^(.+):(%d+)$')
      if s == sumber then
        beban = beban + (tonumber(semua[i + 1]) or 0)
      end
    end
  end
  -- slot aktif bersumber x
  if o.kunci_slot_sumber_hash then
    local slot = redis.call('HGETALL', o.kunci_slot_sumber_hash)
    for i = 1, #slot, 2 do
      if slot[i + 1] == sumber then
        beban = beban + 1
      end
    end
  end
  return beban
end

local function pilih_sumber(KEYS, o, terpakai, slot_aktif)
  if o.urutan_sumber == nil or o.anggaran == nil then
    return nil
  end
  for i = 1, #o.urutan_sumber do
    local s = o.urutan_sumber[i]
    local a = o.anggaran[s]
    if a ~= nil and u_sumber(KEYS, o, s) < a then
      -- ada ruang: pilih sumber ini; beban dicatat saat slot ditulis.
      o._sumber_terpilih = s
      return nil
    end
  end
  -- tidak ada sumber dengan ruang: anggaran_habis + retry_after
  local retry = 0
  -- ember tertua yang paling cepat pulih
  if o.kunci_anggaran then
    local semua = redis.call('HGETALL', o.kunci_anggaran)
    local terlama = nil
    for i = 1, #semua, 2 do
      local ember = tonumber(string.match(semua[i], ':(%d+)$'))
      if ember ~= nil then
        if terlama == nil or ember < terlama then
          terlama = ember
        end
      end
    end
    if terlama ~= nil then
      retry = (terlama + (o.ember_detik or 600)) * 1000000
    end
  end
  -- batas global bila lebih lama
  local now = o.sekarang()
  if retry < now then
    retry = now
  end
  return { kode = 'anggaran_habis', tabel = { 'anggaran_habis', terpakai, slot_aktif, 0, retry, 0, 0 } }
end

local function kunci_slot_sumber(KEYS, o, member)
  if o._sumber_terpilih == nil then
    return nil
  end
  if o.kunci_slot_sumber_hash then
    redis.call('HSET', o.kunci_slot_sumber_hash, member, o._sumber_terpilih)
  end
  o._sumber_terpilih = nil
  return nil
end

return {
  pilih_sumber = pilih_sumber,
  kunci_slot_sumber = kunci_slot_sumber,
}
