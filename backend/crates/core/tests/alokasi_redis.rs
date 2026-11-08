//! AC-8: intake alokasi dengan kunci `pr:antre:dp:{<pool>}:<d>`.
//!
//! Gate F0 jalan lebih dulu, lalu `antre_d >= ruang_intake(d)` menolak 429
//! `kuota_habis`. Request async yang diterima menaikkan `pr:antre:{<pool>}` dan
//! `pr:antre:dp:{<pool>}:<d>` dalam EVAL yang sama (ADR-0006 rev 1 §10).
//!
//! Container Redis asli; dijalankan via `make -C backend uji-redis`. Seam T2
//! dari spec `f1-prioritas-alokasi` (file uji baru, Redis container).

use chrono::{DateTime, Duration, Utc};
use poolrouter_core::kuota::pemulihan::{
    GalatPemulihan, KeadaanPool, SumberRebuild, baca_run_id, bangun_ulang, kunci_antre,
    kunci_antre_dp,
};
use poolrouter_core::kuota::store::{
    GalatStore, HasilIntake, IntakeAlokasi, KonteksKuota, KuotaRedis, KuotaStore, ModeJendela,
};
use redis::aio::MultiplexedConnection;

struct Kosong;

impl SumberRebuild for Kosong {
    async fn keadaan(&self, _: DateTime<Utc>) -> Result<Vec<KeadaanPool>, GalatPemulihan> {
        Ok(vec![])
    }
}

/// Flushed Redis, marked ready for its live run_id.
async fn siap() -> (MultiplexedConnection, KuotaRedis<MultiplexedConnection>) {
    let url = std::env::var("POOLROUTER_TEST_REDIS")
        .expect("POOLROUTER_TEST_REDIS belum diset; jalankan lewat `make -C backend uji-redis`");
    let mut c = redis::Client::open(url)
        .expect("URL Redis tidak sah")
        .get_multiplexed_async_connection()
        .await
        .expect("Redis tidak dapat dihubungi");
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

async fn baca_counter(c: &mut MultiplexedConnection, kunci: &str) -> i64 {
    redis::cmd("GET")
        .arg(kunci)
        .query_async::<Option<i64>>(c)
        .await
        .unwrap()
        .unwrap_or(0)
}

#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn intake_alokasi_menegakkan_antre_dp() {
    let (mut c, store) = siap().await;
    let k = ctx("pool-a", 100);
    let alokasi = IntakeAlokasi {
        downstream_pool_id: 7,
        ruang_intake: 2,
    };
    let dp = kunci_antre_dp("pool-a", 7);
    let f0 = kunci_antre("pool-a");

    // Dua permintaan async masih di bawah `ruang_intake`: lolos, kedua counter naik.
    for i in 1..=2 {
        assert_eq!(
            store
                .intake_alokasi(&k, false, "c", &alokasi)
                .await
                .unwrap(),
            HasilIntake::Lolos {
                terpakai: 0,
                antre: i
            }
        );
    }
    assert_eq!(baca_counter(&mut c, &dp).await, 2, "antre_d naik");
    assert_eq!(baca_counter(&mut c, &f0).await, 2, "antre F0 naik");

    // `antre_d >= ruang_intake` → 429 `kuota_habis`; tidak ada counter naik.
    assert_eq!(
        store
            .intake_alokasi(&k, false, "c", &alokasi)
            .await
            .unwrap(),
        HasilIntake::KuotaHabis {
            terpakai: 0,
            antre: 2
        }
    );
    assert_eq!(baca_counter(&mut c, &dp).await, 2, "antre_d tetap");
    assert_eq!(baca_counter(&mut c, &f0).await, 2, "antre F0 tetap");

    // Cek F0 lebih dulu: `terpakai + antre >= kuota_efektif` menolak
    // `antrean_penuh` walau ruang rute masih lapang.
    let sempit = ctx("pool-b", 1);
    let lapang = IntakeAlokasi {
        downstream_pool_id: 7,
        ruang_intake: 1_000,
    };
    assert!(matches!(
        store
            .intake_alokasi(&sempit, false, "c", &lapang)
            .await
            .unwrap(),
        HasilIntake::Lolos { .. }
    ));
    assert_eq!(
        store
            .intake_alokasi(&sempit, false, "c", &lapang)
            .await
            .unwrap(),
        HasilIntake::AntreanPenuh {
            terpakai: 0,
            antre: 1
        }
    );
    assert_eq!(
        baca_counter(&mut c, &kunci_antre_dp("pool-b", 7)).await,
        1,
        "antre_d tidak naik saat gate F0 menolak"
    );

    // Sync tidak masuk antrean: kedua counter tetap.
    assert!(matches!(
        store.intake_alokasi(&k, true, "c", &alokasi).await.unwrap(),
        HasilIntake::Lolos { .. }
    ));
    assert_eq!(baca_counter(&mut c, &dp).await, 2);
    assert_eq!(baca_counter(&mut c, &f0).await, 2);

    // Downstream id dan ruang tidak sah adalah bug pemanggil, bukan penolakan.
    for buruk in [
        IntakeAlokasi {
            downstream_pool_id: 0,
            ruang_intake: 10,
        },
        IntakeAlokasi {
            downstream_pool_id: 7,
            ruang_intake: -1,
        },
    ] {
        assert!(matches!(
            store.intake_alokasi(&k, false, "c", &buruk).await,
            Err(GalatStore::ArgumenTidakSah(_))
        ));
    }
}

// T2e AC-10: Metrik pinjaman naik saat sumber ≠ sendiri melalui ambil_token_alokasi Lua.
#[tokio::test]
#[ignore = "butuh Redis: make -C backend uji-redis"]
async fn pinjaman_total_naikkan_counter_via_lua() {
    let (_c, store) = siap().await;
    use poolrouter_core::kuota::store::{PermintaanToken, HasilToken};
    use poolrouter_core::kuota::rencana::Sumber;
    use metrics_exporter_prometheus::PrometheusBuilder;

    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();

    // Record metrics in the local recorder context while calling Lua.
    metrics::with_local_recorder(&recorder, || {
        let k = ctx("pool-a", 100);
        let permintaan = PermintaanToken {
            request_id: "test-002".into(),
            nomor_percobaan: 1,
            sinkron: false,
        };
        let urutan = vec![
            poolrouter_core::kuota::rencana::SumberDana {
                sumber: Sumber::Sisa,
                ruang: 10,
            },
        ];
        let downstream_id = 7i64;

        // Synchronous block: spawn a task to run the async call.
        // In tests, it's safe to use block_on on the current runtime.
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                store
                    .ambil_token_alokasi(&k, &permintaan, &urutan, downstream_id)
                    .await
            })
        });

        // Should succeed: token granted from "sisa" source.
        assert!(
            matches!(result, Ok(HasilToken::Diberikan(_))),
            "ambil_token_alokasi failed: {result:?}"
        );
    });

    let out = handle.render();
    // Counter should show 1 increment with labels pool-a, downstream=7, source=sisa.
    assert!(
        out.contains(
            r#"poolrouter_pinjaman_total{downstream_pool_id="7",sumber_anggaran="sisa",upstream_pool_id="pool-a"} 1"#
        ),
        "counter did not increment via Lua; rendered:\n{out}"
    );
}
