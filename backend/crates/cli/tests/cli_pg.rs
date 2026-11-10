//! `backend-cli` terhadap PostgreSQL sungguhan (AC-39, AC-41, AC-42), lewat entrypoint yang sama
//! dengan binary dan role `pr_admin_api`. Baris audit dibaca lewat superuser karena `pr_admin_api`
//! hanya punya INSERT pada `audit_akses`. `#[ignore]`: dijalankan `make -C backend uji-pg`.
//!
//! Basis data dipakai bersama binary lain, jadi setiap uji membuat pengguna sintetis bernama unik;
//! hanya uji seed yang memakai nama tetap (`seed-*`).

use std::io;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use poolrouter_cli::{GalatCli, Konteks, PembacaPassword, Perintah, jalankan};
use poolrouter_core::admin::TokenSesi;
use poolrouter_core::admin::kelola_pengguna::{
    self as kelola, GalatKelola, SEED_ADMIN_AUDITOR, SEED_AUDITOR, SEED_CLIENT_OWNER,
    SEED_OPERATOR, SEED_PENYETUJU, SEED_PLATFORM_ADMIN,
};
use poolrouter_core::admin::seed_dukcapil::{
    SEED_DOWNSTREAM, SEED_ENDPOINT, SEED_PROVIDER, SEED_UPSTREAM,
};
use poolrouter_core::admin::sesi_store::SesiStore;
use poolrouter_core::audit::{Alasan, Peran};
use poolrouter_core::config::Environment;
use secrecy::SecretString;
use sqlx::PgPool;
use tokio::sync::Barrier;
use uuid::Uuid;
use zeroize::Zeroizing;

const NILAI_UJI: &str = "nilai-uji-sintetis-qx-01";
const ALASAN: &str = "SPTJM-UJI-001 penugasan sintetis ARC-55";

struct PasswordSintetis;

impl PembacaPassword for PasswordSintetis {
    fn baca_password_baru(&mut self) -> io::Result<Zeroizing<String>> {
        Ok(Zeroizing::new(NILAI_UJI.to_owned()))
    }
}

async fn pool(var: &str) -> PgPool {
    let url = std::env::var(var)
        .unwrap_or_else(|_| panic!("{var} belum diset; jalankan lewat `make -C backend uji-pg`"));
    PgPool::connect(&url)
        .await
        .expect("database tidak dapat dihubungi")
}

struct Uji {
    admin: PgPool,
    su: PgPool,
    sekarang: DateTime<Utc>,
}

impl Uji {
    async fn baru() -> Self {
        Self {
            admin: pool("POOLROUTER_TEST_DATABASE_URL").await,
            su: pool("POOLROUTER_TEST_DATABASE_URL_SU").await,
            sekarang: Utc::now(),
        }
    }

    fn konteks(&self, env: Environment, seed: Option<&str>) -> Konteks {
        Konteks {
            pool: self.admin.clone(),
            env,
            seed_password: seed.map(|s| SecretString::from(s.to_owned())),
            sekarang: self.sekarang,
        }
    }

    async fn jalankan(&self, p: Perintah) -> Result<String, GalatCli> {
        self.jalankan_di(Environment::Ci, None, p).await
    }

    async fn jalankan_di(
        &self,
        env: Environment,
        seed: Option<&str>,
        p: Perintah,
    ) -> Result<String, GalatCli> {
        let mut out = Vec::new();
        jalankan(p, &self.konteks(env, seed), &mut PasswordSintetis, &mut out).await?;
        Ok(String::from_utf8(out).unwrap())
    }

    /// Fixture: pengguna sintetis dengan peran tertentu, ditulis langsung sebagai `pr_admin_api`.
    async fn pengguna(&self, awalan: &str, peran: &[Peran], aktif: bool) -> (Uuid, String) {
        let id = Uuid::now_v7();
        let username = format!("{awalan}-{}", id.simple());
        sqlx::query(
            "INSERT INTO admin_user (pengguna_id, username, password_hash, aktif) \
             VALUES ($1, $2, 'x', $3)",
        )
        .bind(id)
        .bind(&username)
        .bind(aktif)
        .execute(&self.admin)
        .await
        .unwrap();
        for p in peran {
            assert!(
                !p.butuh_persetujuan(),
                "fixture hanya untuk peran tanpa A-02"
            );
            sqlx::query(
                "INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh) VALUES ($1, $2, $1)",
            )
            .bind(id)
            .bind(p.as_str())
            .execute(&self.admin)
            .await
            .unwrap();
        }
        (id, username)
    }

