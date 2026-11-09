//! Uji seam publik `core::data_balikan` (AC-15–AC-19, spec
//! `f1-webhook-retry-data-balikan` T3).
//!
//! Hanya API publik yang disentuh: konfigurasi skema 2, pemicu, jadwal batch,
//! port `PengirimDataBalikan` beserta adapter tiruan, dan kerahasiaan NIK.
//! Body sintetis, NIK sintetis (aturan repo: data NIK hanya sintetis), tanpa
//! mock jaringan.

use chrono::{DateTime, Utc};
use futures::stream;
use poolrouter_core::admin::Nik;
use poolrouter_core::data_balikan::{
    BatchKeluar, GalatDataBalikan, HasilAck, HasilDiprogam, HasilKirimBatch, ItemDataBalikan,
    KonfigurasiDataBalikan, PemicuBatch, PengirimDataBalikan, PengirimTiruan, evaluasi_pemicu,
    jeda_batch, kebijakan_batch,
};
use poolrouter_core::retry::AcakBatasAtas;
use serde_json::json;

const NIK: &str = "3201010101010001";

fn konfigurasi(jumlah_n: u32) -> KonfigurasiDataBalikan {
    KonfigurasiDataBalikan::parse(
        2,
        &json!({
            "pemicu_jumlah_n": jumlah_n,
            "periode": { "jenis": "harian", "jam": "00:00", "zona": "Asia/Jakarta" },
            "toleransi_detik": 3600,
            "ukuran_batch_maks": 10_000,
            "kirim_batas_percobaan": 5,
            "backoff_base_detik": 300,
            "backoff_cap_detik": 3600,
            "endpoint": "https://contoh.test/balikan",
            "rujukan_secret": "{{secret.DUKCAPIL_BALIKAN}}",
            "format_version": "tiruan-1"
        }),
    )
    .expect("konfigurasi sah")
}

