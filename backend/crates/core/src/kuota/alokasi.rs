//! Budget of one upstream pool and its routes (ADR-0006 rev 1 section 3, 4, 10
//! and 11; spec `f1-prioritas-alokasi`, decisions AL-01..AL-08 and PA1..PA12).
//!
//! One upstream pool owns a daily quota `Q`, which is what the Redis scripts
//! compare `terpakai` against:
//!
//! ```text
//! Q = floor(kuota_harian * (1 - margin_kuota))
//! ```
//!
//! Inside `Q` an active allocated route may hold a *reserved* part, so a route
//! with a share can never be starved by traffic of another route. What is left
//! is *shareable* and every route may take from it (AL-02, AL-03):
//!
//! ```text
//! reserved  = Σ alokasi aktif                 exact: alokasi_nilai
//!                                             persen: floor(Q * nilai / 100)
//! shareable = Q - reserved
//! ```
//!
//! This module is a **prefactor**: it is the model, the arithmetic and the
//! vocabulary (AL-01..AL-08), and it is what the offline budget is computed
//! from. Redis still counts on `Q` alone
//! ([`super::store::KonteksKuota::kuota_efektif`] keeps its meaning); enforcing
//! the reserved part in the token script and in the dispatcher belongs to
//! `KuotaStore` (ARC-109) and to the controller/worker path (ARC-114).
//!
//! Two rules are load-bearing here:
//!
//! - **One budget per upstream pool, one origin (AL-01).** `reserved` can never
//!   exceed `Q`, so a configuration whose allocations do not fit is rejected
//!   instead of being silently scaled down or clipped per route. Rejecting
//!   keeps the arithmetic reversible: the sum of the parts always equals the
//!   whole that was configured.
//! - **The process boundary is not part of the model (AL-06).** A route may be
//!   handed over between components; the reservation is attached to the route,
//!   not to whoever served it. Nothing in this file reads a clock: the caller
//!   passes the closing configuration versions, exactly like the period
//!   arguments of the quota scripts (ADR-0001 rev 15 section 5a.1 point 1d).

/// Which quantity an allocation is expressed in (AL-02, AL-04).
///
/// `Exact` is a count of attempts in the period; `Persen` is a percentage of
/// the pool's own budget `Q`, not of `kuota_harian`. The two can be mixed in
/// one pool, and the DB constraint `chk_alokasi_tipe` keeps the pair
/// (kind, value) consistent before it ever reaches here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JenisAlokasi {
    /// `alokasi_jenis = 'exact'`: a number of attempts, at least 1.
    Exact,
    /// `alokasi_jenis = 'persen'`: a share of `Q`, greater than 0 and at most
    /// 100, with at most 2 decimals.
    Persen,
}

impl JenisAlokasi {
    /// The DB spelling (`downstream_pool_rute.alokasi_jenis`).
    pub fn sebagai_teks(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Persen => "persen",
        }
    }

    /// Parses the DB spelling; unknown text is `None` (fail-closed).
    pub fn dari_teks(teks: &str) -> Option<Self> {
        match teks {
            "exact" => Some(Self::Exact),
            "persen" => Some(Self::Persen),
            _ => None,
        }
    }
}

/// The value of one allocation, already consistent with its kind.
///
/// A `Persen` value is held in hundredths of a percent (2 decimals, the scale
/// of `numeric(12,2)`), so the arithmetic stays integral and the result cannot
/// drift with the platform's floating point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NilaiAlokasi {
    jenis: JenisAlokasi,
    /// `exact`: attempts. `persen`: hundredths of a percent (100_00 = 100%).
    nilai: i64,
}

