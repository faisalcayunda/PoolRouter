//! Adapter antrean RabbitMQ (ADR-0001 bagian 6, ADR-0004 bagian 4.3, ARC-68).
//!
//! Publish dan deklarasi topologi lewat `lapin` langsung (W1); konsumsi lewat
//! apalis 1.0 + `apalis-amqp` `=1.0.0` dengan workaround W2–W4. Tipe apalis
//! dan lapin tidak keluar dari crate ini: dispatcher dan worker hanya melihat
//! port [`JobPublisher`] dan [`JobConsumer`].
//!
//! Arti hasil handler bagi broker:
//! - `Ok` → ack, setelah handler selesai (handler worker mengembalikan `Ok`
//!   hanya sesudah commit PostgreSQL).
//! - [`QueueError::Handler`] → reject tanpa requeue → DLX/DLQ. Handler worker
//!   tidak membedakan galat sementara dan permanen; request tetap
//!   `dispatched` di PostgreSQL dan reconciler mengembalikannya ke `queued`
//!   (ADR-0001 bagian 5 lapisan 2), jadi tidak ada requeue buta.
//! - [`QueueError::Transport`] → nack dengan requeue (W4).
//! - Proses mati sebelum ack → broker mengirim ulang pesan.

mod kodek;
pub mod topologi;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use apalis::prelude::{AbortError, BoxDynError, WorkerBuilder, WorkerContext};
use apalis_amqp::{AmqpBackend, AmqpTask, Config as KonfigurasiAmqp};
use apalis_core::backend::ext::with_codec::WithCodec;
use futures::Sink;
use lapin::{
    BasicProperties, Channel, Confirmation, Connection, ConnectionProperties,
    options::{BasicNackOptions, BasicPublishOptions, ConfirmSelectOptions},
};
use poolrouter_core::queue::{
    JobConsumer, JobPublisher, PenandaKonsumen, QueueError, StatusKonsumen,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::kodek::KodekRequestId;

/// Prefetch bawaan per consumer. ADR-0001 bagian 2 hanya menyebut
/// "concurrency dan prefetch per pool" tanpa angka; nilai kecil membatasi
/// pesan tak-ack yang ikut dikirim ulang saat worker mati (S6).
pub const PREFETCH_BAWAAN: u16 = 4;

/// Jeda awal dan maksimum loop sambung ulang (W3).
const JEDA_AWAL: Duration = Duration::from_secs(1);
pub const JEDA_MAKS: Duration = Duration::from_secs(30);

/// Toleransi liveness worker (W3): berapa lama satu loop consumer boleh
/// tidak tersambung sebelum `/healthz` gagal. Tiga kali [`JEDA_MAKS`]:
/// setelah backoff mencapai batasnya, setiap percobaan memakan jeda 30 detik
/// ditambah waktu connect dan deklarasi (beberapa detik), jadi dalam 90 detik
/// selalu ada minimal dua percobaan sambung ulang penuh. Restart broker biasa
/// atau putus jaringan sesaat pulih di dalam jendela ini tanpa membunuh
/// proses; loop yang gagal terus (406, izin ditolak) tetap ketahuan.
pub const TOLERANSI_LIVENESS: Duration = Duration::from_secs(JEDA_MAKS.as_secs() * 3);

/// Sesi dicatat tersambung setelah worker apalis bertahan selama ini.
/// `basic_qos` dan `basic_consume` terjadi pada poll pertama worker
/// (milidetik); bila ditolak broker (403, 406), `Worker::run` langsung
/// kembali `Err`. Dengan menunggu dua detik, sesi yang gagal di titik itu
/// tidak pernah tercatat tersambung, sehingga tidak mereset toleransi.
const JEDA_STABIL: Duration = Duration::from_secs(2);

fn transport(galat: impl std::fmt::Display) -> QueueError {
    QueueError::Transport(galat.to_string())
}

pub struct ApalisQueue {
    channel: Channel,
    uri: String,
    berhenti: CancellationToken,
    prefetch: u16,
    status: Option<Arc<StatusKonsumen>>,
}

impl ApalisQueue {
    /// Buka koneksi dan channel publish (`confirm_select`). Consumer membuka
    /// koneksinya sendiri per sesi, jadi `uri` disimpan.
    pub async fn connect(uri: &str) -> Result<Self, QueueError> {
        let connection = Connection::connect(uri, ConnectionProperties::default())
            .await
            .map_err(transport)?;
        let channel = connection.create_channel().await.map_err(transport)?;
        channel
            .confirm_select(ConfirmSelectOptions::default())
            .await
            .map_err(transport)?;
        Ok(Self {
            channel,
            uri: uri.to_owned(),
            berhenti: CancellationToken::new(),
            prefetch: PREFETCH_BAWAAN,
            status: None,
        })
    }

    /// Token graceful shutdown untuk [`JobConsumer::run`]: saat dibatalkan,
    /// consumer berhenti mengambil pesan, menunggu handler yang sedang
    /// berjalan, lalu `run` kembali `Ok`.
    pub fn dengan_penghenti(mut self, berhenti: CancellationToken) -> Self {
        self.berhenti = berhenti;
        self
    }

    /// Prefetch consumer (minimal 1).
    pub fn dengan_prefetch(mut self, prefetch: u16) -> Self {
        self.prefetch = prefetch.max(1);
        self
    }

    /// Laporkan status sambungan setiap loop [`JobConsumer::run`] ke
    /// `status` (probe liveness worker, W3).
    pub fn dengan_status(mut self, status: Arc<StatusKonsumen>) -> Self {
        self.status = Some(status);
        self
    }

    /// Deklarasikan topologi untuk `queue` (idempoten, AC-3). Lihat
    /// [`topologi`].
    pub async fn declare(&self, queue: &str) -> Result<(), QueueError> {
        topologi::deklarasi(&self.channel, queue)
            .await
            .map_err(transport)
    }

    /// Satu sesi consumer: koneksi baru, topologi, backend apalis, lalu worker
    /// sampai shutdown atau koneksi putus.
    async fn satu_sesi<H, F>(
        &self,
        queue: &str,
        handler: Arc<H>,
        penanda: Option<Arc<PenandaKonsumen>>,
    ) -> Result<(), QueueError>
    where
        H: Fn(Uuid) -> F + Send + Sync + 'static,
        F: Future<Output = Result<(), QueueError>> + Send + 'static,
    {
        // W3: topologi dibangun ulang di setiap sesi, lewat koneksi sendiri,
        // karena koneksi lama mungkin yang baru saja putus.
        let koneksi = Connection::connect(&self.uri, ConnectionProperties::default())
            .await
            .map_err(transport)?;
        let kanal = koneksi.create_channel().await.map_err(transport)?;
        topologi::deklarasi(&kanal, queue)
            .await
            .map_err(transport)?;
        if let Err(galat) = koneksi.close(200, "OK".into()).await {
            tracing::debug!(error = %galat, "koneksi deklarasi tidak tertutup rapi");
        }

        // Future worker apalis tidak bisa dibuktikan `Send` oleh rustc
        // (batasan lifetime higher-ranked pada `Into<BoxDynError>`), padahal
        // port mewajibkan `run` `Send`. Sesi apalis karena itu dijalankan di
        // thread blocking runtime yang sama lewat `Handle::block_on`, sehingga
        // future-nya tidak perlu `Send`. Butuh runtime multi-thread (bawaan
        // `#[tokio::main]`); satu thread blocking per antrean yang dikonsumsi.
        // Panic di handler mengakhiri sesi ini saja (koneksi ditutup, pesan
        // tak-ack kembali ke broker), lalu loop W3 menyambung ulang.
        let runtime = tokio::runtime::Handle::current();
        let uri = self.uri.clone();
        let nama = queue.to_owned();
        let prefetch = self.prefetch;
        let berhenti = self.berhenti.clone();
        tokio::task::spawn_blocking(move || {
            runtime.block_on(sesi_apalis(uri, nama, prefetch, berhenti, handler, penanda))
        })
        .await
        .map_err(|galat| {
            QueueError::Transport(format!("sesi consumer berakhir tidak normal: {galat}"))
        })?
    }
}

/// Backend apalis-amqp + worker untuk satu antrean, sampai shutdown atau
/// koneksi putus.
async fn sesi_apalis<H, F>(
    uri: String,
    queue: String,
    prefetch: u16,
    berhenti: CancellationToken,
    handler: Arc<H>,
    penanda: Option<Arc<PenandaKonsumen>>,
) -> Result<(), QueueError>
where
    H: Fn(Uuid) -> F + Send + Sync + 'static,
    F: Future<Output = Result<(), QueueError>> + Send + 'static,
{
    let mut backend = AmqpBackend::<Uuid>::new_from_addr(&uri)
        .map_err(transport)?
        .with_config(konfigurasi_apalis(&queue, prefetch));
    // W2: `Sink::poll_ready` mengisi channel backend sebelum worker
    // dibangun. Tanpa ini `poll_next` panic, dan lapisan ack yang
    // mengkloning backend saat worker mulai tidak punya channel.
    futures::future::poll_fn(|cx| {
        <AmqpBackend<Uuid> as Sink<AmqpTask>>::poll_ready(Pin::new(&mut backend), cx)
    })
    .await
    .map_err(transport)?;
    let backend = WithCodec::new(backend, KodekRequestId);

    // Konteks dipegang sendiri (klon berbagi state) supaya shutdown bisa
    // dipicu dari luar. `run_until` tidak dipakai: bila broker menutup
    // channel atau mengirim `basic.cancel`, stream consumer lapin berakhir
    // bersih (`None`), worker selesai `Ok`, dan `run_until` tetap menunggu
    // sinyal shutdown selamanya tanpa menyambung ulang.
    let konteks = WorkerContext::from(format!("poolrouter-{queue}"));
    let worker = WorkerBuilder::new(&konteks)
        .backend(backend)
        .build(move |request_id: Uuid| {
            let handler = Arc::clone(&handler);
            async move { handler(request_id).await.map_err(galat_apalis) }
        });
    let jalan = worker.run();
    tokio::pin!(jalan);
    let stabil = tokio::time::sleep(JEDA_STABIL);
    tokio::pin!(stabil);
    let mut sudah_stabil = false;
    loop {
        tokio::select! {
            biased;
            // Stream berakhir atau galat: kembali ke loop W3.
            hasil = &mut jalan => return hasil.map_err(transport),
            _ = berhenti.cancelled() => break,
            _ = &mut stabil, if !sudah_stabil => {
                sudah_stabil = true;
                if let Some(penanda) = &penanda {
                    penanda.tersambung();
                }
            }
        }
    }
    // Shutdown: berhenti mengambil pesan, tunggu handler yang berjalan.
    if let Err(galat) = konteks.stop() {
        tracing::debug!(error = %galat, "worker apalis sudah berhenti");
    }
    jalan.await.map_err(transport)
}

/// Konfigurasi `apalis-amqp`: argumen antrean identik dengan deklarasi lapin
/// (W4), nack dengan requeue untuk galat transport, tanpa `max_retries` dan
/// tanpa penjadwalan (tidak dibaca/tidak dipakai di 1.0.0).
fn konfigurasi_apalis(queue: &str, prefetch: u16) -> KonfigurasiAmqp {
    KonfigurasiAmqp::default()
        .queue(queue.to_owned())
        .prefetch_count(prefetch)
        .declare_options(topologi::opsi_antrean())
        .declare_arguments(topologi::argumen_antrean(queue))
        .nack_options(BasicNackOptions {
            multiple: false,
            requeue: true,
        })
        .scheduling(false)
}

/// Apa yang dilakukan broker atas `Err` dari handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TindakanGalat {
    /// Nack dengan requeue (W4: galat transport sebelum commit).
    Requeue,
    /// Reject tanpa requeue → DLX/DLQ.
    DeadLetter,
}

