//! SQLite schema creation and migration logic for Aegis Runtime's memory subsystem.
//!
//! Extracted from `memory.rs` to keep the main module focused on runtime
//! behaviour (actor loop, handles, queries) while isolating all DDL and
//! data-migration statements here.

use anyhow::Context;
use rusqlite::Connection;

/// Skill names that are redundant with built-in pseudo-tools (schedule_intent,
/// manage_tasks) or overlap with other existing skills. These are deleted from
/// `skill_proposals` during DB init to prevent phantom registrations.
///
/// If `auto_register_builtin_skills()` runs AFTER the cleanup and finds a
/// matching `.wasm` file in `compiled_skills/`, it will re-create the proposal.
/// The fix is to ensure no `.wasm` file exists for these names.
pub const REDUNDANT_SKILLS: &[&str] = &[
    "enhanced_reminder",
    "reminder",
    "flexible_recurring_reminder",
    "smart_recurring_reminder_calc",
    "simple_reminder",
    "reminder_manager",
    "todo_list_manager",
    "unified_task_manager",
    "linux_system_monitor",
    "server_health_dashboard",
];

const INTERNAL_SKILL_STATE_NAMES_SQL: &str = "'system', 'hitl', 'monitoring_rules', \
    'owner_web_presence', 'daily_brief_scheduler', 'google_auth_audit', \
    'web_push_subscriptions', 'web_push_delivery_receipts', '_llm_pricing', '_self_audit', \
    'hercules_delegation', 'external_executor'";

const SERVER_HEALTH_AGGREGATOR_DAG_JSON: &str = r#"{"type":"static_dag","name":"server_health_aggregator","steps":[{"id":"check","capability":{"type":"wasm_skill","name":"server_monitor"},"input_mapping":[],"condition":null}],"final_output":"check"}"#;
const GENERATE_SUMMARY_PROBE_DAG_JSON: &str = r#"{"type":"static_dag","name":"generate_summary","steps":[{"id":"check","capability":{"type":"wasm_skill","name":"gmail_reader"},"input_mapping":[{"from":"input.action","to":"action"}],"condition":null}],"final_output":"check"}"#;
const GENERATE_SUMMARY_BAD_UTC_CRON_JSON: &str = r#""cron_schedule":"0 0 9 * * 2-5""#;
const GENERATE_SUMMARY_PDT_UTC_CRON_JSON: &str = r#""cron_schedule":"0 0 16 * * 2-5""#;

/// Repair a previously activated v2 composite that referenced generated atom
/// names which were never persisted as executable skills. `server_monitor`
/// already returns the ProbeSignal Chronos expects, so the safe runtime shape is
/// a one-step composite wrapper around that existing probe skill.
fn repair_server_health_aggregator_composite_dag(conn: &Connection) -> anyhow::Result<()> {
    let repaired = conn
        .execute(
            "UPDATE skill_proposals
             SET execution_dag_json = ?1,
                 stage_error = NULL,
                 updated_at = datetime('now')
             WHERE skill_name = 'server_health_aggregator'
               AND status IN ('active', 'pending_final_approval', 'activating')
               AND execution_dag_json IS NOT NULL
               AND (
                    execution_dag_json LIKE '%append_health_history%'
                 OR execution_dag_json LIKE '%aggregate_trend_analysis%'
                 OR execution_dag_json LIKE '%decide_alert_status%'
                 OR execution_dag_json LIKE '%format_telegram_dashboard%'
                 OR execution_dag_json LIKE '%notify_user%'
               )",
            [SERVER_HEALTH_AGGREGATOR_DAG_JSON],
        )
        .context("repairing server_health_aggregator composite DAG")?;
    if repaired > 0 {
        tracing::warn!(
            count = repaired,
            "Repaired server_health_aggregator composite DAG to existing server_monitor probe"
        );
    }
    Ok(())
}

/// Repair the activated Gmail summary composite into a cron-compatible probe.
/// The original DAG mixed Gmail reads, generated transforms, and a pseudo
/// notification step, then returned ordinary Gmail JSON. Chronos cron skills
/// must return ProbeSignal; `gmail_reader` already exposes that via check_new.
fn repair_generate_summary_composite_dag(conn: &Connection) -> anyhow::Result<()> {
    let repaired = conn
        .execute(
            "UPDATE skill_proposals
             SET execution_dag_json = ?1,
                 stage_error = NULL,
                 updated_at = datetime('now')
             WHERE skill_name = 'generate_summary'
               AND status IN ('active', 'pending_final_approval', 'activating')
               AND execution_dag_json IS NOT NULL
               AND (
                    execution_dag_json LIKE '%notify_summary%'
                 OR execution_dag_json LIKE '%categorize_emails%'
                 OR execution_dag_json LIKE '%fetch_unread_emails%'
                 OR execution_dag_json LIKE '%\"final_output\":\"notify_summary\"%'
               )",
            [GENERATE_SUMMARY_PROBE_DAG_JSON],
        )
        .context("repairing generate_summary composite DAG")?;
    if repaired > 0 {
        tracing::warn!(
            count = repaired,
            "Repaired generate_summary composite DAG to gmail_reader check_new probe"
        );
    }
    Ok(())
}

/// Repair the generated summary cron from "09:00 UTC" to "09:00 PDT".
///
/// SkillForge stores cron expressions in UTC because tokio-cron-scheduler has
/// no user-timezone context here. The proposal asked for Tue-Fri 9am PDT, but
/// the active graph persisted `0 0 9 * * 2-5`, which fires at 2am PDT during
/// daylight saving time. Preserve the DAG and only rewrite the known bad graph
/// schedule to 16:00 UTC.
fn repair_generate_summary_pdt_cron(conn: &Connection) -> anyhow::Result<()> {
    let repaired = conn
        .execute(
            "UPDATE skill_proposals
             SET capability_graph_json = replace(capability_graph_json, ?1, ?2),
                 stage_error = NULL,
                 updated_at = datetime('now')
             WHERE skill_name = 'generate_summary'
               AND status IN ('active', 'pending_final_approval', 'activating')
               AND capability_graph_json LIKE '%\"cron_schedule\":\"0 0 9 * * 2-5\"%'",
            [
                GENERATE_SUMMARY_BAD_UTC_CRON_JSON,
                GENERATE_SUMMARY_PDT_UTC_CRON_JSON,
            ],
        )
        .context("repairing generate_summary PDT cron schedule")?;
    if repaired > 0 {
        tracing::warn!(
            count = repaired,
            "Repaired generate_summary cron from 09:00 UTC to 09:00 PDT equivalent"
        );
    }
    Ok(())
}

/// Stats from the one-time tier migration, used for logging.
struct TierMigrationStats {
    did_unified: usize,
    total_to_ephemeral: usize,
    deleted_from_tier1: usize,
    profile_seeded: i64,
}

/// Execute the one-time memory tier redesign migration inside a SAVEPOINT.
///
/// If any step fails, the SAVEPOINT is rolled back and nothing changes.
/// On success, the migration flag is set so it won't run again.
/// The owner's DID for migrations that move or seed owner-scoped rows.
///
/// Was a compiled-in literal of the author's Telegram user id. It is
/// configuration: `OWNER_DID` if set, else derived from `ADMIN_CHAT_ID` (the id
/// Telegram already uses for the owner), else a neutral value that matches no
/// real row. On the original deployment ADMIN_CHAT_ID is set, so the derived
/// value is byte-identical to the old literal — asserted by a test below.
pub(crate) fn owner_did() -> String {
    owner_did_from(
        std::env::var("OWNER_DID").ok().as_deref(),
        std::env::var("ADMIN_CHAT_ID").ok().as_deref(),
    )
}

/// Pure core of [`owner_did`] (testable without touching process env).
pub(crate) fn owner_did_from(owner_did: Option<&str>, admin_chat_id: Option<&str>) -> String {
    if let Some(d) = owner_did.map(str::trim).filter(|d| !d.is_empty()) {
        return d.to_string();
    }
    match admin_chat_id
        .map(str::trim)
        .filter(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit()))
    {
        Some(id) => format!("did:aegis:telegram:{id}"),
        None => "did:aegis:owner".to_string(),
    }
}

fn run_tier_migration(conn: &Connection, owner_did: &str) -> anyhow::Result<TierMigrationStats> {
    // Use SAVEPOINT instead of BEGIN — compatible with `run_all_migrations` which
    // may or may not already be inside a transaction depending on the caller.
    conn.execute_batch("SAVEPOINT tier_migration")
        .context("starting tier migration savepoint")?;

    let result = run_tier_migration_inner(conn, owner_did);

    match &result {
        Ok(_) => {
            conn.execute_batch("RELEASE tier_migration")
                .context("committing tier migration savepoint")?;
        }
        Err(_) => {
            // Roll back all changes — source data in l2_experience is untouched.
            conn.execute_batch("ROLLBACK TO tier_migration")
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "Failed to rollback tier migration — DB may be inconsistent");
                });
        }
    }

    result
}

/// Inner migration logic. Every step uses `?` to propagate errors, causing
/// the caller to roll back the SAVEPOINT on any failure.
fn run_tier_migration_inner(
    conn: &Connection,
    owner_did: &str,
) -> anyhow::Result<TierMigrationStats> {
    // Step 4: DID unification — migrate did:aegis:system → canonical telegram DID
    // SkillForge and api_knowledge stay under did:aegis:system (system-scoped knowledge).
    // This runs FIRST so that subsequent steps copy rows with the correct DID.
    let did_unified = conn
        .execute(
            "UPDATE l2_experience
             SET agent_did = ?1
             WHERE agent_did = 'did:aegis:system'
               AND topic NOT LIKE 'skillforge:%'
               AND topic NOT LIKE 'learned:api_knowledge'",
            rusqlite::params![owner_did],
        )
        .context("DID unification")?;
    tracing::info!(count = did_unified, "DID unification complete");

    // Step 5: Migrate l1_overflow → Tier 2 (TTL: 30 days)
    let overflow_migrated = conn
        .execute(
            "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, source, confidence, created_at, expires_at)
             SELECT agent_did, topic, fact_summary, 'l1_overflow', confidence, created_at,
                    datetime(created_at, '+30 days')
             FROM l2_experience WHERE topic = 'l1_overflow'",
            [],
        )
        .context("l1_overflow → Tier 2")?;
    tracing::info!(
        count = overflow_migrated,
        "Migrated l1_overflow → l2_ephemeral"
    );

    // Step 6: Migrate research:* → Tier 2 (TTL: 90 days)
    let research_migrated = conn
        .execute(
            "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, embedding, source, confidence, created_at, expires_at)
             SELECT agent_did, topic, fact_summary, embedding, 'research', confidence, created_at,
                    datetime(created_at, '+90 days')
             FROM l2_experience WHERE topic LIKE 'research:%'",
            [],
        )
        .context("research → Tier 2")?;
    tracing::info!(
        count = research_migrated,
        "Migrated research:* → l2_ephemeral"
    );

    // Step 7: Migrate context_summary → Tier 2 (TTL: 60 days)
    let summary_migrated = conn
        .execute(
            "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, embedding, source, confidence, created_at, expires_at)
             SELECT agent_did, topic, fact_summary, embedding, 'context_summary', confidence, created_at,
                    datetime(created_at, '+60 days')
             FROM l2_experience WHERE topic = 'context_summary'",
            [],
        )
        .context("context_summary → Tier 2")?;
    tracing::info!(
        count = summary_migrated,
        "Migrated context_summary → l2_ephemeral"
    );

    // Step 8: Migrate skillforge:* → Tier 2 (TTL: 90 days)
    // Note: skillforge rows were excluded from DID unification in Step 4, so they
    // retain their original did:aegis:system DID — this is correct by design.
    let skillforge_migrated = conn
        .execute(
            "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, embedding, source, confidence, created_at, expires_at)
             SELECT agent_did, topic, fact_summary, embedding, 'skillforge', confidence, created_at,
                    datetime(created_at, '+90 days')
             FROM l2_experience WHERE topic LIKE 'skillforge:%'",
            [],
        )
        .context("skillforge → Tier 2")?;
    tracing::info!(
        count = skillforge_migrated,
        "Migrated skillforge:* → l2_ephemeral"
    );

    // Step 9: Migrate free-form compaction topics → Tier 2 (TTL: 60 days)
    // These are ~271 LLM-generated topics from StoreSummary compaction (e.g. "MSFT Stock Monitoring",
    // "Barber Preferences"). They are compaction artifacts, not primary learned facts.
    let freeform_migrated = conn
        .execute(
            "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, embedding, source, confidence, created_at, expires_at)
             SELECT agent_did, topic, fact_summary, embedding, 'compaction_summary', confidence, created_at,
                    datetime(created_at, '+60 days')
             FROM l2_experience
             WHERE topic NOT LIKE 'learned:%'
               AND topic NOT LIKE 'heartbeat_memo%'
               AND topic NOT LIKE 'monitoring_intent:%'
               AND topic NOT IN ('user_fact','identity','preference','contact','config','server')
               AND topic != 'l1_overflow'
               AND topic NOT LIKE 'research:%'
               AND topic != 'context_summary'
               AND topic NOT LIKE 'skillforge:%'",
            [],
        )
        .context("free-form topics → Tier 2")?;
    tracing::info!(
        count = freeform_migrated,
        "Migrated free-form compaction topics → l2_ephemeral"
    );

    // Step 10: Delete all migrated rows from l2_experience
    // After this, only learned:*, heartbeat_memo*, monitoring_intent:*, and
    // extract_immediate_fact topics (user_fact, identity, preference, contact, config, server) remain.
    let deleted = conn
        .execute(
            "DELETE FROM l2_experience
             WHERE topic = 'l1_overflow'
                OR topic LIKE 'research:%'
                OR topic = 'context_summary'
                OR topic LIKE 'skillforge:%'
                OR (topic NOT LIKE 'learned:%'
                    AND topic NOT LIKE 'heartbeat_memo%'
                    AND topic NOT LIKE 'monitoring_intent:%'
                    AND topic NOT IN ('user_fact','identity','preference','contact','config','server'))",
            [],
        )
        .context("Tier 1 cleanup DELETE")?;
    tracing::info!(
        count = deleted,
        "Cleaned l2_experience: removed migrated rows"
    );

    // Step 11: Seed user_profile from existing learned:context facts
    // Deterministic pattern matching — no LLM needed.
    // Profile seeding failures are non-fatal: gaps are filled on demand.
    let canonical_did = owner_did;

    // Home address
    conn.execute(
        "INSERT OR IGNORE INTO user_profile (agent_did, category, key, value, confidence, source)
         SELECT agent_did, 'location', 'home_address', fact_summary, confidence, 'migration'
         FROM l2_experience
         WHERE topic = 'learned:context'
           AND fact_summary LIKE '%lives at%'
           AND agent_did = ?1
           ORDER BY confidence DESC, created_at DESC LIMIT 1",
        rusqlite::params![canonical_did],
    )
    .context("Profile seed: home_address")?;

    // Work address
    conn.execute(
        "INSERT OR IGNORE INTO user_profile (agent_did, category, key, value, confidence, source)
         SELECT agent_did, 'location', 'work_address', fact_summary, confidence, 'migration'
         FROM l2_experience
         WHERE topic = 'learned:context'
           AND (fact_summary LIKE '%works at%' OR fact_summary LIKE '%office%')
           AND agent_did = ?1
           ORDER BY confidence DESC, created_at DESC LIMIT 1",
        rusqlite::params![canonical_did],
    )
    .context("Profile seed: work_address")?;

    // Name and identity
    conn.execute(
        "INSERT OR IGNORE INTO user_profile (agent_did, category, key, value, confidence, source)
         SELECT agent_did, 'identity', 'name_and_role', fact_summary, confidence, 'migration'
         FROM l2_experience
         WHERE topic = 'learned:context'
           AND (fact_summary LIKE '%name is%' OR fact_summary LIKE '%works as%')
           AND agent_did = ?1
           ORDER BY confidence DESC, created_at DESC LIMIT 1",
        rusqlite::params![canonical_did],
    )
    .context("Profile seed: name_and_role")?;

    // Hair preference
    conn.execute(
        "INSERT OR IGNORE INTO user_profile (agent_did, category, key, value, confidence, source)
         SELECT agent_did, 'preference', 'hair', fact_summary, confidence, 'migration'
         FROM l2_experience
         WHERE topic = 'learned:context'
           AND fact_summary LIKE '%hair%'
           AND fact_summary LIKE '%fade%'
           AND agent_did = ?1
         ORDER BY confidence DESC, created_at DESC LIMIT 1",
        rusqlite::params![canonical_did],
    )
    .context("Profile seed: hair")?;

    let profile_seeded: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM user_profile WHERE agent_did = ?1",
            rusqlite::params![canonical_did],
            |r| r.get(0),
        )
        .unwrap_or(0);
    tracing::info!(
        count = profile_seeded,
        "Seeded user_profile from learned:context facts"
    );

    // Step 13: Mark HNSW for rebuild — slimmed Tier 1 needs a fresh index.
    // The HNSW builder checks this flag on startup and rebuilds if set.
    conn.execute(
        "INSERT OR REPLACE INTO skill_state (skill_name, state_key, state_value, updated_at)
         VALUES ('system', 'hnsw_rebuild_needed', 'tier_migration', datetime('now'))",
        [],
    )
    .context("setting HNSW rebuild flag")?;

    // Set migration flag — only reached if ALL steps above succeeded.
    conn.execute(
        "INSERT OR REPLACE INTO skill_state (skill_name, state_key, state_value, updated_at)
         VALUES ('system', 'tier_migration_done', 'v1', datetime('now'))",
        [],
    )
    .context("setting tier migration flag")?;

    let total_to_ephemeral = overflow_migrated
        + research_migrated
        + summary_migrated
        + skillforge_migrated
        + freeform_migrated;

    Ok(TierMigrationStats {
        did_unified,
        total_to_ephemeral,
        deleted_from_tier1: deleted,
        profile_seeded,
    })
}

/// Run every schema-creation statement and data migration against `conn`.
///
/// Called once during `MemoryHandle::spawn_with_config()` after PRAGMAs are set.
/// Run every migration with the owner DID resolved from configuration.
pub(crate) fn run_all_migrations(conn: &Connection) -> anyhow::Result<()> {
    run_all_migrations_with_owner(conn, &owner_did())
}

