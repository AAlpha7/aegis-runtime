//! Process-global Telegram connectivity state for `GET /health` and the
//! post-deploy smoke test.
//!
//! Incident 2026-09-15: the production bot token was revoked at 03:15 UTC.
//! The running process logged `Api(InvalidToken)` from teloxide's update
//! listener; after a restart `TelegramBot::start` caught `get_me()` failing
//! and retried every 60 s under a "DNS/network" WARN for hours while
//! `GET /health` stayed `{"status":"ok"}` (it only knew about the database,
//! HNSW, the memory actor and the skill count). Nobody was told for six hours.
//!
//! This module is the single place the bot reports its state to, and the
//! single place the health endpoint reads it from:
//!
//! * [`classify`] turns a teloxide error into [`TelegramState::InvalidToken`]
//!   (401 Unauthorized, i.e. the token itself is wrong) or
//!   [`TelegramState::Unreachable`] (DNS, TCP, TLS, 5xx, rate limit — anything
//!   a retry can fix), so the two are never logged or reported as one thing.
//! * [`record_state`] / [`record_reachable`] update a process-global snapshot
//!   (`std::sync::OnceLock` + `parking_lot::RwLock`, default
//!   [`TelegramState::Unconfigured`] until `start()` runs).
//! * [`snapshot`] is what `/health` serialises under `components.telegram`, and
//!   [`TelegramHealthSnapshot::is_degraded_at`] decides when the top-level
//!   status flips to `degraded`.

use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use teloxide::{ApiError, RequestError};

/// How long `TelegramBot::start` waits between `get_me()` attempts once the
/// token has been rejected. A 401 cannot fix itself without a new token (and
/// a restart, since teloxide reads `TELOXIDE_TOKEN` at construction), so
/// retrying every 60 s only floods the journal.
pub const INVALID_TOKEN_RETRY_SECS: u64 = 300;

/// Interval of the background `get_me()` probe spawned after a successful
/// startup, so a token revoked *while the process is running* becomes visible
/// on `/health` within five minutes.
pub const PROBE_INTERVAL_SECS: u64 = 300;

/// `Unreachable` shorter than this is a blip (DNS hiccup, Telegram 5xx) and
/// leaves `/health` at `ok`; longer than this it degrades the status.
pub const UNREACHABLE_DEGRADED_AFTER_SECS: i64 = 600;

/// Connectivity state of the Telegram long-polling bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelegramState {
    /// `TelegramBot::start` has not been called (no token, or polling
    /// disabled via `AEGIS_TELEGRAM_POLLING=0`).
    Unconfigured,
    /// `start()` is running its first `get_me()` round; no verdict yet.
    Starting,
    /// The last `get_me()` succeeded.
    Reachable,
    /// Telegram answered 401 Unauthorized: `TELOXIDE_TOKEN` is wrong or
    /// revoked. Retrying does not help; a human has to fix the token.
    InvalidToken,
    /// `get_me()` failed for a reason a retry can fix (DNS, TCP, TLS,
    /// Telegram 5xx, flood control).
    Unreachable,
}

impl TelegramState {
    /// States that count towards `consecutive_failures`.
    fn is_failure(self) -> bool {
        matches!(self, Self::InvalidToken | Self::Unreachable)
    }
}

/// What `/health` reports under `components.telegram`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramHealthSnapshot {
    pub state: TelegramState,
    /// Unix time (seconds) at which the current `state` was entered. Repeated
    /// failures in the same state keep the original timestamp, so
    /// "unreachable for N seconds" can be computed from it.
    pub since_unix: i64,
    /// Username from the last successful `get_me()`; kept across later
    /// failures so the operator sees which bot went away.
    pub bot_username: Option<String>,
    /// Display form of the error behind the current failure state; cleared
    /// on recovery.
    pub last_error: Option<String>,
    /// Number of `get_me()` failures since the last success (or start).
    pub consecutive_failures: u32,
}

impl TelegramHealthSnapshot {
    fn unconfigured(now_unix: i64) -> Self {
        Self {
            state: TelegramState::Unconfigured,
            since_unix: now_unix,
            bot_username: None,
            last_error: None,
            consecutive_failures: 0,
        }
    }