    async fn sesi(&self, pengguna: Uuid) {
        SesiStore::baru(self.admin.clone())
            .terbitkan(
                pengguna,
                &TokenSesi::baru().hash(),
                Some("uji-cli"),
                self.sekarang,
            )
            .await
            .unwrap();
    }

    async fn jumlah_sesi(&self, pengguna: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM admin_session WHERE pengguna_id = $1")
            .bind(pengguna)
            .fetch_one(&self.su)
            .await
            .unwrap()
    }

    async fn jumlah_ubah_peran(&self, target: Uuid) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM audit_akses WHERE aksi = 'ubah_peran' \
             AND filter_kueri->>'target_pengguna_id' = $1::text",
        )
        .bind(target)
        .fetch_one(&self.su)
        .await
        .unwrap()
    }

    async fn jumlah_konfigurasi(&self, target: Uuid) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM audit_konfigurasi WHERE entitas_jenis = 'admin_user' AND entitas_id = $1",
        )
        .bind(target)
        .fetch_one(&self.su)
        .await
        .unwrap()
    }

    async fn peran(&self, username: &str) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT p.peran FROM admin_user_peran p JOIN admin_user u USING (pengguna_id) \
             WHERE u.username = $1 AND p.dicabut_at IS NULL ORDER BY p.peran",
        )
        .bind(username)
        .fetch_all(&self.su)
        .await
        .unwrap()
    }

    /// Basis data uji dipakai bersama, jadi platform_admin aktif milik uji lain ikut terhitung oleh
    /// penjaga. Nonaktifkan semuanya kecuali `tetap`; kembalikan id-nya untuk [`Self::pulihkan`].
    async fn sisakan_admin(&self, tetap: &[Uuid]) -> Vec<Uuid> {
        sqlx::query_scalar(
            "UPDATE admin_user u SET aktif = false \
             WHERE u.aktif AND u.pengguna_id <> ALL($1) AND EXISTS ( \
               SELECT 1 FROM admin_user_peran p WHERE p.pengguna_id = u.pengguna_id \
                 AND p.peran = 'platform_admin' AND p.dicabut_at IS NULL \
                 AND (p.berlaku_sampai IS NULL OR p.berlaku_sampai > now())) \
             RETURNING u.pengguna_id",
        )
        .bind(tetap)
        .fetch_all(&self.su)
        .await
        .unwrap()
    }

    async fn pulihkan(&self, ids: &[Uuid]) {
        sqlx::query("UPDATE admin_user SET aktif = true WHERE pengguna_id = ANY($1)")
            .bind(ids)
            .execute(&self.su)
            .await
            .unwrap();
    }

    async fn id(&self, username: &str) -> Uuid {
        sqlx::query_scalar("SELECT pengguna_id FROM admin_user WHERE username = $1")
            .bind(username)
            .fetch_one(&self.su)
            .await
            .unwrap()
    }
}

fn alasan() -> Alasan {
    Alasan::baru(ALASAN).unwrap()
}

fn buat(username: &str, oleh: &str) -> Perintah {
    Perintah::PenggunaBuat {
        username: username.to_owned(),
        client: None,
        oleh: oleh.to_owned(),
        alasan: alasan(),
    }
}

fn beri(username: &str, peran: Peran, oleh: &str, penyetuju: Option<&str>) -> Perintah {
    Perintah::PeranBeri {
        username: username.to_owned(),
        peran,
        oleh: oleh.to_owned(),
        alasan: alasan(),
        disetujui_oleh: penyetuju.map(str::to_owned),
        berlaku_sampai: None,
    }
}

