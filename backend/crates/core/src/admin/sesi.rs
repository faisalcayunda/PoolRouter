//! Session time rules (ADR-0003 rev 5 section 3.6, §3.6f; NFR-09).
//!
//! Two limits apply at once, and they are not the same kind of thing:
//!
//! - **idle, 30 minutes**: measured from the last activity. Every authenticated
//!   admin request pushes it forward. `perpanjangSesi` is the explicit version
//!   of that push, added for WCAG 2.2.1 so a user who is still working is
//!   warned and can extend instead of being logged out mid-task.
//! - **absolut, 8 hours**: measured from login. **Never** moves — not on
//!   activity, not on `perpanjangSesi`. WCAG 2.2.1 asks for a way to extend the
//!   timeout without losing data; the approved deviation for the absolute cap is
//!   reviewed in F1 with BGN (W-4), and the whole point of the cap is that
//!   activity cannot defeat it.
//!
//! This module is pure arithmetic on `DateTime<Utc>`: the database has the
//! columns, but no trigger enforces them, so the rule has to be a tested
//! function rather than a sentence in an ADR. Keeping it free of I/O also lets
//! the boundary cases (exactly at the limit) be tested without a clock.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

/// Idle limit: 30 minutes (NFR-09).
pub const IDLE_DETIK: i64 = 30 * 60;

/// Absolute limit: 8 hours from login (NFR-09). Not extendable in F0.
pub const ABSOLUT_DETIK: i64 = 8 * 60 * 60;

/// The two limits as one value, so a caller cannot accidentally use one without
/// the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatasSesi {
    pub idle_detik: i64,
    pub absolut_detik: i64,
}

impl BatasSesi {
    /// The F0 values from NFR-09.
    pub const fn f0() -> Self {
        Self {
            idle_detik: IDLE_DETIK,
            absolut_detik: ABSOLUT_DETIK,
        }
    }
}

impl Default for BatasSesi {
    fn default() -> Self {
        Self::f0()
    }
}

/// A stored session, reduced to the five instants the rules need.
///
/// `dibuat_at` is the login time and is what the absolute limit hangs on;
/// `terakhir_aktif_at` is the last activity and feeds the idle limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeadaanSesi {
    pub dibuat_at: DateTime<Utc>,
    pub terakhir_aktif_at: DateTime<Utc>,
}

impl KeadaanSesi {
    pub const fn baru(dibuat_at: DateTime<Utc>, terakhir_aktif_at: DateTime<Utc>) -> Self {
        Self {
            dibuat_at,
            terakhir_aktif_at,
        }
    }
}

/// Why a session is no longer usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SesiKedaluwarsa {
    /// Idle limit passed. Recoverable by logging in again, not by extending.
    Idle,
    /// Absolute limit passed. Not recoverable; in F0 there is no way to push it.
    Absolut,
}

/// What a request did to the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EfekPermintaan {
    /// Request accepted, idle pushed to `now`.
    DiperpanjangIdle { terakhir_aktif_at: DateTime<Utc> },
    /// Request accepted; idle already at or past the new value (a burst of
    /// requests inside the same second), so nothing was written.
    TidakBerubah,
    /// Request rejected: log in again.
    Ditolak(SesiKedaluwarsa),
}

impl EfekPermintaan {
    /// True when the caller must issue a redirect to login (or a 401).
    pub const fn ditolak(&self) -> bool {
        matches!(self, Self::Ditolak(_))
    }
}

/// Evaluate one authenticated request against both limits.
///
/// The absolute limit is checked first: once it is passed, the idle limit is
/// irrelevant and reporting `Idle` would suggest the user can extend their way
/// back in.
pub fn evaluasi(keadaan: KeadaanSesi, sekarang: DateTime<Utc>, batas: BatasSesi) -> EfekPermintaan {
    if sekarang >= keadaan.dibuat_at + Duration::seconds(batas.absolut_detik) {
        return EfekPermintaan::Ditolak(SesiKedaluwarsa::Absolut);
    }
    if sekarang >= keadaan.terakhir_aktif_at + Duration::seconds(batas.idle_detik) {
        return EfekPermintaan::Ditolak(SesiKedaluwarsa::Idle);
    }
    // Push idle to now, but never past the absolute limit: storing an idle
    // deadline beyond it would make `/auth/sesi` promise a session longer than
    // the cap allows.
    let absolut = keadaan.dibuat_at + Duration::seconds(batas.absolut_detik);
    let baru = sekarang.min(absolut);
    if baru > keadaan.terakhir_aktif_at {
        EfekPermintaan::DiperpanjangIdle {
            terakhir_aktif_at: baru,
        }
    } else {
        EfekPermintaan::TidakBerubah
    }
}

