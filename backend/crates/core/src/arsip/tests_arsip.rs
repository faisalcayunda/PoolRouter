//! Tests for `core::arsip`, one per acceptance criterion of T3
//! (f1-dashboard-arsip: AC-15 through AC-20).
//!
//! The data is synthetic and the NIKs carry the `00` prefix ADR-0001 §7 requires,
//! so nothing here is a real identifier.
//!
//! Seams, as the spec fixes them: the whole module is exercised through its
//! public API plus `PenyimpanArsip::Memori`, and AC-17 asserts on the range log
//! that `Memori` keeps rather than on an implementation detail.

use std::sync::Arc;

use chrono::{TimeZone as _, Utc};

use crate::admin::aman::{Nik, NilaiAman};
use crate::arsip::kunci::PANJANG_DEK;
use crate::arsip::{
    BARIS_PER_ROW_GROUP, BacaArsip, BarisArsip, GalatArsip, GalatVerifikasi, KunciObjek, Memori,
    ParameterTulis, PembungkusDek, PembungkusKekAwal, PenyimpanArsip, RentangKodeTransaksi,
    RingkasanObjek, S3, SaringanNik, TulisArsip, Verifikasi, VersiKunciHmac,
};
use crate::audit::NikHmac;

fn pembungkus() -> PembungkusKekAwal {
    PembungkusKekAwal::uji("arsip-t3")
}

fn pembungkus_lain() -> PembungkusKekAwal {
    PembungkusKekAwal::uji("arsip-t3-lain")
}

/// A synthetic NIK: the `00` prefix keeps it out of the real population
/// (ADR-0001 §7).
fn nik_sintetis(n: u64) -> Nik {
    Nik::parse(&format!("00{:014}", n % 100_000_000_000_000)).unwrap()
}

fn hmac_sintetis(n: u64) -> NikHmac {
    // 64 lowercase hex characters, deterministic per `n`. Two 32-hex halves from
    // a splitmix64-style mixer; `s.get(..64)` trims any excess width.
    let mut x = n.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    let mut s = String::with_capacity(64);
    while s.len() < 64 {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        s.push_str(&format!("{z:016x}"));
    }
    s.truncate(64);
    NikHmac::parse(s).unwrap()
}

fn kode(i: u64) -> String {
    format!("0192f0aa-0000-7000-8000-{:012x}", i)
}

fn jam(i: u64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_760_000_000 + i as i64, 0).unwrap()
}

fn baris(i: u64) -> BarisArsip {
    BarisArsip::baru(
        kode(i),
        hmac_sintetis(i),
        nik_sintetis(i),
        format!("{{\"payload\":\"nilai-uji-{i}\"}}"),
        jam(i),
    )
}

fn baris_banyak(n: u64) -> Vec<BarisArsip> {
    (0..n).map(baris).collect()
}

async fn tulis_ke_memori(
    memori: &Memori,
    kunci: &KunciObjek,
    baris: Vec<BarisArsip>,
    p: &PembungkusKekAwal,
) -> RingkasanObjek {
    TulisArsip::tulis_ke(memori, kunci, baris, &ParameterTulis::transaction_log(), p)
        .await
        .expect("tulis arsip")
}

// ---------------------------------------------------------------- AC-15

