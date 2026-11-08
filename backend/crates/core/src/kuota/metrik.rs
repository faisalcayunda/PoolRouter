//! Metrics owned by the quota module (ADR-0001 rev 15 section 5a.9, ADR-0002
//! rev 5 D6 / contract L-06). Names and labels are fixed here so no caller can
//! invent a label.
//!
//! Allowed labels: `upstream_pool_id`, `client_id`, `komponen`. Never NIK,
//! `request_id`, `kode_transaksi` or an IP (ADR-0004 section 4.5). `komponen`
//! comes from the closed set below; any other value is reported as `lainnya`
//! instead of being passed through (contract MT-01).
//!
//! Webserver-level metrics (`poolrouter_penolakan_total`,
//! `poolrouter_penolakan_dibatasi_total`, `poolrouter_penolakan_tanpa_baris_total`)
//! need the downstream endpoint and the HTTP status, so they live with the 429
//! mapping in SHAA-198 part 2, not here.

use metrics::{counter, gauge};

pub const KUOTA_TERPAKAI: &str = "poolrouter_kuota_terpakai";
pub const KUOTA_EFEKTIF: &str = "poolrouter_kuota_efektif";
pub const RETRY_BUDGET_HABIS_TOTAL: &str = "poolrouter_retry_budget_habis_total";
pub const SLOT_KEDALUWARSA_TOTAL: &str = "poolrouter_slot_kedaluwarsa_total";
pub const REDIS_FAILCLOSED_TOTAL: &str = "poolrouter_redis_failclosed_total";
pub const REDIS_REBUILD_TOTAL: &str = "poolrouter_redis_rebuild_total";
pub const LIMITER_PENOLAKAN_TOKEN: &str = "poolrouter_limiter_penolakan_token";
pub const PINJAMAN_TOTAL: &str = "poolrouter_pinjaman_total";

/// Which process reports (`komponen` label).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Komponen {
    Webserver,
    Worker,
    Controller,
    /// Anything outside the closed set (contract MT-01).
    #[default]
    Lainnya,
}

impl Komponen {
    pub fn sebagai_label(self) -> &'static str {
        match self {
            Self::Webserver => "webserver",
            Self::Worker => "worker",
            Self::Controller => "controller",
            Self::Lainnya => "lainnya",
        }
    }

    /// Maps free text to the closed set; unknown text becomes `Lainnya`.
    pub fn dari_teks(teks: &str) -> Self {
        match teks {
            "webserver" => Self::Webserver,
            "worker" => Self::Worker,
            "controller" => Self::Controller,
            _ => Self::Lainnya,
        }
    }
}

/// `poolrouter_kuota_terpakai` and `poolrouter_kuota_efektif` after a read
/// (quota tracker, alert near the limit).
pub fn kuota_dibaca(pool_id: &str, terpakai: i64, efektif: i64) {
    gauge!(KUOTA_TERPAKAI, "upstream_pool_id" => pool_id.to_owned()).set(terpakai as f64);
    gauge!(KUOTA_EFEKTIF, "upstream_pool_id" => pool_id.to_owned()).set(efektif as f64);
}

/// A retry was refused by the hourly budget (section 5a.3).
pub fn retry_budget_habis(pool_id: &str) {
    counter!(RETRY_BUDGET_HABIS_TOTAL, "upstream_pool_id" => pool_id.to_owned()).increment(1);
}

/// Slots that passed their TTL (section 5a.1 point 6): `jumlah` cleaned by the
/// token script, or 1 for an attempt whose slot was gone at count time.
pub fn slot_kedaluwarsa(pool_id: &str, jumlah: u64) {
    if jumlah > 0 {
        counter!(SLOT_KEDALUWARSA_TOTAL, "upstream_pool_id" => pool_id.to_owned())
            .increment(jumlah);
    }
}

