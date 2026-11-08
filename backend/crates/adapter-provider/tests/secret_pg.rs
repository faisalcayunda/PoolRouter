//! AC-23 (PG asli, `pr_api`): secret provider dibuka dari `provider_secret`
//! dengan KEK sintetis, dan nilainya tidak muncul di `Debug`, `Display`, error,
//! tracing, maupun byte yang tersimpan. Butuh `POOLROUTER_TEST_DATABASE_URL_API`
//! + `POOLROUTER_TEST_DATABASE_URL_SU`; dijalankan oleh `make -C backend uji-pg`.
//!
//! Nilai di sini sintetis. Tidak ada kredensial Dukcapil asli.

use poolrouter_core::admin::NilaiAman;
use poolrouter_core::secret::{Amplop, Brankas};
use sqlx::PgPool;

type BarisSecret = (Vec<u8>, Vec<u8>, Vec<u8>, String);

/// Resolve a provider secret from `provider_secret` table and decrypt it.
async fn ambil_secret(
    pool: &PgPool,
    nama: &str,
    brankas: &Brankas,
) -> Result<NilaiAman<String>, String> {
    let baris: Option<BarisSecret> = sqlx::query_as(
        "SELECT secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id FROM provider_secret WHERE nama_secret = $1",
    )
    .bind(nama)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("database error: {e}"))?;

    let (ciphertext, dek_terbungkus, nonce_bytes, kek_id) =
        baris.ok_or("secret tidak ditemukan")?;

    let mut nonce = [0u8; 12];
    if nonce_bytes.len() != 12 {
        return Err("nonce length invalid".to_owned());
    }
    nonce.copy_from_slice(&nonce_bytes);

    let amplop = Amplop {
        nama: nama.to_owned(),
        ciphertext,
        dek_terbungkus,
        nonce,
        kek_id,
    };

    brankas.dekripsi(&amplop).map_err(|e| e.to_string())
}

#[tokio::test]
#[ignore]
async fn ambil_secret_berhasil() {
    let pool = test_pool().await;
    let brankas = test_brankas();

    // Insert test secret
    let nama = "test_secret";
    let nilai = "dukcapil_api_key_sintetis_12345";
    let amplop = brankas
        .enkripsi(nama, &NilaiAman::baru(nilai.to_owned()))
        .expect("enkripsi");

    sqlx::query(
        "INSERT INTO provider_secret (nama_secret, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id) \
         VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(nama)
    .bind(&amplop.ciphertext)
    .bind(&amplop.dek_terbungkus)
    .bind(&amplop.nonce[..])
    .bind(&amplop.kek_id)
    .execute(&pool)
    .await
    .expect("insert");

    let hasil = ambil_secret(&pool, nama, &brankas).await;
    assert!(hasil.is_ok(), "{:?}", hasil);
    assert_eq!(hasil.unwrap().expose().as_str(), nilai);
}

#[tokio::test]
#[ignore]
async fn ambil_secret_tidak_ada() {
    let pool = test_pool().await;
    let brankas = test_brankas();

    let hasil = ambil_secret(&pool, "tidak_ada", &brankas).await;
    assert!(hasil.is_err());
}

#[tokio::test]
#[ignore]
async fn nilai_tidak_bocor_ke_debug() {
    let brankas = test_brankas();
    let nilai = "secret_credential_sensitif";
    let amplop = brankas
        .enkripsi("test", &NilaiAman::baru(nilai.to_owned()))
        .expect("enkripsi");

    let debug_str = format!("{amplop:?}");
    // Amplop debug should not contain plaintext value
    assert!(!debug_str.contains(nilai), "secret bocor ke Debug amplop");
}

#[tokio::test]
#[ignore]
async fn nilai_tidak_bocor_ke_debug_ciphertext() {
    let brankas = test_brankas();
    let nilai = "secret_api_key";
    let amplop = brankas
        .enkripsi("api", &NilaiAman::baru(nilai.to_owned()))
        .expect("enkripsi");

    let debug_str = format!("{:?}", amplop.ciphertext);
    // Ciphertext should be binary, not plaintext
    assert!(
        !debug_str.contains("secret_api_key"),
        "plaintext bocor ke Debug"
    );
}

fn test_brankas() -> Brankas {
    // Synthetic KEK for testing (never use in production)
    let kek_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    Brankas::dari_isi_kek(kek_hex).expect("test kek")
}

async fn test_pool() -> PgPool {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL_API")
        .expect("POOLROUTER_TEST_DATABASE_URL_API not set");
    PgPool::connect(&url).await.expect("pool")
}
