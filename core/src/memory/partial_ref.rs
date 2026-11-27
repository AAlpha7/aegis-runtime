//! Hindsight partial-reference helpers for L2.
//!
//! Borrowed ideas only: epistemic kind, provenance, contradiction retention,
//! observation consolidation, and a lexical/temporal/topic signal fused into
//! the existing weight + MMR recall path. This is not an RRF ranker and it
//! does not vendor Hindsight.

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::{
    EmbeddingClient, MemoryTier, WriteDecision, externalize_long_fact, l2_write_guard,
    normalize_fact_text,
};

/// Unverified writes (no source receipt) cannot be stored as high-confidence facts.
pub const UNVERIFIED_CONFIDENCE_CAP: f64 = 0.45;

/// Existing beliefs at or above this confidence are protected from a conflicting source.
pub const HIGH_CONFIDENCE_PRIOR: f64 = 0.8;

/// Multiplier applied to a contradicted belief. The row is kept.
pub const WEAKEN_FACTOR: f64 = 0.5;

/// Floor after repeated contradiction weakening.
pub const WEAKEN_FLOOR: f64 = 0.05;

/// Pre-MMR fusion weights. Semantic score stays dominant; lexical, temporal,
/// and topic-token signals are light additives. Sum is 1.0. Not reciprocal
/// rank fusion.
pub const HYBRID_SEMANTIC_WEIGHT: f64 = 0.75;
pub const HYBRID_LEXICAL_WEIGHT: f64 = 0.15;
pub const HYBRID_TEMPORAL_WEIGHT: f64 = 0.07;
pub const HYBRID_GRAPH_WEIGHT: f64 = 0.03;

const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;
const BM25_AVGDL: f64 = 32.0;

/// How an L2 row should be read: asserted fact, tentative hypothesis, or retained episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpistemicKind {
    Fact,
    Hypothesis,
    Episode,
}

impl EpistemicKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Hypothesis => "hypothesis",
            Self::Episode => "episode",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "fact" => Some(Self::Fact),
            "hypothesis" => Some(Self::Hypothesis),
            "episode" => Some(Self::Episode),
            _ => None,
        }
    }
}

/// Result of applying the unverified-write cap.
#[derive(Debug, Clone, PartialEq)]
pub struct HardenedWrite {
    pub epistemic_kind: EpistemicKind,
    pub confidence: f64,
    pub verified: bool,
    pub source_receipt: Option<String>,
}

/// Caller-supplied provenance for one L2 write.
#[derive(Debug, Clone)]
pub struct L2ProvenanceWrite {
    pub did: String,
    pub topic: String,
    pub fact: String,
    pub embedding: Option<Vec<f32>>,
    /// `fact`, `hypothesis`, or `episode`. `None` infers from `topic`.
    pub epistemic_kind: Option<String>,
    pub confidence: f64,
    pub source_receipt: Option<String>,
    /// Inclusive belief boundary, RFC3339. Past values are rejected.
    pub valid_until: Option<String>,
    pub session_id: Option<String>,
    /// Stable claim identity. A different fact for the same key is a contradiction,
    /// not an in-place overwrite.
    pub claim_key: Option<String>,
}

/// What the provenance write did. History and the contradicted row stay readable.
#[derive(Debug, Clone, PartialEq)]
pub struct ProvenanceOutcome {
    pub id: i64,
    pub epistemic_kind: String,
    pub confidence: f64,
    pub verified: bool,
    pub contradiction_of: Option<i64>,
    pub history_retained: bool,
}

/// Open contradiction queued for owner review. Both fact ids still exist.
#[derive(Debug, Clone, PartialEq)]
pub struct L2Contradiction {
    pub id: i64,
    pub agent_did: String,
    pub kept_fact_id: i64,
    pub incoming_fact_id: i64,
    pub status: String,
    pub note: Option<String>,
    pub created_at: String,
}

/// Prior text of a fact, kept when a later write refreshes or contradicts it.
#[derive(Debug, Clone, PartialEq)]
pub struct L2FactHistoryEntry {
    pub id: i64,
    pub fact_id: i64,
    pub agent_did: String,
    pub previous_summary: String,
    pub previous_confidence: Option<f64>,
    pub previous_epistemic_kind: Option<String>,
    pub reason: String,
    pub created_at: String,
}

/// Ledger view of one L2 row, including provenance columns.
#[derive(Debug, Clone, PartialEq)]
pub struct L2ProvenanceView {
    pub id: i64,
    pub agent_did: String,
    pub topic: String,
    pub fact_summary: String,
    pub confidence: f64,
    pub epistemic_kind: String,
    pub source_receipt: Option<String>,
    pub valid_until: Option<String>,
    pub session_id: Option<String>,
    pub claim_key: Option<String>,
    pub weakened: bool,
}

/// Explicit observation consolidation. Source episodes are not deleted.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsolidationOutcome {
    pub consolidated_id: i64,
    pub retained_observation_ids: Vec<i64>,
    pub epistemic_kind: String,
    pub confidence: f64,
    pub summary: String,
}

/// One observation fed to [`plan_observation_consolidation`].
#[derive(Debug, Clone)]
pub struct ObservationInput {
    pub id: i64,
    pub text: String,
    pub epistemic_kind: EpistemicKind,
    pub source_receipt: Option<String>,
}

/// Planned consolidated belief. `retained_ids` are the observations that stay.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsolidationPlan {
    pub summary: String,
    pub retained_ids: Vec<i64>,
    pub epistemic_kind: EpistemicKind,
    pub confidence: f64,
}

/// In-memory audit event for [`replay_memory_audit`].
#[derive(Debug, Clone)]
pub struct ReplayEvent {
    pub topic: String,
    pub fact: String,
    pub confidence: f64,
    pub epistemic_kind: Option<EpistemicKind>,
    pub source_receipt: Option<String>,
    pub valid_until: Option<String>,
    pub session_id: Option<String>,
    pub claim_key: Option<String>,
}

/// One row produced by the audit replay.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayRow {
    pub id: u64,
    pub topic: String,
    pub fact: String,
    pub epistemic_kind: EpistemicKind,
    pub confidence: f64,
    pub source_receipt: Option<String>,
    pub valid_until: Option<String>,
    pub session_id: Option<String>,
    pub claim_key: Option<String>,
    pub weakened: bool,
}

/// Prior text recorded by the replay when a row was refreshed or contradicted.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayHistory {
    pub fact_id: u64,
    pub previous_summary: String,
    pub previous_confidence: f64,
    pub reason: String,
}

