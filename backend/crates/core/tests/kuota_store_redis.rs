//! `KuotaStore` over a real Redis (SHAA-195): typed answers for every script,
//! `belum_siap` mapped once, rejection limiter, and the "do not call the
//! upstream" outcome. Run by `make -C backend uji-redis`; `#[ignore]`d because
//! it needs `POOLROUTER_TEST_REDIS`.

use chrono::{DateTime, Duration, Utc};
use poolrouter_core::kuota::pemulihan::{
    GalatPemulihan, JendelaRate, KeadaanPool, SumberRebuild, baca_run_id, bangun_ulang,
};
use poolrouter_core::kuota::store::{
    AsalLepas, BURST_PENOLAKAN, GalatStore, HasilIntake, HasilKlaim, HasilPenolakan, HasilTambah,
    HasilToken, ISI_ULANG_PENOLAKAN_MIKRO, KonteksKuota, KuotaRedis, KuotaStore, ModeJendela,
    PermintaanToken,
};
use redis::aio::MultiplexedConnection;

/// The restart phases of `pemulihan_redis` own the Redis between restarts; these
/// tests flush it, so they only run when no phase is selected.
fn lewati() -> bool {
    std::env::var("POOLROUTER_TEST_FASE").is_ok_and(|f| !f.is_empty())
}

struct Kosong;

impl SumberRebuild for Kosong {
    async fn keadaan(&self, _: DateTime<Utc>) -> Result<Vec<KeadaanPool>, GalatPemulihan> {
        Ok(vec![])
    }
}

async fn koneksi() -> MultiplexedConnection {
    let url = std::env::var("POOLROUTER_TEST_REDIS")
        .expect("POOLROUTER_TEST_REDIS belum diset; jalankan lewat `make -C backend uji-redis`");
    redis::Client::open(url)
        .expect("URL Redis tidak sah")
        .get_multiplexed_async_connection()
        .await
        .expect("Redis tidak dapat dihubungi")
}

/// Flushed Redis, marked ready for its live run_id.
async fn siap() -> (MultiplexedConnection, KuotaRedis<MultiplexedConnection>) {
    let mut c = koneksi().await;
    redis::cmd("FLUSHALL")
        .query_async::<()>(&mut c)
        .await
        .unwrap();
    let store = KuotaRedis::baru(c.clone()).await.unwrap();
    let run_id = baca_run_id(&mut c).await.unwrap();
    bangun_ulang(&mut c, &run_id, &Kosong, Utc::now())
        .await
        .unwrap();
    (c, store)
}

fn ctx(pool: &str, kuota: i64) -> KonteksKuota {
    let sekarang = Utc::now();
    KonteksKuota {
        pool_id: pool.into(),
        periode_mulai: sekarang - Duration::hours(2),
        requested_at: sekarang,
        mode: ModeJendela::Bergulir,
        kuota_efektif: kuota,
    }
}

fn permintaan(id: &str, burst: i64, ttl: i64) -> PermintaanToken {
    PermintaanToken {
        request_id: id.into(),
        nomor_percobaan: 1,
        sync: false,
        maks_konkurensi: 50,
        ttl_slot_mikro: ttl,
        rasio_retry: 0.1,
        retry_min: 20,
        porsi_sync: 0.2,
        jendela_rate: vec![JendelaRate {
            jenis: "tps".into(),
            t_mikro: 1_000_000,
            burst,
        }],
    }
}