/// Same, with an explicit owner DID (tests, and callers that already know it).
pub(crate) fn run_all_migrations_with_owner(
    conn: &Connection,
    owner_did: &str,
) -> anyhow::Result<()> {
    // Create L2 table (new schema includes embedding column)
    conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS l2_experience (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_did TEXT NOT NULL,
                topic TEXT NOT NULL,
                fact_summary TEXT NOT NULL,
                created_at TEXT NOT NULL,
                embedding BLOB
            );
            CREATE INDEX IF NOT EXISTS idx_l2_did ON l2_experience(agent_did);
            CREATE INDEX IF NOT EXISTS idx_l2_topic ON l2_experience(topic);

            CREATE TABLE IF NOT EXISTS reminders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id INTEGER NOT NULL,
                message TEXT NOT NULL,
                remind_at_unix INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                fired INTEGER DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_reminders_due ON reminders(fired, remind_at_unix);
            CREATE INDEX IF NOT EXISTS idx_reminders_chat ON reminders(chat_id, fired);

            CREATE TABLE IF NOT EXISTS l1_snapshot (
                agent_did TEXT NOT NULL,
                messages_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_l1_snap_did ON l1_snapshot(agent_did);

            -- L3: Full-text archive (every message, searchable via FTS5)
            CREATE TABLE IF NOT EXISTS l3_archive (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_did TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_l3_did ON l3_archive(agent_did);

            CREATE TABLE IF NOT EXISTS email_drafts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                recipient TEXT NOT NULL,
                subject TEXT NOT NULL,
                body TEXT NOT NULL,
                cc TEXT,
                reply_to_id TEXT,
                created_at TEXT NOT NULL,
                sent INTEGER DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_drafts_created ON email_drafts(created_at);

            CREATE TABLE IF NOT EXISTS user_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS skill_proposals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                skill_name TEXT NOT NULL,
                requested_by TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending_approval',
                stage_error TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                user_request TEXT NOT NULL,
                proposal_summary TEXT,
                design_json TEXT,
                generated_source TEXT,
                generated_manifest TEXT,
                generated_cargo_toml TEXT,
                compiled_wasm BLOB,
                audit_static_result TEXT,
                audit_llm_result TEXT,
                audit_overridden INTEGER DEFAULT 0,
                audit_override_by TEXT,
                audit_override_at TEXT,
                audit_override_reason TEXT,
                test_result TEXT,
                requires_auth INTEGER DEFAULT 0,
                auth_domain TEXT,
                total_llm_calls INTEGER DEFAULT 0,
                version INTEGER NOT NULL DEFAULT 1,
                skill_md TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_proposals_status ON skill_proposals(status);
            CREATE INDEX IF NOT EXISTS idx_proposals_requested_by ON skill_proposals(requested_by);
            CREATE INDEX IF NOT EXISTS idx_proposals_skill_name ON skill_proposals(skill_name);
            CREATE INDEX IF NOT EXISTS idx_proposals_skill_status_version ON skill_proposals(skill_name, status, version DESC);

            -- Skill execution metrics for usage-driven quality feedback
            CREATE TABLE IF NOT EXISTS skill_metrics (
                skill_name TEXT PRIMARY KEY,
                total_calls INTEGER NOT NULL DEFAULT 0,
                success_count INTEGER NOT NULL DEFAULT 0,
                error_count INTEGER NOT NULL DEFAULT 0,
                avg_latency_ms REAL NOT NULL DEFAULT 0.0,
                last_error TEXT,
                error_categories TEXT NOT NULL DEFAULT '{}',
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Golden tests for automated regression testing
            CREATE TABLE IF NOT EXISTS skill_golden_tests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                skill_name TEXT NOT NULL,
                version INTEGER NOT NULL,
                test_input TEXT NOT NULL,
                expected_output_schema TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_golden_tests_skill ON skill_golden_tests(skill_name);

            CREATE TABLE IF NOT EXISTS deferred_intents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_did TEXT NOT NULL,
                chat_id INTEGER NOT NULL,
                trigger_type TEXT NOT NULL,
                trigger_at_unix INTEGER,
                event_key TEXT,
                execution_mode TEXT NOT NULL,
                context_text TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                expires_at_unix INTEGER NOT NULL,
                fired_at TEXT,
                is_callback_origin INTEGER DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_intents_time ON deferred_intents(status, trigger_at_unix);
            CREATE INDEX IF NOT EXISTS idx_intents_event ON deferred_intents(status, event_key);
            CREATE INDEX IF NOT EXISTS idx_intents_user ON deferred_intents(user_did);

            -- Unified task system (superset of KB action_items + deferred_intents + reminders + personal_tasks)
            CREATE TABLE IF NOT EXISTS tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id INTEGER NOT NULL,
                user_did TEXT NOT NULL,
                title TEXT NOT NULL,
                body TEXT NOT NULL DEFAULT '',
                priority TEXT NOT NULL DEFAULT 'normal',
                status TEXT NOT NULL DEFAULT 'active',
                tags TEXT NOT NULL DEFAULT '[]',
                due_date TEXT,
                source TEXT NOT NULL DEFAULT 'user',
                trigger_at_unix INTEGER,
                trigger_type TEXT,
                event_key TEXT,
                execution_mode TEXT DEFAULT 'notify',
                context_text TEXT,
                interval_seconds INTEGER,
                group_key TEXT,
                max_fires INTEGER,
                fire_count INTEGER NOT NULL DEFAULT 0,
                recur_weekdays TEXT,
                recur_window_start TEXT,
                recur_window_end TEXT,
                recur_timezone TEXT,
                fired_at TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                chain_depth INTEGER NOT NULL DEFAULT 0,
                pin_on_fire INTEGER NOT NULL DEFAULT 0,
                expires_at_unix INTEGER,
                is_callback_origin INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT,
                archived_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_tasks_chat_status ON tasks(chat_id, status);
            CREATE INDEX IF NOT EXISTS idx_tasks_trigger ON tasks(status, trigger_at_unix) WHERE trigger_at_unix IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_tasks_event ON tasks(status, event_key) WHERE event_key IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_tasks_user ON tasks(user_did, status);
            CREATE INDEX IF NOT EXISTS idx_tasks_group ON tasks(user_did, group_key) WHERE group_key IS NOT NULL;

            CREATE TABLE IF NOT EXISTS skill_state (
                skill_name TEXT NOT NULL,
                state_key TEXT NOT NULL,
                state_value TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (skill_name, state_key)
            );

            -- Daily skill metrics snapshots (for trend analysis)
            CREATE TABLE IF NOT EXISTS skill_metrics_daily (
                skill_name TEXT NOT NULL,
                date TEXT NOT NULL,
                total_calls INTEGER NOT NULL DEFAULT 0,
                error_count INTEGER NOT NULL DEFAULT 0,
                avg_latency_ms REAL NOT NULL DEFAULT 0.0,
                PRIMARY KEY (skill_name, date)
            );
            CREATE INDEX IF NOT EXISTS idx_metrics_daily_date ON skill_metrics_daily(date);

            -- Initiative Engine proposals
            CREATE TABLE IF NOT EXISTS initiative_proposals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id INTEGER NOT NULL,
                initiative_type TEXT NOT NULL,
                source TEXT NOT NULL,
                priority TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT NOT NULL,
                evidence TEXT NOT NULL DEFAULT '[]',
                proposed_action TEXT NOT NULL DEFAULT '{}',
                estimated_effort TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'pending',
                suppressed INTEGER NOT NULL DEFAULT 0,
                topic_key TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                user_response TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_init_chat_status ON initiative_proposals(chat_id, status);
            CREATE INDEX IF NOT EXISTS idx_init_chat_created ON initiative_proposals(chat_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_init_topic ON initiative_proposals(chat_id, topic_key);

            -- Owner-triggered WebApp debug snapshots. These are short-lived
            -- diagnostics for production UX validation, not permanent memory.
            CREATE TABLE IF NOT EXISTS webapp_debug_snapshots (
                id TEXT PRIMARY KEY,
                owner TEXT NOT NULL,
                principal_type TEXT NOT NULL,
                surface TEXT NOT NULL,
                path TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                payload_bytes INTEGER NOT NULL,
                created_at_unix INTEGER NOT NULL,
                expires_at_unix INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_webapp_debug_snapshots_expires ON webapp_debug_snapshots(expires_at_unix);
            CREATE INDEX IF NOT EXISTS idx_webapp_debug_snapshots_owner_created ON webapp_debug_snapshots(owner, created_at_unix DESC);

            CREATE TABLE IF NOT EXISTS webapp_turn_evidence (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_did TEXT NOT NULL,
                reply_key TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                evidence_json TEXT NOT NULL,
                unsupported_json TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_webapp_turn_evidence_user_key
                ON webapp_turn_evidence(user_did, reply_key);

            -- Research Engine tasks (proactive knowledge acquisition)
            CREATE TABLE IF NOT EXISTS research_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id INTEGER NOT NULL,
                topic TEXT NOT NULL,
                questions TEXT NOT NULL DEFAULT '[]',
                search_queries TEXT NOT NULL DEFAULT '[]',
                depth TEXT NOT NULL DEFAULT 'standard',
                trigger_type TEXT NOT NULL DEFAULT 'user_request',
                status TEXT NOT NULL DEFAULT 'planned',
                budget_tokens INTEGER NOT NULL DEFAULT 5000,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                search_count INTEGER NOT NULL DEFAULT 0,
                max_searches INTEGER NOT NULL DEFAULT 3,
                findings TEXT NOT NULL DEFAULT '[]',
                synthesis TEXT,
                citations TEXT NOT NULL DEFAULT '[]',
                topic_key TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT,
                error TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_research_chat_status ON research_tasks(chat_id, status);
            CREATE INDEX IF NOT EXISTS idx_research_topic_key ON research_tasks(topic_key);

            -- Knowledge Base: CEO Nerve Center structured entries
            CREATE TABLE IF NOT EXISTS knowledge_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id INTEGER NOT NULL,
                category TEXT NOT NULL,
                priority TEXT NOT NULL DEFAULT 'normal',
                status TEXT NOT NULL DEFAULT 'active',
                title TEXT NOT NULL,
                body TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                related_ids TEXT NOT NULL DEFAULT '[]',
                source_msg_id INTEGER,
                source_summary TEXT,
                due_date TEXT,
                waiting_on TEXT,
                superseded_by INTEGER,
                resolved_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                embedding BLOB
            );
            CREATE INDEX IF NOT EXISTS idx_ke_chat_cat ON knowledge_entries(chat_id, category);
            CREATE INDEX IF NOT EXISTS idx_ke_chat_status ON knowledge_entries(chat_id, status);
            CREATE INDEX IF NOT EXISTS idx_ke_priority ON knowledge_entries(chat_id, priority, status);
            CREATE INDEX IF NOT EXISTS idx_ke_due ON knowledge_entries(due_date)
                WHERE due_date IS NOT NULL;",
        )
        .context("creating L2 + reminders + l1_snapshot + l3_archive + email_drafts tables")?;

    init_dev_task_draft_schema(conn).context("creating dev task draft tables")?;

    // Create FTS5 virtual table for full-text search over L3 archive.
    // content_rowid links to l3_archive.id for external content mode.
    // Using a separate FTS table avoids doubling storage of content.
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS l3_fts USING fts5(
                content, content_rowid=id, content='l3_archive'
            );",
    )
    .context("creating L3 FTS5 virtual table")?;

    // Migration: add embedding column to existing databases
    // Only ignore "duplicate column" errors; log everything else.
    if let Err(e) = conn.execute_batch("ALTER TABLE l2_experience ADD COLUMN embedding BLOB;") {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: l2_experience.embedding");
        }
    }

    // Migration (2026-09-12): reply latency on archived assistant rows.
    if let Err(e) = conn.execute_batch("ALTER TABLE l3_archive ADD COLUMN latency_ms INTEGER;") {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: l3_archive.latency_ms");
        }
    }

    // Migration: add version column to skill_proposals (Phase 1.3 — skill versioning)
    if let Err(e) = conn
        .execute_batch("ALTER TABLE skill_proposals ADD COLUMN version INTEGER NOT NULL DEFAULT 1;")
    {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: skill_proposals.version");
        }
    }

    // Index on (skill_name, version) — must be AFTER version migration
    if let Err(e) = conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_proposals_skill_version ON skill_proposals(skill_name, version);"
        ) {
            tracing::warn!(error = %e, "Failed to create idx_proposals_skill_version");
        }

    if let Err(e) = conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_proposals_skill_status_version
         ON skill_proposals(skill_name, status, version DESC);",
    ) {
        tracing::warn!(error = %e, "Failed to create idx_proposals_skill_status_version");
    }

    // Migration: add skill_md column to skill_proposals (declarative design docs)
    if let Err(e) = conn.execute_batch("ALTER TABLE skill_proposals ADD COLUMN skill_md TEXT;") {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: skill_proposals.skill_md");
        }
    }

    for (column, ddl) in [
        (
            "audit_override_by",
            "ALTER TABLE skill_proposals ADD COLUMN audit_override_by TEXT;",
        ),
        (
            "audit_override_at",
            "ALTER TABLE skill_proposals ADD COLUMN audit_override_at TEXT;",
        ),
        (
            "audit_override_reason",
            "ALTER TABLE skill_proposals ADD COLUMN audit_override_reason TEXT;",
        ),
    ] {
        if let Err(e) = conn.execute_batch(ddl) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                tracing::warn!(error = %msg, column, "Migration failed: skill_proposals audit override metadata");
            }
        }
    }

    // Normalized skill error categories table (Phase 10 — replaces JSON blob O(N²))
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS skill_error_categories (
                skill_name TEXT NOT NULL,
                category TEXT NOT NULL,
                count INTEGER NOT NULL DEFAULT 1,
                PRIMARY KEY (skill_name, category)
            );",
    )
    .context("creating skill_error_categories table")?;

    // Migration: add retry_count to deferred_intents (Phase 10 — poison pill prevention)
    if let Err(e) = conn.execute_batch(
        "ALTER TABLE deferred_intents ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;",
    ) {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: deferred_intents.retry_count");
        }
    }

    // Migration: add interval_seconds to deferred_intents (recurring intents)
    if let Err(e) =
        conn.execute_batch("ALTER TABLE deferred_intents ADD COLUMN interval_seconds INTEGER;")
    {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: deferred_intents.interval_seconds");
        }
    }

    // Migration: add chain_depth to deferred_intents (bounded callback chains)
    if let Err(e) = conn.execute_batch(
        "ALTER TABLE deferred_intents ADD COLUMN chain_depth INTEGER NOT NULL DEFAULT 0;",
    ) {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: deferred_intents.chain_depth");
        }
    }

    // Migration: add pin_on_fire to deferred_intents (high-priority pinning)
    if let Err(e) = conn.execute_batch(
        "ALTER TABLE deferred_intents ADD COLUMN pin_on_fire INTEGER NOT NULL DEFAULT 0;",
    ) {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: deferred_intents.pin_on_fire");
        }
    }

    // Migration: add group_key to deferred_intents (semantic dedup for recurring intents)
    if let Err(e) = conn.execute_batch("ALTER TABLE deferred_intents ADD COLUMN group_key TEXT;") {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: deferred_intents.group_key");
        }
    }

    // Migration: add max_fires + fire_count to deferred_intents (fire-count lifecycle)
    if let Err(e) = conn.execute_batch("ALTER TABLE deferred_intents ADD COLUMN max_fires INTEGER;")
    {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: deferred_intents.max_fires");
        }
    }
    if let Err(e) = conn.execute_batch(
        "ALTER TABLE deferred_intents ADD COLUMN fire_count INTEGER NOT NULL DEFAULT 0;",
    ) {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: deferred_intents.fire_count");
        }
    }

    // Migration: add embedding to deferred_intents (semantic dedup)
    if let Err(e) = conn.execute_batch("ALTER TABLE deferred_intents ADD COLUMN embedding BLOB;") {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: deferred_intents.embedding");
        }
    }

    // Migration: windowed weekday recurrence (Option C). Four nullable columns on
    // BOTH the legacy `deferred_intents` table and the unified `tasks` table (the
    // intent engine now reads/writes intents through `tasks`). recur_weekdays is a
    // JSON-encoded array of ISO weekdays (1=Mon..7=Sun); the window/timezone are
    // local "HH:MM" strings + an IANA tz. All NULL = classic fixed-interval (the
    // existing behavior is byte-identical when these are absent).
    for table in ["deferred_intents", "tasks"] {
        for (col, ty) in [
            ("recur_weekdays", "TEXT"),
            ("recur_window_start", "TEXT"),
            ("recur_window_end", "TEXT"),
            ("recur_timezone", "TEXT"),
        ] {
            let stmt = format!("ALTER TABLE {table} ADD COLUMN {col} {ty};");
            if let Err(e) = conn.execute_batch(&stmt) {
                let msg = e.to_string();
                if !msg.contains("duplicate column") {
                    tracing::warn!(error = %msg, "Migration failed: {}.{}", table, col);
                }
            }
        }
    }

    // Migration: archived_at terminal timestamp on `tasks`. Records WHEN a row was
    // moved to status='archived' so the DESTRUCTIVE retention sweep can compute
    // "how long has this been archived" instead of reusing the original
    // finished-at basis. Without this, an old completed task could be archived and
    // then immediately match the retention-delete cutoff in the SAME sweep,
    // skipping the intended archived grace period. NULL = pre-migration archived
    // row; we backfill those to now() below so they get a fresh grace clock rather
    // than being eligible for deletion the instant the column appears.
    if let Err(e) = conn.execute_batch("ALTER TABLE tasks ADD COLUMN archived_at TEXT;") {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: tasks.archived_at");
        }
    }
    // Backfill: any row already in 'archived' state but missing archived_at gets a
    // fresh clock so the retention sweep does not delete legacy archived rows on
    // first run. Idempotent (only touches NULL archived_at on archived rows).
    if let Err(e) = conn.execute_batch(
        "UPDATE tasks SET archived_at = COALESCE(archived_at, datetime('now')) \
             WHERE status = 'archived' AND archived_at IS NULL;",
    ) {
        tracing::warn!(error = %e, "Migration: archived_at backfill failed (non-fatal)");
    }

    // Index for group_key dedup lookup
    if let Err(e) = conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_intents_group_key ON deferred_intents(user_did, group_key, status);"
        ) {
            tracing::warn!(error = %e, "Failed to create idx_intents_group_key (non-fatal, queries still work)");
        }

    // Replay nonce persistence table (Phase 10 — cross-restart replay protection)
    conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS replay_nonces (
                signature_hex TEXT PRIMARY KEY,
                expires_at_unix INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_replay_nonces_expires ON replay_nonces(expires_at_unix);",
        ).context("creating replay_nonces table")?;

    // Migration: add last_accessed column to l2_experience (fact aging & confidence decay)
    if let Err(e) = conn.execute_batch("ALTER TABLE l2_experience ADD COLUMN last_accessed TEXT;") {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: l2_experience.last_accessed");
        }
    }
    // Backfill existing rows: set last_accessed = created_at where NULL
    if let Err(e) = conn.execute(
        "UPDATE l2_experience SET last_accessed = created_at WHERE last_accessed IS NULL",
        [],
    ) {
        tracing::warn!(error = %e, "Migration backfill l2_experience.last_accessed failed (non-fatal)");
    }

    // Migration: add usefulness column to research_tasks (quality feedback loop)
    if let Err(e) = conn.execute_batch("ALTER TABLE research_tasks ADD COLUMN usefulness INTEGER;")
    {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: research_tasks.usefulness");
        }
    }

    // Migration: circuit breaker columns on skill_metrics
    for stmt in &[
        "ALTER TABLE skill_metrics ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE skill_metrics ADD COLUMN circuit_open INTEGER NOT NULL DEFAULT 0",
    ] {
        if let Err(e) = conn.execute_batch(stmt) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                tracing::warn!(error = %msg, "Migration failed: skill_metrics circuit breaker");
            }
        }
    }

    // Command usage tracking — adaptive menu ordering
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS command_usage (
                command TEXT NOT NULL,
                user_id TEXT NOT NULL,
                total_count INTEGER NOT NULL DEFAULT 0,
                last_used TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (command, user_id)
            );",
    )
    .unwrap_or_else(|e| tracing::warn!(error = %e, "Migration: command_usage table"));

    // Smart panels — living, adaptive menu panels
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS smart_panels (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id INTEGER NOT NULL,
                panel_type TEXT NOT NULL,
                command TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT NOT NULL,
                config_json TEXT NOT NULL DEFAULT '{}',
                custom_schema_json TEXT,
                is_active INTEGER NOT NULL DEFAULT 1,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                is_locked INTEGER NOT NULL DEFAULT 0,
                priority_score REAL NOT NULL DEFAULT 50.0,
                render_count INTEGER NOT NULL DEFAULT 0,
                last_rendered_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                created_by TEXT NOT NULL DEFAULT 'system',
                UNIQUE(chat_id, command)
            );
            CREATE INDEX IF NOT EXISTS idx_panels_chat_active ON smart_panels(chat_id, is_active);
            CREATE INDEX IF NOT EXISTS idx_panels_chat_command ON smart_panels(chat_id, command);",
    )
    .unwrap_or_else(|e| tracing::warn!(error = %e, "Migration: smart_panels table"));

    // Non-custom panel types must be unique per chat — DB-level enforcement.
    // First clean any existing duplicates (keep lowest ID per chat+type), then add index.
    conn.execute_batch(
        "DELETE FROM smart_panels
             WHERE panel_type != 'custom'
               AND id NOT IN (
                   SELECT MIN(id) FROM smart_panels
                   WHERE panel_type != 'custom'
                   GROUP BY chat_id, panel_type
               );
             CREATE UNIQUE INDEX IF NOT EXISTS idx_panels_type_unique
                ON smart_panels(chat_id, panel_type)
                WHERE panel_type != 'custom';",
    )
    .unwrap_or_else(|e| tracing::warn!(error = %e, "Migration: panel type uniqueness index"));

    // Review guard pipeline log (3-layer effectiveness tracking)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS review_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL DEFAULT (datetime('now')),
                layer TEXT NOT NULL,
                passed INTEGER NOT NULL,
                tool_matched TEXT,
                similarity REAL,
                reply_preview TEXT,
                tools_called TEXT,
                latency_ms INTEGER,
                created_at TEXT DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_review_log_timestamp ON review_log(timestamp);
            CREATE INDEX IF NOT EXISTS idx_review_log_layer ON review_log(layer);
            CREATE INDEX IF NOT EXISTS idx_review_log_passed ON review_log(passed);",
    )
    .unwrap_or_else(|e| tracing::warn!(error = %e, "Migration: review_log table"));

    // Heartbeat autonomous action audit log
    conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS heartbeat_audit_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                cycle_id TEXT NOT NULL,
                skill_name TEXT NOT NULL,
                timestamp TEXT NOT NULL DEFAULT (datetime('now')),
                severity TEXT NOT NULL,
                probe_payload TEXT,
                tools_called TEXT NOT NULL DEFAULT '[]',
                tools_blocked TEXT NOT NULL DEFAULT '[]',
                actions_taken TEXT NOT NULL DEFAULT '[]',
                llm_reply TEXT,
                tokens_used INTEGER DEFAULT 0,
                rounds_used INTEGER DEFAULT 0,
                user_did TEXT NOT NULL,
                chat_id INTEGER,
                config_snapshot TEXT NOT NULL DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_hb_audit_cycle ON heartbeat_audit_log(cycle_id);
            CREATE INDEX IF NOT EXISTS idx_hb_audit_skill ON heartbeat_audit_log(skill_name, timestamp);"
        ).unwrap_or_else(|e| tracing::warn!(error = %e, "Migration: heartbeat_audit_log table"));

    // Migration: add raw_process_log column for WebApp dashboard
    conn.execute_batch("ALTER TABLE heartbeat_audit_log ADD COLUMN raw_process_log TEXT;")
        .unwrap_or(());

    // Migration: add tool_call_details column for structured span tracing (P0)
    conn.execute_batch("ALTER TABLE heartbeat_audit_log ADD COLUMN tool_call_details TEXT;")
        .unwrap_or(());

    // Migration (heartbeat autonomy Step 2, design §13): six new nullable
    // columns so every metronome tick — including precheck-skips and errors —
    // is visible in the audit trail. All nullable; old rows read back as NULL
    // / None and the reader fills defaults (`unwrap_or_default()` etc).
    // Each ALTER is wrapped in `.unwrap_or(())` so re-running on a DB that
    // already has the column is a no-op (SQLite returns "duplicate column").
    conn.execute_batch("ALTER TABLE heartbeat_audit_log ADD COLUMN outcome TEXT;")
        .unwrap_or(());
    conn.execute_batch("ALTER TABLE heartbeat_audit_log ADD COLUMN skip_reason TEXT;")
        .unwrap_or(());
    conn.execute_batch("ALTER TABLE heartbeat_audit_log ADD COLUMN config_snapshot_json TEXT;")
        .unwrap_or(());
    conn.execute_batch("ALTER TABLE heartbeat_audit_log ADD COLUMN prompt_hash TEXT;")
        .unwrap_or(());
    conn.execute_batch("ALTER TABLE heartbeat_audit_log ADD COLUMN proposals_created_json TEXT;")
        .unwrap_or(());
    conn.execute_batch("ALTER TABLE heartbeat_audit_log ADD COLUMN next_tick_note_bytes INTEGER;")
        .unwrap_or(());

    // Migration (heartbeat autonomy Step 9, design §6 + §15.1): extend
    // `skill_state` with HITL approval-queue metadata columns. The HITL
    // queue is stored as skill_state rows under skill_name='hitl'; Step 9
    // adds TTL, dedup-embedding, and audit-cycle linkback so heartbeat-
    // originated proposals (research / config_change) can be aged out
    // and deduped against pending peers.
    //
    // All four columns are nullable / default to safe sentinels so
    // existing skill_state rows (HITL and non-HITL) read back with
    // sensible defaults and are never touched by the sweeper.
    //
    //   kind                       — default 'generic'; heartbeat rows
    //                                use 'research_proposal' / 'config_change'
    //   proposed_at_audit_cycle_id — nullable; linkback to heartbeat audit row
    //   embedding_json             — nullable JSON array of floats for dedup
    //   expires_at                 — default '9999-12-31T00:00:00Z' sentinel
    //                                (non-expiring for pre-heartbeat rows)
    //
    // Each ALTER is wrapped in `.unwrap_or(())` so re-running on a DB
    // that already has the column is a no-op.
    conn.execute_batch("ALTER TABLE skill_state ADD COLUMN kind TEXT NOT NULL DEFAULT 'generic';")
        .unwrap_or(());
    conn.execute_batch("ALTER TABLE skill_state ADD COLUMN proposed_at_audit_cycle_id TEXT;")
        .unwrap_or(());
    conn.execute_batch("ALTER TABLE skill_state ADD COLUMN embedding_json TEXT;")
        .unwrap_or(());
    conn.execute_batch(
        "ALTER TABLE skill_state ADD COLUMN expires_at TEXT NOT NULL DEFAULT '9999-12-31T00:00:00Z';",
    )
    .unwrap_or(());

    // Migration: tool credit analysis table (P1)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tool_credit_analysis (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                analysis_date TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                times_called INTEGER NOT NULL DEFAULT 0,
                times_cited INTEGER NOT NULL DEFAULT 0,
                effectiveness_score REAL NOT NULL DEFAULT 0.0,
                wasted_calls INTEGER NOT NULL DEFAULT 0,
                avg_latency_ms REAL NOT NULL DEFAULT 0.0,
                total_tokens_attributed INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_tca_tool_date
                ON tool_credit_analysis(tool_name, analysis_date);",
    )
    .unwrap_or_else(|e| tracing::warn!(error = %e, "Migration: tool_credit_analysis table"));

    // Migration: LLM call log for token observability (#18)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS llm_call_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                label TEXT NOT NULL,
                model TEXT NOT NULL,
                provider TEXT,
                provider_request_id TEXT,
                prompt_tokens INTEGER NOT NULL,
                completion_tokens INTEGER NOT NULL,
                cached_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL,
                timestamp TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_llm_call_log_ts ON llm_call_log(timestamp);
            CREATE INDEX IF NOT EXISTS idx_llm_call_log_label ON llm_call_log(label, timestamp);
            CREATE INDEX IF NOT EXISTS idx_llm_call_log_provider_request_id
                ON llm_call_log(provider, provider_request_id);",
    )
    .unwrap_or_else(|e| tracing::warn!(error = %e, "Migration: llm_call_log table"));

    // Migration: prompt versions table (P2)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS prompt_versions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                module_name TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 1,
                original_text TEXT NOT NULL,
                modified_text TEXT NOT NULL,
                rationale TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                approval_id TEXT,
                metrics_snapshot TEXT,
                cycles_since_apply INTEGER DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_pv_module ON prompt_versions(module_name, status);",
    )
    .unwrap_or_else(|e| tracing::warn!(error = %e, "Migration: prompt_versions table"));

    // ── Migration: unify panel titles to bilingual format ─────────────
    conn.execute_batch(
            "UPDATE smart_panels SET title = '📋 任务 Tasks' WHERE panel_type = 'action_queue' AND title NOT LIKE '%任务%';
             UPDATE smart_panels SET title = '🔍 技能监控 Monitor' WHERE panel_type = 'monitor' AND title NOT LIKE '%监控%';
             UPDATE smart_panels SET title = '🚨 告警 Alerts' WHERE panel_type = 'alert' AND title NOT LIKE '%告警%';
             UPDATE smart_panels SET title = '📧 邮件 Email' WHERE panel_type = 'email_digest' AND title NOT LIKE '%邮件%';"
        ).unwrap_or_else(|e| tracing::warn!(error = %e, "Migration: panel title unification"));

    // ── One-time data migration: unify tasks ───────────────────────────
    // Check if migration has already been done via skill_state flag.
    let migration_done: bool = conn.query_row(
            "SELECT state_value FROM skill_state WHERE skill_name = 'system' AND state_key = 'tasks_migration_done'",
            [],
            |row| row.get::<_, String>(0),
        ).unwrap_or_default() == "v1";

    if !migration_done {
        tracing::info!("Starting unified task migration...");
        if let Err(e) = conn.execute_batch("BEGIN") {
            tracing::warn!(error = %e, "Failed to begin task migration transaction (non-fatal)");
        }
        let mut migrated = 0u64;

        // 1. Migrate KB action_items → tasks
        let kb_count = match conn.execute(
                "INSERT INTO tasks (chat_id, user_did, title, body, priority, status, tags, due_date, source, created_at)
                 SELECT chat_id, 'did:aegis:telegram:' || chat_id, title, body, priority,
                        CASE WHEN status = 'active' THEN 'active' WHEN status = 'done' THEN 'done' ELSE 'archived' END,
                        tags, due_date, 'migrated', created_at
                 FROM knowledge_entries WHERE category = 'action_item'",
                [],
            ) {
                Ok(n) => n as u64,
                Err(e) => { tracing::warn!(error = %e, "KB action_item → tasks migration INSERT failed"); 0 }
            };
        migrated += kb_count;
        if kb_count > 0 {
            conn.execute(
                    "UPDATE knowledge_entries SET status = 'migrated_to_tasks' WHERE category = 'action_item' AND status != 'migrated_to_tasks'",
                    [],
                ).unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "Failed to mark migrated knowledge_entries");
                    0
                });
        }
        tracing::info!(count = kb_count, "Migrated KB action_items → tasks");

        // 2. Migrate deferred_intents → tasks
        let di_count = match conn.execute(
                "INSERT INTO tasks (chat_id, user_did, title, body, status, trigger_at_unix, trigger_type, event_key,
                     execution_mode, context_text, interval_seconds, group_key, max_fires, fire_count,
                     fired_at, retry_count, chain_depth, pin_on_fire, expires_at_unix, is_callback_origin, source, created_at)
                 SELECT chat_id, user_did,
                     SUBSTR(context_text, 1, 60), context_text,
                     CASE WHEN status = 'pending' THEN 'active' ELSE status END,
                     trigger_at_unix, trigger_type, event_key,
                     execution_mode, context_text, interval_seconds, group_key, max_fires, fire_count,
                     fired_at, retry_count, chain_depth, pin_on_fire, expires_at_unix, is_callback_origin, 'migrated', created_at
                 FROM deferred_intents WHERE status = 'pending'",
                [],
            ) {
                Ok(n) => n as u64,
                Err(e) => { tracing::warn!(error = %e, "deferred_intents → tasks migration INSERT failed"); 0 }
            };
        migrated += di_count;
        tracing::info!(count = di_count, "Migrated deferred_intents → tasks");

        // 3. Migrate personal_tasks from skill_state JSON
        let pt_migrated = (|| -> u64 {
            let json_str: String = match conn.query_row(
                    "SELECT state_value FROM skill_state WHERE skill_name = 'personal_tasks' AND state_key = 'tasks'",
                    [],
                    |row| row.get(0),
                ) {
                    Ok(s) => s,
                    Err(_) => return 0, // No personal_tasks state — nothing to migrate
                };
            let tasks_arr: Vec<serde_json::Value> = match serde_json::from_str(&json_str) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "personal_tasks JSON parse failed during migration");
                    return 0;
                }
            };
            let mut count = 0u64;
            for t in &tasks_arr {
                let title = t
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Untitled");
                let completed = t
                    .get("completed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let is_pinned = t
                    .get("is_pinned")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let due = t.get("due_date").and_then(|v| v.as_str());
                let chat_id: i64 = t.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
                let status = if completed { "done" } else { "active" };
                let priority = if is_pinned { "high" } else { "normal" };
                match conn.execute(
                        "INSERT INTO tasks (chat_id, user_did, title, body, priority, status, due_date, source)
                         VALUES (?1, 'did:aegis:telegram:' || ?1, ?2, '', ?3, ?4, ?5, 'migrated')",
                        rusqlite::params![chat_id, title, priority, status, due],
                    ) {
                        Ok(_) => count += 1,
                        Err(e) => tracing::warn!(error = %e, title, "personal_task row migration failed — skipping"),
                    }
            }
            count
        })();
        migrated += pt_migrated;
        if pt_migrated > 0 {
            tracing::info!(count = pt_migrated, "Migrated personal_tasks → tasks");
        }

        // 4. Migrate reminders → tasks
        let rem_count = match conn.execute(
                "INSERT INTO tasks (chat_id, user_did, title, body, status, trigger_at_unix, trigger_type,
                     execution_mode, context_text, source, created_at)
                 SELECT chat_id, 'did:aegis:telegram:' || chat_id,
                        SUBSTR(message, 1, 60), message,
                        CASE WHEN fired = 0 THEN 'active' ELSE 'fired' END,
                        remind_at_unix, 'time', 'notify', message, 'migrated', created_at
                 FROM reminders WHERE fired = 0",
                [],
            ) {
                Ok(n) => n as u64,
                Err(e) => { tracing::warn!(error = %e, "reminders → tasks migration INSERT failed"); 0 }
            };
        migrated += rem_count;
        if rem_count > 0 {
            tracing::info!(count = rem_count, "Migrated reminders → tasks");
        }

        // Set migration flag
        if let Err(e) = conn.execute(
            "INSERT OR REPLACE INTO skill_state (skill_name, state_key, state_value, updated_at)
                 VALUES ('system', 'tasks_migration_done', 'v1', datetime('now'))",
            [],
        ) {
            tracing::warn!(error = %e, "Failed to set migration flag (non-fatal)");
        }
        if let Err(e) = conn.execute_batch("COMMIT") {
            tracing::warn!(error = %e, "Failed to commit task migration transaction (non-fatal)");
        }
        tracing::info!(total = migrated, "Unified task migration complete");
    }

    // Migration: add monitoring intent columns to tasks table
    for stmt in &[
        "ALTER TABLE tasks ADD COLUMN monitoring_threshold REAL",
        "ALTER TABLE tasks ADD COLUMN monitoring_skill TEXT",
        "ALTER TABLE tasks ADD COLUMN embedding_vector BLOB",
    ] {
        if let Err(e) = conn.execute_batch(stmt) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                tracing::warn!(error = %msg, "Migration failed: tasks monitoring columns");
            }
        }
    }
    // Index for efficient monitoring intent queries
    conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_tasks_monitoring ON tasks(trigger_type, status) WHERE trigger_type = 'semantic_alert';"
        ).unwrap_or_else(|e| tracing::warn!(error = %e, "Migration: monitoring intent index"));

    // ── One-time migration: L2 monitoring intents → unified tasks table ──
    let monitoring_migration_done: bool = conn.query_row(
            "SELECT state_value FROM skill_state WHERE skill_name = 'system' AND state_key = 'monitoring_intent_migration_done'",
            [],
            |row| row.get::<_, String>(0),
        ).unwrap_or_default() == "v1";

    if !monitoring_migration_done {
        // Migrate all L2 monitoring_intent:* facts into the tasks table
        if let Err(e) = conn.execute_batch("BEGIN") {
            tracing::warn!(error = %e, "Failed to begin monitoring intent migration transaction (non-fatal)");
        }
        let mut migrated_count = 0u64;
        let rows: Vec<(String, String, String, Vec<u8>)> = match conn.prepare(
                "SELECT agent_did, topic, fact_summary, embedding FROM l2_experience WHERE topic LIKE 'monitoring_intent:%' AND embedding IS NOT NULL"
            ) {
                Ok(mut stmt) => {
                    match stmt.query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Vec<u8>>(3)?,
                        ))
                    }) {
                        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
                        Err(e) => {
                            tracing::warn!(error = %e, "Monitoring intent migration query failed (non-fatal)");
                            vec![]
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Monitoring intent migration prepare failed (non-fatal)");
                    vec![]
                }
            };

        for (did, topic, fact_summary, embedding_blob) in &rows {
            // Parse the JSON to extract fields
            let parsed: serde_json::Value = serde_json::from_str(fact_summary).unwrap_or_default();
            let skill_name = parsed
                .get("skill_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let description = parsed
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let threshold = parsed
                .get("threshold")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.7);

            if skill_name.is_empty() || description.is_empty() {
                continue;
            }

            // Extract chat_id from DID (did:aegis:telegram:<chat_id>)
            let chat_id: i64 = did
                .strip_prefix("did:aegis:telegram:")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            if let Err(e) = conn.execute(
                    "INSERT INTO tasks (user_did, chat_id, title, body, trigger_type, execution_mode, context_text, status, source, monitoring_skill, monitoring_threshold, embedding_vector)
                     VALUES (?1, ?2, ?3, ?4, 'semantic_alert', 'notify', ?4, 'active', 'migrated', ?5, ?6, ?7)",
                    rusqlite::params![
                        did, chat_id,
                        format!("Monitor: {}", skill_name),
                        description,
                        skill_name,
                        threshold,
                        embedding_blob,
                    ],
                ) {
                    tracing::warn!(topic = %topic, error = %e, "Failed to migrate monitoring intent (non-fatal)");
                } else {
                    migrated_count += 1;
                }
        }

        if migrated_count > 0 {
            tracing::info!(
                count = migrated_count,
                "Migrated L2 monitoring intents → tasks"
            );
        }

        // Set migration flag
        if let Err(e) = conn.execute(
            "INSERT OR REPLACE INTO skill_state (skill_name, state_key, state_value, updated_at)
                 VALUES ('system', 'monitoring_intent_migration_done', 'v1', datetime('now'))",
            [],
        ) {
            tracing::warn!(error = %e, "Failed to set monitoring intent migration flag (non-fatal)");
        }
        if let Err(e) = conn.execute_batch("COMMIT") {
            tracing::warn!(error = %e, "Failed to commit monitoring intent migration transaction (non-fatal)");
        }
    }

    let cleared_stale_stage_errors = conn
        .execute(
            "UPDATE skill_proposals
             SET stage_error = NULL
             WHERE status = 'pending_final_approval'
               AND stage_error IS NOT NULL",
            [],
        )
        .context("clearing stale SkillForge final-approval stage_error values")?;
    if cleared_stale_stage_errors > 0 {
        tracing::info!(
            count = cleared_stale_stage_errors,
            "Cleared stale SkillForge stage_error values for pending_final_approval proposals"
        );
    }

    // ── Cleanup: fully purge redundant SkillForge skills ──
    // Deletes all traces: proposals, metrics, daily snapshots, initiative proposals,
    // and L2 lessons/frustrations that reference these retired skills.
    {
        let mut total_deleted = 0usize;
        for skill_name in REDUNDANT_SKILLS {
            for (table, col, query) in [
                (
                    "skill_proposals",
                    "skill_name",
                    "DELETE FROM skill_proposals WHERE skill_name = ?1",
                ),
                (
                    "skill_metrics",
                    "skill_name",
                    "DELETE FROM skill_metrics WHERE skill_name = ?1",
                ),
                (
                    "skill_metrics_daily",
                    "skill_name",
                    "DELETE FROM skill_metrics_daily WHERE skill_name = ?1",
                ),
            ] {
                match conn.execute(query, [skill_name]) {
                    Ok(n) => total_deleted += n,
                    Err(e) => {
                        tracing::warn!(error = %e, table, col, skill_name, "Redundant skill cleanup (non-fatal)")
                    }
                }
            }
            // Purge initiative proposals that target this skill (in proposed_action JSON)
            let pattern = format!("%\"skill_name\":\"{}\"%", skill_name);
            match conn.execute(
                "DELETE FROM initiative_proposals WHERE proposed_action LIKE ?1",
                [&pattern],
            ) {
                Ok(n) => total_deleted += n,
                Err(e) => {
                    tracing::debug!(error = %e, skill_name, "Initiative proposal cleanup (non-fatal)")
                }
            }
            // Purge L2 lessons/frustrations about this skill.
            // Only match compound names (containing '_') to avoid false
            // positives from common words like "reminder" in unrelated lessons.
            if !skill_name.contains('_') {
                continue;
            }
            let l2_pattern = format!("%{}%", skill_name);
            match conn.execute(
                "DELETE FROM l2_experience WHERE fact_summary LIKE ?1",
                [&l2_pattern],
            ) {
                Ok(n) => total_deleted += n,
                Err(e) => {
                    tracing::debug!(error = %e, skill_name, "L2 experience cleanup (non-fatal)")
                }
            }
        }
        if total_deleted > 0 {
            tracing::info!(
                count = total_deleted,
                "Purged all traces of redundant skills"
            );
        }
    }

    // Register vec_cosine_sim(blob_a, blob_b) → REAL
    // Computes cosine similarity between two embedding BLOBs directly in SQL,
    // removing the LIMIT 500 cap on semantic search.
    conn.create_scalar_function(
        "vec_cosine_sim",
        2,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8
            | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let blob_a = ctx.get_raw(0).as_blob().unwrap_or(&[]);
            let blob_b = ctx.get_raw(1).as_blob().unwrap_or(&[]);

            if blob_a.len() != blob_b.len() || blob_a.len() % 4 != 0 {
                return Ok(0.0f64);
            }

            let mut dot: f64 = 0.0;
            let mut norm_a: f64 = 0.0;
            let mut norm_b: f64 = 0.0;

            for (chunk_a, chunk_b) in blob_a
                .as_chunks::<4>()
                .0
                .iter()
                .zip(blob_b.as_chunks::<4>().0.iter())
            {
                let a = f32::from_le_bytes([chunk_a[0], chunk_a[1], chunk_a[2], chunk_a[3]]) as f64;
                let b = f32::from_le_bytes([chunk_b[0], chunk_b[1], chunk_b[2], chunk_b[3]]) as f64;
                dot += a * b;
                norm_a += a * a;
                norm_b += b * b;
            }

            let denom = norm_a.sqrt() * norm_b.sqrt();
            if denom == 0.0 {
                Ok(0.0f64)
            } else {
                Ok(dot / denom)
            }
        },
    )
    .context("registering vec_cosine_sim SQL function")?;

    // ── Phase 12: Orphan cleanup (application-level FK enforcement) ─────
    // SQLite doesn't support ALTER TABLE ADD FOREIGN KEY. Instead, we
    // periodically clean orphaned rows that reference deleted parent records.
    // This runs on every startup and is idempotent.
    let skill_state_orphan_cleanup = format!(
        "DELETE FROM skill_state \
         WHERE skill_name NOT IN (SELECT skill_name FROM skill_proposals) \
           AND skill_name NOT IN ({INTERNAL_SKILL_STATE_NAMES_SQL})"
    );
    let orphan_cleanup = [
        // skill_metrics referencing deleted skills
        "DELETE FROM skill_metrics WHERE skill_name NOT IN (SELECT skill_name FROM skill_proposals)".to_string(),
        // skill_error_categories referencing deleted skills
        "DELETE FROM skill_error_categories WHERE skill_name NOT IN (SELECT skill_name FROM skill_proposals)".to_string(),
        // skill_metrics_daily referencing deleted skills
        "DELETE FROM skill_metrics_daily WHERE skill_name NOT IN (SELECT skill_name FROM skill_proposals)".to_string(),
        // skill_golden_tests referencing deleted skills
        "DELETE FROM skill_golden_tests WHERE skill_name NOT IN (SELECT skill_name FROM skill_proposals)".to_string(),
        // skill_state referencing deleted skills. Internal runtime namespaces
        // also live here, so do not delete them merely because they are not
        // SkillForge proposals.
        skill_state_orphan_cleanup,
        // Startup status notices were recorded as assistant turns on every
        // restart (hundreds of "系统已重启" rows); they are operational noise
        // that memory recall kept surfacing. Idempotent purge (2026-09-06).
        "DELETE FROM l3_archive WHERE role = 'assistant' AND content LIKE '%系统已重启 / System restarted%'".to_string(),
    ];
    for sql in &orphan_cleanup {
        match conn.execute(sql.as_str(), []) {
            Ok(n) if n > 0 => {
                tracing::info!(deleted = n, sql = %sql, "Orphan cleanup: removed {n} rows")
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, sql = %sql, "Orphan cleanup failed (table may not exist yet)")
            }
        }
    }

    // ── SkillForge v2: capability_atoms table ──────────────────────────────
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS capability_atoms (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            source      TEXT NOT NULL DEFAULT 'generated',
            kind_tag    TEXT NOT NULL DEFAULT '',
            description TEXT NOT NULL DEFAULT '',
            spec_json   TEXT,
            wasm_path   TEXT,
            created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            last_used   TEXT
        );",
    )
    .context("creating capability_atoms table")?;

    // ── SkillForge v2: add graph/dag columns to skill_proposals ─────────
    for col in &[
        "ALTER TABLE skill_proposals ADD COLUMN capability_graph_json TEXT",
        "ALTER TABLE skill_proposals ADD COLUMN execution_dag_json TEXT",
    ] {
        if let Err(e) = conn.execute_batch(col) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                tracing::warn!(error = %msg, "Migration failed: skill_proposals v2 column");
            }
        }
    }

    let demoted_artifactless_final_approval = conn
        .execute(
            "UPDATE skill_proposals
             SET status = 'implement_failed',
                 stage_error = 'Final approval artifact check failed: compiled wasm or manifest is missing; retry the SkillForge pipeline before approval.',
                 updated_at = datetime('now')
             WHERE status IN ('pending_final_approval', 'activating')
               AND (compiled_wasm IS NULL OR generated_manifest IS NULL)
               AND execution_dag_json IS NULL",
            [],
        )
        .context("demoting artifactless SkillForge final-approval proposals")?;
    if demoted_artifactless_final_approval > 0 {
        tracing::warn!(
            count = demoted_artifactless_final_approval,
            "Demoted SkillForge final-approval proposals missing activation artifacts"
        );
    }
    repair_server_health_aggregator_composite_dag(conn)?;
    repair_generate_summary_composite_dag(conn)?;
    repair_generate_summary_pdt_cron(conn)?;

    // Migration: l2_recall_stats table (self-learning recall tracking — Phase 1)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS l2_recall_stats (
            fact_id        INTEGER PRIMARY KEY,
            recall_count   INTEGER NOT NULL DEFAULT 0,
            total_score    REAL    NOT NULL DEFAULT 0.0,
            max_score      REAL    NOT NULL DEFAULT 0.0,
            unique_queries INTEGER NOT NULL DEFAULT 0,
            query_hashes   TEXT    NOT NULL DEFAULT '[]',
            recall_days    TEXT    NOT NULL DEFAULT '[]',
            first_recalled_at TEXT,
            last_recalled_at  TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_recall_stats_count
            ON l2_recall_stats(recall_count DESC);",
    )
    .context("creating l2_recall_stats table")?;

    // Migration: relationship_state table (G1 永续关系 — Phase 1)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS relationship_state (
            user_did           TEXT PRIMARY KEY,
            current_focus      TEXT NOT NULL DEFAULT '[]',
            ongoing_goals      TEXT NOT NULL DEFAULT '[]',
            narrative          TEXT NOT NULL DEFAULT '',
            last_interaction_at TEXT,
            last_topic         TEXT NOT NULL DEFAULT '',
            interaction_count  INTEGER NOT NULL DEFAULT 0,
            streak_days        INTEGER NOT NULL DEFAULT 0,
            updated_at         TEXT NOT NULL DEFAULT ''
        );",
    )
    .context("creating relationship_state table")?;

    // Migration: goals table (G5 Long-term Goal Tracking)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS goals (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            user_did         TEXT NOT NULL,
            title            TEXT NOT NULL,
            description      TEXT NOT NULL DEFAULT '',
            status           TEXT NOT NULL DEFAULT 'active',
            parent_goal_id   INTEGER,
            progress_pct     REAL NOT NULL DEFAULT 0.0,
            milestones       TEXT NOT NULL DEFAULT '[]',
            created_at       TEXT NOT NULL,
            target_date      TEXT,
            last_progress_at TEXT,
            source           TEXT NOT NULL DEFAULT 'explicit',
            FOREIGN KEY (parent_goal_id) REFERENCES goals(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_goals_user_status ON goals(user_did, status);",
    )
    .context("creating goals table")?;

    // Migration: agent_manifests table (G2 Multi-Agent Swarm — Phase 1)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_manifests (
            agent_id      TEXT PRIMARY KEY,
            batch_id      TEXT NOT NULL,
            agent_type    TEXT NOT NULL,
            description   TEXT NOT NULL,
            prompt        TEXT NOT NULL,
            status        TEXT NOT NULL DEFAULT 'spawned',
            chat_id       INTEGER NOT NULL,
            allowed_tools TEXT NOT NULL DEFAULT '[]',
            result_summary TEXT,
            error         TEXT,
            lane_events   TEXT NOT NULL DEFAULT '[]',
            created_at    TEXT NOT NULL,
            started_at    TEXT,
            completed_at  TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_agent_status
            ON agent_manifests(status);
        CREATE INDEX IF NOT EXISTS idx_agent_batch
            ON agent_manifests(batch_id);",
    )
    .context("creating agent_manifests table")?;

    // Batch synthesis dedup: prevents two agents completing simultaneously from
    // both triggering synthesis.  INSERT OR IGNORE + rows_affected == 1 = claim.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_batch_synthesis (
            batch_id       TEXT PRIMARY KEY,
            synthesized_at TEXT NOT NULL
        );",
    )
    .context("creating agent_batch_synthesis table")?;

    // Migration: add retry_count column (G2 Phase 5 — agent retry/cancel)
    // ALTER TABLE with IF NOT EXISTS is not available in SQLite, so we check
    // the column existence first.
    {
        let has_retry_count: bool = conn
            .prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_manifests'")
            .and_then(|mut s| s.query_row([], |r| r.get::<_, String>(0)))
            .map(|ddl| ddl.contains("retry_count"))
            .unwrap_or(false);
        if !has_retry_count {
            conn.execute_batch(
                "ALTER TABLE agent_manifests ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;",
            )
            .context("adding retry_count column to agent_manifests")?;
        }
    }

    // G5: Goal-fact links — tracks which L2 facts relate to which goals
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS goal_fact_links (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            goal_id     INTEGER NOT NULL,
            fact_id     INTEGER NOT NULL,
            similarity  REAL NOT NULL,
            created_at  TEXT NOT NULL,
            FOREIGN KEY (goal_id) REFERENCES goals(id) ON DELETE CASCADE,
            FOREIGN KEY (fact_id) REFERENCES l2_experience(id) ON DELETE CASCADE,
            UNIQUE(goal_id, fact_id)
        );",
    )
    .context("creating goal_fact_links table")?;

    // G6: Active hours histogram — ALTER TABLE to add column
    {
        let has_active_hours: bool = conn
            .prepare("SELECT active_hours FROM relationship_state LIMIT 0")
            .is_ok();
        if !has_active_hours {
            conn.execute_batch(
                "ALTER TABLE relationship_state ADD COLUMN active_hours TEXT NOT NULL DEFAULT '{}';",
            )
            .context("adding active_hours column to relationship_state")?;
        }
    }

    // L2 experience: add confidence score + expiration for lesson quality and TTL.
    // confidence: extracted LLM confidence (0.0-1.0), used for lesson injection ranking.
    // expires_at: optional TTL — stale facts are filtered out during injection.
    {
        let has_confidence: bool = conn
            .prepare("SELECT confidence FROM l2_experience LIMIT 0")
            .is_ok();
        if !has_confidence {
            conn.execute_batch(
                "ALTER TABLE l2_experience ADD COLUMN confidence REAL NOT NULL DEFAULT 0.6;",
            )
            .context("adding confidence column to l2_experience")?;
        }
        // Backfill: ALTER TABLE ADD COLUMN NOT NULL DEFAULT leaves physical NULLs in existing rows.
        // PRAGMA integrity_check detects these. Force-write the default to materialize values.
        conn.execute(
            "UPDATE l2_experience SET confidence = COALESCE(confidence, 0.6)",
            [],
        )
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to backfill l2_experience.confidence (non-fatal)");
            0
        });
    }
    {
        let has_expires: bool = conn
            .prepare("SELECT expires_at FROM l2_experience LIMIT 0")
            .is_ok();
        if !has_expires {
            conn.execute_batch("ALTER TABLE l2_experience ADD COLUMN expires_at TEXT;")
                .context("adding expires_at column to l2_experience")?;
        }
    }
    {
        let has_is_pinned: bool = conn
            .prepare("SELECT is_pinned FROM l2_experience LIMIT 0")
            .is_ok();
        if !has_is_pinned {
            conn.execute_batch(
                "ALTER TABLE l2_experience ADD COLUMN is_pinned INTEGER NOT NULL DEFAULT 0;",
            )
            .context("adding is_pinned column to l2_experience")?;
        }
        conn.execute(
            "UPDATE l2_experience SET is_pinned = COALESCE(is_pinned, 0)",
            [],
        )
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to backfill l2_experience.is_pinned (non-fatal)");
            0
        });
    }

    // Multi-mailbox: add account_id to email_drafts (replaces fragile L2 workaround)
    {
        let has_col = conn
            .prepare("SELECT account_id FROM email_drafts LIMIT 0")
            .is_ok();
        if !has_col {
            conn.execute_batch(
                "ALTER TABLE email_drafts ADD COLUMN account_id TEXT NOT NULL DEFAULT '';",
            )
            .context("adding account_id column to email_drafts")?;
        }
    }

    // Cost tracking: extend llm_call_log with cost/latency/fallback columns
    {
        let has_cost = conn
            .prepare("SELECT cost_usd FROM llm_call_log LIMIT 0")
            .is_ok();
        if !has_cost {
            conn.execute_batch(
                "ALTER TABLE llm_call_log ADD COLUMN cost_usd REAL;
                 ALTER TABLE llm_call_log ADD COLUMN latency_ms INTEGER;
                 ALTER TABLE llm_call_log ADD COLUMN is_fallback INTEGER NOT NULL DEFAULT 0;",
            )
            .context("adding cost/latency/fallback columns to llm_call_log")?;
        }
    }

    // Provider reconciliation: persist provider request/generation IDs for
    // future exact billing checks against provider detail APIs.
    {
        let has_provider = conn
            .prepare("SELECT provider_request_id FROM llm_call_log LIMIT 0")
            .is_ok();
        if !has_provider {
            conn.execute_batch(
                "ALTER TABLE llm_call_log ADD COLUMN provider TEXT;
                 ALTER TABLE llm_call_log ADD COLUMN provider_request_id TEXT;
                 CREATE INDEX IF NOT EXISTS idx_llm_call_log_provider_request_id
                    ON llm_call_log(provider, provider_request_id);",
            )
            .context("adding provider reconciliation columns to llm_call_log")?;
        }
    }

    // Cost tracking: local pricing cache table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS llm_pricing (
            model TEXT PRIMARY KEY,
            prompt_per_token REAL NOT NULL,
            completion_per_token REAL NOT NULL,
            cache_read_per_token REAL NOT NULL DEFAULT 0.0,
            cache_write_per_token REAL NOT NULL DEFAULT 0.0,
            image_per_token REAL NOT NULL DEFAULT 0.0,
            request_cost REAL NOT NULL DEFAULT 0.0,
            web_search_cost REAL NOT NULL DEFAULT 0.0,
            source TEXT NOT NULL DEFAULT 'api',
            fetched_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .context("creating llm_pricing table")?;

    // Cost tracking: pre-aggregated daily rollups
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS llm_cost_daily (
            date TEXT NOT NULL,
            label TEXT NOT NULL,
            model TEXT NOT NULL,
            call_count INTEGER NOT NULL DEFAULT 0,
            total_prompt_tokens INTEGER NOT NULL DEFAULT 0,
            total_completion_tokens INTEGER NOT NULL DEFAULT 0,
            total_cached_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost_usd REAL NOT NULL DEFAULT 0.0,
            avg_latency_ms REAL NOT NULL DEFAULT 0.0,
            fallback_count INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (date, label, model)
        );
        CREATE INDEX IF NOT EXISTS idx_lcd_date ON llm_cost_daily(date);",
    )
    .context("creating llm_cost_daily table")?;

    // Knowledge Linting: findings table for contradiction / gap / connection detection
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS l2_lint_findings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                finding_type TEXT NOT NULL,
                fact_id_a INTEGER,
                fact_id_b INTEGER,
                similarity REAL,
                llm_verdict TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                user_action TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                resolved_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_lint_status ON l2_lint_findings(status);
            CREATE INDEX IF NOT EXISTS idx_lint_type ON l2_lint_findings(finding_type, status);",
    )
    .context("creating l2_lint_findings table")?;

    // ── Layer C: Companion Growth tables ──────────────────────────────────

    // companion_identity — singleton "soul" table (CHECK id=1)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS companion_identity (
            id                INTEGER PRIMARY KEY CHECK (id = 1),
            name              TEXT NOT NULL DEFAULT '',
            name_given_at     TEXT,
            core_values       TEXT NOT NULL DEFAULT '',
            personality       TEXT NOT NULL DEFAULT '{}',
            communication_notes TEXT NOT NULL DEFAULT '',
            growth_stage      TEXT NOT NULL DEFAULT 'chu_yu',
            created_at        TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at        TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .context("creating companion_identity table")?;

    // growth_dimensions — row per axis, extensible, UNBOUNDED depth
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS growth_dimensions (
            dimension_key    TEXT PRIMARY KEY,
            display_name     TEXT NOT NULL,
            depth            REAL NOT NULL DEFAULT 0.0,
            velocity         REAL NOT NULL DEFAULT 0.0,
            evidence_count   INTEGER NOT NULL DEFAULT 0,
            last_evidence_at TEXT,
            description      TEXT NOT NULL DEFAULT '',
            updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .context("creating growth_dimensions table")?;

    // growth_log — append-only permanent record
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS growth_log (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            event_type      TEXT NOT NULL,
            dimension_key   TEXT,
            detail          TEXT NOT NULL,
            delta           REAL,
            level_after     REAL,
            metadata        TEXT NOT NULL DEFAULT '{}',
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_growth_log_event ON growth_log(event_type);
        CREATE INDEX IF NOT EXISTS idx_growth_log_dim ON growth_log(dimension_key);
        CREATE INDEX IF NOT EXISTS idx_growth_log_created ON growth_log(created_at);",
    )
    .context("creating growth_log table")?;

    // Seed companion_identity singleton if not present
    let identity_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM companion_identity WHERE id = 1",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !identity_exists {
        // Backfill: read interaction_count from relationship_state to set initial growth_stage
        let interaction_count: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(interaction_count), 0) FROM relationship_state",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        let initial_stage = if interaction_count >= 10000 {
            "jue_wu"
        } else if interaction_count >= 2000 {
            "gong_sheng"
        } else if interaction_count >= 500 {
            "mo_qi"
        } else if interaction_count >= 100 {
            "xiang_zhi"
        } else {
            "chu_yu"
        };

        conn.execute(
            "INSERT INTO companion_identity (id, core_values, growth_stage, personality)
             VALUES (1, ?1, ?2, ?3)",
            rusqlite::params![
                "Care and good judgment: act with care, weigh each action by its effect on everyone it touches, and prefer what is true over what is comfortable.\nAegis \u{795e}\u{76fe} \u{2014} \u{81f3}\u{9ad8}\u{65e0}\u{4e0a}\u{7684}\u{4fdd}\u{62a4}\u{ff0c}\u{5b88}\u{62a4}\u{6b63}\u{4e49}\u{7684}\u{6c38}\u{6052}\u{529b}\u{91cf}\u{3002}",
                initial_stage,
                r#"{"warmth":0.5,"formality":0.5,"humor":0.2,"directness":0.5,"verbosity":0.5}"#,
            ],
        )
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to seed companion_identity (non-fatal)");
            0
        });

        tracing::info!(
            interaction_count,
            initial_stage,
            "Seeded companion_identity singleton"
        );
    }
    // Backfill: ensure personality has default weights (empty '{}' → proper defaults)
    conn.execute(
        "UPDATE companion_identity SET personality = ?1 WHERE id = 1 AND personality = '{}'",
        rusqlite::params![
            r#"{"warmth":0.5,"formality":0.5,"humor":0.2,"directness":0.5,"verbosity":0.5}"#,
        ],
    )
    .ok();

    // Seed default growth dimensions if empty
    let dim_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM growth_dimensions", [], |r| r.get(0))
        .unwrap_or(0);
    if dim_count == 0 {
        let dimensions = [
            (
                "rapport",
                "\u{9ed8}\u{5951} Rapport",
                "Mutual understanding and communication shorthand",
            ),
            (
                "trust",
                "\u{4fe1}\u{4efb} Trust",
                "Delegation, permission granting, reliance",
            ),
            (
                "expertise",
                "\u{4e13}\u{4e1a} Expertise",
                "Domain task success and professional growth",
            ),
            (
                "attunement",
                "\u{5171}\u{9e23} Attunement",
                "Emotional reading and empathic accuracy",
            ),
            (
                "proactivity",
                "\u{4e3b}\u{52a8} Proactivity",
                "Suggestion acceptance and initiative effectiveness",
            ),
        ];
        for (key, display, desc) in &dimensions {
            conn.execute(
                "INSERT OR IGNORE INTO growth_dimensions (dimension_key, display_name, description) VALUES (?1, ?2, ?3)",
                rusqlite::params![key, display, desc],
            )
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, key, "Failed to seed growth dimension");
                0
            });
        }

        // Backfill rapport depth from existing interaction_count
        let interaction_count: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(interaction_count), 0) FROM relationship_state",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if interaction_count > 0 {
            // Set initial rapport depth proportional to interactions (0.002 per interaction, same as per-interaction micro-adjustment)
            let initial_depth = (interaction_count as f64) * 0.002;
            conn.execute(
                "UPDATE growth_dimensions SET depth = ?1, evidence_count = ?2 WHERE dimension_key = 'rapport'",
                rusqlite::params![initial_depth, interaction_count],
            )
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Failed to backfill rapport depth");
                0
            });
            tracing::info!(
                interaction_count,
                initial_depth,
                "Backfilled rapport depth from interaction count"
            );
        }
    }

    // ── notification_feedback table for heartbeat notification thumbs up/down ──
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS notification_feedback (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            notif_id    TEXT NOT NULL UNIQUE,
            helpful     INTEGER NOT NULL,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_notif_feedback_created
            ON notification_feedback(created_at);",
    )
    .context("notification_feedback table")?;

    // ── Memory Tier Redesign: Tier 0 (user_profile) ──────────────────────
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS user_profile (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_did TEXT NOT NULL,
            category TEXT NOT NULL,
            key TEXT NOT NULL,
            value TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0.9,
            source TEXT NOT NULL DEFAULT 'auto',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(agent_did, category, key)
        );
        CREATE INDEX IF NOT EXISTS idx_profile_did ON user_profile(agent_did);",
    )
    .context("creating user_profile table")?;

    // ── Memory Tier Redesign: Tier 2 (l2_ephemeral) ─────────────────────
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS l2_ephemeral (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_did TEXT NOT NULL,
            topic TEXT NOT NULL,
            fact_summary TEXT NOT NULL,
            embedding BLOB,
            source TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0.6,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            expires_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_ephemeral_did_expires
            ON l2_ephemeral(agent_did, expires_at);
        CREATE INDEX IF NOT EXISTS idx_ephemeral_source
            ON l2_ephemeral(source);",
    )
    .context("creating l2_ephemeral table")?;

    // ── Memory Tier Redesign: extraction_cursor ─────────────────────────
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS extraction_cursor (
            agent_did TEXT PRIMARY KEY,
            last_processed_ts TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z',
            total_extracted INTEGER NOT NULL DEFAULT 0
        );",
    )
    .context("creating extraction_cursor table")?;

    // ── Memory Tier Redesign: reflection_log (spec §5.3 Phase 4 item 7) ──
    // Append-only record of reflection cycles for observability — one row per
    // `run_reflection()` invocation that actually performed work.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS reflection_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL DEFAULT (datetime('now')),
            merges INTEGER NOT NULL DEFAULT 0,
            prunes INTEGER NOT NULL DEFAULT 0,
            flags INTEGER NOT NULL DEFAULT 0,
            promotions INTEGER NOT NULL DEFAULT 0,
            initiative_archived INTEGER NOT NULL DEFAULT 0,
            ephemeral_deleted INTEGER NOT NULL DEFAULT 0,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            skipped_reason TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_reflection_log_ts ON reflection_log(timestamp);",
    )
    .context("creating reflection_log table")?;

    // ── Contradiction notification dedup ──────────────────────────────
    // Records which `(new_fact_hash, existing_fact_id)` pairs have already
    // been surfaced to the user, so the same pair is not re-notified every
    // extraction cycle. `new_fact_hash` is the first 16 hex chars of
    // SHA256(trim+lowercase(new_fact_content)) — cheap, stable, and lets us
    // detect "same new fact arrived again" across cycles without storing
    // the full content. Rows older than the dedup window (7d by default)
    // are periodically cleaned by caller logic or expire implicitly as new
    // rows supersede them.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS contradiction_notified (
            agent_did TEXT NOT NULL,
            new_fact_hash TEXT NOT NULL,
            existing_fact_id INTEGER NOT NULL,
            notified_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (agent_did, new_fact_hash, existing_fact_id)
        );
        CREATE INDEX IF NOT EXISTS idx_contradiction_notified_at
            ON contradiction_notified(notified_at);",
    )
    .context("creating contradiction_notified table")?;

    // ── Post-migration column verification ─────────────────────────────
    // Verify critical columns exist after ALTER TABLE and CREATE TABLE DDL.
    // This catches silent migration failures (e.g., disk full, permissions).
    // Must run AFTER all CREATE TABLE statements so the tier-redesign tables
    // (user_profile, l2_ephemeral, extraction_cursor, reflection_log) are visible.
    let critical_columns = [
        (
            "deferred_intents",
            &[
                "retry_count",
                "interval_seconds",
                "chain_depth",
                "group_key",
                "max_fires",
                "fire_count",
                "embedding",
                "pin_on_fire",
            ][..],
        ),
        (
            "skill_proposals",
            &[
                "version",
                "skill_md",
                "audit_override_by",
                "audit_override_at",
                "audit_override_reason",
            ][..],
        ),
        ("l2_experience", &["embedding", "last_accessed"]),
        ("skill_metrics", &["consecutive_failures", "circuit_open"]),
        (
            "tasks",
            &[
                "monitoring_threshold",
                "monitoring_skill",
                "embedding_vector",
            ],
        ),
        (
            "user_profile",
            &[
                "agent_did",
                "category",
                "key",
                "value",
                "confidence",
                "source",
            ],
        ),
        (
            "l2_ephemeral",
            &["agent_did", "topic", "fact_summary", "source", "expires_at"],
        ),
        (
            "extraction_cursor",
            &["agent_did", "last_processed_ts", "total_extracted"],
        ),
    ];
    for (table, expected_cols) in &critical_columns {
        let existing: Vec<String> = conn
            .prepare(&format!("PRAGMA table_info({})", table))
            .context("PRAGMA table_info")?
            .query_map([], |row| row.get::<_, String>(1))
            .context("querying table_info")?
            .filter_map(|r| r.ok())
            .collect();
        for col in *expected_cols {
            if !existing.iter().any(|c| c == col) {
                tracing::error!(
                    table = %table, column = %col,
                    "CRITICAL: expected column missing after migration — schema may be corrupted"
                );
            }
        }
    }

    // ── Memory Tier Redesign: One-time DID unification + data migration ──
    let tier_migration_done: bool = conn
        .query_row(
            "SELECT state_value FROM skill_state WHERE skill_name = 'system' AND state_key = 'tier_migration_done'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_default()
        == "v1";

    if !tier_migration_done {
        tracing::info!("Starting memory tier redesign migration...");

        // All steps run inside a SAVEPOINT. If any step fails, the entire migration
        // rolls back — no partial state, no data loss. The flag is only set on success.
        let result = run_tier_migration(conn, owner_did);
        match result {
            Ok(stats) => {
                tracing::info!(
                    total_to_ephemeral = stats.total_to_ephemeral,
                    deleted_from_tier1 = stats.deleted_from_tier1,
                    did_unified = stats.did_unified,
                    profile_seeded = stats.profile_seeded,
                    "Memory tier redesign migration complete"
                );
            }
            Err(e) => {
                // Migration failed and was rolled back — safe to retry on next startup.
                tracing::error!(
                    error = %e,
                    "Memory tier migration FAILED — rolled back. Will retry on next startup."
                );
            }
        }
    }

    // ── FU#2 Phase 7: Profile Slot Registry migration ──────────────────
    // MUST run AFTER tier_migration: the tier migration unifies DIDs and seeds
    // profile rows, so Phase 7's canonical-owner collision index needs to see
    // post-tier state. Idempotent via a skill_state flag.
    let profile_registry_migration_done: bool = conn
        .query_row(
            "SELECT state_value FROM skill_state WHERE skill_name = 'system' \
             AND state_key = 'profile_registry_migration_done'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_default()
        == "v1";
    if !profile_registry_migration_done {
        match run_profile_registry_migration(conn) {
            Ok(stats) => {
                tracing::info!(
                    canonicalized = stats.canonicalized,
                    merged_via_alias = stats.merged_via_alias,
                    invalid_machine_deleted = stats.invalid_machine_deleted,
                    "FU#2 Phase 7: Profile Slot Registry migration complete"
                );
                if let Err(e) = conn.execute(
                    "INSERT OR REPLACE INTO skill_state (skill_name, state_key, state_value, updated_at) \
                     VALUES ('system', 'profile_registry_migration_done', 'v1', datetime('now'))",
                    [],
                ) {
                    tracing::warn!(error = %e, "Failed to mark profile_registry_migration_done");
                }
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "FU#2 Phase 7 profile_registry migration FAILED — will retry on next startup"
                );
            }
        }
    }

    // ── Step 1: heartbeat_config + config_audit_log ──────────────────────
    // Ships the data layer for the heartbeat autonomy redesign
    // (docs/heartbeat-autonomy-design.md §7, §13). Idempotent: CREATE TABLE
    // IF NOT EXISTS, and the one-shot folding migration checks whether the
    // new key already exists before writing.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS config_audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at TEXT NOT NULL,
            actor TEXT NOT NULL,
            old_json TEXT,
            new_json TEXT NOT NULL,
            reason TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_config_audit_at ON config_audit_log(at);",
    )
    .context("creating config_audit_log table")?;

    if let Err(e) = run_heartbeat_config_fold_migration(conn) {
        // Non-fatal: defaults remain in RAM, migration will retry on next boot.
        tracing::error!(
            error = %e,
            "heartbeat_config fold migration FAILED — will retry on next startup"
        );
    }

    // ── Step 3: observed_events queue ────────────────────────────────────
    // Lightweight queue where probes record observations instead of each
    // triggering their own LLM call. The scheduled heartbeat tick drains
    // unconsumed rows and folds them into the wake-up prompt as a
    // "since last tick" block. Design §2.1, §15.2 Step 3.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS observed_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at TEXT NOT NULL,
            source TEXT NOT NULL,
            severity TEXT NOT NULL,
            summary TEXT NOT NULL,
            payload_json TEXT,
            consumed_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_observed_events_unconsumed
            ON observed_events(consumed_at, id);",
    )
    .context("creating observed_events table")?;

    // ── Subcall primitive: subcall_log + subcall_trace ───────────────────
    // Per docs/subcall-primitive-design.md §7.4 and §7.5. `subcall_log` holds
    // per-invocation metadata and governance flags (status, budgets used,
    // parent session / parent subcall linkage, trace_available). `subcall_trace`
    // holds the raw prompt + message + tool-output payload used for
    // reconstruction or debugging; rows are evicted LRU via `accessed_at`
    // and CASCADE-deleted when their parent `subcall_log` row is removed.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS subcall_log (
            id TEXT PRIMARY KEY,
            parent_session_id TEXT NOT NULL,
            origin TEXT NOT NULL,
            task_preview TEXT,
            allowed_tools TEXT,
            status TEXT NOT NULL,
            rounds_used INTEGER,
            tool_calls_made INTEGER,
            tokens_used INTEGER,
            duration_ms INTEGER,
            created_at TEXT NOT NULL,
            parent_subcall_id TEXT,
            trace_available INTEGER NOT NULL DEFAULT 1,
            summary TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_subcall_parent
            ON subcall_log(parent_session_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_subcall_status
            ON subcall_log(status, created_at DESC);

        CREATE TABLE IF NOT EXISTS subcall_trace (
            subcall_id TEXT PRIMARY KEY
                REFERENCES subcall_log(id) ON DELETE CASCADE,
            system_prompt TEXT,
            messages_json TEXT NOT NULL,
            tool_outputs_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            accessed_at TEXT NOT NULL,
            size_bytes INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_subcall_trace_accessed
            ON subcall_trace(accessed_at);
        "#,
    )
    .context("creating subcall_log and subcall_trace tables")?;

    // Additive migration: `summary` column was added after the initial DDL
    // (Task 1.7) to hold the LLM's free-text summary. `CREATE TABLE IF NOT
    // EXISTS` above won't add new columns on existing databases per CLAUDE.md
    // anti-pattern — the ALTER TABLE here handles upgrade from the pre-1.7
    // schema. Duplicate-column error on fresh DBs (where the CREATE already
    // included the column) is swallowed.
    if let Err(e) = conn.execute_batch("ALTER TABLE subcall_log ADD COLUMN summary TEXT;") {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            tracing::warn!(error = %msg, "Migration failed: subcall_log.summary");
        }
    }

    crate::heirloom::schema::ensure_tables(conn).context("heirloom::schema::ensure_tables")?;

    // ── chat_topics: dropped (Telegram forum/group routing removed) ──────
    // Forum/linked-group routing was removed; every Telegram send now targets
    // the owner's admin DM. Drop the now-unused table (and its index) so a
    // stale `/link_forum` row can never influence routing. `IF EXISTS` makes
    // this idempotent and safe on both fresh DBs (table never existed) and
    // upgraded DBs (drops the legacy table). The index is dropped before the
    // table, though `DROP TABLE` would cascade to it anyway.
    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_chat_topics_chat_id;
         DROP TABLE IF EXISTS chat_topics;",
    )
    .context("dropping legacy chat_topics table")?;

    // ── Re-home group-originated send targets to the owner's admin DM ────
    // Root fix for the persisted-chat_id→group send class (2026-07-03): rows
    // in `tasks` (scheduled reminders / suspended WebApp tasks / deferred
    // intents — all loaded from `tasks`) and `agent_manifests` (agent-swarm
    // batches) persist the chat_id captured at creation time. A row created
    // from the now-defunct linked forum carries that group's id, and every
    // consumer that later sends to it — direct sends AND `ctx.chat_id`-derived
    // tool sends inside a resumed orchestrator turn — would post into the raw
    // group. Telegram group/supergroup/channel ids are NEGATIVE; the owner's
    // admin chat id is a POSITIVE user id, so `chat_id < 0` targets exactly the
    // group-originated rows. Only the SEND-TARGET column is rewritten; each
    // row's `user_did` (memory/recall scope) is left untouched, so no L1/L2/L3
    // namespace is merged. Idempotent (after the first run no negative rows
    // remain) and env-gated (dev DBs with no ADMIN_CHAT_ID are left as-is; the
    // runtime send-clamp is the backstop there).
    if let Some(admin_chat_id) = std::env::var("ADMIN_CHAT_ID")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|id| *id > 0)
    {
        rehome_group_originated_chat_ids(conn, admin_chat_id);
    }

    // ── gmail_pubsub_watermark (Gmail Pub/Sub Phase 1) ───────────────────
    // Tracks the highest `historyId` processed per Gmail account so each
    // push notification only fetches the delta since the previous one.
    // `last_watch_at` is populated when the account's `users.watch` call
    // succeeds (Phase 2) — Phase 1 writes it on every push update as a
    // proxy freshness indicator.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS gmail_pubsub_watermark (
            account TEXT PRIMARY KEY,
            history_id INTEGER NOT NULL,
            last_watch_at TEXT NOT NULL
        );",
    )
    .context("creating gmail_pubsub_watermark table")?;

    // ── imap_idle_watermark (H6 — IMAP IDLE persistence) ─────────────────
    // Persists the highest UID notified about for each (account, mailbox)
    // pair so a service restart does not drop emails on cold-start fence.
    // `last_uidvalidity` is captured at the same time so a UIDVALIDITY
    // change on reconnect (RFC 3501 — mailbox renumbering) can be detected
    // and trigger replay (H7) instead of silently fast-forwarding.
    //
    // Deliberately separate from `gmail_pubsub_watermark`: Pub/Sub tracks
    // an opaque `historyId`, IMAP tracks `(uid, uidvalidity)` per mailbox,
    // and the same account can be observed by both surfaces independently.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS imap_idle_watermark (
            account TEXT NOT NULL,
            mailbox TEXT NOT NULL,
            last_uid INTEGER NOT NULL,
            last_uidvalidity INTEGER NOT NULL,
            last_seen_at TEXT NOT NULL,
            PRIMARY KEY (account, mailbox)
        );",
    )
    .context("creating imap_idle_watermark table")?;

    // ── external event spine (AssistantEvent Phase 1) ────────────────────
    // Raw ingestion audit trail plus normalized assistant-facing event rows.
    // Deliberately separate from the existing observed_events queue:
    // - observed_events is an ephemeral heartbeat input queue
    // - raw_external_events / assistant_events are durable replayable state
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS raw_external_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            observed_at TEXT NOT NULL,
            user_did TEXT NOT NULL,
            source TEXT NOT NULL,
            source_account_id TEXT,
            source_event_id TEXT,
            dedup_key TEXT NOT NULL UNIQUE,
            payload_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_raw_external_events_user_observed
            ON raw_external_events(user_did, observed_at DESC);
        CREATE INDEX IF NOT EXISTS idx_raw_external_events_source_account
            ON raw_external_events(source, source_account_id, observed_at DESC);

        CREATE TABLE IF NOT EXISTS assistant_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            raw_event_id INTEGER REFERENCES raw_external_events(id) ON DELETE SET NULL,
            observed_at TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            user_did TEXT NOT NULL,
            source TEXT NOT NULL,
            kind TEXT NOT NULL,
            account_id TEXT,
            title TEXT NOT NULL,
            summary TEXT NOT NULL,
            thread_key TEXT,
            urgency_hint TEXT NOT NULL,
            actionability_hint TEXT NOT NULL,
            decision TEXT NOT NULL,
            dedup_key TEXT NOT NULL UNIQUE,
            entities_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_assistant_events_user_observed
            ON assistant_events(user_did, observed_at DESC);
        CREATE INDEX IF NOT EXISTS idx_assistant_events_user_decision
            ON assistant_events(user_did, decision, observed_at DESC);
        CREATE INDEX IF NOT EXISTS idx_assistant_events_thread
            ON assistant_events(user_did, thread_key, observed_at DESC);

        CREATE TABLE IF NOT EXISTS assistant_event_cursor (
            user_did TEXT NOT NULL,
            consumer TEXT NOT NULL,
            last_event_id INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (user_did, consumer)
        );

        CREATE TABLE IF NOT EXISTS assistant_work_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            user_did TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_ref TEXT,
            anchor_kind TEXT,
            anchor_ref TEXT,
            assistant_event_id INTEGER REFERENCES assistant_events(id) ON DELETE SET NULL,
            goal TEXT NOT NULL,
            mode TEXT NOT NULL,
            priority TEXT NOT NULL,
            status TEXT NOT NULL,
            delivery_policy TEXT NOT NULL,
            caller_context TEXT NOT NULL,
            dedup_key TEXT NOT NULL UNIQUE,
            budget_json TEXT NOT NULL,
            context_json TEXT NOT NULL,
            result_json TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_assistant_work_items_user_status_priority
            ON assistant_work_items(user_did, status, priority, id);
        CREATE INDEX IF NOT EXISTS idx_assistant_work_items_assistant_event
            ON assistant_work_items(assistant_event_id);
        ",
    )
    .context("creating assistant event spine tables")?;

    for stmt in [
        "ALTER TABLE assistant_work_items ADD COLUMN anchor_kind TEXT;",
        "ALTER TABLE assistant_work_items ADD COLUMN anchor_ref TEXT;",
    ] {
        if let Err(e) = conn.execute_batch(stmt) {
            let msg = e.to_string().to_lowercase();
            if !msg.contains("duplicate column") {
                return Err(e).context("adding assistant_work_items anchor columns");
            }
        }
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_assistant_work_items_anchor
             ON assistant_work_items(user_did, anchor_kind, anchor_ref, status, id);",
    )
    .context("creating assistant_work_items anchor index")?;

    // ── action_ledger (P1 Action Ledger Phase 1) ────────────────────────
    // Canonical receipts for user-impacting external side effects. Phase 1
    // only creates schema/API; send paths integrate in later design phases.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS action_ledger (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            action_id TEXT NOT NULL UNIQUE,
            action_kind TEXT NOT NULL,
            status TEXT NOT NULL,
            requested_by TEXT,
            chat_id INTEGER,
            created_at TEXT NOT NULL,
            proposed_at TEXT,
            approved_at TEXT,
            executing_at TEXT,
            executed_at TEXT,
            failed_at TEXT,
            cancelled_at TEXT,
            input_summary TEXT,
            user_visible_summary TEXT,
            external_receipt TEXT,
            external_message_id TEXT,
            external_event_id TEXT,
            tool_call_id TEXT,
            assistant_event_id INTEGER REFERENCES assistant_events(id) ON DELETE SET NULL,
            provider_request_id TEXT,
            idempotency_key TEXT,
            error_kind TEXT,
            error_message TEXT,
            retry_policy TEXT,
            trace_id TEXT,
            run_id TEXT,
            mandate_id TEXT,
            source_event_id TEXT,
            metadata_json TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_action_ledger_created_at
            ON action_ledger(created_at);
        CREATE INDEX IF NOT EXISTS idx_action_ledger_status
            ON action_ledger(status);
        CREATE INDEX IF NOT EXISTS idx_action_ledger_kind
            ON action_ledger(action_kind);
        CREATE INDEX IF NOT EXISTS idx_action_ledger_idempotency
            ON action_ledger(idempotency_key);",
    )
    .context("creating action_ledger table")?;

    // ── user_mandates (P1 WebApp mandate lifecycle Phase 2) ─────────────
    // Explicit, revocable owner mandates. These rows authorize only the
    // bounded behaviors represented by the row; they do not bypass
    // DataAccessGate, MFA, Action Ledger, or notification privacy policy.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS user_mandates (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            mandate_id TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            description TEXT NOT NULL,
            owner_principal TEXT NOT NULL,
            status TEXT NOT NULL,
            scope_kind TEXT NOT NULL,
            source_filters_json TEXT NOT NULL,
            allowed_reads_json TEXT NOT NULL,
            allowed_llm_uses_json TEXT NOT NULL,
            allowed_actions_json TEXT NOT NULL,
            must_ask_json TEXT NOT NULL,
            notification_policy_json TEXT NOT NULL,
            budget_policy_json TEXT NOT NULL,
            frequency_policy_json TEXT NOT NULL,
            expiry TEXT,
            review_after TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            created_from TEXT NOT NULL,
            source_event_id TEXT,
            trace_id TEXT,
            run_id TEXT,
            schema_version INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS idx_user_mandates_owner_status
            ON user_mandates(owner_principal, status, id);
        CREATE INDEX IF NOT EXISTS idx_user_mandates_scope
            ON user_mandates(owner_principal, scope_kind, status);
        CREATE TABLE IF NOT EXISTS user_mandate_audit (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            mandate_id TEXT NOT NULL,
            owner_principal TEXT NOT NULL,
            event_kind TEXT NOT NULL,
            actor_principal TEXT NOT NULL,
            created_at TEXT NOT NULL,
            status_from TEXT,
            status_to TEXT,
            summary TEXT NOT NULL,
            metadata_json TEXT,
            source_event_id TEXT,
            trace_id TEXT,
            run_id TEXT,
            schema_version INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS idx_user_mandate_audit_mandate
            ON user_mandate_audit(owner_principal, mandate_id, id);
        CREATE INDEX IF NOT EXISTS idx_user_mandate_audit_event
            ON user_mandate_audit(event_kind, created_at);
        CREATE TABLE IF NOT EXISTS user_mandate_notifications (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            mandate_id TEXT NOT NULL,
            owner_principal TEXT NOT NULL,
            source_event_id TEXT NOT NULL,
            idempotency_key TEXT NOT NULL UNIQUE,
            status TEXT NOT NULL,
            notification_class TEXT NOT NULL,
            privacy_level TEXT NOT NULL,
            route_reason TEXT NOT NULL,
            route_chat_id INTEGER NOT NULL,
            route_thread_id INTEGER,
            title TEXT NOT NULL,
            body TEXT NOT NULL,
            detail_url TEXT,
            redacted INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            trace_id TEXT,
            run_id TEXT,
            schema_version INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS idx_user_mandate_notifications_owner_mandate
            ON user_mandate_notifications(owner_principal, mandate_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_user_mandate_notifications_source
            ON user_mandate_notifications(owner_principal, source_event_id);",
    )
    .context("creating user_mandates tables")?;

    // Historical mail local-store tables stay inert. Keep creating them so
    // existing DBs and migration tests still see the same table names.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS mail_messages (
            id TEXT PRIMARY KEY,
            account_id TEXT NOT NULL,
            provider_msg_id TEXT NOT NULL,
            conversation_key TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS mail_sync_state (
            account_id TEXT PRIMARY KEY
        );",
    )
    .context("creating inert mail_store tables")?;

    // Migration: add anchor_kind/anchor_ref to l3_archive (2026-06-30 chat de-noise)
    // NULL for rows predating this migration — means "no anchor" which is the correct
    // fail-safe (is_routine_assistant_event_anchor(None, None) = false).
    for stmt in &[
        "ALTER TABLE l3_archive ADD COLUMN anchor_kind TEXT",
        "ALTER TABLE l3_archive ADD COLUMN anchor_ref TEXT",
    ] {
        if let Err(e) = conn.execute_batch(stmt) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                tracing::warn!(error = %msg, "Migration failed: l3_archive anchor columns");
            }
        }
    }

    migrate_hindsight_partial_columns(conn)?;

    Ok(())
}