#[tokio::test]
#[ignore]
async fn buat_pengguna_hanya_oleh_platform_admin_aktif() {
    let u = Uji::baru().await;
    let (_, admin) = u.pengguna("pa", &[Peran::PlatformAdmin], true).await;
    let (_, admin_mati) = u.pengguna("pa-mati", &[Peran::PlatformAdmin], false).await;
    let (_, pemilik) = u.pengguna("co-bukan-admin", &[], true).await;

    let baru = format!("op-{}", Uuid::now_v7().simple());
    let teks = u.jalankan(buat(&baru, &admin)).await.unwrap();
    assert!(teks.contains(&baru) && !teks.contains(NILAI_UJI));
    let id = u.id(&baru).await;
    assert_eq!(
        u.jumlah_konfigurasi(id).await,
        1,
        "satu audit_konfigurasi buat"
    );
    let hash: String =
        sqlx::query_scalar("SELECT password_hash FROM admin_user WHERE pengguna_id = $1")
            .bind(id)
            .fetch_one(&u.su)
            .await
            .unwrap();
    assert!(hash.starts_with("$argon2id$"));
    let tercatat: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_konfigurasi WHERE entitas_id = $1 \
         AND (nilai_baru::text LIKE '%argon2%' OR nilai_baru::text LIKE $2)",
    )
    .bind(id)
    .bind(format!("%{NILAI_UJI}%"))
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(tercatat, 0, "hash dan password tidak boleh masuk audit");

    let lain = format!("op-{}", Uuid::now_v7().simple());
    let e = u.jalankan(buat(&lain, &admin_mati)).await.unwrap_err();
    assert!(
        matches!(e, GalatCli::Kelola(GalatKelola::PelakuTidakAktif(_))),
        "{e}"
    );
    let e = u.jalankan(buat(&lain, &pemilik)).await.unwrap_err();
    assert!(
        matches!(e, GalatCli::Kelola(GalatKelola::PelakuBukanAdmin(_))),
        "{e}"
    );
    let e = u
        .jalankan(buat(&lain, "tidak-ada-orangnya"))
        .await
        .unwrap_err();
    assert!(
        matches!(e, GalatCli::Kelola(GalatKelola::PelakuTidakAda(_))),
        "{e}"
    );
    let e = u.jalankan(buat(&baru, &admin)).await.unwrap_err();
    assert!(
        matches!(e, GalatCli::Kelola(GalatKelola::UsernameDipakai(_))),
        "{e}"
    );
    let ada: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM admin_user WHERE username = $1)")
            .bind(&lain)
            .fetch_one(&u.su)
            .await
            .unwrap();
    assert!(!ada, "penolakan tidak boleh meninggalkan pengguna");
}

#[tokio::test]
#[ignore]
async fn a02_platform_admin_tidak_bisa_memberi_dirinya_auditor() {
    let u = Uji::baru().await;
    let (id, admin) = u.pengguna("pa", &[Peran::PlatformAdmin], true).await;
    let (_, penyetuju) = u.pengguna("pa-setuju", &[Peran::PlatformAdmin], true).await;

    let e = u
        .jalankan(beri(&admin, Peran::Auditor, &admin, Some(&penyetuju)))
        .await
        .unwrap_err();
    assert!(
        matches!(
            e,
            GalatCli::Kelola(GalatKelola::PeranDiriSendiri(Peran::Auditor))
        ),
        "{e}"
    );
    assert_eq!(u.peran(&admin).await, vec!["platform_admin".to_owned()]);
    assert_eq!(
        u.jumlah_ubah_peran(id).await,
        0,
        "penolakan tidak menulis audit"
    );
}

#[tokio::test]
#[ignore]
async fn beri_dan_cabut_peran_menulis_satu_ubah_peran_dan_mencabut_sesi() {
    let u = Uji::baru().await;
    let (_, admin) = u.pengguna("pa", &[Peran::PlatformAdmin], true).await;
    let (_, penyetuju) = u.pengguna("pa-setuju", &[Peran::PlatformAdmin], true).await;
    let (target, nama) = u.pengguna("calon-auditor", &[], true).await;
    u.sesi(target).await;
    u.sesi(target).await;
    assert_eq!(u.jumlah_sesi(target).await, 2);

    // Tanpa penyetuju: ditolak sebelum menyentuh data.
    let e = u
        .jalankan(beri(&nama, Peran::Auditor, &admin, None))
        .await
        .unwrap_err();
    assert!(
        matches!(e, GalatCli::Kelola(GalatKelola::PenyetujuWajib(_))),
        "{e}"
    );
    assert_eq!(u.jumlah_sesi(target).await, 2);

    u.jalankan(beri(&nama, Peran::Auditor, &admin, Some(&penyetuju)))
        .await
        .unwrap();
    assert_eq!(u.peran(&nama).await, vec!["auditor".to_owned()]);
    assert_eq!(
        u.jumlah_ubah_peran(target).await,
        1,
        "tepat satu ubah_peran"
    );
    assert_eq!(u.jumlah_sesi(target).await, 0, "semua sesi target dicabut");
    let (pelaku, peran_pelaku, sumber_ip, sesi): (Uuid, String, String, String) = sqlx::query_as(
        "SELECT a.pengguna_id, a.peran, a.sumber_ip, a.sesi_id FROM audit_akses a \
         WHERE a.aksi = 'ubah_peran' AND a.filter_kueri->>'target_pengguna_id' = $1::text",
    )
    .bind(target)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(pelaku, u.id(&admin).await);
    assert_eq!(peran_pelaku, "platform_admin");
    assert_eq!((sumber_ip.as_str(), sesi.as_str()), ("cli", "cli"));

    u.sesi(target).await;
    u.jalankan(Perintah::PeranCabut {
        username: nama.clone(),
        peran: Peran::Auditor,
        oleh: admin.clone(),
        alasan: alasan(),
    })
    .await
    .unwrap();
    assert!(u.peran(&nama).await.is_empty());
    assert_eq!(u.jumlah_ubah_peran(target).await, 2);
    assert_eq!(u.jumlah_sesi(target).await, 0);

    // Peran yang pernah dicabut bisa diberikan lagi (baris yang sama diperbarui).
    u.jalankan(beri(&nama, Peran::Auditor, &admin, Some(&penyetuju)))
        .await
        .unwrap();
    assert_eq!(u.peran(&nama).await, vec!["auditor".to_owned()]);
}

