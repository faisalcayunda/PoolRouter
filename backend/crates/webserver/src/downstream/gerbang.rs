//! The intake gate: validation, then the per-client rejection limiter, then the
//! quota decision (ADR-0001 rev 15 sections 4 and 5a.1; ADR-0002 rev 5 D6;
//! SHAA-198 part 2).
//!
//! Two rules make the order non-obvious, so they live here instead of inside a
//! handler:
//!
//! 1. **Every rejection after authentication costs one rejection token**, not
//!    only the ones that write a row. A `Idempotency-Key` on the sync path (400)
//!    and a fingerprint mismatch (422) are rejections too, so a client cannot
//!    dodge the limiter by sending garbage. An accepted request never touches
//!    the bucket.
//! 2. **An exhausted limiter answers 429 `penolakan_berlebih` with no `request`
//!    row.** Writing the row would let a rejected request pile up storage and
//!    would also spend the pool's quota, which is exactly what the limiter
//!    exists to prevent.
//!
//! Everything here is pure: it takes decisions already made by the store and
//! returns the next step, so the whole matrix is unit testable without a
//! database or Redis.

use axum::http::StatusCode;
use poolrouter_core::kuota::store::{HasilIntake, HasilPenolakan};

use crate::api::problem::{JumlahNamaTidakCocok, ProblemKodeDown, ValidasiErrorDown};

use super::intake::TolakIntake;
use super::layers::ApiError;

/// What the caller does next. The handler matches on this and performs the I/O.
///
/// `PartialEq` without `Eq` for the same reason as `Jawaban`.
#[derive(Debug, Clone, PartialEq)]
pub enum Keputusan {
    /// The body is acceptable and no rejection token was spent: ask Redis for a
    /// quota slot.
    CekKuota,
    /// Refuse and persist a `rejected` row with this response.
    TolakDenganBaris(Jawaban),
    /// Refuse without a row, because the limiter is what refused.
    TolakTanpaBaris(Jawaban),
}

/// A refusal, described independently of the HTTP layer so it can be compared
/// in tests.
///
/// `PartialEq` without `Eq`: `errors` carries an `f64` bound.
#[derive(Debug, Clone, PartialEq)]
pub enum Jawaban {
    ValidasiGagal {
        status: u16,
        detail: Option<&'static str>,
        errors: Vec<ValidasiErrorDown>,
        jumlah_nama_tidak_cocok: Option<JumlahNamaTidakCocok>,
    },
    MetadataTidakValid {
        errors: Vec<ValidasiErrorDown>,
        jumlah_nama_tidak_cocok: Option<JumlahNamaTidakCocok>,
    },
    IdempotencyKeyTidakDidukung,
    IdempotencyKeyKonflik,
    IdempotencyKeyTidakSah,
    PenolakanBerlebih {
        retry_after: u64,
    },
    KuotaHabis {
        retry_after: u64,
    },
    PorsiSyncHabis {
        retry_after: u64,
    },
}

impl Jawaban {
    /// The wire response. `jenis_objek` and `request_id` are not part of these
    /// refusals, so they are never set here.
    pub fn into_api_error(self) -> ApiError {
        match self {
            Self::ValidasiGagal {
                status,
                detail,
                errors,
                jumlah_nama_tidak_cocok,
            } => {
                let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
                let err = ApiError::new(status, ProblemKodeDown::ValidasiGagal)
                    .errors(errors, jumlah_nama_tidak_cocok);
                match detail {
                    Some(d) => err.detail(d),
                    None => err,
                }
            }
            Self::MetadataTidakValid {
                errors,
                jumlah_nama_tidak_cocok,
            } => ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, ProblemKodeDown::MetadataTidakValid)
                .errors(errors, jumlah_nama_tidak_cocok),
            Self::IdempotencyKeyTidakDidukung => ApiError::new(
                StatusCode::BAD_REQUEST,
                ProblemKodeDown::IdempotencyKeyTidakDidukung,
            ),
            Self::IdempotencyKeyKonflik => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                ProblemKodeDown::IdempotencyKeyKonflik,
            ),
            Self::IdempotencyKeyTidakSah => {
                ApiError::new(StatusCode::BAD_REQUEST, ProblemKodeDown::ValidasiGagal)
            }
            Self::PenolakanBerlebih { retry_after } => ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                ProblemKodeDown::PenolakanBerlebih,
            )
            .retry_after(retry_after),
            Self::KuotaHabis { retry_after } => {
                ApiError::new(StatusCode::TOO_MANY_REQUESTS, ProblemKodeDown::KuotaHabis)
                    .retry_after(retry_after)
            }
            Self::PorsiSyncHabis { retry_after } => ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                ProblemKodeDown::PorsiSyncHabis,
            )
            .retry_after(retry_after),
        }
    }
}

