//! Keyed GCRA throttle (ADR-0001 section 5a.5): a rate limit, not a lockout.
//!
//! State is per process. Deployments with several webserver replicas get an
//! effective limit of `replicas x rate`; moving the state to Redis is a
//! separate decision (reported in SHAA-198).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Upper bound on tracked keys. Idle keys (theoretical arrival time already in
/// the past) are dropped first, which never changes a decision. When every key
/// is still live the map stays full and a new key is refused (fail closed):
/// the key space is attacker-controlled (unauthenticated `client_id`), so
/// growth must be bounded and the O(n) sweep must not run per request.
const MAX_KEYS: usize = 100_000;

#[derive(Debug)]
pub struct Gcra {
    /// Emission interval T = 1 / rate.
    interval: Duration,
    /// Burst tolerance tau = (burst - 1) * T.
    tolerance: Duration,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    tat: HashMap<String, Instant>,
    /// When the map was last swept; a sweep runs at most once per interval.
    last_sweep: Option<Instant>,
    /// Number of sweeps so far (observed by tests).
    sweeps: u64,
}

impl Gcra {
    /// `per_minute` sustained requests per minute, `burst` immediate requests.
    /// Both must be at least 1.
    pub fn per_minute(per_minute: u32, burst: u32) -> Self {
        assert!(per_minute >= 1 && burst >= 1, "rate and burst must be >= 1");
        let interval = Duration::from_secs(60) / per_minute;
        Self {
            interval,
            tolerance: interval * (burst - 1),
            state: Mutex::new(State::default()),
        }
    }

    /// Admit one request for `key` at `now`. On refusal returns the wait until
    /// the next request would be admitted.
    pub fn check_at(&self, key: &str, now: Instant) -> Result<(), Duration> {
        // A poisoned lock only means another thread panicked mid-update; the map
        // stays structurally valid, so keep serving.
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.tat.len() >= MAX_KEYS && !st.tat.contains_key(key) {
            let due = st
                .last_sweep
                .is_none_or(|t| now.saturating_duration_since(t) >= self.interval);
            if due {
                st.tat.retain(|_, tat| *tat > now);
                st.last_sweep = Some(now);
                st.sweeps += 1;
            }
            if st.tat.len() >= MAX_KEYS {
                // Every tracked key is live: refuse the newcomer instead of growing.
                return Err(self.interval);
            }
        }
        let tat = st.tat.get(key).copied().map_or(now, |t| t.max(now));
        let ahead = tat - now;
        if ahead > self.tolerance {
            return Err(ahead - self.tolerance);
        }
        st.tat.insert(key.to_owned(), tat + self.interval);
        Ok(())
    }

    pub fn check(&self, key: &str) -> Result<(), Duration> {
        self.check_at(key, Instant::now())
    }
}

/// `Retry-After` value: whole seconds, rounded up, at least 1 (ADR-0001 5a.7).
pub fn retry_after_secs(wait: Duration) -> u64 {
    let secs = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    secs.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_refuse_then_recover() {
        let g = Gcra::per_minute(60, 10); // T = 1 s
        let t0 = Instant::now();
        for i in 0..10 {
            assert!(g.check_at("c", t0).is_ok(), "request {i} inside burst");
        }
        let wait = g.check_at("c", t0).unwrap_err();
        assert_eq!(wait, Duration::from_secs(1));
        assert!(g.check_at("c", t0 + Duration::from_millis(999)).is_err());
        assert!(g.check_at("c", t0 + Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn keys_are_independent() {
        let g = Gcra::per_minute(60, 1);
        let t0 = Instant::now();
        assert!(g.check_at("a", t0).is_ok());
        assert!(g.check_at("a", t0).is_err());
        assert!(g.check_at("b", t0).is_ok());
    }

    #[test]
    fn refusal_does_not_push_the_window() {
        let g = Gcra::per_minute(60, 1);
        let t0 = Instant::now();
        assert!(g.check_at("a", t0).is_ok());
        for _ in 0..50 {
            assert!(g.check_at("a", t0).is_err());
        }
        assert!(g.check_at("a", t0 + Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn sustained_rate_matches_spec_values() {
        // Per IP: 120/min => T = 500 ms.
        let g = Gcra::per_minute(120, 1);
        let t0 = Instant::now();
        assert!(g.check_at("ip", t0).is_ok());
        assert_eq!(
            g.check_at("ip", t0).unwrap_err(),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn retry_after_rounds_up_with_floor_of_one() {
        assert_eq!(retry_after_secs(Duration::from_millis(1)), 1);
        assert_eq!(retry_after_secs(Duration::ZERO), 1);
        assert_eq!(retry_after_secs(Duration::from_millis(1001)), 2);
        assert_eq!(retry_after_secs(Duration::from_secs(3)), 3);
    }

    #[test]
    fn idle_keys_are_evicted_at_capacity() {
        let g = Gcra::per_minute(60, 1);
        let t0 = Instant::now();
        for i in 0..MAX_KEYS {
            g.check_at(&format!("k{i}"), t0).unwrap();
        }
        let later = t0 + Duration::from_secs(5);
        g.check_at("new", later).unwrap();
        assert!(g.state.lock().unwrap().tat.len() < MAX_KEYS);
    }

    #[test]
    fn full_map_of_live_keys_refuses_newcomers_without_growing_or_resweeping() {
        let g = Gcra::per_minute(60, 1);
        let t0 = Instant::now();
        for i in 0..MAX_KEYS {
            g.check_at(&format!("k{i}"), t0).unwrap();
        }
        // Newcomers inside the same interval: all keys are still live.
        for i in 0..1_000 {
            let wait = g.check_at(&format!("new{i}"), t0).unwrap_err();
            assert_eq!(wait, Duration::from_secs(1));
        }
        let st = g.state.lock().unwrap();
        assert_eq!(st.tat.len(), MAX_KEYS, "map must not grow");
        assert!(
            st.sweeps <= 1,
            "sweep at most once per interval, got {}",
            st.sweeps
        );
        drop(st);
        // An existing key is still judged normally.
        assert!(g.check_at("k0", t0).is_err());
        // After the interval the keys are idle again, so a newcomer fits.
        assert!(g.check_at("new", t0 + Duration::from_secs(2)).is_ok());
        assert!(g.state.lock().unwrap().tat.len() < MAX_KEYS);
    }
}
