//! Pemicu batch dan jadwal retry batch (AC-16, AC-17).
//!
//! [`evaluasi_pemicu`] memakai bentuk spec apa adanya:
//! `evaluasi_pemicu(jumlah_eligible, completed_at_tertua, sekarang,
//! &konfigurasi, periode_terakhir)` mengembalikan `JumlahN` bila jumlah ≥ N,
//! `Periode(batas)` bila sudah lewat jam kirim sejak batch periode terakhir
//! dan ada eligible yang `completed_at`-nya sebelum batas, selain itu `Tidak`.
//!
//! [`jeda_batch`] adalah full jitter yang memakai ulang
//! [`KebijakanBackoff`](crate::retry::KebijakanBackoff) dan
//! [`SumberAcakBackoff`](crate::retry::SumberAcakBackoff) milik retry webhook
//! (ARC-4): batch dan webhook tidak punya dua tabel angka yang bisa menyimpang.

use chrono::{DateTime, FixedOffset, NaiveDateTime, NaiveTime, TimeZone, Utc};

use super::konfigurasi::KonfigurasiDataBalikan;
use crate::kuota::periode::{KalenderZona, ZonaTetap};
use crate::retry::{KebijakanBackoff, SumberAcakBackoff};

/// Nilai yang dikembalikan evaluasi pemicu.
///
/// Nama varian dan nilai kolom `data_balikan_batch.pemicu` mengikuti migrasi
/// (`CHECK (pemicu IN ('jumlah_n', 'periode'))`) — jangan diubah tanpa
/// migrasi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PemicuBatch {
    /// `pemicu_jumlah_n` tercapai.
    JumlahN,
    /// Sudah lewat jam kirim; `batas` adalah batas waktu lokal yang dilewati.
    Periode(DateTime<Utc>),
    /// Belum waktunya.
    Tidak,
}

impl PemicuBatch {
    /// Nilai untuk kolom `data_balikan_batch.pemicu`. `Tidak` tidak punya
    /// nilai karena tidak pernah disimpan.
    pub fn sebagai_kolom(self) -> Option<&'static str> {
        match self {
            Self::JumlahN => Some("jumlah_n"),
            Self::Periode(_) => Some("periode"),
            Self::Tidak => None,
        }
    }
}

/// Menghitung zona dari IANA name yang dipakai deployment ini.
///
/// Sama seperti kuota: nama yang dikenal dipetakan ke offset tetap. Nama lain
/// ditolak, karena offset yang salah berarti jadwal kirim bergeser satu jam.
pub fn zona_dari_nama(nama: &str) -> Option<ZonaTetap> {
    let detik = match nama {
        "Asia/Jakarta" | "Asia/Pontianak" => 7 * 3600,
        "Asia/Makassar" => 8 * 3600,
        "Asia/Jayapura" => 9 * 3600,
        "UTC" => 0,
        _ => return None,
    };
    ZonaTetap::dari_detik(detik)
}

/// Batas waktu kirim terakhir yang sudah lampau: jam kirim pada tanggal lokal
/// hari ini, atau kemarin bila jam hari ini belum lewat.
///
/// Mengembalikan `None` bila zona tidak dikenal (fail-closed: tidak memutuskan
/// "sudah lampau").
fn batas_terakhir(
    konfigurasi: &KonfigurasiDataBalikan,
    sekarang: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let zona = zona_dari_nama(&konfigurasi.periode.zona)?;
    let offset = zona.offset(sekarang);
    let (j, m) = konfigurasi.periode.jam_kirim;
    let jam = NaiveTime::from_hms_opt(u32::from(j), u32::from(m), 0)?;
    let setempat = sekarang.with_timezone(&offset);
    let hari_ini = setempat.date_naive();
    let hari_ini_batas = lokal_ke_utc(hari_ini.and_time(jam), offset)?;
    if hari_ini_batas <= sekarang {
        Some(hari_ini_batas)
    } else {
        let kemarin = hari_ini.pred_opt()?;
        lokal_ke_utc(kemarin.and_time(jam), offset)
    }
}

fn lokal_ke_utc(lokal: NaiveDateTime, offset: FixedOffset) -> Option<DateTime<Utc>> {
    // Offset tetap tidak punya DST, jadi `single()` selalu ada.
    offset
        .from_local_datetime(&lokal)
        .single()
        .map(|d| d.with_timezone(&Utc))
}

