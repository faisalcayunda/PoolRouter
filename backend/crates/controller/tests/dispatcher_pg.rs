//! Dispatcher di atas PostgreSQL dan Redis nyata (ARC-70, AC-7–AC-11, seam
//! CW10: `jalankan_putaran` dipanggil sebagai fungsi).
//!
//! Dijalankan `make -C backend uji-pg` lewat `cargo test -p
//! poolrouter-controller -- --ignored`. Kode yang diuji memakai role
//! `pr_controller`; fixture ditulis superuser. Skrip Lua dan `KuotaStore`
//! tidak di-mock. Publisher adalah perekam (bukan RabbitMQ): suite uji-pg tidak
//! menyalakan broker, dan kontrak broker milik ARC-68.

mod umum;

use std::time::{Duration, Instant};

use chrono::Utc;
use poolrouter_controller::dispatcher::{
    JedaTetap, KonfigurasiPutaran, RingkasanPutaran, jalankan_putaran,
};
use poolrouter_controller::reconciler::{KonfigurasiReconciler, jalankan_siklus_reconciler};
use poolrouter_core::queue::{JobPublisher, nama_antrean_upstream};
use poolrouter_core::retry::AcakBatasAtas;
use sqlx::PgPool;
use umum::*;
use uuid::Uuid;

fn konf(pool: Uuid) -> KonfigurasiPutaran {
    KonfigurasiPutaran {
        batas_per_pool: 5_000,
        batas_waktu: Duration::from_secs(5),
        saring_pool: Some(vec![pool]),
    }
}

/// Putaran berulang sampai `target` request terkirim, putaran berhenti tanpa
/// kemajuan karena alasan selain rate, atau batas waktu habis.
async fn kirim_sampai<P: JobPublisher>(
    db: &PgPool,
    kuota: &Kuota,
    publisher: &P,
    jeda: &JedaTetap,
    pool: Uuid,
    target: usize,
    batas: Duration,
) -> Vec<RingkasanPutaran> {
    let mulai = Instant::now();
    let mut semua = Vec::new();
    let mut total = 0;
    while total < target && mulai.elapsed() < batas {
        let r = jalankan_putaran(db, kuota, publisher, jeda, &konf(pool), Utc::now())
            .await
            .expect("putaran dispatcher");
        total += r.dikirim.len();
        let tunggu = r.tunggu_mikro;
        let kosong = r.dikirim.is_empty();
        semua.push(r);
        match tunggu {
            Some(t) => {
                tokio::time::sleep(Duration::from_micros(t.clamp(1_000, 1_000_000) as u64)).await
            }
            None if kosong => break,
            None => {}
        }
    }
    semua
}

async fn status_semua(su: &PgPool, ids: &[Uuid]) -> Vec<String> {
    let mut hasil = Vec::new();
    for id in ids {
        hasil.push(baca(su, *id).await.status);
    }
    hasil
}

/// Korelasi peringkat Spearman antara urutan masuk (urutan NIK) dan urutan
/// kirim.
fn spearman(urutan_kirim: &[usize]) -> f64 {
    let n = urutan_kirim.len() as f64;
    let d2: f64 = urutan_kirim
        .iter()
        .enumerate()
        .map(|(peringkat, asal)| {
            let d = peringkat as f64 - *asal as f64;
            d * d
        })
        .sum();
    1.0 - 6.0 * d2 / (n * (n * n - 1.0))
}

