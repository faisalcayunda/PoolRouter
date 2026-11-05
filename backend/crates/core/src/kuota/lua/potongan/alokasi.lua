-- Potongan anggaran alokasi: reserved vs shareable, dihitung dari ARGV.
--
-- BERKAS POTONGAN, bukan skrip. `scripts/rakit-lua.sh` menyisipkannya apa
-- adanya pada baris penanda `-- sisip: potongan/alokasi.lua sebagai alokasi`;
-- hasilnya ada di `rakitan/`. Sandbox Redis tidak punya `dofile`, jadi berkas
-- ini tidak pernah dimuat sendiri (review 778f9fd, temuan blocking 2).
--
-- Padanan Lua dari `core::kuota::alokasi` (spec `f1-prioritas-alokasi`,
-- AL-01..AL-08). Modelnya:
--
--   kuota     = Q = floor(kuota_harian × (1 − margin)), sudah dihitung Rust
--   reserved  = Σ alokasi aktif      exact: nilai
--                                    persen: floor(Q × nilai / 100)
--   shareable = kuota − reserved
--
-- Penjumlahan terjadi DI SINI, bukan di Rust: alokasi yang tidak muat harus
-- ditolak sebagai konfigurasi tidak sah (fail-closed, ADR-0001 rev 15 §5),
-- bukan dipotong per rute. Karena itu potongan ini mengembalikan kode galat
-- `alokasi_tidak_sah` / `alokasi_melebihi_anggaran`, bukan menyesuaikan nilai.
--
-- Angka di sini hanya dipakai bila skripnya masih memakai Q saja (F0). Selama
-- T2–T7 belum mendarat, jalur token dan dispatcher tetap memeriksa `terpakai`
-- terhadap Q; potongan ini sengaja belum disisipkan ke skrip mana pun supaya
-- perilaku F0 tidak berubah sebelum anggaran benar-benar ditegakkan. Saat T2
-- mulai menegakkannya, `uraikan` dipanggil dengan daftar `ARGV` skrip itu.
--
-- Karena disisipkan, isi berkas ini hanya boleh berisi fungsi dan konstanta,
-- tanpa efek samping dan tanpa bergantung pada ARGV global.

local AL_MAKS_ALOKASI = 64
local AL_JENIS_EXACT = 'exact'
local AL_JENIS_PERSEN = 'persen'
local AL_SKALA_PERSEN = 10000 -- persen disimpan dalam perseratus persen (2 desimal)

-- `nilai` (angka atau string) dengan 2 desimal ke perseratus; nil bila bentuknya
-- tidak sah. `tonumber('')` adalah nil, jadi string kosong tertolak di sini.
local function al_bulat_skala(nilai)
  local n = tonumber(nilai)
  if n == nil or n ~= n or n == math.huge or n == -math.huge then
    return nil
  end
  return math.floor(n * 100 + 0.5)
end

-- Mengurai `n` pasangan { jenis, nilai } dari `argv` mulai indeks `awal`.
-- Mengembalikan daftar `{ jenis = ..., nilai = ..., periksa = ... }` atau
-- `nil, kode` bila bentuknya tidak sah. `periksa` adalah bentuk asli nilai
-- sebagai bilangan berskala 100 (exact: percobaan, persen: perseratus persen),
-- supaya pasangan (jenis, nilai) hanya diurai satu kali dan penjumlahannya
-- tidak bergantung pada pembulatan berulang.
--
-- `argv` di sini adalah daftar ARGV milik skrip pemanggil: elemennya string,
-- dan `awal` menunjuk elemen pertama pasangan. Batas 64 alokasi per pool cukup
-- untuk jumlah rute yang dilayani satu pool; lebih dari itu ditolak, tidak
-- dipotong.
local function al_urai(argv, awal, n)
  -- `n` adalah jumlah pasangan, bukan nilai berskala: cukup diubah ke bilangan
  -- bulat. Memakai `al_bulat_skala` di sini akan mengalikannya 100 lalu
  -- membaginya lagi, dan pembagian itu yang membuat 3 pasangan terbaca 0.
  local jumlah = tonumber(n)
  if jumlah == nil or jumlah < 0 or jumlah > AL_MAKS_ALOKASI or jumlah % 1 ~= 0 then
    return nil, 'alokasi_tidak_sah'
  end
  jumlah = math.floor(jumlah)
  local daftar = {}
  for i = 1, jumlah do
    -- ARGV adalah daftar datar: dua elemen per pasangan, bukan tabel bersarang.
    local jenis = argv[awal + 2 * i - 2]
    local nilai = argv[awal + 2 * i - 1]
    local v = al_bulat_skala(nilai)
    if jenis == AL_JENIS_EXACT then
      if v == nil or v < 100 or v % 100 ~= 0 then
        return nil, 'alokasi_tidak_sah'
      end
      daftar[i] = { jenis = AL_JENIS_EXACT, nilai = math.floor(v / 100), periksa = v }
    elseif jenis == AL_JENIS_PERSEN then
      -- `v` berskala 100 (1250 = 12,50%), jadi batas atasnya 100% = 10.000.
      if v == nil or v <= 0 or v > 100 * 100 then
        return nil, 'alokasi_tidak_sah'
      end
      daftar[i] = { jenis = AL_JENIS_PERSEN, nilai = v, periksa = v }
    else
      return nil, 'alokasi_tidak_sah'
    end
  end
  return daftar
