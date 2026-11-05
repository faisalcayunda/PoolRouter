//! Pemetaan status internal `request` ke status API downstream (ADR-0001 §4.1,
//! US-17). Satu fungsi murni dipakai endpoint polling dan webhook, supaya kedua
//! jalur tidak pernah menyimpang.
//!
//! Status API hanya mengeluarkan enam nilai (`queued`, `processing`, `done`,
//! `failed`, `expired`, `cancelled`) dan tidak pernah mundur. Pemetaannya
//! ditentukan oleh status internal **dan** `attempt_count`, karena `queued`
//! berarti "belum pernah dikirim ke provider dan masih bisa dibatalkan".

use serde::{Deserialize, Serialize};

/// Status internal baris `request` (`migrations/20261103000002_request_log.sql`,
/// constraint `status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusInternal {
    Queued,
    Dispatched,
    InFlight,
    Done,
    Failed,
    Expired,
    Cancelled,
    Rejected,
}

impl StatusInternal {
    /// Nilai kolom `request.status`.
    pub fn as_str(self) -> &'static str {
        match self {
            StatusInternal::Queued => "queued",
            StatusInternal::Dispatched => "dispatched",
            StatusInternal::InFlight => "in_flight",
            StatusInternal::Done => "done",
            StatusInternal::Failed => "failed",
            StatusInternal::Expired => "expired",
            StatusInternal::Cancelled => "cancelled",
            StatusInternal::Rejected => "rejected",
        }
    }
}

impl std::str::FromStr for StatusInternal {
    type Err = StatusInternalTidakDikenal;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "queued" => StatusInternal::Queued,
            "dispatched" => StatusInternal::Dispatched,
            "in_flight" => StatusInternal::InFlight,
            "done" => StatusInternal::Done,
            "failed" => StatusInternal::Failed,
            "expired" => StatusInternal::Expired,
            "cancelled" => StatusInternal::Cancelled,
            "rejected" => StatusInternal::Rejected,
            other => return Err(StatusInternalTidakDikenal(other.to_owned())),
        })
    }
}

/// Nilai `request.status` di luar constraint database. Menandakan bug pemanggil
/// atau data yang rusak, bukan input client.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("status internal tidak dikenal: {0}")]
pub struct StatusInternalTidakDikenal(pub String);

/// Status API yang tampil di polling dan webhook. Enam nilai, tidak pernah
/// mundur (ADR-0001 §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusApi {
    Queued,
    Processing,
    Done,
    Failed,
    Expired,
    Cancelled,
}

impl StatusApi {
    /// Nilai yang dikirim ke client, sama dengan enum `ApiStatus` di
    /// `downstream-api.yaml`.
    pub fn as_str(self) -> &'static str {
        match self {
            StatusApi::Queued => "queued",
            StatusApi::Processing => "processing",
            StatusApi::Done => "done",
            StatusApi::Failed => "failed",
            StatusApi::Expired => "expired",
            StatusApi::Cancelled => "cancelled",
        }
    }
}

/// Hasil pemetaan. `rejected` bukan status API: request itu tidak pernah punya
/// `queue_id`, tidak bisa dipolling, dan tidak dikirimi webhook (S-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusTerpetakan {
    Status(StatusApi),
    TidakBisaDipolling,
}

