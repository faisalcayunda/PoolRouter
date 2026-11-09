//! Penyapu di atas PostgreSQL dan Redis nyata (ARC-72, AC-19–21, AC-25; seam
//! CW10: siklus dipanggil sebagai fungsi dengan jam sebagai parameter).
//!
//! Dijalankan `make -C backend uji-pg` lewat `cargo test -p
//! poolrouter-controller -- --ignored`. Kode yang diuji memakai role
//! `pr_controller`; fixture dan "worker/webserver" tiruan di uji race ditulis
//! superuser, karena yang diuji adalah predikat compare-and-set, bukan grant.

mod umum;

use chrono::{Duration, Utc};
use poolrouter_controller::penyapu::{KonfigurasiPenyapu, jalankan_siklus_penyapu};
use sqlx::PgPool;
use umum::*;
use uuid::Uuid;

fn konf() -> KonfigurasiPenyapu {
    KonfigurasiPenyapu {
        batas_baris: 100_000,
        batas_purge: 100_000,
    }
}

fn lewat_ttl() -> Req {
    Req {
        ttl: Some(Utc::now() - Duration::minutes(1)),
        ..Req::queued()
    }
}

/// Webhook fixture tidak boleh dikirim job webhook di uji lain yang memakai
/// database yang sama.
async fn matikan_webhook(su: &PgPool, client_id: Uuid) {
    sqlx::query(
        "UPDATE request SET webhook_status = 'tidak_dikirim'
          WHERE client_id = $1 AND webhook_status = 'menunggu'",
    )
    .bind(client_id)
    .execute(su)
    .await
    .expect("matikan webhook fixture");
}

#[tokio::test]
#[ignore]
async fn ac19_entitas_nonaktif_ditutup_k20_sebelum_ttl_dan_antre_kembali() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(
        &su,
        &OpsiGraf {
            webhook_aktif: true,
            ..OpsiGraf::default()
        },
    )
    .await;
    let endpoint_b = Uuid::now_v7();
    buat_endpoint(&su, &g, endpoint_b).await;

    // 40 request endpoint B (akan dinonaktifkan) dan 60 request endpoint A,
    // semuanya sudah lewat TTL. 1 request B menunggu retry. 1 request A belum
    // lewat TTL (tidak disentuh).
    let mut b = Vec::new();
    for _ in 0..40 {
        b.push(
            buat_request(
                &su,
                &g,
                &Req {
                    endpoint: Some(endpoint_b),
                    ..lewat_ttl()
                },
            )
            .await,
        );
    }
    let b_retry = buat_request(
        &su,
        &g,
        &Req {
            endpoint: Some(endpoint_b),
            attempt: 1,
            ..lewat_ttl()
        },
    )
    .await;
    let mut a = Vec::new();
    for _ in 0..60 {
        a.push(buat_request(&su, &g, &lewat_ttl()).await);
    }
    let a_hidup = buat_request(&su, &g, &Req::queued()).await;

    // Rebuild menghitung `antre` pool dari PostgreSQL: 102 request menunggu.
    let kuota = siapkan_redis(&db).await;
    assert_eq!(antre(g.upool_id).await, 102);

    sqlx::query("UPDATE downstream_endpoint SET aktif = false WHERE downstream_endpoint_id = $1")
        .bind(endpoint_b)
        .execute(&su)
        .await
        .expect("nonaktifkan endpoint B");

    let r = jalankan_siklus_penyapu(&db, Some(&kuota), &konf(), Utc::now())
        .await
        .expect("siklus penyapu");
    assert!(r.k20_cancelled >= 40 && r.k20_failed >= 1 && r.expired >= 60);

    for id in &b {
        let x = baca(&su, *id).await;
        assert_eq!(x.status, "cancelled", "K-20 menang atas TTL (U-50)");
        assert_eq!(x.cancel_reason.as_deref(), Some("endpoint_nonaktif"));
        assert_eq!(x.cancelled_by.as_deref(), Some("sistem"));
        assert_eq!(x.webhook_status.as_deref(), Some("menunggu"));
        assert!(x.input_kosong, "purge P-02 di transaksi yang sama");
        let h = history(&su, *id).await;
        let akhir = h.last().unwrap();
        assert_eq!(akhir.1, "cancelled");
        assert_eq!(akhir.2.as_deref(), Some("endpoint_nonaktif"));
        assert_eq!(akhir.3, Some(endpoint_b), "objek pemicu = endpoint");
    }
    let x = baca(&su, b_retry).await;
    assert_eq!(x.status, "failed");
    assert_eq!(x.failure_reason.as_deref(), Some("objek_nonaktif"));
    assert!(
        x.final_kode_transaksi.is_some(),
        "final log = percobaan terakhir"
    );
    for id in &a {
        let x = baca(&su, *id).await;
        assert_eq!(x.status, "expired");
        assert!(x.input_kosong);
    }
    assert_eq!(baca(&su, a_hidup).await.status, "queued");

    // AC-25: 101 request keluar tanpa klaim slot, `antre` kembali ke 1.
    assert_eq!(antre(g.upool_id).await, 1);
    // Siklus kedua: tidak ada transisi, `antre` tidak turun lagi.
    jalankan_siklus_penyapu(&db, Some(&kuota), &konf(), Utc::now())
        .await
        .expect("siklus kedua");
    assert_eq!(antre(g.upool_id).await, 1);

    matikan_webhook(&su, g.client_id).await;
}

