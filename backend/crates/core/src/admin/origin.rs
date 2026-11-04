//! Origin checking for state-changing admin requests (ADR-0003 rev 5 section
//! 3.6; contract 403 `origin_tidak_valid`).
//!
//! The session cookie is `SameSite=Strict`, which already blocks cross-site
//! sends in every current browser. This check is the second lock, for the cases
//! `SameSite` does not cover: a browser that ignores the attribute, a legacy
//! client, or a same-site deployment where "site" is looser than "origin". It
//! answers one question — did this write come from *our* UI origin — and it
//! fails closed: a write with no `Origin` at all is refused.
//!
//! Failing closed is a deliberate choice and not an oversight. The admin API is
//! called only by the first-party SPA, whose `fetch` always sends `Origin` on
//! POST/PUT/PATCH/DELETE. A missing header therefore means something other than
//! the SPA is writing, and the safe answer is 403.
//!
//! `Sec-Fetch-Site`, when present, is checked too: it costs nothing and catches
//! a case the `Origin` comparison alone would allow — a page on a subdomain of
//! the UI origin, which is `same-site` but not `same-origin`.

use std::fmt;

use reqwest::Url;
use reqwest::header::{HeaderMap, HeaderValue, ORIGIN};

/// Where the admin API expects writes to come from: `POOLROUTER_UI_ORIGIN`
/// (see `core::config`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsalUi(Url);

impl AsalUi {
    /// Parse and normalise the configured origin.
    ///
    /// Normalised to scheme + host + port with no path, because an `Origin`
    /// header never carries a path: comparing against
    /// `https://ui.example.test/` would fail against `https://ui.example.test`.
    /// A URL with a path means the operator misconfigured it, so it is rejected
    /// at startup rather than silently trimmed.
    pub fn baru(raw: &str) -> Result<Self, AsalTidakValid> {
        let url = Url::parse(raw).map_err(|_| AsalTidakValid::BukanUrl)?;
        if !matches!(url.scheme(), "https" | "http") {
            return Err(AsalTidakValid::Skema);
        }
        // ADR-0003 section 3.6: the cookie is `__Host-`, which browsers only
        // accept over HTTPS. An http origin in anything but a local dev setup
        // can never work, so it is flagged here and the caller decides.
        if url.cannot_be_a_base() || url.host().is_none() {
            return Err(AsalTidakValid::BukanUrl);
        }
        if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
            return Err(AsalTidakValid::BukanAsal);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(AsalTidakValid::BukanAsal);
        }
        Ok(Self(url))
    }

    /// The normalised text, e.g. `https://ui.example.test` (port dropped when
    /// it is the scheme default, which is how `Origin` is spelled too).
    pub fn as_str(&self) -> &str {
        self.0.as_str().trim_end_matches('/')
    }

    /// Whether the cookie can carry the `__Host-` prefix. `true` for https
    /// origins; a `http://localhost` dev origin is the one accepted exception
    /// that this returns `false` for.
    pub fn aman_untuk_cookie_host(&self) -> bool {
        self.0.scheme() == "https"
            || matches!(
                self.0.host_str(),
                Some("localhost") | Some("127.0.0.1") | Some("[::1]")
            )
    }
}

impl fmt::Display for AsalUi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The configured UI origin is unusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsalTidakValid {
    BukanUrl,
    Skema,
    /// A path, query, fragment, or userinfo was present.
    BukanAsal,
}

impl fmt::Display for AsalTidakValid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BukanUrl => f.write_str("bukan URL yang sah"),
            Self::Skema => f.write_str("skema harus http atau https"),
            Self::BukanAsal => f.write_str("harus berupa origin saja, tanpa path/query/userinfo"),
        }
    }
}

impl std::error::Error for AsalTidakValid {}

/// Why a request was refused. Both map to 403 `origin_tidak_valid`; the
/// distinction is for the log line and for tests, not for the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginDitolak {
    /// No `Origin` header on a state-changing request.
    Hilang,
    /// `Origin: null` (sandboxed iframe, `file://`, or a stripped referrer).
    Null,
    /// An origin, but not ours.
    BukanAsalKami,
    /// `Sec-Fetch-Site` says cross-site or same-site-but-not-same-origin.
    FetchSite,
}

impl fmt::Display for OriginDitolak {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hilang => f.write_str("header Origin tidak ada"),
            Self::Null => f.write_str("Origin: null"),
            Self::BukanAsalKami => f.write_str("Origin bukan asal UI"),
            Self::FetchSite => f.write_str("Sec-Fetch-Site tidak same-origin"),
        }
    }
}

impl std::error::Error for OriginDitolak {}

/// The HTTP methods that can change state and therefore need the check.
///
/// `GET`/`HEAD`/`OPTIONS` are excluded: `GET` must not write (a separate rule
/// the handlers keep), and `OPTIONS` is preflight, which the browser sends
/// without the cookie.
pub fn butuh_cek(method: &str) -> bool {
    matches!(method, "POST" | "PUT" | "PATCH" | "DELETE")
}

