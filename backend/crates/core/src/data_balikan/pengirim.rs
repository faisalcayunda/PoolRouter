//! Port `PengirimDataBalikan` dan adapter tiruannya (AC-18, ADR-0006 §6 opsi B).
//!
//! Port ini sengaja sempit: dua operasi, `kirim` dan `periksa_ack`. Format
//! kabel, transport, dan autentikasi masih terbuka (D-08), jadi port hanya
//! menyepakati hal yang tidak bergantung pada itu — batch id sebagai kunci
//! idempotensi, item per provider, dan hasil yang bisa diprogram.
//!
//! Body dikirim sebagai stream `Bytes` utuh (bukan path dan bukan
//! `Vec<u8>`): penyusun tidak perlu menahan seluruh payload di memori, dan
//! adapter bisa meneruskan begitu byte pertama tersedia.

use std::collections::VecDeque;
use std::sync::Mutex;

use bytes::Bytes;
use futures::Stream;
use uuid::Uuid;

/// Satu batch siap kirim.
///
/// `batch_id` adalah kunci idempotensi: provider harus memperlakukan dua
/// pengiriman dengan `batch_id` sama sebagai satu batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchKeluar {
    pub batch_id: Uuid,
    pub provider_id: Uuid,
    /// Nilai kolom `data_balikan_batch.pemicu` (`jumlah_n`/`periode`).
    pub pemicu: String,
    pub jumlah_item: u32,
    pub format_version: String,
}

/// Kelas kegagalan batch. Dipisah dari retry webhook karena penyebabnya
/// berbeda: 5xx dan timeout dari provider, atau batch ditolak permanen
/// (misalnya `format_version` tidak dikenal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KelasGagal {
    /// Bisa dicoba lagi dengan jeda [`super::jeda_batch`].
    Sementara,
    /// Tidak dicoba lagi; batch berstatus `gagal_permanen`.
    Permanen,
}

/// Hasil satu pengiriman.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HasilKirimBatch {
    /// Provider menerima dan mengembalikan referensi ack pada panggilan yang
    /// sama.
    Diterima { ack_ref: String },
    /// Provider menerima tanpa ack seketika; ack diperiksa lewat
    /// [`PengirimDataBalikan::periksa_ack`] dengan `r#ref` ini.
    ///
    /// Nama field mengikuti spec (`Dikirim { ref }`); `ref` adalah keyword
    /// Rust, jadi ditulis sebagai raw identifier.
    Dikirim { r#ref: String },
    /// Gagal, bisa dicoba lagi.
    GagalSementara { kelas: KelasGagal },
    /// Ditolak permanen.
    Ditolak { kelas: KelasGagal },
}

/// Hasil pemeriksaan ack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HasilAck {
    Diterima { ack_ref: String },
    Ditolak { alasan: String },
    BelumAda,
}

/// Kontrak pengiriman data balikan ke provider.
///
/// Satu-satunya hal yang harus dipenuhi adapter asli (T8) dan adapter tiruan
/// adalah suite kontrak yang sama: `kirim` harus idempoten terhadap
/// `batch_id`, dan `periksa_ack` harus menjawab `BelumAda` (bukan galat)
/// selama provider belum meng-ack.
pub trait PengirimDataBalikan: Send + Sync {
    fn kirim(
        self: std::sync::Arc<Self>,
        batch: BatchKeluar,
        body: Box<dyn Stream<Item = Bytes> + Send + Unpin>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HasilKirimBatch> + Send>>;

    fn periksa_ack(
        self: std::sync::Arc<Self>,
        batch_id: Uuid,
        r#ref: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HasilAck> + Send>>;
}

/// Satu pengiriman yang tercatat adapter tiruan. Tidak membawa body, hanya
/// hash-nya: isi body berisi NIK dan tidak boleh tersimpan di memori pengirim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RekamanKirim {
    pub batch_id: Uuid,
    pub jumlah_item: u32,
    /// SHA-256 body (hex), supaya uji bisa membandingkan isi tanpa
    /// menyimpannya.
    pub hash_body: String,
}

/// Adapter tiruan: merekam setiap `batch_id`, jumlah item, dan hash body, dan
/// bisa diprogram per percobaan (AC-18).
///
/// Program dibaca berurutan: percobaan pertama memakai `antrian[0]`, dan
/// seterusnya. Bila antrian habis, `hasil_bawaan` yang dipakai. Program
/// `PanikSetelahKirim` mensimulasikan adapter yang menyerahkan body ke
/// jaringan lalu mati sebelum menulis hasil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HasilDiprogam {
    Diterima,
    Dikirim,
    GagalSementara,
    Ditolak,
    /// Panik setelah body terkirim: uji memastikan pemanggil tidak
    /// mengandalkan hasil dan batch tetap bisa disusun ulang dari database.
    PanikSetelahKirim,
}