#[tokio::test]
#[ignore]
async fn ac19_urutan_alasan_tetap_saat_dua_entitas_nonaktif() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    let id = buat_request(&su, &g, &Req::queued()).await;
    // Provider dulu, lalu client: urutan alasan tetap client lebih dulu (U-51).
    sqlx::query("UPDATE provider SET aktif = false WHERE provider_id = $1")
        .bind(g.provider_id)
        .execute(&su)
        .await
        .expect("nonaktifkan provider");
    sqlx::query("UPDATE client SET aktif = false WHERE client_id = $1")
        .bind(g.client_id)
        .execute(&su)
        .await
        .expect("nonaktifkan client");
    jalankan_siklus_penyapu::<Kuota>(&db, None, &konf(), Utc::now())
        .await
        .expect("siklus");
    let x = baca(&su, id).await;
    assert_eq!(x.status, "cancelled");
    assert_eq!(x.cancel_reason.as_deref(), Some("client_nonaktif"));
    assert_eq!(x.webhook_status.as_deref(), Some("tidak_dikirim"));
}

#[tokio::test]
#[ignore]
async fn ac19_entitas_aktif_lagi_sebelum_penyapu_diproses_normal() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    let id = buat_request(&su, &g, &Req::queued()).await;
    for aktif in [false, true] {
        sqlx::query("UPDATE downstream_pool SET aktif = $2 WHERE downstream_pool_id = $1")
            .bind(g.dpool_id)
            .bind(aktif)
            .execute(&su)
            .await
            .expect("ubah downstream pool");
    }
    jalankan_siklus_penyapu::<Kuota>(&db, None, &konf(), Utc::now())
        .await
        .expect("siklus");
    assert_eq!(baca(&su, id).await.status, "queued", "U-53");
}

#[tokio::test]
#[ignore]
async fn ac19_ttl_dan_retry_habis_tetap_jalan_tanpa_redis() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    let queued = buat_request(&su, &g, &lewat_ttl()).await;
    let dispatched = buat_request(
        &su,
        &g,
        &Req {
            status: "dispatched",
            ..lewat_ttl()
        },
    )
    .await;
    let retry = buat_request(
        &su,
        &g,
        &Req {
            attempt: 2,
            ..lewat_ttl()
        },
    )
    .await;

    // Tanpa store: Redis mati (U-20a/b). Langkah PostgreSQL tetap jalan.
    jalankan_siklus_penyapu::<Kuota>(&db, None, &konf(), Utc::now())
        .await
        .expect("siklus tanpa Redis");

    assert_eq!(baca(&su, queued).await.status, "expired");
    assert_eq!(baca(&su, dispatched).await.status, "expired");
    let x = baca(&su, retry).await;
    assert_eq!(x.status, "failed");
    assert_eq!(x.failure_reason.as_deref(), Some("retry_habis"));
    assert_eq!(x.attempt_count, 2, "percobaan sebelumnya tidak berubah");
    assert_eq!(x.webhook_status.as_deref(), Some("tidak_dikirim"));

    // Redis belum siap (pr:siap hilang) juga tidak menghentikan penyapu.
    let kuota = siapkan_redis(&db).await;
    let lagi = buat_request(&su, &g, &lewat_ttl()).await;
    let mut k = klien_redis()
        .get_multiplexed_async_connection()
        .await
        .expect("redis");
    redis::cmd("DEL")
        .arg("pr:siap")
        .query_async::<()>(&mut k)
        .await
        .expect("hapus pr:siap");
    jalankan_siklus_penyapu(&db, Some(&kuota), &konf(), Utc::now())
        .await
        .expect("siklus Redis belum siap");
    assert_eq!(baca(&su, lagi).await.status, "expired");
    siapkan_redis(&db).await;
}