/// One attempt's lifecycle through every script: intake, token, claim, count,
/// release, read.
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn siklus_percobaan_lewat_semua_skrip() {
    if lewati() {
        return;
    }
    let (_c, store) = siap().await;
    let k = ctx("pool-a", 100);

    let intake = store.intake(&k, false, "client-1").await.unwrap();
    assert_eq!(
        intake,
        HasilIntake::Lolos {
            terpakai: 0,
            antre: 1
        }
    );

    let token = store
        .ambil_token(&k, &permintaan("req-1", 10, 60_000_000))
        .await
        .unwrap();
    assert!(token.diberikan(), "{token:?}");

    let klaim = store
        .klaim_slot("pool-a", "req-1", 1, 60_000_000, true)
        .await
        .unwrap();
    assert!(
        matches!(klaim, HasilKlaim::Diperpanjang { antre: 0, .. }),
        "{klaim:?}"
    );

    let tambah = store.tambah_attempts(&k, "req-1", 1).await.unwrap();
    assert_eq!(
        tambah,
        HasilTambah::Dicatat {
            attempts: 1,
            terpakai: 1,
            kuota: 100
        }
    );
    assert!(tambah.boleh_memanggil_upstream());

    // Between count and release the attempt is counted twice, on purpose.
    let antara = store.baca(&k).await.unwrap();
    assert_eq!((antara.terpakai, antara.slot_aktif), (1, 1));

    assert!(
        store
            .lepas_slot("pool-a", "req-1", 1, AsalLepas::Kas)
            .await
            .unwrap()
    );
    assert!(
        !store
            .lepas_slot("pool-a", "req-1", 1, AsalLepas::Kas)
            .await
            .unwrap(),
        "melepas dua kali tidak menghapus apa pun"
    );
    let akhir = store.baca(&k).await.unwrap();
    assert_eq!(
        (akhir.terpakai, akhir.attempts, akhir.slot_aktif),
        (1, 1, 0)
    );
}

/// A slot that expired before the count must stop the call (D1.4).
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn slot_kedaluwarsa_berarti_jangan_panggil_upstream() {
    if lewati() {
        return;
    }
    let (_c, store) = siap().await;
    let k = ctx("pool-b", 100);

    // TTL 1 ms: gone before the count.
    let token = store
        .ambil_token(&k, &permintaan("req-x", 10, 1_000))
        .await
        .unwrap();
    assert!(token.diberikan());
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let klaim = store
        .klaim_slot("pool-b", "req-x", 1, 60_000_000, true)
        .await
        .unwrap();
    assert_eq!(klaim, HasilKlaim::SlotKedaluwarsa);

    let tambah = store.tambah_attempts(&k, "req-x", 1).await.unwrap();
    assert_eq!(tambah, HasilTambah::SlotKedaluwarsa { attempts: 0 });
    assert!(
        !tambah.boleh_memanggil_upstream(),
        "slot kedaluwarsa tidak boleh berujung panggilan upstream"
    );
    let b = store.baca(&k).await.unwrap();
    assert_eq!(b.attempts, 0, "tanpa INCR");

    // A member that was never reserved is refused the same way.
    let asing = store.tambah_attempts(&k, "req-asing", 1).await.unwrap();
    assert!(!asing.boleh_memanggil_upstream());
}

/// Quota, concurrency and rate refusals come back typed, and never as
/// `Diberikan`.
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn penolakan_token_bertipe() {
    if lewati() {
        return;
    }
    let (_c, store) = siap().await;

    // Quota 3: the third slot reaches terpakai(0) + slots(3) >= 3 on the 4th call.
    let k = ctx("pool-c", 3);
    for i in 0..3 {
        let t = store
            .ambil_token(&k, &permintaan(&format!("r{i}"), 100, 60_000_000))
            .await
            .unwrap();
        assert!(t.diberikan(), "{i}: {t:?}");
    }
    let habis = store
        .ambil_token(&k, &permintaan("r3", 100, 60_000_000))
        .await
        .unwrap();
    assert!(matches!(habis, HasilToken::KuotaHabis(_)), "{habis:?}");
    assert!(!habis.diberikan());
    assert_eq!(habis.retry_after_detik(), None);

    // Rate: bucket of one token per second.
    let k = ctx("pool-d", 1000);
    let satu = store
        .ambil_token(&k, &permintaan("a", 1, 60_000_000))
        .await
        .unwrap();
    assert!(satu.diberikan());
    let dua = store
        .ambil_token(&k, &permintaan("b", 1, 60_000_000))
        .await
        .unwrap();
    assert!(matches!(dua, HasilToken::RateLimit(_)), "{dua:?}");
    let tunggu = dua
        .retry_after_detik()
        .expect("rate_limit punya Retry-After");
    assert!((1..=2).contains(&tunggu), "{tunggu}");

    // Concurrency: one slot allowed.
    let k = ctx("pool-e", 1000);
    let mut p = permintaan("c1", 100, 60_000_000);
    p.maks_konkurensi = 1;
    assert!(store.ambil_token(&k, &p).await.unwrap().diberikan());
    p.request_id = "c2".into();
    assert!(matches!(
        store.ambil_token(&k, &p).await.unwrap(),
        HasilToken::KonkurensiPenuh(_)
    ));

    // Duplicate member: refused, fail-closed.
    p.request_id = "c1".into();
    p.maks_konkurensi = 50;
    assert!(matches!(
        store.ambil_token(&k, &p).await.unwrap(),
        HasilToken::SlotGanda(_)
    ));

    // Intake: quota 2, nothing used. Two async calls queue (terpakai 0 + antre
    // 1 < 2 still admits the second), the third reaches 0 + 2 >= 2.
    let k = ctx("pool-f", 2);
    assert!(matches!(
        store.intake(&k, false, "c").await.unwrap(),
        HasilIntake::Lolos { .. }
    ));
    assert!(matches!(
        store.intake(&k, false, "c").await.unwrap(),
        HasilIntake::Lolos { antre: 2, .. }
    ));
    assert!(matches!(
        store.intake(&k, false, "c").await.unwrap(),
        HasilIntake::AntreanPenuh { .. }
    ));
}