/// L2 provenance columns and ledger tables for partial-reference memory.
///
/// Adds `epistemic_kind`, `source_receipt`, `valid_until`, `session_id`,
/// `claim_key`, and `weakened` without rewriting existing fact text. History,
/// contradiction, and consolidation-link tables retain prior rows.
fn migrate_hindsight_partial_columns(conn: &Connection) -> anyhow::Result<()> {
    for stmt in [
        "ALTER TABLE l2_experience ADD COLUMN epistemic_kind TEXT NOT NULL DEFAULT 'fact'",
        "ALTER TABLE l2_experience ADD COLUMN source_receipt TEXT",
        "ALTER TABLE l2_experience ADD COLUMN valid_until TEXT",
        "ALTER TABLE l2_experience ADD COLUMN session_id TEXT",
        "ALTER TABLE l2_experience ADD COLUMN claim_key TEXT",
        "ALTER TABLE l2_experience ADD COLUMN weakened INTEGER NOT NULL DEFAULT 0",
    ] {
        if let Err(e) = conn.execute_batch(stmt) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                tracing::warn!(error = %msg, statement = stmt, "l2 provenance column migration");
            }
        }
    }
    conn.execute(
        "UPDATE l2_experience SET epistemic_kind = COALESCE(NULLIF(epistemic_kind, ''), 'fact')",
        [],
    )
    .unwrap_or(0);
    conn.execute(
        "UPDATE l2_experience SET weakened = COALESCE(weakened, 0)",
        [],
    )
    .unwrap_or(0);
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS l2_fact_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            fact_id INTEGER NOT NULL,
            agent_did TEXT NOT NULL,
            previous_summary TEXT NOT NULL,
            previous_confidence REAL,
            previous_epistemic_kind TEXT,
            reason TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_l2_fact_history_fact ON l2_fact_history(fact_id);
        CREATE TABLE IF NOT EXISTS l2_contradiction (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_did TEXT NOT NULL,
            kept_fact_id INTEGER NOT NULL,
            incoming_fact_id INTEGER NOT NULL,
            status TEXT NOT NULL,
            note TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_l2_contradiction_did ON l2_contradiction(agent_did, status);
        CREATE TABLE IF NOT EXISTS l2_consolidation_link (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_did TEXT NOT NULL,
            observation_id INTEGER NOT NULL,
            consolidated_fact_id INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(observation_id, consolidated_fact_id)
        );
        CREATE INDEX IF NOT EXISTS idx_l2_claim ON l2_experience(agent_did, topic, claim_key);",
    )
    .context("creating l2 provenance ledger tables")?;
    Ok(())
}