#[tokio::test]
#[ignore]
async fn ac7_prioritas_dulu_lalu_acak_k05_tanpa_korelasi_nik() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;

    // 1.000 request prioritas 2 dengan NIK sintetis berurutan, lalu 20 request
    // prioritas 1 yang masuk paling akhir.
    let mut rendah = Vec::new();
    for i in 0..1_000u32 {
        let r = Req {
            prioritas: 2,
            nik: Some(format!("00{i:014}")),
            ..Req::queued()
        };
        rendah.push(buat_request(&su, &g, &r).await);
    }
    // `prioritas_efektif <= prioritas` (AG-01): prioritas 1 = paling tinggi.
    let mut tinggi = Vec::new();
    for _ in 0..20 {
        tinggi.push(buat_request(&su, &g, &Req::queued()).await);
    }
    // Request dengan `not_before` di masa depan tidak boleh ikut.
    let ditunda = buat_request(&su, &g, &Req::queued()).await;
    sqlx::query("UPDATE request SET not_before = now() + interval '1 hour' WHERE request_id = $1")
        .bind(ditunda)
        .execute(&su)
        .await
        .expect("set not_before");

    let kuota = siapkan_redis(&db).await;
    let perekam = Perekam::default();
    kirim_sampai(
        &db,
        &kuota,
        &perekam,
        &JedaTetap(0),
        g.upool_id,
        1_020,
        Duration::from_secs(90),
    )
    .await;

    let terkirim = perekam.id();
    assert_eq!(
        terkirim.len(),
        1_020,
        "semua request siap terkirim tepat sekali"
    );
    let unik: std::collections::HashSet<_> = terkirim.iter().collect();
    assert_eq!(unik.len(), 1_020, "tidak ada publish ganda");
    assert!(
        !terkirim.contains(&ditunda),
        "not_before di masa depan dilewati"
    );
    assert!(
        perekam
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|(q, _, _)| *q == nama_antrean_upstream(g.upool_id)),
        "queue diturunkan dari upstream_pool_id"
    );

    // Prioritas tinggi selalu sebelum prioritas rendah.
    let posisi = |id: &Uuid| terkirim.iter().position(|x| x == id).unwrap();
    let akhir_tinggi = tinggi.iter().map(posisi).max().unwrap();
    let awal_rendah = rendah.iter().map(posisi).min().unwrap();
    assert!(
        akhir_tinggi < awal_rendah,
        "prioritas 1 harus habis sebelum prioritas 2 ({akhir_tinggi} vs {awal_rendah})"
    );

    // K-05: urutan kirim tidak berkorelasi dengan urutan NIK.
    let indeks_rendah: std::collections::HashMap<Uuid, usize> =
        rendah.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let urutan: Vec<usize> = terkirim
        .iter()
        .filter_map(|id| indeks_rendah.get(id).copied())
        .collect();
    let rho = spearman(&urutan);
    println!("ac7: rho Spearman urutan NIK vs urutan kirim = {rho:.4}");
    assert!(rho.abs() < 0.1, "|rho| harus < 0,1, dapat {rho}");

    // AC-9: status `dispatched`, upstream terisi, satu baris history per request.
    let contoh = baca(&su, rendah[0]).await;
    assert_eq!(contoh.status, "dispatched");
    assert_eq!(contoh.upstream_pool_id, Some(g.upool_id));
    let h = history(&su, rendah[0]).await;
    assert_eq!(h.len(), 2);
    assert_eq!(h[1].0.as_deref(), Some("queued"));
    assert_eq!(h[1].1, "dispatched");
    // Slot dipegang untuk percobaan pertama (`attempt_count + 1`).
    assert!(ada_slot(g.upool_id, rendah[0], 1).await);
}

#[tokio::test]
#[ignore]
async fn ac8_kuota_efektif_menolak_dan_request_tetap_queued() {
    let su = pool_su().await;
    let db = pool_controller().await;
    // floor(6 * (1 - 0,1)) = 5.
    let g = buat_graf(
        &su,
        &OpsiGraf {
            kuota_harian: 6,
            margin: "0.100",
            ..OpsiGraf::default()
        },
    )
    .await;
    let mut ids = Vec::new();
    for _ in 0..10 {
        ids.push(buat_request(&su, &g, &Req::queued()).await);
    }
    let kuota = siapkan_redis(&db).await;
    let perekam = Perekam::default();
    kirim_sampai(
        &db,
        &kuota,
        &perekam,
        &JedaTetap(0),
        g.upool_id,
        10,
        Duration::from_secs(10),
    )
    .await;

    let status = status_semua(&su, &ids).await;
    let dispatched = status.iter().filter(|s| *s == "dispatched").count();
    let queued = status.iter().filter(|s| *s == "queued").count();
    assert_eq!(dispatched, 5, "token hanya sebanyak kuota efektif");
    assert_eq!(queued, 5, "yang ditolak tetap queued, tanpa transisi");
    assert_eq!(perekam.id().len(), 5, "tanpa publish untuk yang ditolak");
    for id in ids.iter().filter(|id| !perekam.id().contains(id)) {
        assert_eq!(history(&su, *id).await.len(), 1, "tanpa baris history baru");
    }
}