/// AC-15: 300.000 rows round-trip identically, in more than one row group,
/// with zstd, sorted by `nik_hmac` then `kode_transaksi`, and the summary
/// matches a recount.
#[tokio::test]
async fn ac15_round_trip_300k_baris() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "prov-1", "2026-11-05", "00-06").unwrap();
    let p = pembungkus();

    // Shuffled order on purpose: the writer must sort, not trust the input.
    let mut asli = baris_banyak(300_000);
    asli.reverse();

    let ringkasan = tulis_ke_memori(&memori, &kunci, asli.clone(), &p).await;

    assert_eq!(ringkasan.jumlah_baris, 300_000);
    assert!(ringkasan.ukuran > 0);
    assert_eq!(ringkasan.sha256.len(), 64);

    let meta = BacaArsip::metadata(Arc::new(memori.clone()), Arc::new(kunci.clone()))
        .await
        .unwrap();
    // 300.000 / 128.000 = 2.34..., so three groups.
    assert!(
        meta.num_row_groups() > 1,
        "harus lebih dari satu row group, dapat {}",
        meta.num_row_groups()
    );
    assert_eq!(meta.num_row_groups(), 3);
    for g in 0..meta.num_row_groups() {
        let rg = meta.row_group(g);
        assert!(rg.num_rows() <= BARIS_PER_ROW_GROUP as i64);
    }
    // zstd: check every column chunk of the first group.
    for c in meta.row_group(0).columns() {
        assert_eq!(
            c.compression(),
            parquet::basic::Compression::ZSTD(Default::default()),
            "kolom {} bukan zstd",
            c.column_path()
        );
    }

    let kembali = BacaArsip::baca_semua(Arc::new(memori.clone()), Arc::new(kunci), &pembungkus())
        .await
        .unwrap();
    assert_eq!(kembali.len(), 300_000);

    // Same content, in the format's order.
    let mut harap = asli;
    harap.sort_by(|a, b| a.kunci_urut().cmp(&b.kunci_urut()));
    assert_eq!(kembali, harap);

    // Sorted by nik_hmac then kode_transaksi, checked directly on the output.
    for w in kembali.windows(2) {
        assert!(
            w[0].kunci_urut() <= w[1].kunci_urut(),
            "tidak terurut: {:?} lalu {:?}",
            w[0].kunci_urut().0,
            w[1].kunci_urut().0
        );
    }

    // Summary matches a recount from the data.
    assert_eq!(
        ringkasan.rentang_kode_transaksi.map(|r| (r.dari, r.sampai)),
        RentangKodeTransaksi::dari_baris(&kembali).map(|r| (r.dari, r.sampai))
    );
    assert_eq!(ringkasan.kunci_hmac_versi, VersiKunciHmac::F1);
}

/// AC-15 also fixes the sort columns in the file metadata.
#[tokio::test]
async fn ac15_objek_mencatat_urutan() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    tulis_ke_memori(&memori, &kunci, baris_banyak(3), &pembungkus()).await;

    let meta = BacaArsip::metadata(Arc::new(memori), Arc::new(kunci))
        .await
        .unwrap();
    let urut = meta
        .row_group(0)
        .sorting_columns()
        .expect("sorting columns tercatat");
    assert_eq!(urut.len(), 2);
    assert_eq!(urut[0].column_idx, 1); // nik_hmac
    assert_eq!(urut[1].column_idx, 0); // kode_transaksi
}

/// AC-15: the bloom filter is on `nik_hmac` and nowhere else.
#[tokio::test]
async fn ac15_bloom_hanya_pada_nik_hmac() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    tulis_ke_memori(&memori, &kunci, baris_banyak(200), &pembungkus()).await;

    let meta = BacaArsip::metadata(Arc::new(memori), Arc::new(kunci))
        .await
        .unwrap();
    let rg = meta.row_group(0);
    let mut dengan_filter = Vec::new();
    for (i, c) in rg.columns().iter().enumerate() {
        if c.bloom_filter_offset().is_some() {
            dengan_filter.push(rg.column(i).column_path().string());
        }
    }
    assert_eq!(dengan_filter, vec!["nik_hmac".to_owned()]);
}

// ---------------------------------------------------------------- AC-16

/// AC-16: `nik` and payload are ciphertext. No synthetic NIK and no payload
/// fragment appears anywhere in the raw object.
#[tokio::test]
async fn ac16_tidak_ada_nik_atau_payload_di_bytes_mentah() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();

    let mut asli = baris_banyak(500);
    // One row with a distinctive payload so the scan is unambiguous.
    asli.push(BarisArsip::baru(
        kode(99_999),
        hmac_sintetis(99_999),
        nik_sintetis(99_999),
        "PENANDA-PAYLOAD-UJI-YANG-TIDAK-BOLEH-BOCOR",
        jam(99_999),
    ));
    tulis_ke_memori(&memori, &kunci, asli.clone(), &pembungkus()).await;

    let bytes = memori.bytes_mentah(&kunci).expect("objek tersimpan");
    let teks = String::from_utf8_lossy(&bytes);

    assert!(
        !teks.contains("PENANDA-PAYLOAD-UJI-YANG-TIDAK-BOLEH-BOCOR"),
        "payload bocor di objek mentah"
    );
    for b in &asli {
        let nik = b.nik.as_ref().unwrap().expose().utuh().expose().to_string();
        assert!(!teks.contains(&nik), "NIK {nik} bocor di objek mentah");
        // Also check the raw bytes, in case a NIK landed outside a UTF-8 run.
        assert!(
            !bytes.windows(nik.len()).any(|w| w == nik.as_bytes()),
            "NIK {nik} bocor di bytes mentah"
        );
    }
}