/// The result of the explicit `POST /admin/v1/auth/sesi/perpanjang`.
///
/// Kept separate from [`evaluasi`] because the ADR gives the extension one extra
/// promise: it resets the idle timer **and nothing else**. A function that
/// cannot return a new `dibuat_at` makes "the absolute cap is never moved" a
/// property of the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HasilPerpanjang {
    /// Idle moved to `terakhir_aktif_at`; absolute unchanged.
    Diperpanjang {
        terakhir_aktif_at: DateTime<Utc>,
        absolut_berakhir_pada: DateTime<Utc>,
    },
    /// Session already past a limit; the caller answers 401.
    Ditolak(SesiKedaluwarsa),
}

/// `perpanjangSesi` (ADR-0003 §3.6f). Extends idle only.
pub fn perpanjang(
    keadaan: KeadaanSesi,
    sekarang: DateTime<Utc>,
    batas: BatasSesi,
) -> HasilPerpanjang {
    let absolut_berakhir_pada = keadaan.dibuat_at + Duration::seconds(batas.absolut_detik);
    match evaluasi(keadaan, sekarang, batas) {
        EfekPermintaan::Ditolak(alasan) => HasilPerpanjang::Ditolak(alasan),
        // Both "pushed" and "already equal" extend to `now` from the caller's
        // point of view: the endpoint always reports the same shape, and the
        // UPDATE only runs for the first case.
        EfekPermintaan::DiperpanjangIdle { .. } | EfekPermintaan::TidakBerubah => {
            HasilPerpanjang::Diperpanjang {
                terakhir_aktif_at: sekarang.min(absolut_berakhir_pada),
                absolut_berakhir_pada,
            }
        }
    }
}

/// `GET /admin/v1/auth/sesi` body: `SesiStatus` in admin-api.yaml.
///
/// All three fields are always present, and the two deadlines are reported even
/// when they are in the past — a 401 has no body, so the client reads the
/// status *before* it expires to arm the WCAG 2.2.1 warning dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StatusSesi {
    pub idle_berakhir_pada: DateTime<Utc>,
    pub absolut_berakhir_pada: DateTime<Utc>,
    pub waktu_server: DateTime<Utc>,
}

impl StatusSesi {
    /// Build the body. `waktu_server` is the same `now` used for the arithmetic,
    /// so the client's countdown does not inherit clock skew from a second
    /// `Utc::now()` call.
    pub fn hitung(keadaan: KeadaanSesi, sekarang: DateTime<Utc>, batas: BatasSesi) -> Self {
        let absolut_berakhir_pada = keadaan.dibuat_at + Duration::seconds(batas.absolut_detik);
        Self {
            // Near the cap the idle deadline is the cap itself (ADR-0003 rev 5
            // §3.6f): the UI must never be promised more than the session has.
            idle_berakhir_pada: (keadaan.terakhir_aktif_at + Duration::seconds(batas.idle_detik))
                .min(absolut_berakhir_pada),
            absolut_berakhir_pada,
            waktu_server: sekarang,
        }
    }

    /// Seconds until the idle deadline, floored at zero. The UI warning fires on
    /// this, so a negative value must not reach it.
    pub fn sisa_idle_detik(&self) -> i64 {
        (self.idle_berakhir_pada - self.waktu_server)
            .num_seconds()
            .max(0)
    }

    /// Seconds until the absolute deadline, floored at zero.
    pub fn sisa_absolut_detik(&self) -> i64 {
        (self.absolut_berakhir_pada - self.waktu_server)
            .num_seconds()
            .max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn t(detik: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 3, 8, 0, 0).unwrap() + Duration::seconds(detik)
    }

    fn sesi(dibuat: i64, aktif: i64) -> KeadaanSesi {
        KeadaanSesi::baru(t(dibuat), t(aktif))
    }

    #[test]
    fn f0_limits_are_the_nfr09_values() {
        assert_eq!(BatasSesi::f0().idle_detik, 1800);
        assert_eq!(BatasSesi::f0().absolut_detik, 28_800);
        assert_eq!(BatasSesi::default(), BatasSesi::f0());
    }