#[tokio::test]
#[ignore]
async fn ac8_redis_belum_siap_tidak_ada_token() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    let id = buat_request(&su, &g, &Req::queued()).await;
    let kuota = siapkan_redis(&db).await;

    let mut k = klien_redis()
        .get_multiplexed_async_connection()
        .await
        .expect("redis");
    redis::cmd("DEL")
        .arg("pr:siap")
        .query_async::<()>(&mut k)
        .await
        .expect("hapus pr:siap");

    let perekam = Perekam::default();
    let r = jalankan_putaran(
        &db,
        &kuota,
        &perekam,
        &JedaTetap(0),
        &konf(g.upool_id),
        Utc::now(),
    )
    .await
    .expect("putaran");
    assert!(r.redis_tidak_siap, "belum_siap = fail-closed");
    assert!(r.dikirim.is_empty());
    assert!(perekam.id().is_empty());
    assert_eq!(baca(&su, id).await.status, "queued");

    // Pulihkan untuk uji berikutnya.
    siapkan_redis(&db).await;
}

#[tokio::test]
#[ignore]
async fn ac8_retry_budget_habis_melewati_retry_tapi_request_baru_jalan() {
    let su = pool_su().await;
    let db = pool_controller().await;
    // rasio 0 dan minimum 0: budget retry selalu habis.
    let g = buat_graf(
        &su,
        &OpsiGraf {
            retry_rasio: "0.000",
            retry_min: 0,
            ..OpsiGraf::default()
        },
    )
    .await;
    let mut retry = Vec::new();
    for _ in 0..3 {
        retry.push(
            buat_request(
                &su,
                &g,
                &Req {
                    attempt: 1,
                    ..Req::queued()
                },
            )
            .await,
        );
    }
    let mut baru = Vec::new();
    for _ in 0..3 {
        baru.push(buat_request(&su, &g, &Req::queued()).await);
    }
    let kuota = siapkan_redis(&db).await;
    let perekam = Perekam::default();
    let ringkasan = kirim_sampai(
        &db,
        &kuota,
        &perekam,
        &JedaTetap(0),
        g.upool_id,
        6,
        Duration::from_secs(5),
    )
    .await;

    for id in &baru {
        assert_eq!(
            baca(&su, *id).await.status,
            "dispatched",
            "request baru tetap jalan"
        );
    }
    for id in &retry {
        assert_eq!(baca(&su, *id).await.status, "queued", "retry dilewati");
    }
    assert!(ringkasan.iter().map(|r| r.dilewati_retry).sum::<usize>() >= 3);
}

#[tokio::test]
#[ignore]
async fn ac9_publish_gagal_tidak_rollback_lalu_reconciler_memulihkan() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g = buat_graf(&su, &OpsiGraf::default()).await;
    let mut ids = Vec::new();
    for _ in 0..4 {
        ids.push(buat_request(&su, &g, &Req::queued()).await);
    }
    let kuota = siapkan_redis(&db).await;
    let sekarang = Utc::now();
    let r = jalankan_putaran(
        &db,
        &kuota,
        &PublisherGagal,
        &JedaTetap(0),
        &konf(g.upool_id),
        sekarang,
    )
    .await
    .expect("putaran");
    assert_eq!(r.publish_gagal, 4);
    for id in &ids {
        assert_eq!(
            baca(&su, *id).await.status,
            "dispatched",
            "status tidak di-rollback"
        );
        assert!(ada_slot(g.upool_id, *id, 1).await, "slot dipegang");
    }

    // AC-22: lewat 2 menit, reconciler mengembalikan ke queued dan melepas slot.
    let r = jalankan_siklus_reconciler(
        &db,
        Some(&kuota),
        &AcakBatasAtas,
        // Database uji dipakai bersama: request `dispatched` milik uji lain
        // juga terpilih, jadi batasnya dinaikkan supaya fixture ini pasti
        // masuk satu siklus.
        &KonfigurasiReconciler {
            batas_baris: 100_000,
            ..KonfigurasiReconciler::default()
        },
        sekarang + chrono::Duration::seconds(121),
    )
    .await
    .expect("reconciler");
    for id in &ids {
        assert!(r.dikembalikan.contains(id));
        assert_eq!(baca(&su, *id).await.status, "queued");
        assert!(!ada_slot(g.upool_id, *id, 1).await, "slot dilepas");
    }

    // Dispatcher mem-publish ulang.
    let perekam = Perekam::default();
    kirim_sampai(
        &db,
        &kuota,
        &perekam,
        &JedaTetap(0),
        g.upool_id,
        4,
        Duration::from_secs(5),
    )
    .await;
    for id in &ids {
        assert!(perekam.id().contains(id), "di-publish ulang");
        assert_eq!(baca(&su, *id).await.status, "dispatched");
    }
}

