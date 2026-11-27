use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Timelike;

use aegis_runtime_core::admin::{AdminState, admin_router};
use aegis_runtime_core::agent_skills;
use aegis_runtime_core::chronos::ChronosEngine;
use aegis_runtime_core::credential_store;
use aegis_runtime_core::daily_brief::{
    ScheduledBriefConfig, ScheduledBriefKind, parse_scheduled_brief_time,
    scheduled_brief_is_due_with_loaded_state, scheduled_brief_within_window,
};
use aegis_runtime_core::embedding::EmbeddingClient;
use aegis_runtime_core::google_auth::GoogleAuthManager;
use aegis_runtime_core::identity::AgentIdentity;
use aegis_runtime_core::intent::{DeferredIntentEngine, IntentEvent};
use aegis_runtime_core::memory::{MemoryHandle, SkillProposalUpdate};
use aegis_runtime_core::memory::{STATUS_ACTIVE, STATUS_DEACTIVATED};
use aegis_runtime_core::models::SkillCapability;
use aegis_runtime_core::orchestrator::Orchestrator;
use aegis_runtime_core::sandbox::{SandboxEngine, auth_headers_from_env};
use aegis_runtime_core::settings::RuntimeSettings;
use aegis_runtime_core::settings_audit;
use aegis_runtime_core::telegram::{
    AutonomousHeartbeatHandler, TelegramBot, TelegramHeartbeatHandler, markdown_to_telegram_html,
    send_html_to_topic, wrap_thread_id,
};
use aegis_runtime_core::telegram_notification_router::{
    NotificationClass, PrivacyLevel, proactive_route_request, route_telegram_notification,
    topic_for_class,
};
use aegis_runtime_core::topic_router::TopicKind;
use aegis_runtime_core::utils::parse_timezone_offset;

/// Tunnel URL freshness threshold: files older than this are considered stale.
const TUNNEL_URL_FRESHNESS_SECS: u64 = 3_600;

/// Maximum number of Tokio blocking threads.
///
/// `sandbox.execute()` is synchronous and blocks for up to 60 s during HTTP
/// I/O inside `host_fetch`. Callers offload it via `tokio::task::spawn_blocking`.
/// Tokio's default cap is 512; without an explicit limit a burst of concurrent
/// heartbeats + user tool calls can exhaust OS thread limits on constrained VMs.
///
/// Sizing rationale:
/// - Up to 12 skill heartbeats (per-skill AtomicBool ensures at most one active
///   at a time per skill, so ≤ 12 concurrent blocking threads from heartbeats).
/// - Up to MAX_TOTAL_TOOL_CALLS=10 tool calls per orchestrator turn.
/// - Small headroom for DB actor, LLM streaming, miscellaneous async work.
///   32 threads gives comfortable room while keeping the OS thread budget bounded.
const MAX_BLOCKING_THREADS: usize = 32;

fn routed_telegram_destination(
    admin_chat_id: i64,
    notification_class: NotificationClass,
    privacy_level: PrivacyLevel,
) -> (
    teloxide::types::ChatId,
    Option<teloxide::types::ThreadId>,
    TopicKind,
) {
    let route = route_telegram_notification(
        admin_chat_id,
        proactive_route_request(notification_class, privacy_level, None),
    );
    (
        teloxide::types::ChatId(route.chat_id),
        wrap_thread_id(route.thread_id),
        route
            .topic_kind
            .unwrap_or_else(|| topic_for_class(notification_class)),
    )
}

fn routed_proactive_progress_chat_id(admin_chat_id: i64) -> i64 {
    route_telegram_notification(
        admin_chat_id,
        proactive_route_request(
            NotificationClass::RoutineNotification,
            PrivacyLevel::PrivateDmOnly,
            None,
        ),
    )
    .chat_id
}

const DAILY_BRIEF_SCHEDULER_STATE: &str = "daily_brief_scheduler";

fn scheduled_brief_config(
    settings: &RuntimeSettings,
    kind: ScheduledBriefKind,
) -> ScheduledBriefConfig {
    let default_time = match kind {
        ScheduledBriefKind::DailyBrief => "09:00",
        ScheduledBriefKind::EveningReview => "18:00",
    };
    let enabled = settings
        .get(kind.enabled_setting_key())
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let local_time = settings
        .get(kind.time_setting_key())
        .and_then(|v| v.as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_time.to_string());

    ScheduledBriefConfig {
        enabled,
        local_time,
    }
}

fn current_brief_local_clock(settings: &RuntimeSettings) -> (String, String, u32, u32) {
    let utc_now = chrono::Utc::now();
    let tz_str = settings
        .get_if_stored("user_timezone")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("USER_TIMEZONE")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(aegis_runtime_core::settings::user_tz_default);

    if let Ok(iana_tz) = tz_str.parse::<chrono_tz::Tz>() {
        let local = utc_now.with_timezone(&iana_tz);
        return (
            local.format("%Y-%m-%d").to_string(),
            tz_str,
            local.hour(),
            local.minute(),
        );
    }

    if let Ok(offset_secs) = parse_timezone_offset(&tz_str) {
        let utc_fixed = chrono::FixedOffset::east_opt(0).expect("UTC offset 0 is always valid");
        let offset = chrono::FixedOffset::east_opt(offset_secs).unwrap_or(utc_fixed);
        let local = utc_now.with_timezone(&offset);
        return (
            local.format("%Y-%m-%d").to_string(),
            tz_str,
            local.hour(),
            local.minute(),
        );
    }

    (
        utc_now.format("%Y-%m-%d").to_string(),
        "UTC".to_string(),
        utc_now.hour(),
        utc_now.minute(),
    )
}

fn scheduled_brief_title(kind: ScheduledBriefKind) -> &'static str {
    match kind {
        ScheduledBriefKind::DailyBrief => "Daily Brief",
        ScheduledBriefKind::EveningReview => "Evening Review",
    }
}

fn scheduled_brief_action_id(kind: ScheduledBriefKind, local_date: &str) -> String {
    format!("scheduled-brief:{}:{local_date}", kind.as_str())
}

fn start_scheduled_brief_web_ledger(
    db_path: &str,
    kind: ScheduledBriefKind,
    local_date: &str,
    timezone: &str,
) -> Result<String> {
    let action_id = scheduled_brief_action_id(kind, local_date);
    let conn = aegis_runtime_core::encrypted_sqlite::open(db_path)
        .with_context(|| format!("opening action ledger db for {}", kind.as_str()))?;
    let store = aegis_runtime_core::action_ledger::ActionLedgerStore::new(&conn);
    store.create_proposed(&aegis_runtime_core::action_ledger::NewAction {
        action_id: action_id.clone(),
        action_kind: aegis_runtime_core::action_ledger::ActionKind::Other,
        requested_by: Some("system:scheduled_brief".to_string()),
        input_summary: Some(format!(
            "{} scheduled for {local_date} ({timezone})",
            scheduled_brief_title(kind)
        )),
        user_visible_summary: Some(format!("{} delivered to Web", scheduled_brief_title(kind))),
        idempotency_key: Some(action_id.clone()),
        source_event_id: Some(format!("scheduled_brief:{}", kind.as_str())),
        metadata_json: Some(
            serde_json::json!({
                "lane": "scheduled_brief",
                "kind": kind.as_str(),
                "local_date": local_date,
                "timezone": timezone,
                "delivery_surface": "web_owner_timeline",
            })
            .to_string(),
        ),
        ..Default::default()
    })?;
    store.transition(
        &action_id,
        aegis_runtime_core::action_ledger::ActionStatus::Executing,
        &aegis_runtime_core::action_ledger::ActionUpdate::default(),
    )?;
    Ok(action_id)
}

fn finish_scheduled_brief_web_ledger(
    db_path: &str,
    action_id: &str,
    owner_did: &str,
) -> Result<()> {
    let conn = aegis_runtime_core::encrypted_sqlite::open(db_path)
        .with_context(|| format!("opening action ledger db for {action_id}"))?;
    let store = aegis_runtime_core::action_ledger::ActionLedgerStore::new(&conn);
    store.transition(
        action_id,
        aegis_runtime_core::action_ledger::ActionStatus::Executed,
        &aegis_runtime_core::action_ledger::ActionUpdate {
            external_receipt: Some(scheduled_brief_web_receipt(owner_did)),
            ..Default::default()
        },
    )?;
    Ok(())
}

fn scheduled_brief_web_receipt(owner_did: &str) -> String {
    serde_json::json!({
        "type": "web_owner_notice_delivery",
        "surface": "web_owner_timeline",
        "owner_did": owner_did,
    })
    .to_string()
}

async fn maybe_send_scheduled_personal_brief(
    _bot: &teloxide::Bot,
    orchestrator: &Orchestrator,
    memory: &MemoryHandle,
    settings: &RuntimeSettings,
    action_ledger_db_path: &str,
    admin_chat_id: i64,
    kind: ScheduledBriefKind,
) {
    let config = scheduled_brief_config(settings, kind);
    if !config.enabled {
        return;
    }

    let Some((scheduled_hour, scheduled_minute)) = parse_scheduled_brief_time(&config.local_time)
    else {
        tracing::warn!(
            kind = kind.as_str(),
            local_time = %config.local_time,
            "[DailyBrief] scheduled brief skipped because configured time is not strict HH:MM"
        );
        return;
    };

    let (local_date, timezone, local_hour, local_minute) = current_brief_local_clock(settings);
    if !scheduled_brief_within_window(local_hour, local_minute, scheduled_hour, scheduled_minute) {
        return;
    }

    let last_sent = match memory
        .load_skill_state(DAILY_BRIEF_SCHEDULER_STATE, kind.state_key())
        .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::warn!(
                error = %err,
                kind = kind.as_str(),
                "[DailyBrief] scheduled brief state load failed; skipping send to avoid duplicates"
            );
            return;
        }
    };

    if !scheduled_brief_is_due_with_loaded_state(
        &config,
        &local_date,
        local_hour,
        local_minute,
        true,
        last_sent.as_deref(),
    ) {
        return;
    }

    let scheduled_user_did = format!("did:aegis:telegram:{admin_chat_id}");
    let output = orchestrator
        .build_scheduled_personal_brief_output(kind, Some(admin_chat_id), Some(&scheduled_user_did))
        .await;
    let markdown = output.to_markdown(&settings.display_language());
    let has_owner_visible_signal = output.has_owner_visible_signal();
    let owner_did = aegis_runtime_core::owner_presence::canonical_owner_did_for_chat(admin_chat_id);
    let owner_active =
        aegis_runtime_core::owner_presence::owner_web_presence_is_active(memory, &owner_did).await;
    let mut web_push_delivered = false;
    let action_id = if has_owner_visible_signal {
        web_push_delivered =
            aegis_runtime_core::telegram::record_owner_web_notice_if_push_delivered(
                memory,
                admin_chat_id,
                &markdown,
            )
            .await;
        let action_id = match start_scheduled_brief_web_ledger(
            action_ledger_db_path,
            kind,
            &local_date,
            &timezone,
        ) {
            Ok(action_id) => action_id,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    kind = kind.as_str(),
                    "[DailyBrief] scheduled brief skipped because Web Action Ledger start failed"
                );
                return;
            }
        };
        if let Err(err) =
            finish_scheduled_brief_web_ledger(action_ledger_db_path, &action_id, &owner_did)
        {
            tracing::warn!(
                error = %err,
                kind = kind.as_str(),
                action_id,
                "[DailyBrief] scheduled brief Web Action Ledger finish failed after Web delivery"
            );
            return;
        }
        Some(action_id)
    } else {
        tracing::info!(
            kind = kind.as_str(),
            local_date = %local_date,
            timezone = %timezone,
            "[DailyBrief] scheduled brief had no owner-visible signal; suppressing owner notice"
        );
        None
    };

    if let Err(err) = memory
        .store_skill_state(DAILY_BRIEF_SCHEDULER_STATE, kind.state_key(), &local_date)
        .await
    {
        tracing::warn!(
                error = %err,
                kind = kind.as_str(),
                local_date = %local_date,
                "[DailyBrief] scheduled brief state store failed after Web delivery"
        );
        return;
    }

    tracing::info!(
        kind = kind.as_str(),
        local_date = %local_date,
        timezone = %timezone,
        action_id = action_id.as_deref().unwrap_or("none"),
        owner_active,
        has_owner_visible_signal,
        web_push_delivered,
        "[DailyBrief] scheduled {} cycle completed",
        scheduled_brief_title(kind)
    );
}

fn spawn_scheduled_personal_brief_loop(
    bot: teloxide::Bot,
    orchestrator: Arc<Orchestrator>,
    memory: MemoryHandle,
    settings: Arc<RuntimeSettings>,
    action_ledger_db_path: String,
    admin_chat_id: i64,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            maybe_send_scheduled_personal_brief(
                &bot,
                &orchestrator,
                &memory,
                &settings,
                &action_ledger_db_path,
                admin_chat_id,
                ScheduledBriefKind::DailyBrief,
            )
            .await;
            maybe_send_scheduled_personal_brief(
                &bot,
                &orchestrator,
                &memory,
                &settings,
                &action_ledger_db_path,
                admin_chat_id,
                ScheduledBriefKind::EveningReview,
            )
            .await;
        }
    });
}

