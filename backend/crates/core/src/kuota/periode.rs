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
pub const DETIK_EMBER: i64 = MENIT_EMBER * 60;

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

    // Every `berlaku_mulai` is a period boundary: a change is scheduled at the
    // end of the running period and the walk below ends a period at the next
    // `berlaku_mulai` at the latest (KP-02). So the period containing
    // `requested_at` can be found by starting at the newest rule that already
    // applies, instead of at the first rule of a chain that may be years old
    // (review 370c692, non-blocking 6). `periode_dari_awal` below is the
    // slow reference the tests compare against.
    let awal = riwayat
        .iter()
        .rposition(|aturan| aturan.berlaku_mulai <= requested_at)
        .map_or(pertama.berlaku_mulai, |i| riwayat[i].berlaku_mulai);
    telusuri(requested_at, riwayat, awal)
}

/// Walk periods forward from `mulai`, a known period boundary, to the one that
/// contains `requested_at`.
fn telusuri<Z: KalenderZona>(
    requested_at: DateTime<Utc>,
    riwayat: &[AturanReset<Z>],
    awal: DateTime<Utc>,
) -> Result<Periode, GalatKuota> {
    let mut mulai = awal;
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

/// Number of the 10-minute bucket that contains `requested_at`, counted from the
/// Unix epoch in bucket units (`epoch / 600`).
///
/// This is the `no_ember` the Redis scripts receive: the hash field is
/// `b{no_ember}` and the rolling window is the half-open range
/// `no_ember - (JUMLAH_EMBER_HARIAN - 1) ..= no_ember`, that is
/// `JUMLAH_EMBER_HARIAN` buckets counted in bucket units. The scripts derive
/// that range in the same unit (`potongan/bersama.lua`), so the bucket width
/// never appears in Lua and the window cannot silently collapse to
/// `JUMLAH_EMBER_HARIAN` seconds (SHAA-195 review, blocking finding 1).
///
/// Unlike [`indeks_ember`] this value is not reduced modulo anything: it is the
/// absolute bucket number, so two processes that see the same `requested_at`
/// derive the same field no matter how many buckets the ring has.
pub fn nomor_ember(requested_at: DateTime<Utc>) -> Result<i64, GalatKuota> {
    let detik = ember_mulai(requested_at)?.timestamp();
    Ok(detik.div_euclid(DETIK_EMBER))
}

/// Index of the bucket that contains `requested_at`, in `0..jumlah_ember`.
///
/// The index is `(epoch / 600) mod jumlah_ember`, so it keeps the same value
/// across restarts and needs no stored state to rebuild after a failover
/// (ADR-0001 rev 15 section 5a.2). It is for reporting and metrics; the Redis
/// hash field suffix is [`nomor_ember`], which is NOT reduced modulo, because
/// the window has to count `JUMLAH_EMBER_HARIAN` distinct buckets.
pub fn indeks_ember(requested_at: DateTime<Utc>, jumlah_ember: u32) -> Result<u32, GalatKuota> {
    if jumlah_ember == 0 {
        return Err(GalatKuota::JumlahEmberTidakSah(jumlah_ember));
    }
    let nomor = nomor_ember(requested_at)?;
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
    fn nomor_ember_dihitung_dalam_satuan_ember_bukan_detik() {
        // Nilai yang sama dengan `no_ember` di skrip Lua: `epoch / 600`.
        assert_eq!(
            nomor_ember(utc("1970-01-01T00:00:00Z")).expect("ember ada"),
            0
        );
        assert_eq!(
            nomor_ember(utc("1970-01-01T00:09:59.999Z")).expect("ember ada"),
            0
        );
        assert_eq!(
            nomor_ember(utc("1970-01-01T00:10:00Z")).expect("ember ada"),
            1
        );
        let satu_jam = nomor_ember(utc("1970-01-01T01:00:00Z")).expect("ember ada");
        assert_eq!(satu_jam, 6, "satu jam = enam ember, bukan 3600 detik");
        assert_eq!(
            nomor_ember(utc("1969-12-31T23:50:00Z")).expect("ember ada"),
            -1
        );
    }

    #[test]
    fn nomor_ember_tetap_untuk_seluruh_isi_ember() {
        let awal = nomor_ember(utc("2026-11-03T03:00:00Z")).expect("ember ada");
        for detik in [0, 1, 599] {
            let saat = utc("2026-11-03T03:00:00Z") + Duration::seconds(detik);
            assert_eq!(nomor_ember(saat).expect("ember ada"), awal, "detik {detik}");
        }
        let berikut = nomor_ember(utc("2026-11-03T03:10:00Z")).expect("ember ada");
        assert_eq!(berikut, awal + 1);
    }

    #[test]
    fn jendela_gulir_menutup_24_jam_penuh() {
        // Kontrak dengan `potongan/bersama.lua`: jendela adalah
        // `nomor_ember - (JUMLAH_EMBER_HARIAN - 1) ..= nomor_ember`, jadi batas
        // bawahnya tepat 24 jam sebelum awal ember berjalan. Kalau satuan di
        // sini bergeser ke detik, selisihnya jadi 145 detik dan uji ini gagal
        // (SHAA-195 review, temuan blocking 1).
        let saat = utc("2026-11-03T03:07:42.500Z");
        let nomor = nomor_ember(saat).expect("ember ada");
        let batas = nomor - (i64::from(JUMLAH_EMBER_HARIAN) - 1);
        let lebar_detik = (nomor - batas) * DETIK_EMBER;
        assert_eq!(
            i64::from(JUMLAH_EMBER_HARIAN - 1) * DETIK_EMBER,
            lebar_detik,
            "jendela harus {JUMLAH_EMBER_HARIAN} ember"
        );
        assert!(
            lebar_detik >= 24 * 3600,
            "jendela {lebar_detik} detik lebih pendek dari 24 jam"
        );
        let batas_bawah = ember_mulai(saat).expect("ember ada") - Duration::seconds(lebar_detik);
        assert_eq!(
            batas_bawah,
            utc("2026-11-02T03:00:00Z"),
            "batas bawah jendela harus tepat 24 jam sebelum awal ember berjalan"
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

    #[test]
    fn jendela_24_jam_selalu_144_ember_dan_ember_145_di_luar() {
        // Not derived from the production formula: the bucket numbers of two
        // instants exactly 24 h apart differ by 144, wherever they fall in the
        // bucket, so a window of `JUMLAH_EMBER_HARIAN` = 145 buckets always
        // holds the instant 24 h ago and never the one 24 h 10 min ago
        // (review 370c692, non-blocking 1).
        for detik in [
            0,
            1,
            599,
            600,
            601,
            3_599,
            86_399,
            1_793_000_123,
            1_793_000_599,
        ] {
            let t = DateTime::from_timestamp(1_700_000_000 + detik, 0).expect("instan sah");
            let sekarang = nomor_ember(t).expect("nomor");
            let sehari = nomor_ember(t - Duration::hours(24)).expect("nomor");
            let lebih = nomor_ember(t - Duration::hours(24) - Duration::minutes(MENIT_EMBER))
                .expect("nomor");
            assert_eq!(sekarang - sehari, 144, "t = {t}");
            assert_eq!(sekarang - lebih, 145, "t = {t}");
            let batas = sekarang - (i64::from(JUMLAH_EMBER_HARIAN) - 1);
            assert!(sehari >= batas, "24 jam lalu harus di dalam jendela");
            assert!(lebih < batas, "24 jam 10 menit lalu harus di luar jendela");
        }
    }

    use proptest::prelude::*;

    /// A random chain of reset rules. Every rule starts after the previous one,
    /// with random reset time and a fixed-offset zone.
    fn rantai() -> impl Strategy<Value = Vec<AturanReset<ZonaTetap>>> {
        let aturan = (
            1_i64..2_000,
            0_u32..24,
            prop::sample::select(vec![0_i32, 1, 2]),
            0_u32..2,
        );
        (
            1_700_000_000_i64..1_800_000_000,
            prop::collection::vec(aturan, 1..5),
        )
            .prop_map(|(awal, daftar)| {
                let mut t = awal;
                daftar
                    .into_iter()
                    .map(|(jarak_jam, jam_reset, zona, menit)| {
                        t += jarak_jam * 3_600;
                        let detik = [7, 8, 9][zona as usize] * 3_600;
                        AturanReset::baru(
                            DateTime::from_timestamp(t, 0).expect("instan sah"),
                            NaiveTime::from_hms_opt(jam_reset, menit * 30, 0).expect("jam sah"),
                            ZonaTetap::dari_detik(detik).expect("offset sah"),
                        )
                    })
                    .collect()
            })
    }

    proptest! {
        // The period chain is gap-free, non-overlapping, at most a day long,
        // contains the instant asked for, and the jump to the newest rule gives
        // exactly what the walk from the first rule gives (blocking-free
        // non-blocking 6 of review 370c692).
        #[test]
        fn rantai_periode_bersambung_tanpa_tumpang_tindih(
            riwayat in rantai(),
            selisih in 0_i64..(400 * 86_400),
        ) {
            let pertama = riwayat[0].berlaku_mulai;
            let t = pertama + Duration::seconds(selisih);

            let p = periode_untuk(t, &riwayat).expect("periode ada");
            prop_assert!(p.mulai <= t && t < p.selesai);
            prop_assert!(p.selesai > p.mulai);
            prop_assert!(p.selesai - p.mulai <= Duration::hours(24));

            // Same answer as the slow reference.
            let acuan = telusuri(t, &riwayat, pertama).expect("acuan ada");
            prop_assert_eq!(p, acuan);

            // No gap and no overlap with the next period...
            let berikut = periode_untuk(p.selesai, &riwayat).expect("berikut ada");
            prop_assert_eq!(berikut.mulai, p.selesai);
            // ...nor with the previous one.
            if p.mulai > pertama {
                let sebelum = periode_untuk(p.mulai - Duration::seconds(1), &riwayat)
                    .expect("sebelum ada");
                prop_assert_eq!(sebelum.selesai, p.mulai);
            }
            // Every rule start is a period start.
            for aturan in &riwayat {
                if aturan.berlaku_mulai <= t {
                    let di_batas = periode_untuk(aturan.berlaku_mulai, &riwayat)
                        .expect("batas ada");
                    prop_assert_eq!(di_batas.mulai, aturan.berlaku_mulai);
                }
            }
        }
    }
}
