//! Hybrid Logical Clock (HLC) for total event ordering.
//!
//! Invariants:
//! - tick() is strictly monotonic: every call produces HlcTimestamp strictly
//!   greater than every prior return value, even if the OS clock regresses.
//! - (phys, log) comparison is lexicographic — `PartialOrd`/`Ord` on the
//!   struct field order is correct.
//! - log counter resets to 0 whenever phys advances.
//! - observe() incorporates a peer HLC without violating monotonicity.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// Hybrid Logical Clock timestamp. Ordering = lexicographic on (phys, log).
///
/// **Serialization note**: `phys` is stringified as a JSON string (not a
/// JSON number) because RFC 8785 JCS number formatting (ECMA-262
/// shortest-float via ryu_js) silently f64-coerces u64 values above the
/// 2^53 safe-integer range. Production nanosecond epoch timestamps
/// (~1.77×10^18 in 2026) are far past 2^53 (~9×10^15), so without
/// stringification two distinct sub-millisecond-adjacent events would
/// produce identical `event_hash` bytes — silently breaking tamper
/// detection. See tests `t4_hlc_phys_distinct_high_values_roundtrip`
/// and `p7_hlc_phys_distinct_u64_produce_distinct_event_hash` in
/// `tests.rs` for the regression gate.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HlcTimestamp {
    /// Physical component: nanoseconds since Unix epoch.
    /// Serialized as a JSON string (not number) — see struct docstring.
    #[serde(with = "u64_as_string")]
    pub phys: u64,
    /// Logical counter: increments when multiple events share a physical tick.
    /// `u32` is safely under 2^53, serialized as a plain JSON number.
    pub log: u32,
}

/// Serde helper: serialize `u64` as a JSON string (not a number) to avoid
/// JCS f64 coercion at values > 2^53. Deserialization tolerates either
/// string or number for backwards compatibility.
mod u64_as_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        // Accept either `"1234"` or `1234` — number variant is only safe for
        // values ≤ 2^53 but included for test-fixture ergonomics.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StringOrNumber {
            String(String),
            Number(u64),
        }
        match StringOrNumber::deserialize(d)? {
            StringOrNumber::String(s) => s.parse::<u64>().map_err(serde::de::Error::custom),
            StringOrNumber::Number(n) => Ok(n),
        }
    }
}

impl HlcTimestamp {
    pub const fn zero() -> Self {
        Self { phys: 0, log: 0 }
    }
}

/// HLC state machine. Thread-safe via internal non-poisoning Mutex
/// (parking_lot — matches repo convention per CLAUDE.md).
pub struct Hlc {
    state: Mutex<HlcTimestamp>,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl Hlc {
    /// Create with the real monotonic-OS clock.
    pub fn new() -> Self {
        Self::with_clock(Box::new(|| {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0)
        }))
    }

    /// Create with an injectable clock (used by tests to simulate regression).
    pub fn with_clock(clock: Box<dyn Fn() -> u64 + Send + Sync>) -> Self {
        Self {
            state: Mutex::new(HlcTimestamp::zero()),
            clock,
        }
    }

    /// Produce a new HLC timestamp strictly greater than every prior one.
    pub fn tick(&self) -> HlcTimestamp {
        let now = (self.clock)();
        let mut state = self.state.lock();
        let next = if now > state.phys {
            HlcTimestamp { phys: now, log: 0 }
        } else {
            HlcTimestamp {
                phys: state.phys,
                log: state.log.saturating_add(1),
            }
        };
        *state = next;
        next
    }