/// Check a state-changing request.
///
/// Safe methods always pass: the caller may invoke this for every request
/// without thinking about the method, and `GET` cannot be forged into a write
/// by a cross-site form.
pub fn periksa(method: &str, headers: &HeaderMap, asal: &AsalUi) -> Result<(), OriginDitolak> {
    if !butuh_cek(method) {
        return Ok(());
    }

    // Sec-Fetch-Site first: when a browser sends it, it is authoritative and
    // cheaper to reason about than parsing Origin.
    if let Some(site) = headers.get("sec-fetch-site") {
        match site.to_str() {
            // Same-origin: the SPA. `none` is a user-initiated navigation, which
            // cannot be a cross-site form post, so it is allowed (a direct
            // download link should still work).
            Ok("same-origin") | Ok("none") => {}
            // Everything else — cross-site, same-site (subdomain), malformed.
            _ => return Err(OriginDitolak::FetchSite),
        }
    }

    let nilai = headers
        .get(ORIGIN)
        .ok_or(OriginDitolak::Hilang)?
        .to_str()
        .map_err(|_| OriginDitolak::BukanAsalKami)?;

    if nilai.eq_ignore_ascii_case("null") {
        return Err(OriginDitolak::Null);
    }

    // Compare normalised forms, not raw strings: `https://ui.example.test:443`
    // and `https://ui.example.test` are the same origin, and a browser may send
    // either.
    let diterima = normalkan(nilai).ok_or(OriginDitolak::BukanAsalKami)?;
    if diterima == asal.as_str() {
        Ok(())
    } else {
        Err(OriginDitolak::BukanAsalKami)
    }
}

/// Normalise an `Origin` header value the same way [`AsalUi::baru`] normalises
/// the configured value. `Origin` is serialised as scheme://host[:port], with
/// the port omitted when it is the scheme default; `Url` does the same, so a
/// URL round-trip is the implementation.
fn normalkan(nilai: &str) -> Option<String> {
    let url = Url::parse(nilai).ok()?;
    if !matches!(url.scheme(), "https" | "http") || url.host().is_none() {
        return None;
    }
    Some(url.as_str().trim_end_matches('/').to_owned())
}