/// Apakah batch harus disusun sekarang, dan atas dasar apa (AC-16).
///
/// - `jumlah_eligible`: jumlah item eligible yang menunggu.
/// - `completed_at_tertua`: `completed_at` item eligible paling tua, bila ada.
/// - `periode_terakhir`: waktu batch periode terakhir disusun, bila ada.
/// - `sekarang`: sumber waktu yang bisa diinjeksi (uji jam).
pub fn evaluasi_pemicu(
    jumlah_eligible: u32,
    completed_at_tertua: Option<DateTime<Utc>>,
    sekarang: DateTime<Utc>,
    konfigurasi: &KonfigurasiDataBalikan,
    periode_terakhir: Option<DateTime<Utc>>,
) -> PemicuBatch {
    if jumlah_eligible >= konfigurasi.pemicu_jumlah_n && jumlah_eligible > 0 {
        return PemicuBatch::JumlahN;
    }
    // Sisa item di bawah ambang boleh dikirim setelah jam kirim, tetapi hanya
    // item yang sudah eligible sebelum batas itu (spec AC-16). Tanpa item
    // seperti itu, tidak ada yang perlu dikirim.
    let Some(batas) = batas_terakhir(konfigurasi, sekarang) else {
        return PemicuBatch::Tidak;
    };
    let Some(tertua) = completed_at_tertua else {
        return PemicuBatch::Tidak;
    };
    if tertua >= batas {
        return PemicuBatch::Tidak;
    }
    // Satu batch periode per batas: bila batch terakhir sudah disusun pada
    // atau setelah batas ini, periode ini sudah dilayani.
    if let Some(terakhir) = periode_terakhir
        && terakhir >= batas
    {
        return PemicuBatch::Tidak;
    }
    PemicuBatch::Periode(batas)
}

/// Jeda sebelum batch gagal dicoba lagi: full jitter (AC-17).
///
/// `acak(0, min(cap, base × 2^(n−1)))` lewat [`SumberAcakBackoff`], dengan
/// `base`/`cap` dari [`KebijakanBackoff`] pihak pemanggil. `Retry-After` tidak
/// dipakai: itu sinyal client webhook, bukan provider batch (WB4).
pub fn jeda_batch(
    percobaan: u32,
    kebijakan: &KebijakanBackoff,
    acak: &dyn SumberAcakBackoff,
) -> chrono::Duration {
    let batas = kebijakan.batas_atas_ms(percobaan);
    chrono::Duration::milliseconds(acak.acak_hingga(batas) as i64)
}

/// Kebijakan retry batch dari konfigurasi, supaya T6 tidak merakit ulang angka
/// (AC-17 + AC-15).
pub fn kebijakan_batch(konfigurasi: &KonfigurasiDataBalikan) -> KebijakanBackoff {
    KebijakanBackoff {
        base_detik: konfigurasi.backoff_base_detik,
        cap_detik: konfigurasi.backoff_cap_detik,
        batas_percobaan: konfigurasi.kirim_batas_percobaan,
    }
}

#[cfg(test)]
mod uji {
    use super::*;
    use crate::retry::AcakBatasAtas;

    /// Acak yang selalu menarik setengah batas atas, untuk jeda yang pasti.
    struct AcakSetengah;
    impl SumberAcakBackoff for AcakSetengah {
        fn acak_hingga(&self, batas_ms: u64) -> u64 {
            batas_ms / 2
        }
    }

    fn konf(jumlah_n: u32) -> KonfigurasiDataBalikan {
        KonfigurasiDataBalikan {
            pemicu_jumlah_n: jumlah_n,
            ..KonfigurasiDataBalikan::default()
        }
    }

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn ac16_tabel_jumlah_n_min_satu_n_dan_plus_satu() {
        let k = konf(100);
        let sekarang = t("2026-11-05T10:00:00Z");
        // Lengkap dengan completed_at lama, supaya pemicu periode juga
        // memenuhi syarat; yang membedakan hanya jumlah.
        let tertua = Some(t("2026-11-05T00:00:00Z"));
        for (jumlah, harap) in [
            (99_u32, PemicuBatch::Tidak),
            (98, PemicuBatch::Tidak),
            (100, PemicuBatch::JumlahN),
            (101, PemicuBatch::JumlahN),
        ] {
            let hasil = evaluasi_pemicu(jumlah, tertua, sekarang, &k, None);
            assert_eq!(hasil, harap, "jumlah {jumlah}: {hasil:?}");
        }
    }

    #[test]
    fn ac16_periode_menyala_setelah_jam_kirim_dengan_item_lebih_tua() {
        let k = konf(100);
        // Batch periode terakhir disusun dua hari lalu.
        let terakhir = Some(t("2026-11-03T17:00:00Z"));
        // Sekarang 01.00 WIB 5 Nov (18.00 UTC 4 Nov). Batas terakhir adalah
        // 00.00 WIB 5 Nov = 17.00 UTC 4 Nov.
        let sekarang = t("2026-11-04T18:00:00Z");
        let tertua = Some(t("2026-11-04T16:00:00Z")); // sebelum batas
        assert_eq!(
            evaluasi_pemicu(10, tertua, sekarang, &k, terakhir),
            PemicuBatch::Periode(t("2026-11-04T17:00:00Z"))
        );
    }

