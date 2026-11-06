//! Kebijakan retry webhook (ADR-0006 §5, ARC-120/ARC-118).
use chrono::{DateTime, Duration, Utc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keputusan { Retry, Final }

pub fn klasifikasi(hasil: &str, http_status: Option<i32>, error_class: Option<&str>) -> Keputusan {
    if hasil == "terkirim" { return Keputusan::Final; }
    if hasil == "tidak_dikirim" { return Keputusan::Final; }
    if let Some(code) = http_status {
        match code {
            408 | 429 => return Keputusan::Retry,
            500..=599 => return Keputusan::Retry,
            300..=399 => return Keputusan::Final,
            400 | 410 => return Keputusan::Final,
            401..=499 => return Keputusan::Final,
            _ => {}
        }
    }
    match error_class {
        Some("gagal_jaringan") | Some("klaim_kedaluwarsa") => Keputusan::Retry,
        Some("ditolak_allowlist") | Some("gagal_internal") | Some("dekripsi_gagal") => Keputusan::Final,
        _ => Keputusan::Final,
    }
}

pub fn jeda(n: u32, base: u32, cap: u32, jitter_persen: u32, acak: u64, retry_after: Option<Duration>) -> Duration {
    if let Some(ra) = retry_after {
        let capped = ra.min(Duration::seconds(cap as i64));
        return capped;
    }
    let kali = 1u32.checked_shl((n.saturating_sub(1)).min(31)).unwrap_or(u32::MAX);
    let nominal = (base as u64).saturating_mul(kali as u64).min(cap as u64);
    let jitter = (acak % (2 * jitter_persen as u64 + 1)) as i64 - jitter_persen as i64;
    let faktor = 100i64 + jitter;
    let ms = (nominal as i64 * 1000 * faktor / 100).max(0);
    Duration::milliseconds(ms.min(cap as i64 * 1000))
}

pub fn parse_retry_after(val: &str, sekarang: DateTime<Utc>, cap: Duration) -> Option<Duration> {
    let v = val.trim();
    if v.is_empty() { return None; }
    if let Ok(sec) = v.parse::<i64>() {
        if sec < 0 { return None; }
        return Some(Duration::seconds(sec).min(cap));
    }
    // HTTP-date not fully parsed; return None for invalid
    None
}