/// A call failed closed because Redis was unusable (`belum_siap` or a
/// transport error).
pub fn redis_failclosed(komponen: Komponen) {
    counter!(REDIS_FAILCLOSED_TOTAL, "komponen" => komponen.sebagai_label()).increment(1);
}

/// A rebuild wrote the quota state (section 5a.2).
pub fn redis_rebuild(komponen: Komponen) {
    counter!(REDIS_REBUILD_TOTAL, "komponen" => komponen.sebagai_label()).increment(1);
}

/// Tokens left in a client's rejection bucket (ADR-0002 rev 5 D6, L-06).
pub fn limiter_token(client_id: &str, sisa: i64) {
    gauge!(LIMITER_PENOLAKAN_TOKEN, "client_id" => client_id.to_owned()).set(sisa as f64);
}

/// Token granted from a non-self source (AC-10, T2e). `sumber_txt` is the pool id
/// for own allocation, "sisa" for shareable, or the downstream pool id for loan.
pub fn pinjaman_total(upstream_pool_id: &str, sumber_txt: &str, downstream_pool_id: i64) {
    counter!(
        PINJAMAN_TOTAL,
        "upstream_pool_id" => upstream_pool_id.to_owned(),
        "downstream_pool_id" => downstream_pool_id.to_string(),
        "sumber_anggaran" => sumber_txt.to_owned()
    )
    .increment(1);
}

#[cfg(test)]
mod tests {
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
            kuota_dibaca("pool-1", 30, 100);
            retry_budget_habis("pool-1");
            slot_kedaluwarsa("pool-1", 2);
            redis_failclosed(Komponen::Webserver);
            redis_rebuild(Komponen::Controller);
            limiter_token("client-1", 7);
            pinjaman_total("pool-1", "sisa", 2);
            pinjaman_total("pool-1", "3", 2);
        });
        for harap in [
            r#"poolrouter_kuota_terpakai{upstream_pool_id="pool-1"} 30"#,
            r#"poolrouter_kuota_efektif{upstream_pool_id="pool-1"} 100"#,
            r#"poolrouter_retry_budget_habis_total{upstream_pool_id="pool-1"} 1"#,
            r#"poolrouter_slot_kedaluwarsa_total{upstream_pool_id="pool-1"} 2"#,
            r#"poolrouter_redis_failclosed_total{komponen="webserver"} 1"#,
            r#"poolrouter_redis_rebuild_total{komponen="controller"} 1"#,
            r#"poolrouter_limiter_penolakan_token{client_id="client-1"} 7"#,
            r#"poolrouter_pinjaman_total{downstream_pool_id="2",sumber_anggaran="sisa",upstream_pool_id="pool-1"} 1"#,
            r#"poolrouter_pinjaman_total{downstream_pool_id="2",sumber_anggaran="3",upstream_pool_id="pool-1"} 1"#,
        ] {
            assert!(out.contains(harap), "tidak ada: {harap}\n{out}");
        }
    }

    #[test]
    fn komponen_di_luar_himpunan_menjadi_lainnya() {
        assert_eq!(Komponen::dari_teks("webserver"), Komponen::Webserver);
        assert_eq!(Komponen::dari_teks("worker"), Komponen::Worker);
        assert_eq!(Komponen::dari_teks("controller"), Komponen::Controller);
        assert_eq!(Komponen::dari_teks("10.0.0.7"), Komponen::Lainnya);
        assert_eq!(Komponen::dari_teks("").sebagai_label(), "lainnya");
        let out = render(|| redis_failclosed(Komponen::dari_teks("nik-3174")));
        assert!(
            out.contains(r#"poolrouter_redis_failclosed_total{komponen="lainnya"} 1"#),
            "{out}"
        );
        assert!(!out.contains("nik-3174"));
    }

    #[test]
    fn slot_kedaluwarsa_nol_tidak_membuat_deret() {
        let out = render(|| slot_kedaluwarsa("pool-1", 0));
        assert!(!out.contains("poolrouter_slot_kedaluwarsa_total"), "{out}");
    }
}