#[tokio::test]
#[ignore]
async fn nonaktifkan_mencabut_sesi_dan_aktifkan_kembali() {
    let u = Uji::baru().await;
    let (_, admin) = u.pengguna("pa", &[Peran::PlatformAdmin], true).await;
    let (target, nama) = u.pengguna("staf", &[], true).await;
    u.sesi(target).await;

    u.jalankan(Perintah::PenggunaUbahAktif {
        username: nama.clone(),
        aktif: false,
        oleh: admin.clone(),
        alasan: alasan(),
    })
    .await
    .unwrap();
    let aktif: bool = sqlx::query_scalar("SELECT aktif FROM admin_user WHERE pengguna_id = $1")
        .bind(target)
        .fetch_one(&u.su)
        .await
        .unwrap();
    assert!(!aktif);
    assert_eq!(u.jumlah_sesi(target).await, 0);
    assert_eq!(u.jumlah_ubah_peran(target).await, 1);
    assert_eq!(u.jumlah_konfigurasi(target).await, 1);

    // Pengguna nonaktif tidak bisa menjadi pelaku.
    let e = u
        .jalankan(buat(&format!("x-{}", Uuid::now_v7().simple()), &nama))
        .await
        .unwrap_err();
    assert!(
        matches!(e, GalatCli::Kelola(GalatKelola::PelakuTidakAktif(_))),
        "{e}"
    );

    u.jalankan(Perintah::PenggunaUbahAktif {
        username: nama.clone(),
        aktif: true,
        oleh: admin.clone(),
        alasan: alasan(),
    })
    .await
    .unwrap();
    assert_eq!(u.jumlah_ubah_peran(target).await, 2);

    let daftar = u.jalankan(Perintah::Daftar).await.unwrap();
    let baris = daftar
        .lines()
        .find(|l| l.starts_with(&format!("{nama}\t")))
        .expect("pengguna ada di daftar");
    assert!(baris.contains("\tya\t"));
    assert!(!daftar.contains("argon2"), "daftar tanpa hash");
}

#[tokio::test]
#[ignore]
async fn auditor_berlaku_sampai_wajib_di_poc() {
    let u = Uji::baru().await;
    let (_, admin) = u.pengguna("pa", &[Peran::PlatformAdmin], true).await;
    let (_, penyetuju) = u.pengguna("pa-setuju", &[Peran::PlatformAdmin], true).await;
    let (_, nama) = u.pengguna("auditor-poc", &[], true).await;

    let e = u
        .jalankan_di(
            Environment::Poc,
            None,
            beri(&nama, Peran::Auditor, &admin, Some(&penyetuju)),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(e, GalatCli::Kelola(GalatKelola::BerlakuSampaiWajib)),
        "{e}"
    );

    let mut p = beri(&nama, Peran::Auditor, &admin, Some(&penyetuju));
    if let Perintah::PeranBeri { berlaku_sampai, .. } = &mut p {
        *berlaku_sampai = Some(u.sekarang + Duration::days(30));
    }
    u.jalankan_di(Environment::Poc, None, p).await.unwrap();
    assert_eq!(u.peran(&nama).await, vec!["auditor".to_owned()]);
}

