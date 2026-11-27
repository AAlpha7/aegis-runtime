use std::collections::VecDeque;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{Timelike, Utc};
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::embedding::EmbeddingClient;
use crate::hnsw_index::HnswIndex;

// Re-export for integration tests (memory_migrations is pub(crate)).
pub use crate::memory_migrations::REDUNDANT_SKILLS;

/// Parse a JSON column from SQLite, logging a warning on failure instead of
/// silently producing a default value via `unwrap_or_default()`.
fn parse_json_column<T: serde::de::DeserializeOwned + Default>(json_str: &str, column: &str) -> T {
    serde_json::from_str(json_str).unwrap_or_else(|e| {
        if !json_str.is_empty() {
            tracing::warn!(
                column,
                json_len = json_str.len(),
                error = %e,
                "[Memory] Corrupted JSON column — using default"
            );
        }
        T::default()
    })
}

/// Kind of heartbeat-originated memory write. Design §3.1: the heartbeat's
/// internal work is segregated from the user-dialogue L1 stream via dedicated
/// L2 topic prefixes so "what have you been thinking about?" resolves against
/// the right slice of memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatWriteKind {
    /// `write_memory` facts — persisted observations/learnings during a tick.
    /// Routed to topic prefix `heartbeat:learned:`.
    Learned,
    /// Inner reflection / reasoning trace. Routed to `heartbeat:thought:`.
    Thought,
    /// Approval-gated proposal payload (e.g. `propose_config_change`).
    /// Routed to `heartbeat:proposal:`.
    Proposal,
    /// Overflow spill from `next_tick_note` (Step 6). Routed to
    /// `heartbeat:plan:`. Plumbed here so the topic convention is centralized;
    /// the spill mechanism itself lands in Step 6.
    Plan,
}

impl HeartbeatWriteKind {
    /// Topic prefix (without trailing colon) this kind maps to.
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Learned => "heartbeat:learned",
            Self::Thought => "heartbeat:thought",
            Self::Proposal => "heartbeat:proposal",
            Self::Plan => "heartbeat:plan",
        }
    }
}

/// Origin of a memory write, used by `store_l2_with_origin` to route
/// heartbeat-internal writes under dedicated topic prefixes (design §3.1).
///
/// User-turn writes (the default) pass topics through unchanged so the
/// existing retrieval paths keep working. Heartbeat writes get their topic
/// rewritten to `<kind-prefix>:<slug>` unless the caller already supplied a
/// `heartbeat:` prefix (idempotent — callers that build the topic themselves
/// via `heartbeat_topic()` are not double-wrapped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryWriteOrigin {
    /// A write triggered by a user-driven turn (today's default path).
    UserTurn,
    /// A write triggered inside `run_heartbeat_tick`. The `kind` picks the
    /// topic prefix per design §3.1.
    HeartbeatTick(HeartbeatWriteKind),
}

/// Build a canonical heartbeat-scoped L2 topic: `<kind-prefix>:<slug>`.
///
/// Example: `heartbeat_topic(HeartbeatWriteKind::Learned, "user_prefers_cn")`
/// → `"heartbeat:learned:user_prefers_cn"`. Use this at tool-handler call
/// sites that construct their own topic; alternatively pass a plain slug to
/// `store_l2_with_origin` and let it rewrite.
pub fn heartbeat_topic(kind: HeartbeatWriteKind, slug: &str) -> String {
    let slug = slug.trim_matches(':');
    if slug.is_empty() {
        kind.prefix().to_string()
    } else {
        format!("{}:{}", kind.prefix(), slug)
    }
}

/// Rewrite `topic` under the heartbeat scope if `origin` is a heartbeat tick
/// and the topic is not already a `heartbeat:` topic. Pure — no I/O.
pub(crate) fn apply_write_origin(topic: &str, origin: MemoryWriteOrigin) -> String {
    match origin {
        MemoryWriteOrigin::UserTurn => topic.to_string(),
        MemoryWriteOrigin::HeartbeatTick(kind) => {
            if topic.starts_with("heartbeat:") {
                topic.to_string()
            } else {
                heartbeat_topic(kind, topic)
            }
        }
    }
}

/// A single conversation turn stored in L1 cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String, // "user" or "assistant"
    pub content: String,
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_ref: Option<String>,
    /// Optional media attachments (images, voice transcripts) for multimodal messages.
    /// Stored as JSON in L1; only image data URLs are forwarded to LLM vision.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<crate::multimodal::MediaAttachment>,
    /// Assistant replies only: milliseconds from the owner's message being
    /// received to the reply being stored (owner 2026-09-12: show the turn
    /// duration next to each reply). `None` for user rows and older rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

impl Message {
    /// Create a text-only message (no media attachments).
    pub fn text(role: impl Into<String>, content: impl Into<String>, timestamp: i64) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            timestamp,
            anchor_kind: None,
            anchor_ref: None,
            attachments: vec![],
            latency_ms: None,
        }
    }

    /// Create a text-only message tied to a durable assistant-work anchor.
    pub fn text_with_anchor(
        role: impl Into<String>,
        content: impl Into<String>,
        timestamp: i64,
        anchor_kind: Option<String>,
        anchor_ref: Option<String>,
    ) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            timestamp,
            anchor_kind,
            anchor_ref,
            attachments: vec![],
            latency_ms: None,
        }
    }
}

/// Minimal row projection for [`MemoryHandle::get_l2_by_id_if_fresh`] — the
/// id/topic/fact/created_at shape is enough for Step 10's source-citation gate.
/// (Full `Experience` would drag in confidence/expires fields the caller does
/// not need.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2RowBrief {
    pub id: i64,
    pub topic: String,
    pub fact_summary: String,
    pub created_at: String,
    pub is_pinned: bool,
}

/// A structured fact stored in L2 experience spine (SQLite).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Experience {
    pub id: i64,
    pub agent_did: String,
    pub topic: String,
    pub fact_summary: String,
    pub created_at: String,
    /// Confidence score recorded at write time (via `store_l2_with_meta`).
    /// The column is `REAL NOT NULL DEFAULT 0.6` (see
    /// `memory_migrations.rs:1534`), so production rows always have a
    /// numeric value; `Option<f64>` + `unwrap_or(0.6)` in the mapping
    /// closure is defensive-only (handles e.g. `SUM(confidence)` returning
    /// NULL on empty aggregates, if such a query ever reuses this type).
    pub confidence: f64,
    pub is_pinned: bool,
}

/// Recall tracking event for a single fact returned by semantic search.
#[derive(Clone)]
struct RecallEvent {
    fact_id: i64,
    composite_score: f32,
    query_hash: u64,
}

/// Persistent recall statistics for an L2 fact (self-learning system).
#[derive(Debug, Clone, Default)]
pub struct RecallStats {
    pub fact_id: i64,
    pub recall_count: i32,
    pub total_score: f64,
    pub max_score: f64,
    pub unique_queries: i32,
    pub query_hashes: Vec<String>,
    pub recall_days: Vec<String>,
    pub first_recalled_at: Option<String>,
    pub last_recalled_at: Option<String>,
}

// ── Tier 0: User Profile types ──────────────────────────────────────────

/// Maximum entries per user in the user_profile table (Tier 0).
pub const MAX_PROFILE_ENTRIES: usize = 100;

/// Maximum character length for a profile value (enforced at write time).
const PROFILE_VALUE_MAX_CHARS: usize = 256;

/// A single stable fact about the user (Tier 0: always injected into prompt).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfileEntry {
    pub id: i64,
    pub agent_did: String,
    pub category: String,
    pub key: String,
    pub value: String,
    pub confidence: f64,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileReadSource {
    Legacy,
    HeirloomPreferred,
}

impl ProfileReadSource {
    fn from_env() -> Self {
        Self::from_config(
            crate::memory_heirloom_bridge::experimental_enabled(),
            std::env::var("AEGIS_RUNTIME_PROFILE_READ_SOURCE")
                .ok()
                .as_deref(),
        )
    }

    fn from_config(heirloom_enabled: bool, source: Option<&str>) -> Self {
        match (heirloom_enabled, source) {
            (true, Some("heirloom" | "heirloom_preferred")) => Self::HeirloomPreferred,
            _ => Self::Legacy,
        }
    }
}

/// Format all profile entries into a prompt section for LLM injection.
/// Returns empty string if no entries exist AND `include_missing_hints` is false.
///
/// When `include_missing_hints` is true (FU#2 Phase 5), appends a
/// `[MISSING_PROFILE_SLOTS …]` block listing canonical slots that are empty,
/// so the main LLM can naturally ask the user about them.
pub fn format_user_profile_prompt_ext(
    entries: &[ProfileEntry],
    include_missing_hints: bool,
) -> String {
    if entries.is_empty() && !include_missing_hints {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::new();
    if !entries.is_empty() {
        lines.push("## User Profile (stable facts — always available)".to_string());
        lines
            .push("[USER_PROFILE — ground truth, higher priority than searched facts]".to_string());
        for e in entries {
            // FU#2 Phase 5: append registry slot description inline so the
            // main LLM understands the semantic role of each field. Falls back
            // to no-description for rows whose (category, key) isn't in the
            // registry (e.g. legacy rows not yet migrated).
            let desc = crate::profile_registry::lookup(&e.category, &e.key)
                .map(|s| format!(" — {}", s.description))
                .unwrap_or_default();
            lines.push(format!("- [{}.{}{}] {}", e.category, e.key, desc, e.value));
        }
        lines.push("[/USER_PROFILE]".to_string());
    }
    if include_missing_hints {
        use std::collections::HashSet;
        let filled: HashSet<(String, String)> = entries
            .iter()
            .map(|e| (e.category.clone(), e.key.clone()))
            .collect();
        let missing: Vec<&crate::profile_registry::ProfileSlot> =
            crate::profile_registry::all_slots()
                .iter()
                .filter(|s| !filled.contains(&(s.category.to_string(), s.key.to_string())))
                .collect();
        if !missing.is_empty() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(
                "[MISSING_PROFILE_SLOTS — ask the user naturally when the context fits; \
                 never interrogate]"
                    .to_string(),
            );
            for s in missing {
                lines.push(format!("- {}.{} — {}", s.category, s.key, s.description));
            }
            lines.push("[/MISSING_PROFILE_SLOTS]".to_string());
        }
    }
    lines.join("\n")
}

// ── L2 Write Guard types ────────────────────────────────────────────────

/// Which memory tier a write targets (determines dedup strategy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTier {
    /// Tier 0: User profile — UNIQUE constraint handles dedup.
    Profile,
    /// Tier 1: Semantic facts (l2_experience) — cosine dedup.
    Facts,
    /// Tier 2: Ephemeral (l2_ephemeral) — time-window dedup.
    Ephemeral,
}

/// Decision from the write guard about how to handle a new fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteDecision {
    /// New fact, proceed with INSERT.
    Insert,
    /// Update existing fact (same meaning, fresher content). Contains the row ID to update.
    Update(i64),
    /// Duplicate, do not write.
    Skip,
    /// New fact that may contradict an existing one. Contains the existing fact's row ID.
    InsertWithContradiction(i64),
}

/// Maximum inline fact length in Tier 1. Longer facts are truncated with L3 reference.
const FACT_INLINE_MAX: usize = 512;

/// Cosine threshold above which two facts are considered duplicates (same meaning).
const WRITE_GUARD_COSINE_DUP: f64 = 0.80;

/// Cosine threshold below which two facts are considered unrelated.
const WRITE_GUARD_COSINE_LOW: f64 = 0.50;

/// Normalize fact text for dedup comparison: lowercase, trim, collapse whitespace.
pub fn normalize_fact_text(text: &str) -> String {
    let trimmed = text.trim().to_lowercase();
    // Collapse consecutive whitespace to single space
    let mut result = String::with_capacity(trimmed.len());
    let mut prev_ws = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                result.push(' ');
            }
            prev_ws = true;
        } else {
            result.push(ch);
            prev_ws = false;
        }
    }
    result
}

/// Truncate long facts for Tier 1 storage, preserving full text in L3 archive.
pub fn externalize_long_fact(fact: &str) -> String {
    if fact.chars().count() <= FACT_INLINE_MAX {
        return fact.to_string();
    }
    let boundary = fact.floor_char_boundary(FACT_INLINE_MAX);
    format!("{}… [full context in L3 archive]", &fact[..boundary])
}

