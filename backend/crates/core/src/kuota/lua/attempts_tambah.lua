-- Naikkan counter percobaan sesudah baris log percobaan commit (§5, §5a.1
-- butir 7). Dipanggil sekali per percobaan, setelah pemanggilan upstream
-- selesai mengembalikan jawaban yang dicatat.
--
-- Slot diverifikasi lebih dulu: percobaan tanpa slot sah tidak boleh
-- menaikkan counter apa pun. Itu menutup celah "proses macet lalu hidup lagi
-- sesudah TTL": percobaannya sudah terjadi tetapi tidak boleh muncul sebagai
-- hitungan kuota yang tidak pernah dipesan (D1.9), dan `attempts` pun tidak
-- berubah sehingga invarian `panggilan == attempts` tetap utuh.
--
-- KEYS (D1.7):
--   1 pr:siap                          gerbang kesiapan
--   2 pr:attempts:{<pool>}:<periode>   counter percobaan periode ini
--   3 pr:ember:{<pool>}                hash ember 10 menit
--   4 pr:slot:{<pool>}                 semaphore konkurensi (verifikasi saja)
--
-- ARGV:
--   1 run_id               dicocokkan dengan KEYS[1]
--   2 member               `<request_id>:<nomor_percobaan>`
--   3 mode                 `bergulir` | `kalender`
--   4 kuota_efektif        untuk penanda `ok_terlampaui`
--   5 jumlah_ember         `JUMLAH_EMBER` (145)
--   6 no_ember             `floor(requested_at_unix / 600)`, BUKAN jam Redis
--
-- Kembalian:
--   {'belum_siap'} / {'kunci_tidak_sah'} / {<kode galat>}
--   {'slot_kedaluwarsa', attempts}     tanpa INCR, tanpa perubahan ember
--   {'ok', attempts, terpakai, kuota}              `terpakai <= kuota`
--   {'ok_terlampaui', attempts, terpakai, kuota}   alarm: kuota terlampaui
--
-- `ok_terlampaui` adalah jaring pengaman, bukan jalur normal: kalau gerbang
-- M-02 benar, `terpakai` tidak pernah melewati kuota. Pemanggil menaikkan
-- metrik alarm dan tetap mencatat percobaannya, karena percobaannya memang
-- sudah terjadi; yang salah bukan pencatatannya.
--
-- Field ember memakai nomor ember dari `requested_at` (bukan `TIME`), dan
-- field yang lebih tua dari jendela dipangkas di sini — satu-satunya tempat
-- pemangkasan (D2.9). Tanpa pemangkasan, hash tumbuh tanpa batas.
--
-- Field `terpakai` tidak lagi ditulis ke hash (D2.9): nilainya turunan dari
-- field ember, dan menyimpannya membuat dua sumber kebenaran yang bisa
-- berbeda setelah failover.

-- sisip: potongan/siap.lua sebagai siap
-- sisip: potongan/bersama.lua sebagai bersama

local run_id = ARGV[1]
if not siap(KEYS[1], run_id) then
  return { 'belum_siap' }
end
if not bersama.kunci_sah(KEYS[1], KEYS[2], KEYS[3], KEYS[4]) then
  return { 'kunci_tidak_sah' }
end

local member = ARGV[2]
if member == nil or member == '' then
  return { 'member_tidak_sah' }
end

local a = {
  mode = ARGV[3],
  kuota_efektif = ARGV[4],
  jumlah_ember = ARGV[5],
  no_ember = ARGV[6]
}
local galat = bersama.periksa(a)
if galat then
  return { galat }
end

local now = bersama.mikro()
local skor = redis.call('ZSCORE', KEYS[4], member)
if not skor or tonumber(skor) <= now then
  local tetap = tonumber(redis.call('GET', KEYS[2]) or '0')
  return { 'slot_kedaluwarsa', tetap }
end

local attempts = redis.call('INCR', KEYS[2])
redis.call('HINCRBY', KEYS[3], 'b' .. a.no_ember, 1)

local batas = bersama.batas_jendela(a)
local field = redis.call('HKEYS', KEYS[3])
for i = 1, #field do
  local n = tonumber(string.match(field[i], bersama.POLA_FIELD_EMBER))
  if n ~= nil and n < batas then
    redis.call('HDEL', KEYS[3], field[i])
  end
end

local terpakai = bersama.terpakai(KEYS[2], KEYS[3], a)
if terpakai > a.kuota_efektif then
  return { 'ok_terlampaui', attempts, terpakai, a.kuota_efektif }
end

return { 'ok', attempts, terpakai, a.kuota_efektif }
