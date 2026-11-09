//! Reconciler di atas PostgreSQL dan Redis nyata (ARC-72, AC-22–24; seam
//! CW10: siklus dipanggil sebagai fungsi dengan jam sebagai parameter).
//!
//! AC-22 (dispatched macet) diuji bersama dispatcher di `dispatcher_pg.rs`
//! (publish gagal → reconciler → publish ulang). Di sini: `in_flight` async dan
//! sync, serta race dua arah reconciler vs webserver (minimal 100 ulangan).
//! "Webserver" tiruan ditulis superuser dengan predikat yang sama dengan jalur
//! sync (`status_akses IS NULL` untuk log, `status = 'in_flight'` untuk hasil).

mod umum;

use chrono::{Duration, Utc};
use poolrouter_controller::reconciler::{KonfigurasiReconciler, jalankan_siklus_reconciler};
use poolrouter_core::retry::AcakBatasAtas;
use sqlx::PgPool;
use umum::*;
use uuid::Uuid;

fn konf() -> KonfigurasiReconciler {
    KonfigurasiReconciler {
        batas_baris: 100_000,
        ..KonfigurasiReconciler::default()
    }
}

fn in_flight_async(attempt: i32) -> Req {
    Req {
        status: "in_flight",
        attempt,
        umur_log: Duration::minutes(10),
        ttl: Some(Utc::now() + Duration::hours(1)),
        ..Req::queued()
    }
}

#[tokio::test]
#[ignore]
async fn ac23_async_in_flight_kembali_queued_dengan_not_before_atau_retry_habis() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    let dijadwal = buat_request(&su, &g, &in_flight_async(1)).await;
    // Batas percobaan pool 3: percobaan ketiga habis (AC-18).
    let habis = buat_request(&su, &g, &in_flight_async(3)).await;
    // TTL 2 s lagi: not_before (5 s, stub batas atas) >= TTL → retry_habis.
    let dekat_ttl = buat_request(
        &su,
        &g,
        &Req {
            ttl: Some(Utc::now() + Duration::seconds(2)),
            ..in_flight_async(1)
        },
    )
    .await;
    // Log baru 30 s: belum lewat ambang 90 s, tidak disentuh.
    let segar = buat_request(
        &su,
        &g,
        &Req {
            umur_log: Duration::seconds(30),
            ..in_flight_async(1)
        },
    )
    .await;

    let kuota = siapkan_redis(&db).await;
    // Rebuild memasang slot `in_flight` dengan nomor `attempt_count`.
    assert!(ada_slot(g.upool_id, dijadwal, 1).await);
    let antre_awal = antre(g.upool_id).await;

    let sekarang = Utc::now();
    let r = jalankan_siklus_reconciler(&db, Some(&kuota), &AcakBatasAtas, &konf(), sekarang)
        .await
        .expect("reconciler");

    let x = baca(&su, dijadwal).await;
    assert_eq!(x.status, "queued");
    assert!(r.dijadwalkan_ulang.contains(&dijadwal));
    let nb = x.not_before.expect("not_before terisi");
    let selisih = (nb - sekarang).num_milliseconds();
    assert!(
        (4_990..=5_010).contains(&selisih),
        "base 5 s × 2^0, stub batas atas: {selisih} ms"
    );
    assert_eq!(
        status_log_terakhir(&su, dijadwal).await.as_deref(),
        Some("tidak_diketahui")
    );
    assert!(!ada_slot(g.upool_id, dijadwal, 1).await, "slot dilepas");

    for id in [habis, dekat_ttl] {
        let y = baca(&su, id).await;
        assert_eq!(y.status, "failed");
        assert_eq!(y.failure_reason.as_deref(), Some("retry_habis"));
        assert!(y.final_kode_transaksi.is_some());
        assert_eq!(
            status_log_terakhir(&su, id).await.as_deref(),
            Some("tidak_diketahui")
        );
    }
    assert_eq!(baca(&su, segar).await.status, "in_flight");
    assert_eq!(status_log_terakhir(&su, segar).await, None);

    // Request yang kembali menunggu menambah `antre` satu (CW3).
    assert_eq!(antre(g.upool_id).await, antre_awal + 1);

    // History satu baris per transisi.
    let h = history(&su, dijadwal).await;
    assert_eq!(h.last().unwrap().0.as_deref(), Some("in_flight"));
    assert_eq!(h.last().unwrap().1, "queued");
}

async fn sync_in_flight(su: &PgPool, g: &Graf) -> Uuid {
    buat_request(
        su,
        g,
        &Req {
            mode: "sync",
            status: "in_flight",
            attempt: 1,
            umur_log: Duration::minutes(10),
            ..Req::queued()
        },
    )
    .await
}

#[tokio::test]
#[ignore]
async fn ac24_sync_in_flight_failed_timeout_sync_tidak_pernah_queued() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    let id = sync_in_flight(&su, &g).await;
    let r = jalankan_siklus_reconciler::<Kuota>(&db, None, &AcakBatasAtas, &konf(), Utc::now())
        .await
        .expect("reconciler");
    assert!(r.timeout_sync.contains(&id));
    let x = baca(&su, id).await;
    assert_eq!(x.status, "failed");
    assert_eq!(x.failure_reason.as_deref(), Some("timeout_sync"));
    assert!(x.output_kosong);
    assert_eq!(x.webhook_status, None, "sync tanpa webhook");
    assert_eq!(
        status_log_terakhir(&su, id).await.as_deref(),
        Some("tidak_diketahui")
    );
    assert!(
        history(&su, id).await.iter().all(|h| h.1 != "queued"),
        "request sync tidak pernah masuk queued (U-21)"
    );
}