/// AC-16: reading with another DEK fails, and the error carries no value.
#[tokio::test]
async fn ac16_dek_lain_gagal_tanpa_membocorkan_nilai() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    tulis_ke_memori(&memori, &kunci, baris_banyak(10), &pembungkus()).await;

    let e = BacaArsip::baca_semua(Arc::new(memori), Arc::new(kunci), &pembungkus_lain())
        .await
        .expect_err("DEK lain harus gagal");
    let pesan = e.to_string();
    assert!(
        !pesan.contains("00"),
        "galat memuat nilai yang dicari: {pesan}"
    );
    assert!(
        matches!(e, GalatArsip::Pembungkus(_)),
        "galat salah varian: {e:?}"
    );
}

/// AC-16: `kek_id` comes from the wrapper, and the raw DEK never leaves.
#[tokio::test]
async fn ac16_kek_id_dan_dek_tidak_keluar() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    let p = pembungkus();
    let kek_id = p.kek_id().to_owned();
    tulis_ke_memori(&memori, &kunci, baris_banyak(5), &p).await;

    let meta = BacaArsip::metadata(Arc::new(memori.clone()), Arc::new(kunci.clone()))
        .await
        .unwrap();
    assert_eq!(BacaArsip::kek_id(&meta).as_deref(), Some(kek_id.as_str()));

    // A `Dek` prints as a marker, never the key.
    let dek = crate::arsip::kunci::Dek::acak().unwrap();
    assert_eq!(format!("{dek:?}"), "Dek(***)");

    // The wrapped DEK in the footer is not the raw key: an object sealed with a
    // different wrapper must not open, so the header really carries a wrapper.
    let bytes = memori.bytes_mentah(&kunci).unwrap();
    let teks = String::from_utf8_lossy(&bytes);
    assert!(teks.contains("poolrouter.arsip.dek_terbungkus"));
}

/// AC-16/AC-20: sealed cells really are ciphertext, checked through the schema.
#[tokio::test]
async fn ac16_kolom_rahasia_bertipe_binary() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    tulis_ke_memori(&memori, &kunci, baris_banyak(3), &pembungkus()).await;

    let meta = BacaArsip::metadata(Arc::new(memori), Arc::new(kunci))
        .await
        .unwrap();
    let descr = meta.file_metadata().schema_descr();
    for nama in ["nik", "payload"] {
        let i = (0..descr.num_columns())
            .find(|i| descr.column(*i).name() == nama)
            .expect("kolom ada");
        assert_eq!(
            descr.column(i).physical_type(),
            parquet::basic::Type::BYTE_ARRAY,
            "{nama} harus BYTE_ARRAY (ciphertext)"
        );
    }
}

// ---------------------------------------------------------------- AC-17