/// Rejection limiter: burst 20, one token per second, 21st is limited (U-45a).
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn limiter_penolakan_per_client() {
    if lewati() {
        return;
    }
    let (_c, store) = siap().await;
    for i in 0..BURST_PENOLAKAN {
        let h = store
            .ambil_token_penolakan("client-1", ISI_ULANG_PENOLAKAN_MIKRO, BURST_PENOLAKAN)
            .await
            .unwrap();
        assert_eq!(
            h,
            HasilPenolakan::Lolos {
                sisa_token: BURST_PENOLAKAN - 1 - i
            }
        );
    }
    let ke21 = store
        .ambil_token_penolakan("client-1", ISI_ULANG_PENOLAKAN_MIKRO, BURST_PENOLAKAN)
        .await
        .unwrap();
    assert!(matches!(ke21, HasilPenolakan::Dibatasi { .. }), "{ke21:?}");
    assert_eq!(ke21.retry_after_detik(), Some(1));

    // Another client is untouched, and so is the quota of any pool.
    let lain = store
        .ambil_token_penolakan("client-2", ISI_ULANG_PENOLAKAN_MIKRO, BURST_PENOLAKAN)
        .await
        .unwrap();
    assert_eq!(lain, HasilPenolakan::Lolos { sisa_token: 19 });
}

/// Not ready: every method says `BelumSiap`, writes nothing, and the store
/// recovers on its own once the controller has rebuilt Redis.
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn belum_siap_dipetakan_sekali_dan_pulih_setelah_rebuild() {
    if lewati() {
        return;
    }
    let mut c = koneksi().await;
    redis::cmd("FLUSHALL")
        .query_async::<()>(&mut c)
        .await
        .unwrap();
    let store = KuotaRedis::baru(c.clone()).await.unwrap();
    let k = ctx("pool-g", 100);

    assert!(matches!(
        store.intake(&k, false, "c").await,
        Err(GalatStore::BelumSiap)
    ));
    assert!(matches!(
        store
            .ambil_token(&k, &permintaan("r1", 10, 60_000_000))
            .await,
        Err(GalatStore::BelumSiap)
    ));
    assert!(matches!(
        store.klaim_slot("pool-g", "r1", 1, 60_000_000, true).await,
        Err(GalatStore::BelumSiap)
    ));
    assert!(matches!(
        store.tambah_attempts(&k, "r1", 1).await,
        Err(GalatStore::BelumSiap)
    ));
    assert!(matches!(
        store.lepas_slot("pool-g", "r1", 1, AsalLepas::Kas).await,
        Err(GalatStore::BelumSiap)
    ));
    assert!(matches!(store.baca(&k).await, Err(GalatStore::BelumSiap)));
    assert!(matches!(
        store
            .ambil_token_penolakan("client-1", ISI_ULANG_PENOLAKAN_MIKRO, BURST_PENOLAKAN)
            .await,
        Err(GalatStore::BelumSiap)
    ));
    let kunci: Vec<String> = redis::cmd("KEYS")
        .arg("pr:*")
        .query_async(&mut c)
        .await
        .unwrap();
    assert!(kunci.is_empty(), "penolakan menulis kunci: {kunci:?}");

    // A stale run_id in the store: the failed call refreshed it, and the rebuild
    // marks the live one, so the next call goes through.
    let live = baca_run_id(&mut c).await.unwrap();
    bangun_ulang(&mut c, &live, &Kosong, Utc::now())
        .await
        .unwrap();
    assert_eq!(store.run_id(), live);
    assert!(store.intake(&k, false, "c").await.is_ok());
}