/// Pemetaan normatif ADR-0001 §4.1: status internal + `attempt_count` →
/// status API.
///
/// `attempt_count` yang tidak masuk akal untuk statusnya (mis. `in_flight`
/// dengan 0) tetap dipetakan menurut kolom status, karena constraint database
/// sudah menjamin kombinasinya dan klien tidak boleh menerima 500 karenanya.
pub fn status_api(status: StatusInternal, attempt_count: i32) -> StatusTerpetakan {
    use StatusTerpetakan::{Status, TidakBisaDipolling};
    match status {
        // Belum pernah dikirim ke provider: masih bisa dibatalkan.
        StatusInternal::Queued | StatusInternal::Dispatched if attempt_count == 0 => {
            Status(StatusApi::Queued)
        }
        // Menunggu retry: sudah terkirim sekali, pembatalan ditolak 409.
        StatusInternal::Queued | StatusInternal::Dispatched => Status(StatusApi::Processing),
        StatusInternal::InFlight => Status(StatusApi::Processing),
        StatusInternal::Done => Status(StatusApi::Done),
        StatusInternal::Failed => Status(StatusApi::Failed),
        StatusInternal::Expired => Status(StatusApi::Expired),
        StatusInternal::Cancelled => Status(StatusApi::Cancelled),
        StatusInternal::Rejected => TidakBisaDipolling,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Satu baris tabel status API ADR-0001 §4.1.
    struct Baris {
        status: StatusInternal,
        attempt_count: i32,
        ingin: StatusTerpetakan,
        catatan: &'static str,
    }

    fn tabel() -> Vec<Baris> {
        use StatusInternal as I;
        use StatusTerpetakan::{Status as S, TidakBisaDipolling};
        vec![
            Baris {
                status: I::Queued,
                attempt_count: 0,
                ingin: S(StatusApi::Queued),
                catatan: "queued, attempt_count 0",
            },
            Baris {
                status: I::Dispatched,
                attempt_count: 0,
                ingin: S(StatusApi::Queued),
                catatan: "dispatched, attempt_count 0",
            },
            Baris {
                status: I::Queued,
                attempt_count: 1,
                ingin: S(StatusApi::Processing),
                catatan: "queued menunggu retry",
            },
            Baris {
                status: I::Dispatched,
                attempt_count: 3,
                ingin: S(StatusApi::Processing),
                catatan: "dispatched menunggu retry",
            },
            Baris {
                status: I::InFlight,
                attempt_count: 1,
                ingin: S(StatusApi::Processing),
                catatan: "in_flight",
            },
            Baris {
                status: I::Done,
                attempt_count: 1,
                ingin: S(StatusApi::Done),
                catatan: "done",
            },
            Baris {
                status: I::Failed,
                attempt_count: 3,
                ingin: S(StatusApi::Failed),
                catatan: "failed",
            },
            Baris {
                status: I::Expired,
                attempt_count: 0,
                ingin: S(StatusApi::Expired),
                catatan: "expired",
            },
            Baris {
                status: I::Cancelled,
                attempt_count: 0,
                ingin: S(StatusApi::Cancelled),
                catatan: "cancelled",
            },
            Baris {
                status: I::Rejected,
                attempt_count: 0,
                ingin: TidakBisaDipolling,
                catatan: "rejected tidak pernah dipolling",
            },
        ]
    }

    /// AC-7: setiap baris tabel ADR-0001 §4.1 punya kasus di sini.
    #[test]
    fn setiap_baris_tabel_adr_terpetakan() {
        for b in tabel() {
            assert_eq!(
                status_api(b.status, b.attempt_count),
                b.ingin,
                "baris `{}`",
                b.catatan
            );
        }
    }

    /// Status API tidak pernah mundur: urutannya queued → processing → terminal,
    /// dan `attempt_count` hanya bisa menaikkan queued ke processing.
    #[test]
    fn attempt_count_menaikkan_queued_ke_processing_tanpa_mundur() {
        for status in [StatusInternal::Queued, StatusInternal::Dispatched] {
            assert_eq!(
                status_api(status, 0),
                StatusTerpetakan::Status(StatusApi::Queued)
            );
            for n in [1, 2, 10, 100] {
                assert_eq!(
                    status_api(status, n),
                    StatusTerpetakan::Status(StatusApi::Processing),
                    "attempt_count {n}"
                );
            }
        }
    }

    /// Setiap status internal punya pemetaan; tidak ada yang jatuh ke panic atau
    /// nilai tak terdefinisi.
    #[test]
    fn semua_status_internal_terpetakan() {
        let semua = [
            StatusInternal::Queued,
            StatusInternal::Dispatched,
            StatusInternal::InFlight,
            StatusInternal::Done,
            StatusInternal::Failed,
            StatusInternal::Expired,
            StatusInternal::Cancelled,
            StatusInternal::Rejected,
        ];
        let mut dapat_status = Vec::new();
        for s in semua {
            match status_api(s, 0) {
                StatusTerpetakan::Status(api) => dapat_status.push(api.as_str()),
                StatusTerpetakan::TidakBisaDipolling => {
                    assert_eq!(s, StatusInternal::Rejected);
                }
            }
        }
        let unik: std::collections::BTreeSet<_> = dapat_status.into_iter().collect();
        assert_eq!(unik.len(), 6, "enam status API, bukan lebih: {unik:?}");
    }

    /// `rejected` tidak punya `queue_id`, jadi tidak boleh muncul sebagai status
    /// API dalam bentuk apa pun.
    #[test]
    fn rejected_bukan_status_api() {
        for n in [0, 1, 5] {
            assert_eq!(
                status_api(StatusInternal::Rejected, n),
                StatusTerpetakan::TidakBisaDipolling
            );
        }
    }

    #[test]
    fn status_internal_round_trips_through_column_value() {
        for s in [
            StatusInternal::Queued,
            StatusInternal::Dispatched,
            StatusInternal::InFlight,
            StatusInternal::Done,
            StatusInternal::Failed,
            StatusInternal::Expired,
            StatusInternal::Cancelled,
            StatusInternal::Rejected,
        ] {
            assert_eq!(s.as_str().parse::<StatusInternal>().unwrap(), s);
        }
        assert!("InFlight".parse::<StatusInternal>().is_err());
        assert!("unknown".parse::<StatusInternal>().is_err());
    }
}