/// Construct the experimental `HeirloomBridge` only for an opted-in path.
///
/// Returns `Some(bridge)` on success, `None` in degraded mode (logs a
/// bilingual warn and continues legacy-only). A disabled path returns before
/// creating directories, opening databases, or constructing development seams.
/// The DB path comes from [`resolve_heirloom_event_log_path`]
/// (`HEIRLOOM_EVENT_LOG_DB`, else next to `MEMORY_DB_PATH`); the parent
/// directory is created on demand so a clean first run never degrades over a
/// missing data directory.
///
/// Error handling (FIX 10 — no silent .unwrap_or(0)):
/// - Parent-directory or connection-open failure → bilingual warn, return None
/// - ensure_tables failure → bilingual warn, return None
/// - Empty-log count query: explicit match against rusqlite::Error. "No
///   rows" is not possible for SELECT COUNT(*); any Err is logged and
///   bootstrap is skipped — the bridge is still returned so subsequent
///   appends can attempt recovery.
/// - bootstrap_fresh_log_with_types failure → bilingual warn, return None
fn build_heirloom_bridge(
    owner_did_str: &str,
    db_path: Option<&Path>,
) -> Option<Arc<aegis_runtime_core::memory_heirloom_bridge::HeirloomBridge>> {
    let db_path = db_path?;
    use aegis_runtime_core::heirloom::writer::{HeirloomWriter, WriterConfig};
    use aegis_runtime_core::heirloom::{
        DbBackedSchemaRegistry, Hlc, LineageId, SchemaRegistry,
        seams::{
            ConstitutionTuple, EthicsValidator, InMemoryZeroizer, NoopWelfareOracle, PayloadCodec,
            PermissiveEthics, PlaintextCodec, SecureEnclave, SoftEnclave, WelfareOracle,
            ZeroizeHook,
        },
        types::{Did, OwnerState},
        writer::FixedOwnerState,
    };
    use aegis_runtime_core::memory_heirloom_bridge::HeirloomBridge;

    let db_path = db_path.display().to_string();

    // FIX 10 — bilingual warn on any setup failure, return None (degraded).
    if let Some(parent) = Path::new(&db_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(
            error = %e,
            db_path = %db_path,
            "heirloom 数据目录创建失败，进入 degraded 模式 / \
             heirloom data directory could not be created, entering degraded mode"
        );
        return None;
    }
    let conn = match aegis_runtime_core::encrypted_sqlite::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                error = %e,
                db_path = %db_path,
                "heirloom bridge 初始化失败，进入 degraded 模式 / \
                 heirloom bridge init failed, entering degraded mode (dual-write disabled)"
            );
            return None;
        }
    };
    if let Err(e) = conn
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;")
    {
        tracing::warn!(
            error = %e,
            db_path = %db_path,
            "heirloom bridge pragmas 失败，进入 degraded 模式 / \
             heirloom bridge pragmas failed, entering degraded mode"
        );
        return None;
    }
    if let Err(e) = aegis_runtime_core::heirloom::schema::ensure_tables(&conn) {
        tracing::warn!(
            error = %e,
            db_path = %db_path,
            "heirloom ensure_tables 失败，进入 degraded 模式 / \
             heirloom ensure_tables failed, entering degraded mode"
        );
        return None;
    }

    let bridge_did = Did::new(owner_did_str);

    // Phase E seams — every one is the Phase D test default (NG12 Phase F
    // promotes these to real settings-backed impls).
    let hlc = Arc::new(Hlc::new());
    let enclave: Arc<dyn SecureEnclave> = Arc::new(SoftEnclave::new_dev(42));
    let codec: Arc<dyn PayloadCodec> = Arc::new(PlaintextCodec::new());
    let ethics: Arc<dyn EthicsValidator> = Arc::new(PermissiveEthics::new());
    let welfare: Arc<dyn WelfareOracle> = Arc::new(NoopWelfareOracle::new());
    let zeroize: Arc<dyn ZeroizeHook> = Arc::new(InMemoryZeroizer::new());
    let owner_state_provider = Arc::new(FixedOwnerState(OwnerState::Alive));
    let bridge_did_for_did_closure = bridge_did.clone();
    let did_provider: Arc<dyn Fn() -> Did + Send + Sync> =
        Arc::new(move || bridge_did_for_did_closure.clone());
    // FIX 1 — use the real promoted helper instead of unimplemented!().
    let bridge_did_for_ethics_closure = bridge_did.clone();
    let ethics_ctx_builder: Arc<
        dyn Fn() -> aegis_runtime_core::heirloom::seams::EthicsContext + Send + Sync,
    > = Arc::new(move || {
        aegis_runtime_core::heirloom::seams::ethics_validator::default_permissive_ethics_ctx(
            &bridge_did_for_ethics_closure,
        )
    });

    // Second connection for the registry (WAL-safe).
    let registry_conn = match aegis_runtime_core::encrypted_sqlite::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                error = %e,
                db_path = %db_path,
                "heirloom registry read-conn 失败，进入 degraded 模式 / \
                 heirloom registry read-conn failed, entering degraded mode"
            );
            return None;
        }
    };
    let schema_registry: Arc<dyn SchemaRegistry> = Arc::new(DbBackedSchemaRegistry::new(Arc::new(
        parking_lot::Mutex::new(registry_conn),
    )));

    let writer = match HeirloomWriter::new(
        conn,
        hlc,
        enclave,
        codec,
        ethics,
        welfare,
        zeroize,
        owner_state_provider,
        did_provider,
        ethics_ctx_builder,
        schema_registry,
        WriterConfig::default(),
    ) {
        Ok(w) => Arc::new(w),
        Err(e) => {
            tracing::warn!(
                error = ?e,
                db_path = %db_path,
                "heirloom writer 构造失败，进入 degraded 模式 / \
                 heirloom writer construction failed, entering degraded mode"
            );
            return None;
        }
    };

    // The schema gate (`DbBackedSchemaRegistry`) reads `heirloom_schema_registry`,
    // which only the SchemaRegistryProjector fills. Without it, bootstrap
    // writes all `schema:type_defined` events but the registry stays empty and
    // every later shim write soft-fails with `schema 拒绝 / unknown event_type`
    // — invisible until 2026-09-20 because the bridge never initialized on a
    // machine without the author's /opt tree. Register it BEFORE bootstrap.
    writer.register_projector(Arc::new(
        aegis_runtime_core::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
    ));

    // Bootstrap-if-empty (G11 + FIX 9 closes Phase D [~] deferred item).
    // FIX 10: explicit match, NO .unwrap_or(0). A SELECT COUNT(*) cannot
    // return "no rows", but any Err here indicates DB-layer breakage — we
    // log and continue without bootstrapping (the bridge still returns;
    // the next real append will surface any persistent breakage via
    // try_emit's soft-fail + DivergenceCounter).
    let count_result: Result<i64, rusqlite::Error> = {
        let c = writer.shared_connection();
        c.lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
    };
    let n: i64 = match count_result {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(
                error = %e,
                db_path = %db_path,
                "heirloom bridge count query 失败，进入 degraded 模式 / \
                 heirloom bridge count query failed, entering degraded mode"
            );
            return None;
        }
    };
    if n == 0
        && let Err(e) = writer.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:aegis:main"),
            bridge_did.clone(),
            ConstitutionTuple {
                l0_hash: [0; 32],
                l1_hash: [0; 32],
                l2_manifest_hash: [0; 32],
            },
            aegis_runtime_core::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS,
        )
    {
        tracing::warn!(
            error = ?e,
            db_path = %db_path,
            "heirloom bootstrap 失败，进入 degraded 模式 / \
             heirloom bootstrap failed, entering degraded mode"
        );
        return None;
    }

    // Logs bootstrapped by the pre-projector wiring have the events but an
    // empty registry; replay the stored schema events once so those installs
    // stop rejecting every write. No-op on a healthy database.
    match writer.backfill_schema_registry_from_log() {
        Ok(0) => {}
        Ok(replayed) => eprintln!(
            "[Heirloom] Rebuilt heirloom_schema_registry from {replayed} stored schema event(s) (log was bootstrapped without the registry projector)"
        ),
        Err(e) => tracing::warn!(
            error = ?e,
            db_path = %db_path,
            "heirloom schema registry 回填失败（写入将被 schema 门拒绝）/ \
             heirloom schema registry backfill failed (writes will be rejected by the schema gate)"
        ),
    }

    Some(Arc::new(HeirloomBridge::new_with_env(writer, bridge_did)))
}

/// Seconds between re-reads of `health_probe_interval_minutes` while the
/// runtime health probe is paused (the console's cost-saving pause sets it to 0).
const HEALTH_PROBE_PAUSED_POLL_SECS: u64 = 60;

/// Live value of `health_probe_interval_minutes` (0 = paused / disabled),
/// clamped like every other reader of the setting.
fn health_probe_interval_minutes(settings: &RuntimeSettings) -> u64 {
    settings.get_u64_clamped("health_probe_interval_minutes", 60, 0, 24 * 60)
}

#[derive(Debug, PartialEq, Eq)]
enum HealthProbeTick {
    /// `health_probe_interval_minutes == 0`: sleep, then re-read the setting.
    Paused { log_transition: bool },
    /// Probe now, then wait `interval` before the next iteration.
    Run {
        log_resumed: bool,
        interval: std::time::Duration,
    },
}

/// Pure decision for one health-probe loop iteration, from the freshly read
/// interval and whether the previous iteration was paused. Transition logs
/// fire once per pause/resume edge, never per tick.
fn plan_health_probe_tick(interval_minutes: u64, was_paused: bool) -> HealthProbeTick {
    if interval_minutes == 0 {
        HealthProbeTick::Paused {
            log_transition: !was_paused,
        }
    } else {
        HealthProbeTick::Run {
            log_resumed: was_paused,
            interval: std::time::Duration::from_secs(interval_minutes * 60),
        }
    }
}

fn main() -> Result<std::process::ExitCode> {
    // Build a custom Tokio runtime so we can set max_blocking_threads, which is
    // not exposed as a #[tokio::main] attribute. All other settings mirror what
    // #[tokio::main] produces by default (multi-thread, all features enabled).
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .build()
        .context("building Tokio runtime")?;
    let code = runtime.block_on(async_main())?;
    // CLI commands can leave a memory actor finishing SQLCipher/OpenSSL work.
    // Dropping Tokio joins blocking tasks before Rust returns the process status;
    // process::exit inside async_main would skip this and race native teardown.
    drop(runtime);
    Ok(std::process::ExitCode::from(
        u8::try_from(code).context("invalid CLI exit code")?,
    ))
}