/// Result of replaying provenance, contradiction, and expiry rules.
/// This harness does not emit benchmark scores.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayReport {
    pub rows: Vec<ReplayRow>,
    pub contradictions: Vec<(u64, u64)>,
    pub history: Vec<ReplayHistory>,
    pub rejected_expired: usize,
}

struct ExistingClaim {
    id: i64,
    confidence: f64,
    source_receipt: Option<String>,
    session_id: Option<String>,
    fact_summary: String,
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn sanitize_confidence(confidence: f64) -> f64 {
    if confidence.is_finite() {
        confidence.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn now_secs() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn parse_valid_until(raw: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .map(|dt| dt.with_timezone(&Utc))
        .context("valid_until must be RFC3339")
}

/// A belief is active when it has no boundary or the boundary is still in the future.
/// Unparseable boundaries fail closed (inactive).
pub fn belief_is_active(valid_until: Option<&str>, now: DateTime<Utc>) -> bool {
    match nonempty(valid_until) {
        None => true,
        Some(raw) => parse_valid_until(raw).map(|dt| dt > now).unwrap_or(false),
    }
}

pub fn infer_epistemic_kind(topic: &str) -> EpistemicKind {
    let topic = topic.trim().to_ascii_lowercase();
    if topic.starts_with("episode:") || topic.starts_with("session:") {
        EpistemicKind::Episode
    } else if topic.starts_with("hypothesis:")
        || topic.starts_with("heartbeat:thought")
        || topic.contains("unverified")
    {
        EpistemicKind::Hypothesis
    } else {
        EpistemicKind::Fact
    }
}

/// Legacy inserts: infer kind from the topic. A sub-cap confidence cannot stay a fact.
pub(super) fn legacy_insert_kind(topic: &str, confidence: Option<f64>) -> String {
    let mut kind = infer_epistemic_kind(topic);
    if let Some(confidence) = confidence
        && sanitize_confidence(confidence) < UNVERIFIED_CONFIDENCE_CAP
        && kind == EpistemicKind::Fact
    {
        kind = EpistemicKind::Hypothesis;
    }
    kind.as_str().to_string()
}

/// Missing receipt downgrades an asserted fact to a hypothesis and caps confidence.
pub fn harden_unverified(
    kind: EpistemicKind,
    confidence: f64,
    receipt: Option<&str>,
) -> HardenedWrite {
    let confidence = sanitize_confidence(confidence);
    let receipt = nonempty(receipt).map(ToOwned::to_owned);
    if receipt.is_some() {
        return HardenedWrite {
            epistemic_kind: kind,
            confidence,
            verified: true,
            source_receipt: receipt,
        };
    }
    let epistemic_kind = if kind == EpistemicKind::Fact {
        EpistemicKind::Hypothesis
    } else {
        kind
    };
    HardenedWrite {
        epistemic_kind,
        confidence: confidence.min(UNVERIFIED_CONFIDENCE_CAP),
        verified: false,
        source_receipt: None,
    }
}

/// High-confidence sourced beliefs are not replaced by a different receipt or session.
pub fn refuses_silent_overwrite(
    existing_confidence: f64,
    existing_receipt: Option<&str>,
    incoming_receipt: Option<&str>,
    existing_session: Option<&str>,
    incoming_session: Option<&str>,
) -> bool {
    if sanitize_confidence(existing_confidence) < HIGH_CONFIDENCE_PRIOR {
        return false;
    }
    let receipt_conflict = match (nonempty(existing_receipt), nonempty(incoming_receipt)) {
        (Some(existing), Some(incoming)) => existing != incoming,
        (Some(_), None) => true,
        _ => false,
    };
    let session_conflict = match (nonempty(existing_session), nonempty(incoming_session)) {
        (Some(existing), Some(incoming)) => existing != incoming,
        _ => false,
    };
    receipt_conflict || session_conflict
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{4E00}'..='\u{9FFF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{3040}'..='\u{309F}'
            | '\u{30A0}'..='\u{30FF}'
            | '\u{AC00}'..='\u{D7AF}'
    )
}

fn tokenize(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut tokens = Vec::new();
    for word in lower.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if !word.is_empty() {
            tokens.push(word.to_string());
        }
    }
    let mut prev_cjk: Option<char> = None;
    for ch in lower.chars() {
        if is_cjk(ch) {
            tokens.push(ch.to_string());
            if let Some(prev) = prev_cjk {
                tokens.push(format!("{prev}{ch}"));
            }
            prev_cjk = Some(ch);
        } else {
            prev_cjk = None;
        }
    }
    tokens
}

/// Single-document BM25 saturation in `[0, 1]`. No corpus IDF and no inverted index.
pub fn bm25_lite(query: &str, document: &str) -> f64 {
    let query_terms = tokenize(query);
    if query_terms.is_empty() {
        return 0.0;
    }
    let doc_terms = tokenize(document);
    let doc_len = doc_terms.len() as f64;
    if doc_len == 0.0 {
        return 0.0;
    }
    let mut freq: std::collections::HashMap<&str, i32> = std::collections::HashMap::new();
    for term in &doc_terms {
        *freq.entry(term.as_str()).or_insert(0) += 1;
    }
    let mut seen = std::collections::HashSet::new();
    let mut score = 0.0;
    let mut unique = 0.0;
    for term in &query_terms {
        if !seen.insert(term.as_str()) {
            continue;
        }
        unique += 1.0;
        let tf = f64::from(*freq.get(term.as_str()).unwrap_or(&0));
        let denom = tf + BM25_K1 * (1.0 - BM25_B + BM25_B * doc_len / BM25_AVGDL);
        if denom > 0.0 {
            let saturated = tf * (BM25_K1 + 1.0) / denom;
            score += saturated / (BM25_K1 + 1.0);
        }
    }
    if unique == 0.0 {
        0.0
    } else {
        (score / unique).clamp(0.0, 1.0)
    }
}

/// Exponential freshness in `(0, 1]`. Unparseable timestamps stay neutral at 0.5.
pub fn temporal_freshness(created_at: &str, now: DateTime<Utc>) -> f64 {
    let Some(created) = DateTime::parse_from_rfc3339(created_at)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
    else {
        return 0.5;
    };
    let age_days = (now - created).num_seconds() as f64 / 86_400.0;
    let age_days = age_days.max(0.0);
    let lambda = 2.0_f64.ln() / 30.0;
    (-lambda * age_days).exp()
}

/// Shared token overlap between the query and the fact topic. Not a belief graph.
pub fn topic_graph_signal(query: &str, topic: &str) -> f64 {
    let query_terms = tokenize(query);
    if query_terms.is_empty() {
        return 0.0;
    }
    let topic_terms: std::collections::HashSet<String> = tokenize(topic).into_iter().collect();
    if topic_terms.is_empty() {
        return 0.0;
    }
    let mut seen = std::collections::HashSet::new();
    let mut hits = 0.0;
    let mut unique = 0.0;
    for term in &query_terms {
        if !seen.insert(term.as_str()) {
            continue;
        }
        unique += 1.0;
        if topic_terms.contains(term) {
            hits += 1.0;
        }
    }
    if unique == 0.0 { 0.0 } else { hits / unique }
}

/// Fuse a light lexical/temporal/topic signal into the score MMR already consumes.
pub fn fuse_hybrid_relevance(semantic: f64, lexical: f64, temporal: f64, graph: f64) -> f64 {
    HYBRID_SEMANTIC_WEIGHT * semantic.max(0.0)
        + HYBRID_LEXICAL_WEIGHT * lexical.clamp(0.0, 1.0)
        + HYBRID_TEMPORAL_WEIGHT * temporal.clamp(0.0, 1.0)
        + HYBRID_GRAPH_WEIGHT * graph.clamp(0.0, 1.0)
}

pub fn plan_observation_consolidation(
    observations: &[ObservationInput],
) -> Result<ConsolidationPlan> {
    if observations.len() < 2 {
        anyhow::bail!("consolidation needs at least two observations");
    }
    let retained_ids: Vec<i64> = observations.iter().map(|obs| obs.id).collect();
    let any_unverified = observations
        .iter()
        .any(|obs| nonempty(obs.source_receipt.as_deref()).is_none());
    let epistemic_kind = if any_unverified {
        EpistemicKind::Hypothesis
    } else {
        EpistemicKind::Fact
    };
    let mut parts = Vec::with_capacity(observations.len() + 1);
    parts.push(format!(
        "Consolidated {} observations (history retained).",
        observations.len()
    ));
    for obs in observations {
        parts.push(format!(
            "[#{} {}] {}",
            obs.id,
            obs.epistemic_kind.as_str(),
            obs.text.trim()
        ));
    }
    Ok(ConsolidationPlan {
        summary: parts.join(" "),
        retained_ids,
        epistemic_kind,
        confidence: if any_unverified {
            UNVERIFIED_CONFIDENCE_CAP
        } else {
            0.7
        },
    })
}

pub(super) fn snapshot_fact_history(
    conn: &Connection,
    fact_id: i64,
    did: &str,
    reason: &str,
) -> Result<bool> {
    let now = now_secs();
    let inserted = conn.execute(
        "INSERT INTO l2_fact_history (
            fact_id, agent_did, previous_summary, previous_confidence,
            previous_epistemic_kind, reason, created_at
         )
         SELECT id, agent_did, fact_summary, confidence, COALESCE(epistemic_kind, 'fact'), ?3, ?4
         FROM l2_experience
         WHERE id = ?1 AND agent_did = ?2",
        params![fact_id, did, reason, now],
    )?;
    Ok(inserted == 1)
}

pub(super) fn weaken_and_queue(
    conn: &Connection,
    did: &str,
    kept_id: i64,
    incoming_id: i64,
    note: &str,
) -> Result<()> {
    snapshot_fact_history(conn, kept_id, did, "contradiction")?;
    conn.execute(
        "UPDATE l2_experience
         SET confidence = MAX(?2, confidence * ?3), weakened = 1
         WHERE id = ?1 AND agent_did = ?4",
        params![kept_id, WEAKEN_FLOOR, WEAKEN_FACTOR, did],
    )?;
    conn.execute(
        "INSERT INTO l2_contradiction (
            agent_did, kept_fact_id, incoming_fact_id, status, note, created_at
         ) VALUES (?1, ?2, ?3, 'open', ?4, ?5)",
        params![did, kept_id, incoming_id, note, now_secs()],
    )?;
    Ok(())
}