    #[test]
    fn an_active_request_pushes_idle_to_now() {
        let s = sesi(0, 600);
        assert_eq!(
            evaluasi(s, t(900), BatasSesi::f0()),
            EfekPermintaan::DiperpanjangIdle {
                terakhir_aktif_at: t(900)
            }
        );
    }

    #[test]
    fn idle_expires_exactly_at_thirty_minutes() {
        let s = sesi(0, 0);
        // One second before: still fine.
        assert!(!evaluasi(s, t(IDLE_DETIK - 1), BatasSesi::f0()).ditolak());
        // Exactly at the limit: expired. The boundary is `>=` so a session can
        // never be honoured at the instant its deadline has passed.
        assert_eq!(
            evaluasi(s, t(IDLE_DETIK), BatasSesi::f0()),
            EfekPermintaan::Ditolak(SesiKedaluwarsa::Idle)
        );
        assert_eq!(
            evaluasi(s, t(IDLE_DETIK + 1), BatasSesi::f0()),
            EfekPermintaan::Ditolak(SesiKedaluwarsa::Idle)
        );
    }

    #[test]
    fn activity_never_moves_the_absolute_limit() {
        let batas = BatasSesi::f0();
        // A session that is active right up to the cap, one request every 10
        // minutes. Every request strictly before the cap is honoured ...
        let mut aktif = 0;
        while aktif < ABSOLUT_DETIK {
            let efek = evaluasi(sesi(0, aktif.saturating_sub(600)), t(aktif), batas);
            assert!(!efek.ditolak(), "menit ke-{}", aktif / 60);
            aktif += 600;
        }
        // ... and the request at exactly 8 hours from login is refused, no
        // matter how recently the session was used.
        assert_eq!(
            evaluasi(sesi(0, ABSOLUT_DETIK - 1), t(ABSOLUT_DETIK), batas),
            EfekPermintaan::Ditolak(SesiKedaluwarsa::Absolut)
        );
    }

    #[test]
    fn absolute_is_reported_ahead_of_idle() {
        // Both passed: the reason is `Absolut`, because telling the user "idle"
        // would imply extending can help when it cannot.
        let s = sesi(0, 0);
        assert_eq!(
            evaluasi(s, t(ABSOLUT_DETIK + 600), BatasSesi::f0()),
            EfekPermintaan::Ditolak(SesiKedaluwarsa::Absolut)
        );
    }

    #[test]
    fn a_burst_inside_one_second_writes_nothing() {
        // Same instant twice: the second request must not issue an UPDATE that
        // sets the column to the value it already holds.
        let s = sesi(0, 600);
        assert_eq!(
            evaluasi(s, t(600), BatasSesi::f0()),
            EfekPermintaan::TidakBerubah
        );
    }

    #[test]
    fn idle_is_never_pushed_past_the_absolute_limit() {
        // A request one second before the cap must not be recorded as "active
        // at now", which would make the reported idle deadline (now + 30 min)
        // land half an hour after the session must die.
        let batas = BatasSesi::f0();
        let s = sesi(0, ABSOLUT_DETIK - 601);
        match evaluasi(s, t(ABSOLUT_DETIK - 1), batas) {
            EfekPermintaan::DiperpanjangIdle { terakhir_aktif_at } => {
                assert_eq!(terakhir_aktif_at, t(ABSOLUT_DETIK - 1));
                let status = StatusSesi::hitung(
                    KeadaanSesi::baru(s.dibuat_at, terakhir_aktif_at),
                    t(ABSOLUT_DETIK - 1),
                    batas,
                );
                // The cap is reported separately and the session dies there:
                // extending never buys time past the 8h instant.
                assert_eq!(status.absolut_berakhir_pada, t(ABSOLUT_DETIK));
                assert_eq!(status.sisa_absolut_detik(), 1);
            }
            lain => panic!("harusnya diperpanjang, dapat {lain:?}"),
        }
    }

    #[test]
    fn perpanjang_extends_idle_and_leaves_absolute_alone() {
        let batas = BatasSesi::f0();
        let s = sesi(0, 1500);
        let hasil = perpanjang(s, t(1700), batas);
        assert_eq!(
            hasil,
            HasilPerpanjang::Diperpanjang {
                terakhir_aktif_at: t(1700),
                absolut_berakhir_pada: t(ABSOLUT_DETIK),
            }
        );
    }

    #[test]
    fn perpanjang_does_not_resurrect_an_expired_idle_session() {
        // Idle expired -> 401. Extending is for a session still alive.
        let s = sesi(0, 0);
        assert_eq!(
            perpanjang(s, t(IDLE_DETIK), BatasSesi::f0()),
            HasilPerpanjang::Ditolak(SesiKedaluwarsa::Idle)
        );
    }

