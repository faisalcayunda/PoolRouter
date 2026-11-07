//! Controller job for dispatching one data-balikan job per eligible provider.

use chrono::{DateTime, Utc};
use poolrouter_core::queue::{JobPublisher, QueueError};
use std::sync::{Arc, Mutex};
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

async fn terbitkan<P: JobPublisher + Send + Sync>(publisher: &P, providers: Vec<Uuid>) -> Result<(), QueueError> {
    for provider_id in providers {
        publisher.publish(NAMA_QUEUE, provider_id).await?;
    }
    Ok(())
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

            terbitkan(&self.publisher, providers)
                .await
                .map_err(|e| e.to_string())?;
            Ok(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Publisher(Arc<Mutex<Vec<(String, Uuid)>>>);

    impl JobPublisher for Publisher {
        async fn publish(&self, queue: &str, request_id: Uuid) -> Result<(), QueueError> {
            self.0.lock().unwrap().push((queue.to_owned(), request_id));
            Ok(())
        }
    }

    #[tokio::test]
    async fn menerbitkan_tepat_satu_pesan_per_provider() {
        let messages = Arc::new(Mutex::new(Vec::new()));
        let publisher = Publisher(messages.clone());
        let eligible = [Uuid::from_u128(1), Uuid::from_u128(2)];

        terbitkan(&publisher, eligible.into_iter().collect()).await.unwrap();

        let messages = messages.lock().unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|(queue, _)| queue == NAMA_QUEUE));
        assert_eq!(messages.iter().map(|(_, id)| *id).collect::<Vec<_>>(), eligible);
    }
}
