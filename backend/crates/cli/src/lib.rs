//! `backend-cli`: pengelolaan pengguna dan peran admin F0 lewat baris perintah (ADR-0003 rev 6
//! §3.6c, spec f0-admin-api AC-39–AC-42, ARC-55).
//!
//! Crate ini hanya mengurai argumen, membaca password, dan mencetak hasil. Aturan A-02, audit, dan
//! pencabutan sesi ada di `poolrouter_core::admin::kelola_pengguna`, layanan yang sama yang akan
//! dipakai handler admin API pengguna/peran di F1.
//!
//! Password tidak pernah lewat argumen (AC-40): opsi `--password` tidak dikenal, dan nilai opsi yang
//! tidak dikenal tidak pernah dicetak balik di pesan galat.

use std::io::{self, BufRead, IsTerminal, Write};
use std::process::{Command, Stdio};

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use poolrouter_core::admin::kelola_pengguna::{self as kelola, GalatKelola, PemberianPeran};
use poolrouter_core::audit::{Alasan, AlasanTidakValid, Peran};
use poolrouter_core::config::Environment;
use secrecy::{ExposeSecret, SecretString};
use sqlx::PgPool;
use uuid::Uuid;
use zeroize::Zeroizing;

/// Teks bantuan. Nama perintah final ditetapkan di sini dan didokumentasikan di `backend/README.md`.
pub const BANTUAN: &str = "\
pemakaian: backend-cli <perintah> [opsi]

perintah:
  bootstrap --username <u> --alasan <teks>
      platform admin pertama; hanya saat tabel pengguna kosong
  pengguna buat --username <u> --oleh <admin> --alasan <teks> [--client <uuid>]
  pengguna nonaktifkan --username <u> --oleh <admin> --alasan <teks>
  pengguna aktifkan --username <u> --oleh <admin> --alasan <teks>
  peran beri --username <u> --peran <peran> --oleh <admin> --alasan <teks>
             [--disetujui-oleh <admin lain>] [--berlaku-sampai <YYYY-MM-DD|RFC 3339>]
  peran cabut --username <u> --peran <peran> --oleh <admin> --alasan <teks>
  daftar
  seed pengguna        hanya POOLROUTER_ENV dev/ci, password dari POOLROUTER_SEED_PASSWORD_FILE

peran: platform_admin, client_owner, operator, auditor
password dibaca dari TTY tanpa echo atau dari stdin; tidak pernah dari argumen.";

/// Satu perintah hasil urai argumen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Perintah {
    Bantuan,
    Bootstrap {
        username: String,
        alasan: Alasan,
    },
    PenggunaBuat {
        username: String,
        client: Option<Uuid>,
        oleh: String,
        alasan: Alasan,
    },
    PenggunaUbahAktif {
        username: String,
        aktif: bool,
        oleh: String,
        alasan: Alasan,
    },
    PeranBeri {
        username: String,
        peran: Peran,
        oleh: String,
        alasan: Alasan,
        disetujui_oleh: Option<String>,
        berlaku_sampai: Option<DateTime<Utc>>,
    },
    PeranCabut {
        username: String,
        peran: Peran,
        oleh: String,
        alasan: Alasan,
    },
    Daftar,
    SeedPengguna,
}

/// Argumen yang tidak bisa diurai. Pesan memuat nama opsi saja, tidak pernah nilainya.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GalatArg {
    #[error("perintah tidak dikenal; lihat `backend-cli bantuan`")]
    PerintahTidakDikenal,
    #[error("opsi tidak dikenal: {0}")]
    OpsiTidakDikenal(String),
    #[error("opsi {0} butuh nilai")]
    NilaiHilang(String),
    #[error("opsi {0} diberikan lebih dari sekali")]
    OpsiGanda(String),
    #[error("opsi {0} wajib")]
    OpsiWajib(&'static str),
    #[error("argumen tidak dikenal (opsi ditulis `--nama nilai`)")]
    ArgumenLepas,
    #[error("peran tidak dikenal; pilih platform_admin, client_owner, operator, atau auditor")]
    PeranTidakDikenal,
    #[error("--client harus UUID")]
    ClientTidakValid,
    #[error("--berlaku-sampai harus YYYY-MM-DD atau RFC 3339")]
    TanggalTidakValid,
    #[error("--alasan: {0}")]
    Alasan(AlasanTidakValid),
}

struct Opsi {
    pasangan: Vec<(String, String)>,
}

