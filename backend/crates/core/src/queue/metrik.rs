//! Metrik antrean (ADR-0001 rev 15 bagian 5a.9, ARC-228). Nama tetap di sini
//! supaya pemanggil tidak menciptakan label sendiri.

use metrics::gauge;

/// Jumlah pesan di semua DLQ upstream pool, tanpa label (tabel 5a.9). Alert
/// `poolrouter_dlq_pesan > 0`: docs/runbook/dlq.md.
pub const DLQ_PESAN: &str = "poolrouter_dlq_pesan";

/// Set `poolrouter_dlq_pesan` ke total pesan di semua DLQ.
pub fn dlq_diukur(jumlah: u64) {
    gauge!(DLQ_PESAN).set(jumlah as f64);
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics_exporter_prometheus::PrometheusBuilder;

    #[test]
    fn dlq_diukur_mengisi_gauge_tanpa_label() {
        let rekaman = PrometheusBuilder::new().build_recorder();
        metrics::with_local_recorder(&rekaman, || {
            dlq_diukur(7);
            dlq_diukur(3);
        });
        let teks = rekaman.handle().render();
        assert!(
            teks.lines().any(|b| b == "poolrouter_dlq_pesan 3"),
            "{teks}"
        );
    }
}
