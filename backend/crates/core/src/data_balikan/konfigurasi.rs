//! Konfigurasi data balikan pada `config_schema_version` 2 (AC-15).
//!
//! Nama field, default, dan aturan penolakan mengikuti spec
//! `f1-webhook-retry-data-balikan` baris "Konfigurasi data balikan" dan AC-15
//! apa adanya: `pemicu_jumlah_n`, `periode`, `toleransi_detik`,
//! `ukuran_batch_maks`, `kirim_batas_percobaan`, `backoff_base_detik`,
//! `backoff_cap_detik`, `endpoint`, `rujukan_secret`, `format_version`.
//!
//! Dokumen versi 1 tetap valid dan tidak mengenal blok ini; pembacaan blok
//! hanya untuk versi 2.

use serde::Deserialize;

/// Default `pemicu_jumlah_n` (WB11).
pub const PEMICU_JUMLAH_N_DEFAULT: u32 = 10_000;
/// Default `toleransi_detik` (3.600 s = 1 jam, WB11).
pub const TOLERANSI_DETIK_DEFAULT: u32 = 3_600;
/// Default `ukuran_batch_maks` (WB11).
pub const UKURAN_BATCH_MAKS_DEFAULT: u32 = 10_000;
/// Default `kirim_batas_percobaan` (spec, AC-30 memakai angka yang sama).
pub const KIRIM_BATAS_PERCOBAAN_DEFAULT: u32 = 5;
/// Default `backoff_base_detik`.
pub const BACKOFF_BASE_DETIK_DEFAULT: u32 = 300;
/// Default `backoff_cap_detik`.
pub const BACKOFF_CAP_DETIK_DEFAULT: u32 = 3_600;
/// Default `format_version` untuk adapter tiruan (WB11).
pub const FORMAT_VERSION_DEFAULT: &str = "tiruan-1";

/// Batas atas `ukuran_batch_maks` (AC-15: 1–10.000).
const UKURAN_BATCH_MAKS_BATAS: u32 = 10_000;
/// Batas atas `kirim_batas_percobaan` (AC-15: 1–10).
const KIRIM_BATAS_PERCOBAAN_BATAS: u32 = 10;

/// Galat pembacaan blok konfigurasi.
///
/// Pesannya menyebut nama field, tidak pernah nilainya: `endpoint` dan
/// `rujukan_secret` tidak boleh bocor ke log (D6 "log tanpa rahasia").
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GalatDataBalikan {
    /// Dokumen versi 1 tidak mengenal blok `data_balikan`.
    #[error("config_schema_version {0} tidak mengenal blok data_balikan; versi 2 diperlukan")]
    SkemaTerlaluTua(u32),
    /// Dokumen versi 3 ke atas ditulis untuk pembaca yang belum ada.
    #[error("config_schema_version {0} belum dikenal pembaca ini; versi terbaru 2")]
    SkemaTerlaluBaru(u32),
    #[error("field `data_balikan.{0}` tidak sah: {1}")]
    TidakSah(&'static str, &'static str),
}

/// Periode harian pada waktu lokal zona [`Periode::zona`].
///
/// Spec mengunci `harian` 00.00 `Asia/Jakarta`; tipe ini menyimpan jam dan
/// zona saat ini sebagai data supaya perubahan tidak perlu mengubah tipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Periode {
    /// Selalu `harian` pada skema 2. Disimpan supaya pembaca berikutnya
    /// melihat nilai yang benar-benar dikonfigurasi, bukan asumsi.
    pub jenis: String,
    pub jam_kirim: (u8, u8),
    pub zona: String,
}

impl Periode {
    /// `true` bila konfigurasi memang memakai periode harian, satu-satunya
    /// bentuk yang dikenal skema 2.
    pub fn harian(&self) -> bool {
        self.jenis == "harian"
    }
}

/// Blok `data_balikan` pada dokumen konfigurasi provider.
///
/// `endpoint` dan `rujukan_secret` disimpan apa adanya. `rujukan_secret`
/// berbentuk `{{secret.X}}`; nilainya tidak pernah ada di sini (secret
/// diselesaikan worker, AC-21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KonfigurasiDataBalikan {
    pub pemicu_jumlah_n: u32,
    pub periode: Periode,
    pub toleransi_detik: u32,
    pub ukuran_batch_maks: u32,
    pub kirim_batas_percobaan: u32,
    pub backoff_base_detik: u32,
    pub backoff_cap_detik: u32,
    pub endpoint: String,
    pub rujukan_secret: String,
    pub format_version: String,
}

