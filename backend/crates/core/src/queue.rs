//! Transport-independent queue ports.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("queue transport: {0}")]
    Transport(String),
    #[error("handler: {0}")]
    Handler(String),
}

pub trait JobPublisher: Send + Sync {
    fn publish(
        &self,
        queue: &str,
        request_id: Uuid,
    ) -> impl Future<Output = Result<(), QueueError>> + Send;
}

/// Nama antrean job request untuk satu upstream pool (ADR-0001 §6, AC-3 spec
/// `f0-controller-worker`): satu queue durable per upstream pool, namanya
/// diturunkan dari `upstream_pool_id`. Dispatcher (penerbit) dan worker
/// (konsumen) memakai fungsi yang sama supaya nama tidak bisa menyimpang.
pub fn nama_antrean_upstream(upstream_pool_id: Uuid) -> String {
    format!("pr.jobs.upstream.{upstream_pool_id}")
}

/// Satu koneksi antrean bisa dibagi beberapa job controller (dispatcher,
/// penjadwal balikan) tanpa membuka koneksi broker kedua.
impl<T: JobPublisher> JobPublisher for std::sync::Arc<T> {
    fn publish(
        &self,
        queue: &str,
        request_id: Uuid,
    ) -> impl Future<Output = Result<(), QueueError>> + Send {
        (**self).publish(queue, request_id)
    }
}

pub trait JobConsumer: Send + Sync {
    fn run<H, F>(
        &self,
        queue: &str,
        handler: H,
    ) -> impl Future<Output = Result<(), QueueError>> + Send
    where
        H: Fn(Uuid) -> F + Send + Sync + 'static,
        F: Future<Output = Result<(), QueueError>> + Send + 'static;
}

/// Keadaan satu loop consumer.
#[derive(Debug, Clone, Copy)]
enum Keadaan {
    Tersambung,
    /// Belum (lagi) tersambung sejak instan ini.
    Terputus(Instant),
}

/// Status sambungan semua loop consumer worker, dibaca probe liveness
/// (`/healthz`, ADR-0004 bagian 4.3 W3: liveness gagal selama loop belum
/// tersambung).
///
/// Setiap loop consumer mendaftar lewat [`StatusKonsumen::daftar`] dan
/// melapor lewat [`PenandaKonsumen`]. Proses dianggap hidup bila setiap loop
/// yang terdaftar tersambung, atau terputus belum lebih lama dari
/// `toleransi`. Tanpa loop terdaftar (belum ada upstream pool) proses tetap
/// hidup: tidak ada yang bisa diperbaiki dengan restart.
#[derive(Debug)]
pub struct StatusKonsumen {
    toleransi: Duration,
    /// Indeks = slot satu loop; `None` = loop sudah selesai (shutdown).
    slot: Mutex<Vec<Option<Keadaan>>>,
}

impl StatusKonsumen {
    pub fn baru(toleransi: Duration) -> Arc<Self> {
        Arc::new(Self {
            toleransi,
            slot: Mutex::new(Vec::new()),
        })
    }

    /// Daftarkan satu loop consumer. Loop baru dihitung terputus sejak
    /// sekarang, jadi grace saat start sama dengan `toleransi`.
    pub fn daftar(self: &Arc<Self>) -> PenandaKonsumen {
        self.daftar_pada(Instant::now())
    }

    fn daftar_pada(self: &Arc<Self>, kini: Instant) -> PenandaKonsumen {
        let mut slot = self.kunci();
        slot.push(Some(Keadaan::Terputus(kini)));
        PenandaKonsumen {
            status: Arc::clone(self),
            indeks: slot.len() - 1,
        }
    }

    /// Hidup bila tidak ada loop yang terputus lebih lama dari `toleransi`.
    pub fn hidup(&self) -> bool {
        self.hidup_pada(Instant::now())
    }

    fn hidup_pada(&self, kini: Instant) -> bool {
        self.kunci().iter().flatten().all(|keadaan| match keadaan {
            Keadaan::Tersambung => true,
            Keadaan::Terputus(sejak) => kini.saturating_duration_since(*sejak) <= self.toleransi,
        })
    }

    /// Jumlah loop yang sedang tersambung.
    pub fn jumlah_tersambung(&self) -> usize {
        self.kunci()
            .iter()
            .flatten()
            .filter(|keadaan| matches!(keadaan, Keadaan::Tersambung))
            .count()
    }

