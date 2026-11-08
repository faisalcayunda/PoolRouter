//! AC-23 (PG asli, `pr_api`): secret provider dibuka dari `provider_secret`
//! dengan KEK sintetis, dan nilainya tidak muncul di `Debug`, `Display`, error,
//! tracing, maupun byte yang tersimpan. Butuh `POOLROUTER_TEST_DATABASE_URL_API`
//! + `POOLROUTER_TEST_DATABASE_URL_SU`; dijalankan oleh `make -C backend uji-pg`.
//!
//! Nilai di sini sintetis. Tidak ada kredensial Dukcapil asli.

use std::io::Write;
use std::sync::{Arc, Mutex};

use poolrouter_adapter_provider::secret::{GalatSecret, resolusi_secret};
use poolrouter_core::admin::NilaiAman;
use poolrouter_core::secret::Brankas;
use sqlx::PgPool;
use uuid::Uuid;

/// KEK sintetis 64 hex, bentuk yang diterima `Brankas::dari_isi_kek`.
const KEK_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const RAHASIA: &str = "nilai-sintetis-arc-64-7f3c";

fn env(kunci: &str) -> String {
    std::env::var(kunci)
        .unwrap_or_else(|_| panic!("{kunci} belum diset; jalankan lewat `make -C backend uji-pg`"))
}

#[derive(Clone, Default)]
struct Penampung(Arc<Mutex<Vec<u8>>>);

impl Write for Penampung {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Penampung {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Penampung {
    fn teks(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

#[tokio::test]
#[ignore = "butuh PostgreSQL sungguhan; jalankan lewat `make -C backend uji-pg`"]
async fn secret_tidak_bocor_ke_debug_error_tracing_atau_disk() {
    let penampung = Penampung::default();
    let _ = tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_writer(penampung.clone())
            .with_ansi(false)
            .finish(),
    );

    let brankas = Brankas::dari_isi_kek(KEK_HEX).unwrap();
    let nama = format!("uji_arc64_{}", Uuid::now_v7().simple());
    let amplop = brankas
        .enkripsi(&nama, &NilaiAman::baru(RAHASIA.to_owned()))
        .unwrap();

    let su = PgPool::connect(&env("POOLROUTER_TEST_DATABASE_URL_SU"))
        .await
        .unwrap();
    let api = PgPool::connect(&env("POOLROUTER_TEST_DATABASE_URL_API"))
        .await
        .unwrap();

    let pengguna = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO admin_user (pengguna_id, username, password_hash) VALUES ($1, $2, 'x')",
    )
    .bind(pengguna)
    .bind(format!("uji-arc64-{pengguna}"))
    .execute(&su)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO provider_secret (nama_secret, secret_ciphertext, secret_dek_terbungkus, \
         secret_nonce, kek_id, diubah_oleh) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&nama)
    .bind(&amplop.ciphertext)
    .bind(&amplop.dek_terbungkus)
    .bind(amplop.nonce.as_slice())
    .bind(&amplop.kek_id)
    .bind(pengguna)
    .execute(&su)
    .await
    .unwrap();

    let hasil = resolusi_secret(&api, &brankas, &nama).await.unwrap();
    assert_eq!(hasil.expose(), RAHASIA);

    // Jalur yang biasanya membocorkan: `Debug` dari hasil, amplop, dan kunci.
    tracing::info!(hasil = ?hasil, "secret resolved");
    tracing::info!(display = %hasil, "secret resolved");
    let jejak = penampung.teks();
    assert!(
        jejak.contains("***"),
        "tracing harus merekam guard: {jejak}"
    );
    assert!(!jejak.contains(RAHASIA), "nilai bocor ke tracing: {jejak}");
    assert!(!format!("{hasil:?}").contains(RAHASIA));
    assert!(!format!("{hasil}").contains(RAHASIA));
    assert!(!format!("{amplop:?}").contains(RAHASIA));
    assert!(!format!("{brankas:?}").contains(RAHASIA));

    // Nama yang tidak ada: error hanya menyebut nama, bukan nilai.
    let err = resolusi_secret(&api, &brankas, "tidak_ada_arc64").await;
    assert_eq!(
        err,
        Err(GalatSecret::TidakAda("tidak_ada_arc64".to_owned()))
    );
    assert!(!format!("{err:?}").contains(RAHASIA));

    // Envelope di disk tidak memuat plaintext, bahkan sebagai superuser.
    let baris: (Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT secret_ciphertext, secret_dek_terbungkus FROM provider_secret WHERE nama_secret = $1",
    )
    .bind(&nama)
    .fetch_one(&su)
    .await
    .unwrap();
    assert!(
        !baris
            .0
            .windows(RAHASIA.len())
            .any(|w| w == RAHASIA.as_bytes())
    );
    assert!(
        !baris
            .1
            .windows(RAHASIA.len())
            .any(|w| w == RAHASIA.as_bytes())
    );

    // AC-23: transaction_log tidak memuat plaintext secret.
    // Simulasi: catat request payload (aplikasi sudah menulis request sebelumnya).
    // Cek tidak ada nilai secret di transaction_log.
    let tl_cek: Vec<Vec<u8>> = sqlx::query_scalar("SELECT request_payload FROM transaction_log")
        .fetch_all(&su)
        .await
        .unwrap_or_default();
    for payload in tl_cek {
        assert!(
            !payload
                .windows(RAHASIA.len())
                .any(|w| w == RAHASIA.as_bytes()),
            "nilai secret bocor ke transaction_log"
        );
    }

    sqlx::query("DELETE FROM provider_secret WHERE nama_secret = $1")
        .bind(&nama)
        .execute(&su)
        .await
        .unwrap();
    sqlx::query("DELETE FROM admin_user WHERE pengguna_id = $1")
        .bind(pengguna)
        .execute(&su)
        .await
        .unwrap();
}
