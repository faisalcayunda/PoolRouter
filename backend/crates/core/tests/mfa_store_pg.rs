//! Integrasi modul MFA ke PostgreSQL sungguhan (SHAA-196). Dijalankan via `make -C backend uji-pg`.
//! Semua tes `#[ignore]` karena butuh `POOLROUTER_TEST_DATABASE_URL` dan container Postgres
//! yang sudah menjalankan migrasi. Jam adalah parameter, jadi tidak ada `sleep`.
//! Semua nilai di sini sintetis.

use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use poolrouter_core::admin::mfa::totp;
use poolrouter_core::admin::mfa::{
    GalatMfa, HasilPemulihan, HasilPeriksa, HasilVerifikasi, MfaStore,
};
use poolrouter_core::secret::Brankas;

// ───────────────────────────────────────── helpers ─────────────────────────────────────────

async fn pool() -> PgPool {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL")
        .expect("POOLROUTER_TEST_DATABASE_URL belum diset; jalankan via make uji-pg");
    PgPool::connect(&url)
        .await
        .expect("database tidak dapat dihubungi")
}

/// Reads of `audit_akses` need the superuser connection: `pr_admin_api` only has INSERT.
async fn pool_su() -> PgPool {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL_SU")
        .expect("POOLROUTER_TEST_DATABASE_URL_SU belum diset; jalankan via make uji-pg");
    PgPool::connect(&url)
        .await
        .expect("database su tidak dapat dihubungi")
}

/// Synthetic clock at TOTP step 100 (avoid step 0 colliding with `langkah_terakhir` DEFAULT 0).
fn t0() -> DateTime<Utc> {
    Utc.timestamp_opt(100 * totp::LANGKAH_DETIK, 0).unwrap()
}

fn store(pool: &PgPool) -> MfaStore {
    let brankas = Brankas::dari_isi_kek(&"11".repeat(32)).expect("KEK sintetis valid");
    MfaStore::baru(pool.clone(), brankas)
}

/// Synthetic admin user with `platform_admin` role (audit path and role lookup).
async fn pengguna(pool: &PgPool) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO admin_user (pengguna_id, username, password_hash, aktif) \
         VALUES ($1, $2, 'x', true)",
    )
    .bind(id)
    .bind(format!("sintetis-{id}"))
    .execute(pool)
    .await
    .expect("insert admin_user");
    sqlx::query(
        "INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh) \
         VALUES ($1, 'platform_admin', $1)",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("insert peran");
    id
}

// ───────────────────────────────────────── AC-4 ─────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn ac4_tolerance_window_accepts_minus_zero_plus_one() {
    const SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let detik = 1_111_111_111_i64;
    let langkah = detik / totp::LANGKAH_DETIK;

    let kode = totp::hitung(SECRET, langkah as u64);
    let sekarang = Utc.timestamp_opt(detik, 0).unwrap();
    assert_eq!(totp::cocok(SECRET, &kode, sekarang), Some(langkah as u64));

    let kode_lama = totp::hitung(SECRET, (langkah - 1) as u64);
    assert_eq!(
        totp::cocok(SECRET, &kode_lama, sekarang),
        Some((langkah - 1) as u64)
    );
    let kode_depan = totp::hitung(SECRET, (langkah + 1) as u64);
    assert_eq!(
        totp::cocok(SECRET, &kode_depan, sekarang),
        Some((langkah + 1) as u64)
    );
    let kode_min2 = totp::hitung(SECRET, (langkah - 2) as u64);
    assert_eq!(totp::cocok(SECRET, &kode_min2, sekarang), None);
    let kode_plus2 = totp::hitung(SECRET, (langkah + 2) as u64);
    assert_eq!(totp::cocok(SECRET, &kode_plus2, sekarang), None);
}

// ───────────────────────────────────────── AC-3 ─────────────────────────────────────────

