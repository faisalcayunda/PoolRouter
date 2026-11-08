-- Potongan tahap anggaran F1 (AC-6): pilih sumber dana dan catat sumber slot.
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya pada baris penanda `-- sisip: potongan/alokasi_anggaran.lua sebagai
-- alokasi_anggaran`; hasilnya ada di `rakitan/`.
--
-- Dipakai sebagai `o.pilih_sumber` / `o.kunci_slot_sumber` pada
-- `potongan/ambil_token.lua` (lihat kontrak hook di `potongan/kerangka.lua`).
--
-- Kunci (ditetapkan skrip pemanggil, sesudah kunci F0 dan jendela rate):
--   kunci_anggaran      `pr:anggaran:{<pool>}`, field `<sumber>:<ember>` →
--                       cacah percobaan yang sudah commit
--   kunci_slot_sumber_hash   `pr:slot_sumber:{<pool>}`, field member → sumber
--
-- `u_x = Σ field <x>:<ember> di jendela + slot aktif bersumber x`. Slot aktif
-- dibaca dari `KEYS[4]` (`pr:slot`), yang sudah dibersihkan TTL di langkah 3
-- `ambil_token`; entri `slot_sumber` tanpa slot hidup adalah sisa slot
-- kedaluwarsa dan dihapus di sini supaya bebannya tidak abadi (AC-7).

local function buat(anggota, kunci_anggaran, kunci_slot_sumber_hash, ember_detik)
  local terpilih = nil

  local function beban(KEYS, o, sumber)
    local jumlah = 0
    local semua = redis.call('HGETALL', kunci_anggaran)
    for i = 1, #semua, 2 do
      local s, n = string.match(semua[i], '^(.-):(%d+)$')
      if s == sumber then
        n = tonumber(n)
        if bersama.di_jendela(o.a, n) then
          jumlah = jumlah + tonumber(semua[i + 1])
        end
      end
    end
    local slot = redis.call('HGETALL', kunci_slot_sumber_hash)
    for i = 1, #slot, 2 do
      if slot[i + 1] == sumber then
        if redis.call('ZSCORE', KEYS[4], slot[i]) then
          jumlah = jumlah + 1
        else
          redis.call('HDEL', kunci_slot_sumber_hash, slot[i])
        end
      end
    end
    return jumlah
  end

  local function pilih_sumber(KEYS, o, terpakai, slot_aktif)
    for i = 1, #anggota do
      local bb = beban(KEYS, o, anggota[i].sumber)
      if bb < anggota[i].batas then
        terpilih = anggota[i].sumber
        return nil
      end
    end

    -- Hanya sumber pada `urutan_sumber` tiket ini yang boleh menentukan
    -- retry_after: hash anggaran dipakai bersama semua downstream pool di
    -- upstream pool yang sama, jadi ember tua milik pool lain tidak boleh
    -- memengaruhi pemulihan permintaan ini (AC-6).
    local sumber_set = {}
    for i = 1, #anggota do
      sumber_set[anggota[i].sumber] = true
    end
    local tertua = nil
    local semua = redis.call('HGETALL', kunci_anggaran)
    for i = 1, #semua, 2 do
      local s, n = string.match(semua[i], '^(.-):(%d+)$')
      if s ~= nil and sumber_set[s] then
        n = tonumber(n)
        if n ~= nil and bersama.di_jendela(o.a, n) and (tertua == nil or n < tertua) then
          tertua = n
        end
      end
    end
    local tunggu = 0
    if tertua ~= nil then
      tunggu = math.max(
        0,
        (tertua + o.a.jumlah_ember) * ember_detik * 1000000 - o.sekarang()
      )
    end
    return {
      kode = 'anggaran_habis',
      tabel = { 'anggaran_habis', terpakai, slot_aktif, 0, tunggu, 0, 0 }
    }
  end

  local function kunci_slot_sumber(KEYS, o, member)
    if terpilih ~= nil then
      redis.call('HSET', kunci_slot_sumber_hash, tostring(member), tostring(terpilih))
    end
  end

  return { pilih_sumber = pilih_sumber, kunci_slot_sumber = kunci_slot_sumber }
end

return { buat = buat }