#[tokio::test]
#[ignore]
async fn ac10_jeda_di_dalam_slot_dan_tidak_lebih_dari_10_per_detik() {
    let su = pool_su().await;
    let db = pool_controller().await;
    // 10 TPS tanpa burst: satu token per 100 ms.
    let g = buat_graf(
        &su,
        &OpsiGraf {
            parameter: serde_json::json!({
                "rate": { "tps": { "t_mikro": 100_000, "burst": 1 } },
                "maks_konkurensi": 1_000
            }),
            ..OpsiGraf::default()
        },
    )
    .await;
    // Ukuran dipilih 30 request (± 3 s jam dinding), bukan 1.000 (± 100 s):
    // skrip GCRA memakai jam Redis, jadi uji ini tidak bisa dipercepat.
    for _ in 0..30 {
        buat_request(&su, &g, &Req::queued()).await;
    }
    let kuota = siapkan_redis(&db).await;
    let perekam = Perekam::default();
    // Jeda tetap 60 ms (< 100 ms slot): jeda ditarik di dalam slot token, jadi
    // throughput tetap ditentukan rate.
    kirim_sampai(
        &db,
        &kuota,
        &perekam,
        &JedaTetap(60_000),
        g.upool_id,
        30,
        Duration::from_secs(20),
    )
    .await;

    let waktu: Vec<Instant> = perekam
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|(_, _, t)| *t)
        .collect();
    assert_eq!(waktu.len(), 30);
    // Jendela bergulir 1 s (toleransi jam 5 ms untuk latensi Rust-Redis).
    let jendela = Duration::from_millis(995);
    let maks = waktu
        .iter()
        .map(|a| {
            waktu
                .iter()
                .filter(|b| **b >= *a && **b - *a < jendela)
                .count()
        })
        .max()
        .unwrap();
    println!("ac10: maks publish per detik bergulir = {maks}");
    assert!(maks <= 10, "maksimal 10 per detik bergulir, dapat {maks}");
    let durasi = *waktu.last().unwrap() - waktu[0];
    assert!(
        durasi >= Duration::from_millis(2_700),
        "30 token pada 10 TPS butuh >= 2,9 s, dapat {durasi:?}"
    );
}

#[tokio::test]
#[ignore]
async fn ac11_dua_dispatcher_paralel_tanpa_dispatch_ganda_dan_dalam_kuota() {
    let su = pool_su().await;
    let db = pool_controller().await;
    // floor(25 * 0,8) = 20.
    let g = buat_graf(
        &su,
        &OpsiGraf {
            kuota_harian: 25,
            margin: "0.200",
            ..OpsiGraf::default()
        },
    )
    .await;
    let mut ids = Vec::new();
    for _ in 0..50 {
        ids.push(buat_request(&su, &g, &Req::queued()).await);
    }
    let kuota_a = siapkan_redis(&db).await;
    let kuota_b = Kuota::baru(
        klien_redis()
            .get_multiplexed_async_connection()
            .await
            .expect("redis b"),
    )
    .await
    .expect("store b");
    // Tunggu bucket rate terisi (rebuild memulai bucket kosong).
    tokio::time::sleep(Duration::from_millis(1_200)).await;

    let (pa, pb) = (Perekam::default(), Perekam::default());
    let jeda = JedaTetap(0);
    let (ra, rb) = tokio::join!(
        kirim_sampai(
            &db,
            &kuota_a,
            &pa,
            &jeda,
            g.upool_id,
            50,
            Duration::from_secs(5)
        ),
        kirim_sampai(
            &db,
            &kuota_b,
            &pb,
            &jeda,
            g.upool_id,
            50,
            Duration::from_secs(5)
        ),
    );
    assert!(!ra.is_empty() && !rb.is_empty());

    let mut semua = pa.id();
    semua.extend(pb.id());
    let unik: std::collections::HashSet<_> = semua.iter().collect();
    assert_eq!(
        unik.len(),
        semua.len(),
        "setiap request di-dispatch paling banyak sekali"
    );
    assert!(
        semua.len() <= 20,
        "token tidak melampaui kuota efektif: {}",
        semua.len()
    );
    let dispatched = status_semua(&su, &ids)
        .await
        .into_iter()
        .filter(|s| s == "dispatched")
        .count();
    assert_eq!(dispatched, semua.len(), "status sama dengan publish");
    for id in &semua {
        let h = history(&su, *id).await;
        assert_eq!(
            h.iter().filter(|b| b.1 == "dispatched").count(),
            1,
            "tepat satu transisi per request"
        );
    }
}

