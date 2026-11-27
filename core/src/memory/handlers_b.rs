use super::*;

/// Extra L1 messages tolerated before a batch trim (see the push handler).
pub(crate) const L1_TRIM_SLACK: usize = 20;

// ── Memory core handler ─────────────────────────────────────────────────────

pub(super) fn handle_memory_core_cmd(
    conn: &rusqlite::Connection,
    cmd: MemoryCommand,
    db_path_str: &str,
    max_db_size: u64,
    size_warning_issued: &mut bool,
    l1_caches: &mut std::collections::HashMap<String, VecDeque<Message>>,
    l1_max_size: usize,
) {
    match cmd {
        MemoryCommand::PushL1 { did, msg, reply } => {
            // L3: Archive every message for full-text search (zero-cost local op)
            // Warn-and-continue per Phase E D7 / FIX R2-3 — archive failure is NOT a
            // legacy-write failure for G6 ordering; caller-observable success is
            // the in-memory cache.push_back below.
            if let Err(e) = archive_to_l3(conn, &did, &msg) {
                tracing::debug!(error = %e, "L3 archive write failed (non-fatal)");
            }

            let cache = l1_caches.entry(did.clone()).or_default();
            // TTL eviction: remove messages older than 7 days. They go through the
            // same overflow summary as size-based eviction (before, a quiet week
            // dropped the whole conversation with no digest written).
            let cutoff = Utc::now().timestamp() - L1_TTL_SECS;
            let mut ttl_evicted: Vec<Message> = Vec::new();
            cache.retain(|m| {
                if m.timestamp >= cutoff {
                    true
                } else {
                    ttl_evicted.push(m.clone());
                    false
                }
            });
            for evicted in ttl_evicted {
                let summary = extract_facts_from_message(&evicted);
                if summary.is_empty() {
                    continue;
                }
                if l2_write_guard(
                    conn,
                    &did,
                    "l1_overflow",
                    &summary,
                    None,
                    MemoryTier::Ephemeral,
                ) == WriteDecision::Skip
                {
                    continue;
                }
                if let Err(e) = conn.execute(
                    "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, source, created_at, expires_at)
                     VALUES (?1, 'l1_overflow', ?2, 'l1_overflow', datetime('now'), datetime('now', '+30 days'))",
                    rusqlite::params![did, summary],
                ) {
                    tracing::warn!(error = %e, "[MemoryActor] L1 TTL overflow insert failed");
                    break;
                }
            }
            cache.push_back(msg); // ← G6 "legacy-write success" point for Phase E.
            // Trim in batches, not one message per push: a window that slides by
            // one every turn changes the conversation prefix every turn and
            // defeats the provider prompt cache (2026-09-06). Let the window
            // grow to max + L1_TRIM_SLACK, then cut back to max.
            let trim_now = cache.len() > l1_max_size + L1_TRIM_SLACK;
            while trim_now && cache.len() > l1_max_size {
                if let Some(evicted) = cache.pop_front() {
                    // L1→Tier 2 overflow: store in l2_ephemeral with 30-day TTL.
                    // Note: message is already archived in L3 (line above), so
                    // this is a best-effort short-term enrichment, not the only copy.
                    let summary = extract_facts_from_message(&evicted);
                    if !summary.is_empty() {
                        // Apply write guard cooldown to prevent duplicate overflow entries
                        let decision = l2_write_guard(
                            conn,
                            &did,
                            "l1_overflow",
                            &summary,
                            None,
                            MemoryTier::Ephemeral,
                        );
                        if decision == WriteDecision::Skip {
                            continue;
                        }
                        if let Err(e) = conn.execute(
                            "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, source, created_at, expires_at)
                             VALUES (?1, 'l1_overflow', ?2, 'l1_overflow', datetime('now'), datetime('now', '+30 days'))",
                            rusqlite::params![did, summary],
                        ) {
                            // Same warn-and-continue as today (NOT a legacy-write failure).
                            tracing::warn!(error = %e, "[MemoryActor] L1→Tier2 overflow insert failed — stopping eviction flush");
                            break;
                        }
                    }
                }
            }
            // Ack success: legacy cache.push_back (the G6 success point) is done.
            mem_reply!(reply, Ok(()));
        }
        MemoryCommand::GetL1 { did, reply } => {
            let msgs = l1_caches
                .get(&did)
                .map(|c| c.iter().cloned().collect())
                .unwrap_or_default();
            mem_reply!(reply, msgs);
        }
        MemoryCommand::StoreL2 {
            did,
            topic,
            fact,
            embedding,
            reply,
        } => {
            // Apply write guard: check for duplicates/contradictions
            let decision = l2_write_guard(
                conn,
                &did,
                &topic,
                &fact,
                embedding.as_deref(),
                MemoryTier::Facts,
            );

            let contradicts_id = match decision {
                WriteDecision::Skip => {
                    tracing::debug!(topic = %topic, "l2_write_guard: Skip (duplicate)");
                    mem_reply!(reply, Ok(-1i64));
                    return;
                }
                WriteDecision::Update(existing_id) => {
                    // Refresh content, but keep the previous summary in l2_fact_history.
                    let externalized = externalize_long_fact(&fact);
                    let now = Utc::now().to_rfc3339();
                    let result = (|| -> Result<i64> {
                        snapshot_fact_history(conn, existing_id, &did, "update")?;
                        conn.execute(
                            "UPDATE l2_experience SET fact_summary = ?1, last_accessed = ?2
                             WHERE id = ?3",
                            rusqlite::params![externalized, now, existing_id],
                        )?;
                        Ok(existing_id)
                    })();
                    tracing::debug!(id = existing_id, topic = %topic, "l2_write_guard: Update");
                    mem_reply!(reply, result);
                    return;
                }
                WriteDecision::InsertWithContradiction(existing_id) => {
                    tracing::info!(
                        existing_id,
                        topic = %topic,
                        "l2_write_guard: InsertWithContradiction — new fact may contradict existing"
                    );
                    Some(existing_id)
                }
                WriteDecision::Insert => None,
            };

            // Check per-DID fact count limit
            let fact_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM l2_experience WHERE agent_did = ?1",
                    rusqlite::params![did],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            if fact_count >= MAX_FACTS_PER_DID as i64 {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!(
                        "Per-user fact limit reached ({MAX_FACTS_PER_DID} facts). \
                         Older facts must be cleaned up before storing new ones."
                    ))
                );
                return;
            }

            // Check database size before writing
            if is_db_over_quota(conn, db_path_str, max_db_size, size_warning_issued) {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!(
                        "Memory storage limit reached. \
                         To expand, set MEMORY_MAX_SIZE_GB in .env and restart. \
                         Current writes are blocked to protect disk space."
                    ))
                );
                return;
            }

            // Content externalization: truncate long facts for Tier 1 storage
            let externalized = externalize_long_fact(&fact);
            let epistemic_kind = legacy_insert_kind(&topic, None);
            let now = Utc::now().to_rfc3339();
            let result = conn
                .execute(
                    "INSERT INTO l2_experience (
                        agent_did, topic, fact_summary, created_at, embedding, last_accessed,
                        epistemic_kind
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?4, ?6)",
                    rusqlite::params![did, topic, externalized, now, embedding, epistemic_kind],
                )
                .map(|_| conn.last_insert_rowid())
                .map_err(anyhow::Error::from);
            if let (Ok(new_id), Some(existing_id)) = (&result, contradicts_id)
                && let Err(e) = weaken_and_queue(conn, &did, existing_id, *new_id, "write_guard")
            {
                tracing::warn!(error = %e, "contradiction queue append failed");
            }
            mem_reply!(reply, result);
        }
        MemoryCommand::StoreL2WithMeta {
            did,
            topic,
            fact,
            embedding,
            confidence,
            expires_at,
            reply,
        } => {
            // Apply write guard (same as StoreL2) — this is the highest-volume
            // writer and needs dedup most.
            let decision = l2_write_guard(
                conn,
                &did,
                &topic,
                &fact,
                embedding.as_deref(),
                MemoryTier::Facts,
            );

            let contradicts_id = match decision {
                WriteDecision::Skip => {
                    tracing::debug!(topic = %topic, "l2_write_guard: Skip (duplicate) [WithMeta]");
                    mem_reply!(reply, Ok(-1i64));
                    return;
                }
                WriteDecision::Update(existing_id) => {
                    let externalized = externalize_long_fact(&fact);
                    let now = Utc::now().to_rfc3339();
                    let result = (|| -> Result<i64> {
                        snapshot_fact_history(conn, existing_id, &did, "update")?;
                        conn.execute(
                            "UPDATE l2_experience SET fact_summary = ?1, last_accessed = ?2, confidence = ?3
                             WHERE id = ?4",
                            rusqlite::params![externalized, now, confidence, existing_id],
                        )?;
                        Ok(existing_id)
                    })();
                    tracing::debug!(id = existing_id, topic = %topic, "l2_write_guard: Update [WithMeta]");
                    mem_reply!(reply, result);
                    return;
                }
                WriteDecision::InsertWithContradiction(existing_id) => {
                    tracing::info!(
                        existing_id,
                        topic = %topic,
                        "l2_write_guard: InsertWithContradiction [WithMeta]"
                    );
                    Some(existing_id)
                }
                WriteDecision::Insert => None,
            };

            let fact_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM l2_experience WHERE agent_did = ?1",
                    rusqlite::params![did],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            if fact_count >= MAX_FACTS_PER_DID as i64 {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!(
                        "Per-user fact limit reached ({MAX_FACTS_PER_DID} facts)."
                    ))
                );
                return;
            }
            if is_db_over_quota(conn, db_path_str, max_db_size, size_warning_issued) {
                mem_reply!(reply, Err(anyhow::anyhow!("Memory storage limit reached.")));
                return;
            }
            let externalized = externalize_long_fact(&fact);
            let epistemic_kind = legacy_insert_kind(&topic, Some(confidence));
            let now = Utc::now().to_rfc3339();
            let result = conn
                .execute(
                    "INSERT INTO l2_experience \
                         (agent_did, topic, fact_summary, created_at, embedding, last_accessed, confidence, expires_at, epistemic_kind) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?4, ?6, ?7, ?8)",
                    rusqlite::params![did, topic, externalized, now, embedding, confidence, expires_at, epistemic_kind],
                )
                .map(|_| conn.last_insert_rowid())
                .map_err(anyhow::Error::from);
            if let (Ok(new_id), Some(existing_id)) = (&result, contradicts_id)
                && let Err(e) = weaken_and_queue(conn, &did, existing_id, *new_id, "write_guard")
            {
                tracing::warn!(error = %e, "contradiction queue append failed");
            }
            mem_reply!(reply, result);
        }
        MemoryCommand::QueryL2Semantic {
            did,
            query_embedding,
            hnsw: hnsw_ref,
            query_text,
            reply,
        } => {
            let query_vec = EmbeddingClient::from_blob(&query_embedding);
            let results = semantic_search(
                conn,
                &did,
                &query_vec,
                hnsw_ref.as_deref(),
                query_text.as_deref(),
            );
            mem_reply!(reply, results);
        }
        MemoryCommand::GetAllL2 { did, reply } => {
            let results = (|| -> Option<Vec<Experience>> {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, agent_did, topic, fact_summary, created_at, confidence, is_pinned
                             FROM l2_experience
                             WHERE agent_did = ?1
                             ORDER BY is_pinned DESC, created_at DESC
                             LIMIT 50",
                    )
                    .map_err(|e| {
                        tracing::error!(error = %e, "l2_experience prepare failed");
                        e
                    })
                    .ok()?;
                let rows: Vec<_> = stmt
                    .query_map(rusqlite::params![did], |row| {
                        Ok(Experience {
                            id: row.get(0)?,
                            agent_did: row.get(1)?,
                            topic: row.get(2)?,
                            fact_summary: row.get(3)?,
                            created_at: row.get(4)?,
                            confidence: row.get::<_, Option<f64>>(5)?.unwrap_or(0.6),
                            is_pinned: row.get::<_, Option<i64>>(6)?.unwrap_or(0) != 0,
                        })
                    })
                    .map_err(|e| {
                        tracing::error!(error = %e, "l2_experience query failed");
                        e
                    })
                    .ok()?
                    .filter_map(row_ok)
                    .collect();
                Some(rows)
            })()
            .unwrap_or_default();
            mem_reply!(reply, results);
        }
        MemoryCommand::GetL2ById { did, id, reply } => {
            let result = (|| -> Result<Option<Experience>> {
                let mut stmt = conn.prepare(
                    "SELECT id, agent_did, topic, fact_summary, created_at, confidence, is_pinned
                     FROM l2_experience
                     WHERE agent_did = ?1 AND id = ?2
                     LIMIT 1",
                )?;
                let mut rows = stmt.query(rusqlite::params![did, id])?;
                let Some(row) = rows.next()? else {
                    return Ok(None);
                };
                Ok(Some(Experience {
                    id: row.get(0)?,
                    agent_did: row.get(1)?,
                    topic: row.get(2)?,
                    fact_summary: row.get(3)?,
                    created_at: row.get(4)?,
                    confidence: row.get::<_, Option<f64>>(5)?.unwrap_or(0.6),
                    is_pinned: row.get::<_, Option<i64>>(6)?.unwrap_or(0) != 0,
                }))
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::TouchL2ById { did, id, reply } => {
            let result = conn
                .execute(
                    "UPDATE l2_experience SET last_accessed = ?1
                     WHERE id = ?2 AND agent_did = ?3",
                    rusqlite::params![Utc::now().to_rfc3339(), id, did],
                )
                .map(|changed| changed > 0)
                .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::DeleteL2 { did, id, reply } => {
            let result = conn
                .execute(
                    "DELETE FROM l2_experience WHERE id = ?1 AND agent_did = ?2",
                    rusqlite::params![id, did],
                )
                .map(|changed| {
                    if changed > 0 {
                        // Cascade: clean up orphaned recall stats
                        if let Err(e) =
                            conn.execute("DELETE FROM l2_recall_stats WHERE fact_id = ?1", [id])
                        {
                            tracing::debug!(error = %e, id, "cascade delete recall_stats failed");
                        }
                    }
                    changed > 0
                })
                .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::UpdateL2 {
            did,
            id,
            fact,
            confidence,
            reply,
        } => {
            let externalized = externalize_long_fact(&fact);
            let now = Utc::now().to_rfc3339();
            let result = (|| -> Result<bool> {
                if !snapshot_fact_history(conn, id, &did, "update")? {
                    return Ok(false);
                }
                let changed = conn.execute(
                    "UPDATE l2_experience
                     SET fact_summary = ?1, confidence = ?2, last_accessed = ?3, embedding = NULL
                     WHERE id = ?4 AND agent_did = ?5",
                    rusqlite::params![externalized, confidence, now, id, did],
                )?;
                Ok(changed > 0)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::SetL2Pinned {
            did,
            id,
            pinned,
            reply,
        } => {
            let result = conn
                .execute(
                    "UPDATE l2_experience SET is_pinned = ?1, last_accessed = ?2
                     WHERE id = ?3 AND agent_did = ?4",
                    rusqlite::params![if pinned { 1 } else { 0 }, Utc::now().to_rfc3339(), id, did],
                )
                .map(|changed| changed > 0)
                .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::CheckDbSize { reply } => {
            let current = get_total_db_size(conn, db_path_str);
            mem_reply!(
                reply,
                DbSizeStatus {
                    current_bytes: current,
                    max_bytes: max_db_size,
                    exceeded: current >= max_db_size,
                }
            );
        }
        MemoryCommand::Healthcheck { reply } => {
            mem_reply!(reply, true);
        }
        MemoryCommand::CountEmbeddedFacts { reply } => {
            // Counts rows in the durable, HNSW-indexed corpus. Excludes
            // `l1_overflow` because those are transient L1 snapshots that
            // aren't part of the semantic corpus surface.
            let n: rusqlite::Result<i64> = conn.query_row(
                "SELECT COUNT(*) FROM l2_experience \
                 WHERE embedding IS NOT NULL AND topic != 'l1_overflow'",
                [],
                |r| r.get(0),
            );
            mem_reply!(reply, n.map_err(anyhow::Error::from));
        }
        MemoryCommand::QueryL3Fulltext { did, query, reply } => {
            let results = fulltext_search_l3(conn, &did, &query);
            mem_reply!(reply, results);
        }
        MemoryCommand::ListRecentL3 { did, limit, reply } => {
            let results = list_recent_l3(conn, &did, limit);
            mem_reply!(reply, results);
        }
        MemoryCommand::ListTriggerArchive {
            did,
            created_at_unix,
            task_id,
            reply,
        } => {
            mem_reply!(
                reply,
                trigger_archive_for(conn, &did, created_at_unix, task_id)
            );
        }

        _ => {
            tracing::error!("BUG: non-memory-core command routed to handle_memory_core_cmd");
        }
    }
}

// ── Row-parsing helpers (eliminate duplicate struct construction) ────────────

pub(super) fn parse_heartbeat_audit_row(
    row: &rusqlite::Row,
) -> rusqlite::Result<HeartbeatAuditEntry> {
    let tools_called_json: String = row.get(6)?;
    let tools_blocked_json: String = row.get(7)?;
    let actions_taken_json: String = row.get(8)?;
    Ok(HeartbeatAuditEntry {
        id: row.get(0)?,
        cycle_id: row.get(1)?,
        skill_name: row.get(2)?,
        timestamp: row.get(3)?,
        severity: row.get(4)?,
        probe_payload: row.get(5)?,
        tools_called: parse_json_column(&tools_called_json, "tools_called"),
        tools_blocked: parse_json_column(&tools_blocked_json, "tools_blocked"),
        actions_taken: parse_json_column(&actions_taken_json, "actions_taken"),
        llm_reply: row.get(9)?,
        tokens_used: row.get(10)?,
        rounds_used: row.get(11)?,
        user_did: row.get(12)?,
        chat_id: row.get(13)?,
        raw_process_log: row.get(14)?,
        tool_call_details: row.get(15)?,
        outcome: row.get(16)?,
        skip_reason: row.get(17)?,
        config_snapshot_json: row.get(18)?,
        prompt_hash: row.get(19)?,
        proposals_created_json: row.get(20)?,
        next_tick_note_bytes: row.get(21)?,
    })
}

pub(super) fn parse_prompt_override_row(row: &rusqlite::Row) -> rusqlite::Result<PromptOverride> {
    Ok(PromptOverride {
        id: row.get(0)?,
        module_name: row.get(1)?,
        version: row.get(2)?,
        original_text: row.get(3)?,
        modified_text: row.get(4)?,
        rationale: row.get(5)?,
        status: row.get(6)?,
        approval_id: row.get(7)?,
        cycles_since_apply: row.get(8)?,
        created_at: row.get(9)?,
    })
}

// ── Monitoring intent handler ──────────────────────────────────────────────

pub(super) fn handle_monitoring_cmd(
    conn: &rusqlite::Connection,
    cmd: MemoryCommand,
    db_path_str: &str,
    max_db_size: u64,
    size_warning_issued: &mut bool,
) {
    match cmd {
        MemoryCommand::StoreMonitoringIntent {
            user_did,
            skill_name,
            description,
            threshold,
            embedding,
            reply,
        } => {
            if is_db_over_quota(conn, db_path_str, max_db_size, size_warning_issued) {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!("DB quota exceeded, write blocked"))
                );
                return;
            }
            let result = (|| -> Result<i64> {
                conn.execute(
                    "UPDATE tasks SET status = 'cancelled', updated_at = datetime('now') WHERE user_did = ?1 AND trigger_type = 'semantic_alert' AND monitoring_skill = ?2 AND status = 'active'",
                    rusqlite::params![user_did, skill_name],
                )?;
                let chat_id: i64 = user_did
                    .strip_prefix("did:aegis:telegram:")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                conn.execute(
                    "INSERT INTO tasks (user_did, chat_id, title, body, trigger_type, execution_mode, context_text, status, source, monitoring_skill, monitoring_threshold, embedding_vector)
                     VALUES (?1, ?2, ?3, ?4, 'semantic_alert', 'notify', ?4, 'active', 'system', ?5, ?6, ?7)",
                    rusqlite::params![
                        user_did, chat_id,
                        format!("Monitor: {}", skill_name),
                        description, skill_name, threshold, embedding,
                    ],
                )?;
                Ok(conn.last_insert_rowid())
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::ListMonitoringIntents { user_did, reply } => {
            let result = (|| -> Result<Vec<MonitoringIntentRecord>> {
                let mut stmt = conn.prepare(
                    "SELECT id, user_did, monitoring_skill, body, monitoring_threshold, embedding_vector, created_at
                     FROM tasks WHERE user_did = ?1 AND trigger_type = 'semantic_alert' AND status = 'active'
                     ORDER BY id DESC"
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![user_did], |row| {
                        let emb_blob: Vec<u8> = row.get(5)?;
                        let embedding: Vec<f32> = emb_blob
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                            .collect();
                        Ok(MonitoringIntentRecord {
                            id: row.get(0)?,
                            user_did: row.get(1)?,
                            monitoring_skill: row.get::<_, String>(2)?,
                            description: row.get::<_, String>(3)?,
                            monitoring_threshold: row.get::<_, f64>(4)? as f32,
                            embedding,
                            created_at: row.get(6)?,
                        })
                    })?
                    .filter_map(row_ok)
                    .collect();
                Ok(rows)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::DeleteMonitoringIntent { intent_id, reply } => {
            let result = conn
                .execute(
                    "UPDATE tasks SET status = 'cancelled', updated_at = datetime('now') WHERE id = ?1 AND trigger_type = 'semantic_alert' AND status = 'active'",
                    rusqlite::params![intent_id],
                )
                .map(|_| ())
                .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::GetAllMonitoringIntents { reply } => {
            let result = (|| -> Result<Vec<MonitoringIntentRecord>> {
                let mut stmt = conn.prepare(
                    "SELECT id, user_did, monitoring_skill, body, monitoring_threshold, embedding_vector, created_at
                     FROM tasks WHERE trigger_type = 'semantic_alert' AND status = 'active' AND embedding_vector IS NOT NULL
                     ORDER BY id DESC"
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        let emb_blob: Vec<u8> = row.get(5)?;
                        let embedding: Vec<f32> = emb_blob
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                            .collect();
                        Ok(MonitoringIntentRecord {
                            id: row.get(0)?,
                            user_did: row.get(1)?,
                            monitoring_skill: row.get::<_, String>(2)?,
                            description: row.get::<_, String>(3)?,
                            monitoring_threshold: row.get::<_, f64>(4)? as f32,
                            embedding,
                            created_at: row.get(6)?,
                        })
                    })?
                    .filter_map(row_ok)
                    .collect();
                Ok(rows)
            })();
            mem_reply!(reply, result);
        }

        _ => {
            tracing::error!("BUG: non-monitoring command routed to handle_monitoring_cmd");
        }
    }
}

// ── Replay nonce + skill state handler ─────────────────────────────────────

pub(super) fn handle_replay_state_cmd(conn: &rusqlite::Connection, cmd: MemoryCommand) {
    match cmd {
        MemoryCommand::PersistReplayNonce {
            sig_hex,
            expires_at,
        } => {
            if let Err(e) = conn.execute(
                "INSERT OR IGNORE INTO replay_nonces (signature_hex, expires_at_unix) VALUES (?1, ?2)",
                rusqlite::params![sig_hex, expires_at],
            ) {
                tracing::warn!(error = %e, sig_hex = %sig_hex, "Failed to persist replay nonce — replay protection data lost");
            }
        }
        MemoryCommand::LoadReplayNonces {
            min_expires_at,
            reply,
        } => {
            let result = (|| -> Result<Vec<(String, u64)>> {
                let mut stmt = conn.prepare(
                    "SELECT signature_hex, expires_at_unix FROM replay_nonces WHERE expires_at_unix >= ?1"
                )?;
                let rows = stmt.query_map(rusqlite::params![min_expires_at], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
                })?;
                Ok(rows.filter_map(row_ok).collect())
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::DeleteExpiredReplayNonces { now_unix } => {
            if let Err(e) = conn.execute(
                "DELETE FROM replay_nonces WHERE expires_at_unix < ?1",
                rusqlite::params![now_unix],
            ) {
                tracing::warn!(error = %e, "Failed to delete expired replay nonces");
            }
        }
        MemoryCommand::StoreSkillState {
            skill_name,
            key,
            value,
            reply,
        } => {
            const MAX_VALUE_SIZE: usize = 64 * 1024;
            if value.len() > MAX_VALUE_SIZE {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!(
                        "skill state value too large ({} bytes, max {MAX_VALUE_SIZE})",
                        value.len()
                    ))
                );
                return;
            }
            const MAX_KEYS_PER_SKILL: i64 = 100;
            let existing: bool = conn
                .query_row(
                    "SELECT 1 FROM skill_state WHERE skill_name = ?1 AND state_key = ?2",
                    rusqlite::params![skill_name, key],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            if !existing {
                let key_count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM skill_state WHERE skill_name = ?1",
                        rusqlite::params![skill_name],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                if key_count >= MAX_KEYS_PER_SKILL {
                    mem_reply!(
                        reply,
                        Err(anyhow::anyhow!(
                            "skill '{skill_name}' has reached the maximum of {MAX_KEYS_PER_SKILL} state keys"
                        ))
                    );
                    return;
                }
            }
            let now = Utc::now().to_rfc3339();
            let result = conn.execute(
                "INSERT OR REPLACE INTO skill_state (skill_name, state_key, state_value, updated_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![skill_name, key, value, now],
            ).map(|_| ()).map_err(|e| anyhow::anyhow!("store_skill_state: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::LoadSkillState {
            skill_name,
            key,
            reply,
        } => {
            let result = conn.query_row(
                "SELECT state_value FROM skill_state WHERE skill_name = ?1 AND state_key = ?2",
                rusqlite::params![skill_name, key],
                |row| row.get::<_, String>(0),
            );
            match result {
                Ok(val) => {
                    mem_reply!(reply, Ok(Some(val)));
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    mem_reply!(reply, Ok(None));
                }
                Err(e) => {
                    mem_reply!(reply, Err(anyhow::anyhow!("load_skill_state: {e}")));
                }
            }
        }
        MemoryCommand::DeleteSkillState {
            skill_name,
            key,
            reply,
        } => {
            let result = conn
                .execute(
                    "DELETE FROM skill_state WHERE skill_name = ?1 AND state_key = ?2",
                    rusqlite::params![skill_name, key],
                )
                .map(|_| ())
                .map_err(|e| anyhow::anyhow!("delete_skill_state: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::ListSkillStateKeys { skill_name, reply } => {
            let result = (|| -> Result<Vec<(String, String)>> {
                let mut stmt = conn.prepare(
                    "SELECT state_key, state_value FROM skill_state WHERE skill_name = ?1 AND state_value != ''",
                )?;
                let rows = stmt.query_map(rusqlite::params![skill_name], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                Ok(rows.filter_map(row_ok).collect())
            })();
            mem_reply!(reply, result);
        }

        MemoryCommand::StoreHitlApproval {
            key,
            value,
            kind,
            proposed_at_audit_cycle_id,
            embedding_json,
            expires_at,
            reply,
        } => {
            // Mirror the store_skill_state max-size / max-keys guards so
            // heartbeat proposals cannot bypass the normal K/V hygiene.
            const MAX_VALUE_SIZE: usize = 64 * 1024;
            if value.len() > MAX_VALUE_SIZE {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!(
                        "hitl approval value too large ({} bytes, max {MAX_VALUE_SIZE})",
                        value.len()
                    ))
                );
                return;
            }
            const MAX_KEYS_PER_SKILL: i64 = 100;
            let existing: bool = conn
                .query_row(
                    "SELECT 1 FROM skill_state WHERE skill_name = 'hitl' AND state_key = ?1",
                    rusqlite::params![key],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            if !existing {
                let key_count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM skill_state WHERE skill_name = 'hitl'",
                        rusqlite::params![],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                if key_count >= MAX_KEYS_PER_SKILL {
                    mem_reply!(
                        reply,
                        Err(anyhow::anyhow!(
                            "hitl queue has reached the maximum of {MAX_KEYS_PER_SKILL} pending approvals"
                        ))
                    );
                    return;
                }
            }
            let now = Utc::now().to_rfc3339();
            let result = conn
                .execute(
                    "INSERT OR REPLACE INTO skill_state \
                   (skill_name, state_key, state_value, updated_at, \
                    kind, proposed_at_audit_cycle_id, embedding_json, expires_at) \
                 VALUES ('hitl', ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        key,
                        value,
                        now,
                        kind,
                        proposed_at_audit_cycle_id,
                        embedding_json,
                        expires_at,
                    ],
                )
                .map(|_| ())
                .map_err(|e| anyhow::anyhow!("store_hitl_approval: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::SweepExpiredHitl { now_rfc3339, reply } => {
            // Delete only rows with a real expiry (skip sentinel). Scope to
            // skill_name='hitl' so generic skill_state rows — which also bear
            // the sentinel by default — are never touched even if some
            // future migration narrows the sentinel.
            const SENTINEL: &str = "9999-12-31T00:00:00Z";
            let result = conn
                .execute(
                    "DELETE FROM skill_state \
                     WHERE skill_name = 'hitl' \
                       AND expires_at < ?1 \
                       AND expires_at != ?2",
                    rusqlite::params![now_rfc3339, SENTINEL],
                )
                .map(|n| n as u64)
                .map_err(|e| anyhow::anyhow!("sweep_expired_hitl: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::ListPendingHitlWithEmbeddings {
            kind,
            since_rfc3339,
            reply,
        } => {
            let result = (|| -> Result<Vec<(String, String)>> {
                let mut stmt = conn.prepare(
                    "SELECT state_key, embedding_json FROM skill_state \
                     WHERE skill_name = 'hitl' \
                       AND kind = ?1 \
                       AND state_value != '' \
                       AND embedding_json IS NOT NULL \
                       AND updated_at >= ?2",
                )?;
                let rows = stmt.query_map(rusqlite::params![kind, since_rfc3339], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                Ok(rows.filter_map(row_ok).collect())
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::CountHitlPendingByKind { kind, reply } => {
            let result = conn
                .query_row(
                    "SELECT COUNT(*) FROM skill_state \
                     WHERE skill_name = 'hitl' \
                       AND kind = ?1 \
                       AND state_value != ''",
                    rusqlite::params![kind],
                    |row| row.get::<_, i64>(0),
                )
                .map(|n| n.max(0) as u32)
                .map_err(|e| anyhow::anyhow!("count_hitl_pending_by_kind: {e}"));
            mem_reply!(reply, result);
        }

        MemoryCommand::CountHitlPendingUpdatedSince {
            since_rfc3339,
            reply,
        } => {
            let result = conn
                .query_row(
                    "SELECT COUNT(*) FROM skill_state \
                     WHERE skill_name = 'hitl' \
                       AND state_value != '' \
                       AND updated_at >= ?1",
                    rusqlite::params![since_rfc3339],
                    |row| row.get::<_, i64>(0),
                )
                .map(|n| n.max(0) as u32)
                .map_err(|e| anyhow::anyhow!("count_hitl_pending_updated_since: {e}"));
            mem_reply!(reply, result);
        }

        _ => {
            tracing::error!("BUG: non-replay-state command routed to handle_replay_state_cmd");
        }
    }
}

// ── Context / L2 topic / fact aging / command usage handler ────────────────

pub(super) fn handle_context_cmd(
    conn: &rusqlite::Connection,
    cmd: MemoryCommand,
    _db_path_str: &str,
    _max_db_size: u64,
    _size_warning_issued: &mut bool,
) {
    match cmd {
        MemoryCommand::QueryL2ByTopic { did, topic, reply } => {
            let results = (|| -> Vec<(String, Option<Vec<f32>>)> {
                let mut stmt = match conn.prepare(
                    "SELECT fact_summary, embedding FROM l2_experience
                     WHERE agent_did = ?1 AND topic = ?2
                     AND (expires_at IS NULL OR expires_at > datetime('now'))
                     LIMIT 100",
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoryActor] query failed");
                        return Vec::new();
                    }
                };
                let rows = match stmt.query_map(rusqlite::params![did, topic], |row| {
                    let fact: String = row.get(0)?;
                    let blob: Option<Vec<u8>> = row.get(1)?;
                    Ok((fact, blob))
                }) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoryActor] query failed");
                        return Vec::new();
                    }
                };
                rows.filter_map(row_ok)
                    .map(|(fact, blob)| {
                        let emb = blob.map(|b| EmbeddingClient::from_blob(&b));
                        (fact, emb)
                    })
                    .collect()
            })();
            mem_reply!(reply, results);
        }
        MemoryCommand::DeleteL2ByTopic { did, topic, reply } => {
            let result = (|| -> Result<(u64, Vec<i64>)> {
                // Collect IDs before deleting so caller can mark HNSW entries
                let ids: Vec<i64> = conn
                    .prepare("SELECT id FROM l2_experience WHERE agent_did = ?1 AND topic = ?2")?
                    .query_map(rusqlite::params![&did, &topic], |row| row.get(0))?
                    .filter_map(row_ok)
                    .collect();
                let count = conn.execute(
                    "DELETE FROM l2_experience WHERE agent_did = ?1 AND topic = ?2",
                    rusqlite::params![did, topic],
                )?;
                // Cascade: clean up orphaned recall stats
                if !ids.is_empty() {
                    let ph: Vec<String> = (0..ids.len()).map(|i| format!("?{}", i + 1)).collect();
                    let sql = format!(
                        "DELETE FROM l2_recall_stats WHERE fact_id IN ({})",
                        ph.join(",")
                    );
                    if let Ok(mut stmt) = conn.prepare(&sql) {
                        let p: Vec<Box<dyn rusqlite::ToSql>> =
                            ids.iter().map(|id| Box::new(*id) as _).collect();
                        let pr: Vec<&dyn rusqlite::ToSql> = p.iter().map(|x| x.as_ref()).collect();
                        if let Err(e) = stmt.execute(pr.as_slice()) {
                            tracing::debug!(error = %e, "cascade delete recall_stats by topic failed");
                        }
                    }
                }
                Ok((count as u64, ids))
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::StoreTurnEvidence {
            user_did,
            reply_key,
            evidence_json,
            unsupported_json,
        } => {
            let created_at = Utc::now().timestamp();
            if let Err(e) = conn.execute(
                "INSERT INTO webapp_turn_evidence (user_did, reply_key, created_at, evidence_json, unsupported_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![user_did, reply_key, created_at, evidence_json, unsupported_json],
            ) {
                tracing::warn!(error = %e, "StoreTurnEvidence insert failed");
            } else if let Err(e) = conn.execute(
                "DELETE FROM webapp_turn_evidence WHERE user_did=?1 AND id NOT IN (SELECT id FROM webapp_turn_evidence WHERE user_did=?1 ORDER BY id DESC LIMIT 200)",
                rusqlite::params![user_did],
            ) {
                tracing::warn!(error = %e, "StoreTurnEvidence cap delete failed");
            }
        }
        MemoryCommand::StoreSummary {
            did,
            summary,
            facts,
        } => {
            // Route ALL StoreSummary output to Tier 2 (l2_ephemeral).
            // StoreSummary produces compaction artifacts, not curated learned facts.
            // Curated facts live under the `learned:*` topics.

            // Main summary → context_summary source, TTL=60 days
            // Apply write guard to prevent duplicate summaries from tight compaction loops.
            let summary_decision = l2_write_guard(
                conn,
                &did,
                "context_summary",
                &summary,
                None,
                MemoryTier::Ephemeral,
            );
            if summary_decision != WriteDecision::Skip
                && let Err(e) = conn.execute(
                    "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, source, created_at, expires_at)
                     VALUES (?1, 'context_summary', ?2, 'context_summary', datetime('now'), datetime('now', '+60 days'))",
                    rusqlite::params![did, summary],
                )
            {
                tracing::warn!(error = %e, "StoreSummary context insert failed");
            }

            // Individual extracted facts → compaction_summary source, TTL=60 days
            for (topic, fact) in facts {
                // Apply write guard to prevent duplicate compaction facts
                let decision = l2_write_guard(
                    conn,
                    &did,
                    "compaction_summary",
                    &fact,
                    None,
                    MemoryTier::Ephemeral,
                );
                if decision == WriteDecision::Skip {
                    continue;
                }
                if let Err(e) = conn.execute(
                    "INSERT INTO l2_ephemeral (agent_did, topic, fact_summary, source, created_at, expires_at)
                     VALUES (?1, ?2, ?3, 'compaction_summary', datetime('now'), datetime('now', '+60 days'))",
                    rusqlite::params![did, topic, fact],
                ) {
                    tracing::warn!(error = %e, "StoreSummary fact insert failed");
                }
            }
        }
        MemoryCommand::TouchL2Facts { ids } => {
            touch_l2_last_accessed(conn, &ids);
        }
        MemoryCommand::TrackCommandUsage { command, user_id } => {
            if let Err(e) = conn.execute(
                "INSERT INTO command_usage (command, user_id, total_count, last_used)
                 VALUES (?1, ?2, 1, datetime('now'))
                 ON CONFLICT(command, user_id) DO UPDATE SET
                   total_count = total_count + 1,
                   last_used = datetime('now')",
                rusqlite::params![command, user_id],
            ) {
                tracing::warn!(command = %command, error = %e, "Failed to track command usage");
            }
            if rand::random::<u8>() < PROBABILISTIC_CLEANUP_THRESHOLD
                && let Err(e) = conn.execute(
                    "DELETE FROM command_usage WHERE last_used < datetime('now', '-90 days')",
                    [],
                )
            {
                tracing::warn!(error = %e, "Failed to clean stale command usage");
            }
        }
        MemoryCommand::GetCommandUsageStats { user_id, reply } => {
            let result = (|| -> Option<Vec<(String, u64)>> {
                let mut stmt = conn
                    .prepare(
                        "SELECT command, total_count FROM command_usage
                     WHERE user_id = ?1
                     ORDER BY total_count DESC",
                    )
                    .ok()?;
                let rows = stmt
                    .query_map(rusqlite::params![user_id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
                    })
                    .ok()?;
                Some(rows.filter_map(row_ok).collect())
            })()
            .unwrap_or_default();
            mem_reply!(reply, result);
        }
        MemoryCommand::QueryStaleFacts {
            did,
            days_threshold,
            reply,
        } => {
            let result = (|| -> Result<Vec<StaleFact>> {
                let threshold_ts =
                    (Utc::now() - chrono::Duration::days(days_threshold)).to_rfc3339();
                let mut stmt = conn.prepare(
                    "SELECT id, topic, fact_summary, created_at, last_accessed, is_pinned,
                            CAST((julianday('now') - julianday(COALESCE(last_accessed, created_at))) AS INTEGER) AS days_stale
                     FROM l2_experience
                     WHERE agent_did = ?1
                       AND COALESCE(last_accessed, created_at) < ?2
                     ORDER BY last_accessed ASC
                     LIMIT 50",
                )?;
                let rows = stmt.query_map(rusqlite::params![did, threshold_ts], |row| {
                    Ok(StaleFact {
                        id: row.get(0)?,
                        topic: row.get(1)?,
                        fact_summary: row.get(2)?,
                        created_at: row.get(3)?,
                        last_accessed: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        is_pinned: row.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
                        days_stale: row.get(6)?,
                    })
                })?;
                Ok(rows.filter_map(row_ok).collect())
            })();
            mem_reply!(reply, result);
        }

        #[cfg(test)]
        MemoryCommand::TestBackdateL2LastAccessed {
            id,
            days_ago,
            reply,
        } => {
            let result = (|| -> Result<()> {
                let ts = (Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339();
                conn.execute(
                    "UPDATE l2_experience SET last_accessed = ?1, created_at = ?1 WHERE id = ?2",
                    rusqlite::params![ts, id],
                )?;
                Ok(())
            })();
            mem_reply!(reply, result);
        }

        _ => {
            tracing::error!("BUG: non-context command routed to handle_context_cmd");
        }
    }
}

// ── Analytics / review / heartbeat audit / credit handler ──────────────────

pub(super) fn read_assistant_event_cursor(
    conn: &rusqlite::Connection,
    user_did: &str,
    consumer: &str,
) -> Result<i64> {
    Ok(conn
        .query_row(
            "SELECT last_event_id FROM assistant_event_cursor \
             WHERE user_did = ?1 AND consumer = ?2",
            rusqlite::params![user_did, consumer],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

pub(super) fn write_assistant_event_cursor(
    conn: &rusqlite::Connection,
    user_did: &str,
    consumer: &str,
    last_event_id: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO assistant_event_cursor (user_did, consumer, last_event_id, updated_at) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(user_did, consumer) DO UPDATE SET \
             last_event_id = excluded.last_event_id, \
             updated_at = excluded.updated_at",
        rusqlite::params![
            user_did,
            consumer,
            last_event_id,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

pub(super) fn parse_assistant_event_row(
    row: &rusqlite::Row,
) -> rusqlite::Result<crate::assistant_events::StoredAssistantEvent> {
    Ok(crate::assistant_events::StoredAssistantEvent {
        id: row.get(0)?,
        event: crate::assistant_events::AssistantEvent {
            raw_event_id: row.get(1)?,
            observed_at: row.get(2)?,
            occurred_at: row.get(3)?,
            user_did: row.get(4)?,
            source: match row.get::<_, String>(5)?.as_str() {
                "calendar" => crate::assistant_events::AssistantEventSource::Calendar,
                "drive" => crate::assistant_events::AssistantEventSource::Drive,
                "chronos" => crate::assistant_events::AssistantEventSource::Chronos,
                "telegram" => crate::assistant_events::AssistantEventSource::Telegram,
                "browser" => crate::assistant_events::AssistantEventSource::Browser,
                // Fail-safe: unknown source (version skew) → Unknown, never error.
                // assistant_event_enters_conversation treats Unknown source as
                // conversation-visible so events from a newer writer are never lost.
                _ => crate::assistant_events::AssistantEventSource::Unknown,
            },
            kind: match row.get::<_, String>(6)?.as_str() {
                "email_received" => crate::assistant_events::AssistantEventKind::EmailReceived,
                "email_thread_updated" => {
                    crate::assistant_events::AssistantEventKind::EmailThreadUpdated
                }
                "calendar_invite_received" => {
                    crate::assistant_events::AssistantEventKind::CalendarInviteReceived
                }
                "calendar_event_changed" => {
                    crate::assistant_events::AssistantEventKind::CalendarEventChanged
                }
                "drive_file_uploaded" => {
                    crate::assistant_events::AssistantEventKind::DriveFileUploaded
                }
                "drive_file_shared" => crate::assistant_events::AssistantEventKind::DriveFileShared,
                "drive_comment_added" => {
                    crate::assistant_events::AssistantEventKind::DriveCommentAdded
                }
                // Fail-safe: unknown kind → Unknown, never error.
                _ => crate::assistant_events::AssistantEventKind::Unknown,
            },
            account_id: row.get(7)?,
            title: row.get(8)?,
            summary: row.get(9)?,
            thread_key: row.get(10)?,
            urgency_hint: match row.get::<_, String>(11)?.as_str() {
                "low" => crate::assistant_events::EventUrgencyHint::Low,
                "medium" => crate::assistant_events::EventUrgencyHint::Medium,
                "high" => crate::assistant_events::EventUrgencyHint::High,
                // Fail-safe: unknown urgency → Unknown (not High), falls through
                // to decision/source checks in assistant_event_enters_conversation.
                _ => crate::assistant_events::EventUrgencyHint::Unknown,
            },
            actionability_hint: match row.get::<_, String>(12)?.as_str() {
                "passive" => crate::assistant_events::EventActionabilityHint::Passive,
                "review" => crate::assistant_events::EventActionabilityHint::Review,
                "act" => crate::assistant_events::EventActionabilityHint::Act,
                // Fail-safe: unknown actionability → Unknown (not Act), falls
                // through to decision/source checks.
                _ => crate::assistant_events::EventActionabilityHint::Unknown,
            },
            decision: match row.get::<_, String>(13)?.as_str() {
                "notify_now" => crate::assistant_events::AssistantEventDecision::NotifyNow,
                "callback_now" => crate::assistant_events::AssistantEventDecision::CallbackNow,
                "digest_later" => crate::assistant_events::AssistantEventDecision::DigestLater,
                "taskify" => crate::assistant_events::AssistantEventDecision::Taskify,
                "memory_only" => crate::assistant_events::AssistantEventDecision::MemoryOnly,
                "ignore" => crate::assistant_events::AssistantEventDecision::Ignore,
                // Fail-safe: unknown decision → Unknown (not NotifyNow) →
                // assistant_event_enters_conversation returns true (conversation-visible).
                // The cursor advances past this row; the notifier is NEVER stalled.
                _ => crate::assistant_events::AssistantEventDecision::Unknown,
            },
            dedup_key: row.get(14)?,
            entities: serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(15)?)
                .unwrap_or(serde_json::Value::Null),
        },
    })
}

pub(super) fn parse_assistant_work_item_row(
    row: &rusqlite::Row,
) -> rusqlite::Result<crate::assistant_work_items::StoredAssistantWorkItem> {
    let source_kind = row.get::<_, String>(4)?;
    let mode = row.get::<_, String>(10)?;
    let priority = row.get::<_, String>(11)?;
    let status = row.get::<_, String>(12)?;
    let delivery_policy = row.get::<_, String>(13)?;
    let caller_context = row.get::<_, String>(14)?;

    Ok(crate::assistant_work_items::StoredAssistantWorkItem {
        id: row.get(0)?,
        item: crate::assistant_work_items::AssistantWorkItem {
            created_at: row.get(1)?,
            updated_at: row.get(2)?,
            user_did: row.get(3)?,
            source_kind: crate::assistant_work_items::AssistantWorkSourceKind::from_str(
                &source_kind,
            )
            .ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::other(format!(
                        "unknown assistant work source kind: {source_kind}"
                    ))),
                )
            })?,
            source_ref: row.get(5)?,
            anchor_kind: row
                .get::<_, Option<String>>(6)?
                .as_deref()
                .and_then(crate::assistant_work_items::AssistantWorkAnchorKind::from_str),
            anchor_ref: row.get(7)?,
            assistant_event_id: row.get(8)?,
            goal: row.get(9)?,
            mode: crate::assistant_work_items::AssistantWorkMode::from_str(&mode).ok_or_else(
                || {
                    rusqlite::Error::FromSqlConversionFailure(
                        10,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::other(format!(
                            "unknown assistant work mode: {mode}"
                        ))),
                    )
                },
            )?,
            priority: crate::assistant_work_items::AssistantWorkPriority::from_str(&priority)
                .ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        11,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::other(format!(
                            "unknown assistant work priority: {priority}"
                        ))),
                    )
                })?,
            status: crate::assistant_work_items::AssistantWorkStatus::from_str(&status)
                .ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        12,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::other(format!(
                            "unknown assistant work status: {status}"
                        ))),
                    )
                })?,
            delivery_policy: crate::assistant_work_items::AssistantWorkDeliveryPolicy::from_str(
                &delivery_policy,
            )
            .ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    13,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::other(format!(
                        "unknown assistant work delivery policy: {delivery_policy}"
                    ))),
                )
            })?,
            caller_context: crate::assistant_work_items::AssistantWorkCallerContext::from_str(
                &caller_context,
            )
            .ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    14,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::other(format!(
                        "unknown assistant work caller context: {caller_context}"
                    ))),
                )
            })?,
            dedup_key: row.get(15)?,
            budget_profile: serde_json::from_str(&row.get::<_, String>(16)?).unwrap_or(
                crate::assistant_work_items::AssistantWorkBudgetProfile::bounded_callback_review(),
            ),
            context: serde_json::from_str(&row.get::<_, String>(17)?)
                .unwrap_or(serde_json::Value::Null),
            result: row
                .get::<_, Option<String>>(18)?
                .and_then(|json| serde_json::from_str(&json).ok()),
        },
    })
}

pub(super) fn query_assistant_work_items(
    conn: &rusqlite::Connection,
    user_did: &str,
    statuses: Option<&[String]>,
    source_kinds: Option<&[String]>,
    modes: Option<&[String]>,
    caller_contexts: Option<&[String]>,
    limit: usize,
) -> Result<Vec<crate::assistant_work_items::StoredAssistantWorkItem>> {
    let mut sql = String::from(
        "SELECT id, created_at, updated_at, user_did, source_kind, source_ref, anchor_kind, anchor_ref, assistant_event_id, \
                goal, mode, priority, status, delivery_policy, caller_context, dedup_key, \
                budget_json, context_json, result_json \
         FROM assistant_work_items WHERE user_did = ?1",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(user_did.to_string())];
    if let Some(statuses) = statuses
        && !statuses.is_empty()
    {
        let placeholders = (0..statuses.len())
            .map(|idx| format!("?{}", idx + 2))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND status IN ({placeholders})"));
        for status in statuses {
            params.push(Box::new(status.clone()));
        }
    }
    if let Some(source_kinds) = source_kinds
        && !source_kinds.is_empty()
    {
        let placeholders = (0..source_kinds.len())
            .map(|idx| format!("?{}", params.len() + idx + 1))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND source_kind IN ({placeholders})"));
        for source_kind in source_kinds {
            params.push(Box::new(source_kind.clone()));
        }
    }
    if let Some(modes) = modes
        && !modes.is_empty()
    {
        let placeholders = (0..modes.len())
            .map(|idx| format!("?{}", params.len() + idx + 1))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND mode IN ({placeholders})"));
        for mode in modes {
            params.push(Box::new(mode.clone()));
        }
    }
    if let Some(caller_contexts) = caller_contexts
        && !caller_contexts.is_empty()
    {
        let placeholders = (0..caller_contexts.len())
            .map(|idx| format!("?{}", params.len() + idx + 1))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND caller_context IN ({placeholders})"));
        for caller_context in caller_contexts {
            params.push(Box::new(caller_context.clone()));
        }
    }
    let limit_param = params.len() + 1;
    sql.push_str(&format!(
        " ORDER BY CASE priority \
              WHEN 'interrupt_now' THEN 0 \
              WHEN 'soon' THEN 1 \
              ELSE 2 \
          END ASC, id ASC LIMIT ?{limit_param}"
    ));
    params.push(Box::new(limit as i64));
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), parse_assistant_work_item_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(super) fn query_new_assistant_events_for_consumer(
    conn: &rusqlite::Connection,
    user_did: &str,
    consumer: &str,
    limit: usize,
    decisions: Option<&[String]>,
    exclude_decisions: Option<&[String]>,
) -> Result<(Vec<crate::assistant_events::StoredAssistantEvent>, i64)> {
    let last_seen_id = read_assistant_event_cursor(conn, user_did, consumer)?;
    let mut sql = String::from(
        "SELECT id, raw_event_id, observed_at, occurred_at, user_did, source, kind, account_id, \
                title, summary, thread_key, urgency_hint, actionability_hint, decision, dedup_key, entities_json \
         FROM assistant_events \
         WHERE user_did = ?1 AND id > ?2",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> =
        vec![Box::new(user_did.to_string()), Box::new(last_seen_id)];
    if let Some(decisions) = decisions
        && !decisions.is_empty()
    {
        let placeholders = (0..decisions.len())
            .map(|idx| format!("?{}", idx + 3))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND decision IN ({placeholders})"));
        for decision in decisions {
            params.push(Box::new(decision.clone()));
        }
    }
    if let Some(exclude) = exclude_decisions
        && !exclude.is_empty()
    {
        let start = params.len() + 1;
        let placeholders = (0..exclude.len())
            .map(|idx| format!("?{}", idx + start))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND decision NOT IN ({placeholders})"));
        for decision in exclude {
            params.push(Box::new(decision.clone()));
        }
    }
    let limit_param = params.len() + 1;
    sql.push_str(&format!(" ORDER BY id ASC LIMIT ?{limit_param}"));
    params.push(Box::new(limit as i64));
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), parse_assistant_event_row)?;
    let mut out = Vec::new();
    let mut newest_id = last_seen_id;
    for row in rows {
        let row = row?;
        newest_id = newest_id.max(row.id);
        out.push(row);
    }
    Ok((out, newest_id))
}

pub(super) fn handle_analytics_cmd(conn: &rusqlite::Connection, cmd: MemoryCommand) {
    const AUDIT_SELECT_COLS: &str = "id, cycle_id, skill_name, timestamp, severity, probe_payload, \
         tools_called, tools_blocked, actions_taken, llm_reply, tokens_used, \
         rounds_used, user_did, chat_id, raw_process_log, tool_call_details, \
         outcome, skip_reason, config_snapshot_json, prompt_hash, \
         proposals_created_json, next_tick_note_bytes";

    match cmd {
        MemoryCommand::SnapshotDailyMetrics => {
            let today = Utc::now().format("%Y-%m-%d").to_string();
            let result = conn.execute(
                "INSERT OR REPLACE INTO skill_metrics_daily (skill_name, date, total_calls, error_count, avg_latency_ms) \
                 SELECT skill_name, ?1, total_calls, error_count, avg_latency_ms \
                 FROM skill_metrics",
                rusqlite::params![today],
            );
            if let Err(e) = result {
                tracing::debug!(error = %e, "Failed to snapshot daily metrics");
            }
        }
        MemoryCommand::GetDailyMetricsTrend { skill_name, days, reply } => {
            let since = (Utc::now() - chrono::Duration::days(days)).format("%Y-%m-%d").to_string();
            let result = (|| {
                let mut stmt = conn.prepare(
                    "SELECT date, total_calls, error_count, avg_latency_ms \
                     FROM skill_metrics_daily \
                     WHERE skill_name = ?1 AND date >= ?2 \
                     ORDER BY date ASC"
                )?;
                let rows = stmt.query_map(rusqlite::params![skill_name, since], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)? as u64,
                        row.get::<_, i64>(2)? as u64,
                        row.get::<_, f64>(3)?,
                    ))
                })?;
                Ok::<_, rusqlite::Error>(rows.filter_map(row_ok).collect::<Vec<_>>())
            })();
            let val = match result {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("Query failed in GetDailyMetricsTrend: {}", e);
                    Vec::new()
                }
            };
            mem_reply!(reply, val);
        }
        MemoryCommand::StoreReviewRecord {
            layer, passed, tool_matched, similarity,
            reply_preview, tools_called, latency_ms,
        } => {
            if let Err(e) = conn.execute(
                "INSERT INTO review_log (layer, passed, tool_matched, similarity, reply_preview, tools_called, latency_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    layer, passed as i32, tool_matched, similarity,
                    reply_preview, tools_called, latency_ms as i64,
                ],
            ) {
                tracing::warn!(layer = %layer, error = %e, "Failed to store review record");
            }
        }
        MemoryCommand::GetReviewStats { since_hours, reply } => {
            let stats = {
                let cutoff_ts = (Utc::now() - chrono::Duration::hours(since_hours as i64))
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string();
                let sql =
                    "SELECT
                        COUNT(*) as total,
                        SUM(CASE WHEN layer = 'fast_pass' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN layer = 'no_completion_signal' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN layer = 'semantic_pass' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN layer = 'semantic_flag' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN layer = 'llm_review' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN passed = 0 THEN 1 ELSE 0 END),
                        COALESCE(AVG(latency_ms), 0)
                     FROM review_log
                     WHERE timestamp >= ?1";
                conn.query_row(sql, rusqlite::params![cutoff_ts], |row| {
                    Ok(crate::evaluators::ReviewStats {
                        total: row.get::<_, i64>(0).unwrap_or(0) as u64,
                        fast_pass: row.get::<_, i64>(1).unwrap_or(0) as u64,
                        no_completion_signal: row.get::<_, i64>(2).unwrap_or(0) as u64,
                        semantic_pass: row.get::<_, i64>(3).unwrap_or(0) as u64,
                        semantic_flag: row.get::<_, i64>(4).unwrap_or(0) as u64,
                        llm_review: row.get::<_, i64>(5).unwrap_or(0) as u64,
                        flagged: row.get::<_, i64>(6).unwrap_or(0) as u64,
                        avg_latency_ms: row.get::<_, f64>(7).unwrap_or(0.0),
                    })
                }).unwrap_or_default()
            };
            mem_reply!(reply, stats);
        }
        MemoryCommand::GetRecentFlaggedReviews { limit, reply } => {
            let results = (|| -> Vec<(String, String, String, Option<String>, Option<f64>, i64)> {
                let mut stmt = match conn.prepare(
                    "SELECT timestamp, layer, reply_preview, tool_matched, similarity, latency_ms
                     FROM review_log WHERE passed = 0
                     ORDER BY timestamp DESC LIMIT ?1"
                ) {
                    Ok(s) => s,
                    Err(e) => { tracing::warn!(error = %e, "[MemoryActor] query failed"); return Vec::new(); }
                };
                let rows = match stmt.query_map(rusqlite::params![limit], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2).unwrap_or_default(),
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<f64>>(4)?,
                        row.get::<_, i64>(5).unwrap_or(0),
                    ))
                }) {
                    Ok(r) => r,
                    Err(e) => { tracing::warn!(error = %e, "[MemoryActor] query failed"); return Vec::new(); }
                };
                rows.filter_map(row_ok).collect()
            })();
            mem_reply!(reply, results);
        }
        MemoryCommand::StoreHeartbeatAudit {
            cycle_id, skill_name, severity, probe_payload,
            tools_called, tools_blocked, actions_taken,
            llm_reply, tokens_used, rounds_used,
            user_did, chat_id, config_snapshot, raw_process_log,
            tool_call_details, outcome, skip_reason, config_snapshot_json,
            prompt_hash, proposals_created_json, next_tick_note_bytes, reply,
        } => {
            let result = conn.execute(
                "INSERT INTO heartbeat_audit_log (cycle_id, skill_name, severity, probe_payload, \
                    tools_called, tools_blocked, actions_taken, llm_reply, tokens_used, rounds_used, \
                    user_did, chat_id, config_snapshot, raw_process_log, tool_call_details, \
                    outcome, skip_reason, config_snapshot_json, prompt_hash, \
                    proposals_created_json, next_tick_note_bytes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, \
                    ?16, ?17, ?18, ?19, ?20, ?21)",
                rusqlite::params![cycle_id, skill_name, severity, probe_payload, tools_called,
                    tools_blocked, actions_taken, llm_reply, tokens_used, rounds_used,
                    user_did, chat_id, config_snapshot, raw_process_log, tool_call_details,
                    outcome, skip_reason, config_snapshot_json, prompt_hash,
                    proposals_created_json, next_tick_note_bytes],
            ).map(|_| ()).map_err(|e| anyhow::anyhow!("store_heartbeat_audit: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::SubcallLogInsert {
            id, parent_session_id, origin, task_preview, allowed_tools_json,
            status, rounds_used, tool_calls_made, tokens_used, duration_ms,
            created_at, parent_subcall_id, trace_available, summary, reply,
        } => {
            let trace_flag: i64 = if trace_available { 1 } else { 0 };
            let result = conn.execute(
                "INSERT INTO subcall_log (id, parent_session_id, origin, task_preview, \
                    allowed_tools, status, rounds_used, tool_calls_made, tokens_used, \
                    duration_ms, created_at, parent_subcall_id, trace_available, summary) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                rusqlite::params![
                    id, parent_session_id, origin, task_preview, allowed_tools_json,
                    status, rounds_used, tool_calls_made, tokens_used, duration_ms,
                    created_at, parent_subcall_id, trace_flag, summary
                ],
            ).map(|_| ()).map_err(|e| anyhow::anyhow!("subcall_log_insert: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::SubcallTraceInsert {
            subcall_id, system_prompt, messages_json, tool_outputs_json,
            created_at, accessed_at, size_bytes, reply,
        } => {
            let result = conn.execute(
                "INSERT INTO subcall_trace (subcall_id, system_prompt, messages_json, \
                    tool_outputs_json, created_at, accessed_at, size_bytes) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![subcall_id, system_prompt, messages_json,
                    tool_outputs_json, created_at, accessed_at, size_bytes],
            ).map(|_| ()).map_err(|e| anyhow::anyhow!("subcall_trace_insert: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::SubcallFetchTrace { subcall_id, reply } => {
            let result = (|| -> Result<(bool, bool, Option<String>, Option<Vec<(String, String)>>)> {
                // 1) Read trace_available + summary from subcall_log.
                //    Distinguish NotFound (no row) from Evicted (row exists,
                //    trace_available=0) so the LLM can tell "I hallucinated
                //    this id" from "my real id expired".
                let log_row: Option<(i64, Option<String>)> = conn
                    .query_row(
                        "SELECT trace_available, summary FROM subcall_log WHERE id = ?1",
                        rusqlite::params![subcall_id],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)),
                    )
                    .ok();
                let (trace_available_flag, summary_opt) = match log_row {
                    Some((flag, s)) => (flag != 0, s),
                    None => return Ok((false, false, None, None)), // NotFound
                };
                if !trace_available_flag {
                    // Evicted: return summary so recall can surface it.
                    return Ok((true, false, summary_opt, None));
                }
                // 2) Read tool_outputs_json from subcall_trace and parse.
                let tool_outputs_json: Option<String> = conn
                    .query_row(
                        "SELECT tool_outputs_json FROM subcall_trace WHERE subcall_id = ?1",
                        rusqlite::params![subcall_id],
                        |r| r.get::<_, String>(0),
                    )
                    .ok();
                let tool_outputs = match tool_outputs_json {
                    Some(j) => serde_json::from_str::<Vec<(String, String)>>(&j).ok(),
                    None => None,
                };
                Ok((true, true, summary_opt, tool_outputs))
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::SubcallTouchAccessed { subcall_id, reply } => {
            let result = conn
                .execute(
                    "UPDATE subcall_trace SET accessed_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
                     WHERE subcall_id = ?1",
                    rusqlite::params![subcall_id],
                )
                .map(|_| ())
                .map_err(|e| anyhow::anyhow!("subcall_touch_accessed: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::SubcallTtlSweep { reply } => {
            // TTL sweep (design §7.5, Task 1.8). Deletes subcall_log rows
            // (and CASCADE-deletes subcall_trace — PRAGMA foreign_keys=ON
            // is set at connection open) older than 90 days whose trace
            // is either already evicted (LEFT JOIN miss) or untouched for
            // 30+ days. Uses SQLite's datetime() so both RFC3339 and
            // datetime('now') formats compare correctly — see CLAUDE.md
            // SQLite-datetime anti-pattern.
            let result = (|| -> Result<usize> {
                let n = conn.execute(
                    "DELETE FROM subcall_log \
                     WHERE id IN ( \
                         SELECT sl.id FROM subcall_log sl \
                         LEFT JOIN subcall_trace st ON st.subcall_id = sl.id \
                         WHERE datetime(sl.created_at) < datetime('now', '-90 days') \
                           AND (st.accessed_at IS NULL \
                                OR datetime(st.accessed_at) < datetime('now', '-30 days')) \
                     )",
                    [],
                )?;
                Ok(n)
            })()
            .map_err(|e: anyhow::Error| anyhow::anyhow!("subcall_ttl_sweep: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::SubcallEnforceLruCap {
            total_bytes_cap,
            reply,
        } => {
            // LRU soft-cap enforcement (design §7.5, Task 1.8). Single
            // transaction so a concurrent insert/recall cannot see the
            // flip-state half-way through: (a) read current total, (b)
            // select rows oldest-first until cumulative size reaches
            // the overflow, (c) delete those trace rows and flip
            // trace_available=0 on their subcall_log rows.
            let result = (|| -> Result<usize> {
                let tx = conn.unchecked_transaction()?;
                let total: i64 = tx
                    .query_row(
                        "SELECT COALESCE(SUM(size_bytes), 0) FROM subcall_trace",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                if total <= total_bytes_cap {
                    return Ok(0);
                }
                let overflow = total - total_bytes_cap;

                // Oldest-first iteration until we've reclaimed `overflow`
                // bytes. Collect the ids (Rust-side), then do the DELETE /
                // UPDATE in bulk so we never hold a statement + mutable
                // connection at the same time.
                let mut victims: Vec<String> = Vec::new();
                let mut freed: i64 = 0;
                {
                    let mut stmt = tx.prepare(
                        "SELECT subcall_id, size_bytes FROM subcall_trace \
                         ORDER BY accessed_at ASC",
                    )?;
                    let mut rows = stmt.query([])?;
                    while let Some(row) = rows.next()? {
                        let id: String = row.get(0)?;
                        let size: i64 = row.get(1)?;
                        victims.push(id);
                        freed += size.max(0);
                        if freed >= overflow {
                            break;
                        }
                    }
                }

                if victims.is_empty() {
                    return Ok(0);
                }

                // Bulk delete + flag flip. Use the same txn so either both
                // or neither land; recall can't observe "trace gone but
                // trace_available still 1".
                let mut evicted = 0usize;
                for id in &victims {
                    tx.execute(
                        "DELETE FROM subcall_trace WHERE subcall_id = ?1",
                        rusqlite::params![id],
                    )?;
                    tx.execute(
                        "UPDATE subcall_log SET trace_available = 0 WHERE id = ?1",
                        rusqlite::params![id],
                    )?;
                    evicted += 1;
                }
                tx.commit()?;
                Ok(evicted)
            })()
            .map_err(|e: anyhow::Error| anyhow::anyhow!("subcall_enforce_lru_cap: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::RecordObservedEvent { source, severity, summary, payload_json, reply } => {
            let result = (|| -> Result<i64> {
                let at = chrono::Utc::now().to_rfc3339();
                conn.execute(
                    "INSERT INTO observed_events (at, source, severity, summary, payload_json) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![at, source, severity, summary, payload_json],
                )?;
                Ok(conn.last_insert_rowid())
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::TakeUnconsumedObservedEvents { limit, reply } => {
            let result = (|| -> Result<Vec<ObservedEvent>> {
                // Transaction: select unconsumed ids, read rows, mark consumed.
                // Single txn so a concurrent tick can't see the same event twice.
                let tx = conn.unchecked_transaction()?;
                let mut ids: Vec<i64> = Vec::new();
                let mut events: Vec<ObservedEvent> = Vec::new();
                {
                    let mut stmt = tx.prepare(
                        "SELECT id, at, source, severity, summary, payload_json \
                         FROM observed_events \
                         WHERE consumed_at IS NULL \
                         ORDER BY id ASC LIMIT ?1",
                    )?;
                    let rows = stmt.query_map([limit as i64], |row| {
                        Ok(ObservedEvent {
                            id: row.get(0)?,
                            at: row.get(1)?,
                            source: row.get(2)?,
                            severity: row.get(3)?,
                            summary: row.get(4)?,
                            payload_json: row.get(5)?,
                        })
                    })?;
                    for r in rows {
                        let ev = r?;
                        ids.push(ev.id);
                        events.push(ev);
                    }
                }
                if !ids.is_empty() {
                    let now = chrono::Utc::now().to_rfc3339();
                    // Build `IN (?,?,...)` clause dynamically; ids from our own query.
                    let placeholders = vec!["?"; ids.len()].join(",");
                    let sql = format!(
                        "UPDATE observed_events SET consumed_at = ? WHERE id IN ({placeholders})"
                    );
                    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(ids.len() + 1);
                    params.push(Box::new(now));
                    for id in &ids {
                        params.push(Box::new(*id));
                    }
                    let param_refs: Vec<&dyn rusqlite::ToSql> =
                        params.iter().map(|p| p.as_ref()).collect();
                    tx.execute(&sql, param_refs.as_slice())?;
                }
                tx.commit()?;
                Ok(events)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::StoreRawExternalEvent { event, reply } => {
            let result = (|| -> Result<i64> {
                let payload_json =
                    serde_json::to_string(&event.payload).context("serializing raw external event payload")?;
                conn.execute(
                    "INSERT OR IGNORE INTO raw_external_events \
                     (observed_at, user_did, source, source_account_id, source_event_id, dedup_key, payload_json) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        &event.observed_at,
                        &event.user_did,
                        event.source.as_str(),
                        event.source_account_id.as_deref(),
                        event.source_event_id.as_deref(),
                        &event.dedup_key,
                        payload_json
                    ],
                )?;
                let row_id: i64 = conn.query_row(
                    "SELECT id FROM raw_external_events WHERE dedup_key = ?1",
                    rusqlite::params![&event.dedup_key],
                    |row| row.get(0),
                )?;
                Ok(row_id)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::StoreAssistantEvent { event, reply } => {
            let result = (|| -> Result<i64> {
                let entities_json =
                    serde_json::to_string(&event.entities).context("serializing assistant event entities")?;
                conn.execute(
                    "INSERT OR IGNORE INTO assistant_events \
                     (raw_event_id, observed_at, occurred_at, user_did, source, kind, account_id, \
                      title, summary, thread_key, urgency_hint, actionability_hint, decision, dedup_key, entities_json) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    rusqlite::params![
                        event.raw_event_id,
                        &event.observed_at,
                        &event.occurred_at,
                        &event.user_did,
                        event.source.as_str(),
                        event.kind.as_str(),
                        event.account_id.as_deref(),
                        &event.title,
                        &event.summary,
                        event.thread_key.as_deref(),
                        event.urgency_hint.as_str(),
                        event.actionability_hint.as_str(),
                        event.decision.as_str(),
                        &event.dedup_key,
                        entities_json
                    ],
                )?;
                let row_id: i64 = conn.query_row(
                    "SELECT id FROM assistant_events WHERE dedup_key = ?1",
                    rusqlite::params![&event.dedup_key],
                    |row| row.get(0),
                )?;
                Ok(row_id)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::StoreAssistantWorkItem { item, reply } => {
            let result = (|| -> Result<i64> {
                let budget_json = serde_json::to_string(&item.budget_profile)
                    .context("serializing assistant work budget")?;
                let context_json =
                    serde_json::to_string(&item.context).context("serializing assistant work context")?;
                let result_json = item
                    .result
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .context("serializing assistant work result")?;
                conn.execute(
                    "INSERT OR IGNORE INTO assistant_work_items \
                     (created_at, updated_at, user_did, source_kind, source_ref, anchor_kind, anchor_ref, assistant_event_id, \
                      goal, mode, priority, status, delivery_policy, caller_context, dedup_key, \
                      budget_json, context_json, result_json) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
                    rusqlite::params![
                        &item.created_at,
                        &item.updated_at,
                        &item.user_did,
                        item.source_kind.as_str(),
                        item.source_ref.as_deref(),
                        item.anchor_kind.map(|kind| kind.as_str()),
                        item.anchor_ref.as_deref(),
                        item.assistant_event_id,
                        &item.goal,
                        item.mode.as_str(),
                        item.priority.as_str(),
                        item.status.as_str(),
                        item.delivery_policy.as_str(),
                        item.caller_context.as_str(),
                        &item.dedup_key,
                        budget_json,
                        context_json,
                        result_json,
                    ],
                )?;
                let row_id: i64 = conn.query_row(
                    "SELECT id FROM assistant_work_items WHERE dedup_key = ?1",
                    rusqlite::params![&item.dedup_key],
                    |row| row.get(0),
                )?;
                Ok(row_id)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::ListAssistantWorkItems {
            user_did,
            statuses,
            source_kinds,
            modes,
            caller_contexts,
            limit,
            reply,
        } => {
            let result = query_assistant_work_items(
                conn,
                &user_did,
                statuses.as_deref(),
                source_kinds.as_deref(),
                modes.as_deref(),
                caller_contexts.as_deref(),
                limit,
            );
            mem_reply!(reply, result);
        }
        MemoryCommand::ClaimNextAssistantWorkItem {
            user_did,
            filter,
            reply,
        } => {
            let result = (|| -> Result<Option<crate::assistant_work_items::StoredAssistantWorkItem>> {
                let tx = conn.unchecked_transaction()?;
                let statuses = filter
                    .as_ref()
                    .and_then(|f| (!f.statuses.is_empty()).then_some(f.statuses.as_slice()))
                    .map(|items| items.to_vec())
                    .unwrap_or_else(|| vec![String::from("queued")]);
                let source_kinds = filter
                    .as_ref()
                    .and_then(|f| (!f.source_kinds.is_empty()).then_some(f.source_kinds.as_slice()))
                    .map(|items| items.to_vec());
                let modes = filter
                    .as_ref()
                    .and_then(|f| (!f.modes.is_empty()).then_some(f.modes.as_slice()))
                    .map(|items| items.to_vec());
                let caller_contexts = filter
                    .as_ref()
                    .and_then(|f| (!f.caller_contexts.is_empty()).then_some(f.caller_contexts.as_slice()))
                    .map(|items| items.to_vec());
                let mut rows = query_assistant_work_items(
                    &tx,
                    &user_did,
                    Some(&statuses),
                    source_kinds.as_deref(),
                    modes.as_deref(),
                    caller_contexts.as_deref(),
                    1,
                )?;
                let Some(mut row) = rows.pop() else {
                    tx.commit()?;
                    return Ok(None);
                };
                let now = chrono::Utc::now().to_rfc3339();
                tx.execute(
                    "UPDATE assistant_work_items SET status = 'running', updated_at = ?2 WHERE id = ?1",
                    rusqlite::params![row.id, &now],
                )?;
                row.item.status = crate::assistant_work_items::AssistantWorkStatus::Running;
                row.item.updated_at = now;
                tx.commit()?;
                Ok(Some(row))
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::UpdateAssistantWorkItem {
            id,
            status,
            result,
            reply,
        } => {
            let result = (|| -> Result<()> {
                let result_json = result
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .context("serializing assistant work result update")?;
                conn.execute(
                    "UPDATE assistant_work_items \
                     SET status = ?2, updated_at = ?3, result_json = ?4 WHERE id = ?1",
                    rusqlite::params![
                        id,
                        status.as_str(),
                        chrono::Utc::now().to_rfc3339(),
                        result_json
                    ],
                )?;
                Ok(())
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::ListRecentTurnEvidence {
            user_did,
            limit,
            reply,
        } => {
            let result = (|| -> Result<Vec<TurnEvidenceRecord>> {
                let mut stmt = conn.prepare(
                    "SELECT reply_key, created_at, evidence_json, unsupported_json \
                     FROM webapp_turn_evidence WHERE user_did = ?1 ORDER BY id DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(rusqlite::params![user_did, limit as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?;
                let mut out = Vec::new();
                for row in rows {
                    let (reply_key, created_at, evidence_json, unsupported_json) = row?;
                    let Ok(evidence) = serde_json::from_str(&evidence_json) else {
                        continue;
                    };
                    let Ok(unsupported_numbers) = serde_json::from_str(&unsupported_json) else {
                        continue;
                    };
                    out.push(TurnEvidenceRecord {
                        reply_key,
                        created_at,
                        evidence,
                        unsupported_numbers,
                    });
                }
                Ok(out)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::ListRecentAssistantEvents { user_did, limit, reply } => {
            let result = (|| -> Result<Vec<crate::assistant_events::StoredAssistantEvent>> {
                let mut stmt = conn.prepare(
                    "SELECT id, raw_event_id, observed_at, occurred_at, user_did, source, kind, account_id, \
                            title, summary, thread_key, urgency_hint, actionability_hint, decision, dedup_key, entities_json \
                     FROM assistant_events WHERE user_did = ?1 ORDER BY id DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![user_did, limit as i64],
                    parse_assistant_event_row,
                )?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row?);
                }
                Ok(out)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::GetAssistantEventById { id, reply } => {
            let result = (|| -> Result<Option<crate::assistant_events::StoredAssistantEvent>> {
                let mut stmt = conn.prepare(
                    "SELECT id, raw_event_id, observed_at, occurred_at, user_did, source, kind, account_id, \
                            title, summary, thread_key, urgency_hint, actionability_hint, decision, dedup_key, entities_json \
                     FROM assistant_events WHERE id = ?1",
                )?;
                let mut rows = stmt.query(rusqlite::params![id])?;
                let Some(row) = rows.next()? else {
                    return Ok(None);
                };
                Ok(Some(parse_assistant_event_row(row)?))
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::PeekNewAssistantEventsForConsumer {
            user_did,
            consumer,
            limit,
            decisions,
            exclude_decisions,
            reply,
        } => {
            let result = (|| -> Result<Vec<crate::assistant_events::StoredAssistantEvent>> {
                let (out, _) = query_new_assistant_events_for_consumer(
                    conn,
                    &user_did,
                    &consumer,
                    limit,
                    decisions.as_deref(),
                    exclude_decisions.as_deref(),
                )?;
                Ok(out)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::CommitAssistantEventCursor {
            user_did,
            consumer,
            last_event_id,
            reply,
        } => {
            let result = (|| -> Result<()> {
                let tx = conn.unchecked_transaction()?;
                let current_last_seen = read_assistant_event_cursor(&tx, &user_did, &consumer)?;
                if last_event_id > current_last_seen {
                    write_assistant_event_cursor(&tx, &user_did, &consumer, last_event_id)?;
                }
                tx.commit()?;
                Ok(())
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::QueryHeartbeatAudit { skill_name, limit, reply } => {
            let result = (|| -> Result<Vec<HeartbeatAuditEntry>> {
                let limit_i64 = limit as i64;
                let (query, params): (String, Vec<Box<dyn rusqlite::ToSql>>) = if let Some(ref skill) = skill_name {
                    (
                        format!("SELECT {} FROM heartbeat_audit_log WHERE skill_name = ?1 ORDER BY id DESC LIMIT ?2", AUDIT_SELECT_COLS),
                        vec![Box::new(skill.clone()) as Box<dyn rusqlite::ToSql>, Box::new(limit_i64)],
                    )
                } else {
                    (
                        format!("SELECT {} FROM heartbeat_audit_log ORDER BY id DESC LIMIT ?1", AUDIT_SELECT_COLS),
                        vec![Box::new(limit_i64) as Box<dyn rusqlite::ToSql>],
                    )
                };
                let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
                let mut stmt = conn.prepare(&query)?;
                let rows = stmt.query_map(param_refs.as_slice(), |row| {
                    parse_heartbeat_audit_row(row)
                })?;
                Ok(rows.filter_map(row_ok).collect())
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::SumTodaysHeartbeatTokens { reply } => {
            let result = (|| -> Result<u32> {
                let sum: i64 = conn.query_row(
                    "SELECT COALESCE(SUM(tokens_used), 0) FROM heartbeat_audit_log \
                     WHERE date(timestamp) = date('now')",
                    [],
                    |row| row.get(0),
                )?;
                Ok(sum.max(0) as u32)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::QueryHeartbeatAuditRecent { lookback_days, reply } => {
            let result = (|| -> Result<Vec<HeartbeatAuditEntry>> {
                let mut stmt = conn.prepare(
                    &format!("SELECT {} FROM heartbeat_audit_log \
                     WHERE tool_call_details IS NOT NULL \
                     AND timestamp >= datetime('now', ?1) \
                     ORDER BY id DESC", AUDIT_SELECT_COLS)
                )?;
                let offset_str = format!("-{} days", lookback_days);
                let rows = stmt.query_map([&offset_str], |row| {
                    parse_heartbeat_audit_row(row)
                })?;
                Ok(rows.filter_map(row_ok).collect())
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::StoreCreditAnalysis {
            analysis_date, tool_name, times_called, times_cited,
            effectiveness_score, wasted_calls, avg_latency_ms,
            total_tokens_attributed, reply,
        } => {
            let result = conn.execute(
                "INSERT INTO tool_credit_analysis \
                 (analysis_date, tool_name, times_called, times_cited, \
                  effectiveness_score, wasted_calls, avg_latency_ms, total_tokens_attributed) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                 ON CONFLICT(tool_name, analysis_date) DO UPDATE SET \
                   times_called = excluded.times_called, \
                   times_cited = excluded.times_cited, \
                   effectiveness_score = excluded.effectiveness_score, \
                   wasted_calls = excluded.wasted_calls, \
                   avg_latency_ms = excluded.avg_latency_ms, \
                   total_tokens_attributed = excluded.total_tokens_attributed",
                rusqlite::params![
                    analysis_date, tool_name, times_called, times_cited,
                    effectiveness_score, wasted_calls, avg_latency_ms,
                    total_tokens_attributed
                ],
            ).map(|_| ()).map_err(|e| anyhow::anyhow!("store credit analysis: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::QueryLatestCreditAnalysis { lookback_days, reply } => {
            let result = (|| -> Result<Vec<crate::chronos::ToolCreditScore>> {
                let mut stmt = conn.prepare(
                    "SELECT tool_name, times_called, times_cited, \
                     effectiveness_score, wasted_calls, avg_latency_ms \
                     FROM tool_credit_analysis \
                     WHERE analysis_date >= date('now', ?1) \
                     ORDER BY effectiveness_score DESC"
                )?;
                let offset_str = format!("-{} days", lookback_days);
                let rows = stmt.query_map([&offset_str], |row| {
                    Ok(crate::chronos::ToolCreditScore {
                        tool_name: row.get(0)?,
                        times_called: row.get::<_, i32>(1)? as u32,
                        times_cited: row.get::<_, i32>(2)? as u32,
                        effectiveness_score: row.get(3)?,
                        wasted_calls: row.get::<_, i32>(4)? as u32,
                        avg_latency_ms: row.get(5)?,
                    })
                })?;
                Ok(rows.filter_map(row_ok).collect())
            })();
            mem_reply!(reply, result);
        }

        MemoryCommand::StoreLlmCallLog {
            label, model, provider, provider_request_id, prompt_tokens, completion_tokens, cached_tokens, total_tokens,
            cost_usd, latency_ms, is_fallback,
        } => {
            if let Err(e) = conn.execute(
                "INSERT INTO llm_call_log (label, model, provider, provider_request_id, prompt_tokens, completion_tokens, cached_tokens, total_tokens, cost_usd, latency_ms, is_fallback) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![label, model, provider, provider_request_id, prompt_tokens, completion_tokens, cached_tokens, total_tokens, cost_usd, latency_ms.map(|v| v as i64), is_fallback as i32],
            ) {
                tracing::warn!(error = %e, "Failed to store LLM call log (non-fatal)");
            }
        }

        // ── Cost analytics queries ────────────────────────────────────────

        MemoryCommand::QueryCostByLabel { days, reply } => {
            let result = (|| -> anyhow::Result<Vec<CostByLabel>> {
                let param = format!("-{days} days");
                let mut stmt = conn.prepare(
                    "SELECT label, COALESCE(SUM(cost_usd),0), COUNT(*), COALESCE(AVG(latency_ms),0) \
                     FROM llm_call_log \
                     WHERE timestamp > datetime('now', ?1) \
                     GROUP BY label ORDER BY SUM(cost_usd) DESC",
                )?;
                let rows = stmt.query_map(rusqlite::params![param], |row| {
                    Ok(CostByLabel {
                        label: row.get(0)?,
                        total_cost: row.get(1)?,
                        call_count: row.get(2)?,
                        avg_latency_ms: row.get(3)?,
                    })
                })?;
                Ok(rows.filter_map(row_ok).collect())
            })();
            mem_reply!(reply, result);
        }

        MemoryCommand::QueryCostByModel { days, reply } => {
            let result = (|| -> anyhow::Result<Vec<CostByModel>> {
                let param = format!("-{days} days");
                let mut stmt = conn.prepare(
                    "SELECT model, COALESCE(SUM(cost_usd),0), COALESCE(SUM(prompt_tokens),0), \
                     COALESCE(SUM(completion_tokens),0) \
                     FROM llm_call_log \
                     WHERE timestamp > datetime('now', ?1) \
                     GROUP BY model ORDER BY SUM(cost_usd) DESC",
                )?;
                let rows = stmt.query_map(rusqlite::params![param], |row| {
                    Ok(CostByModel {
                        model: row.get(0)?,
                        total_cost: row.get(1)?,
                        prompt_tokens: row.get(2)?,
                        completion_tokens: row.get(3)?,
                    })
                })?;
                Ok(rows.filter_map(row_ok).collect())
            })();
            mem_reply!(reply, result);
        }

        MemoryCommand::QueryDailyCostTotals { days, reply } => {
            let result = (|| -> anyhow::Result<Vec<DailyCostTotal>> {
                let param = format!("-{days} days");
                let mut stmt = conn.prepare(
                    "SELECT DATE(timestamp), COALESCE(SUM(cost_usd),0), COUNT(*) \
                     FROM llm_call_log \
                     WHERE timestamp > datetime('now', ?1) \
                     GROUP BY DATE(timestamp) ORDER BY DATE(timestamp)",
                )?;
                let rows = stmt.query_map(rusqlite::params![param], |row| {
                    Ok(DailyCostTotal {
                        date: row.get(0)?,
                        cost: row.get(1)?,
                        calls: row.get(2)?,
                    })
                })?;
                Ok(rows.filter_map(row_ok).collect())
            })();
            mem_reply!(reply, result);
        }

        MemoryCommand::QueryCostCoverage { days, reply } => {
            let result = (|| -> anyhow::Result<CostCoverage> {
                let param = format!("-{days} days");
                let (
                    row_count,
                    priced_row_count,
                    null_cost_rows,
                    api_row_count,
                    api_priced_row_count,
                    api_null_cost_rows,
                    zero_marginal_subscription_rows,
                    total_prompt_tokens,
                    total_completion_tokens,
                    total_cached_tokens,
                    total_tokens,
                    min_timestamp,
                    max_timestamp,
                ): (
                    i64,
                    i64,
                    i64,
                    i64,
                    i64,
                    i64,
                    i64,
                    i64,
                    i64,
                    i64,
                    i64,
                    Option<String>,
                    Option<String>,
                ) = conn.query_row(
                    "WITH classified AS (
                         SELECT *,
                            CASE
                              WHEN COALESCE(provider,'') IN ('codex_oauth','xai_subscription','grok_subscription')
                                OR (provider IS NULL AND (model LIKE 'grok-%' OR model LIKE 'gpt-5.5%'))
                              THEN 1 ELSE 0
                            END AS zero_marginal_subscription
                         FROM llm_call_log
                         WHERE timestamp > datetime('now', ?1)
                     )
                     SELECT COUNT(*), COUNT(cost_usd), COALESCE(SUM(cost_usd IS NULL),0), \
                            COALESCE(SUM(CASE WHEN zero_marginal_subscription = 0 THEN 1 ELSE 0 END),0), \
                            COALESCE(SUM(CASE WHEN zero_marginal_subscription = 0 AND cost_usd IS NOT NULL THEN 1 ELSE 0 END),0), \
                            COALESCE(SUM(CASE WHEN zero_marginal_subscription = 0 AND cost_usd IS NULL THEN 1 ELSE 0 END),0), \
                            COALESCE(SUM(CASE WHEN zero_marginal_subscription = 1 THEN 1 ELSE 0 END),0), \
                            COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0), \
                            COALESCE(SUM(cached_tokens),0), COALESCE(SUM(total_tokens),0), \
                            MIN(timestamp), MAX(timestamp) \
                     FROM classified",
                    rusqlite::params![param],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                            row.get(8)?,
                            row.get(9)?,
                            row.get(10)?,
                            row.get(11)?,
                            row.get(12)?,
                        ))
                    },
                )?;

                let (
                    pricing_rows,
                    pricing_sources,
                    pricing_min_updated_at,
                    pricing_max_updated_at,
                ): (i64, i64, Option<String>, Option<String>) = conn
                    .query_row(
                        "SELECT COUNT(*), COUNT(DISTINCT source), MIN(updated_at), MAX(updated_at) \
                         FROM llm_pricing",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .unwrap_or((0, 0, None, None));

                Ok(CostCoverage {
                    row_count,
                    priced_row_count,
                    null_cost_rows,
                    api_row_count,
                    api_priced_row_count,
                    api_null_cost_rows,
                    zero_marginal_subscription_rows,
                    total_prompt_tokens,
                    total_completion_tokens,
                    total_cached_tokens,
                    total_tokens,
                    min_timestamp,
                    max_timestamp,
                    pricing_rows,
                    pricing_sources,
                    pricing_min_updated_at,
                    pricing_max_updated_at,
                })
            })();
            mem_reply!(reply, result);
        }

        MemoryCommand::SnapshotDailyCostRollup => {
            if let Err(e) = conn.execute_batch(
                "INSERT OR REPLACE INTO llm_cost_daily \
                   (date, label, model, call_count, total_prompt_tokens, total_completion_tokens, \
                    total_cached_tokens, total_cost_usd, avg_latency_ms, fallback_count) \
                 SELECT DATE(timestamp), label, model, COUNT(*), SUM(prompt_tokens), \
                   SUM(completion_tokens), SUM(cached_tokens), COALESCE(SUM(cost_usd),0), \
                   COALESCE(AVG(latency_ms),0), SUM(CASE WHEN is_fallback THEN 1 ELSE 0 END) \
                 FROM llm_call_log \
                 WHERE DATE(timestamp) = DATE('now','-1 day') \
                 GROUP BY DATE(timestamp), label, model",
            ) {
                tracing::warn!(error = %e, "[CostAnalytics] Failed to snapshot daily cost rollup");
            } else {
                tracing::info!("[CostAnalytics] Daily cost rollup snapshot completed");
            }
        }

        MemoryCommand::BackfillCostUsd { reply } => {
            let result = (|| -> anyhow::Result<BackfillResult> {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT model, provider FROM llm_call_log WHERE cost_usd IS NULL",
                )?;
                let model_providers: Vec<(String, Option<String>)> = stmt
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .filter_map(row_ok)
                    .collect();

                let mut total_updated = 0usize;
                let mut models_missing = Vec::new();

                for (model, provider) in &model_providers {
                    let provider_ref = provider.as_deref();
                    if crate::llm_pricing::is_zero_marginal_subscription_usage(provider_ref, model)
                    {
                        total_updated += conn.execute(
                            "UPDATE llm_call_log SET cost_usd = 0.0
                             WHERE cost_usd IS NULL AND model = ?1
                               AND ((provider IS NULL AND ?2 IS NULL) OR provider = ?2)",
                            rusqlite::params![model, provider_ref],
                        )?;
                    } else {
                        let db_pricing_rows: i64 = conn.query_row(
                            "SELECT COUNT(*) FROM llm_pricing WHERE model = ?1",
                            rusqlite::params![model],
                            |row| row.get(0),
                        )?;
                        if db_pricing_rows > 0 {
                        let updated = conn.execute(
                            "UPDATE llm_call_log SET cost_usd = ( \
                               SELECT CASE WHEN p.prompt_per_token IS NOT NULL THEN \
                                 (prompt_tokens - COALESCE(cached_tokens,0)) * p.prompt_per_token + \
                                 COALESCE(cached_tokens,0) * p.cache_read_per_token + \
                                 completion_tokens * p.completion_per_token + \
                                 p.request_cost \
                               END \
                               FROM llm_pricing p WHERE p.model = llm_call_log.model \
                             ) \
                             WHERE cost_usd IS NULL AND model = ?1
                               AND ((provider IS NULL AND ?2 IS NULL) OR provider = ?2)",
                            rusqlite::params![model, provider_ref],
                        )?;
                            total_updated += updated;
                        } else if crate::llm_pricing::calculate_cost(model, 1, 0, 0).is_some() {
                            let mut update_stmt = conn.prepare(
                                "SELECT rowid, prompt_tokens, completion_tokens, cached_tokens \
                                 FROM llm_call_log
                                 WHERE cost_usd IS NULL AND model = ?1
                                   AND ((provider IS NULL AND ?2 IS NULL) OR provider = ?2)",
                            )?;
                            let rows: Vec<(i64, u32, u32, u32)> = update_stmt
                                .query_map(rusqlite::params![model, provider_ref], |row| {
                                    Ok((
                                        row.get(0)?,
                                        row.get::<_, u32>(1)?,
                                        row.get::<_, u32>(2)?,
                                        row.get::<_, u32>(3).unwrap_or(0),
                                    ))
                                })?
                                .filter_map(row_ok)
                                .collect();
                            for (rowid, pt, ct, cached) in &rows {
                                if let Some(cost) =
                                    crate::llm_pricing::calculate_cost(model, *pt, *ct, *cached)
                                {
                                    conn.execute(
                                        "UPDATE llm_call_log SET cost_usd = ?1 WHERE rowid = ?2",
                                        rusqlite::params![cost, rowid],
                                    )?;
                                    total_updated += 1;
                                }
                            }
                        } else {
                            models_missing.push(model.clone());
                        }
                    }
                }

                let rows_still_null: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM llm_call_log WHERE cost_usd IS NULL",
                    [],
                    |row| row.get(0),
                )?;
                let rows_still_null = rows_still_null as usize;

                Ok(BackfillResult {
                    rows_updated: total_updated,
                    rows_still_null,
                    models_missing,
                })
            })();
            mem_reply!(reply, result);
        }

        _ => {
            tracing::error!("BUG: non-analytics command routed to handle_analytics_cmd");
        }
    }
}

// ── Self-audit query handler ──────────────────────────────────────────────

pub(super) fn handle_self_audit_cmd(conn: &rusqlite::Connection, cmd: MemoryCommand) {
    match cmd {
        MemoryCommand::AuditHeartbeatNoopRatio { hours, reply } => {
            let result = (|| -> anyhow::Result<(i64, i64)> {
                let row = conn.query_row(
                    "SELECT \
                       COUNT(CASE WHEN outcome IN ('noop', 'maintenance_only', 'acted') THEN 1 END) AS total, \
                       COUNT(CASE WHEN outcome = 'noop' THEN 1 END) AS noop \
                     FROM heartbeat_audit_log \
                     WHERE skill_name = '__heartbeat__' \
                       AND timestamp > datetime('now', ?1)",
                    rusqlite::params![format!("-{hours} hours")],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                )?;
                Ok((row.1, row.0))
            })();
            mem_reply!(reply, result);
        }

        MemoryCommand::AuditRecentHeartbeatNoopCount { hours, reply } => {
            let result = conn
                .query_row(
                    "SELECT COUNT(*) \
                     FROM heartbeat_audit_log \
                     WHERE skill_name = '__heartbeat__' \
                       AND outcome = 'noop' \
                       AND timestamp > datetime('now', ?1)",
                    rusqlite::params![format!("-{hours} hours")],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(|e| anyhow::anyhow!("audit_recent_heartbeat_noop_count: {e}"));
            mem_reply!(reply, result);
        }

        MemoryCommand::AuditRecentToolBlocks { hours, reply } => {
            let result = (|| -> anyhow::Result<Vec<(String, i64)>> {
                // Parse tools_blocked JSON arrays and aggregate tool names.
                let mut stmt = conn.prepare(
                    "SELECT tools_blocked FROM heartbeat_audit_log \
                     WHERE tools_blocked != '[]' \
                       AND timestamp > datetime('now', ?1)",
                )?;
                let rows: Vec<String> = stmt
                    .query_map(rusqlite::params![format!("-{hours} hours")], |r| r.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();

                let mut counts: std::collections::HashMap<String, i64> =
                    std::collections::HashMap::new();
                for json_str in &rows {
                    if let Ok(arr) = serde_json::from_str::<Vec<String>>(json_str) {
                        for tool in arr {
                            *counts.entry(tool).or_insert(0) += 1;
                        }
                    }
                }
                let mut result: Vec<(String, i64)> = counts.into_iter().collect();
                result.sort_by_key(|a| std::cmp::Reverse(a.1));
                Ok(result)
            })();
            mem_reply!(reply, result);
        }

        MemoryCommand::AuditRecentLlmCalls { hours, reply } => {
            let result = conn
                .query_row(
                    "SELECT COUNT(*) FROM llm_call_log WHERE timestamp > datetime('now', ?1)",
                    rusqlite::params![format!("-{hours} hours")],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(|e| anyhow::anyhow!("audit_recent_llm_calls: {e}"));
            mem_reply!(reply, result);
        }

        MemoryCommand::AuditRecentHeartbeatCount { hours, reply } => {
            let result = conn.query_row(
                "SELECT COUNT(*) FROM heartbeat_audit_log WHERE timestamp > datetime('now', ?1)",
                rusqlite::params![format!("-{hours} hours")],
                |r| r.get::<_, i64>(0),
            ).map_err(|e| anyhow::anyhow!("audit_recent_heartbeat_count: {e}"));
            mem_reply!(reply, result);
        }

        MemoryCommand::AuditRecentLlmEligibleHeartbeatCount { hours, reply } => {
            let result = conn
                .query_row(
                    "SELECT COUNT(*) \
                 FROM heartbeat_audit_log \
                 WHERE timestamp > datetime('now', ?1) \
                   AND skill_name = '__heartbeat__' \
                   AND outcome IN ('noop', 'maintenance_only', 'acted')",
                    rusqlite::params![format!("-{hours} hours")],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(|e| anyhow::anyhow!("audit_recent_llm_eligible_heartbeat_count: {e}"));
            mem_reply!(reply, result);
        }

        MemoryCommand::AuditDailyCacheRates { days, reply } => {
            let result = (|| -> anyhow::Result<Vec<(String, f64)>> {
                let mut stmt = conn.prepare(
                    "SELECT DATE(timestamp) AS day, \
                       CASE WHEN SUM(prompt_tokens) > 0 \
                            THEN SUM(cached_tokens) * 100.0 / SUM(prompt_tokens) \
                            ELSE 0.0 END AS cache_pct \
                     FROM llm_call_log \
                     WHERE timestamp > datetime('now', ?1) \
                     GROUP BY day ORDER BY day",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![format!("-{days} days")], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(rows)
            })();
            mem_reply!(reply, result);
        }

        MemoryCommand::AuditDailyTokenTotals { days, reply } => {
            let result = (|| -> anyhow::Result<Vec<(String, i64)>> {
                let mut stmt = conn.prepare(
                    "SELECT DATE(timestamp) AS day, SUM(total_tokens) AS total \
                     FROM llm_call_log \
                     WHERE timestamp > datetime('now', ?1) \
                     GROUP BY day ORDER BY day",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![format!("-{days} days")], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(rows)
            })();
            mem_reply!(reply, result);
        }

        #[cfg(test)]
        MemoryCommand::TestBackdateHeartbeatAuditByOutcome {
            outcome,
            hours_ago,
            reply,
        } => {
            let result = (|| -> anyhow::Result<u64> {
                let n = conn.execute(
                    "UPDATE heartbeat_audit_log \
                     SET timestamp = datetime('now', ?1) \
                     WHERE outcome = ?2",
                    rusqlite::params![format!("-{hours_ago} hours"), outcome],
                )?;
                Ok(n as u64)
            })();
            mem_reply!(reply, result);
        }

        _ => {
            tracing::error!("BUG: non-audit command routed to handle_self_audit_cmd");
        }
    }
}

// ── Prompt version handler ─────────────────────────────────────────────────

pub(super) fn handle_prompt_version_cmd(conn: &rusqlite::Connection, cmd: MemoryCommand) {
    const PROMPT_SELECT_COLS: &str = "id, module_name, version, original_text, modified_text, \
         rationale, status, approval_id, cycles_since_apply, created_at";

    match cmd {
        MemoryCommand::StorePromptVersion {
            module_name,
            version,
            original_text,
            modified_text,
            rationale,
            status,
            approval_id,
            reply,
        } => {
            let result = conn.execute(
                "INSERT INTO prompt_versions \
                 (module_name, version, original_text, modified_text, rationale, status, approval_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    module_name, version, original_text, modified_text,
                    rationale, status, approval_id
                ],
            ).map(|_| conn.last_insert_rowid())
             .map_err(|e| anyhow::anyhow!("store prompt version: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::QueryActivePromptOverrides { reply } => {
            let result = (|| -> Result<Vec<PromptOverride>> {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM prompt_versions WHERE status = 'active' \
                     ORDER BY module_name, version DESC",
                    PROMPT_SELECT_COLS
                ))?;
                let rows = stmt.query_map([], parse_prompt_override_row)?;
                Ok(rows.filter_map(row_ok).collect())
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::UpdatePromptVersionStatus { id, status, reply } => {
            let result = conn.execute(
                "UPDATE prompt_versions SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
                rusqlite::params![status, id],
            ).map(|_| ()).map_err(|e| anyhow::anyhow!("update prompt version status: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::QueryPromptVersionHistory {
            module_name,
            limit,
            reply,
        } => {
            let result = (|| -> Result<Vec<PromptOverride>> {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM prompt_versions WHERE module_name = ?1 \
                     ORDER BY version DESC LIMIT ?2",
                    PROMPT_SELECT_COLS
                ))?;
                let rows = stmt.query_map(rusqlite::params![module_name, limit as i64], |row| {
                    parse_prompt_override_row(row)
                })?;
                Ok(rows.filter_map(row_ok).collect())
            })();
            mem_reply!(reply, result);
        }

        _ => {
            tracing::error!("BUG: non-prompt-version command routed to handle_prompt_version_cmd");
        }
    }
}