fn normalize_boundary(valid_until: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = nonempty(valid_until) else {
        return Ok(None);
    };
    let dt = parse_valid_until(raw)?;
    if dt <= Utc::now() {
        anyhow::bail!("valid_until is already in the past");
    }
    Ok(Some(dt.to_rfc3339_opts(SecondsFormat::Secs, true)))
}

fn load_claim(
    conn: &Connection,
    did: &str,
    topic: &str,
    claim_key: &str,
) -> Result<Option<ExistingClaim>> {
    conn.query_row(
        "SELECT id, confidence, source_receipt, session_id, fact_summary
         FROM l2_experience
         WHERE agent_did = ?1 AND topic = ?2 AND claim_key = ?3
           AND (valid_until IS NULL OR valid_until > strftime('%Y-%m-%dT%H:%M:%SZ','now'))
         ORDER BY confidence DESC, id DESC
         LIMIT 1",
        params![did, topic, claim_key],
        |row| {
            Ok(ExistingClaim {
                id: row.get(0)?,
                confidence: row.get::<_, Option<f64>>(1)?.unwrap_or(0.6),
                source_receipt: row.get(2)?,
                session_id: row.get(3)?,
                fact_summary: row.get(4)?,
            })
        },
    )
    .optional()
    .context("load claim")
}

fn load_fact(conn: &Connection, did: &str, id: i64) -> Result<Option<ExistingClaim>> {
    conn.query_row(
        "SELECT id, confidence, source_receipt, session_id, fact_summary
         FROM l2_experience
         WHERE id = ?1 AND agent_did = ?2",
        params![id, did],
        |row| {
            Ok(ExistingClaim {
                id: row.get(0)?,
                confidence: row.get::<_, Option<f64>>(1)?.unwrap_or(0.6),
                source_receipt: row.get(2)?,
                session_id: row.get(3)?,
                fact_summary: row.get(4)?,
            })
        },
    )
    .optional()
    .context("load fact")
}