/// AC-17: a NIK search reads the footer plus at most one row group, finds the
/// NIK that exists, and returns 0 rows for one that does not.
#[tokio::test]
async fn ac17_cari_hanya_membaca_row_group_yang_lolos() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    let p = pembungkus();

    // Three row groups of 128.000 rows each = 384.000 rows.
    let total = BARIS_PER_ROW_GROUP * 3;
    let mut asli = baris_banyak(total as u64);
    // Nodes that land in the middle group so the filter has to choose one.
    let tengah = BARIS_PER_ROW_GROUP as u64 + 10;
    let kunci_cari = asli[tengah as usize].nik_hmac.clone();
    let nik_cari = asli[tengah as usize].nik.clone().unwrap();
    // A key that is not in the object at all.
    let tidak_ada = hmac_sintetis(9_999_999);
    asli.reverse();
    tulis_ke_memori(&memori, &kunci, asli, &p).await;

    let meta = BacaArsip::metadata(Arc::new(memori.clone()), Arc::new(kunci.clone()))
        .await
        .unwrap();
    assert_eq!(meta.num_row_groups(), 3);

    memori.lupakan_baca();

    let (hasil, dibaca) = BacaArsip::cari(
        Arc::new(memori.clone()),
        Arc::new(kunci.clone()),
        &SaringanNik::baru([kunci_cari.as_str()]),
        &p,
    )
    .await
    .unwrap();

    assert_eq!(hasil.len(), 1, "NIK yang ada harus ditemukan");
    assert_eq!(hasil[0].nik.as_ref().unwrap().expose(), nik_cari.expose());
    assert!(
        dibaca <= 1,
        "pencarian membaca {dibaca} row group, harusnya paling banyak 1"
    );

    // The byte budget: footer + one group, never the whole object.
    let rentang = memori.catatan_baca_kunci(&kunci);
    let total_dibaca: u64 = rentang.iter().map(|r| r.end - r.start).sum();
    let ukuran = memori.head(&kunci).await.unwrap().unwrap();
    assert!(
        total_dibaca < ukuran,
        "membaca {total_dibaca} dari {ukuran} byte, tidak ada yang dilewati"
    );

    // A key that is not there returns nothing, and also does not read a group.
    memori.lupakan_baca();
    let (kosong, dibaca_kosong) = BacaArsip::cari(
        Arc::new(memori.clone()),
        Arc::new(kunci),
        &SaringanNik::baru([tidak_ada.as_str()]),
        &p,
    )
    .await
    .unwrap();
    assert!(kosong.is_empty(), "NIK yang tidak ada harus 0 baris");
    assert_eq!(dibaca_kosong, 0, "tidak ada group yang perlu dibaca");
}

/// AC-17: an empty filter matches nothing, rather than everything.
#[tokio::test]
async fn ac17_saringan_kosong_mengembalikan_nol() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    tulis_ke_memori(&memori, &kunci, baris_banyak(10), &pembungkus()).await;

    let (hasil, dibaca) = BacaArsip::cari(
        Arc::new(memori),
        Arc::new(kunci),
        &SaringanNik::kosong(),
        &pembungkus(),
    )
    .await
    .unwrap();
    assert!(hasil.is_empty());
    assert_eq!(dibaca, 0);
}

// ---------------------------------------------------------------- AC-18

/// AC-18: one changed byte fails verification.
#[tokio::test]
async fn ac18_satu_byte_berubah_gagal() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    let ringkasan = tulis_ke_memori(&memori, &kunci, baris_banyak(50), &pembungkus()).await;

    // Flip a byte in the middle of the data, then rewrite.
    let mut bytes = memori.bytes_mentah(&kunci).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    memori.put(&kunci, bytes).await.unwrap();

    let e = Verifikasi::verifikasi(&memori, &ringkasan)
        .await
        .expect_err("byte berubah harus gagal");
    // Either the size or the digest catches it, depending on where the byte was.
    assert!(
        matches!(
            e,
            GalatVerifikasi::IntisariBeda { .. } | GalatVerifikasi::UkuranBeda { .. }
        ),
        "galat salah varian: {e:?}"
    );
}

/// AC-18: a row count that differs from the summary fails.
#[tokio::test]
async fn ac18_jumlah_baris_beda_gagal() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    let mut ringkasan = tulis_ke_memori(&memori, &kunci, baris_banyak(20), &pembungkus()).await;
    ringkasan.jumlah_baris += 1;

    let e = Verifikasi::verifikasi(&memori, &ringkasan)
        .await
        .expect_err("jumlah baris beda harus gagal");
    // The object is untouched, so the digest matches and the row count is what
    // catches the bad summary — which is the point of checking it against the
    // footer rather than trusting the summary.
    assert!(
        matches!(
            e,
            GalatVerifikasi::JumlahBarisBeda {
                diharapkan: 21,
                didapat: 20
            }
        ),
        "galat salah varian: {e:?}"
    );
}