#[derive(Clone)]
struct PenangkapLog(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for PenangkapLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::writer::MakeWriter<'a> for PenangkapLog {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
#[ignore]
async fn ac3_no_secret_leak_in_tracing_logs() {
    use tracing_subscriber::filter::LevelFilter;

    let penangkap = PenangkapLog(Arc::new(std::sync::Mutex::new(Vec::new())));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(LevelFilter::DEBUG)
        .with_writer(penangkap.clone())
        .with_ansi(false)
        .finish();

    let _guard = tracing::subscriber::set_default(subscriber);

    let pool = pool().await;
    let s = store(&pool);
    let id = pengguna(&pool).await;

    let daftar = s.daftar(id).await.expect("daftar");
    let rahasia = daftar.rahasia().to_string();
    let kode = totp::hitung(daftar.rahasia(), 100);
    let pemulihan = s.konfirmasi(id, &kode, t0()).await.expect("konfirmasi");
    let kode_pemulihan = pemulihan.daftar()[0].clone();
    let _ = s
        .pakai_kode_pemulihan(id, &kode_pemulihan, "10.0.0.1", "sesi-uji", t0())
        .await;

    let penerbit = pengguna(&pool).await;
    let target = pengguna(&pool).await;
    let (_id, daftar_kode) = s
        .terbit_kode_daftar(penerbit, target, t0())
        .await
        .expect("kode daftar");

    drop(_guard);

    let logs = String::from_utf8_lossy(&penangkap.0.lock().unwrap()).to_string();
    for nilai in [
        rahasia.as_str(),
        kode.as_str(),
        kode_pemulihan.as_str(),
        daftar_kode.expose(),
    ] {
        assert!(
            !logs.contains(nilai),
            "log membocorkan nilai sintetis: {nilai}"
        );
    }
}

// ───────────────────────────────────────── AC-5 ─────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn ac5_parallel_verification_exactly_one_accept() {
    let pool = pool().await;
    let s = store(&pool);
    let id = pengguna(&pool).await;

    let daftar = s.daftar(id).await.expect("daftar");
    let kode_100 = totp::hitung(daftar.rahasia(), 100);
    s.konfirmasi(id, &kode_100, t0()).await.expect("konfirmasi");

    // Step 101 is in window of t0+30s; 10 verifikasi paralel: hanya satu diterima.
    let kode_101 = totp::hitung(daftar.rahasia(), 101);
    let sekarang = t0() + Duration::seconds(30);

    let handles: Vec<_> = (0..10)
        .map(|_| {
            let s = s.clone();
            let kode = kode_101.clone();
            tokio::spawn(async move { s.verifikasi(id, &kode, sekarang).await })
        })
        .collect();

    let mut diterima = 0;
    for h in handles {
        if let HasilVerifikasi::Terima { langkah } = h.await.expect("task") {
            assert_eq!(langkah, 101);
            diterima += 1;
        }
    }
    assert_eq!(diterima, 1, "anti-replay: tepat satu yang diterima");
}

#[tokio::test]
#[ignore]
async fn ac5_older_step_rejected_even_within_window() {
    let pool = pool().await;
    let s = store(&pool);
    let id = pengguna(&pool).await;

    let daftar = s.daftar(id).await.expect("daftar");
    let kode_100 = totp::hitung(daftar.rahasia(), 100);
    s.konfirmasi(id, &kode_100, t0()).await.expect("konfirmasi");

    // Step 100 sudah terpakai (langkah_terakhir=100), jadi ditolak walau masih di jendela.
    let hasil = s
        .verifikasi(id, &kode_100, t0() + Duration::seconds(15))
        .await;
    assert!(matches!(
        hasil,
        HasilVerifikasi::Tolak(GalatMfa::KodeTidakCocok)
    ));
}

// ───────────────────────────────────────── AC-6 ─────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn ac6_ciphertext_dari_pengguna_lain_gagal_didekripsi() {
    let pool = pool().await;
    let s = store(&pool);
    let a = pengguna(&pool).await;
    let b = pengguna(&pool).await;

    let daftar_a = s.daftar(a).await.expect("daftar A");
    let _daftar_b = s.daftar(b).await.expect("daftar B");

    // Salin ciphertext A ke baris B (simulasi penyalinan baris).
    sqlx::query(
        "UPDATE pengguna_mfa SET secret_ciphertext=a.secret_ciphertext, \
                secret_dek_terbungkus=a.secret_dek_terbungkus, secret_nonce=a.secret_nonce, kek_id=a.kek_id \
         FROM pengguna_mfa a WHERE pengguna_mfa.pengguna_id=$1 AND a.pengguna_id=$2",
    )
    .bind(b)
    .bind(a)
    .execute(&pool)
    .await
    .expect("salin amplop");

    let kode_a = totp::hitung(daftar_a.rahasia(), 100);
    let hasil = s.konfirmasi(b, &kode_a, t0()).await;
    assert!(
        matches!(hasil, Err(GalatMfa::Brankas(_))),
        "AAD pengguna_id harus menolak amplop asing"
    );
}

// ───────────────────────────────────────── AC-7 ─────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn ac7_kode_pemulihan_satu_kali_dan_audit() {
    use metrics_exporter_prometheus::PrometheusBuilder;

    let pool = pool().await;
    let s = store(&pool);
    let id = pengguna(&pool).await;
    let daftar = s.daftar(id).await.expect("daftar");
    let kode_100 = totp::hitung(daftar.rahasia(), 100);
    let kode = s.konfirmasi(id, &kode_100, t0()).await.expect("konfirmasi");
    let contoh = kode.daftar()[0].clone();

    let handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("metrics recorder sekali per proses");

    let handles: Vec<_> = (0..10)
        .map(|_| {
            let s = s.clone();
            let kode = contoh.clone();
            tokio::spawn(async move {
                s.pakai_kode_pemulihan(id, &kode, "10.0.0.1", "sesi-uji", t0())
                    .await
            })
        })
        .collect();

    let mut diterima = 0;
    for h in handles {
        if let HasilPemulihan::Terima {
            sisa_kode_pemulihan,
        } = h.await.expect("task")
        {
            assert_eq!(sisa_kode_pemulihan, 9);
            diterima += 1;
        }
    }
    assert_eq!(diterima, 1, "single-use: tepat satu yang diterima");

    let su = pool_su().await;
    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_akses WHERE pengguna_id=$1 AND aksi='pakai_kode_pemulihan'",
    )
    .bind(id)
    .fetch_one(&su)
    .await
    .expect("hitung audit");
    assert_eq!(audit, 1, "satu baris audit_akses");

    let filter: serde_json::Value = sqlx::query_scalar(
        "SELECT filter_kueri FROM audit_akses WHERE pengguna_id=$1 AND aksi='pakai_kode_pemulihan'",
    )
    .bind(id)
    .fetch_one(&su)
    .await
    .expect("filter kueri");
    assert!(
        filter.get("kode_id").is_some(),
        "filter_kueri.kode_id wajib ada"
    );

    let snapshot = handle.render();
    assert!(
        snapshot.contains("poolrouter_mfa_kode_pemulihan_total 1"),
        "metrik naik tepat satu: {snapshot}"
    );
}