/// The rejection a `TolakIntake` becomes, before the limiter is consulted.
pub fn jawaban_untuk(tolak: TolakIntake) -> Jawaban {
    match tolak {
        TolakIntake::BodyTidakSah(detail) => Jawaban::ValidasiGagal {
            status: 400,
            detail: Some(detail),
            errors: Vec::new(),
            jumlah_nama_tidak_cocok: None,
        },
        TolakIntake::InputTidakValid {
            errors,
            jumlah_nama_tidak_cocok,
        } => Jawaban::ValidasiGagal {
            status: 422,
            detail: None,
            errors,
            jumlah_nama_tidak_cocok,
        },
        TolakIntake::MetadataTidakValid {
            errors,
            jumlah_nama_tidak_cocok,
        } => Jawaban::MetadataTidakValid {
            errors,
            jumlah_nama_tidak_cocok,
        },
        TolakIntake::IdempotencyKeyTidakDidukung => Jawaban::IdempotencyKeyTidakDidukung,
        TolakIntake::IdempotencyKeyKonflik => Jawaban::IdempotencyKeyKonflik,
        TolakIntake::IdempotencyKeyTidakSah => Jawaban::IdempotencyKeyTidakSah,
    }
}

/// Whether a refusal is ever persisted as a `rejected` row.
///
/// The idempotency refusals are the exception, and the reason is in the schema:
/// `chk_reject_http` allows only `validasi_gagal`, `metadata_tidak_valid`,
/// `kuota_habis`, `porsi_sync_habis` and `objek_nonaktif` as `reject_reason`.
/// ADR-0001 section 4 therefore calls these "penolakan tanpa baris": they still
/// cost a rejection token, they just never write a row.
fn menulis_baris(j: &Jawaban) -> bool {
    !matches!(
        j,
        Jawaban::IdempotencyKeyTidakDidukung
            | Jawaban::IdempotencyKeyKonflik
            | Jawaban::IdempotencyKeyTidakSah
            | Jawaban::PenolakanBerlebih { .. }
    )
}

/// The gate. `tolak` is the validation outcome; `limiter` is the client's
/// rejection bucket, consulted **only** when validation failed.
pub fn putuskan(tolak: Option<TolakIntake>, limiter: HasilPenolakan) -> Keputusan {
    let Some(tolak) = tolak else {
        // Accepted: the rejection bucket is untouched.
        return Keputusan::CekKuota;
    };
    // The bucket is consulted first: an exhausted limiter wins over whichever
    // refusal was coming, because answering it costs no further work.
    if matches!(limiter, HasilPenolakan::Dibatasi { .. }) {
        return Keputusan::TolakTanpaBaris(Jawaban::PenolakanBerlebih {
            // `retry_after_detik` is the one place the rounding rule lives.
            retry_after: limiter.retry_after_detik().unwrap_or(1),
        });
    }
    let jawaban = jawaban_untuk(tolak);
    if menulis_baris(&jawaban) {
        Keputusan::TolakDenganBaris(jawaban)
    } else {
        Keputusan::TolakTanpaBaris(jawaban)
    }
}