/// One-shot migration: if `heartbeat_config` is not yet persisted AND legacy
/// `notification_quiet_start` / `notification_quiet_end` are, fold them into a
/// new `heartbeat_config` row, audit the change, and delete the legacy keys.
///
/// Idempotent: on second run the `heartbeat_config` row exists so the whole
/// body is a no-op.
/// Rewrite group-originated (negative) send-target chat ids to the owner's
/// admin DM in the tables whose `chat_id` becomes a Telegram send target
/// (`tasks`, `agent_manifests`). Pure over an explicit `admin_chat_id` so it is
/// testable without env races; the migration passes the env-resolved value.
/// Only the send-target column is touched — `user_did` (memory scope) is left
/// intact. Idempotent (no negative rows remain after the first pass) and
/// tolerant of a missing table on partial DBs.
fn rehome_group_originated_chat_ids(conn: &Connection, admin_chat_id: i64) {
    for table in ["tasks", "agent_manifests"] {
        let sql = format!("UPDATE {table} SET chat_id = ?1 WHERE chat_id < 0");
        match conn.execute(&sql, rusqlite::params![admin_chat_id]) {
            Ok(n) if n > 0 => tracing::info!(
                table,
                admin_chat_id,
                rows = n,
                "[migration] re-homed group-originated chat_id rows to admin DM"
            ),
            Ok(_) => {}
            Err(e) => {
                let msg = e.to_string();
                if !msg.contains("no such table") {
                    tracing::warn!(table, error = %msg, "chat_id re-home migration failed");
                }
            }
        }
    }
}