// ───────────────────────────────────────── AC-8 ─────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn ac8_kode_daftar_kedaluwarsa_dan_terikat_target() {
    let pool = pool().await;
    let s = store(&pool);
    let penerbit = pengguna(&pool).await;
    let target = pengguna(&pool).await;

    let (_id, kode) = s
        .terbit_kode_daftar(penerbit, target, t0())
        .await
        .expect("terbit");

    assert!(matches!(
        s.periksa_kode_daftar(target, kode.expose(), t0()).await,
        HasilPeriksa::Berlaku
    ));
    assert!(matches!(
        s.periksa_kode_daftar(target, kode.expose(), t0() + Duration::hours(25))
            .await,
        HasilPeriksa::TidakBerlaku(GalatMfa::KodeDaftarTidakBerlaku)
    ));
}

#[tokio::test]
#[ignore]
async fn ac8_kode_daftar_pengguna_lain_ditolak() {
    let pool = pool().await;
    let s = store(&pool);
    let penerbit = pengguna(&pool).await;
    let target = pengguna(&pool).await;
    let lain = pengguna(&pool).await;

    let (_id, kode) = s
        .terbit_kode_daftar(penerbit, target, t0())
        .await
        .expect("terbit");
    assert!(matches!(
        s.periksa_kode_daftar(lain, kode.expose(), t0()).await,
        HasilPeriksa::TidakBerlaku(GalatMfa::KodeDaftarTidakBerlaku)
    ));
}

#[tokio::test]
#[ignore]
async fn ac8_penerbit_tidak_boleh_menjadi_target() {
    let pool = pool().await;
    let s = store(&pool);
    let id = pengguna(&pool).await;

    let hasil = s.terbit_kode_daftar(id, id, t0()).await;
    assert!(matches!(hasil, Err(GalatMfa::PenerbitSamaDenganTarget)));
}

#[tokio::test]
#[ignore]
async fn ac8_lima_kegagalan_menggugurkan_kode() {
    let pool = pool().await;
    let s = store(&pool);
    let penerbit = pengguna(&pool).await;
    let target = pengguna(&pool).await;

    let (_id, kode) = s
        .terbit_kode_daftar(penerbit, target, t0())
        .await
        .expect("terbit");
    for _ in 0..5 {
        assert!(matches!(
            s.periksa_kode_daftar(target, "salah-sintetis", t0()).await,
            HasilPeriksa::TidakBerlaku(GalatMfa::KodeDaftarTidakBerlaku)
        ));
    }

    let alasan: Option<String> =
        sqlx::query_scalar("SELECT gugur_alasan FROM kode_daftar_mfa WHERE kode_id=$1")
            .bind(_id)
            .fetch_one(&pool)
            .await
            .expect("gugur_alasan");
    assert_eq!(alasan.as_deref(), Some("gagal_5x"));
    assert!(matches!(
        s.periksa_kode_daftar(target, kode.expose(), t0()).await,
        HasilPeriksa::TidakBerlaku(GalatMfa::KodeDaftarTidakBerlaku)
    ));
}

#[tokio::test]
#[ignore]
async fn ac8_kode_aktif_lama_diganti() {
    let pool = pool().await;
    let s = store(&pool);
    let penerbit = pengguna(&pool).await;
    let target = pengguna(&pool).await;

    let (id_lama, _kode_lama) = s
        .terbit_kode_daftar(penerbit, target, t0())
        .await
        .expect("terbit lama");
    let (_id_baru, _kode_baru) = s
        .terbit_kode_daftar(penerbit, target, t0() + Duration::seconds(5))
        .await
        .expect("terbit baru");

    let alasan: Option<String> =
        sqlx::query_scalar("SELECT gugur_alasan FROM kode_daftar_mfa WHERE kode_id=$1")
            .bind(id_lama)
            .fetch_one(&pool)
            .await
            .expect("gugur_alasan");
    assert_eq!(alasan.as_deref(), Some("diganti"));
}
