use super::*;

/// Get the current database file size via SQLite's page_count * page_size.
/// Returns 0 for in-memory databases.
pub(super) fn get_db_page_size(conn: &Connection) -> u64 {
    let result: Result<(i64, i64), _> = conn.query_row(
        "SELECT page_count, page_size FROM pragma_page_count(), pragma_page_size()",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );
    match result {
        Ok((page_count, page_size)) => (page_count as u64) * (page_size as u64),
        Err(_) => 0,
    }
}

/// Get total database size including WAL and SHM files.
/// This gives a more accurate picture of actual disk usage than page_count alone,
/// since WAL can grow significantly between checkpoints.
pub(super) fn get_total_db_size(conn: &Connection, db_path: &str) -> u64 {
    let page_size = get_db_page_size(conn);
    let wal_size = std::fs::metadata(format!("{}-wal", db_path))
        .map(|m| m.len())
        .unwrap_or(0);
    let shm_size = std::fs::metadata(format!("{}-shm", db_path))
        .map(|m| m.len())
        .unwrap_or(0);
    page_size + wal_size + shm_size
}

/// Check if the database has exceeded its size quota.
/// Returns `true` if over quota (writes should be blocked).
/// Also logs a warning at 90% capacity (once).
pub(super) fn is_db_over_quota(
    conn: &Connection,
    db_path: &str,
    max_db_size: u64,
    size_warning_issued: &mut bool,
) -> bool {
    let db_size = get_total_db_size(conn, db_path);
    if db_size >= max_db_size {
        if !*size_warning_issued {
            tracing::warn!(
                db_size_gb = db_size as f64 / (1024.0 * 1024.0 * 1024.0),
                max_size_gb = max_db_size as f64 / (1024.0 * 1024.0 * 1024.0),
                "Database size has reached the configured limit. Writes are now blocked. \
                 Increase MEMORY_MAX_SIZE_GB or clean up old data."
            );
            *size_warning_issued = true;
        }
        return true;
    }
    // Warn at 90% capacity
    if db_size >= max_db_size * 9 / 10 && !*size_warning_issued {
        tracing::warn!(
            capacity_pct = (db_size as f64 / max_db_size as f64) * 100.0,
            db_size_gb = db_size as f64 / (1024.0 * 1024.0 * 1024.0),
            max_size_gb = max_db_size as f64 / (1024.0 * 1024.0 * 1024.0),
            "Database is at high capacity. Consider increasing MEMORY_MAX_SIZE_GB \
             or cleaning up old data."
        );
        *size_warning_issued = true;
    }
    false
}

/// Flush all L1 caches to the l1_snapshot table (REPLACE semantics).
pub(super) fn flush_l1_to_db(
    conn: &Connection,
    l1_caches: &std::collections::HashMap<String, VecDeque<Message>>,
) -> Result<usize> {
    let mut count = 0;
    let now = Utc::now().to_rfc3339();
    for (did, cache) in l1_caches {
        if cache.is_empty() {
            continue;
        }
        let messages: Vec<&Message> = cache.iter().collect();
        let json = serde_json::to_string(&messages)
            .with_context(|| format!("serializing L1 cache for {did}"))?;
        conn.execute(
            "INSERT OR REPLACE INTO l1_snapshot (agent_did, messages_json, updated_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![did, json, now],
        )
        .with_context(|| format!("flushing L1 snapshot for {did}"))?;
        count += 1;
    }
    Ok(count)
}

/// Restore L1 caches from the l1_snapshot table, then clear the snapshot rows.
pub(super) fn restore_l1_from_db(
    conn: &Connection,
    l1_caches: &mut std::collections::HashMap<String, VecDeque<Message>>,
    l1_max_size: usize,
) -> Result<usize> {
    let mut stmt = conn
        .prepare("SELECT agent_did, messages_json FROM l1_snapshot")
        .context("preparing l1_snapshot SELECT")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .context("querying l1_snapshot")?
        .filter_map(row_ok)
        .collect();

    let mut count = 0;
    for (did, json) in &rows {
        let messages: Vec<Message> = match serde_json::from_str(json) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(did = %did, error = %e, "Skipping corrupted L1 snapshot");
                continue;
            }
        };
        let cache = l1_caches.entry(did.clone()).or_default();
        for msg in messages {
            // Drop startup-status notices persisted by older builds (see the
            // l3_archive purge in memory_migrations): not conversation memory.
            if msg.role == "assistant" && msg.content.contains("系统已重启 / System restarted")
            {
                continue;
            }
            cache.push_back(msg);
            // Respect l1_max_size during restore
            while cache.len() > l1_max_size {
                cache.pop_front();
            }
        }
        count += 1;
    }

    // Clear snapshot rows after successful restore
    if count > 0
        && let Err(e) = conn.execute("DELETE FROM l1_snapshot", [])
    {
        tracing::warn!(error = %e, "Failed to clear L1 snapshot after restore");
    }

    Ok(count)
}

/// Parse a knowledge_entries row into a KnowledgeEntry struct.
pub(super) fn parse_kb_row(row: &rusqlite::Row) -> rusqlite::Result<KnowledgeEntry> {
    let tags_json: String = row.get(7)?;
    let related_json: String = row.get(8)?;
    Ok(KnowledgeEntry {
        id: row.get(0)?,
        chat_id: row.get(1)?,
        category: row.get(2)?,
        priority: row.get(3)?,
        status: row.get(4)?,
        title: row.get(5)?,
        body: row.get(6)?,
        tags: parse_json_column(&tags_json, "knowledge_tags"),
        related_ids: parse_json_column(&related_json, "knowledge_related_ids"),
        source_msg_id: row.get(9)?,
        source_summary: row.get(10)?,
        due_date: row.get(11)?,
        waiting_on: row.get(12)?,
        superseded_by: row.get(13)?,
        resolved_at: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
    })
}

/// Archive a message to L3 (full-text archive) and sync to FTS5 index.
pub(super) fn archive_to_l3(conn: &Connection, did: &str, msg: &Message) -> Result<()> {
    let content = msg.content.trim();
    // Skip empty/one-character messages. Measured in characters, not bytes:
    // "好"/"不要" (3 bytes per CJK char) used to be dropped, leaving holes in
    // the archive exactly where the owner approved or declined something.
    if content.chars().count() < 2 {
        return Ok(());
    }
    // Use the message's own timestamp for `created_at` so L1 (message.timestamp) and
    // L3 (entry.created_at) produce identical vnext_chat_history_turn_key dedup values.
    // For new messages, msg.timestamp ≈ Utc::now() (within seconds); using the message's
    // own timestamp makes dedup deterministic and avoids 300-second-bucket boundary flakiness.
    let archived_at = chrono::DateTime::<Utc>::from_timestamp(msg.timestamp, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    conn.execute(
        "INSERT INTO l3_archive (agent_did, role, content, created_at, anchor_kind, anchor_ref, latency_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            did,
            msg.role,
            content,
            archived_at,
            msg.anchor_kind.as_deref(),
            msg.anchor_ref.as_deref(),
            msg.latency_ms.map(|ms| ms as i64)
        ],
    )?;
    let rowid = conn.last_insert_rowid();
    // Sync to FTS5 index
    conn.execute(
        "INSERT INTO l3_fts (rowid, content) VALUES (?1, ?2)",
        rusqlite::params![rowid, content],
    )?;
    Ok(())
}

/// Full-text search over L3 archive using FTS5.
/// Returns up to 10 entries ranked by relevance, filtered by DID.
pub(super) fn fulltext_search_l3(conn: &Connection, did: &str, query: &str) -> Vec<L3Entry> {
    // FTS5 MATCH requires non-empty query
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    // Sanitize query: strip double quotes and wrap in FTS5 phrase quotes
    // to prevent FTS5 query injection (e.g. NEAR, OR, NOT operators)
    let safe_query = query.replace('"', "");
    let safe_query = format!("\"{}\"", safe_query);

    let mut stmt = match conn.prepare(
        "SELECT a.id, a.agent_did, a.role, a.content, a.created_at, a.anchor_kind, a.anchor_ref, a.latency_ms
         FROM l3_archive a
         JOIN l3_fts f ON a.id = f.rowid
         WHERE l3_fts MATCH ?1 AND a.agent_did = ?2
         ORDER BY f.rank
         LIMIT 10",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(error = %e, "L3 fulltext search prepare failed");
            return Vec::new();
        }
    };

    let results: Vec<L3Entry> = match stmt.query_map(rusqlite::params![safe_query, did], |row| {
        Ok(L3Entry {
            id: row.get(0)?,
            agent_did: row.get(1)?,
            role: row.get(2)?,
            content: row.get(3)?,
            created_at: row.get(4)?,
            anchor_kind: row.get(5)?,
            anchor_ref: row.get(6)?,
            latency_ms: row
                .get::<_, Option<i64>>(7)?
                .and_then(|value| u64::try_from(value).ok()),
        })
    }) {
        Ok(rows) => rows.filter_map(row_ok).collect(),
        Err(e) => {
            tracing::debug!(error = %e, "L3 fulltext search query failed");
            Vec::new()
        }
    };
    results
}

