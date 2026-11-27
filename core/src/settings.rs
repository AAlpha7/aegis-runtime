use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Subscription provider the owner binds to foreground Aegis chat, either from
/// Settings → Connect → **Use for chat** or from the pre-boot
/// `aegis_runtime_core setup-auth` step. Both paths persist exactly
/// [`ChatProviderSelection::runtime_settings`], so a machine set up before the
/// first boot ends in the same state as one bound from the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatProviderSelection {
    Codex,
    Grok,
}

impl ChatProviderSelection {
    /// Value stored in `llm_provider`.
    pub fn provider_key(self) -> &'static str {
        match self {
            Self::Codex => "codex_oauth",
            Self::Grok => "xai_subscription",
        }
    }

    pub fn owner_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Grok => "Grok",
        }
    }

    /// Every runtime setting the selection writes, in write order. Background
    /// use and paid API fallback stay off: the owner opted into one
    /// subscription for foreground chat, nothing more.
    pub fn runtime_settings(self) -> Vec<(&'static str, Value)> {
        vec![
            (
                "llm_provider",
                Value::String(self.provider_key().to_string()),
            ),
            // Empty = provider default (`xai_subscription_model` / `codex_model`).
            // Clears any OpenRouter bootstrap slug copied from .env.example so
            // configured_model logs match the routed Grok/Codex model.
            ("llm_model", Value::String(String::new())),
            ("codex_enabled", Value::Bool(self == Self::Codex)),
            ("xai_subscription_enabled", Value::Bool(self == Self::Grok)),
            ("external_executor_enabled", Value::Bool(true)),
            ("external_executor_primary", Value::String("auto".into())),
            ("external_executor_fallback", Value::String("none".into())),
            ("web_search_provider", Value::String("auto".into())),
            ("web_search_model", Value::String(String::new())),
            ("codex_allow_background_tasks", Value::Bool(false)),
            (
                "xai_subscription_allow_background_tasks",
                Value::Bool(false),
            ),
            ("codex_background_task_allowlist", Value::Array(vec![])),
            (
                "xai_subscription_background_task_allowlist",
                Value::Array(vec![]),
            ),
            ("paid_api_fallback_enabled", Value::Bool(false)),
            ("prompt_optimization_allow_paid_api", Value::Bool(false)),
            ("skillfile_optimization_allow_paid_api", Value::Bool(false)),
        ]
    }
}