impl Opsi {
    /// Urai `--nama nilai` dengan daftar nama yang diizinkan. Bentuk `--nama=nilai` sengaja tidak
    /// didukung, dan opsi yang tidak dikenal dilaporkan tanpa nilainya (mis. `--password=...`).
    fn urai(args: &[String], dikenal: &[&str]) -> Result<Self, GalatArg> {
        let mut pasangan: Vec<(String, String)> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            let Some(nama) = a.strip_prefix("--") else {
                return Err(GalatArg::ArgumenLepas);
            };
            let nama = nama.split('=').next().unwrap_or_default();
            let opsi = format!("--{nama}");
            if a.contains('=') || !dikenal.contains(&nama) {
                return Err(GalatArg::OpsiTidakDikenal(opsi));
            }
            if pasangan.iter().any(|(n, _)| n == nama) {
                return Err(GalatArg::OpsiGanda(opsi));
            }
            let nilai = args.get(i + 1).ok_or(GalatArg::NilaiHilang(opsi))?;
            pasangan.push((nama.to_owned(), nilai.clone()));
            i += 2;
        }
        Ok(Self { pasangan })
    }

    fn ambil(&self, nama: &str) -> Option<String> {
        self.pasangan
            .iter()
            .find(|(n, _)| n == nama)
            .map(|(_, v)| v.clone())
    }

    fn wajib(&self, nama: &'static str) -> Result<String, GalatArg> {
        self.ambil(nama)
            .filter(|v| !v.trim().is_empty())
            .ok_or(GalatArg::OpsiWajib(nama))
    }

    fn alasan(&self) -> Result<Alasan, GalatArg> {
        Alasan::baru(self.wajib("alasan")?).map_err(GalatArg::Alasan)
    }

    fn peran(&self) -> Result<Peran, GalatArg> {
        self.wajib("peran")?
            .parse::<Peran>()
            .map_err(|_| GalatArg::PeranTidakDikenal)
    }
}

/// `YYYY-MM-DD` dibaca sebagai akhir hari itu WIB (pukul 24.00 +07.00); selain itu RFC 3339.
pub fn urai_berlaku_sampai(teks: &str) -> Result<DateTime<Utc>, GalatArg> {
    if let Ok(t) = DateTime::parse_from_rfc3339(teks) {
        return Ok(t.with_timezone(&Utc));
    }
    let tanggal =
        NaiveDate::parse_from_str(teks, "%Y-%m-%d").map_err(|_| GalatArg::TanggalTidakValid)?;
    let wib = FixedOffset::east_opt(7 * 3600).ok_or(GalatArg::TanggalTidakValid)?;
    tanggal
        .succ_opt()
        .and_then(|besok| besok.and_hms_opt(0, 0, 0))
        .and_then(|t| t.and_local_timezone(wib).single())
        .map(|t| t.with_timezone(&Utc))
        .ok_or(GalatArg::TanggalTidakValid)
}

/// Urai argumen (tanpa nama program) menjadi satu [`Perintah`].
pub fn urai(args: &[String]) -> Result<Perintah, GalatArg> {
    let kata: Vec<&str> = args.iter().map(String::as_str).collect();
    match kata.as_slice() {
        [] | ["bantuan"] | ["--help"] | ["-h"] => Ok(Perintah::Bantuan),
        ["bootstrap", ..] => {
            let o = Opsi::urai(&args[1..], &["username", "alasan"])?;
            Ok(Perintah::Bootstrap {
                username: o.wajib("username")?,
                alasan: o.alasan()?,
            })
        }
        ["pengguna", "buat", ..] => {
            let o = Opsi::urai(&args[2..], &["username", "oleh", "alasan", "client"])?;
            let client = match o.ambil("client") {
                Some(c) => Some(c.parse::<Uuid>().map_err(|_| GalatArg::ClientTidakValid)?),
                None => None,
            };
            Ok(Perintah::PenggunaBuat {
                username: o.wajib("username")?,
                client,
                oleh: o.wajib("oleh")?,
                alasan: o.alasan()?,
            })
        }
        ["pengguna", aksi @ ("nonaktifkan" | "aktifkan"), ..] => {
            let o = Opsi::urai(&args[2..], &["username", "oleh", "alasan"])?;
            Ok(Perintah::PenggunaUbahAktif {
                username: o.wajib("username")?,
                aktif: *aksi == "aktifkan",
                oleh: o.wajib("oleh")?,
                alasan: o.alasan()?,
            })
        }
        ["peran", "beri", ..] => {
            let o = Opsi::urai(
                &args[2..],
                &[
                    "username",
                    "peran",
                    "oleh",
                    "alasan",
                    "disetujui-oleh",
                    "berlaku-sampai",
                ],
            )?;
            let berlaku_sampai = match o.ambil("berlaku-sampai") {
                Some(t) => Some(urai_berlaku_sampai(&t)?),
                None => None,
            };
            Ok(Perintah::PeranBeri {
                username: o.wajib("username")?,
                peran: o.peran()?,
                oleh: o.wajib("oleh")?,
                alasan: o.alasan()?,
                disetujui_oleh: o.ambil("disetujui-oleh"),
                berlaku_sampai,
            })
        }
        ["peran", "cabut", ..] => {
            let o = Opsi::urai(&args[2..], &["username", "peran", "oleh", "alasan"])?;
            Ok(Perintah::PeranCabut {
                username: o.wajib("username")?,
                peran: o.peran()?,
                oleh: o.wajib("oleh")?,
                alasan: o.alasan()?,
            })
        }
        ["daftar", ..] => {
            Opsi::urai(&args[1..], &[])?;
            Ok(Perintah::Daftar)
        }
        ["seed", "pengguna", ..] => {
            Opsi::urai(&args[2..], &[])?;
            Ok(Perintah::SeedPengguna)
        }
        _ => Err(GalatArg::PerintahTidakDikenal),
    }
}