/// Why a configuration was refused. Every variant is a configuration error,
/// never a quota decision (ADR-0001 rev 15 section 5: fail-closed).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GalatAlokasi {
    #[error("kuota_harian harus positif, bukan {0}")]
    KuotaTidakSah(i64),
    #[error("margin_kuota {0} tidak sah: harus 0.001..=0.200")]
    MarginTidakSah(f64),
    #[error("alokasi {nilai} tidak sah untuk jenis {jenis}: {alasan}")]
    NilaiTidakSah {
        jenis: &'static str,
        nilai: String,
        alasan: &'static str,
    },
    /// AL-01: the reservations do not fit in `Q`. `lebih` is how far over.
    #[error("alokasi melebihi anggaran: reserved {reserved} > Q {kuota}, lebih {lebih}")]
    MelebihiAnggaran {
        kuota: i64,
        reserved: i64,
        lebih: i64,
    },
}

/// `Q = floor(kuota_harian * (1 - margin))` (AL-01).
///
/// The product is computed in integers from the margin's permille, the scale of
/// `numeric(4,3)` in the schema and of [`MARGIN_MIN`]/[`MARGIN_MAKS`] here, so
/// `margin_kuota = 0.010` gives exactly the same `Q` as the SQL trigger
/// `tg_upstream_pool_alokasi` does. A margin outside the range is refused, not
/// clamped: the admin API already refuses it at the edge, and a value that
/// reaches this function out of range means a bug upstream of it.
pub const MARGIN_MIN: f64 = 0.001;
pub const MARGIN_MAKS: f64 = 0.200;

/// The "no margin" value, for a caller that wants `Q == kuota_harian`.
pub const MARGIN_NOL: f64 = 0.0;

pub fn kuota_efektif(kuota_harian: i64, margin: f64) -> Result<i64, GalatAlokasi> {
    if kuota_harian < 1 {
        return Err(GalatAlokasi::KuotaTidakSah(kuota_harian));
    }
    // `MARGIN_NOL` is accepted so the model can be used without a margin; the
    // configured column never stores it (`CHECK margin_kuota BETWEEN 0.001 AND 0.200`).
    let margin_boleh_nol = margin == MARGIN_NOL;
    if !(MARGIN_MIN..=MARGIN_MAKS).contains(&margin) && !margin_boleh_nol {
        return Err(GalatAlokasi::MarginTidakSah(margin));
    }
    let permil = permil_dari_margin(margin);
    // floor(kuota * (1000 - permil) / 1000), all in i64: kuota_harian is a
    // bigint in the schema, so the product stays well inside i64.
    Ok(kuota_harian * (1000 - permil) / 1000)
}

/// `margin` (0.000..=1.000) to its permille, rounding half away from zero the
/// way `numeric(4,3)` stores it.
fn permil_dari_margin(margin: f64) -> i64 {
    (margin * 1000.0).round() as i64
}

impl NilaiAlokasi {
    /// An `exact` allocation of `jumlah` attempts (at least 1, AL-02).
    pub fn exact(jumlah: i64) -> Result<Self, GalatAlokasi> {
        if jumlah < 1 {
            return Err(GalatAlokasi::NilaiTidakSah {
                jenis: JenisAlokasi::Exact.sebagai_teks(),
                nilai: jumlah.to_string(),
                alasan: "exact harus >= 1 dan bulat",
            });
        }
        Ok(Self {
            jenis: JenisAlokasi::Exact,
            nilai: jumlah,
        })
    }

    /// A `persen` allocation (greater than 0, at most 100, at most 2 decimals,
    /// AL-02). The value is rounded to 2 decimals, the scale the column keeps.
    pub fn persen(persen: f64) -> Result<Self, GalatAlokasi> {
        let seratus = (persen * 100.0).round();
        if !seratus.is_finite() || !(0.0..=100.0 * 100.0).contains(&seratus) || seratus <= 0.0 {
            return Err(GalatAlokasi::NilaiTidakSah {
                jenis: JenisAlokasi::Persen.sebagai_teks(),
                nilai: persen.to_string(),
                alasan: "persen harus > 0 dan <= 100",
            });
        }
        Ok(Self {
            jenis: JenisAlokasi::Persen,
            nilai: seratus as i64,
        })
    }