async fn async_main() -> Result<i32> {
    // 0. Initialize structured logging with runtime-reloadable filter
    //    Supports RUST_LOG env var; level can be changed at runtime via set_log_level tool.
    aegis_runtime_core::logging::init();

    // Capture the database key before dotenvy reads .env. A key stored next
    // to the databases would defeat protection against a copied state dir.
    let external_db_key = std::env::var("AEGIS_RUNTIME_DB_KEY_HEX").ok();

    // 1. Load environment variables
    dotenvy::dotenv().ok();
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("aegis_runtime_core {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    // `--print-pubkey`: print the Ed25519 public key the Aegis Executor must
    // trust (its AEGIS_PUBKEY_HEX) and exit. Operators use it when provisioning
    // shared/executor.env; the key used to be hardcoded in the tracked unit.
    if std::env::args().any(|a| a == "--print-pubkey") {
        let key_path = env_or("IDENTITY_KEY_PATH", DEFAULT_IDENTITY_KEY_PATH);
        let identity = AgentIdentity::load_or_generate(Path::new(&key_path))?;
        println!("{}", identity.pubkey_hex());
        return Ok(0);
    }
    let protected_memory_path = env_or("MEMORY_DB_PATH", DEFAULT_MEMORY_DB_PATH);
    aegis_runtime_core::remote_access::validate_deployment(
        &env_or("AEGIS_ACCESS_MODE", ""),
        std::env::var("AEGIS_PUBLIC_URL").ok().as_deref(),
        &env_or("OAUTH_CALLBACK_BIND", "127.0.0.1"),
        &env_or("AEGIS_PUBLIC_RISK_ACCEPTED", ""),
    )?;
    let protected_heirloom_path =
        aegis_runtime_core::memory_heirloom_bridge::experimental_enabled().then(|| {
            resolve_heirloom_event_log_path(
                std::env::var("HEIRLOOM_EVENT_LOG_DB").ok().as_deref(),
                &protected_memory_path,
            )
        });
    aegis_runtime_core::encrypted_sqlite::configure(
        Path::new(&protected_memory_path),
        protected_heirloom_path.as_deref(),
        aegis_runtime_core::encrypted_sqlite::load_secret(
            external_db_key,
            !Path::new(&protected_memory_path).exists()
                && !protected_heirloom_path
                    .as_ref()
                    .is_some_and(|path| path.exists()),
        )?,
    )
    .context("configuring active encrypted databases")?;
    aegis_runtime_core::encrypted_sqlite::preflight_existing()
        .context("verifying active encrypted databases")?;
    // `setup-auth`: bind owner chat to Grok / Codex / an API key BEFORE the
    // first boot (headless twin of Settings → Connect → Use for chat) and
    // exit. Writes the same data files the service opens below.
    if let Some(command) = parse_setup_auth_args(std::env::args().skip(1))? {
        // Operator-facing CLI: keep the memory-migration INFO chatter out of
        // the way unless the operator asked for it via RUST_LOG.
        if std::env::var_os("RUST_LOG").is_none() {
            let _ = aegis_runtime_core::logging::set_level("warn");
        }
        return run_setup_auth(command).await;
    }
    // `skill-opt`: one explicit skill-file evolution command, then exit.
    // Boot and heartbeat do not call this. A round never writes the live
    // SKILL.md; `skill-opt approve` is the only live write.
    match aegis_runtime_core::skill_opt::parse_skill_opt_args(std::env::args().skip(1)) {
        Ok(Some(command)) => {
            if std::env::var_os("RUST_LOG").is_none() {
                let _ = aegis_runtime_core::logging::set_level("warn");
            }
            let code = match aegis_runtime_core::skill_opt::execute_cli(command).await {
                Ok(code) => code,
                Err(err) => {
                    eprintln!("skill-opt: {err}");
                    1
                }
            };
            return Ok(code);
        }
        Ok(None) => {}
        Err(err) => {
            eprintln!("skill-opt: {err}");
            return Ok(2);
        }
    }

    // 1a. If a tunnel URL file exists and is recent (< 1 hour old), use it
    //     to override AEGIS_PUBLIC_URL. This handles ephemeral trycloudflare.com
    //     URLs that change on each tunnel restart.
    let tunnel_url_path = Path::new("/opt/aegis-runtime/shared/tunnel_url");
    if tunnel_url_path.exists() {
        let is_recent = std::fs::metadata(tunnel_url_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|mtime| mtime.elapsed().ok())
            .map(|age| age.as_secs() < TUNNEL_URL_FRESHNESS_SECS)
            .unwrap_or(false);

        if is_recent {
            match std::fs::read_to_string(tunnel_url_path) {
                Ok(url) => {
                    let url = url.trim();
                    if !url.is_empty() {
                        // SAFETY: called during single-threaded startup, before tokio runtime.
                        #[allow(unsafe_code)]
                        unsafe {
                            std::env::set_var("AEGIS_PUBLIC_URL", url)
                        };
                        eprintln!("[Startup] Using tunnel URL from file: {url}");
                    }
                }
                Err(e) => {
                    eprintln!("[Startup] Warning: could not read tunnel URL file: {e}");
                }
            }
        } else {
            eprintln!("[Startup] Tunnel URL file is stale (> 1 hour old), ignoring");
        }
    }

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "Aegis Runtime Protocol starting"
    );
    eprintln!("Single-binary AI agent infrastructure with zero-trust security");
    eprintln!();

    // 1b. Validate critical configuration early (fail-fast).
    // grok build / Codex / OPENAI_API_KEY — exactly one of the three is enough.
    let startup_reasoner = validate_config()?;
    eprintln!("[Reasoner] {}", startup_reasoner.label());

    // 2. Load or generate Ed25519 identity
    let key_path = env_or("IDENTITY_KEY_PATH", DEFAULT_IDENTITY_KEY_PATH);
    let identity = Arc::new(
        AgentIdentity::load_or_generate(Path::new(&key_path))
            .context("loading/generating agent identity")?,
    );
    eprintln!("[Identity] DID: {}", identity.did);

    // 2b. Initialize credential store (AES-256-GCM, key derived from identity)
    let cred_db_path = env_or("CREDENTIAL_DB_PATH", DEFAULT_CREDENTIAL_DB_PATH);
    let aes_key = credential_store::derive_key_from_ed25519(&identity.signing_key_bytes());
    let cred_store_result = aegis_runtime_core::encrypted_sqlite::open(&cred_db_path)
        .map_err(|e| anyhow::anyhow!(e))
        .and_then(|conn| {
            credential_store::CredentialStore::open(conn, aes_key).map_err(|e| anyhow::anyhow!(e))
        });

    // 2c. Load Google OAuth2 credentials (Calendar + Drive).
    let google_auth: Option<Arc<GoogleAuthManager>> = match &cred_store_result {
        Ok(cred_store) => {
            eprintln!("[Credentials] Store at {cred_db_path}");
            match GoogleAuthManager::from_credential_store(cred_store) {
                Ok(Some(mgr)) => {
                    eprintln!(
                        "[GoogleAuth] Loaded OAuth2 credentials ({} scope(s))",
                        mgr.scopes().len()
                    );
                    Some(Arc::new(mgr))
                }
                Ok(None) => {
                    eprintln!(
                        "[GoogleAuth] Not configured (run setup_google(action=\"setup\") or POST /auth/google/setup for localhost manual setup)"
                    );
                    None
                }
                Err(e) => {
                    eprintln!("[GoogleAuth] Warning: failed to load credentials (non-fatal): {e}");
                    None
                }
            }
        }
        Err(e) => {
            eprintln!("[Credentials] Warning: credential store unavailable (non-fatal): {e}");
            eprintln!(
                "[Credentials] Google OAuth2 disabled. Set CREDENTIAL_DB_PATH to a writable path."
            );
            None
        }
    };

    // 3. Initialize memory actor (before sandbox, so skill loading can check DB state)
    let db_path = env_or("MEMORY_DB_PATH", DEFAULT_MEMORY_DB_PATH);
    let max_db_size_gb: f64 = std::env::var("MEMORY_MAX_SIZE_GB")
        .unwrap_or_else(|_| "20".to_string())
        .parse()
        .unwrap_or(20.0);
    let max_db_size_bytes = (max_db_size_gb * 1024.0 * 1024.0 * 1024.0) as u64;
    let l1_cache_size: usize = std::env::var("L1_CACHE_SIZE")
        .unwrap_or_else(|_| "50".to_string())
        .parse()
        .unwrap_or(50);
    let mut memory = MemoryHandle::spawn_with_config(
        Path::new(&db_path),
        aegis_runtime_core::memory::MemoryConfig {
            l1_max_size: l1_cache_size,
            max_db_size: max_db_size_bytes,
        },
    )
    .context("spawning memory actor")?;
    eprintln!("[Memory] SQLite at {db_path} (max {max_db_size_gb} GB, L1={l1_cache_size})");

    // ── Phase E (W17) — CTO round-1 FIX: wire HeirloomBridge BEFORE any
    //     `memory.clone()` ─────────────────────────────────────────────────
    //
    // `MemoryHandle` derives `Clone` and `heirloom_bridge` is an
    // `Option<Arc<HeirloomBridge>>` — each clone carries its OWN independent
    // Option instance. If the bridge is installed AFTER any downstream clone
    // (orchestrator, sandbox, telegram, etc.), those clones retain `bridge = None`
    // and every shim write through them is silently legacy-only. The
    // previous shadow-binding pattern at the end of init was a no-op for
    // every engine constructed earlier — Phase E was fully dormant in
    // production. See `feedback_paired_fix_grep_siblings.md`: grep siblings
    // of any mutation; here we found 20+ `memory.clone()` sites.
    //
    // DID source-of-truth: `did:aegis:telegram:{ADMIN_CHAT_ID}`, with the
    // `OWNER_DID` env var as an override. This matches the formula used by
    // Orchestrator user_did (orchestrator.rs:4004/4023/4288), so the heirloom
    // log stays single-DID.
    //
    // Only `AEGIS_RUNTIME_HEIRLOOM_BRIDGE=on` supplies an active path.
    // On any setup failure (connection-open,
    // ensure_tables, bootstrap), emits a bilingual warn (FIX 10) and
    // continues in degraded mode (bridge=None, legacy-only writes —
    // identical to pre-Phase-E).
    let admin_chat_id_for_bridge: i64 = std::env::var("ADMIN_CHAT_ID")
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .unwrap_or(0);
    let owner_did_str = std::env::var("OWNER_DID")
        .unwrap_or_else(|_| format!("did:aegis:telegram:{}", admin_chat_id_for_bridge));
    if protected_heirloom_path.is_some() {
        eprintln!(
            "[Heirloom] EXPERIMENTAL: development custody, internal plaintext codec and permissive ethics; no production custody guarantee"
        );
    }
    memory = match build_heirloom_bridge(&owner_did_str, protected_heirloom_path.as_deref()) {
        Some(bridge) => {
            eprintln!("[Heirloom] Experimental dual-write enabled (owner_did={owner_did_str})");
            memory.with_heirloom_bridge(bridge)
        }
        None => {
            if protected_heirloom_path.is_some() {
                eprintln!("[Heirloom] Initialization failed — legacy-only writes");
            } else {
                eprintln!(
                    "[Heirloom] Disabled — legacy memory only; existing heirloom files left untouched"
                );
            }
            memory
        }
    };

    // Register global LLM usage → DB channel so every log_llm_usage() call
    // is automatically persisted to llm_call_log.
    aegis_runtime_core::llm_client::init_llm_log_db(memory.clone());

    // Hydrate process-local pricing cache before any backfill or LLM logging
    // relies on it. Missing rows remain honest NULLs until pricing is fetched.
    match aegis_runtime_core::encrypted_sqlite::open(&db_path) {
        Ok(conn) => {
            let count = aegis_runtime_core::llm_pricing::load_pricing_from_db(&conn);
            if count > 0 {
                eprintln!("[Pricing] Loaded {count} model pricing rows into cache");
            }
        }
        Err(e) => eprintln!("[Pricing] Warning: failed to open DB for pricing cache: {e}"),
    }

    // Initialize HNSW vector index for O(log N) semantic search
    {
        use aegis_runtime_core::hnsw_index::{HnswConfig, HnswIndex};
        let hnsw = std::sync::Arc::new(HnswIndex::new(HnswConfig::default()));
        // Build index from existing embeddings (blocks until complete)
        let build_conn = aegis_runtime_core::encrypted_sqlite::open(&db_path)
            .context("opening DB for HNSW build")?;
        match hnsw.build_from_db(&build_conn, &identity.did) {
            Ok(n) if n > 0 => {
                eprintln!("[HNSW] Built index with {n} embeddings (O(log N) search enabled)")
            }
            Ok(_) => eprintln!("[HNSW] No embeddings found, index will build incrementally"),
            Err(e) => {
                eprintln!("[HNSW] Warning: index build failed (falling back to SQL scan): {e}")
            }
        }
        drop(build_conn);
        memory.set_hnsw_index(hnsw);
    }

    // Restore L1 cache from previous session (if any)
    match memory.restore_l1().await {
        Ok(0) => {} // No snapshot to restore — fresh start
        Ok(n) => eprintln!("[Memory] Restored L1 cache for {n} agent(s) from previous session"),
        Err(e) => eprintln!("[Memory] Warning: L1 restore failed (non-fatal): {e}"),
    }

    // Backfill cost_usd for any LLM call log rows that were recorded before pricing was available.
    match memory.backfill_cost_usd().await {
        Ok(result) if result.rows_updated > 0 => {
            eprintln!(
                "[CostTracking] Backfilled cost_usd for {} rows ({} still unpriced, {} models missing)",
                result.rows_updated,
                result.rows_still_null,
                result.models_missing.len()
            );
        }
        Ok(_) => {} // Nothing to backfill
        Err(e) => eprintln!("[CostTracking] Warning: backfill failed (non-fatal): {e}"),
    }

    // 3b. Initialize runtime settings (persisted in SQLite, cached in memory)
    let settings = Arc::new(RuntimeSettings::new());
    // Load persisted settings from SQLite
    match memory.load_all_settings().await {
        Ok(persisted) if !persisted.is_empty() => {
            eprintln!(
                "[Settings] Loaded {} persisted setting(s) from SQLite",
                persisted.len()
            );
            let report = settings.load_from_map(persisted);
            // Persist legacy key migrations to SQLite (delete old, insert new)
            for (old_key, new_key) in &report.key_renames {
                if let Some(val) = settings.get(new_key) {
                    if let Err(e) = memory.store_setting(new_key, val).await {
                        tracing::warn!(key = new_key, error = %e, "Settings migration: failed to store new key");
                    }
                    if let Err(e) = memory.delete_setting(old_key).await {
                        tracing::warn!(key = old_key, error = %e, "Settings migration: failed to delete old key");
                    }
                }
                eprintln!("[Settings] Migrated legacy key: {old_key} → {new_key}");
            }
            // Same-key rewrites (retired model ids). Store the new value; do not
            // delete the key, or the next boot would lose the setting entirely.
            for (key, value) in &report.value_rewrites {
                if let Err(e) = memory.store_setting(key, value.clone()).await {
                    tracing::warn!(key, error = %e, "Settings migration: failed to store rewritten value");
                }
                eprintln!("[Settings] Rewrote retired setting value: {key}");
            }
        }
        Ok(_) => {} // No persisted settings — use defaults
        Err(e) => eprintln!("[Settings] Warning: failed to load settings (non-fatal): {e}"),
    }
    settings_audit::log_runtime_settings_audit(&settings);
    // 4. Initialize Wasm sandbox and load skills (skips deactivated skills via DB check)
    // 4.0 Auto-register built-in skills so deactivate/reactivate works for all skills.
    auto_register_builtin_skills(&memory, Path::new("compiled_skills")).await?;

    // A skill is only truly deactivated if it has a deactivated registry row and
    // NO active one — a skill can carry rows from several versions at once.
    let active_skill_names: std::collections::HashSet<String> = memory
        .list_skill_proposals(None, Some(STATUS_ACTIVE))
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.skill_name)
        .collect();
    let deactivated_skills: std::collections::HashSet<String> = memory
        .list_skill_proposals(None, Some(STATUS_DEACTIVATED))
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.skill_name)
        .filter(|name| !active_skill_names.contains(name))
        .collect();
    let sandbox = SandboxEngine::new().context("creating sandbox engine")?;
    sandbox.set_system_did(identity.did.clone());
    sandbox.set_memory_handle(memory.clone());
    load_skills_from_directory(&sandbox, Path::new("compiled_skills"), &deactivated_skills)?;

    // 4b. Inject auth headers for known services
    let auth_headers = auth_headers_from_env();
    if !auth_headers.is_empty() {
        let services: Vec<&str> = auth_headers.keys().map(|k| k.as_str()).collect();
        eprintln!("[Auth] Static auth headers for: {}", services.join(", "));
    }
    sandbox.set_auth_headers(auth_headers);

    // 4b-ii. Register Google OAuth2 dynamic auth for Calendar and Drive.
    if let Some(ref gauth) = google_auth {
        sandbox.set_dynamic_auth(&["www.googleapis.com"], gauth.clone());
        eprintln!("[Auth] Google OAuth2 dynamic auth for: www.googleapis.com");
    }

    // 4c. Register all loaded skills as toggleable settings
    for cap in sandbox.loaded_skills() {
        // Preserve legacy defaults for the original skills (e.g. server_monitor)
        let default =
            aegis_runtime_core::settings::legacy_skill_default(&cap.skill_name).unwrap_or(true);
        settings.register_skill(&cap.skill_name, &cap.description, default);
    }

    let sandbox = Arc::new(sandbox);

    // Hydrate ReplayGuard from SQLite (cross-restart replay protection)
    if let Err(e) = sandbox.replay_guard().load_from_db(&memory).await {
        tracing::warn!(error = %e, "Failed to load replay nonces from DB (starting fresh)");
    }

    let skill_count = sandbox.loaded_skills().len();
    eprintln!("[Sandbox] Loaded {skill_count} skills (all registered as toggleable settings)");

    // 5. Initialize embedding client (semantic memory)
    // Embedding can use a separate provider (e.g. Gemini) from the LLM (OpenRouter).
    // Falls back to OPENAI_* if EMBEDDING_* not set, for backward compatibility.
    // A usable OPENAI_API_KEY is one of the three startup reasoners. When the
    // install is grok-build / Codex only, keep an empty key so the OpenAI-
    // compatible client is constructed but does not claim a real provider.
    let api_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|key| aegis_runtime_core::startup_reasoner::llm_api_key_usable(Some(key)))
        .unwrap_or_default();
    let api_base = std::env::var("OPENAI_API_BASE").unwrap_or_else(|_| {
        if api_key.is_empty() {
            String::new()
        } else {
            "https://openrouter.ai/api/v1".to_string()
        }
    });
    let embedding_api_base =
        std::env::var("EMBEDDING_API_BASE").unwrap_or_else(|_| api_base.clone());
    let embedding_api_key = std::env::var("EMBEDDING_API_KEY").unwrap_or_else(|_| api_key.clone());
    // Settings-first startup priority (2026-04-18): runtime setting (SQLite)
    // > env var (.env) > hard-coded default. In production the user cannot
    // edit .env — they propose changes via `update_setting` / HITL, and the
    // live client hot-reloads via `EmbeddingClient::apply_settings_update`.
    // The env var remains a first-boot bootstrap and ops-only override.
    //
    // IMPORTANT: use `get_if_stored`, NOT `get`. `RuntimeSettings::new()`
    // pre-seeds `values` with every registered default, so `get` would
    // always return `Some(default)` and the env-var fallback would never
    // fire — that bug shipped in 519469c, causing the service to ignore
    // `EMBEDDING_MODEL=google/gemini-embedding-001` until the user stored
    // the key explicitly via Telegram. `get_if_stored` returns `Some`
    // only when the value was actually persisted (via `set` / `load_from_map`).
    let embedding_model = settings
        .get_if_stored("embedding_primary_model")
        .and_then(|v| v.as_str().map(String::from))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("EMBEDDING_MODEL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "openai/text-embedding-3-small".to_string());
    // Optional: backup embedding model for primary/backup auto-failover.
    // Must share api_base / api_key / dimensions with the primary.
    // If unset, failover is disabled and primary errors bubble up as before.
    let embedding_backup_model = settings
        .get_if_stored("embedding_backup_model")
        .and_then(|v| v.as_str().map(String::from))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("EMBEDDING_BACKUP_MODEL").ok())
        .filter(|s| !s.trim().is_empty());
    let embedding_dims: u32 = settings
        .get_if_stored("embedding_dimensions")
        .and_then(|v| v.as_u64())
        .map(|n| (n as u32).clamp(1, 16_384))
        .or_else(|| {
            std::env::var("EMBEDDING_DIMENSIONS")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .map(|n| n.clamp(1, 16_384))
        })
        .unwrap_or(512);
    // Optional: override the recovery-probe cooldown (seconds). Default 60s.
    // Clamped to [1, 86400] so a typo doesn't disable recovery (0s) or park it
    // past the interval of any reasonable process lifetime.
    let embedding_probe_cooldown_secs: u64 = settings
        .get_if_stored("embedding_probe_cooldown_secs")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1, 86_400))
        .or_else(|| {
            std::env::var("EMBEDDING_PROBE_COOLDOWN_SECS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .map(|v| v.clamp(1, 86_400))
        })
        .unwrap_or(60);
    // Optional: override the 24h permanent-promotion threshold. Default 86400s
    // (24h). Clamped to [3600, 604_800] so the promotion can't trigger on
    // spurious minute-long blips (< 1 h) or be parked past a week.
    let embedding_promote_after_secs: u64 = settings
        .get_if_stored("embedding_promote_after_secs")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(3_600, 604_800))
        .or_else(|| {
            std::env::var("EMBEDDING_PROMOTE_AFTER_SECS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .map(|v| v.clamp(3_600, 604_800))
        })
        .unwrap_or(86_400);

    // Back-fill the runtime settings cache with the resolved values so that
    // subsequent reads (tests, tool observability, reset-to-default) see the
    // actual live values, not the registered-default placeholder. Failures
    // are non-fatal — the client already holds the resolved values directly.
    if let Err(e) = settings.set(
        "embedding_primary_model",
        serde_json::Value::String(embedding_model.clone()),
    ) {
        tracing::warn!(error = %e, "Failed to back-fill embedding_primary_model");
    }
    if let Err(e) = settings.set(
        "embedding_backup_model",
        serde_json::Value::String(embedding_backup_model.clone().unwrap_or_default()),
    ) {
        tracing::warn!(error = %e, "Failed to back-fill embedding_backup_model");
    }
    if let Err(e) = settings.set(
        "embedding_dimensions",
        serde_json::Value::Number(serde_json::Number::from(embedding_dims)),
    ) {
        tracing::warn!(error = %e, "Failed to back-fill embedding_dimensions");
    }
    if let Err(e) = settings.set(
        "embedding_probe_cooldown_secs",
        serde_json::Value::Number(serde_json::Number::from(embedding_probe_cooldown_secs)),
    ) {
        tracing::warn!(error = %e, "Failed to back-fill embedding_probe_cooldown_secs");
    }
    if let Err(e) = settings.set(
        "embedding_promote_after_secs",
        serde_json::Value::Number(serde_json::Number::from(embedding_promote_after_secs)),
    ) {
        tracing::warn!(error = %e, "Failed to back-fill embedding_promote_after_secs");
    }

    // Startup: honor a persisted permanent-promotion. If a previous run wrote
    // `embedding_primary_promoted_to` to user_settings, that value becomes the
    // live primary (overriding the env var) and the backup slot is forced to
    // None per the "abandoned old primary" design. The env originals are
    // still passed through so the hot-reload revert tool
    // `unset_embedding_primary_promotion` can restore them in-process without
    // a service restart.
    let persisted_settings_map = memory.load_all_settings().await.unwrap_or_default();
    let persisted_promotion_to = persisted_settings_map
        .get("embedding_primary_promoted_to")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let persisted_promotion_at = persisted_settings_map
        .get("embedding_primary_promoted_at")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown)")
        .to_string();

    let (effective_primary, effective_backup) = match persisted_promotion_to.as_deref() {
        Some(promoted) => {
            // Escalated from info→warn per CTO round-1: a stale DB promotion
            // silently overriding the env var is an operator-surprising state
            // and must be visible in default-level logs.
            tracing::warn!(
                env_primary = %embedding_model,
                db_primary = %promoted,
                at = %persisted_promotion_at,
                "[Embedding] Honoring persisted promotion \u{2014} env primary `{embedding_model}` OVERRIDDEN by DB promotion `{promoted}` from {persisted_promotion_at}. Call unset_embedding_primary_promotion to revert (hot-reload, no restart needed)."
            );
            eprintln!(
                "[Embedding] \u{26a0}\u{fe0f}  Honoring persisted promotion: env primary `{embedding_model}` OVERRIDDEN by promotion `{promoted}` from {persisted_promotion_at}. Run unset_embedding_primary_promotion to revert (hot-reload)."
            );
            // Per design §4: when a promotion is active, force backup=None.
            // The override model has no backup until an operator manually
            // reconfigures (or calls the revert tool to restore env originals).
            (promoted.to_string(), None)
        }
        None => (embedding_model.clone(), embedding_backup_model.clone()),
    };

    let embedder = EmbeddingClient::new_with_env_and_promote_threshold(
        &embedding_api_base,
        &embedding_api_key,
        &effective_primary,
        effective_backup.clone(),
        // Env originals — separate from effective so the hot-reload revert
        // path has a snapshot of the boot-time config to restore.
        &embedding_model,
        embedding_backup_model.clone(),
        embedding_dims,
        std::time::Duration::from_secs(embedding_probe_cooldown_secs),
        embedding_promote_after_secs,
    );
    match effective_backup.as_deref() {
        Some(backup) => eprintln!(
            "[Embedding] {effective_primary} ({embedding_dims}d), backup={backup}, probe_cooldown={embedding_probe_cooldown_secs}s, promote_after={embedding_promote_after_secs}s"
        ),
        None => eprintln!(
            "[Embedding] {effective_primary} ({embedding_dims}d), backup=disabled, probe_cooldown={embedding_probe_cooldown_secs}s, promote_after={embedding_promote_after_secs}s"
        ),
    }
    if let Some(reason) = embedder.not_configured_reason() {
        eprintln!(
            "[Embedding] \u{26a0}\u{fe0f}  NOT CONFIGURED ({reason}) — L2 semantic memory disabled. \
             Do not ignore this as a silent 401. Install the local dependency \
             (Oracle-aligned Ollama embedder): scripts/ensure-local-embedding.sh \
             — or set EMBEDDING_API_BASE + EMBEDDING_API_KEY / a usable OPENAI_API_KEY."
        );
        tracing::error!(
            reason = %reason,
            "[Embedding] not configured — run scripts/ensure-local-embedding.sh for L2"
        );
    }

    // 5b. Embedding corpus consent check.
    //
    // Detects when the live primary model differs from the model that produced
    // the persisted L2 corpus (cross-model cosine becomes meaningless). The
    // check runs once at startup:
    //   - FreshInstall (no marker + no rows) → write marker, no alert.
    //   - Match → no-op.
    //   - Mismatch → queue a one-shot bilingual Telegram alert for the admin.
    //     The alert is ONLY sent; the marker is NOT auto-updated (the user
    //     decides via `trigger_corpus_reembed`).
    // See `docs/DEPLOYMENT.md §3.3.3` for full semantics.
    let pending_corpus_alert: Option<String> =
        match aegis_runtime_core::embedding_corpus::detect_corpus_mismatch(&memory, &embedder).await
        {
            Ok(check) => match check.status {
                aegis_runtime_core::embedding_corpus::CorpusStatus::FreshInstall => {
                    if let Err(e) = aegis_runtime_core::embedding_corpus::write_marker(
                        &memory,
                        &check.live_primary,
                    )
                    .await
                    {
                        tracing::warn!(error = %e, "[EmbeddingCorpus] failed to write fresh-install marker");
                    } else {
                        tracing::info!(model = %check.live_primary, "[EmbeddingCorpus] fresh install \u{2014} marker initialized");
                    }
                    None
                }
                aegis_runtime_core::embedding_corpus::CorpusStatus::Match => {
                    tracing::debug!(model = %check.live_primary, rows = check.vector_count, "[EmbeddingCorpus] marker matches live primary");
                    None
                }
                aegis_runtime_core::embedding_corpus::CorpusStatus::Mismatch => {
                    tracing::warn!(
                        previous = ?check.marker_before,
                        current = %check.live_primary,
                        rows = check.vector_count,
                        "[EmbeddingCorpus] \u{26a0}\u{fe0f} corpus model MISMATCH \u{2014} semantic recall will be degraded until user approves trigger_corpus_reembed"
                    );
                    eprintln!(
                        "[EmbeddingCorpus] \u{26a0}\u{fe0f}  Primary embedding model changed (marker={:?} live={}). {} vectors are cross-model. Semantic recall will degrade until user approves `trigger_corpus_reembed`.",
                        check.marker_before, check.live_primary, check.vector_count
                    );
                    Some(aegis_runtime_core::embedding_corpus::build_mismatch_alert_html(&check))
                }
            },
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "[EmbeddingCorpus] consistency check failed (non-fatal, startup continues)");
                None
            }
        };

    // 6. Initialize orchestrator (LLM + permission inversion)
    // Model priority: persisted setting (SQLite) > env var > default.
    // Uses `get_if_stored` for the same reason the embedding block does —
    // otherwise the registered default would mask the env var silently.
    // Empty llm_model means "provider default" (codex_model / xai_subscription_model).
    // Do not fall back to a hard-coded Gemini id — production may already store a
    // legacy value, and that is left untouched when present.
    let model = settings
        .get_if_stored("llm_model")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("LLM_MODEL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_default();

    // Store resolved model in settings (so active_model() reads it at runtime)
    if let Err(e) = settings.set("llm_model", serde_json::Value::String(model.clone())) {
        tracing::warn!(error = %e, "Failed to set llm_model in settings");
    }

    if (startup_reasoner.is_external_executor()
        || settings
            .get("llm_provider")
            .and_then(|v| v.as_str().map(str::to_owned))
            .is_some_and(|p| matches!(p.as_str(), "codex_oauth" | "xai_subscription")))
        && settings
            .get_if_stored("external_executor_enabled")
            .is_none()
    {
        if let Err(e) = settings.set("external_executor_enabled", serde_json::Value::Bool(true)) {
            tracing::warn!(
                error = %e,
                "Failed to enable external_executor_enabled for CLI-only startup"
            );
        } else {
            eprintln!(
                "[Reasoner] external_executor_enabled=true (startup via {})",
                startup_reasoner.label()
            );
        }
    }

    // 6a. Discover external Agent Skills (SKILL.md format)
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let discovered_agent_skills = agent_skills::discover_agent_skills(&workspace_root);
    if !discovered_agent_skills.is_empty() {
        eprintln!(
            "[AgentSkills] Discovered {} external skill(s)",
            discovered_agent_skills.len()
        );
    }

    let mut orchestrator = Orchestrator::new(
        &api_key,
        &api_base,
        &model,
        sandbox.clone(),
        identity.clone(),
        memory.clone(),
        embedder.clone(),
        settings.clone(),
    );
    orchestrator.set_agent_skills(discovered_agent_skills);
    if let Some(hc) = aegis_runtime_core::hercules_client::HerculesClient::from_env() {
        tracing::info!(
            base = hc.base(),
            production = hc.is_production(),
            "Hercules delegation enabled (delegate_task)"
        );
        orchestrator.set_hercules_client(Arc::new(hc));
    }
    let mut external_executors =
        aegis_runtime_core::orchestrator_tools_external::ExternalExecutors::from_settings(
            &settings,
        );
    // `from_env()` still falls back to the legacy /opt path unconditionally;
    // resolve the scratch parent here so a checkout that is not installed under
    // /opt keeps its runs next to its own data (see `resolve_executor_runs_dir`).
    external_executors.runs_dir = resolve_executor_runs_dir(
        std::env::var("AEGIS_EXECUTOR_RUNS_DIR").ok().as_deref(),
        std::env::var("MEMORY_DB_PATH").ok().as_deref(),
        |dir| dir.is_dir(),
    );
    let external_executors = Arc::new(external_executors);
    tracing::info!(
        grok_bin = %external_executors.grok_bin,
        codex_bin = %external_executors.codex_bin,
        runs_dir = %external_executors.runs_dir.display(),
        "External executors wired (run_external)"
    );
    orchestrator.set_external_executors(external_executors);

    // 6b-i. Initialize MCP client (external MCP server connections)
    let mcp_client: Option<Arc<aegis_runtime_core::mcp_client::McpClientManager>> = {
        match aegis_runtime_core::mcp_client::load_config() {
            Ok(config) if !config.servers.is_empty() => {
                let mgr = Arc::new(aegis_runtime_core::mcp_client::McpClientManager::new());
                let results = mgr.connect_all(&config).await;
                let mut connected = 0;
                let mut failed = 0;
                for (name, result) in &results {
                    match result {
                        Ok(()) => {
                            connected += 1;
                        }
                        Err(e) => {
                            failed += 1;
                            eprintln!("[MCP] Failed to connect to '{}': {:#}", name, e);
                        }
                    }
                }
                let total_tools: usize = mgr.get_all_tools().len();
                eprintln!(
                    "[MCP] {connected}/{} server(s) connected, {total_tools} tool(s) available",
                    config.servers.len()
                );
                if failed > 0 {
                    eprintln!("[MCP] {failed} server(s) failed to connect (will retry on use)");
                }
                orchestrator.set_mcp_client(mgr.clone());
                Some(mgr)
            }
            Ok(_) => {
                eprintln!(
                    "[MCP] No MCP servers configured (set MCP_SERVERS_CONFIG or create ~/.aegis-runtime/mcp_servers.json)"
                );
                None
            }
            Err(e) => {
                eprintln!("[MCP] Warning: failed to load MCP config (non-fatal): {e:#}");
                None
            }
        }
    };

    // 6b-iii. Wire progress feedback channel for real-time Telegram typing indicators.
    // The channel now carries typed `ProgressEvent`s; the `ProgressRouter` fans each
    // event to this Telegram sink (decoded byte-identically to the legacy string) and,
    // for registered turns, to a per-turn webapp sink (later slices). `progress_rx` is
    // drained by `spawn_progress_listener`, which decodes back to the Telegram string.
    let (progress_tx, progress_rx) =
        tokio::sync::mpsc::channel::<aegis_runtime_core::progress_router::ProgressEvent>(256);
    let progress_router = aegis_runtime_core::progress_router::ProgressRouter::new(progress_tx);
    orchestrator.set_progress_tx(progress_router.progress_sender());
    // Slice 2: share the SAME `ProgressRouter` instance between the orchestrator
    // (which emits through `progress_sender()`) and `AdminState` (which
    // `register_turn`s per-turn webapp SSE sinks). Because both hold clones of
    // the one `Arc<RouterInner>`, a turn registered on the AdminState side is
    // visible to the orchestrator's sender, so its typed events fan out to the
    // SSE stream. Stored in AdminState below.
    let progress_router = std::sync::Arc::new(progress_router);

    // Intent event channel (shared by the deferred intent engine and its producers)
    let (intent_event_tx, intent_event_rx) = tokio::sync::mpsc::channel::<IntentEvent>(256);

    // Wire Telegram bot to orchestrator for outbound photo/voice sending
    if std::env::var("TELOXIDE_TOKEN").is_ok() {
        orchestrator.set_telegram_bot(teloxide::Bot::from_env());
    }

    // Wire credential store to orchestrator for Google OAuth setup
    if cred_store_result.is_ok() {
        orchestrator.set_credential_store(cred_db_path.clone(), aes_key);
    }
    if let Ok(public_url) = std::env::var("AEGIS_PUBLIC_URL")
        && !public_url.is_empty()
    {
        orchestrator.set_public_url(public_url.clone());
        eprintln!("[OAuth] Public URL configured: {public_url}");
    }

    // Pre-store Google OAuth credentials from env vars (if provided)
    if let (Ok(env_cid), Ok(env_cs)) = (
        std::env::var("GOOGLE_CLIENT_ID"),
        std::env::var("GOOGLE_CLIENT_SECRET"),
    ) && !env_cid.is_empty()
        && !env_cs.is_empty()
        && google_auth.is_none()
    {
        // Only pre-store if OAuth is not already configured
        if let Err(e) = orchestrator.store_google_client_credentials(&env_cid, &env_cs) {
            eprintln!("[OAuth] Warning: failed to pre-store Google client credentials: {e}");
        } else {
            eprintln!("[OAuth] Pre-stored Google OAuth client credentials from env");
        }
    }

    let orchestrator = Arc::new(orchestrator);

    // ── MCP Server mode ─────────────────────────────────────────────────
    // When AEGIS_MCP_SERVER=1, run as an MCP server subprocess on stdio
    // for Claude Desktop / Cursor / VS Code. No Telegram, no background
    // services — just tool execution via the standard gate chain.
    if std::env::var("AEGIS_MCP_SERVER").is_ok_and(|v| v == "1") {
        use aegis_runtime_core::mcp_server::{AegisMcpServer, ToolExecutor, ToolKind};

        eprintln!("[MCP] Starting in MCP server mode (stdio)");

        // Build executor that routes tool calls through the orchestrator
        let orch_for_mcp = orchestrator.clone();
        let executor: ToolExecutor = Arc::new(move |name: String, args: serde_json::Value| {
            let orch = orch_for_mcp.clone();
            Box::pin(async move {
                let result_str = orch.execute_mcp_tool(&name, args).await?;
                // Parse as JSON if possible, else wrap as string value
                match serde_json::from_str::<serde_json::Value>(&result_str) {
                    Ok(v) => Ok(v),
                    Err(_) => Ok(serde_json::Value::String(result_str)),
                }
            })
        });

        let mcp_server = AegisMcpServer::new(executor);

        // Register pseudo-tools (curated subset — excludes Telegram-specific tools)
        let pseudo_schemas = aegis_runtime_core::tool_schemas::mcp_pseudo_tool_schemas();
        mcp_server.register_from_oai_schemas(&pseudo_schemas, ToolKind::PseudoTool);

        // Register Wasm skills
        let loaded_skills = sandbox.loaded_skills();
        mcp_server.register_wasm_skills(&loaded_skills);

        let tool_count = mcp_server.tool_count();
        eprintln!(
            "[MCP] {} tool(s) registered ({} pseudo + {} wasm)",
            tool_count,
            pseudo_schemas.len(),
            loaded_skills.len(),
        );

        // Serve on stdio (blocks until client disconnects)
        aegis_runtime_core::mcp_server::serve_stdio(mcp_server).await?;

        // Graceful shutdown
        graceful_shutdown(&memory).await;
        return Ok(0);
    }

    eprintln!("[LLM] Model: {model}");

    let embedder_for_chronos = embedder.clone();
    let embedder_for_reembed = embedder.clone();
    let embedder_for_probe = embedder.clone();

    // 6f. Start daily subcall storage governance (design §7.5, Task 1.8):
    //     TTL sweep (90d age + 30d accessed_at) + LRU soft-cap enforcement
    //     (500 MB). Sleep-based daily schedule targeting 04:00 UTC.
    //     Warn-on-error only; losing a sweep is recoverable.
    let _subcall_gc_handle = {
        let memory = memory.clone();
        tokio::spawn(async move {
            use aegis_runtime_core::subcall::LRU_TOTAL_BYTES_CAP;
            tracing::info!("[SubcallGC] Storage governance task started");
            // Compute duration until the next 04:00 UTC boundary so the
            // first tick aligns even if the process starts mid-day. After
            // each run, sleep ≥23h to avoid double-runs from clock drift.
            fn duration_until_next_utc(hour: u32, minute: u32) -> std::time::Duration {
                let now = chrono::Utc::now();
                let today_target = now
                    .date_naive()
                    .and_hms_opt(hour, minute, 0)
                    .unwrap_or_default();
                let today_target = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                    today_target,
                    chrono::Utc,
                );
                let target = if now < today_target {
                    today_target
                } else {
                    today_target + chrono::Duration::days(1)
                };
                let diff = (target - now).num_seconds().clamp(60, 86400) as u64;
                std::time::Duration::from_secs(diff)
            }

            loop {
                let sleep_dur = duration_until_next_utc(4, 0);
                tracing::info!(
                    sleep_secs = sleep_dur.as_secs(),
                    "[SubcallGC] Sleeping until next 04:00 UTC governance tick"
                );
                tokio::time::sleep(sleep_dur).await;

                match memory.subcall_ttl_sweep().await {
                    Ok(n) => {
                        tracing::info!(deleted = n, "[SubcallGC] TTL sweep complete");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "[SubcallGC] subcall_ttl_sweep failed");
                    }
                }
                match memory.subcall_enforce_lru_cap(LRU_TOTAL_BYTES_CAP).await {
                    Ok(n) => {
                        tracing::info!(evicted = n, "[SubcallGC] LRU cap enforcement complete");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "[SubcallGC] subcall_enforce_lru_cap failed");
                    }
                }

                // Sleep ≥23h to avoid double-runs from clock drift / re-entry.
                tokio::time::sleep(std::time::Duration::from_secs(23 * 3600)).await;
            }
        })
    };
    eprintln!(
        "[SubcallGC] Storage governance task started (daily at 04:00 UTC, 90d TTL + 500 MB LRU cap)"
    );

    // 6g. Hourly safety-net sweep for the in-memory SubcallRegistry
    //     (design §10 Risk #4). Each user turn / heartbeat tick creates a
    //     SessionGcGuard that removes its session on Drop; the hourly sweep
    //     reclaims any session whose guard was skipped (e.g. process panic
    //     without unwind). Threshold 1 hour — any session that has been
    //     idle that long is safe to evict because an active user turn
    //     would have touched last_seen more recently.
    let _subcall_registry_sweep_handle = {
        let registry = orchestrator.subcall_registry().clone();
        tokio::spawn(async move {
            tracing::info!("[SubcallGC] Session registry sweep task started");
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(3600));
            // Skip the first immediate tick so the log line above is the only
            // "task started" signal post-deploy checks need to key off.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let swept = registry.sweep_stale_sessions(chrono::Duration::hours(1));
                if swept > 0 {
                    tracing::info!(
                        swept,
                        "[SubcallGC] Registry hourly sweep removed stale sessions"
                    );
                }
            }
        })
    };

    // 7. Start axum admin control panel
    let axum_port: u16 = std::env::var("AXUM_PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .unwrap_or(3000);

    let admin_api_token = std::env::var("ADMIN_API_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    if admin_api_token.is_none() {
        eprintln!(
            "[Admin] Warning: ADMIN_API_TOKEN not set. \
             Protected endpoints will return 403 until configured."
        );
    }

    // Local login link (README on-ramp). Policy lives in
    // `local_login_token_from_lookup`: an explicit AEGIS_LOCAL_LOGIN_TOKEN, or
    // a random token minted here when neither Google Login nor Telegram can
    // vouch for the owner; "off" or an existing principal disables it.
    let local_login = if std::env::var("AEGIS_PUBLIC_URL").is_ok_and(|url| !url.is_empty()) {
        None
    } else {
        aegis_runtime_core::web_console_auth::local_login_token_from_lookup(
            |key| std::env::var(key).ok(),
            aegis_runtime_core::web_console_auth::generate_local_login_token,
        )?
    };

    let admin_state = Arc::new(AdminState {
        sandbox: sandbox.clone(),
        memory: memory.clone(),
        api_token: admin_api_token,
        settings: settings.clone(),
        credential_db_path: if cred_store_result.is_ok() {
            Some(cred_db_path.clone())
        } else {
            None
        },
        credential_aes_key: if cred_store_result.is_ok() {
            Some(aes_key)
        } else {
            None
        },
        google_auth: parking_lot::RwLock::new(google_auth.clone()),
        telegram_bot: if std::env::var("TELOXIDE_TOKEN").is_ok() {
            Some(teloxide::Bot::from_env())
        } else {
            None
        },
        identity: Some(identity.clone()),
        public_url: std::env::var("AEGIS_PUBLIC_URL")
            .ok()
            .filter(|s| !s.is_empty()),
        bot_token: std::env::var("TELOXIDE_TOKEN")
            .ok()
            .filter(|s| !s.is_empty()),
        admin_chat_id: std::env::var("ADMIN_CHAT_ID")
            .ok()
            .and_then(|s| s.parse().ok()),
        orchestrator: Some(orchestrator.clone()),
        event_tx: Some(intent_event_tx.clone()),
        db_path: Some(db_path.clone()),
        progress_router: Some(progress_router.clone()),
        local_login_token: local_login.as_ref().map(|login| login.token.clone()),
    });
    aegis_runtime_core::admin::spawn_vnext_avatar_auto_refresh_loop(admin_state.clone());
    let admin_app = admin_router(admin_state.clone());

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{axum_port}"))
        .await
        .with_context(|| {
            format!(
                "Failed to bind admin API to 127.0.0.1:{axum_port}. \
                 Is another instance running? Check with: ss -tlnp | grep {axum_port}"
            )
        })?;
    eprintln!("[Admin] Control panel at http://127.0.0.1:{axum_port}");
    tokio::spawn(async move {
        axum::serve(listener, admin_app).await.ok();
    });

    // 7a. Console + OAuth callback listener. Always on: it serves /ui/, the
    //     session API and every login route, so a fresh install reaches the
    //     console on loopback. AEGIS_PUBLIC_URL only changes what gets
    //     advertised (Telegram deep links via set_public_url, Google
    //     callbacks); it is not a precondition for listening.
    {
        let public_url_configured = std::env::var("AEGIS_PUBLIC_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some();
        let oauth_port: u16 = std::env::var("OAUTH_CALLBACK_PORT")
            .unwrap_or_else(|_| "8765".to_string())
            .parse()
            .unwrap_or(8765);
        let oauth_bind = aegis_runtime_core::admin::oauth_callback_bind_addr(
            std::env::var("OAUTH_CALLBACK_BIND").ok().as_deref(),
        );
        let display_host = aegis_runtime_core::admin::web_console_display_host(&oauth_bind);
        let callback_app = aegis_runtime_core::admin::oauth_callback_router(admin_state.clone());
        match tokio::net::TcpListener::bind(format!("{oauth_bind}:{oauth_port}")).await {
            Ok(listener) => {
                if public_url_configured {
                    eprintln!("[OAuth] Public callback listener at {oauth_bind}:{oauth_port}");
                } else {
                    eprintln!("[Web Console] http://{display_host}:{oauth_port}/ui/");
                }
                if let Some(local_login) = &local_login {
                    // The token itself goes to stderr only, never to the log.
                    eprintln!(
                        "[Web Console] Local login (valid until restart): \
                         http://{display_host}:{oauth_port}/auth/login/local?token={}",
                        local_login.token
                    );
                    tracing::info!(
                        generated = local_login.generated,
                        "Web Console local login enabled; link printed once to stderr"
                    );
                }
                tokio::spawn(async move {
                    axum::serve(listener, callback_app).await.ok();
                });
            }
            Err(e) => {
                eprintln!(
                    "[Web Console] Warning: failed to bind {oauth_bind}:{oauth_port}: {e}. \
                     The console (/ui/) and the Google OAuth callback will not be reachable; \
                     set OAUTH_CALLBACK_PORT to a free port."
                );
            }
        }
    }

    // 8. Start Chronos heartbeat engine
    let admin_chat_id: i64 = std::env::var("ADMIN_CHAT_ID")
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .unwrap_or(0);

    let has_telegram = std::env::var("TELOXIDE_TOKEN").is_ok();

    if has_telegram && admin_chat_id != 0 {
        let tg_bot = Arc::new(TelegramBot::new(admin_chat_id));

        // Wire orchestrator progress feedback (progressive editing) to Telegram
        aegis_runtime_core::telegram::spawn_progress_listener(
            tg_bot.bot(),
            progress_rx,
            tg_bot.progress_messages.clone(),
            settings.clone(),
            memory.clone(),
            admin_chat_id,
        );

        spawn_scheduled_personal_brief_loop(
            tg_bot.bot(),
            orchestrator.clone(),
            memory.clone(),
            settings.clone(),
            db_path.clone(),
            admin_chat_id,
        );
        eprintln!("[DailyBrief] Scheduled deterministic brief loop started (opt-in)");

        // Durable assistant-event notifier: consumes Calendar/Drive-originated
        // assistant events independently of heartbeat so important events can
        // notify promptly. Lower-signal digest assembly is owned by
        // heartbeat-generated work items instead of this direct notifier lane.
        {
            let notifier_orchestrator = orchestrator.clone();
            let notifier_user_did = format!("did:aegis:telegram:{admin_chat_id}");
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    if let Err(e) = notifier_orchestrator
                        .process_assistant_event_notifications(
                            &notifier_user_did,
                            routed_proactive_progress_chat_id(admin_chat_id),
                            50,
                        )
                        .await
                    {
                        tracing::warn!(
                            error = %e,
                            "[AssistantEvent] notifier tick failed"
                        );
                    }
                }
            });
        }

        // Bounded callback-review work-item runner: consumes durable
        // callback_now reviews separately from the immediate/digest notifier
        // lane so Google-triggered background reviews share one queue and one
        // delivery policy.
        {
            let work_orchestrator = orchestrator.clone();
            let work_user_did = format!("did:aegis:telegram:{admin_chat_id}");
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    if let Err(e) = work_orchestrator
                        .process_callback_review_work_items(
                            &work_user_did,
                            routed_proactive_progress_chat_id(admin_chat_id),
                            10,
                        )
                        .await
                    {
                        tracing::warn!(
                            error = %e,
                            "[AssistantWorkItem] callback-review tick failed"
                        );
                    }
                }
            });
        }

        // Deferred-intent bounded work-item runner: scheduled llm_callback
        // intents now enqueue into the same durable work substrate instead of
        // directly running a separate callback persona.
        {
            let work_orchestrator = orchestrator.clone();
            let work_user_did = format!("did:aegis:telegram:{admin_chat_id}");
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    if let Err(e) = work_orchestrator
                        .process_deferred_intent_work_items(
                            &work_user_did,
                            routed_proactive_progress_chat_id(admin_chat_id),
                            10,
                        )
                        .await
                    {
                        tracing::warn!(
                            error = %e,
                            "[AssistantWorkItem] deferred-intent tick failed"
                        );
                    }
                }
            });
        }

        // Warm embedding caches in the background so the first chat turn after
        // a restart does not pay for skill/tool description embeddings.
        {
            let warm_orchestrator = orchestrator.clone();
            tokio::spawn(async move {
                warm_orchestrator.warm_embedding_caches().await;
            });
        }

        // Hercules delegation reconciliation: refresh open delegated tasks so
        // `delegate_task status` / the Work view reflect Hercules state.
        if orchestrator.hercules_configured() {
            let sync_orchestrator = orchestrator.clone();
            let sync_user_did = format!("did:aegis:telegram:{admin_chat_id}");
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    match sync_orchestrator.process_delegation_sync().await {
                        Ok(transitions) => {
                            for t in &transitions {
                                tracing::info!(
                                    task_id = %t.hercules_task_id,
                                    from = %t.from,
                                    to = %t.to,
                                    work_item_id = ?t.work_item_id,
                                    "[delegation-sync] Hercules task moved"
                                );
                            }
                            if !transitions.is_empty() {
                                sync_orchestrator
                                    .notify_delegation_transitions(
                                        &sync_user_did,
                                        routed_proactive_progress_chat_id(admin_chat_id),
                                        &transitions,
                                    )
                                    .await;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "[delegation-sync] tick failed");
                        }
                    }
                }
            });
        }

        // Heartbeat-generated follow-up runner: synthesis/escalation work is
        // durable and bounded like the other background secretary lanes.
        {
            let work_orchestrator = orchestrator.clone();
            let work_user_did = format!("did:aegis:telegram:{admin_chat_id}");
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    if let Err(e) = work_orchestrator
                        .process_heartbeat_follow_up_work_items(
                            &work_user_did,
                            routed_proactive_progress_chat_id(admin_chat_id),
                            10,
                        )
                        .await
                    {
                        tracing::warn!(
                            error = %e,
                            "[AssistantWorkItem] heartbeat follow-up tick failed"
                        );
                    }
                }
            });
        }

        // Heartbeat-generated digest assembly runner: low-signal mail digest
        // batches now flow through the same durable work substrate instead of
        // being emitted directly by the mail notifier lane.
        {
            let work_orchestrator = orchestrator.clone();
            let work_user_did = format!("did:aegis:telegram:{admin_chat_id}");
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    if let Err(e) = work_orchestrator
                        .process_heartbeat_digest_work_items(
                            &work_user_did,
                            routed_proactive_progress_chat_id(admin_chat_id),
                            4,
                        )
                        .await
                    {
                        tracing::warn!(
                            error = %e,
                            "[AssistantWorkItem] heartbeat digest-assembly tick failed"
                        );
                    }
                }
            });
        }

        // Send the one-shot embedding-corpus consent alert if startup detected
        // a model mismatch. Fire-and-forget: we must not block bot startup on
        // a single send. Failure just means the user won't be notified until
        // they interact — not a safety issue (no data was destroyed).
        if let Some(alert_md) = pending_corpus_alert.clone() {
            let alert_bot = tg_bot.bot();
            let alert_memory = memory.clone();
            tokio::spawn(async move {
                let (alert_chat, _, alert_topic) = routed_telegram_destination(
                    admin_chat_id,
                    NotificationClass::ApprovalRequest,
                    PrivacyLevel::SystemOnly,
                );
                let html = aegis_runtime_core::telegram::markdown_to_telegram_html(&alert_md);
                // Startup consent request for embedding-model migration — requires
                // user approval before wiping + re-embedding the corpus.
                let web_msg = format!(
                    "{alert_md}\n\nWeb action: ask Aegis to run `trigger_corpus_reembed` if you approve re-embedding."
                );
                if aegis_runtime_core::telegram::record_owner_web_notice_if_push_delivered(
                    &alert_memory,
                    admin_chat_id,
                    &web_msg,
                )
                .await
                {
                    return;
                }
                if let Err(e) = aegis_runtime_core::telegram::send_html_to_topic(
                    &alert_bot,
                    alert_chat,
                    alert_topic,
                    &html,
                )
                .await
                {
                    tracing::warn!(error = %e, "[EmbeddingCorpus] failed to send startup consent alert");
                } else {
                    tracing::info!("[EmbeddingCorpus] startup consent alert sent to admin");
                }
            });
        }

        // Start Google OAuth2 token health monitor (hourly validation).
        if google_auth.is_some() {
            let monitor_bot = tg_bot.clone();
            let admin_state_for_monitor = admin_state.clone();
            let monitor_memory = memory.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
                interval.tick().await; // skip immediate first tick
                // A revoked/expired refresh token is permanent until the owner
                // re-authorizes: alert on the edge and re-nag daily, not hourly.
                let mut auth_gate = aegis_runtime_core::alert_gate::AlertGate::new(
                    std::time::Duration::from_secs(24 * 3600),
                    std::time::Duration::from_secs(24 * 3600),
                );
                loop {
                    interval.tick().await;
                    let Some(gauth_check) =
                        aegis_runtime_core::admin::resolve_live_google_auth_manager(
                            &admin_state_for_monitor,
                        )
                    else {
                        continue;
                    };
                    match tokio::task::spawn_blocking(move || gauth_check.validate()).await {
                        Ok(Ok(())) => auth_gate.record_success("google_oauth"),
                        Ok(Err(e)) => {
                            if !auth_gate.record_failure(
                                "google_oauth",
                                1,
                                std::time::Instant::now(),
                            ) {
                                tracing::warn!(error = %e, "Google OAuth2 token still failing (owner already alerted)");
                                continue;
                            }
                            let msg = format!(
                                "⚠️ Google OAuth2 token refresh failed: {e}\n\n\
                                 To re-authorize, open Web Settings and refresh the Google connection. \
                                 Do not paste Google client secrets into chat."
                            );
                            if aegis_runtime_core::telegram::record_owner_web_notice_if_push_delivered(
                                &monitor_memory,
                                admin_chat_id,
                                &msg,
                            )
                            .await
                            {
                                continue;
                            }
                            let html = markdown_to_telegram_html(&msg);
                            let (auth_chat, _, auth_topic) = routed_telegram_destination(
                                admin_chat_id,
                                NotificationClass::SystemStatus,
                                PrivacyLevel::SystemOnly,
                            );
                            if let Err(te) =
                                send_html_to_topic(&monitor_bot.bot(), auth_chat, auth_topic, &html)
                                    .await
                            {
                                tracing::warn!(error = %te, "Failed to send Google auth alert");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Google auth validation task panicked");
                        }
                    }
                }
            });
            eprintln!("[GoogleAuth] Token health monitor started (1h interval)");
        }

        // Start Deferred Intent engine (time poller + event matcher + expiry cleanup)
        let intent_engine = DeferredIntentEngine::new(
            memory.clone(),
            orchestrator.clone(),
            admin_chat_id,
            intent_event_rx,
        );
        let _intent_handles = intent_engine.start(tg_bot.bot());
        eprintln!("[Intent] Deferred intent engine started");

        // Start Chronos with Telegram alerts + semantic triage
        let mut chronos_engine =
            ChronosEngine::new(sandbox.clone(), identity.clone(), settings.clone());
        chronos_engine.set_triage(embedder_for_chronos, memory.clone());
        let chronos = Arc::new(chronos_engine);
        // Restore daily token counter from DB (survives process restarts)
        chronos.restore_daily_tokens_from_db(&memory).await;
        // Wire chronos to orchestrator for monitoring intent pseudo-tools
        orchestrator.set_chronos(chronos.clone());
        // Load monitoring intent cache from L2 at startup
        chronos.refresh_intent_cache(&identity.did).await;
        // P2: Load active prompt overrides from DB
        orchestrator.load_prompt_overrides_from_db().await;
        // Step 1: Load heartbeat agent-loop config (docs/heartbeat-autonomy-design.md §7).
        // Migration of legacy notification_quiet_start/end → heartbeat_config ran
        // inside run_all_migrations at DB open; this populates the in-memory cache.
        orchestrator.load_heartbeat_config_from_db().await;
        let telegram_handler = TelegramHeartbeatHandler::new(tg_bot.bot(), admin_chat_id)
            .with_memory(memory.clone())
            .with_settings(settings.clone())
            .with_action_ledger_db_path(db_path.clone());
        let heartbeat_handler = Arc::new(AutonomousHeartbeatHandler::new(telegram_handler));
        heartbeat_handler.set_orchestrator(orchestrator.clone());

        let has_cron_skills = sandbox
            .loaded_skills()
            .iter()
            .any(|s| s.cron_schedule.is_some());

        // Clone handler and chronos for true heartbeat before passing to scheduler
        let handler_for_true_hb = heartbeat_handler.clone();
        let chronos_for_true_hb = chronos.clone();

        let mut chronos_scheduler = None;
        if has_cron_skills {
            chronos_scheduler = Some(
                chronos
                    .start_scheduler(heartbeat_handler)
                    .await
                    .context("starting Chronos scheduler")?,
            );
            eprintln!("[Chronos] Heartbeat scheduler started");
        }

        // Load True Heartbeat config from user_settings (persisted across restarts)
        if let Some(val) = settings.get("true_heartbeat_config") {
            match serde_json::from_value::<aegis_runtime_core::chronos::TrueHeartbeatConfig>(val) {
                Ok(cfg) => {
                    eprintln!(
                        "[TrueHeartbeat] Loaded config from settings: enabled={}, interval={}min, quiet_hours={:?}",
                        cfg.enabled, cfg.interval_minutes, cfg.quiet_hours
                    );
                    chronos_for_true_hb.set_true_heartbeat_config(cfg);
                }
                Err(e) => {
                    eprintln!("[TrueHeartbeat] Failed to parse config from settings: {e}");
                }
            }
        }

        // Start True Heartbeat (fixed-interval, unconditional LLM wake-up)
        let true_hb_config = chronos_for_true_hb.get_true_heartbeat_config();
        let _true_heartbeat_handle = if true_hb_config.enabled {
            let handle = chronos_for_true_hb.start_true_heartbeat(
                handler_for_true_hb,
                orchestrator.llm_busy_flag(),
                admin_chat_id,
                orchestrator.progress_sender(),
            );
            eprintln!(
                "[TrueHeartbeat] Started — interval {}min, quiet_hours {:?}",
                true_hb_config.interval_minutes, true_hb_config.quiet_hours,
            );
            Some(handle)
        } else {
            eprintln!("[TrueHeartbeat] Disabled (set true_heartbeat.enabled=true to activate)");
            None
        };

        // Task-lifecycle maintenance (auto-archive long-finished tasks + DESTRUCTIVE
        // retention-delete of tasks archived past the configured age) runs in its
        // OWN always-on task, independent of true_heartbeat.enabled — in production
        // the true-heartbeat is disabled, yet these sweeps must still run.
        let _task_lifecycle_handle = chronos_for_true_hb.start_task_lifecycle_maintenance();
        eprintln!("[TaskLifecycle] Maintenance task spawned (archive + retention sweeps)");

        // 9. Reminder polling loop removed — reminders are now unified into the tasks table
        //    and handled by the intent engine's time poller (trigger_type='time', execution_mode='notify')
        eprintln!("[Tasks] Reminders unified into tasks table — handled by intent engine");

        // 9b. Runtime health probe — periodic self-check (Layer 3 verification)
        //
        // Verifies critical subsystems on a configurable cadence:
        // - L2 recall can find known critical data (addresses)
        // - HNSW index is functional
        // - Embedding API is reachable
        // Alerts via Telegram if any check fails.
        {
            let probe_memory = memory.clone();
            // Share failover state with the main embedder so the probe sees the
            // same degraded flag and its "ping" calls contribute to the shared
            // recovery probe record (no split-brain state). Because cloning an
            // EmbeddingClient shares the inner state Arc, peek + clear of the
            // promotion-alert queue here operate on the SAME one-shot set by the
            // working embedder.
            let probe_embedder = embedder_for_probe;
            let probe_bot = tg_bot.bot();
            let probe_did = identity.did.clone();
            // Clone the orchestrator Arc so the probe can flip the owner-visible
            // `embedder_degraded` snapshot on the degraded/recovery transition
            // (webapp status projection reads it; mirrors `llm_degraded`).
            let probe_orchestrator = orchestrator.clone();
            let promote_alert_memory = memory.clone();
            let probe_settings = settings.clone();
            let probe_interval_minutes = health_probe_interval_minutes(&probe_settings);
            tokio::spawn(async move {
                // Wait 2 minutes after startup before first probe
                tokio::time::sleep(std::time::Duration::from_secs(120)).await;
                // Edge-detect degraded transitions so we alert on state CHANGES
                // (primary went down, or primary came back) rather than every tick.
                let mut last_health: Option<aegis_runtime_core::orchestrator::EmbedderHealth> =
                    None;
                // The interval is re-read on every iteration: the console's
                // cost-saving pause persists `health_probe_interval_minutes = 0`
                // and resume restores it. The task sleeps through a pause and
                // picks probing back up on resume — returning here used to leave
                // embedder health unmonitored until the next process restart.
                let mut paused = false;
                let mut next_due = tokio::time::Instant::now();
                loop {
                    // Wait for the cadence first, THEN read the setting, so a
                    // pause that lands during a long wait takes effect without
                    // one more probe firing on wake-up.
                    tokio::time::sleep_until(next_due).await;
                    let interval_minutes = health_probe_interval_minutes(&probe_settings);
                    match plan_health_probe_tick(interval_minutes, paused) {
                        HealthProbeTick::Paused { log_transition } => {
                            if log_transition {
                                tracing::info!(
                                    "[HealthProbe] paused (health_probe_interval_minutes=0); re-checking the setting every {HEALTH_PROBE_PAUSED_POLL_SECS}s"
                                );
                            }
                            paused = true;
                            tokio::time::sleep(std::time::Duration::from_secs(
                                HEALTH_PROBE_PAUSED_POLL_SECS,
                            ))
                            .await;
                            // Probe promptly once resumed instead of waiting a
                            // full interval first.
                            next_due = tokio::time::Instant::now();
                            continue;
                        }
                        HealthProbeTick::Run {
                            log_resumed,
                            interval,
                        } => {
                            if log_resumed {
                                tracing::info!(
                                    interval_minutes,
                                    "[HealthProbe] resumed after cost-saving pause"
                                );
                            }
                            paused = false;
                            next_due = tokio::time::Instant::now() + interval;
                        }
                    }
                    // Resolve destination per-tick through the notification
                    // router (always the owner's admin DM; group/forum routing
                    // was removed).
                    let (probe_system_chat_id, _, probe_system_topic) = routed_telegram_destination(
                        admin_chat_id,
                        NotificationClass::SystemStatus,
                        PrivacyLevel::SystemOnly,
                    );
                    let (probe_approval_chat_id, _, probe_approval_topic) =
                        routed_telegram_destination(
                            admin_chat_id,
                            NotificationClass::ApprovalRequest,
                            PrivacyLevel::SystemOnly,
                        );
                    let mut failures: Vec<String> = Vec::new();

                    // Check 1: Embedding API reachable.
                    //
                    // Note: `probe_embedder.embed(...)` TRANSPARENTLY fails over
                    // to backup when primary is down, so Ok(…) alone masks a
                    // primary outage. We check `is_degraded()` AFTER the call
                    // to surface the backup-serving state as a distinct alert.
                    match probe_embedder.embed("health check").await {
                        Ok(emb) if emb.len() > 10 => {}
                        Ok(emb) => failures.push(format!(
                            "Embedding API returned {} dims (expected >10)",
                            emb.len()
                        )),
                        Err(e) => failures.push(format!("Embedding API unreachable: {e}")),
                    }

                    // Owner-visible embedding health, keyed on the ACTUAL probe
                    // outcome (3-state), not the failover flag alone: with no
                    // backup configured `is_degraded()` may never flip on a
                    // primary failure, and once set it stays set even if the
                    // backup later fails — a boolean edge misses both the
                    // no-backup total outage AND the backup→unreachable
                    // escalation (Codex). Fire on any STATE change (no per-tick
                    // spam); build one alert, one send path.
                    let currently_degraded = probe_embedder.is_degraded();
                    let probe_ok = failures.is_empty();
                    let current_health = aegis_runtime_core::orchestrator::classify_embedder_health(
                        probe_ok,
                        currently_degraded,
                    );
                    if last_health != Some(current_health) {
                        use aegis_runtime_core::orchestrator::EmbedderHealth;
                        let primary = probe_embedder.primary_model();
                        // (message, is_recovery). None ⇒ no owner alert (e.g.
                        // the first-ever sample landing Healthy).
                        let alert: Option<(String, bool)> = match current_health {
                            EmbedderHealth::Healthy => {
                                if matches!(
                                    last_health,
                                    Some(
                                        EmbedderHealth::BackupServing | EmbedderHealth::Unreachable
                                    )
                                ) {
                                    probe_orchestrator.clear_embedder_degraded();
                                    Some((
                                        format!(
                                            "\u{2705} Primary embedding model recovered — `{primary}` is serving again; semantic dedup resumed / 主嵌入模型 `{primary}` 已恢复，语义去重已恢复"
                                        ),
                                        true,
                                    ))
                                } else {
                                    None
                                }
                            }
                            EmbedderHealth::BackupServing => {
                                let backup_opt = probe_embedder.backup_model();
                                let backup =
                                    backup_opt.clone().unwrap_or_else(|| "backup".to_string());
                                // Snapshot set BEFORE the best-effort send so the
                                // webapp projection is correct even if the push fails.
                                probe_orchestrator
                                    .mark_embedder_degraded(primary.clone(), backup_opt);
                                Some((
                                    format!(
                                        "\u{26a0}\u{fe0f} Primary embedding model DOWN — serving from backup `{backup}`; semantic dedup PAUSED (possible duplicate memory entries until recovery) / 主嵌入模型 `{primary}` 故障, 已切换备份 `{backup}`；语义去重已暂停，恢复前可能出现重复记忆条目"
                                    ),
                                    false,
                                ))
                            }
                            EmbedderHealth::Unreachable => {
                                probe_orchestrator.mark_embedder_degraded(primary.clone(), None);
                                Some((
                                    format!(
                                        "\u{1f6d1} Embedding UNREACHABLE — primary `{primary}` and any backup are down; semantic dedup + recall degraded until recovery / 嵌入服务完全不可达 — 主 `{primary}` 与备份均故障；语义去重与召回降级，恢复前受影响"
                                    ),
                                    false,
                                ))
                            }
                        };
                        if let Some((msg, is_recovery)) = alert {
                            if is_recovery {
                                tracing::info!("{msg}");
                            } else {
                                tracing::warn!("{msg}");
                            }
                            if !aegis_runtime_core::telegram::record_owner_web_notice_if_push_delivered(
                                &probe_memory,
                                admin_chat_id,
                                &msg,
                            )
                            .await
                            {
                                let html =
                                    aegis_runtime_core::telegram::markdown_to_telegram_html(&msg);
                                if let Err(e) = aegis_runtime_core::telegram::send_html_to_topic(
                                    &probe_bot,
                                    probe_system_chat_id,
                                    probe_system_topic,
                                    &html,
                                )
                                .await
                                {
                                    tracing::warn!(error = %e, "Health probe: failed to send embedder health alert");
                                }
                            }
                        }
                        last_health = Some(current_health);
                    }

                    // Embedding failures are now OWNED by the dedicated,
                    // transition-gated embedder-health signal above (which also
                    // sets the persistent webapp snapshot). Drain them from the
                    // generic failure list so the "Health Probe FAILED" alert
                    // below neither double-notifies on the outage tick nor spams
                    // every tick for the same embedding outage (Codex). Other
                    // failures (L2 memory, semantic search) still surface there.
                    failures.retain(|f| !f.starts_with("Embedding API"));

                    // 24h permanent-promotion alert drain. The embedding
                    // client queues a one-shot payload when the backup is
                    // promoted; the probe task picks it up on the next tick
                    // and does the two durable side-effects:
                    //   (1) persist the three `embedding_primary_promoted_*`
                    //       keys to user_settings so the next process start
                    //       honors the new primary (main.rs startup reads
                    //       these before constructing EmbeddingClient).
                    //   (2) send a bilingual Telegram alert so the operator
                    //       knows the permanent switch happened.
                    //
                    // Ordering: peek-before-take. We persist first, alert
                    // second, and ONLY clear the queued alert after BOTH
                    // succeeded. If persist fails (disk full, DB quota),
                    // the alert stays queued for the next tick to retry.
                    // If alert send fails but persist succeeded, we still
                    // clear — the DB carries the state for restart-time
                    // recovery. Fix for CTO round-1 silent-data-loss finding.
                    if let Some(alert) = probe_embedder.peek_pending_promotion_alert() {
                        tracing::warn!(
                            from = %alert.from,
                            to = %alert.to,
                            at = %alert.at_iso,
                            "[Embedding] Persisting permanent promotion and emitting alert"
                        );

                        // Step 1: persist all 3 DB keys. If ANY fails, leave
                        // the alert queued so next tick retries.
                        let mut persist_ok = true;
                        for (key, val) in &[
                            ("embedding_primary_promoted_from", alert.from.as_str()),
                            ("embedding_primary_promoted_to", alert.to.as_str()),
                            ("embedding_primary_promoted_at", alert.at_iso.as_str()),
                        ] {
                            if let Err(e) = promote_alert_memory
                                .store_setting(key, serde_json::Value::String(val.to_string()))
                                .await
                            {
                                tracing::error!(
                                    key = key,
                                    error = %e,
                                    "[Embedding] Failed to persist promotion setting \u{2014} alert stays queued, next health-probe tick will retry"
                                );
                                persist_ok = false;
                            }
                        }

                        if !persist_ok {
                            // Skip alert + clear. Retry on next tick.
                        } else {
                            let promote_msg = format!(
                                "\u{1f198} Primary embedding model `{from}` has been DOWN for 24h+. \
                                 Promoted backup `{to}` to permanent primary. \
                                 Run `unset_embedding_primary_promotion` to revert (hot-reload, no restart needed). \
                                 / 主 embedding 模型 `{from}` 已故障 24h+，\
                                 已将备份 `{to}` 提升为永久主模型。\
                                 运行 `unset_embedding_primary_promotion` 恢复（热重载，无需重启）。",
                                from = alert.from,
                                to = alert.to,
                            );
                            let html = aegis_runtime_core::telegram::markdown_to_telegram_html(
                                &promote_msg,
                            );
                            if aegis_runtime_core::telegram::record_owner_web_notice_if_push_delivered(
                                &promote_alert_memory,
                                admin_chat_id,
                                &promote_msg,
                            )
                            .await
                            {
                                probe_embedder.clear_pending_promotion_alert();
                            } else if let Err(e) = aegis_runtime_core::telegram::send_html_to_topic(
                                &probe_bot,
                                probe_system_chat_id,
                                probe_system_topic,
                                &html,
                            )
                            .await
                            {
                                tracing::warn!(error = %e, "Health probe: failed to send promotion alert (DB persist already succeeded)");
                            }

                            // Step 3: clear only after persist+alert attempt.
                            // (Alert send failure is non-critical — DB is the
                            // source of truth for restart-time recovery.)
                            probe_embedder.clear_pending_promotion_alert();

                            // Follow-up: the promotion just changed the live
                            // primary. If the corpus marker now points to the
                            // abandoned old-primary model, surface the cross-
                            // model alert so the user can decide whether to
                            // reembed. Best-effort — any failure is logged
                            // only, never blocks the probe loop.
                            match aegis_runtime_core::embedding_corpus::detect_corpus_mismatch(
                                &promote_alert_memory,
                                &probe_embedder,
                            )
                            .await
                            {
                                Ok(chk)
                                    if matches!(
                                        chk.status,
                                        aegis_runtime_core::embedding_corpus::CorpusStatus::Mismatch
                                    ) =>
                                {
                                    let follow_md =
                                        aegis_runtime_core::embedding_corpus::build_mismatch_alert_html(
                                            &chk,
                                        );
                                    let follow_html =
                                        aegis_runtime_core::telegram::markdown_to_telegram_html(
                                            &follow_md,
                                        );
                                    // Consent alert: asks user to approve a corpus
                                    // re-embed against the newly-promoted model.
                                    let follow_web_msg = format!(
                                        "{follow_md}\n\nWeb action: ask Aegis to run `trigger_corpus_reembed` if you approve re-embedding."
                                    );
                                    if !aegis_runtime_core::telegram::record_owner_web_notice_if_push_delivered(
                                        &promote_alert_memory,
                                        admin_chat_id,
                                        &follow_web_msg,
                                    )
                                    .await
                                    {
                                        if let Err(e) =
                                            aegis_runtime_core::telegram::send_html_to_topic(
                                                &probe_bot,
                                                probe_approval_chat_id,
                                                probe_approval_topic,
                                                &follow_html,
                                            )
                                            .await
                                        {
                                            tracing::warn!(
                                                error = %e,
                                                "[EmbeddingCorpus] failed to send post-promotion consent alert"
                                            );
                                        } else {
                                            tracing::info!(
                                                "[EmbeddingCorpus] post-promotion consent alert sent"
                                            );
                                        }
                                    }
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    tracing::warn!(
                                        error = %format!("{e:#}"),
                                        "[EmbeddingCorpus] post-promotion consistency check failed"
                                    );
                                }
                            }
                        }
                    }

                    // Check 2: L2 has structured facts
                    match probe_memory.get_all_l2(&probe_did).await {
                        Ok(facts) => {
                            // Note: get_all_l2 uses the system DID, user facts are under telegram DID
                            // Just verify the memory subsystem is responsive
                            tracing::debug!(
                                "[HealthProbe] L2 responsive, {} facts for system DID",
                                facts.len()
                            );
                        }
                        Err(e) => failures.push(format!("L2 memory query failed: {e}")),
                    }

                    // Check 3: Semantic search functional
                    if let Ok(emb) = probe_embedder.embed("test query").await {
                        // Query with a known user DID — we use a generic query to verify
                        // the semantic search pipeline doesn't panic
                        if let Err(e) = probe_memory.query_l2_semantic(&probe_did, &emb).await {
                            failures.push(format!("Semantic search failed: {e}"));
                        }
                    }

                    if !failures.is_empty() {
                        let alert = format!(
                            "\u{1f6a8} Health Probe FAILED ({} issue{}):\n{}",
                            failures.len(),
                            if failures.len() > 1 { "s" } else { "" },
                            failures
                                .iter()
                                .map(|f| format!("• {f}"))
                                .collect::<Vec<_>>()
                                .join("\n")
                        );
                        tracing::error!("{alert}");
                        if aegis_runtime_core::telegram::record_owner_web_notice_if_push_delivered(
                            &probe_memory,
                            admin_chat_id,
                            &alert,
                        )
                        .await
                        {
                            continue;
                        }
                        let html = aegis_runtime_core::telegram::markdown_to_telegram_html(&alert);
                        if let Err(e) = aegis_runtime_core::telegram::send_html_to_topic(
                            &probe_bot,
                            probe_system_chat_id,
                            probe_system_topic,
                            &html,
                        )
                        .await
                        {
                            tracing::warn!(error = %e, "Health probe: failed to send alert to Telegram");
                        }
                    } else {
                        tracing::info!("[HealthProbe] All checks passed");
                    }
                }
            });
            if probe_interval_minutes == 0 {
                eprintln!(
                    "[HealthProbe] Runtime health probe paused (health_probe_interval_minutes=0); it resumes by itself when the setting is restored"
                );
            } else {
                eprintln!(
                    "[HealthProbe] Runtime health probe started ({probe_interval_minutes}min interval, first check in 2min)"
                );
            }
        }

        // 10. Start circuit breaker notification listener
        //
        // Consumes CircuitBreakerEvent from the memory actor and sends Telegram
        // notifications with inline action buttons (Fix / Deactivate / Force Retry).
        if let Some(mut cb_rx) = memory.take_circuit_breaker_rx() {
            let cb_bot = tg_bot.bot();
            let cb_memory = memory.clone();
            tokio::spawn(async move {
                use teloxide::payloads::SendMessageSetters;
                use teloxide::requests::Requester;
                use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

                while let Some(event) = cb_rx.recv().await {
                    let (cid, thread, _) = routed_telegram_destination(
                        admin_chat_id,
                        NotificationClass::SystemStatus,
                        PrivacyLevel::SystemOnly,
                    );
                    let text = format!(
                        "\u{26a0}\u{fe0f} Circuit breaker OPEN for skill '{}'\n\
                         {} consecutive failures.\n\
                         Last error: {}\n\n\
                         The skill is paused — choose an action:",
                        event.skill_name,
                        event.consecutive_failures,
                        event.last_error.as_deref().unwrap_or("unknown"),
                    );
                    eprintln!("[CircuitBreaker] {}", text);
                    let web_text = format!(
                        "{text}\n\nWeb actions: ask Aegis to deactivate or retry skill `{}`.",
                        event.skill_name
                    );
                    if aegis_runtime_core::telegram::record_owner_web_notice_if_push_delivered(
                        &cb_memory,
                        admin_chat_id,
                        &web_text,
                    )
                    .await
                    {
                        continue;
                    }
                    let html = aegis_runtime_core::telegram::markdown_to_telegram_html(&text);
                    let keyboard = InlineKeyboardMarkup::new(vec![vec![
                        InlineKeyboardButton::callback(
                            "\u{23f8} Deactivate",
                            format!("cb_deactivate:{}", event.skill_name),
                        ),
                        InlineKeyboardButton::callback(
                            "\u{25b6}\u{fe0f} Force Retry",
                            format!("cb_retry:{}", event.skill_name),
                        ),
                    ]]);
                    let mut req = cb_bot
                        .send_message(cid, &html)
                        .parse_mode(teloxide::types::ParseMode::Html)
                        .reply_markup(keyboard);
                    if let Some(tid) = thread {
                        req = req.message_thread_id(tid);
                    }
                    let result = req.await;
                    if let Err(e) = result {
                        tracing::warn!(error = %e, "Failed to send circuit breaker notification");
                    }
                }
            });
            eprintln!("[CircuitBreaker] Notification listener started");
        }

        // 10b. Start skill health sentinel (notifies user about unhealthy skills)
        //
        // Monitor only — does NOT auto-fix. Alerts the user once per skill per
        // process lifetime. Skips skills with circuit_open to avoid duplicate alerts.
        {
            let sentinel_memory = memory.clone();
            let sentinel_bot = tg_bot.bot();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(600));
                interval.tick().await; // skip first immediate tick
                let mut alerted: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                loop {
                    interval.tick().await;
                    let metrics = match sentinel_memory.get_all_skill_metrics().await {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    for m in metrics {
                        // Skip skills with open circuit breaker (already notified)
                        if m.circuit_open {
                            continue;
                        }
                        if m.total_calls < 5 || m.error_count * 100 / m.total_calls.max(1) <= 50 {
                            continue;
                        }
                        if alerted.contains(&m.skill_name) {
                            continue;
                        }
                        alerted.insert(m.skill_name.clone());
                        let text = format!(
                            "\u{26a0}\u{fe0f} Skill '{}' is unhealthy: {}% error rate ({}/{} calls).\n\
                             Last error: {}\n\n\
                             You can ask me to deactivate it (deactivate_skill) or reload it (reactivate_skill).",
                            m.skill_name,
                            m.error_count * 100 / m.total_calls,
                            m.error_count,
                            m.total_calls,
                            m.last_error.as_deref().unwrap_or("unknown")
                        );
                        eprintln!("[HealthSentinel] {}", text);
                        if aegis_runtime_core::telegram::record_owner_web_notice_if_push_delivered(
                            &sentinel_memory,
                            admin_chat_id,
                            &text,
                        )
                        .await
                        {
                            continue;
                        }
                        let html = markdown_to_telegram_html(&text);
                        let (cid, _, topic) = routed_telegram_destination(
                            admin_chat_id,
                            NotificationClass::SystemStatus,
                            PrivacyLevel::SystemOnly,
                        );
                        if let Err(e) = send_html_to_topic(&sentinel_bot, cid, topic, &html).await {
                            tracing::warn!(error = %e, "Health sentinel: failed to send alert");
                        }
                    }
                }
            });
            eprintln!("[HealthSentinel] Background skill health monitor started (10-min interval)");
        }

        // 10c-ii. Self-Audit Engine (zero-LLM-cost continuous integrity monitor)
        //
        // Three tiers of deterministic checks:
        //   Invariant checks (startup + every 6h) — config consistency
        //   Rate monitors (every 10 min)          — metric anomalies
        //   Trend detectors (every 6h)            — slow degradation
        // Plus daily digest and a lightweight watchdog.
        {
            let audit_memory = memory.clone();
            let audit_bot = tg_bot.bot();
            let audit_settings = settings.clone();
            let audit_skills = sandbox.loaded_skills().to_vec();
            let audit_chat_id = admin_chat_id;
            tokio::spawn(async move {
                aegis_runtime_core::self_audit::run_audit_engine(
                    audit_memory,
                    audit_bot,
                    audit_settings,
                    audit_skills,
                    audit_chat_id,
                )
                .await;
            });

            // Watchdog: monitors audit engine liveness (separate task so a
            // panic in the main engine doesn't silence the watchdog).
            let watchdog_memory = memory.clone();
            let watchdog_bot = tg_bot.bot();
            tokio::spawn(async move {
                aegis_runtime_core::self_audit::run_audit_watchdog(
                    watchdog_memory,
                    watchdog_bot,
                    admin_chat_id,
                )
                .await;
            });

            eprintln!(
                "[SelfAudit] Integrity monitor started (10-min rate checks, 6h invariant/trend, daily digest)"
            );

            // Budget Monitor (checks LLM spend against budget thresholds every 15 min)
            aegis_runtime_core::llm_pricing::spawn_budget_monitor(
                memory.clone(),
                settings.clone(),
                tg_bot.bot(),
                admin_chat_id,
            );

            let pricing_http = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            aegis_runtime_core::llm_pricing::spawn_pricing_refresh_task(
                memory.clone(),
                pricing_http,
                api_base.clone(),
                db_path.clone(),
            );
        }

        // 10c-iii. Periodic health analyzer (1h interval, skill error rate monitoring)
        //
        // Scans skill_metrics and sends Telegram recommendations for degraded
        // skills (error_rate > 50%). Recommend only — never auto-fixes.
        {
            let (health_tx, mut health_rx) = tokio::sync::mpsc::channel::<String>(32);
            aegis_runtime_core::health_analyzer::spawn_health_analyzer(memory.clone(), health_tx);
            let health_bot = tg_bot.bot();
            let health_memory = memory.clone();
            tokio::spawn(async move {
                while let Some(msg) = health_rx.recv().await {
                    if aegis_runtime_core::telegram::record_owner_web_notice_if_push_delivered(
                        &health_memory,
                        admin_chat_id,
                        &msg,
                    )
                    .await
                    {
                        continue;
                    }
                    let html = markdown_to_telegram_html(&msg);
                    let (cid, _, topic) = routed_telegram_destination(
                        admin_chat_id,
                        NotificationClass::SystemStatus,
                        PrivacyLevel::SystemOnly,
                    );
                    if let Err(e) = send_html_to_topic(&health_bot, cid, topic, &html).await {
                        tracing::warn!(error = %e, "Health analyzer: failed to send notification");
                    }
                }
            });
            eprintln!("[HealthAnalyzer] Periodic skill health analyzer started (1h interval)");
        }

        // 10d. Start periodic L1 cache flush (every 5 minutes)
        //
        // Prevents data loss on OOM kill, panic, or SIGKILL where the
        // graceful shutdown handler never runs. Complements the existing
        // shutdown-time FlushL1ToDb.
        {
            let flush_memory = memory.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
                interval.tick().await; // skip first immediate tick
                loop {
                    interval.tick().await;
                    match flush_memory.flush_l1().await {
                        Ok(n) if n > 0 => {
                            tracing::info!(agents = n, "Periodic L1 cache flush completed");
                        }
                        Ok(_) => {} // nothing to flush
                        Err(e) => {
                            tracing::warn!(error = %e, "Periodic L1 cache flush failed");
                        }
                    }
                }
            });
            eprintln!("[Memory] Periodic L1 flush started (5-min interval)");
        }

        // 10c. Background re-embedding migration
        //
        // When the embedding model or dimensions change, existing L2 facts have
        // stale embedding BLOBs. This task detects the mismatch (by comparing
        // BLOB size to expected_dims * 4) and re-embeds in the background.
        //
        // Shares `reembed_in_progress` with the user-initiated
        // `trigger_corpus_reembed` tool (WARN #4 — single-flight across the
        // two sibling paths) and writes the `embedding_corpus_model` marker
        // on a clean full pass (FAIL #2 — silences spurious consent alerts
        // after legitimate dim changes).
        {
            let reembed_db_path = db_path.clone();
            let reembed_embedder = embedder_for_reembed;
            let reembed_memory = memory.clone();
            let reembed_guard = orchestrator.reembed_in_progress_flag();
            let expected_blob_size = (embedding_dims * 4) as usize;
            tokio::spawn(async move {
                // Wait a bit for other startup tasks to settle
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                if let Err(e) = reembed_stale_facts(
                    &reembed_db_path,
                    &reembed_memory,
                    &reembed_embedder,
                    expected_blob_size,
                    reembed_guard,
                )
                .await
                {
                    tracing::warn!(error = %format!("{e:#}"), "Background re-embedding migration failed");
                }
            });
        }

        // 11. Start Telegram bot with graceful shutdown on SIGTERM/Ctrl+C
        eprintln!("[Telegram] Bot starting (long-polling)...");
        eprintln!();
        eprintln!("Aegis Runtime is ready. Send a message to your Telegram bot.");
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("registering SIGTERM handler")?;
        // AEGIS_TELEGRAM_POLLING=0: keep the bot object (Web Console init-data
        // auth needs the token and owner id) but never poll Telegram — used by
        // the isolated Hercules instance for browser evidence.
        let polling_enabled = std::env::var("AEGIS_TELEGRAM_POLLING")
            .map(|v| v.trim() != "0")
            .unwrap_or(true);
        tokio::select! {
            _ = async {
                if polling_enabled {
                    tg_bot.start(orchestrator).await;
                } else {
                    eprintln!("[Telegram] polling disabled (AEGIS_TELEGRAM_POLLING=0); Web Console only");
                    std::future::pending::<()>().await;
                }
            } => {}
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\n[Shutdown] Received SIGINT, shutting down gracefully...");
            }
            _ = sigterm.recv() => {
                eprintln!("\n[Shutdown] Received SIGTERM, shutting down gracefully...");
            }
        }

        // Graceful shutdown: signal IDLE listener, stop scheduler
        shutdown_background_tasks(&mut chronos_scheduler, &mcp_client).await;

        // Graceful shutdown: persist L1 + WAL checkpoint (10s timeout)
        graceful_shutdown(&memory).await;
    } else {
        eprintln!();
        eprintln!("[Warning] TELOXIDE_TOKEN or ADMIN_CHAT_ID not set.");
        eprintln!("Running in headless mode (no Telegram). Admin API only.");
        eprintln!("Set TELOXIDE_TOKEN and ADMIN_CHAT_ID in .env to enable Telegram.");
        eprintln!();
        eprintln!("Aegis Runtime is ready. Admin API at http://127.0.0.1:{axum_port}");

        // Periodic L1 cache flush (every 5 minutes) — headless mode
        {
            let flush_memory = memory.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
                interval.tick().await; // skip first immediate tick
                loop {
                    interval.tick().await;
                    match flush_memory.flush_l1().await {
                        Ok(n) if n > 0 => {
                            tracing::info!(agents = n, "Periodic L1 cache flush completed");
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!(error = %e, "Periodic L1 cache flush failed");
                        }
                    }
                }
            });
            eprintln!("[Memory] Periodic L1 flush started (5-min interval)");
        }

        // Keep the process alive for the admin API
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("registering SIGTERM handler")?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\n[Shutdown] Received SIGINT, shutting down gracefully...");
            }
            _ = sigterm.recv() => {
                eprintln!("\n[Shutdown] Received SIGTERM, shutting down gracefully...");
            }
        }

        // Headless mode has no scheduler, but may have IDLE
        shutdown_background_tasks(&mut None, &mcp_client).await;

        // Graceful shutdown: persist L1 + WAL checkpoint (10s timeout)
        graceful_shutdown(&memory).await;
    }

    Ok(0)
}

