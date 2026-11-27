//! Self-Audit Engine: zero-LLM-cost continuous system integrity monitor.
//!
//! Runs three tiers of deterministic checks:
//! 1. **Invariant checks** (startup + every 6h) — configuration consistency
//! 2. **Rate monitors** (every 10 min) — metric anomaly detection
//! 3. **Trend detectors** (every 6h) — slow degradation over time
//!
//! Each check returns zero or more `AuditFinding`s that are processed by
//! a graduated reporter: first fire → log only, 3rd consecutive → Telegram alert,
//! Critical → immediate Telegram.
//!
//! A daily digest summarises system health even when everything is healthy,
//! giving the user baseline visibility.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Timelike, Utc};
use teloxide::types::ChatId;

use crate::chronos;
use crate::memory::MemoryHandle;
use crate::models::SkillCapability;
use crate::settings::RuntimeSettings;
use crate::telegram::{markdown_to_telegram_html, send_html_to_topic};
use crate::telegram_notification_router::{
    NotificationClass, PrivacyLevel, proactive_route_request, route_telegram_notification,
    topic_for_class,
};
use crate::tool_gate::{CallerContext, GateVerdict, ToolGate};
use crate::topic_router::TopicKind;

// ── Constants ──────────────────────────────────────────────────────────

/// Interval between rate-check cycles (10 minutes).
const RATE_CHECK_INTERVAL: Duration = Duration::from_secs(600);

/// Interval between invariant + trend checks (6 hours).
const SLOW_CHECK_INTERVAL: Duration = Duration::from_secs(21_600);

/// Consecutive rate-check fires before escalating to Telegram.
const ALERT_THRESHOLD: u32 = 3;
/// After an alert, stay quiet at least this long (then 3x, capped).
const ALERT_RENAG_MIN: Duration = Duration::from_secs(3600);
const ALERT_RENAG_MAX: Duration = Duration::from_secs(24 * 3600);
/// Watchdog: grace period after engine start before a missing tick counts.
const WATCHDOG_START_GRACE: Duration = Duration::from_secs(1800);

/// Heartbeat no-op ratio above which we flag waste.
const MAX_NOOP_RATIO: f64 = 0.70;

/// Suppress stale heartbeat-noop findings once recent ticks are zero-cost skips.
const RECENT_NOOP_WINDOW_HOURS: u32 = 2;

/// Cache hit decline (percentage points) that triggers a trend alert.
const CACHE_DECLINE_THRESHOLD: f64 = 10.0;

/// Token cost increase (factor) that triggers a trend alert.
const TOKEN_COST_INCREASE_FACTOR: f64 = 1.20;

/// Hour of day (UTC) at which the daily digest is sent.
const DIGEST_HOUR_UTC: u32 = 1; // 9 AM UTC+8

/// skill_state key prefix for self-audit data.
const STATE_SKILL: &str = "_self_audit";

fn routed_self_audit_destination(admin_chat_id: i64) -> (ChatId, TopicKind) {
    let notification_class = NotificationClass::SystemStatus;
    let route = route_telegram_notification(
        admin_chat_id,
        proactive_route_request(notification_class, PrivacyLevel::SystemOnly, None),
    );
    (
        ChatId(route.chat_id),
        route
            .topic_kind
            .unwrap_or_else(|| topic_for_class(notification_class)),
    )
}

// ── Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuditSeverity {
    Info,
    Warning,
    Critical,
}