/// Build the `Origin` value for outbound requests the backend itself makes to
/// the UI's own endpoints (rare, but the wrapper keeps the header spelling in
/// one place).
pub fn header_asal(asal: &AsalUi) -> HeaderValue {
    HeaderValue::from_str(asal.as_str()).expect("origin selalu ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asal() -> AsalUi {
        AsalUi::baru("https://ui.example.test").unwrap()
    }

    fn h(pasangan: &[(&'static str, &str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pasangan {
            m.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        m
    }

    #[test]
    fn origin_is_normalised_on_construction() {
        let a = AsalUi::baru("https://ui.example.test/").unwrap();
        assert_eq!(a.as_str(), "https://ui.example.test");
        // URL's Display puts back the trailing slash; as_str must not.
        assert!(!a.as_str().ends_with('/'));
    }

    #[test]
    fn default_port_is_equivalent() {
        // The configured origin and the header may disagree on an explicit
        // default port; both normalise to the same thing.
        let mut m = h(&[("origin", "https://ui.example.test:443")]);
        assert_eq!(periksa("POST", &m, &asal()), Ok(()));

        let dengan_port = AsalUi::baru("https://ui.example.test:443").unwrap();
        m = h(&[("origin", "https://ui.example.test")]);
        assert_eq!(periksa("POST", &m, &dengan_port), Ok(()));
    }

    #[test]
    fn non_default_port_is_significant() {
        let mut m = h(&[("origin", "https://ui.example.test:8443")]);
        assert_eq!(
            periksa("POST", &m, &asal()),
            Err(OriginDitolak::BukanAsalKami)
        );
        m = h(&[("origin", "https://ui.example.test")]);
        let lain = AsalUi::baru("https://ui.example.test:8443").unwrap();
        assert_eq!(
            periksa("POST", &m, &lain),
            Err(OriginDitolak::BukanAsalKami)
        );
    }

    #[test]
    fn scheme_is_significant() {
        let m = h(&[("origin", "http://ui.example.test")]);
        assert_eq!(
            periksa("POST", &m, &asal()),
            Err(OriginDitolak::BukanAsalKami)
        );
    }

    #[test]
    fn a_lookalike_host_is_refused() {
        for jahat in [
            "https://ui.example.test.evil.com",
            "https://evil-ui.example.test",
            "https://ui.example.test:443.evil.com",
            "https://ui-example.test",
        ] {
            let m = h(&[("origin", jahat)]);
            assert_eq!(
                periksa("POST", &m, &asal()),
                Err(OriginDitolak::BukanAsalKami),
                "{jahat} seharusnya ditolak"
            );
        }
    }

    #[test]
    fn origin_null_is_refused() {
        let m = h(&[("origin", "null")]);
        assert_eq!(periksa("POST", &m, &asal()), Err(OriginDitolak::Null));
        // Case-insensitively, since the token is not case-sensitive in practice.
        let m = h(&[("origin", "NULL")]);
        assert_eq!(periksa("POST", &m, &asal()), Err(OriginDitolak::Null));
    }

    #[test]
    fn a_missing_origin_on_a_write_fails_closed() {
        let m = HeaderMap::new();
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            assert_eq!(
                periksa(method, &m, &asal()),
                Err(OriginDitolak::Hilang),
                "{method}"
            );
        }
    }

    #[test]
    fn safe_methods_never_need_an_origin() {
        let m = HeaderMap::new();
        for method in ["GET", "HEAD", "OPTIONS", "get"] {
            assert_eq!(periksa(method, &m, &asal()), Ok(()), "{method}");
        }
    }

    #[test]
    fn a_garbage_origin_is_refused() {
        for rusak in ["bukan url", "", "ftp://ui.example.test", "https://"] {
            let m = h(&[("origin", rusak)]);
            assert_eq!(
                periksa("POST", &m, &asal()),
                Err(OriginDitolak::BukanAsalKami),
                "{rusak:?}"
            );
        }
    }

    #[test]
    fn sec_fetch_site_same_origin_passes() {
        let m = h(&[
            ("origin", "https://ui.example.test"),
            ("sec-fetch-site", "same-origin"),
        ]);
        assert_eq!(periksa("POST", &m, &asal()), Ok(()));
    }

    #[test]
    fn sec_fetch_site_cross_site_is_refused_even_with_a_good_origin() {
        // Defense in depth: a browser that says cross-site is not trusted, even
        // if the Origin looks right (which should be impossible, hence the
        // belt-and-braces).
        let m = h(&[
            ("origin", "https://ui.example.test"),
            ("sec-fetch-site", "cross-site"),
        ]);
        assert_eq!(periksa("POST", &m, &asal()), Err(OriginDitolak::FetchSite));
    }

    #[test]
    fn sec_fetch_site_same_site_is_refused() {
        // A page on a sibling subdomain would send Origin of that subdomain;
        // this catches it with the header even before the Origin comparison.
        let m = h(&[
            ("origin", "https://admin.ui.example.test"),
            ("sec-fetch-site", "same-site"),
        ]);
        assert_eq!(periksa("POST", &m, &asal()), Err(OriginDitolak::FetchSite));
    }

    #[test]
    fn sec_fetch_site_none_is_allowed() {
        // A direct navigation (link click, address bar) has no initiator.
        let m = h(&[
            ("origin", "https://ui.example.test"),
            ("sec-fetch-site", "none"),
        ]);
        assert_eq!(periksa("POST", &m, &asal()), Ok(()));
    }

    #[test]
    fn absent_sec_fetch_site_falls_back_to_origin() {
        let m = h(&[("origin", "https://ui.example.test")]);
        assert_eq!(periksa("POST", &m, &asal()), Ok(()));
    }

    #[test]
    fn a_malformed_sec_fetch_site_fails_closed() {
        let m = h(&[
            ("origin", "https://ui.example.test"),
            ("sec-fetch-site", "bukan-nilai"),
        ]);
        assert_eq!(periksa("POST", &m, &asal()), Err(OriginDitolak::FetchSite));
    }

    #[test]
    fn configuration_rejects_a_path_or_query() {
        for salah in [
            "https://ui.example.test/admin",
            "https://ui.example.test/?x=1",
            "https://ui.example.test/#frag",
            "https://user:pass@ui.example.test",
        ] {
            assert_eq!(
                AsalUi::baru(salah).unwrap_err(),
                AsalTidakValid::BukanAsal,
                "{salah}"
            );
        }
    }

    #[test]
    fn configuration_rejects_a_non_http_scheme() {
        assert_eq!(
            AsalUi::baru("ftp://ui.example.test").unwrap_err(),
            AsalTidakValid::Skema
        );
        assert_eq!(
            AsalUi::baru("file:///tmp/ui").unwrap_err(),
            AsalTidakValid::Skema
        );
        assert_eq!(
            AsalUi::baru("bukan url").unwrap_err(),
            AsalTidakValid::BukanUrl
        );
    }

    #[test]
    fn cookie_host_prefix_requires_https_or_localhost() {
        assert!(asal().aman_untuk_cookie_host());
        assert!(
            AsalUi::baru("http://localhost:5173")
                .unwrap()
                .aman_untuk_cookie_host()
        );
        assert!(
            AsalUi::baru("http://127.0.0.1:5173")
                .unwrap()
                .aman_untuk_cookie_host()
        );
        // A plain-http deployment cannot set a __Host- cookie: flagged so the
        // caller fails startup instead of silently dropping the cookie.
        assert!(
            !AsalUi::baru("http://ui.example.test")
                .unwrap()
                .aman_untuk_cookie_host()
        );
    }

    #[test]
    fn header_helper_matches_as_str() {
        let hv = header_asal(&asal());
        assert_eq!(hv.to_str().unwrap(), "https://ui.example.test");
    }

    #[test]
    fn display_is_the_normalised_origin() {
        assert_eq!(asal().to_string(), "https://ui.example.test");
    }
}