fn tindakan_galat(galat: &QueueError) -> TindakanGalat {
    match galat {
        QueueError::Transport(_) => TindakanGalat::Requeue,
        QueueError::Handler(_) => TindakanGalat::DeadLetter,
    }
}

/// `AbortError` membuat status task `Killed`; `apalis-amqp` 1.0.0 lalu
/// me-reject dengan `requeue = false`. Galat biasa berstatus `Failed` dan
/// di-nack dengan `nack_options`.
fn galat_apalis(galat: QueueError) -> BoxDynError {
    match tindakan_galat(&galat) {
        TindakanGalat::DeadLetter => Box::new(AbortError::new(galat)),
        TindakanGalat::Requeue => Box::new(galat),
    }
}

impl JobPublisher for ApalisQueue {
    /// W1: sukses hanya bila broker meng-ack tanpa mengembalikan pesan.
    /// Pesan tak terarah (`Ack` dengan pesan kembali), `Nack`, dan
    /// `NotRequested` adalah galat.
    async fn publish(&self, queue: &str, request_id: Uuid) -> Result<(), QueueError> {
        let confirm = self
            .channel
            .basic_publish(
                topologi::EXCHANGE_JOB.into(),
                queue.into(),
                BasicPublishOptions {
                    mandatory: true,
                    ..Default::default()
                },
                &kodek::body(request_id),
                BasicProperties::default()
                    .with_delivery_mode(2)
                    .with_message_id(request_id.to_string().into()),
            )
            .await
            .map_err(transport)?;
        match confirm.await.map_err(transport)? {
            Confirmation::Ack(None) => Ok(()),
            Confirmation::Ack(Some(_)) => Err(QueueError::Transport(format!(
                "pesan tidak terarah ke antrean {queue}"
            ))),
            Confirmation::Nack(_) => Err(QueueError::Transport(format!(
                "broker menolak publish ke antrean {queue}"
            ))),
            Confirmation::NotRequested => Err(QueueError::Transport(
                "publisher confirm tidak aktif".into(),
            )),
        }
    }
}

