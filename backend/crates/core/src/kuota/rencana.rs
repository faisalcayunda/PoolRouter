//! The plan of a pool's budget for the dispatcher (ADR-0006 rev 1 section 4
//! and 11; spec `f1-prioritas-alokasi`, AC-1..AC-3).
//!
//! [`Anggaran`](super::alokasi::Anggaran) answers "how much may this one route
//! still do"; the dispatcher needs the next question — *where* may it take a
//! token from, and in what order. Three pieces answer it, all pure:
//!
//! ```text
//! rencana_anggaran(d)  a_i per route, plus `a_sisa` = Q − Σ a_i
//! urutan_sumber(d)     [own allocation, sisa, shareable of other pools]
//! ruang_intake(d)      how many attempts pool d may accept right now
//! ```
//!
//! The order matters as much as the amounts: a route spends its own
//! reservation first, then the pool's `sisa`, and only then borrows — AL-05 and
//! AL-07 say a route may borrow, not that it may starve someone else. A pool
//! whose reservation is exhausted must take from `sisa` before touching another
//! pool's reservation, which is why the list is ordered rather than summed.
//!
//! Everything here is a pure function of the plan handed in; nothing reads
//! Redis or the clock, so the dispatcher order can be tested exhaustively
//! (AC-15) and the Lua side has one place to agree with.

use super::alokasi::{Anggaran, GalatAlokasi, NilaiAlokasi};

/// One active route of a pool: its `downstream_pool_id` and its allocation.
///
/// `alokasi = None` is a route without allocation: it has `a_i = 0` and lives
/// entirely off `sisa` (AC-1).
#[derive(Debug, Clone, PartialEq)]
pub struct Rute {
    pub downstream_pool_id: i64,
    pub alokasi: Option<NilaiAlokasi>,
}

impl Rute {
    /// A route that holds a reservation.
    pub fn beralokasi(downstream_pool_id: i64, alokasi: NilaiAlokasi) -> Self {
        Self {
            downstream_pool_id,
            alokasi: Some(alokasi),
        }
    }

    /// A route that may only borrow from what is shareable.
    pub fn tanpa_alokasi(downstream_pool_id: i64) -> Self {
        Self {
            downstream_pool_id,
            alokasi: None,
        }
    }
}

/// `a_i` per route and `a_sisa` of one pool (AC-1).
///
/// `a_i` is in attempts, the same unit as `Q` and `terpakai`; a route without
/// allocation gets `0`. `a_sisa` is what no reservation holds — the shareable
/// part — and is never negative, because a plan over `Q` is an error and not a
/// plan (AC-1, AL-01).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RencanaAnggaran {
    anggaran: Anggaran,
    anggaran_rute: Vec<(i64, i64)>,
    sisa: i64,
}

impl RencanaAnggaran {
    /// Builds the plan of one pool.
    ///
    /// The reservations are summed exactly as [`Anggaran::dari_alokasi`] sums
    /// them, so a plan and a budget can never disagree about `Σ a_i`; when they
    /// do not fit, the error carries the excess (AC-1, U-64).
    pub fn baru(kuota_harian: i64, margin: f64, rute: &[Rute]) -> Result<Self, GalatAlokasi> {
        let mut alokasi = Vec::with_capacity(rute.len());
        for r in rute {
            if let Some(a) = r.alokasi {
                alokasi.push(a);
            }
        }
        let anggaran = Anggaran::dari_alokasi(kuota_harian, margin, &alokasi)?;
        let kuota = anggaran.kuota_efektif();
        let mut per_rute = Vec::with_capacity(rute.len());
        let mut jumlah = 0i64;
        for r in rute {
            let a_i = r.alokasi.map(|a| a.reserved(kuota)).unwrap_or(0);
            jumlah = jumlah.saturating_add(a_i);
            per_rute.push((r.downstream_pool_id, a_i));
        }
        Ok(Self {
            sisa: (kuota - jumlah).max(0),
            anggaran,
            anggaran_rute: per_rute,
        })
    }

    /// The budget the plan was built against.
    pub fn anggaran(&self) -> &Anggaran {
        &self.anggaran
    }

    /// `a_sisa = Q − Σ a_i`, the part every route may take from (AC-1).
    pub fn sisa(&self) -> i64 {
        self.sisa
    }

    /// `a_i` of one route, `0` for a route without allocation or not in the
    /// plan.
    pub fn anggaran_rute(&self, downstream_pool_id: i64) -> i64 {
        self.anggaran_rute
            .iter()
            .filter(|(id, _)| *id == downstream_pool_id)
            .map(|(_, a_i)| *a_i)
            .sum()
    }

    /// `a_i` of every route, in the order they were handed in.
    pub fn per_rute(&self) -> &[(i64, i64)] {
        &self.anggaran_rute
    }
}

/// One source of tokens on a route's list (AC-2).
///
/// The tag is what the dispatcher and the metric label read: `sendiri` is the
/// route's own reservation, `sisa` the pool's unreserved part, `pinjam` another
/// pool's shareable part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sumber {
    /// `a_i` of the route itself.
    Sendiri,
    /// `a_sisa` of the pool.
    Sisa,
    /// The shareable part of another pool, by its id.
    Pinjam { upstream_pool_id: i64 },
}