impl Default for KonfigurasiDataBalikan {
    /// Default WB11 untuk semua field yang punya default, ditambah dua field
    /// wajib yang sengaja **kosong**: `endpoint` dan `rujukan_secret` tidak
    /// punya default (spec hanya menyebut bentuknya), sehingga
    /// [`KonfigurasiDataBalikan::parse`] menolak dokumen yang tidak mengisinya.
    fn default() -> Self {
        Self {
            pemicu_jumlah_n: PEMICU_JUMLAH_N_DEFAULT,
            periode: Periode {
                jenis: "harian".to_owned(),
                jam_kirim: (0, 0),
                zona: "Asia/Jakarta".to_owned(),
            },
            toleransi_detik: TOLERANSI_DETIK_DEFAULT,
            ukuran_batch_maks: UKURAN_BATCH_MAKS_DEFAULT,
            kirim_batas_percobaan: KIRIM_BATAS_PERCOBAAN_DEFAULT,
            backoff_base_detik: BACKOFF_BASE_DETIK_DEFAULT,
            backoff_cap_detik: BACKOFF_CAP_DETIK_DEFAULT,
            endpoint: String::new(),
            rujukan_secret: String::new(),
            format_version: FORMAT_VERSION_DEFAULT.to_owned(),
        }
    }
}

/// Bentuk mentah dari dokumen konfigurasi (`admin-api.yaml` skema 2).
///
/// Field yang tidak dikenal ditolak, bukan diabaikan: ini konfigurasi egress
/// data pribadi, salah ketik nama field harus terlihat.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mentah {
    pemicu_jumlah_n: Option<u32>,
    periode: Option<PeriodeMentah>,
    toleransi_detik: Option<u32>,
    ukuran_batch_maks: Option<u32>,
    kirim_batas_percobaan: Option<u32>,
    backoff_base_detik: Option<u32>,
    backoff_cap_detik: Option<u32>,
    endpoint: Option<String>,
    rujukan_secret: Option<String>,
    format_version: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PeriodeMentah {
    jenis: Option<String>,
    jam: Option<String>,
    zona: Option<String>,
}

impl KonfigurasiDataBalikan {
    /// Membaca blok `data_balikan` dari dokumen konfigurasi versi 2.
    ///
    /// `versi` adalah `config_schema_version` dokumen. Versi 1 ditolak karena
    /// belum mengenal blok ini; versi 3 ke atas ditolak karena ada pembaca yang
    /// belum diperbarui.
    pub fn parse(versi: u32, nilai: &serde_json::Value) -> Result<Self, GalatDataBalikan> {
        match versi {
            2 => {}
            1 => return Err(GalatDataBalikan::SkemaTerlaluTua(1)),
            lain => return Err(GalatDataBalikan::SkemaTerlaluBaru(lain)),
        }
        let mentah: Mentah = serde_json::from_value(nilai.clone())
            .map_err(|_| GalatDataBalikan::TidakSah("data_balikan", "bentuk tidak dikenal"))?;
        let dasar = Self::default();

        // Urutan penting: `ukuran_batch_maks` harus sah sebelum dipakai
        // memeriksa `pemicu_jumlah_n` (AC-15 membandingkannya).
        let ukuran_batch_maks = mentah.ukuran_batch_maks.unwrap_or(dasar.ukuran_batch_maks);
        if !(1..=UKURAN_BATCH_MAKS_BATAS).contains(&ukuran_batch_maks) {
            return Err(GalatDataBalikan::TidakSah(
                "ukuran_batch_maks",
                "harus antara 1 dan 10000",
            ));
        }
        let pemicu_jumlah_n = mentah.pemicu_jumlah_n.unwrap_or(dasar.pemicu_jumlah_n);
        if pemicu_jumlah_n < 1 || pemicu_jumlah_n > ukuran_batch_maks {
            return Err(GalatDataBalikan::TidakSah(
                "pemicu_jumlah_n",
                "harus antara 1 dan ukuran_batch_maks",
            ));
        }
        let kirim_batas_percobaan = mentah
            .kirim_batas_percobaan
            .unwrap_or(dasar.kirim_batas_percobaan);
        if !(1..=KIRIM_BATAS_PERCOBAAN_BATAS).contains(&kirim_batas_percobaan) {
            return Err(GalatDataBalikan::TidakSah(
                "kirim_batas_percobaan",
                "harus antara 1 dan 10",
            ));
        }
        let backoff_base_detik = mentah
            .backoff_base_detik
            .unwrap_or(dasar.backoff_base_detik);
        let backoff_cap_detik = mentah.backoff_cap_detik.unwrap_or(dasar.backoff_cap_detik);
        if backoff_base_detik > backoff_cap_detik {
            return Err(GalatDataBalikan::TidakSah(
                "backoff_base_detik",
                "tidak boleh melebihi backoff_cap_detik",
            ));
        }
        let toleransi_detik = mentah.toleransi_detik.unwrap_or(dasar.toleransi_detik);
        // `endpoint` dan `rujukan_secret` tidak punya default WB11, dan AC-15
        // tidak memberi nilai kosong sebagai pengecualian: "" bukan https dan
        // bukan `{{secret.X}}`. Blok yang belum diisi ditolak, bukan diterima
        // setengah jadi.
        let endpoint = mentah.endpoint.unwrap_or(dasar.endpoint);
        if !endpoint.starts_with("https://") {
            return Err(GalatDataBalikan::TidakSah("endpoint", "harus https://"));
        }
        let rujukan_secret = mentah.rujukan_secret.unwrap_or(dasar.rujukan_secret);
        if !bentuk_rujukan_secret(&rujukan_secret) {
            return Err(GalatDataBalikan::TidakSah(
                "rujukan_secret",
                "harus berbentuk {{secret.X}}",
            ));
        }
        let format_version = mentah.format_version.unwrap_or(dasar.format_version);
        if format_version.trim().is_empty() {
            return Err(GalatDataBalikan::TidakSah(
                "format_version",
                "tidak boleh kosong",
            ));
        }
        let periode = match mentah.periode {
            Some(p) => Self::baca_periode(p, &dasar.periode)?,
            None => dasar.periode,
        };
        Ok(Self {
            pemicu_jumlah_n,
            periode,
            toleransi_detik,
            ukuran_batch_maks,
            kirim_batas_percobaan,
            backoff_base_detik,
            backoff_cap_detik,
            endpoint,
            rujukan_secret,
            format_version,
        })
    }

