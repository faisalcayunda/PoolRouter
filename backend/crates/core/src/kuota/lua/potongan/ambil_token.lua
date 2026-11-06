-- Potongan pemberian token: gerbang M-02 sesudah argumen sah
-- (ADR-0001 rev 15 §5a.1 butir 3, 5, 6, 7 dan §5a.3; ADR-0006 rev 1 §10).
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya pada baris penanda `-- sisip: potongan/ambil_token.lua sebagai
-- ambil_token`; hasilnya ada di `rakitan/`. Sandbox Redis tidak punya
-- `dofile`, jadi berkas ini tidak pernah dimuat sendiri.
--
-- Kenapa dipotong. Langkah kuota, konkurensi, anggaran retry dan GCRA adalah
-- bagian yang akan berubah saat F1 menambah prioritas dan peminjaman; skripnya
-- sendiri tidak. Memisahkannya membuat perubahan itu terlihat di satu tempat
-- dan membuat `kuota_ambil_token.lua` tinggal berisi urutan langkah (PA4).
--
-- Kontrak tidak berubah: potongan ini membaca KEYS dan ARGV yang sama dengan
-- skrip F0, mengembalikan tabel yang sama, dan tidak menambah kunci apa pun.
-- `kuota_ambil_token.lua` tetap yang menetapkan urutan KEYS; potongan ini
-- menerimanya sebagai argumen `k` dan `o`.
--
-- Yang di luar potongan: kesiapan Redis, validasi `request_id`, `nomor`,
-- `maks`, `ttl`, `rasio_retry`, `retry_min`, `porsi`, jumlah jendela dan
-- pembacaan `waktu beri` — semuanya tetap di skrip pemanggil, karena itu
-- urusan bentuk panggilan, bukan isi keputusan. Potongan ini dimulai sesudah
-- semuanya dinyatakan sah.
--
-- Pemakaian di skrip:
--
--   local status = ambil_token(KEYS, {
--     a = a, sync = sync, maks = maks, ttl = ttl,
--     rasio_retry = rasio_retry, retry_min = retry_min, porsi = porsi,
--     jendela = jendela, nomor = nomor, request_id = request_id,
--     sekarang = fungsi_mikro
--   })
--   if status.kode ~= 'ok' then return status.tabel end
--
-- Rancangan `o`:

-- `o.a`          hasil `bersama.periksa` (mode, kuota_efektif, jumlah_ember, no_ember)
-- `o.sync`       '0' | '1'
-- `o.maks`       batas slot aktif
-- `o.ttl`        masa hidup slot, mikrodetik
-- `o.rasio_retry`, `o.retry_min`  anggaran retry (ADR-0001 §5a.3)
-- `o.porsi`      pecahan kapasitas untuk sync, 0..1
-- `o.jendela`    daftar `{ t, burst, kunci, kunci_sync }`
-- `o.nomor`      nomor percobaan
-- `o.request_id` UUID percobaan
-- `o.sekarang`   fungsi pengembali mikrodetik (Redis TIME)
-- `o.pilih_sumber`     hook F1 (opsional): dipanggil sesudah semua gates F0,
--                      sebelum menulis TTL/slot; argumen `(KEYS, o, terpakai,
--                      slot_aktif)`; kembalikan nil atau `{ kode, tabel }`
-- `o.kunci_slot_sumber` hook F1 (opsional): dipanggil sesudah slot ditulis;
--                       argumen `(KEYS, o, member)`; kembalikan nil
--
-- Kembalian: `{ kode = ..., tabel = <bentuk kembalian skrip F0> }`. `kode`
-- adalah `tabel[1]`, disediakan supaya pemanggil bisa memeriksa tanpa
-- mengindeks tabel.

