//! Quota period and 10-minute bucket arithmetic (ADR-0001 rev 15 section
//! 5a.1 point 1d, KP-02, DQ-05).
//!
//! The period is the unit the whole quota design hangs on: the daily counter
//! key is `attempts:{pool}:{periode_mulai}`, the log row carries the same
//! `periode_mulai`, and DQ-05 reconciles the two with
//! `periode_mulai <= requested_at < periode_selesai`. So the function here must
//! produce a *chain*: contiguous periods, no overlap, and a
//! `periode_mulai` that is unique per pool.
//!
//! A reset configuration change takes effect at the end of the running period,
//! never in the middle of one (KP-02): `AturanReset::berlaku_mulai` is always a
//! period end. The period that carries the change is therefore shorter (or
//! longer) than 24 hours, and ADR-0001 rev 15 requires it to stay under 24
//! hours.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};

/// Bucket width used for the rolling 24-hour window.
pub const MENIT_EMBER: i64 = 10;

/// Bucket width in seconds.
const DETIK_EMBER: i64 = MENIT_EMBER * 60;

/// Number of buckets that cover 24 hours, plus the running bucket: 144 full
/// buckets and the one that is being filled right now. The rolling window sums
/// the finished buckets and adds the running one, so a full day is 145 keys.
pub const JUMLAH_EMBER_HARIAN: u32 = 145;

/// Guard against a history that never advances. A period is at most about 24
/// hours, so this bound covers centuries of chaining.
const BATAS_LANGKAH: u32 = 100_000;

/// Errors this module reports. All of them are fail-closed: the caller treats
/// every one as "quota not available", never as "quota available"
/// (ADR-0001 rev 15 section 5).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GalatKuota {
    #[error("riwayat aturan reset kosong; periode tidak dapat dihitung")]
    RiwayatKosong,
    #[error(
        "riwayat aturan reset tidak naik pada indeks {indeks}; berlaku_mulai harus unik dan berurutan"
    )]
    RiwayatTidakUrut { indeks: usize },
    #[error(
        "requested_at {requested_at} mendahului aturan reset pertama {batas}; periode tidak dapat dihitung"
    )]
    DiLuarRiwayat {
        requested_at: DateTime<Utc>,
        batas: DateTime<Utc>,
    },
    #[error("jumlah_ember {0} tidak sah; harus paling sedikit 1")]
    JumlahEmberTidakSah(u32),
    #[error("waktu lokal {0} tidak ada di zona yang dipasang")]
    WaktuLokalTidakAda(NaiveDateTime),
    #[error("perhitungan periode tidak konvergen")]
    TidakKonvergen,
}

/// How this module reads wall-clock time in a configured reset zone.
///
/// ADR-0001 rev 15 and the `upstream_pool` trigger (KP-01) require an IANA name
/// (`reset_kuota_zona`, default `Asia/Jakarta`) and reject fixed offsets, so
/// production needs an IANA resolver. This trait keeps the period algorithm
/// free of that choice: the period arithmetic only needs the UTC offset that
/// applies at an instant.
pub trait KalenderZona: Copy {
    /// UTC offset that applies at `instan`.
    fn offset(&self, instan: DateTime<Utc>) -> FixedOffset;
}

/// A zone with a constant UTC offset.
///
/// It is the implementation the tests and the fixed-offset zones of this
/// deployment use (`Asia/Jakarta` +07:00, `Asia/Makassar` +08:00,
/// `Asia/Jayapura` +09:00, `UTC` +00:00). A zone with DST needs a resolver that
/// can answer for the instant, which a single offset cannot express; see
/// [`KalenderZona`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZonaTetap(FixedOffset);

impl ZonaTetap {
    /// Builds a constant-offset zone from a whole number of seconds.
    pub fn dari_detik(detik: i32) -> Option<Self> {
        FixedOffset::east_opt(detik).map(Self)
    }
}

impl KalenderZona for ZonaTetap {
    fn offset(&self, _instan: DateTime<Utc>) -> FixedOffset {
        self.0
    }
}

/// One entry of the reset history of an upstream pool.
///
/// `berlaku_mulai` is the `periode_selesai` of the period that was running when
/// the change was scheduled (KP-02), so the change never splits a period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AturanReset<Z: KalenderZona> {
    /// First instant this rule governs. Must be a period boundary.
    pub berlaku_mulai: DateTime<Utc>,
    /// Local wall-clock time of the daily reset.
    pub jam: NaiveTime,
    /// Zone the reset time is expressed in.
    pub zona: Z,
}