    fn baca_periode(p: PeriodeMentah, dasar: &Periode) -> Result<Periode, GalatDataBalikan> {
        let jenis = p.jenis.unwrap_or_else(|| dasar.jenis.clone());
        if jenis != "harian" {
            return Err(GalatDataBalikan::TidakSah("periode.jenis", "harus harian"));
        }
        let jam_kirim = match p.jam {
            Some(ref s) => baca_jam(s)?,
            None => dasar.jam_kirim,
        };
        if jam_kirim != (0, 0) {
            return Err(GalatDataBalikan::TidakSah("periode.jam", "harus 00:00"));
        }
        let zona = p.zona.unwrap_or_else(|| dasar.zona.clone());
        if zona != "Asia/Jakarta" {
            return Err(GalatDataBalikan::TidakSah(
                "periode.zona",
                "harus Asia/Jakarta",
            ));
        }
        Ok(Periode {
            jenis,
            jam_kirim,
            zona,
        })
    }
}

/// `{{secret.X}}` dengan `X` tidak kosong (AC-15, pola V-05).
fn bentuk_rujukan_secret(s: &str) -> bool {
    let Some(dalam) = s
        .strip_prefix("{{secret.")
        .and_then(|r| r.strip_suffix("}}"))
    else {
        return false;
    };
    !dalam.trim().is_empty()
}

fn baca_jam(s: &str) -> Result<(u8, u8), GalatDataBalikan> {
    let (j, m) = s
        .split_once(':')
        .ok_or(GalatDataBalikan::TidakSah("periode.jam", "harus HH:MM"))?;
    if j.len() != 2 || m.len() != 2 {
        return Err(GalatDataBalikan::TidakSah("periode.jam", "harus HH:MM"));
    }
    let j: u8 = j
        .parse()
        .map_err(|_| GalatDataBalikan::TidakSah("periode.jam", "harus HH:MM"))?;
    let m: u8 = m
        .parse()
        .map_err(|_| GalatDataBalikan::TidakSah("periode.jam", "harus HH:MM"))?;
    if j > 23 || m > 59 {
        return Err(GalatDataBalikan::TidakSah(
            "periode.jam",
            "jam 0-23 dan menit 0-59",
        ));
    }
    Ok((j, m))
}

#[cfg(test)]
mod uji {
    use super::*;
    use serde_json::json;