local function ambil_token(KEYS, o)
  -- Langkah 3: bersihkan slot yang skornya sudah lewat, lalu hitung yang aktif.
  local now = o.sekarang()
  local kedaluwarsa = redis.call('ZREMRANGEBYSCORE', KEYS[4], '-inf', now)
  local slot_aktif = redis.call('ZCARD', KEYS[4])

  -- Langkah 4: gerbang kuota. `slot_aktif` ikut dihitung, karena setiap slot
  -- adalah percobaan yang akan terjadi walau belum di-INCR.
  local terpakai = bersama.terpakai(KEYS[2], KEYS[3], o.a)
  if terpakai + slot_aktif >= o.a.kuota_efektif then
    return { kode = 'kuota_habis', tabel = { 'kuota_habis', terpakai, slot_aktif, kedaluwarsa, 0, 0, 0 } }
  end

  -- Langkah 5: gerbang konkurensi upstream.
  if slot_aktif >= o.maks then
    return { kode = 'konkurensi_penuh', tabel = { 'konkurensi_penuh', terpakai, slot_aktif, kedaluwarsa, 0, 0, 0 } }
  end

  -- Langkah 6: anggaran retry, jendela bergulir 1 jam dari 60 bucket menit
  -- (ADR-0001 §5a.3). Percobaan ulang (`nomor > 1`) hanya diberi token bila
  -- `retry_jendela < max(retry_min, rasio_retry × percobaan_jendela)`.
  -- `percobaan_jendela` menghitung SEMUA token yang diberikan dalam jendela
  -- (percobaan pertama dan retry), `retry_jendela` hanya retry. Pembacaan
  -- murni; penulisannya di langkah 8, bersama pemberian token.
  local menit = math.floor(now / 60000000)
  local MENIT_JENDELA_RETRY = 60
  if o.nomor > 1 then
    local batas_bawah = menit - (MENIT_JENDELA_RETRY - 1)
    local semua = redis.call('HGETALL', KEYS[5])
    local percobaan_jendela = 0
    local retry_jendela = 0
    for i = 1, #semua, 2 do
      local jenis, m = string.match(semua[i], '^([pr])(%d+)$')
      m = tonumber(m)
      if jenis ~= nil and m >= batas_bawah and m <= menit then
        local nilai = tonumber(semua[i + 1])
        if jenis == 'p' then
          percobaan_jendela = percobaan_jendela + nilai
        else
          retry_jendela = retry_jendela + nilai
        end
      end
    end
    local anggaran = math.max(o.retry_min, o.rasio_retry * percobaan_jendela)
    if retry_jendela >= anggaran then
      return { kode = 'retry_budget_habis', tabel = { 'retry_budget_habis', terpakai, slot_aktif, kedaluwarsa, 0, 0, 0 } }
    end
  end

  -- Langkah 7: semua jendela GCRA, tanpa menulis apa pun. Kalau ada yang
  -- menolak, skrip berhenti sebagai penolakan murni: tidak ada TAT yang
  -- tersimpan dan tidak ada slot yang dipesan. Waktu tunggu diambil yang
  -- terbesar, karena semua jendela harus terlewati sebelum panggilan
  -- berikutnya diizinkan.
  if o.sync == '1' and o.porsi == 0 then
    return { kode = 'porsi_sync_habis', tabel = { 'porsi_sync_habis', terpakai, slot_aktif, kedaluwarsa, 0, 0, 0 } }
  end

  local tunggu = 0
  local isi = math.huge
  local isi_porsi = math.huge

  local function periksa_jendela(kunci, burst, t)
    local tat = tonumber(redis.call('GET', kunci) or '0')
    if tat < now then
      tat = now
    end
    local geser = (burst - 1) * t
    if tat - geser > now then
      local sisa = (tat - geser) - now
      if sisa > tunggu then
        tunggu = sisa
      end
    end
    local sisa_token = math.floor((now - tat) / t) + burst
    if sisa_token < 0 then
      sisa_token = 0
    elseif sisa_token > burst then
      sisa_token = burst
    end
    return tat, sisa_token
  end

  for i = 1, #o.jendela do
    local w = o.jendela[i]
    local _, sisa = periksa_jendela(w.kunci, w.burst, w.t)
    if sisa < isi then
      isi = sisa
    end
    if o.sync == '1' then
      local burst_porsi = math.floor(w.burst * o.porsi)
      if burst_porsi < 1 then
        burst_porsi = 1
      end
      local _, sisa_porsi = periksa_jendela(w.kunci_sync, burst_porsi, w.t)
      if sisa_porsi < isi_porsi then
        isi_porsi = sisa_porsi
      end
    end
  end

  if isi == math.huge then
    isi = 0
  end
  if isi_porsi == math.huge then
    isi_porsi = 0
  end

  if tunggu > 0 then
    return { kode = 'rate_limit', tabel = { 'rate_limit', terpakai, slot_aktif, kedaluwarsa, tunggu, isi, isi_porsi } }
  end

  -- Tahap anggaran F1 (hook, AC-5/AC-6): sesudah SEMUA gates F0 dan sebelum
  -- tulis. Hook menolak dengan `{ kode, tabel }` seperti rejection global;
  -- saat itu belum ada kunci TTL atau slot yang ditulis (hanya pembersihan
  -- slot kedaluwarsa dari langkah 3). `opsi` nil = F0 apa adanya.
  if o.pilih_sumber then
    local tolak = o.pilih_sumber(KEYS, o, terpakai, slot_aktif)
    if tolak then
      return tolak
    end
  end

  -- Langkah 8: tulis. TAT baru = TAT lama + T, dan TTL-nya `TAT_baru − now +
  -- 60 s`, bukan `TAT − now`: dengan TTL lama, kunci bisa hilang sebelum TAT
  -- terlewati sehingga satu menit pertama sesudahnya menerima burst penuh
  -- (keputusan Shu D2.9).
  for i = 1, #o.jendela do
    local w = o.jendela[i]
    local pasangan = { { w.kunci, w.burst } }
    if o.sync == '1' then
      local burst_porsi = math.floor(w.burst * o.porsi)
      if burst_porsi < 1 then
        burst_porsi = 1
      end
      pasangan[#pasangan + 1] = { w.kunci_sync, burst_porsi }
    end
    for j = 1, #pasangan do
      local kunci = pasangan[j][1]
      local tat = tonumber(redis.call('GET', kunci) or '0')
      if tat < now then
        tat = now
      end
      local baru = tat + w.t
      redis.call('SET', kunci, tostring(baru), 'PX', math.floor((baru - now) / 1000) + 60000)
    end
  end

  -- Slot: member `<request_id>:<nomor_percobaan>`, skor `waktu_beri + ttl_slot`.
  -- `NX` membuat pemberian ulang untuk percobaan yang sama gagal keras
  -- (`slot_ganda`) alih-alih memperpanjang diam-diam: itu selalu bug pemanggil.
  local member = o.request_id .. ':' .. tostring(o.nomor)
  local ditambah = redis.call('ZADD', KEYS[4], 'NX', now + o.ttl, member)
  if ditambah == 0 then
    return { kode = 'slot_ganda', tabel = { 'slot_ganda', terpakai, slot_aktif, kedaluwarsa, 0, isi, isi_porsi } }
  end

  -- Sumber slot (hook T2c): dicatat sesudah slot benar-benar milik pemanggil,
  -- supaya `slot_klaim.lua`/`slot_lepas.lua` bisa mengembalikan bebannya.
  if o.kunci_slot_sumber then
    o.kunci_slot_sumber(KEYS, o, member)
  end

  -- Anggaran retry: catat percobaan (`p<menit>`) dan, bila retry, `r<menit>`.
  -- Bucket yang lebih tua dari jendela dipangkas di sini, satu-satunya tempat
  -- hash ini ditulis, supaya hash tidak tumbuh tanpa batas. TTL 2 jam menjaga
  -- hash dari pool yang berhenti dipakai.
  redis.call('HINCRBY', KEYS[5], 'p' .. menit, 1)
  if o.nomor > 1 then
    redis.call('HINCRBY', KEYS[5], 'r' .. menit, 1)
  end
  local batas_pangkas = menit - (MENIT_JENDELA_RETRY - 1)
  local lama = redis.call('HKEYS', KEYS[5])
  for i = 1, #lama do
    local m = tonumber(string.match(lama[i], '^[pr](%d+)$'))
    if m == nil or m < batas_pangkas then
      redis.call('HDEL', KEYS[5], lama[i])
    end
  end
  redis.call('PEXPIRE', KEYS[5], 7200000)

  return { kode = 'ok', tabel = { 'ok', terpakai, slot_aktif + 1, kedaluwarsa, 0, isi, isi_porsi } }
end

return {
  ambil_token = ambil_token
}