async fn hitung_semua(u: &Uji) -> (i64, i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM admin_user), (SELECT count(*) FROM admin_user_peran), \
                (SELECT count(*) FROM client), (SELECT count(*) FROM audit_akses), \
                (SELECT count(*) FROM audit_konfigurasi)",
    )
    .fetch_one(&u.su)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore]
async fn seed_pengguna_hanya_dev_ci_dan_idempoten() {
    let u = Uji::baru().await;

    // Di luar dev/ci: keluar non-nol tanpa perubahan, walau password seed tersedia.
    let awal = hitung_semua(&u).await;
    for env in [Environment::Staging, Environment::Poc, Environment::Prod] {
        let e = u
            .jalankan_di(env, Some(NILAI_UJI), Perintah::SeedPengguna)
            .await
            .unwrap_err();
        assert!(
            matches!(e, GalatCli::Kelola(GalatKelola::SeedDitolak)),
            "{e}"
        );
        assert_ne!(e.kode_keluar(), 0);
    }
    let e = u
        .jalankan_di(Environment::Ci, None, Perintah::SeedPengguna)
        .await
        .unwrap_err();
    assert!(matches!(e, GalatCli::SeedPasswordHilang), "{e}");
    assert_eq!(
        hitung_semua(&u).await,
        awal,
        "penolakan seed tanpa perubahan"
    );

    let teks = u
        .jalankan_di(Environment::Ci, Some(NILAI_UJI), Perintah::SeedPengguna)
        .await
        .unwrap();
    assert!(!teks.contains(NILAI_UJI));
    let setelah_pertama = hitung_semua(&u).await;
    assert_eq!(setelah_pertama.0 - awal.0, 6, "enam pengguna seed");

    for (username, peran) in [
        (SEED_PLATFORM_ADMIN, vec!["platform_admin"]),
        (SEED_PENYETUJU, vec!["platform_admin"]),
        (SEED_OPERATOR, vec!["operator"]),
        (SEED_AUDITOR, vec!["auditor"]),
        (SEED_CLIENT_OWNER, vec!["client_owner"]),
        (SEED_ADMIN_AUDITOR, vec!["auditor", "platform_admin"]),
    ] {
        assert_eq!(u.peran(username).await, peran, "peran {username}");
    }
    let client: Option<Uuid> =
        sqlx::query_scalar("SELECT client_id FROM admin_user WHERE username = $1")
            .bind(SEED_CLIENT_OWNER)
            .fetch_one(&u.su)
            .await
            .unwrap();
    assert!(
        client.is_some(),
        "client owner seed terikat client sintetis"
    );

    // Jalan kedua (juga di dev): tidak ada baris tambahan di tabel mana pun.
    let teks = u
        .jalankan_di(Environment::Dev, Some(NILAI_UJI), Perintah::SeedPengguna)
        .await
        .unwrap();
    assert!(
        teks.contains("0 pengguna baru, 0 peran baru, 0 client baru"),
        "{teks}"
    );
    assert_eq!(hitung_semua(&u).await, setelah_pertama);
}