fn l3_entry_from_row(row: &rusqlite::Row) -> rusqlite::Result<L3Entry> {
    Ok(L3Entry {
        id: row.get(0)?,
        agent_did: row.get(1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        created_at: row.get(4)?,
        anchor_kind: row.get(5)?,
        anchor_ref: row.get(6)?,
        latency_ms: row
            .get::<_, Option<i64>>(7)?
            .and_then(|value| u64::try_from(value).ok()),
    })
}

const L3_ENTRY_COLUMNS: &str =
    "id, agent_did, role, content, created_at, anchor_kind, anchor_ref, latency_ms";

/// `origin_turn_id` stored on the task or reminder creation receipt.
pub(super) fn task_origin_turn_id(conn: &Connection, task_id: i64) -> Option<String> {
    let store = crate::action_ledger::ActionLedgerStore::new(conn);
    for action_id in [format!("reminder:{task_id}"), format!("task:{task_id}")] {
        let Ok(Some(entry)) = store.get_by_action_id(&action_id) else {
            continue;
        };
        let Some(meta) = entry.metadata_json.as_deref() else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(meta) else {
            continue;
        };
        let Some(turn) = json
            .get("origin_turn_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|turn| !turn.is_empty())
        else {
            continue;
        };
        return Some(turn.to_string());
    }
    None
}

/// Rows for the turn that created `task_id`, or the archive window around
/// `created_at_unix` when that receipt has no turn id.
pub(super) fn trigger_archive_for(
    conn: &Connection,
    did: &str,
    created_at_unix: i64,
    task_id: Option<i64>,
) -> crate::memory::TriggerArchive {
    if let Some(task_id) = task_id
        && let Some(turn_id) = task_origin_turn_id(conn, task_id)
    {
        let entries = list_l3_by_webapp_turn(conn, did, &turn_id);
        if !entries.is_empty() {
            return crate::memory::TriggerArchive {
                entries,
                exact_turn: true,
            };
        }
    }
    let start = chrono::DateTime::<chrono::Utc>::from_timestamp(
        created_at_unix.saturating_sub(TRIGGER_ARCHIVE_WINDOW_SECS),
        0,
    )
    .map(|stamp| stamp.to_rfc3339());
    let end = chrono::DateTime::<chrono::Utc>::from_timestamp(
        created_at_unix.saturating_add(TRIGGER_ARCHIVE_WINDOW_SECS),
        0,
    )
    .map(|stamp| stamp.to_rfc3339());
    let (Some(start), Some(end)) = (start, end) else {
        return crate::memory::TriggerArchive::default();
    };
    crate::memory::TriggerArchive {
        entries: list_l3_between(conn, did, &start, &end),
        exact_turn: false,
    }
}

const TRIGGER_ARCHIVE_WINDOW_SECS: i64 = 600;

pub(super) fn list_l3_by_webapp_turn(conn: &Connection, did: &str, turn_id: &str) -> Vec<L3Entry> {
    let sql = format!(
        "SELECT {L3_ENTRY_COLUMNS}
         FROM l3_archive
         WHERE agent_did = ?1 AND anchor_kind = ?2 AND anchor_ref = ?3
         ORDER BY id ASC
         LIMIT 12"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(stmt) => stmt,
        Err(error) => {
            tracing::debug!(%error, "L3 turn list prepare failed");
            return Vec::new();
        }
    };
    match stmt.query_map(
        rusqlite::params![
            did,
            crate::progress_router::WEBAPP_TURN_ANCHOR_KIND,
            turn_id
        ],
        l3_entry_from_row,
    ) {
        Ok(rows) => rows.filter_map(row_ok).collect(),
        Err(error) => {
            tracing::debug!(%error, "L3 turn list query failed");
            Vec::new()
        }
    }
}

pub(super) fn list_l3_between(
    conn: &Connection,
    did: &str,
    start_rfc3339: &str,
    end_rfc3339: &str,
) -> Vec<L3Entry> {
    let sql = format!(
        "SELECT {L3_ENTRY_COLUMNS}
         FROM l3_archive
         WHERE agent_did = ?1 AND created_at >= ?2 AND created_at <= ?3
         ORDER BY id ASC
         LIMIT 80"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(stmt) => stmt,
        Err(error) => {
            tracing::debug!(%error, "L3 window list prepare failed");
            return Vec::new();
        }
    };
    match stmt.query_map(
        rusqlite::params![did, start_rfc3339, end_rfc3339],
        l3_entry_from_row,
    ) {
        Ok(rows) => rows.filter_map(row_ok).collect(),
        Err(error) => {
            tracing::debug!(%error, "L3 window list query failed");
            Vec::new()
        }
    }
}

/// Recent L3 entries in chronological order, filtered by DID.
pub(super) fn list_recent_l3(conn: &Connection, did: &str, limit: usize) -> Vec<L3Entry> {
    let limit = limit.clamp(1, 500) as i64;
    let mut stmt = match conn.prepare(
        "SELECT id, agent_did, role, content, created_at, anchor_kind, anchor_ref, latency_ms
         FROM (
             SELECT id, agent_did, role, content, created_at, anchor_kind, anchor_ref, latency_ms
             FROM l3_archive
             WHERE agent_did = ?1
             ORDER BY id DESC
             LIMIT ?2
         )
         ORDER BY id ASC",
    ) {
        Ok(stmt) => stmt,
        Err(e) => {
            tracing::debug!(error = %e, "L3 recent list prepare failed");
            return Vec::new();
        }
    };

    match stmt.query_map(rusqlite::params![did, limit], |row| {
        Ok(L3Entry {
            id: row.get(0)?,
            agent_did: row.get(1)?,
            role: row.get(2)?,
            content: row.get(3)?,
            created_at: row.get(4)?,
            anchor_kind: row.get(5)?,
            anchor_ref: row.get(6)?,
            latency_ms: row
                .get::<_, Option<i64>>(7)?
                .and_then(|value| u64::try_from(value).ok()),
        })
    }) {
        Ok(rows) => rows.filter_map(row_ok).collect(),
        Err(e) => {
            tracing::debug!(error = %e, "L3 recent list query failed");
            Vec::new()
        }
    }
}

/// Lexical channel over recent active L2 rows. Hits already in the semantic
/// pool are left to cosine; new hits enter with cosine 0 so fusion, not a
/// second ranker, decides their place before MMR.
fn merge_lexical_channel(
    conn: &Connection,
    did: &str,
    query: &str,
    candidates: &mut Vec<(Experience, f32)>,
) {
    const SCAN_LIMIT: i64 = 200;
    const KEEP: usize = 15;
    const MIN_LEXICAL: f64 = 0.2;

    let mut stmt = match conn.prepare(
        "SELECT id, agent_did, topic, fact_summary, created_at, confidence
         FROM l2_experience
         WHERE agent_did = ?1 AND topic != 'l1_overflow'
           AND (expires_at IS NULL OR expires_at > datetime('now'))
           AND (valid_until IS NULL OR valid_until > strftime('%Y-%m-%dT%H:%M:%SZ','now'))
         ORDER BY id DESC
         LIMIT ?2",
    ) {
        Ok(stmt) => stmt,
        Err(e) => {
            tracing::debug!(error = %e, "lexical channel prepare failed");
            return;
        }
    };
    let rows = match stmt.query_map(rusqlite::params![did, SCAN_LIMIT], |row| {
        Ok(Experience {
            id: row.get(0)?,
            agent_did: row.get(1)?,
            topic: row.get(2)?,
            fact_summary: row.get(3)?,
            created_at: row.get(4)?,
            confidence: row.get::<_, Option<f64>>(5)?.unwrap_or(0.6),
            is_pinned: false,
        })
    }) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::debug!(error = %e, "lexical channel query failed");
            return;
        }
    };

    let seen: std::collections::HashSet<i64> = candidates.iter().map(|(exp, _)| exp.id).collect();
    let mut additions: Vec<(Experience, f64)> = rows
        .filter_map(row_ok)
        .filter(|exp| !seen.contains(&exp.id))
        .filter_map(|exp| {
            let lexical = super::partial_ref::bm25_lite(query, &exp.fact_summary);
            (lexical >= MIN_LEXICAL).then_some((exp, lexical))
        })
        .collect();
    additions.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.id.cmp(&b.0.id))
    });
    additions.truncate(KEEP);
    for (exp, _) in additions {
        candidates.push((exp, 0.0));
    }
}

fn load_weakened_ids(conn: &Connection, fact_ids: &[i64]) -> std::collections::HashSet<i64> {
    let mut weakened = std::collections::HashSet::new();
    if fact_ids.is_empty() {
        return weakened;
    }
    let placeholders: Vec<String> = (0..fact_ids.len()).map(|i| format!("?{}", i + 1)).collect();
    let sql = format!(
        "SELECT id FROM l2_experience WHERE COALESCE(weakened, 0) = 1 AND id IN ({})",
        placeholders.join(",")
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(fact_ids.len());
    for id in fact_ids {
        params.push(Box::new(*id));
    }
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = match conn.prepare(&sql) {
        Ok(stmt) => stmt,
        Err(e) => {
            tracing::debug!(error = %e, "weakened-id lookup prepare failed");
            return weakened;
        }
    };
    let rows = match stmt.query_map(param_refs.as_slice(), |row| row.get::<_, i64>(0)) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::debug!(error = %e, "weakened-id lookup failed");
            return weakened;
        }
    };
    for id in rows.flatten() {
        weakened.insert(id);
    }
    weakened
}

/// Semantic search pipeline (4 stages):
/// 1. Candidate retrieval — HNSW (fast) or SQL fallback (O(N)), plus an optional lexical channel
/// 2. 6-dimensional recall scoring — zero LLM calls — then pre-MMR hybrid fusion when a query string is present
/// 3. MMR diversity reranking (λ = 0.7). Not reciprocal rank fusion.
/// 4. Fire-and-forget recall tracking
pub(super) fn semantic_search(
    conn: &Connection,
    did: &str,
    query_vec: &[f32],
    hnsw: Option<&HnswIndex>,
    query_text: Option<&str>,
) -> Vec<Experience> {
    // ── Stage 1: candidate retrieval (SEMANTIC_CANDIDATE_K results) ──
    let mut candidates = semantic_search_candidates(conn, did, query_vec, hnsw);
    let lexical_query = query_text.map(str::trim).filter(|q| !q.is_empty());
    if let Some(query) = lexical_query {
        merge_lexical_channel(conn, did, query, &mut candidates);
    }
    if candidates.is_empty() {
        return Vec::new();
    }

    // ── Stage 2: 6-dim scoring, then optional pre-MMR hybrid fusion ──
    let fact_ids: Vec<i64> = candidates.iter().map(|(e, _)| e.id).collect();
    let stats_map = batch_get_recall_stats(conn, &fact_ids);
    let weakened = load_weakened_ids(conn, &fact_ids);
    let now = Utc::now();

    let mut scored: Vec<(Experience, f64)> = candidates
        .into_iter()
        .map(|(exp, cosine)| {
            let recall_score = stats_map
                .get(&exp.id)
                .map(|s| compute_recall_score(s, &DEFAULT_RECALL_WEIGHTS))
                .unwrap_or(0.0);
            let age = days_since_timestamp(&exp.created_at);
            let base = composite_search_score(cosine as f64, recall_score, age);
            // Apply per-topic multiplier so high-value learned:context/goal/preference
            // facts rank above lower-value prefixes at similar cosine.
            let mut composite = base * topic_weight(&exp.topic);
            if weakened.contains(&exp.id) {
                composite *= 0.5;
            }
            if let Some(query) = lexical_query {
                let lexical = super::partial_ref::bm25_lite(query, &exp.fact_summary);
                let temporal = super::partial_ref::temporal_freshness(&exp.created_at, now);
                let graph = super::partial_ref::topic_graph_signal(query, &exp.topic);
                composite =
                    super::partial_ref::fuse_hybrid_relevance(composite, lexical, temporal, graph);
            }
            (exp, composite)
        })
        .collect();

    // Query text makes the fused score the relevance MMR sees. Pools smaller
    // than SEMANTIC_TOP_K skip MMR reordering, so sort here or the lexical
    // channel would not change what the caller reads first.
    if lexical_query.is_some() {
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.id.cmp(&b.0.id))
        });
    }

    // ── Stage 3: MMR diversity reranking ──
    // MMR produces a diversity-optimal ordering — do NOT re-sort by score
    // after it runs. The sort above only feeds MMR its relevance input.
    let final_results = mmr_rerank(scored, SEMANTIC_TOP_K, MMR_LAMBDA);

    let experiences: Vec<Experience> = final_results.iter().map(|(e, _)| e.clone()).collect();

    // Touch last_accessed
    let result_ids: Vec<i64> = experiences.iter().map(|e| e.id).collect();
    touch_l2_last_accessed(conn, &result_ids);

    // ── Stage 4: fire-and-forget recall recording ──
    let query_hash = fnv1a_hash(&EmbeddingClient::to_blob(query_vec));
    let events: Vec<RecallEvent> = experiences
        .iter()
        .zip(final_results.iter())
        .map(|(exp, (_, score))| RecallEvent {
            fact_id: exp.id,
            composite_score: *score as f32,
            query_hash,
        })
        .collect();
    record_recall_events(conn, &events);

    experiences
}

/// Stage 1: retrieve SEMANTIC_CANDIDATE_K candidates with raw cosine scores.
pub(super) fn semantic_search_candidates(
    conn: &Connection,
    did: &str,
    query_vec: &[f32],
    hnsw: Option<&HnswIndex>,
) -> Vec<(Experience, f32)> {
    // Try HNSW path first
    if let Some(index) = hnsw
        && index.is_ready()
        && !index.is_empty()
    {
        let candidates = index.search(query_vec, SEMANTIC_CANDIDATE_K * 3, SIMILARITY_THRESHOLD);
        if !candidates.is_empty() {
            return hnsw_fetch_candidates(conn, did, &candidates);
        }
    }

    // Fallback: SQL full-table scan
    sql_fetch_candidates(conn, did, query_vec)
}