/// Parse timezone string with logged fallback to UTC.
/// Default `CODEX_HOME` for the Codex CLI adapter.
///
/// Was hardcoded to `/home/aegis-runtime/.codex` — the author's service account.
/// Deriving it from the environment keeps that exact value on the original
/// deployment (its unit sets `HOME=/home/aegis-runtime`) while being correct for
/// anyone else who runs this.
pub fn default_codex_home() -> String {
    codex_home_from(
        std::env::var("CODEX_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// Pure core of [`default_codex_home`], so the precedence is testable without
/// mutating process environment (which races under a parallel test runner).
pub(crate) fn codex_home_from(codex_home: Option<&str>, home: Option<&str>) -> String {
    if let Some(explicit) = codex_home
        && !explicit.trim().is_empty()
    {
        return explicit.to_string();
    }
    match home {
        Some(h) if !h.trim().is_empty() => format!("{}/.codex", h.trim_end_matches('/')),
        _ => ".codex".to_string(),
    }
}

pub fn parse_tz_str(tz_str: &str) -> chrono_tz::Tz {
    match tz_str.parse::<chrono_tz::Tz>() {
        Ok(tz) => tz,
        Err(_) => {
            tracing::warn!(timezone = %tz_str, "[Settings] Invalid timezone, falling back to UTC");
            chrono_tz::Tz::UTC
        }
    }
}

/// Parse user timezone from RuntimeSettings with logged fallback.
///
/// NOTE: this is the **scheduling/general** resolver and intentionally
/// preserves its historical fallback chain (`.get("user_timezone")` →
/// `parse_tz_str` → UTC on an unparseable value). It backs non-display paths
/// such as notification quiet-hours and reminder-snooze target math, so its
/// behavior is deliberately NOT changed by the user-timezone display work.
/// With nothing stored, `.get` yields the registry default, which is the
/// host zone ([`user_tz_default`]).
///
/// For **user-facing display** strings (WebApp projection, Telegram archive
/// dates), use [`resolve_display_user_tz`] / [`resolve_user_tz_string`], which
/// additionally honor the `USER_TIMEZONE` env var and substitute the host-zone
/// default for an invalid stored value.
pub fn parse_user_tz(settings: &RuntimeSettings) -> chrono_tz::Tz {
    let tz_str = settings
        .get("user_timezone")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "UTC".to_string());
    parse_tz_str(&tz_str)
}

/// Display-only timezone resolver: the parsed `Tz` form of
/// [`resolve_user_tz_string`] (DB-stored → `USER_TIMEZONE` env → host zone →
/// UTC). Use this for user-facing display formatting so an invalid/skewed
/// stored value never leaks UTC into a human-read string on a host that has
/// a zone, while leaving scheduling math ([`parse_user_tz`]) untouched.
pub fn resolve_display_user_tz(settings: &RuntimeSettings) -> chrono_tz::Tz {
    parse_tz_str(&resolve_user_tz_string(settings))
}

/// Resolve the user's timezone as an IANA string, applying the canonical
/// priority chain: DB-stored `user_timezone` setting → `USER_TIMEZONE` env
/// → host zone → UTC (see [`user_tz_default`]). Uses
/// [`RuntimeSettings::get_if_stored`] so the registered registry default does
/// NOT shadow the `USER_TIMEZONE` env var (see the paired-fix note in the
/// tests below).
///
/// Unlike [`parse_user_tz`], this returns the raw IANA string (validated to
/// parse as a `chrono_tz::Tz`; on an invalid stored value it logs and falls
/// back to the default). The string form is what the webapp needs to pass to
/// `Intl.DateTimeFormat({ timeZone })` so user-facing times render in the
/// owner's timezone rather than the browser's.
pub fn resolve_user_tz_string(settings: &RuntimeSettings) -> String {
    resolve_user_tz_string_with_env(settings, std::env::var("USER_TIMEZONE").ok().as_deref())
}

/// Last resort when the host's zone is unknown or not an IANA name.
pub(crate) const LAST_RESORT_TZ: &str = "UTC";

/// Default IANA timezone when neither a DB-stored setting nor `USER_TIMEZONE`
/// env is present: the host's zone as reported by `iana-time-zone`
/// (`/etc/timezone`, `/etc/localtime`, …), or UTC when that is unknown or not
/// a name `chrono_tz` recognises. This is also the registry default for
/// `user_timezone`, so [`parse_user_tz`] and the display resolvers agree.
/// Production sets `USER_TIMEZONE` explicitly, so this only decides the zone
/// on hosts that never configured one. Resolved once per process.
pub fn user_tz_default() -> String {
    static HOST_ZONE: LazyLock<String> =
        LazyLock::new(|| user_tz_default_from(iana_time_zone::get_timezone().ok().as_deref()));
    HOST_ZONE.clone()
}

/// Pure core of [`user_tz_default`] with the host lookup injected.
pub(crate) fn user_tz_default_from(host_zone: Option<&str>) -> String {
    match host_zone.map(str::trim).filter(|zone| !zone.is_empty()) {
        Some(zone) if zone.parse::<chrono_tz::Tz>().is_ok() => zone.to_string(),
        Some(zone) => {
            tracing::warn!(
                timezone = %zone,
                "[Settings] host timezone is not an IANA zone name, using UTC as the default"
            );
            LAST_RESORT_TZ.to_string()
        }
        None => LAST_RESORT_TZ.to_string(),
    }
}

/// [`resolve_user_tz_string`] with the env value injected so it is testable
/// without mutating process env (the repo denies `unsafe`, which
/// `std::env::set_var` requires on 2024 edition).
pub(crate) fn resolve_user_tz_string_with_env(
    settings: &RuntimeSettings,
    env_value: Option<&str>,
) -> String {
    resolve_user_tz_string_from(settings, env_value, &user_tz_default())
}

/// Pure core of [`resolve_user_tz_string`]: the env value and the terminal
/// default (normally [`user_tz_default`]) are both injected.
pub(crate) fn resolve_user_tz_string_from(
    settings: &RuntimeSettings,
    env_value: Option<&str>,
    default_zone: &str,
) -> String {
    let candidate = settings
        .get_if_stored("user_timezone")
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            env_value
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_zone.to_string());

    // Validate it parses as a real IANA zone; if not, fall back to the
    // default rather than emit a string the frontend's Intl would reject.
    if candidate.parse::<chrono_tz::Tz>().is_ok() {
        candidate
    } else {
        tracing::warn!(
            timezone = %candidate,
            "[Settings] resolve_user_tz_string: invalid IANA timezone, using default"
        );
        default_zone.to_string()
    }
}

// ── Static registry ─────────────────────────────────────────────────────────

/// Definition of a known runtime setting.
#[derive(Debug, Clone)]
pub struct SettingDef {
    pub key: String,
    pub default: Value,
    pub description: String,
}

/// Legacy key migration map: old hardcoded key → (new skill_<name> key, old default).
const LEGACY_MIGRATIONS: &[(&str, &str, bool)] = &[
    ("server_monitoring", "skill_server_monitor", true),
    ("news_digest", "skill_news_digest", true),
    ("calendar_briefing", "skill_calendar_reader", true),
    ("github_alerts", "skill_code_assistant", true),
];

/// Returns the legacy default for a skill, if it was one of the original 6.
pub fn legacy_skill_default(skill_name: &str) -> Option<bool> {
    let key = format!("skill_{}", skill_name);
    LEGACY_MIGRATIONS
        .iter()
        .find(|&&(_, new, _)| new == key.as_str())
        .map(|&(_, _, default)| default)
}

/// Returns the system (non-skill) setting definitions.
/// Default assistant call-name (owner decision 2026-09-08): 安琪, "Angie" in English UIs.
pub const DEFAULT_ASSISTANT_NAME_ZH: &str = "安琪";
pub const DEFAULT_ASSISTANT_NAME_EN: &str = "Angie";

pub fn system_setting_definitions() -> Vec<SettingDef> {
    vec![
        SettingDef {
            key: "llm_model".to_string(),
            default: Value::String(String::new()),
            description: "LLM模型 (active language model; empty = provider default)".to_string(),
        },
        SettingDef {
            key: "llm_provider".to_string(),
            default: Value::String("codex_oauth".to_string()),
            description: "LLM provider backend (codex_oauth, xai_subscription/grok_subscription, openrouter, openai_api, openai_compat, local). Unknown values fail closed; current Phase 1 preserves existing direct OpenAI-compatible call sites until router migration.".to_string(),
        },
        SettingDef {
            key: "paid_api_fallback_enabled".to_string(),
            default: Value::Bool(false),
            description: "Allow fallback to paid OpenAI-compatible API calls when subscription-backed Codex OAuth is unavailable. Default false to prevent unexpected API spend.".to_string(),
        },
        SettingDef {
            key: "prompt_optimization_allow_paid_api".to_string(),
            default: Value::Bool(false),
            description: "Allow PromptOptimization to spend paid API credits. Default false. paid_api_fallback_enabled does not grant this class. A denied call fails closed.".to_string(),
        },
        SettingDef {
            key: "skillfile_optimization_allow_paid_api".to_string(),
            default: Value::Bool(false),
            description: "Allow SkillOptimization to spend paid API credits. Default false. paid_api_fallback_enabled does not grant this class. A denied call fails closed and does not write a live SKILL.md. The key is skillfile_ rather than skill_ so it is not treated as a heartbeat skill toggle.".to_string(),
        },
        SettingDef {
            key: "codex_enabled".to_string(),
            default: Value::Bool(false),
            description: "Enable personal Codex OAuth provider for foreground user-initiated tasks. Default false until a real backend is configured.".to_string(),
        },
        SettingDef {
            key: "codex_model".to_string(),
            default: Value::String("gpt-5.5".to_string()),
            description: "Official Codex CLI model identifier for foreground reasoning/coding tasks.".to_string(),
        },
        SettingDef {
            key: "codex_reasoning_effort".to_string(),
            default: Value::String("medium".to_string()),
            description: "Requested Codex reasoning effort for foreground reasoning where the selected Codex model supports it.".to_string(),
        },
        SettingDef {
            key: "codex_foreground_only".to_string(),
            default: Value::Bool(true),
            description: "Restrict Codex OAuth to foreground user/coding requests by default.".to_string(),
        },
        SettingDef {
            key: "codex_allow_background_tasks".to_string(),
            default: Value::Bool(false),
            description: "Allow Codex OAuth for autonomous background tasks. Default false to protect subscription usage and account safety.".to_string(),
        },
        SettingDef {
            key: "codex_background_task_allowlist".to_string(),
            default: Value::Array(vec![]),
            description: "Background LlmTaskClass names allowed to use Codex when codex_allow_background_tasks is false. Example: [\"prompt_optimization\"] or [\"skill_optimization\"]. Unknown names are ignored. Default empty. Does not enable paid API fallback. Granting one class does not grant the other.".to_string(),
        },
        SettingDef {
            key: "codex_max_concurrency".to_string(),
            default: Value::Number(serde_json::Number::from(1)),
            description: "Maximum concurrent Codex OAuth requests. Keep low for personal subscription safety.".to_string(),
        },
        SettingDef {
            key: "codex_daily_request_limit".to_string(),
            default: Value::Number(serde_json::Number::from(100)),
            description: "Local daily request cap for Codex OAuth requests; this is a local safety cap, not the provider limit.".to_string(),
        },
        SettingDef {
            key: "codex_cli_binary".to_string(),
            default: Value::String("/usr/bin/codex".to_string()),
            description: "Path to the official OpenAI Codex CLI used by the local Codex OAuth adapter.".to_string(),
        },
        SettingDef {
            key: "codex_cli_home".to_string(),
            default: Value::String(default_codex_home()),
            description: "CODEX_HOME directory for the service account's official Codex CLI login cache.".to_string(),
        },
        SettingDef {
            key: "codex_cli_workdir".to_string(),
            default: Value::String("/tmp".to_string()),
            description: "Read-only working directory for Codex CLI foreground chat adapter runs.".to_string(),
        },
        SettingDef {
            key: "codex_cli_timeout_secs".to_string(),
            default: Value::Number(serde_json::Number::from(90)),
            description: "Timeout for one Codex CLI foreground chat adapter request.".to_string(),
        },
        SettingDef {
            key: "codex_retry_models".to_string(),
            default: Value::String(String::new()),
            description: "Comma-separated Codex app-server models to try after the requested one fails (empty = none).".to_string(),
        },
        SettingDef {
            key: "external_executor_enabled".to_string(),
            default: Value::Bool(false),
            description: "外部执行层 (run_external via grok build / codex)".to_string(),
        },
        SettingDef {
            key: "external_executor_primary".to_string(),
            default: Value::String("auto".to_string()),
            description: "External executor: auto follows the selected subscription; grok or codex explicitly overrides it.".to_string(),
        },
        SettingDef {
            key: "external_executor_fallback".to_string(),
            default: Value::String("none".to_string()),
            description: "Explicit fallback when the primary fails to spawn (grok|codex|none). Default none.".to_string(),
        },
        SettingDef {
            key: "external_executor_timeout_secs".to_string(),
            default: Value::Number(serde_json::Number::from(600)),
            description: "Default timeout in seconds for one run_external task (clamped 30–3600).".to_string(),
        },
        SettingDef {
            key: "xai_subscription_enabled".to_string(),
            default: Value::Bool(false),
            description: "Enable owner-authorized Grok/xAI subscription provider for foreground user-initiated tasks. Default false until an official/verifiable device-code or OAuth-style flow is configured.".to_string(),
        },
        SettingDef {
            key: "xai_subscription_model".to_string(),
            default: Value::String("grok-4.7".to_string()),
            description: "Grok/xAI subscription model identifier for foreground reasoning. This setting does not enable paid API fallback.".to_string(),
        },
        SettingDef {
            key: "web_search_provider".to_string(),
            default: Value::String("auto".to_string()),
            description: "auto follows the chat subscription (Grok or Codex). Search failures do not switch services. API search requires an explicit verified choice.".to_string(),
        },
        SettingDef {
            key: "web_search_model".to_string(),
            default: Value::String(String::new()),
            description: "Web search model. Subscription default is grok-4.7 (bare id). OpenRouter models keep the :online suffix. Does not change xai_subscription_model.".to_string(),
        },
        SettingDef {
            key: "settings_in_stable_prompt".to_string(),
            default: Value::Bool(true),
            description: "Put the settings/accounts summary in the stable (cacheable) system prompt instead of the per-turn block. Runtime A/B knob for provider prompt caching.".to_string(),
        },
        SettingDef {
            key: "volatile_context_late".to_string(),
            default: Value::Bool(true),
            description: "Place the per-turn system context (time, settings, recalled facts) after the conversation history instead of before it, so the provider prompt cache covers the stable prompt plus history. Set false to restore the old order.".to_string(),
        },
        SettingDef {
            key: "tool_list_stable".to_string(),
            default: Value::Bool(true),
            description: "Send the same tool list on every turn instead of a per-message semantic subset. A stable tool list keeps the provider prefix cache warm (2026-09-06: grok-4.6 hit 1.5% cache with the per-turn subset and spent ~30 s re-reading a 34K prompt); costs a few thousand prompt tokens.".to_string(),
        },
        SettingDef {
            key: "xai_subscription_reasoning_effort".to_string(),
            // Owner decision 2026-09-06: medium for chat (3–4 s faster per round
            // than high at the same cache hit, no visible quality loss).
            default: Value::String("medium".to_string()),
            description: "Reasoning effort sent with Grok/xAI subscription chat requests (low|medium|high). Empty omits the field; grok-4.7 accepts high.".to_string(),
        },
        SettingDef {
            key: "chat_reasoning_effort_casual".to_string(),
            default: Value::String("low".to_string()),
            description: "寒暄/简短消息的推理档位 (low|medium|high)".to_string(),
        },
        SettingDef {
            key: "chat_history_window_messages".to_string(),
            default: Value::Number(serde_json::Number::from(24)),
            description: "Number of recent L1 messages included in the chat prompt (clamped 6–200). The L1 store itself is unchanged.".to_string(),
        },
        SettingDef {
            key: "xai_subscription_api_base".to_string(),
            default: Value::String("https://api.x.ai/v1".to_string()),
            description: "xAI native API base for Grok subscription requests. Defaults to the official xAI API host.".to_string(),
        },
        SettingDef {
            key: "xai_subscription_foreground_only".to_string(),
            default: Value::Bool(true),
            description: "Restrict Grok/xAI subscription usage to foreground owner requests by default.".to_string(),
        },
        SettingDef {
            key: "xai_subscription_allow_background_tasks".to_string(),
            default: Value::Bool(false),
            description: "Allow Grok/xAI subscription for autonomous background tasks. Default false to protect subscription usage and account safety.".to_string(),
        },
        SettingDef {
            key: "xai_subscription_background_task_allowlist".to_string(),
            default: Value::Array(vec![]),
            description: "Background LlmTaskClass names allowed to use the Grok subscription when xai_subscription_allow_background_tasks is false. Grant prompt_optimization or skill_optimization alone without opening heartbeat or review. Unknown names are ignored. Default empty. A denied class fails closed and never spends paid API credits. Granting one class does not grant the other.".to_string(),
        },
        SettingDef {
            key: "xai_subscription_max_concurrency".to_string(),
            default: Value::Number(serde_json::Number::from(1)),
            description: "Maximum concurrent Grok/xAI subscription requests. Keep low for personal subscription safety.".to_string(),
        },
        SettingDef {
            key: "xai_subscription_daily_request_limit".to_string(),
            default: Value::Number(serde_json::Number::from(100)),
            description: "Local daily request cap for Grok/xAI subscription requests; this is a local safety cap, not the provider limit.".to_string(),
        },
        SettingDef {
            key: "aegis_avatar_auto_refresh_enabled".to_string(),
            default: Value::Bool(false),
            description: "Allow Aegis to refresh her own profile avatar on a weekly cadence using an explicitly connected image-capable subscription provider. Default false; no paid API fallback.".to_string(),
        },
        SettingDef {
            key: "aegis_avatar_auto_refresh_interval_days".to_string(),
            default: Value::Number(serde_json::Number::from(7)),
            description: "Minimum days between automatic Aegis avatar refreshes.".to_string(),
        },
        SettingDef {
            key: "aegis_avatar_auto_refresh_provider".to_string(),
            default: Value::String("xai_subscription".to_string()),
            description: "Preferred image capability provider for Aegis avatar refresh. Subscription-first; unsupported providers fail closed.".to_string(),
        },
        SettingDef {
            key: "aegis_avatar_auto_refresh_last_at_unix".to_string(),
            default: Value::Number(serde_json::Number::from(0)),
            description: "Last successful automatic Aegis avatar refresh timestamp.".to_string(),
        },
        SettingDef {
            key: "aegis_avatar_auto_refresh_prompt_hint".to_string(),
            default: Value::String(String::new()),
            description: "Optional owner style hint for Aegis self-generated weekly avatar refreshes.".to_string(),
        },
        SettingDef {
            key: "user_timezone".to_string(),
            default: Value::String(user_tz_default()),
            description: "用户时区 (IANA timezone, e.g. Asia/Shanghai, America/New_York, Europe/London；缺省为主机时区)".to_string(),
        },
        SettingDef {
            key: "notification_quiet_start".to_string(),
            default: Value::Number(serde_json::Number::from(255)),
            description: "通知免打扰开始时 (hour 0-23 in user_timezone, 255=disabled). 在此时段内仅 Urgent 通知会发送".to_string(),
        },
        SettingDef {
            key: "notification_quiet_end".to_string(),
            default: Value::Number(serde_json::Number::from(255)),
            description: "通知免打扰结束时 (hour 0-23 in user_timezone, 255=disabled)".to_string(),
        },
        SettingDef {
            key: "notification_daily_max".to_string(),
            default: Value::Number(serde_json::Number::from(20)),
            description: "每日通知上限 (超出后降为 Routine 级别，仅记录不发送)".to_string(),
        },
        SettingDef {
            key: "fallback_model".to_string(),
            default: Value::String(String::new()),
            description: "备用LLM模型 (fallback model on OpenRouter when primary is unavailable; empty = disabled)".to_string(),
        },
        SettingDef {
            key: "fallback_model_authorized".to_string(),
            default: Value::Bool(false),
            description: "授权自动切换备用模型 (must be true for auto-fallback; false = error with hint)".to_string(),
        },
        SettingDef {
            key: "vision_routing_enabled".to_string(),
            default: Value::Bool(true),
            description: "图片识别路由 (route messages containing images to vision_model for better OCR; falls back to active llm_model when disabled)".to_string(),
        },
        SettingDef {
            key: "vision_model".to_string(),
            default: Value::String(String::new()),
            description: "图片识别模型 (model used when vision_routing_enabled=true and the turn contains an image; empty = provider default / no dedicated vision override)".to_string(),
        },
        SettingDef {
            key: "health_probe_interval_minutes".to_string(),
            default: Value::Number(serde_json::Number::from(60)),
            description: "上游健康探针间隔 (minutes between OpenRouter/Calendar health checks; 0=disabled)".to_string(),
        },
        SettingDef {
            key: "notification_aggregation_window_secs".to_string(),
            default: Value::Number(serde_json::Number::from(300)),
            description: "通知聚合窗口 (seconds to buffer non-Urgent notifications before combined delivery; 0=immediate)".to_string(),
        },
        SettingDef {
            key: "profile_missing_slot_hints".to_string(),
            default: Value::Bool(true),
            description: "Profile缺失提示 (inject unpopulated canonical Profile slots into system prompt so LLM can ask about them naturally)".to_string(),
        },
        SettingDef {
            key: "summarization_model".to_string(),
            default: Value::String("".to_string()),
            description: "摘要模型 (model for context summarization; empty = use active LLM model)".to_string(),
        },
        SettingDef {
            key: "calendar_invite_others_enabled".to_string(),
            default: Value::Bool(false),
            description: "允许发送日历邀请给他人 (when false, calendar_create_event / calendar_update_event reject requests whose `attendees` contain anyone other than the user's own email; default false — user must explicitly enable before the LLM can invite non-self attendees)".to_string(),
        },
        SettingDef {
            key: "daily_brief_schedule_enabled".to_string(),
            default: Value::Bool(true),
            description: "定时日报发送 (deterministic Daily Brief delivery to the Web owner timeline; default true after explicit user approval, no LLM call)".to_string(),
        },
        SettingDef {
            key: "daily_brief_local_time".to_string(),
            default: Value::String("09:00".to_string()),
            description: "定时日报本地时间 (HH:MM in user_timezone for Daily Brief delivery when daily_brief_schedule_enabled is true)".to_string(),
        },
        SettingDef {
            key: "evening_review_schedule_enabled".to_string(),
            default: Value::Bool(true),
            description: "定时晚间回顾发送 (deterministic Evening Review delivery to the Web owner timeline; default true after explicit user approval, no LLM call)".to_string(),
        },
        SettingDef {
            key: "evening_review_local_time".to_string(),
            default: Value::String("18:00".to_string()),
            description: "定时晚间回顾本地时间 (HH:MM in user_timezone for Evening Review delivery when evening_review_schedule_enabled is true)".to_string(),
        },
        SettingDef {
            key: "mcp_enabled".to_string(),
            default: Value::Bool(false),
            description: "MCP协议 (enable external MCP server tools; requires per-server approval on first use)".to_string(),
        },
        SettingDef {
            key: "mcp_approved_servers".to_string(),
            default: Value::String(String::new()),
            description: "已批准的MCP服务器 (comma-separated list of approved MCP server keys; managed via HITL approval flow)".to_string(),
        },
        SettingDef {
            key: "review_model".to_string(),
            default: Value::String("".to_string()),
            description: "审查模型 Review model (agent review guard; empty = use active LLM model)".to_string(),
        },
        SettingDef {
            key: "display_language".to_string(),
            default: Value::String("zh".to_string()),
            description: "显示语言 (language for system output and notifications: zh, en, ja, etc.)".to_string(),
        },
        SettingDef {
            key: "assistant_display_name".to_string(),
            default: Value::String(DEFAULT_ASSISTANT_NAME_ZH.to_string()),
            description: "对话助理的称呼（默认 安琪；英文界面显示 Angie）。Aegis 仍是系统名。".to_string(),
        },
        SettingDef {
            key: "true_heartbeat_config".to_string(),
            default: serde_json::json!({
                "enabled": false,
                "interval_minutes": 15,
                "autonomous_overrides": null
            }),
            description: "真心跳配置 (true heartbeat: fixed-interval LLM system review; JSON object with enabled, interval_minutes, autonomous_overrides). Quiet hours are now in heartbeat_config.".to_string(),
        },
        SettingDef {
            key: "heartbeat_config".to_string(),
            default: serde_json::to_value(crate::heartbeat_config::HeartbeatConfig::default())
                .unwrap_or(Value::Null),
            description: "心跳Agent配置 (heartbeat agent-loop config: frequency, quiet hours, per-tick budgets, dedup threshold, controlled_thaw guardrails; see docs/heartbeat-autonomy-design.md §7)".to_string(),
        },
        SettingDef {
            key: "cost_saving_pause_snapshot".to_string(),
            default: serde_json::json!({}),
            description: "Cost-saving pause snapshot (JSON object of owner-approved background settings captured before pausing so Settings can restore them explicitly).".to_string(),
        },
        SettingDef {
            key: "heartbeat_memo".to_string(),
            default: Value::String(String::new()),
            description: "心跳便签 (heartbeat scratchpad: persistent memo updated by system after each cycle; archived to L2 during quiet hours)".to_string(),
        },
        SettingDef {
            key: "model_audio_input".to_string(),
            default: Value::String("auto".to_string()),
            description: "模型音频输入 ('auto'=自动检测, 'true'=强制原生音频, 'false'=始终STT)".to_string(),
        },
        SettingDef {
            key: "model_capability_overrides".to_string(),
            default: serde_json::json!({}),
            description: "模型能力覆盖 (JSON object keyed by model id, with input_modalities/output_modalities arrays; used for multimodal routing)".to_string(),
        },
        // ── Budget alert settings ────────────────────────────────────────
        SettingDef {
            key: "budget_daily_usd".to_string(),
            default: serde_json::json!(5.0),
            description: "日API预算 (daily USD budget for LLM API calls)".to_string(),
        },
        SettingDef {
            key: "budget_weekly_usd".to_string(),
            default: serde_json::json!(25.0),
            description: "周API预算 (weekly USD budget)".to_string(),
        },
        SettingDef {
            key: "budget_monthly_usd".to_string(),
            default: serde_json::json!(80.0),
            description: "月API预算 (monthly USD budget)".to_string(),
        },
        SettingDef {
            key: "tool_disclosure".to_string(),
            default: serde_json::Value::String("natural".to_string()),
            description: "工具展示模式 ('natural'=自然融入不显示工具名, 'debug'=回复末尾附带工具摘要, 'hidden'=完全不提及工具)".to_string(),
        },
        SettingDef {
            key: "companion_name".to_string(),
            default: Value::String(String::new()),
            description: "AI\u{4F34}\u{4FA3}\u{540D}\u{5B57} (give your companion a name; empty = unnamed companion)".to_string(),
        },
        // ── Embedding runtime reconfig (settings → env → default) ────────
        //
        // These 6 keys were env-only until 2026-04-18. They are now runtime-
        // reconfigurable via `update_setting` (HITL-gated) and hot-reload
        // into the live `EmbeddingClient` via `apply_settings_update`. In
        // production, operators do not edit .env — they ask the LLM to
        // propose a change and approve via Telegram.
        SettingDef {
            key: "embedding_primary_model".to_string(),
            default: Value::String("openai/text-embedding-3-small".to_string()),
            description: "Embedding主模型 (primary embedding model for semantic memory, HNSW, dedup). Changing this may require corpus reembed — the hot-reload path surfaces the consent alert. Startup priority: setting > EMBEDDING_MODEL env > default.".to_string(),
        },
        SettingDef {
            key: "embedding_backup_model".to_string(),
            default: Value::String(String::new()),
            description: "Embedding备份模型 (backup model for auto-failover; must share api_base/key/dimensions with primary; empty = disabled). Startup priority: setting > EMBEDDING_BACKUP_MODEL env > empty.".to_string(),
        },
        SettingDef {
            key: "embedding_dimensions".to_string(),
            default: Value::Number(serde_json::Number::from(512)),
            description: "Embedding维度 (vector dimensions; 1..16384). Changing this REQUIRES a full corpus reembed — existing vectors of any other dim are incomparable. Startup priority: setting > EMBEDDING_DIMENSIONS env > 512.".to_string(),
        },
        SettingDef {
            key: "embedding_probe_cooldown_secs".to_string(),
            default: Value::Number(serde_json::Number::from(60)),
            description: "Embedding主模型恢复探测冷却 (seconds between recovery probes during degraded mode; 1..86400). Hot-reloads into live client. Startup priority: setting > EMBEDDING_PROBE_COOLDOWN_SECS env > 60.".to_string(),
        },
        SettingDef {
            key: "embedding_promote_after_secs".to_string(),
            default: Value::Number(serde_json::Number::from(86_400)),
            description: "Embedding永久升级阈值 (seconds of continuous downtime before backup is permanently promoted to primary; 3600..604800). Hot-reloads. Startup priority: setting > EMBEDDING_PROMOTE_AFTER_SECS env > 86400.".to_string(),
        },
        SettingDef {
            key: "task_archive_after_days".to_string(),
            default: Value::Number(serde_json::Number::from(30)),
            description: "任务归档阈值 (days after a task is finished — done/fired/cancelled — before it is auto-archived out of normal query scope; default 30). Archived tasks are hidden from default listings unless explicitly queried with status='archived'.".to_string(),
        },
        SettingDef {
            key: "task_delete_archived_after_days".to_string(),
            default: Value::Number(serde_json::Number::from(180)),
            description: "归档任务保留期 (days an archived task is retained before it is permanently DELETED; default 180 ≈ 6 months). Only status='archived' rows older than this are deleted; invalid/0/negative values fall back to the 180-day default and never delete.".to_string(),
        },
    ]
}

/// Maps a Wasm skill name to its corresponding setting key.
/// All skills are now mapped via the `skill_<name>` convention.
pub fn skill_to_setting_key(skill_name: &str) -> Option<String> {
    Some(format!("skill_{}", skill_name))
}

/// Look up the human-readable description for a system setting by key.
/// Returns `None` for skill settings (dynamically registered) or unknown keys.
///
/// Used by HITL approval flows to enrich "Config Change Approval Request"
/// messages with the meaning of the setting being changed — so the user
/// sees *why* the change matters, not just the raw key.
pub fn describe_system_setting(key: &str) -> Option<String> {
    system_setting_definitions()
        .into_iter()
        .find(|def| def.key == key)
        .map(|def| def.description)
}

// ── Snapshots ───────────────────────────────────────────────────────────────

/// A serializable snapshot of a single setting (for API/prompt rendering).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingSnapshot {
    pub key: String,
    pub value: Value,
    pub default: Value,
    pub description: String,
}