impl JobConsumer for ApalisQueue {
    /// Konsumsi `queue` sampai token shutdown dibatalkan. Koneksi putus atau
    /// worker apalis berhenti → sambung ulang dengan backoff (W3); pesan yang
    /// belum di-ack dikirim ulang broker.
    async fn run<H, F>(&self, queue: &str, handler: H) -> Result<(), QueueError>
    where
        H: Fn(Uuid) -> F + Send + Sync + 'static,
        F: Future<Output = Result<(), QueueError>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        // Didaftarkan per loop (satu `ApalisQueue` dipakai beberapa antrean);
        // di-drop saat `run` kembali sehingga shutdown tidak menggagalkan
        // liveness.
        let penanda = self.status.as_ref().map(|status| Arc::new(status.daftar()));
        let mut jeda = JEDA_AWAL;
        loop {
            if self.berhenti.is_cancelled() {
                return Ok(());
            }
            let mulai = Instant::now();
            let hasil = self
                .satu_sesi(queue, Arc::clone(&handler), penanda.clone())
                .await;
            if let Some(penanda) = &penanda {
                penanda.terputus();
            }
            match hasil {
                Ok(()) if self.berhenti.is_cancelled() => return Ok(()),
                Ok(()) => tracing::warn!(antrean = queue, "consumer berhenti; menyambung ulang"),
                Err(galat) => {
                    tracing::warn!(antrean = queue, error = %galat, "consumer terputus; menyambung ulang");
                }
            }
            if mulai.elapsed() > JEDA_MAKS {
                jeda = JEDA_AWAL;
            }
            tokio::select! {
                _ = self.berhenti.cancelled() => return Ok(()),
                _ = tokio::time::sleep(jeda) => {}
            }
            jeda = (jeda * 2).min(JEDA_MAKS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn galat_handler_ke_dlq_galat_transport_requeue() {
        assert_eq!(
            tindakan_galat(&QueueError::Handler("postgres".into())),
            TindakanGalat::DeadLetter
        );
        assert_eq!(
            tindakan_galat(&QueueError::Transport("putus".into())),
            TindakanGalat::Requeue
        );
        assert!(
            galat_apalis(QueueError::Handler("x".into()))
                .downcast_ref::<AbortError>()
                .is_some()
        );
        assert!(
            galat_apalis(QueueError::Transport("x".into()))
                .downcast_ref::<AbortError>()
                .is_none()
        );
    }

    /// Toleransi liveness harus memuat minimal dua percobaan sambung ulang
    /// penuh pada backoff maksimum ditambah jeda stabil satu sesi.
    #[test]
    fn toleransi_liveness_memuat_dua_percobaan_backoff_maksimum() {
        assert_eq!(TOLERANSI_LIVENESS, Duration::from_secs(90));
        assert!(TOLERANSI_LIVENESS >= JEDA_MAKS * 2 + JEDA_STABIL);
    }

    #[test]
    fn konfigurasi_apalis_sama_dengan_topologi_lapin() {
        let q = "pr.jobs.upstream.0190a000-0000-7000-8000-000000000001";
        let k = konfigurasi_apalis(q, 3);
        assert_eq!(k.queue.as_ref(), q);
        assert_eq!(k.prefetch_count, 3);
        assert_eq!(k.declare_arguments, topologi::argumen_antrean(q));
        assert!(k.declare_options.durable);
        assert!(k.nack_options.requeue);
        assert!(!k.scheduling);
    }
}