    #[test]
    fn ac16_pergantian_hari_wib_menggeser_batas() {
        let k = konf(100);
        let terakhir = Some(t("2026-11-01T17:00:00Z"));
        // Item eligible sejak jauh sebelum kedua batas, supaya yang membedakan
        // hanya jam sekarang.
        let tertua = Some(t("2026-11-01T00:00:00Z"));
        // 16.59 UTC 4 Nov = 23.59 WIB 4 Nov: batas terakhir 00.00 WIB 4 Nov
        // = 17.00 UTC 3 Nov.
        assert_eq!(
            evaluasi_pemicu(1, tertua, t("2026-11-04T16:59:00Z"), &k, terakhir),
            PemicuBatch::Periode(t("2026-11-03T17:00:00Z"))
        );
        // 17.00 UTC 4 Nov = 00.00 WIB 5 Nov: batas bergeser ke hari ini.
        assert_eq!(
            evaluasi_pemicu(1, tertua, t("2026-11-04T17:00:00Z"), &k, terakhir),
            PemicuBatch::Periode(t("2026-11-04T17:00:00Z"))
        );
    }

    #[test]
    fn ac16_periode_tidak_menyala_bila_batch_terakhir_sudah_meliputi_batasnya() {
        let k = konf(100);
        let sekarang = t("2026-11-04T18:00:00Z");
        let tertua = Some(t("2026-11-04T16:00:00Z"));
        // Batch terakhir disusun tepat pada batas itu.
        assert_eq!(
            evaluasi_pemicu(10, tertua, sekarang, &k, Some(t("2026-11-04T17:00:00Z"))),
            PemicuBatch::Tidak
        );
    }

    #[test]
    fn ac16_periode_tidak_menyala_bila_semua_item_lebih_baru_dari_batas() {
        let k = konf(100);
        let sekarang = t("2026-11-04T18:00:00Z");
        // Item tertua pun lahir setelah batas (mis. barusan masuk).
        let tertua = Some(t("2026-11-04T17:30:00Z"));
        assert_eq!(
            evaluasi_pemicu(10, tertua, sekarang, &k, None),
            PemicuBatch::Tidak
        );
    }

    #[test]
    fn ac16_tanpa_item_selalu_tidak() {
        let k = konf(100);
        let sekarang = t("2026-11-05T10:00:00Z");
        assert_eq!(
            evaluasi_pemicu(0, None, sekarang, &k, None),
            PemicuBatch::Tidak
        );
    }

    #[test]
    fn ac17_tabel_batas_atas_sesuai_spec() {
        // Spec: n = 1..5 -> 300, 600, 1.200, 2.400, 3.600 detik.
        let k = KebijakanBackoff {
            base_detik: 300,
            cap_detik: 3_600,
            batas_percobaan: 5,
        };
        for (n, harap_detik) in [
            (1_u32, 300_i64),
            (2, 600),
            (3, 1_200),
            (4, 2_400),
            (5, 3_600),
        ] {
            assert_eq!(
                jeda_batch(n, &k, &AcakBatasAtas),
                chrono::Duration::seconds(harap_detik),
                "n = {n}"
            );
        }
    }

    #[test]
    fn ac17_full_jitter_di_bawah_batas_atas() {
        let k = KebijakanBackoff {
            base_detik: 300,
            cap_detik: 3_600,
            batas_percobaan: 5,
        };
        assert_eq!(
            jeda_batch(1, &k, &AcakSetengah),
            chrono::Duration::milliseconds(150_000)
        );
        for n in 1..=10 {
            let jeda = jeda_batch(n, &k, &AcakBatasAtas);
            assert!(jeda.num_milliseconds() <= 3_600_000, "n = {n}");
        }
    }

    #[test]
    fn kebijakan_batch_dibaca_dari_konfigurasi() {
        let k = KonfigurasiDataBalikan {
            backoff_base_detik: 120,
            backoff_cap_detik: 900,
            kirim_batas_percobaan: 7,
            ..KonfigurasiDataBalikan::default()
        };
        let kebijakan = kebijakan_batch(&k);
        assert_eq!(kebijakan.base_detik, 120);
        assert_eq!(kebijakan.cap_detik, 900);
        assert_eq!(kebijakan.batas_percobaan, 7);
    }

    #[test]
    fn ac16_zona_tidak_dikenal_fail_closed() {
        let k = KonfigurasiDataBalikan {
            periode: super::super::konfigurasi::Periode {
                jenis: "harian".to_owned(),
                jam_kirim: (0, 0),
                zona: "Europe/Berlin".to_owned(),
            },
            ..KonfigurasiDataBalikan::default()
        };
        assert_eq!(
            evaluasi_pemicu(
                1,
                Some(t("2020-01-01T00:00:00Z")),
                t("2026-11-05T10:00:00Z"),
                &k,
                None
            ),
            PemicuBatch::Tidak
        );
    }

    #[test]
    fn pemicu_menyimpan_nilai_kolom_yang_sesuai_check_database() {
        // Migrasi: CHECK (pemicu IN ('jumlah_n', 'periode')).
        assert_eq!(PemicuBatch::JumlahN.sebagai_kolom(), Some("jumlah_n"));
        assert_eq!(
            PemicuBatch::Periode(t("2026-11-05T00:00:00Z")).sebagai_kolom(),
            Some("periode")
        );
        assert_eq!(PemicuBatch::Tidak.sebagai_kolom(), None);
    }
}