/// What [`RuntimeSettings::load_from_map`] changed, so startup can write it back.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsLoadReport {
    /// `(old_key, new_key)` renames. The caller stores `new_key` and deletes `old_key`.
    pub key_renames: Vec<(String, String)>,
    /// `(key, new_value)` replacements on the same key. The caller stores
    /// `new_value` and must not delete the key.
    pub value_rewrites: Vec<(String, Value)>,
}

/// Retired code defaults for `xai_subscription_model`.
///
/// Only these exact ids move to the current registry default. The same strings
/// on any other key, and any other subscription model the owner stored, stay.
fn retired_xai_subscription_model_replacement(
    key: &str,
    value: &Value,
    default_model: &str,
) -> Option<Value> {
    if key != "xai_subscription_model" {
        return None;
    }
    let id = value.as_str()?;
    if (id == "grok-4.3" || id == "grok-4.6") && id != default_model {
        Some(Value::String(default_model.to_string()))
    } else {
        None
    }
}

// ── RuntimeSettings ─────────────────────────────────────────────────────────

/// Thread-safe, in-memory cache of user-configurable runtime settings.
/// Backed by SQLite for persistence (via MemoryHandle), but all reads
/// hit the RwLock cache for zero-latency access in hot paths.
pub struct RuntimeSettings {
    values: RwLock<HashMap<String, Value>>,
    /// Dynamic registry of setting definitions (system + skill toggles).
    registry: RwLock<Vec<SettingDef>>,
    /// Keys that have been EXPLICITLY stored by the user/app (via `set` or
    /// `load_from_map`), as distinct from keys still holding their seeded
    /// registry default from `new()`. Used by `get_if_stored()` to enforce
    /// the startup priority chain (DB-stored → env → default) correctly —
    /// without this distinction, the pre-seeded default in `values` would
    /// masquerade as a user-configured value and shadow the env fallback
    /// (the 2026-04-18 production bug).
    stored_keys: RwLock<HashSet<String>>,
}