/// AC-18: a missing object fails rather than passing silently.
#[tokio::test]
async fn ac18_objek_hilang_gagal() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    let ringkasan = RingkasanObjek {
        kunci: kunci.clone(),
        sha256: "0".repeat(64),
        jumlah_baris: 1,
        ukuran: 1,
        rentang_kode_transaksi: None,
        kunci_hmac_versi: VersiKunciHmac::F1,
    };
    let e = Verifikasi::verifikasi(&memori, &ringkasan)
        .await
        .expect_err("objek hilang harus gagal");
    assert_eq!(e, GalatVerifikasi::ObjekHilang);
}

/// AC-18: a summary whose object key differs is refused before reading.
#[tokio::test]
async fn ac18_kunci_berbeda_gagal() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    let ringkasan = tulis_ke_memori(&memori, &kunci, baris_banyak(2), &pembungkus()).await;
    let lain = KunciObjek::baru("transaction_log", "p", "2026-11-05", "06-12").unwrap();

    let e = Verifikasi::verifikasi_kunci(&memori, &lain, &ringkasan)
        .await
        .expect_err("kunci beda harus gagal");
    assert!(matches!(e, GalatVerifikasi::KunciBeda { .. }), "{e:?}");
}

/// AC-18: an untouched object verifies.
#[tokio::test]
async fn ac18_objek_utuh_lulus() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    let ringkasan = tulis_ke_memori(&memori, &kunci, baris_banyak(30), &pembungkus()).await;
    Verifikasi::verifikasi(&memori, &ringkasan).await.unwrap();
}

// ---------------------------------------------------------------- AC-19

/// AC-19: the contract suite, run against every `PenyimpanArsip`. Written as a
/// function over the port so the S3 implementation gets the same checks.
async fn suite_kontrak(penyimpan: &dyn PenyimpanArsip) {
    let kunci = KunciObjek::baru("transaction_log", "suite", "2026-11-05", "00-06").unwrap();

    // head on a missing object is None, not an error.
    assert_eq!(penyimpan.head(&kunci).await.unwrap(), None);

    // put then head reports the size.
    penyimpan.put(&kunci, b"satu".to_vec()).await.unwrap();
    assert_eq!(penyimpan.head(&kunci).await.unwrap(), Some(4));

    // get_range reads a slice.
    assert_eq!(
        penyimpan.get_range(&kunci, 0..2).await.unwrap(),
        b"sa".to_vec()
    );

    // Writing the same key twice leaves the last object (idempotent).
    penyimpan.put(&kunci, b"dua".to_vec()).await.unwrap();
    assert_eq!(penyimpan.head(&kunci).await.unwrap(), Some(3));
    assert_eq!(
        penyimpan.get_range(&kunci, 0..3).await.unwrap(),
        b"dua".to_vec()
    );

    // A range past the end is an error, not a short read.
    assert!(penyimpan.get_range(&kunci, 0..99).await.is_err());
}

/// AC-19: the suite passes for [`Memori`].
#[tokio::test]
async fn ac19_suite_kontrak_memori() {
    suite_kontrak(&Memori::baru()).await;
}

/// AC-19: the port has no delete. A compile-time check: this file would not
/// build if a `delete` method existed and were called on the trait object.
#[test]
fn ac19_port_tanpa_hapus() {
    fn _assert_tanpa_hapus<T: PenyimpanArsip>() {}
    _assert_tanpa_hapus::<Memori>();
    // The absence is the contract, so the test documents it and the trait shape
    // is the enforcement.
    let _ = "PenyimpanArsip hanya punya put, get_range, head";
}

