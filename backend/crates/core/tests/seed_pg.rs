//! Seed of synthetic users against a real PostgreSQL (SHAA-196, D-4). Run by
//! `make -C backend uji-pg`; `#[ignore]`d because it needs
//! `POOLROUTER_TEST_DATABASE_URL` (role `pr_admin_api`).

use poolrouter_core::admin::password::verifikasi_password;
use poolrouter_core::admin::seed::{GalatSeed, LaporanSeed, jalankan};
use poolrouter_core::config::Environment;
use sqlx::PgPool;

const PASSWORD: &str = "kata-sandi-seed-sintetis";

async fn pool() -> PgPool {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL").expect(
        "POOLROUTER_TEST_DATABASE_URL belum diset; jalankan lewat `make -C backend uji-pg`",
    );
    PgPool::connect(&url).await.expect("database")
}

async fn hitung(pool: &PgPool, sql: impl AsRef<str>) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_ref().to_owned()))
        .fetch_one(pool)
        .await
        .unwrap()
}

const N_PENGGUNA: &str = "SELECT count(*) FROM admin_user WHERE username LIKE 'seed-%'";
const N_PERAN: &str = "SELECT count(*) FROM admin_user_peran p JOIN admin_user u USING (pengguna_id) \
                       WHERE u.username LIKE 'seed-%'";
const N_CLIENT: &str = "SELECT count(*) FROM client WHERE nama = 'seed-client-sintetis'";

#[tokio::test]
#[ignore]
async fn seed_ditolak_di_luar_dev_dan_ci_tanpa_menyentuh_basis_data() {
    let pool = pool().await;
    let sebelum = (
        hitung(&pool, N_PENGGUNA).await,
        hitung(&pool, N_PERAN).await,
        hitung(&pool, N_CLIENT).await,
    );
    for env in [Environment::Staging, Environment::Poc, Environment::Prod] {
        let r = jalankan(&pool, env, PASSWORD).await;
        assert!(
            matches!(r, Err(GalatSeed::LingkunganDitolak(e)) if e == env),
            "{env:?}"
        );
    }
    let sesudah = (
        hitung(&pool, N_PENGGUNA).await,
        hitung(&pool, N_PERAN).await,
        hitung(&pool, N_CLIENT).await,
    );
    assert_eq!(sebelum, sesudah);
}

#[tokio::test]
#[ignore]
async fn seed_menolak_password_pendek() {
    let pool = pool().await;
    let r = jalankan(&pool, Environment::Dev, "pendek").await;
    assert!(matches!(r, Err(GalatSeed::Password(_))));
}

#[tokio::test]
#[ignore]
async fn seed_membuat_satu_pengguna_per_peran_dan_idempoten() {
    let pool = pool().await;
    for env in [Environment::Dev, Environment::Ci] {
        // First run of the test DB adds rows; a later one (or `ci` after `dev`) adds none.
        jalankan(&pool, env, PASSWORD).await.expect("seed");
    }
    let lagi = jalankan(&pool, Environment::Dev, PASSWORD).await.unwrap();
    assert_eq!(
        lagi,
        LaporanSeed::default(),
        "seed kedua tidak menambah baris"
    );
    assert_eq!(hitung(&pool, N_PENGGUNA).await, 5);
    assert_eq!(hitung(&pool, N_PERAN).await, 5);
    assert_eq!(hitung(&pool, N_CLIENT).await, 1);

    // Each of the four roles has at least one user.
    for peran in ["platform_admin", "client_owner", "operator", "auditor"] {
        let n = hitung(
            &pool,
            &format!(
                "SELECT count(*) FROM admin_user_peran p JOIN admin_user u USING (pengguna_id) \
                 WHERE u.username LIKE 'seed-%' AND p.peran = '{peran}'"
            ),
        )
        .await;
        assert!(n >= 1, "peran {peran} tanpa pengguna");
    }
    // client_owner is bound to the synthetic client; the others are not.
    assert_eq!(
        hitung(
            &pool,
            "SELECT count(*) FROM admin_user u JOIN client c USING (client_id) \
             WHERE u.username = 'seed-client-owner' AND c.nama = 'seed-client-sintetis'"
        )
        .await,
        1
    );
    assert_eq!(
        hitung(
            &pool,
            "SELECT count(*) FROM admin_user WHERE username LIKE 'seed-%' AND client_id IS NOT NULL"
        )
        .await,
        1
    );
    // A-02 holds for operator and auditor: proposer, approver and subject differ.
    assert_eq!(
        hitung(
            &pool,
            "SELECT count(*) FROM admin_user_peran p JOIN admin_user u USING (pengguna_id) \
             WHERE u.username LIKE 'seed-%' AND p.peran IN ('operator','auditor') \
               AND p.diajukan_oleh <> p.pengguna_id AND p.disetujui_oleh <> p.pengguna_id \
               AND p.disetujui_oleh <> p.diajukan_oleh"
        )
        .await,
        2
    );
    // Exactly one user has the English UI.
    assert_eq!(
        hitung(
            &pool,
            "SELECT count(*) FROM admin_user WHERE username LIKE 'seed-%' AND bahasa_ui = 'en'"
        )
        .await,
        1
    );
    // The stored hash verifies the seed password and not another.
    let hash: String =
        sqlx::query_scalar("SELECT password_hash FROM admin_user WHERE username = 'seed-operator'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(verifikasi_password(PASSWORD, &hash));
    assert!(!verifikasi_password("bukan-kata-sandi-seed", &hash));
}