/// Validated before enqueueing a database write. Publication cannot fail after
/// commit; the registry only adds definitions and never removes them.
pub(crate) struct PreparedSetting {
    pub key: String,
    pub stored: Option<Value>,
    pub effective: Value,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeSettings {
    /// Creates a new RuntimeSettings populated with system defaults.
    ///
    /// Note: the `values` map is pre-seeded with registered defaults so that
    /// hot-path reads via [`Self::get`] never return `None` for a known key.
    /// The `stored_keys` set stays empty — callers who need to distinguish
    /// "user explicitly stored this" from "this is the default" MUST use
    /// [`Self::get_if_stored`].
    pub fn new() -> Self {
        let defs = system_setting_definitions();
        let mut map = HashMap::new();
        for def in &defs {
            map.insert(def.key.clone(), def.default.clone());
        }
        Self {
            values: RwLock::new(map),
            registry: RwLock::new(defs),
            stored_keys: RwLock::new(HashSet::new()),
        }
    }

    /// Register a skill toggle setting. If already registered, this is a no-op.
    /// Returns the setting key (e.g. `skill_web_reader`).
    pub fn register_skill(
        &self,
        skill_name: &str,
        description: &str,
        default_enabled: bool,
    ) -> String {
        let key = format!("skill_{}", skill_name);
        let default = Value::Bool(default_enabled);

        // Check if already registered
        {
            let reg = self.registry.read();
            if reg.iter().any(|d| d.key == key) {
                return key;
            }
        }

        // Register the new definition
        {
            let mut reg = self.registry.write();
            // Double-check after acquiring write lock
            if reg.iter().any(|d| d.key == key) {
                return key;
            }
            reg.push(SettingDef {
                key: key.clone(),
                default: default.clone(),
                description: description.to_string(),
            });
        }

        // Set default value if not already present
        {
            let mut vals = self.values.write();
            vals.entry(key.clone()).or_insert(default);
        }

        key
    }

    /// Returns the effective value for `key`: the stored value if one was
    /// explicitly set, otherwise the registered default. Returns `None` only
    /// if the key is not in the registry at all.
    ///
    /// This reflects the EFFECTIVE current value (explicit user store OR the
    /// seeded registry default — they are indistinguishable at this level).
    /// Use [`Self::get_if_stored`] if you need to distinguish "user set this"
    /// from "this is just the default".
    ///
    /// **If you intend to layer an env-var or other fallback AFTER this call,
    /// use [`Self::get_if_stored`] instead.** `get` always returns `Some(default)`
    /// for any registered key, so `self.get(k).or_else(|| env::var(...))` will
    /// silently shadow the env var. Discovered production bug 2026-04-18
    /// (commit 7380cf6 fixed the primary sites in `main.rs`; CTO round-1
    /// paired-fix sweep caught 7 more sibling sites in the same shape).
    pub fn get(&self, key: &str) -> Option<Value> {
        self.values.read().get(key).cloned()
    }

    /// Returns the value ONLY IF it was explicitly stored by the user or by
    /// bulk-load from the persistence layer — NOT if the key is still
    /// holding its seeded registry default.
    ///
    /// This is the correct lookup for startup-priority chains of the form:
    ///
    /// ```text
    /// DB-stored value  >  env var  >  registry default
    /// ```
    ///
    /// Using [`Self::get`] for such chains is a bug: because `new()` pre-
    /// populates `values` with every registered default, `get` always
    /// returns `Some(default)` and the env-var fallback is never reached.
    /// That exact mistake silently shipped the wrong embedding model to
    /// production on 2026-04-18 (commit 519469c). Prefer `get_if_stored`
    /// at any site that intends to fall back to a different source.
    pub fn get_if_stored(&self, key: &str) -> Option<Value> {
        let stored = self.stored_keys.read();
        if !stored.contains(key) {
            return None;
        }
        self.values.read().get(key).cloned()
    }

    /// Test-only: returns `true` if the key has been explicitly stored (not
    /// just seeded with a default).
    #[cfg(test)]
    pub fn is_stored(&self, key: &str) -> bool {
        self.stored_keys.read().contains(key)
    }

    /// Convenience method: returns `true` if the setting is truthy.
    /// Handles both `Value::Bool(true)` and `Value::String("true")` (LLM may send
    /// boolean settings as strings, which survive SQLite roundtrip as strings).
    /// For unknown keys, returns the registry default if it exists.
    /// Unknown `skill_*` keys default to `true` (unregistered = not gated = always enabled).
    /// Other unknown keys default to `false`.
    pub fn is_enabled(&self, key: &str) -> bool {
        match self.get(key) {
            Some(Value::Bool(b)) => b,
            Some(Value::String(ref s)) => s.eq_ignore_ascii_case("true"),
            _ => {
                // Fall back to registry default
                self.registry
                    .read()
                    .iter()
                    .find(|d| d.key == key)
                    .and_then(|d| d.default.as_bool())
                    .unwrap_or_else(|| key.starts_with("skill_"))
            }
        }
    }

    /// The assistant's call-name for the given UI locale. The default pair is
    /// 安琪 / Angie; a custom value applies to every locale. "Aegis" stays the
    /// system name regardless.
    pub fn assistant_display_name(&self, locale: &str) -> String {
        let custom = self
            .get("assistant_display_name")
            .and_then(|v| v.as_str().map(str::trim).map(String::from))
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_ASSISTANT_NAME_ZH.to_string());
        if custom == DEFAULT_ASSISTANT_NAME_ZH && locale.starts_with("en") {
            DEFAULT_ASSISTANT_NAME_EN.to_string()
        } else {
            custom
        }
    }

    /// Returns the user's preferred display language (e.g. "zh", "en").
    /// Used by prompt modules to instruct LLM on output language.
    pub fn display_language(&self) -> String {
        self.get("display_language")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "zh".to_string())
    }

    /// Returns a human-readable language name for prompt injection.
    pub fn display_language_name(&self) -> &'static str {
        let lang = self.display_language();
        match lang.as_str() {
            "zh-CN" | "zh" | "zh-TW" => "中文 (Chinese)",
            "en" | "en-US" | "en-GB" => "English",
            "ja" => "日本語 (Japanese)",
            "ko" => "한국어 (Korean)",
            "es" => "Español (Spanish)",
            "fr" => "Français (French)",
            "de" => "Deutsch (German)",
            _ => "中文 (Chinese)", // default fallback
        }
    }

    /// Returns a numeric setting clamped to a safe range.
    ///
    /// Accepts JSON numbers and numeric strings because settings can arrive
    /// through LLM tools, SQLite, and admin scripts. Invalid values fall back to
    /// `default`.
    pub fn get_u64_clamped(&self, key: &str, default: u64, min: u64, max: u64) -> u64 {
        let raw = self.get(key);
        let parsed = match raw {
            Some(Value::Number(n)) => n.as_u64(),
            Some(Value::String(s)) => s.trim().parse::<u64>().ok(),
            _ => None,
        }
        .unwrap_or(default);

        parsed.clamp(min, max)
    }

    /// Sets a setting value. Returns `Err` if the key is not in the registry.
    /// Returns the old value on success.
    pub fn set(&self, key: &str, value: Value) -> Result<Option<Value>, SettingsError> {
        let updates = self.prepare_updates(vec![(key.to_owned(), Some(value))])?;
        Ok(self.publish_updates(updates).remove(key).flatten())
    }

    pub(crate) fn prepare_updates(
        &self,
        updates: Vec<(String, Option<Value>)>,
    ) -> Result<Vec<PreparedSetting>, SettingsError> {
        let registry = self.registry.read();
        updates
            .into_iter()
            .map(|(key, value)| {
                let default = registry
                    .iter()
                    .find(|def| def.key == key)
                    .ok_or_else(|| SettingsError::UnknownKey(key.clone()))?
                    .default
                    .clone();
                let stored = value.map(|value| match &value {
                    Value::String(s) if default.is_boolean() && s.eq_ignore_ascii_case("true") => {
                        Value::Bool(true)
                    }
                    Value::String(s) if default.is_boolean() && s.eq_ignore_ascii_case("false") => {
                        Value::Bool(false)
                    }
                    _ => value,
                });
                let effective = stored.clone().unwrap_or(default);
                Ok(PreparedSetting {
                    key,
                    stored,
                    effective,
                })
            })
            .collect()
    }

    /// Publish the entire committed group under one cache lock. Always acquire
    /// stored_keys before values, matching get_if_stored and load_from_map.
    pub(crate) fn publish_updates(
        &self,
        updates: Vec<PreparedSetting>,
    ) -> HashMap<String, Option<Value>> {
        let mut stored = self.stored_keys.write();
        let mut values = self.values.write();
        let mut previous = HashMap::new();
        for update in updates {
            if update.stored.is_some() {
                stored.insert(update.key.clone());
            } else {
                stored.remove(&update.key);
            }
            let old = values.insert(update.key.clone(), update.effective);
            previous.entry(update.key).or_insert(old);
        }
        previous
    }

    /// Resets a setting to its registry default. Returns `Err` if the key is unknown.
    /// Returns the old value before reset.
    ///
    /// Also clears the "explicitly stored" marker, so subsequent
    /// [`Self::get_if_stored`] calls return `None` and any startup-priority
    /// chain will fall back to env / hard-coded default on the next read.
    pub fn reset(&self, key: &str) -> Result<Option<Value>, SettingsError> {
        let updates = self.prepare_updates(vec![(key.to_owned(), None)])?;
        Ok(self.publish_updates(updates).remove(key).flatten())
    }

    /// Returns snapshots of all settings (current value + default + description).
    pub fn all(&self) -> Vec<SettingSnapshot> {
        let vals = self.values.read();
        let reg = self.registry.read();
        reg.iter()
            .map(|def| {
                let value = vals
                    .get(&def.key)
                    .cloned()
                    .unwrap_or_else(|| def.default.clone());
                SettingSnapshot {
                    key: def.key.clone(),
                    value,
                    default: def.default.clone(),
                    description: def.description.clone(),
                }
            })
            .collect()
    }

    /// Bulk-load persisted settings from a map (e.g. from SQLite at startup).
    /// Handles legacy key migration: old keys are transparently mapped to new `skill_*` keys.
    /// Also rewrites retired `xai_subscription_model` ids onto the current default.
    /// The caller persists both kinds of change to SQLite.
    pub fn load_from_map(&self, map: HashMap<String, Value>) -> SettingsLoadReport {
        let mut key_renames = Vec::new();
        let mut value_rewrites = Vec::new();
        let mut stored = self.stored_keys.write();
        let mut vals = self.values.write();
        let reg = self.registry.read();
        let registry_keys: Vec<&str> = reg.iter().map(|d| d.key.as_str()).collect();
        let xai_model_default = reg
            .iter()
            .find(|def| def.key == "xai_subscription_model")
            .and_then(|def| def.default.as_str().map(str::to_owned))
            .unwrap_or_else(|| "grok-4.7".to_string());

        for (k, v) in map {
            // Check for legacy key migration
            if let Some(&(_, new_key, _)) = LEGACY_MIGRATIONS.iter().find(|&&(old, _, _)| old == k)
                && registry_keys.contains(&new_key)
            {
                vals.insert(new_key.to_string(), v);
                stored.insert(new_key.to_string());
                key_renames.push((k, new_key.to_string()));
                continue;
            }

            // Apply if key is in registry. Skill toggles can be registered after
            // persisted settings are loaded, so keep stored skill_* rows even
            // when the runtime has not registered that skill definition yet.
            // register_skill() later uses entry(...).or_insert(default), which
            // preserves this loaded owner override.
            if registry_keys.contains(&k.as_str()) || k.starts_with("skill_") {
                stored.insert(k.clone());
                let value = if let Some(rewritten) =
                    retired_xai_subscription_model_replacement(&k, &v, &xai_model_default)
                {
                    value_rewrites.push((k.clone(), rewritten.clone()));
                    rewritten
                } else {
                    v
                };
                vals.insert(k, value);
            }
        }

        SettingsLoadReport {
            key_renames,
            value_rewrites,
        }
    }

    /// Renders settings as human-readable text for injection into the LLM system prompt.
    /// First sentence of a setting description, capped, for the prompt: the
    /// full text lives in the admin API and the `update_setting` schema; the
    /// model only needs to know what the key means.
    fn prompt_description(description: &str) -> String {
        let first = description
            .split_inclusive(['.', '。'])
            .next()
            .unwrap_or(description)
            .trim();
        if first.chars().count() <= 90 {
            first.to_string()
        } else {
            let cut: String = first.chars().take(88).collect();
            format!("{cut}…")
        }
    }

    pub fn format_for_prompt(&self) -> String {
        let vals = self.values.read();
        let reg = self.registry.read();

        let mut lines = Vec::new();
        lines.push("## User Settings (toggle via update_setting tool)".to_string());

        // System settings first
        let system_defs = system_setting_definitions();
        let system_keys: Vec<&str> = system_defs.iter().map(|d| d.key.as_str()).collect();
        lines.push("### System".to_string());
        for def in reg.iter().filter(|d| system_keys.contains(&d.key.as_str())) {
            let current_val = vals
                .get(&def.key)
                .cloned()
                .unwrap_or_else(|| def.default.clone());
            if let Some(b) = current_val.as_bool() {
                let default = def.default.as_bool().unwrap_or(false);
                let on_off = if b { "ON" } else { "OFF" };
                let default_str = if default { "on" } else { "off" };
                lines.push(format!(
                    "- {}: {} (default: {}) — {}",
                    def.key,
                    on_off,
                    default_str,
                    Self::prompt_description(&def.description)
                ));
            } else if let Some(s) = current_val.as_str() {
                let default_str = def.default.as_str().unwrap_or("?");
                lines.push(format!(
                    "- {}: {} (default: {}) — {}",
                    def.key,
                    s,
                    default_str,
                    Self::prompt_description(&def.description)
                ));
            } else {
                // JSON object or other complex value (e.g., true_heartbeat_config)
                let compact = serde_json::to_string(&current_val).unwrap_or_else(|_| "?".into());
                lines.push(format!(
                    "- {}: {} — {}",
                    def.key,
                    compact,
                    Self::prompt_description(&def.description)
                ));
            }
        }

        // Skill toggles
        let skill_defs: Vec<&SettingDef> =
            reg.iter().filter(|d| d.key.starts_with("skill_")).collect();
        if !skill_defs.is_empty() {
            lines.push(
                "### Skill Toggles (heartbeat/proactive alerts only — on-demand calls always work)"
                    .to_string(),
            );
            for def in skill_defs {
                let current = vals
                    .get(&def.key)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let default = def.default.as_bool().unwrap_or(false);
                let on_off = if current { "ON" } else { "OFF" };
                let default_str = if default { "on" } else { "off" };
                lines.push(format!(
                    "- {}: {} (default: {}) — {}",
                    def.key,
                    on_off,
                    default_str,
                    Self::prompt_description(&def.description)
                ));
            }
        }

        lines.join("\n")
    }
}

