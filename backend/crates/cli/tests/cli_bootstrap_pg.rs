//! Bootstrap platform admin pertama (AC-39) terhadap PostgreSQL sungguhan sebagai `pr_admin_api`.
//!
//! Bootstrap hanya sah saat `admin_user` kosong, sedangkan basis data `uji-pg` dipakai bersama semua
//! binary uji. Karena itu binary ini berdiri sendiri dan dijalankan PALING AWAL di `uji-pg-cargo`
//! (baris pertama target itu di `backend/Makefile`). `#[ignore]`: butuh `POOLROUTER_TEST_DATABASE_URL`.

use std::io;

use chrono::Utc;
use poolrouter_cli::{GalatCli, Konteks, PembacaPassword, Perintah, jalankan};
use poolrouter_core::admin::kelola_pengguna::GalatKelola;
use poolrouter_core::audit::Alasan;
use poolrouter_core::config::Environment;
use sqlx::PgPool;
use zeroize::Zeroizing;

const PASSWORD: &str = "kata-sandi-sintetis-bootstrap-01";

struct PasswordSintetis;

impl PembacaPassword for PasswordSintetis {
    fn baca_password_baru(&mut self) -> io::Result<Zeroizing<String>> {
        Ok(Zeroizing::new(PASSWORD.to_owned()))
    }
}

async fn pool(var: &str) -> PgPool {
    let url = std::env::var(var)
        .unwrap_or_else(|_| panic!("{var} belum diset; jalankan lewat `make -C backend uji-pg`"));
    PgPool::connect(&url)
        .await
        .expect("database tidak dapat dihubungi")
}

fn bootstrap(username: &str) -> Perintah {
    Perintah::Bootstrap {
        username: username.to_owned(),
        alasan: Alasan::baru("bootstrap lingkungan uji ARC-55").unwrap(),
    }
}

#[tokio::test]
#[ignore]
async fn bootstrap_hanya_saat_tabel_pengguna_kosong() {
    let admin = pool("POOLROUTER_TEST_DATABASE_URL").await;
    let su = pool("POOLROUTER_TEST_DATABASE_URL_SU").await;
    let jumlah: i64 = sqlx::query_scalar("SELECT count(*) FROM admin_user")
        .fetch_one(&su)
        .await
        .unwrap();
    assert_eq!(
        jumlah, 0,
        "prasyarat: admin_user harus kosong; binary ini wajib jalan pertama di uji-pg-cargo"
    );

    let k = Konteks {
        pool: admin,
        env: Environment::Ci,
        seed_password: None,
        sekarang: Utc::now(),
    };
    let mut out = Vec::new();
    jalankan(
        bootstrap("admin-pertama"),
        &k,
        &mut PasswordSintetis,
        &mut out,
    )
    .await
    .expect("bootstrap pada tabel kosong");
    let teks = String::from_utf8(out).unwrap();
    assert!(teks.contains("admin-pertama"));
    assert!(!teks.contains(PASSWORD), "password tidak boleh tercetak");

    let (aktif, hash, peran): (bool, String, String) = sqlx::query_as(
        "SELECT u.aktif, u.password_hash, p.peran FROM admin_user u \
         JOIN admin_user_peran p USING (pengguna_id) WHERE u.username = 'admin-pertama'",
    )
    .fetch_one(&su)
    .await
    .unwrap();
    assert!(aktif);
    assert!(hash.starts_with("$argon2id$"));
    assert_eq!(peran, "platform_admin");

    let (akses, konfigurasi): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM audit_akses WHERE aksi = 'ubah_peran' \
                   AND filter_kueri->>'target' = 'admin-pertama'), \
                (SELECT count(*) FROM audit_konfigurasi WHERE entitas_jenis = 'admin_user' \
                   AND nilai_baru->>'username' = 'admin-pertama')",
    )
    .fetch_one(&su)
    .await
    .unwrap();
    assert_eq!((akses, konfigurasi), (1, 1));

    // Bootstrap kedua ditolak karena tabel sudah berisi, dan tidak menulis apa pun.
    let mut out = Vec::new();
    let e = jalankan(
        bootstrap("admin-kedua"),
        &k,
        &mut PasswordSintetis,
        &mut out,
    )
    .await
    .unwrap_err();
    assert!(matches!(e, GalatCli::Kelola(GalatKelola::BootstrapDitolak)));
    assert_ne!(e.kode_keluar(), 0);
    let ada: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM admin_user WHERE username = 'admin-kedua')",
    )
    .fetch_one(&su)
    .await
    .unwrap();
    assert!(!ada);
}