    pub fn jenis(self) -> JenisAlokasi {
        self.jenis
    }

    /// `exact`: attempts. `persen`: hundredths of a percent (100_00 = 100).
    pub fn nilai(self) -> i64 {
        self.nilai
    }

    /// The reserved part in attempts (AL-03). For `persen` it rounds **down**,
    /// so the sum of the shares can only be smaller than `Q`, never larger: the
    /// route is never promised an attempt the pool cannot deliver.
    pub fn reserved(self, kuota: i64) -> i64 {
        match self.jenis {
            JenisAlokasi::Exact => self.nilai,
            JenisAlokasi::Persen => kuota * self.nilai / 10_000,
        }
    }

    /// The value as it is written back to `downstream_pool_rute.alokasi_nilai`
    /// (`numeric(12,2)`): `exact` as an integer, `persen` as percent with 2
    /// decimals.
    pub fn nilai_desimal(self) -> f64 {
        match self.jenis {
            JenisAlokasi::Exact => self.nilai as f64,
            JenisAlokasi::Persen => self.nilai as f64 / 100.0,
        }
    }
}

/// The budget of one upstream pool, already checked against AL-01.
///
/// Built by [`Anggaran::baru`] and [`Anggaran::dari_alokasi`]; there is no way
/// to obtain an `Anggaran` whose reservations do not fit its quota.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anggaran {
    kuota_efektif: i64,
    reserved: i64,
}

impl Anggaran {
    /// The budget without any reservation: everything is shareable.
    pub fn baru(kuota_harian: i64, margin: f64) -> Result<Self, GalatAlokasi> {
        Ok(Self {
            kuota_efektif: kuota_efektif(kuota_harian, margin)?,
            reserved: 0,
        })
    }

    /// A budget with the reservations of its active routes (AL-01: the sum must
    /// fit in `Q`).
    pub fn dari_alokasi(
        kuota_harian: i64,
        margin: f64,
        alokasi: &[NilaiAlokasi],
    ) -> Result<Self, GalatAlokasi> {
        let kuota = kuota_efektif(kuota_harian, margin)?;
        let mut reserved: i64 = 0;
        for a in alokasi {
            reserved = reserved.saturating_add(a.reserved(kuota));
        }
        if reserved > kuota {
            return Err(GalatAlokasi::MelebihiAnggaran {
                kuota,
                reserved,
                lebih: reserved - kuota,
            });
        }
        Ok(Self {
            kuota_efektif: kuota,
            reserved,
        })
    }

    /// `Q`, the number every Redis script knows as `kuota_efektif`.
    pub fn kuota_efektif(&self) -> i64 {
        self.kuota_efektif
    }

    /// The part of `Q` no other route may take (AL-02, AL-03).
    pub fn reserved(&self) -> i64 {
        self.reserved
    }

    /// The part every route may take from, reservations honoured (AL-03).
    pub fn shareable(&self) -> i64 {
        self.kuota_efektif - self.reserved
    }

    /// The ceiling of one route: attempts it may still make in the period.
    ///
    /// A route with an allocation owns its reservation and may borrow from what
    /// is left of the shareable part; a route without allocation may only
    /// borrow (AL-05, AL-07). Both arguments are counts already spent:
    /// `terpakai_rute` by this route, `terpakai_pool` by every route of the
    /// pool, so `terpakai_shareable = terpakai_pool - terpakai_rute` is what the
    /// other routes took from the shareable part.
    ///
    /// The answer never goes below zero: an exhausted budget is `0` attempts,
    /// not a negative allowance that would read as a large one.
    pub fn batas_rute(
        &self,
        alokasi: Option<NilaiAlokasi>,
        terpakai_rute: i64,
        terpakai_pool: i64,
    ) -> i64 {
        let terpakai_rute = terpakai_rute.max(0);
        let terpakai_pool = terpakai_pool.max(0);
        let sisa_shareable = (self.shareable() - (terpakai_pool - terpakai_rute)).max(0);
        match alokasi {
            Some(a) => (a.reserved(self.kuota_efektif) - terpakai_rute).max(0) + sisa_shareable,
            None => (self.kuota_efektif - terpakai_pool).max(0),
        }
    }