    #[test]
    fn perpanjang_cannot_pass_the_absolute_cap() {
        let batas = BatasSesi::f0();
        let s = sesi(0, ABSOLUT_DETIK - 100);
        assert_eq!(
            perpanjang(s, t(ABSOLUT_DETIK), batas),
            HasilPerpanjang::Ditolak(SesiKedaluwarsa::Absolut)
        );
    }

    #[test]
    fn repeated_extension_still_hits_the_cap() {
        // The WCAG 2.2.1 loop someone would actually try: extend every 25
        // minutes, well inside the 30-minute idle window. The session must still
        // die at 8 hours.
        let batas = BatasSesi::f0();
        let langkah = 25 * 60;
        let mut aktif = 0;
        // Every extension before the cap succeeds ...
        while aktif + langkah < ABSOLUT_DETIK {
            aktif += langkah;
            assert!(
                !matches!(
                    perpanjang(sesi(0, aktif - langkah), t(aktif), batas),
                    HasilPerpanjang::Ditolak(_)
                ),
                "ekstensi menit ke-{} seharusnya boleh",
                aktif / 60
            );
        }
        // ... and the first one at or past the cap is refused.
        let di_cap = ABSOLUT_DETIK;
        assert_eq!(
            perpanjang(sesi(0, di_cap - langkah), t(di_cap), batas),
            HasilPerpanjang::Ditolak(SesiKedaluwarsa::Absolut)
        );
    }

    #[test]
    fn idle_deadline_is_clamped_to_the_absolute_cap() {
        let batas = BatasSesi::f0();
        let hampir = ABSOLUT_DETIK - 600;
        let status = StatusSesi::hitung(sesi(0, hampir), t(hampir), batas);
        assert_eq!(status.idle_berakhir_pada, t(ABSOLUT_DETIK));
        assert_eq!(status.idle_berakhir_pada, status.absolut_berakhir_pada);
    }

    #[test]
    fn status_reports_both_deadlines_from_one_clock() {
        let batas = BatasSesi::f0();
        let status = StatusSesi::hitung(sesi(0, 600), t(1200), batas);
        assert_eq!(status.idle_berakhir_pada, t(600 + IDLE_DETIK));
        assert_eq!(status.absolut_berakhir_pada, t(ABSOLUT_DETIK));
        assert_eq!(status.waktu_server, t(1200));
        assert_eq!(status.sisa_idle_detik(), 1200);
        assert_eq!(status.sisa_absolut_detik(), ABSOLUT_DETIK - 1200);
    }

    #[test]
    fn remaining_seconds_never_go_negative() {
        let batas = BatasSesi::f0();
        let status = StatusSesi::hitung(sesi(0, 0), t(ABSOLUT_DETIK + 10), batas);
        assert_eq!(status.sisa_idle_detik(), 0);
        assert_eq!(status.sisa_absolut_detik(), 0);
    }

    #[test]
    fn serialised_status_uses_the_contract_field_names() {
        // The Web UI reads these names straight from admin-api.yaml; a rename
        // here would break the dialog without failing any type check on the
        // TypeScript side.
        let status = StatusSesi::hitung(sesi(0, 0), t(0), BatasSesi::f0());
        let json = serde_json::to_value(status).unwrap();
        let obj = json.as_object().unwrap();
        let mut kunci: Vec<_> = obj.keys().cloned().collect();
        kunci.sort();
        assert_eq!(
            kunci,
            [
                "absolut_berakhir_pada",
                "idle_berakhir_pada",
                "waktu_server"
            ]
        );
        // Whole-second RFC 3339, the format the examples in the spec use.
        assert_eq!(obj["waktu_server"], "2026-11-03T08:00:00Z");
    }

    #[test]
    fn a_shorter_batas_is_honoured() {
        // The helper takes the limits as a value, so F1 can change them without
        // this module changing.
        let pendek = BatasSesi {
            idle_detik: 60,
            absolut_detik: 120,
        };
        assert_eq!(
            evaluasi(sesi(0, 0), t(60), pendek),
            EfekPermintaan::Ditolak(SesiKedaluwarsa::Idle)
        );
        assert_eq!(
            evaluasi(sesi(0, 90), t(120), pendek),
            EfekPermintaan::Ditolak(SesiKedaluwarsa::Absolut)
        );
    }
}