    /// Whether the health endpoint should report `degraded` because of the
    /// bot: an invalid token is always degraded (it never fixes itself), an
    /// unreachable API only after [`UNREACHABLE_DEGRADED_AFTER_SECS`].
    pub fn is_degraded_at(&self, now_unix: i64) -> bool {
        match self.state {
            TelegramState::InvalidToken => true,
            TelegramState::Unreachable => {
                now_unix.saturating_sub(self.since_unix) > UNREACHABLE_DEGRADED_AFTER_SECS
            }
            TelegramState::Unconfigured | TelegramState::Starting | TelegramState::Reachable => {
                false
            }
        }
    }
}

static SNAPSHOT: OnceLock<RwLock<TelegramHealthSnapshot>> = OnceLock::new();

fn global() -> &'static RwLock<TelegramHealthSnapshot> {
    SNAPSHOT.get_or_init(|| RwLock::new(TelegramHealthSnapshot::unconfigured(now_unix())))
}

/// Current Unix time in seconds (0 if the clock is before the epoch).
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Record a state observed by the bot. Returns the state that was current
/// before this call so callers can log transitions (ERROR on the first
/// `InvalidToken`, INFO on recovery) without logging every probe.
pub fn record_state(state: TelegramState, error: Option<String>) -> TelegramState {
    record_state_at(state, error, now_unix())
}

/// [`record_state`] with an explicit clock, for deterministic tests of the
/// "unreachable for more than N seconds" rule.
pub(crate) fn record_state_at(
    state: TelegramState,
    error: Option<String>,
    now_unix: i64,
) -> TelegramState {
    let mut snap = global().write();
    let previous = snap.state;
    if previous != state {
        snap.state = state;
        snap.since_unix = now_unix;
    }
    if state.is_failure() {
        snap.consecutive_failures = snap.consecutive_failures.saturating_add(1);
        snap.last_error = error;
    } else {
        snap.consecutive_failures = 0;
        snap.last_error = None;
    }
    previous
}

/// Record a successful `get_me()`. Returns the previous state (see
/// [`record_state`]).
pub fn record_reachable(username: impl Into<String>) -> TelegramState {
    record_reachable_at(username.into(), now_unix())
}

fn record_reachable_at(username: String, now_unix: i64) -> TelegramState {
    let mut snap = global().write();
    let previous = snap.state;
    if previous != TelegramState::Reachable {
        snap.state = TelegramState::Reachable;
        snap.since_unix = now_unix;
    }
    snap.bot_username = Some(username);
    snap.last_error = None;
    snap.consecutive_failures = 0;
    previous
}

/// A copy of the current process-global snapshot.
pub fn snapshot() -> TelegramHealthSnapshot {
    global().read().clone()
}

/// Map a teloxide error to the state it proves.
///
/// * `Api(InvalidToken)` is teloxide's parse of Telegram's 401 `Unauthorized`
///   (and 404 `Not Found`, which Telegram returns for a malformed token).
/// * `Api(Unknown(text))` is any description teloxide did not recognise; an
///   `Unauthorized` / `Invalid bot token` text in there is still a token
///   problem.
/// * Everything else (network, JSON, I/O, flood control, other API errors)
///   is something a retry can fix.
pub fn classify(err: &RequestError) -> TelegramState {
    match err {
        RequestError::Api(ApiError::InvalidToken) => TelegramState::InvalidToken,
        RequestError::Api(ApiError::Unknown(text)) if looks_like_token_rejection(text) => {
            TelegramState::InvalidToken
        }
        _ => TelegramState::Unreachable,
    }
}

fn looks_like_token_rejection(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("unauthorized") || lower.contains("invalid bot token")
}

/// Delay before the next startup `get_me()` attempt: a rejected token waits
/// [`INVALID_TOKEN_RETRY_SECS`] regardless of the network backoff; a network
/// failure keeps the caller's exponential backoff.
pub fn retry_delay(state: TelegramState, network_backoff: Duration) -> Duration {
    match state {
        TelegramState::InvalidToken => Duration::from_secs(INVALID_TOKEN_RETRY_SECS),
        _ => network_backoff,
    }
}

/// Serialises every test that touches the process-global snapshot (cargo runs
/// unit tests of one binary in parallel threads) and resets it to
/// `Unconfigured` both when acquired and when dropped, so an assertion
/// failure in one test cannot leak state into another.
#[cfg(test)]
pub(crate) struct TestGuard {
    _lock: parking_lot::MutexGuard<'static, ()>,
}

