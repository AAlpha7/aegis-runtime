//! Re-alert gating for periodic monitors.
//!
//! Every background monitor in this crate used to alert on *every* tick while
//! a condition persisted (hourly, every 10 min, every 30 min ...). `AlertGate`
//! turns that into: alert once when a failure streak reaches `threshold`, then
//! stay quiet for `min_renag`, tripling the quiet period on each re-nag up to
//! `max_renag`. A success clears the streak so the next edge alerts again.
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct AlertGate {
    min_renag: Duration,
    max_renag: Duration,
    entries: HashMap<String, Entry>,
}

struct Entry {
    consecutive: u32,
    last_alert: Option<Instant>,
    renag: Duration,
}

impl AlertGate {
    pub fn new(min_renag: Duration, max_renag: Duration) -> Self {
        Self {
            min_renag,
            max_renag: max_renag.max(min_renag),
            entries: HashMap::new(),
        }
    }

    /// Record one failed check for `key`; returns whether to alert now.
    pub fn record_failure(&mut self, key: &str, threshold: u32, now: Instant) -> bool {
        let min_renag = self.min_renag;
        let max_renag = self.max_renag;
        let e = self.entries.entry(key.to_string()).or_insert(Entry {
            consecutive: 0,
            last_alert: None,
            renag: min_renag,
        });
        e.consecutive += 1;
        if e.consecutive < threshold.max(1) {
            return false;
        }
        match e.last_alert {
            None => {
                e.last_alert = Some(now);
                e.renag = min_renag;
                true
            }
            Some(last) if now.saturating_duration_since(last) >= e.renag => {
                e.last_alert = Some(now);
                e.renag = (e.renag * 3).min(max_renag);
                true
            }
            Some(_) => false,
        }
    }

    /// Record a passing check for `key`: clears the streak and re-arms the edge.
    pub fn record_success(&mut self, key: &str) {
        self.entries.remove(key);
    }

    pub fn consecutive(&self, key: &str) -> u32 {
        self.entries.get(key).map(|e| e.consecutive).unwrap_or(0)
    }

    /// Keys currently in a failure streak.
    pub fn failing_keys(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const H: Duration = Duration::from_secs(3600);

    #[test]
    fn alerts_once_threshold_reached_and_not_before() {
        let mut g = AlertGate::new(H, 24 * H);
        let t0 = Instant::now();
        assert!(!g.record_failure("k", 3, t0));
        assert!(!g.record_failure("k", 3, t0));
        assert!(
            g.record_failure("k", 3, t0),
            "third consecutive failure alerts"
        );
        assert_eq!(g.consecutive("k"), 3);
    }

    #[test]
    fn repeat_failures_inside_renag_window_are_silent() {
        let mut g = AlertGate::new(H, 24 * H);
        let t0 = Instant::now();
        assert!(g.record_failure("k", 1, t0));
        assert!(!g.record_failure("k", 1, t0 + Duration::from_secs(600)));
        assert!(!g.record_failure("k", 1, t0 + Duration::from_secs(3000)));
    }

    #[test]
    fn renag_interval_triples_up_to_max() {
        let mut g = AlertGate::new(H, 10 * H);
        let t0 = Instant::now();
        assert!(g.record_failure("k", 1, t0));
        assert!(
            g.record_failure("k", 1, t0 + H),
            "re-nag after min interval"
        );
        assert!(
            !g.record_failure("k", 1, t0 + 3 * H),
            "next interval is 3h, not 2h"
        );
        assert!(g.record_failure("k", 1, t0 + 4 * H));
        assert!(
            !g.record_failure("k", 1, t0 + 12 * H),
            "9h interval not yet elapsed"
        );
        assert!(g.record_failure("k", 1, t0 + 13 * H));
        assert!(g.record_failure("k", 1, t0 + 23 * H), "capped at max 10h");
    }

    #[test]
    fn success_resets_so_next_streak_alerts_again() {
        let mut g = AlertGate::new(H, 24 * H);
        let t0 = Instant::now();
        assert!(g.record_failure("k", 1, t0));
        g.record_success("k");
        assert_eq!(g.consecutive("k"), 0);
        assert!(
            g.record_failure("k", 1, t0 + Duration::from_secs(1)),
            "edge after recovery alerts immediately"
        );
    }
}