    /// Merge a peer's HLC into our state, producing a strictly-greater local
    /// timestamp. Used for replica sync (Phase A wires but does not exercise).
    pub fn observe(&self, peer: HlcTimestamp) -> HlcTimestamp {
        let now = (self.clock)();
        let mut state = self.state.lock();
        let max_phys = now.max(state.phys).max(peer.phys);
        let next = if max_phys == state.phys && max_phys == peer.phys {
            // Same phys on both sides — take max log and bump.
            HlcTimestamp {
                phys: max_phys,
                log: state.log.max(peer.log).saturating_add(1),
            }
        } else if max_phys == state.phys {
            // Our phys wins; bump local log.
            HlcTimestamp {
                phys: max_phys,
                log: state.log.saturating_add(1),
            }
        } else if max_phys == peer.phys {
            // Peer phys wins; adopt peer log + 1.
            HlcTimestamp {
                phys: max_phys,
                log: peer.log.saturating_add(1),
            }
        } else {
            // New phys (OS clock advanced past both); reset log.
            HlcTimestamp {
                phys: max_phys,
                log: 0,
            }
        };
        *state = next;
        next
    }

    /// Current tip (for recovery; does not tick).
    pub fn peek(&self) -> HlcTimestamp {
        *self.state.lock()
    }
}