/// A store whose cached run_id is stale (Redis restarted under it) fails closed
/// and then follows the live run_id.
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn run_id_usang_di_store_gagal_tertutup_lalu_mengikuti() {
    if lewati() {
        return;
    }
    let (mut c, store) = siap().await;
    let k = ctx("pool-h", 100);
    let live = baca_run_id(&mut c).await.unwrap();

    // Make the store's cached run_id wrong by pointing pr:siap at another id and
    // refreshing nothing: the script sees a mismatch.
    redis::cmd("SET")
        .arg("pr:siap")
        .arg("run-id-lain")
        .query_async::<()>(&mut c)
        .await
        .unwrap();
    assert!(matches!(
        store.intake(&k, false, "c").await,
        Err(GalatStore::BelumSiap)
    ));
    // Still refused until a rebuild marks the live run_id.
    assert!(matches!(
        store.intake(&k, false, "c").await,
        Err(GalatStore::BelumSiap)
    ));
    bangun_ulang(&mut c, &live, &Kosong, Utc::now())
        .await
        .unwrap();
    assert!(store.intake(&k, false, "c").await.is_ok());
}

/// Bad arguments never reach Redis.
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn argumen_tidak_sah_ditolak_sebelum_redis() {
    if lewati() {
        return;
    }
    let (mut c, store) = siap().await;
    let sebelum: i64 = redis::cmd("DBSIZE").query_async(&mut c).await.unwrap();

    let mut buruk = ctx("pool{x}", 100);
    assert!(matches!(
        store.intake(&buruk, false, "c").await,
        Err(GalatStore::ArgumenTidakSah(_))
    ));
    buruk = ctx("pool-i", 0);
    assert!(matches!(
        store.baca(&buruk).await,
        Err(GalatStore::ArgumenTidakSah(_))
    ));
    assert!(matches!(
        store
            .ambil_token(&ctx("pool-i", 10), &permintaan("a:b", 1, 1_000))
            .await,
        Err(GalatStore::ArgumenTidakSah(_))
    ));
    assert!(matches!(
        store
            .ambil_token_penolakan("c{1}", ISI_ULANG_PENOLAKAN_MIKRO, BURST_PENOLAKAN)
            .await,
        Err(GalatStore::ArgumenTidakSah(_))
    ));
    // Bad numbers that only the script can judge come back as `Skrip`, not as a
    // decision.
    let r = store.ambil_token_penolakan("c1", 0, BURST_PENOLAKAN).await;
    assert!(
        matches!(r, Err(GalatStore::Skrip(ref k)) if k == "t_tidak_sah"),
        "{r:?}"
    );

    let sesudah: i64 = redis::cmd("DBSIZE").query_async(&mut c).await.unwrap();
    assert_eq!(sebelum, sesudah);
}