/// Webserver tiruan: lengkapi log lalu tulis hasil, dengan predikat jalur sync.
async fn webserver_selesai(su: &PgPool, id: Uuid) -> bool {
    let mut tx = su.begin().await.expect("tx webserver");
    let kode: Option<Uuid> = sqlx::query_scalar(
        "UPDATE transaction_log SET status_akses = 'berhasil', responded_at = now()
          WHERE request_id = $1 AND attempt_no = 1 AND status_akses IS NULL
          RETURNING kode_transaksi",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .expect("log webserver");
    let Some(kode) = kode else {
        return false;
    };
    let n = sqlx::query(
        "UPDATE request
            SET status = 'done', output_payload = '{\"cocok\": true}'::jsonb, sumber_hasil = 'provider',
                final_kode_transaksi = $2, completed_at = now()
          WHERE request_id = $1 AND status = 'in_flight'",
    )
    .bind(id)
    .bind(kode)
    .execute(&mut *tx)
    .await
    .expect("hasil webserver")
    .rows_affected();
    if n != 1 {
        return false;
    }
    sqlx::query(
        "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
         SELECT $1, MAX(seq) + 1, 'in_flight', 'done', 'webserver'
           FROM request_status_history WHERE request_id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .expect("history webserver");
    tx.commit().await.expect("commit webserver");
    true
}

#[tokio::test]
#[ignore]
async fn ac24_race_dua_arah_reconciler_vs_webserver_100_ulangan() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    let (mut menang_reconciler, mut menang_webserver) = (0, 0);
    for i in 0..100u64 {
        let id = sync_in_flight(&su, &g).await;
        // Urutan acak: jeda kecil di salah satu sisi, berganti-ganti semu acak.
        let jeda = std::time::Duration::from_micros((i * 7_919) % 3_000);
        let reconciler_dulu = (i.wrapping_mul(2_654_435_761) >> 7) % 2 == 0;
        let rec = async {
            if !reconciler_dulu {
                tokio::time::sleep(jeda).await;
            }
            jalankan_siklus_reconciler::<Kuota>(&db, None, &AcakBatasAtas, &konf(), Utc::now())
                .await
                .expect("reconciler")
        };
        let web = async {
            if reconciler_dulu {
                tokio::time::sleep(jeda).await;
            }
            webserver_selesai(&su, id).await
        };
        let (r, web_menang) = tokio::join!(rec, web);
        let x = baca(&su, id).await;
        let log = status_log_terakhir(&su, id).await;
        let rec_menang = r.timeout_sync.contains(&id);
        assert!(rec_menang ^ web_menang, "tepat satu pemenang (ulangan {i})");
        if rec_menang {
            assert_eq!(x.status, "failed");
            assert_eq!(x.failure_reason.as_deref(), Some("timeout_sync"));
            assert_eq!(
                log.as_deref(),
                Some("tidak_diketahui"),
                "hasil terlambat tidak menimpa"
            );
            assert!(x.output_kosong, "output_payload tetap NULL");
            menang_reconciler += 1;
        } else {
            assert_eq!(x.status, "done");
            assert_eq!(log.as_deref(), Some("berhasil"));
            menang_webserver += 1;
        }
    }
    println!(
        "ac24 race: reconciler menang {menang_reconciler}, webserver menang {menang_webserver}"
    );
}

#[tokio::test]
#[ignore]
async fn ac25_galat_di_tengah_siklus_tidak_membuang_efek_redis_baris_yang_sudah_commit() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    // `in_flight` urut `requested_at` log: dua baris pertama commit, baris
    // ketiga gagal karena fault injection.
    let mut ok = Vec::new();
    for menit in [10, 9] {
        ok.push(
            buat_request(
                &su,
                &g,
                &Req {
                    umur_log: Duration::minutes(menit),
                    ..in_flight_async(1)
                },
            )
            .await,
        );
    }
    let gagal = buat_request(
        &su,
        &g,
        &Req {
            umur_log: Duration::minutes(8),
            ..in_flight_async(1)
        },
    )
    .await;

    let kuota = siapkan_redis(&db).await;
    let antre_awal = antre(g.upool_id).await;
    for id in ok.iter().chain([&gagal]) {
        assert!(ada_slot(g.upool_id, *id, 1).await);
    }

    let nama = pasang_galat_update(&su, gagal).await;
    let r =
        jalankan_siklus_reconciler(&db, Some(&kuota), &AcakBatasAtas, &konf(), Utc::now()).await;
    lepas_galat(&su, &nama).await;

    let galat = r.expect_err("galat PostgreSQL di baris ketiga harus dikembalikan");
    assert!(
        galat.contains("galat uji"),
        "galat asli diteruskan: {galat}"
    );
    for id in &ok {
        assert_eq!(baca(&su, *id).await.status, "queued");
        assert!(
            !ada_slot(g.upool_id, *id, 1).await,
            "slot baris commit dilepas"
        );
    }
    assert_eq!(
        baca(&su, gagal).await.status,
        "in_flight",
        "baris gagal di-rollback"
    );
    assert!(ada_slot(g.upool_id, gagal, 1).await);
    // AC-25/CW3: dua request kembali menunggu, `antre` naik dua.
    assert_eq!(antre(g.upool_id).await, antre_awal + 2);

    jalankan_siklus_reconciler(&db, Some(&kuota), &AcakBatasAtas, &konf(), Utc::now())
        .await
        .expect("siklus sesudah galat dilepas");
    assert_eq!(baca(&su, gagal).await.status, "queued");
    assert_eq!(antre(g.upool_id).await, antre_awal + 3);
}
