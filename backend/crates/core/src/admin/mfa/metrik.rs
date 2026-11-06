//! Metrics owned by MFA: one counter for recovery codes used (AC-7).

pub const KODE_PEMULIHAN_TOTAL: &str = "poolrouter_mfa_kode_pemulihan_total";

/// AC-7: exactly one increment per recovery code consumed.
pub fn kode_pemulihan() {
    metrics::counter!(KODE_PEMULIHAN_TOTAL).increment(1);
}