/// Adapter tiruan pengirim data balikan.
#[derive(Debug)]
pub struct PengirimTiruan {
    antrian: Mutex<VecDeque<HasilDiprogam>>,
    hasil_bawaan: HasilDiprogam,
    rekaman: Mutex<Vec<RekamanKirim>>,
    ack: Mutex<Vec<(Uuid, String, HasilAck)>>,
}

impl Default for PengirimTiruan {
    fn default() -> Self {
        Self::baru()
    }
}

impl PengirimTiruan {
    /// Adapter yang selalu berhasil menerima.
    pub fn baru() -> Self {
        Self {
            antrian: Mutex::new(VecDeque::new()),
            hasil_bawaan: HasilDiprogam::Diterima,
            rekaman: Mutex::new(Vec::new()),
            ack: Mutex::new(Vec::new()),
        }
    }

    /// Adapter yang hasilnya bisa diprogram per percobaan.
    pub fn dengan_program(program: Vec<HasilDiprogam>) -> Self {
        let p = Self::baru();
        *p.antrian.lock().unwrap() = program.into();
        p
    }

    /// Hasil yang dipakai saat antrian program habis.
    pub fn dengan_bawaan(mut self, hasil: HasilDiprogam) -> Self {
        self.hasil_bawaan = hasil;
        self
    }

    /// Menambahkan satu jawaban ack untuk `(batch_id, referensi)`.
    pub fn program_ack(&self, batch_id: Uuid, r#ref: &str, hasil: HasilAck) {
        self.ack
            .lock()
            .unwrap()
            .push((batch_id, r#ref.to_owned(), hasil));
    }

    /// Semua pengiriman yang tercatat, berurutan.
    pub fn rekaman(&self) -> Vec<RekamanKirim> {
        self.rekaman.lock().unwrap().clone()
    }

    /// Berapa kali `batch_id` dikirim.
    pub fn jumlah_kirim(&self, batch_id: Uuid) -> usize {
        self.rekaman
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.batch_id == batch_id)
            .count()
    }

    fn ambil_hasil(&self) -> HasilDiprogam {
        let mut q = self.antrian.lock().unwrap();
        q.pop_front().unwrap_or(self.hasil_bawaan)
    }

    /// Merekam pengiriman. Dipisah supaya uji bisa memakainya tanpa `kirim`.
    pub fn catat(&self, batch_id: Uuid, jumlah_item: u32, body: &[u8]) {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(body);
        self.rekaman.lock().unwrap().push(RekamanKirim {
            batch_id,
            jumlah_item,
            hash_body: format!("{hash:x}"),
        });
    }
}

impl PengirimDataBalikan for PengirimTiruan {
    fn kirim(
        self: std::sync::Arc<Self>,
        batch: BatchKeluar,
        body: Box<dyn Stream<Item = Bytes> + Send + Unpin>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HasilKirimBatch> + Send>> {
        use futures::StreamExt;
        Box::pin(async move {
            let mut buf = Vec::new();
            let mut body = body;
            while let Some(chunk) = body.next().await {
                buf.extend_from_slice(&chunk);
            }
            self.catat(batch.batch_id, batch.jumlah_item, &buf);
            match self.ambil_hasil() {
                HasilDiprogam::Diterima => HasilKirimBatch::Diterima {
                    ack_ref: format!("ack-{}", batch.batch_id),
                },
                HasilDiprogam::Dikirim => HasilKirimBatch::Dikirim {
                    r#ref: format!("ref-{}", batch.batch_id),
                },
                HasilDiprogam::GagalSementara => HasilKirimBatch::GagalSementara {
                    kelas: KelasGagal::Sementara,
                },
                HasilDiprogam::Ditolak => HasilKirimBatch::Ditolak {
                    kelas: KelasGagal::Permanen,
                },
                HasilDiprogam::PanikSetelahKirim => {
                    panic!(
                        "pengirim tiruan panik setelah kirim (batch {})",
                        batch.batch_id
                    )
                }
            }
        })
    }

    fn periksa_ack(
        self: std::sync::Arc<Self>,
        batch_id: Uuid,
        r#ref: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HasilAck> + Send>> {
        let r_owned = r#ref.to_owned();
        let ack_list = self.ack.lock().unwrap().clone();
        Box::pin(async move {
            ack_list
                .iter()
                .find(|(b, r, _)| *b == batch_id && r == &r_owned)
                .map(|(_, _, h)| h.clone())
                .unwrap_or(HasilAck::BelumAda)
        })
    }
}

#[cfg(test)]
mod uji {
    use super::*;
    use futures::stream;