/// Retry budget over the last hour (ADR-0001 5a.3) through the store: retries
/// beyond `retry_min` are refused with a typed answer, first attempts never.
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn retry_budget_jendela_satu_jam() {
    if lewati() {
        return;
    }
    let (_c, store) = siap().await;
    let k = ctx("pool-r", 1_000_000);
    let mut p = permintaan("rt-0", 1_000_000, 60_000_000);
    p.jendela_rate[0].t_mikro = 1;
    p.maks_konkurensi = 1000;
    p.rasio_retry = 0.0;
    p.retry_min = 2;
    p.nomor_percobaan = 2;

    let mut hasil = Vec::new();
    for i in 0..4 {
        p.request_id = format!("rt-{i}");
        hasil.push(store.ambil_token(&k, &p).await.unwrap());
    }
    assert!(hasil[0].diberikan() && hasil[1].diberikan(), "{hasil:?}");
    assert!(
        matches!(hasil[2], HasilToken::RetryBudgetHabis(_)),
        "{hasil:?}"
    );
    assert!(
        matches!(hasil[3], HasilToken::RetryBudgetHabis(_)),
        "{hasil:?}"
    );
    assert!(!hasil[2].diberikan());

    // A first attempt is never limited by the retry budget.
    p.request_id = "baru".into();
    p.nomor_percobaan = 1;
    assert!(store.ambil_token(&k, &p).await.unwrap().diberikan());

    // 0 and 0: no retry at all (there is no "no budget" value).
    let k0 = ctx("pool-r0", 1_000_000);
    p.request_id = "r0".into();
    p.nomor_percobaan = 2;
    p.retry_min = 0;
    assert!(matches!(
        store.ambil_token(&k0, &p).await.unwrap(),
        HasilToken::RetryBudgetHabis(_)
    ));

    // Bad ratio comes back as a script error, not as a decision.
    p.rasio_retry = 1.5;
    let r = store.ambil_token(&k0, &p).await;
    assert!(
        matches!(r, Err(GalatStore::Skrip(ref k)) if k == "rasio_retry_tidak_sah"),
        "{r:?}"
    );
}

/// The store reports the catalogue metrics of section 5a.9 on its own, with
/// closed labels (no request_id, no NIK).
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn metrik_kuota_dicatat_oleh_store() {
    if lewati() {
        return;
    }
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    // Thread-local recorder: `#[tokio::test]` runs on the current thread.
    let _penjaga = metrics::set_default_local_recorder(&recorder);

    let mut c = koneksi().await;
    redis::cmd("FLUSHALL")
        .query_async::<()>(&mut c)
        .await
        .unwrap();
    let store = KuotaRedis::baru(c.clone())
        .await
        .unwrap()
        .dengan_komponen(poolrouter_core::kuota::metrik::Komponen::Webserver);
    let k = ctx("pool-m", 100);

    // Not ready: fail-closed counter.
    assert!(matches!(
        store.intake(&k, false, "c").await,
        Err(GalatStore::BelumSiap)
    ));
    let run_id = baca_run_id(&mut c).await.unwrap();
    bangun_ulang(&mut c, &run_id, &Kosong, Utc::now())
        .await
        .unwrap();

    // Slot that expires: TTL 1 ms, then the count finds it gone.
    assert!(
        store
            .ambil_token(&k, &permintaan("req-m", 10, 1_000))
            .await
            .unwrap()
            .diberikan()
    );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let t = store.tambah_attempts(&k, "req-m", 1).await.unwrap();
    assert!(!t.boleh_memanggil_upstream());

    // Retry budget refusal.
    let mut p = permintaan("rt-0", 1_000_000, 60_000_000);
    p.jendela_rate[0].t_mikro = 1;
    p.rasio_retry = 0.0;
    p.retry_min = 0;
    p.nomor_percobaan = 2;
    assert!(matches!(
        store.ambil_token(&k, &p).await.unwrap(),
        HasilToken::RetryBudgetHabis(_)
    ));

    // Quota read and rejection limiter.
    store.baca(&k).await.unwrap();
    store
        .ambil_token_penolakan("client-m", ISI_ULANG_PENOLAKAN_MIKRO, BURST_PENOLAKAN)
        .await
        .unwrap();

    let out = handle.render();
    for harap in [
        r#"poolrouter_redis_failclosed_total{komponen="webserver"} 1"#,
        r#"poolrouter_slot_kedaluwarsa_total{upstream_pool_id="pool-m"}"#,
        r#"poolrouter_retry_budget_habis_total{upstream_pool_id="pool-m"} 1"#,
        r#"poolrouter_kuota_terpakai{upstream_pool_id="pool-m"}"#,
        r#"poolrouter_kuota_efektif{upstream_pool_id="pool-m"} 100"#,
        r#"poolrouter_limiter_penolakan_token{client_id="client-m"} 19"#,
    ] {
        assert!(out.contains(harap), "tidak ada: {harap}\n{out}");
    }
    for terlarang in ["req-m", "rt-0"] {
        assert!(!out.contains(terlarang), "label bocor: {terlarang}");
    }
}
