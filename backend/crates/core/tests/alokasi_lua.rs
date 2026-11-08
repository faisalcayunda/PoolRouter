//! The budget model in Rust and in Lua must agree (ARC-108, AL-01..AL-05).
//!
//! `core::kuota::alokasi` decides what a pool's reservations are; the chunk
//! `lua/potongan/alokasi.lua` is the copy the token script and the dispatcher
//! will run inside Redis, where the same numbers have to come out of string
//! `ARGV`. Two implementations of one rule drift silently, so this test pins
//! them against each other: it loads the committed chunk into real Redis and
//! compares a table of inputs with the Rust model.
//!
//! The chunk is wrapped in the shape the assembler produces
//! (`local alokasi = (function() … end)()`), so the bytes under test are the
//! bytes `scripts/rakit-lua.sh` inserts. Run by `make -C backend uji-redis`;
//! `#[ignore]`d because it needs `POOLROUTER_TEST_REDIS`.

use poolrouter_core::kuota::alokasi::{Anggaran, NilaiAlokasi};

/// One input both implementations must answer the same way.
#[derive(Debug, Clone)]
struct Kasus {
    kuota_harian: i64,
    margin: f64,
    /// `(jenis, nilai)` as the DB stores them; `jenis` is `exact` or `persen`.
    alokasi: Vec<(&'static str, f64)>,
    terpakai_rute: i64,
    terpakai_pool: i64,
    /// `None`: the route has no allocation (AL-05).
    rute: Option<(&'static str, f64)>,
}

fn nilai(jenis: &str, nilai: f64) -> Option<NilaiAlokasi> {
    match jenis {
        "exact" if nilai >= 0.0 && nilai.fract() == 0.0 && nilai <= i64::MAX as f64 => {
            NilaiAlokasi::exact(nilai as i64).ok()
        }
        "exact" => None,
        "persen" => NilaiAlokasi::persen(nilai).ok(),
        _ => None,
    }
}

impl Kasus {
    /// `(Q, reserved, shareable, batas_rute)`, or `None` when the configuration
    /// is refused. `batas_rute` is `None` for the "budget only" cases.
    fn ke_rust(&self) -> Option<(i64, i64, i64, Option<i64>)> {
        let daftar = self
            .alokasi
            .iter()
            .map(|(j, n)| nilai(j, *n))
            .collect::<Option<Vec<_>>>()?;
        let a = Anggaran::dari_alokasi(self.kuota_harian, self.margin, &daftar).ok()?;
        let batas = match &self.rute {
            Some((j, n)) => {
                Some(a.batas_rute(Some(nilai(j, *n)?), self.terpakai_rute, self.terpakai_pool))
            }
            None => Some(a.batas_rute(None, self.terpakai_rute, self.terpakai_pool)),
        };
        Some((a.kuota_efektif(), a.reserved(), a.shareable(), batas))
    }
}

fn kasus_uji() -> Vec<Kasus> {
    let ex = |n: i64| ("exact", n as f64);
    let ps = |n: f64| ("persen", n);
    let mut k = Vec::new();

    for (kuota_harian, margin) in [(300, 0.01), (37, 0.01), (1_000, 0.2), (100, 0.001)] {
        // Tanpa alokasi: seluruh anggaran shareable.
        k.push(Kasus {
            kuota_harian,
            margin,
            alokasi: vec![],
            terpakai_rute: 0,
            terpakai_pool: 0,
            rute: None,
        });
        // Exact dicampur persen, dengan pemakaian rute dan pool.
        k.push(Kasus {
            kuota_harian,
            margin,
            alokasi: vec![ex(10), ps(12.5), ex(3)],
            terpakai_rute: 7,
            terpakai_pool: 25,
            rute: Some(ex(10)),
        });
        // Persen pecahan: pembulatan ke bawah harus sama di kedua sisi.
        k.push(Kasus {
            kuota_harian,
            margin,
            alokasi: vec![ps(33.33), ps(0.01)],
            terpakai_rute: 0,
            terpakai_pool: 0,
            rute: Some(ps(33.33)),
        });
        // Rute tanpa alokasi ketika pool sudah terpakai.
        k.push(Kasus {
            kuota_harian,
            margin,
            alokasi: vec![ex(1)],
            terpakai_rute: 2,
            terpakai_pool: 5,
            rute: None,
        });
        // 100% persen: reserved == Q, tidak ada yang shareable.
        k.push(Kasus {
            kuota_harian,
            margin,
            alokasi: vec![ps(100.0)],
            terpakai_rute: 0,
            terpakai_pool: 0,
            rute: Some(ps(100.0)),
        });
    }

    // Ditolak kedua sisi: melebihi anggaran (AL-01).
    k.push(Kasus {
        kuota_harian: 300,
        margin: 0.01,
        alokasi: vec![ex(298)],
        terpakai_rute: 0,
        terpakai_pool: 0,
        rute: None,
    });
    k.push(Kasus {
        kuota_harian: 300,
        margin: 0.01,
        alokasi: vec![ex(200), ps(40.0)],
        terpakai_rute: 0,
        terpakai_pool: 0,
        rute: None,
    });
    // Ditolak kedua sisi: nilai tidak sah.
    for alokasi in [
        vec![ex(0)],
        vec![ex(-3)],
        vec![ps(0.0)],
        vec![ps(-1.0)],
        vec![ps(100.5)],
    ] {
        k.push(Kasus {
            kuota_harian: 300,
            margin: 0.01,
            alokasi,
            terpakai_rute: 0,
            terpakai_pool: 0,
            rute: None,
        });
    }
    // Ditolak kedua sisi: margin di luar rentang kolom.
    k.push(Kasus {
        kuota_harian: 300,
        margin: 0.5,
        alokasi: vec![],
        terpakai_rute: 0,
        terpakai_pool: 0,
        rute: None,
    });
    k
}

/// The wrapper the assembler produces plus a driver: one case per entry of the
/// JSON array in `ARGV[1]`, answers as flat strings.
fn pembungkus(sumber: &str) -> String {
    format!(
        r#"local alokasi = (function()
{sumber}
end)()
local d = cjson.decode(ARGV[1])
local hasil = {{}}
for i, k in ipairs(d.kasus) do
  local daftar, kode = alokasi.urai(k.argv, 1, k.n)
  if daftar == nil then
    hasil[i] = {{ 'galat', kode }}
  else
    local a, kode2 = alokasi.hitung(k.kuota, daftar)
    if a == nil then
      hasil[i] = {{ 'galat', kode2 }}
    else
      local baris = {{ 'ok', a.kuota, a.reserved, a.shareable }}
      if k.rute_jenis ~= 'tidak' then
        local rute = nil
        if k.rute_jenis ~= '' then
          -- Same shape as a parsed allocation: `nilai` carries what `skala`
          -- produces for that jenis (exact: attempts, persen: hundredths of a
          -- percent), so the route is costed like the pool's allocations.
          rute = {{ jenis = k.rute_jenis, nilai = k.rute_nilai }}
        end
        baris[#baris + 1] = alokasi.batas_rute(a, rute, k.terpakai_rute, k.terpakai_pool)
      end
      hasil[i] = baris
    end
  end
end
return hasil
"#
    )
}

/// `Q`, which is what the chunk receives: the margin is a Rust-side
/// calculation (`kuota_efektif()`), the scripts only ever see the result.
fn kuota_efektif(k: &Kasus) -> i64 {
    poolrouter_core::kuota::alokasi::kuota_efektif(k.kuota_harian, k.margin).unwrap_or(-1)
}

/// The value as `ARGV` carries it: `persen` as percent with up to two decimals
/// (`12.5`, `33.33`) and `exact` as whole attempts. The chunk's `skala` turns
/// both into hundredths itself, so this is the same text a script gets from the
/// dispatcher — and the shape the DB column `numeric(12,2)` holds.
fn nilai_argv(jenis: &str, nilai: f64) -> String {
    match jenis {
        "persen" => format!("{nilai}"),
        _ => format!("{}", nilai as i64),
    }
}

/// The route's value as the chunk's `urai` leaves it: `exact` in attempts,
/// `persen` in hundredths of a percent (`12.5` -> `1250`).
fn nilai_nilai(jenis: &str, nilai: f64) -> String {
    match jenis {
        "persen" => format!("{}", (nilai * 100.0).round() as i64),
        _ => format!("{}", nilai as i64),
    }
}

/// The cases as the driver's JSON. The payload is small and fully controlled
/// (alphanumeric strings and numbers), so it is written by hand: the workspace
/// has no JSON dependency and this test must not add one.
fn json_kasus(kasus: &[Kasus]) -> String {
    let mut isi = String::from("{\"kasus\":[");
    for (i, k) in kasus.iter().enumerate() {
        if i > 0 {
            isi.push(',');
        }
        // `ARGV` carries what the DB column carries: `persen` in hundredths of
        // a percent (`numeric(12,2)`), `exact` as a whole number.
        let mut argv = Vec::new();
        for (j, n) in &k.alokasi {
            argv.push(format!("\"{j}\""));
            argv.push(format!("\"{}\"", nilai_argv(j, *n)));
        }
        let (rute_jenis, rute_nilai) = match &k.rute {
            Some((j, n)) => (*j, nilai_nilai(j, *n)),
            None => ("", String::from("0")),
        };
        isi.push_str(&format!(
            "{{\"argv\":[{}],\"n\":{},\"kuota\":\"{}\",\"rute_jenis\":\"{rute_jenis}\",\
             \"rute_nilai\":{rute_nilai},\"terpakai_rute\":{},\"terpakai_pool\":{}}}",
            argv.join(","),
            k.alokasi.len(),
            kuota_efektif(k),
            k.terpakai_rute,
            k.terpakai_pool,
        ));
    }
    isi.push_str("]}");
    isi
}

fn bandingkan(rust: Option<(i64, i64, i64, Option<i64>)>, lua: &[String]) -> Result<(), String> {
    match (rust, lua.first().map(String::as_str)) {
        (None, Some("galat")) => Ok(()),
        (Some(_), Some("galat")) => Err(format!("Rust menerima, Lua menolak: {lua:?}")),
        (None, _) => Err(format!("Rust menolak, Lua menerima: {lua:?}")),
        (Some((q, r, s, batas)), Some("ok")) => {
            let angka = |i: usize, nama: &str| -> Result<i64, String> {
                lua.get(i)
                    .and_then(|v| v.parse::<i64>().ok())
                    .ok_or_else(|| format!("{nama} bukan angka: {lua:?}"))
            };
            if (q, r, s)
                != (
                    angka(1, "kuota")?,
                    angka(2, "reserved")?,
                    angka(3, "shareable")?,
                )
            {
                return Err(format!(
                    "anggaran berbeda: Rust ({q},{r},{s}) vs Lua {lua:?}"
                ));
            }
            if let Some(b) = batas {
                let ab = angka(4, "batas_rute")?;
                if b != ab {
                    return Err(format!("batas_rute berbeda: Rust {b} vs Lua {ab}"));
                }
            }
            Ok(())
        }
        _ => Err(format!("balasan Lua tidak dikenal: {lua:?}")),
    }
}

#[tokio::test]
#[ignore = "butuh Redis sungguhan; jalankan lewat make -C backend uji-redis"]
async fn model_rust_dan_potongan_lua_sepakat() {
    let url = std::env::var("POOLROUTER_TEST_REDIS")
        .expect("POOLROUTER_TEST_REDIS belum diset; jalankan lewat `make -C backend uji-redis`");
    let mut koneksi = redis::Client::open(url)
        .expect("URL Redis tidak sah")
        .get_multiplexed_async_connection()
        .await
        .expect("Redis tidak dapat dihubungi");

    let skrip = pembungkus(include_str!("../src/kuota/lua/potongan/alokasi.lua"));
    let kasus = kasus_uji();
    let payload = json_kasus(&kasus);

    // Argumen Lua 1 adalah `ARGV[1]`; tidak ada KEYS yang dipakai potongan ini.
    let jawaban: Vec<Vec<String>> = redis::cmd("EVAL")
        .arg(&skrip)
        .arg(0)
        .arg(&payload)
        .query_async(&mut koneksi)
        .await
        .expect("EVAL anggaran");
    assert_eq!(
        jawaban.len(),
        kasus.len(),
        "satu balasan per kasus, dapat {} untuk {}: {:?}",
        jawaban.len(),
        kasus.len(),
        &jawaban[..jawaban.len().min(3)]
    );
    for (i, k) in kasus.iter().enumerate() {
        bandingkan(k.ke_rust(), &jawaban[i]).unwrap_or_else(|e| panic!("kasus {i} {k:?}: {e}"));
    }
}

/// The chunk speaks the same `exact` / `persen` the column does; a rename on
/// one side would otherwise only surface in production.
#[test]
fn jenis_di_potongan_lua_sama_dengan_di_rust() {
    use poolrouter_core::kuota::alokasi::JenisAlokasi;

    let sumber = include_str!("../src/kuota/lua/potongan/alokasi.lua");
    for jenis in [JenisAlokasi::Exact, JenisAlokasi::Persen] {
        let teks = format!("'{}'", jenis.sebagai_teks());
        assert!(sumber.contains(&teks), "potongan Lua tidak mengenal {teks}");
    }
}

/// A reservation is in attempts; `persen` values are in hundredths of a
/// percent. The chunk must go through the stored hundredths (`periksa`), not
/// the decimal form: `floor(297 × 33.33 / 10_000)` is `0` to Lua, while the
/// stored `3333` gives `98`. Rust never had the choice — `NilaiAlokasi` keeps
/// hundredths — so this pins the Lua side of that decision on its own, without
/// a Redis.
#[test]
fn potongan_tidak_mengalikan_persen_desimal() {
    let sumber = include_str!("../src/kuota/lua/potongan/alokasi.lua");
    let reserved = sumber
        .split("local function al_reserved")
        .nth(1)
        .expect("al_reserved ada di potongan");
    let badan = &reserved[..reserved.find("\nend").unwrap_or(reserved.len())];
    assert!(
        badan.contains("AL_SKALA_PERSEN"),
        "al_reserved harus membagi dengan AL_SKALA_PERSEN (perseratus persen)"
    );
    assert!(
        !sumber.contains("alokasi.nilai_desimal"),
        "potongan tidak boleh memakai bentuk desimal nilai"
    );
}