/// Seed hanya membuat yang belum ada. Pengguna seed yang sengaja dinonaktifkan (mis. lewat
/// `pengguna nonaktifkan`) tidak boleh dipakai ulang diam-diam, dan tidak diaktifkan kembali: seed
/// ditolak tanpa perubahan apa pun. Namanya diurutkan sesudah uji seed lain (lihat catatan di bawah).
#[tokio::test]
#[ignore]
async fn seed_pengguna_menolak_pengguna_seed_nonaktif() {
    let u = Uji::baru().await;
    u.jalankan_di(Environment::Ci, Some(NILAI_UJI), Perintah::SeedPengguna)
        .await
        .unwrap();

    for username in [
        SEED_PLATFORM_ADMIN,
        SEED_PENYETUJU,
        SEED_OPERATOR,
        SEED_AUDITOR,
        SEED_CLIENT_OWNER,
        SEED_ADMIN_AUDITOR,
    ] {
        let set_aktif = |aktif: bool| {
            sqlx::query("UPDATE admin_user SET aktif = $2 WHERE username = $1")
                .bind(username)
                .bind(aktif)
                .execute(&u.su)
        };
        set_aktif(false).await.unwrap();
        let sebelum = hitung_semua(&u).await;
        let hasil = u
            .jalankan_di(Environment::Ci, Some(NILAI_UJI), Perintah::SeedPengguna)
            .await;
        let setelah = hitung_semua(&u).await;
        let aktif: bool = sqlx::query_scalar("SELECT aktif FROM admin_user WHERE username = $1")
            .bind(username)
            .fetch_one(&u.su)
            .await
            .unwrap();
        // Pulihkan dulu supaya kegagalan di bawah tidak merusak uji lain.
        set_aktif(true).await.unwrap();

        let e = hasil.expect_err(username);
        assert!(
            matches!(&e, GalatCli::Kelola(GalatKelola::SeedNonaktif(n)) if n == username),
            "{username}: {e}"
        );
        assert_ne!(e.kode_keluar(), 0);
        assert!(
            !aktif,
            "{username} tidak boleh diaktifkan kembali oleh seed"
        );
        assert_eq!(setelah, sebelum, "{username}: penolakan tanpa perubahan");
    }

    // Setelah dipulihkan, seed kembali idempoten.
    let teks = u
        .jalankan_di(Environment::Ci, Some(NILAI_UJI), Perintah::SeedPengguna)
        .await
        .unwrap();
    assert!(teks.contains("0 pengguna baru, 0 peran baru"), "{teks}");
} 
fn cabut_admin(username: &str, oleh: &str) -> Perintah {
    Perintah::PeranCabut {
        username: username.to_owned(),
        peran: Peran::PlatformAdmin,
        oleh: oleh.to_owned(),
        alasan: alasan(),
    }
}

#[tokio::test]
#[ignore]
async fn platform_admin_aktif_terakhir_tidak_bisa_dicabut_perannya() {
    let u = Uji::baru().await;
    let (a, nama_a) = u
        .pengguna("pa-terakhir", &[Peran::PlatformAdmin], true)
        .await;
    let (b, nama_b) = u.pengguna("pa-lain", &[Peran::PlatformAdmin], true).await;
    let dinonaktifkan = u.sisakan_admin(&[a, b]).await;
    u.sesi(b).await;

    // Hasil dikumpulkan dulu supaya admin uji lain dipulihkan sebelum assert apa pun.
    let r_dua_admin = u.jalankan(cabut_admin(&nama_a, &nama_a)).await;
    let peran_a = u.peran(&nama_a).await;
    let r_terakhir = u.jalankan(cabut_admin(&nama_b, &nama_b)).await;
    let peran_b = u.peran(&nama_b).await;
    let sesi_b = u.jumlah_sesi(b).await;
    let audit_b = u.jumlah_ubah_peran(b).await;
    u.pulihkan(&dinonaktifkan).await;

    r_dua_admin.expect("masih ada admin lain: pencabutan peran sendiri boleh");
    assert!(peran_a.is_empty());
    let e = r_terakhir.unwrap_err();
    assert!(
        matches!(e, GalatCli::Kelola(GalatKelola::AdminTerakhir(_))),
        "{e}"
    );
    assert!(
        e.to_string().contains("platform_admin aktif terakhir"),
        "{e}"
    );
    assert_ne!(e.kode_keluar(), 0);
    assert_eq!(peran_b, vec!["platform_admin".to_owned()], "peran utuh");
    assert_eq!(sesi_b, 1, "sesi tidak dicabut oleh operasi yang ditolak");
    assert_eq!(audit_b, 0, "penolakan tidak menulis audit");
}