impl Default for Hlc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Test helper: a fake clock that returns whatever the AtomicU64 holds.
    fn fake_clock(state: Arc<AtomicU64>) -> Box<dyn Fn() -> u64 + Send + Sync> {
        Box::new(move || state.load(Ordering::SeqCst))
    }

    #[test]
    fn t4_hlc_tick_advances_phys_when_clock_moves() {
        let clk = Arc::new(AtomicU64::new(1000));
        let hlc = Hlc::with_clock(fake_clock(clk.clone()));
        let t1 = hlc.tick();
        assert_eq!(t1, HlcTimestamp { phys: 1000, log: 0 });
        clk.store(2000, Ordering::SeqCst);
        let t2 = hlc.tick();
        assert_eq!(t2, HlcTimestamp { phys: 2000, log: 0 });
        assert!(t2 > t1);
    }

    #[test]
    fn t4_hlc_tick_bumps_log_when_clock_stalls() {
        let clk = Arc::new(AtomicU64::new(1000));
        let hlc = Hlc::with_clock(fake_clock(clk.clone()));
        let t1 = hlc.tick();
        let t2 = hlc.tick();
        let t3 = hlc.tick();
        assert_eq!(t1, HlcTimestamp { phys: 1000, log: 0 });
        assert_eq!(t2, HlcTimestamp { phys: 1000, log: 1 });
        assert_eq!(t3, HlcTimestamp { phys: 1000, log: 2 });
    }

    #[test]
    fn t4_hlc_tick_keeps_monotonic_under_clock_regression() {
        let clk = Arc::new(AtomicU64::new(5000));
        let hlc = Hlc::with_clock(fake_clock(clk.clone()));
        let t1 = hlc.tick();
        // OS clock regresses (NTP rewind).
        clk.store(100, Ordering::SeqCst);
        let t2 = hlc.tick();
        assert!(t2 > t1, "HLC must not regress under clock regression");
        assert_eq!(t2.phys, 5000);
        assert_eq!(t2.log, 1);
    }

    #[test]
    fn t4_hlc_zero_is_less_than_anything_positive() {
        assert!(HlcTimestamp::zero() < HlcTimestamp { phys: 1, log: 0 });
        assert!(HlcTimestamp::zero() < HlcTimestamp { phys: 0, log: 1 });
    }

    #[test]
    fn t4_hlc_ordering_is_lexicographic() {
        let a = HlcTimestamp { phys: 100, log: 5 };
        let b = HlcTimestamp { phys: 100, log: 6 };
        let c = HlcTimestamp { phys: 101, log: 0 };
        assert!(a < b);
        assert!(b < c);
        assert!(a < c);
    }

    #[test]
    fn t4_hlc_serde_json_shape_stringifies_phys() {
        let ts = HlcTimestamp {
            phys: 1729123456789,
            log: 3,
        };
        let json = serde_json::to_string(&ts).unwrap();
        // phys is a JSON string (not number) — see HlcTimestamp docstring.
        assert_eq!(json, r#"{"phys":"1729123456789","log":3}"#);
        let back: HlcTimestamp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ts);
    }

    #[test]
    fn t4_hlc_phys_distinct_high_values_roundtrip_exactly() {
        // Values past the 2^53 safe-float boundary MUST roundtrip losslessly.
        // Two sub-ms-distinct ns epochs in 2026 are the regression target.
        let a = HlcTimestamp {
            phys: 1_729_123_456_789_000_001,
            log: 0,
        };
        let b = HlcTimestamp {
            phys: 1_729_123_456_789_000_002,
            log: 0,
        };
        assert_ne!(a, b);
        let ja = serde_json::to_string(&a).unwrap();
        let jb = serde_json::to_string(&b).unwrap();
        assert_ne!(
            ja, jb,
            "distinct u64 phys values must serialize differently"
        );
        let back_a: HlcTimestamp = serde_json::from_str(&ja).unwrap();
        let back_b: HlcTimestamp = serde_json::from_str(&jb).unwrap();
        assert_eq!(back_a, a);
        assert_eq!(back_b, b);
    }

    #[test]
    fn t4_hlc_phys_deserializes_from_plain_number_for_back_compat() {
        // Backwards-compat: old test fixtures may write phys as a number.
        // Accept it on read (values in this test are ≤ 2^53 so lossless).
        let json = r#"{"phys":1000,"log":2}"#;
        let ts: HlcTimestamp = serde_json::from_str(json).unwrap();
        assert_eq!(ts, HlcTimestamp { phys: 1000, log: 2 });
    }

    #[test]
    fn t4_hlc_observe_adopts_peer_when_peer_is_ahead() {
        let clk = Arc::new(AtomicU64::new(1000));
        let hlc = Hlc::with_clock(fake_clock(clk.clone()));
        let _ = hlc.tick(); // local at phys=1000, log=0
        let peer = HlcTimestamp { phys: 9999, log: 7 };
        let t = hlc.observe(peer);
        assert_eq!(t.phys, 9999);
        assert_eq!(t.log, 8);
    }

    #[test]
    fn t4_hlc_observe_keeps_local_when_local_is_ahead() {
        let clk = Arc::new(AtomicU64::new(9000));
        let hlc = Hlc::with_clock(fake_clock(clk.clone()));
        let _ = hlc.tick(); // local at phys=9000, log=0
        let peer = HlcTimestamp { phys: 100, log: 0 };
        let t = hlc.observe(peer);
        assert_eq!(t.phys, 9000);
        assert_eq!(t.log, 1);
    }

    #[test]
    fn t4_hlc_observe_handles_tie_by_bumping_log() {
        let clk = Arc::new(AtomicU64::new(5000));
        let hlc = Hlc::with_clock(fake_clock(clk.clone()));
        let _ = hlc.tick(); // local at phys=5000, log=0
        let peer = HlcTimestamp { phys: 5000, log: 4 };
        let t = hlc.observe(peer);
        assert_eq!(t.phys, 5000);
        // max(local.log=0, peer.log=4) + 1 = 5
        assert_eq!(t.log, 5);
    }

    #[test]
    fn t4_hlc_peek_does_not_advance() {
        let clk = Arc::new(AtomicU64::new(1000));
        let hlc = Hlc::with_clock(fake_clock(clk.clone()));
        let t1 = hlc.tick();
        let p1 = hlc.peek();
        let p2 = hlc.peek();
        assert_eq!(t1, p1);
        assert_eq!(p1, p2);
    }

    #[test]
    fn t4_hlc_concurrent_ticks_are_unique() {
        use std::thread;
        let clk = Arc::new(AtomicU64::new(1000));
        let hlc = Arc::new(Hlc::with_clock(fake_clock(clk.clone())));
        let mut handles = vec![];
        for _ in 0..8 {
            let h = hlc.clone();
            handles.push(thread::spawn(move || {
                (0..100).map(|_| h.tick()).collect::<Vec<_>>()
            }));
        }
        let mut all: Vec<HlcTimestamp> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 800, "all HLC ticks must be unique");
    }

    #[test]
    fn t4_hlc_default_constructs_with_real_clock() {
        let hlc = Hlc::default();
        let t = hlc.tick();
        assert!(t.phys > 0, "real clock should produce nonzero phys");
    }
}