/// Graf kedua yang ikut upstream pool `g` lewat downstream pool dan endpoint
/// baru (F0 mengizinkan banyak downstream pool ke satu upstream pool).
async fn tambah_downstream_pool(su: &PgPool, g: &Graf) -> Graf {
    let d = Graf {
        dpool_id: Uuid::now_v7(),
        endpoint_id: Uuid::now_v7(),
        ..*g
    };
    sqlx::query(
        "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas, ttl_antrean_detik)
         VALUES ($1, $2, 1, 3600)",
    )
    .bind(d.dpool_id)
    .bind(format!("arc229-{}", d.dpool_id.simple()))
    .execute(su)
    .await
    .expect("downstream_pool");
    buat_endpoint(su, &d, d.endpoint_id).await;
    sqlx::query(
        "INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id) VALUES ($1, $2)",
    )
    .bind(d.dpool_id)
    .bind(d.upool_id)
    .execute(su)
    .await
    .expect("downstream_pool_rute");
    d
}

#[tokio::test]
#[ignore]
async fn kandidat_urut_prioritas_lintas_downstream_pool_dan_hanya_milik_pool_ini() {
    let su = pool_su().await;
    let db = pool_controller().await;
    let g1 = buat_graf(&su, &OpsiGraf::default()).await;
    let g2 = tambah_downstream_pool(&su, &g1).await;
    let lain = buat_graf(&su, &OpsiGraf::default()).await;

    // Prioritas 2 di downstream pool pertama, prioritas 1 di yang kedua, dan
    // satu request menunggu retry (upstream_pool_id terisi) prioritas 1.
    let mut rendah = Vec::new();
    for _ in 0..3 {
        let r = Req {
            prioritas: 2,
            ..Req::queued()
        };
        rendah.push(buat_request(&su, &g1, &r).await);
    }
    let mut tinggi = Vec::new();
    for _ in 0..3 {
        tinggi.push(buat_request(&su, &g2, &Req::queued()).await);
    }
    let retry = buat_request(
        &su,
        &g1,
        &Req {
            attempt: 1,
            ..Req::queued()
        },
    )
    .await;
    tinggi.push(retry);

    // Bukan milik pool ini: antrean upstream pool lain, dan request yang
    // menunggu retry di upstream pool lain walau downstream pool-nya sama.
    let asing = buat_request(&su, &lain, &Req::queued()).await;
    let g_campur = Graf {
        upool_id: lain.upool_id,
        provider_id: lain.provider_id,
        ..g1
    };
    let retry_asing = buat_request(
        &su,
        &g_campur,
        &Req {
            attempt: 1,
            ..Req::queued()
        },
    )
    .await;

    let kuota = siapkan_redis(&db).await;
    let perekam = Perekam::default();
    kirim_sampai(
        &db,
        &kuota,
        &perekam,
        &JedaTetap(0),
        g1.upool_id,
        7,
        Duration::from_secs(10),
    )
    .await;

    let terkirim = perekam.id();
    assert_eq!(terkirim.len(), 7, "hanya tujuh request milik pool ini");
    let himpunan = |ids: &[Uuid]| {
        ids.iter()
            .copied()
            .collect::<std::collections::HashSet<Uuid>>()
    };
    assert_eq!(
        himpunan(&terkirim[..4]),
        himpunan(&tinggi),
        "prioritas 1 lintas downstream pool dan retry habis dulu"
    );
    assert_eq!(himpunan(&terkirim[4..]), himpunan(&rendah));
    for id in [asing, retry_asing] {
        assert_eq!(baca(&su, id).await.status, "queued", "bukan milik pool ini");
    }
}

