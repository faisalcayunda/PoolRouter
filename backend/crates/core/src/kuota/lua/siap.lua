-- Gerbang kesiapan Redis (ADR-0001 rev 15 §5a.2 butir 2).
--
-- Setiap skrip kuota memanggil ini lebih dulu. Kunci `pr:siap` ditulis
-- controller sebagai langkah terakhir rebuild, berisi `run_id` Redis yang
-- sedang hidup. Pemanggil mengirim `ARGV[...]` = run_id yang dibacanya dari
-- `INFO server` saat (re)connect. Kalau nilai berbeda, Redis sudah restart
-- atau replika sudah dipromosikan, sehingga counter di memori belum tentu
-- mencerminkan PostgreSQL. Pemanggil memperlakukan `belum_siap` sebagai
-- "Redis tidak tersedia" (fail-closed, §5), bukan sebagai "kuota tersedia".
--
-- Karena skrip lain di direktori ini memanggil berkas ini dengan
-- `dofile`, berkas ini hanya boleh berisi fungsi, tanpa efek samping.

local KUNCI_SIAP = 'pr:siap'

-- Mengembalikan `true` bila Redis siap untuk run_id ini, `false` bila belum.
local function siap(run_id)
  if run_id == nil or run_id == '' then
    return false
  end
  local nilai = redis.call('GET', KUNCI_SIAP)
  if not nilai then
    return false
  end
  return nilai == run_id
end

return {
  KUNCI_SIAP = KUNCI_SIAP,
  siap = siap
}