    /// Field wajib yang tidak punya default WB11; ditambahkan ke setiap kasus
    /// supaya uji per-field tidak ikut gagal karena field lain kosong.
    fn dengan_wajib(v: serde_json::Value) -> serde_json::Value {
        let mut obj = match v {
            serde_json::Value::Object(m) => m,
            lain => panic!("kasus uji harus objek, bukan {lain:?}"),
        };
        obj.entry("endpoint")
            .or_insert_with(|| json!("https://contoh.test/balikan"));
        obj.entry("rujukan_secret")
            .or_insert_with(|| json!("{{secret.X}}"));
        serde_json::Value::Object(obj)
    }

    fn parse(v: serde_json::Value) -> Result<KonfigurasiDataBalikan, GalatDataBalikan> {
        KonfigurasiDataBalikan::parse(2, &dengan_wajib(v))
    }

    /// Tanpa penambahan field wajib: untuk uji yang sengaja mengosongkannya.
    fn parse_apa_adanya(v: serde_json::Value) -> Result<KonfigurasiDataBalikan, GalatDataBalikan> {
        KonfigurasiDataBalikan::parse(2, &v)
    }

    fn lengkap() -> serde_json::Value {
        json!({
            "pemicu_jumlah_n": 5000,
            "periode": { "jenis": "harian", "jam": "00:00", "zona": "Asia/Jakarta" },
            "toleransi_detik": 3600,
            "ukuran_batch_maks": 6000,
            "kirim_batas_percobaan": 5,
            "backoff_base_detik": 300,
            "backoff_cap_detik": 3600,
            "endpoint": "https://contoh.test/balikan",
            "rujukan_secret": "{{secret.DUKCAPIL_BALIKAN}}",
            "format_version": "tiruan-1"
        })
    }

    #[test]
    fn blok_lengkap_terbaca() {
        let k = parse(lengkap()).unwrap();
        assert_eq!(k.pemicu_jumlah_n, 5000);
        assert!(k.periode.harian());
        assert_eq!(k.periode.jam_kirim, (0, 0));
        assert_eq!(k.periode.zona, "Asia/Jakarta");
        assert_eq!(k.toleransi_detik, 3600);
        assert_eq!(k.ukuran_batch_maks, 6000);
        assert_eq!(k.kirim_batas_percobaan, 5);
        assert_eq!(k.backoff_base_detik, 300);
        assert_eq!(k.backoff_cap_detik, 3600);
        assert_eq!(k.endpoint, "https://contoh.test/balikan");
        assert_eq!(k.rujukan_secret, "{{secret.DUKCAPIL_BALIKAN}}");
        assert_eq!(k.format_version, "tiruan-1");
    }

    #[test]
    fn default_dipakai_untuk_field_yang_tidak_ada() {
        // `endpoint`/`rujukan_secret` wajib, jadi hanya keduanya yang diisi.
        let k = parse(json!({
            "endpoint": "https://contoh.test/balikan",
            "rujukan_secret": "{{secret.X}}"
        }))
        .unwrap();
        assert_eq!(k.pemicu_jumlah_n, PEMICU_JUMLAH_N_DEFAULT);
        assert_eq!(k.toleransi_detik, TOLERANSI_DETIK_DEFAULT);
        assert_eq!(k.ukuran_batch_maks, UKURAN_BATCH_MAKS_DEFAULT);
        assert_eq!(k.kirim_batas_percobaan, KIRIM_BATAS_PERCOBAAN_DEFAULT);
        assert_eq!(k.backoff_base_detik, BACKOFF_BASE_DETIK_DEFAULT);
        assert_eq!(k.backoff_cap_detik, BACKOFF_CAP_DETIK_DEFAULT);
        assert_eq!(k.format_version, FORMAT_VERSION_DEFAULT);
        assert_eq!(k.periode.jam_kirim, (0, 0));
        assert_eq!(k.periode.zona, "Asia/Jakarta");
        assert!(k.periode.harian());
    }

    #[test]
    fn versi_1_terlalu_tua_dan_versi_3_terlalu_baru() {
        assert_eq!(
            KonfigurasiDataBalikan::parse(1, &json!({})),
            Err(GalatDataBalikan::SkemaTerlaluTua(1))
        );
        assert_eq!(
            KonfigurasiDataBalikan::parse(3, &json!({})),
            Err(GalatDataBalikan::SkemaTerlaluBaru(3))
        );
    }