fn insert_fact(
    conn: &Connection,
    write: &L2ProvenanceWrite,
    hardened: &HardenedWrite,
    valid_until: Option<&str>,
    claim_key: Option<&str>,
) -> Result<i64> {
    let now = now_secs();
    let externalized = externalize_long_fact(&write.fact);
    let embedding = write.embedding.as_deref().map(EmbeddingClient::to_blob);
    conn.execute(
        "INSERT INTO l2_experience (
            agent_did, topic, fact_summary, created_at, embedding, last_accessed,
            confidence, epistemic_kind, source_receipt, valid_until, session_id,
            claim_key, weakened
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?4, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
        params![
            write.did,
            write.topic,
            externalized,
            now,
            embedding,
            hardened.confidence,
            hardened.epistemic_kind.as_str(),
            hardened.source_receipt,
            valid_until,
            nonempty(write.session_id.as_deref()),
            claim_key,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn refresh_fact(
    conn: &Connection,
    did: &str,
    id: i64,
    write: &L2ProvenanceWrite,
    hardened: &HardenedWrite,
    valid_until: Option<&str>,
) -> Result<()> {
    snapshot_fact_history(conn, id, did, "update")?;
    let now = now_secs();
    let externalized = externalize_long_fact(&write.fact);
    let embedding = write.embedding.as_deref().map(EmbeddingClient::to_blob);
    conn.execute(
        "UPDATE l2_experience
         SET fact_summary = ?1, last_accessed = ?2, confidence = ?3, epistemic_kind = ?4,
             source_receipt = ?5, valid_until = ?6, session_id = ?7, embedding = COALESCE(?8, embedding)
         WHERE id = ?9 AND agent_did = ?10",
        params![
            externalized,
            now,
            hardened.confidence,
            hardened.epistemic_kind.as_str(),
            hardened.source_receipt,
            valid_until,
            nonempty(write.session_id.as_deref()),
            embedding,
            id,
            did,
        ],
    )?;
    Ok(())
}

fn contradict(
    conn: &Connection,
    write: &L2ProvenanceWrite,
    hardened: &HardenedWrite,
    valid_until: Option<&str>,
    claim_key: Option<&str>,
    existing: &ExistingClaim,
    note: &str,
) -> Result<ProvenanceOutcome> {
    let new_id = insert_fact(conn, write, hardened, valid_until, claim_key)?;
    weaken_and_queue(conn, &write.did, existing.id, new_id, note)?;
    Ok(ProvenanceOutcome {
        id: new_id,
        epistemic_kind: hardened.epistemic_kind.as_str().to_string(),
        confidence: hardened.confidence,
        verified: hardened.verified,
        contradiction_of: Some(existing.id),
        history_retained: true,
    })
}

pub(super) fn apply_provenance_write(
    conn: &Connection,
    write: &L2ProvenanceWrite,
) -> Result<ProvenanceOutcome> {
    if write.did.trim().is_empty() || write.topic.trim().is_empty() || write.fact.trim().is_empty()
    {
        anyhow::bail!("provenance write requires did, topic, and fact");
    }
    let requested = match write.epistemic_kind.as_deref() {
        Some(raw) => EpistemicKind::parse(raw)
            .context("epistemic_kind must be fact, hypothesis, or episode")?,
        None => infer_epistemic_kind(&write.topic),
    };
    let hardened = harden_unverified(requested, write.confidence, write.source_receipt.as_deref());
    let valid_until = normalize_boundary(write.valid_until.as_deref())?;
    let claim_key = nonempty(write.claim_key.as_deref()).map(ToOwned::to_owned);
    let tx = conn.unchecked_transaction()?;
    let outcome = apply_in_tx(
        &tx,
        write,
        &hardened,
        valid_until.as_deref(),
        claim_key.as_deref(),
    )?;
    tx.commit()?;
    Ok(outcome)
}

fn apply_in_tx(
    conn: &Connection,
    write: &L2ProvenanceWrite,
    hardened: &HardenedWrite,
    valid_until: Option<&str>,
    claim_key: Option<&str>,
) -> Result<ProvenanceOutcome> {
    if let Some(claim_key) = claim_key
        && let Some(existing) = load_claim(conn, &write.did, &write.topic, claim_key)?
    {
        return apply_claim(conn, write, hardened, valid_until, claim_key, &existing);
    }

    let embedding = write.embedding.as_deref().map(EmbeddingClient::to_blob);
    let decision = l2_write_guard(
        conn,
        &write.did,
        &write.topic,
        &write.fact,
        embedding.as_deref(),
        MemoryTier::Facts,
    );
    match decision {
        WriteDecision::Skip => Ok(ProvenanceOutcome {
            id: -1,
            epistemic_kind: hardened.epistemic_kind.as_str().to_string(),
            confidence: hardened.confidence,
            verified: hardened.verified,
            contradiction_of: None,
            history_retained: false,
        }),
        WriteDecision::Update(existing_id) => {
            let Some(existing) = load_fact(conn, &write.did, existing_id)? else {
                anyhow::bail!("write guard referenced missing fact {existing_id}");
            };
            apply_cosine_update(conn, write, hardened, valid_until, claim_key, &existing)
        }
        WriteDecision::InsertWithContradiction(existing_id) => {
            let Some(existing) = load_fact(conn, &write.did, existing_id)? else {
                anyhow::bail!("write guard referenced missing fact {existing_id}");
            };
            contradict(
                conn,
                write,
                hardened,
                valid_until,
                claim_key,
                &existing,
                "write_guard",
            )
        }
        WriteDecision::Insert => {
            let id = insert_fact(conn, write, hardened, valid_until, claim_key)?;
            Ok(ProvenanceOutcome {
                id,
                epistemic_kind: hardened.epistemic_kind.as_str().to_string(),
                confidence: hardened.confidence,
                verified: hardened.verified,
                contradiction_of: None,
                history_retained: false,
            })
        }
    }
}

fn unchanged_outcome(existing: &ExistingClaim, hardened: &HardenedWrite) -> ProvenanceOutcome {
    ProvenanceOutcome {
        id: existing.id,
        epistemic_kind: hardened.epistemic_kind.as_str().to_string(),
        confidence: existing.confidence,
        verified: hardened.verified,
        contradiction_of: None,
        history_retained: false,
    }
}

fn refreshed_outcome(existing_id: i64, hardened: &HardenedWrite) -> ProvenanceOutcome {
    ProvenanceOutcome {
        id: existing_id,
        epistemic_kind: hardened.epistemic_kind.as_str().to_string(),
        confidence: hardened.confidence,
        verified: hardened.verified,
        contradiction_of: None,
        history_retained: true,
    }
}

fn is_protected(
    existing: &ExistingClaim,
    write: &L2ProvenanceWrite,
    hardened: &HardenedWrite,
) -> bool {
    refuses_silent_overwrite(
        existing.confidence,
        existing.source_receipt.as_deref(),
        hardened.source_receipt.as_deref(),
        existing.session_id.as_deref(),
        write.session_id.as_deref(),
    )
}

/// Same claim key, different text: keep both. Identical text refreshes unless a
/// conflicting source would raise a protected prior.
fn apply_claim(
    conn: &Connection,
    write: &L2ProvenanceWrite,
    hardened: &HardenedWrite,
    valid_until: Option<&str>,
    claim_key: &str,
    existing: &ExistingClaim,
) -> Result<ProvenanceOutcome> {
    let same_text = normalize_fact_text(&existing.fact_summary) == normalize_fact_text(&write.fact);
    if same_text && is_protected(existing, write, hardened) {
        return Ok(unchanged_outcome(existing, hardened));
    }
    if same_text {
        refresh_fact(conn, &write.did, existing.id, write, hardened, valid_until)?;
        return Ok(refreshed_outcome(existing.id, hardened));
    }
    contradict(
        conn,
        write,
        hardened,
        valid_until,
        Some(claim_key),
        existing,
        "claim_key",
    )
}

/// Cosine duplicate: refresh the row and keep the previous text in history.
/// A conflicting source cannot replace a protected prior; it is queued instead.
fn apply_cosine_update(
    conn: &Connection,
    write: &L2ProvenanceWrite,
    hardened: &HardenedWrite,
    valid_until: Option<&str>,
    claim_key: Option<&str>,
    existing: &ExistingClaim,
) -> Result<ProvenanceOutcome> {
    let same_text = normalize_fact_text(&existing.fact_summary) == normalize_fact_text(&write.fact);
    if is_protected(existing, write, hardened) {
        if same_text {
            return Ok(unchanged_outcome(existing, hardened));
        }
        return contradict(
            conn,
            write,
            hardened,
            valid_until,
            claim_key,
            existing,
            "write_guard",
        );
    }
    refresh_fact(conn, &write.did, existing.id, write, hardened, valid_until)?;
    Ok(refreshed_outcome(existing.id, hardened))
}

pub(super) fn list_contradictions(conn: &Connection, did: &str) -> Result<Vec<L2Contradiction>> {
    let mut stmt = conn.prepare(
        "SELECT id, agent_did, kept_fact_id, incoming_fact_id, status, note, created_at
         FROM l2_contradiction
         WHERE agent_did = ?1 AND status = 'open'
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map(params![did], |row| {
        Ok(L2Contradiction {
            id: row.get(0)?,
            agent_did: row.get(1)?,
            kept_fact_id: row.get(2)?,
            incoming_fact_id: row.get(3)?,
            status: row.get(4)?,
            note: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("list contradictions")
}

pub(super) fn list_fact_history(
    conn: &Connection,
    did: &str,
    fact_id: i64,
) -> Result<Vec<L2FactHistoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT h.id, h.fact_id, h.agent_did, h.previous_summary, h.previous_confidence,
                h.previous_epistemic_kind, h.reason, h.created_at
         FROM l2_fact_history h
         JOIN l2_experience e ON e.id = h.fact_id
         WHERE h.fact_id = ?1 AND e.agent_did = ?2
         ORDER BY h.id ASC",
    )?;
    let rows = stmt.query_map(params![fact_id, did], |row| {
        Ok(L2FactHistoryEntry {
            id: row.get(0)?,
            fact_id: row.get(1)?,
            agent_did: row.get(2)?,
            previous_summary: row.get(3)?,
            previous_confidence: row.get(4)?,
            previous_epistemic_kind: row.get(5)?,
            reason: row.get(6)?,
            created_at: row.get(7)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("list fact history")
}

pub(super) fn get_provenance_view(
    conn: &Connection,
    did: &str,
    id: i64,
) -> Result<Option<L2ProvenanceView>> {
    conn.query_row(
        "SELECT id, agent_did, topic, fact_summary, confidence, epistemic_kind,
                source_receipt, valid_until, session_id, claim_key, weakened
         FROM l2_experience
         WHERE id = ?1 AND agent_did = ?2",
        params![id, did],
        |row| {
            Ok(L2ProvenanceView {
                id: row.get(0)?,
                agent_did: row.get(1)?,
                topic: row.get(2)?,
                fact_summary: row.get(3)?,
                confidence: row.get::<_, Option<f64>>(4)?.unwrap_or(0.6),
                epistemic_kind: row
                    .get::<_, Option<String>>(5)?
                    .unwrap_or_else(|| "fact".into()),
                source_receipt: row.get(6)?,
                valid_until: row.get(7)?,
                session_id: row.get(8)?,
                claim_key: row.get(9)?,
                weakened: row.get::<_, Option<i64>>(10)?.unwrap_or(0) != 0,
            })
        },
    )
    .optional()
    .context("get provenance")
}

pub(super) fn consolidate_observations(
    conn: &Connection,
    did: &str,
    topic: &str,
) -> Result<ConsolidationOutcome> {
    let mut stmt = conn.prepare(
        "SELECT id, fact_summary, epistemic_kind, source_receipt
         FROM l2_experience
         WHERE agent_did = ?1 AND topic = ?2 AND epistemic_kind = 'episode'
           AND (valid_until IS NULL OR valid_until > strftime('%Y-%m-%dT%H:%M:%SZ','now'))
           AND id NOT IN (SELECT observation_id FROM l2_consolidation_link)
         ORDER BY id ASC",
    )?;
    let observations = stmt
        .query_map(params![did, topic], |row| {
            let kind_raw: String = row.get(2)?;
            Ok(ObservationInput {
                id: row.get(0)?,
                text: row.get(1)?,
                epistemic_kind: EpistemicKind::parse(&kind_raw).unwrap_or(EpistemicKind::Episode),
                source_receipt: row.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let plan = plan_observation_consolidation(&observations)?;
    let tx = conn.unchecked_transaction()?;
    let receipt = format!(
        "consolidation:{}",
        plan.retained_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    let write = L2ProvenanceWrite {
        did: did.to_string(),
        topic: topic.to_string(),
        fact: plan.summary.clone(),
        embedding: None,
        epistemic_kind: Some(plan.epistemic_kind.as_str().to_string()),
        confidence: plan.confidence,
        source_receipt: Some(receipt),
        valid_until: None,
        session_id: None,
        claim_key: None,
    };
    let hardened = harden_unverified(
        plan.epistemic_kind,
        plan.confidence,
        write.source_receipt.as_deref(),
    );
    let consolidated_id = insert_fact(&tx, &write, &hardened, None, None)?;
    let now = now_secs();
    for observation_id in &plan.retained_ids {
        tx.execute(
            "INSERT INTO l2_consolidation_link (
                agent_did, observation_id, consolidated_fact_id, created_at
             ) VALUES (?1, ?2, ?3, ?4)",
            params![did, observation_id, consolidated_id, now],
        )?;
    }
    tx.commit()?;
    Ok(ConsolidationOutcome {
        consolidated_id,
        retained_observation_ids: plan.retained_ids,
        epistemic_kind: plan.epistemic_kind.as_str().to_string(),
        confidence: plan.confidence,
        summary: plan.summary,
    })
}

pub(super) fn handle_partial_ref_cmd(conn: &Connection, cmd: super::MemoryCommand) {
    match cmd {
        super::MemoryCommand::StoreL2Provenance { write, reply } => {
            mem_reply!(reply, apply_provenance_write(conn, &write));
        }
        super::MemoryCommand::ListL2Contradictions { did, reply } => {
            mem_reply!(reply, list_contradictions(conn, &did));
        }
        super::MemoryCommand::ListL2FactHistory {
            did,
            fact_id,
            reply,
        } => {
            mem_reply!(reply, list_fact_history(conn, &did, fact_id));
        }
        super::MemoryCommand::GetL2Provenance { did, id, reply } => {
            mem_reply!(reply, get_provenance_view(conn, &did, id));
        }
        super::MemoryCommand::ConsolidateL2Observations { did, topic, reply } => {
            mem_reply!(reply, consolidate_observations(conn, &did, &topic));
        }
        _ => {
            tracing::error!("handle_partial_ref_cmd received a non-provenance command");
        }
    }
}

struct MutableReplayRow {
    id: u64,
    topic: String,
    fact: String,
    epistemic_kind: EpistemicKind,
    confidence: f64,
    source_receipt: Option<String>,
    valid_until: Option<String>,
    session_id: Option<String>,
    claim_key: Option<String>,
    weakened: bool,
}

impl MutableReplayRow {
    fn snapshot(&self) -> ReplayRow {
        ReplayRow {
            id: self.id,
            topic: self.topic.clone(),
            fact: self.fact.clone(),
            epistemic_kind: self.epistemic_kind,
            confidence: self.confidence,
            source_receipt: self.source_receipt.clone(),
            valid_until: self.valid_until.clone(),
            session_id: self.session_id.clone(),
            claim_key: self.claim_key.clone(),
            weakened: self.weakened,
        }
    }
}

/// Replay claim-key provenance, contradiction retention, and expiry rejection.
/// Cosine write-guard behavior stays covered by the SQLite path.
pub fn replay_memory_audit(events: &[ReplayEvent], now: DateTime<Utc>) -> ReplayReport {
    let mut rows: Vec<MutableReplayRow> = Vec::new();
    let mut contradictions = Vec::new();
    let mut history = Vec::new();
    let mut rejected_expired = 0usize;
    let mut next_id = 1u64;

    for event in events {
        if nonempty(event.valid_until.as_deref())
            .is_some_and(|raw| !belief_is_active(Some(raw), now))
        {
            rejected_expired += 1;
            continue;
        }
        let requested = event
            .epistemic_kind
            .unwrap_or_else(|| infer_epistemic_kind(&event.topic));
        let hardened =
            harden_unverified(requested, event.confidence, event.source_receipt.as_deref());
        let claim_key = nonempty(event.claim_key.as_deref()).map(ToOwned::to_owned);
        let existing_idx = claim_key.as_ref().and_then(|key| {
            rows.iter().rposition(|row| {
                row.topic == event.topic
                    && row.claim_key.as_deref() == Some(key.as_str())
                    && belief_is_active(row.valid_until.as_deref(), now)
            })
        });
        if let Some(idx) = existing_idx {
            let same_text =
                normalize_fact_text(&rows[idx].fact) == normalize_fact_text(&event.fact);
            let protected = refuses_silent_overwrite(
                rows[idx].confidence,
                rows[idx].source_receipt.as_deref(),
                hardened.source_receipt.as_deref(),
                rows[idx].session_id.as_deref(),
                event.session_id.as_deref(),
            );
            if same_text && protected {
                continue;
            }
            if same_text {
                history.push(ReplayHistory {
                    fact_id: rows[idx].id,
                    previous_summary: rows[idx].fact.clone(),
                    previous_confidence: rows[idx].confidence,
                    reason: "update".to_string(),
                });
                rows[idx].fact = event.fact.clone();
                rows[idx].confidence = hardened.confidence;
                rows[idx].epistemic_kind = hardened.epistemic_kind;
                rows[idx].source_receipt = hardened.source_receipt.clone();
                continue;
            }
            history.push(ReplayHistory {
                fact_id: rows[idx].id,
                previous_summary: rows[idx].fact.clone(),
                previous_confidence: rows[idx].confidence,
                reason: "contradiction".to_string(),
            });
            rows[idx].confidence = (rows[idx].confidence * WEAKEN_FACTOR).max(WEAKEN_FLOOR);
            rows[idx].weakened = true;
            let kept_id = rows[idx].id;
            let incoming_id = next_id;
            next_id += 1;
            contradictions.push((kept_id, incoming_id));
            rows.push(MutableReplayRow {
                id: incoming_id,
                topic: event.topic.clone(),
                fact: event.fact.clone(),
                epistemic_kind: hardened.epistemic_kind,
                confidence: hardened.confidence,
                source_receipt: hardened.source_receipt.clone(),
                valid_until: event.valid_until.clone(),
                session_id: event.session_id.clone(),
                claim_key: claim_key.clone(),
                weakened: false,
            });
            continue;
        }
        let id = next_id;
        next_id += 1;
        rows.push(MutableReplayRow {
            id,
            topic: event.topic.clone(),
            fact: event.fact.clone(),
            epistemic_kind: hardened.epistemic_kind,
            confidence: hardened.confidence,
            source_receipt: hardened.source_receipt,
            valid_until: event.valid_until.clone(),
            session_id: event.session_id.clone(),
            claim_key,
            weakened: false,
        });
    }

    ReplayReport {
        rows: rows.iter().map(MutableReplayRow::snapshot).collect(),
        contradictions,
        history,
        rejected_expired,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "actual {actual} expected {expected}"
        );
    }

    #[test]
    fn unverified_fact_becomes_capped_hypothesis() {
        let hardened = harden_unverified(EpistemicKind::Fact, 0.95, None);
        assert_eq!(hardened.epistemic_kind, EpistemicKind::Hypothesis);
        approx(hardened.confidence, UNVERIFIED_CONFIDENCE_CAP);
        assert!(!hardened.verified);
    }

    #[test]
    fn receipt_keeps_asserted_fact() {
        let hardened = harden_unverified(EpistemicKind::Fact, 0.91, Some("rcpt-1"));
        assert_eq!(hardened.epistemic_kind, EpistemicKind::Fact);
        approx(hardened.confidence, 0.91);
        assert!(hardened.verified);
    }

    #[test]
    fn episode_without_receipt_stays_episode_but_is_capped() {
        let hardened = harden_unverified(EpistemicKind::Episode, 0.99, Some("  "));
        assert_eq!(hardened.epistemic_kind, EpistemicKind::Episode);
        approx(hardened.confidence, UNVERIFIED_CONFIDENCE_CAP);
    }

    #[test]
    fn topic_inference_matches_prefixes() {
        assert_eq!(infer_epistemic_kind("episode:trip"), EpistemicKind::Episode);
        assert_eq!(
            infer_epistemic_kind("heartbeat:thought:night"),
            EpistemicKind::Hypothesis
        );
        assert_eq!(infer_epistemic_kind("learned:context"), EpistemicKind::Fact);
    }

    #[test]
    fn past_boundary_is_inactive_and_corrupt_boundary_fails_closed() {
        let now = DateTime::parse_from_rfc3339("2026-09-26T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(belief_is_active(None, now));
        assert!(!belief_is_active(Some("2026-09-25T00:00:00Z"), now));
        assert!(belief_is_active(Some("2026-09-27T00:00:00Z"), now));
        assert!(!belief_is_active(Some("not-a-date"), now));
    }

    #[test]
    fn cross_session_prior_blocks_a_different_receipt() {
        assert!(refuses_silent_overwrite(
            0.9,
            Some("rcpt-a"),
            Some("rcpt-b"),
            Some("session-1"),
            Some("session-2"),
        ));
        assert!(!refuses_silent_overwrite(
            0.9,
            Some("rcpt-a"),
            Some("rcpt-a"),
            Some("session-1"),
            Some("session-1"),
        ));
        assert!(!refuses_silent_overwrite(
            0.4,
            Some("rcpt-a"),
            Some("rcpt-b"),
            None,
            None,
        ));
    }

    #[test]
    fn bm25_prefers_overlapping_terms_including_cjk() {
        let hit = bm25_lite("seattle residence", "seattle residence downtown");
        let miss = bm25_lite("seattle residence", "purple widget");
        assert!(hit > miss);
        assert!(miss < 1e-9);
        let cjk = bm25_lite("住在西雅图", "用户住在西雅图");
        assert!(cjk > 0.0);
    }

    #[test]
    fn fusion_lifts_a_lexical_hit_above_a_weaker_semantic_score() {
        let semantic_only = fuse_hybrid_relevance(0.20, 0.0, 0.0, 0.0);
        let lexical_hit = fuse_hybrid_relevance(0.05, 1.0, 0.0, 0.0);
        assert!(lexical_hit > semantic_only);
    }

    #[test]
    fn consolidation_retains_ids_and_downgrades_unverified_input() {
        let plan = plan_observation_consolidation(&[
            ObservationInput {
                id: 1,
                text: "saw rain".into(),
                epistemic_kind: EpistemicKind::Episode,
                source_receipt: Some("obs-1".into()),
            },
            ObservationInput {
                id: 2,
                text: "saw sun".into(),
                epistemic_kind: EpistemicKind::Episode,
                source_receipt: None,
            },
        ])
        .unwrap();
        assert_eq!(plan.retained_ids, vec![1, 2]);
        assert_eq!(plan.epistemic_kind, EpistemicKind::Hypothesis);
        assert!(plan.summary.contains("[#1 episode]"));
        assert!(plan.summary.contains("[#2 episode]"));
        approx(plan.confidence, UNVERIFIED_CONFIDENCE_CAP);
    }

    #[test]
    fn replay_blocks_pollution_keeps_contradiction_and_rejects_expiry() {
        let now = DateTime::parse_from_rfc3339("2026-09-26T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let report = replay_memory_audit(
            &[
                ReplayEvent {
                    topic: "profile:residence".into(),
                    fact: "lives in Seattle".into(),
                    confidence: 0.99,
                    epistemic_kind: Some(EpistemicKind::Fact),
                    source_receipt: None,
                    valid_until: None,
                    session_id: Some("s1".into()),
                    claim_key: Some("residence".into()),
                },
                ReplayEvent {
                    topic: "profile:residence".into(),
                    fact: "lives in Seattle".into(),
                    confidence: 0.99,
                    epistemic_kind: Some(EpistemicKind::Fact),
                    source_receipt: Some("rcpt-a".into()),
                    valid_until: None,
                    session_id: Some("s1".into()),
                    claim_key: Some("residence".into()),
                },
                ReplayEvent {
                    topic: "profile:residence".into(),
                    fact: "lives in Boston".into(),
                    confidence: 0.95,
                    epistemic_kind: Some(EpistemicKind::Fact),
                    source_receipt: Some("rcpt-b".into()),
                    valid_until: None,
                    session_id: Some("s2".into()),
                    claim_key: Some("residence".into()),
                },
                ReplayEvent {
                    topic: "profile:residence".into(),
                    fact: "lives on the moon".into(),
                    confidence: 0.99,
                    epistemic_kind: Some(EpistemicKind::Fact),
                    source_receipt: Some("rcpt-c".into()),
                    valid_until: Some("2020-01-01T00:00:00Z".into()),
                    session_id: Some("s3".into()),
                    claim_key: Some("residence".into()),
                },
            ],
            now,
        );

        assert_eq!(report.rejected_expired, 1);
        assert!(
            report
                .rows
                .iter()
                .any(|row| row.fact.contains("Seattle") && !row.weakened)
                || report
                    .history
                    .iter()
                    .any(|row| row.previous_summary.contains("Seattle"))
        );
        let boston = report
            .rows
            .iter()
            .find(|row| row.fact.contains("Boston"))
            .unwrap();
        let seattle = report
            .rows
            .iter()
            .find(|row| row.fact.contains("Seattle"))
            .unwrap();
        assert!(seattle.weakened);
        assert!(seattle.confidence < 0.9);
        assert!(!boston.weakened);
        assert_eq!(report.contradictions.len(), 1);
        assert!(
            report
                .history
                .iter()
                .any(|row| row.previous_summary.contains("Seattle"))
        );
        assert!(
            report.rows.iter().all(
                |row| row.epistemic_kind != EpistemicKind::Fact || row.source_receipt.is_some()
            )
        );
    }

    #[tokio::test]
    async fn sqlite_provenance_retains_contradiction_and_consolidates_episodes() {
        let mem = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
        let did = "did:aegis:ed25519:abc";
        let first = mem
            .store_l2_with_provenance(L2ProvenanceWrite {
                did: did.into(),
                topic: "profile:residence".into(),
                fact: "User lives in Seattle".into(),
                embedding: None,
                epistemic_kind: Some("fact".into()),
                confidence: 0.92,
                source_receipt: Some("rcpt-a".into()),
                valid_until: Some("2099-01-01T00:00:00Z".into()),
                session_id: Some("session-1".into()),
                claim_key: Some("residence".into()),
            })
            .await
            .unwrap();
        assert_eq!(first.epistemic_kind, "fact");
        assert!(first.verified);

        let polluted = mem
            .store_l2_with_provenance(L2ProvenanceWrite {
                did: did.into(),
                topic: "profile:note".into(),
                fact: "User is definitely a cat".into(),
                embedding: None,
                epistemic_kind: Some("fact".into()),
                confidence: 0.99,
                source_receipt: None,
                valid_until: None,
                session_id: Some("session-2".into()),
                claim_key: None,
            })
            .await
            .unwrap();
        assert_eq!(polluted.epistemic_kind, "hypothesis");
        assert!(polluted.confidence <= UNVERIFIED_CONFIDENCE_CAP);

        let expired = mem
            .store_l2_with_provenance(L2ProvenanceWrite {
                did: did.into(),
                topic: "profile:residence".into(),
                fact: "User lives on Mars".into(),
                embedding: None,
                epistemic_kind: Some("fact".into()),
                confidence: 0.9,
                source_receipt: Some("rcpt-old".into()),
                valid_until: Some("2000-01-01T00:00:00Z".into()),
                session_id: Some("session-old".into()),
                claim_key: Some("residence".into()),
            })
            .await;
        assert!(expired.is_err());

        let second = mem
            .store_l2_with_provenance(L2ProvenanceWrite {
                did: did.into(),
                topic: "profile:residence".into(),
                fact: "User lives in Boston".into(),
                embedding: None,
                epistemic_kind: Some("fact".into()),
                confidence: 0.93,
                source_receipt: Some("rcpt-b".into()),
                valid_until: Some("2099-01-01T00:00:00Z".into()),
                session_id: Some("session-2".into()),
                claim_key: Some("residence".into()),
            })
            .await
            .unwrap();
        assert_eq!(second.contradiction_of, Some(first.id));

        let kept = mem.get_l2_provenance(did, first.id).await.unwrap().unwrap();
        assert!(kept.fact_summary.contains("Seattle"));
        assert!(kept.weakened);
        assert!(kept.confidence < 0.92);
        let incoming = mem
            .get_l2_provenance(did, second.id)
            .await
            .unwrap()
            .unwrap();
        assert!(incoming.fact_summary.contains("Boston"));
        let queue = mem.list_l2_contradictions(did).await.unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].kept_fact_id, first.id);
        let history = mem.list_l2_fact_history(did, first.id).await.unwrap();
        assert!(
            history
                .iter()
                .any(|row| row.previous_summary.contains("Seattle"))
        );

        for (idx, text) in ["landed in rain", "left at dusk"].into_iter().enumerate() {
            mem.store_l2_with_provenance(L2ProvenanceWrite {
                did: did.into(),
                topic: "episode:trip".into(),
                fact: text.into(),
                embedding: None,
                epistemic_kind: Some("episode".into()),
                confidence: 0.8,
                source_receipt: Some(format!("obs-{idx}")),
                valid_until: None,
                session_id: Some("session-1".into()),
                claim_key: None,
            })
            .await
            .unwrap();
        }
        let consolidated = mem
            .consolidate_l2_observations(did, "episode:trip")
            .await
            .unwrap();
        assert_eq!(consolidated.retained_observation_ids.len(), 2);
        assert_eq!(consolidated.epistemic_kind, "fact");
        for id in &consolidated.retained_observation_ids {
            let view = mem.get_l2_provenance(did, *id).await.unwrap().unwrap();
            assert_eq!(view.epistemic_kind, "episode");
        }
        let belief = mem
            .get_l2_provenance(did, consolidated.consolidated_id)
            .await
            .unwrap()
            .unwrap();
        assert!(belief.fact_summary.contains("[#"));
    }

    #[test]
    fn semantic_search_keeps_lexical_hits_and_drops_expired_boundaries() {
        use super::super::queries::semantic_search;

        let conn = Connection::open_in_memory().unwrap();
        crate::memory_migrations::run_all_migrations(&conn).unwrap();
        let did = "did:aegis:ed25519:search";
        let aligned = EmbeddingClient::to_blob(&[1.0, 0.0, 0.0]);
        let orthogonal = EmbeddingClient::to_blob(&[0.0, 1.0, 0.0]);
        let now = now_secs();
        conn.execute(
            "INSERT INTO l2_experience (
                agent_did, topic, fact_summary, created_at, embedding, last_accessed,
                confidence, epistemic_kind, valid_until, weakened
             ) VALUES (?1, 'notes', 'purple widget', ?2, ?3, ?2, 0.6, 'fact', NULL, 0)",
            params![did, now, aligned],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO l2_experience (
                agent_did, topic, fact_summary, created_at, embedding, last_accessed,
                confidence, epistemic_kind, valid_until, weakened
             ) VALUES (?1, 'residence', 'seattle residence downtown', ?2, ?3, ?2, 0.6, 'fact', NULL, 0)",
            params![did, now, orthogonal],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO l2_experience (
                agent_did, topic, fact_summary, created_at, embedding, last_accessed,
                confidence, epistemic_kind, valid_until, weakened
             ) VALUES (?1, 'notes', 'expired secret', ?2, ?3, ?2, 0.6, 'fact', '2000-01-01T00:00:00Z', 0)",
            params![did, now, aligned],
        )
        .unwrap();

        let hits = semantic_search(
            &conn,
            did,
            &[1.0, 0.0, 0.0],
            None,
            Some("seattle residence"),
        );
        let texts: Vec<&str> = hits.iter().map(|row| row.fact_summary.as_str()).collect();
        assert!(
            texts.iter().any(|text| text.contains("seattle")),
            "lexical channel should surface the orthogonal fact: {texts:?}"
        );
        assert!(
            texts.iter().all(|text| !text.contains("expired")),
            "past valid_until must stay out of recall: {texts:?}"
        );
    }
}