/// HNSW path: fetch Experience records by IDs, return with raw cosine scores.
pub(super) fn hnsw_fetch_candidates(
    conn: &Connection,
    did: &str,
    candidates: &[crate::hnsw_index::AnnResult],
) -> Vec<(Experience, f32)> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let placeholders: Vec<String> = (0..candidates.len())
        .map(|i| format!("?{}", i + 2))
        .collect();
    let sql = format!(
        "SELECT id, agent_did, topic, fact_summary, created_at, confidence \
         FROM l2_experience \
         WHERE agent_did = ?1 AND topic != 'l1_overflow' \
         AND (expires_at IS NULL OR expires_at > datetime('now')) \
         AND (valid_until IS NULL OR valid_until > strftime('%Y-%m-%dT%H:%M:%SZ','now')) \
         AND id IN ({})",
        placeholders.join(","),
    );

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(1 + candidates.len());
    params.push(Box::new(did.to_string()));
    for c in candidates {
        params.push(Box::new(c.l2_id));
    }
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(error = %e, "hnsw_fetch prepare failed");
            return Vec::new();
        }
    };

    let rows = match stmt.query_map(param_refs.as_slice(), |row| {
        Ok(Experience {
            id: row.get(0)?,
            agent_did: row.get(1)?,
            topic: row.get(2)?,
            fact_summary: row.get(3)?,
            created_at: row.get(4)?,
            confidence: row.get::<_, Option<f64>>(5)?.unwrap_or(0.6),
            is_pinned: false,
        })
    }) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "hnsw_fetch query failed");
            return Vec::new();
        }
    };

    // Map l2_id → cosine similarity from HNSW results
    let sim_map: std::collections::HashMap<i64, f32> =
        candidates.iter().map(|c| (c.l2_id, c.similarity)).collect();

    let mut results: Vec<(Experience, f32)> = rows
        .filter_map(row_ok)
        .filter_map(|exp| {
            let sim = *sim_map.get(&exp.id)?;
            Some((exp, sim))
        })
        .collect();

    // Sort by raw cosine descending, truncate to candidate pool size
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(SEMANTIC_CANDIDATE_K);
    results
}

/// SQL fallback: full-table scan with vec_cosine_sim, returns cosine scores.
pub(super) fn sql_fetch_candidates(
    conn: &Connection,
    did: &str,
    query_vec: &[f32],
) -> Vec<(Experience, f32)> {
    let query_blob = EmbeddingClient::to_blob(query_vec);

    let mut stmt = match conn.prepare(
        "WITH scored AS (
            SELECT id, agent_did, topic, fact_summary, created_at, confidence,
                   vec_cosine_sim(embedding, ?2) AS cosine_score
            FROM l2_experience
            WHERE agent_did = ?1 AND embedding IS NOT NULL
              AND topic != 'l1_overflow'
              AND (expires_at IS NULL OR expires_at > datetime('now'))
              AND (valid_until IS NULL OR valid_until > strftime('%Y-%m-%dT%H:%M:%SZ','now'))
        )
        SELECT id, agent_did, topic, fact_summary, created_at, cosine_score, confidence
        FROM scored
        WHERE cosine_score >= ?3
        ORDER BY cosine_score DESC
        LIMIT ?4",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(error = %e, "sql_fetch_candidates prepare failed");
            return Vec::new();
        }
    };

    let rows = match stmt.query_map(
        rusqlite::params![
            did,
            query_blob,
            SIMILARITY_THRESHOLD as f64,
            SEMANTIC_CANDIDATE_K as i64
        ],
        |row| {
            // NOTE: cosine_score stays at index 5 (existing row.get(5) preserved).
            // confidence is appended at index 6 so no existing index shifts.
            Ok((
                Experience {
                    id: row.get(0)?,
                    agent_did: row.get(1)?,
                    topic: row.get(2)?,
                    fact_summary: row.get(3)?,
                    created_at: row.get(4)?,
                    confidence: row.get::<_, Option<f64>>(6)?.unwrap_or(0.6),
                    is_pinned: false,
                },
                row.get::<_, f64>(5)? as f32,
            ))
        },
    ) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "sql_fetch_candidates query failed");
            return Vec::new();
        }
    };

    rows.filter_map(row_ok).collect()
}

/// Update last_accessed timestamp for a set of L2 fact IDs.
pub(super) fn touch_l2_last_accessed(conn: &Connection, ids: &[i64]) {
    if ids.is_empty() {
        return;
    }
    let now_str = Utc::now().to_rfc3339();
    let placeholders: Vec<String> = (0..ids.len()).map(|i| format!("?{}", i + 2)).collect();
    let sql = format!(
        "UPDATE l2_experience SET last_accessed = ?1 WHERE id IN ({})",
        placeholders.join(",")
    );
    if let Ok(mut stmt) = conn.prepare(&sql) {
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now_str)];
        for id in ids {
            params.push(Box::new(*id));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        if let Err(e) = stmt.execute(param_refs.as_slice()) {
            tracing::debug!(error = %e, "touch_l2_last_accessed failed");
        }
    }
}

// ── Self-learning: recall tracking + 6-dim scoring + MMR ───────────────

/// FNV-1a hash over arbitrary bytes (zero-dependency).
pub(super) fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Record recall events (fire-and-forget, errors logged at debug level).
pub(super) fn record_recall_events(conn: &Connection, events: &[RecallEvent]) {
    if events.is_empty() {
        return;
    }
    let now = Utc::now();
    let today = now.format("%Y-%m-%d").to_string();
    let now_rfc = now.to_rfc3339();

    for event in events {
        let hash_hex = format!("{:016x}", event.query_hash);

        if let Err(e) = conn.execute(
            "INSERT INTO l2_recall_stats (fact_id, recall_count, total_score, max_score,
                unique_queries, query_hashes, recall_days, first_recalled_at, last_recalled_at)
             VALUES (?1, 1, ?2, ?2, 1, json_array(?3), json_array(?4), ?5, ?5)
             ON CONFLICT(fact_id) DO UPDATE SET
                recall_count = recall_count + 1,
                total_score = total_score + ?2,
                max_score = MAX(max_score, ?2),
                last_recalled_at = ?5",
            rusqlite::params![
                event.fact_id,
                event.composite_score as f64,
                hash_hex,
                today,
                now_rfc
            ],
        ) {
            tracing::debug!(error = %e, fact_id = event.fact_id, "recall_stats upsert failed");
        }

        update_recall_hashes(conn, event.fact_id, &hash_hex);
        update_recall_days(conn, event.fact_id, &today);
    }
}

/// Append a query hash to the JSON array (deduplicated, max 32 entries).
pub(super) fn update_recall_hashes(conn: &Connection, fact_id: i64, hash_hex: &str) {
    let current: String = conn
        .query_row(
            "SELECT query_hashes FROM l2_recall_stats WHERE fact_id = ?1",
            [fact_id],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "[]".to_string());

    let mut hashes: Vec<String> = serde_json::from_str(&current).unwrap_or_default();
    if hashes.contains(&hash_hex.to_string()) {
        return; // already tracked
    }
    hashes.push(hash_hex.to_string());
    if hashes.len() > 32 {
        hashes.drain(..hashes.len() - 32);
    }
    let new_json = serde_json::to_string(&hashes).unwrap_or_else(|_| "[]".to_string());
    let unique_count = hashes.len() as i32;
    if let Err(e) = conn.execute(
        "UPDATE l2_recall_stats SET query_hashes = ?1, unique_queries = ?2 WHERE fact_id = ?3",
        rusqlite::params![new_json, unique_count, fact_id],
    ) {
        tracing::debug!(error = %e, fact_id, "update_recall_hashes failed");
    }
}

/// Append today's date to the recall_days JSON array (deduplicated, max 16 entries).
pub(super) fn update_recall_days(conn: &Connection, fact_id: i64, today: &str) {
    let current: String = conn
        .query_row(
            "SELECT recall_days FROM l2_recall_stats WHERE fact_id = ?1",
            [fact_id],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "[]".to_string());

    let mut days: Vec<String> = serde_json::from_str(&current).unwrap_or_default();
    if days.contains(&today.to_string()) {
        return; // already recorded today
    }
    days.push(today.to_string());
    if days.len() > 16 {
        days.drain(..days.len() - 16);
    }
    let new_json = serde_json::to_string(&days).unwrap_or_else(|_| "[]".to_string());
    if let Err(e) = conn.execute(
        "UPDATE l2_recall_stats SET recall_days = ?1 WHERE fact_id = ?2",
        rusqlite::params![new_json, fact_id],
    ) {
        tracing::debug!(error = %e, fact_id, "update_recall_days failed");
    }
}

/// Batch-fetch recall stats for a set of fact IDs (single SQL query).
pub(super) fn batch_get_recall_stats(
    conn: &Connection,
    fact_ids: &[i64],
) -> std::collections::HashMap<i64, RecallStats> {
    let mut map = std::collections::HashMap::new();
    if fact_ids.is_empty() {
        return map;
    }
    let placeholders: Vec<String> = (0..fact_ids.len()).map(|i| format!("?{}", i + 1)).collect();
    let sql = format!(
        "SELECT fact_id, recall_count, total_score, max_score, unique_queries,
                query_hashes, recall_days, first_recalled_at, last_recalled_at
         FROM l2_recall_stats WHERE fact_id IN ({})",
        placeholders.join(",")
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(fact_ids.len());
    for id in fact_ids {
        params.push(Box::new(*id));
    }
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(error = %e, "batch_get_recall_stats prepare failed");
            return map;
        }
    };
    let rows = match stmt.query_map(param_refs.as_slice(), |row| {
        let hashes_json: String = row.get(5)?;
        let days_json: String = row.get(6)?;
        Ok(RecallStats {
            fact_id: row.get(0)?,
            recall_count: row.get(1)?,
            total_score: row.get(2)?,
            max_score: row.get(3)?,
            unique_queries: row.get(4)?,
            query_hashes: serde_json::from_str(&hashes_json).unwrap_or_default(),
            recall_days: serde_json::from_str(&days_json).unwrap_or_default(),
            first_recalled_at: row.get(7)?,
            last_recalled_at: row.get(8)?,
        })
    }) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "batch_get_recall_stats query failed");
            return map;
        }
    };
    for row in rows.flatten() {
        map.insert(row.fact_id, row);
    }
    map
}

/// Days elapsed since an RFC-3339 timestamp.
pub(super) fn days_since_timestamp(ts: &str) -> f64 {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|dt| {
            let elapsed = Utc::now().signed_duration_since(dt);
            (elapsed.num_seconds() as f64 / 86400.0).max(0.0)
        })
        .unwrap_or(0.0)
}

/// Compute the span in days between the earliest and latest dates in a list.
pub(super) fn compute_day_span(days: &[String]) -> f64 {
    if days.len() < 2 {
        return 0.0;
    }
    let dates: Vec<&str> = {
        let mut sorted: Vec<&str> = days.iter().map(|s| s.as_str()).collect();
        sorted.sort();
        sorted
    };
    let parse = |s: &str| -> Option<chrono::NaiveDate> { s.parse().ok() };
    match (parse(dates.first().unwrap()), parse(dates.last().unwrap())) {
        (Some(first), Some(last)) => (last - first).num_days() as f64,
        _ => 0.0,
    }
}

/// 6-dimensional weighted recall score (all components clamped to [0, 1]).
pub fn compute_recall_score(stats: &RecallStats, weights: &RecallScoreWeights) -> f64 {
    // 1. Frequency: log(1 + count) / log(11), capped at ~10 recalls → 1.0
    let freq = (1.0 + stats.recall_count as f64).ln() / 11.0_f64.ln();

    // 2. Relevance: average composite score across all recalls
    let rel = if stats.recall_count > 0 {
        stats.total_score / stats.recall_count as f64
    } else {
        0.0
    };

    // 3. Diversity: unique_queries / 5, at 5+ unique queries → 1.0
    let div = stats.unique_queries as f64 / 5.0;

    // 4. Recency: exponential decay, half-life = 14 days
    let rec = stats
        .last_recalled_at
        .as_ref()
        .map(|ts| {
            let days = days_since_timestamp(ts);
            let lambda = 2.0_f64.ln() / 14.0;
            (-lambda * days).exp()
        })
        .unwrap_or(0.0);

    // 5. Consolidation: spaced recall across multiple days
    let consol = {
        let n_days = stats.recall_days.len();
        if n_days == 0 {
            0.0
        } else if n_days == 1 {
            0.2
        } else {
            let spacing = ((n_days as f64).ln() / 5.0_f64.ln()).min(1.0);
            let span = (compute_day_span(&stats.recall_days) / 7.0).min(1.0);
            0.55 * spacing + 0.45 * span
        }
    };

    // 6. Conceptual: proxy via query diversity breadth
    let concept = (stats.unique_queries as f64 / 8.0).min(1.0);

    let score = weights.frequency * freq.clamp(0.0, 1.0)
        + weights.relevance * rel.clamp(0.0, 1.0)
        + weights.diversity * div.clamp(0.0, 1.0)
        + weights.recency * rec.clamp(0.0, 1.0)
        + weights.consolidation * consol.clamp(0.0, 1.0)
        + weights.conceptual * concept.clamp(0.0, 1.0);
    score.clamp(0.0, 1.0)
}