#[tokio::test]
#[ignore]
async fn ac19_race_penyapu_vs_gerbang_worker_tepat_satu_transisi() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    let mut menang_penyapu = 0;
    let mut menang_worker = 0;
    for i in 0..30 {
        // Menunggu retry, `dispatched`, TTL lewat: penyapu ingin `failed`,
        // worker ingin `in_flight` (U-27).
        let id = buat_request(
            &su,
            &g,
            &Req {
                status: "dispatched",
                attempt: 1,
                ..lewat_ttl()
            },
        )
        .await;
        let worker = async {
            // Iterasi genap: worker ditunda 0–1,5 s supaya penyapu (yang
            // memindai seluruh basis data uji) juga sempat menang.
            if i % 2 == 0 {
                tokio::time::sleep(std::time::Duration::from_millis((i as u64 / 2 % 4) * 500))
                    .await;
            }
            let mut tx = su.begin().await.expect("tx worker");
            let n = sqlx::query(
                "UPDATE request SET status = 'in_flight', attempt_count = attempt_count + 1
                  WHERE request_id = $1 AND status = 'dispatched'",
            )
            .bind(id)
            .execute(&mut *tx)
            .await
            .expect("CAS worker")
            .rows_affected();
            if n == 1 {
                sqlx::query(
                    "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
                     SELECT $1, MAX(seq) + 1, 'dispatched', 'in_flight', 'worker'
                       FROM request_status_history WHERE request_id = $1",
                )
                .bind(id)
                .execute(&mut *tx)
                .await
                .expect("history worker");
                tx.commit().await.expect("commit worker");
            }
            n == 1
        };
        let penyapu = async {
            if i % 2 == 1 {
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            jalankan_siklus_penyapu::<Kuota>(&db, None, &konf(), Utc::now())
                .await
                .expect("siklus");
        };
        let (worker_menang, ()) = tokio::join!(worker, penyapu);
        let x = baca(&su, id).await;
        let keluar: Vec<_> = history(&su, id)
            .await
            .into_iter()
            .filter(|h| h.0.as_deref() == Some("dispatched"))
            .collect();
        assert_eq!(
            keluar.len(),
            1,
            "tepat satu transisi keluar dari dispatched"
        );
        if worker_menang {
            assert_eq!(x.status, "in_flight");
            menang_worker += 1;
        } else {
            assert_eq!(x.status, "failed");
            assert_eq!(x.failure_reason.as_deref(), Some("retry_habis"));
            menang_penyapu += 1;
        }
    }
    println!("ac19 race: penyapu menang {menang_penyapu}, worker menang {menang_worker}");
    assert!(
        menang_penyapu > 0 && menang_worker > 0,
        "kedua arah race harus terjadi minimal sekali"
    );
}

#[tokio::test]
#[ignore]
async fn ac20_log_menggantung_ditutup_tidak_diketahui_dan_tidak_ditimpa() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    // Timeout pool 30 s: ambang 90 s. Log 10 menit ditutup, log 30 detik tidak.
    let lama = buat_request(
        &su,
        &g,
        &Req {
            mode: "sync",
            status: "in_flight",
            attempt: 1,
            umur_log: Duration::minutes(10),
            ..Req::queued()
        },
    )
    .await;
    let baru = buat_request(
        &su,
        &g,
        &Req {
            mode: "sync",
            status: "in_flight",
            attempt: 1,
            umur_log: Duration::seconds(30),
            ..Req::queued()
        },
    )
    .await;

    let r = jalankan_siklus_penyapu::<Kuota>(&db, None, &konf(), Utc::now())
        .await
        .expect("siklus");
    assert!(r.log_ditutup >= 1);
    assert_eq!(
        status_log_terakhir(&su, lama).await.as_deref(),
        Some("tidak_diketahui")
    );
    assert_eq!(status_log_terakhir(&su, baru).await, None);

    // Respons terlambat memakai `WHERE status_akses IS NULL`: nol baris.
    let n = sqlx::query(
        "UPDATE transaction_log SET status_akses = 'berhasil', responded_at = now()
          WHERE request_id = $1 AND status_akses IS NULL",
    )
    .bind(lama)
    .execute(&su)
    .await
    .expect("respons terlambat")
    .rows_affected();
    assert_eq!(n, 0);
    assert_eq!(
        status_log_terakhir(&su, lama).await.as_deref(),
        Some("tidak_diketahui")
    );
}