/// Dua platform_admin saling menonaktifkan pada waktu yang sama: masing-masing melihat admin lain
/// masih aktif, jadi tanpa serialisasi keduanya lolos dan sistem terkunci. Beberapa putaran dengan
/// pasangan baru karena balapan ini tidak bisa dipaksa tanpa kait di dalam transaksi.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn dua_penonaktifan_bersamaan_tidak_mengunci_sistem() {
    let u = Uji::baru().await;
    let mut dinonaktifkan = Vec::new();
    let mut putaran = Vec::new();
    for _ in 0..16 {
        let (a, nama_a) = u.pengguna("pa-balap", &[Peran::PlatformAdmin], true).await;
        let (b, nama_b) = u.pengguna("pa-balap", &[Peran::PlatformAdmin], true).await;
        dinonaktifkan.extend(u.sisakan_admin(&[a, b]).await);
        let gerbang = Arc::new(Barrier::new(2));
        let nonaktifkan = |target: String, oleh: String| {
            let (pool, gerbang) = (u.admin.clone(), gerbang.clone());
            tokio::spawn(async move {
                gerbang.wait().await;
                kelola::ubah_aktif(&pool, &target, false, &oleh, &alasan(), Utc::now()).await
            })
        };
        let t1 = nonaktifkan(nama_b.clone(), nama_a.clone());
        let t2 = nonaktifkan(nama_a.clone(), nama_b.clone());
        let (r1, r2) = (t1.await.unwrap(), t2.await.unwrap());
        let aktif: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM admin_user WHERE pengguna_id = ANY($1) AND aktif",
        )
        .bind(vec![a, b])
        .fetch_one(&u.su)
        .await
        .unwrap();
        putaran.push((r1, r2, aktif));
    }
    u.pulihkan(&dinonaktifkan).await;

    for (i, (r1, r2, aktif)) in putaran.into_iter().enumerate() {
        assert_eq!(
            [r1.is_ok(), r2.is_ok()].iter().filter(|b| **b).count(),
            1,
            "putaran {i}: tepat satu berhasil ({r1:?} / {r2:?})"
        );
        assert_eq!(aktif, 1, "putaran {i}: tepat satu admin tetap aktif");
        let ditolak = r1.err().or(r2.err()).unwrap();
        assert!(
            matches!(
                ditolak,
                GalatKelola::PelakuTidakAktif(_) | GalatKelola::AdminTerakhir(_)
            ),
            "putaran {i}: {ditolak}"
        );
    }
}

/// Dua platform_admin mencabut peran platform_admin miliknya sendiri pada waktu yang sama. Pelaku
/// tetap aktif di jalur ini, jadi hanya `jaga_admin_terakhir` di bawah kunci yang bisa menolak
/// salah satunya; tanpa serialisasi keduanya melihat admin lain masih berperan dan sama-sama lolos.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn dua_pencabutan_peran_bersamaan_tidak_mengunci_sistem() {
    let u = Uji::baru().await;
    let mut dinonaktifkan = Vec::new();
    let mut putaran = Vec::new();
    for _ in 0..16 {
        let (a, nama_a) = u.pengguna("pa-cabut", &[Peran::PlatformAdmin], true).await;
        let (b, nama_b) = u.pengguna("pa-cabut", &[Peran::PlatformAdmin], true).await;
        dinonaktifkan.extend(u.sisakan_admin(&[a, b]).await);
        let gerbang = Arc::new(Barrier::new(2));
        let cabut = |nama: String| {
            let (pool, gerbang) = (u.admin.clone(), gerbang.clone());
            tokio::spawn(async move {
                gerbang.wait().await;
                kelola::cabut_peran(
                    &pool,
                    &nama,
                    Peran::PlatformAdmin,
                    &nama,
                    &alasan(),
                    Utc::now(),
                )
                .await
            })
        };
        let t1 = cabut(nama_a.clone());
        let t2 = cabut(nama_b.clone());
        let (r1, r2) = (t1.await.unwrap(), t2.await.unwrap());
        let masih_berperan: i64 = sqlx::query_scalar(
            "SELECT count(DISTINCT pengguna_id) FROM admin_user_peran \
             WHERE pengguna_id = ANY($1) AND peran = 'platform_admin' AND dicabut_at IS NULL \
               AND (berlaku_sampai IS NULL OR berlaku_sampai > now())",
        )
        .bind(vec![a, b])
        .fetch_one(&u.su)
        .await
        .unwrap();
        putaran.push((r1, r2, masih_berperan));
    }
    u.pulihkan(&dinonaktifkan).await;

    for (i, (r1, r2, masih_berperan)) in putaran.into_iter().enumerate() {
        assert_eq!(
            [r1.is_ok(), r2.is_ok()].iter().filter(|b| **b).count(),
            1,
            "putaran {i}: tepat satu berhasil ({r1:?} / {r2:?})"
        );
        assert_eq!(
            masih_berperan, 1,
            "putaran {i}: tepat satu tetap platform_admin"
        );
        let ditolak = r1.err().or(r2.err()).unwrap();
        assert!(
            matches!(ditolak, GalatKelola::AdminTerakhir(_)),
            "putaran {i}: {ditolak}"
        );
    }
}

