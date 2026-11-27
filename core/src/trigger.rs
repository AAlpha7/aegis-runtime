//! Unified trigger source for scheduled / event-driven skill activations.
//!
//! Currently only `Cron` is wired into the scheduler (via `chronos_probes.rs`).
//! `Webhook` and `Event` are reserved placeholders for future extensions,
//! inspired by Anthropic Claude Code Routines' `/fire` HTTP trigger and GitHub
//! event triggers. See `.claude/memory/reference_anthropic_claude_routines.md`.
//!
//! This module also hosts [`validate_cron_interval`], a registration-time
//! defensive guard that rejects cron expressions whose next-two-fires interval
//! is below [`MIN_CRON_INTERVAL_SECS`]. This catches the well-documented
//! production trap where `*/5 * * * * *` (every 5 seconds) is confused with
//! `0 */5 * * * *` (every 5 minutes). See CLAUDE.md "Anti-Patterns / Cron /
//! Scheduler".
//!
//! The threshold is a `const` rather than a user-tunable setting on purpose:
//! it is a defensive guard against foot-guns, not a tunable policy.

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use croner::parser::{CronParser, Seconds};

/// Minimum allowed interval between consecutive cron fires, in seconds.
///
/// Chosen because every heartbeat tick runs a WASM probe + optional LLM call,
/// which easily exceeds 5s on its own; sub-30s cadences only make sense for
/// tight health-probes, which this project does not have. Bump this if the
/// need arises — but see the CLAUDE.md anti-pattern note first.
pub const MIN_CRON_INTERVAL_SECS: i64 = 30;

/// Validate that a 6-field tokio-cron-scheduler expression does not fire more
/// often than [`MIN_CRON_INTERVAL_SECS`].
///
/// Invoked at scheduler-registration time (see
/// `chronos_probes::start_scheduler_with_accounts`). The error message cites
/// the trap concretely so the operator can fix the expression in one shot.
pub fn validate_cron_interval(cron_expr: &str, skill_name: &str) -> Result<()> {
    // Match tokio-cron-scheduler 0.15's parser configuration exactly so we
    // reject the same pattern set the scheduler itself accepts. If this
    // validation passes and tcs's parser then rejects the same expression, we
    // have a bug; but at least we never OK something tcs would silently run
    // every second.
    let cron = CronParser::builder()
        .seconds(Seconds::Required)
        .dom_and_dow(true)
        .build()
        .parse(cron_expr)
        .map_err(|e| {
            anyhow!(
                "cron `{cron_expr}` for skill `{skill_name}` failed to parse: {e}. \
                 Expected 6 fields (sec min hour day month dow), e.g. `0 */5 * * * *` for every 5 minutes."
            )
        })?;

    // Compute the next two fire times and their delta. We use Utc because
    // tokio-cron-scheduler defaults to system-local; for interval computation
    // the timezone is irrelevant (the delta between two consecutive fires is
    // the same in any zone for all expressions this validator cares about).
    let now = Utc::now();
    let mut iter = cron.iter_after(now);
    let t1 = iter.next().with_context(|| {
        format!("cron `{cron_expr}` for skill `{skill_name}` produced no upcoming fire times")
    })?;
    let t2 = iter.next().with_context(|| {
        format!("cron `{cron_expr}` for skill `{skill_name}` produced only one upcoming fire time")
    })?;

    let delta = (t2 - t1).num_seconds();
    if delta < MIN_CRON_INTERVAL_SECS {
        return Err(anyhow!(
            "cron `{cron_expr}` for skill `{skill_name}` fires every {delta}s (< {min}s minimum). \
             If you meant every 5 minutes, use `0 */5 * * * *` (6 fields: sec min hour day month dow).",
            min = MIN_CRON_INTERVAL_SECS
        ));
    }

    Ok(())
}

/// Unified trigger source for scheduled / event-driven skill activations.
///
/// Currently only `Cron` is wired into the scheduler. `Webhook` and `Event`
/// are placeholders for future extensions (inspired by Anthropic Claude Code
/// Routines' /fire HTTP trigger and GitHub event triggers — see
/// .claude/memory/reference_anthropic_claude_routines.md).
#[derive(Debug, Clone)]
pub enum Trigger {
    /// 6-field tokio-cron-scheduler expression (sec min hour day month dow).
    /// Validated at construction via [`Trigger::cron`].
    Cron(String),
    /// Reserved: HTTP POST trigger with bearer-token auth on a /fire-style endpoint.
    #[allow(dead_code)]
    Webhook { path: String, secret_env: String },
    /// Reserved: in-process event trigger (e.g., new message, settings change).
    #[allow(dead_code)]
    Event(String),
}

impl Trigger {
    /// Construct a Cron trigger, running the minimum-interval validator.
    pub fn cron(expr: impl Into<String>, skill_name: &str) -> Result<Self> {
        let expr = expr.into();
        validate_cron_interval(&expr, skill_name)?;
        Ok(Self::Cron(expr))
    }

    /// Access the underlying cron expression, if this is a `Cron` trigger.
    ///
    /// Used by `chronos_probes.rs` to pass the validated string to
    /// `tokio_cron_scheduler::Job::new_async`.
    pub fn as_cron_str(&self) -> Option<&str> {
        match self {
            Trigger::Cron(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_5_second_cron_rejected() {
        let err = validate_cron_interval("*/5 * * * * *", "calendar_probe").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("every 5s"),
            "error should cite `every 5s`, got: {msg}"
        );
        assert!(
            msg.contains("0 */5 * * * *"),
            "error should suggest `0 */5 * * * *`, got: {msg}"
        );
        assert!(
            msg.contains("calendar_probe"),
            "error should include skill name, got: {msg}"
        );
    }

    #[test]
    fn validates_30_second_cron_accepted() {
        validate_cron_interval("*/30 * * * * *", "test_skill").expect("30s cron must pass");
    }

    #[test]
    fn validates_5_minute_cron_accepted() {
        validate_cron_interval("0 */5 * * * *", "test_skill").expect("5min cron must pass");
    }

    #[test]
    fn validates_hourly_cron_accepted() {
        validate_cron_interval("0 0 * * * *", "test_skill").expect("hourly cron must pass");
    }

    #[test]
    fn validates_invalid_cron_returns_parse_error() {
        let err = validate_cron_interval("not a cron", "weird_skill").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("failed to parse") || msg.contains("Expected 6 fields"),
            "error should mention parse failure, got: {msg}"
        );
        assert!(
            msg.contains("weird_skill"),
            "error should include skill name even on parse failure, got: {msg}"
        );
    }

    #[test]
    fn trigger_cron_valid_constructs() {
        let t = Trigger::cron("0 */5 * * * *", "test_skill").expect("valid cron");
        assert!(matches!(t, Trigger::Cron(_)));
        assert_eq!(t.as_cron_str(), Some("0 */5 * * * *"));
    }

    #[test]
    fn trigger_cron_too_fast_rejected() {
        let err = Trigger::cron("*/5 * * * * *", "test_skill").unwrap_err();
        assert!(err.to_string().contains("every 5s"));
    }
}