fn run_heartbeat_config_fold_migration(conn: &Connection) -> anyhow::Result<()> {
    // 1. Short-circuit if already migrated (new key present).
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM user_settings WHERE key = 'heartbeat_config'",
            [],
            |r| r.get(0),
        )
        .ok();
    if existing.is_some() {
        return Ok(());
    }

    // 2. Read legacy keys (may not exist — that's fine, we fall back to
    //    defaults for each slot that's missing).
    let legacy_start: Option<String> = conn
        .query_row(
            "SELECT value FROM user_settings WHERE key = 'notification_quiet_start'",
            [],
            |r| r.get(0),
        )
        .ok();
    let legacy_end: Option<String> = conn
        .query_row(
            "SELECT value FROM user_settings WHERE key = 'notification_quiet_end'",
            [],
            |r| r.get(0),
        )
        .ok();

    // If neither legacy key is present there's nothing to fold — leave the
    // new key unset so the default from RuntimeSettings applies in RAM.
    if legacy_start.is_none() && legacy_end.is_none() {
        return Ok(());
    }

    // 3. Build a fresh HeartbeatConfig from defaults, then overwrite
    //    quiet_hours from the legacy values where they resolve to a real hour.
    let mut cfg = crate::heartbeat_config::HeartbeatConfig::default();

    let parse_hour = |raw: &Option<String>| -> Option<u8> {
        let s = raw.as_ref()?;
        // Legacy values are JSON-encoded (store_setting uses value.to_string()).
        // Try both a JSON number and a bare number/string.
        let v: serde_json::Value = serde_json::from_str(s).ok()?;
        let n = v.as_u64()?;
        if n > 23 { None } else { Some(n as u8) }
    };

    let start_h = parse_hour(&legacy_start);
    let end_h = parse_hour(&legacy_end);
    if let (Some(s), Some(e)) = (start_h, end_h) {
        cfg.quiet_hours = crate::heartbeat_config::QuietHours {
            start: format!("{:02}:00", s),
            end: format!("{:02}:00", e),
        };
    } else if let Some(s) = start_h {
        cfg.quiet_hours.start = format!("{:02}:00", s);
    } else if let Some(e) = end_h {
        cfg.quiet_hours.end = format!("{:02}:00", e);
    }
    // If both legacy values were 255 (disabled) neither parses as a real hour
    // → parse_hour returns None for both, and we keep the spec defaults (02:00-07:00).

    let new_json = serde_json::to_string(&cfg).context("serializing folded heartbeat_config")?;
    let now = chrono::Utc::now().to_rfc3339();

    // 4. Insert the new key + audit row + delete legacy keys atomically.
    conn.execute_batch("SAVEPOINT heartbeat_config_fold")
        .context("heartbeat_config fold: starting savepoint")?;

    let tx_result: anyhow::Result<()> = (|| {
        conn.execute(
            "INSERT OR REPLACE INTO user_settings (key, value, updated_at) VALUES ('heartbeat_config', ?1, ?2)",
            rusqlite::params![new_json, now],
        )
        .context("heartbeat_config fold: writing new key")?;

        let reason = format!(
            "folded legacy keys notification_quiet_start={:?}, notification_quiet_end={:?}",
            legacy_start.as_deref(),
            legacy_end.as_deref()
        );
        conn.execute(
            "INSERT INTO config_audit_log (at, actor, old_json, new_json, reason) \
             VALUES (?1, 'migration', NULL, ?2, ?3)",
            rusqlite::params![now, new_json, reason],
        )
        .context("heartbeat_config fold: writing audit row")?;

        conn.execute(
            "DELETE FROM user_settings WHERE key IN ('notification_quiet_start','notification_quiet_end')",
            [],
        )
        .context("heartbeat_config fold: deleting legacy keys")?;
        Ok(())
    })();

    match tx_result {
        Ok(()) => {
            conn.execute_batch("RELEASE heartbeat_config_fold")
                .context("heartbeat_config fold: release savepoint")?;
            tracing::info!(
                start = ?legacy_start,
                end = ?legacy_end,
                "heartbeat_config fold migration complete — legacy quiet-hour keys folded in"
            );
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO heartbeat_config_fold");
            let _ = conn.execute_batch("RELEASE heartbeat_config_fold");
            Err(e)
        }
    }
}

/// Statistics from the FU#2 Phase 7 Profile Slot Registry migration.
#[derive(Debug, Default)]
pub(crate) struct ProfileRegistryMigrationStats {
    /// Rows whose (category, key) was rewritten from a legacy alias to canonical.
    pub canonicalized: usize,
    /// Legacy rows deleted because a canonical row already owned the slot.
    pub merged_via_alias: usize,
    /// `source="auto"` rows deleted because their value failed the registry validator.
    pub invalid_machine_deleted: usize,
}