impl<Z: KalenderZona> AturanReset<Z> {
    pub fn baru(berlaku_mulai: DateTime<Utc>, jam: NaiveTime, zona: Z) -> Self {
        Self {
            berlaku_mulai,
            jam,
            zona,
        }
    }
}

/// One quota period: `[mulai, selesai)`, plus the local date it belongs to for
/// reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Periode {
    /// Start of the period; this is the value of `periode_mulai` in the log row
    /// and in the counter key.
    pub mulai: DateTime<Utc>,
    /// End of the period. Exclusive.
    pub selesai: DateTime<Utc>,
    /// Local calendar date of `mulai`, in the zone of the rule that governs the
    /// period. Used for reporting only; the contract keys on `periode_mulai`,
    /// not on `tanggal`.
    pub tanggal_lokal: NaiveDate,
}

/// Period that contains `requested_at`.
///
/// `riwayat` is ordered by `berlaku_mulai`, oldest first, and its first entry
/// is the start of the chain this call is allowed to know about. An instant
/// before that first entry is an error rather than a guess: the counter for it
/// may already exist under a period this process cannot name, and inventing one
/// would let two periods share a key (M-02).
pub fn periode_untuk<Z: KalenderZona>(
    requested_at: DateTime<Utc>,
    riwayat: &[AturanReset<Z>],
) -> Result<Periode, GalatKuota> {
    let (pertama, _) = riwayat.split_first().ok_or(GalatKuota::RiwayatKosong)?;

    for (indeks, pasangan) in riwayat.windows(2).enumerate() {
        if pasangan[1].berlaku_mulai <= pasangan[0].berlaku_mulai {
            return Err(GalatKuota::RiwayatTidakUrut { indeks: indeks + 1 });
        }
    }

    if requested_at < pertama.berlaku_mulai {
        return Err(GalatKuota::DiLuarRiwayat {
            requested_at,
            batas: pertama.berlaku_mulai,
        });
    }

    let mut mulai = pertama.berlaku_mulai;
    for _ in 0..BATAS_LANGKAH {
        let posisi = posisi_aturan(riwayat, mulai);
        let aturan = riwayat.get(posisi).ok_or(GalatKuota::TidakKonvergen)?;

        let batas_reset = reset_berikut(mulai, aturan)?;
        let selesai = match riwayat.get(posisi + 1) {
            // The change does not split the period that is running when it was
            // scheduled, so the period ends at the change even if the old
            // reset time has not arrived yet (KP-02).
            Some(berikut) => batas_reset.min(berikut.berlaku_mulai),
            None => batas_reset,
        };

        if selesai <= mulai {
            return Err(GalatKuota::TidakKonvergen);
        }

        if requested_at < selesai {
            let offset = aturan.zona.offset(mulai);
            return Ok(Periode {
                mulai,
                selesai,
                tanggal_lokal: mulai.with_timezone(&offset).date_naive(),
            });
        }

        mulai = selesai;
    }

    Err(GalatKuota::TidakKonvergen)
}

/// Index of the rule that governs a period starting at `mulai`: the last entry
/// whose `berlaku_mulai` is not later than `mulai`.
fn posisi_aturan<Z: KalenderZona>(riwayat: &[AturanReset<Z>], mulai: DateTime<Utc>) -> usize {
    riwayat
        .iter()
        .rposition(|aturan| aturan.berlaku_mulai <= mulai)
        .unwrap_or(0)
}

/// First occurrence of the rule's local reset time strictly after `setelah`.
fn reset_berikut<Z: KalenderZona>(
    setelah: DateTime<Utc>,
    aturan: &AturanReset<Z>,
) -> Result<DateTime<Utc>, GalatKuota> {
    let offset = aturan.zona.offset(setelah);
    let lokal = setelah.with_timezone(&offset).naive_local();
    let mut kandidat = lokal.date().and_time(aturan.jam);
    if kandidat <= lokal {
        let besok = lokal.date().succ_opt().ok_or(GalatKuota::TidakKonvergen)?;
        kandidat = besok.and_time(aturan.jam);
    }

    let mut hasil = ke_utc(kandidat, offset)?;
    // One refinement step: if the zone's offset differs at the candidate
    // instant, the first guess landed on a different wall-clock time. A zone
    // whose offset moves more than once per day cannot be expressed by
    // `KalenderZona`, so a single step is enough for every zone this trait
    // admits.
    let offset_kandidat = aturan.zona.offset(hasil);
    if offset_kandidat != offset {
        let ulang = ke_utc(kandidat, offset_kandidat)?;
        if ulang > setelah {
            hasil = ulang;
        }
    }

    if hasil <= setelah {
        return Err(GalatKuota::TidakKonvergen);
    }
    Ok(hasil)
}