/// One entry of [`urutan_sumber`]: where to take from and how much is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SumberDana {
    pub sumber: Sumber,
    /// Attempts available at this source, never negative.
    pub ruang: i64,
    /// Other pools sharing this source, smallest id first. Empty for `sendiri`
    /// and `sisa`, which belong to one pool each.
    pub pool_donor: &'static [i64],
}

impl SumberDana {
    fn baru(sumber: Sumber, ruang: i64) -> Self {
        Self {
            sumber,
            ruang: ruang.max(0),
            pool_donor: &[],
        }
    }
}

/// A pool that may lend from its shareable part (AC-2, AC-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolPeminjam {
    pub upstream_pool_id: i64,
    /// What this pool may still lend: its shareable part minus what it lent.
    pub ruang: i64,
}

/// Where pool `d` may take a token from, in order (AC-2).
///
/// With `boleh_pinjam_d = false` the list is just `[anggaran d]`. With `true` it
/// is `[anggaran d, sisa, lalu shareable pool lain]`, the borrowed pools ordered
/// by the largest room first and ties broken by the smallest
/// `upstream_pool_id`, so the order is total and does not depend on how the
/// pools happened to arrive.
///
/// The result never contains another pool's *reservation*: a borrowed source is
/// a shareable part only. That is the invariant the property test pins.
pub fn urutan_sumber(
    rencana: &RencanaAnggaran,
    downstream_pool_id: i64,
    boleh_pinjam_d: bool,
    peminjam: &[PoolPeminjam],
) -> Vec<SumberDana> {
    let mut hasil = Vec::with_capacity(2 + peminjam.len());
    hasil.push(SumberDana::baru(
        Sumber::Sendiri,
        rencana.anggaran_rute(downstream_pool_id),
    ));
    if !boleh_pinjam_d {
        return hasil;
    }
    hasil.push(SumberDana::baru(Sumber::Sisa, rencana.sisa()));
    let mut lain: Vec<&PoolPeminjam> = peminjam.iter().collect();
    // Largest room first; the id only breaks ties, so the order is stable.
    lain.sort_by(|a, b| {
        b.ruang
            .max(0)
            .cmp(&a.ruang.max(0))
            .then(a.upstream_pool_id.cmp(&b.upstream_pool_id))
    });
    for p in lain {
        hasil.push(SumberDana::baru(
            Sumber::Pinjam {
                upstream_pool_id: p.upstream_pool_id,
            },
            p.ruang,
        ));
    }
    hasil
}