    fn batch() -> BatchKeluar {
        BatchKeluar {
            batch_id: Uuid::nil(),
            provider_id: Uuid::nil(),
            pemicu: "jumlah_n".to_owned(),
            jumlah_item: 3,
            format_version: "tiruan-1".to_owned(),
        }
    }

    fn body() -> impl Stream<Item = Bytes> + Send {
        stream::iter(vec![Bytes::from_static(b"a"), Bytes::from_static(b"bc")])
    }

    #[tokio::test]
    async fn hasil_sukses_tercatat_dengan_hash_body() {
        let p = PengirimTiruan::baru();
        let hasil = p.kirim(batch(), body()).await;
        assert!(matches!(hasil, HasilKirimBatch::Diterima { .. }));
        let r = p.rekaman();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].jumlah_item, 3);
        // SHA-256("abc")
        assert_eq!(
            r[0].hash_body,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn body_utuh_diterima_atau_hash_cocok() {
        // Body dipecah di tengah, hash harus sama dengan body utuh.
        let p = PengirimTiruan::baru();
        let _ = p.kirim(batch(), body()).await;
        let p2 = PengirimTiruan::baru();
        let _ = p2
            .kirim(batch(), stream::iter(vec![Bytes::from_static(b"abc")]))
            .await;
        assert_eq!(p.rekaman()[0].hash_body, p2.rekaman()[0].hash_body);
    }

    #[tokio::test]
    async fn kelas_gagal_bisa_dipilih() {
        let p = PengirimTiruan::dengan_program(vec![
            HasilDiprogam::GagalSementara,
            HasilDiprogam::Ditolak,
            HasilDiprogam::Dikirim,
            HasilDiprogam::Diterima,
        ]);
        assert!(matches!(
            p.kirim(batch(), body()).await,
            HasilKirimBatch::GagalSementara {
                kelas: KelasGagal::Sementara
            }
        ));
        assert!(matches!(
            p.kirim(batch(), body()).await,
            HasilKirimBatch::Ditolak {
                kelas: KelasGagal::Permanen
            }
        ));
        assert!(matches!(
            p.kirim(batch(), body()).await,
            HasilKirimBatch::Dikirim { .. }
        ));
        assert!(matches!(
            p.kirim(batch(), body()).await,
            HasilKirimBatch::Diterima { .. }
        ));
    }

    #[tokio::test]
    async fn percobaan_berikutnya_memakai_bawaan_saat_program_habis() {
        let p = PengirimTiruan::dengan_program(vec![HasilDiprogam::GagalSementara])
            .dengan_bawaan(HasilDiprogam::Ditolak);
        assert!(matches!(
            p.kirim(batch(), body()).await,
            HasilKirimBatch::GagalSementara { .. }
        ));
        assert!(matches!(
            p.kirim(batch(), body()).await,
            HasilKirimBatch::Ditolak { .. }
        ));
    }

    #[tokio::test]
    async fn panik_setelah_kirim_tetap_tercatat() {
        let p = PengirimTiruan::dengan_program(vec![HasilDiprogam::PanikSetelahKirim]);
        let hasil = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
            p.kirim(batch(), body()),
        ))
        .await;
        assert!(hasil.is_err());
        assert_eq!(p.jumlah_kirim(Uuid::nil()), 1);
    }

    #[tokio::test]
    async fn kirim_ulang_batch_id_sama_tercatat_dua_kali() {
        let p = PengirimTiruan::baru();
        let _ = p.kirim(batch(), body()).await;
        let _ = p.kirim(batch(), body()).await;
        assert_eq!(p.jumlah_kirim(Uuid::nil()), 2);
        // Idempotensi adalah kewajiban penerima; port hanya menjamin
        // `batch_id` yang sama terkirim, bukan diterima sekali.
        assert_eq!(p.rekaman()[0].batch_id, p.rekaman()[1].batch_id);
    }

    #[tokio::test]
    async fn ack_bisa_diprogam_dan_bawaan_belum_ada() {
        let p = PengirimTiruan::baru();
        assert_eq!(p.periksa_ack(Uuid::nil(), "ref").await, HasilAck::BelumAda);
        p.program_ack(
            Uuid::nil(),
            "ref",
            HasilAck::Diterima {
                ack_ref: "ack-1".to_owned(),
            },
        );
        assert_eq!(
            p.periksa_ack(Uuid::nil(), "ref").await,
            HasilAck::Diterima {
                ack_ref: "ack-1".to_owned()
            }
        );
        // Referensi lain tetap belum ada.
        assert_eq!(p.periksa_ack(Uuid::nil(), "lain").await, HasilAck::BelumAda);
    }
}
