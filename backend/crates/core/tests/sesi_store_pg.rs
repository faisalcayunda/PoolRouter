//! `SesiStore` against a real PostgreSQL (SHAA-196, ADR-0003 rev 5 §3.6f).
//! Run by `make -C backend uji-pg`; `#[ignore]`d because it needs
//! `POOLROUTER_TEST_DATABASE_URL` (role `pr_admin_api`). The clock is a
//! parameter, so every test moves time forward instead of sleeping.

use chrono::{DateTime, Duration, TimeZone, Utc};
use poolrouter_core::admin::sesi::{ABSOLUT_DETIK, IDLE_DETIK};
use poolrouter_core::admin::sesi_store::{SesiDitolak, SesiStore};
use poolrouter_core::admin::{SesiKedaluwarsa, TokenSesi};
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> PgPool {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL").expect(
        "POOLROUTER_TEST_DATABASE_URL belum diset; jalankan lewat `make -C backend uji-pg`",
    );
    PgPool::connect(&url)
        .await
        .expect("database tidak dapat dihubungi")
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 8, 0, 0).unwrap()
}

fn menit(m: i64) -> Duration {
    Duration::minutes(m)
}

/// A fresh synthetic user per test, so tests do not share sessions.
async fn pengguna(pool: &PgPool, aktif: bool) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO admin_user (pengguna_id, username, password_hash, aktif) VALUES ($1, $2, 'x', $3)",
    )
    .bind(id)
    .bind(format!("sintetis-{}", id.simple()))
    .bind(aktif)
    .execute(pool)
    .await
    .expect("insert admin_user");
    id
}

async fn terbit(
    store: &SesiStore,
    pool: &PgPool,
) -> (Uuid, poolrouter_core::admin::token::HashToken) {
    let uid = pengguna(pool, true).await;
    let hash = TokenSesi::baru().hash();
    store
        .terbitkan(uid, &hash, Some("uji"), t0())
        .await
        .expect("terbitkan");
    (uid, hash)
}

#[tokio::test]
#[ignore]
async fn get_sesi_berulang_tidak_menunda_berakhirnya_sesi() {
    let pool = pool().await;
    let store = SesiStore::baru(pool.clone());
    let (_, hash) = terbit(&store, &pool).await;

    for m in [5, 10, 20, 29] {
        let s = store.status(&hash, t0() + menit(m)).await.unwrap().unwrap();
        assert_eq!(
            s.idle_berakhir_pada,
            t0() + menit(30),
            "poll pada +{m} menit"
        );
    }
    let mati = store.status(&hash, t0() + menit(30)).await.unwrap();
    assert_eq!(mati, Err(SesiDitolak::Kedaluwarsa(SesiKedaluwarsa::Idle)));
}

#[tokio::test]
#[ignore]
async fn perpanjang_menggeser_idle_saja() {
    let pool = pool().await;
    let store = SesiStore::baru(pool.clone());
    let (_, hash) = terbit(&store, &pool).await;

    let s = store
        .perpanjang(&hash, t0() + menit(20))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.idle_berakhir_pada, t0() + menit(50));
    assert_eq!(
        s.absolut_berakhir_pada,
        t0() + Duration::seconds(ABSOLUT_DETIK)
    );
    assert_eq!(s.waktu_server, t0() + menit(20));

    // The write is real: still alive at +49, dead at +50.
    assert!(store.status(&hash, t0() + menit(49)).await.unwrap().is_ok());
    assert_eq!(
        store.status(&hash, t0() + menit(50)).await.unwrap(),
        Err(SesiDitolak::Kedaluwarsa(SesiKedaluwarsa::Idle))
    );
}