/// Composite search score: combines cosine relevance with recall-based memory value.
pub(super) fn composite_search_score(cosine: f64, recall_score: f64, fact_age_days: f64) -> f64 {
    const COLD_START_DAYS: f64 = 7.0;
    const COLD_START_FLOOR: f64 = 0.5;

    let effective_recall = if fact_age_days < COLD_START_DAYS {
        let t = fact_age_days / COLD_START_DAYS;
        COLD_START_FLOOR * (1.0 - t) + recall_score * t
    } else {
        recall_score
    };

    // 0.6 base: high-cosine new facts aren't buried by low-recall old facts
    // 0.4 weight: recall_score influences up to ±40% of final ranking
    cosine * (0.6 + 0.4 * effective_recall)
}

/// Per-topic multiplier applied to the composite score.
/// Boosts high-value learned facts (context, goals, preferences) and lightly
/// demotes lower-value prefixes so semantic search favors actionable knowledge.
/// Per spec §3.3.
pub fn topic_weight(topic: &str) -> f64 {
    if topic.starts_with("learned:context") {
        1.2
    } else if topic.starts_with("learned:preference") || topic.starts_with("learned:goal") {
        1.1
    } else if topic.starts_with("learned:habit") || topic.starts_with("learned:workflow") {
        1.0
    } else if topic.starts_with("learned:interest") {
        0.9
    } else if topic.starts_with("learned:lesson") {
        // Lessons have a separate injection channel — demote so they don't
        // dominate semantic search results.
        0.8
    } else {
        0.9
    }
}

/// Check if a character is in the CJK Unified Ideographs range.
pub(super) fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}' // CJK Extension A
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility
        | '\u{3040}'..='\u{309F}' // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
    )
}

/// Tokenize text for MMR similarity: ASCII words + CJK bigrams/unigrams.
pub(super) fn tokenize_for_mmr(text: &str) -> std::collections::HashSet<String> {
    let lower = text.to_lowercase();
    let mut tokens = std::collections::HashSet::new();

    // ASCII words
    for word in lower.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if !word.is_empty() {
            tokens.insert(word.to_string());
        }
    }

    // CJK bigrams + unigrams
    let chars: Vec<char> = lower.chars().collect();
    let mut prev_cjk: Option<char> = None;
    for &ch in &chars {
        if is_cjk(ch) {
            tokens.insert(ch.to_string());
            if let Some(prev) = prev_cjk {
                tokens.insert(format!("{prev}{ch}"));
            }
            prev_cjk = Some(ch);
        } else {
            prev_cjk = None;
        }
    }
    tokens
}

/// Jaccard similarity between two token sets.
pub(super) fn jaccard_similarity(
    a: &std::collections::HashSet<String>,
    b: &std::collections::HashSet<String>,
) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.len() as f64 + b.len() as f64 - intersection;
    intersection / union
}

/// MMR (Maximal Marginal Relevance) reranking for result diversity.
/// λ controls relevance vs diversity trade-off (higher = more relevance).
pub(super) fn mmr_rerank(
    candidates: Vec<(Experience, f64)>,
    final_k: usize,
    lambda: f64,
) -> Vec<(Experience, f64)> {
    if candidates.len() <= 1 || candidates.len() <= final_k {
        return candidates;
    }

    // Pre-tokenize all candidates
    let token_cache: Vec<std::collections::HashSet<String>> = candidates
        .iter()
        .map(|(exp, _)| tokenize_for_mmr(&exp.fact_summary))
        .collect();

    // Normalize scores to [0, 1]
    let max_score = candidates
        .iter()
        .map(|(_, s)| *s)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_score = candidates
        .iter()
        .map(|(_, s)| *s)
        .fold(f64::INFINITY, f64::min);
    let range = max_score - min_score;

    let mut selected: Vec<usize> = Vec::with_capacity(final_k);
    let mut remaining: std::collections::HashSet<usize> = (0..candidates.len()).collect();

    while selected.len() < final_k && !remaining.is_empty() {
        let mut best_idx = 0;
        let mut best_mmr = f64::NEG_INFINITY;

        for &idx in &remaining {
            let norm_rel = if range > 0.0 {
                (candidates[idx].1 - min_score) / range
            } else {
                1.0
            };

            let max_sim = selected
                .iter()
                .map(|&sel| jaccard_similarity(&token_cache[idx], &token_cache[sel]))
                .fold(0.0_f64, f64::max);

            let mmr = lambda * norm_rel - (1.0 - lambda) * max_sim;
            if mmr > best_mmr {
                best_mmr = mmr;
                best_idx = idx;
            }
        }

        selected.push(best_idx);
        remaining.remove(&best_idx);
    }

    selected
        .into_iter()
        .map(|i| candidates[i].clone())
        .collect()
}

// ── G1: Relationship State helpers ──────────────────────────────────────

// ── Tier 0: User Profile SQL handlers ──────────────────────────────────────

