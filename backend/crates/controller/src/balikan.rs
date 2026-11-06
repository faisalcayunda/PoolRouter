//! Controller job for dispatching one data-balikan job per eligible provider.

use chrono::{DateTime, Utc};
use poolrouter_core::queue::{JobPublisher, QueueError};
use sqlx::PgPool;
use uuid::Uuid;

pub const NAMA_JOB: &str = "pr_job_balikan";
pub const NAMA_QUEUE: &str = "pr.jobs.balikan";

/// Reads only provider configuration. The controller never reads
/// `data_balikan_item`, preserving the database permission boundary.
pub struct Balikan<P> {
    pool: PgPool,
    publisher: P,
}

impl<P> Balikan<P> {
    pub fn baru(pool: PgPool, publisher: P) -> Self {
        Self { pool, publisher }
    }
}

impl<P> crate::jadwal::Pekerjaan for Balikan<P>
where
    P: JobPublisher + Send + Sync,
{
    fn nama(&self) -> &'static str { NAMA_JOB }

    fn jalankan(
        &self,
        _sekarang: DateTime<Utc>,
    ) -> crate::jadwal::KotakFutur<'_, Result<bool, String>> {
        Box::pin(async move {
            let providers: Vec<Uuid> = sqlx::query_scalar(
                "SELECT provider_id FROM provider WHERE aktif AND data_balikan_aktif ORDER BY provider_id",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())?;

            for provider_id in providers {
                self.publisher
                    .publish(NAMA_QUEUE, provider_id)
                    .await
                    .map_err(|e: QueueError| e.to_string())?;
            }
            Ok(true)
        })
    }
}