end

-- Bagian yang dipesan satu alokasi, dalam percobaan. `persen` dibulatkan ke
-- Bawah, sama seperti `NilaiAlokasi::reserved` di Rust: jumlah bagian tidak
-- pernah lebih besar dari Q.
--
-- Nama parameternya BUKAN `kuota`: nama seperti itu mudah tertukar dengan
-- `kuota_harian` di skrip pemanggil, dan salah satunya membulatkan ke bawah
-- sehingga tidak pernah kelihatan sampai angkanya dibandingkan dengan Rust
-- (uji `tests/alokasi_lua.rs` menangkapnya).
local function al_reserved(alokasi, q)
  if alokasi.jenis == AL_JENIS_EXACT then
    return alokasi.nilai
  end
  return math.floor(q * alokasi.nilai / AL_SKALA_PERSEN)
end

-- Anggaran satu pool: `reserved`, `shareable`, dan `kuota`. Mengembalikan
-- `tabel` atau `nil, kode`.
local function al_hitung(q_masuk, daftar)
  local q = tonumber(q_masuk)
  if q == nil or q ~= q or q < 1 or q % 1 ~= 0 then
    return nil, 'kuota_efektif_tidak_sah'
  end
  q = math.floor(q)
  local reserved = 0
  for i = 1, #daftar do
    reserved = reserved + al_reserved(daftar[i], q)
  end
  if reserved > q then
    return nil, 'alokasi_melebihi_anggaran'
  end
  return { kuota = q, reserved = reserved, shareable = q - reserved }
end

-- Batas satu rute dalam percobaan, dengan `terpakai_rute` = percobaan rute ini
-- dan `terpakai_pool` = percobaan seluruh rute pool di periode yang sama.
-- Padanan `Anggaran::batas_rute` di Rust; tidak pernah negatif.
--
-- Parameter keduanya bernama `rute`, dan nama itu juga dipakai untuk angka
-- "percobaan rute ini": pada versi pertama keduanya bertabrakan sehingga
-- cabang ber-alokasi selalu membaca angka, bukan tabel, dan menjawab Q utuh.
-- `uji-lua` tidak bisa melihatnya karena asersinya sendiri yang salah; uji
-- `tests/alokasi_lua.rs` menangkapnya karena membandingkan dengan Rust.
local function al_batas_rute(anggaran, al_rute, terpakai_rute, terpakai_pool)
  local rute = math.max(tonumber(terpakai_rute) or 0, 0)
  local pool = math.max(tonumber(terpakai_pool) or 0, 0)
  local sisa_shareable = anggaran.shareable - (pool - rute)
  if sisa_shareable < 0 then
    sisa_shareable = 0
  end
  if al_rute == nil then
    local batas = anggaran.kuota - pool
    if batas < 0 then
      return 0
    end
    return batas
  end
  -- `al_reserved` tidak menjepit hasilnya. Persen di atas 100% ditolak saat
  -- urai, tetapi rute bisa sampai di sini dengan nilai yang belum lewat urai
  -- (mis. dari ARGV yang dirakit pemanggil): jepit di sini supaya tidak ada
  -- jatah negatif yang terbaca sebagai angka besar, sama seperti `.max(0)` di
  -- `Anggaran::batas_rute` Rust.
  local sisa_reserved = al_reserved(al_rute, anggaran.kuota) - rute
  if sisa_reserved < 0 then
    sisa_reserved = 0
  end
  return sisa_reserved + sisa_shareable
end

return {
  MAKS_ALOKASI = AL_MAKS_ALOKASI,
  JENIS_EXACT = AL_JENIS_EXACT,
  JENIS_PERSEN = AL_JENIS_PERSEN,
  urai = al_urai,
  hitung = al_hitung,
  reserved = al_reserved,
  skala = al_bulat_skala,
  batas_rute = al_batas_rute
}
