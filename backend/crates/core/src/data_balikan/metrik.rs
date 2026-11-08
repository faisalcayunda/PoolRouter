//! Metrics owned by the data balikan module (spec `f1-webhook-retry-data-balikan`,
//! AC-30/AC-38).
//!
//! Label `hasil` is a closed set so no caller can invent a value. NIK, `request_id`,
//! `kode_transaksi`, and the body never become labels (ADR-0004 section 4.5).

use metrics::{counter, gauge};

pub const BATCH_TOTAL: &str = "poolrouter_data_balikan_batch_total";
pub const TERTUNDA: &str = "poolrouter_data_balikan_tertunda";

/// Terminal or retryable outcome of one batch attempt, as reported to metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HasilBatch {
    Diterima,
    Gagal,
    GagalPermanen,
}

impl HasilBatch {
    pub fn sebagai_label(self) -> &'static str {
        match self {
            Self::Diterima => "diterima",
            Self::Gagal => "gagal",
            Self::GagalPermanen => "gagal_permanen",
        }
    }
}

/// `poolrouter_data_balikan_batch_total{hasil}`: one increment per settled attempt.
pub fn batch(hasil: HasilBatch) {
    counter!(BATCH_TOTAL, "hasil" => hasil.sebagai_label()).increment(1);
}

/// `poolrouter_data_balikan_tertunda`: item eligible yang belum masuk batch (DQ-20).
pub fn tertunda(jumlah: u64) {
    gauge!(TERTUNDA).set(jumlah as f64);
}

#[cfg(test)]
mod uji {
    use super::*;
    use metrics_exporter_prometheus::PrometheusBuilder;

    fn render(isi: impl FnOnce()) -> String {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, isi);
        handle.render()
    }

    #[test]
    fn nama_dan_label_mengikuti_katalog() {
        let out = render(|| {
            batch(HasilBatch::Diterima);
            batch(HasilBatch::Gagal);
            batch(HasilBatch::GagalPermanen);
            tertunda(3);
        });
        for harap in [
            r#"poolrouter_data_balikan_batch_total{hasil="diterima"} 1"#,
            r#"poolrouter_data_balikan_batch_total{hasil="gagal"} 1"#,
            r#"poolrouter_data_balikan_batch_total{hasil="gagal_permanen"} 1"#,
            r#"poolrouter_data_balikan_tertunda 3"#,
        ] {
            assert!(out.contains(harap), "{harap} tidak ada di:\n{out}");
        }
    }
}