    /// Attempts still available in the period, whatever the route: `Q` minus
    /// what has been used.
    pub fn sisa(&self, terpakai: i64) -> i64 {
        (self.kuota_efektif - terpakai.max(0)).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kuota_efektif_sama_dengan_trigger_sql() {
        // floor(300 * (1 - 0.010)) = floor(297.0) = 297.
        assert_eq!(kuota_efektif(300, 0.010), Ok(297));
        // floor(37 * (1 - 0.01)) = floor(36.63) = 36.
        assert_eq!(kuota_efektif(37, 0.01), Ok(36));
        // floor(1 * (1 - 0.2)) = floor(0.8) = 0: the caller must treat that as
        // a configuration problem, not as "quota empty".
        assert_eq!(kuota_efektif(1, 0.2), Ok(0));
        assert_eq!(kuota_efektif(100, MARGIN_NOL), Ok(100));
    }

    #[test]
    fn kuota_efektif_menolak_masukan_tidak_sah() {
        assert_eq!(kuota_efektif(0, 0.01), Err(GalatAlokasi::KuotaTidakSah(0)));
        assert_eq!(
            kuota_efektif(100, 0.000_9),
            Err(GalatAlokasi::MarginTidakSah(0.000_9))
        );
        assert_eq!(
            kuota_efektif(100, 0.201),
            Err(GalatAlokasi::MarginTidakSah(0.201))
        );
    }

    #[test]
    fn alokasi_exact_dan_persen_divalidasi() {
        assert!(NilaiAlokasi::exact(0).is_err());
        assert!(NilaiAlokasi::exact(-1).is_err());
        assert_eq!(NilaiAlokasi::exact(5).unwrap().reserved(297), 5);

        assert!(NilaiAlokasi::persen(0.0).is_err());
        assert!(NilaiAlokasi::persen(-1.0).is_err());
        assert!(NilaiAlokasi::persen(100.01).is_err());
        assert!(NilaiAlokasi::persen(f64::NAN).is_err());
        assert_eq!(NilaiAlokasi::persen(100.0).unwrap().nilai(), 10_000);
        assert_eq!(NilaiAlokasi::persen(2.5).unwrap().nilai(), 250);
    }

    #[test]
    fn persen_dibulatkan_ke_bawah() {
        // 10% of 297 = 29.7 -> 29.
        assert_eq!(NilaiAlokasi::persen(10.0).unwrap().reserved(297), 29);
        // 33.33% of 300 = 99.99 -> 99.
        assert_eq!(NilaiAlokasi::persen(33.33).unwrap().reserved(300), 99);
        // 100% of 297 = 297.
        assert_eq!(NilaiAlokasi::persen(100.0).unwrap().reserved(297), 297);
    }

    #[test]
    fn jenis_alokasi_bolak_balik_lewat_teks() {
        for j in [JenisAlokasi::Exact, JenisAlokasi::Persen] {
            assert_eq!(JenisAlokasi::dari_teks(j.sebagai_teks()), Some(j));
        }
        assert_eq!(JenisAlokasi::dari_teks("persentase"), None);
        assert_eq!(JenisAlokasi::dari_teks(""), None);
    }

    #[test]
    fn anggaran_membagi_reserved_dan_shareable() {
        // Q = 297; 30 exact + 10% (29) reserved = 59; shareable = 238.
        let a = Anggaran::dari_alokasi(
            300,
            0.01,
            &[
                NilaiAlokasi::exact(30).unwrap(),
                NilaiAlokasi::persen(10.0).unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(a.kuota_efektif(), 297);
        assert_eq!(a.reserved(), 59);
        assert_eq!(a.shareable(), 238);
        assert_eq!(a.reserved() + a.shareable(), a.kuota_efektif());
    }

    #[test]
    fn alokasi_yang_tidak_muat_ditolak_bukan_dipotong() {
        // Q = 297; 300 exact does not fit.
        let g = Anggaran::dari_alokasi(300, 0.01, &[NilaiAlokasi::exact(300).unwrap()]);
        assert_eq!(
            g,
            Err(GalatAlokasi::MelebihiAnggaran {
                kuota: 297,
                reserved: 300,
                lebih: 3,
            })
        );
        // Tepat pas: 297 exact + 0% is allowed; reserved == Q means nothing is
        // shareable, which is a legitimate (if strict) configuration.
        let pas = Anggaran::dari_alokasi(300, 0.01, &[NilaiAlokasi::exact(297).unwrap()]).unwrap();
        assert_eq!(pas.shareable(), 0);
        // One more is refused.
        assert!(Anggaran::dari_alokasi(300, 0.01, &[NilaiAlokasi::exact(298).unwrap()]).is_err());
    }

    #[test]
    fn batas_rute_menghormati_reserved_dan_shareable() {
        let a = Anggaran::dari_alokasi(
            300,
            0.01,
            &[
                NilaiAlokasi::exact(30).unwrap(),
                NilaiAlokasi::persen(10.0).unwrap(),
            ],
        )
        .unwrap();
        // Pool-wide usage counts every route, including the reservations of
        // routes that have not spent them: `terpakai_pool` 297 leaves
        // 238 - (297 - 110) = 51 for anyone, and A takes all of it because its
        // reservation is spent and the shareable part is what pays.
        assert_eq!(
            a.batas_rute(Some(NilaiAlokasi::exact(30).unwrap()), 110, 297),
            51
        );
        // Same pool usage, but route B (10% = 29 reserved) has spent nothing:
        // it still owns its 29 and shares the same 51.
        assert_eq!(
            a.batas_rute(Some(NilaiAlokasi::persen(10.0).unwrap()), 0, 297),
            29
        );
        // Route A alone, nothing else used: its 30 plus the 238 shareable, so
        // 268, slightly less than Q because the other 10% route holds 29 it has
        // not spent. A reservation is not surrendered by not using it.
        assert_eq!(
            a.batas_rute(Some(NilaiAlokasi::exact(30).unwrap()), 0, 0),
            268
        );
        // A route without allocation borrows from the shareable part only; it
        // has no reservation to fall back on.
        assert_eq!(a.batas_rute(None, 20, 110), 187);
        // Exhausted: nobody gets a negative allowance.
        assert_eq!(
            a.batas_rute(Some(NilaiAlokasi::exact(30).unwrap()), 30, 297),
            0
        );
        assert_eq!(a.batas_rute(None, 0, 297), 0);
        assert_eq!(a.batas_rute(None, 0, 10_000), 0);
        // A route cannot spend more than its own usage when the pool total is
        // inconsistent (a stale read): the shareable part is then all that is
        // left, never a negative remainder. Here the other routes' usage goes
        // negative, which is clamped to zero, so the answer is Q - 10.
        assert_eq!(a.batas_rute(None, 999, 10), 287);
    }

    #[test]
    fn sisa_periode_tidak_pernah_negatif() {
        let a = Anggaran::baru(300, 0.01).unwrap();
        assert_eq!(a.sisa(0), 297);
        assert_eq!(a.sisa(296), 1);
        assert_eq!(a.sisa(297), 0);
        assert_eq!(a.sisa(1_000), 0);
        assert_eq!(a.sisa(-5), 297);
    }

    #[test]
    fn nilai_desimal_mengikuti_skala_kolom() {
        assert_eq!(NilaiAlokasi::exact(30).unwrap().nilai_desimal(), 30.0);
        assert_eq!(NilaiAlokasi::persen(2.5).unwrap().nilai_desimal(), 2.5);
        assert_eq!(NilaiAlokasi::persen(100.0).unwrap().nilai_desimal(), 100.0);
    }
}