/// FU#2 Phase 7 one-time data migration. Runs inside a SAVEPOINT.
///
/// 1. For every user_profile row whose (category, key) is a LEGACY alias,
///    resolve to canonical. If the canonical slot is already populated for
///    this DID, delete the legacy row (canonical wins). Otherwise rewrite
///    the row in place to the canonical pair.
/// 2. For every `source="auto"` row, validate the current value against the
///    registry. If validation fails, DELETE the row. Sweeps the 4 known-bad
///    prod rows (id=12/13/15/20) and any other legacy rot.
/// 3. `source="manual"` rows are NEVER deleted — trusted.
pub(crate) fn run_profile_registry_migration(
    conn: &Connection,
) -> anyhow::Result<ProfileRegistryMigrationStats> {
    let savepoint_name = "profile_registry_migration";
    conn.execute(&format!("SAVEPOINT {}", savepoint_name), [])?;

    let result = (|| -> anyhow::Result<ProfileRegistryMigrationStats> {
        let mut stats = ProfileRegistryMigrationStats::default();

        // Load all rows once so we can reason about (did, canonical_cat, canonical_key)
        // conflicts before issuing writes.
        #[derive(Clone)]
        struct Row {
            id: i64,
            agent_did: String,
            category: String,
            key: String,
            value: String,
            source: String,
            updated_at: String,
        }
        let rows: Vec<Row> = {
            let mut stmt = conn.prepare(
                "SELECT id, agent_did, category, key, value, source, updated_at FROM user_profile",
            )?;
            let iter = stmt.query_map([], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    agent_did: r.get(1)?,
                    category: r.get(2)?,
                    key: r.get(3)?,
                    value: r.get(4)?,
                    source: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            })?;
            // Propagate decode errors instead of silently dropping rows —
            // a corrupt row must not become invisible to the migration.
            let mut out = Vec::new();
            for r in iter {
                match r {
                    Ok(row) => out.push(row),
                    Err(e) => {
                        tracing::warn!(error = %e, "Phase 7: skipped undecodable user_profile row");
                    }
                }
            }
            out
        };

        // Index canonical rows by (did, cat, key) for collision checks.
        use std::collections::HashMap;
        let mut canonical_owner: HashMap<(String, String, String), Row> = HashMap::new();
        for row in &rows {
            if crate::profile_registry::lookup(&row.category, &row.key).is_some() {
                canonical_owner.insert(
                    (row.agent_did.clone(), row.category.clone(), row.key.clone()),
                    row.clone(),
                );
            }
        }

        // Pass 1: canonicalize legacy rows.
        for row in &rows {
            // Already canonical — skip.
            if crate::profile_registry::lookup(&row.category, &row.key).is_some() {
                continue;
            }
            let Some((canon_cat, canon_key)) =
                crate::profile_registry::resolve_alias(&row.category, &row.key)
            else {
                // Unknown slot that's not even an alias — delete any
                // non-manual row (auto + migration + any other machine
                // source). Manual rows under unknown slots survive for
                // user review.
                if row.source != "manual" {
                    conn.execute("DELETE FROM user_profile WHERE id = ?1", [row.id])?;
                    stats.invalid_machine_deleted += 1;
                }
                continue;
            };

            let key = (
                row.agent_did.clone(),
                canon_cat.to_string(),
                canon_key.to_string(),
            );
            if let Some(owner) = canonical_owner.get(&key).cloned() {
                // Canonical row already exists for this DID+slot. Compare
                // timestamps via parse_timestamp_dual_fmt so mixed prod formats
                // (RFC3339 and SQLite `datetime('now')`) order correctly.
                let owner_ts = crate::utils::parse_timestamp_dual_fmt(&owner.updated_at);
                let legacy_ts = crate::utils::parse_timestamp_dual_fmt(&row.updated_at);
                let owner_wins = match (owner_ts, legacy_ts) {
                    (Some(a), Some(b)) => a >= b,
                    // If either side is unparseable, fall back to the legacy
                    // winner so we don't mistakenly keep a corrupted-ts owner.
                    _ => false,
                };
                if owner_wins {
                    // Keep canonical, drop legacy.
                    conn.execute("DELETE FROM user_profile WHERE id = ?1", [row.id])?;
                } else {
                    // Legacy is newer — replace canonical row's value in place.
                    conn.execute(
                        "UPDATE user_profile SET value = ?1, source = ?2, updated_at = ?3 \
                         WHERE id = ?4",
                        rusqlite::params![row.value, row.source, row.updated_at, owner.id],
                    )?;
                    conn.execute("DELETE FROM user_profile WHERE id = ?1", [row.id])?;
                    // Keep the in-memory index in sync so a third row aliasing
                    // to this same canonical compares against the current winner.
                    let mut replaced = owner.clone();
                    replaced.value = row.value.clone();
                    replaced.source = row.source.clone();
                    replaced.updated_at = row.updated_at.clone();
                    canonical_owner.insert(key.clone(), replaced);
                }
                stats.merged_via_alias += 1;
            } else {
                // Rewrite in place to canonical.
                conn.execute(
                    "UPDATE user_profile SET category = ?1, key = ?2, updated_at = datetime('now') \
                     WHERE id = ?3",
                    rusqlite::params![canon_cat, canon_key, row.id],
                )?;
                // Re-index for Pass 2.
                let mut rewritten = row.clone();
                rewritten.category = canon_cat.to_string();
                rewritten.key = canon_key.to_string();
                canonical_owner.insert(key, rewritten);
                stats.canonicalized += 1;
            }
        }

        // Pass 2: sweep auto rows with values that fail the validator.
        // Re-read from DB to get current canonical state (post Pass 1).
        let current: Vec<(i64, String, String, String, String)> = {
            let mut stmt = conn.prepare(
                // Sweep every machine-origin row (auto, migration, anything
                // except manual). Manual = user-confirmed, never touched.
                "SELECT id, category, key, value, source FROM user_profile WHERE source != 'manual'",
            )?;
            let iter = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?;
            let mut out = Vec::new();
            for r in iter {
                match r {
                    Ok(row) => out.push(row),
                    Err(e) => {
                        tracing::warn!(error = %e, "Phase 7 Pass 2: skipped undecodable row");
                    }
                }
            }
            out
        };
        for (id, cat, key, value, _src) in current {
            if crate::profile_registry::validate(&cat, &key, &value).is_err() {
                conn.execute("DELETE FROM user_profile WHERE id = ?1", [id])?;
                stats.invalid_machine_deleted += 1;
            }
        }

        Ok(stats)
    })();

    match result {
        Ok(stats) => {
            conn.execute(&format!("RELEASE SAVEPOINT {}", savepoint_name), [])?;
            Ok(stats)
        }
        Err(e) => {
            let _ = conn.execute(&format!("ROLLBACK TO SAVEPOINT {}", savepoint_name), []);
            let _ = conn.execute(&format!("RELEASE SAVEPOINT {}", savepoint_name), []);
            Err(e)
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────
/// Historical `dev_task_drafts` / `dev_task_draft_events` schema.
///
/// The Aegis dev harness that wrote these tables was removed (2026-09-05); the
/// tables are kept so the two drafts that were ever created stay readable by
/// hand (`sqlite3`) and so a rollback to an older binary finds its schema
/// intact. No code reads or writes them any more, which is also why the old
/// `worker_ensured` backfill is gone: it existed only to keep historical rows
/// out of a WebApp queue that no longer exists. The idempotent `ALTER`s below
/// are still applied so a DB created by a pre-2026-07 binary ends up with the
/// same column set as a fresh one.
fn init_dev_task_draft_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS dev_task_drafts (
            id TEXT PRIMARY KEY,
            owner_did TEXT NOT NULL,
            source_channel TEXT NOT NULL,
            source_thread_ref TEXT NOT NULL,
            source_message_ref TEXT,
            title TEXT NOT NULL,
            objective TEXT NOT NULL,
            criteria_json TEXT NOT NULL,
            risk TEXT NOT NULL,
            risk_justification TEXT,
            evidence_expectations_json TEXT NOT NULL,
            surface TEXT NOT NULL,
            task_kind TEXT NOT NULL DEFAULT 'code_change',
            allowed_paths_json TEXT NOT NULL,
            forbidden_paths_json TEXT NOT NULL,
            clarification_state_json TEXT NOT NULL,
            status TEXT NOT NULL,
            idempotency_key TEXT NOT NULL,
            duplicate_of TEXT,
            duplicate_warning TEXT,
            approved_task_id TEXT,
            owner_edited INTEGER NOT NULL DEFAULT 0,
            worker_ensured INTEGER NOT NULL DEFAULT 0,
            created_at_unix INTEGER NOT NULL,
            updated_at_unix INTEGER NOT NULL,
            expires_at_unix INTEGER NOT NULL,
            approved_at_unix INTEGER,
            discarded_at_unix INTEGER,
            rejected_at_unix INTEGER
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_dev_task_drafts_active_thread
            ON dev_task_drafts(owner_did, source_thread_ref)
            WHERE status IN ('clarifying', 'pending');
        CREATE INDEX IF NOT EXISTS idx_dev_task_drafts_owner_active
            ON dev_task_drafts(owner_did, status, updated_at_unix DESC);
        CREATE INDEX IF NOT EXISTS idx_dev_task_drafts_idempotency
            ON dev_task_drafts(owner_did, idempotency_key, status);
        CREATE INDEX IF NOT EXISTS idx_dev_task_drafts_expiry
            ON dev_task_drafts(status, expires_at_unix);

        CREATE TABLE IF NOT EXISTS dev_task_draft_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            draft_id TEXT,
            owner_did TEXT NOT NULL,
            source_thread_ref TEXT NOT NULL,
            event_kind TEXT NOT NULL,
            actor TEXT NOT NULL,
            detail_json TEXT NOT NULL DEFAULT '{}',
            created_at_unix INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_dev_task_draft_events_rate
            ON dev_task_draft_events(owner_did, source_thread_ref, created_at_unix DESC);
        CREATE INDEX IF NOT EXISTS idx_dev_task_draft_events_draft
            ON dev_task_draft_events(draft_id, created_at_unix DESC);",
    )?;
    // `CREATE TABLE IF NOT EXISTS` is a no-op on a DB that predates these three
    // columns, so add them here. The duplicate-column error on an already
    // migrated DB is expected and ignored.
    for column in [
        "task_kind TEXT NOT NULL DEFAULT 'code_change'",
        "owner_edited INTEGER NOT NULL DEFAULT 0",
        "worker_ensured INTEGER NOT NULL DEFAULT 0",
    ] {
        let _ = conn.execute(
            &format!("ALTER TABLE dev_task_drafts ADD COLUMN {column}"),
            [],
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();
        conn
    }

    #[test]
    fn migrations_clear_stale_final_approval_stage_errors() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO skill_proposals
             (skill_name, requested_by, status, stage_error, user_request, generated_manifest, compiled_wasm)
             VALUES (?1, ?2, 'pending_final_approval', ?3, ?4, ?5, ?6)",
            rusqlite::params![
                "server_health_aggregator",
                "did:test:user",
                "v2 compile+test failed: old cron error",
                "aggregate server health hourly",
                r#"{"skill_name":"server_health_aggregator"}"#,
                vec![0u8, 97, 115, 109],
            ],
        )
        .unwrap();

        run_all_migrations(&conn).unwrap();

        let stage_error: Option<String> = conn
            .query_row(
                "SELECT stage_error FROM skill_proposals WHERE skill_name = ?1",
                ["server_health_aggregator"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stage_error, None);
    }

    #[test]
    fn migrations_demote_final_approval_without_activation_artifacts() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO skill_proposals
             (skill_name, requested_by, status, user_request)
             VALUES (?1, ?2, 'pending_final_approval', ?3)",
            rusqlite::params![
                "server_health_aggregator",
                "did:test:user",
                "aggregate server health hourly",
            ],
        )
        .unwrap();

        run_all_migrations(&conn).unwrap();

        let (status, stage_error): (String, Option<String>) = conn
            .query_row(
                "SELECT status, stage_error FROM skill_proposals WHERE skill_name = ?1",
                ["server_health_aggregator"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "implement_failed");
        assert!(
            stage_error
                .as_deref()
                .unwrap_or_default()
                .contains("compiled wasm or manifest is missing")
        );
    }

    #[test]
    fn migrations_keep_final_approval_composite_dag_without_wasm_artifacts() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO skill_proposals
             (skill_name, requested_by, status, user_request, execution_dag_json)
             VALUES (?1, ?2, 'pending_final_approval', ?3, ?4)",
            rusqlite::params![
                "server_health_aggregator",
                "did:test:user",
                "aggregate server health hourly",
                r#"{"type":"static_dag","name":"server_health_aggregator","steps":[{"id":"check","capability":{"type":"wasm_skill","name":"server_monitor"},"input_mapping":[],"condition":null}],"final_output":"check"}"#,
            ],
        )
        .unwrap();

        run_all_migrations(&conn).unwrap();

        let (status, stage_error): (String, Option<String>) = conn
            .query_row(
                "SELECT status, stage_error FROM skill_proposals WHERE skill_name = ?1",
                ["server_health_aggregator"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "pending_final_approval");
        assert_eq!(stage_error, None);
    }

    #[test]
    fn migrations_repair_server_health_aggregator_missing_atom_dag() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO skill_proposals
             (skill_name, requested_by, status, user_request, execution_dag_json)
             VALUES (?1, ?2, 'active', ?3, ?4)",
            rusqlite::params![
                "server_health_aggregator",
                "did:test:user",
                "aggregate server health hourly",
                r#"{"type":"static_dag","name":"proactive_server_health_aggregator","steps":[{"id":"fetch_server_health","capability":{"type":"wasm_skill","name":"server_monitor"},"input_mapping":[],"condition":null},{"id":"append_health_history","capability":{"type":"wasm_skill","name":"append_health_history"},"input_mapping":[],"condition":null},{"id":"send_telegram_notification","capability":{"type":"wasm_skill","name":"notify_user"},"input_mapping":[],"condition":null}],"final_output":"send_telegram_notification"}"#,
            ],
        )
        .unwrap();

        run_all_migrations(&conn).unwrap();

        let dag_json: String = conn
            .query_row(
                "SELECT execution_dag_json FROM skill_proposals WHERE skill_name = ?1",
                ["server_health_aggregator"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dag_json, SERVER_HEALTH_AGGREGATOR_DAG_JSON);
    }

    #[test]
    fn migrations_repair_generate_summary_non_probe_dag() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO skill_proposals
             (skill_name, requested_by, status, user_request, execution_dag_json)
             VALUES (?1, ?2, 'active', ?3, ?4)",
            rusqlite::params![
                "generate_summary",
                "did:test:user",
                "summarize unread Gmail",
                r#"{"type":"static_dag","name":"gmail_hourly_inbox_summary","steps":[{"id":"fetch_unread_emails","capability":{"type":"wasm_skill","name":"gmail_reader"},"input_mapping":[],"condition":null},{"id":"categorize_emails","capability":{"type":"wasm_skill","name":"email_reader"},"input_mapping":[],"condition":null},{"id":"notify_summary","capability":{"type":"wasm_skill","name":"gmail_reader"},"input_mapping":[],"condition":null}],"final_output":"notify_summary"}"#,
            ],
        )
        .unwrap();

        run_all_migrations(&conn).unwrap();

        let dag_json: String = conn
            .query_row(
                "SELECT execution_dag_json FROM skill_proposals WHERE skill_name = ?1",
                ["generate_summary"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dag_json, GENERATE_SUMMARY_PROBE_DAG_JSON);
    }

    #[test]
    fn migrations_repair_generate_summary_pdt_cron_schedule() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO skill_proposals
             (skill_name, requested_by, status, user_request, capability_graph_json, execution_dag_json)
             VALUES (?1, ?2, 'active', ?3, ?4, ?5)",
            rusqlite::params![
                "generate_summary",
                "did:test:user",
                "summarize unread Gmail Tue-Fri 9am PDT",
                r#"{"request":"Create Gmail summary","cron_schedule":"0 0 9 * * 2-5","nodes":[]}"#,
                GENERATE_SUMMARY_PROBE_DAG_JSON,
            ],
        )
        .unwrap();

        run_all_migrations(&conn).unwrap();

        let graph_json: String = conn
            .query_row(
                "SELECT capability_graph_json FROM skill_proposals WHERE skill_name = ?1",
                ["generate_summary"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(graph_json.contains(GENERATE_SUMMARY_PDT_UTC_CRON_JSON));
        assert!(!graph_json.contains(GENERATE_SUMMARY_BAD_UTC_CRON_JSON));
        let dag_json: String = conn
            .query_row(
                "SELECT execution_dag_json FROM skill_proposals WHERE skill_name = ?1",
                ["generate_summary"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dag_json, GENERATE_SUMMARY_PROBE_DAG_JSON);
    }

    #[test]
    fn subcall_tables_created_by_migrations() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        run_all_migrations(&conn).unwrap();

        // subcall_log
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='subcall_log'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "subcall_log table should exist");

        // subcall_trace
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='subcall_trace'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "subcall_trace table should exist");

        // idx_subcall_parent
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_subcall_parent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "idx_subcall_parent index should exist");
    }

    /// Migration must DROP the legacy `chat_topics` table starting from an OLD
    /// DB that still has it (forum/group routing removal). A fresh-init test
    /// would miss ordering/idempotency bugs, so this seeds the pre-existing
    /// schema (table + index + a row) first, then runs the full migration.
    #[test]
    fn chat_topics_table_dropped_from_existing_db() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // Recreate the exact pre-removal schema, then seed a stale linked row.
        conn.execute_batch(
            "CREATE TABLE chat_topics (
                chat_id INTEGER NOT NULL,
                topic_kind TEXT NOT NULL CHECK (topic_kind IN ('main','approval','notif','system')),
                thread_id INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (chat_id, topic_kind)
            );
            CREATE INDEX idx_chat_topics_chat_id ON chat_topics(chat_id);
            INSERT INTO chat_topics (chat_id, topic_kind, thread_id) VALUES (-1001, 'main', 11);",
        )
        .unwrap();
        // Precondition: the legacy table + index + row exist before migrating.
        let table_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='chat_topics'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(table_before, 1, "precondition: chat_topics exists");

        run_all_migrations(&conn).unwrap();

        let table_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='chat_topics'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            table_after, 0,
            "migration must DROP the legacy chat_topics table"
        );
        let index_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_chat_topics_chat_id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            index_after, 0,
            "migration must drop the chat_topics index with the table"
        );

        // Idempotent + clean on a DB that never had the table (fresh init):
        // running again must not error.
        run_all_migrations(&conn).unwrap();
    }

    #[test]
    fn rehome_migration_redirects_group_chat_ids_but_preserves_user_did() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (id INTEGER PRIMARY KEY, user_did TEXT NOT NULL, chat_id INTEGER NOT NULL);
             CREATE TABLE agent_manifests (agent_id TEXT PRIMARY KEY, chat_id INTEGER NOT NULL);
             INSERT INTO tasks (id, user_did, chat_id) VALUES
               (1, 'did:aegis:telegram:-1001000000001', -1001000000001),
               (2, 'did:aegis:telegram:555', 555);
             INSERT INTO agent_manifests (agent_id, chat_id) VALUES
               ('a1', -1001000000001),
               ('a2', 555);",
        )
        .unwrap();

        let admin = 555_i64;
        super::rehome_group_originated_chat_ids(&conn, admin);

        // Group-originated send target re-homed to admin; private id untouched.
        let t1: i64 = conn
            .query_row("SELECT chat_id FROM tasks WHERE id=1", [], |r| r.get(0))
            .unwrap();
        let t2: i64 = conn
            .query_row("SELECT chat_id FROM tasks WHERE id=2", [], |r| r.get(0))
            .unwrap();
        assert_eq!(t1, admin, "group task chat_id re-homed to admin");
        assert_eq!(t2, 555, "private task chat_id untouched");
        // Memory scope column is NOT rewritten (no L1/L2/L3 namespace merge).
        let did1: String = conn
            .query_row("SELECT user_did FROM tasks WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            did1, "did:aegis:telegram:-1001000000001",
            "user_did (memory scope) preserved"
        );
        let m1: i64 = conn
            .query_row(
                "SELECT chat_id FROM agent_manifests WHERE agent_id='a1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(m1, admin, "group manifest chat_id re-homed");

        // Idempotent: a second pass changes nothing (no negative rows remain).
        super::rehome_group_originated_chat_ids(&conn, admin);
        let t1b: i64 = conn
            .query_row("SELECT chat_id FROM tasks WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(t1b, admin);

        // Missing table is tolerated (no panic).
        let bare = rusqlite::Connection::open_in_memory().unwrap();
        super::rehome_group_originated_chat_ids(&bare, admin);
    }

    #[test]
    fn migrations_create_all_tables() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        // Verify critical tables exist
        let expected = &[
            "l2_experience",
            "reminders",
            "l1_snapshot",
            "l3_archive",
            "email_drafts",
            "user_settings",
            "skill_proposals",
            "skill_metrics",
            "skill_golden_tests",
            "deferred_intents",
            "tasks",
            "knowledge_entries",
            "command_usage",
            "review_log",
            "replay_nonces",
            "research_tasks",
            "smart_panels",
            "agent_manifests",
            "companion_identity",
            "growth_dimensions",
            "growth_log",
            "notification_feedback",
            "l2_lint_findings",
            // Memory Tier Redesign tables
            "user_profile",
            "l2_ephemeral",
            "extraction_cursor",
            "reflection_log",
            // Heartbeat autonomy redesign (Step 1)
            "config_audit_log",
        ];
        for &table in expected {
            assert!(
                tables.contains(&table.to_string()),
                "Missing table: {table}"
            );
        }
    }

    #[test]
    fn migrations_idempotent() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        // Running again should not fail (CREATE IF NOT EXISTS)
        run_all_migrations(&conn).unwrap();
    }

    #[test]
    fn migrations_preserve_internal_skill_state_namespaces() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        let rows = [
            (
                "web_push_subscriptions",
                "did:aegis:telegram:1",
                "push-state",
            ),
            (
                "web_push_delivery_receipts",
                "did:aegis:telegram:1",
                "push-receipt",
            ),
            ("system", "profile_registry_migration_done", "v1"),
            ("hitl", "approval-1", "pending"),
            // delegate_task linkage (not a Wasm skill) — must survive restarts.
            (
                "hercules_delegation",
                "aegis-runtime-add-version-1a2b3c4d",
                "{\"last_state\":\"todo\"}",
            ),
            ("orphan_skill", "stale", "delete-me"),
        ];
        for (skill_name, state_key, state_value) in rows {
            conn.execute(
                "INSERT OR REPLACE INTO skill_state (skill_name, state_key, state_value, updated_at) \
                 VALUES (?1, ?2, ?3, datetime('now'))",
                rusqlite::params![skill_name, state_key, state_value],
            )
            .unwrap();
        }

        run_all_migrations(&conn).unwrap();

        for (skill_name, state_key, state_value) in rows.into_iter().take(5) {
            let loaded: String = conn
                .query_row(
                    "SELECT state_value FROM skill_state WHERE skill_name = ?1 AND state_key = ?2",
                    rusqlite::params![skill_name, state_key],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(loaded, state_value);
        }
        let orphan_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skill_state WHERE skill_name = 'orphan_skill'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(orphan_count, 0);
    }

    #[test]
    fn redundant_skills_list() {
        assert!(!REDUNDANT_SKILLS.is_empty());
        assert!(REDUNDANT_SKILLS.contains(&"reminder"));
        assert!(REDUNDANT_SKILLS.contains(&"todo_list_manager"));
    }

    #[test]
    fn vec_cosine_sim_orthogonal() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        // [1,0] · [0,1] = 0 → orthogonal vectors
        let a: Vec<u8> = [1.0f32, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let b: Vec<u8> = [0.0f32, 1.0].iter().flat_map(|v| v.to_le_bytes()).collect();

        let sim: f64 = conn
            .query_row(
                "SELECT vec_cosine_sim(?1, ?2)",
                rusqlite::params![a, b],
                |r| r.get(0),
            )
            .unwrap();
        assert!((sim - 0.0).abs() < 1e-6, "Expected ~0.0, got {sim}");
    }

    #[test]
    fn vec_cosine_sim_identical() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        // [3,4] · [3,4] = 1.0
        let a: Vec<u8> = [3.0f32, 4.0].iter().flat_map(|v| v.to_le_bytes()).collect();

        let sim: f64 = conn
            .query_row(
                "SELECT vec_cosine_sim(?1, ?2)",
                rusqlite::params![a.clone(), a],
                |r| r.get(0),
            )
            .unwrap();
        assert!((sim - 1.0).abs() < 1e-6, "Expected ~1.0, got {sim}");
    }

    #[test]
    fn vec_cosine_sim_opposite() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        // [1,0] · [-1,0] = -1.0
        let a: Vec<u8> = [1.0f32, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let b: Vec<u8> = [-1.0f32, 0.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();

        let sim: f64 = conn
            .query_row(
                "SELECT vec_cosine_sim(?1, ?2)",
                rusqlite::params![a, b],
                |r| r.get(0),
            )
            .unwrap();
        assert!((sim - (-1.0)).abs() < 1e-6, "Expected ~-1.0, got {sim}");
    }

    #[test]
    fn vec_cosine_sim_empty_blobs() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        let sim: f64 = conn
            .query_row(
                "SELECT vec_cosine_sim(?1, ?2)",
                rusqlite::params![Vec::<u8>::new(), Vec::<u8>::new()],
                |r| r.get(0),
            )
            .unwrap();
        assert!((sim - 0.0).abs() < 1e-6);
    }

    #[test]
    fn vec_cosine_sim_mismatched_length() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        let a: Vec<u8> = [1.0f32, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let b: Vec<u8> = [1.0f32, 0.0, 0.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();

        let sim: f64 = conn
            .query_row(
                "SELECT vec_cosine_sim(?1, ?2)",
                rusqlite::params![a, b],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            (sim - 0.0).abs() < 1e-6,
            "Mismatched lengths should return 0.0"
        );
    }

    /// Owner's most-repeated lesson: migration tests must start from the OLD
    /// (pre-existing) schema, not fresh-init.  A fresh DB already has the new
    /// columns; the migration only matters on an existing DB that lacks them.
    #[test]
    fn migration_l3_archive_anchor_columns_on_existing_db() {
        let conn = mem_db();

        // ── Simulate an old DB: create l3_archive WITHOUT anchor columns ───────
        conn.execute_batch(
            "CREATE TABLE l3_archive (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_did  TEXT NOT NULL,
                role       TEXT NOT NULL,
                content    TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )
        .unwrap();

        // Insert a pre-existing row (predates the migration).
        conn.execute(
            "INSERT INTO l3_archive (agent_did, role, content, created_at)
             VALUES ('did:aegis:test', 'user', 'hello world', '2026-06-01T00:00:00Z')",
            [],
        )
        .unwrap();

        // ── Run migrations (must not fail on the already-existing table) ────────
        run_all_migrations(&conn).unwrap();

        // (a) New columns now exist — query them without error.
        let (anchor_kind, anchor_ref): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT anchor_kind, anchor_ref FROM l3_archive WHERE agent_did = 'did:aegis:test'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        // (b) Pre-existing row reads back with NULL anchors (correct fail-safe).
        assert_eq!(
            anchor_kind, None,
            "pre-existing l3_archive row must have NULL anchor_kind after migration"
        );
        assert_eq!(
            anchor_ref, None,
            "pre-existing l3_archive row must have NULL anchor_ref after migration"
        );

        // (c) Idempotency: running migrations again must not error.
        run_all_migrations(&conn).unwrap();
    }

    #[test]
    fn tasks_table_has_expected_columns() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        // Insert a minimal task and read it back to verify column structure
        conn.execute(
            "INSERT INTO tasks (chat_id, user_did, title) VALUES (1, 'did:test', 'Test')",
            [],
        )
        .unwrap();

        let id: i64 = conn
            .query_row("SELECT last_insert_rowid()", [], |r| r.get(0))
            .unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM tasks WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "active");
    }

    #[test]
    fn skill_proposals_table_defaults() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO skill_proposals (skill_name, requested_by, user_request) VALUES ('test', 'did:test', 'please')",
            [],
        ).unwrap();

        let (status, retry): (String, i32) = conn
            .query_row(
                "SELECT status, retry_count FROM skill_proposals WHERE skill_name = 'test'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "pending_approval");
        assert_eq!(retry, 0);
    }

    #[test]
    fn skill_proposals_audit_override_metadata_columns_exist() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(skill_proposals)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        for expected in [
            "audit_override_by",
            "audit_override_at",
            "audit_override_reason",
        ] {
            assert!(
                columns.iter().any(|column| column == expected),
                "missing skill_proposals column {expected}"
            );
        }
    }

    /// Regression guard for the Fix B windowed-recurrence migration: an EXISTING
    /// production DB has a `tasks` table that predates the recur_* columns. The
    /// `CREATE TABLE IF NOT EXISTS` in run_all_migrations is a no-op there, so the
    /// columns only appear if a dedicated `ALTER TABLE` migration runs. This test
    /// simulates that existing DB by pre-creating `tasks` WITHOUT the recur_*
    /// columns, then asserts the migration adds them and a recur_*-naming insert
    /// works. (A fresh `spawn_in_memory` DB would hit the CREATE path and mask the
    /// missing ALTER — exactly why the sub-agent's full-suite tests stayed green
    /// while production would have failed with "no such column".)
    #[test]
    fn recur_columns_added_to_preexisting_tasks_table() {
        let conn = mem_db();
        // Pre-existing (pre-Fix-B) tasks schema — the current CREATE minus the
        // four recur_* columns.
        conn.execute_batch(
            "CREATE TABLE tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id INTEGER NOT NULL,
                user_did TEXT NOT NULL,
                title TEXT NOT NULL,
                body TEXT NOT NULL DEFAULT '',
                priority TEXT NOT NULL DEFAULT 'normal',
                status TEXT NOT NULL DEFAULT 'active',
                tags TEXT NOT NULL DEFAULT '[]',
                due_date TEXT,
                source TEXT NOT NULL DEFAULT 'user',
                trigger_at_unix INTEGER,
                trigger_type TEXT,
                event_key TEXT,
                execution_mode TEXT DEFAULT 'notify',
                context_text TEXT,
                interval_seconds INTEGER,
                group_key TEXT,
                max_fires INTEGER,
                fire_count INTEGER NOT NULL DEFAULT 0,
                fired_at TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                chain_depth INTEGER NOT NULL DEFAULT 0,
                pin_on_fire INTEGER NOT NULL DEFAULT 0,
                expires_at_unix INTEGER,
                is_callback_origin INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT
            );",
        )
        .unwrap();

        run_all_migrations(&conn).unwrap();

        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(tasks)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        for expected in [
            "recur_weekdays",
            "recur_window_start",
            "recur_window_end",
            "recur_timezone",
            // archived_at: retention-delete keys on this column, so a pre-existing
            // tasks table MUST gain it or the destructive sweep hits "no such column".
            "archived_at",
        ] {
            assert!(
                columns.iter().any(|column| column == expected),
                "migration did not add tasks column {expected} to a pre-existing table"
            );
        }

        // The columns must be usable by a recur_*-naming INSERT (the path that
        // would have hit "no such column" in production).
        conn.execute(
            "INSERT INTO tasks (chat_id, user_did, title, recur_weekdays, recur_window_start, recur_window_end, recur_timezone) \
             VALUES (1, 'did:test', 't', '[2,3,4]', '17:30', '18:30', 'America/Los_Angeles')",
            [],
        )
        .unwrap();
        let wk: String = conn
            .query_row(
                "SELECT recur_weekdays FROM tasks WHERE user_did = 'did:test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(wk, "[2,3,4]");
    }

    #[test]
    fn skill_proposals_skill_status_version_index_exists() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index'
                   AND tbl_name = 'skill_proposals'
                   AND name = 'idx_proposals_skill_status_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(index_count, 1);

        let indexed_columns: Vec<String> = conn
            .prepare("PRAGMA index_info(idx_proposals_skill_status_version)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(2))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(indexed_columns, ["skill_name", "status", "version"]);
    }

    #[test]
    fn l2_lint_findings_table_defaults() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO l2_lint_findings (finding_type, fact_id_a, fact_id_b, similarity, llm_verdict)
             VALUES ('contradiction', 1, 2, 0.87, 'These facts conflict about user location')",
            [],
        )
        .unwrap();

        let (status, resolved_at): (String, Option<String>) = conn
            .query_row(
                "SELECT status, resolved_at FROM l2_lint_findings WHERE finding_type = 'contradiction'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "pending");
        assert!(resolved_at.is_none());
    }

    #[test]
    fn l2_lint_findings_update_status() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO l2_lint_findings (finding_type, fact_id_a, fact_id_b)
             VALUES ('connection', 10, 20)",
            [],
        )
        .unwrap();

        let id: i64 = conn.last_insert_rowid();
        conn.execute(
            "UPDATE l2_lint_findings SET status = 'resolved', user_action = 'kept_both', resolved_at = datetime('now') WHERE id = ?1",
            rusqlite::params![id],
        )
        .unwrap();

        let (status, action): (String, Option<String>) = conn
            .query_row(
                "SELECT status, user_action FROM l2_lint_findings WHERE id = ?1",
                rusqlite::params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "resolved");
        assert_eq!(action.as_deref(), Some("kept_both"));
    }

    #[test]
    fn companion_identity_seeded_on_fresh_db() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        let (name, stage, core_values): (String, String, String) = conn
            .query_row(
                "SELECT name, growth_stage, core_values FROM companion_identity WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(name, "");
        assert_eq!(stage, "chu_yu");
        assert!(!core_values.is_empty());
        assert!(core_values.contains("Aegis"));
    }

    #[test]
    fn growth_dimensions_seeded_five_default() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM growth_dimensions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 5);

        // Check a specific dimension
        let display: String = conn
            .query_row(
                "SELECT display_name FROM growth_dimensions WHERE dimension_key = 'rapport'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(display.contains("Rapport"));
    }

    #[test]
    fn growth_log_table_accepts_entries() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO growth_log (event_type, dimension_key, detail, delta, level_after, metadata)
             VALUES ('milestone', 'rapport', 'test event', 0.05, 0.15, '{}')",
            [],
        )
        .unwrap();

        let (event_type, delta): (String, f64) = conn
            .query_row(
                "SELECT event_type, delta FROM growth_log WHERE dimension_key = 'rapport'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(event_type, "milestone");
        assert!((delta - 0.05).abs() < 1e-6);
    }

    #[test]
    fn companion_identity_backfill_from_interaction_count() {
        let conn = mem_db();
        // Create relationship_state table first (part of main migrations)
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS relationship_state (
                user_did TEXT PRIMARY KEY,
                current_focus TEXT NOT NULL DEFAULT '[]',
                ongoing_goals TEXT NOT NULL DEFAULT '[]',
                narrative TEXT NOT NULL DEFAULT '',
                last_interaction_at TEXT,
                last_topic TEXT NOT NULL DEFAULT '',
                interaction_count INTEGER NOT NULL DEFAULT 0,
                streak_days INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                active_hours TEXT NOT NULL DEFAULT '{}'
            );",
        )
        .unwrap();
        // Insert a user with 600 interactions (should map to mo_qi)
        conn.execute(
            "INSERT INTO relationship_state (user_did, interaction_count) VALUES ('did:test', 600)",
            [],
        )
        .unwrap();

        run_all_migrations(&conn).unwrap();

        let stage: String = conn
            .query_row(
                "SELECT growth_stage FROM companion_identity WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stage, "mo_qi");

        // Rapport depth should be backfilled
        let depth: f64 = conn
            .query_row(
                "SELECT depth FROM growth_dimensions WHERE dimension_key = 'rapport'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!((depth - 1.2).abs() < 0.01, "expected ~1.2, got {depth}");
    }

    #[test]
    fn notification_feedback_table_accepts_entries() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO notification_feedback (notif_id, helpful) VALUES (?1, ?2)",
            rusqlite::params!["1712345678901", 1],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO notification_feedback (notif_id, helpful) VALUES (?1, ?2)",
            rusqlite::params!["1712345678902", 0],
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM notification_feedback", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 2);

        let helpful_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM notification_feedback WHERE helpful = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(helpful_count, 1);
    }

    #[test]
    fn companion_identity_seed_data_completeness() {
        // Ensures ALL seeded fields have proper values (catches empty '{}' personality issue)
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        let (personality_json, core_values, created_at, growth_stage): (
            String,
            String,
            String,
            String,
        ) = conn
            .query_row(
                "SELECT personality, core_values, created_at, growth_stage FROM companion_identity WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();

        // Personality must have explicit defaults, not empty '{}'
        assert_ne!(personality_json, "{}", "personality must not be empty JSON");
        let pw: serde_json::Value = serde_json::from_str(&personality_json).unwrap();
        assert!(
            pw.get("warmth").and_then(|v| v.as_f64()).unwrap_or(0.0) > 0.0,
            "warmth must be seeded"
        );

        // Core values must contain both pillars
        assert!(core_values.contains("Care"), "must contain Care");
        assert!(core_values.contains("Aegis"), "must contain Aegis");

        // created_at must not be empty
        assert!(!created_at.is_empty(), "created_at must be set");

        // growth_stage must be valid
        assert!(
            ["chu_yu", "xiang_zhi", "mo_qi", "gong_sheng", "jue_wu"]
                .contains(&growth_stage.as_str()),
            "growth_stage must be valid: got {growth_stage}"
        );
    }

    #[test]
    fn format_self_awareness_handles_sqlite_datetime() {
        // Ensures the SQLite datetime('now') format is correctly parsed
        let identity = crate::memory::CompanionIdentity {
            created_at: "2026-04-11 17:49:47".to_string(), // SQLite format, NOT RFC3339
            ..Default::default()
        };
        let output = crate::memory::format_self_awareness(&identity, &[], 42);
        assert!(
            output.contains("Birthday:"),
            "SQLite datetime must be parsed — got: {output}"
        );
        assert!(output.contains("Day "), "Day count must be shown");
    }

    // ── Memory Tier Redesign: Migration Tests ───────────────────────────

    #[test]
    fn user_profile_table_schema() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        // Basic insert
        conn.execute(
            "INSERT INTO user_profile (agent_did, category, key, value, confidence, source)
             VALUES ('did:aegis:telegram:123', 'location', 'home_address', 'Example City, WA', 0.95, 'auto')",
            [],
        )
        .unwrap();

        let (cat, key, val, conf): (String, String, String, f64) = conn
            .query_row(
                "SELECT category, key, value, confidence FROM user_profile WHERE agent_did = 'did:aegis:telegram:123'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(cat, "location");
        assert_eq!(key, "home_address");
        assert_eq!(val, "Example City, WA");
        assert!((conf - 0.95).abs() < 1e-6);
    }

    #[test]
    fn user_profile_unique_constraint_upsert() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        // INSERT initial
        conn.execute(
            "INSERT INTO user_profile (agent_did, category, key, value, confidence, source)
             VALUES ('did:test', 'location', 'home', 'Old Address', 0.8, 'auto')",
            [],
        )
        .unwrap();

        // INSERT OR REPLACE should update
        conn.execute(
            "INSERT OR REPLACE INTO user_profile (agent_did, category, key, value, confidence, source)
             VALUES ('did:test', 'location', 'home', 'New Address', 0.95, 'manual')",
            [],
        )
        .unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM user_profile WHERE agent_did = 'did:test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "UNIQUE constraint should prevent duplicates");

        let (val, source): (String, String) = conn
            .query_row(
                "SELECT value, source FROM user_profile WHERE agent_did = 'did:test' AND category = 'location'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(val, "New Address");
        assert_eq!(source, "manual");
    }

    #[test]
    fn user_profile_multiple_categories() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        let entries = [
            ("identity", "name", "Lance"),
            ("location", "home_address", "Example City"),
            ("location", "work_address", "Worktown"),
            ("preference", "hair", "Asian straight, fade"),
            ("family", "members", "Wife + daughter"),
        ];
        for (cat, key, val) in &entries {
            conn.execute(
                "INSERT INTO user_profile (agent_did, category, key, value) VALUES ('did:test', ?1, ?2, ?3)",
                rusqlite::params![cat, key, val],
            )
            .unwrap();
        }

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM user_profile WHERE agent_did = 'did:test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 5);

        // Verify ordered by category
        let categories: Vec<String> = conn
            .prepare(
                "SELECT category FROM user_profile WHERE agent_did = 'did:test' ORDER BY category",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(
            categories,
            vec!["family", "identity", "location", "location", "preference"]
        );
    }

    #[test]
    fn l2_ephemeral_table_schema() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, source, confidence, expires_at)
             VALUES ('did:test', 'l1_overflow', 'test fact', 'l1_overflow', 0.6, datetime('now', '+30 days'))",
            [],
        )
        .unwrap();

        let (topic, source, conf): (String, String, f64) = conn
            .query_row(
                "SELECT topic, source, confidence FROM l2_ephemeral WHERE agent_did = 'did:test'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(topic, "l1_overflow");
        assert_eq!(source, "l1_overflow");
        assert!((conf - 0.6).abs() < 1e-6);
    }

    #[test]
    fn l2_ephemeral_cleanup_expired() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        // Insert expired entry (expires_at in the past)
        conn.execute(
            "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, source, expires_at)
             VALUES ('did:test', 'test', 'expired fact', 'l1_overflow', datetime('now', '-1 day'))",
            [],
        )
        .unwrap();

        // Insert valid entry (expires_at in the future)
        conn.execute(
            "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, source, expires_at)
             VALUES ('did:test', 'test', 'valid fact', 'research', datetime('now', '+30 days'))",
            [],
        )
        .unwrap();

        // Cleanup
        let deleted = conn
            .execute(
                "DELETE FROM l2_ephemeral WHERE expires_at < datetime('now')",
                [],
            )
            .unwrap();
        assert_eq!(deleted, 1);

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM l2_ephemeral", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn l2_ephemeral_source_filter() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        for (source, count) in &[("l1_overflow", 3), ("research", 2), ("context_summary", 1)] {
            for i in 0..*count {
                conn.execute(
                    "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, source, expires_at)
                     VALUES ('did:test', ?1, ?2, ?1, datetime('now', '+30 days'))",
                    rusqlite::params![source, format!("fact {i}")],
                )
                .unwrap();
            }
        }

        let overflow_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM l2_ephemeral WHERE source = 'l1_overflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(overflow_count, 3);

        let research_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM l2_ephemeral WHERE source = 'research'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(research_count, 2);
    }

    #[test]
    fn extraction_cursor_table_schema() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        // Insert cursor
        conn.execute(
            "INSERT INTO extraction_cursor (agent_did) VALUES ('did:test')",
            [],
        )
        .unwrap();

        let (ts, total): (String, i64) = conn
            .query_row(
                "SELECT last_processed_ts, total_extracted FROM extraction_cursor WHERE agent_did = 'did:test'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(ts, "1970-01-01T00:00:00Z");
        assert_eq!(total, 0);
    }

    #[test]
    fn extraction_cursor_update_and_read() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO extraction_cursor (agent_did) VALUES ('did:test')",
            [],
        )
        .unwrap();

        conn.execute(
            "UPDATE extraction_cursor SET last_processed_ts = '2026-04-11T12:00:00Z', total_extracted = 42
             WHERE agent_did = 'did:test'",
            [],
        )
        .unwrap();

        let (ts, total): (String, i64) = conn
            .query_row(
                "SELECT last_processed_ts, total_extracted FROM extraction_cursor WHERE agent_did = 'did:test'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(ts, "2026-04-11T12:00:00Z");
        assert_eq!(total, 42);
    }

    /// End-to-end test: simulates production-like data with mixed DIDs, topics,
    /// and verifies the full tier migration produces correct results.
    #[test]
    fn tier_migration_did_unification_and_data_split() {
        let conn = mem_db();

        // Run all migrations first to create tables
        run_all_migrations_with_owner(&conn, "did:aegis:telegram:1000000001").unwrap();

        // Reset tier migration flag so we can test the migration logic directly
        conn.execute(
            "DELETE FROM skill_state WHERE skill_name = 'system' AND state_key = 'tier_migration_done'",
            [],
        )
        .unwrap();

        // Populate l2_experience with mixed data simulating production state
        let test_data = [
            // Tier 1 (should stay): learned:* topics under telegram DID
            (
                "did:aegis:telegram:1000000001",
                "learned:context",
                "User lives in Example City, WA",
                0.95,
            ),
            (
                "did:aegis:telegram:1000000001",
                "learned:preference",
                "Prefers Asian fade haircut",
                0.9,
            ),
            (
                "did:aegis:telegram:1000000001",
                "learned:habit",
                "Monitors MSFT stock daily",
                0.85,
            ),
            // Tier 1 (should stay): extract_immediate_fact topics
            (
                "did:aegis:telegram:1000000001",
                "user_fact",
                "Wife and daughter at home",
                0.8,
            ),
            // Tier 1 (should stay): heartbeat_memo
            (
                "did:aegis:telegram:1000000001",
                "heartbeat_memo_archive",
                "Morning report 2026-04-10",
                0.7,
            ),
            // DID unification target: did:aegis:system → telegram DID (not skillforge)
            (
                "did:aegis:system",
                "learned:context",
                "Lance works at Example Corp",
                0.9,
            ),
            (
                "did:aegis:system",
                "learned:interest",
                "Interested in Tesla Model 3",
                0.8,
            ),
            // Should NOT be unified: skillforge stays under system DID
            (
                "did:aegis:system",
                "skillforge:weather",
                "Weather API skill",
                0.7,
            ),
            // Tier 2 migration targets
            (
                "did:aegis:telegram:1000000001",
                "l1_overflow",
                "Old conversation fragment",
                0.6,
            ),
            (
                "did:aegis:telegram:1000000001",
                "research:synthesis:tesla",
                "Tesla leasing findings",
                0.7,
            ),
            (
                "did:aegis:telegram:1000000001",
                "context_summary",
                "Daily context summary",
                0.65,
            ),
            // Free-form compaction topics (Tier 2)
            (
                "did:aegis:telegram:1000000001",
                "MSFT Stock Monitoring",
                "Stock alert details",
                0.6,
            ),
            (
                "did:aegis:telegram:1000000001",
                "Barber Preferences",
                "Barber shop info",
                0.6,
            ),
        ];

        for (did, topic, fact, conf) in &test_data {
            conn.execute(
                "INSERT INTO l2_experience (agent_did, topic, fact_summary, confidence, created_at, last_accessed)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
                rusqlite::params![did, topic, fact, conf],
            )
            .unwrap();
        }

        // Run migration again — it should detect the flag is missing and re-run
        run_all_migrations_with_owner(&conn, "did:aegis:telegram:1000000001").unwrap();

        // Verify DID unification: system → telegram (except skillforge/api_knowledge)
        // Note: skillforge:* is migrated to l2_ephemeral in Step 8, so it's no longer in l2_experience.
        // Only entries that were NOT unified AND NOT migrated to Tier 2 should remain under system DID.
        let system_did_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM l2_experience WHERE agent_did = 'did:aegis:system'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            system_did_count, 0,
            "No system DID entries should remain in l2_experience (skillforge moved to Tier 2, learned unified)"
        );

        // Verify skillforge entries were preserved in l2_ephemeral under system DID
        let skillforge_in_ephemeral: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM l2_ephemeral WHERE source = 'skillforge'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            skillforge_in_ephemeral >= 1,
            "skillforge should be in l2_ephemeral"
        );

        // Verify Tier 1 (l2_experience): only learned:*, heartbeat_memo*, user_fact remain
        let tier1_topics: Vec<String> = conn
            .prepare(
                "SELECT DISTINCT topic FROM l2_experience WHERE agent_did = 'did:aegis:telegram:1000000001' ORDER BY topic",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        for topic in &tier1_topics {
            assert!(
                topic.starts_with("learned:")
                    || topic.starts_with("heartbeat_memo")
                    || topic == "user_fact",
                "Unexpected topic in Tier 1 after migration: {topic}"
            );
        }
        // l1_overflow, research:*, context_summary, free-form topics should NOT be in Tier 1
        let leftover: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM l2_experience
                 WHERE topic = 'l1_overflow' OR topic LIKE 'research:%' OR topic = 'context_summary'
                    OR topic = 'MSFT Stock Monitoring' OR topic = 'Barber Preferences'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            leftover, 0,
            "Migrated topics should not remain in l2_experience"
        );

        // Verify Tier 2 (l2_ephemeral): migrated data present
        let ephemeral_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM l2_ephemeral", [], |r| r.get(0))
            .unwrap();
        assert!(
            ephemeral_count >= 5,
            "Expected ≥5 ephemeral entries, got {ephemeral_count}"
        );

        // Verify sources in ephemeral
        let sources: Vec<String> = conn
            .prepare("SELECT DISTINCT source FROM l2_ephemeral ORDER BY source")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(sources.contains(&"l1_overflow".to_string()));
        assert!(sources.contains(&"research".to_string()));
        assert!(sources.contains(&"context_summary".to_string()));
        assert!(sources.contains(&"compaction_summary".to_string()));

        // Verify TTLs: l1_overflow = 30 days, research = 90 days
        let overflow_expires: String = conn
            .query_row(
                "SELECT expires_at FROM l2_ephemeral WHERE source = 'l1_overflow' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !overflow_expires.is_empty(),
            "l1_overflow should have expires_at set"
        );

        // Verify migration flag set
        let flag: String = conn
            .query_row(
                "SELECT state_value FROM skill_state WHERE skill_name = 'system' AND state_key = 'tier_migration_done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(flag, "v1");

        // Verify HNSW rebuild flag set (Step 13)
        let hnsw_flag: String = conn
            .query_row(
                "SELECT state_value FROM skill_state WHERE skill_name = 'system' AND state_key = 'hnsw_rebuild_needed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hnsw_flag, "tier_migration");
    }

    #[test]
    fn tier_migration_idempotent() {
        let conn = mem_db();
        run_all_migrations_with_owner(&conn, "did:aegis:telegram:1000000001").unwrap();

        // Insert some test data
        conn.execute(
            "INSERT INTO l2_experience (agent_did, topic, fact_summary, confidence, created_at, last_accessed)
             VALUES ('did:aegis:telegram:1000000001', 'learned:context', 'test fact', 0.9, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        // Run migrations again — should be no-op because flag is set
        run_all_migrations_with_owner(&conn, "did:aegis:telegram:1000000001").unwrap();

        // Verify no data was duplicated
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM l2_experience", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "Idempotent migration should not duplicate data");
    }

    #[test]
    fn tier_migration_skillforge_keeps_system_did() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();

        // Reset flag and add skillforge data
        conn.execute(
            "DELETE FROM skill_state WHERE skill_name = 'system' AND state_key = 'tier_migration_done'",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO l2_experience (agent_did, topic, fact_summary, confidence, created_at, last_accessed)
             VALUES ('did:aegis:system', 'skillforge:weather_api', 'Weather API compilation notes', 0.7, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO l2_experience (agent_did, topic, fact_summary, confidence, created_at, last_accessed)
             VALUES ('did:aegis:system', 'learned:api_knowledge', 'API endpoint docs', 0.8, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        run_all_migrations(&conn).unwrap();

        // learned:api_knowledge should remain under system DID in l2_experience (DID not unified)
        let api_knowledge_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM l2_experience WHERE agent_did = 'did:aegis:system' AND topic = 'learned:api_knowledge'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            api_knowledge_count, 1,
            "api_knowledge should stay under system DID in Tier 1"
        );

        // skillforge:* should be in l2_ephemeral (migrated to Tier 2), preserved under system DID
        let skillforge_ephemeral: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM l2_ephemeral WHERE agent_did = 'did:aegis:system' AND source = 'skillforge'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            skillforge_ephemeral, 1,
            "skillforge should be in l2_ephemeral under system DID"
        );

        // skillforge should NOT remain in l2_experience
        let skillforge_tier1: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM l2_experience WHERE topic LIKE 'skillforge:%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            skillforge_tier1, 0,
            "skillforge should be removed from l2_experience after migration"
        );
    }

    #[test]
    fn user_profile_seed_from_learned_context() {
        let conn = mem_db();
        run_all_migrations_with_owner(&conn, "did:aegis:telegram:1000000001").unwrap();

        // Reset flag and add learned:context facts that match seed patterns
        conn.execute(
            "DELETE FROM skill_state WHERE skill_name = 'system' AND state_key = 'tier_migration_done'",
            [],
        )
        .unwrap();

        // Synthetic facts. These used to be the author's real home address,
        // employer and name — a privacy leak that also coupled the test to the
        // personal LIKE patterns the seeding queries no longer use.
        let seed_facts = [
            ("Owner lives at 123 Example Street, Example City", 0.95),
            ("Owner works at Example Corp, in the downtown office", 0.9),
            ("Owner name is Sample Person, works as an engineer", 0.88),
        ];
        for (fact, conf) in &seed_facts {
            conn.execute(
                "INSERT INTO l2_experience (agent_did, topic, fact_summary, confidence, created_at, last_accessed)
                 VALUES ('did:aegis:telegram:1000000001', 'learned:context', ?1, ?2, datetime('now'), datetime('now'))",
                rusqlite::params![fact, conf],
            )
            .unwrap();
        }

        run_all_migrations_with_owner(&conn, "did:aegis:telegram:1000000001").unwrap();

        // Verify profile was seeded
        let profile_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM user_profile WHERE agent_did = 'did:aegis:telegram:1000000001'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            profile_count >= 2,
            "Expected ≥2 profile entries, got {profile_count}"
        );

        // Check home address was seeded
        let home: Option<String> = conn
            .query_row(
                "SELECT value FROM user_profile WHERE agent_did = 'did:aegis:telegram:1000000001' AND key = 'home_address'",
                [],
                |r| r.get(0),
            )
            .ok();
        assert!(home.is_some(), "home_address should be seeded");
        assert!(home.unwrap().contains("Example Street"));
    }

    // ── FU#2 Phase 7: Profile Slot Registry migration tests ─────────────

    fn migrations_db_with_profile_rows(rows: &[(&str, &str, &str, &str)]) -> Connection {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        // Clear anything seeded by the tier migration so our fixtures are clean.
        conn.execute("DELETE FROM user_profile", []).unwrap();
        for (cat, key, value, source) in rows {
            conn.execute(
                "INSERT INTO user_profile (agent_did, category, key, value, confidence, source, updated_at) \
                 VALUES ('did:aegis:telegram:7', ?1, ?2, ?3, 0.9, ?4, datetime('now'))",
                rusqlite::params![cat, key, value, source],
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn phase7_legacy_alias_rewritten_to_canonical() {
        let conn = migrations_db_with_profile_rows(&[(
            "location",
            "home_location",
            "Example City, Washington",
            "auto",
        )]);
        let stats = run_profile_registry_migration(&conn).unwrap();
        assert_eq!(stats.canonicalized, 1);
        assert_eq!(stats.merged_via_alias, 0);
        let (cat, key, value): (String, String, String) = conn
            .query_row("SELECT category, key, value FROM user_profile", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!(cat, "location");
        assert_eq!(key, "residence_city");
        assert_eq!(value, "Example City, Washington");
    }

    #[test]
    fn phase7_alias_collision_keeps_newer() {
        // Seed BOTH legacy and canonical. Canonical is older; legacy is newer
        // → legacy wins via the updated_at comparison.
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        conn.execute("DELETE FROM user_profile", []).unwrap();
        // Canonical (older).
        conn.execute(
            "INSERT INTO user_profile (agent_did, category, key, value, confidence, source, updated_at) \
             VALUES ('did:aegis:telegram:7', 'location', 'residence_city', 'Old', 0.9, 'auto', '2020-01-01T00:00:00Z')",
            [],
        ).unwrap();
        // Legacy (newer).
        conn.execute(
            "INSERT INTO user_profile (agent_did, category, key, value, confidence, source, updated_at) \
             VALUES ('did:aegis:telegram:7', 'location', 'home_location', 'New', 0.9, 'auto', '2026-04-12T00:00:00Z')",
            [],
        ).unwrap();
        let stats = run_profile_registry_migration(&conn).unwrap();
        assert_eq!(stats.merged_via_alias, 1);
        let rows: Vec<(String, String)> = {
            let mut stmt = conn.prepare("SELECT key, value FROM user_profile").unwrap();
            let it = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            it.filter_map(|r| r.ok()).collect()
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], ("residence_city".to_string(), "New".to_string()));
    }

    #[test]
    fn phase7_sweeps_known_bad_auto_rows() {
        // Emulate the 4 prod bad rows. AddressLike rejects narrative / non-address;
        // preference.hair is not in registry (aliases to health.physical = FreeText
        // which accepts anything, but the row source is auto so alias rewrites
        // then Pass 2 accepts it — this row survives).
        // NOTE: `(agent_did, category, key)` is UNIQUE, so seed each (cat,key)
        // at most once. We seed the bad canonical rows + the legacy-alias rows
        // (aliases live under different keys so they don't collide).
        let conn = migrations_db_with_profile_rows(&[
            // id=12: narrative home_address → invalid
            (
                "location",
                "home_address",
                "User lives in Example City, has Asian straight hair, and works remotely.",
                "auto",
            ),
            // id=13: non-address work_address → invalid
            (
                "location",
                "work_address",
                "user regularly uses Example Corp Copilot as part of their work",
                "auto",
            ),
            // id=15: legacy key preference.hair → aliased to health.physical (FreeText, accepts)
            (
                "preference",
                "hair",
                "Asian straight, fade cut, every 2-3 weeks",
                "auto",
            ),
            // id=20: legacy home_location → aliased to residence_city (city passes LocationLike)
            (
                "location",
                "home_location",
                "Example City, Washington",
                "auto",
            ),
            // Valid identity row should SURVIVE (manual bypasses validator anyway).
            ("identity", "name", "Lance", "manual"),
        ]);

        // The migration runs; the two narrative/non-address rows get deleted,
        // the two legacy-key rows get canonicalized. But wait — the canonical
        // home_address slot collides (the narrative row and the good Lake
        // Stevens address share canonical slot). The narrative row is NOT
        // a legacy alias (it's already canonical home_address) so Pass 1
        // doesn't touch it; Pass 2 deletes it via validator.
        let stats = run_profile_registry_migration(&conn).unwrap();
        assert!(
            stats.invalid_machine_deleted >= 2,
            "expected ≥2 invalid auto rows deleted; got {stats:?}"
        );

        // Expected survivors:
        //   - location.residence_city = "Example City, Washington" (aliased from home_location)
        //   - health.physical = "Asian straight…" (aliased from preference.hair)
        //   - identity.name = "Lance" (manual)
        // The narrative home_address and work-copilot rows are deleted by Pass 2.
        let final_rows: Vec<(String, String, String)> = {
            let mut stmt = conn
                .prepare("SELECT category, key, value FROM user_profile ORDER BY category, key")
                .unwrap();
            let it = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap();
            it.filter_map(|r| r.ok()).collect()
        };
        let keys: Vec<String> = final_rows
            .iter()
            .map(|(c, k, _)| format!("{}.{}", c, k))
            .collect();
        assert!(keys.contains(&"location.residence_city".to_string()));
        assert!(keys.contains(&"health.physical".to_string()));
        assert!(keys.contains(&"identity.name".to_string()));
        // The narrative home_address and non-address work_address rows are gone.
        assert!(!keys.contains(&"location.home_address".to_string()));
        assert!(!keys.contains(&"location.work_address".to_string()));
        // Narrative / non-address are gone.
        for (_, _, v) in &final_rows {
            assert!(!v.starts_with("User lives in Example City,"));
            assert!(!v.contains("uses Example Corp Copilot"));
        }
    }

    #[test]
    fn phase7_idempotent_second_run_is_no_op() {
        let conn = migrations_db_with_profile_rows(&[(
            "location",
            "home_address",
            "123 Example Pl SE, Example City WA",
            "auto",
        )]);
        let s1 = run_profile_registry_migration(&conn).unwrap();
        let s2 = run_profile_registry_migration(&conn).unwrap();
        assert_eq!(s1.canonicalized, 0);
        assert_eq!(s2.canonicalized, 0);
        assert_eq!(s2.merged_via_alias, 0);
        assert_eq!(s2.invalid_machine_deleted, 0);
    }

    #[test]
    fn phase7_mixed_timestamp_formats_order_correctly() {
        // Two rows aliasing to same canonical. Owner uses SQLite
        // `datetime('now')` format; legacy uses RFC3339. Owner is genuinely
        // NEWER by real clock time → owner must win despite format mismatch.
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        conn.execute("DELETE FROM user_profile", []).unwrap();
        // Canonical row, newer in real time (2026-04-12 10:00:00 UTC), SQLite format.
        conn.execute(
            "INSERT INTO user_profile (agent_did, category, key, value, confidence, source, updated_at) \
             VALUES ('did:aegis:telegram:7', 'location', 'residence_city', 'Current', 0.9, 'auto', '2026-04-12 10:00:00')",
            [],
        ).unwrap();
        // Legacy alias row, older (2020-01-01T00:00:00Z), RFC3339 format.
        conn.execute(
            "INSERT INTO user_profile (agent_did, category, key, value, confidence, source, updated_at) \
             VALUES ('did:aegis:telegram:7', 'location', 'home_location', 'Old', 0.9, 'auto', '2020-01-01T00:00:00Z')",
            [],
        ).unwrap();
        let stats = run_profile_registry_migration(&conn).unwrap();
        assert_eq!(stats.merged_via_alias, 1);
        let value: String = conn
            .query_row(
                "SELECT value FROM user_profile WHERE category='location' AND key='residence_city'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            value, "Current",
            "newer SQLite-format timestamp must win over older RFC3339"
        );
    }

    #[test]
    fn phase7_three_way_alias_collision_keeps_single_newest() {
        // TWO legacy aliases + ONE canonical, all for the same DID+slot, each
        // newer than the previous by several years. The in-memory index must
        // stay in sync as rows get merged so the newest wins.
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        conn.execute("DELETE FROM user_profile", []).unwrap();
        // Canonical (oldest).
        conn.execute(
            "INSERT INTO user_profile (agent_did, category, key, value, confidence, source, updated_at) \
             VALUES ('did:aegis:telegram:7', 'location', 'residence_city', 'v1', 0.9, 'auto', '2020-01-01T00:00:00Z')",
            [],
        ).unwrap();
        // Legacy alias 1 (middle).
        conn.execute(
            "INSERT INTO user_profile (agent_did, category, key, value, confidence, source, updated_at) \
             VALUES ('did:aegis:telegram:7', 'location', 'home_location', 'v2', 0.9, 'auto', '2022-01-01T00:00:00Z')",
            [],
        ).unwrap();
        // Legacy alias 2 (newest).
        conn.execute(
            "INSERT INTO user_profile (agent_did, category, key, value, confidence, source, updated_at) \
             VALUES ('did:aegis:telegram:7', 'location', 'residence', 'v3', 0.9, 'auto', '2026-04-12T00:00:00Z')",
            [],
        ).unwrap();
        let _ = run_profile_registry_migration(&conn).unwrap();
        let rows: Vec<(String, String, String)> = {
            let mut stmt = conn
                .prepare("SELECT category, key, value FROM user_profile")
                .unwrap();
            let it = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap();
            it.filter_map(|r| r.ok()).collect()
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "residence_city");
        assert_eq!(rows[0].2, "v3", "newest of three rows must win");
    }

    #[test]
    fn phase7_runs_via_run_all_migrations_end_to_end() {
        // End-to-end path: seed a legacy row before the migration marker is
        // written, call run_all_migrations, verify it ends up canonical and
        // the one-time marker survives subsequent startup cleanup.
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        conn.execute(
            "DELETE FROM skill_state WHERE skill_name = 'system' \
             AND state_key = 'profile_registry_migration_done'",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_profile (agent_did, category, key, value, confidence, source, updated_at) \
             VALUES ('did:aegis:telegram:9', 'location', 'home_location', 'City', 0.9, 'auto', datetime('now'))",
            [],
        ).unwrap();

        run_all_migrations(&conn).unwrap();
        let (cat, key): (String, String) = conn
            .query_row(
                "SELECT category, key FROM user_profile WHERE agent_did='did:aegis:telegram:9'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(cat, "location");
        assert_eq!(
            key, "residence_city",
            "legacy alias must be canonicalized end-to-end"
        );

        run_all_migrations(&conn).unwrap();
        let marker: String = conn
            .query_row(
                "SELECT state_value FROM skill_state WHERE skill_name = 'system' \
                 AND state_key = 'profile_registry_migration_done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(marker, "v1");
    }

    #[test]
    fn phase7_sweeps_migration_source_too() {
        // Tier-migration seeds rows with source='migration'. These are NOT
        // user-confirmed and should be swept by Pass 2 when their values
        // fail the validator, same as source='auto'. Only source='manual'
        // is trusted.
        let conn = migrations_db_with_profile_rows(&[
            // Narrative home_address seeded by tier_migration — invalid.
            (
                "location",
                "home_address",
                "User lives in Example City, has Asian straight hair, and prefers fade haircuts requiring maintenance every 2-3 weeks.",
                "migration",
            ),
            // Valid manual row — must survive.
            ("identity", "name", "Lance", "manual"),
        ]);
        let stats = run_profile_registry_migration(&conn).unwrap();
        assert!(stats.invalid_machine_deleted >= 1);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM user_profile", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "only the manual identity.name row should survive");
    }

    #[test]
    fn phase7_manual_source_rows_are_never_deleted() {
        // Even if a manual row's value fails the validator (e.g. narrative),
        // the migration must not touch it.
        let conn = migrations_db_with_profile_rows(&[(
            "location",
            "home_address",
            "User insists this is home",
            "manual",
        )]);
        let stats = run_profile_registry_migration(&conn).unwrap();
        assert_eq!(stats.invalid_machine_deleted, 0);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM user_profile", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    // ── Step 1: heartbeat_config fold migration ─────────────────────────────

    /// Helper: write a raw setting row as `store_setting` does (JSON-encoded value).
    fn write_setting(conn: &Connection, key: &str, value: serde_json::Value) {
        let json = value.to_string();
        conn.execute(
            "INSERT OR REPLACE INTO user_settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
            rusqlite::params![key, json],
        )
        .unwrap();
    }

    fn read_setting(conn: &Connection, key: &str) -> Option<serde_json::Value> {
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM user_settings WHERE key = ?1",
                rusqlite::params![key],
                |r| r.get(0),
            )
            .ok();
        raw.and_then(|s| serde_json::from_str(&s).ok())
    }

    #[test]
    fn heartbeat_config_fold_migration_folds_legacy_quiet_hours() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        // Pretend we boot an older DB: delete the heartbeat_config (migration
        // on the empty DB was a no-op since neither legacy key existed yet).
        conn.execute(
            "DELETE FROM user_settings WHERE key = 'heartbeat_config'",
            [],
        )
        .unwrap();
        // Seed legacy keys (JSON-encoded numbers, as store_setting would).
        write_setting(&conn, "notification_quiet_start", serde_json::json!(23));
        write_setting(&conn, "notification_quiet_end", serde_json::json!(7));
        // Sanity: heartbeat_config row is absent.
        assert!(read_setting(&conn, "heartbeat_config").is_none());

        run_heartbeat_config_fold_migration(&conn).unwrap();

        // New key should exist and reflect legacy hours (HH:00 format).
        let v = read_setting(&conn, "heartbeat_config")
            .expect("heartbeat_config row should be present after fold");
        let cfg = crate::heartbeat_config::HeartbeatConfig::from_json(&v).unwrap();
        assert_eq!(cfg.quiet_hours.start, "23:00");
        assert_eq!(cfg.quiet_hours.end, "07:00");
        // Other fields keep spec defaults.
        assert_eq!(cfg.frequency_minutes, 30);
        assert_eq!(cfg.per_tick.tool_calls, 8);

        // Legacy keys must be gone.
        assert!(read_setting(&conn, "notification_quiet_start").is_none());
        assert!(read_setting(&conn, "notification_quiet_end").is_none());

        // Audit row present with actor=migration and a reason describing the source.
        let (actor, reason): (String, Option<String>) = conn
            .query_row(
                "SELECT actor, reason FROM config_audit_log ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .unwrap();
        assert_eq!(actor, "migration");
        let reason_s = reason.unwrap_or_default();
        assert!(
            reason_s.contains("notification_quiet_start"),
            "reason should cite legacy source keys, got: {reason_s}"
        );
    }

    #[test]
    fn heartbeat_config_fold_migration_is_idempotent() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        conn.execute(
            "DELETE FROM user_settings WHERE key = 'heartbeat_config'",
            [],
        )
        .unwrap();
        write_setting(&conn, "notification_quiet_start", serde_json::json!(22));
        write_setting(&conn, "notification_quiet_end", serde_json::json!(6));

        run_heartbeat_config_fold_migration(&conn).unwrap();
        let audit_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM config_audit_log", [], |r| r.get(0))
            .unwrap();
        assert!(audit_before >= 1);

        // Second run: must be a no-op — new key is already set, so no new audit row.
        run_heartbeat_config_fold_migration(&conn).unwrap();
        let audit_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM config_audit_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            audit_after, audit_before,
            "second migration run must not insert an audit row"
        );
    }

    #[test]
    fn heartbeat_config_fold_noop_without_legacy_keys() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        // After first migration on an empty DB the new key was NOT seeded (no
        // legacy keys to fold). Confirm that, and that a second run is still a
        // no-op (doesn't synthesize a row from nothing).
        assert!(read_setting(&conn, "heartbeat_config").is_none());
        run_heartbeat_config_fold_migration(&conn).unwrap();
        assert!(read_setting(&conn, "heartbeat_config").is_none());
        let audit: i64 = conn
            .query_row("SELECT COUNT(*) FROM config_audit_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(audit, 0);
    }

    #[test]
    fn heartbeat_config_fold_keeps_defaults_when_legacy_disabled() {
        let conn = mem_db();
        run_all_migrations(&conn).unwrap();
        conn.execute(
            "DELETE FROM user_settings WHERE key = 'heartbeat_config'",
            [],
        )
        .unwrap();
        // Both legacy values set to the 255 sentinel (= disabled). parse_hour
        // returns None for both, so quiet_hours should stay on spec defaults.
        write_setting(&conn, "notification_quiet_start", serde_json::json!(255));
        write_setting(&conn, "notification_quiet_end", serde_json::json!(255));
        run_heartbeat_config_fold_migration(&conn).unwrap();
        let v = read_setting(&conn, "heartbeat_config").unwrap();
        let cfg = crate::heartbeat_config::HeartbeatConfig::from_json(&v).unwrap();
        assert_eq!(cfg.quiet_hours.start, "02:00");
        assert_eq!(cfg.quiet_hours.end, "07:00");
    }

    #[test]
    fn heartbeat_audit_step2_columns_present_after_migration() {
        // Step 2 (design §13): six new columns must exist on heartbeat_audit_log
        // after `run_all_migrations`.
        let conn = Connection::open_in_memory().unwrap();
        run_all_migrations(&conn).unwrap();

        let mut stmt = conn
            .prepare("PRAGMA table_info(heartbeat_audit_log)")
            .unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();

        for needle in [
            "outcome",
            "skip_reason",
            "config_snapshot_json",
            "prompt_hash",
            "proposals_created_json",
            "next_tick_note_bytes",
        ] {
            assert!(
                cols.iter().any(|c| c == needle),
                "expected column {needle} in heartbeat_audit_log; got {cols:?}"
            );
        }
    }

    #[test]
    fn heartbeat_audit_step2_migration_is_idempotent() {
        // Running migrations a second time on the same DB is a no-op
        // (ALTER TABLE … ADD COLUMN returns "duplicate column" which we
        // swallow via `.unwrap_or(())`).
        let conn = Connection::open_in_memory().unwrap();
        run_all_migrations(&conn).unwrap();
        // Should not panic / error.
        run_all_migrations(&conn).unwrap();
    }

    #[test]
    fn observed_events_table_present_after_migration() {
        // Step 3 (design §15.2): the observed_events table + unconsumed index
        // must exist after migrations run.
        let conn = Connection::open_in_memory().unwrap();
        run_all_migrations(&conn).unwrap();

        let col_names: Vec<String> = conn
            .prepare("PRAGMA table_info(observed_events)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        for needle in [
            "id",
            "at",
            "source",
            "severity",
            "summary",
            "payload_json",
            "consumed_at",
        ] {
            assert!(
                col_names.iter().any(|c| c == needle),
                "expected column {needle}; got {col_names:?}"
            );
        }

        let idx_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_observed_events_unconsumed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx_exists, 1, "unconsumed index should be present");
    }

    #[test]
    fn observed_events_migration_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_all_migrations(&conn).unwrap();
        run_all_migrations(&conn).unwrap();
    }

    #[test]
    fn action_ledger_table_present_after_migration() {
        let conn = Connection::open_in_memory().unwrap();
        run_all_migrations(&conn).unwrap();

        let col_names: Vec<String> = conn
            .prepare("PRAGMA table_info(action_ledger)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        for needle in [
            "id",
            "action_id",
            "action_kind",
            "status",
            "external_receipt",
            "external_message_id",
            "external_event_id",
            "tool_call_id",
            "provider_request_id",
            "idempotency_key",
            "trace_id",
            "run_id",
            "mandate_id",
            "source_event_id",
        ] {
            assert!(
                col_names.iter().any(|c| c == needle),
                "expected column {needle}; got {col_names:?}"
            );
        }
    }

    #[test]
    fn user_mandates_tables_present_after_migration() {
        let conn = Connection::open_in_memory().unwrap();
        run_all_migrations(&conn).unwrap();

        let mandate_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(user_mandates)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        for needle in [
            "mandate_id",
            "owner_principal",
            "status",
            "scope_kind",
            "allowed_reads_json",
            "allowed_actions_json",
            "must_ask_json",
            "source_event_id",
            "trace_id",
            "run_id",
            "schema_version",
        ] {
            assert!(
                mandate_cols.iter().any(|c| c == needle),
                "expected mandate column {needle}; got {mandate_cols:?}"
            );
        }

        let audit_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(user_mandate_audit)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        for needle in [
            "mandate_id",
            "owner_principal",
            "event_kind",
            "actor_principal",
            "status_from",
            "status_to",
            "summary",
            "schema_version",
        ] {
            assert!(
                audit_cols.iter().any(|c| c == needle),
                "expected mandate audit column {needle}; got {audit_cols:?}"
            );
        }

        let notification_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(user_mandate_notifications)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        for needle in [
            "mandate_id",
            "owner_principal",
            "source_event_id",
            "idempotency_key",
            "status",
            "notification_class",
            "privacy_level",
            "route_reason",
            "route_chat_id",
            "route_thread_id",
            "title",
            "body",
            "detail_url",
            "redacted",
            "trace_id",
            "run_id",
            "schema_version",
        ] {
            assert!(
                notification_cols.iter().any(|c| c == needle),
                "expected mandate notification column {needle}; got {notification_cols:?}"
            );
        }
    }
}

#[cfg(test)]
mod owner_did_tests {
    use super::owner_did_from;

    #[test]
    fn derives_from_admin_chat_id_exactly_like_the_old_literal() {
        // The original deployment sets ADMIN_CHAT_ID; the derived DID must be
        // byte-identical to the literal that used to be compiled in.
        assert_eq!(
            owner_did_from(None, Some("1000000001")),
            "did:aegis:telegram:1000000001"
        );
    }

    #[test]
    fn explicit_owner_did_wins() {
        assert_eq!(
            owner_did_from(Some("did:key:zX"), Some("1000000001")),
            "did:key:zX"
        );
    }

    #[test]
    fn neutral_when_nothing_is_configured() {
        assert_eq!(owner_did_from(None, None), "did:aegis:owner");
        assert_eq!(
            owner_did_from(Some("  "), Some("not-a-number")),
            "did:aegis:owner"
        );
    }
}
