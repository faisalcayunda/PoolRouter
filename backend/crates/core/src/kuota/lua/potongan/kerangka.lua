-- Potongan kerangka: bentuk panggilan gerbang M-02 yang dipakai bersama
-- `kuota_ambil_token.lua` (F0) dan `kuota_ambil_token_alokasi.lua` (F1).
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya pada baris penanda `-- sisip: ... sebagai kerangka`; hasilnya ada di
-- `rakitan/`. Sandbox Redis tidak punya `dofile`, jadi berkas ini tidak pernah
-- dimuat sendiri.
--
-- Kenapa dipotong. Validasi ARGV dan urutan gates F0 hanya boleh punya satu
-- salinan; skrip alokasi F1 membedakan diri lewat `opsi`, bukan lewat salinan
-- validasi. `opsi` berisi hook opsional untuk T2b/T2c:
--   * `pilih_sumber(KEYS, o, terpakai, slot_aktif)` → nil atau
--     `{ kode = '...', tabel = { ... } }`, dipanggil di
--     `potongan/ambil_token.lua` sesudah semua gates F0 dan sebelum tulis;
--   * `kunci_slot_sumber(KEYS, o, member)` → mencatat sumber slot yang baru
--     ditulis;
--   * `jumlah_kunci_tambahan` → kunci di luar bentuk F0 yang sudah dideklarasikan
--     pemanggil (0 bila tidak ada).
-- `opsi` nil berarti perilaku F0 apa adanya (AC-5).

local function kerangka(KEYS, ARGV, opsi)
  local run_id = ARGV[1]
  if not siap(KEYS[1], run_id) then
    return { 'belum_siap' }
  end

  local request_id = ARGV[2]
  if request_id == nil or request_id == '' or string.find(request_id, ':', 1, true) then
    return { 'request_id_tidak_sah' }
  end

  local nomor = bersama.bulat(ARGV[3])
  if nomor == nil or nomor < 1 then
    return { 'nomor_percobaan_tidak_sah' }
  end

  local a = {
    mode = ARGV[4],
    kuota_efektif = ARGV[6],
    jumlah_ember = ARGV[7],
    no_ember = ARGV[8]
  }
  local galat = bersama.periksa(a)
  if galat then
    return { galat }
  end

  local sync = ARGV[5]
  if sync ~= '0' and sync ~= '1' then
    return { 'sync_tidak_sah' }
  end

  local maks = bersama.bulat(ARGV[9])
  if maks == nil or maks < 1 then
    return { 'maks_konkurensi_tidak_sah' }
  end

  local ttl = bersama.bulat(ARGV[10])
  if ttl == nil or ttl < 1 then
    return { 'ttl_slot_tidak_sah' }
  end

  -- Anggaran retry (ADR-0001 §5a.3): `rasio_retry` 0..1 dan `retry_min` bulat >= 0.
  -- Tidak ada nilai "tanpa anggaran": `0` dan `0` berarti tidak ada retry sama
  -- sekali, karena syaratnya `retry_jendela < max(retry_min, rasio × percobaan)`.
  local rasio_retry = tonumber(ARGV[11])
  if rasio_retry == nil or rasio_retry ~= rasio_retry or rasio_retry < 0 or rasio_retry > 1 then
    return { 'rasio_retry_tidak_sah' }
  end
  local retry_min = bersama.bulat(ARGV[12])
  if retry_min == nil or retry_min < 0 then
    return { 'retry_min_tidak_sah' }
  end

  local porsi = tonumber(ARGV[13])
  if porsi == nil or porsi < 0 or porsi > 1 then
    return { 'porsi_sync_tidak_sah' }
  end

  local n = bersama.bulat(ARGV[14])
  if n == nil or n < 1 then
    return { 'jumlah_jendela_tidak_sah' }
  end
  -- Skrip F0 tidak punya kunci tambahan; skrip alokasi menambahkan kunci
  -- tahap anggaran dan menyebut jumlahnya di `opsi` (AC-6).
  local tambahan = 0
  if opsi ~= nil and opsi.jumlah_kunci_tambahan ~= nil then
    tambahan = opsi.jumlah_kunci_tambahan
  end
  if #KEYS ~= 5 + 2 * n + tambahan then
    return { 'kunci_tidak_sah' }
  end
  if not bersama.kunci_sah(KEYS[1], KEYS[2], KEYS[3], KEYS[4], KEYS[5]) then
    return { 'kunci_tidak_sah' }
  end

  local jendela = {}
  for i = 1, n do
    local t = bersama.bulat(ARGV[14 + 2 * i - 1])
    local burst = bersama.bulat(ARGV[14 + 2 * i])
    if t == nil or t < 1 or burst == nil or burst < 1 then
      return { 'jendela_tidak_sah' }
    end
    local kunci = KEYS[5 + 2 * i - 1]
    local kunci_sync = KEYS[5 + 2 * i]
    if not bersama.kunci_sah(kunci, kunci_sync) then
      return { 'kunci_tidak_sah' }
    end
    jendela[i] = { t = t, burst = burst, kunci = kunci, kunci_sync = kunci_sync }
  end

  local o = {
    a = a,
    sync = sync,
    maks = maks,
    ttl = ttl,
    rasio_retry = rasio_retry,
    retry_min = retry_min,
    porsi = porsi,
    jendela = jendela,
    nomor = nomor,
    request_id = request_id,
    sekarang = bersama.mikro
  }
  if opsi ~= nil then
    o.pilih_sumber = opsi.pilih_sumber
    o.kunci_slot_sumber = opsi.kunci_slot_sumber
    o.ambil_sumber = opsi.ambil_sumber
  end

  local hasil = ambil_token.ambil_token(KEYS, o)
  return hasil.tabel
end

return { jalankan = kerangka }