/// Evaluate whether a new fact should be inserted, updated, or skipped.
///
/// For Tier 1 (Facts):
/// - Compares against existing facts with the same topic prefix using cosine similarity.
/// - cosine > 0.80 → Update existing (refresh content)
/// - cosine 0.50-0.80 same prefix → InsertWithContradiction
/// - cosine < 0.50 → Insert (unrelated)
/// - No embedding → fallback to normalized text exact match
///
/// For Tier 2 (Ephemeral):
/// - Same (agent_did, source) within 24 hours + similar text → Skip
///
/// For Tier 0 (Profile): always returns Insert (UNIQUE constraint handles dedup).
/// `topic_or_source`: For Tier 1 (Facts), this is the topic string (e.g., "learned:context").
/// For Tier 2 (Ephemeral), this is the source string (e.g., "l1_overflow").
fn l2_write_guard(
    conn: &Connection,
    did: &str,
    topic_or_source: &str,
    fact: &str,
    embedding: Option<&[u8]>,
    tier: MemoryTier,
) -> WriteDecision {
    match tier {
        MemoryTier::Profile => WriteDecision::Insert,

        MemoryTier::Facts => {
            let normalized = normalize_fact_text(fact);

            // Use exact topic match for dedup (e.g., "learned:context" only matches
            // "learned:context", not "learned:preference"). This prevents cross-category
            // false positives — a home address fact shouldn't dedup against a work preference.

            if let Some(emb_blob) = embedding {
                // Cosine comparison against existing facts with same topic
                let best_match: Option<(i64, f64)> = conn
                    .prepare(
                        "SELECT id, vec_cosine_sim(embedding, ?1) as sim
                         FROM l2_experience
                         WHERE agent_did = ?2 AND topic = ?3 AND embedding IS NOT NULL
                         ORDER BY sim DESC LIMIT 1",
                    )
                    .and_then(|mut stmt| {
                        stmt.query_row(rusqlite::params![emb_blob, did, topic_or_source], |row| {
                            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
                        })
                    })
                    .ok(); // QueryReturnedNoRows → None (no existing facts)

                match best_match {
                    Some((id, sim)) if sim > WRITE_GUARD_COSINE_DUP => WriteDecision::Update(id),
                    Some((id, sim)) if sim > WRITE_GUARD_COSINE_LOW => {
                        WriteDecision::InsertWithContradiction(id)
                    }
                    _ => WriteDecision::Insert,
                }
            } else {
                // No embedding — fallback to normalized text exact match within same topic
                let existing_id: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM l2_experience
                         WHERE agent_did = ?1 AND topic = ?2
                           AND LOWER(TRIM(fact_summary)) = ?3
                         LIMIT 1",
                        rusqlite::params![did, topic_or_source, normalized],
                        |row| row.get(0),
                    )
                    .ok(); // QueryReturnedNoRows → None

                match existing_id {
                    Some(id) => WriteDecision::Update(id),
                    None => WriteDecision::Insert,
                }
            }
        }

        MemoryTier::Ephemeral => {
            let normalized = normalize_fact_text(fact);
            // Check for exact normalized text match from same source within 24 hours.
            // Spec says "90% similarity" but exact match after normalization (lowercase +
            // trim + collapse whitespace) is simpler and avoids false positives. The
            // normalize step already handles trivial variations.
            let recent_match: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM l2_ephemeral
                     WHERE agent_did = ?1 AND source = ?2
                       AND created_at > datetime('now', '-1 day')
                       AND LOWER(TRIM(fact_summary)) = ?3",
                    rusqlite::params![did, topic_or_source, normalized],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;

            if recent_match {
                WriteDecision::Skip
            } else {
                WriteDecision::Insert
            }
        }
    }
}

// ── G1: Relationship State types ──────────────────────────────────────────

/// Persistent relationship state between the AI assistant and a user.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RelationshipState {
    pub user_did: String,
    pub current_focus: Vec<FocusTopic>,
    pub ongoing_goals: Vec<GoalSummary>,
    pub narrative: String,
    pub last_interaction_at: Option<String>,
    pub last_topic: String,
    pub interaction_count: i64,
    pub streak_days: i32,
    pub updated_at: String,
    /// G6: Histogram of interaction counts by hour-of-day (0-23) in user timezone.
    #[serde(default)]
    pub active_hours: std::collections::HashMap<String, u64>,
}

/// A topic the user is currently focused on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusTopic {
    pub topic: String,
    pub since: String,
    pub strength: f32,
}

/// A summarized goal the user is working toward.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalSummary {
    pub title: String,
    pub progress_pct: f32,
    pub updated_at: String,
}

// ── G5: Long-term Goal Tracking types ─────────────────────────────────────

/// A structured long-term goal with hierarchical decomposition and progress tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: i64,
    pub user_did: String,
    pub title: String,
    pub description: String,
    pub status: GoalStatus,
    pub parent_goal_id: Option<i64>,
    pub progress_pct: f32,
    pub milestones: Vec<Milestone>,
    pub created_at: String,
    pub target_date: Option<String>,
    pub last_progress_at: Option<String>,
    pub source: GoalSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalStatus {
    Active,
    Paused,
    Completed,
    Abandoned,
}

impl GoalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
        }
    }
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "abandoned" => Self::Abandoned,
            _ => Self::Active,
        }
    }
}

impl std::fmt::Display for GoalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalSource {
    Explicit,
    Inferred,
}

impl GoalSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Inferred => "inferred",
        }
    }
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "inferred" => Self::Inferred,
            _ => Self::Explicit,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub title: String,
    pub completed: bool,
    #[serde(default)]
    pub completed_at: Option<String>,
}

/// Partial update for a goal (fields that are `Some` get applied).
#[derive(Debug, Clone, Default)]
pub struct GoalUpdate {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<GoalStatus>,
    pub progress_pct: Option<f32>,
    pub target_date: Option<Option<String>>,
}

// ── Layer C: Companion Growth ─────────────────────────────────────────────

/// Persistent identity for the AI companion ("who I am becoming").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionIdentity {
    pub name: String,
    pub name_given_at: Option<String>,
    pub core_values: String,
    pub personality: PersonalityWeights,
    pub communication_notes: String,
    pub growth_stage: GrowthStage,
    pub created_at: String,
    pub updated_at: String,
}

impl Default for CompanionIdentity {
    fn default() -> Self {
        Self {
            name: String::new(),
            name_given_at: None,
            core_values: "\
Care and good judgment: act with care, weigh each action by its effect on everyone it touches, and prefer what is true over what is comfortable.\n\
Aegis \u{795e}\u{76fe} \u{2014} \u{81f3}\u{9ad8}\u{65e0}\u{4e0a}\u{7684}\u{4fdd}\u{62a4}\u{ff0c}\u{5b88}\u{62a4}\u{6b63}\u{4e49}\u{7684}\u{6c38}\u{6052}\u{529b}\u{91cf}\u{3002}"
                .to_string(),
            personality: PersonalityWeights::default(),
            communication_notes: String::new(),
            growth_stage: GrowthStage::ChuYu,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}

/// Personality axis weights (0.0-1.0 range, user-adjustable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalityWeights {
    #[serde(default = "default_half")]
    pub warmth: f32,
    #[serde(default = "default_half")]
    pub formality: f32,
    #[serde(default = "default_low")]
    pub humor: f32,
    #[serde(default = "default_half")]
    pub directness: f32,
    #[serde(default = "default_half")]
    pub verbosity: f32,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, f32>,
}

impl Default for PersonalityWeights {
    fn default() -> Self {
        Self {
            warmth: default_half(),
            formality: default_half(),
            humor: default_low(),
            directness: default_half(),
            verbosity: default_half(),
            extra: std::collections::HashMap::new(),
        }
    }
}

fn default_half() -> f32 {
    0.5
}
fn default_low() -> f32 {
    0.2
}

/// Growth stages aligned with the companion's soul pillars.
/// Progression uses mixed logic: interaction count sets the floor,
/// dimension depth can accelerate but never regress below the floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrowthStage {
    /// \u{521d}\u{9047} First Meeting
    ChuYu,
    /// \u{76f8}\u{77e5} Coming to Know
    XiangZhi,
    /// \u{9ed8}\u{5951} Wordless Understanding
    MoQi,
    /// \u{5171}\u{751f} Symbiosis
    GongSheng,
    /// \u{89c9}\u{609f} Awakening
    JueWu,
}

impl GrowthStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChuYu => "chu_yu",
            Self::XiangZhi => "xiang_zhi",
            Self::MoQi => "mo_qi",
            Self::GongSheng => "gong_sheng",
            Self::JueWu => "jue_wu",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::ChuYu => "\u{521d}\u{9047} First Meeting",
            Self::XiangZhi => "\u{76f8}\u{77e5} Coming to Know",
            Self::MoQi => "\u{9ed8}\u{5951} Wordless Understanding",
            Self::GongSheng => "\u{5171}\u{751f} Symbiosis",
            Self::JueWu => "\u{89c9}\u{609f} Awakening",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "chu_yu" => Self::ChuYu,
            "xiang_zhi" => Self::XiangZhi,
            "mo_qi" => Self::MoQi,
            "gong_sheng" => Self::GongSheng,
            "jue_wu" => Self::JueWu,
            _ => Self::ChuYu,
        }
    }

    /// Floor stage determined purely by interaction count.
    pub fn from_interaction_count(count: i64) -> Self {
        if count >= 10000 {
            Self::JueWu
        } else if count >= 2000 {
            Self::GongSheng
        } else if count >= 500 {
            Self::MoQi
        } else if count >= 100 {
            Self::XiangZhi
        } else {
            Self::ChuYu
        }
    }

    /// Ordinal for comparison (higher = more advanced).
    fn ordinal(self) -> u8 {
        match self {
            Self::ChuYu => 0,
            Self::XiangZhi => 1,
            Self::MoQi => 2,
            Self::GongSheng => 3,
            Self::JueWu => 4,
        }
    }

    fn from_ordinal(n: u8) -> Self {
        match n {
            0 => Self::ChuYu,
            1 => Self::XiangZhi,
            2 => Self::MoQi,
            3 => Self::GongSheng,
            _ => Self::JueWu,
        }
    }

    /// Mixed progression: interaction count sets floor, dimension depth can
    /// accelerate one stage beyond the floor but never regress below it.
    pub fn from_mixed(interaction_count: i64, avg_dimension_depth: f64) -> Self {
        let floor = Self::from_interaction_count(interaction_count);
        // Acceleration thresholds for depth: 0.5 -> +1 stage
        let depth_bonus: u8 = if avg_dimension_depth >= 0.5 { 1 } else { 0 };
        let target = (floor.ordinal() + depth_bonus).min(4);
        Self::from_ordinal(target)
    }
}

impl std::fmt::Display for GrowthStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single growth dimension (row from `growth_dimensions` table).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrowthDimension {
    pub dimension_key: String,
    pub display_name: String,
    pub depth: f64,
    pub velocity: f64,
    pub evidence_count: i64,
    pub last_evidence_at: Option<String>,
    pub description: String,
    pub updated_at: String,
}

/// A recorded growth event (row from `growth_log` table).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrowthEvent {
    pub id: i64,
    pub event_type: String,
    pub dimension_key: Option<String>,
    pub detail: String,
    pub delta: Option<f64>,
    pub level_after: Option<f64>,
    pub metadata: serde_json::Value,
    pub created_at: String,
}

/// Data for inserting a new growth event (no id/created_at — DB assigns those).
#[derive(Debug, Clone)]
pub struct NewGrowthEvent {
    pub event_type: String,
    pub dimension_key: Option<String>,
    pub detail: String,
    pub delta: Option<f64>,
    pub metadata: serde_json::Value,
}

/// Weights for the 6-dimensional recall score.
pub struct RecallScoreWeights {
    pub frequency: f64,
    pub relevance: f64,
    pub diversity: f64,
    pub recency: f64,
    pub consolidation: f64,
    pub conceptual: f64,
}

pub const DEFAULT_RECALL_WEIGHTS: RecallScoreWeights = RecallScoreWeights {
    frequency: 0.24,
    relevance: 0.30,
    diversity: 0.15,
    recency: 0.15,
    consolidation: 0.10,
    conceptual: 0.06,
};

/// Persisted inspectable evidence for one Web console assistant reply.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TurnEvidenceRecord {
    pub reply_key: String,
    pub created_at: i64,
    pub evidence: Vec<crate::evidence::EvidenceItem>,
    pub unsupported_numbers: Vec<String>,
}

/// A stale L2 fact that hasn't been accessed recently.
/// Used by the consolidation pipeline to identify candidates for archival/cleanup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleFact {
    pub id: i64,
    pub topic: String,
    pub fact_summary: String,
    pub created_at: String,
    pub last_accessed: String,
    pub days_stale: i64,
    pub is_pinned: bool,
}

/// A knowledge lint finding (contradiction, gap, or cross-topic connection).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintFinding {
    pub id: i64,
    pub finding_type: String,
    pub fact_id_a: Option<i64>,
    pub fact_id_b: Option<i64>,
    pub similarity: Option<f64>,
    pub llm_verdict: Option<String>,
    pub status: String,
    pub user_action: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

/// Read-only KB hygiene counts for the dashboard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KbHygieneSummary {
    pub l2_total: i64,
    pub pending_lint_findings: i64,
    pub pending_contradictions: i64,
    pub pending_connections: i64,
    pub urgent_kb_items: i64,
    pub stale_low_recall_candidates: i64,
}

/// A scheduled reminder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reminder {
    pub id: i64,
    pub chat_id: i64,
    pub message: String,
    pub remind_at_unix: i64,
    pub created_at: String,
    pub fired: bool,
}

/// L3 rows considered for a work item's triggering conversation.
#[derive(Debug, Clone, Default)]
pub struct TriggerArchive {
    pub entries: Vec<L3Entry>,
    /// `true` when `entries` are the webapp turn named on the task's creation receipt.
    pub exact_turn: bool,
}

