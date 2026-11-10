//! Bootstrap platform admin pertama (AC-39), termasuk beberapa bootstrap bersamaan (ARC-238),
//! terhadap PostgreSQL sungguhan sebagai `pr_admin_api`.
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

const NILAI_UJI: &str = "nilai-uji-sintetis-qx-01";

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

fn bootstrap(username: &str) -> Perintah {
    Perintah::Bootstrap {
        username: username.to_owned(),
        alasan: Alasan::baru("bootstrap lingkungan uji ARC-55").unwrap(),
    }
}

/// Satu percobaan bootstrap dengan keluarannya sendiri, supaya beberapa bisa jalan bersamaan.
async fn coba(k: &Konteks, username: &str) -> (String, Result<(), GalatCli>, Vec<u8>) {
    let mut out = Vec::new();
    let r = jalankan(bootstrap(username), k, &mut PasswordSintetis, &mut out).await;
    (username.to_owned(), r, out)
}

#[tokio::test]
#[ignore]
async fn bootstrap_bersamaan_hanya_satu_berhasil_dan_hanya_saat_tabel_kosong() {
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

    // Empat bootstrap bersamaan pada tabel kosong (advisory lock lalu `EXISTS` pada READ COMMITTED):
    // tepat satu menang, sisanya ditolak. Tanpa lock, semuanya melihat tabel kosong dan semuanya masuk.
    // Trigger sementara menunda penyisipan admin_user 0,3 detik supaya jendela balapan lebar dan uji
    // ini gagal bila lock dibuang, bukan bergantung pada nasib penjadwalan.
    sqlx::query(
        "CREATE FUNCTION uji_bootstrap_lambat() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN PERFORM pg_sleep(0.3); RETURN NEW; END $$",
    )
    .execute(&su)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER uji_bootstrap_lambat BEFORE INSERT ON admin_user \
         FOR EACH ROW EXECUTE FUNCTION uji_bootstrap_lambat()",
    )
    .execute(&su)
    .await
    .unwrap();
    let k = Konteks {
        pool: admin,
        env: Environment::Ci,
        seed_password: None,
        sekarang: Utc::now(),
    };
    let hasil = tokio::join!(
        coba(&k, "admin-balap-0"),
        coba(&k, "admin-balap-1"),
        coba(&k, "admin-balap-2"),
        coba(&k, "admin-balap-3"),
    );
    let hasil = [hasil.0, hasil.1, hasil.2, hasil.3];
    sqlx::query("DROP TRIGGER uji_bootstrap_lambat ON admin_user")
        .execute(&su)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION uji_bootstrap_lambat()")
        .execute(&su)
        .await
        .unwrap();
    let menang: Vec<_> = hasil.iter().filter(|(_, r, _)| r.is_ok()).collect();
    assert_eq!(menang.len(), 1, "tepat satu bootstrap bersamaan berhasil");
    for (nama, r, out) in &hasil {
        if let Err(e) = r {
            assert!(
                matches!(e, GalatCli::Kelola(GalatKelola::BootstrapDitolak)),
                "{nama}: {e}"
            );
            assert_ne!(e.kode_keluar(), 0);
            assert!(
                out.is_empty(),
                "bootstrap yang ditolak tidak mencetak apa pun"
            );
        }
    }
    let (pemenang, _, out) = menang[0];
    let teks = String::from_utf8(out.clone()).unwrap();
    assert!(teks.contains(pemenang));
    assert!(!teks.contains(NILAI_UJI), "password tidak boleh tercetak");
    let semua: Vec<String> = sqlx::query_scalar("SELECT username FROM admin_user")
        .fetch_all(&su)
        .await
        .unwrap();
    assert_eq!(
        semua,
        vec![pemenang.clone()],
        "tabel pengguna berisi satu admin"
    );

    let (aktif, hash, peran): (bool, String, String) = sqlx::query_as(
        "SELECT u.aktif, u.password_hash, p.peran FROM admin_user u \
         JOIN admin_user_peran p USING (pengguna_id) WHERE u.username = $1",
    )
    .bind(pemenang)
    .fetch_one(&su)
    .await
    .unwrap();
    assert!(aktif);
    assert!(hash.starts_with("$argon2id$"));
    assert_eq!(peran, "platform_admin");

    let (akses, konfigurasi): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM audit_akses WHERE aksi = 'ubah_peran' \
                   AND filter_kueri->>'target' = $1), \
                (SELECT count(*) FROM audit_konfigurasi WHERE entitas_jenis = 'admin_user' \
                   AND nilai_baru->>'username' = $1)",
    )
    .bind(pemenang)
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
