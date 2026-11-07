//! Kebijakan retry webhook (ADR-0006 §5, ARC-120/ARC-118).
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use crate::retry::SumberAcakBackoff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keputusan { Retry, Final }

pub fn klasifikasi(hasil: &str, http_status: Option<i32>, error_class: Option<&str>) -> Keputusan {
    if hasil == "terkirim" { return Keputusan::Final; }
    if hasil == "tidak_dikirim" { return Keputusan::Final; }
    if let Some(code) = http_status {
        match code {
            408 | 429 => return Keputusan::Retry,
            500..=599 => return Keputusan::Retry,
            _ => return Keputusan::Final,
        }
    }
    match error_class {
        Some("gagal_jaringan") | Some("klaim_kedaluwarsa") => Keputusan::Retry,
        _ => Keputusan::Final,
    }
}

/// Jeda retry: min(cap, base * 4^(n-1)) lalu jitter ±jitter_persen via SumberAcakBackoff.
pub fn jeda(n: u32, base: u32, cap: u32, jitter_persen: u32, acak: &dyn SumberAcakBackoff, retry_after: Option<Duration>) -> Duration {
    if let Some(ra) = retry_after {
        return ra.min(Duration::seconds(cap as i64));
    }
    let kali = 4u64.pow(n.saturating_sub(1));
    let nominal = (base as u64).saturating_mul(kali).min(cap as u64);
    // jitter: acak_hingga returns 0..=batas; batas = jitter_persen (in percent units)
    let batas = (jitter_persen as u64).max(1);
    let draw = acak.acak_hingga(batas * 2);
    let faktor = 100i64 + (draw as i64 - jitter_persen as i64);
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
    // HTTP-date (IMF-fixdate): "Sun, 06 Nov 1994 08:49:37 GMT"
    if let Ok(dt) = DateTime::parse_from_rfc2822(v) {
        let delta = dt.with_timezone(&Utc) - sekarang;
        if delta.num_seconds() < 0 { return None; }
        return Some(delta.min(cap));
    }
    // Fallback format
    if let Ok(ndt) = NaiveDateTime::parse_from_str(v, "%a, %d %b %Y %H:%M:%S GMT") {
        let dt = ndt.and_utc();
        let delta = dt - sekarang;
        if delta.num_seconds() < 0 { return None; }
        return Some(delta.min(cap));
    }
    None
}
