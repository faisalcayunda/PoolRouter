//! Transport-independent queue ports.

use std::future::Future;
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
