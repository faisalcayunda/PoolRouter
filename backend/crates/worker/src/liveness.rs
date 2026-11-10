//! Pengawas liveness worker (ARC-226).

use std::time::Duration;

use poolrouter_core::runtime::Liveness;

/// Selesai begitu `live` melaporkan proses tidak hidup; memeriksa tiap
/// `interval`.
pub async fn tunggu_mati(live: &dyn Liveness, interval: Duration) {
    let mut tik = tokio::time::interval(interval);
    while live.alive() {
        tik.tick().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Saklar(AtomicBool);
    impl Liveness for Saklar {
        fn alive(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    #[tokio::test]
    async fn tunggu_mati_baru_selesai_setelah_liveness_gagal() {
        let saklar = Arc::new(Saklar(AtomicBool::new(true)));
        let interval = Duration::from_millis(5);

        let pengawas = {
            let saklar = Arc::clone(&saklar);
            tokio::spawn(async move { tunggu_mati(&*saklar, interval).await })
        };
        // Selama hidup, pengawas tidak boleh selesai.
        let masih_hidup = tokio::time::timeout(Duration::from_millis(100), async {
            while !pengawas.is_finished() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(masih_hidup.is_err(), "selesai padahal proses masih hidup");

        saklar.0.store(false, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(5), pengawas)
            .await
            .expect("pengawas harus selesai setelah liveness gagal")
            .expect("task tidak panic");
    }
}