/// Background re-embedding migration for L2 facts.
///
/// Opens a separate SQLite connection (WAL mode allows concurrent reads).
/// Finds facts whose embedding BLOB size doesn't match the expected dimensions,
/// re-embeds them using the current embedder, and updates in place.
/// Rate-limited to avoid hammering the embedding API.
///
/// ## Corpus-marker discipline (CTO round-1 FAIL #2)
///
/// This sibling code path homogenizes the HNSW-indexed corpus whenever the
/// primary dimensions change (e.g. operator edits `.env` and restarts). On a
/// full-success pass (every targeted row re-embedded under a single snapshot
/// model, no rows skipped for degraded-mode, no embed errors) we write the
/// `embedding_corpus_model` marker to `current_primary`. Without this, the
/// user would approve `trigger_corpus_reembed` next turn and pay the full
/// re-embed cost for a corpus that's already consistent — perpetual nag.
///
/// We share the snapshot-abort invariant with
/// [`crate::embedding_corpus::full_corpus_reembed`]: if the embedder flips
/// primary/backup mid-pass, we bail out early AND do NOT update the marker.
/// The next startup's `detect_corpus_mismatch` re-fires the consent alert.
///
/// ## Single-flight (WARN #4)
///
/// `reembed_in_progress` is the same `Arc<AtomicBool>` that guards the
/// user-initiated `trigger_corpus_reembed` tool. Acquiring it here prevents
/// the startup path from racing the tool if a user happens to approve a
/// reembed within ~10s of startup (this task sleeps 10s before running; a
/// user-triggered reembed during that window would otherwise double-touch
/// the corpus). If the guard is already held, we log and skip — the in-
/// progress pass will leave the corpus consistent and write its own marker.
async fn reembed_stale_facts(
    db_path: &str,
    memory: &aegis_runtime_core::memory::MemoryHandle,
    embedder: &EmbeddingClient,
    expected_blob_size: usize,
    reembed_in_progress: Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    use std::sync::atomic::Ordering;

    // Acquire the single-flight guard. If a user-initiated reembed is already
    // running, skip — we'd only duplicate work.
    if reembed_in_progress
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        tracing::info!(
            "Re-embedding migration: skipped (another reembed is in progress — likely user-initiated trigger_corpus_reembed)"
        );
        return Ok(());
    }
    struct ReembedGuard<'a>(&'a Arc<std::sync::atomic::AtomicBool>);
    impl Drop for ReembedGuard<'_> {
        fn drop(&mut self) {
            self.0.store(false, std::sync::atomic::Ordering::Release);
        }
    }
    let _guard = ReembedGuard(&reembed_in_progress);

    // Phase 1: read stale facts on a blocking thread (rusqlite is !Send)
    let db = db_path.to_string();
    let ebs = expected_blob_size;
    let rows: Vec<(i64, String)> =
        tokio::task::spawn_blocking(move || -> Result<Vec<(i64, String)>> {
            let conn = aegis_runtime_core::encrypted_sqlite::open(&db)
                .context("reembed: opening SQLite")?;
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
            let mut stmt = conn.prepare(
                "SELECT id, fact_summary FROM l2_experience \
             WHERE embedding IS NULL OR LENGTH(embedding) != ?1",
            )?;
            let rows: Vec<(i64, String)> = stmt
                .query_map([ebs as i64], |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
        .await??;

    if rows.is_empty() {
        tracing::info!("Re-embedding migration: all L2 facts already match current dimensions");
        return Ok(());
    }

    tracing::info!(
        count = rows.len(),
        expected_blob_size,
        "Re-embedding migration: updating stale L2 embeddings in background"
    );

    // Phase 2: re-embed each fact (async HTTP calls)
    //
    // Degraded-mode policy (approach b): if the embedder falls back to the
    // backup model during this pass, DO NOT write backup-model vectors into
    // l2_experience — different models produce non-comparable vectors and
    // mixing them corrupts HNSW similarity scores. Rows stay NULL and will
    // be picked up on the next startup when the primary has recovered.
    //
    // Snapshot invariant (CTO round-1 FAIL #2, shared with
    // `full_corpus_reembed`): snapshot `active_model()` + `is_degraded()` at
    // the start of the loop. If either flips mid-pass, abort — any row we
    // could add from here would be cross-model relative to the early rows.
    let model_snapshot = embedder.active_model();
    let degraded_snapshot = embedder.is_degraded();
    let mut updates: Vec<(i64, Vec<u8>)> = Vec::new();
    let mut errors = 0usize;
    let mut skipped_degraded = 0usize;
    let mut aborted_identity_flip = false;
    for (id, fact) in &rows {
        // Pre-embed identity check.
        if embedder.active_model() != model_snapshot || embedder.is_degraded() != degraded_snapshot
        {
            tracing::warn!(
                snapshot = %model_snapshot,
                observed = %embedder.active_model(),
                "Re-embedding: embedder identity flipped mid-pass, aborting"
            );
            aborted_identity_flip = true;
            break;
        }
        match embedder.embed(fact).await {
            Ok(emb) => {
                if embedder.is_degraded() {
                    // Vector came from backup model — do NOT persist. This keeps
                    // the HNSW-indexed store model-pure.
                    skipped_degraded += 1;
                    tracing::debug!(
                        id,
                        "Re-embedding: skipping persist (embedder in degraded/backup mode)"
                    );
                } else {
                    updates.push((*id, EmbeddingClient::to_blob(&emb)));
                }
            }
            Err(e) => {
                tracing::warn!(id, error = %format!("{e:#}"), "Re-embedding: embed API failed");
                errors += 1;
            }
        }
        // Rate limit: ~10 req/sec to stay within API limits
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    if skipped_degraded > 0 {
        tracing::info!(
            skipped_degraded,
            "Re-embedding: some rows skipped due to degraded mode; will retry on next restart after primary recovers"
        );
    }

    // Phase 3: batch-write updates on a blocking thread
    let db2 = db_path.to_string();
    let success = updates.len();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = aegis_runtime_core::encrypted_sqlite::open(&db2)
            .context("reembed: opening SQLite for updates")?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        for (id, blob) in &updates {
            if let Err(e) = conn.execute(
                "UPDATE l2_experience SET embedding = ?1 WHERE id = ?2",
                rusqlite::params![blob, id],
            ) {
                tracing::warn!(id, error = %e, "Re-embedding: DB update failed");
            }
        }
        Ok(())
    })
    .await??;

    tracing::info!(
        success,
        errors,
        total = rows.len(),
        "Re-embedding migration complete"
    );

    // Marker invariant: only update the corpus marker if we ran a COMPLETE,
    // CLEAN pass under a SINGLE model snapshot. "Complete" = no abort, "clean"
    // = no degraded-mode skips and no embed errors. On partial passes the
    // corpus still has a NULL/stale tail, so advertising `model_snapshot` as
    // the canonical corpus model would be a lie.
    if !aborted_identity_flip && skipped_degraded == 0 && errors == 0 {
        if let Err(e) =
            aegis_runtime_core::embedding_corpus::write_marker(memory, &model_snapshot).await
        {
            tracing::warn!(
                error = %e,
                "Re-embedding: corpus marker write failed (corpus is consistent; marker may be stale and re-fire consent alert next restart)"
            );
        } else {
            tracing::info!(
                model = %model_snapshot,
                "Re-embedding: corpus marker updated after clean full pass"
            );
        }
    }

    Ok(())
}

/// Graceful shutdown: flush L1 cache to SQLite and WAL checkpoint.
/// Times out after 10 seconds to avoid hanging on exit.
async fn graceful_shutdown(memory: &MemoryHandle) {
    eprintln!("[Shutdown] Persisting L1 cache and checkpointing WAL...");
    match tokio::time::timeout(std::time::Duration::from_secs(10), memory.shutdown()).await {
        Ok(Ok(())) => eprintln!("[Shutdown] Memory persisted successfully."),
        Ok(Err(e)) => eprintln!("[Shutdown] Memory persistence error (data may be lost): {e}"),
        Err(_) => {
            eprintln!("[Shutdown] Memory persistence timed out after 10s (data may be lost).")
        }
    }
}

/// Shut down background tasks: IDLE listener, Chronos scheduler, MCP servers.
/// Each subsystem gets a 5-second timeout so a hung component doesn't block the
/// entire shutdown sequence.
async fn shutdown_background_tasks(
    scheduler: &mut Option<tokio_cron_scheduler::JobScheduler>,
    mcp: &Option<Arc<aegis_runtime_core::mcp_client::McpClientManager>>,
) {
    let timeout_dur = std::time::Duration::from_secs(5);

    // 1. Stop Chronos scheduler
    if let Some(sched) = scheduler {
        match tokio::time::timeout(timeout_dur, sched.shutdown()).await {
            Ok(Ok(())) => eprintln!("[Shutdown] Chronos scheduler stopped"),
            Ok(Err(e)) => eprintln!("[Shutdown] Chronos scheduler stop error: {e:?}"),
            Err(_) => eprintln!("[Shutdown] Chronos scheduler stop timed out (5s)"),
        }
    }

    // 3. Disconnect MCP servers
    if let Some(mgr) = mcp {
        match tokio::time::timeout(timeout_dur, mgr.shutdown()).await {
            Ok(()) => eprintln!("[Shutdown] MCP servers disconnected"),
            Err(_) => eprintln!("[Shutdown] MCP shutdown timed out (5s)"),
        }
    }
}

// ── Skill loading & config validation (extracted to main_skill_loader.rs) ────
#[path = "main_skill_loader.rs"]
mod skill_loader;
use skill_loader::*;

// ── Pre-boot owner-chat auth (`aegis_runtime_core setup-auth …`) ─────────────
#[path = "main_setup_auth.rs"]
mod setup_auth;
use setup_auth::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routed_telegram_destination_system_status_falls_back_to_admin_dm() {
        let (chat_id, thread_id, topic) = routed_telegram_destination(
            123_456_789,
            NotificationClass::SystemStatus,
            PrivacyLevel::SystemOnly,
        );

        assert_eq!(chat_id, teloxide::types::ChatId(123_456_789));
        assert_eq!(thread_id, None);
        assert_eq!(topic, TopicKind::System);
    }

    #[test]
    fn scheduled_brief_action_id_is_stable_per_kind_and_local_date() {
        assert_eq!(
            scheduled_brief_action_id(ScheduledBriefKind::DailyBrief, "2026-04-30"),
            "scheduled-brief:daily_brief:2026-04-30"
        );
        assert_eq!(
            scheduled_brief_action_id(ScheduledBriefKind::EveningReview, "2026-04-30"),
            "scheduled-brief:evening_review:2026-04-30"
        );
    }

    #[test]
    fn scheduled_brief_config_uses_user_approved_default_times() {
        let settings = RuntimeSettings::new();

        assert_eq!(
            scheduled_brief_config(&settings, ScheduledBriefKind::DailyBrief),
            ScheduledBriefConfig {
                enabled: true,
                local_time: "09:00".to_string(),
            }
        );
        assert_eq!(
            scheduled_brief_config(&settings, ScheduledBriefKind::EveningReview),
            ScheduledBriefConfig {
                enabled: true,
                local_time: "18:00".to_string(),
            }
        );
    }

    #[test]
    fn scheduled_brief_web_receipt_records_owner_timeline_delivery() {
        let receipt = scheduled_brief_web_receipt("did:aegis:telegram:123");
        let parsed: serde_json::Value =
            serde_json::from_str(&receipt).expect("receipt should be valid JSON");

        assert_eq!(parsed["type"].as_str(), Some("web_owner_notice_delivery"));
        assert_eq!(parsed["surface"].as_str(), Some("web_owner_timeline"));
        assert_eq!(parsed["owner_did"].as_str(), Some("did:aegis:telegram:123"));
    }

    #[test]
    fn load_skills_reads_manifest_from_cwd_relative_path() {
        // Binary tests run with CWD = package dir (core/). The function uses
        // CWD-relative paths, so we must chdir to workspace root to match
        // how the binary runs in dev (cargo run) and prod (systemd WorkingDirectory).
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("core/ should have a parent directory");
        std::env::set_current_dir(workspace_root).expect("failed to chdir to workspace root");

        // If manifest.json is found, web_reader gets allowed_domains: ["*"].
        // If manifest.json is NOT found (old env!() bug), it falls back to empty vec.
        let sandbox = SandboxEngine::new().expect("failed to create sandbox");
        load_skills_from_directory(&sandbox, Path::new("compiled_skills"), &Default::default())
            .expect("load_skills_from_directory failed");

        let skills = sandbox.loaded_skills();
        let web_reader = skills.iter().find(|s| s.skill_name == "web_reader");

        assert!(
            web_reader.is_some(),
            "web_reader skill should be loaded from compiled_skills/"
        );
        assert!(
            !web_reader.unwrap().allowed_domains.is_empty(),
            "web_reader should have allowed_domains from manifest.json (not empty default)"
        );
    }

    #[test]
    fn load_skills_ignores_retired_search_artifact_without_compiling_it() {
        let dir = tempfile::tempdir().unwrap();
        // Invalid bytes would fail loading if the retired artifact were touched.
        std::fs::write(dir.path().join("web_search.wasm"), b"retired artifact").unwrap();
        let sandbox = SandboxEngine::new().unwrap();
        load_skills_from_directory(&sandbox, dir.path(), &Default::default()).unwrap();
        assert!(sandbox.loaded_skills().is_empty());
    }

    #[test]
    fn load_skills_missing_directory_is_ok() {
        let sandbox = SandboxEngine::new().expect("failed to create sandbox");
        let result = load_skills_from_directory(
            &sandbox,
            Path::new("nonexistent_directory_xyz"),
            &Default::default(),
        );
        assert!(result.is_ok(), "missing directory should return Ok");
        assert!(
            sandbox.loaded_skills().is_empty(),
            "no skills should be loaded from missing directory"
        );
    }

    // ── TLD validation tests ─────────────────────────────────────────

    #[test]
    fn validate_domains_rejects_bare_tld() {
        let result = validate_allowed_domains("evil_skill", &["com".to_string()]);
        assert!(result.is_err(), "bare TLD 'com' must be rejected");
        assert!(result.unwrap_err().to_string().contains("too broad"));
    }

    #[test]
    fn validate_domains_rejects_dot_prefix() {
        let result = validate_allowed_domains("skill", &[".example.com".to_string()]);
        assert!(result.is_err(), "dot-prefixed domain must be rejected");
    }

    #[test]
    fn validate_domains_accepts_specific_domain() {
        let result = validate_allowed_domains("skill", &["api.example.com".to_string()]);
        assert!(result.is_ok(), "specific domain must be accepted");
    }

    #[test]
    fn validate_domains_accepts_wildcard() {
        let result = validate_allowed_domains("skill", &["*".to_string()]);
        assert!(result.is_ok(), "wildcard '*' must be accepted");
    }

    #[test]
    fn validate_domains_accepts_empty() {
        let result = validate_allowed_domains("skill", &[]);
        assert!(result.is_ok(), "empty domain list must be accepted");
    }

    #[test]
    fn validate_domains_case_insensitive_tld() {
        let result = validate_allowed_domains("skill", &["COM".to_string()]);
        assert!(result.is_err(), "TLD check must be case-insensitive");
    }

    // ── Auto-register built-in skills tests ──────────────────────────

    /// Helper: create a temp dir with fake .wasm files for testing auto_register.
    fn setup_fake_compiled_skills(names: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("create temp dir");
        for name in names {
            std::fs::write(dir.path().join(format!("{name}.wasm")), b"fake-wasm")
                .expect("write fake wasm");
        }
        dir
    }

    #[test]
    fn auto_register_creates_proposal_for_new_builtin_skill() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let memory = MemoryHandle::spawn_in_memory().unwrap();
            let dir = setup_fake_compiled_skills(&["test_skill_alpha"]);

            auto_register_builtin_skills(&memory, dir.path())
                .await
                .expect("auto_register should succeed");

            // Verify a proposal was created with active status
            let proposals = memory
                .list_skill_proposals(None, Some(STATUS_ACTIVE))
                .await
                .unwrap();
            let found = proposals
                .iter()
                .find(|p| p.skill_name == "test_skill_alpha");
            assert!(
                found.is_some(),
                "built-in skill should have an active proposal after auto-register"
            );
            assert_eq!(found.unwrap().requested_by, "built-in");
        });
    }

    #[test]
    fn auto_register_skips_already_deactivated_skill() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let memory = MemoryHandle::spawn_in_memory().unwrap();
            let dir = setup_fake_compiled_skills(&["test_skill_beta"]);

            // Pre-create a deactivated proposal
            let id = memory
                .create_skill_proposal("test_skill_beta", "user", "user deactivated this", None)
                .await
                .unwrap();
            memory
                .update_skill_proposal(
                    id,
                    SkillProposalUpdate {
                        status: Some(STATUS_DEACTIVATED.to_string()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();

            // Run auto-register — should NOT create a new record
            auto_register_builtin_skills(&memory, dir.path())
                .await
                .expect("auto_register should succeed");

            // Should still have only 1 proposal, and it should remain deactivated
            let all = memory.list_skill_proposals(None, None).await.unwrap();
            let matching: Vec<_> = all
                .iter()
                .filter(|p| p.skill_name == "test_skill_beta")
                .collect();
            assert_eq!(
                matching.len(),
                1,
                "should not duplicate proposal for existing deactivated skill"
            );
            assert_eq!(matching[0].status, STATUS_DEACTIVATED);
        });
    }

    #[test]
    fn auto_register_skips_already_active_skill() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let memory = MemoryHandle::spawn_in_memory().unwrap();
            let dir = setup_fake_compiled_skills(&["test_skill_gamma"]);

            // Pre-create an active proposal
            let id = memory
                .create_skill_proposal(
                    "test_skill_gamma",
                    "built-in",
                    "previously registered",
                    None,
                )
                .await
                .unwrap();
            memory
                .update_skill_proposal(
                    id,
                    SkillProposalUpdate {
                        status: Some(STATUS_ACTIVE.to_string()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();

            // Run auto-register — should NOT create a duplicate
            auto_register_builtin_skills(&memory, dir.path())
                .await
                .expect("auto_register should succeed");

            let all = memory.list_skill_proposals(None, None).await.unwrap();
            let matching: Vec<_> = all
                .iter()
                .filter(|p| p.skill_name == "test_skill_gamma")
                .collect();
            assert_eq!(
                matching.len(),
                1,
                "should not duplicate proposal for existing active skill"
            );
        });
    }

    #[test]
    fn health_probe_survives_pause_and_logs_each_transition_once() {
        use std::time::Duration;
        // Freshly read setting per iteration: running → paused → paused →
        // resumed (new interval) → running. Before the fix the task returned
        // on the first 0 and never came back.
        let mut paused = false;
        let mut plans = Vec::new();
        for interval_minutes in [60, 0, 0, 30, 30] {
            let plan = plan_health_probe_tick(interval_minutes, paused);
            paused = matches!(plan, HealthProbeTick::Paused { .. });
            plans.push(plan);
        }
        assert_eq!(
            plans,
            vec![
                HealthProbeTick::Run {
                    log_resumed: false,
                    interval: Duration::from_secs(3600),
                },
                HealthProbeTick::Paused {
                    log_transition: true
                },
                HealthProbeTick::Paused {
                    log_transition: false
                },
                HealthProbeTick::Run {
                    log_resumed: true,
                    interval: Duration::from_secs(1800),
                },
                HealthProbeTick::Run {
                    log_resumed: false,
                    interval: Duration::from_secs(1800),
                },
            ]
        );
    }

    #[test]
    fn health_probe_interval_is_reread_from_live_settings() {
        let settings = RuntimeSettings::new();
        assert_eq!(health_probe_interval_minutes(&settings), 60);
        // Cost-saving pause persists 0 …
        settings
            .set("health_probe_interval_minutes", serde_json::json!(0))
            .unwrap();
        assert_eq!(health_probe_interval_minutes(&settings), 0);
        // … and resume restores a value the loop must pick up without restart.
        settings
            .set("health_probe_interval_minutes", serde_json::json!(15))
            .unwrap();
        assert_eq!(health_probe_interval_minutes(&settings), 15);
        // Negative control: a paused value never plans a probe run, whatever
        // the previous state was.
        for was_paused in [false, true] {
            assert!(matches!(
                plan_health_probe_tick(0, was_paused),
                HealthProbeTick::Paused { .. }
            ));
        }
    }
}