/// Sumber password untuk pengguna baru. Produksi memakai [`PasswordTerminal`]; uji memakai nilai
/// sintetis (seam T8).
pub trait PembacaPassword {
    fn baca_password_baru(&mut self) -> io::Result<Zeroizing<String>>;
}

/// Baca dari TTY tanpa echo (dengan konfirmasi) atau, bila stdin bukan TTY, satu baris dari stdin.
pub struct PasswordTerminal;

/// Mematikan echo terminal lewat `stty` (tanpa `unsafe`), dan menyalakannya lagi saat di-drop,
/// termasuk saat pembacaan gagal.
struct GemaMati;

impl GemaMati {
    fn pasang() -> io::Result<Self> {
        let status = Command::new("stty")
            .arg("-echo")
            .stdin(Stdio::inherit())
            .status()?;
        if !status.success() {
            return Err(io::Error::other(
                "tidak bisa mematikan echo terminal; kirim password lewat stdin",
            ));
        }
        Ok(Self)
    }
}

impl Drop for GemaMati {
    fn drop(&mut self) {
        let _ = Command::new("stty")
            .arg("echo")
            .stdin(Stdio::inherit())
            .status();
    }
}

fn baca_baris(stdin: &io::Stdin) -> io::Result<Zeroizing<String>> {
    let mut baris = Zeroizing::new(String::new());
    stdin.lock().read_line(&mut baris)?;
    let panjang = baris.trim_end_matches(['\n', '\r']).len();
    baris.truncate(panjang);
    Ok(baris)
}

fn baca_tanpa_echo(stdin: &io::Stdin, prompt: &str) -> io::Result<Zeroizing<String>> {
    let mut err = io::stderr();
    write!(err, "{prompt}: ")?;
    err.flush()?;
    let gema = GemaMati::pasang()?;
    let hasil = baca_baris(stdin);
    drop(gema);
    writeln!(err)?;
    hasil
}

impl PembacaPassword for PasswordTerminal {
    fn baca_password_baru(&mut self) -> io::Result<Zeroizing<String>> {
        let stdin = io::stdin();
        if !stdin.is_terminal() {
            return baca_baris(&stdin);
        }
        let pertama = baca_tanpa_echo(&stdin, "Password")?;
        let kedua = baca_tanpa_echo(&stdin, "Ulangi password")?;
        if *pertama != *kedua {
            return Err(io::Error::other("password tidak sama"));
        }
        Ok(pertama)
    }
}

/// Lingkungan eksekusi entrypoint. `seed_password` berasal dari `POOLROUTER_SEED_PASSWORD_FILE`
/// lewat `core::config` (secret hanya boleh lewat berkas).
pub struct Konteks {
    pub pool: PgPool,
    pub env: Environment,
    pub seed_password: Option<SecretString>,
    pub sekarang: DateTime<Utc>,
}