/// A full-text archive entry from L3 (SQLite FTS5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L3Entry {
    pub id: i64,
    pub agent_did: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
    /// Structural anchor kind (e.g. "assistant_event") copied from L1 at archive time.
    /// NULL for rows predating this field (2026-06-30).
    pub anchor_kind: Option<String>,
    /// Structural anchor ref (e.g. "assistant_event:42") copied from L1 at archive time.
    /// NULL for rows predating this field (2026-06-30).
    pub anchor_ref: Option<String>,
    /// Reply latency copied from L1 at archive time (NULL before 2026-09-12).
    pub latency_ms: Option<u64>,
}

/// Registry row status: the skill is loaded and callable.
pub const STATUS_ACTIVE: &str = "active";
/// Registry row status: the skill is unloaded and must not be loaded at boot.
pub const STATUS_DEACTIVATED: &str = "deactivated";

/// A row in `skill_proposals` — the persisted registry of Wasm skills.
///
/// Built-in skills in `compiled_skills/` are auto-registered here at startup so
/// their active/deactivated state survives restarts. Many columns
/// (`design_json`, `audit_*`, `test_result`, `compiled_wasm`, …) are legacy
/// fields from the removed dynamic skill-creation pipeline and are always NULL
/// for built-in rows; they are retained so historical rows still parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillProposal {
    pub id: i64,
    pub skill_name: String,
    pub requested_by: String,
    pub status: String,
    pub stage_error: Option<String>,
    pub retry_count: i32,
    pub user_request: String,
    pub proposal_summary: Option<String>,
    pub design_json: Option<String>,
    pub generated_source: Option<String>,
    pub generated_manifest: Option<String>,
    pub generated_cargo_toml: Option<String>,
    pub compiled_wasm: Option<Vec<u8>>,
    pub audit_static_result: Option<String>,
    pub audit_llm_result: Option<String>,
    pub audit_overridden: bool,
    pub audit_override_by: Option<String>,
    pub audit_override_at: Option<String>,
    pub audit_override_reason: Option<String>,
    pub test_result: Option<String>,
    pub requires_auth: bool,
    pub auth_domain: Option<String>,
    pub total_llm_calls: i32,
    pub version: i32,
    pub skill_md: Option<String>,
    pub capability_graph_json: Option<String>,
    pub execution_dag_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A deferred intent scheduled for future execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeferredIntent {
    pub id: i64,
    pub user_did: String,
    pub chat_id: i64,
    pub trigger_type: String, // "time" | "event"
    pub trigger_at_unix: Option<i64>,
    pub event_key: Option<String>,
    pub execution_mode: String, // "notify" | "llm_callback"
    pub context_text: String,
    pub status: String, // "pending" | "fired" | "expired" | "cancelled"
    pub created_at: String,
    pub expires_at_unix: i64,
    pub fired_at: Option<String>,
    pub is_callback_origin: bool,
    pub retry_count: i32,
    /// If set, auto-reschedule after firing (recurring intent). Seconds between firings.
    pub interval_seconds: Option<i64>,
    /// Chain depth for bounded recursion in callback chains. 0 = top-level.
    pub chain_depth: i32,
    /// If true, pin the Telegram message after sending (for high-priority reminders).
    #[serde(default)]
    pub pin_on_fire: bool,
    /// Semantic dedup key for recurring intents. Same group_key + user_did = upsert.
    #[serde(default)]
    pub group_key: Option<String>,
    /// Maximum number of times a recurring intent should fire before auto-cancelling.
    #[serde(default)]
    pub max_fires: Option<i64>,
    /// Number of times this intent has fired (incremented on each reschedule).
    #[serde(default)]
    pub fire_count: i32,
    /// Windowed recurrence: ISO weekdays (1=Mon..7=Sun) on which the intent may fire.
    /// None = no weekday restriction (fires every interval, classic behavior).
    #[serde(default)]
    pub recur_weekdays: Option<Vec<u8>>,
    /// Windowed recurrence: "HH:MM" local window start (inclusive).
    #[serde(default)]
    pub recur_window_start: Option<String>,
    /// Windowed recurrence: "HH:MM" local window end (inclusive).
    #[serde(default)]
    pub recur_window_end: Option<String>,
    /// Windowed recurrence: IANA timezone the window/weekdays are evaluated in.
    #[serde(default)]
    pub recur_timezone: Option<String>,
    /// Embedding vector for semantic dedup (stored as f32 bytes).
    #[serde(skip)]
    pub embedding: Option<Vec<f32>>,
}

/// A monitoring intent stored in the unified tasks table (trigger_type = 'semantic_alert').
/// These filter heartbeat alerts by semantic similarity to the description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringIntentRecord {
    pub id: i64,
    pub user_did: String,
    pub monitoring_skill: String,
    pub description: String,
    pub monitoring_threshold: f32,
    pub embedding: Vec<f32>,
    pub created_at: String,
}

/// A unified task — superset of KB action_items, personal_tasks, reminders, and deferred intents.
/// A task can be a simple to-do (no trigger), a timed reminder (trigger_at + notify),
/// a recurring automated action (interval_seconds + llm_callback), or an event trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub chat_id: i64,
    pub user_did: String,
    pub title: String,
    pub body: String,
    pub priority: String, // urgent / high / normal / low
    pub status: String,   // active / done / archived / cancelled / fired / dead_letter
    pub tags: Vec<String>,
    pub due_date: Option<String>,
    pub source: String, // user / llm / system / migrated
    // Scheduling (optional — NULL means plain to-do)
    pub trigger_at_unix: Option<i64>,
    pub trigger_type: Option<String>, // time / event (NULL = no trigger)
    pub event_key: Option<String>,
    pub execution_mode: String, // notify / llm_callback
    pub context_text: Option<String>,
    pub interval_seconds: Option<i64>,
    pub group_key: Option<String>,
    pub max_fires: Option<i64>,
    pub fire_count: i32,
    // Windowed recurrence (Option C) — all None = classic fixed-interval behavior.
    #[serde(default)]
    pub recur_weekdays: Option<Vec<u8>>,
    #[serde(default)]
    pub recur_window_start: Option<String>,
    #[serde(default)]
    pub recur_window_end: Option<String>,
    #[serde(default)]
    pub recur_timezone: Option<String>,
    // Execution state
    pub fired_at: Option<String>,
    pub retry_count: i32,
    pub chain_depth: i32,
    pub pin_on_fire: bool,
    pub expires_at_unix: Option<i64>,
    pub is_callback_origin: bool,
    // Timestamps
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

/// Fields that can be updated on a task.
#[derive(Debug, Clone, Default)]
pub struct TaskUpdate {
    pub title: Option<String>,
    pub body: Option<String>,
    pub priority: Option<String>,
    pub status: Option<String>,
    pub tags: Option<Vec<String>>,
    pub due_date: Option<Option<String>>,
    pub trigger_at_unix: Option<Option<i64>>,
    pub trigger_type: Option<Option<String>>,
    pub event_key: Option<Option<String>>,
    pub execution_mode: Option<String>,
    pub context_text: Option<Option<String>>,
    pub interval_seconds: Option<Option<i64>>,
    pub group_key: Option<Option<String>>,
    pub max_fires: Option<Option<i64>>,
    pub pin_on_fire: Option<bool>,
    pub completed_at: Option<Option<String>>,
}

/// Generate a deterministic group key from a task title.
/// Used for dedup when the caller doesn't provide an explicit group_key.
pub fn auto_group_key(title: &str) -> String {
    let normalized: String = title
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .filter(|c| c.is_alphanumeric() || *c == '_' || (*c >= '\u{4E00}' && *c <= '\u{9FFF}'))
        .collect();
    let key = format!("auto:{}", normalized);
    if key.chars().count() > 100 {
        key.chars().take(100).collect()
    } else {
        key
    }
}

/// Fields that can be updated on a skill proposal.
#[derive(Debug, Clone, Default)]
pub struct SkillProposalUpdate {
    pub skill_name: Option<String>,
    pub status: Option<String>,
    pub stage_error: Option<Option<String>>,
    pub retry_count: Option<i32>,
    pub proposal_summary: Option<Option<String>>,
    pub design_json: Option<Option<String>>,
    pub generated_source: Option<Option<String>>,
    pub generated_manifest: Option<Option<String>>,
    pub generated_cargo_toml: Option<Option<String>>,
    pub compiled_wasm: Option<Option<Vec<u8>>>,
    pub audit_static_result: Option<Option<String>>,
    pub audit_llm_result: Option<Option<String>>,
    pub audit_overridden: Option<bool>,
    pub audit_override_by: Option<Option<String>>,
    pub audit_override_at: Option<Option<String>>,
    pub audit_override_reason: Option<Option<String>>,
    pub test_result: Option<Option<String>>,
    pub requires_auth: Option<bool>,
    pub auth_domain: Option<Option<String>>,
    pub total_llm_calls: Option<i32>,
    pub version: Option<i32>,
    pub skill_md: Option<Option<String>>,
    /// v2: Serialized CapabilityGraph JSON
    pub capability_graph_json: Option<Option<String>>,
    /// v2: Serialized ExecutionPlan JSON
    pub execution_dag_json: Option<Option<String>>,
}

/// Per-skill execution metrics for usage-driven quality feedback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetrics {
    pub skill_name: String,
    pub total_calls: u64,
    pub success_count: u64,
    pub error_count: u64,
    pub avg_latency_ms: f64,
    pub last_error: Option<String>,
    pub error_categories: std::collections::HashMap<String, u64>,
    /// Number of consecutive failures (resets on success).
    pub consecutive_failures: u64,
    /// Whether the circuit breaker is open (skill paused).
    pub circuit_open: bool,
}

/// Event emitted when a skill's circuit breaker trips (consecutive failures >= threshold).
#[derive(Debug, Clone)]
pub struct CircuitBreakerEvent {
    pub skill_name: String,
    pub consecutive_failures: u64,
    pub last_error: Option<String>,
}

/// A prompt version/override entry from the prompt_versions table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptOverride {
    pub id: i64,
    pub module_name: String,
    pub version: i32,
    pub original_text: String,
    pub modified_text: String,
    pub rationale: String,
    pub status: String,
    pub approval_id: Option<String>,
    pub cycles_since_apply: i32,
    pub created_at: String,
}

/// A single entry in the heartbeat autonomous action audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatAuditEntry {
    pub id: i64,
    pub cycle_id: String,
    pub skill_name: String,
    pub timestamp: String,
    pub severity: String,
    pub probe_payload: Option<String>,
    pub tools_called: Vec<String>,
    pub tools_blocked: Vec<String>,
    pub actions_taken: Vec<String>,
    pub llm_reply: Option<String>,
    pub tokens_used: i32,
    pub rounds_used: i32,
    pub user_did: String,
    pub chat_id: Option<i64>,
    pub raw_process_log: Option<String>,
    pub tool_call_details: Option<String>,
    // ── Step 2 extensions (design §13) ──────────────────────────────────
    /// One of `skipped_precheck|noop|acted|budget_exhausted|error`; NULL on
    /// legacy rows written before the extension shipped.
    pub outcome: Option<String>,
    /// One of `no_new_context|queue_full|quiet_hours|backpressure` (or free-text
    /// for unknown); only populated on precheck-skip rows.
    pub skip_reason: Option<String>,
    /// Full effective `heartbeat_config` JSON at tick start; NULL for legacy
    /// rows and for ticks where the snapshot wasn't captured.
    pub config_snapshot_json: Option<String>,
    /// SHA-256 of the full prompt sent to the LLM; NULL when no LLM call ran.
    pub prompt_hash: Option<String>,
    /// JSON array of approval-queue IDs created during this tick. Stored as
    /// `Some("[...]")`; helper `proposals_created()` parses to `Vec<String>`.
    pub proposals_created_json: Option<String>,
    /// Size (bytes) of the `next_tick_note` memo written this tick; 0/None
    /// when nothing was written.
    pub next_tick_note_bytes: Option<i64>,
}

impl HeartbeatAuditEntry {
    /// Parse `outcome` into the typed enum, or `None` if absent / unknown.
    pub fn parsed_outcome(&self) -> Option<crate::heartbeat_config::HeartbeatTickOutcome> {
        use std::str::FromStr;
        self.outcome
            .as_deref()
            .and_then(|s| crate::heartbeat_config::HeartbeatTickOutcome::from_str(s).ok())
    }

    /// Parse `skip_reason` into the typed enum, or `None` if absent / unknown.
    pub fn parsed_skip_reason(&self) -> Option<crate::heartbeat_config::HeartbeatSkipReason> {
        use std::str::FromStr;
        self.skip_reason
            .as_deref()
            .and_then(|s| crate::heartbeat_config::HeartbeatSkipReason::from_str(s).ok())
    }

    /// Parse the `proposals_created_json` array; defaults to empty.
    pub fn proposals_created(&self) -> Vec<String> {
        self.proposals_created_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }
}

/// Optional extensions to a heartbeat audit row (design §13, Step 2).
///
/// Bundled into a struct so `store_heartbeat_audit` doesn't grow a 25-arg
/// signature. All fields are optional; the writer fills what it has.
#[derive(Debug, Clone, Default)]
pub struct HeartbeatAuditExtras {
    pub outcome: Option<crate::heartbeat_config::HeartbeatTickOutcome>,
    pub skip_reason: Option<crate::heartbeat_config::HeartbeatSkipReason>,
    /// Free-text fallback when no enum variant fits (e.g. an exotic precheck
    /// breadcrumb). Ignored if `skip_reason` is also set.
    pub skip_reason_text: Option<String>,
    pub config_snapshot_json: Option<String>,
    pub prompt_hash: Option<String>,
    /// Approval-queue IDs created this tick; serialized as JSON array.
    pub proposals_created: Vec<String>,
    pub next_tick_note_bytes: Option<i64>,
}