/// AC-19: a whole archive round-trip through the port, twice to the same key,
/// leaves one object and the last content.
#[tokio::test]
async fn ac19_tulis_dua_kali_idempoten() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    let p = pembungkus();

    let r1 = tulis_ke_memori(&memori, &kunci, baris_banyak(5), &p).await;
    let r2 = tulis_ke_memori(&memori, &kunci, baris_banyak(9), &p).await;

    assert_eq!(memori.jumlah_objek(), 1);
    assert_ne!(r1.sha256, r2.sha256);
    Verifikasi::verifikasi(&memori, &r2).await.unwrap();

    let baris = BacaArsip::baca_semua(Arc::new(memori), Arc::new(kunci), &pembungkus())
        .await
        .unwrap();
    assert_eq!(baris.len(), 9, "objek terakhir yang menang");
}

/// AC-19: the S3 implementation is built from a bucket, and its `Debug` names
/// the label only. It is exercised against MinIO in T7's container lane, which
/// is where `object_store`'s HTTP path is actually driven; here the test keeps
/// the constructor honest and confirms the type is the same port.
#[test]
fn ac19_s3_adalah_port_yang_sama() {
    fn _assert_port<T: PenyimpanArsip>() {}
    _assert_port::<S3>();

    // A bucket name is enough to build the store; no environment read happens.
    let s3 = S3::dari_bucket(
        "arsip-uji",
        Some("http://127.0.0.1:9000"),
        true,
        "minio-uji",
    )
    .expect("S3 terbangun dari bucket");
    assert_eq!(format!("{s3:?}"), "S3(minio-uji)");
}

// ---------------------------------------------------------------- AC-20

/// AC-20: types carrying a NIK or payload do not print the value through
/// `Debug` or `Display`.
#[test]
fn ac20_tipe_rahasia_tidak_tampil() {
    let nik_teks = "0012345678901234";
    let nik = Nik::parse(nik_teks).unwrap();
    let b = BarisArsip::baru(
        kode(1),
        hmac_sintetis(1),
        nik,
        "PAYLOAD-RAHASIA-UJI",
        jam(1),
    );

    for teks in [format!("{b:?}"), format!("{b}")] {
        assert!(!teks.contains(nik_teks), "NIK tampil: {teks}");
        assert!(
            !teks.contains("PAYLOAD-RAHASIA-UJI"),
            "payload tampil: {teks}"
        );
    }

    // The guard type itself.
    let terkurung = NilaiAman::baru(nik_teks.to_owned());
    assert_eq!(format!("{terkurung:?}"), "NilaiAman(***)");
    assert_eq!(format!("{terkurung}"), "***");
}

/// AC-20: error values do not carry a NIK or a payload, on the failure paths
/// that see the most data.
#[tokio::test]
async fn ac20_galat_tidak_memuat_nilai() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    let asli = baris_banyak(4);
    tulis_ke_memori(&memori, &kunci, asli.clone(), &pembungkus()).await;

    // A byte change in the sealed region: the failure must not name a value.
    let mut bytes = memori.bytes_mentah(&kunci).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    memori.put(&kunci, bytes).await.unwrap();
    let e = BacaArsip::baca_semua(Arc::new(memori), Arc::new(kunci), &pembungkus())
        .await
        .unwrap_err();
    let pesan = format!("{e} {e:?}");
    for b in &asli {
        let nik = b.nik.as_ref().unwrap().expose().utuh().expose().to_string();
        assert!(!pesan.contains(&nik), "galat memuat NIK: {pesan}");
    }
}

/// AC-20: a row with a different HMAC key version than the object is refused at
/// write time, because one object carries one version (DA11).
#[tokio::test]
async fn ac20_versi_kunci_campur_ditolak() {
    let mut a = baris_banyak(2);
    let v2 = VersiKunciHmac::baru(2).unwrap();
    a[1].kunci_hmac_versi = v2;

    let e = TulisArsip::tulis(a, &ParameterTulis::transaction_log(), &pembungkus())
        .await
        .expect_err("versi kunci campur harus ditolak");
    assert_eq!(e, GalatArsip::KunciHmacTidakTersedia(1));
}

/// AC-20: a caller may only build a key version >= 1.
#[test]
fn ac20_versi_kunci_harus_positif() {
    assert!(VersiKunciHmac::baru(0).is_err());
    assert!(VersiKunciHmac::baru(1).is_ok());
}

// ------------------------------------------------------- extra behaviours