/// Get all profile entries for a DID, ordered by category then key.
pub(super) fn get_user_profile(conn: &Connection, did: &str) -> Result<Vec<ProfileEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, agent_did, category, key, value, confidence, source, created_at, updated_at
         FROM user_profile WHERE agent_did = ?1
         ORDER BY category, key",
    )?;
    let entries = stmt
        .query_map([did], |row| {
            Ok(ProfileEntry {
                id: row.get(0)?,
                agent_did: row.get(1)?,
                category: row.get(2)?,
                key: row.get(3)?,
                value: row.get(4)?,
                confidence: row.get(5)?,
                source: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(entries)
}

#[cfg(test)]
thread_local! {
    /// Test-only switch. Tests that need synthetic (category, key) pairs (e.g.
    /// capacity-limit exercises that require >17 unique slots) set this to true
    /// to bypass the registry gate. NEVER used in production code paths.
    pub(crate) static BYPASS_PROFILE_REGISTRY: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

pub(super) fn profile_registry_bypass_active() -> bool {
    #[cfg(test)]
    {
        BYPASS_PROFILE_REGISTRY.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        false
    }
}

#[derive(Debug)]
struct RawHeirloomProfileEventRow {
    event_id_uuid: String,
    schema_version: i64,
    event_type: String,
    hlc_phys: i64,
    hlc_log: i64,
    observed_at_rfc3339: String,
    prev_hash: Vec<u8>,
    l0_hash: Vec<u8>,
    l1_hash: Vec<u8>,
    l2_manifest_hash: Vec<u8>,
    lineage_id: String,
    author_did: String,
    sensitivity_tier: String,
    bundle_ids_json: String,
    payload_encoding_json: String,
    payload: Vec<u8>,
    signatures_json: String,
}

pub(super) fn load_profile_slot_events_from_heirloom(
    bridge: &crate::memory_heirloom_bridge::HeirloomBridge,
    did: &str,
) -> Result<Vec<crate::heirloom::EventEnvelope>> {
    let shared = bridge.writer().shared_connection();
    let conn = shared.lock();
    let mut stmt = conn.prepare(
        "SELECT event_id_uuid, schema_version, event_type, hlc_phys, hlc_log,
                observed_at_rfc3339, prev_hash, l0_hash, l1_hash, l2_manifest_hash,
                lineage_id, author_did, sensitivity_tier, bundle_ids_json,
                payload_encoding_json, payload, signatures_json
         FROM heirloom_event_log
         WHERE event_type = 'profile:slot_upserted' AND author_did = ?1
         ORDER BY hlc_phys, hlc_log, observed_at_rfc3339, event_id_uuid",
    )?;
    let rows = stmt
        .query_map([did], |row| {
            Ok(RawHeirloomProfileEventRow {
                event_id_uuid: row.get(0)?,
                schema_version: row.get(1)?,
                event_type: row.get(2)?,
                hlc_phys: row.get(3)?,
                hlc_log: row.get(4)?,
                observed_at_rfc3339: row.get(5)?,
                prev_hash: row.get(6)?,
                l0_hash: row.get(7)?,
                l1_hash: row.get(8)?,
                l2_manifest_hash: row.get(9)?,
                lineage_id: row.get(10)?,
                author_did: row.get(11)?,
                sensitivity_tier: row.get(12)?,
                bundle_ids_json: row.get(13)?,
                payload_encoding_json: row.get(14)?,
                payload: row.get(15)?,
                signatures_json: row.get(16)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    rows.into_iter()
        .map(raw_profile_event_row_to_envelope)
        .collect()
}

fn raw_profile_event_row_to_envelope(
    row: RawHeirloomProfileEventRow,
) -> Result<crate::heirloom::EventEnvelope> {
    let payload_encoding: crate::heirloom::enums::PayloadEncoding =
        serde_json::from_str(&row.payload_encoding_json).with_context(|| {
            format!(
                "parsing payload_encoding_json for Heirloom profile event {}",
                row.event_id_uuid
            )
        })?;
    if payload_encoding != crate::heirloom::enums::PayloadEncoding::Plaintext {
        anyhow::bail!(
            "unsupported Heirloom profile payload encoding for event {}",
            row.event_id_uuid
        );
    }

    let sensitivity_tier =
        crate::heirloom::sensitivity::sensitivity_tier_from_wire(&row.sensitivity_tier)
            .with_context(|| {
                format!(
                    "parsing sensitivity_tier for Heirloom profile event {}",
                    row.event_id_uuid
                )
            })?;
    let observed_at = chrono::DateTime::parse_from_rfc3339(&row.observed_at_rfc3339)
        .with_context(|| {
            format!(
                "parsing observed_at_rfc3339 for Heirloom profile event {}",
                row.event_id_uuid
            )
        })?
        .with_timezone(&Utc);

    Ok(crate::heirloom::EventEnvelope {
        event_id: uuid::Uuid::parse_str(&row.event_id_uuid)
            .with_context(|| format!("parsing event_id_uuid {}", row.event_id_uuid))?,
        schema_version: u32::try_from(row.schema_version)
            .context("Heirloom profile schema_version out of range")?,
        event_type: crate::heirloom::enums::EventTypeTag::new(row.event_type),
        prev_hash: bytes32(row.prev_hash, "prev_hash")?,
        hlc_ts: crate::heirloom::HlcTimestamp {
            phys: u64::try_from(row.hlc_phys).context("Heirloom profile hlc_phys negative")?,
            log: u32::try_from(row.hlc_log).context("Heirloom profile hlc_log out of range")?,
        },
        observed_at,
        l0_hash: bytes32(row.l0_hash, "l0_hash")?,
        l1_hash: bytes32(row.l1_hash, "l1_hash")?,
        l2_manifest_hash: bytes32(row.l2_manifest_hash, "l2_manifest_hash")?,
        lineage_id: crate::heirloom::LineageId::new(row.lineage_id),
        author_did: crate::heirloom::types::Did::new(row.author_did),
        signatures: serde_json::from_str(&row.signatures_json)
            .context("parsing Heirloom profile signatures_json")?,
        sensitivity_tier,
        bundle_ids: serde_json::from_str(&row.bundle_ids_json)
            .context("parsing Heirloom profile bundle_ids_json")?,
        payload_encoding,
        payload: decode_plaintext_profile_payload(&row.payload)?,
    })
}

pub(super) fn decode_plaintext_profile_payload(payload: &[u8]) -> Result<Vec<u8>> {
    const PLAINTEXT_FRAME_TAG: &[u8] = b"aegis:plaintext-v1:";
    if payload.starts_with(b"{") {
        return Ok(payload.to_vec());
    }
    if !payload.starts_with(PLAINTEXT_FRAME_TAG) {
        anyhow::bail!("missing plaintext-v1 framing tag");
    }
    let rest = &payload[PLAINTEXT_FRAME_TAG.len()..];
    let body_start = rest
        .iter()
        .position(|b| *b == b'\n')
        .map(|idx| idx + 1)
        .context("missing plaintext-v1 header terminator")?;
    Ok(rest[body_start..].to_vec())
}

pub(super) fn bytes32(bytes: Vec<u8>, field: &'static str) -> Result<[u8; 32]> {
    if bytes.len() != 32 {
        anyhow::bail!("{field} expected 32 bytes, got {}", bytes.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

pub(super) fn profile_entries_from_projection(
    did: &str,
    projection: &crate::heirloom::projectors::ProfileProjection,
) -> Vec<ProfileEntry> {
    projection
        .slots
        .iter()
        .map(|slot| {
            let updated_at =
                chrono::DateTime::from_timestamp_millis(slot.last_updated_hlc_phys as i64)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_default();
            ProfileEntry {
                id: 0,
                agent_did: did.to_string(),
                category: slot.category.clone(),
                key: slot.key.clone(),
                value: slot.value.clone(),
                confidence: slot.confidence,
                source: slot.source.clone(),
                created_at: updated_at.clone(),
                updated_at,
            }
        })
        .collect()
}

/// Upsert a profile entry with capacity, source-priority, and value-length enforcement.
/// `source='manual'` entries cannot be overwritten by `source='auto'`.
/// Value is truncated to `PROFILE_VALUE_MAX_CHARS` if too long (defense-in-depth).
///
/// FU#2 Phase 3 hard gate: unknown (category, key) slots and values that fail
/// the registry validator are rejected. Legacy keys are canonicalized via
/// `profile_registry::resolve_alias` so all writers (reflection, instant_promote,
/// manual, future update_profile tool) converge on the same schema.
pub(super) fn upsert_profile_entry(conn: &Connection, entry: &ProfileEntry) -> Result<()> {
    // Truncate value if over limit (also enforced at the async API layer)
    let value = if entry.value.chars().count() > PROFILE_VALUE_MAX_CHARS {
        let boundary = entry.value.floor_char_boundary(PROFILE_VALUE_MAX_CHARS);
        &entry.value[..boundary]
    } else {
        &entry.value
    };

    // Profile Slot Registry gate: canonicalize aliases then validate.
    // Manual writes bypass the validator (trusted path — user explicitly confirmed
    // the value), but aliases are still resolved so the schema stays canonical.
    // Test-only BYPASS_PROFILE_REGISTRY short-circuits both lookups.
    let (canonical_category, canonical_key) = if profile_registry_bypass_active() {
        (entry.category.clone(), entry.key.clone())
    } else {
        match crate::profile_registry::resolve_alias(&entry.category, &entry.key) {
            Some((c, k)) => (c.to_string(), k.to_string()),
            None => {
                anyhow::bail!(
                    "upsert_profile_entry: unknown slot {}.{} — not in Profile Slot Registry",
                    entry.category,
                    entry.key
                );
            }
        }
    };
    if !profile_registry_bypass_active()
        && entry.source != "manual"
        && let Err(e) =
            crate::profile_registry::validate(&canonical_category, &canonical_key, value)
    {
        anyhow::bail!(
            "upsert_profile_entry: value rejected for {}.{}: {}",
            canonical_category,
            canonical_key,
            e
        );
    }

    // Check if existing entry has source='manual' — auto cannot overwrite manual
    let existing_source: Option<String> = match conn.query_row(
        "SELECT source FROM user_profile WHERE agent_did = ?1 AND category = ?2 AND key = ?3",
        rusqlite::params![entry.agent_did, canonical_category, canonical_key],
        |row| row.get(0),
    ) {
        Ok(s) => Some(s),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => {
            tracing::warn!(
                error = %e,
                category = %entry.category,
                key = %entry.key,
                "failed to check existing profile source"
            );
            return Err(e.into());
        }
    };

    if existing_source.as_deref() == Some("manual") && entry.source != "manual" {
        tracing::debug!(
            category = %entry.category,
            key = %entry.key,
            "Skipping auto profile update — manual entry takes priority"
        );
        return Ok(());
    }

    // Enforce capacity limit (only check on INSERT, not on UPDATE of existing key)
    if existing_source.is_none() {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM user_profile WHERE agent_did = ?1",
            [&entry.agent_did],
            |r| r.get(0),
        )?;
        if count >= MAX_PROFILE_ENTRIES as i64 {
            anyhow::bail!(
                "user_profile capacity limit ({MAX_PROFILE_ENTRIES}) reached for {}",
                entry.agent_did
            );
        }
    }

    conn.execute(
        "INSERT OR REPLACE INTO user_profile (agent_did, category, key, value, confidence, source, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
        rusqlite::params![
            entry.agent_did,
            canonical_category,
            canonical_key,
            value,
            entry.confidence,
            entry.source,
        ],
    )?;
    Ok(())
}

/// Delete a profile entry. Returns true if a row was actually deleted.
pub(super) fn delete_profile_entry(
    conn: &Connection,
    did: &str,
    category: &str,
    key: &str,
) -> Result<bool> {
    let deleted = conn.execute(
        "DELETE FROM user_profile WHERE agent_did = ?1 AND category = ?2 AND key = ?3",
        rusqlite::params![did, category, key],
    )?;
    Ok(deleted > 0)
}

/// Read the relationship state row for a user, returning a default if none exists.
pub(super) fn get_relationship_state(conn: &Connection, did: &str) -> Result<RelationshipState> {
    let mut stmt = conn.prepare(
        "SELECT user_did, current_focus, ongoing_goals, narrative,
                last_interaction_at, last_topic, interaction_count,
                streak_days, updated_at, active_hours
         FROM relationship_state WHERE user_did = ?1",
    )?;
    let result = stmt.query_row([did], |row| {
        let focus_json: String = row.get(1)?;
        let goals_json: String = row.get(2)?;
        let active_hours_json: String =
            row.get::<_, String>(9).unwrap_or_else(|_| "{}".to_string());
        Ok(RelationshipState {
            user_did: row.get(0)?,
            current_focus: serde_json::from_str(&focus_json).unwrap_or_default(),
            ongoing_goals: serde_json::from_str(&goals_json).unwrap_or_default(),
            narrative: row.get(3)?,
            last_interaction_at: row.get(4)?,
            last_topic: row.get(5)?,
            interaction_count: row.get(6)?,
            streak_days: row.get(7)?,
            updated_at: row.get(8)?,
            active_hours: serde_json::from_str(&active_hours_json).unwrap_or_default(),
        })
    });
    match result {
        Ok(state) => Ok(state),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(RelationshipState {
            user_did: did.to_string(),
            ..Default::default()
        }),
        Err(e) => Err(e.into()),
    }
}

/// Update the relationship state after each user interaction.
/// UPSERT: creates the row on first call, increments thereafter.
/// Also updates the active hours histogram (G6).
pub(super) fn update_relationship_interaction(
    conn: &Connection,
    did: &str,
    topic: &str,
    user_tz: &str,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let today = &now[..10]; // YYYY-MM-DD

    // Read existing state to compute streak
    let existing = get_relationship_state(conn, did)?;
    let new_streak = compute_streak_days(&existing, today);

    // G6: Compute current hour in user's timezone for active hours histogram
    let tz = crate::settings::parse_tz_str(user_tz);
    let local_hour = chrono::Utc::now().with_timezone(&tz).hour();

    // Read existing active_hours histogram, increment current hour
    let active_hours_json: String = conn
        .query_row(
            "SELECT active_hours FROM relationship_state WHERE user_did = ?1",
            [did],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "{}".to_string());
    let mut histogram: std::collections::HashMap<String, u64> =
        serde_json::from_str(&active_hours_json).unwrap_or_default();
    *histogram.entry(local_hour.to_string()).or_insert(0) += 1;
    let updated_hours = serde_json::to_string(&histogram).unwrap_or_else(|_| "{}".to_string());

    conn.execute(
        "INSERT INTO relationship_state
            (user_did, last_interaction_at, last_topic, interaction_count,
             streak_days, updated_at, active_hours)
         VALUES (?1, ?2, ?3, 1, ?4, ?2, ?5)
         ON CONFLICT(user_did) DO UPDATE SET
            last_interaction_at = ?2,
            last_topic = ?3,
            interaction_count = interaction_count + 1,
            streak_days = ?4,
            updated_at = ?2,
            active_hours = ?5",
        rusqlite::params![did, now, topic, new_streak, updated_hours],
    )?;
    Ok(())
}

// ── Layer C: Companion Growth DB functions ───────────────────────────────

pub(super) fn get_companion_identity(conn: &Connection) -> Result<CompanionIdentity> {
    let result = conn.query_row(
        "SELECT name, name_given_at, core_values, personality, communication_notes,
                growth_stage, created_at, updated_at
         FROM companion_identity WHERE id = 1",
        [],
        |row| {
            let personality_json: String = row.get::<_, String>(3).unwrap_or_else(|_| "{}".into());
            Ok(CompanionIdentity {
                name: row.get(0)?,
                name_given_at: row.get(1)?,
                core_values: row.get(2)?,
                personality: parse_json_column(&personality_json, "personality"),
                communication_notes: row.get(4)?,
                growth_stage: GrowthStage::from_str_lossy(
                    &row.get::<_, String>(5).unwrap_or_default(),
                ),
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        },
    );
    match result {
        Ok(id) => Ok(id),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(CompanionIdentity::default()),
        Err(e) => Err(e.into()),
    }
}

pub(super) fn set_companion_name(conn: &Connection, name: &str) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE companion_identity SET name = ?1, name_given_at = ?2, updated_at = ?2 WHERE id = 1",
        rusqlite::params![name, now],
    )?;
    Ok(())
}

pub(super) fn update_personality_weights(
    conn: &Connection,
    weights: &PersonalityWeights,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let json = serde_json::to_string(weights)?;
    conn.execute(
        "UPDATE companion_identity SET personality = ?1, updated_at = ?2 WHERE id = 1",
        rusqlite::params![json, now],
    )?;
    Ok(())
}

pub(super) fn update_communication_notes(conn: &Connection, notes: &str) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE companion_identity SET communication_notes = ?1, updated_at = ?2 WHERE id = 1",
        rusqlite::params![notes, now],
    )?;
    Ok(())
}

pub(super) fn update_growth_stage(conn: &Connection, stage: GrowthStage) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE companion_identity SET growth_stage = ?1, updated_at = ?2 WHERE id = 1",
        rusqlite::params![stage.as_str(), now],
    )?;
    Ok(())
}

pub(super) fn list_growth_dimensions(conn: &Connection) -> Result<Vec<GrowthDimension>> {
    let mut stmt = conn.prepare(
        "SELECT dimension_key, display_name, depth, velocity, evidence_count,
                last_evidence_at, description, updated_at
         FROM growth_dimensions ORDER BY dimension_key",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(GrowthDimension {
                dimension_key: row.get(0)?,
                display_name: row.get(1)?,
                depth: row.get(2)?,
                velocity: row.get(3)?,
                evidence_count: row.get(4)?,
                last_evidence_at: row.get(5)?,
                description: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?
        .filter_map(row_ok)
        .collect();
    Ok(rows)
}

/// Apply evidence to a growth dimension: update depth + velocity (EMA), log event.
/// All in one transaction. Depth is UNBOUNDED (no max clamp), only min at 0.0.
pub(super) fn apply_growth_evidence(
    conn: &Connection,
    dimension_key: &str,
    delta: f64,
    detail: &str,
    metadata: &serde_json::Value,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let metadata_str = serde_json::to_string(metadata)?;

    conn.execute_batch("BEGIN")?;
    let tx_result = (|| -> Result<()> {
        // Read current values
        let (current_depth, current_velocity): (f64, f64) = conn
            .query_row(
                "SELECT depth, velocity FROM growth_dimensions WHERE dimension_key = ?1",
                rusqlite::params![dimension_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| anyhow::anyhow!("dimension '{}' not found: {}", dimension_key, e))?;

        // Update depth (unbounded, min 0.0) and velocity (EMA alpha=0.1)
        let new_depth = (current_depth + delta).max(0.0);
        let alpha = 0.1;
        let new_velocity = current_velocity * (1.0 - alpha) + delta * alpha;

        conn.execute(
            "UPDATE growth_dimensions SET
                depth = ?1, velocity = ?2, evidence_count = evidence_count + 1,
                last_evidence_at = ?3, updated_at = ?3
             WHERE dimension_key = ?4",
            rusqlite::params![new_depth, new_velocity, now, dimension_key],
        )?;

        // Append growth_log entry
        conn.execute(
            "INSERT INTO growth_log (event_type, dimension_key, detail, delta, level_after, metadata, created_at)
             VALUES ('dimension_shift', ?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![dimension_key, detail, delta, new_depth, metadata_str, now],
        )?;

        Ok(())
    })();

    match tx_result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            conn.execute_batch("ROLLBACK").ok();
            Err(e)
        }
    }
}

pub(super) fn log_growth_event(conn: &Connection, event: &NewGrowthEvent) -> Result<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    let metadata_str = serde_json::to_string(&event.metadata)?;
    conn.execute(
        "INSERT INTO growth_log (event_type, dimension_key, detail, delta, metadata, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            event.event_type,
            event.dimension_key,
            event.detail,
            event.delta,
            metadata_str,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub(super) fn query_growth_log(
    conn: &Connection,
    event_type: Option<&str>,
    limit: i64,
) -> Result<Vec<GrowthEvent>> {
    let (sql, params): (String, Vec<Box<dyn rusqlite::ToSql>>) = if let Some(et) = event_type {
        (
            "SELECT id, event_type, dimension_key, detail, delta, level_after, metadata, created_at
             FROM growth_log WHERE event_type = ?1 ORDER BY created_at DESC LIMIT ?2"
                .to_string(),
            vec![Box::new(et.to_string()), Box::new(limit)],
        )
    } else {
        (
            "SELECT id, event_type, dimension_key, detail, delta, level_after, metadata, created_at
             FROM growth_log ORDER BY created_at DESC LIMIT ?1"
                .to_string(),
            vec![Box::new(limit)],
        )
    };
    let pr: Vec<&dyn rusqlite::ToSql> = params.iter().map(|x| x.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(pr.as_slice(), |row| {
            let meta_str: String = row.get::<_, String>(6).unwrap_or_else(|_| "{}".into());
            Ok(GrowthEvent {
                id: row.get(0)?,
                event_type: row.get(1)?,
                dimension_key: row.get(2)?,
                detail: row.get(3)?,
                delta: row.get(4)?,
                level_after: row.get(5)?,
                metadata: serde_json::from_str(&meta_str).unwrap_or(serde_json::Value::Null),
                created_at: row.get(7)?,
            })
        })?
        .filter_map(row_ok)
        .collect();
    Ok(rows)
}

pub(super) fn adjust_personality_trait(
    conn: &Connection,
    trait_key: &str,
    value: f32,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    // Read current personality JSON
    let json_str: String = conn.query_row(
        "SELECT personality FROM companion_identity WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    let mut weights: PersonalityWeights = serde_json::from_str(&json_str).unwrap_or_default();

    // Clamp value to [0.0, 1.0]
    let clamped = value.clamp(0.0, 1.0);
    match trait_key {
        "warmth" => weights.warmth = clamped,
        "formality" => weights.formality = clamped,
        "humor" => weights.humor = clamped,
        "directness" => weights.directness = clamped,
        "verbosity" => weights.verbosity = clamped,
        other => {
            weights.extra.insert(other.to_string(), clamped);
        }
    }

    let new_json = serde_json::to_string(&weights)?;
    conn.execute(
        "UPDATE companion_identity SET personality = ?1, updated_at = ?2 WHERE id = 1",
        rusqlite::params![new_json, now],
    )?;
    Ok(())
}

// ── G5: Goal Tracking DB functions ───────────────────────────────────────

pub(super) fn create_goal(conn: &Connection, did: &str, goal: &Goal) -> Result<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    let milestones_json = serde_json::to_string(&goal.milestones)?;
    conn.execute(
        "INSERT INTO goals (user_did, title, description, status, parent_goal_id,
             progress_pct, milestones, created_at, target_date, last_progress_at, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            did,
            goal.title,
            goal.description,
            goal.status.as_str(),
            goal.parent_goal_id,
            goal.progress_pct as f64,
            milestones_json,
            now,
            goal.target_date,
            goal.last_progress_at,
            goal.source.as_str(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub(super) fn list_goals(
    conn: &Connection,
    did: &str,
    status: Option<&GoalStatus>,
) -> Result<Vec<Goal>> {
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match status {
        Some(s) => (
            "SELECT id, user_did, title, description, status, parent_goal_id,
                    progress_pct, milestones, created_at, target_date,
                    last_progress_at, source
             FROM goals WHERE user_did = ?1 AND status = ?2
             ORDER BY created_at DESC",
            vec![Box::new(did.to_string()), Box::new(s.as_str().to_string())],
        ),
        None => (
            "SELECT id, user_did, title, description, status, parent_goal_id,
                    progress_pct, milestones, created_at, target_date,
                    last_progress_at, source
             FROM goals WHERE user_did = ?1
             ORDER BY created_at DESC",
            vec![Box::new(did.to_string())],
        ),
    };
    let mut stmt = conn.prepare(sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(param_refs.as_slice(), read_goal_row)?;
    let mut goals = Vec::new();
    for row in rows {
        goals.push(row?);
    }
    Ok(goals)
}

pub(super) fn get_goal(conn: &Connection, id: i64) -> Result<Option<Goal>> {
    let mut stmt = conn.prepare(
        "SELECT id, user_did, title, description, status, parent_goal_id,
                progress_pct, milestones, created_at, target_date,
                last_progress_at, source
         FROM goals WHERE id = ?1",
    )?;
    match stmt.query_row([id], read_goal_row) {
        Ok(goal) => Ok(Some(goal)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(super) fn read_goal_row(row: &rusqlite::Row) -> rusqlite::Result<Goal> {
    let status_str: String = row.get(4)?;
    let milestones_json: String = row.get(7)?;
    let source_str: String = row.get(11)?;
    Ok(Goal {
        id: row.get(0)?,
        user_did: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        status: GoalStatus::from_str_lossy(&status_str),
        parent_goal_id: row.get(5)?,
        progress_pct: row.get::<_, f64>(6)? as f32,
        milestones: parse_json_column(&milestones_json, "milestones"),
        created_at: row.get(8)?,
        target_date: row.get(9)?,
        last_progress_at: row.get(10)?,
        source: GoalSource::from_str_lossy(&source_str),
    })
}

pub(super) fn update_goal(conn: &Connection, id: i64, updates: &GoalUpdate) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut sets = vec!["last_progress_at = ?1".to_string()];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
    let mut idx = 2;

    if let Some(ref title) = updates.title {
        sets.push(format!("title = ?{idx}"));
        params.push(Box::new(title.clone()));
        idx += 1;
    }
    if let Some(ref desc) = updates.description {
        sets.push(format!("description = ?{idx}"));
        params.push(Box::new(desc.clone()));
        idx += 1;
    }
    if let Some(ref status) = updates.status {
        sets.push(format!("status = ?{idx}"));
        params.push(Box::new(status.as_str().to_string()));
        idx += 1;
    }
    if let Some(pct) = updates.progress_pct {
        sets.push(format!("progress_pct = ?{idx}"));
        params.push(Box::new(pct.clamp(0.0, 1.0) as f64));
        idx += 1;
    }
    if let Some(ref td) = updates.target_date {
        sets.push(format!("target_date = ?{idx}"));
        params.push(Box::new(td.clone()));
        idx += 1;
    }
    let _ = idx;

    let sql = format!(
        "UPDATE goals SET {} WHERE id = ?{}",
        sets.join(", "),
        params.len() + 1
    );
    params.push(Box::new(id));
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = conn.execute(&sql, param_refs.as_slice())?;
    if rows == 0 {
        anyhow::bail!("goal {id} not found");
    }
    Ok(())
}

pub(super) fn add_milestone(conn: &Connection, goal_id: i64, title: &str) -> Result<()> {
    let milestones_json: String = conn.query_row(
        "SELECT milestones FROM goals WHERE id = ?1",
        [goal_id],
        |row| row.get(0),
    )?;
    let mut milestones: Vec<Milestone> = parse_json_column(&milestones_json, "milestones");
    milestones.push(Milestone {
        title: title.to_string(),
        completed: false,
        completed_at: None,
    });
    let updated_json = serde_json::to_string(&milestones)?;
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE goals SET milestones = ?1, last_progress_at = ?2 WHERE id = ?3",
        rusqlite::params![updated_json, now, goal_id],
    )?;
    Ok(())
}

pub(super) fn complete_milestone(
    conn: &Connection,
    goal_id: i64,
    milestone_idx: usize,
) -> Result<()> {
    let milestones_json: String = conn.query_row(
        "SELECT milestones FROM goals WHERE id = ?1",
        [goal_id],
        |row| row.get(0),
    )?;
    let mut milestones: Vec<Milestone> = parse_json_column(&milestones_json, "milestones");
    if milestone_idx >= milestones.len() {
        anyhow::bail!(
            "milestone index {} out of range (goal has {} milestones)",
            milestone_idx,
            milestones.len()
        );
    }
    milestones[milestone_idx].completed = true;
    milestones[milestone_idx].completed_at = Some(chrono::Utc::now().to_rfc3339());
    let updated_json = serde_json::to_string(&milestones)?;

    // Auto-update progress_pct based on milestone completion ratio
    let completed = milestones.iter().filter(|m| m.completed).count() as f64;
    let total = milestones.len() as f64;
    let progress = if total > 0.0 { completed / total } else { 0.0 };
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "UPDATE goals SET milestones = ?1, progress_pct = ?2, last_progress_at = ?3 WHERE id = ?4",
        rusqlite::params![updated_json, progress, now, goal_id],
    )?;
    Ok(())
}

// ── Agent Swarm DB functions (G2) ────────────────────────────────────────

/// G5: Link a fact to a goal. INSERT OR IGNORE prevents duplicates.
pub(super) fn link_fact_to_goal(
    conn: &Connection,
    goal_id: i64,
    fact_id: i64,
    similarity: f32,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO goal_fact_links (goal_id, fact_id, similarity, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![goal_id, fact_id, similarity, now],
    )?;
    Ok(())
}

/// Compute streak_days based on previous state and today's date.
/// Streak increments if last interaction was yesterday; resets to 1 if gap > 1 day.
pub(super) fn compute_streak_days(state: &RelationshipState, today: &str) -> i32 {
    let Some(ref last) = state.last_interaction_at else {
        return 1; // first interaction ever
    };
    let last_date = &last[..10.min(last.len())];
    if last_date == today {
        // Same day — keep current streak
        return state.streak_days.max(1);
    }

    // Parse dates to compute day gap
    let Ok(last_nd) = chrono::NaiveDate::parse_from_str(last_date, "%Y-%m-%d") else {
        return 1;
    };
    let Ok(today_nd) = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d") else {
        return 1;
    };
    let gap = (today_nd - last_nd).num_days();
    if gap == 1 {
        state.streak_days + 1
    } else {
        1 // streak broken (also covers negative gap from corrupt data)
    }
}

/// Format a ~150-200 token self-awareness string for the companion.
/// Injected as `<self_awareness>` section in the volatile prompt.
pub fn format_self_awareness(
    identity: &CompanionIdentity,
    dimensions: &[GrowthDimension],
    interaction_count: i64,
) -> String {
    let mut lines = Vec::new();

    // Name + birthday
    if !identity.name.is_empty() {
        lines.push(format!("Name: {}", identity.name));
    }
    if !identity.created_at.is_empty() {
        // Show day count since creation
        if let Ok(created) =
            chrono::DateTime::parse_from_rfc3339(&identity.created_at).or_else(|_| {
                // SQLite datetime('now') produces "YYYY-MM-DD HH:MM:SS" (no T, no TZ)
                chrono::NaiveDateTime::parse_from_str(&identity.created_at, "%Y-%m-%d %H:%M:%S")
                    .map(|ndt| ndt.and_utc().fixed_offset())
            })
        {
            let days = chrono::Utc::now()
                .signed_duration_since(created.with_timezone(&chrono::Utc))
                .num_days();
            lines.push(format!(
                "Birthday: {} (Day {})",
                &identity.created_at[..10],
                days.max(1)
            ));
        }
    }

    // Growth stage
    lines.push(format!(
        "Growth stage: {} ({} interactions)",
        identity.growth_stage.display_name(),
        interaction_count,
    ));

    // Twin soul pillars (compact)
    lines.push("Twin soul pillars:".to_string());
    for line in identity.core_values.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            lines.push(format!("  {}", trimmed));
        }
    }

    // Personality profile (compact key=value)
    let pw = &identity.personality;
    lines.push(format!(
        "Personality profile:\n  warmth={:.2} formality={:.2} humor={:.2} directness={:.2} verbosity={:.2}",
        pw.warmth, pw.formality, pw.humor, pw.directness, pw.verbosity
    ));

    // Growth dimensions (with trend arrow)
    if !dimensions.is_empty() {
        lines.push("Growth dimensions:".to_string());
        for dim in dimensions {
            let trend = if dim.velocity > 0.001 {
                " (rising)"
            } else if dim.velocity < -0.001 {
                " (falling)"
            } else {
                " (stable)"
            };
            lines.push(format!("  {}: {:.2}{}", dim.display_name, dim.depth, trend));
        }
    }

    // Style notes
    if !identity.communication_notes.is_empty() {
        lines.push(format!("Style notes: {}", identity.communication_notes));
    }

    // Warn on critical missing fields (helps catch seed data issues early)
    if identity.core_values.is_empty() {
        tracing::warn!(
            "[Growth] companion_identity.core_values is empty — soul pillars missing from prompt"
        );
    }
    if identity.created_at.is_empty() {
        tracing::warn!("[Growth] companion_identity.created_at is empty — birthday unknown");
    }

    lines.join("\n")
}

/// Format a ~200 token relationship context string for LLM prompt injection.
pub(super) fn format_relationship_context(
    state: &RelationshipState,
    active_goals: &[Goal],
) -> String {
    // No interaction yet — return empty (no context to inject)
    if state.interaction_count == 0 {
        return String::new();
    }

    let mut lines = Vec::new();

    // Last interaction + topic
    if let Some(ref last) = state.last_interaction_at {
        let elapsed = humanize_elapsed(last);
        if !state.last_topic.is_empty() {
            lines.push(format!("上次交互: {}，话题: {}", elapsed, state.last_topic));
        } else {
            lines.push(format!("上次交互: {}", elapsed));
        }
    }

    // Current focus (top 3, strength > 0.3)
    let active_focus: Vec<_> = state
        .current_focus
        .iter()
        .filter(|f| f.strength > 0.3)
        .take(3)
        .collect();
    if !active_focus.is_empty() {
        let topics: Vec<_> = active_focus
            .iter()
            .enumerate()
            .map(|(i, f)| format!("({}) {}", i + 1, f.topic))
            .collect();
        lines.push(format!("当前关注: {}", topics.join(" ")));
    }

    // Ongoing goals from G5 goals table (top 3)
    if !active_goals.is_empty() {
        let goals: Vec<_> = active_goals
            .iter()
            .take(3)
            .map(|g| format!("{}({}%)", g.title, (g.progress_pct * 100.0) as i32))
            .collect();
        lines.push(format!("进行中: {}", goals.join(", ")));
    } else if !state.ongoing_goals.is_empty() {
        // Fallback to legacy ongoing_goals in relationship_state
        let goals: Vec<_> = state
            .ongoing_goals
            .iter()
            .take(3)
            .map(|g| format!("{}({}%)", g.title, (g.progress_pct * 100.0) as i32))
            .collect();
        lines.push(format!("进行中: {}", goals.join(", ")));
    }

    // Narrative summary
    if !state.narrative.is_empty() {
        lines.push(format!("近况: {}", state.narrative));
    }

    // Interaction stats
    if state.interaction_count > 1 {
        lines.push(format!(
            "累计交互: {} 次，连续: {} 天",
            state.interaction_count, state.streak_days
        ));
    }

    lines.join("\n")
}

/// Human-readable elapsed time from an RFC 3339 timestamp to now.
pub(super) fn humanize_elapsed(timestamp: &str) -> String {
    let Ok(ts) = chrono::DateTime::parse_from_rfc3339(timestamp) else {
        return "未知时间".to_string();
    };
    let elapsed = chrono::Utc::now().signed_duration_since(ts);
    let minutes = elapsed.num_minutes();
    let hours = elapsed.num_hours();
    let days = elapsed.num_days();

    if minutes < 1 {
        // Covers both "just now" and negative durations (clock skew / corrupt data)
        "刚刚".to_string()
    } else if minutes < 60 {
        format!("{}分钟前", minutes)
    } else if hours < 24 {
        format!("{}小时前", hours)
    } else if days == 1 {
        "昨天".to_string()
    } else if days < 7 {
        format!("{}天前", days)
    } else if days < 30 {
        format!("{}周前", days / 7)
    } else {
        format!("{}个月前", days / 30)
    }
}

/// Extract a topic summary from the user's message without LLM calls.
/// Returns the first sentence (up to 50 chars), or a keyword-based summary.
pub fn extract_topic_from_message(message: &str) -> String {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Split on sentence-ending punctuation (Chinese and ASCII)
    let first_sentence = trimmed
        .split(['。', '？', '！', '.', '?', '!', '\n'])
        .next()
        .unwrap_or(trimmed)
        .trim();

    // Cap at 50 chars, break at word/char boundary
    if first_sentence.chars().count() <= 50 {
        first_sentence.to_string()
    } else {
        first_sentence.chars().take(50).collect::<String>() + "…"
    }
}

// ── Skill error category helpers ────────────────────────────────────────

/// Query error categories from the normalized table for a skill.
pub(super) fn query_error_categories(
    conn: &Connection,
    skill_name: &str,
) -> std::collections::HashMap<String, u64> {
    let mut stmt = match conn
        .prepare("SELECT category, count FROM skill_error_categories WHERE skill_name = ?1")
    {
        Ok(s) => s,
        Err(_) => return std::collections::HashMap::new(),
    };
    let rows = match stmt.query_map(rusqlite::params![skill_name], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
    }) {
        Ok(r) => r,
        Err(_) => return std::collections::HashMap::new(),
    };
    rows.filter_map(row_ok).collect()
}

// ── Deferred intent helpers (now reading from unified tasks table) ──────

/// Query the tasks table and map results to DeferredIntent structs.
/// Expected SELECT columns (in order): id, user_did, chat_id, trigger_type, trigger_at_unix,
/// event_key, execution_mode, COALESCE(NULLIF(context_text, ''), title) AS context_text, status, created_at, expires_at_unix, fired_at,
/// is_callback_origin, retry_count, interval_seconds, chain_depth, pin_on_fire, group_key,
/// max_fires, fire_count
pub(super) fn query_deferred_intents_from_tasks(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Vec<DeferredIntent> {
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(error = %e, "Failed to prepare tasks query for intents");
            return Vec::new();
        }
    };
    stmt.query_map(params, |row| {
        let is_cb: i32 = row.get(12)?;
        let status_raw: String = row.get(8)?;
        // Map 'active' back to 'pending' for intent engine compatibility
        let status = if status_raw == "active" {
            "pending".to_string()
        } else {
            status_raw
        };
        Ok(DeferredIntent {
            id: row.get(0)?,
            user_did: row.get(1)?,
            chat_id: row.get(2)?,
            trigger_type: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            trigger_at_unix: row.get(4)?,
            event_key: row.get(5)?,
            execution_mode: row
                .get::<_, Option<String>>(6)?
                .unwrap_or_else(|| "notify".to_string()),
            context_text: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
            status,
            created_at: row.get(9)?,
            expires_at_unix: row.get::<_, Option<i64>>(10)?.unwrap_or(0),
            fired_at: row.get(11)?,
            is_callback_origin: is_cb != 0,
            retry_count: row.get(13).unwrap_or(0),
            interval_seconds: row.get(14).unwrap_or(None),
            chain_depth: row.get(15).unwrap_or(0),
            pin_on_fire: row.get::<_, i32>(16).unwrap_or(0) != 0,
            group_key: row.get(17).unwrap_or(None),
            max_fires: row.get(18).unwrap_or(None),
            fire_count: row.get(19).unwrap_or(0),
            recur_weekdays: row
                .get::<_, Option<String>>(20)
                .unwrap_or(None)
                .and_then(|s| parse_recur_weekdays_json(&s)),
            recur_window_start: row.get(21).unwrap_or(None),
            recur_window_end: row.get(22).unwrap_or(None),
            recur_timezone: row.get(23).unwrap_or(None),
            embedding: None,
        })
    })
    .map(|rows| rows.filter_map(row_ok).collect())
    .unwrap_or_default()
}

/// Decode the JSON-encoded `recur_weekdays` column (e.g. "[2,3,4]") into a
/// `Vec<u8>`. Returns `None` on empty/invalid input so a corrupt cell degrades
/// to "no weekday restriction" rather than failing the whole row.
pub(super) fn parse_recur_weekdays_json(s: &str) -> Option<Vec<u8>> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<Vec<u8>>(trimmed)
        .ok()
        .filter(|v| !v.is_empty())
}

/// Window-snap guard applied at the actor's trigger-write chokepoint. Loads the
/// task row's windowed-recurrence constraints and returns the first allowed
/// instant at/after `requested_trigger`. For a non-windowed row (all recur_*
/// NULL) or a missing/unreadable row it returns `requested_trigger` unchanged
/// (no-op) and is idempotent. Centralizing here is the ROOT fix: EVERY path that
/// rewrites a recurring intent's `trigger_at_unix` (manual reschedule, the panel
/// "skip", the Telegram snooze active/inactive paths, any future writer) routes
/// through one of the actor handlers below, so a windowed recurring intent can
/// never be advanced out of its window regardless of the caller. The firing path
/// pre-snaps via `next_windowed_trigger`; snapping again here is a harmless no-op.
pub(super) fn snap_trigger_for_task(
    conn: &rusqlite::Connection,
    id: i64,
    requested_trigger: i64,
) -> i64 {
    match conn.query_row(
        "SELECT recur_weekdays, recur_window_start, recur_window_end, recur_timezone FROM tasks WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    ) {
        Ok((wk, ws, we, tz)) => {
            let weekdays = wk.as_deref().and_then(parse_recur_weekdays_json);
            crate::intent::snap_into_window(
                requested_trigger,
                weekdays.as_deref(),
                ws.as_deref(),
                we.as_deref(),
                tz.as_deref(),
            )
        }
        Err(_) => requested_trigger,
    }
}

/// Map a row from the tasks table to a Task struct.
/// Expected columns: id(0), chat_id(1), user_did(2), title(3), body(4), priority(5),
/// status(6), tags(7), due_date(8), source(9), trigger_at_unix(10), trigger_type(11),
/// event_key(12), execution_mode(13), context_text(14), interval_seconds(15),
/// group_key(16), max_fires(17), fire_count(18), fired_at(19), retry_count(20),
/// chain_depth(21), pin_on_fire(22), expires_at_unix(23), is_callback_origin(24),
/// created_at(25), updated_at(26), completed_at(27),
/// recur_weekdays(28), recur_window_start(29), recur_window_end(30), recur_timezone(31)
pub(super) fn map_task_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let tags_json: String = row.get(7)?;
    let tags: Vec<String> = parse_json_column(&tags_json, "task_tags");
    Ok(Task {
        id: row.get(0)?,
        chat_id: row.get(1)?,
        user_did: row.get(2)?,
        title: row.get(3)?,
        body: row.get(4)?,
        priority: row.get(5)?,
        status: row.get(6)?,
        tags,
        due_date: row.get(8)?,
        source: row.get(9)?,
        trigger_at_unix: row.get(10)?,
        trigger_type: row.get(11)?,
        event_key: row.get(12)?,
        execution_mode: row
            .get::<_, Option<String>>(13)?
            .unwrap_or_else(|| "notify".to_string()),
        context_text: row.get(14)?,
        interval_seconds: row.get(15)?,
        group_key: row.get(16)?,
        max_fires: row.get(17)?,
        fire_count: row.get(18).unwrap_or(0),
        recur_weekdays: row
            .get::<_, Option<String>>(28)
            .unwrap_or(None)
            .and_then(|s| parse_recur_weekdays_json(&s)),
        recur_window_start: row.get(29).unwrap_or(None),
        recur_window_end: row.get(30).unwrap_or(None),
        recur_timezone: row.get(31).unwrap_or(None),
        fired_at: row.get(19)?,
        retry_count: row.get(20).unwrap_or(0),
        chain_depth: row.get(21).unwrap_or(0),
        pin_on_fire: row.get::<_, i32>(22).unwrap_or(0) != 0,
        expires_at_unix: row.get(23)?,
        is_callback_origin: row.get::<_, i32>(24).unwrap_or(0) != 0,
        created_at: row.get(25)?,
        updated_at: row.get(26)?,
        completed_at: row.get(27)?,
    })
}

/// Get a single task by ID.
pub(super) fn query_single_task(conn: &Connection, id: i64) -> Option<Task> {
    conn.query_row(
        "SELECT id, chat_id, user_did, title, body, priority, status, tags, due_date, source,
                trigger_at_unix, trigger_type, event_key, execution_mode, context_text,
                interval_seconds, group_key, max_fires, fire_count, fired_at,
                retry_count, chain_depth, pin_on_fire, expires_at_unix, is_callback_origin,
                created_at, updated_at, completed_at,
                recur_weekdays, recur_window_start, recur_window_end, recur_timezone
         FROM tasks WHERE id = ?1",
        rusqlite::params![id],
        map_task_row,
    )
    .ok()
}

// ── Skill registry (skill_proposals) helpers ─────────────────────────────

pub(super) fn get_skill_proposal_row(conn: &Connection, id: i64) -> Option<SkillProposal> {
    conn.query_row(
        "SELECT id, skill_name, requested_by, status, stage_error, retry_count,
                user_request, proposal_summary, design_json, generated_source,
                generated_manifest, generated_cargo_toml, compiled_wasm,
                audit_static_result, audit_llm_result, audit_overridden,
                audit_override_by, audit_override_at, audit_override_reason,
                test_result, requires_auth, auth_domain, total_llm_calls,
                version, skill_md, capability_graph_json, execution_dag_json,
                created_at, updated_at
         FROM skill_proposals WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            Ok(SkillProposal {
                id: row.get(0)?,
                skill_name: row.get(1)?,
                requested_by: row.get(2)?,
                status: row.get(3)?,
                stage_error: row.get(4)?,
                retry_count: row.get(5)?,
                user_request: row.get(6)?,
                proposal_summary: row.get(7)?,
                design_json: row.get(8)?,
                generated_source: row.get(9)?,
                generated_manifest: row.get(10)?,
                generated_cargo_toml: row.get(11)?,
                compiled_wasm: row.get(12)?,
                audit_static_result: row.get(13)?,
                audit_llm_result: row.get(14)?,
                audit_overridden: row.get::<_, i32>(15)? != 0,
                audit_override_by: row.get(16)?,
                audit_override_at: row.get(17)?,
                audit_override_reason: row.get(18)?,
                test_result: row.get(19)?,
                requires_auth: row.get::<_, i32>(20)? != 0,
                auth_domain: row.get(21)?,
                total_llm_calls: row.get(22)?,
                version: row.get(23)?,
                skill_md: row.get(24)?,
                capability_graph_json: row.get(25)?,
                execution_dag_json: row.get(26)?,
                created_at: row.get(27)?,
                updated_at: row.get(28)?,
            })
        },
    )
    .map_err(|e| {
        tracing::error!(id, error = %e, "get_skill_proposal_row query failed");
        e
    })
    .ok()
}

pub(super) fn list_skill_proposals_rows(
    conn: &Connection,
    requested_by: Option<&str>,
    status_filter: Option<&str>,
) -> Vec<SkillProposal> {
    // Build dynamic SQL with optional filters
    let mut sql = String::from(
        "SELECT id, skill_name, requested_by, status, stage_error, retry_count,
                user_request, proposal_summary, design_json, generated_source,
                generated_manifest, generated_cargo_toml, compiled_wasm,
                audit_static_result, audit_llm_result, audit_overridden,
                audit_override_by, audit_override_at, audit_override_reason,
                test_result, requires_auth, auth_domain, total_llm_calls,
                version, skill_md, capability_graph_json, execution_dag_json,
                created_at, updated_at
         FROM skill_proposals WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(rb) = requested_by {
        params.push(Box::new(rb.to_string()));
        sql.push_str(&format!(" AND requested_by = ?{}", params.len()));
    }
    if let Some(sf) = status_filter {
        // Support comma-separated status values (e.g. "pending_approval,designing")
        let statuses: Vec<&str> = sf.split(',').map(|s| s.trim()).collect();
        if statuses.len() == 1 {
            params.push(Box::new(sf.to_string()));
            sql.push_str(&format!(" AND status = ?{}", params.len()));
        } else {
            let placeholders: Vec<String> = statuses
                .iter()
                .map(|s| {
                    params.push(Box::new(s.to_string()));
                    format!("?{}", params.len())
                })
                .collect();
            sql.push_str(&format!(" AND status IN ({})", placeholders.join(",")));
        }
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT 50");

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "list_skill_proposals prepare failed (possible schema mismatch)");
            return Vec::new();
        }
    };
    let rows = match stmt.query_map(param_refs.as_slice(), |row| {
        Ok(SkillProposal {
            id: row.get(0)?,
            skill_name: row.get(1)?,
            requested_by: row.get(2)?,
            status: row.get(3)?,
            stage_error: row.get(4)?,
            retry_count: row.get(5)?,
            user_request: row.get(6)?,
            proposal_summary: row.get(7)?,
            design_json: row.get(8)?,
            generated_source: row.get(9)?,
            generated_manifest: row.get(10)?,
            generated_cargo_toml: row.get(11)?,
            compiled_wasm: row.get(12)?,
            audit_static_result: row.get(13)?,
            audit_llm_result: row.get(14)?,
            audit_overridden: row.get::<_, i32>(15)? != 0,
            audit_override_by: row.get(16)?,
            audit_override_at: row.get(17)?,
            audit_override_reason: row.get(18)?,
            test_result: row.get(19)?,
            requires_auth: row.get::<_, i32>(20)? != 0,
            auth_domain: row.get(21)?,
            total_llm_calls: row.get(22)?,
            version: row.get(23)?,
            skill_md: row.get(24)?,
            capability_graph_json: row.get(25)?,
            execution_dag_json: row.get(26)?,
            created_at: row.get(27)?,
            updated_at: row.get(28)?,
        })
    }) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "list_skill_proposals query failed");
            return Vec::new();
        }
    };
    rows.filter_map(|r| {
        r.map_err(|e| {
            tracing::error!(error = %e, "list_skill_proposals row parse failed (possible schema mismatch)");
            e
        }).ok()
    }).collect()
}

pub(super) fn update_skill_proposal_row(
    conn: &Connection,
    id: i64,
    updates: &SkillProposalUpdate,
) -> Result<()> {
    let mut set_clauses = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    sql_set!(params, set_clauses, updates, skill_name, "skill_name");
    sql_set!(params, set_clauses, updates, status, "status");
    sql_set!(params, set_clauses, updates, stage_error, "stage_error");
    sql_set_num!(params, set_clauses, updates, retry_count, "retry_count");
    sql_set!(
        params,
        set_clauses,
        updates,
        proposal_summary,
        "proposal_summary"
    );
    sql_set!(params, set_clauses, updates, design_json, "design_json");
    sql_set!(
        params,
        set_clauses,
        updates,
        generated_source,
        "generated_source"
    );
    sql_set!(
        params,
        set_clauses,
        updates,
        generated_manifest,
        "generated_manifest"
    );
    sql_set!(
        params,
        set_clauses,
        updates,
        generated_cargo_toml,
        "generated_cargo_toml"
    );
    sql_set!(params, set_clauses, updates, compiled_wasm, "compiled_wasm");
    sql_set!(
        params,
        set_clauses,
        updates,
        audit_static_result,
        "audit_static_result"
    );
    sql_set!(
        params,
        set_clauses,
        updates,
        audit_llm_result,
        "audit_llm_result"
    );
    sql_set_bool!(
        params,
        set_clauses,
        updates,
        audit_overridden,
        "audit_overridden"
    );
    sql_set!(
        params,
        set_clauses,
        updates,
        audit_override_by,
        "audit_override_by"
    );
    sql_set!(
        params,
        set_clauses,
        updates,
        audit_override_at,
        "audit_override_at"
    );
    sql_set!(
        params,
        set_clauses,
        updates,
        audit_override_reason,
        "audit_override_reason"
    );
    sql_set!(params, set_clauses, updates, test_result, "test_result");
    sql_set_bool!(params, set_clauses, updates, requires_auth, "requires_auth");
    sql_set!(params, set_clauses, updates, auth_domain, "auth_domain");
    sql_set_num!(
        params,
        set_clauses,
        updates,
        total_llm_calls,
        "total_llm_calls"
    );
    sql_set_num!(params, set_clauses, updates, version, "version");
    sql_set!(params, set_clauses, updates, skill_md, "skill_md");
    sql_set!(
        params,
        set_clauses,
        updates,
        capability_graph_json,
        "capability_graph_json"
    );
    sql_set!(
        params,
        set_clauses,
        updates,
        execution_dag_json,
        "execution_dag_json"
    );

    if set_clauses.is_empty() {
        return Ok(());
    }

    // Always update updated_at
    let now = Utc::now().to_rfc3339();
    params.push(Box::new(now));
    set_clauses.push(format!("updated_at = ?{}", params.len()));

    params.push(Box::new(id));
    let sql = format!(
        "UPDATE skill_proposals SET {} WHERE id = ?{}",
        set_clauses.join(", "),
        params.len()
    );

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    conn.execute(&sql, param_refs.as_slice())
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("update_skill_proposal: {e}"))
}