fn t(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn batch(jumlah_item: u32) -> BatchKeluar {
    BatchKeluar {
        batch_id: uuid::Uuid::from_u128(7),
        provider_id: uuid::Uuid::from_u128(9),
        pemicu: "jumlah_n".to_owned(),
        jumlah_item,
        format_version: "tiruan-1".to_owned(),
    }
}

// AC-15: konfigurasi skema 2.
#[test]
fn ac15_konfigurasi_skema_2_terbaca_lengkap() {
    let k = konfigurasi(250);
    assert_eq!(k.pemicu_jumlah_n, 250);
    assert_eq!(k.ukuran_batch_maks, 10_000);
    assert_eq!(k.kirim_batas_percobaan, 5);
    assert_eq!(k.backoff_base_detik, 300);
    assert_eq!(k.backoff_cap_detik, 3_600);
    assert_eq!(k.endpoint, "https://contoh.test/balikan");
    assert_eq!(k.rujukan_secret, "{{secret.DUKCAPIL_BALIKAN}}");
    assert_eq!(k.format_version, "tiruan-1");
    assert!(k.periode.harian());
    assert_eq!(k.periode.jam_kirim, (0, 0));
    assert_eq!(k.periode.zona, "Asia/Jakarta");
}

#[test]
fn ac15_default_wb11_diisi_untuk_field_yang_tidak_ada() {
    // `endpoint` dan `rujukan_secret` wajib (tidak punya default WB11).
    let k = KonfigurasiDataBalikan::parse(
        2,
        &json!({
            "endpoint": "https://contoh.test/balikan",
            "rujukan_secret": "{{secret.X}}"
        }),
    )
    .expect("blok minimal sah");
    assert_eq!(k.pemicu_jumlah_n, 10_000);
    assert_eq!(k.ukuran_batch_maks, 10_000);
    assert_eq!(k.toleransi_detik, 3_600);
    assert_eq!(k.periode.jam_kirim, (0, 0));
    assert_eq!(k.periode.zona, "Asia/Jakarta");
    assert_eq!(k.format_version, "tiruan-1");
}

#[test]
fn ac15_setiap_aturan_penolakan_berlaku() {
    // Satu baris per aturan AC-15, supaya tabel spec bisa diaudit langsung.
    let kasus = [
        json!({ "pemicu_jumlah_n": 0 }),
        json!({ "pemicu_jumlah_n": 10_001 }),
        json!({ "ukuran_batch_maks": 0 }),
        json!({ "ukuran_batch_maks": 10_001 }),
        json!({ "kirim_batas_percobaan": 0 }),
        json!({ "kirim_batas_percobaan": 11 }),
        json!({ "backoff_base_detik": 3_601, "backoff_cap_detik": 3_600 }),
        json!({ "endpoint": "http://contoh.test" }),
        json!({ "rujukan_secret": "DUKCAPIL_BALIKAN" }),
        // Tanpa nilai kosong sebagai jalan keluar, termasuk blok kosong.
        json!({}),
        json!({ "endpoint": "", "rujukan_secret": "{{secret.X}}" }),
        json!({ "endpoint": "https://contoh.test", "rujukan_secret": "" }),
    ];
    for (i, v) in kasus.iter().enumerate() {
        assert!(
            KonfigurasiDataBalikan::parse(2, v).is_err(),
            "kasus {i} seharusnya ditolak: {v}"
        );
    }
}

#[test]
fn ac15_skema_1_ditolak() {
    assert_eq!(
        KonfigurasiDataBalikan::parse(1, &json!({})),
        Err(GalatDataBalikan::SkemaTerlaluTua(1))
    );
}

// AC-16: pemicu jumlah-n dan periode.
#[test]
fn ac16_pemicu_jumlah_n_pada_ambang() {
    let k = konfigurasi(100);
    let sekarang = t("2026-11-05T10:00:00Z");
    let tertua = Some(t("2026-11-05T00:00:00Z"));
    assert_eq!(
        evaluasi_pemicu(99, tertua, sekarang, &k, None),
        PemicuBatch::Tidak
    );
    assert_eq!(
        evaluasi_pemicu(100, tertua, sekarang, &k, None),
        PemicuBatch::JumlahN
    );
}

#[test]
fn ac16_pemicu_periode_harian_asia_jakarta() {
    let k = konfigurasi(100);
    // 01.00 WIB 5 Nov; batch terakhir 3 Nov; ada item eligible lebih tua
    // dari 00.00 WIB 5 Nov.
    let hasil = evaluasi_pemicu(
        10,
        Some(t("2026-11-04T16:00:00Z")),
        t("2026-11-04T18:00:00Z"),
        &k,
        Some(t("2026-11-03T17:00:00Z")),
    );
    assert_eq!(hasil, PemicuBatch::Periode(t("2026-11-04T17:00:00Z")));
    assert_eq!(hasil.sebagai_kolom(), Some("periode"));
}

// AC-17: jadwal retry batch memakai ulang kebijakan retry webhook.
#[test]
fn ac17_jadwal_batch_full_jitter_memakai_kebijakan_retry_yang_sama() {
    let k = konfigurasi(100);
    let kebijakan = kebijakan_batch(&k);
    // base 300 s, cap 3600 s: batas atas 300, 600, 1200, 2400, 3600 detik.
    for (n, harap) in [
        (1_u32, 300_i64),
        (2, 600),
        (3, 1_200),
        (4, 2_400),
        (5, 3_600),
    ] {
        assert_eq!(
            jeda_batch(n, &kebijakan, &AcakBatasAtas),
            chrono::Duration::seconds(harap),
            "n = {n}"
        );
    }
}

// AC-18: port + adapter tiruan.
async fn kontrak_pengirim<P: PengirimDataBalikan>(pengirim: std::sync::Arc<P>) {
    let body = || {
        Box::new(stream::iter(vec![bytes::Bytes::from_static(
            b"{\"item\":[]}",
        )]))
    };
    assert!(matches!(
        pengirim.clone().kirim(batch(10), body()).await,
        HasilKirimBatch::GagalSementara { .. }
    ));
    assert!(matches!(
        pengirim.clone().kirim(batch(10), body()).await,
        HasilKirimBatch::Diterima { .. }
    ));
}

#[tokio::test]
async fn ac18_adapter_tiruan_bisa_diprogam_per_percobaan() {
    let pengirim = std::sync::Arc::new(PengirimTiruan::dengan_program(vec![
        HasilDiprogam::GagalSementara,
        HasilDiprogam::Diterima,
    ]));
    kontrak_pengirim(pengirim.clone()).await;
    let rekaman = pengirim.rekaman();
    assert_eq!(rekaman.len(), 2);
    assert_eq!(rekaman[0].batch_id, rekaman[1].batch_id);
    assert_eq!(rekaman[0].jumlah_item, 10);
    assert_eq!(pengirim.jumlah_kirim(batch(10).batch_id), 2);
}

#[tokio::test]
async fn ac18_ack_tersedia_lewat_port() {
    let pengirim = std::sync::Arc::new(PengirimTiruan::baru());
    let b = batch(3);
    assert_eq!(
        pengirim.clone().periksa_ack(b.batch_id, "ref-1").await,
        HasilAck::BelumAda
    );
    pengirim.program_ack(
        b.batch_id,
        "ref-1",
        HasilAck::Diterima {
            ack_ref: "ack-1".to_owned(),
        },
    );
    assert_eq!(
        pengirim.clone().periksa_ack(b.batch_id, "ref-1").await,
        HasilAck::Diterima {
            ack_ref: "ack-1".to_owned()
        }
    );
}

#[tokio::test]
async fn ac18_dikirim_membawa_ref_untuk_periksa_ack() {
    let pengirim =
        std::sync::Arc::new(PengirimTiruan::dengan_program(vec![HasilDiprogam::Dikirim]));
    let b = batch(4);
    let HasilKirimBatch::Dikirim { r#ref } = pengirim
        .clone()
        .kirim(batch(4), Box::new(stream::iter(Vec::<bytes::Bytes>::new())))
        .await
    else {
        panic!("program Dikirim harus menghasilkan Dikirim");
    };
    pengirim.program_ack(
        b.batch_id,
        &r#ref,
        HasilAck::Diterima {
            ack_ref: "ack-9".to_owned(),
        },
    );
    assert_eq!(
        pengirim.clone().periksa_ack(b.batch_id, &r#ref).await,
        HasilAck::Diterima {
            ack_ref: "ack-9".to_owned()
        }
    );
}

#[tokio::test]
async fn ac18_panik_setelah_kirim_tetap_tercatat() {
    let pengirim = std::sync::Arc::new(PengirimTiruan::dengan_program(vec![
        HasilDiprogam::PanikSetelahKirim,
    ]));
    let hasil =
        futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(pengirim.clone().kirim(
            batch(2),
            Box::new(stream::iter(vec![bytes::Bytes::from_static(b"x")])),
        )))
        .await;
    assert!(hasil.is_err());
    assert_eq!(pengirim.jumlah_kirim(batch(2).batch_id), 1);
}

// AC-19: NIK tidak tercetak.
#[test]
fn ac19_item_tidak_menampilkan_nik_atau_field_input() {
    let item = ItemDataBalikan::baru(
        uuid::Uuid::nil(),
        Nik::parse(NIK).expect("NIK sintetis 16 digit"),
        json!({ "nama_lgkp": "BUDI" }),
    );
    let tercetak = format!("{}/{}/{:?}", item.nik(), item.field_input(), item.nik());
    assert!(!tercetak.contains(NIK), "{tercetak}");
    assert!(!tercetak.contains("BUDI"), "{tercetak}");
    // Nilai masih bisa dibaca adapter secara eksplisit.
    assert_eq!(*item.nik().expose(), NIK);
}
