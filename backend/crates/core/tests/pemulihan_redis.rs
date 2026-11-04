//! Fail-closed, failover detection and rebuild against a real Redis
//! (ADR-0001 rev 15 section 5 and 5a.2; SHAA-195 acceptance criterion
//! "uji fail-closed ... terhadap Redis container").
//!
//! Every test is `#[ignore]`d: they need a Redis, so they run through
//! `make -C backend uji-redis`, which starts a throwaway container, sets
//! `POOLROUTER_TEST_REDIS`, and restarts the container between the two
//! restart phases so `run_id` really changes.
//!
//! Phases (`POOLROUTER_TEST_FASE`):
//! - unset: the tests that need no restart;
//! - `sebelum`: rebuild once, print `RUN_ID_LAMA=<id>`;
//! - `sesudah`: Redis restarted; with `POOLROUTER_TEST_RUN_ID_LAMA` set, prove
//!   dispatch stops, then the rebuild restores the counters.

use chrono::{DateTime, Utc};
use poolrouter_core::kuota::pemulihan::{
    GalatPemulihan, JendelaRate, KUNCI_SIAP, KeadaanPool, Kesiapan, SlotAktif, SumberRebuild,
    baca_run_id, bangun_ulang, kunci_antre, kunci_attempts, kunci_ember, kunci_retry, kunci_rl,
    kunci_slot, nomor_ember_absolut, periksa_kesiapan, pulihkan_bila_perlu,
};
use redis::Value;
use redis::aio::MultiplexedConnection;

const AMBIL_TOKEN: &str = include_str!("../src/kuota/lua/rakitan/kuota_ambil_token.lua");
const BACA: &str = include_str!("../src/kuota/lua/rakitan/kuota_baca.lua");
const INTAKE: &str = include_str!("../src/kuota/lua/rakitan/kuota_intake.lua");

const POOL: &str = "pool-uji";

async fn koneksi() -> MultiplexedConnection {
    let url = std::env::var("POOLROUTER_TEST_REDIS")
        .expect("POOLROUTER_TEST_REDIS belum diset; jalankan lewat `make -C backend uji-redis`");
    redis::Client::open(url)
        .expect("URL Redis tidak sah")
        .get_multiplexed_async_connection()
        .await
        .expect("Redis tidak dapat dihubungi")
}

fn fase() -> String {
    std::env::var("POOLROUTER_TEST_FASE").unwrap_or_default()
}

/// Fixed facts instead of PostgreSQL.
struct Tetap(Vec<KeadaanPool>);

impl SumberRebuild for Tetap {
    async fn keadaan(&self, _: DateTime<Utc>) -> Result<Vec<KeadaanPool>, GalatPemulihan> {
        Ok(self.0.clone())
    }
}

fn keadaan(sekarang: DateTime<Utc>) -> KeadaanPool {
    let e = nomor_ember_absolut(sekarang);
    KeadaanPool {
        pool_id: POOL.into(),
        periode_mulai: sekarang - chrono::Duration::hours(3),
        // The snapshot is behind the log: the rebuild must pick the log (larger).
        snapshot_attempts: 20,
        log_periode: 30,
        // 30 in the period, spread over the last buckets. One bucket is older
        // than the rolling window and must be dropped.
        log_ember: vec![(e, 10), (e - 3, 15), (e - 10, 5), (e - 400, 99)],
        antre: 4,
        slot: vec![
            SlotAktif {
                request_id: "req-a".into(),
                nomor_percobaan: 1,
            },
            SlotAktif {
                request_id: "req-b".into(),
                nomor_percobaan: 2,
            },
        ],
        ttl_slot_mikro: 120_000_000,
        jendela_rate: vec![JendelaRate {
            jenis: "tps".into(),
            t_mikro: 1_000_000,
            burst: 5,
        }],
        porsi_sync: 0.4,
    }
}