async fn hitung_rantai_dukcapil(u: &Uji) -> (i64, i64, i64, i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM provider), (SELECT count(*) FROM provider_config_versi), \
                (SELECT count(*) FROM upstream_pool), (SELECT count(*) FROM downstream_pool), \
                (SELECT count(*) FROM downstream_pool_rute), \
                (SELECT count(*) FROM downstream_endpoint), \
                (SELECT count(*) FROM audit_konfigurasi)",
    )
    .fetch_one(&u.su)
    .await
    .unwrap()
}

/// Namanya diurutkan sesudah `seed_pengguna_hanya_dev_ci_dan_idempoten` (uji berjalan menurut abjad
/// dengan `--test-threads=1`): uji itu menghitung pengguna seed dari nol, sedangkan uji ini
/// menjalankan `seed pengguna` sendiri karena `seed dukcapil` membutuhkannya.
#[tokio::test]
#[ignore]
async fn seed_pengguna_lalu_seed_dukcapil_hanya_dev_ci_dan_idempoten() {
    let u = Uji::baru().await;
    u.jalankan_di(Environment::Ci, Some(NILAI_UJI), Perintah::SeedPengguna)
        .await
        .unwrap();

    // Di luar dev/ci: ditolak tanpa perubahan di tabel mana pun.
    let awal = hitung_rantai_dukcapil(&u).await;
    for env in [Environment::Staging, Environment::Poc, Environment::Prod] {
        let e = u
            .jalankan_di(env, None, Perintah::SeedDukcapil)
            .await
            .unwrap_err();
        assert!(
            matches!(e, GalatCli::Kelola(GalatKelola::SeedDitolak)),
            "{e}"
        );
        assert_ne!(e.kode_keluar(), 0);
    }
    assert_eq!(hitung_rantai_dukcapil(&u).await, awal);

    let teks = u
        .jalankan_di(Environment::Ci, None, Perintah::SeedDukcapil)
        .await
        .unwrap();
    let pertama = hitung_rantai_dukcapil(&u).await;
    assert!(
        teks.contains(
            "1 provider baru, 1 upstream pool baru, 1 downstream pool baru, 1 endpoint baru"
        ),
        "{teks}"
    );
    // provider, versi provider, upstream, downstream, rute, endpoint, lima baris audit.
    assert_eq!(
        (
            pertama.0 - awal.0,
            pertama.1 - awal.1,
            pertama.2 - awal.2,
            pertama.3 - awal.3,
            pertama.4 - awal.4,
            pertama.5 - awal.5,
            pertama.6 - awal.6
        ),
        (1, 1, 1, 1, 1, 1, 4)
    );

    // Rantai utuh: endpoint aktif -> downstream pool -> rute -> upstream pool -> provider, dan
    // kedua penunjuk versi aktif menunjuk versi yang ada.
    let utuh: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM downstream_endpoint e \
         JOIN downstream_pool dp ON dp.downstream_pool_id = e.downstream_pool_id \
         JOIN downstream_pool_rute r ON r.downstream_pool_id = dp.downstream_pool_id \
         JOIN upstream_pool up ON up.upstream_pool_id = r.upstream_pool_id \
         JOIN provider p ON p.provider_id = up.provider_id AND p.provider_id = e.provider_id \
         JOIN provider_config_versi pv \
           ON pv.provider_id = p.provider_id AND pv.versi = p.config_versi_aktif \
         JOIN endpoint_config_versi ev \
           ON ev.downstream_endpoint_id = e.downstream_endpoint_id AND ev.versi = e.config_versi_aktif \
         WHERE e.nama = $1 AND e.aktif AND dp.nama = $2 AND up.nama = $3 AND p.nama = $4 AND p.aktif",
    )
    .bind(SEED_ENDPOINT)
    .bind(SEED_DOWNSTREAM)
    .bind(SEED_UPSTREAM)
    .bind(SEED_PROVIDER)
    .fetch_one(&u.su)
    .await
    .unwrap();
    assert_eq!(utuh, 1, "rantai seed dukcapil utuh");

    // Jalan kedua: tidak ada baris tambahan, termasuk audit.
    let teks = u
        .jalankan_di(Environment::Dev, None, Perintah::SeedDukcapil)
        .await
        .unwrap();
    assert!(
        teks.contains(
            "0 provider baru, 0 upstream pool baru, 0 downstream pool baru, 0 endpoint baru"
        ),
        "{teks}"
    );
    assert_eq!(hitung_rantai_dukcapil(&u).await, pertama);
}