fn ke_utc(lokal: NaiveDateTime, offset: FixedOffset) -> Result<DateTime<Utc>, GalatKuota> {
    offset
        .from_local_datetime(&lokal)
        .single()
        .map(|instan| instan.with_timezone(&Utc))
        .ok_or(GalatKuota::WaktuLokalTidakAda(lokal))
}

/// Start of the 10-minute bucket that contains `requested_at`, computed on the
/// UTC timeline so every process derives the same key from the same instant.
pub fn ember_mulai(requested_at: DateTime<Utc>) -> Result<DateTime<Utc>, GalatKuota> {
    let detik = requested_at.timestamp();
    let dasar = detik.div_euclid(DETIK_EMBER) * DETIK_EMBER;
    DateTime::from_timestamp(dasar, 0).ok_or(GalatKuota::TidakKonvergen)
}

/// Index of the bucket that contains `requested_at`, in `0..jumlah_ember`.
///
/// The index is `(epoch / 600) mod jumlah_ember`, so it keeps the same value
/// across restarts and needs no stored state to rebuild after a failover
/// (ADR-0001 rev 15 section 5a.2).
pub fn indeks_ember(requested_at: DateTime<Utc>, jumlah_ember: u32) -> Result<u32, GalatKuota> {
    if jumlah_ember == 0 {
        return Err(GalatKuota::JumlahEmberTidakSah(jumlah_ember));
    }
    let dasar = ember_mulai(requested_at)?;
    let nomor = dasar.timestamp().div_euclid(DETIK_EMBER);
    Ok(nomor.rem_euclid(i64::from(jumlah_ember)) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn utc(teks: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(teks)
            .expect("instan uji harus RFC 3339")
            .with_timezone(&Utc)
    }

    fn jam(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).expect("jam uji sah")
    }

    fn jakarta() -> ZonaTetap {
        ZonaTetap::dari_detik(7 * 3_600).expect("+07:00 sah")
    }

    fn makassar() -> ZonaTetap {
        ZonaTetap::dari_detik(8 * 3_600).expect("+08:00 sah")
    }

    /// Default configuration: daily reset at 00:00 Asia/Jakarta. The anchor is
    /// a boundary (00:00 local), which is what the controller writes.
    fn riwayat_bawaan() -> Vec<AturanReset<ZonaTetap>> {
        vec![AturanReset::baru(
            utc("2026-10-31T17:00:00Z"),
            jam(0, 0),
            jakarta(),
        )]
    }

    #[test]
    fn periode_harian_00_00_jakarta() {
        let riwayat = riwayat_bawaan();
        let periode = periode_untuk(utc("2026-11-03T03:00:00Z"), &riwayat).expect("periode ada");

        assert_eq!(periode.mulai, utc("2026-11-02T17:00:00Z"));
        assert_eq!(periode.selesai, utc("2026-11-03T17:00:00Z"));
        assert_eq!(
            periode.tanggal_lokal,
            NaiveDate::from_ymd_opt(2026, 11, 3).expect("tanggal sah")
        );
        assert_eq!(periode.selesai - periode.mulai, Duration::hours(24));
    }

    #[test]
    fn batas_periode_1_ms_sebelum_reset_masuk_periode_lama() {
        let riwayat = riwayat_bawaan();
        let sebelum =
            periode_untuk(utc("2026-11-03T16:59:59.999Z"), &riwayat).expect("periode ada");
        let tepat = periode_untuk(utc("2026-11-03T17:00:00Z"), &riwayat).expect("periode ada");

        assert_eq!(sebelum.mulai, utc("2026-11-02T17:00:00Z"));
        assert_eq!(sebelum.selesai, utc("2026-11-03T17:00:00Z"));
        assert_eq!(tepat.mulai, utc("2026-11-03T17:00:00Z"));
        // The boundary belongs to exactly one period: no gap, no overlap.
        assert_eq!(tepat.mulai, sebelum.selesai);
    }

    #[test]
    fn periode_bersambung_tanpa_tumpang_tindih_dan_mulai_unik() {
        let riwayat = riwayat_bawaan();
        let mut mulai = utc("2026-11-01T03:00:00Z");
        let mut terlihat: Vec<DateTime<Utc>> = Vec::new();

        for _ in 0..5 {
            let periode = periode_untuk(mulai, &riwayat).expect("periode ada");
            assert_eq!(periode.selesai - periode.mulai, Duration::hours(24));
            terlihat.push(periode.mulai);
            mulai = periode.selesai;
        }

        terlihat.dedup();
        assert_eq!(terlihat.len(), 5, "periode_mulai harus unik");
    }

    #[test]
    fn reset_maju_00_00_ke_17_00() {
        // The change was scheduled during the period of 4 Nov local, so it takes
        // effect at that period's end (5 Nov 00:00 local) and the next reset is
        // 17:00 local.
        let riwayat = vec![
            AturanReset::baru(utc("2026-11-02T17:00:00Z"), jam(0, 0), jakarta()),
            AturanReset::baru(utc("2026-11-04T17:00:00Z"), jam(17, 0), jakarta()),
        ];

        let transisi = periode_untuk(utc("2026-11-04T18:00:00Z"), &riwayat).expect("periode ada");
        assert_eq!(transisi.mulai, utc("2026-11-04T17:00:00Z"));
        assert_eq!(transisi.selesai, utc("2026-11-05T10:00:00Z"));
        assert!(transisi.selesai - transisi.mulai < Duration::hours(24));
        assert!(transisi.selesai - transisi.mulai > Duration::zero());

        let sesudah = periode_untuk(utc("2026-11-05T10:00:00Z"), &riwayat).expect("periode ada");
        assert_eq!(sesudah.mulai, utc("2026-11-05T10:00:00Z"));
        assert_eq!(sesudah.selesai, utc("2026-11-06T10:00:00Z"));
        assert_eq!(sesudah.selesai - sesudah.mulai, Duration::hours(24));
    }

    #[test]
    fn reset_mundur_17_00_ke_00_00() {
        let riwayat = vec![
            AturanReset::baru(utc("2026-11-02T10:00:00Z"), jam(17, 0), jakarta()),
            AturanReset::baru(utc("2026-11-04T10:00:00Z"), jam(0, 0), jakarta()),
        ];

        let transisi = periode_untuk(utc("2026-11-04T12:00:00Z"), &riwayat).expect("periode ada");
        assert_eq!(transisi.mulai, utc("2026-11-04T10:00:00Z"));
        assert_eq!(transisi.selesai, utc("2026-11-04T17:00:00Z"));
        assert!(transisi.selesai - transisi.mulai < Duration::hours(24));

        let sesudah = periode_untuk(utc("2026-11-04T17:00:00Z"), &riwayat).expect("periode ada");
        assert_eq!(sesudah.mulai, utc("2026-11-04T17:00:00Z"));
        assert_eq!(sesudah.selesai, utc("2026-11-05T17:00:00Z"));
    }

    #[test]
    fn perubahan_zona_jakarta_ke_makassar() {
        // WIB (+07:00) to WITA (+08:00): the reset stays at 00:00 local, which
        // moves one hour earlier on the UTC timeline. The period that carries
        // the change is 23 hours, never more than a day.
        let riwayat = vec![
            AturanReset::baru(utc("2026-11-02T17:00:00Z"), jam(0, 0), jakarta()),
            AturanReset::baru(utc("2026-11-04T17:00:00Z"), jam(0, 0), makassar()),
        ];

        let transisi = periode_untuk(utc("2026-11-04T18:00:00Z"), &riwayat).expect("periode ada");
        assert_eq!(transisi.mulai, utc("2026-11-04T17:00:00Z"));
        assert_eq!(transisi.selesai, utc("2026-11-05T16:00:00Z"));
        assert_eq!(transisi.selesai - transisi.mulai, Duration::hours(23));

        let sesudah = periode_untuk(utc("2026-11-05T16:00:00Z"), &riwayat).expect("periode ada");
        assert_eq!(sesudah.mulai, utc("2026-11-05T16:00:00Z"));
        assert_eq!(sesudah.selesai, utc("2026-11-06T16:00:00Z"));
        assert_eq!(sesudah.selesai - sesudah.mulai, Duration::hours(24));
    }

    #[test]
    fn periode_transisi_selalu_di_bawah_24_jam() {
        let riwayat = vec![
            AturanReset::baru(utc("2026-11-02T17:00:00Z"), jam(0, 0), jakarta()),
            AturanReset::baru(utc("2026-11-03T17:00:00Z"), jam(23, 59), jakarta()),
            AturanReset::baru(utc("2026-11-04T16:59:00Z"), jam(0, 0), makassar()),
        ];

        let mut mulai = utc("2026-11-02T17:00:00Z");
        for _ in 0..6 {
            let periode = periode_untuk(mulai, &riwayat).expect("periode ada");
            assert!(periode.selesai > periode.mulai);
            assert!(periode.selesai - periode.mulai <= Duration::hours(24));
            mulai = periode.selesai;
        }
    }

    #[test]
    fn riwayat_kosong_ditolak() {
        let kosong: Vec<AturanReset<ZonaTetap>> = Vec::new();
        let galat = periode_untuk(utc("2026-11-03T03:00:00Z"), &kosong).expect_err("harus galat");
        assert_eq!(galat, GalatKuota::RiwayatKosong);
    }

    #[test]
    fn riwayat_tidak_berurutan_ditolak() {
        let riwayat = vec![
            AturanReset::baru(utc("2026-11-04T17:00:00Z"), jam(0, 0), jakarta()),
            AturanReset::baru(utc("2026-11-02T17:00:00Z"), jam(17, 0), jakarta()),
        ];

        let galat = periode_untuk(utc("2026-11-05T03:00:00Z"), &riwayat).expect_err("harus galat");
        assert_eq!(galat, GalatKuota::RiwayatTidakUrut { indeks: 1 });
    }

    #[test]
    fn instan_sebelum_riwayat_ditolak() {
        let riwayat = riwayat_bawaan();
        let galat = periode_untuk(utc("2026-10-31T16:59:59Z"), &riwayat).expect_err("harus galat");
        assert_eq!(
            galat,
            GalatKuota::DiLuarRiwayat {
                requested_at: utc("2026-10-31T16:59:59Z"),
                batas: utc("2026-10-31T17:00:00Z"),
            }
        );
    }

    #[test]
    fn ember_adalah_potongan_10_menit_pada_garis_waktu_utc() {
        assert_eq!(
            ember_mulai(utc("2026-11-03T03:07:42.500Z")).expect("ember ada"),
            utc("2026-11-03T03:00:00Z")
        );
        assert_eq!(
            ember_mulai(utc("2026-11-03T03:09:59.999Z")).expect("ember ada"),
            utc("2026-11-03T03:00:00Z")
        );
        assert_eq!(
            ember_mulai(utc("2026-11-03T03:10:00Z")).expect("ember ada"),
            utc("2026-11-03T03:10:00Z")
        );
    }

    #[test]
    fn indeks_ember_di_dalam_rentang_dan_naik_per_ember() {
        let awal = utc("2026-11-03T03:00:00Z");
        let indeks_awal = indeks_ember(awal, JUMLAH_EMBER_HARIAN).expect("indeks ada");
        assert!(indeks_awal < JUMLAH_EMBER_HARIAN);

        let berikut = indeks_ember(awal + Duration::minutes(MENIT_EMBER), JUMLAH_EMBER_HARIAN)
            .expect("indeks ada");
        assert_eq!(berikut, (indeks_awal + 1) % JUMLAH_EMBER_HARIAN);

        // Every bucket of a well-formed day stays inside the range, including
        // the running one; a window of 145 keys covers 24 hours plus the bucket
        // in progress.
        for langkah in 0..JUMLAH_EMBER_HARIAN {
            let instan = awal + Duration::minutes(MENIT_EMBER * i64::from(langkah));
            let indeks = indeks_ember(instan, JUMLAH_EMBER_HARIAN).expect("indeks ada");
            assert!(indeks < JUMLAH_EMBER_HARIAN);
        }
    }

    #[test]
    fn indeks_ember_menolak_jumlah_nol() {
        let galat = indeks_ember(utc("2026-11-03T03:00:00Z"), 0).expect_err("harus galat");
        assert_eq!(galat, GalatKuota::JumlahEmberTidakSah(0));
    }
}