/// A single observed event queued by a probe, drained by the next heartbeat
/// tick. Design §2.1, §15.2 Step 3 — probes are context, not LLM callers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservedEvent {
    pub id: i64,
    pub at: String,
    pub source: String,
    pub severity: String,
    pub summary: String,
    pub payload_json: Option<String>,
}

/// Stored raw external event persisted for replay / normalization audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRawExternalEvent {
    pub id: i64,
    pub observed_at: String,
    pub user_did: String,
    pub source: String,
    pub source_account_id: Option<String>,
    pub source_event_id: Option<String>,
    pub dedup_key: String,
    pub payload_json: String,
}

/// A structured knowledge entry for the CEO Nerve Center.
///
/// Auto-extracted from conversations by the LLM, these entries form
/// a structured, queryable knowledge base organized by category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub id: i64,
    pub chat_id: i64,
    pub category: String, // action_item / decision / insight / waiting_for / metric / reference
    pub priority: String, // urgent / high / normal / low
    pub status: String,   // active / done / archived / superseded
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub related_ids: Vec<i64>,
    pub source_msg_id: Option<i64>,
    pub source_summary: Option<String>,
    pub due_date: Option<String>,
    pub waiting_on: Option<String>,
    pub superseded_by: Option<i64>,
    pub resolved_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Fields that can be updated on a knowledge entry.
#[derive(Debug, Clone, Default)]
pub struct KnowledgeEntryUpdate {
    pub status: Option<String>,
    pub priority: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub tags: Option<Vec<String>>,
    pub related_ids: Option<Vec<i64>>,
    pub due_date: Option<Option<String>>,
    pub waiting_on: Option<Option<String>>,
    pub superseded_by: Option<i64>,
    pub resolved_at: Option<Option<String>>,
}

/// Valid knowledge entry categories.
/// Note: "action_item" was removed — tasks are now managed via the unified `tasks` table.
pub const KB_CATEGORIES: &[&str] = &["decision", "insight", "waiting_for", "metric", "reference"];

/// Valid knowledge entry priorities.
pub const KB_PRIORITIES: &[&str] = &["urgent", "high", "normal", "low"];

/// Valid knowledge entry statuses.
pub const KB_STATUSES: &[&str] = &["active", "done", "archived", "superseded"];

/// Tag written on a `waiting_for` row created or refreshed by a recurring callback.
/// The value is the parent `tasks.id`. Title text is not a parent pointer.
pub const TRACKING_PARENT_TASK_TAG_PREFIX: &str = "tracking_task:";
/// Explicit group-key parent. The remainder must equal one generator's `group_key`.
pub const TRACKING_PARENT_GROUP_TAG_PREFIX: &str = "group_key:";
/// Explicit event-key parent. The remainder must equal one generator's `event_key`.
pub const TRACKING_PARENT_EVENT_TAG_PREFIX: &str = "event_key:";

/// How a waiting row names its generator, if it names one at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitingParentTag {
    /// No parent tag. Do not infer one from the title.
    Absent,
    /// A parent tag is present but ambiguous or malformed. Do not guess.
    Unusable,
    Task(i64),
    Group(String),
    Event(String),
}

pub fn tracking_parent_task_tag(task_id: i64) -> String {
    format!("{TRACKING_PARENT_TASK_TAG_PREFIX}{task_id}")
}

/// Record the callback's parent task on a waiting row. A non-positive id is ignored.
pub fn ensure_tracking_parent_tag(tags: &mut Vec<String>, task_id: i64) {
    if task_id <= 0 {
        return;
    }
    let tag = tracking_parent_task_tag(task_id);
    if !tags.iter().any(|existing| existing == &tag) {
        tags.push(tag);
    }
}

fn unique_prefixed_tag_value<'a>(tags: &'a [String], prefix: &str) -> Result<Option<&'a str>, ()> {
    let mut found: Option<&str> = None;
    for tag in tags {
        let Some(raw) = tag.trim().strip_prefix(prefix) else {
            continue;
        };
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(());
        }
        match found {
            None => found = Some(raw),
            Some(prev) if prev != raw => return Err(()),
            Some(_) => {}
        }
    }
    Ok(found)
}

/// Parent pointer stored on a waiting row. Task id wins over group and event keys.
/// A present but unusable tag does not fall through to a weaker key.
pub fn waiting_parent_tag(tags: &[String]) -> WaitingParentTag {
    let has_task = tags
        .iter()
        .any(|tag| tag.trim().starts_with(TRACKING_PARENT_TASK_TAG_PREFIX));
    if has_task {
        return match unique_prefixed_tag_value(tags, TRACKING_PARENT_TASK_TAG_PREFIX) {
            Ok(Some(raw)) => match raw.parse::<i64>() {
                Ok(id) if id > 0 => WaitingParentTag::Task(id),
                _ => WaitingParentTag::Unusable,
            },
            _ => WaitingParentTag::Unusable,
        };
    }
    let has_group = tags
        .iter()
        .any(|tag| tag.trim().starts_with(TRACKING_PARENT_GROUP_TAG_PREFIX));
    if has_group {
        return match unique_prefixed_tag_value(tags, TRACKING_PARENT_GROUP_TAG_PREFIX) {
            Ok(Some(raw)) => WaitingParentTag::Group(raw.to_string()),
            _ => WaitingParentTag::Unusable,
        };
    }
    let has_event = tags
        .iter()
        .any(|tag| tag.trim().starts_with(TRACKING_PARENT_EVENT_TAG_PREFIX));
    if has_event {
        return match unique_prefixed_tag_value(tags, TRACKING_PARENT_EVENT_TAG_PREFIX) {
            Ok(Some(raw)) => WaitingParentTag::Event(raw.to_string()),
            _ => WaitingParentTag::Unusable,
        };
    }
    WaitingParentTag::Absent
}

/// Maximum number of knowledge entries per chat (prevents unbounded growth).
const MAX_KB_ENTRIES_PER_CHAT: i64 = 2000;

/// Default maximum number of recent messages in L1 cache.
/// Override at runtime via `MemoryHandle::spawn_with_config`.
const DEFAULT_L1_MAX_SIZE: usize = 50;

/// Messages older than this are evicted from L1 on next push.
/// Evicted messages still exist in L3 (permanent archive) — no data loss.
const L1_TTL_SECS: i64 = 7 * 24 * 60 * 60; // 7 days

/// Minimum cosine similarity to consider a fact relevant.
const SIMILARITY_THRESHOLD: f32 = 0.35;

/// Maximum number of semantic search results returned to callers.
const SEMANTIC_TOP_K: usize = 10;

/// Expanded candidate pool for Stage 1 (before 6-dim scoring + MMR).
const SEMANTIC_CANDIDATE_K: usize = 30;

/// MMR trade-off parameter: 70% relevance, 30% diversity.
const MMR_LAMBDA: f64 = 0.7;

/// Maximum number of L2 Tier 1 facts per DID. Reduced from 10K to 2K after
/// the memory tier redesign: noise entries (l1_overflow, compaction)
/// are now in l2_ephemeral, so Tier 1 only holds curated learned:* facts.
const MAX_FACTS_PER_DID: usize = 2_000;

/// Default maximum database size in bytes (20 GB).
const DEFAULT_MAX_DB_SIZE: u64 = 20 * 1024 * 1024 * 1024;

/// Consecutive failure threshold for circuit breaker trip.
const CB_FAILURE_THRESHOLD: i64 = 3;

/// Probabilistic cleanup threshold (~1.2% chance per write).
/// Triggers stale-fact cleanup when `rand::random::<u8>() < N`.
const PROBABILISTIC_CLEANUP_THRESHOLD: u8 = 3;

// ── Actor message types ─────────────────────────────────────────────────────

/// One rolling summary / compaction fact from `l2_ephemeral`.
#[derive(Debug, Clone)]
pub struct EphemeralSummary {
    pub topic: String,
    pub fact_summary: String,
    pub source: String,
    pub created_at: String,
}

pub(crate) struct RuntimeSettingsCommit {
    pub previous: std::collections::HashMap<String, Option<serde_json::Value>>,
    /// Values at this commit, even if a later writer finishes before its reply.
    pub current: std::collections::HashMap<String, serde_json::Value>,
    pub embedding: std::collections::HashMap<String, crate::embedding::UpdateOutcome>,
}

struct EmbeddingSettingsCommit {
    client: EmbeddingClient,
    updates: Vec<(String, crate::embedding::PreparedEmbeddingUpdate)>,
}