/// Kegagalan menjalankan perintah.
#[derive(Debug, thiserror::Error)]
pub enum GalatCli {
    #[error("{0}")]
    Arg(#[from] GalatArg),
    #[error("{0}")]
    Kelola(#[from] GalatKelola),
    #[error("POOLROUTER_SEED_PASSWORD_FILE wajib untuk seed pengguna")]
    SeedPasswordHilang,
    #[error("membaca password atau menulis keluaran: {0}")]
    Io(#[from] io::Error),
}

impl GalatCli {
    /// 2 untuk kesalahan pemakaian, 1 untuk penolakan atau kegagalan lain.
    pub fn kode_keluar(&self) -> u8 {
        match self {
            Self::Arg(_) => 2,
            _ => 1,
        }
    }
}

/// Jalankan satu perintah. Semua efek samping lewat parameter: koneksi, pembaca password, keluaran.
pub async fn jalankan(
    perintah: Perintah,
    k: &Konteks,
    pembaca: &mut dyn PembacaPassword,
    out: &mut dyn Write,
) -> Result<(), GalatCli> {
    match perintah {
        Perintah::Bantuan => writeln!(out, "{BANTUAN}")?,
        Perintah::Bootstrap { username, alasan } => {
            let password = pembaca.baca_password_baru()?;
            let id = kelola::bootstrap(&k.pool, &username, &password, &alasan).await?;
            writeln!(out, "platform admin pertama dibuat: {username} ({id})")?;
        }
        Perintah::PenggunaBuat {
            username,
            client,
            oleh,
            alasan,
        } => {
            let password = pembaca.baca_password_baru()?;
            let id = kelola::buat_pengguna(
                &k.pool, &username, client, &password, &oleh, &alasan, k.sekarang,
            )
            .await?;
            writeln!(out, "pengguna dibuat: {username} ({id})")?;
        }
        Perintah::PenggunaUbahAktif {
            username,
            aktif,
            oleh,
            alasan,
        } => {
            kelola::ubah_aktif(&k.pool, &username, aktif, &oleh, &alasan, k.sekarang).await?;
            let status = if aktif { "diaktifkan" } else { "dinonaktifkan" };
            writeln!(out, "pengguna {username} {status}; semua sesinya dicabut")?;
        }
        Perintah::PeranBeri {
            username,
            peran,
            oleh,
            alasan,
            disetujui_oleh,
            berlaku_sampai,
        } => {
            let p = PemberianPeran {
                target: &username,
                peran,
                oleh: &oleh,
                disetujui_oleh: disetujui_oleh.as_deref(),
                berlaku_sampai,
            };
            kelola::beri_peran(&k.pool, &p, &alasan, k.env, k.sekarang).await?;
            writeln!(
                out,
                "peran {peran} diberikan ke {username}; semua sesinya dicabut"
            )?;
        }
        Perintah::PeranCabut {
            username,
            peran,
            oleh,
            alasan,
        } => {
            kelola::cabut_peran(&k.pool, &username, peran, &oleh, &alasan, k.sekarang).await?;
            writeln!(
                out,
                "peran {peran} dicabut dari {username}; semua sesinya dicabut"
            )?;
        }
        Perintah::Daftar => {
            writeln!(out, "username\taktif\tperan\tclient_id")?;
            for p in kelola::daftar(&k.pool, k.sekarang).await? {
                let peran: Vec<&str> = p.peran.iter().map(|r| r.as_str()).collect();
                let client = p.client_id.map(|c| c.to_string()).unwrap_or_default();
                writeln!(
                    out,
                    "{}\t{}\t{}\t{}",
                    p.username,
                    if p.aktif { "ya" } else { "tidak" },
                    peran.join(","),
                    client
                )?;
            }
        }
        Perintah::SeedPengguna => {
            // Guard lingkungan lebih dulu, supaya penolakan tidak bergantung pada ada-tidaknya
            // berkas password (AC-42: di luar dev/ci keluar non-nol tanpa perubahan).
            if !k.env.allows_seed() {
                return Err(GalatKelola::SeedDitolak.into());
            }
            let password = k
                .seed_password
                .as_ref()
                .ok_or(GalatCli::SeedPasswordHilang)?;
            let h =
                kelola::seed_pengguna(&k.pool, k.env, password.expose_secret(), k.sekarang).await?;
            writeln!(
                out,
                "seed pengguna: {} pengguna baru, {} peran baru, {} client baru",
                h.pengguna_baru, h.peran_baru, h.client_baru
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn a(teks: &[&str]) -> Vec<String> {
        teks.iter().map(|s| (*s).to_owned()).collect()
    }

    const ALASAN: &str = "SPTJM-001 penugasan auditor PoC";

    #[test]
    fn opsi_password_tidak_dikenal() {
        let e = urai(&a(&[
            "pengguna",
            "buat",
            "--username",
            "budi",
            "--oleh",
            "admin",
            "--alasan",
            ALASAN,
            "--password",
            "nilai-uji-xq",
        ]))
        .unwrap_err();
        assert_eq!(e, GalatArg::OpsiTidakDikenal("--password".into()));
        let e = urai(&a(&["bootstrap", "--password", "x", "--username", "admin"])).unwrap_err();
        assert_eq!(e, GalatArg::OpsiTidakDikenal("--password".into()));
    }

    #[test]
    fn nilai_opsi_tak_dikenal_tidak_dicetak_balik() {
        let e = urai(&a(&["bootstrap", "--password=nilai-uji-xq"])).unwrap_err();
        assert_eq!(e, GalatArg::OpsiTidakDikenal("--password".into()));
        assert!(!e.to_string().contains("nilai-uji-xq"));
    }

    #[test]
    fn oleh_dan_alasan_wajib() {
        let e = urai(&a(&[
            "pengguna",
            "nonaktifkan",
            "--username",
            "budi",
            "--alasan",
            ALASAN,
        ]))
        .unwrap_err();
        assert_eq!(e, GalatArg::OpsiWajib("oleh"));
        let e = urai(&a(&[
            "peran",
            "cabut",
            "--username",
            "budi",
            "--peran",
            "auditor",
            "--oleh",
            "admin",
        ]))
        .unwrap_err();
        assert_eq!(e, GalatArg::OpsiWajib("alasan"));
        let e = urai(&a(&[
            "pengguna",
            "aktifkan",
            "--username",
            "budi",
            "--oleh",
            "admin",
            "--alasan",
            "pendek",
        ]))
        .unwrap_err();
        assert!(matches!(e, GalatArg::Alasan(_)));
    }

    #[test]
    fn peran_beri_lengkap() {
        let p = urai(&a(&[
            "peran",
            "beri",
            "--username",
            "siti",
            "--peran",
            "auditor",
            "--oleh",
            "admin",
            "--disetujui-oleh",
            "admin2",
            "--alasan",
            ALASAN,
            "--berlaku-sampai",
            "2026-12-31",
        ]))
        .unwrap();
        let Perintah::PeranBeri {
            peran,
            disetujui_oleh,
            berlaku_sampai,
            ..
        } = p
        else {
            panic!("hasil urai bukan PeranBeri");
        };
        assert_eq!(peran, Peran::Auditor);
        assert_eq!(disetujui_oleh.as_deref(), Some("admin2"));
        // Akhir 31 Desember WIB = 31 Desember 17.00 UTC.
        assert_eq!(
            berlaku_sampai,
            Some(Utc.with_ymd_and_hms(2026, 12, 31, 17, 0, 0).unwrap())
        );
    }

    #[test]
    fn peran_dan_perintah_tak_dikenal_ditolak() {
        let e = urai(&a(&[
            "peran",
            "beri",
            "--username",
            "x",
            "--peran",
            "dewa",
            "--oleh",
            "a",
            "--alasan",
            ALASAN,
        ]))
        .unwrap_err();
        assert_eq!(e, GalatArg::PeranTidakDikenal);
        assert_eq!(
            urai(&a(&["hapus"])).unwrap_err(),
            GalatArg::PerintahTidakDikenal
        );
        assert_eq!(
            urai(&a(&["daftar", "--semua", "x"])).unwrap_err(),
            GalatArg::OpsiTidakDikenal("--semua".into())
        );
        assert_eq!(
            urai(&a(&["seed", "pengguna"])).unwrap(),
            Perintah::SeedPengguna
        );
        assert_eq!(urai(&a(&[])).unwrap(), Perintah::Bantuan);
    }

    #[test]
    fn opsi_ganda_dan_nilai_hilang() {
        let e = urai(&a(&["bootstrap", "--username", "a", "--username", "b"])).unwrap_err();
        assert_eq!(e, GalatArg::OpsiGanda("--username".into()));
        let e = urai(&a(&["bootstrap", "--username"])).unwrap_err();
        assert_eq!(e, GalatArg::NilaiHilang("--username".into()));
    }

    #[test]
    fn berlaku_sampai_rfc3339() {
        assert_eq!(
            urai_berlaku_sampai("2026-12-01T10:00:00+07:00").unwrap(),
            Utc.with_ymd_and_hms(2026, 12, 1, 3, 0, 0).unwrap()
        );
        assert!(urai_berlaku_sampai("31-12-2026").is_err());
    }

    #[test]
    fn bantuan_tidak_menawarkan_opsi_password() {
        assert!(!BANTUAN.contains("--password"));
    }
}