    fn kunci(&self) -> std::sync::MutexGuard<'_, Vec<Option<Keadaan>>> {
        // Isi slot selalu konsisten (satu penugasan per operasi), jadi kunci
        // yang ter-poison tetap aman dipakai.
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn ubah(&self, indeks: usize, ubah: impl FnOnce(&mut Option<Keadaan>)) {
        if let Some(slot) = self.kunci().get_mut(indeks) {
            ubah(slot);
        }
    }
}

/// Pelapor status satu loop consumer. Saat di-drop (loop selesai karena
/// shutdown) slot-nya tidak lagi dihitung.
#[derive(Debug)]
pub struct PenandaKonsumen {
    status: Arc<StatusKonsumen>,
    indeks: usize,
}

impl PenandaKonsumen {
    /// Sesi consumer tersambung dan menerima pesan.
    pub fn tersambung(&self) {
        self.status
            .ubah(self.indeks, |slot| *slot = Some(Keadaan::Tersambung));
    }

    /// Sesi consumer berakhir.
    pub fn terputus(&self) {
        self.terputus_pada(Instant::now());
    }

    fn terputus_pada(&self, kini: Instant) {
        // Instan terputus yang lama dipertahankan: sesi yang gagal berulang
        // tanpa pernah tersambung tidak boleh mereset jeda toleransi.
        self.status.ubah(self.indeks, |slot| {
            if matches!(slot, Some(Keadaan::Tersambung)) {
                *slot = Some(Keadaan::Terputus(kini));
            }
        });
    }
}

impl Drop for PenandaKonsumen {
    fn drop(&mut self) {
        self.status.ubah(self.indeks, |slot| *slot = None);
    }
}

impl crate::runtime::Liveness for StatusKonsumen {
    fn alive(&self) -> bool {
        self.hidup()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOLERANSI: Duration = Duration::from_secs(90);

    #[test]
    fn tanpa_loop_terdaftar_tetap_hidup() {
        let status = StatusKonsumen::baru(TOLERANSI);
        assert!(status.hidup());
        assert_eq!(status.jumlah_tersambung(), 0);
    }

    #[test]
    fn grace_saat_start_lalu_gagal_bila_tak_pernah_tersambung() {
        let status = StatusKonsumen::baru(TOLERANSI);
        let t0 = Instant::now();
        let p = status.daftar_pada(t0);
        assert!(status.hidup_pada(t0 + TOLERANSI), "masih dalam grace start");
        // Sesi gagal berulang (mis. 406 PRECONDITION_FAILED) tidak mereset grace.
        p.terputus_pada(t0 + Duration::from_secs(30));
        p.terputus_pada(t0 + Duration::from_secs(60));
        assert!(!status.hidup_pada(t0 + TOLERANSI + Duration::from_secs(1)));
    }

    #[test]
    fn tersambung_hidup_terputus_diberi_toleransi_reconnect() {
        let status = StatusKonsumen::baru(TOLERANSI);
        let t0 = Instant::now();
        let p = status.daftar_pada(t0);
        p.tersambung();
        assert_eq!(status.jumlah_tersambung(), 1);
        assert!(status.hidup_pada(t0 + Duration::from_secs(3600)));

        let putus = t0 + Duration::from_secs(3600);
        p.terputus_pada(putus);
        assert_eq!(status.jumlah_tersambung(), 0);
        assert!(status.hidup_pada(putus + TOLERANSI), "reconnect normal");
        assert!(!status.hidup_pada(putus + TOLERANSI + Duration::from_secs(1)));

        // Tersambung lagi: hidup lagi, lalu putus berikutnya dihitung baru.
        p.tersambung();
        assert!(status.hidup_pada(putus + TOLERANSI * 2));
    }

    #[test]
    fn satu_loop_terputus_lama_menggagalkan_walau_loop_lain_tersambung() {
        let status = StatusKonsumen::baru(TOLERANSI);
        let t0 = Instant::now();
        let a = status.daftar_pada(t0);
        let _b = status.daftar_pada(t0);
        a.tersambung();
        assert!(!status.hidup_pada(t0 + TOLERANSI + Duration::from_secs(1)));
        assert_eq!(status.jumlah_tersambung(), 1);
    }

    #[test]
    fn loop_selesai_tidak_dihitung() {
        let status = StatusKonsumen::baru(TOLERANSI);
        let t0 = Instant::now();
        let p = status.daftar_pada(t0);
        drop(p);
        assert!(status.hidup_pada(t0 + TOLERANSI * 10));
    }
}