/// Errors from the settings system.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("unknown setting key: {0}")]
    UnknownKey(String),
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_populates_system_defaults() {
        let s = RuntimeSettings::new();
        let defs = system_setting_definitions();
        for def in &defs {
            let val = s.get(&def.key);
            assert!(val.is_some(), "missing default for {}", def.key);
            assert_eq!(val.unwrap(), def.default, "wrong default for {}", def.key);
        }
    }

    #[test]
    fn set_and_get_roundtrip() {
        let s = RuntimeSettings::new();
        // calendar_invite_others_enabled defaults to false
        assert_eq!(
            s.get("calendar_invite_others_enabled"),
            Some(Value::Bool(false))
        );

        // Set to true
        let old = s
            .set("calendar_invite_others_enabled", Value::Bool(true))
            .unwrap();
        assert_eq!(old, Some(Value::Bool(false)));
        assert_eq!(
            s.get("calendar_invite_others_enabled"),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn set_rejects_unknown_key() {
        let s = RuntimeSettings::new();
        let err = s.set("nonexistent_key", Value::Bool(true));
        assert!(err.is_err());
        match err.unwrap_err() {
            SettingsError::UnknownKey(k) => assert_eq!(k, "nonexistent_key"),
        }
    }

    #[test]
    fn is_enabled_reflects_current_value() {
        let s = RuntimeSettings::new();
        // calendar_invite_others_enabled defaults to false
        assert!(!s.is_enabled("calendar_invite_others_enabled"));

        // Toggle
        s.set("calendar_invite_others_enabled", Value::Bool(true))
            .unwrap();
        assert!(s.is_enabled("calendar_invite_others_enabled"));
    }

    #[test]
    fn get_u64_clamped_reads_numeric_settings_safely() {
        let s = RuntimeSettings::new();

        assert_eq!(
            s.get_u64_clamped("aegis_avatar_auto_refresh_interval_days", 7, 5, 1440),
            7
        );

        s.set(
            "aegis_avatar_auto_refresh_interval_days",
            Value::Number(serde_json::Number::from(30)),
        )
        .unwrap();
        assert_eq!(
            s.get_u64_clamped("aegis_avatar_auto_refresh_interval_days", 7, 5, 1440),
            30
        );

        s.set(
            "aegis_avatar_auto_refresh_interval_days",
            Value::String("2".to_string()),
        )
        .unwrap();
        assert_eq!(
            s.get_u64_clamped("aegis_avatar_auto_refresh_interval_days", 7, 5, 1440),
            5
        );

        s.set(
            "aegis_avatar_auto_refresh_interval_days",
            Value::String("999999".to_string()),
        )
        .unwrap();
        assert_eq!(
            s.get_u64_clamped("aegis_avatar_auto_refresh_interval_days", 7, 5, 1440),
            1440
        );
    }

    #[test]
    fn is_enabled_unknown_key_returns_false() {
        let s = RuntimeSettings::new();
        assert!(!s.is_enabled("totally_unknown"));
    }

    #[test]
    fn reset_restores_default() {
        let s = RuntimeSettings::new();
        s.set("calendar_invite_others_enabled", Value::Bool(true))
            .unwrap();
        assert!(s.is_enabled("calendar_invite_others_enabled"));

        let old = s.reset("calendar_invite_others_enabled").unwrap();
        assert_eq!(old, Some(Value::Bool(true)));
        assert!(!s.is_enabled("calendar_invite_others_enabled")); // back to default false
    }

    #[test]
    fn reset_rejects_unknown_key() {
        let s = RuntimeSettings::new();
        assert!(s.reset("nonexistent").is_err());
    }

    #[test]
    fn all_returns_complete_snapshots() {
        let s = RuntimeSettings::new();
        let snaps = s.all();
        assert_eq!(snaps.len(), system_setting_definitions().len());
        // Check one specific
        let calendar_invite = snaps
            .iter()
            .find(|s| s.key == "calendar_invite_others_enabled")
            .unwrap();
        assert_eq!(calendar_invite.value, Value::Bool(false));
        assert_eq!(calendar_invite.default, Value::Bool(false));
        assert!(!calendar_invite.description.is_empty());
    }

    #[test]
    fn load_from_map_rewrites_only_retired_xai_subscription_model_ids() {
        for id in ["grok-4.3", "grok-4.6"] {
            let s = RuntimeSettings::new();
            let mut map = HashMap::new();
            map.insert(
                "xai_subscription_model".to_string(),
                Value::String(id.to_string()),
            );
            let report = s.load_from_map(map);
            assert_eq!(
                s.get("xai_subscription_model"),
                Some(Value::String("grok-4.7".to_string())),
                "retired id {id} must follow the current default"
            );
            assert!(s.is_stored("xai_subscription_model"));
            assert_eq!(
                report.value_rewrites,
                vec![(
                    "xai_subscription_model".to_string(),
                    Value::String("grok-4.7".to_string()),
                )],
                "startup must be told to persist the rewrite for {id}"
            );
            assert!(report.key_renames.is_empty());
        }

        let current = RuntimeSettings::new();
        let mut current_map = HashMap::new();
        current_map.insert(
            "xai_subscription_model".to_string(),
            Value::String("grok-4.7".to_string()),
        );
        let current_report = current.load_from_map(current_map);
        assert_eq!(
            current.get("xai_subscription_model"),
            Some(Value::String("grok-4.7".to_string()))
        );
        assert!(
            current_report.value_rewrites.is_empty(),
            "the current default must not be rewritten on every boot"
        );

        let s = RuntimeSettings::new();
        let mut map = HashMap::new();
        map.insert(
            "xai_subscription_model".to_string(),
            Value::String("owner-picked-model".to_string()),
        );
        map.insert(
            "llm_model".to_string(),
            Value::String("grok-4.6".to_string()),
        );
        map.insert(
            "fallback_model".to_string(),
            Value::String("grok-4.3".to_string()),
        );
        map.insert(
            "codex_model".to_string(),
            Value::String("grok-4.6".to_string()),
        );
        let report = s.load_from_map(map);
        assert!(report.value_rewrites.is_empty());
        assert!(report.key_renames.is_empty());
        assert_eq!(
            s.get("xai_subscription_model"),
            Some(Value::String("owner-picked-model".to_string())),
            "an owner-chosen subscription model stays stored"
        );
        assert_eq!(
            s.get("llm_model"),
            Some(Value::String("grok-4.6".to_string())),
            "a stored chat override is not a retired subscription default"
        );
        assert_eq!(
            s.get("fallback_model"),
            Some(Value::String("grok-4.3".to_string()))
        );
        assert_eq!(
            s.get("codex_model"),
            Some(Value::String("grok-4.6".to_string()))
        );
    }

    #[test]
    fn load_from_map_applies_known_keys_only() {
        let s = RuntimeSettings::new();
        let mut map = HashMap::new();
        map.insert(
            "calendar_invite_others_enabled".to_string(),
            Value::Bool(true),
        );
        map.insert("unknown_garbage".to_string(), Value::Bool(true));

        let report = s.load_from_map(map);

        // Known key was applied
        assert!(s.is_enabled("calendar_invite_others_enabled"));
        // Unknown key was ignored
        assert!(s.get("unknown_garbage").is_none());
        // No migrations needed
        assert!(report.key_renames.is_empty());
        assert!(report.value_rewrites.is_empty());
    }

    #[test]
    fn prompt_descriptions_are_first_sentence_and_capped() {
        assert_eq!(
            RuntimeSettings::prompt_description("Short one. Long explanation follows."),
            "Short one."
        );
        assert_eq!(
            RuntimeSettings::prompt_description("中文说明。后面还有很长的解释"),
            "中文说明。"
        );
        let long = "x".repeat(200);
        let out = RuntimeSettings::prompt_description(&long);
        assert!(out.chars().count() <= 90, "{}", out.chars().count());
        assert!(out.ends_with('…'));
    }

    #[test]
    fn format_for_prompt_contains_all_keys() {
        let s = RuntimeSettings::new();
        let prompt = s.format_for_prompt();
        for def in system_setting_definitions() {
            assert!(prompt.contains(&def.key), "prompt missing {}", def.key);
            assert!(
                prompt.contains(&RuntimeSettings::prompt_description(&def.description)),
                "prompt missing (shortened) description for {}",
                def.key
            );
        }
        assert!(prompt.contains("ON") || prompt.contains("OFF"));
    }

    #[test]
    fn skill_to_setting_key_returns_skill_prefix() {
        assert_eq!(
            skill_to_setting_key("web_reader"),
            Some("skill_web_reader".to_string())
        );
        assert_eq!(
            skill_to_setting_key("server_monitor"),
            Some("skill_server_monitor".to_string())
        );
        assert_eq!(
            skill_to_setting_key("reminder"),
            Some("skill_reminder".to_string())
        );
    }

    // ── Dynamic registry tests (TDD) ────────────────────────────────────

    #[test]
    fn register_skill_adds_toggle() {
        let s = RuntimeSettings::new();
        let key = s.register_skill("reminder", "提醒技能 (reminder skill)", true);
        assert_eq!(key, "skill_reminder");

        // Should be enabled by default
        assert!(s.is_enabled("skill_reminder"));

        // Should appear in all()
        let snaps = s.all();
        let reminder = snaps.iter().find(|s| s.key == "skill_reminder");
        assert!(reminder.is_some(), "skill_reminder must appear in all()");
        assert_eq!(reminder.unwrap().value, Value::Bool(true));
    }

    #[test]
    fn register_skill_idempotent() {
        let s = RuntimeSettings::new();
        s.register_skill("web_reader", "desc", true);
        // Set to false
        s.set("skill_web_reader", Value::Bool(false)).unwrap();
        // Re-register should NOT reset value
        s.register_skill("web_reader", "desc2", true);
        assert!(!s.is_enabled("skill_web_reader"));
    }

    #[test]
    fn register_skill_allows_set_and_reset() {
        let s = RuntimeSettings::new();
        s.register_skill("http_api", "HTTP API skill", true);

        // Can toggle off
        s.set("skill_http_api", Value::Bool(false)).unwrap();
        assert!(!s.is_enabled("skill_http_api"));

        // Can reset to default
        s.reset("skill_http_api").unwrap();
        assert!(s.is_enabled("skill_http_api"));
    }

    #[test]
    fn register_skill_appears_in_prompt() {
        let s = RuntimeSettings::new();
        s.register_skill("calendar_reader", "日历读取 (calendar reader)", true);
        let prompt = s.format_for_prompt();
        assert!(
            prompt.contains("skill_calendar_reader"),
            "prompt must contain skill key"
        );
        assert!(
            prompt.contains("Skill Toggles"),
            "prompt must have skill section"
        );
    }

    #[test]
    fn legacy_migration_maps_old_keys() {
        let s = RuntimeSettings::new();
        // Register the skills that legacy keys map to
        s.register_skill("news_digest", "news", true);
        s.register_skill("server_monitor", "server monitor", true);

        let mut map = HashMap::new();
        // Old key with a custom value
        map.insert("news_digest".to_string(), Value::Bool(false));
        map.insert("server_monitoring".to_string(), Value::Bool(false));

        let report = s.load_from_map(map);

        // Values should be applied to new keys
        assert!(!s.is_enabled("skill_news_digest"));
        assert!(!s.is_enabled("skill_server_monitor"));

        // Should report migrations
        assert_eq!(report.key_renames.len(), 2);
        assert!(report.value_rewrites.is_empty());
        assert!(
            report
                .key_renames
                .iter()
                .any(|(old, new)| old == "news_digest" && new == "skill_news_digest")
        );
        assert!(
            report
                .key_renames
                .iter()
                .any(|(old, new)| old == "server_monitoring" && new == "skill_server_monitor")
        );
    }

    #[test]
    fn legacy_key_not_migrated_if_skill_not_registered() {
        let s = RuntimeSettings::new();
        // Don't register news_digest — legacy key should be ignored
        let mut map = HashMap::new();
        map.insert("news_digest".to_string(), Value::Bool(false));

        let report = s.load_from_map(map);
        assert!(report.key_renames.is_empty());
        assert!(s.get("skill_news_digest").is_none());
    }

    #[test]
    fn persisted_skill_toggle_survives_load_before_skill_registration() {
        let s = RuntimeSettings::new();
        let mut map = HashMap::new();
        map.insert("skill_web_reader".to_string(), Value::Bool(false));

        let report = s.load_from_map(map);
        assert!(report.key_renames.is_empty());
        assert!(!s.is_enabled("skill_web_reader"));

        s.register_skill("web_reader", "Web reader", true);
        assert!(
            !s.is_enabled("skill_web_reader"),
            "registered skill default must not overwrite persisted owner toggle"
        );
    }

    #[test]
    fn format_for_prompt_groups_system_and_skills() {
        let s = RuntimeSettings::new();
        s.register_skill("web_reader", "web reader", true);
        s.register_skill("reminder", "reminder", true);
        let prompt = s.format_for_prompt();

        // Should have both sections
        assert!(prompt.contains("### System"));
        assert!(prompt.contains("### Skill Toggles"));

        // System keys should come before skill keys
        let system_pos = prompt.find("### System").unwrap();
        let skill_pos = prompt.find("### Skill Toggles").unwrap();
        assert!(system_pos < skill_pos);
    }

    #[test]
    fn register_skill_default_false() {
        let s = RuntimeSettings::new();
        s.register_skill("experimental", "experimental skill", false);
        assert!(!s.is_enabled("skill_experimental"));
    }

    #[test]
    fn is_enabled_handles_string_true() {
        let s = RuntimeSettings::new();
        // Simulate what happens when LLM sends "true" as a string
        // and it survives SQLite roundtrip as Value::String("true")
        s.values.write().insert(
            "codex_enabled".to_string(),
            Value::String("true".to_string()),
        );
        assert!(
            s.is_enabled("codex_enabled"),
            "String 'true' should be treated as enabled"
        );

        s.values.write().insert(
            "codex_enabled".to_string(),
            Value::String("false".to_string()),
        );
        assert!(
            !s.is_enabled("codex_enabled"),
            "String 'false' should be treated as disabled"
        );

        s.values.write().insert(
            "codex_enabled".to_string(),
            Value::String("TRUE".to_string()),
        );
        assert!(
            s.is_enabled("codex_enabled"),
            "String 'TRUE' should be case-insensitive"
        );
    }

    #[test]
    fn set_coerces_string_to_bool() {
        let s = RuntimeSettings::new();
        // LLM sends "true" as a string for a boolean setting
        s.set("codex_enabled", Value::String("true".to_string()))
            .unwrap();
        assert_eq!(s.get("codex_enabled"), Some(Value::Bool(true)));

        s.set("codex_enabled", Value::String("false".to_string()))
            .unwrap();
        assert_eq!(s.get("codex_enabled"), Some(Value::Bool(false)));

        // Non-boolean settings should NOT be coerced
        s.set("user_timezone", Value::String("true".to_string()))
            .unwrap();
        assert_eq!(
            s.get("user_timezone"),
            Some(Value::String("true".to_string()))
        );
    }

    // ── heartbeat_memo registration (TDD — Bug #12) ─────────────────────

    /// heartbeat_memo MUST be a registered setting so it can be read/written
    /// through the RuntimeSettings cache. Without registration,
    /// settings.get("heartbeat_memo") always returns None.
    #[test]
    fn heartbeat_memo_is_registered() {
        let defs = system_setting_definitions();
        assert!(
            defs.iter().any(|d| d.key == "heartbeat_memo"),
            "heartbeat_memo must be in system_setting_definitions()"
        );
    }

    #[test]
    fn heartbeat_memo_get_set_roundtrip() {
        let s = RuntimeSettings::new();
        // Default should be empty string
        let val = s.get("heartbeat_memo");
        assert!(val.is_some(), "heartbeat_memo must be accessible via get()");
        assert_eq!(val.unwrap(), Value::String(String::new()));

        // Set and read back
        s.set("heartbeat_memo", Value::String("test memo".to_string()))
            .unwrap();
        assert_eq!(
            s.get("heartbeat_memo"),
            Some(Value::String("test memo".to_string()))
        );
    }

    #[test]
    fn heartbeat_memo_survives_load_from_map() {
        let s = RuntimeSettings::new();
        let mut map = HashMap::new();
        map.insert(
            "heartbeat_memo".to_string(),
            Value::String("persisted memo".to_string()),
        );

        s.load_from_map(map);

        assert_eq!(
            s.get("heartbeat_memo"),
            Some(Value::String("persisted memo".to_string()))
        );
    }

    #[test]
    fn parse_tz_str_valid() {
        assert_eq!(parse_tz_str("Asia/Shanghai"), chrono_tz::Tz::Asia__Shanghai);
        assert_eq!(parse_tz_str("UTC"), chrono_tz::Tz::UTC);
        assert_eq!(
            parse_tz_str("America/New_York"),
            chrono_tz::Tz::America__New_York
        );
    }

    #[test]
    fn parse_tz_str_invalid_falls_back() {
        assert_eq!(parse_tz_str("NotATimezone"), chrono_tz::Tz::UTC);
        assert_eq!(parse_tz_str(""), chrono_tz::Tz::UTC);
    }

    // ── Embedding/LLM settings registration (2026-04-18 refactor) ────────
    //
    // These keys must exist in the registry so `update_setting` (which
    // validates keys against the registry) can propose changes. A missing
    // entry would also break the `settings → env → default` startup
    // priority implemented in main.rs (the setting lookup returns None,
    // forcing a fallback to env even when the user explicitly set a value).

    #[test]
    fn embedding_refactor_keys_are_registered() {
        let defs = system_setting_definitions();
        let keys = [
            "embedding_primary_model",
            "embedding_backup_model",
            "embedding_dimensions",
            "embedding_probe_cooldown_secs",
            "embedding_promote_after_secs",
        ];
        for key in &keys {
            assert!(
                defs.iter().any(|d| d.key == *key),
                "{} must be in system_setting_definitions",
                key
            );
        }
        // llm_model is already registered; assert to guard against accidental removal.
        assert!(
            defs.iter().any(|d| d.key == "llm_model"),
            "llm_model must remain registered"
        );
    }

    #[test]
    fn llm_provider_routing_keys_are_registered() {
        let defs = system_setting_definitions();
        let keys = [
            "llm_provider",
            "paid_api_fallback_enabled",
            "prompt_optimization_allow_paid_api",
            "skillfile_optimization_allow_paid_api",
            "codex_enabled",
            "external_executor_enabled",
            "external_executor_primary",
            "external_executor_fallback",
            "external_executor_timeout_secs",
            "codex_model",
            "codex_reasoning_effort",
            "codex_foreground_only",
            "codex_allow_background_tasks",
            "codex_background_task_allowlist",
            "codex_max_concurrency",
            "codex_daily_request_limit",
            "codex_cli_binary",
            "codex_cli_home",
            "codex_cli_workdir",
            "codex_cli_timeout_secs",
            "xai_subscription_enabled",
            "xai_subscription_model",
            "web_search_provider",
            "web_search_model",
            "assistant_display_name",
            "xai_subscription_reasoning_effort",
            "chat_reasoning_effort_casual",
            "chat_history_window_messages",
            "xai_subscription_api_base",
            "xai_subscription_foreground_only",
            "xai_subscription_allow_background_tasks",
            "xai_subscription_background_task_allowlist",
            "xai_subscription_max_concurrency",
            "xai_subscription_daily_request_limit",
            "aegis_avatar_auto_refresh_enabled",
            "aegis_avatar_auto_refresh_interval_days",
            "aegis_avatar_auto_refresh_provider",
            "aegis_avatar_auto_refresh_last_at_unix",
            "aegis_avatar_auto_refresh_prompt_hint",
        ];
        for key in keys {
            assert!(
                defs.iter().any(|d| d.key == key),
                "{key} must be in system_setting_definitions"
            );
        }
    }

    #[test]
    fn llm_provider_routing_defaults_are_cost_safe() {
        let s = RuntimeSettings::new();
        assert_eq!(
            s.get("llm_provider"),
            Some(Value::String("codex_oauth".to_string()))
        );
        assert_eq!(s.get("paid_api_fallback_enabled"), Some(Value::Bool(false)));
        assert_eq!(
            s.get("prompt_optimization_allow_paid_api"),
            Some(Value::Bool(false))
        );
        assert_eq!(
            s.get("skillfile_optimization_allow_paid_api"),
            Some(Value::Bool(false))
        );
        assert_eq!(s.get("codex_enabled"), Some(Value::Bool(false)));
        assert_eq!(s.get("external_executor_enabled"), Some(Value::Bool(false)));
        assert_eq!(
            s.get("external_executor_primary"),
            Some(Value::String("auto".to_string()))
        );
        assert_eq!(
            s.get("external_executor_fallback"),
            Some(Value::String("none".to_string()))
        );
        assert_eq!(
            s.get("external_executor_timeout_secs"),
            Some(Value::Number(serde_json::Number::from(600)))
        );
        assert_eq!(s.get("codex_foreground_only"), Some(Value::Bool(true)));
        assert_eq!(
            s.get("codex_allow_background_tasks"),
            Some(Value::Bool(false))
        );
        assert_eq!(
            s.get("codex_background_task_allowlist"),
            Some(Value::Array(vec![]))
        );
        assert_eq!(
            s.get("codex_max_concurrency"),
            Some(Value::Number(serde_json::Number::from(1)))
        );
        assert_eq!(
            s.get("codex_daily_request_limit"),
            Some(Value::Number(serde_json::Number::from(100)))
        );
        assert_eq!(
            s.get("codex_cli_binary"),
            Some(Value::String("/usr/bin/codex".to_string()))
        );
        assert_eq!(
            s.get("codex_cli_home"),
            Some(Value::String(default_codex_home()))
        );
        assert_eq!(
            s.get("codex_cli_workdir"),
            Some(Value::String("/tmp".to_string()))
        );
        assert_eq!(
            s.get("codex_cli_timeout_secs"),
            Some(Value::Number(serde_json::Number::from(90)))
        );
        assert_eq!(s.get("xai_subscription_enabled"), Some(Value::Bool(false)));
        assert_eq!(
            s.get("xai_subscription_model"),
            Some(Value::String("grok-4.7".to_string()))
        );
        assert_eq!(
            s.get("web_search_provider"),
            Some(Value::String("auto".to_string()))
        );
        assert_eq!(
            s.get("web_search_model"),
            Some(Value::String(String::new()))
        );
        assert_eq!(
            s.get("xai_subscription_reasoning_effort"),
            Some(Value::String("medium".to_string()))
        );
        assert_eq!(
            s.get("chat_reasoning_effort_casual"),
            Some(Value::String("low".to_string()))
        );
        assert_eq!(
            s.get("chat_history_window_messages"),
            Some(Value::Number(serde_json::Number::from(24)))
        );
        assert_eq!(
            s.get("xai_subscription_api_base"),
            Some(Value::String("https://api.x.ai/v1".to_string()))
        );
        assert_eq!(
            s.get("xai_subscription_foreground_only"),
            Some(Value::Bool(true))
        );
        assert_eq!(
            s.get("xai_subscription_allow_background_tasks"),
            Some(Value::Bool(false))
        );
        assert_eq!(
            s.get("xai_subscription_background_task_allowlist"),
            Some(Value::Array(vec![]))
        );
        assert_eq!(
            s.get("xai_subscription_max_concurrency"),
            Some(Value::Number(serde_json::Number::from(1)))
        );
        assert_eq!(
            s.get("xai_subscription_daily_request_limit"),
            Some(Value::Number(serde_json::Number::from(100)))
        );
        assert_eq!(
            s.get("aegis_avatar_auto_refresh_enabled"),
            Some(Value::Bool(false))
        );
        assert_eq!(
            s.get("aegis_avatar_auto_refresh_interval_days"),
            Some(Value::Number(serde_json::Number::from(7)))
        );
        assert_eq!(
            s.get("aegis_avatar_auto_refresh_provider"),
            Some(Value::String("xai_subscription".to_string()))
        );
    }

    // ── Vision routing registration (2026-04-19) ────────────────────────
    //
    // Both keys must be in the registry so `update_setting` can propose
    // toggles and the orchestrator's `chat_create_with_fallback` can read
    // defaults via `settings.get(...)` without a None surprise.

    #[test]
    fn vision_routing_keys_are_registered() {
        let defs = system_setting_definitions();
        for key in ["vision_routing_enabled", "vision_model"] {
            assert!(
                defs.iter().any(|d| d.key == key),
                "{} must be in system_setting_definitions",
                key
            );
        }
    }

    #[test]
    fn vision_routing_defaults_are_sensible() {
        let s = RuntimeSettings::new();
        // Feature on by default — users with vision attachments get routed
        // unless they opt out.
        assert_eq!(s.get("vision_routing_enabled"), Some(Value::Bool(true)));
        // Empty vision_model means provider default (no hard-coded Gemini).
        let model = s
            .get("vision_model")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        assert_eq!(model, "");
    }

    #[test]
    fn scheduled_brief_default_times_match_user_decision() {
        let s = RuntimeSettings::new();

        assert_eq!(
            s.get("daily_brief_local_time"),
            Some(Value::String("09:00".to_string()))
        );
        assert_eq!(
            s.get("evening_review_local_time"),
            Some(Value::String("18:00".to_string()))
        );
        assert_eq!(
            s.get("daily_brief_schedule_enabled"),
            Some(Value::Bool(true))
        );
        assert_eq!(
            s.get("evening_review_schedule_enabled"),
            Some(Value::Bool(true))
        );
    }

    /// Pins that `vision_routing_enabled` is a live runtime toggle: the
    /// `update_setting` tool (and any other caller writing through
    /// `RuntimeSettings::set`) must be able to flip it off and have the
    /// subsequent `get` observe the new value. Guards against a future
    /// refactor that accidentally makes the key read-only from the registry.
    #[test]
    fn vision_routing_enabled_toggle_roundtrip() {
        let s = RuntimeSettings::new();
        assert_eq!(s.get("vision_routing_enabled"), Some(Value::Bool(true))); // default
        s.set("vision_routing_enabled", Value::Bool(false))
            .expect("set on a registered key must succeed");
        assert_eq!(s.get("vision_routing_enabled"), Some(Value::Bool(false)));
    }

    #[test]
    fn embedding_refactor_defaults_are_sensible() {
        let s = RuntimeSettings::new();
        // Primary default is non-empty and looks like a model id.
        let primary = s.get("embedding_primary_model").unwrap();
        let primary_s = primary.as_str().expect("primary default is string");
        assert!(
            primary_s.contains('/'),
            "primary default should look like `provider/model`, got {primary_s}"
        );
        // Backup defaults to empty string (disabled).
        assert_eq!(
            s.get("embedding_backup_model"),
            Some(Value::String(String::new()))
        );
        // Numeric defaults.
        assert_eq!(
            s.get("embedding_dimensions"),
            Some(Value::Number(serde_json::Number::from(512)))
        );
        assert_eq!(
            s.get("embedding_probe_cooldown_secs"),
            Some(Value::Number(serde_json::Number::from(60)))
        );
        assert_eq!(
            s.get("embedding_promote_after_secs"),
            Some(Value::Number(serde_json::Number::from(86_400)))
        );
    }

    /// Helper mirroring the FIXED startup resolution logic in `main.rs`
    /// (see `core/src/main.rs:429-486` and `:639-651`):
    ///
    ///     DB-stored setting > env var > hard-coded default
    ///
    /// Uses [`RuntimeSettings::get_if_stored`] so a seeded registry default
    /// does NOT masquerade as a user-configured value. The previous version
    /// of this helper used `get`, which accepted the pre-seeded default as
    /// a "user setting" and completely bypassed the env-var fallback — that
    /// bug shipped to production in commit 519469c on 2026-04-18.
    fn resolve_string_with_priority(
        settings: &RuntimeSettings,
        setting_key: &str,
        env_value: Option<&str>,
        default_value: &str,
    ) -> String {
        settings
            .get_if_stored(setting_key)
            .and_then(|v| v.as_str().map(String::from))
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                env_value
                    .map(|s| s.to_string())
                    .filter(|s| !s.trim().is_empty())
            })
            .unwrap_or_else(|| default_value.to_string())
    }

    // ── Startup priority regression tests ───────────────────────────────
    //
    // These tests encode the ONLY acceptable startup priority for any key
    // that has (a) a registered default AND (b) an env-var fallback:
    //
    //     DB-stored setting  >  env var  >  hard-coded default
    //
    // The 2026-04-18 production bug happened because `get` returns the
    // pre-seeded registry default as if it were a user-stored value; the
    // env var then never gets a chance. `resolve_string_with_priority`
    // uses `get_if_stored`, so the pre-seeded default no longer masks env.

    #[test]
    fn startup_priority_setting_wins_over_env_and_default() {
        let s = RuntimeSettings::new();
        s.set(
            "embedding_primary_model",
            Value::String("setting-wins".into()),
        )
        .unwrap();
        let got = resolve_string_with_priority(
            &s,
            "embedding_primary_model",
            Some("env-value"),
            "default-value",
        );
        assert_eq!(got, "setting-wins");
    }

    #[test]
    fn startup_priority_env_wins_over_default_when_setting_is_default() {
        // When the registry default is returned (empty for backup_model),
        // the env var should be honored. The startup path filters empty
        // strings from the setting so env gets a chance.
        let s = RuntimeSettings::new();
        // Backup defaults to "" — so a non-empty env should win over the
        // default-empty setting value.
        let got = resolve_string_with_priority(
            &s,
            "embedding_backup_model",
            Some("baai/bge-small-en"),
            "",
        );
        assert_eq!(got, "baai/bge-small-en");
    }

    #[test]
    fn startup_priority_default_wins_when_no_setting_no_env() {
        let s = RuntimeSettings::new();
        // Primary default is `openai/text-embedding-3-small`, but with
        // `get_if_stored` that default does NOT leak through — only the
        // hardcoded-default passed to the helper should surface.
        let got =
            resolve_string_with_priority(&s, "embedding_primary_model", None, "hardcoded-default");
        assert_eq!(got, "hardcoded-default");
    }

    /// Canonical regression test for the 2026-04-18 production bug:
    /// no DB row, env var is set, the registry default is a non-empty string.
    /// The env var MUST win. If this test fails, the bug has returned.
    #[test]
    fn startup_priority_env_wins_when_only_default_seeded() {
        let s = RuntimeSettings::new();
        // The registry pre-seeds `embedding_primary_model` with the
        // non-empty default `openai/text-embedding-3-small`. Historically
        // (commit 519469c) this default leaked through and shadowed the
        // env var — that is precisely the bug being regression-tested.
        assert_eq!(
            s.get("embedding_primary_model"),
            Some(Value::String("openai/text-embedding-3-small".to_string())),
            "pre-condition: registry default is pre-seeded into values"
        );
        assert!(
            !s.is_stored("embedding_primary_model"),
            "pre-condition: the key is NOT yet marked as user-stored"
        );

        let got = resolve_string_with_priority(
            &s,
            "embedding_primary_model",
            Some("google/gemini-embedding-001"),
            "openai/text-embedding-3-small",
        );
        // Env MUST win over the pre-seeded registry default.
        assert_eq!(
            got, "google/gemini-embedding-001",
            "env var must win over pre-seeded registry default (2026-04-18 regression guard)"
        );
    }

    /// Regression test for `llm_model` — same category of bug as the
    /// embedding block. A non-empty fallback default must NOT shadow a set
    /// env var when only the registry default (now empty) is seeded.
    #[test]
    fn startup_priority_llm_model_env_wins_when_only_default_seeded() {
        let s = RuntimeSettings::new();
        let got =
            resolve_string_with_priority(&s, "llm_model", Some("anthropic/claude-sonnet-4-5"), "");
        assert_eq!(got, "anthropic/claude-sonnet-4-5");
    }

    /// CTO round-1 paired-fix sweep (2026-04-18): `user_timezone` has a
    /// non-empty registered default (the host zone, UTC at worst), so every call-site
    /// of the shape `settings.get("user_timezone").or_else(|| env::var(...))`
    /// was silently shadowing `USER_TIMEZONE`. Seven sibling sites were
    /// migrated to `get_if_stored`; this test pins the invariant that makes
    /// their env fallback reachable.
    #[test]
    fn get_if_stored_user_timezone_allows_env_fallback() {
        let s = RuntimeSettings::new();
        // Pre-condition: registry default is the non-empty host zone that
        // historically leaked through `.get().filter(!empty).or_else(env)`.
        let default_zone = user_tz_default();
        assert!(!default_zone.is_empty());
        assert_eq!(
            s.get("user_timezone"),
            Some(Value::String(default_zone)),
            "pre-condition: registry default is pre-seeded",
        );
        // `get_if_stored` must return None so that downstream `.or_else(env)`
        // branches are actually reachable.
        assert!(
            s.get_if_stored("user_timezone").is_none(),
            "user_timezone must not leak its default through get_if_stored",
        );
    }

    /// CTO round-1 paired-fix: `review_model`'s registered default happens
    /// to be `""` today, which masks the same bug behind `.filter(!empty)`.
    /// This test documents that `get_if_stored` is the correct lookup so
    /// the chain stays correct even if the default becomes non-empty later.
    #[test]
    fn get_if_stored_review_model_allows_env_fallback() {
        let s = RuntimeSettings::new();
        assert!(
            s.get_if_stored("review_model").is_none(),
            "review_model must not leak its default through get_if_stored",
        );
    }

    /// CTO round-2 regression guard (2026-04-18): the R2 paired-fix sweep
    /// (commit 10c5007) migrated 6 `user_timezone` sites to `get_if_stored`
    /// but silently dropped the terminal `.unwrap_or_else("Asia/Shanghai")`
    /// clause that main.rs:446 retains. The net effect was a behavior flip:
    /// for users with no DB-stored tz and no `USER_TIMEZONE` env, the chain
    /// returned `None` and every downstream (PanelManager, parse_trigger_at,
    /// execute_get_current_time) silently fell back to UTC instead of the
    /// registry default (then `Asia/Shanghai`; since 2026-09-16 the host
    /// zone, see [`user_tz_default`]).
    ///
    /// This test pins the canonical R1 chain — when neither DB nor env is
    /// set, the terminal default MUST surface. If this test fails, the
    /// dropped-terminal bug has returned.
    #[test]
    fn startup_priority_user_timezone_falls_back_to_terminal_default() {
        // SAFETY: the `env` arg to `resolve_string_with_priority` is the
        // value we want to simulate, not the process env — this avoids
        // touching `std::env::var("USER_TIMEZONE")` and the associated
        // cross-test races that would require an env_lock.
        let s = RuntimeSettings::new();
        // Pre-condition: `user_timezone` is NOT stored (fresh instance).
        assert!(
            s.get_if_stored("user_timezone").is_none(),
            "pre-condition: user_timezone must not be stored",
        );

        // Canonical R1 chain: no stored value, no env value → terminal
        // default must win. Pre-R2 (commit 10c5007) this was implicit via
        // the registered default leaking through `get`. Post-R2 this MUST
        // be explicit via the terminal `.unwrap_or_else`/`.or(Some(...))`.
        let default_zone = user_tz_default();
        let got = resolve_string_with_priority(&s, "user_timezone", None, &default_zone);
        assert_eq!(
            got, default_zone,
            "terminal host-zone default must surface when neither DB \
             nor env is set (2026-04-18 R2-drop regression guard)",
        );
    }

    /// DB-stored value loaded via `load_from_map` (the real startup path)
    /// must beat env. This is the real production flow: SQLite persistence
    /// layer returns a HashMap, main.rs calls `settings.load_from_map`,
    /// and then reads via `get_if_stored`.
    #[test]
    fn startup_priority_db_stored_wins_over_env_via_load_from_map() {
        let s = RuntimeSettings::new();
        let mut map = HashMap::new();
        map.insert(
            "embedding_primary_model".to_string(),
            Value::String("user/custom-model".to_string()),
        );
        s.load_from_map(map);

        assert!(
            s.is_stored("embedding_primary_model"),
            "load_from_map must mark the key as user-stored"
        );

        let got = resolve_string_with_priority(
            &s,
            "embedding_primary_model",
            Some("env-should-lose"),
            "default-should-also-lose",
        );
        assert_eq!(got, "user/custom-model");
    }

    /// Empty-string stored values should NOT win over env (they're the
    /// equivalent of "I once set this to blank then forgot" — we want env
    /// to take over). `resolve_string_with_priority` filters empty strings.
    #[test]
    fn startup_priority_empty_stored_value_falls_back_to_env() {
        let s = RuntimeSettings::new();
        s.set("embedding_primary_model", Value::String("".into()))
            .unwrap();
        // Sanity: the key IS now marked stored (the user did call `set`),
        // but the value is empty.
        assert!(s.is_stored("embedding_primary_model"));

        let got = resolve_string_with_priority(
            &s,
            "embedding_primary_model",
            Some("env-fallback"),
            "default",
        );
        assert_eq!(got, "env-fallback");
    }

    // ── `get_if_stored` / `stored_keys` unit tests ──────────────────────

    #[test]
    fn stored_keys_starts_empty_after_new() {
        let s = RuntimeSettings::new();
        // Every known registry key is accessible via `get` (seeded default).
        // NONE are accessible via `get_if_stored` (nothing has been stored).
        for def in system_setting_definitions() {
            assert!(
                s.get(&def.key).is_some(),
                "pre-seeded default must be readable via get for {}",
                def.key
            );
            assert!(
                s.get_if_stored(&def.key).is_none(),
                "key must NOT be marked as stored until explicitly set: {}",
                def.key
            );
            assert!(!s.is_stored(&def.key));
        }
    }

    #[test]
    fn set_marks_key_as_stored() {
        let s = RuntimeSettings::new();
        assert!(s.get_if_stored("embedding_primary_model").is_none());

        s.set(
            "embedding_primary_model",
            Value::String("user-choice".into()),
        )
        .unwrap();

        assert_eq!(
            s.get_if_stored("embedding_primary_model"),
            Some(Value::String("user-choice".into()))
        );
        assert!(s.is_stored("embedding_primary_model"));
    }

    #[test]
    fn load_from_map_marks_keys_as_stored() {
        let s = RuntimeSettings::new();
        let mut map = HashMap::new();
        map.insert(
            "embedding_dimensions".to_string(),
            Value::Number(serde_json::Number::from(768)),
        );
        map.insert(
            "embedding_primary_model".to_string(),
            Value::String("persisted/model".into()),
        );
        s.load_from_map(map);

        assert!(s.is_stored("embedding_dimensions"));
        assert!(s.is_stored("embedding_primary_model"));
        assert_eq!(
            s.get_if_stored("embedding_dimensions"),
            Some(Value::Number(serde_json::Number::from(768)))
        );
    }

    #[test]
    fn load_from_map_does_not_mark_unknown_keys() {
        let s = RuntimeSettings::new();
        let mut map = HashMap::new();
        map.insert("totally_unknown_key".to_string(), Value::Bool(true));
        s.load_from_map(map);
        assert!(!s.is_stored("totally_unknown_key"));
        assert!(s.get_if_stored("totally_unknown_key").is_none());
    }

    #[test]
    fn load_from_map_legacy_migration_marks_new_key_as_stored() {
        let s = RuntimeSettings::new();
        s.register_skill("server_monitor", "server", true);

        let mut map = HashMap::new();
        map.insert("server_monitoring".to_string(), Value::Bool(false));
        s.load_from_map(map);

        // New key is marked stored; old key is not registered at all.
        assert!(s.is_stored("skill_server_monitor"));
        assert!(!s.is_stored("server_monitoring"));
    }

    #[test]
    fn reset_unmarks_key_so_get_if_stored_returns_none() {
        let s = RuntimeSettings::new();
        s.set(
            "embedding_primary_model",
            Value::String("user-choice".into()),
        )
        .unwrap();
        assert!(s.is_stored("embedding_primary_model"));

        s.reset("embedding_primary_model").unwrap();

        assert!(!s.is_stored("embedding_primary_model"));
        assert!(s.get_if_stored("embedding_primary_model").is_none());
        // Effective value via `get` should now be the default again.
        assert_eq!(
            s.get("embedding_primary_model"),
            Some(Value::String("openai/text-embedding-3-small".to_string()))
        );
    }

    #[test]
    fn parse_user_tz_from_settings() {
        let s = RuntimeSettings::new();
        // Default is the host zone (registered default), UTC when unknown.
        assert_eq!(parse_user_tz(&s), parse_tz_str(&user_tz_default()));

        // Set a different valid timezone
        s.set(
            "user_timezone",
            Value::String("America/New_York".to_string()),
        )
        .unwrap();
        assert_eq!(parse_user_tz(&s), chrono_tz::Tz::America__New_York);

        // Set an invalid timezone → scheduling resolver falls back to UTC
        // (historical behavior, intentionally unchanged by the display work).
        s.set("user_timezone", Value::String("Invalid/Zone".to_string()))
            .unwrap();
        assert_eq!(parse_user_tz(&s), chrono_tz::Tz::UTC);
    }

    #[test]
    fn resolve_display_user_tz_invalid_stored_does_not_leak_utc() {
        // Regression for the user-facing display path (Telegram archive date,
        // WebApp projection): a bad stored timezone must NOT produce a
        // UTC-formatted user-visible date on a host that has a zone — the
        // display resolver substitutes the host-zone default instead.
        // Scheduling (parse_user_tz) deliberately still falls back to UTC.
        let s = RuntimeSettings::new();
        s.set("user_timezone", Value::String("Not/AZone".to_string()))
            .unwrap();
        // Host-zone branch with the lookup injected.
        let host = user_tz_default_from(Some("Europe/London"));
        let display = parse_tz_str(&resolve_user_tz_string_from(&s, None, &host));
        assert_eq!(display, chrono_tz::Tz::Europe__London);
        assert_ne!(display, chrono_tz::Tz::UTC);
        // The ambient resolver uses this process's host zone.
        assert_eq!(
            resolve_display_user_tz(&s),
            parse_tz_str(&user_tz_default())
        );
        // The scheduling resolver intentionally differs (UTC) for the same input.
        assert_eq!(parse_user_tz(&s), chrono_tz::Tz::UTC);
    }

    #[test]
    fn resolve_user_tz_string_stored_wins_over_env_and_default() {
        let s = RuntimeSettings::new();
        s.set(
            "user_timezone",
            Value::String("America/New_York".to_string()),
        )
        .unwrap();
        // Even with an env value present, the stored DB setting wins.
        assert_eq!(
            resolve_user_tz_string_with_env(&s, Some("Europe/London")),
            "America/New_York"
        );
    }

    #[test]
    fn resolve_user_tz_string_env_wins_over_default_when_unset() {
        // Fresh settings: user_timezone NOT stored (registry default only).
        let s = RuntimeSettings::new();
        assert_eq!(
            resolve_user_tz_string_with_env(&s, Some("Europe/London")),
            "Europe/London",
            "env must beat the hardcoded default when nothing is stored"
        );
    }

    #[test]
    fn resolve_user_tz_string_invalid_stored_falls_back_to_default() {
        let s = RuntimeSettings::new();
        s.set("user_timezone", Value::String("Not/AZone".to_string()))
            .unwrap();
        // Unlike parse_user_tz (which falls to UTC), the string resolver
        // returns the host-zone default so the frontend Intl never sees junk.
        assert_eq!(
            resolve_user_tz_string_from(&s, None, "Europe/Berlin"),
            "Europe/Berlin"
        );
        assert_eq!(resolve_user_tz_string_with_env(&s, None), user_tz_default());
    }

    #[test]
    fn resolve_user_tz_string_defaults_to_host_zone_when_nothing_configured() {
        // Fresh settings, no env → the host zone is the terminal default.
        let s = RuntimeSettings::new();
        let host = user_tz_default_from(Some("Europe/Berlin"));
        let got = resolve_user_tz_string_from(&s, None, &host);
        assert_eq!(
            got, "Europe/Berlin",
            "host zone must be the terminal default"
        );
        // Negative control against the pre-2026-09-16 hard-coded default:
        // this assertion fails if the resolver ignores the host lookup.
        assert_ne!(got, "Asia/Shanghai");
        // Ambient: the process host zone, whatever it is on this machine.
        assert_eq!(resolve_user_tz_string_with_env(&s, None), user_tz_default());
    }

    #[test]
    fn user_tz_default_tracks_iana_time_zone_lookup() {
        let expected = match iana_time_zone::get_timezone() {
            Ok(zone) if zone.parse::<chrono_tz::Tz>().is_ok() => zone,
            _ => "UTC".to_string(),
        };
        assert_eq!(user_tz_default(), expected);
        assert!(user_tz_default().parse::<chrono_tz::Tz>().is_ok());
    }

    #[test]
    fn user_tz_default_from_falls_back_to_utc() {
        assert_eq!(user_tz_default_from(None), "UTC");
        assert_eq!(user_tz_default_from(Some("")), "UTC");
        assert_eq!(user_tz_default_from(Some("  ")), "UTC");
        assert_eq!(user_tz_default_from(Some("Not/AZone")), "UTC");
        assert_eq!(
            user_tz_default_from(Some(" America/New_York ")),
            "America/New_York"
        );
    }
}

#[cfg(test)]
mod codex_home_default_tests {
    use super::codex_home_from;

    #[test]
    fn production_unit_home_yields_the_previous_hardcoded_path() {
        // The original deployment's systemd unit sets HOME=/home/aegis-runtime, so
        // replacing the literal "/home/aegis-runtime/.codex" default with a
        // HOME-derived one must not change that deployment's behaviour.
        assert_eq!(
            codex_home_from(None, Some("/home/aegis-runtime")),
            "/home/aegis-runtime/.codex"
        );
    }

    #[test]
    fn codex_home_env_wins_over_home() {
        assert_eq!(
            codex_home_from(Some("/custom/codex"), Some("/home/someone")),
            "/custom/codex"
        );
    }

    #[test]
    fn blank_values_fall_through() {
        assert_eq!(
            codex_home_from(Some("  "), Some("/home/x")),
            "/home/x/.codex"
        );
        assert_eq!(codex_home_from(None, Some("  ")), ".codex");
        assert_eq!(codex_home_from(None, None), ".codex");
    }

    #[test]
    fn trailing_slash_does_not_double_up() {
        assert_eq!(codex_home_from(None, Some("/home/x/")), "/home/x/.codex");
    }
}