/// The quota decision (K-12). `terpakai` and `antre` come from the store so the
/// `Retry-After` can be derived by the caller when it has the window.
pub fn jawaban_kuota(hasil: &HasilIntake, retry_after: u64) -> Option<Jawaban> {
    match hasil {
        HasilIntake::Lolos { .. } => None,
        // `terpakai >= kuota_efektif` and `terpakai + antre >= kuota_efektif`
        // are both 429 `kuota_habis` for the client (ADR-0001 section 4); they
        // differ only in the metric label.
        HasilIntake::KuotaHabis { .. } | HasilIntake::AntreanPenuh { .. } => {
            Some(Jawaban::KuotaHabis { retry_after })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tolak_input() -> TolakIntake {
        TolakIntake::InputTidakValid {
            errors: vec![ValidasiErrorDown {
                field: "NIK".to_owned(),
                kode: "wajib".to_owned(),
                batas: None,
            }],
            jumlah_nama_tidak_cocok: None,
        }
    }

    #[test]
    fn body_diterima_tidak_menyentuh_bucket_penolakan() {
        // Even a throttled bucket does not matter when nothing was rejected.
        let k = putuskan(
            None,
            HasilPenolakan::Dibatasi {
                tunggu_mikro: 1_500_000,
            },
        );
        assert_eq!(k, Keputusan::CekKuota);
    }

    #[test]
    fn penolakan_dengan_token_menulis_baris() {
        let k = putuskan(Some(tolak_input()), HasilPenolakan::Lolos { sisa_token: 5 });
        let Keputusan::TolakDenganBaris(j) = k else {
            panic!("harus menolak dengan baris");
        };
        let Jawaban::ValidasiGagal { status, .. } = j else {
            panic!("harus validasi gagal");
        };
        assert_eq!(status, 422);
    }

    #[test]
    fn penolakan_tanpa_token_tidak_menulis_baris_dan_memakai_retry_after() {
        let k = putuskan(
            Some(tolak_input()),
            HasilPenolakan::Dibatasi {
                tunggu_mikro: 1_500_000,
            },
        );
        let Keputusan::TolakTanpaBaris(j) = k else {
            panic!("harus menolak tanpa baris");
        };
        assert_eq!(j, Jawaban::PenolakanBerlebih { retry_after: 2 });
    }

    #[test]
    fn retry_after_penolakan_minimal_satu_detik() {
        let k = putuskan(
            Some(tolak_input()),
            HasilPenolakan::Dibatasi { tunggu_mikro: 1 },
        );
        let Keputusan::TolakTanpaBaris(Jawaban::PenolakanBerlebih { retry_after }) = k else {
            panic!("harus penolakan berlebih");
        };
        assert_eq!(retry_after, 1, "detik dibulatkan ke atas, minimal 1");
    }

    #[test]
    fn penolakan_tanpa_baris_juga_berlaku_untuk_idempotency_di_sync() {
        // The ADR lists this case explicitly: 400 with no row, but it still
        // costs a rejection token.
        let k = putuskan(
            Some(TolakIntake::IdempotencyKeyTidakDidukung),
            HasilPenolakan::Dibatasi {
                tunggu_mikro: 2_000_000,
            },
        );
        assert_eq!(
            k,
            Keputusan::TolakTanpaBaris(Jawaban::PenolakanBerlebih { retry_after: 2 })
        );
    }

    #[test]
    fn idempotency_di_sync_dengan_token_tetap_tanpa_baris() {
        // `chk_reject_http` has no `idempotency_key_*` reject_reason, so the row
        // can never be written even when a token was available.
        let k = putuskan(
            Some(TolakIntake::IdempotencyKeyTidakDidukung),
            HasilPenolakan::Lolos { sisa_token: 1 },
        );
        assert_eq!(
            k,
            Keputusan::TolakTanpaBaris(Jawaban::IdempotencyKeyTidakDidukung)
        );
    }

    #[test]
    fn konflik_sidik_juga_tanpa_baris() {
        let k = putuskan(
            Some(TolakIntake::IdempotencyKeyKonflik),
            HasilPenolakan::Lolos { sisa_token: 9 },
        );
        assert_eq!(
            k,
            Keputusan::TolakTanpaBaris(Jawaban::IdempotencyKeyKonflik)
        );
    }

    #[test]
    fn hanya_penolakan_yang_punya_reject_reason_yang_menulis_baris() {
        for tolak in [
            TolakIntake::BodyTidakSah("bukan json"),
            tolak_input(),
            TolakIntake::MetadataTidakValid {
                errors: vec![],
                jumlah_nama_tidak_cocok: None,
            },
        ] {
            let k = putuskan(Some(tolak), HasilPenolakan::Lolos { sisa_token: 1 });
            assert!(
                matches!(k, Keputusan::TolakDenganBaris(_)),
                "harus menulis baris: {k:?}"
            );
        }
    }

    #[test]
    fn body_bukan_json_menjadi_400_bukan_422() {
        let j = jawaban_untuk(TolakIntake::BodyTidakSah("bukan json"));
        let Jawaban::ValidasiGagal { status, .. } = j else {
            panic!("harus validasi gagal");
        };
        assert_eq!(status, 400);
    }

    #[test]
    fn metadata_memakai_kode_metadata_bukan_validasi() {
        let j = jawaban_untuk(TolakIntake::MetadataTidakValid {
            errors: vec![],
            jumlah_nama_tidak_cocok: None,
        });
        let err = j.clone().into_api_error();
        assert_eq!(err.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(matches!(j, Jawaban::MetadataTidakValid { .. }));
    }

    #[test]
    fn kuota_habis_dan_antrean_penuh_sama_sama_kuota_habis() {
        for hasil in [
            HasilIntake::KuotaHabis {
                terpakai: 100,
                antre: 0,
            },
            HasilIntake::AntreanPenuh {
                terpakai: 90,
                antre: 10,
            },
        ] {
            assert_eq!(
                jawaban_kuota(&hasil, 42),
                Some(Jawaban::KuotaHabis { retry_after: 42 })
            );
        }
    }

    #[test]
    fn kuota_lolos_tidak_menghasilkan_jawaban() {
        assert_eq!(
            jawaban_kuota(&HasilIntake::Lolos { terpakai: 1, antre: 0 }, 42),
            None
        );
    }

    #[test]
    fn kuota_habis_selalu_membawa_retry_after() {
        let j = jawaban_kuota(
            &HasilIntake::KuotaHabis {
                terpakai: 1,
                antre: 0,
            },
            0,
        )
        .unwrap();
        let err = j.into_api_error();
        assert_eq!(err.status(), StatusCode::TOO_MANY_REQUESTS);
        // `.retry_after` clamps to at least 1 (ADR-0001 section 5a.7).
        assert_eq!(err.retry_after_secs(), Some(1));
    }

    #[test]
    fn body_penolakan_berlebih_hanya_empat_field() {
        // contract L-05: type, title, status, kode; never error, never errors.
        let j = Jawaban::PenolakanBerlebih { retry_after: 3 }.into_api_error();
        assert_eq!(j.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(j.retry_after_secs(), Some(3));
    }
}