fn teks(v: &Value) -> String {
    match v {
        Value::BulkString(b) => String::from_utf8_lossy(b).into_owned(),
        Value::SimpleString(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        lain => panic!("bukan teks: {lain:?}"),
    }
}

fn angka(v: &Value) -> i64 {
    match v {
        Value::Int(i) => *i,
        lain => teks(lain).parse().expect("bukan angka"),
    }
}

/// `kuota_baca`: (kode, terpakai, attempts, slot_aktif, retry_after).
async fn baca(
    c: &mut MultiplexedConnection,
    run_id: &str,
    sekarang: DateTime<Utc>,
    periode_mulai: DateTime<Utc>,
    kuota: i64,
) -> Vec<Value> {
    redis::Script::new(BACA)
        .key(KUNCI_SIAP)
        .key(kunci_attempts(POOL, periode_mulai))
        .key(kunci_ember(POOL))
        .key(kunci_slot(POOL))
        .arg(run_id)
        .arg("bergulir")
        .arg(kuota)
        .arg(145)
        .arg(nomor_ember_absolut(sekarang))
        .invoke_async(c)
        .await
        .expect("kuota_baca gagal")
}

/// `kuota_ambil_token` for window tps (t=1s, burst=5).
async fn ambil_token(
    c: &mut MultiplexedConnection,
    run_id: &str,
    sekarang: DateTime<Utc>,
    periode_mulai: DateTime<Utc>,
    request_id: &str,
    kuota: i64,
) -> String {
    let hasil: Vec<Value> = redis::Script::new(AMBIL_TOKEN)
        .key(KUNCI_SIAP)
        .key(kunci_attempts(POOL, periode_mulai))
        .key(kunci_ember(POOL))
        .key(kunci_slot(POOL))
        .key(kunci_retry(POOL))
        .key(kunci_rl(POOL, "tps", false))
        .key(kunci_rl(POOL, "tps", true))
        .arg(run_id)
        .arg(request_id)
        .arg(1)
        .arg("bergulir")
        .arg(0)
        .arg(kuota)
        .arg(145)
        .arg(nomor_ember_absolut(sekarang))
        .arg(100)
        .arg(120_000_000_i64)
        .arg(0)
        .arg("0.4")
        .arg(1)
        .arg(1_000_000)
        .arg(5)
        .invoke_async(c)
        .await
        .expect("kuota_ambil_token gagal");
    teks(&hasil[0])
}

async fn bersihkan(c: &mut MultiplexedConnection) {
    redis::cmd("FLUSHALL")
        .query_async::<()>(c)
        .await
        .expect("FLUSHALL gagal");
}

/// Fresh Redis: not ready, scripts refuse, so dispatch and sync stop (section 5).
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn redis_baru_gagal_tertutup() {
    if !fase().is_empty() {
        return;
    }
    let mut c = koneksi().await;
    bersihkan(&mut c).await;
    let sekarang = Utc::now();
    let k = keadaan(sekarang);

    let run_id = baca_run_id(&mut c).await.unwrap();
    assert!(matches!(
        periksa_kesiapan(&mut c).await.unwrap(),
        Kesiapan::PerluRebuild { tercatat: None, .. }
    ));

    // Right run_id, but pr:siap was never written.
    assert_eq!(
        ambil_token(&mut c, &run_id, sekarang, k.periode_mulai, "req-1", 1000).await,
        "belum_siap"
    );
    // Wrong run_id.
    assert_eq!(
        ambil_token(
            &mut c,
            "bukan-run-id",
            sekarang,
            k.periode_mulai,
            "req-1",
            1000
        )
        .await,
        "belum_siap"
    );
    // Intake refuses as well, and nothing was written by the refusals.
    let intake: Vec<Value> = redis::Script::new(INTAKE)
        .key(KUNCI_SIAP)
        .key(kunci_attempts(POOL, k.periode_mulai))
        .key(kunci_ember(POOL))
        .key(kunci_antre(POOL))
        .arg(&run_id)
        .arg("bergulir")
        .arg(0)
        .arg("c1")
        .arg(1000)
        .arg(145)
        .arg(nomor_ember_absolut(sekarang))
        .invoke_async(&mut c)
        .await
        .unwrap();
    assert_eq!(teks(&intake[0]), "belum_siap");
    let kunci: Vec<String> = redis::cmd("KEYS")
        .arg("pr:*")
        .query_async(&mut c)
        .await
        .unwrap();
    assert!(kunci.is_empty(), "penolakan menulis kunci: {kunci:?}");
}

/// Rebuild restores exactly what section 5a.2 says, and only then does the
/// gate open.
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn rebuild_memulihkan_counter_dan_membuka_gerbang() {
    if !fase().is_empty() {
        return;
    }
    let mut c = koneksi().await;
    bersihkan(&mut c).await;
    let sekarang = Utc::now();
    let k = keadaan(sekarang);
    let run_id = baca_run_id(&mut c).await.unwrap();
    let sumber = Tetap(vec![k.clone()]);

    let ringkas = pulihkan_bila_perlu(&mut c, &sumber, sekarang)
        .await
        .unwrap()
        .expect("harus membangun ulang");
    assert_eq!(ringkas.run_id, run_id);
    assert_eq!(ringkas.attempts, vec![(POOL.to_owned(), 30)]);
    assert_eq!(
        periksa_kesiapan(&mut c).await.unwrap(),
        Kesiapan::Siap {
            run_id: run_id.clone()
        }
    );
    // Second tick is a no-op.
    assert!(
        pulihkan_bila_perlu(&mut c, &sumber, sekarang)
            .await
            .unwrap()
            .is_none()
    );

    // Counter = max(snapshot 20, log 30).
    let attempts: i64 = redis::cmd("GET")
        .arg(kunci_attempts(POOL, k.periode_mulai))
        .query_async(&mut c)
        .await
        .unwrap();
    assert_eq!(attempts, 30);
    let antre: i64 = redis::cmd("GET")
        .arg(kunci_antre(POOL))
        .query_async(&mut c)
        .await
        .unwrap();
    assert_eq!(antre, 4);

    // Rolling buckets: three inside the window, the 400-bucket-old one dropped.
    let ember: std::collections::HashMap<String, i64> = redis::cmd("HGETALL")
        .arg(kunci_ember(POOL))
        .query_async(&mut c)
        .await
        .unwrap();
    let e = nomor_ember_absolut(sekarang);
    assert_eq!(ember.len(), 3, "{ember:?}");
    assert_eq!(ember[&format!("b{e}")], 10);
    assert_eq!(ember[&format!("b{}", e - 3)], 15);
    assert_eq!(ember[&format!("b{}", e - 10)], 5);

    // Semaphore: one member per open attempt, member format D1.2.
    let slot: Vec<String> = redis::cmd("ZRANGE")
        .arg(kunci_slot(POOL))
        .arg(0)
        .arg(-1)
        .query_async(&mut c)
        .await
        .unwrap();
    assert_eq!(slot, vec!["req-a:1".to_owned(), "req-b:2".to_owned()]);

    // kuota_baca agrees: terpakai 30 (rolling), attempts 30, slot_aktif 2.
    let r = baca(&mut c, &run_id, sekarang, k.periode_mulai, 1000).await;
    assert_eq!(teks(&r[0]), "ok");
    assert_eq!(
        (angka(&r[1]), angka(&r[2]), angka(&r[3])),
        (30, 30, 2),
        "{r:?}"
    );

    // The rebuilt slots count against the quota: terpakai 30 + 2 slots = 32,
    // so a quota of 32 is already exhausted and a quota of 33 admits one call.
    assert_eq!(
        ambil_token(&mut c, &run_id, sekarang, k.periode_mulai, "req-n1", 32).await,
        "kuota_habis"
    );
    assert_eq!(
        ambil_token(&mut c, &run_id, sekarang, k.periode_mulai, "req-n2", 33).await,
        "ok"
    );
    // ... and then the quota is full again (30 + 3 slots).
    assert_eq!(
        ambil_token(&mut c, &run_id, sekarang, k.periode_mulai, "req-n3", 33).await,
        "kuota_habis"
    );
}

/// GCRA keys start with one token (D2.6): after recovery the first call passes
/// and the second is rate limited, instead of a full burst of 5.
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn bucket_rate_mulai_tanpa_burst_setelah_rebuild() {
    if !fase().is_empty() {
        return;
    }
    let mut c = koneksi().await;
    bersihkan(&mut c).await;
    let sekarang = Utc::now();
    let k = keadaan(sekarang);
    let run_id = baca_run_id(&mut c).await.unwrap();
    bangun_ulang(&mut c, &run_id, &Tetap(vec![k.clone()]), sekarang)
        .await
        .unwrap();

    let a = ambil_token(
        &mut c,
        &run_id,
        sekarang,
        k.periode_mulai,
        "req-r1",
        100_000,
    )
    .await;
    let b = ambil_token(
        &mut c,
        &run_id,
        sekarang,
        k.periode_mulai,
        "req-r2",
        100_000,
    )
    .await;
    assert_eq!((a.as_str(), b.as_str()), ("ok", "rate_limit"));
}

/// A rebuild for a `run_id` that is no longer the live one must not mark Redis
/// ready.
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn rebuild_run_id_usang_tidak_membuka_gerbang() {
    if !fase().is_empty() {
        return;
    }
    let mut c = koneksi().await;
    bersihkan(&mut c).await;
    let sekarang = Utc::now();
    let galat = bangun_ulang(
        &mut c,
        "run-id-lama",
        &Tetap(vec![keadaan(sekarang)]),
        sekarang,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(galat, GalatPemulihan::RunIdBerubah { .. }),
        "{galat}"
    );
    let siap: Option<String> = redis::cmd("GET")
        .arg(KUNCI_SIAP)
        .query_async(&mut c)
        .await
        .unwrap();
    assert_eq!(siap, None);
}

/// Phase 1 of the restart test: rebuild, check it is serving, remember run_id.
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn restart_fase_sebelum() {
    if fase() != "sebelum" {
        return;
    }
    let mut c = koneksi().await;
    bersihkan(&mut c).await;
    let sekarang = Utc::now();
    let k = keadaan(sekarang);
    let run_id = baca_run_id(&mut c).await.unwrap();
    bangun_ulang(&mut c, &run_id, &Tetap(vec![k.clone()]), sekarang)
        .await
        .unwrap();
    assert_eq!(
        ambil_token(&mut c, &run_id, sekarang, k.periode_mulai, "req-s1", 1000).await,
        "ok"
    );
    println!("RUN_ID_LAMA={run_id}");
}

/// Phase 2: Redis was restarted (state lost, new `run_id`). Dispatch must stop
/// for both the old and the new run_id, and the rebuild must bring the counter
/// back so the quota is still enforced.
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn restart_fase_sesudah() {
    if fase() != "sesudah" {
        return;
    }
    let lama = std::env::var("POOLROUTER_TEST_RUN_ID_LAMA").expect("run_id lama belum diteruskan");
    let mut c = koneksi().await;
    let sekarang = Utc::now();
    let k = keadaan(sekarang);

    let baru = baca_run_id(&mut c).await.unwrap();
    assert_ne!(lama, baru, "Redis tidak benar-benar restart: run_id sama");
    println!("fase sesudah: run_id lama {lama}, baru {baru}");
    assert!(matches!(
        periksa_kesiapan(&mut c).await.unwrap(),
        Kesiapan::PerluRebuild { tercatat: None, .. }
    ));
    // Dispatch is stopped for a caller holding the old run_id and for one that
    // already reconnected: no token, no quota decision.
    for id in [&lama, &baru] {
        assert_eq!(
            ambil_token(&mut c, id, sekarang, k.periode_mulai, "req-s2", 1000).await,
            "belum_siap"
        );
    }
    let kosong: Option<String> = redis::cmd("GET")
        .arg(kunci_attempts(POOL, k.periode_mulai))
        .query_async(&mut c)
        .await
        .unwrap();
    assert_eq!(kosong, None, "state hilang, belum dibangun ulang");

    // Rebuild: the counter comes back from PostgreSQL facts, not from zero.
    pulihkan_bila_perlu(&mut c, &Tetap(vec![k.clone()]), sekarang)
        .await
        .unwrap()
        .expect("harus membangun ulang");
    let r = baca(&mut c, &baru, sekarang, k.periode_mulai, 1000).await;
    assert_eq!((teks(&r[0]).as_str(), angka(&r[2])), ("ok", 30));
    // The old run_id stays refused after the rebuild.
    assert_eq!(
        ambil_token(&mut c, &lama, sekarang, k.periode_mulai, "req-s3", 1000).await,
        "belum_siap"
    );
    // Quota is enforced from the restored counter: 30 + 2 slots + 1 = 33.
    assert_eq!(
        ambil_token(&mut c, &baru, sekarang, k.periode_mulai, "req-s4", 32).await,
        "kuota_habis"
    );
    assert_eq!(
        ambil_token(&mut c, &baru, sekarang, k.periode_mulai, "req-s5", 33).await,
        "ok"
    );
}