#[tokio::test]
#[ignore]
async fn ac21_purge_p03_menghormati_ttl_downstream_pool() {
    let su = pool_su().await;
    let db = pool_controller().await;
    // TTL downstream pool 1 jam.
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    let selesai_lama = buat_request(
        &su,
        &g,
        &Req {
            status: "done",
            attempt: 1,
            umur_log: Duration::hours(3),
            selesai: Some(Utc::now() - Duration::hours(2)),
            webhook_status_done: "menunggu",
            ..Req::queued()
        },
    )
    .await;
    let selesai_baru = buat_request(
        &su,
        &g,
        &Req {
            status: "done",
            attempt: 1,
            umur_log: Duration::minutes(20),
            selesai: Some(Utc::now() - Duration::minutes(10)),
            webhook_status_done: "terkirim",
            ..Req::queued()
        },
    )
    .await;

    jalankan_siklus_penyapu::<Kuota>(&db, None, &konf(), Utc::now())
        .await
        .expect("siklus");
    let x = baca(&su, selesai_lama).await;
    assert!(
        x.output_kosong && x.output_purged,
        "lewat completed_at + TTL: dipurge, apa pun webhook_status"
    );
    let y = baca(&su, selesai_baru).await;
    assert!(
        !y.output_kosong && !y.output_purged,
        "masih di jendela polling (S-05)"
    );
    matikan_webhook(&su, g.client_id).await;
}

#[tokio::test]
#[ignore]
async fn ac25_galat_di_tengah_siklus_tidak_membuang_efek_redis_baris_yang_sudah_commit() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    // Langkah TTL urut `ttl_expires_at`: dua baris pertama commit, baris
    // ketiga gagal karena fault injection.
    let mut ok = Vec::new();
    for menit in [3, 2] {
        ok.push(
            buat_request(
                &su,
                &g,
                &Req {
                    ttl: Some(Utc::now() - Duration::minutes(menit)),
                    ..Req::queued()
                },
            )
            .await,
        );
    }
    let gagal = buat_request(
        &su,
        &g,
        &Req {
            ttl: Some(Utc::now() - Duration::seconds(1)),
            ..Req::queued()
        },
    )
    .await;

    let kuota = siapkan_redis(&db).await;
    assert_eq!(antre(g.upool_id).await, 3);

    let nama = pasang_galat_update(&su, gagal).await;
    let r = jalankan_siklus_penyapu(&db, Some(&kuota), &konf(), Utc::now()).await;
    lepas_galat(&su, &nama).await;

    let galat = r.expect_err("galat PostgreSQL di baris ketiga harus dikembalikan");
    assert!(
        galat.contains("galat uji"),
        "galat asli diteruskan: {galat}"
    );
    for id in &ok {
        assert_eq!(baca(&su, *id).await.status, "expired");
    }
    assert_eq!(
        baca(&su, gagal).await.status,
        "queued",
        "baris gagal di-rollback"
    );
    // AC-25: efek dua transisi yang sudah commit tetap dijalankan.
    assert_eq!(antre(g.upool_id).await, 1);

    // Siklus berikutnya menutup sisa tanpa mengurangi `antre` dua kali.
    jalankan_siklus_penyapu(&db, Some(&kuota), &konf(), Utc::now())
        .await
        .expect("siklus sesudah galat dilepas");
    assert_eq!(baca(&su, gagal).await.status, "expired");
    assert_eq!(antre(g.upool_id).await, 0);
}