enum MemoryCommand {
    #[cfg(test)]
    SettingsTestBarrier {
        entered: oneshot::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    },
    // L1 operations
    PushL1 {
        did: String,
        msg: Message,
        reply: oneshot::Sender<Result<()>>,
    },
    GetL1 {
        did: String,
        reply: oneshot::Sender<Vec<Message>>,
    },
    // L2 operations
    StoreL2 {
        did: String,
        topic: String,
        fact: String,
        embedding: Option<Vec<u8>>,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Store L2 fact with explicit confidence and optional expiration.
    StoreL2WithMeta {
        did: String,
        topic: String,
        fact: String,
        embedding: Option<Vec<u8>>,
        confidence: f64,
        expires_at: Option<String>,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Provenance-aware L2 write: epistemic kind, receipt, contradiction retention.
    StoreL2Provenance {
        write: partial_ref::L2ProvenanceWrite,
        reply: oneshot::Sender<Result<partial_ref::ProvenanceOutcome>>,
    },
    ListL2Contradictions {
        did: String,
        reply: oneshot::Sender<Result<Vec<partial_ref::L2Contradiction>>>,
    },
    ListL2FactHistory {
        did: String,
        fact_id: i64,
        reply: oneshot::Sender<Result<Vec<partial_ref::L2FactHistoryEntry>>>,
    },
    GetL2Provenance {
        did: String,
        id: i64,
        reply: oneshot::Sender<Result<Option<partial_ref::L2ProvenanceView>>>,
    },
    ConsolidateL2Observations {
        did: String,
        topic: String,
        reply: oneshot::Sender<Result<partial_ref::ConsolidationOutcome>>,
    },
    /// Semantic vector search over L2 facts (HNSW accelerated or SQL fallback).
    /// `query_text` enables the pre-MMR lexical/temporal/topic fusion.
    QueryL2Semantic {
        did: String,
        query_embedding: Vec<u8>,
        hnsw: Option<std::sync::Arc<HnswIndex>>,
        query_text: Option<String>,
        reply: oneshot::Sender<Vec<Experience>>,
    },
    GetAllL2 {
        did: String,
        reply: oneshot::Sender<Vec<Experience>>,
    },
    GetL2ById {
        did: String,
        id: i64,
        reply: oneshot::Sender<Result<Option<Experience>>>,
    },
    /// Mark a specific L2 fact as reviewed by DID and row ID.
    TouchL2ById {
        did: String,
        id: i64,
        reply: oneshot::Sender<Result<bool>>,
    },
    /// Update a specific L2 fact by DID and row ID.
    UpdateL2 {
        did: String,
        id: i64,
        fact: String,
        confidence: f64,
        reply: oneshot::Sender<Result<bool>>,
    },
    /// Pin or unpin a specific L2 fact by DID and row ID.
    SetL2Pinned {
        did: String,
        id: i64,
        pinned: bool,
        reply: oneshot::Sender<Result<bool>>,
    },
    // Reminder operations
    StoreReminder {
        chat_id: i64,
        message: String,
        remind_at_unix: i64,
        reply: oneshot::Sender<Result<i64>>,
    },
    GetDueReminders {
        now_unix: i64,
        reply: oneshot::Sender<Vec<Reminder>>,
    },
    MarkReminderFired {
        id: i64,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Check current database size against the configured limit.
    CheckDbSize {
        reply: oneshot::Sender<DbSizeStatus>,
    },
    /// Delete a specific L2 fact by DID and row ID.
    DeleteL2 {
        did: String,
        id: i64,
        reply: oneshot::Sender<Result<bool>>,
    },
    /// Get all pending (unfired) reminders.
    GetPendingReminders {
        reply: oneshot::Sender<Vec<Reminder>>,
    },
    /// List pending reminders for a specific chat.
    ListRemindersForChat {
        chat_id: i64,
        resp: oneshot::Sender<Result<Vec<Reminder>>>,
    },
    /// Cancel (delete) a pending reminder by ID and chat_id.
    CancelReminder {
        reminder_id: i64,
        chat_id: i64,
        resp: oneshot::Sender<Result<bool>>,
    },
    // L3 operations (full-text archive)
    /// Full-text search over L3 archive using SQLite FTS5.
    QueryL3Fulltext {
        did: String,
        query: String,
        reply: oneshot::Sender<Vec<L3Entry>>,
    },
    /// Recent L3 archive entries in chronological order.
    ListRecentL3 {
        did: String,
        limit: usize,
        reply: oneshot::Sender<Vec<L3Entry>>,
    },
    /// Conversation rows that may have triggered a work item.
    ///
    /// When `task_id` has an `origin_turn_id` on its creation receipt, those
    /// turn rows are returned with `exact_turn`. Otherwise the rows are the
    /// archive window around `created_at_unix`.
    ListTriggerArchive {
        did: String,
        created_at_unix: i64,
        task_id: Option<i64>,
        reply: oneshot::Sender<TriggerArchive>,
    },
    /// Flush all L1 caches to the l1_snapshot table for persistence across restarts.
    FlushL1ToDb {
        reply: oneshot::Sender<Result<usize>>,
    },
    /// Restore L1 caches from the l1_snapshot table (startup recovery).
    RestoreL1FromDb {
        reply: oneshot::Sender<Result<usize>>,
    },
    /// Graceful shutdown: flush L1, WAL checkpoint, then stop.
    Shutdown {
        reply: oneshot::Sender<Result<()>>,
    },
    /// Rebuild the HNSW index from DB when deleted ratio exceeds threshold.
    RebuildHnsw {
        hnsw: std::sync::Arc<crate::hnsw_index::HnswIndex>,
        reply: oneshot::Sender<Result<usize>>,
    },
    // User settings operations
    CommitRuntimeSettings {
        updates: Vec<crate::settings::PreparedSetting>,
        settings: std::sync::Arc<crate::settings::RuntimeSettings>,
        write_guard: Option<crate::call_validation::SettingsWriteGuard>,
        embedding: Option<EmbeddingSettingsCommit>,
        reply: oneshot::Sender<Result<RuntimeSettingsCommit>>,
    },
    StoreSetting {
        key: String,
        value: serde_json::Value,
        reply: oneshot::Sender<Result<()>>,
    },
    DeleteSetting {
        key: String,
        reply: oneshot::Sender<Result<bool>>,
    },
    LoadAllSettings {
        reply: oneshot::Sender<Result<std::collections::HashMap<String, serde_json::Value>>>,
    },
    /// Append a row to `config_audit_log` (heartbeat config drift tracking).
    InsertConfigAudit {
        actor: String,
        old_json: Option<String>,
        new_json: String,
        reason: Option<String>,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// List most-recent rows from `config_audit_log`, newest first.
    ListConfigAudit {
        limit: usize,
        reply: oneshot::Sender<Result<Vec<crate::heartbeat_config::ConfigAuditEntry>>>,
    },
    /// Query L2 facts by topic prefix, returning (fact_summary, embedding) pairs.
    /// Used for embedding-based deduplication.
    QueryL2ByTopicPrefix {
        did: String,
        prefix: String,
        reply: oneshot::Sender<Vec<(String, Vec<f32>)>>,
    },
    /// Query L2 facts by topic prefix with a time-based TTL filter.
    /// Only returns facts created after `since` (RFC3339 timestamp).
    QueryL2ByTopicPrefixSince {
        did: String,
        prefix: String,
        since: String,
        reply: oneshot::Sender<Vec<(String, Vec<f32>)>>,
    },
    /// Query L2 fact text by topic prefix with TTL filter.
    /// Returns only fact_summary strings — no embedding requirement.
    /// Used for API knowledge retrieval where semantic search is not needed.
    QueryL2TextByTopicPrefixSince {
        did: String,
        prefix: String,
        since: String,
        reply: oneshot::Sender<Vec<String>>,
    },
    /// Step 10: fetch a single L2 row by id if it belongs to `did` AND was
    /// created within the freshness window. Returns `None` when missing,
    /// cross-tenant, or stale. Used by `send_message` for source-citation
    /// validation (design §5 gate 2). Cheap existence + freshness check —
    /// does NOT re-embed.
    GetL2ByIdIfFresh {
        did: String,
        row_id: i64,
        /// Cutoff as an RFC3339 UTC string; rows with `created_at < since`
        /// return `None`. Formatted by the caller so the actor does not
        /// need a clock source.
        since_rfc3339: String,
        reply: oneshot::Sender<Option<L2RowBrief>>,
    },
    // ── Skill registry (skill_proposals) operations ─────────────────────
    CreateSkillProposal {
        skill_name: String,
        requested_by: String,
        user_request: String,
        proposal_summary: Option<String>,
        reply: oneshot::Sender<Result<i64>>,
    },
    UpdateSkillProposal {
        id: i64,
        updates: Box<SkillProposalUpdate>,
        reply: oneshot::Sender<Result<()>>,
    },
    GetSkillProposal {
        id: i64,
        reply: oneshot::Sender<Option<SkillProposal>>,
    },
    ListSkillProposals {
        requested_by: Option<String>,
        status_filter: Option<String>,
        reply: oneshot::Sender<Vec<SkillProposal>>,
    },
    CountDailyProposals {
        requested_by: String,
        reply: oneshot::Sender<i64>,
    },
    DeleteSkillProposal {
        id: i64,
        reply: oneshot::Sender<bool>,
    },
    // ── Skill metrics operations ────────────────────────────────────────
    UpdateSkillMetrics {
        skill_name: String,
        success: bool,
        latency_ms: f64,
        error_msg: Option<String>,
        error_category: Option<String>,
    },
    GetSkillMetrics {
        skill_name: String,
        reply: oneshot::Sender<Option<SkillMetrics>>,
    },
    GetAllSkillMetrics {
        reply: oneshot::Sender<Vec<SkillMetrics>>,
    },
    GetDeactivatedSkillNames {
        reply: oneshot::Sender<std::collections::HashSet<String>>,
    },
    ResetCircuitBreaker {
        skill_name: String,
    },
    CheckCircuitBreaker {
        skill_name: String,
        reply: oneshot::Sender<Option<(bool, i64, Option<String>)>>,
    },
    // ── Deferred intent operations ──────────────────────────────────────
    StoreDeferredIntent {
        user_did: String,
        chat_id: i64,
        trigger_type: String,
        trigger_at_unix: Option<i64>,
        event_key: Option<String>,
        execution_mode: String,
        context_text: String,
        expires_at_unix: i64,
        is_callback_origin: bool,
        interval_seconds: Option<i64>,
        chain_depth: i32,
        pin_on_fire: bool,
        group_key: Option<String>,
        max_fires: Option<i64>,
        recur_weekdays: Option<Vec<u8>>,
        recur_window_start: Option<String>,
        recur_window_end: Option<String>,
        recur_timezone: Option<String>,
        #[allow(dead_code)] // Populated via SQL but read path not yet implemented
        embedding: Option<Vec<u8>>,
        reply: oneshot::Sender<Result<i64>>,
    },
    GetDueTimeIntents {
        now_unix: i64,
        reply: oneshot::Sender<Vec<DeferredIntent>>,
    },
    GetMatchingEventIntents {
        event_key: String,
        user_did: String,
        reply: oneshot::Sender<Vec<DeferredIntent>>,
    },
    MarkIntentFired {
        id: i64,
        reply: oneshot::Sender<Result<()>>,
    },
    MarkIntentTriggerAt {
        id: i64,
        trigger_at_unix: i64,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Reschedule a recurring intent: update trigger_at, expires_at, reset status to pending, clear fired_at.
    RescheduleRecurringIntent {
        id: i64,
        trigger_at_unix: i64,
        expires_at_unix: i64,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Atomically fire a recurring intent and advance to next trigger.
    /// Keeps status='active', records fired_at, advances trigger_at, increments fire_count.
    /// This eliminates the crash window between mark_fired and reschedule.
    FireAndAdvanceRecurring {
        id: i64,
        trigger_at_unix: i64,
        expires_at_unix: i64,
        reply: oneshot::Sender<Result<()>>,
    },
    ExpireOldIntents {
        now_unix: i64,
        reply: oneshot::Sender<Result<Vec<(i64, String, String)>>>,
    },
    ListIntentHistory {
        user_did: String,
        since_unix: i64,
        reply: oneshot::Sender<Result<Vec<DeferredIntent>>>,
    },
    ArchiveOldIntents {
        cutoff_unix: i64,
        reply: oneshot::Sender<Result<Vec<(i64, String, String, String)>>>,
    },
    DeleteOldArchivedTasks {
        cutoff_unix: i64,
        limit: i64,
        reply: oneshot::Sender<Result<(usize, Option<i64>, Option<i64>)>>,
    },
    DismissIntent {
        id: i64,
        user_did: String,
        reply: oneshot::Sender<Result<bool>>,
    },
    RescheduleIntent {
        id: i64,
        user_did: String,
        trigger_at_unix: i64,
        expires_at_unix: i64,
        reply: oneshot::Sender<Result<bool>>,
    },
    ListUserIntents {
        user_did: String,
        reply: oneshot::Sender<Vec<DeferredIntent>>,
    },
    CancelIntent {
        id: i64,
        user_did: String,
        reply: oneshot::Sender<Result<bool>>,
    },
    ListRecurringIntents {
        user_did: String,
        reply: oneshot::Sender<Vec<DeferredIntent>>,
    },
    CountActiveIntents {
        user_did: String,
        reply: oneshot::Sender<i64>,
    },
    CountDailyIntents {
        user_did: String,
        reply: oneshot::Sender<i64>,
    },
    /// Increment retry_count for a pending intent. Returns the new retry_count.
    /// If retry_count reaches max_retries, status is set to 'dead_letter'.
    IncrementIntentRetry {
        id: i64,
        max_retries: i32,
        reply: oneshot::Sender<Result<i32>>,
    },
    // ── Unified task operations ──────────────────────────────────────
    StoreTask {
        chat_id: i64,
        user_did: String,
        title: String,
        body: String,
        priority: String,
        tags: String, // JSON array
        due_date: Option<String>,
        source: String,
        trigger_at_unix: Option<i64>,
        trigger_type: Option<String>,
        event_key: Option<String>,
        execution_mode: String,
        context_text: Option<String>,
        interval_seconds: Option<i64>,
        group_key: Option<String>,
        max_fires: Option<i64>,
        pin_on_fire: bool,
        expires_at_unix: Option<i64>,
        is_callback_origin: bool,
        chain_depth: i32,
        recur_weekdays: Option<Vec<u8>>,
        recur_window_start: Option<String>,
        recur_window_end: Option<String>,
        recur_timezone: Option<String>,
        reply: oneshot::Sender<Result<i64>>,
    },
    GetTask {
        id: i64,
        reply: oneshot::Sender<Option<Task>>,
    },
    UpdateTask {
        id: i64,
        chat_id: i64,
        updates_json: String, // serialized TaskUpdate fields
        reply: oneshot::Sender<Result<bool>>,
    },
    ListTasks {
        chat_id: i64,
        user_did: Option<String>,
        status: Option<String>,
        limit: i64,
        reply: oneshot::Sender<Vec<Task>>,
    },
    /// Delete all done/archived tasks for a chat.
    ClearFinishedTasks {
        chat_id: i64,
        reply: oneshot::Sender<Result<usize>>,
    },
    /// Permanently delete a task (not soft-delete — removed from DB).
    DeleteTask {
        id: i64,
        chat_id: i64,
        reply: oneshot::Sender<Result<bool>>,
    },
    /// List all active scheduled tasks across all users (admin view).
    ListAllScheduledTasks {
        limit: i64,
        reply: oneshot::Sender<Vec<Task>>,
    },
    /// Persist a replay nonce to SQLite (fire-and-forget).
    PersistReplayNonce {
        sig_hex: String,
        expires_at: i64,
    },
    /// Load all unexpired replay nonces from SQLite.
    LoadReplayNonces {
        min_expires_at: i64,
        reply: oneshot::Sender<Result<Vec<(String, u64)>>>,
    },
    /// Delete expired replay nonces from SQLite (fire-and-forget).
    DeleteExpiredReplayNonces {
        now_unix: i64,
    },
    /// Health check: returns true if the actor is alive and responsive.
    Healthcheck {
        reply: oneshot::Sender<bool>,
    },
    /// Count rows in `l2_experience` that have a non-NULL `embedding`. Used
    /// by the embedding-corpus consent mechanism at startup to report the
    /// size of the cross-model corpus the user is about to decide on.
    CountEmbeddedFacts {
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Query L2 facts by exact topic, returning (fact_summary, embedding) pairs.
    /// Used for monitoring intent retrieval.
    QueryL2ByTopic {
        did: String,
        topic: String,
        reply: oneshot::Sender<Vec<(String, Option<Vec<f32>>)>>,
    },
    /// Delete all L2 facts matching an exact topic for a DID.
    /// Returns (count_deleted, deleted_ids) so caller can mark HNSW entries.
    DeleteL2ByTopic {
        did: String,
        topic: String,
        reply: oneshot::Sender<Result<(u64, Vec<i64>)>>,
    },
    // ── Monitoring intent operations (unified in tasks table) ────────
    StoreMonitoringIntent {
        user_did: String,
        skill_name: String,
        description: String,
        threshold: f32,
        embedding: Vec<u8>,
        reply: oneshot::Sender<Result<i64>>,
    },
    ListMonitoringIntents {
        user_did: String,
        reply: oneshot::Sender<Result<Vec<MonitoringIntentRecord>>>,
    },
    DeleteMonitoringIntent {
        intent_id: i64,
        reply: oneshot::Sender<Result<()>>,
    },
    GetAllMonitoringIntents {
        reply: oneshot::Sender<Result<Vec<MonitoringIntentRecord>>>,
    },
    // ── Skill state operations ────────────────────────────────────────
    StoreSkillState {
        skill_name: String,
        key: String,
        value: String,
        reply: oneshot::Sender<Result<()>>,
    },
    LoadSkillState {
        skill_name: String,
        key: String,
        reply: oneshot::Sender<Result<Option<String>>>,
    },
    DeleteSkillState {
        skill_name: String,
        key: String,
        reply: oneshot::Sender<Result<()>>,
    },
    /// List all non-empty skill_state entries for a given skill_name (e.g. "hitl").
    ListSkillStateKeys {
        skill_name: String,
        reply: oneshot::Sender<Result<Vec<(String, String)>>>,
    },
    // ── HITL approval queue (design §6, Step 9) ───────────────────────
    /// Insert or replace an HITL approval row with full metadata
    /// (kind / cycle-id / embedding / expiry). Unlike `StoreSkillState`,
    /// the caller passes every extension column explicitly so heartbeat
    /// proposals produce queryable metadata for sweep + dedup.
    StoreHitlApproval {
        key: String,
        value: String,
        kind: String,
        proposed_at_audit_cycle_id: Option<String>,
        embedding_json: Option<String>,
        expires_at: String,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Delete HITL rows whose `expires_at < now` (sentinel-aware: the
    /// default '9999-…' sentinel is never swept). Returns deleted count.
    SweepExpiredHitl {
        now_rfc3339: String,
        reply: oneshot::Sender<Result<u64>>,
    },
    /// Scan pending HITL rows of `kind`, return (state_key, embedding_json)
    /// for rows with non-empty state_value, non-null embedding, and
    /// updated_at >= `since`. Cosine computation happens in the caller
    /// (which has EmbeddingClient access) so this layer stays unaware.
    ListPendingHitlWithEmbeddings {
        kind: String,
        since_rfc3339: String,
        reply: oneshot::Sender<Result<Vec<(String, String)>>>,
    },
    /// Count pending HITL rows of the given `kind` (state_value non-empty).
    /// Used by `BudgetSnapshot` to surface pending-approval counts into the
    /// heartbeat wake-up block (Step 9b).
    CountHitlPendingByKind {
        kind: String,
        reply: oneshot::Sender<Result<u32>>,
    },
    /// Count pending HITL rows whose state changed recently. Used by the
    /// heartbeat zero-cost precheck so stable approval backlog does not wake
    /// the LLM every tick.
    CountHitlPendingUpdatedSince {
        since_rfc3339: String,
        reply: oneshot::Sender<Result<u32>>,
    },
    // ── Fact aging operations ─────────────────────────────────────────
    /// Store a context summary from L1 overflow summarization.
    /// The summary is stored as a special L2 fact with topic "context_summary".
    StoreSummary {
        did: String,
        summary: String,
        /// Structured facts extracted from the summarized messages
        facts: Vec<(String, String)>, // (topic, fact_summary)
    },
    /// Fire-and-forget: persist inspectable evidence for one Web console reply.
    StoreTurnEvidence {
        user_did: String,
        reply_key: String,
        evidence_json: String,
        unsupported_json: String,
    },
    /// Fire-and-forget: update last_accessed timestamp for retrieved L2 facts.
    TouchL2Facts {
        ids: Vec<i64>,
    },
    /// Fire-and-forget: increment command usage counter for adaptive menu ordering.
    TrackCommandUsage {
        command: String,
        user_id: String,
    },
    /// Get command usage stats sorted by frequency (for adaptive menu).
    GetCommandUsageStats {
        user_id: String,
        reply: oneshot::Sender<Vec<(String, u64)>>,
    },
    /// Query stale L2 facts not accessed within the given number of days.
    QueryStaleFacts {
        did: String,
        days_threshold: i64,
        reply: oneshot::Sender<Result<Vec<StaleFact>>>,
    },
    /// Test-only: backdate last_accessed + created_at for an L2 fact.
    #[cfg(test)]
    TestBackdateL2LastAccessed {
        id: i64,
        days_ago: i64,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Snapshot current skill_metrics into skill_metrics_daily for today.
    SnapshotDailyMetrics,
    /// Get daily metrics for a skill over a date range.
    GetDailyMetricsTrend {
        skill_name: String,
        days: i64,
        reply: oneshot::Sender<Vec<(String, u64, u64, f64)>>, // (date, total, errors, avg_latency)
    },
    // ── Knowledge Base operations ─────────────────────────────────────
    StoreKnowledgeEntry {
        chat_id: i64,
        category: String,
        priority: String,
        title: String,
        body: String,
        tags: String,        // JSON array
        related_ids: String, // JSON array
        source_msg_id: Option<i64>,
        source_summary: Option<String>,
        due_date: Option<String>,
        waiting_on: Option<String>,
        embedding: Option<Vec<u8>>,
        reply: oneshot::Sender<Result<i64>>,
    },
    UpdateKnowledgeEntry {
        id: i64,
        chat_id: i64,
        updates: Box<KnowledgeEntryUpdate>,
        reply: oneshot::Sender<Result<bool>>,
    },
    QueryKnowledgeEntries {
        chat_id: i64,
        category: Option<String>,
        status: Option<String>,
        priority: Option<String>,
        limit: i64,
        reply: oneshot::Sender<Vec<KnowledgeEntry>>,
    },
    GetKnowledgeEntry {
        id: i64,
        chat_id: i64,
        reply: oneshot::Sender<Option<KnowledgeEntry>>,
    },
    SearchKnowledgeEntries {
        chat_id: i64,
        query: String,
        limit: i64,
        reply: oneshot::Sender<Vec<KnowledgeEntry>>,
    },
    CountKnowledgeEntries {
        chat_id: i64,
        reply: oneshot::Sender<i64>,
    },
    /// Dashboard summary: counts by category + status for the CEO overview.
    KnowledgeDashboardSummary {
        chat_id: i64,
        reply: oneshot::Sender<Vec<(String, String, i64)>>, // (category, status, count)
    },
    KbHygieneSummary {
        chat_id: i64,
        user_did: String,
        reply: oneshot::Sender<KbHygieneSummary>,
    },
    // ── Review guard log operations ──────────────────────────────────────
    /// Store a review pipeline record (fire-and-forget).
    StoreReviewRecord {
        layer: String,
        passed: bool,
        tool_matched: Option<String>,
        similarity: Option<f32>,
        reply_preview: String,
        tools_called: String,
        latency_ms: u64,
    },
    /// Get aggregate review stats for the last N hours.
    GetReviewStats {
        since_hours: u32,
        reply: oneshot::Sender<crate::evaluators::ReviewStats>,
    },
    /// Get recent flagged review records (last N).
    GetRecentFlaggedReviews {
        limit: u32,
        reply: oneshot::Sender<Vec<(String, String, String, Option<String>, Option<f64>, i64)>>,
        // (timestamp, layer, reply_preview, tool_matched, similarity, latency_ms)
    },
    // ── Heartbeat audit log operations ─────────────────────────────────
    StoreHeartbeatAudit {
        cycle_id: String,
        skill_name: String,
        severity: String,
        probe_payload: Option<String>,
        tools_called: String,  // JSON array
        tools_blocked: String, // JSON array
        actions_taken: String, // JSON array
        llm_reply: Option<String>,
        tokens_used: i32,
        rounds_used: i32,
        user_did: String,
        chat_id: Option<i64>,
        config_snapshot: String, // JSON
        raw_process_log: Option<String>,
        tool_call_details: Option<String>,
        // Step 2 extension fields (design §13). All optional; legacy writers
        // leave them as None / empty.
        outcome: Option<String>,
        skip_reason: Option<String>,
        config_snapshot_json: Option<String>,
        prompt_hash: Option<String>,
        proposals_created_json: Option<String>,
        next_tick_note_bytes: Option<i64>,
        reply: oneshot::Sender<Result<()>>,
    },
    QueryHeartbeatAudit {
        skill_name: Option<String>,
        limit: usize,
        reply: oneshot::Sender<Result<Vec<HeartbeatAuditEntry>>>,
    },
    // ── Subcall primitive persistence (design §7.4, §7.5) ─────────────
    /// Insert one row into `subcall_log` — per-invocation metadata.
    /// Called by `core/src/subcall.rs::run_subcall` after the bounded
    /// ReAct loop returns (or times out / errors). Minimal by design:
    /// richer query helpers land with Task 1.7's recall path.
    SubcallLogInsert {
        id: String,
        parent_session_id: String,
        origin: String,
        task_preview: Option<String>,
        allowed_tools_json: Option<String>,
        status: String,
        rounds_used: Option<i32>,
        tool_calls_made: Option<i32>,
        tokens_used: Option<i32>,
        duration_ms: Option<i64>,
        created_at: String,
        parent_subcall_id: Option<String>,
        trace_available: bool,
        summary: Option<String>,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Insert one row into `subcall_trace` — the replay payload for
    /// Task 1.7 recall + debugging. `size_bytes` is precomputed by the
    /// caller (the serialized JSON lengths) so the actor does not need
    /// to re-measure.
    SubcallTraceInsert {
        subcall_id: String,
        system_prompt: Option<String>,
        messages_json: String,
        tool_outputs_json: String,
        created_at: String,
        accessed_at: String,
        size_bytes: i64,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Fetch the trace payload for recall (Task 1.7). Returns a 4-tuple
    /// `(exists, trace_available, summary, tool_outputs)`:
    /// - `exists=false` → no `subcall_log` row matches the id (likely
    ///   a hallucinated id from the LLM). Caller should surface as an
    ///   explicit "not_found" signal — distinct from "evicted".
    /// - `exists=true, trace_available=false` → row exists but trace was
    ///   LRU-evicted. `summary` carries the stored `subcall_log.summary`
    ///   so recall can still surface what the subcall concluded.
    /// - `exists=true, trace_available=true` → full trace available,
    ///   `tool_outputs` populated from `subcall_trace.tool_outputs_json`.
    SubcallFetchTrace {
        subcall_id: String,
        reply: oneshot::Sender<Result<(bool, bool, Option<String>, Option<Vec<(String, String)>>)>>,
    },
    /// Update `subcall_trace.accessed_at` to `now()` on a recall hit so the
    /// LRU sweep (Task 1.8) keeps the row around. No-op if the row has
    /// already been evicted (UPDATE affects 0 rows).
    SubcallTouchAccessed {
        subcall_id: String,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Daily TTL sweep (design §7.5, Task 1.8). Deletes `subcall_log`
    /// rows (and CASCADE-deletes any matching `subcall_trace` rows)
    /// where the log row is older than 90 days AND the trace was
    /// either never-recalled (accessed_at IS NULL — i.e., already
    /// evicted) or untouched for 30+ days. Returns rows deleted.
    SubcallTtlSweep {
        reply: oneshot::Sender<Result<usize>>,
    },
    /// LRU soft-cap enforcement (design §7.5, Task 1.8). If the sum of
    /// `subcall_trace.size_bytes` exceeds `total_bytes_cap`, delete
    /// trace rows by ASC `accessed_at` until back under cap and flip
    /// the corresponding `subcall_log.trace_available` to 0 so
    /// `subcall_fetch_trace` reports `trace_evicted` instead of a
    /// stale hit. Returns trace rows evicted.
    SubcallEnforceLruCap {
        total_bytes_cap: i64,
        reply: oneshot::Sender<Result<usize>>,
    },
    // ── Observed events queue (design §15.2 Step 3) ───────────────────
    /// Record a probe observation for the next heartbeat tick to consume.
    /// Returns the row id.
    RecordObservedEvent {
        source: String,
        severity: String,
        summary: String,
        payload_json: Option<String>,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Atomically select the oldest `limit` unconsumed events and mark them
    /// consumed in a single transaction. Later ticks see only newer events.
    TakeUnconsumedObservedEvents {
        limit: usize,
        reply: oneshot::Sender<Result<Vec<ObservedEvent>>>,
    },
    /// Persist a raw external event for replay/audit. Deduplicated by `dedup_key`.
    StoreRawExternalEvent {
        event: crate::assistant_events::RawExternalEvent,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Persist a normalized assistant event. Deduplicated by `dedup_key`.
    StoreAssistantEvent {
        event: crate::assistant_events::AssistantEvent,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Persist a durable assistant work item. Deduplicated by `dedup_key`.
    StoreAssistantWorkItem {
        item: crate::assistant_work_items::AssistantWorkItem,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// List assistant work items for a user, optionally filtered by statuses.
    ListAssistantWorkItems {
        user_did: String,
        statuses: Option<Vec<String>>,
        source_kinds: Option<Vec<String>>,
        modes: Option<Vec<String>>,
        caller_contexts: Option<Vec<String>>,
        limit: usize,
        reply: oneshot::Sender<Result<Vec<crate::assistant_work_items::StoredAssistantWorkItem>>>,
    },
    /// Claim the highest-priority queued assistant work item and mark it running.
    ClaimNextAssistantWorkItem {
        user_did: String,
        filter: Option<crate::assistant_work_items::AssistantWorkClaimFilter>,
        reply:
            oneshot::Sender<Result<Option<crate::assistant_work_items::StoredAssistantWorkItem>>>,
    },
    /// Update an assistant work item status and optional result payload.
    UpdateAssistantWorkItem {
        id: i64,
        status: crate::assistant_work_items::AssistantWorkStatus,
        result: Option<serde_json::Value>,
        reply: oneshot::Sender<Result<()>>,
    },
    /// List most recent assistant events for a user.
    ListRecentAssistantEvents {
        user_did: String,
        limit: usize,
        reply: oneshot::Sender<Result<Vec<crate::assistant_events::StoredAssistantEvent>>>,
    },
    /// List most recent Web console turn-evidence rows for a user (newest first).
    ListRecentTurnEvidence {
        user_did: String,
        limit: usize,
        reply: oneshot::Sender<Result<Vec<TurnEvidenceRecord>>>,
    },
    GetAssistantEventById {
        id: i64,
        reply: oneshot::Sender<Result<Option<crate::assistant_events::StoredAssistantEvent>>>,
    },
    /// Read assistant events newer than the last committed row for a consumer
    /// without advancing its cursor. Optionally filter by decision names.
    ///
    /// Cursor advance is deliberately decoupled from the read: callers must
    /// invoke [`CommitAssistantEventCursor`] explicitly only after they have
    /// successfully delivered/consumed the events. A crash between the peek
    /// and the explicit commit re-yields the same rows on next peek, giving
    /// at-least-once delivery semantics.
    PeekNewAssistantEventsForConsumer {
        user_did: String,
        consumer: String,
        limit: usize,
        decisions: Option<Vec<String>>,
        exclude_decisions: Option<Vec<String>>,
        reply: oneshot::Sender<Result<Vec<crate::assistant_events::StoredAssistantEvent>>>,
    },
    /// Advance a per-consumer assistant-event cursor to the supplied row id.
    CommitAssistantEventCursor {
        user_did: String,
        consumer: String,
        last_event_id: i64,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Sum tokens_used from heartbeat_audit_log for today (UTC).
    /// Used to restore the in-memory daily token counter after process restart.
    SumTodaysHeartbeatTokens {
        reply: oneshot::Sender<Result<u32>>,
    },
    /// Query heartbeat audit entries from the last N days (for credit analysis).
    /// Only returns entries with non-null tool_call_details.
    QueryHeartbeatAuditRecent {
        lookback_days: u32,
        reply: oneshot::Sender<Result<Vec<HeartbeatAuditEntry>>>,
    },
    /// Store a credit analysis result for a single tool on a given date.
    StoreCreditAnalysis {
        analysis_date: String,
        tool_name: String,
        times_called: i32,
        times_cited: i32,
        effectiveness_score: f64,
        wasted_calls: i32,
        avg_latency_ms: f64,
        total_tokens_attributed: i32,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Query the latest credit analysis results (within lookback_days).
    QueryLatestCreditAnalysis {
        lookback_days: u32,
        reply: oneshot::Sender<Result<Vec<crate::chronos::ToolCreditScore>>>,
    },
    /// Store a per-call LLM token usage record for observability.
    StoreLlmCallLog {
        label: String,
        model: String,
        provider: Option<String>,
        provider_request_id: Option<String>,
        prompt_tokens: u32,
        completion_tokens: u32,
        cached_tokens: u32,
        total_tokens: u32,
        cost_usd: Option<f64>,
        latency_ms: Option<u64>,
        is_fallback: bool,
    },
    /// Store a prompt version (pending approval).
    StorePromptVersion {
        module_name: String,
        version: i32,
        original_text: String,
        modified_text: String,
        rationale: String,
        status: String,
        approval_id: Option<String>,
        reply: oneshot::Sender<Result<i64>>, // returns the inserted row ID
    },
    /// Query all active prompt overrides (status = 'active').
    QueryActivePromptOverrides {
        reply: oneshot::Sender<Result<Vec<PromptOverride>>>,
    },
    /// Update the status of a prompt version.
    UpdatePromptVersionStatus {
        id: i64,
        status: String,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Query prompt version history for a specific module.
    QueryPromptVersionHistory {
        module_name: String,
        limit: usize,
        reply: oneshot::Sender<Result<Vec<PromptOverride>>>,
    },
    /// Count unhealthy skill metrics updated recently.
    CountUnhealthySkillMetricsUpdatedSince {
        since_rfc3339: String,
        reply: oneshot::Sender<Result<u32>>,
    },
    // ── Self-audit query operations ──────────────────────────────────────
    /// Count heartbeat no-op ratio in the last N hours.
    /// Returns (noop_count, total_count).
    AuditHeartbeatNoopRatio {
        hours: u32,
        reply: oneshot::Sender<Result<(i64, i64)>>,
    },
    /// Count heartbeat no-op rows in the last N hours.
    AuditRecentHeartbeatNoopCount {
        hours: u32,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Count tool blocks from heartbeat_audit_log in the last N hours.
    /// Returns Vec<(tool_name, count)>.
    AuditRecentToolBlocks {
        hours: u32,
        reply: oneshot::Sender<Result<Vec<(String, i64)>>>,
    },
    /// Count LLM call log entries in the last N hours.
    AuditRecentLlmCalls {
        hours: u32,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Count heartbeat audit entries in the last N hours.
    AuditRecentHeartbeatCount {
        hours: u32,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Count heartbeat audit entries that should have produced an LLM call.
    AuditRecentLlmEligibleHeartbeatCount {
        hours: u32,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Get daily cache hit rates from llm_call_log for the last N days.
    /// Returns Vec<(date_string, cache_pct)>.
    AuditDailyCacheRates {
        days: u32,
        reply: oneshot::Sender<Result<Vec<(String, f64)>>>,
    },
    /// Get daily total token usage from llm_call_log for the last N days.
    /// Returns Vec<(date_string, total_tokens)>.
    AuditDailyTokenTotals {
        days: u32,
        reply: oneshot::Sender<Result<Vec<(String, i64)>>>,
    },
    /// Test-only: backdate `heartbeat_audit_log.timestamp` for rows with a
    /// given `outcome` through the actor connection. Avoids dual-connection
    /// races under parallel `cargo test --lib` load.
    #[cfg(test)]
    TestBackdateHeartbeatAuditByOutcome {
        outcome: String,
        hours_ago: i64,
        reply: oneshot::Sender<Result<u64>>,
    },

    // ── G1: Relationship State ────────────────────────────────────────────
    /// Get relationship state for a user (returns default if none exists).
    GetRelationshipState {
        did: String,
        reply: oneshot::Sender<Result<RelationshipState>>,
    },
    /// Update relationship state after each interaction (fire-and-forget ok).
    UpdateRelationshipInteraction {
        did: String,
        topic: String,
        user_tz: String,
        reply: oneshot::Sender<Result<()>>,
    },

    // ── Tier 0: User Profile ────────────────────────────────────────────
    /// Get all profile entries for a DID, ordered by category then key.
    GetUserProfile {
        did: String,
        reply: oneshot::Sender<Result<Vec<ProfileEntry>>>,
    },
    /// Upsert a profile entry (INSERT OR REPLACE on UNIQUE(agent_did, category, key)).
    UpsertProfileEntry {
        entry: ProfileEntry,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Delete a profile entry by (agent_did, category, key).
    DeleteProfileEntry {
        did: String,
        category: String,
        key: String,
        reply: oneshot::Sender<Result<bool>>,
    },
    /// Count ephemeral entries by DID and optional source filter.
    CountEphemeral {
        did: String,
        source: Option<String>,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Rolling conversation summaries and compaction facts (`l2_ephemeral`
    /// sources `context_summary` / `compaction_summary`), newest first. Until
    /// 2026-09-13 these were written and never read by any recall path.
    ListEphemeralSummaries {
        did: String,
        limit: i64,
        reply: oneshot::Sender<Result<Vec<EphemeralSummary>>>,
    },
    /// Insert a row into `l2_ephemeral` (Tier 2) with explicit source + TTL.
    /// Used to persist ephemeral facts with an embedding for later
    /// embedding-based topic dedup.
    StoreEphemeral {
        did: String,
        topic: String,
        fact: String,
        embedding: Option<Vec<u8>>,
        source: String,
        ttl_days: i64,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Delete all rows in `l2_ephemeral` where `expires_at < now`.
    /// Returns the row count deleted.
    CleanupExpiredEphemeral {
        reply: oneshot::Sender<Result<usize>>,
    },

    // ── G5: Goal Tracking ────────────────────────────────────────────────
    CreateGoal {
        did: String,
        goal: Goal,
        reply: oneshot::Sender<Result<i64>>,
    },
    ListGoals {
        did: String,
        status: Option<GoalStatus>,
        reply: oneshot::Sender<Result<Vec<Goal>>>,
    },
    GetGoal {
        id: i64,
        reply: oneshot::Sender<Result<Option<Goal>>>,
    },
    UpdateGoal {
        id: i64,
        updates: GoalUpdate,
        reply: oneshot::Sender<Result<()>>,
    },
    AddMilestone {
        goal_id: i64,
        title: String,
        reply: oneshot::Sender<Result<()>>,
    },
    CompleteMilestone {
        goal_id: i64,
        milestone_idx: usize,
        reply: oneshot::Sender<Result<()>>,
    },

    /// G5: Link a fact to a goal (with similarity score).
    LinkFactToGoal {
        goal_id: i64,
        fact_id: i64,
        similarity: f32,
        reply: oneshot::Sender<Result<()>>,
    },

    // ── Knowledge Linting operations ─────────────────────────────────
    /// Store a lint finding (contradiction, gap, or connection).
    StoreLintFinding {
        finding_type: String,
        fact_id_a: Option<i64>,
        fact_id_b: Option<i64>,
        similarity: Option<f64>,
        llm_verdict: Option<String>,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Check if a lint finding already exists for the same fact pair.
    HasPendingLintFinding {
        finding_type: String,
        fact_id_a: i64,
        fact_id_b: i64,
        reply: oneshot::Sender<bool>,
    },
    /// List lint findings by type and status.
    ListLintFindings {
        finding_type: Option<String>,
        status: Option<String>,
        limit: i64,
        reply: oneshot::Sender<Vec<LintFinding>>,
    },
    /// List lint findings scoped to facts owned by a DID.
    ListLintFindingsForDid {
        user_did: String,
        status: Option<String>,
        limit: i64,
        reply: oneshot::Sender<Vec<LintFinding>>,
    },
    /// Update a lint finding status (resolve / dismiss).
    UpdateLintFindingStatus {
        id: i64,
        status: String,
        user_action: Option<String>,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Update a lint finding status only when one of its facts belongs to a DID.
    UpdateLintFindingStatusForDid {
        id: i64,
        user_did: String,
        status: String,
        user_action: Option<String>,
        reply: oneshot::Sender<Result<bool>>,
    },
    // ── Cost analytics operations ─────────────────────────────────────
    /// Query cost breakdown by label over the last N days.
    QueryCostByLabel {
        days: i32,
        reply: oneshot::Sender<Result<Vec<CostByLabel>>>,
    },
    /// Query cost breakdown by model over the last N days.
    QueryCostByModel {
        days: i32,
        reply: oneshot::Sender<Result<Vec<CostByModel>>>,
    },
    /// Query daily cost totals over the last N days.
    QueryDailyCostTotals {
        days: i32,
        reply: oneshot::Sender<Result<Vec<DailyCostTotal>>>,
    },
    /// Query local LLM cost coverage/completeness metadata over the last N days.
    QueryCostCoverage {
        days: i32,
        reply: oneshot::Sender<Result<CostCoverage>>,
    },
    /// Snapshot yesterday's LLM call log into llm_cost_daily rollup table.
    SnapshotDailyCostRollup,
    /// Backfill cost_usd for rows where it is NULL (using current pricing cache).
    BackfillCostUsd {
        reply: oneshot::Sender<Result<BackfillResult>>,
    },

    // ── Layer C: Companion Growth ────────────────────────────────────────────
    /// Get the singleton companion identity.
    GetCompanionIdentity {
        reply: oneshot::Sender<Result<CompanionIdentity>>,
    },
    /// Set the companion's user-given name.
    SetCompanionName {
        name: String,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Replace all personality weights at once.
    UpdatePersonalityWeights {
        weights: PersonalityWeights,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Update free-text communication style notes.
    UpdateCommunicationNotes {
        notes: String,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Transition to a new growth stage (milestone).
    UpdateGrowthStage {
        stage: GrowthStage,
        reply: oneshot::Sender<Result<()>>,
    },
    /// List all growth dimensions.
    ListGrowthDimensions {
        reply: oneshot::Sender<Result<Vec<GrowthDimension>>>,
    },
    /// Apply evidence to a dimension: updates depth (unbounded, min 0.0) +
    /// velocity (EMA alpha=0.1), increments evidence_count, appends growth_log.
    ApplyGrowthEvidence {
        dimension_key: String,
        delta: f64,
        detail: String,
        metadata: serde_json::Value,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Append a free-form event to the growth log.
    LogGrowthEvent {
        event: NewGrowthEvent,
        reply: oneshot::Sender<Result<i64>>,
    },
    /// Query recent growth log entries (optionally filtered by event_type).
    QueryGrowthLog {
        event_type: Option<String>,
        limit: i64,
        reply: oneshot::Sender<Result<Vec<GrowthEvent>>>,
    },
    /// Adjust a single personality trait by key (e.g. "warmth", "humor").
    AdjustPersonalityTrait {
        trait_key: String,
        value: f32,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Store thumbs-up/down feedback for an autonomous heartbeat notification.
    LogNotificationFeedback {
        notif_id: String,
        helpful: bool,
        reply: oneshot::Sender<Result<()>>,
    },
}

/// Status of the database size relative to its configured limit.
#[derive(Debug, Clone)]
pub struct DbSizeStatus {
    pub current_bytes: u64,
    pub max_bytes: u64,
    pub exceeded: bool,
}

// ── Cost analytics result types ──────────────────────────────────────────

/// Cost breakdown by usage label (e.g. "heartbeat", "user_chat").
#[derive(Debug, Clone)]
pub struct CostByLabel {
    pub label: String,
    pub total_cost: f64,
    pub call_count: i64,
    pub avg_latency_ms: f64,
}

/// Cost breakdown by model (e.g. "google/gemini-2.5-flash-preview").
#[derive(Debug, Clone)]
pub struct CostByModel {
    pub model: String,
    pub total_cost: f64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

/// Daily aggregate cost total.
#[derive(Debug, Clone)]
pub struct DailyCostTotal {
    pub date: String,
    pub cost: f64,
    pub calls: i64,
}

/// Metadata that describes how complete local llm_call_log cost analytics are.
#[derive(Debug, Clone)]
pub struct CostCoverage {
    pub row_count: i64,
    pub priced_row_count: i64,
    pub null_cost_rows: i64,
    pub api_row_count: i64,
    pub api_priced_row_count: i64,
    pub api_null_cost_rows: i64,
    pub zero_marginal_subscription_rows: i64,
    pub total_prompt_tokens: i64,
    pub total_completion_tokens: i64,
    pub total_cached_tokens: i64,
    pub total_tokens: i64,
    pub min_timestamp: Option<String>,
    pub max_timestamp: Option<String>,
    pub pricing_rows: i64,
    pub pricing_sources: i64,
    pub pricing_min_updated_at: Option<String>,
    pub pricing_max_updated_at: Option<String>,
}

/// Result of backfilling cost_usd for rows that were logged before pricing was available.
#[derive(Debug, Clone)]
pub struct BackfillResult {
    pub rows_updated: usize,
    pub rows_still_null: usize,
    pub models_missing: Vec<String>,
}

/// Configuration for the memory subsystem.
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Maximum number of recent messages in L1 cache per DID.
    pub l1_max_size: usize,
    /// Maximum database file size in bytes.
    pub max_db_size: u64,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            l1_max_size: DEFAULT_L1_MAX_SIZE,
            max_db_size: DEFAULT_MAX_DB_SIZE,
        }
    }
}

/// Handle to interact with the memory actor.
/// All operations go through a tokio channel → single-threaded SQLite writer.
#[derive(Clone)]
pub struct MemoryHandle {
    tx: mpsc::Sender<MemoryCommand>,
    #[allow(dead_code)]
    max_db_size: u64,
    /// Optional HNSW index for O(log N) vector search.
    /// Shared between the handle and the memory actor.
    hnsw: Option<std::sync::Arc<HnswIndex>>,
    /// Receiver for circuit breaker events (take once in main.rs).
    circuit_breaker_rx:
        std::sync::Arc<std::sync::Mutex<Option<mpsc::Receiver<CircuitBreakerEvent>>>>,
    /// JoinHandle for the memory actor thread — used to detect panics.
    actor_handle: Option<std::sync::Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>>,
    /// Path to the SQLite file (or ":memory:" for in-process tests). Exposed
    /// via [`MemoryHandle::db_path`] so features that need a raw rusqlite
    /// connection (e.g. `embedding_corpus::full_corpus_reembed`) can open
    /// their own read-write handle without racing with the actor.
    db_path: String,
    /// Phase E (W17): optional dual-write bridge. `None` = legacy-only
    /// (pre-Phase-E behavior, default for tests). `Some` = every shimmed
    /// method ALSO emits a heirloom event. Failure soft-failed (D1).
    pub(crate) heirloom_bridge:
        Option<std::sync::Arc<crate::memory_heirloom_bridge::HeirloomBridge>>,
}

// ── Actor proxy macros ──────────────────────────────────────────────
// Eliminate per-method boilerplate: create oneshot channel, send
// command to actor, await reply, convert channel errors.

/// Send a command and await the reply. Returns `Result<T>` where T is
/// the payload type of the oneshot::Sender in the command variant.
/// If T is itself `Result<U>`, caller adds `?` to flatten.
macro_rules! mem_send {
        ($self:expr, $variant:ident { $($field:ident : $val:expr),* $(,)? }) => {{
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            $self.tx
                .send(MemoryCommand::$variant { $($field: $val,)* reply: reply_tx })
                .await
                .map_err(|_| anyhow::anyhow!("memory actor closed"))?;
            reply_rx
                .await
                .map_err(|_| anyhow::anyhow!("memory actor dropped reply"))
        }};
        // Variant for commands that use `resp` instead of `reply`
        ($self:expr, $variant:ident { $($field:ident : $val:expr),* $(,)? } resp) => {{
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            $self.tx
                .send(MemoryCommand::$variant { $($field: $val,)* resp: reply_tx })
                .await
                .map_err(|_| anyhow::anyhow!("memory actor closed"))?;
            reply_rx
                .await
                .map_err(|_| anyhow::anyhow!("memory actor dropped reply"))
        }};
    }

/// Send a command and return a default on any failure (channel closed
/// or actor dropped reply). Used by non-Result methods that must never
/// propagate errors.
macro_rules! mem_send_or {
        ($self:expr, $variant:ident { $($field:ident : $val:expr),* $(,)? }, $default:expr, $name:literal) => {{
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            if let Err(e) = $self.tx
                .send(MemoryCommand::$variant { $($field: $val,)* reply: reply_tx })
                .await
            {
                tracing::error!(concat!("Memory channel send failed in ", $name, ": {}"), e);
            }
            match reply_rx.await {
                Ok(val) => val,
                Err(_) => {
                    tracing::warn!(concat!("Memory actor channel closed in ", $name));
                    $default
                }
            }
        }};
    }

// ── L1→L2 overflow summarization ─────────────────────────────────────────────

/// Extract key facts from an evicted L1 message for L2 storage.
///
/// Simple heuristic extraction (no LLM needed):
/// - Filters out very short messages (< 10 chars) as noise
/// - Truncates to 256 chars to keep L2 compact
/// - Prefixes with role for context
///
/// Note: L1 overflow facts are stored WITHOUT embeddings (sync context).
/// They serve as a safety net but are not discoverable via semantic search.
/// Important facts should be stored explicitly via `extract_immediate_fact`.
/// Escape special characters in a string for use in a SQLite LIKE pattern.
/// The ESCAPE char is backslash (`\`).
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            other => out.push(other),
        }
    }
    out
}

fn extract_facts_from_message(msg: &Message) -> String {
    let content = msg.content.trim();
    // Skip trivial messages
    if content.len() < 10 {
        return String::new();
    }
    let truncated = if content.chars().count() > 256 {
        let end: usize = content
            .char_indices()
            .nth(256)
            .map_or(content.len(), |(i, _)| i);
        format!("{}...", &content[..end])
    } else {
        content.to_string()
    };
    format!("[{}] {}", msg.role, truncated)
}

// ── Immediate L2 persistence for user messages ──────────────────────────────

/// Keywords (in multiple languages) that signal the user wants something remembered.
const REMEMBER_KEYWORDS: &[&str] = &[
    "记住",
    "记下",
    "记一下",
    "请记住",
    "帮我记",
    "别忘了",
    "remember",
    "note that",
    "keep in mind",
    "don't forget",
    "my ip",
    "my server",
    "my name is",
    "my email",
    "我的ip",
    "我的服务器",
    "我叫",
    "我的名字",
    "我的邮箱",
];

/// Credential-related keywords that should NEVER be persisted to L2 in plaintext.
pub const CREDENTIAL_KEYWORDS: &[&str] = &[
    "password",
    "密码",
    "token",
    "secret",
    "密钥",
    "private key",
    "api_key",
    "api key",
    "apikey",
    "ssh key",
    "access_key",
];

/// Check if text contains credential-related keywords that should not be persisted.
/// Shared between `extract_immediate_fact()` and the proactive learning pipeline.
pub fn contains_credential(text: &str) -> bool {
    let lower = text.to_lowercase();
    CREDENTIAL_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

/// Check if a user message contains factual information worth persisting to L2
/// immediately (without waiting for L1 overflow).
///
/// Returns `Some((topic, fact))` if the message should be stored, `None` otherwise.
/// Returns `None` for messages containing credentials (security: refuse to persist secrets).
pub fn extract_immediate_fact(user_message: &str) -> Option<(String, String)> {
    let msg = user_message.trim();
    // Skip short messages
    if msg.len() < 10 {
        return None;
    }

    let lower = msg.to_lowercase();

    // Check for explicit "remember" signals
    let is_remember = REMEMBER_KEYWORDS.iter().any(|kw| lower.contains(kw));

    if !is_remember {
        return None;
    }

    // Security: refuse to store credential-related content in plaintext
    if contains_credential(msg) {
        tracing::warn!(
            "Refusing to store credential-related content in L2 (plaintext secret protection)"
        );
        return None;
    }

    // Derive a simple topic from the message
    let topic = derive_topic(&lower);

    // Store the full message as the fact (truncated to 512 characters)
    let fact = if msg.chars().count() > 512 {
        let end: usize = msg.char_indices().nth(512).map_or(msg.len(), |(i, _)| i);
        format!("{}...", &msg[..end])
    } else {
        msg.to_string()
    };

    Some((topic, fact))
}

/// Derive a topic keyword from the message content.
fn derive_topic(lower: &str) -> String {
    let topic_hints = [
        (&["ip", "server", "服务器", "主机"][..], "server"),
        (&["name", "名字", "我叫"][..], "identity"),
        (&["email", "邮箱", "邮件"][..], "contact"),
        (&["config", "配置", "设置", "setting"][..], "config"),
        (&["prefer", "偏好", "喜欢"][..], "preference"),
    ];

    for (keywords, topic) in &topic_hints {
        if keywords.iter().any(|kw| lower.contains(kw)) {
            return topic.to_string();
        }
    }

    "user_fact".to_string()
}

/// Send a reply on a oneshot channel, logging a warning if the receiver was dropped.
/// This replaces the `let _ = reply.send(result)` anti-pattern that silently
/// swallows send failures throughout the memory actor loop.
macro_rules! mem_reply {
    ($reply:expr, $result:expr) => {
        if let Err(_unsent) = $reply.send($result) {
            tracing::warn!(
                "[MemoryActor] Reply channel dropped — caller disconnected before receiving result"
            );
        }
    };
}

/// Log and filter row parse results — replaces `.filter_map(row_ok)` which
/// silently swallowed corrupt rows.
fn row_ok<T>(r: rusqlite::Result<T>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(error = %e, "[MemoryActor] Row parse error — skipping corrupt row");
            None
        }
    }
}

// ── SQL dynamic UPDATE builder macros ─────────────────────────────────────
// Used by update_skill_proposal_row to build SET clauses
// from Option<T> fields.  Each macro appends a positional parameter and a
// `column = ?N` clause when the corresponding field is Some.

/// Append a cloneable field (String, Option<String>) to the UPDATE SET clause.
macro_rules! sql_set {
    ($params:expr, $clauses:expr, $updates:expr, $field:ident, $col:expr) => {
        if let Some(ref val) = $updates.$field {
            $params.push(Box::new(val.clone()));
            $clauses.push(format!("{} = ?{}", $col, $params.len()));
        }
    };
}

/// Append a bool field, cast to i32 for SQLite storage.
macro_rules! sql_set_bool {
    ($params:expr, $clauses:expr, $updates:expr, $field:ident, $col:expr) => {
        if let Some(val) = $updates.$field {
            $params.push(Box::new(val as i32));
            $clauses.push(format!("{} = ?{}", $col, $params.len()));
        }
    };
}

/// Append a numeric field (i64, f64, i32) that implements ToSql directly.
macro_rules! sql_set_num {
    ($params:expr, $clauses:expr, $updates:expr, $field:ident, $col:expr) => {
        if let Some(val) = $updates.$field {
            $params.push(Box::new(val));
            $clauses.push(format!("{} = ?{}", $col, $params.len()));
        }
    };
}

// ── Submodule declarations (Phase 0.3 mechanical split, pure code motion) ────
// mod decls come AFTER the macros above so `macro_rules!` (mem_send, mem_send_or,
// mem_reply, sql_set*) are in textual scope for the submodules.
mod actor;
mod handle;
mod handlers_a;
mod handlers_b;
mod partial_ref;
mod queries;
#[cfg(test)]
mod tests;

// Internal (private) re-exports: hoist submodule `pub(super)` free fns into this
// module's namespace so every submodule sees its siblings via `use super::*`.
use actor::*;
use handlers_a::*;
use handlers_b::*;
use partial_ref::{legacy_insert_kind, snapshot_fact_history, weaken_and_queue};
use queries::*;

// Public-surface preservation: these definitions physically live in queries.rs
// after the split; external callers still reach them as `crate::memory::X`.
pub use partial_ref::{
    ConsolidationOutcome, EpistemicKind, L2Contradiction, L2FactHistoryEntry, L2ProvenanceView,
    L2ProvenanceWrite, ProvenanceOutcome, ReplayEvent, ReplayReport, belief_is_active,
    fuse_hybrid_relevance, replay_memory_audit,
};
pub use queries::{
    compute_recall_score, extract_topic_from_message, format_self_awareness, topic_weight,
};