/// Isi antrean `n` request `queued` untuk satu downstream pool lewat satu
/// INSERT. Prioritas berputar 1..3. Tanpa history: dipakai di dalam transaksi
/// yang di-rollback, jadi tidak ada sisa di database uji bersama.
async fn isi_antrean(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    g: &Graf,
    n: i64,
    attempt: i32,
) {
    sqlx::query(
        "INSERT INTO request
                (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id,
                 provider_id, upstream_pool_id, provider_config_version, endpoint_config_version,
                 mode, prioritas, prioritas_efektif, status, metadata, input_payload, received_at,
                 attempt_count, ttl_expires_at, dispatch_key)
         SELECT gen_random_uuid(), $1, $2, $3, $4, $5, CASE WHEN $7::int > 0 THEN $6::uuid END, 1, 1,
                'async', 1 + (i % 3), 1 + (i % 3), 'queued', '{}'::jsonb,
                '{\"nik\": \"0012345678901234\"}'::jsonb, now(), $7::int, now() + interval '1 hour',
                gen_random_uuid()
           FROM generate_series(1, $8::bigint) AS i",
    )
    .bind(g.client_id)
    .bind(g.kredensial_id)
    .bind(g.endpoint_id)
    .bind(g.dpool_id)
    .bind(g.provider_id)
    .bind(g.upool_id)
    .bind(attempt)
    .bind(n)
    .execute(&mut **tx)
    .await
    .expect("isi antrean");
}

/// Rencana kueri kandidat, dibaca sebagai teks, untuk rencana khusus (nilai
/// terikat) dan rencana generik (sqlx menyimpan statement per koneksi, dan
/// PostgreSQL pindah ke rencana generik setelah lima eksekusi).
async fn rencana_calon(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    mode: &str,
    pool: Uuid,
) -> String {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SET LOCAL plan_cache_mode = {mode}"
    )))
    .execute(&mut **tx)
    .await
    .expect("plan_cache_mode");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "PREPARE calon_229 (uuid, timestamptz, uuid[]) AS {}",
        poolrouter_controller::dispatcher::sql_calon()
    )))
    .execute(&mut **tx)
    .await
    .expect("prepare kueri kandidat");
    // Literal, bukan parameter terikat: EXPLAIN adalah pernyataan utilitas.
    let baris: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "EXPLAIN EXECUTE calon_229('{pool}', now(), '{{}}')"
    )))
    .fetch_all(&mut **tx)
    .await
    .expect("explain");
    sqlx::query("DEALLOCATE calon_229")
        .execute(&mut **tx)
        .await
        .expect("dealocate");
    baris
        .into_iter()
        .map(|(b,)| b)
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
#[ignore]
async fn kandidat_memakai_request_dispatch_idx_pada_antrean_besar() {
    let su = pool_su().await;
    let g1 = buat_graf(&su, &OpsiGraf::default()).await;
    let g2 = tambah_downstream_pool(&su, &g1).await;
    let g3 = tambah_downstream_pool(&su, &g1).await;
    let lain = buat_graf(&su, &OpsiGraf::default()).await;

    // Semua di dalam satu transaksi superuser yang di-rollback: antrean besar
    // (pool ini tiga downstream pool, ditambah antrean upstream pool lain
    // yang lebih besar dan berprioritas sama) tidak tersisa di database bersama.
    let mut tx = su.begin().await.expect("tx");
    for g in [&g1, &g2, &g3] {
        isi_antrean(&mut tx, g, 10_000, 0).await;
    }
    isi_antrean(&mut tx, &g1, 500, 1).await;
    isi_antrean(&mut tx, &lain, 30_000, 0).await;
    sqlx::query("ANALYZE request")
        .execute(&mut *tx)
        .await
        .expect("analyze");

    for mode in ["force_custom_plan", "force_generic_plan"] {
        let rencana = rencana_calon(&mut tx, mode, g1.upool_id).await;
        println!("--- rencana {mode}\n{rencana}");
        assert!(
            rencana.contains("request_dispatch_idx"),
            "{mode}: indeks parsial dispatcher tidak dipakai:\n{rencana}"
        );
        assert!(
            !rencana.contains("Seq Scan on request"),
            "{mode}: seq scan pada request:\n{rencana}"
        );
    }
    tx.rollback().await.expect("rollback");
}