/// How many attempts pool `d` may accept right now (AC-3).
///
/// The pool's own room — `Q − terpakai`, never negative — plus, only when it may
/// borrow, the pool's `sisa` and the shareable parts of the other pools
/// (ADR-0006 §4 butir 7). Accepting more than this would promise attempts the
/// pool has no budget for, and the intake gate would then reject them later
/// with a code that looks like a quota problem rather than a capacity one.
pub fn ruang_intake(
    rencana: &RencanaAnggaran,
    terpakai: i64,
    boleh_pinjam_d: bool,
    peminjam: &[PoolPeminjam],
) -> i64 {
    let sendiri = rencana.anggaran().sisa(terpakai);
    if !boleh_pinjam_d {
        return sendiri;
    }
    let mut total = sendiri.saturating_add(rencana.sisa());
    for p in peminjam {
        total = total.saturating_add(p.ruang.max(0));
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persen(p: f64) -> NilaiAlokasi {
        NilaiAlokasi::persen(p).unwrap()
    }

    fn exact(n: i64) -> NilaiAlokasi {
        NilaiAlokasi::exact(n).unwrap()
    }

    #[test]
    fn rencana_exact_dan_persen() {
        // Q = floor(300 × 0.99) = 297; 10% of 297 = 29.7 -> 29; exact 30.
        let r = RencanaAnggaran::baru(
            300,
            0.01,
            &[
                Rute::beralokasi(1, exact(30)),
                Rute::beralokasi(2, persen(10.0)),
                Rute::tanpa_alokasi(3),
            ],
        )
        .unwrap();
        assert_eq!(r.anggaran().kuota_efektif(), 297);
        assert_eq!(r.anggaran_rute(1), 30);
        assert_eq!(r.anggaran_rute(2), 29);
        assert_eq!(r.anggaran_rute(3), 0);
        assert_eq!(r.sisa(), 297 - 59);
        assert_eq!(r.anggaran().reserved(), 59);
    }

    #[test]
    fn rencana_kelebihan_bukan_rencana() {
        let galat = RencanaAnggaran::baru(
            300,
            0.01,
            &[
                Rute::beralokasi(1, exact(200)),
                Rute::beralokasi(2, exact(100)),
            ],
        );
        assert_eq!(
            galat,
            Err(GalatAlokasi::MelebihiAnggaran {
                kuota: 297,
                reserved: 300,
                lebih: 3,
            })
        );
    }

    #[test]
    fn rencana_persen_dibulatkan_ke_bawah() {
        // 33.33% of 297 = 98.99 -> 98, so a plan of three such routes fits.
        let r = RencanaAnggaran::baru(
            300,
            0.01,
            &[
                Rute::beralokasi(1, persen(33.33)),
                Rute::beralokasi(2, persen(33.33)),
                Rute::beralokasi(3, persen(33.33)),
            ],
        )
        .unwrap();
        assert_eq!(r.anggaran_rute(1), 98);
        assert_eq!(r.sisa(), 297 - 294);
    }

    #[test]
    fn urutan_tanpa_pinjam_hanya_anggaran_sendiri() {
        let r = RencanaAnggaran::baru(300, 0.01, &[Rute::beralokasi(1, exact(30))]).unwrap();
        let u = urutan_sumber(
            &r,
            1,
            false,
            &[PoolPeminjam {
                upstream_pool_id: 9,
                ruang: 100,
            }],
        );
        assert_eq!(
            u.iter().map(|s| s.sumber).collect::<Vec<_>>(),
            vec![Sumber::Sendiri]
        );
    }

    #[test]
    fn urutan_pinjam_terbesar_dulu_seri_id_terkecil() {
        let r = RencanaAnggaran::baru(300, 0.01, &[Rute::beralokasi(1, exact(30))]).unwrap();
        let u = urutan_sumber(
            &r,
            1,
            true,
            &[
                PoolPeminjam {
                    upstream_pool_id: 7,
                    ruang: 50,
                },
                PoolPeminjam {
                    upstream_pool_id: 3,
                    ruang: 50,
                },
                PoolPeminjam {
                    upstream_pool_id: 5,
                    ruang: 80,
                },
            ],
        );
        let sumber: Vec<Sumber> = u.iter().map(|s| s.sumber).collect();
        assert_eq!(
            sumber,
            vec![
                Sumber::Sendiri,
                Sumber::Sisa,
                Sumber::Pinjam {
                    upstream_pool_id: 5
                },
                Sumber::Pinjam {
                    upstream_pool_id: 3
                },
                Sumber::Pinjam {
                    upstream_pool_id: 7
                },
            ]
        );
    }

    #[test]
    fn urutan_tidak_memuat_reserved_pool_lain() {
        // Pool lain hanya menyumbang bagian shareable-nya: `ruang` diberikan
        // pemanggil dari `Anggaran::shareable()`, dan tidak ada jalur di sini
        // yang membaca `reserved` pool lain.
        let r = RencanaAnggaran::baru(300, 0.01, &[Rute::beralokasi(1, exact(30))]).unwrap();
        let u = urutan_sumber(
            &r,
            1,
            true,
            &[PoolPeminjam {
                upstream_pool_id: 2,
                ruang: 10,
            }],
        );
        for s in &u {
            if let Sumber::Pinjam { upstream_pool_id } = s.sumber {
                assert_ne!(upstream_pool_id, 1, "pool sendiri tidak boleh jadi donor");
            }
        }
    }

    #[test]
    fn urutan_memakai_reserved_rute_downstream_saja() {
        let r = RencanaAnggaran::baru(
            300,
            0.01,
            &[
                Rute::beralokasi(1, exact(30)),
                Rute::beralokasi(2, exact(60)),
            ],
        )
        .unwrap();

        let u = urutan_sumber(&r, 1, true, &[]);

        assert_eq!(u[0].sumber, Sumber::Sendiri);
        assert_eq!(u[0].ruang, r.anggaran_rute(1));
        assert_eq!(u[0].ruang, 30);
    }

    #[test]
    fn ruang_intake_tanpa_pinjam_hanya_sisa_sendiri() {
        let r = RencanaAnggaran::baru(300, 0.01, &[]).unwrap();
        assert_eq!(
            ruang_intake(
                &r,
                100,
                false,
                &[PoolPeminjam {
                    upstream_pool_id: 2,
                    ruang: 50
                }]
            ),
            197
        );
        // Terpakai melebihi Q: nol, bukan negatif.
        assert_eq!(ruang_intake(&r, 400, false, &[]), 0);
    }

    #[test]
    fn ruang_intake_dengan_pinjam_menambah_sisa_dan_shareable_lain() {
        let r = RencanaAnggaran::baru(300, 0.01, &[Rute::beralokasi(1, exact(30))]).unwrap();
        let ruang = ruang_intake(
            &r,
            100,
            true,
            &[
                PoolPeminjam {
                    upstream_pool_id: 2,
                    ruang: 50,
                },
                PoolPeminjam {
                    upstream_pool_id: 3,
                    ruang: 25,
                },
            ],
        );
        // sendiri 197 + sisa 267 + 50 + 25
        assert_eq!(ruang, 197 + r.sisa() + 75);
    }

    #[test]
    fn ruang_tidak_pernah_negatif() {
        let r = RencanaAnggaran::baru(300, 0.01, &[]).unwrap();
        assert_eq!(
            ruang_intake(
                &r,
                10_000,
                true,
                &[PoolPeminjam {
                    upstream_pool_id: 2,
                    ruang: -5
                }]
            ),
            r.sisa()
        );
    }
}