    #[test]
    fn ac15_pemicu_jumlah_n_di_luar_ukuran_batch_ditolak() {
        // AC-15: pemicu_jumlah_n < 1 atau > ukuran_batch_maks.
        assert!(parse(json!({ "pemicu_jumlah_n": 0 })).is_err());
        assert!(parse(json!({ "pemicu_jumlah_n": 101, "ukuran_batch_maks": 100 })).is_err());
        assert!(parse(json!({ "pemicu_jumlah_n": 100, "ukuran_batch_maks": 100 })).is_ok());
    }

    #[test]
    fn ac15_ukuran_batch_maks_di_luar_1_sampai_10000_ditolak() {
        assert!(parse(json!({ "ukuran_batch_maks": 0 })).is_err());
        assert!(parse(json!({ "ukuran_batch_maks": 10_001 })).is_err());
        assert!(parse(json!({ "ukuran_batch_maks": 10_000 })).is_ok());
    }

    #[test]
    fn ac15_kirim_batas_percobaan_di_luar_1_sampai_10_ditolak() {
        assert!(parse(json!({ "kirim_batas_percobaan": 0 })).is_err());
        assert!(parse(json!({ "kirim_batas_percobaan": 11 })).is_err());
        assert!(parse(json!({ "kirim_batas_percobaan": 10 })).is_ok());
    }

    #[test]
    fn ac15_backoff_base_melebihi_cap_ditolak() {
        assert!(parse(json!({ "backoff_base_detik": 3601, "backoff_cap_detik": 3600 })).is_err());
        assert!(parse(json!({ "backoff_base_detik": 3600, "backoff_cap_detik": 3600 })).is_ok());
    }

    #[test]
    fn ac15_endpoint_bukan_https_ditolak() {
        assert!(parse(json!({ "endpoint": "http://contoh.test" })).is_err());
        assert!(parse(json!({ "endpoint": "contoh.test" })).is_err());
        assert!(parse(json!({ "endpoint": "https://contoh.test" })).is_ok());
    }

    #[test]
    fn ac15_rujukan_secret_bukan_bentuk_secret_ditolak() {
        assert!(parse(json!({ "rujukan_secret": "DUKCAPIL_BALIKAN" })).is_err());
        assert!(parse(json!({ "rujukan_secret": "{{secret.}}" })).is_err());
        assert!(parse(json!({ "rujukan_secret": "${DUKCAPIL_BALIKAN}" })).is_err());
        assert!(parse(json!({ "rujukan_secret": "{{secret.X}}" })).is_ok());
    }

    #[test]
    fn field_tidak_dikenal_ditolak() {
        assert!(parse(json!({ "jumlah_n": 10 })).is_err());
        assert!(parse(json!({ "toleransi_jadwal_jam": 1 })).is_err());
    }

    #[test]
    fn periode_selain_harian_dan_jam_tidak_sah_ditolak() {
        assert!(parse(json!({ "periode": { "jenis": "mingguan" } })).is_err());
        assert!(parse(json!({ "periode": { "jam": "24:00" } })).is_err());
        assert!(parse(json!({ "periode": { "jam": "00:60" } })).is_err());
        assert!(parse(json!({ "periode": { "jam": "0:00" } })).is_err());
        assert!(parse(json!({ "periode": { "zona": "  " } })).is_err());
        assert!(parse(json!({ "periode": { "jam": "07:30" } })).is_err());
        assert!(parse(json!({ "periode": { "zona": "Asia/Jakarta" } })).is_ok());
    }

    #[test]
    fn format_version_kosong_ditolak() {
        let mut v = lengkap();
        v["format_version"] = json!("");
        assert!(parse_apa_adanya(v).is_err());
    }

    #[test]
    fn ac15_endpoint_dan_rujukan_secret_wajib_terisi() {
        // AC-15 tidak memberi pengecualian nilai kosong: "" bukan https dan
        // bukan `{{secret.X}}`.
        assert!(parse_apa_adanya(json!({})).is_err());
        assert!(
            parse_apa_adanya(json!({
                "endpoint": "https://contoh.test",
                "rujukan_secret": ""
            }))
            .is_err()
        );
        assert!(
            parse_apa_adanya(json!({
                "endpoint": "",
                "rujukan_secret": "{{secret.X}}"
            }))
            .is_err()
        );
    }

    #[test]
    fn galat_menyebut_nama_field_bukan_nilai() {
        let e = parse(json!({
            "endpoint": "http://rahasia.contoh.test/token"
        }))
        .unwrap_err();
        let pesan = e.to_string();
        assert!(pesan.contains("endpoint"));
        // Nilai endpoint tidak boleh muncul di pesan galat.
        assert!(!pesan.contains("rahasia"), "{pesan}");
    }
}