#[cfg(test)]
static TEST_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[cfg(test)]
pub(crate) fn test_guard() -> TestGuard {
    let lock = TEST_LOCK.lock();
    reset_for_tests();
    TestGuard { _lock: lock }
}

#[cfg(test)]
impl Drop for TestGuard {
    fn drop(&mut self) {
        reset_for_tests();
    }
}

#[cfg(test)]
fn reset_for_tests() {
    *global().write() = TelegramHealthSnapshot::unconfigured(now_unix());
}

/// Backdate the current state's `since_unix` (tests of the "unreachable for
/// more than N seconds" rule cannot wait ten real minutes). Callers hold a
/// [`TestGuard`].
#[cfg(test)]
pub(crate) fn set_since_unix_for_tests(since_unix: i64) {
    global().write().since_unix = since_unix;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid_token() -> RequestError {
        RequestError::Api(ApiError::InvalidToken)
    }

    fn unknown(text: &str) -> RequestError {
        RequestError::Api(ApiError::Unknown(text.to_string()))
    }

    /// A real `RequestError::Network` without touching the network: reqwest
    /// stores an invalid-URL builder error and surfaces it from `build()`.
    /// The client comes from teloxide's own reqwest (a different major
    /// version than the core crate's), which is the one `From` accepts.
    fn network_error() -> RequestError {
        let client = teloxide::net::default_reqwest_settings()
            .build()
            .expect("teloxide reqwest client");
        let err = client
            .get("http://[::1")
            .build()
            .expect_err("an unparsable URL must fail to build");
        RequestError::from(err)
    }

    #[test]
    fn classify_maps_teloxide_invalid_token_to_invalid_token() {
        assert_eq!(classify(&invalid_token()), TelegramState::InvalidToken);
    }

    #[test]
    fn classify_maps_unknown_unauthorized_text_to_invalid_token() {
        assert_eq!(
            classify(&unknown("Unauthorized")),
            TelegramState::InvalidToken
        );
        // Case-insensitive, and the alternate wording Telegram uses.
        assert_eq!(
            classify(&unknown("UNAUTHORIZED: token revoked")),
            TelegramState::InvalidToken
        );
        assert_eq!(
            classify(&unknown("invalid BOT token")),
            TelegramState::InvalidToken
        );
    }

    #[test]
    fn classify_maps_network_and_other_errors_to_unreachable() {
        let net = network_error();
        assert!(
            matches!(net, RequestError::Network(_)),
            "fixture must be a Network error, got {net:?}"
        );
        assert_eq!(classify(&net), TelegramState::Unreachable);
        assert_eq!(
            classify(&RequestError::RetryAfter(
                teloxide::types::Seconds::from_seconds(3)
            )),
            TelegramState::Unreachable
        );
        assert_eq!(
            classify(&RequestError::Io(std::sync::Arc::new(
                std::io::Error::other("disk")
            ))),
            TelegramState::Unreachable
        );
        // An unknown API text that is NOT about the token stays retryable.
        assert_eq!(
            classify(&unknown("Bad Request: chat not found")),
            TelegramState::Unreachable
        );
        assert_eq!(
            classify(&RequestError::Api(ApiError::BotBlocked)),
            TelegramState::Unreachable
        );
    }

    #[test]
    fn retry_delay_waits_five_minutes_on_an_invalid_token_only() {
        let backoff = Duration::from_secs(60);
        assert_eq!(
            retry_delay(TelegramState::InvalidToken, backoff),
            Duration::from_secs(300)
        );
        assert_eq!(retry_delay(TelegramState::Unreachable, backoff), backoff);
        assert_eq!(
            retry_delay(TelegramState::Unreachable, Duration::from_secs(5)),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn state_serialises_as_snake_case() {
        assert_eq!(
            serde_json::to_value(TelegramState::InvalidToken).unwrap(),
            serde_json::json!("invalid_token")
        );
        assert_eq!(
            serde_json::to_value(TelegramState::Unconfigured).unwrap(),
            serde_json::json!("unconfigured")
        );
        let parsed: TelegramState = serde_json::from_str("\"reachable\"").unwrap();
        assert_eq!(parsed, TelegramState::Reachable);
    }

    #[test]
    fn default_snapshot_is_unconfigured() {
        let _guard = test_guard();
        let snap = snapshot();
        assert_eq!(snap.state, TelegramState::Unconfigured);
        assert_eq!(snap.consecutive_failures, 0);
        assert_eq!(snap.bot_username, None);
        assert_eq!(snap.last_error, None);
        assert!(!snap.is_degraded_at(now_unix() + 100_000));
    }

    #[test]
    fn snapshot_transitions_track_failures_and_recovery() {
        let _guard = test_guard();

        assert_eq!(
            record_state_at(TelegramState::Starting, None, 1_000),
            TelegramState::Unconfigured
        );
        let snap = snapshot();
        assert_eq!(snap.state, TelegramState::Starting);
        assert_eq!(snap.since_unix, 1_000);
        assert_eq!(snap.consecutive_failures, 0);

        // Two failures in the same state: counter climbs, `since_unix` stays
        // at the first failure, the latest error text wins.
        assert_eq!(
            record_state_at(TelegramState::InvalidToken, Some("first".into()), 1_005),
            TelegramState::Starting
        );
        assert_eq!(
            record_state_at(TelegramState::InvalidToken, Some("second".into()), 1_305),
            TelegramState::InvalidToken
        );
        let snap = snapshot();
        assert_eq!(snap.state, TelegramState::InvalidToken);
        assert_eq!(snap.since_unix, 1_005);
        assert_eq!(snap.consecutive_failures, 2);
        assert_eq!(snap.last_error.as_deref(), Some("second"));
        assert!(snap.is_degraded_at(1_305));

        // Recovery clears the failure bookkeeping and records the username.
        assert_eq!(
            record_reachable_at("aegis_bot".into(), 2_000),
            TelegramState::InvalidToken
        );
        let snap = snapshot();
        assert_eq!(snap.state, TelegramState::Reachable);
        assert_eq!(snap.since_unix, 2_000);
        assert_eq!(snap.bot_username.as_deref(), Some("aegis_bot"));
        assert_eq!(snap.last_error, None);
        assert_eq!(snap.consecutive_failures, 0);
        assert!(!snap.is_degraded_at(9_999_999));

        // A later probe failure keeps the username so the operator can see
        // which bot disappeared, and a repeated success does not reset
        // `since_unix`.
        assert_eq!(
            record_reachable_at("aegis_bot".into(), 2_300),
            TelegramState::Reachable
        );
        assert_eq!(snapshot().since_unix, 2_000);
        assert_eq!(
            record_state_at(TelegramState::Unreachable, Some("dns".into()), 2_600),
            TelegramState::Reachable
        );
        let snap = snapshot();
        assert_eq!(snap.bot_username.as_deref(), Some("aegis_bot"));
        assert_eq!(snap.last_error.as_deref(), Some("dns"));
        assert_eq!(snap.consecutive_failures, 1);
    }

    #[test]
    fn unreachable_degrades_only_after_ten_minutes() {
        let _guard = test_guard();
        record_state_at(TelegramState::Unreachable, Some("dns".into()), 10_000);
        let snap = snapshot();
        assert!(!snap.is_degraded_at(10_000));
        assert!(!snap.is_degraded_at(10_600));
        assert!(snap.is_degraded_at(10_601));

        // The threshold is measured from the FIRST failure, not the latest.
        record_state_at(TelegramState::Unreachable, Some("dns again".into()), 10_500);
        assert!(snapshot().is_degraded_at(10_601));

        // Starting is never degraded, however long it takes.
        record_state_at(TelegramState::Starting, None, 10_000);
        assert!(!snapshot().is_degraded_at(99_999));
    }

    #[test]
    fn test_guard_reset_returns_the_global_to_unconfigured() {
        // Held for the whole test so no other test can interleave; the reset
        // exercised here is exactly what `TestGuard::drop` runs (while the
        // lock is still held, since `Drop::drop` runs before field drops).
        let _guard = test_guard();
        record_state(TelegramState::InvalidToken, Some("boom".into()));
        record_reachable("someone");
        record_state(TelegramState::Unreachable, Some("dns".into()));
        let dirty = snapshot();
        assert_eq!(dirty.state, TelegramState::Unreachable);
        assert_eq!(dirty.bot_username.as_deref(), Some("someone"));

        reset_for_tests();

        let clean = snapshot();
        assert_eq!(clean.state, TelegramState::Unconfigured);
        assert_eq!(clean.consecutive_failures, 0);
        assert_eq!(clean.bot_username, None);
        assert_eq!(clean.last_error, None);
    }
}