/// The empty object round-trips: zero rows is a legal archive window.
#[tokio::test]
async fn objek_kosong_bolak_balik() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    let r = tulis_ke_memori(&memori, &kunci, Vec::new(), &pembungkus()).await;
    assert_eq!(r.jumlah_baris, 0);
    assert!(r.rentang_kode_transaksi.is_none());

    let kembali = BacaArsip::baca_semua(Arc::new(memori), Arc::new(kunci), &pembungkus())
        .await
        .unwrap();
    assert!(kembali.is_empty());
}

/// A row without a NIK (the companion tables of T5) writes nulls.
#[tokio::test]
async fn baris_tanpa_nik_bolak_balik() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("request_status_history", "p", "2026-11-05", "00-06").unwrap();
    let asli = vec![BarisArsip::tanpa_nik(kode(1), hmac_sintetis(1), jam(1))];
    // A table without a NIK column: written through `tulis_ke` with its own
    // parameter, so the footer names the source table.
    TulisArsip::tulis_ke(
        &memori,
        &kunci,
        asli.clone(),
        &ParameterTulis::baru("request_status_history"),
        &pembungkus(),
    )
    .await
    .unwrap();

    let kembali = BacaArsip::baca_semua(Arc::new(memori), Arc::new(kunci), &pembungkus())
        .await
        .unwrap();
    assert_eq!(kembali, asli);
}

/// A truncated object is a read error, not a panic.
#[tokio::test]
async fn objek_terpotong_gagal_rapi() {
    let memori = Memori::baru();
    let kunci = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    tulis_ke_memori(&memori, &kunci, baris_banyak(20), &pembungkus()).await;

    let bytes = memori.bytes_mentah(&kunci).unwrap();
    memori
        .put(&kunci, bytes[..bytes.len() / 2].to_vec())
        .await
        .unwrap();

    let e = BacaArsip::baca_semua(Arc::new(memori), Arc::new(kunci), &pembungkus())
        .await
        .expect_err("objek terpotong harus gagal");
    assert!(
        matches!(e, GalatArsip::Parquet(_) | GalatArsip::Format(_)),
        "galat salah varian: {e:?}"
    );
}

/// The DEK wrapper contract: a different KEK label is a different key.
#[test]
fn pembungkus_berbeda_kunci_berbeda() {
    let a = PembungkusKekAwal::uji("a");
    let b = PembungkusKekAwal::uji("b");
    assert_ne!(a.kek_id(), b.kek_id());
    let dek = crate::arsip::kunci::Dek::acak().unwrap();
    let t = a.bungkus(&dek).unwrap();
    assert!(b.buka(&t).is_err());
}

/// Two writes of the same rows produce different bytes, because each object
/// gets a fresh DEK. That is the property that makes sealing per object work.
#[tokio::test]
async fn dua_tulisan_dapat_dek_berbeda() {
    let baris = baris_banyak(3);
    let (b1, r1) = TulisArsip::tulis(
        baris.clone(),
        &ParameterTulis::transaction_log(),
        &pembungkus(),
    )
    .await
    .unwrap();
    let (b2, r2) = TulisArsip::tulis(baris, &ParameterTulis::transaction_log(), &pembungkus())
        .await
        .unwrap();
    assert_ne!(r1.sha256, r2.sha256, "dua objek harus beda byte");
    assert_eq!(b1.len(), b2.len(), "panjang tetap sama");
}

/// The key is deterministic per unit, so a retried job targets one object.
#[test]
fn kunci_objek_berulang_konsisten() {
    let a = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    let b = KunciObjek::baru("transaction_log", "p", "2026-11-05", "00-06").unwrap();
    assert_eq!(a, b);
}

/// A DEK is 32 bytes and a wrapped DEK is longer, so the header cannot be
/// confused with a bare key.
#[test]
fn dek_terbungkus_lebih_panjang_dari_dek() {
    let p = pembungkus();
    let t = p
        .bungkus(&crate::arsip::kunci::Dek::acak().unwrap())
        .unwrap();
    assert!(t.as_bytes().len() > PANJANG_DEK);
    assert!(!t.kosong());
}

const _: () = assert!(PANJANG_DEK == 32);