#[tokio::test]
#[ignore]
async fn perpanjang_tidak_mengubah_dibuat_kedaluwarsa_dan_token() {
    let pool = pool().await;
    let store = SesiStore::baru(pool.clone());
    let (uid, hash) = terbit(&store, &pool).await;
    let sebelum: (DateTime<Utc>, DateTime<Utc>, String) = sqlx::query_as(
        "SELECT dibuat_at, kedaluwarsa_at, encode(token_sha256, 'hex') FROM admin_session WHERE pengguna_id = $1",
    )
    .bind(uid)
    .fetch_one(&pool)
    .await
    .unwrap();

    store
        .perpanjang(&hash, t0() + menit(10))
        .await
        .unwrap()
        .unwrap();

    let sesudah: (DateTime<Utc>, DateTime<Utc>, String) = sqlx::query_as(
        "SELECT dibuat_at, kedaluwarsa_at, encode(token_sha256, 'hex') FROM admin_session WHERE pengguna_id = $1",
    )
    .bind(uid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(sebelum, sesudah);
}

#[tokio::test]
#[ignore]
async fn mendekati_batas_absolut_idle_sama_dengan_absolut() {
    let pool = pool().await;
    let store = SesiStore::baru(pool.clone());
    let (_, hash) = terbit(&store, &pool).await;

    // Keep the session alive every 25 minutes up to 10 minutes before the cap.
    let mut m = 25;
    while m < ABSOLUT_DETIK / 60 - 10 {
        store
            .perpanjang(&hash, t0() + menit(m))
            .await
            .unwrap()
            .unwrap();
        m += 25;
    }
    let sekarang = t0() + Duration::seconds(ABSOLUT_DETIK) - menit(10);
    let s = store.perpanjang(&hash, sekarang).await.unwrap().unwrap();
    assert_eq!(s.idle_berakhir_pada, s.absolut_berakhir_pada);

    // After the cap, extension is 401 and nothing resurrects the session.
    let lewat = t0() + Duration::seconds(ABSOLUT_DETIK);
    assert_eq!(
        store.perpanjang(&hash, lewat).await.unwrap(),
        Err(SesiDitolak::Kedaluwarsa(SesiKedaluwarsa::Absolut))
    );
    assert_eq!(
        store.autentikasi(&hash, lewat).await.unwrap().err(),
        Some(SesiDitolak::Kedaluwarsa(SesiKedaluwarsa::Absolut))
    );
}

#[tokio::test]
#[ignore]
async fn autentikasi_menggeser_idle_dan_sesi_idle_tidak_hidup_lagi() {
    let pool = pool().await;
    let store = SesiStore::baru(pool.clone());
    let (_, hash) = terbit(&store, &pool).await;

    store
        .autentikasi(&hash, t0() + menit(29))
        .await
        .unwrap()
        .unwrap();
    assert!(store.status(&hash, t0() + menit(58)).await.unwrap().is_ok());
    assert_eq!(
        store
            .autentikasi(&hash, t0() + menit(59))
            .await
            .unwrap()
            .err(),
        Some(SesiDitolak::Kedaluwarsa(SesiKedaluwarsa::Idle))
    );
    // Once idle-expired, a later request cannot bring it back.
    assert_eq!(
        store.status(&hash, t0() + menit(60)).await.unwrap(),
        Err(SesiDitolak::Kedaluwarsa(SesiKedaluwarsa::Idle))
    );
    let _ = IDLE_DETIK;
}

#[tokio::test]
#[ignore]
async fn cabut_dan_pengguna_nonaktif_membuat_token_tidak_dikenal() {
    let pool = pool().await;
    let store = SesiStore::baru(pool.clone());

    let (_, hash) = terbit(&store, &pool).await;
    assert!(store.cabut(&hash).await.unwrap());
    assert!(!store.cabut(&hash).await.unwrap());
    assert_eq!(
        store.status(&hash, t0()).await.unwrap(),
        Err(SesiDitolak::TidakDikenal)
    );

    let (uid, hash2) = terbit(&store, &pool).await;
    sqlx::query("UPDATE admin_user SET aktif = false WHERE pengguna_id = $1")
        .bind(uid)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        store.autentikasi(&hash2, t0()).await.unwrap().err(),
        Some(SesiDitolak::TidakDikenal)
    );
}