impl std::fmt::Display for AuditSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Info => write!(f, "INFO"),
            Self::Warning => write!(f, "WARN"),
            Self::Critical => write!(f, "CRIT"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuditFinding {
    pub check_name: &'static str,
    pub severity: AuditSeverity,
    pub message: String,
}

/// Shared context for all checks — immutable after construction.
pub struct AuditContext {
    pub memory: MemoryHandle,
    pub loaded_skills: Vec<SkillCapability>,
    pub autonomous_allowed_tools: Vec<String>,
    pub settings: Arc<RuntimeSettings>,
}

/// Mutable state tracking consecutive fires for graduated response.
struct AuditState {
    consecutive_fires: HashMap<&'static str, u32>,
    gate: crate::alert_gate::AlertGate,
    last_slow_check: Instant,
    digest_sent_date: Option<String>,
}

impl AuditState {
    /// Record that `check` fired this cycle and decide whether to alert now.
    /// Returns the consecutive-fire count when an alert should go out.
    /// Critical alerts on the first fire; everything else after
    /// `ALERT_THRESHOLD`. Either way a persisting condition re-nags with
    /// backoff (1h, 3h, 9h, capped at 24h) instead of every cycle.
    fn decide_alert(
        &mut self,
        check: &'static str,
        severity: AuditSeverity,
        now: Instant,
    ) -> Option<u32> {
        let threshold = match severity {
            AuditSeverity::Critical => 1,
            _ => ALERT_THRESHOLD,
        };
        let alert = self.gate.record_failure(check, threshold, now);
        let count = self.gate.consecutive(check);
        self.consecutive_fires.insert(check, count);
        alert.then_some(count)
    }

    fn new() -> Self {
        Self {
            consecutive_fires: HashMap::new(),
            gate: crate::alert_gate::AlertGate::new(ALERT_RENAG_MIN, ALERT_RENAG_MAX),
            // Force slow checks to run on first cycle.
            last_slow_check: Instant::now() - SLOW_CHECK_INTERVAL,
            digest_sent_date: None,
        }
    }
}

// ── Main Entry Point ───────────────────────────────────────────────────

/// Run the self-audit engine (spawned as a background tokio task).
pub async fn run_audit_engine(
    memory: MemoryHandle,
    bot: teloxide::Bot,
    settings: Arc<RuntimeSettings>,
    loaded_skills: Vec<SkillCapability>,
    chat_id: i64,
) {
    let ctx = AuditContext {
        memory,
        loaded_skills,
        autonomous_allowed_tools: chronos::default_autonomous_allowed_tools(),
        settings,
    };
    let mut state = AuditState::new();

    // Restore digest_sent_date from DB (survive restarts).
    if let Ok(Some(date)) = ctx
        .memory
        .load_skill_state(STATE_SKILL, "digest_date")
        .await
    {
        state.digest_sent_date = Some(date);
    }

    // Run invariant checks immediately on startup.
    let findings = run_invariant_checks(&ctx).await;
    report_findings(&ctx.memory, &findings, &mut state, &bot, chat_id).await;

    let mut interval = tokio::time::interval(RATE_CHECK_INTERVAL);
    interval.tick().await; // skip first immediate tick

    loop {
        interval.tick().await;

        // Write liveness tick for watchdog.
        ctx.memory
            .store_skill_state(STATE_SKILL, "last_tick", &Utc::now().to_rfc3339())
            .await
            .ok();

        // Rate checks every cycle.
        let findings = run_rate_checks(&ctx).await;
        report_findings(&ctx.memory, &findings, &mut state, &bot, chat_id).await;

        // Invariant + trend checks every SLOW_CHECK_INTERVAL.
        if state.last_slow_check.elapsed() >= SLOW_CHECK_INTERVAL {
            let inv = run_invariant_checks(&ctx).await;
            report_findings(&ctx.memory, &inv, &mut state, &bot, chat_id).await;

            let trend = run_trend_checks(&ctx).await;
            report_findings(&ctx.memory, &trend, &mut state, &bot, chat_id).await;

            state.last_slow_check = Instant::now();
        }

        // Daily digest (check hour, send once per day).
        try_send_daily_digest(&ctx, &mut state, &bot, chat_id).await;
    }
}

/// Watchdog alert decision: ignore staleness during the start grace period,
/// alert on the healthy→stale edge, then re-nag with backoff instead of every
/// 30-minute tick.
fn watchdog_should_alert(
    stale: bool,
    since_start: Duration,
    gate: &mut crate::alert_gate::AlertGate,
    now: Instant,
) -> bool {
    if !stale {
        gate.record_success("audit_engine");
        return false;
    }
    if since_start < WATCHDOG_START_GRACE {
        return false;
    }
    gate.record_failure("audit_engine", 1, now)
}

/// Lightweight watchdog: monitors self-audit engine liveness.
pub async fn run_audit_watchdog(memory: MemoryHandle, bot: teloxide::Bot, chat_id: i64) {
    let mut interval = tokio::time::interval(Duration::from_secs(1800));
    interval.tick().await; // skip first
    let started = Instant::now();
    let mut gate = crate::alert_gate::AlertGate::new(
        Duration::from_secs(6 * 3600),
        Duration::from_secs(24 * 3600),
    );

    loop {
        interval.tick().await;

        let stale = match memory.load_skill_state(STATE_SKILL, "last_tick").await {
            Ok(Some(ts)) => chrono::DateTime::parse_from_rfc3339(&ts)
                .map(|t| Utc::now().signed_duration_since(t).num_seconds() > 1500)
                .unwrap_or(true),
            _ => true, // no tick recorded yet — might still be starting
        };

        if watchdog_should_alert(stale, started.elapsed(), &mut gate, Instant::now()) {
            let msg = "Self-audit engine appears stalled (no tick in 25 min). \
                       Check process logs for panics.";
            tracing::error!("[SelfAudit:Watchdog] {msg}");
            if crate::telegram::record_owner_web_notice_if_push_delivered(&memory, chat_id, msg)
                .await
            {
                continue;
            }
            let html = markdown_to_telegram_html(msg);
            let (dest, topic) = routed_self_audit_destination(chat_id);
            if let Err(e) = send_html_to_topic(&bot, dest, topic, &html).await {
                tracing::warn!(error = %e, "[SelfAudit:Watchdog] Failed to send alert");
            }
        }
    }
}

// ── Invariant Checks ───────────────────────────────────────────────────

async fn run_invariant_checks(ctx: &AuditContext) -> Vec<AuditFinding> {
    let mut findings = Vec::new();
    findings.extend(check_toolgate_allowlist_sync(ctx));
    findings.extend(check_cron_interval_sanity(ctx));
    // Note: `check_disabled_skill_cron` was removed 2026-04-17. Its premise
    // (disabled skills generate a debug skip-log every cron tick) was
    // invalidated by the 2026-04-11 fix that skips cron registration entirely
    // at startup for disabled skills (see `chronos_probes::should_register_cron_for_skill`).
    // The check itself had become the only remaining source of `disabled_skill_cron`
    // log noise every 10 min.
    findings
}

/// Verify every tool in the heartbeat allowed-tools list has an Allow
/// policy in the ToolGate. A mismatch means the LLM is offered a tool
/// but blocked at execution — wasting tokens on the call + fallback.
fn check_toolgate_allowlist_sync(ctx: &AuditContext) -> Vec<AuditFinding> {
    let gate = ToolGate::new();
    let mut findings = Vec::new();

    for tool in &ctx.autonomous_allowed_tools {
        let verdict = gate.evaluate(tool, "{}", CallerContext::Heartbeat);
        if matches!(verdict, GateVerdict::Block { .. }) {
            findings.push(AuditFinding {
                check_name: "toolgate_sync",
                severity: AuditSeverity::Warning,
                message: format!(
                    "Tool '{}' is in heartbeat allowed_tools but blocked by ToolGate. \
                     Each heartbeat cycle wastes tokens calling this tool and falling back.",
                    tool
                ),
            });
        }
    }
    findings
}

/// Minimum interval between cron firings (60 seconds).
/// Rejects schedules that fire more than once per minute to prevent
/// resource exhaustion from overly aggressive heartbeats.
const MIN_CRON_INTERVAL_SECS: i64 = 60;

/// Reject cron expressions that fire too frequently.
///
/// Uses the `croner` crate (already a transitive dependency via
/// tokio-cron-scheduler) to compute the next 2 occurrences and checks that the
/// gap is at least `MIN_CRON_INTERVAL_SECS`. Returns an error message if the
/// schedule is too frequent, or `None` if acceptable.
pub(crate) fn reject_too_frequent_cron(cron_expr: &str) -> Option<String> {
    let parser = croner::parser::CronParser::builder()
        .seconds(croner::parser::Seconds::Optional)
        .build();
    let cron = match parser.parse(cron_expr) {
        Ok(cron) => cron,
        Err(e) => {
            return Some(format!("cron schedule '{}' is invalid: {}", cron_expr, e));
        }
    };

    let now = chrono::Utc::now();
    let first = match cron.find_next_occurrence(&now, false) {
        Ok(first) => first,
        Err(e) => {
            return Some(format!(
                "cron schedule '{}' has no next occurrence: {}",
                cron_expr, e
            ));
        }
    };
    let second = match cron.find_next_occurrence(&first, false) {
        Ok(second) => second,
        Err(e) => {
            return Some(format!(
                "cron schedule '{}' has no second occurrence for cadence check: {}",
                cron_expr, e
            ));
        }
    };

    let gap_secs = (second - first).num_seconds();
    if gap_secs < MIN_CRON_INTERVAL_SECS {
        Some(format!(
            "cron schedule '{}' fires every {}s — minimum interval is {}s (once per minute)",
            cron_expr, gap_secs, MIN_CRON_INTERVAL_SECS
        ))
    } else {
        None
    }
}

/// Verify all loaded skills' cron schedules have sane intervals (>= 60s).
/// Catches 6-field cron misreads (e.g. `*/2 * * * * *` = every 2 seconds).
fn check_cron_interval_sanity(ctx: &AuditContext) -> Vec<AuditFinding> {
    let mut findings = Vec::new();
    for skill in &ctx.loaded_skills {
        if let Some(ref cron_expr) = skill.cron_schedule
            && let Some(msg) = reject_too_frequent_cron(cron_expr)
        {
            findings.push(AuditFinding {
                check_name: "cron_interval",
                severity: AuditSeverity::Critical,
                message: format!("Skill '{}': {}", skill.skill_name, msg),
            });
        }
    }
    findings
}

// ── Rate Monitors ──────────────────────────────────────────────────────

async fn run_rate_checks(ctx: &AuditContext) -> Vec<AuditFinding> {
    let mut findings = Vec::new();
    findings.extend(check_heartbeat_noop_ratio(ctx).await);
    findings.extend(check_tool_block_rate(ctx).await);
    findings.extend(check_llm_log_coverage(ctx).await);
    findings
}

/// Alert if > 70% of heartbeat cycles in the last 24h were no-ops
/// (no tools called, zero tokens used). Indicates the LLM is invoked
/// but has nothing to do — pure waste.
async fn check_heartbeat_noop_ratio(ctx: &AuditContext) -> Vec<AuditFinding> {
    let (noop, total) = match ctx.memory.audit_heartbeat_noop_ratio(24).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    if total < 10 {
        return Vec::new(); // not enough data
    }
    let ratio = noop as f64 / total as f64;
    if ratio > MAX_NOOP_RATIO {
        let recent_noop = match ctx
            .memory
            .audit_recent_heartbeat_noop_count(RECENT_NOOP_WINDOW_HOURS)
            .await
        {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        if recent_noop == 0 {
            tracing::info!(
                noop,
                total,
                recent_window_hours = RECENT_NOOP_WINDOW_HOURS,
                "[SelfAudit] heartbeat_noop ratio is stale; suppressing alert because no recent heartbeat noop rows exist"
            );
            return Vec::new();
        }
        vec![AuditFinding {
            check_name: "heartbeat_noop",
            severity: AuditSeverity::Warning,
            message: format!(
                "Heartbeat no-op ratio is {:.0}% ({}/{} LLM-eligible cycles in 24h; {} noop in the last {}h). \
                 Each no-op wastes ~6K tokens. Consider adding zero-cost prechecks.",
                ratio * 100.0,
                noop,
                total,
                recent_noop,
                RECENT_NOOP_WINDOW_HOURS
            ),
        }]
    } else {
        Vec::new()
    }
}

/// Alert if tools are being blocked by ToolGate during heartbeat cycles.
async fn check_tool_block_rate(ctx: &AuditContext) -> Vec<AuditFinding> {
    let blocks = match ctx.memory.audit_recent_tool_blocks(1).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    if blocks.is_empty() {
        return Vec::new();
    }
    let total: i64 = blocks.iter().map(|(_, c)| *c).sum();
    if total < 3 {
        return Vec::new();
    }
    let tool_list: Vec<String> = blocks
        .iter()
        .map(|(name, count)| format!("{}({}x)", name, count))
        .collect();
    vec![AuditFinding {
        check_name: "tool_block_rate",
        severity: AuditSeverity::Warning,
        message: format!(
            "{} tool blocks in the last hour: {}. \
             Check ToolGate allow list synchronisation.",
            total,
            tool_list.join(", ")
        ),
    }]
}

/// Verify the LLM call log global channel is working: if heartbeats ran
/// in the last hour but zero LLM calls were recorded, the channel is broken.
async fn check_llm_log_coverage(ctx: &AuditContext) -> Vec<AuditFinding> {
    let llm_count = ctx.memory.audit_recent_llm_calls(1).await.unwrap_or(0);
    let heartbeat_count = ctx
        .memory
        .audit_recent_llm_eligible_heartbeat_count(1)
        .await
        .unwrap_or(0);

    if heartbeat_count > 0 && llm_count == 0 {
        vec![AuditFinding {
            check_name: "llm_log_coverage",
            severity: AuditSeverity::Warning,
            message: format!(
                "{} heartbeat cycles in the last hour but 0 LLM calls logged to DB. \
                 The global llm_call_log channel may be broken.",
                heartbeat_count
            ),
        }]
    } else {
        Vec::new()
    }
}

// ── Trend Detectors ────────────────────────────────────────────────────

async fn run_trend_checks(ctx: &AuditContext) -> Vec<AuditFinding> {
    let mut findings = Vec::new();
    findings.extend(check_cache_rate_decline(ctx).await);
    findings.extend(check_token_cost_increase(ctx).await);
    findings.extend(check_skill_error_creep(ctx).await);
    findings.extend(check_unpriced_models(ctx).await);
    findings.extend(check_pricing_staleness(ctx).await);
    findings.extend(check_cost_reconciliation(ctx).await);
    findings
}

/// Detect LLM cache hit rate declining over 7 days.
async fn check_cache_rate_decline(ctx: &AuditContext) -> Vec<AuditFinding> {
    let rates = match ctx.memory.audit_daily_cache_rates(8).await {
        Ok(v) if v.len() >= 4 => v,
        _ => return Vec::new(),
    };

    // The last bucket is TODAY, one hour old when the digest runs at
    // 01:00 UTC — it read 0.0% every morning and made this finding permanent
    // (and ignored). Compare the last COMPLETE day against the days before it.
    let complete = &rates[..rates.len() - 1];
    let latest = complete.last().unwrap().1;
    let prior_avg: f64 = complete[..complete.len() - 1]
        .iter()
        .map(|(_, r)| *r)
        .sum::<f64>()
        / (complete.len() - 1) as f64;
    let decline = prior_avg - latest;

    if decline > CACHE_DECLINE_THRESHOLD {
        vec![AuditFinding {
            check_name: "cache_decline",
            severity: AuditSeverity::Warning,
            message: format!(
                "LLM cache hit rate declined {:.1} points: prior avg {:.1}% → today {:.1}%. \
                 Check if recurring calls exceed the 5-min cache TTL.",
                decline, prior_avg, latest
            ),
        }]
    } else {
        Vec::new()
    }
}

/// Detect daily token cost increasing over 7 days.
async fn check_token_cost_increase(ctx: &AuditContext) -> Vec<AuditFinding> {
    let totals = match ctx.memory.audit_daily_token_totals(7).await {
        Ok(v) if v.len() >= 3 => v,
        _ => return Vec::new(),
    };

    let latest = totals.last().unwrap().1 as f64;
    let prior: Vec<f64> = totals[..totals.len() - 1]
        .iter()
        .map(|(_, t)| *t as f64)
        .collect();
    let prior_avg: f64 = prior.iter().sum::<f64>() / prior.len() as f64;

    if prior_avg > 0.0 && latest > prior_avg * TOKEN_COST_INCREASE_FACTOR {
        let pct = ((latest / prior_avg) - 1.0) * 100.0;
        vec![AuditFinding {
            check_name: "token_cost",
            severity: AuditSeverity::Warning,
            message: format!(
                "Daily token usage up {:.0}%: prior avg {:.0}K → today {:.0}K.",
                pct,
                prior_avg / 1000.0,
                latest / 1000.0
            ),
        }]
    } else {
        Vec::new()
    }
}

/// Detect gradual skill error rate increases using the existing
/// (but previously unused) skill_metrics_daily table.
async fn check_skill_error_creep(ctx: &AuditContext) -> Vec<AuditFinding> {
    let mut findings = Vec::new();

    for skill in &ctx.loaded_skills {
        let trend = ctx
            .memory
            .get_daily_metrics_trend(&skill.skill_name, 7)
            .await;
        if trend.len() < 3 {
            continue;
        }

        let latest_errors = trend.last().unwrap().2;
        if latest_errors < 5 {
            continue;
        }
        let prior_avg: f64 = trend[..trend.len() - 1]
            .iter()
            .map(|(_, _, errors, _)| *errors as f64)
            .sum::<f64>()
            / (trend.len() - 1) as f64;

        if prior_avg > 0.0 && latest_errors as f64 > prior_avg * 1.5 {
            findings.push(AuditFinding {
                check_name: "skill_error_creep",
                severity: AuditSeverity::Warning,
                message: format!(
                    "Skill '{}' errors trending up: prior avg {:.0}/day → today {}.",
                    skill.skill_name, prior_avg, latest_errors
                ),
            });
        }
    }
    findings
}

// ── Cost Integrity Checks ─────────────────────────────────────────────

/// Check for unpriced models: if >5% of LLM calls in the last 24h have
/// NULL cost_usd, pricing data is likely missing for some models.
async fn check_unpriced_models(ctx: &AuditContext) -> Vec<AuditFinding> {
    let coverage = match ctx.memory.query_cost_coverage(1).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    unpriced_models_finding(&coverage).into_iter().collect()
}

/// Check pricing data staleness: warn if llm_pricing table has no
/// recent entries (no API fetch in >48h). This uses the query_cost_by_model
/// as a proxy — if we have calls but no pricing, the cache may be stale.
async fn check_pricing_staleness(ctx: &AuditContext) -> Vec<AuditFinding> {
    let coverage = match ctx.memory.query_cost_coverage(2).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    pricing_staleness_finding(&coverage).into_iter().collect()
}

fn unpriced_models_finding(coverage: &crate::memory::CostCoverage) -> Option<AuditFinding> {
    if coverage.api_row_count > 10
        && coverage.api_null_cost_rows as f64 / coverage.api_row_count as f64 > 0.05
    {
        Some(AuditFinding {
            check_name: "unpriced_models",
            severity: AuditSeverity::Warning,
            message: format!(
                "{}/{} API-backed LLM calls in the last 24h have unknown cost_usd (NULL), not zero. \
                 Subscription zero-marginal rows excluded: {}. Run pricing update/backfill or add pricing for missing API models.",
                coverage.api_null_cost_rows,
                coverage.api_row_count,
                coverage.zero_marginal_subscription_rows
            ),
        })
    } else {
        None
    }
}

fn pricing_staleness_finding(coverage: &crate::memory::CostCoverage) -> Option<AuditFinding> {
    if coverage.api_null_cost_rows > 0 && coverage.pricing_rows == 0 {
        Some(AuditFinding {
            check_name: "pricing_staleness",
            severity: AuditSeverity::Warning,
            message: format!(
                "llm_pricing table is empty while {}/{} recent API-backed LLM calls have unknown cost_usd. \
                 Subscription foreground calls are zero-marginal and excluded; refresh provider pricing, then backfill.",
                coverage.api_null_cost_rows, coverage.api_row_count
            ),
        })
    } else {
        None
    }
}

/// Basic cost reconciliation: compare daily local cost totals for anomalies.
/// If today's cost is >3x the 7-day average, flag a potential billing issue.
async fn check_cost_reconciliation(ctx: &AuditContext) -> Vec<AuditFinding> {
    let daily = match ctx.memory.query_daily_cost_totals(7).await {
        Ok(v) if v.len() >= 3 => v,
        _ => return Vec::new(),
    };

    let today_cost = daily.last().unwrap().cost;
    let prior_avg: f64 =
        daily[..daily.len() - 1].iter().map(|d| d.cost).sum::<f64>() / (daily.len() - 1) as f64;

    if prior_avg > 0.001 && today_cost > prior_avg * 3.0 {
        vec![AuditFinding {
            check_name: "cost_reconciliation",
            severity: AuditSeverity::Warning,
            message: format!(
                "Today's LLM cost ${:.4} is {:.1}x the 7-day avg ${:.4}. \
                 Check for runaway calls or pricing anomalies.",
                today_cost,
                today_cost / prior_avg,
                prior_avg
            ),
        }]
    } else {
        Vec::new()
    }
}

// ── Graduated Reporter ─────────────────────────────────────────────────

/// All check names, used to reset counters when a check passes.
const ALL_CHECK_NAMES: &[&str] = &[
    "toolgate_sync",
    "cron_interval",
    "heartbeat_noop",
    "tool_block_rate",
    "llm_log_coverage",
    "cache_decline",
    "token_cost",
    "skill_error_creep",
    "unpriced_models",
    "pricing_staleness",
    "cost_reconciliation",
];

async fn report_findings(
    memory: &MemoryHandle,
    findings: &[AuditFinding],
    state: &mut AuditState,
    bot: &teloxide::Bot,
    chat_id: i64,
) {
    // Increment counters for fired checks.
    for f in findings {
        if let Some(count) = state.decide_alert(f.check_name, f.severity, Instant::now()) {
            let text = format!(
                "[SelfAudit] [{}] {}: {} (consecutive: {})",
                f.severity, f.check_name, f.message, count
            );
            tracing::warn!("{text}");
            if crate::telegram::record_owner_web_notice_if_push_delivered(memory, chat_id, &text)
                .await
            {
                continue;
            }
            let html = markdown_to_telegram_html(&text);
            let (dest, topic) = routed_self_audit_destination(chat_id);
            if let Err(e) = send_html_to_topic(bot, dest, topic, &html).await {
                tracing::warn!(error = %e, "[SelfAudit] Failed to send alert");
            }
        } else {
            tracing::info!(
                "[SelfAudit] [{}] {}: {}",
                f.severity,
                f.check_name,
                f.message
            );
        }
    }

    // Reset counters for checks that passed this cycle.
    let fired_names: Vec<&str> = findings.iter().map(|f| f.check_name).collect();
    for &name in ALL_CHECK_NAMES {
        if !fired_names.contains(&name) {
            state.gate.record_success(name);
            if let Some(prev) = state.consecutive_fires.remove(name)
                && prev >= ALERT_THRESHOLD
            {
                tracing::info!("[SelfAudit] [RESOLVED] {name}");
            }
        }
    }
}

// ── Daily Digest ───────────────────────────────────────────────────────

async fn try_send_daily_digest(
    ctx: &AuditContext,
    state: &mut AuditState,
    bot: &teloxide::Bot,
    chat_id: i64,
) {
    let now = Utc::now();
    let today = now.format("%Y-%m-%d").to_string();

    // Already sent today?
    if state.digest_sent_date.as_deref() == Some(today.as_str()) {
        return;
    }
    // Not the right hour?
    if now.hour() != DIGEST_HOUR_UTC {
        return;
    }

    // Compute digest.
    let inv = run_invariant_checks(ctx).await;
    let rate = run_rate_checks(ctx).await;
    let trend = run_trend_checks(ctx).await;

    let total_inv = 3u32; // number of invariant checks
    let total_rate = 4u32; // number of rate checks
    let total_trend = 6u32; // number of trend checks (3 original + 3 cost checks)

    let inv_pass = total_inv - inv.len() as u32;
    let rate_pass = total_rate - rate.len() as u32;
    let trend_pass = total_trend - trend.len() as u32;

    let mut lines = Vec::new();
    lines.push("<b>System Integrity Report</b>".to_string());
    lines.push(String::new());

    // Invariants
    let inv_status = if inv.is_empty() { "✅" } else { "⚠" };
    lines.push(format!(
        "<b>Invariants:</b> {}/{} {inv_status}",
        inv_pass, total_inv
    ));
    for f in &inv {
        lines.push(format!("  • {}: {}", f.check_name, f.message));
    }

    // Rate monitors
    let rate_status = if rate.is_empty() { "✅" } else { "⚠" };
    lines.push(format!(
        "<b>Rate Monitors:</b> {}/{} {rate_status}",
        rate_pass, total_rate
    ));
    for f in &rate {
        lines.push(format!("  • {}: {}", f.check_name, f.message));
    }

    // Trends
    let trend_status = if trend.is_empty() { "✅" } else { "⚠" };
    lines.push(format!(
        "<b>Trends (7d):</b> {}/{} {trend_status}",
        trend_pass, total_trend
    ));
    for f in &trend {
        lines.push(format!("  • {}: {}", f.check_name, f.message));
    }

    // Active findings summary
    let active = state
        .consecutive_fires
        .iter()
        .filter(|(_, c)| **c >= ALERT_THRESHOLD)
        .count();
    lines.push(format!("\n<b>Active alerts:</b> {active}"));

    let digest = lines.join("\n");
    let _ = bot;
    crate::telegram::record_owner_web_notice(&ctx.memory, chat_id, &digest).await;

    // Mark sent.
    state.digest_sent_date = Some(today.clone());
    ctx.memory
        .store_skill_state(STATE_SKILL, "digest_date", &today)
        .await
        .ok();

    tracing::info!("[SelfAudit] Daily digest sent for {today}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heartbeat_config::HeartbeatTickOutcome;
    use crate::memory::{HeartbeatAuditExtras, MemoryHandle};

    async fn store_heartbeat_outcome(
        mem: &MemoryHandle,
        cycle_id: &str,
        outcome: HeartbeatTickOutcome,
    ) {
        let extras = HeartbeatAuditExtras {
            outcome: Some(outcome),
            ..Default::default()
        };
        mem.store_heartbeat_audit_ext(
            cycle_id,
            "__heartbeat__",
            "info",
            None,
            &[],
            &[],
            &[],
            None,
            0,
            0,
            "did:aegis:test",
            Some(1),
            "{}",
            None,
            None,
            &extras,
        )
        .await
        .unwrap();
    }

    fn audit_context(mem: MemoryHandle) -> AuditContext {
        AuditContext {
            memory: mem,
            loaded_skills: Vec::new(),
            autonomous_allowed_tools: chronos::default_autonomous_allowed_tools(),
            settings: Arc::new(RuntimeSettings::new()),
        }
    }

    #[tokio::test]
    async fn heartbeat_noop_warning_suppressed_when_no_recent_noops_exist() {
        // In-memory + actor-serialized backdate: dual-connection file DBs race
        // under parallel `cargo test --lib` (second writer vs actor WAL).
        let mem = MemoryHandle::spawn_in_memory().unwrap();

        for idx in 0..10 {
            store_heartbeat_outcome(&mem, &format!("noop-{idx}"), HeartbeatTickOutcome::Noop).await;
        }
        store_heartbeat_outcome(&mem, "skip-recent", HeartbeatTickOutcome::SkippedPrecheck).await;

        // 3h ago is outside RECENT_NOOP_WINDOW_HOURS (2h) but still in the 24h ratio window.
        let updated = mem
            .test_backdate_heartbeat_audit_by_outcome("noop", 3)
            .await
            .expect("actor backdate");
        assert_eq!(updated, 10, "all seeded noop rows must be backdated");

        let findings = check_heartbeat_noop_ratio(&audit_context(mem)).await;
        assert!(
            findings.is_empty(),
            "stale 24h noop ratio must not keep alerting after recent ticks are zero-cost skips"
        );
    }

    #[tokio::test]
    async fn heartbeat_noop_warning_ignores_zero_cost_skipped_precheck_heavy_window() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();

        for idx in 0..30 {
            store_heartbeat_outcome(
                &mem,
                &format!("skip-{idx}"),
                HeartbeatTickOutcome::SkippedPrecheck,
            )
            .await;
        }

        let findings = check_heartbeat_noop_ratio(&audit_context(mem)).await;
        assert!(
            findings.is_empty(),
            "zero-cost skipped_precheck-heavy windows must not fire heartbeat_noop warnings"
        );
    }

    #[tokio::test]
    async fn heartbeat_noop_warning_counts_maintenance_only_in_denominator() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();

        for idx in 0..8 {
            store_heartbeat_outcome(&mem, &format!("noop-{idx}"), HeartbeatTickOutcome::Noop).await;
        }
        for idx in 0..4 {
            store_heartbeat_outcome(
                &mem,
                &format!("maintenance-{idx}"),
                HeartbeatTickOutcome::MaintenanceOnly,
            )
            .await;
        }
        for idx in 0..2 {
            store_heartbeat_outcome(&mem, &format!("acted-{idx}"), HeartbeatTickOutcome::Acted)
                .await;
        }

        let findings = check_heartbeat_noop_ratio(&audit_context(mem)).await;
        assert!(
            findings.is_empty(),
            "maintenance_only rows must count as LLM-eligible denominator rows"
        );
    }

    #[tokio::test]
    async fn heartbeat_noop_warning_excludes_skipped_precheck_from_denominator() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();

        for idx in 0..8 {
            store_heartbeat_outcome(&mem, &format!("noop-{idx}"), HeartbeatTickOutcome::Noop).await;
        }
        for idx in 0..2 {
            store_heartbeat_outcome(&mem, &format!("acted-{idx}"), HeartbeatTickOutcome::Acted)
                .await;
        }
        for idx in 0..30 {
            store_heartbeat_outcome(
                &mem,
                &format!("skip-{idx}"),
                HeartbeatTickOutcome::SkippedPrecheck,
            )
            .await;
        }

        let findings = check_heartbeat_noop_ratio(&audit_context(mem)).await;
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].check_name, "heartbeat_noop");
        assert!(
            findings[0].message.contains("8/10 LLM-eligible cycles"),
            "skipped_precheck rows must stay out of the ratio denominator: {}",
            findings[0].message
        );
    }

    #[tokio::test]
    async fn heartbeat_noop_warning_fires_when_recent_noops_exist() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();

        for idx in 0..10 {
            store_heartbeat_outcome(&mem, &format!("noop-{idx}"), HeartbeatTickOutcome::Noop).await;
        }

        let findings = check_heartbeat_noop_ratio(&audit_context(mem)).await;
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].check_name, "heartbeat_noop");
        assert!(findings[0].message.contains("noop in the last 2h"));
    }

    /// The `duplicate_panels` rate check was removed with the Smart Panel
    /// feature (2026-09-16): the audit must neither run it nor keep its name in
    /// the counter-reset list. Negative control: the remaining rate checks
    /// still run on the same context (a recent-noop window fires
    /// `heartbeat_noop`) and the reset list still carries their names.
    #[tokio::test]
    async fn rate_checks_no_longer_include_duplicate_panels() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        for idx in 0..10 {
            store_heartbeat_outcome(&mem, &format!("noop-{idx}"), HeartbeatTickOutcome::Noop).await;
        }

        let findings = run_rate_checks(&audit_context(mem)).await;
        let names: Vec<&str> = findings.iter().map(|f| f.check_name).collect();
        assert!(
            names.contains(&"heartbeat_noop"),
            "remaining rate checks must still run: {names:?}"
        );
        assert!(
            !names.contains(&"duplicate_panels"),
            "retired check must not fire: {names:?}"
        );
        assert!(!ALL_CHECK_NAMES.contains(&"duplicate_panels"));
        assert!(ALL_CHECK_NAMES.contains(&"heartbeat_noop"));
        assert!(ALL_CHECK_NAMES.contains(&"llm_log_coverage"));
    }

    fn cost_coverage_for_test(
        row_count: i64,
        api_row_count: i64,
        api_null_cost_rows: i64,
        pricing_rows: i64,
        zero_marginal_subscription_rows: i64,
    ) -> crate::memory::CostCoverage {
        crate::memory::CostCoverage {
            row_count,
            priced_row_count: row_count - api_null_cost_rows,
            null_cost_rows: api_null_cost_rows,
            api_row_count,
            api_priced_row_count: api_row_count - api_null_cost_rows,
            api_null_cost_rows,
            zero_marginal_subscription_rows,
            total_prompt_tokens: 1,
            total_completion_tokens: 1,
            total_cached_tokens: 0,
            total_tokens: 2,
            min_timestamp: Some("2026-05-23 00:00:00".to_string()),
            max_timestamp: Some("2026-05-23 00:00:01".to_string()),
            pricing_rows,
            pricing_sources: 0,
            pricing_min_updated_at: None,
            pricing_max_updated_at: None,
        }
    }

    #[test]
    fn pricing_warnings_distinguish_unknown_api_cost_from_subscription_zero() {
        let subscription_only = cost_coverage_for_test(20, 0, 0, 0, 20);
        assert!(unpriced_models_finding(&subscription_only).is_none());
        assert!(pricing_staleness_finding(&subscription_only).is_none());

        let api_unknown = cost_coverage_for_test(20, 20, 12, 0, 0);
        let unpriced = unpriced_models_finding(&api_unknown).unwrap();
        assert_eq!(unpriced.check_name, "unpriced_models");
        assert!(
            unpriced
                .message
                .contains("unknown cost_usd (NULL), not zero")
        );

        let stale = pricing_staleness_finding(&api_unknown).unwrap();
        assert_eq!(stale.check_name, "pricing_staleness");
        assert!(stale.message.contains("llm_pricing table is empty"));
    }

    #[test]
    fn rate_check_alerts_at_threshold_then_backs_off_instead_of_every_cycle() {
        let mut state = AuditState::new();
        let t0 = Instant::now();
        let ten_min = Duration::from_secs(600);
        assert!(
            state
                .decide_alert("c", AuditSeverity::Warning, t0)
                .is_none()
        );
        assert!(
            state
                .decide_alert("c", AuditSeverity::Warning, t0 + ten_min)
                .is_none()
        );
        assert_eq!(
            state.decide_alert("c", AuditSeverity::Warning, t0 + 2 * ten_min),
            Some(3)
        );
        assert!(
            state
                .decide_alert("c", AuditSeverity::Warning, t0 + 3 * ten_min)
                .is_none(),
            "4th consecutive fire must not alert again 10 min later"
        );
        assert!(
            state
                .decide_alert("c", AuditSeverity::Warning, t0 + 5 * ten_min)
                .is_none()
        );
        assert_eq!(
            state.decide_alert("c", AuditSeverity::Warning, t0 + 9 * ten_min),
            Some(6),
            "re-nag once an hour has passed since the alert"
        );
    }

    #[test]
    fn critical_alerts_immediately_but_still_backs_off() {
        let mut state = AuditState::new();
        let t0 = Instant::now();
        assert_eq!(
            state.decide_alert("k", AuditSeverity::Critical, t0),
            Some(1)
        );
        assert!(
            state
                .decide_alert("k", AuditSeverity::Critical, t0 + Duration::from_secs(600))
                .is_none()
        );
    }

    #[test]
    fn watchdog_waits_for_start_grace_and_does_not_repeat_every_30_minutes() {
        let mut gate = crate::alert_gate::AlertGate::new(
            Duration::from_secs(6 * 3600),
            Duration::from_secs(24 * 3600),
        );
        let t0 = Instant::now();
        assert!(
            !watchdog_should_alert(true, Duration::from_secs(600), &mut gate, t0),
            "inside 30-min start grace"
        );
        assert!(watchdog_should_alert(
            true,
            Duration::from_secs(2400),
            &mut gate,
            t0 + Duration::from_secs(1800)
        ));
        assert!(
            !watchdog_should_alert(
                true,
                Duration::from_secs(4200),
                &mut gate,
                t0 + Duration::from_secs(3600)
            ),
            "30 min later: silent"
        );
        assert!(
            !watchdog_should_alert(
                false,
                Duration::from_secs(6000),
                &mut gate,
                t0 + Duration::from_secs(5400)
            ),
            "healthy tick never alerts"
        );
        assert!(
            watchdog_should_alert(
                true,
                Duration::from_secs(7800),
                &mut gate,
                t0 + Duration::from_secs(7200)
            ),
            "healthy->stale edge alerts again"
        );
    }

    // ── reject_too_frequent_cron ─────────────────────────────────────────

    #[test]
    fn cron_every_second_rejected() {
        let result = reject_too_frequent_cron("* * * * * *");
        assert!(result.is_some(), "should reject every-second schedule");
    }

    #[test]
    fn cron_hourly_accepted() {
        let result = reject_too_frequent_cron("0 * * * *");
        assert!(result.is_none(), "hourly schedule should be accepted");
    }

    #[test]
    fn cron_every_5_minutes_accepted() {
        let result = reject_too_frequent_cron("*/5 * * * *");
        assert!(result.is_none(), "every-5-min schedule should be accepted");
    }

    #[test]
    fn cron_cadence_check_rejects_parser_errors() {
        let result = reject_too_frequent_cron("99 * * * *");

        assert!(
            result
                .as_deref()
                .is_some_and(|msg| msg.contains("cron schedule '99 * * * *' is invalid")),
            "invalid cron should be reported, got {result:?}"
        );
    }
}
