use poolrouter_core::retry::SumberAcakBackoff;
use poolrouter_core::webhook::retry::{klasifikasi, jeda, parse_retry_after, Keputusan};

struct Stub(u64);
impl SumberAcakBackoff for Stub {
    fn acak_hingga(&self, batas: u64) -> u64 { self.0.min(batas) }
}

#[test]
fn ac1_klasifikasi_tabel() {
    assert_eq!(klasifikasi("terkirim", Some(200), None), Keputusan::Final);
    assert_eq!(klasifikasi("gagal_http", Some(408), None), Keputusan::Retry);
    assert_eq!(klasifikasi("gagal_http", Some(429), None), Keputusan::Retry);
    assert_eq!(klasifikasi("gagal_http", Some(500), None), Keputusan::Retry);
    assert_eq!(klasifikasi("gagal_http", Some(503), None), Keputusan::Retry);
    assert_eq!(klasifikasi("gagal_http", Some(400), None), Keputusan::Final);
    assert_eq!(klasifikasi("gagal_http", Some(410), None), Keputusan::Final);
    assert_eq!(klasifikasi("gagal_http", Some(301), None), Keputusan::Final);
    assert_eq!(klasifikasi("gagal_jaringan", None, Some("gagal_jaringan")), Keputusan::Retry);
    assert_eq!(klasifikasi("gagal_http", None, Some("ditolak_allowlist")), Keputusan::Final);
    assert_eq!(klasifikasi("gagal_http", None, Some("gagal_internal")), Keputusan::Final);
}

#[test]
fn ac2_jeda_default() {
    // stub tengah: acak_hingga returns batas/2? Use AcakBatasAtas-like upper bound? Use stub 0.8..1.2 via draw.
    // Middle stub: draw = jitter_persen (100% factor) -> nominal
    let stub = Stub(20); // draw 20, jitter = 20-20=0 -> factor 100
    for (n, expected) in [(1u32,30i64),(2,120),(3,480),(4,1920),(5,7200)] {
        let d = jeda(n, 30, 7200, 20, &stub, None);
        assert_eq!(d.num_seconds(), expected, "n={}", n);
    }
}

#[test]
fn ac3_parse_retry_after() {
    use chrono::Utc;
    let now = Utc::now();
    let cap = chrono::Duration::seconds(7200);
    assert_eq!(parse_retry_after("10", now, cap).unwrap().num_seconds(), 10);
    assert!(parse_retry_after("-5", now, cap).is_none());
    assert!(parse_retry_after("", now, cap).is_none());
    assert!(parse_retry_after("abc", now, cap).is_none());
    // cap
    assert_eq!(parse_retry_after("999999", now, cap).unwrap().num_seconds(), 7200);
}

#[test]
fn ac3_parse_retry_after_http_date() {
    use chrono::{Duration, Utc};
    let now = Utc::now();
    let cap = Duration::seconds(7200);
    // Valid HTTP-date 30s in the future.
    let masa_depan = now + Duration::seconds(30);
    let fmt = masa_depan.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    let d = parse_retry_after(&fmt, now, cap).expect("HTTP-date valid harus terparse");
    assert!((d.num_seconds() - 30).abs() <= 1);
    // Expired HTTP-date (past) -> None.
    let masa_lalu = now - Duration::seconds(30);
    let fmt_lalu = masa_lalu.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    assert!(parse_retry_after(&fmt_lalu, now, cap).is_none());
    // Invalid date string -> None.
    assert!(parse_retry_after("Not a date", now, cap).is_none());
    // Far-future HTTP-date is capped.
    let jauh = now + Duration::seconds(99999);
    let fmt_jauh = jauh.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    assert_eq!(parse_retry_after(&fmt_jauh, now, cap).unwrap().num_seconds(), 7200);
}

#[test]
fn ac2_jeda_jitter_batas_bawah_atas() {
    // Lower bound stub: draw = 0 -> jitter = -20 -> factor 80 -> 0.8x nominal.
    let bawah = Stub(0);
    let d = jeda(1, 30, 7200, 20, &bawah, None);
    assert_eq!(d.num_milliseconds(), 30 * 1000 * 80 / 100);

    // Upper bound stub: draw = 2*jitter_persen -> jitter = +20 -> factor 120 -> 1.2x nominal.
    let atas = Stub(40);
    let d = jeda(1, 30, 7200, 20, &atas, None);
    assert_eq!(d.num_milliseconds(), 30 * 1000 * 120 / 100);

    // jitter_persen = 0 is deterministic: factor 100 regardless of the draw.
    let apa_saja = Stub(999);
    let d = jeda(2, 30, 7200, 0, &apa_saja, None);
    assert_eq!(d.num_seconds(), 120);

    // Cap enforced even with upper-bound jitter.
    let d = jeda(5, 30, 7200, 20, &atas, None);
    assert_eq!(d.num_seconds(), 7200);
}
