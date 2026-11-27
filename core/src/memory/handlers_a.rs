use super::*;

// ── Extracted handler groups (Phase 3 refactor) ─────────────────────────────

/// Handle deferred intent / scheduled task commands.
pub(super) fn handle_intent_cmd(
    conn: &rusqlite::Connection,
    cmd: MemoryCommand,
    db_path_str: &str,
    max_db_size: u64,
    size_warning_issued: &mut bool,
) {
    match cmd {
        MemoryCommand::StoreDeferredIntent {
            user_did,
            chat_id,
            trigger_type,
            trigger_at_unix,
            event_key,
            execution_mode,
            context_text,
            expires_at_unix,
            is_callback_origin,
            interval_seconds,
            chain_depth,
            pin_on_fire,
            group_key,
            max_fires,
            recur_weekdays,
            recur_window_start,
            recur_window_end,
            recur_timezone,
            embedding: _,
            reply,
        } => {
            if is_db_over_quota(conn, db_path_str, max_db_size, size_warning_issued) {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!("DB quota exceeded, write blocked"))
                );
                return;
            }
            let recur_weekdays_json = recur_weekdays
                .as_ref()
                .and_then(|w| serde_json::to_string(w).ok());
            // Atomic upsert: cancel existing pending intent with same group_key + user_did
            // Wrapped in a transaction to prevent race conditions between SELECT/UPDATE/INSERT
            let result = (|| -> Result<i64> {
                conn.execute_batch("BEGIN")?;
                let inner_result = (|| -> Result<i64> {
                    if let Some(ref gk) = group_key {
                        let cancelled: Vec<i64> = conn.prepare(
                                "SELECT id FROM tasks WHERE group_key = ?1 AND user_did = ?2 AND status = 'active'"
                            )?.query_map(rusqlite::params![gk, user_did], |row| row.get(0))?
                              .filter_map(|r| match r {
                                  Ok(v) => Some(v),
                                  Err(e) => {
                                      tracing::warn!("Dropped row in group-key upsert query: {}", e);
                                      None
                                  }
                              }).collect();
                        if !cancelled.is_empty() {
                            conn.execute(
                                    "UPDATE tasks SET status = 'cancelled', updated_at = datetime('now') WHERE group_key = ?1 AND user_did = ?2 AND status = 'active'",
                                    rusqlite::params![gk, user_did],
                                )?;
                            tracing::info!(
                                group_key = %gk,
                                cancelled_ids = ?cancelled,
                                "Group-key upsert: cancelled existing tasks"
                            );
                        }
                    }
                    conn.execute(
                            "INSERT INTO tasks (user_did, chat_id, title, body, trigger_type, trigger_at_unix, event_key, execution_mode, context_text, expires_at_unix, is_callback_origin, interval_seconds, chain_depth, pin_on_fire, group_key, max_fires, fire_count, source, recur_weekdays, recur_window_start, recur_window_end, recur_timezone)
                             VALUES (?1, ?2, SUBSTR(?7, 1, 60), ?7, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0, 'system', ?15, ?16, ?17, ?18)",
                            rusqlite::params![
                                user_did, chat_id, trigger_type, trigger_at_unix, event_key,
                                execution_mode, context_text, expires_at_unix,
                                is_callback_origin as i32, interval_seconds, chain_depth,
                                pin_on_fire as i32, group_key, max_fires,
                                recur_weekdays_json, recur_window_start, recur_window_end, recur_timezone
                            ],
                        )?;
                    Ok(conn.last_insert_rowid())
                })();
                match inner_result {
                    Ok(row_id) => {
                        conn.execute_batch("COMMIT")?;
                        Ok(row_id)
                    }
                    Err(e) => {
                        if let Err(rb_err) = conn.execute_batch("ROLLBACK") {
                            tracing::error!(error = %rb_err, "[MemoryActor] ROLLBACK failed during intent creation abort");
                        }
                        Err(e)
                    }
                }
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::GetDueTimeIntents { now_unix, reply } => {
            let intents = query_deferred_intents_from_tasks(
                conn,
                "SELECT id, user_did, chat_id, trigger_type, trigger_at_unix, event_key, execution_mode, COALESCE(NULLIF(context_text, ''), title) AS context_text, status, created_at, expires_at_unix, fired_at, is_callback_origin, retry_count, interval_seconds, chain_depth, pin_on_fire, group_key, max_fires, fire_count, recur_weekdays, recur_window_start, recur_window_end, recur_timezone FROM tasks WHERE status = 'active' AND trigger_type = 'time' AND trigger_at_unix <= ?1 AND (expires_at_unix IS NULL OR expires_at_unix > ?1) ORDER BY trigger_at_unix ASC, id ASC LIMIT 50",
                rusqlite::params![now_unix],
            );
            mem_reply!(reply, intents);
        }
        MemoryCommand::GetMatchingEventIntents {
            event_key,
            user_did,
            reply,
        } => {
            let intents = query_deferred_intents_from_tasks(
                conn,
                "SELECT id, user_did, chat_id, trigger_type, trigger_at_unix, event_key, execution_mode, COALESCE(NULLIF(context_text, ''), title) AS context_text, status, created_at, expires_at_unix, fired_at, is_callback_origin, retry_count, interval_seconds, chain_depth, pin_on_fire, group_key, max_fires, fire_count, recur_weekdays, recur_window_start, recur_window_end, recur_timezone FROM tasks WHERE status = 'active' AND trigger_type = 'event' AND event_key = ?1 AND user_did = ?2 LIMIT 50",
                rusqlite::params![event_key, user_did],
            );
            mem_reply!(reply, intents);
        }
        MemoryCommand::MarkIntentFired { id, reply } => {
            let now = Utc::now().to_rfc3339();
            let result = conn
                    .execute(
                        "UPDATE tasks SET status = 'fired', fired_at = ?1, retry_count = 0 WHERE id = ?2 AND status = 'active'",
                        rusqlite::params![now, id],
                    )
                    .map(|_| ())
                    .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::MarkIntentTriggerAt {
            id,
            trigger_at_unix,
            reply,
        } => {
            let result = conn
                .execute(
                    "UPDATE tasks SET trigger_at_unix = ?1 WHERE id = ?2 AND status = 'active'",
                    rusqlite::params![trigger_at_unix, id],
                )
                .map(|_| ())
                .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::RescheduleRecurringIntent {
            id,
            trigger_at_unix,
            expires_at_unix,
            reply,
        } => {
            // ROOT window-snap (see snap_trigger_for_task): a manual/skip reschedule
            // of a windowed recurring intent must land in its allowed window.
            let snapped_trigger = snap_trigger_for_task(conn, id, trigger_at_unix);
            let result = conn
                    .execute(
                        "UPDATE tasks SET trigger_at_unix = ?1, expires_at_unix = ?2, status = 'active', fired_at = NULL, fire_count = fire_count + 1, retry_count = 0 WHERE id = ?3",
                        rusqlite::params![snapped_trigger, expires_at_unix, id],
                    )
                    .map(|_| ())
                    .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::FireAndAdvanceRecurring {
            id,
            trigger_at_unix,
            expires_at_unix,
            reply,
        } => {
            let now = Utc::now().to_rfc3339();
            let result = (|| -> Result<()> {
                conn.execute(
                        "UPDATE tasks SET fired_at = ?1, trigger_at_unix = ?2, expires_at_unix = ?3, fire_count = fire_count + 1, retry_count = 0 WHERE id = ?4 AND status = 'active'",
                        rusqlite::params![now, trigger_at_unix, expires_at_unix, id],
                    )?;
                // Check if max_fires has been reached; if so, mark completed
                let (fire_count, max_fires): (i64, Option<i64>) = conn.query_row(
                    "SELECT fire_count, max_fires FROM tasks WHERE id = ?1",
                    rusqlite::params![id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                if let Some(max) = max_fires
                    && max > 0
                    && fire_count >= max
                {
                    conn.execute(
                        "UPDATE tasks SET status = 'completed' WHERE id = ?1",
                        rusqlite::params![id],
                    )?;
                    tracing::info!(
                        id = id,
                        fire_count = fire_count,
                        max_fires = max,
                        "Recurring intent reached max_fires, marked completed"
                    );
                }
                Ok(())
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::ExpireOldIntents { now_unix, reply } => {
            let result = (|| -> Result<Vec<(i64, String, String)>> {
                let mut stmt = conn.prepare(
                    "UPDATE tasks SET status = 'cancelled', updated_at = datetime('now') \
                         WHERE status = 'active' AND trigger_type IS NOT NULL \
                         AND expires_at_unix IS NOT NULL AND expires_at_unix < ?1 \
                         RETURNING id, user_did, context_text",
                )?;
                let items: Vec<(i64, String, String)> = stmt
                    .query_map(rusqlite::params![now_unix], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?
                    .filter_map(row_ok)
                    .collect();
                Ok(items)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::ListIntentHistory {
            user_did,
            since_unix,
            reply,
        } => {
            let since_dt = chrono::DateTime::from_timestamp(since_unix, 0)
                .unwrap_or_default()
                .to_rfc3339();
            let intents = query_deferred_intents_from_tasks(
                conn,
                "SELECT id, user_did, chat_id, trigger_type, trigger_at_unix, event_key, execution_mode, COALESCE(NULLIF(context_text, ''), title) AS context_text, status, created_at, expires_at_unix, fired_at, is_callback_origin, retry_count, interval_seconds, chain_depth, pin_on_fire, group_key, max_fires, fire_count, recur_weekdays, recur_window_start, recur_window_end, recur_timezone FROM tasks WHERE user_did = ?1 AND status IN ('fired', 'cancelled') AND trigger_type IS NOT NULL AND strftime('%s', COALESCE(fired_at, created_at)) >= strftime('%s', ?2) ORDER BY strftime('%s', COALESCE(fired_at, created_at)) DESC LIMIT 50",
                rusqlite::params![user_did, since_dt],
            );
            mem_reply!(reply, Ok(intents));
        }
        MemoryCommand::ArchiveOldIntents { cutoff_unix, reply } => {
            // Auto-archive long-finished tasks so they leave normal query scope.
            // Covers done/fired/cancelled/completed (plain to-dos AND scheduled
            // intents — the `trigger_type IS NOT NULL` restriction is intentionally
            // dropped so a completed plain `done` to-do is archived too).
            // 'completed' = a recurring intent that exhausted max_fires (a genuinely
            // finished state, matching the owner's "已完成之类" intent). 'dead_letter'
            // is deliberately NOT archived — it is an error/failure state the owner
            // may still want surfaced for action. The "finished at" basis is
            // the LATEST of fired_at / completed_at / updated_at / created_at (a
            // COALESCE chain dated a recurring task by its old fired_at and
            // archived it the same afternoon it was completed). `active`
            // (and any other live status) is NEVER touched.
            let result = (|| -> Result<Vec<(i64, String, String, String)>> {
                const ARCHIVE_SELECT: &str = "SELECT id, user_did, COALESCE(NULLIF(context_text, ''), title), status FROM tasks WHERE status IN ('done','fired','cancelled','completed') AND CAST(MAX(strftime('%s', IFNULL(fired_at, created_at)), strftime('%s', IFNULL(completed_at, created_at)), strftime('%s', updated_at), strftime('%s', created_at)) AS INTEGER) < ?1";
                let mut stmt = conn.prepare(ARCHIVE_SELECT)?;
                let items: Vec<(i64, String, String, String)> = stmt
                    .query_map(rusqlite::params![cutoff_unix], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                    })?
                    .filter_map(row_ok)
                    .collect();
                // Stamp archived_at = now so the retention-delete sweep measures
                // "time since archived" (a fresh grace window) rather than reusing
                // the original finished-at basis — otherwise an old finished task
                // would be archived AND immediately past the delete cutoff in the
                // same sweep, skipping the archived grace period entirely.
                conn.execute(
                        "UPDATE tasks SET status = 'archived', archived_at = datetime('now') WHERE status IN ('done','fired','cancelled','completed') AND CAST(MAX(strftime('%s', IFNULL(fired_at, created_at)), strftime('%s', IFNULL(completed_at, created_at)), strftime('%s', updated_at), strftime('%s', created_at)) AS INTEGER) < ?1",
                        rusqlite::params![cutoff_unix],
                    )?;
                Ok(items)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::DeleteOldArchivedTasks {
            cutoff_unix,
            limit,
            reply,
        } => {
            // DESTRUCTIVE retention sweep: permanently delete tasks that have been
            // `archived` for longer than the retention cutoff. SAFETY: only
            // status='archived' rows; age is measured from `archived_at` (when the
            // row entered archived state), NOT the original finished-at basis, so a
            // freshly-archived row always gets its full grace window. Batch bounded
            // by a hard cap inside this primitive so neither a backlog nor a
            // mis-configured/hostile caller-supplied limit can trigger one giant
            // delete. Returns (deleted_count, min_id, max_id) for auditable logging.
            let result = (|| -> Result<(usize, Option<i64>, Option<i64>)> {
                // Hard blast-radius cap, enforced HERE (not just by the caller): a
                // non-positive limit deletes nothing; anything larger is clamped to
                // RETENTION_BATCH_LIMIT. The destructive primitive never trusts the
                // caller-supplied bound.
                const RETENTION_BATCH_LIMIT: i64 = 500;
                if limit <= 0 {
                    return Ok((0, None, None));
                }
                let effective_limit = limit.min(RETENTION_BATCH_LIMIT);
                // archived_at is the age basis; COALESCE to the finished-at basis only
                // for any legacy archived row whose archived_at the backfill missed
                // (defence-in-depth — the migration backfills NULL archived_at to now).
                const VICTIM_SELECT: &str = "SELECT id FROM tasks WHERE status = 'archived' AND CAST(strftime('%s', COALESCE(archived_at, fired_at, completed_at, updated_at, created_at)) AS INTEGER) < ?1 ORDER BY CAST(strftime('%s', COALESCE(archived_at, fired_at, completed_at, updated_at, created_at)) AS INTEGER) ASC LIMIT ?2";
                let mut stmt = conn.prepare(VICTIM_SELECT)?;
                let ids: Vec<i64> = stmt
                    .query_map(rusqlite::params![cutoff_unix, effective_limit], |row| {
                        row.get(0)
                    })?
                    .filter_map(row_ok)
                    .collect();
                if ids.is_empty() {
                    return Ok((0, None, None));
                }
                let min_id = ids.iter().copied().min();
                let max_id = ids.iter().copied().max();
                // Delete by id, re-asserting status='archived' as a belt-and-suspenders
                // guard so a status change between SELECT and DELETE can never delete a
                // re-activated task.
                let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql = format!(
                    "DELETE FROM tasks WHERE status = 'archived' AND id IN ({placeholders})"
                );
                let params: Vec<&dyn rusqlite::types::ToSql> = ids
                    .iter()
                    .map(|id| id as &dyn rusqlite::types::ToSql)
                    .collect();
                let deleted = conn.execute(&sql, params.as_slice())?;
                Ok((deleted, min_id, max_id))
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::DismissIntent {
            id,
            user_did,
            reply,
        } => {
            let result = conn
                    .execute(
                        "UPDATE tasks SET status = 'archived', archived_at = datetime('now') WHERE id = ?1 AND user_did = ?2 AND status IN ('fired', 'cancelled')",
                        rusqlite::params![id, user_did],
                    )
                    .map(|changed| changed > 0)
                    .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::RescheduleIntent {
            id,
            user_did,
            trigger_at_unix,
            expires_at_unix,
            reply,
        } => {
            // ROOT window-snap (see snap_trigger_for_task): reactivating a
            // fired/cancelled windowed recurring intent (e.g. Telegram snooze of an
            // inactive reminder) must also land in its allowed window.
            let snapped_trigger = snap_trigger_for_task(conn, id, trigger_at_unix);
            let result = conn
                    .execute(
                        "UPDATE tasks SET status = 'active', trigger_at_unix = ?1, expires_at_unix = ?2, fired_at = NULL WHERE id = ?3 AND user_did = ?4 AND status IN ('fired', 'cancelled')",
                        rusqlite::params![snapped_trigger, expires_at_unix, id, user_did],
                    )
                    .map(|changed| changed > 0)
                    .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::ListUserIntents { user_did, reply } => {
            let intents = query_deferred_intents_from_tasks(
                conn,
                "SELECT id, user_did, chat_id, trigger_type, trigger_at_unix, event_key, execution_mode, COALESCE(NULLIF(context_text, ''), title) AS context_text, status, created_at, expires_at_unix, fired_at, is_callback_origin, retry_count, interval_seconds, chain_depth, pin_on_fire, group_key, max_fires, fire_count, recur_weekdays, recur_window_start, recur_window_end, recur_timezone FROM tasks WHERE user_did = ?1 AND status = 'active' AND trigger_type IS NOT NULL ORDER BY id DESC",
                rusqlite::params![user_did],
            );
            mem_reply!(reply, intents);
        }
        MemoryCommand::ListRecurringIntents { user_did, reply } => {
            let intents = query_deferred_intents_from_tasks(
                conn,
                "SELECT id, user_did, chat_id, trigger_type, trigger_at_unix, event_key, execution_mode, COALESCE(NULLIF(context_text, ''), title) AS context_text, status, created_at, expires_at_unix, fired_at, is_callback_origin, retry_count, interval_seconds, chain_depth, pin_on_fire, group_key, max_fires, fire_count, recur_weekdays, recur_window_start, recur_window_end, recur_timezone FROM tasks WHERE user_did = ?1 AND status = 'active' AND interval_seconds IS NOT NULL ORDER BY id DESC LIMIT 20",
                rusqlite::params![user_did],
            );
            mem_reply!(reply, intents);
        }
        MemoryCommand::CancelIntent {
            id,
            user_did,
            reply,
        } => {
            let changed = conn
                    .execute(
                        "UPDATE tasks SET status = 'cancelled', updated_at = datetime('now') WHERE id = ?1 AND user_did = ?2 AND status = 'active'",
                        rusqlite::params![id, user_did],
                    )
                    .unwrap_or(0);
            mem_reply!(reply, Ok(changed > 0));
        }
        MemoryCommand::CountActiveIntents { user_did, reply } => {
            let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM tasks WHERE user_did = ?1 AND status = 'active' AND trigger_type IS NOT NULL",
                        rusqlite::params![user_did],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
            mem_reply!(reply, count);
        }
        MemoryCommand::CountDailyIntents { user_did, reply } => {
            let today = Utc::now().format("%Y-%m-%d").to_string();
            let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM tasks WHERE user_did = ?1 AND trigger_type IS NOT NULL AND created_at >= ?2",
                        rusqlite::params![user_did, today],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
            mem_reply!(reply, count);
        }

        MemoryCommand::IncrementIntentRetry {
            id,
            max_retries,
            reply,
        } => {
            let result = (|| -> Result<i32> {
                conn.execute(
                        "UPDATE tasks SET retry_count = retry_count + 1 WHERE id = ?1 AND status IN ('active', 'fired')",
                        rusqlite::params![id],
                    )?;
                let new_count: i32 = conn.query_row(
                    "SELECT retry_count FROM tasks WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get(0),
                )?;
                if new_count >= max_retries {
                    conn.execute(
                        "UPDATE tasks SET status = 'dead_letter' WHERE id = ?1",
                        rusqlite::params![id],
                    )?;
                    tracing::warn!(
                        intent_id = id,
                        retry_count = new_count,
                        "task moved to dead_letter"
                    );
                }
                Ok(new_count)
            })();
            mem_reply!(reply, result);
        }
        _ => {
            tracing::error!("BUG: non-intent command routed to handle_intent_cmd");
        }
    }
}

/// Handle knowledge base commands.
pub(super) fn handle_knowledge_cmd(
    conn: &rusqlite::Connection,
    cmd: MemoryCommand,
    db_path_str: &str,
    max_db_size: u64,
    size_warning_issued: &mut bool,
) {
    match cmd {
        MemoryCommand::StoreKnowledgeEntry {
            chat_id,
            category,
            priority,
            title,
            body,
            tags,
            related_ids,
            source_msg_id,
            source_summary,
            due_date,
            waiting_on,
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
            // Enforce per-chat entry limit
            let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM knowledge_entries WHERE chat_id = ?1 AND status = 'active'",
                    rusqlite::params![chat_id], |row| row.get(0),
                ).unwrap_or(0);
            if count >= MAX_KB_ENTRIES_PER_CHAT {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!(
                        "Knowledge base limit reached ({MAX_KB_ENTRIES_PER_CHAT} active entries). \
                         Archive or resolve old entries first."
                    ))
                );
                return;
            }
            let now = Utc::now().to_rfc3339();
            let result = conn
                .execute(
                    "INSERT INTO knowledge_entries \
                     (chat_id, category, priority, title, body, tags, related_ids, \
                      source_msg_id, source_summary, due_date, waiting_on, embedding, \
                      created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    rusqlite::params![
                        chat_id,
                        category,
                        priority,
                        title,
                        body,
                        tags,
                        related_ids,
                        source_msg_id,
                        source_summary,
                        due_date,
                        waiting_on,
                        embedding,
                        now,
                        now,
                    ],
                )
                .map(|_| conn.last_insert_rowid())
                .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::UpdateKnowledgeEntry {
            id,
            chat_id,
            updates,
            reply,
        } => {
            let mut sets = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

            if let Some(ref s) = updates.status {
                sets.push("status = ?");
                params.push(Box::new(s.clone()));
            }
            if let Some(ref p) = updates.priority {
                sets.push("priority = ?");
                params.push(Box::new(p.clone()));
            }
            if let Some(ref t) = updates.title {
                sets.push("title = ?");
                params.push(Box::new(t.clone()));
            }
            if let Some(ref b) = updates.body {
                sets.push("body = ?");
                params.push(Box::new(b.clone()));
            }
            if let Some(ref t) = updates.tags {
                sets.push("tags = ?");
                params.push(Box::new(
                    serde_json::to_string(t).unwrap_or_else(|_| "[]".into()),
                ));
            }
            if let Some(ref r) = updates.related_ids {
                sets.push("related_ids = ?");
                params.push(Box::new(
                    serde_json::to_string(r).unwrap_or_else(|_| "[]".into()),
                ));
            }
            if let Some(ref d) = updates.due_date {
                sets.push("due_date = ?");
                params.push(Box::new(d.clone()));
            }
            if let Some(ref w) = updates.waiting_on {
                sets.push("waiting_on = ?");
                params.push(Box::new(w.clone()));
            }
            if let Some(s) = updates.superseded_by {
                sets.push("superseded_by = ?");
                params.push(Box::new(s));
                // Auto-set status to superseded
                if updates.status.is_none() {
                    sets.push("status = 'superseded'");
                }
            }
            if let Some(ref r) = updates.resolved_at {
                sets.push("resolved_at = ?");
                params.push(Box::new(r.clone()));
            }

            if sets.is_empty() {
                mem_reply!(reply, Ok(false));
                return;
            }

            sets.push("updated_at = ?");
            params.push(Box::new(Utc::now().to_rfc3339()));

            // Build numbered parameters
            let set_clause: String = sets
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    if s.contains('?') {
                        s.replacen('?', &format!("?{}", i + 1), 1)
                    } else {
                        s.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");

            let n = params.len();
            params.push(Box::new(id));
            params.push(Box::new(chat_id));

            let sql = format!(
                "UPDATE knowledge_entries SET {} WHERE id = ?{} AND chat_id = ?{}",
                set_clause,
                n + 1,
                n + 2,
            );

            let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let result = conn
                .execute(&sql, param_refs.as_slice())
                .map(|changed| changed > 0)
                .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::QueryKnowledgeEntries {
            chat_id,
            category,
            status,
            priority,
            limit,
            reply,
        } => {
            let results = (|| -> Vec<KnowledgeEntry> {
                let mut conditions = vec!["chat_id = ?1".to_string()];
                let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(chat_id)];
                let mut idx = 2;

                if let Some(ref c) = category {
                    conditions.push(format!("category = ?{idx}"));
                    params.push(Box::new(c.clone()));
                    idx += 1;
                }
                if let Some(ref s) = status {
                    conditions.push(format!("status = ?{idx}"));
                    params.push(Box::new(s.clone()));
                    idx += 1;
                }
                if let Some(ref p) = priority {
                    conditions.push(format!("priority = ?{idx}"));
                    params.push(Box::new(p.clone()));
                    idx += 1;
                }
                let limit_param_idx = idx;
                params.push(Box::new(limit));

                let sql = format!(
                    "SELECT id, chat_id, category, priority, status, title, body, tags, \
                         related_ids, source_msg_id, source_summary, due_date, waiting_on, \
                         superseded_by, resolved_at, created_at, updated_at \
                         FROM knowledge_entries WHERE {} \
                         ORDER BY \
                           CASE priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 \
                           WHEN 'normal' THEN 2 ELSE 3 END, \
                           created_at DESC \
                         LIMIT ?{limit_param_idx}",
                    conditions.join(" AND "),
                );

                let param_refs: Vec<&dyn rusqlite::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();
                let mut stmt = match conn.prepare(&sql) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = %e, "kb query prepare");
                        return vec![];
                    }
                };
                let rows = match stmt.query_map(param_refs.as_slice(), parse_kb_row) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!(error = %e, "kb query exec");
                        return vec![];
                    }
                };
                rows.filter_map(|r| match r {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::warn!(error = %e, "Dropped row in knowledge base query");
                        None
                    }
                })
                .collect()
            })();
            mem_reply!(reply, results);
        }
        MemoryCommand::GetKnowledgeEntry { id, chat_id, reply } => {
            let result = conn.query_row(
                "SELECT id, chat_id, category, priority, status, title, body, tags, \
                     related_ids, source_msg_id, source_summary, due_date, waiting_on, \
                     superseded_by, resolved_at, created_at, updated_at \
                     FROM knowledge_entries WHERE id = ?1 AND chat_id = ?2",
                rusqlite::params![id, chat_id],
                parse_kb_row,
            );
            match result {
                Ok(entry) => {
                    mem_reply!(reply, Some(entry));
                }
                Err(_) => {
                    mem_reply!(reply, None);
                }
            }
        }
        MemoryCommand::SearchKnowledgeEntries {
            chat_id,
            query,
            limit,
            reply,
        } => {
            let results = (|| -> Vec<KnowledgeEntry> {
                let pattern = format!("%{}%", escape_like(&query));
                let sql = "SELECT id, chat_id, category, priority, status, title, body, tags, \
                         related_ids, source_msg_id, source_summary, due_date, waiting_on, \
                         superseded_by, resolved_at, created_at, updated_at \
                         FROM knowledge_entries \
                         WHERE chat_id = ?1 AND status = 'active' \
                           AND (title LIKE ?2 ESCAPE '\\' OR body LIKE ?2 ESCAPE '\\' OR tags LIKE ?2 ESCAPE '\\') \
                         ORDER BY \
                           CASE priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 \
                           WHEN 'normal' THEN 2 ELSE 3 END, \
                           created_at DESC \
                         LIMIT ?3";
                let mut stmt = match conn.prepare(sql) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = %e, "kb search prepare");
                        return vec![];
                    }
                };
                let rows = match stmt
                    .query_map(rusqlite::params![chat_id, pattern, limit], parse_kb_row)
                {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!(error = %e, "kb search exec");
                        return vec![];
                    }
                };
                rows.filter_map(|r| match r {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::warn!(error = %e, "Dropped row in knowledge base search");
                        None
                    }
                })
                .collect()
            })();
            mem_reply!(reply, results);
        }
        MemoryCommand::CountKnowledgeEntries { chat_id, reply } => {
            let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM knowledge_entries WHERE chat_id = ?1 AND status = 'active'",
                    rusqlite::params![chat_id], |row| row.get(0),
                ).unwrap_or(0);
            mem_reply!(reply, count);
        }
        MemoryCommand::KnowledgeDashboardSummary { chat_id, reply } => {
            let stats = (|| -> Vec<(String, String, i64)> {
                let mut stmt = match conn.prepare(
                    "SELECT category, status, COUNT(*) FROM knowledge_entries \
                         WHERE chat_id = ?1 GROUP BY category, status ORDER BY category, status",
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoryActor] query failed");
                        return Vec::new();
                    }
                };
                let rows = match stmt.query_map(rusqlite::params![chat_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                }) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoryActor] query failed");
                        return Vec::new();
                    }
                };
                rows.filter_map(row_ok).collect()
            })();
            mem_reply!(reply, stats);
        }
        MemoryCommand::KbHygieneSummary {
            chat_id,
            user_did,
            reply,
        } => {
            let count_pending_lint = |finding_type: Option<&str>| -> i64 {
                let mut sql = String::from(
                    "SELECT COUNT(*)
                     FROM l2_lint_findings f
                     LEFT JOIN l2_experience a ON a.id = f.fact_id_a
                     LEFT JOIN l2_experience b ON b.id = f.fact_id_b
                     WHERE f.status = 'pending'
                       AND (a.agent_did = ?1 OR b.agent_did = ?1)",
                );
                if finding_type.is_some() {
                    sql.push_str(" AND f.finding_type = ?2");
                }
                let mut stmt = match conn.prepare(&sql) {
                    Ok(stmt) => stmt,
                    Err(e) => {
                        tracing::warn!(error = %e, "kb_hygiene_summary lint prepare failed");
                        return 0;
                    }
                };
                let result = if let Some(kind) = finding_type {
                    stmt.query_row(rusqlite::params![user_did.as_str(), kind], |row| row.get(0))
                } else {
                    stmt.query_row(rusqlite::params![user_did.as_str()], |row| row.get(0))
                };
                result.unwrap_or(0)
            };
            let l2_total = conn
                .query_row(
                    "SELECT COUNT(*) FROM l2_experience WHERE agent_did = ?1",
                    rusqlite::params![user_did.as_str()],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            let urgent_kb_items = conn
                .query_row(
                    "SELECT COUNT(*) FROM knowledge_entries
                     WHERE chat_id = ?1 AND status = 'active' AND priority = 'urgent'",
                    rusqlite::params![chat_id],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            mem_reply!(
                reply,
                KbHygieneSummary {
                    l2_total,
                    pending_lint_findings: count_pending_lint(None),
                    pending_contradictions: count_pending_lint(Some("contradiction")),
                    pending_connections: count_pending_lint(Some("connection")),
                    urgent_kb_items,
                    stale_low_recall_candidates: 0,
                }
            );
        }
        _ => {
            tracing::error!("BUG: non-knowledge command routed to handle_knowledge_cmd");
        }
    }
}

/// Handle reminder commands.
pub(super) fn handle_reminder_cmd(
    conn: &rusqlite::Connection,
    cmd: MemoryCommand,
    db_path_str: &str,
    max_db_size: u64,
    size_warning_issued: &mut bool,
) {
    match cmd {
        MemoryCommand::StoreReminder {
            chat_id,
            message,
            remind_at_unix,
            reply,
        } => {
            if is_db_over_quota(conn, db_path_str, max_db_size, size_warning_issued) {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!("DB quota exceeded, write blocked"))
                );
                return;
            }
            let result = conn
                .execute(
                    "INSERT INTO reminders (chat_id, message, remind_at_unix, created_at, fired)
                         VALUES (?1, ?2, ?3, ?4, 0)",
                    rusqlite::params![chat_id, message, remind_at_unix, Utc::now().to_rfc3339()],
                )
                .map(|_| conn.last_insert_rowid())
                .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::GetDueReminders { now_unix, reply } => {
            let results = (|| -> Option<Vec<Reminder>> {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, chat_id, message, remind_at_unix, created_at, fired
                             FROM reminders
                             WHERE fired = 0 AND remind_at_unix <= ?1
                             ORDER BY remind_at_unix ASC
                             LIMIT 50",
                    )
                    .map_err(|e| {
                        tracing::error!(error = %e, "reminders prepare failed");
                        e
                    })
                    .ok()?;
                let rows: Vec<_> = stmt
                    .query_map(rusqlite::params![now_unix], |row| {
                        Ok(Reminder {
                            id: row.get(0)?,
                            chat_id: row.get(1)?,
                            message: row.get(2)?,
                            remind_at_unix: row.get(3)?,
                            created_at: row.get(4)?,
                            fired: row.get::<_, i32>(5)? != 0,
                        })
                    })
                    .map_err(|e| {
                        tracing::error!(error = %e, "reminders query failed");
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
        MemoryCommand::MarkReminderFired { id, reply } => {
            let result = conn
                .execute(
                    "UPDATE reminders SET fired = 1 WHERE id = ?1",
                    rusqlite::params![id],
                )
                .map(|_| ())
                .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::GetPendingReminders { reply } => {
            let result: Vec<Reminder> = conn
                    .prepare("SELECT id, chat_id, message, remind_at_unix, created_at, fired FROM reminders WHERE fired = 0 ORDER BY remind_at_unix ASC")
                    .and_then(|mut stmt| {
                        let rows = stmt.query_map([], |row| {
                            Ok(Reminder {
                                id: row.get(0)?,
                                chat_id: row.get(1)?,
                                message: row.get(2)?,
                                remind_at_unix: row.get(3)?,
                                created_at: row.get(4)?,
                                fired: row.get::<_, i32>(5)? != 0,
                            })
                        })?;
                        rows.collect::<Result<Vec<_>, _>>()
                    })
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "GetPendingReminders query failed");
                        Vec::new()
                    });
            mem_reply!(reply, result);
        }
        MemoryCommand::ListRemindersForChat { chat_id, resp } => {
            let result = conn
                    .prepare("SELECT id, chat_id, message, remind_at_unix, created_at, fired FROM reminders WHERE chat_id = ?1 AND fired = 0 ORDER BY remind_at_unix ASC")
                    .and_then(|mut stmt| {
                        let rows = stmt.query_map(rusqlite::params![chat_id], |row| {
                            Ok(Reminder {
                                id: row.get(0)?,
                                chat_id: row.get(1)?,
                                message: row.get(2)?,
                                remind_at_unix: row.get(3)?,
                                created_at: row.get(4)?,
                                fired: row.get::<_, i32>(5)? != 0,
                            })
                        })?;
                        rows.collect::<Result<Vec<_>, _>>()
                    })
                    .map_err(anyhow::Error::from);
            mem_reply!(resp, result);
        }
        MemoryCommand::CancelReminder {
            reminder_id,
            chat_id,
            resp,
        } => {
            let result = conn
                .execute(
                    "DELETE FROM reminders WHERE id = ?1 AND chat_id = ?2 AND fired = 0",
                    rusqlite::params![reminder_id, chat_id],
                )
                .map(|changed| changed > 0)
                .map_err(anyhow::Error::from);
            mem_reply!(resp, result);
        }
        _ => {
            tracing::error!("BUG: non-reminder command routed to handle_reminder_cmd");
        }
    }
}

/// Handle skill proposal commands.
pub(super) fn handle_skill_proposal_cmd(
    conn: &rusqlite::Connection,
    cmd: MemoryCommand,
    db_path_str: &str,
    max_db_size: u64,
    size_warning_issued: &mut bool,
) {
    match cmd {
        MemoryCommand::CreateSkillProposal {
            skill_name,
            requested_by,
            user_request,
            proposal_summary,
            reply,
        } => {
            if is_db_over_quota(conn, db_path_str, max_db_size, size_warning_issued) {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!("DB quota exceeded, write blocked"))
                );
                return;
            }
            let now = Utc::now().to_rfc3339();
            let result = conn
                    .execute(
                        "INSERT INTO skill_proposals (skill_name, requested_by, user_request, proposal_summary, status, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, 'pending_approval', ?5, ?5)",
                        rusqlite::params![skill_name, requested_by, user_request, proposal_summary, now],
                    )
                    .map(|_| conn.last_insert_rowid())
                    .map_err(anyhow::Error::from);
            mem_reply!(reply, result);
        }
        MemoryCommand::UpdateSkillProposal { id, updates, reply } => {
            if is_db_over_quota(conn, db_path_str, max_db_size, size_warning_issued) {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!("DB quota exceeded, write blocked"))
                );
                return;
            }
            let result = update_skill_proposal_row(conn, id, &updates);
            mem_reply!(reply, result);
        }
        MemoryCommand::GetSkillProposal { id, reply } => {
            let result = get_skill_proposal_row(conn, id);
            mem_reply!(reply, result);
        }
        MemoryCommand::ListSkillProposals {
            requested_by,
            status_filter,
            reply,
        } => {
            let result =
                list_skill_proposals_rows(conn, requested_by.as_deref(), status_filter.as_deref());
            mem_reply!(reply, result);
        }
        MemoryCommand::CountDailyProposals {
            requested_by,
            reply,
        } => {
            let today = Utc::now().format("%Y-%m-%d").to_string();
            let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM skill_proposals WHERE requested_by = ?1 AND created_at >= ?2",
                        rusqlite::params![requested_by, today],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
            mem_reply!(reply, count);
        }
        MemoryCommand::DeleteSkillProposal { id, reply } => {
            let changed = conn
                .execute(
                    "DELETE FROM skill_proposals WHERE id = ?1",
                    rusqlite::params![id],
                )
                .unwrap_or(0);
            mem_reply!(reply, changed > 0);
        }
        _ => {
            tracing::error!("BUG: non-skill-proposal command routed to handle_skill_proposal_cmd");
        }
    }
}

/// Handle L2 topic-prefix query commands (read-only).
pub(super) fn handle_l2_topic_query_cmd(conn: &rusqlite::Connection, cmd: MemoryCommand) {
    match cmd {
        MemoryCommand::QueryL2ByTopicPrefix { did, prefix, reply } => {
            let results = (|| -> Vec<(String, Vec<f32>)> {
                let like_pattern = format!("{}%", escape_like(&prefix));
                let mut stmt = match conn.prepare(
                        "SELECT fact_summary, embedding FROM l2_experience
                         WHERE agent_did = ?1 AND topic LIKE ?2 ESCAPE '\\' AND embedding IS NOT NULL
                         AND (expires_at IS NULL OR expires_at > datetime('now'))
                         LIMIT 1000",
                    ) {
                        Ok(s) => s,
                        Err(e) => { tracing::warn!(error = %e, "[MemoryActor] query failed"); return Vec::new(); }
                    };
                let rows = match stmt.query_map(rusqlite::params![did, like_pattern], |row| {
                    let fact: String = row.get(0)?;
                    let blob: Vec<u8> = row.get(1)?;
                    Ok((fact, blob))
                }) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoryActor] query failed");
                        return Vec::new();
                    }
                };
                rows.filter_map(row_ok)
                    .map(|(fact, blob)| (fact, EmbeddingClient::from_blob(&blob)))
                    .collect()
            })();
            mem_reply!(reply, results);
        }

        MemoryCommand::QueryL2ByTopicPrefixSince {
            did,
            prefix,
            since,
            reply,
        } => {
            let results = (|| -> Vec<(String, Vec<f32>)> {
                let like_pattern = format!("{}%", escape_like(&prefix));
                let mut stmt = match conn.prepare(
                    "SELECT fact_summary, embedding FROM l2_experience
                         WHERE agent_did = ?1 AND topic LIKE ?2 ESCAPE '\\'
                           AND embedding IS NOT NULL AND created_at >= ?3
                           AND (expires_at IS NULL OR expires_at > datetime('now'))
                         ORDER BY created_at DESC
                         LIMIT 1000",
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoryActor] query failed");
                        return Vec::new();
                    }
                };
                let rows =
                    match stmt.query_map(rusqlite::params![did, like_pattern, since], |row| {
                        let fact: String = row.get(0)?;
                        let blob: Vec<u8> = row.get(1)?;
                        Ok((fact, blob))
                    }) {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!(error = %e, "[MemoryActor] query failed");
                            return Vec::new();
                        }
                    };
                rows.filter_map(row_ok)
                    .map(|(fact, blob)| (fact, EmbeddingClient::from_blob(&blob)))
                    .collect()
            })();
            mem_reply!(reply, results);
        }

        MemoryCommand::QueryL2TextByTopicPrefixSince {
            did,
            prefix,
            since,
            reply,
        } => {
            let results = (|| -> Vec<String> {
                let like_pattern = format!("{}%", escape_like(&prefix));
                let mut stmt = match conn.prepare(
                    "SELECT fact_summary FROM l2_experience
                         WHERE agent_did = ?1 AND topic LIKE ?2 ESCAPE '\\'
                           AND created_at >= ?3
                           AND (expires_at IS NULL OR expires_at > datetime('now'))
                         ORDER BY created_at DESC
                         LIMIT 1000",
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoryActor] query failed");
                        return Vec::new();
                    }
                };
                let rows =
                    match stmt.query_map(rusqlite::params![did, like_pattern, since], |row| {
                        let fact: String = row.get(0)?;
                        Ok(fact)
                    }) {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!(error = %e, "[MemoryActor] query failed");
                            return Vec::new();
                        }
                    };
                rows.filter_map(row_ok).collect()
            })();
            mem_reply!(reply, results);
        }

        MemoryCommand::GetL2ByIdIfFresh {
            did,
            row_id,
            since_rfc3339,
            reply,
        } => {
            let result: Option<L2RowBrief> = (|| -> Option<L2RowBrief> {
                let mut stmt = match conn.prepare(
                    "SELECT id, topic, fact_summary, created_at, is_pinned FROM l2_experience
                       WHERE id = ?1 AND agent_did = ?2 AND created_at >= ?3
                         AND (expires_at IS NULL OR expires_at > datetime('now'))
                       LIMIT 1",
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoryActor] get_l2_by_id_if_fresh prepare failed");
                        return None;
                    }
                };
                let mut rows = match stmt.query(rusqlite::params![row_id, did, since_rfc3339]) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoryActor] get_l2_by_id_if_fresh query failed");
                        return None;
                    }
                };
                match rows.next() {
                    Ok(Some(row)) => Some(L2RowBrief {
                        id: row.get(0).ok()?,
                        topic: row.get(1).ok()?,
                        fact_summary: row.get(2).ok()?,
                        created_at: row.get(3).ok()?,
                        is_pinned: row.get::<_, Option<i64>>(4).ok()?.unwrap_or(0) != 0,
                    }),
                    _ => None,
                }
            })();
            let _ = reply.send(result);
        }
        _ => {
            tracing::error!("BUG: non-l2-topic-query command routed to handle_l2_topic_query_cmd");
        }
    }
}

/// Handle user setting commands.
pub(super) fn handle_setting_cmd(
    conn: &rusqlite::Connection,
    cmd: MemoryCommand,
    db_path_str: &str,
    max_db_size: u64,
    size_warning_issued: &mut bool,
) {
    match cmd {
        MemoryCommand::CommitRuntimeSettings {
            updates,
            settings,
            write_guard,
            embedding,
            reply,
        } => {
            let result = (|| -> Result<_> {
                if updates.iter().any(|update| update.stored.is_some())
                    && is_db_over_quota(conn, db_path_str, max_db_size, size_warning_issued)
                {
                    anyhow::bail!("DB quota exceeded, write blocked");
                }
                let transaction = conn.unchecked_transaction()?;
                let now = Utc::now().to_rfc3339();
                for update in &updates {
                    if let Some(value) = &update.stored {
                        transaction.execute(
                            "INSERT OR REPLACE INTO user_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                            rusqlite::params![update.key, value.to_string(), now],
                        )?;
                    } else {
                        transaction
                            .execute("DELETE FROM user_settings WHERE key = ?1", [&update.key])?;
                    }
                }
                transaction.commit()?;
                // Infallible publication stays on the serialized actor, before
                // replying or processing another write, even if the caller left.
                let current = updates
                    .iter()
                    .map(|update| (update.key.clone(), update.effective.clone()))
                    .collect();
                let previous = settings.publish_updates(updates);
                let mut outcomes = std::collections::HashMap::new();
                if let Some(embedding) = embedding {
                    for (key, plan) in embedding.updates {
                        outcomes.insert(key, embedding.client.apply_prepared_settings_update(plan));
                    }
                }
                Ok(RuntimeSettingsCommit {
                    previous,
                    current,
                    embedding: outcomes,
                })
            })();
            drop(write_guard);
            mem_reply!(reply, result);
        }
        MemoryCommand::StoreSetting { key, value, reply } => {
            if is_db_over_quota(conn, db_path_str, max_db_size, size_warning_issued) {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!("DB quota exceeded, write blocked"))
                );
                return;
            }
            let json_str = value.to_string();
            let now = Utc::now().to_rfc3339();
            let result = conn
                    .execute(
                        "INSERT OR REPLACE INTO user_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                        rusqlite::params![key, json_str, now],
                    )
                    .map(|_| ())
                    .map_err(|e| anyhow::anyhow!("store_setting: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::DeleteSetting { key, reply } => {
            let result = conn
                .execute(
                    "DELETE FROM user_settings WHERE key = ?1",
                    rusqlite::params![key],
                )
                .map(|n| n > 0)
                .map_err(|e| anyhow::anyhow!("delete_setting: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::LoadAllSettings { reply } => {
            let result = (|| -> Result<std::collections::HashMap<String, serde_json::Value>> {
                let mut stmt = conn.prepare("SELECT key, value FROM user_settings")?;
                let rows = stmt.query_map([], |row| {
                    let key: String = row.get(0)?;
                    let val_str: String = row.get(1)?;
                    Ok((key, val_str))
                })?;
                let mut map = std::collections::HashMap::new();
                for row in rows {
                    let (key, val_str) = row?;
                    if let Ok(val) = serde_json::from_str(&val_str) {
                        map.insert(key, val);
                    }
                }
                Ok(map)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::InsertConfigAudit {
            actor,
            old_json,
            new_json,
            reason,
            reply,
        } => {
            if is_db_over_quota(conn, db_path_str, max_db_size, size_warning_issued) {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!("DB quota exceeded, write blocked"))
                );
                return;
            }
            let now = Utc::now().to_rfc3339();
            let result = conn
                .execute(
                    "INSERT INTO config_audit_log (at, actor, old_json, new_json, reason) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![now, actor, old_json, new_json, reason],
                )
                .map(|_| conn.last_insert_rowid())
                .map_err(|e| anyhow::anyhow!("insert_config_audit: {e}"));
            mem_reply!(reply, result);
        }
        MemoryCommand::ListConfigAudit { limit, reply } => {
            let result = (|| -> Result<Vec<crate::heartbeat_config::ConfigAuditEntry>> {
                let mut stmt = conn.prepare(
                    "SELECT id, at, actor, old_json, new_json, reason \
                         FROM config_audit_log ORDER BY id DESC LIMIT ?1",
                )?;
                let rows = stmt.query_map([limit as i64], |row| {
                    Ok(crate::heartbeat_config::ConfigAuditEntry {
                        id: row.get(0)?,
                        at: row.get(1)?,
                        actor: row.get(2)?,
                        old_json: row.get(3)?,
                        new_json: row.get(4)?,
                        reason: row.get(5)?,
                    })
                })?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })();
            mem_reply!(reply, result);
        }
        _ => {
            tracing::error!("BUG: non-setting command routed to handle_setting_cmd");
        }
    }
}

// ── Skill metrics handler ───────────────────────────────────────────────────

/// Parse a `skill_metrics` row into a [`SkillMetrics`] struct.
///
/// Expects columns: skill_name, total_calls, success_count, error_count,
/// avg_latency_ms, last_error, consecutive_failures, circuit_open.
pub(super) fn metrics_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillMetrics> {
    Ok(SkillMetrics {
        skill_name: row.get(0)?,
        total_calls: row.get::<_, i64>(1)? as u64,
        success_count: row.get::<_, i64>(2)? as u64,
        error_count: row.get::<_, i64>(3)? as u64,
        avg_latency_ms: row.get(4)?,
        last_error: row.get(5)?,
        error_categories: std::collections::HashMap::new(),
        consecutive_failures: row.get::<_, i64>(6)? as u64,
        circuit_open: row.get::<_, i64>(7)? != 0,
    })
}

pub(super) fn handle_skill_metrics_cmd(
    conn: &rusqlite::Connection,
    cmd: MemoryCommand,
    db_path_str: &str,
    max_db_size: u64,
    size_warning_issued: &mut bool,
    circuit_breaker_tx: &mpsc::Sender<CircuitBreakerEvent>,
) {
    match cmd {
        MemoryCommand::UpdateSkillMetrics {
            skill_name,
            success,
            latency_ms,
            error_msg,
            error_category,
        } => {
            if is_db_over_quota(conn, db_path_str, max_db_size, size_warning_issued) {
                tracing::warn!(skill_name = %skill_name, "DB quota exceeded, skipping metrics update");
                return;
            }
            // Upsert skill metrics
            let existing: Option<(u64, u64, u64, f64, String, i64, bool)> = conn.query_row(
                    "SELECT total_calls, success_count, error_count, avg_latency_ms, error_categories,
                            consecutive_failures, circuit_open
                     FROM skill_metrics WHERE skill_name = ?1",
                    rusqlite::params![skill_name],
                    |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64, row.get::<_, i64>(2)? as u64, row.get(3)?, row.get(4)?,
                              row.get(5)?, row.get::<_, i64>(6)? != 0)),
                ).map_err(|e| {
                    if !matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                        tracing::error!(skill_name = %skill_name, error = %e, "skill_metrics query failed (possible schema mismatch)");
                    }
                    e
                }).ok();

            let now = Utc::now().to_rfc3339();
            match existing {
                Some((total, succ, err, avg_lat, _cats_json, prev_consec, prev_open)) => {
                    let new_total = total + 1;
                    let new_succ = if success { succ + 1 } else { succ };
                    let new_err = if success { err } else { err + 1 };
                    let new_avg = (avg_lat * total as f64 + latency_ms) / new_total as f64;

                    // Circuit breaker: track consecutive failures
                    let (new_consec, new_open) = if success {
                        (0i64, false)
                    } else {
                        let c = prev_consec + 1;
                        (c, c >= CB_FAILURE_THRESHOLD)
                    };

                    // Update counters (no JSON read-modify-write)
                    if let Err(e) = conn.execute(
                        "UPDATE skill_metrics SET total_calls=?1, success_count=?2, error_count=?3,
                             avg_latency_ms=?4, last_error=?5, updated_at=?6,
                             consecutive_failures=?8, circuit_open=?9
                             WHERE skill_name=?7",
                        rusqlite::params![
                            new_total as i64,
                            new_succ as i64,
                            new_err as i64,
                            new_avg,
                            error_msg,
                            now,
                            skill_name,
                            new_consec,
                            new_open as i64
                        ],
                    ) {
                        tracing::warn!(error = %e, skill_name = %skill_name, "Failed to UPDATE skill_metrics — metrics silently lost");
                    }

                    // Emit circuit breaker event on 0→1 transition
                    if new_open && !prev_open {
                        tracing::warn!(
                            skill_name = %skill_name,
                            consecutive_failures = new_consec,
                            "Circuit breaker OPEN — skill paused after {} consecutive failures",
                            new_consec,
                        );
                        let _ = circuit_breaker_tx.try_send(CircuitBreakerEvent {
                            skill_name: skill_name.clone(),
                            consecutive_failures: new_consec as u64,
                            last_error: error_msg.clone(),
                        });
                    }
                }
                None => {
                    let (init_consec, init_open) = if success { (0i64, false) } else { (1, false) };
                    if let Err(e) = conn.execute(
                            "INSERT INTO skill_metrics (skill_name, total_calls, success_count, error_count,
                             avg_latency_ms, last_error, error_categories, updated_at,
                             consecutive_failures, circuit_open)
                             VALUES (?1, 1, ?2, ?3, ?4, ?5, '{}', ?6, ?7, ?8)",
                            rusqlite::params![
                                skill_name,
                                if success { 1i64 } else { 0i64 },
                                if success { 0i64 } else { 1i64 },
                                latency_ms,
                                error_msg,
                                now,
                                init_consec,
                                init_open as i64,
                            ],
                        ) {
                            tracing::warn!(error = %e, skill_name = %skill_name, "Failed to INSERT skill_metrics — metrics silently lost");
                        }
                }
            }

            // Atomic O(1) error category increment in normalized table
            if let Some(cat) = &error_category
                && let Err(e) = conn.execute(
                    "INSERT INTO skill_error_categories (skill_name, category, count)
                         VALUES (?1, ?2, 1)
                         ON CONFLICT(skill_name, category) DO UPDATE SET count = count + 1",
                    rusqlite::params![skill_name, cat],
                )
            {
                tracing::warn!(error = %e, skill_name = %skill_name, category = %cat, "Failed to upsert skill_error_categories");
            }
        }

        MemoryCommand::GetSkillMetrics { skill_name, reply } => {
            let result = conn.query_row(
                    "SELECT skill_name, total_calls, success_count, error_count, avg_latency_ms,
                            last_error, consecutive_failures, circuit_open
                     FROM skill_metrics WHERE skill_name = ?1",
                    rusqlite::params![skill_name],
                    metrics_from_row,
                ).map_err(|e| {
                    if !matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                        tracing::error!(skill_name = %skill_name, error = %e, "GetSkillMetrics query failed (possible schema mismatch)");
                    }
                    e
                }).ok();
            // Populate error_categories from normalized table
            let result = result.map(|mut m| {
                m.error_categories = query_error_categories(conn, &m.skill_name);
                m
            });
            mem_reply!(reply, result);
        }

        MemoryCommand::GetAllSkillMetrics { reply } => {
            let results = (|| -> Vec<SkillMetrics> {
                let mut stmt = match conn.prepare(
                    "SELECT skill_name, total_calls, success_count, error_count, avg_latency_ms,
                                last_error, consecutive_failures, circuit_open
                         FROM skill_metrics ORDER BY total_calls DESC",
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoryActor] query failed");
                        return Vec::new();
                    }
                };
                let rows = match stmt.query_map([], metrics_from_row) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoryActor] query failed");
                        return Vec::new();
                    }
                };
                rows.filter_map(row_ok).collect()
            })();
            // Populate error_categories from normalized table for each metric
            let results: Vec<SkillMetrics> = results
                .into_iter()
                .map(|mut m| {
                    m.error_categories = query_error_categories(conn, &m.skill_name);
                    m
                })
                .collect();
            mem_reply!(reply, results);
        }

        MemoryCommand::CountUnhealthySkillMetricsUpdatedSince {
            since_rfc3339,
            reply,
        } => {
            let result = conn
                .query_row(
                    "SELECT COUNT(*) FROM skill_metrics \
                     WHERE updated_at >= ?1 \
                       AND (circuit_open != 0 OR consecutive_failures > 0)",
                    rusqlite::params![since_rfc3339],
                    |row| row.get::<_, i64>(0),
                )
                .map(|n| n.max(0) as u32)
                .map_err(|e| anyhow::anyhow!("count_unhealthy_skill_metrics_updated_since: {e}"));
            mem_reply!(reply, result);
        }

        MemoryCommand::GetDeactivatedSkillNames { reply } => {
            let result = (|| -> std::collections::HashSet<String> {
                let mut stmt = match conn
                    .prepare("SELECT skill_name FROM skill_proposals WHERE status = 'deactivated'")
                {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to query deactivated skill names");
                        return std::collections::HashSet::new();
                    }
                };
                let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to execute deactivated skill names query");
                        return std::collections::HashSet::new();
                    }
                };
                rows.filter_map(row_ok).collect()
            })();
            mem_reply!(reply, result);
        }

        MemoryCommand::ResetCircuitBreaker { skill_name } => {
            if let Err(e) = conn.execute(
                    "UPDATE skill_metrics SET consecutive_failures=0, circuit_open=0 WHERE skill_name=?1",
                    rusqlite::params![skill_name],
                ) {
                    tracing::warn!(skill_name = %skill_name, error = %e, "Failed to reset circuit breaker");
                } else {
                    tracing::info!(skill_name = %skill_name, "Circuit breaker reset");
                }
        }
        MemoryCommand::CheckCircuitBreaker { skill_name, reply } => {
            let result = conn.query_row(
                    "SELECT circuit_open, consecutive_failures, last_error FROM skill_metrics WHERE skill_name=?1",
                    rusqlite::params![skill_name],
                    |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, i64>(1)?, row.get::<_, Option<String>>(2)?)),
                ).ok();
            mem_reply!(reply, result);
        }

        _ => {
            tracing::error!("BUG: non-skill-metrics command routed to handle_skill_metrics_cmd");
        }
    }
}

// ── Task handler ────────────────────────────────────────────────────────────

#[allow(unused_variables)]
pub(super) fn handle_task_cmd(
    conn: &rusqlite::Connection,
    cmd: MemoryCommand,
    db_path_str: &str,
    max_db_size: u64,
    size_warning_issued: &mut bool,
) {
    match cmd {
        MemoryCommand::StoreTask {
            chat_id,
            user_did,
            title,
            body,
            priority,
            tags,
            due_date,
            source,
            trigger_at_unix,
            trigger_type,
            event_key,
            execution_mode,
            context_text,
            interval_seconds,
            group_key,
            max_fires,
            pin_on_fire,
            expires_at_unix,
            is_callback_origin,
            chain_depth,
            recur_weekdays,
            recur_window_start,
            recur_window_end,
            recur_timezone,
            reply,
        } => {
            if is_db_over_quota(conn, db_path_str, max_db_size, size_warning_issued) {
                mem_reply!(
                    reply,
                    Err(anyhow::anyhow!("DB quota exceeded, write blocked"))
                );
                return;
            }
            let recur_weekdays_json = recur_weekdays
                .as_ref()
                .and_then(|w| serde_json::to_string(w).ok());
            let result = (|| -> Result<i64> {
                let effective_gk = group_key.clone().unwrap_or_else(|| auto_group_key(&title));
                conn.execute(
                        "UPDATE tasks SET status = 'cancelled', updated_at = datetime('now') WHERE group_key = ?1 AND user_did = ?2 AND status = 'active'",
                        rusqlite::params![effective_gk, user_did],
                    )?;
                conn.execute(
                        "INSERT INTO tasks (chat_id, user_did, title, body, priority, tags, due_date, source,
                             trigger_at_unix, trigger_type, event_key, execution_mode, context_text,
                             interval_seconds, group_key, max_fires, pin_on_fire, expires_at_unix,
                             is_callback_origin, chain_depth,
                             recur_weekdays, recur_window_start, recur_window_end, recur_timezone)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
                        rusqlite::params![
                            chat_id, user_did, title, body, priority, tags, due_date, source,
                            trigger_at_unix, trigger_type, event_key, execution_mode, context_text,
                            interval_seconds, effective_gk, max_fires, pin_on_fire as i32, expires_at_unix,
                            is_callback_origin as i32, chain_depth,
                            recur_weekdays_json, recur_window_start, recur_window_end, recur_timezone
                        ],
                    )?;
                Ok(conn.last_insert_rowid())
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::GetTask { id, reply } => {
            let task = query_single_task(conn, id);
            mem_reply!(reply, task);
        }
        MemoryCommand::UpdateTask {
            id,
            chat_id,
            updates_json,
            reply,
        } => {
            let result = (|| -> Result<bool> {
                let updates: serde_json::Value = serde_json::from_str(&updates_json)?;
                let mut sets = Vec::new();
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                let mut idx = 1;

                // Defensive: null values are SKIPPED (absent = preserve). An earlier
                // revision emitted `SET col = NULL` for `"col": null` inputs, which
                // violated NOT NULL constraints (e.g. `body TEXT NOT NULL DEFAULT ''`)
                // whenever a caller built the updates JSON with `args.get(k)` for
                // absent keys (those serialize to `null`).
                //
                // To explicitly clear a nullable column back to NULL, callers pass
                // `"clear_fields": ["col_a","col_b"]`. Only nullable columns are
                // honored (CLEAR_FIELDS_ALLOWED); requests to clear NOT NULL columns
                // are silently ignored to avoid re-introducing the body=NULL bug.
                const CLEAR_FIELDS_ALLOWED: &[&str] = &[
                    "due_date",
                    "trigger_at_unix",
                    "trigger_type",
                    "event_key",
                    "context_text",
                    "interval_seconds",
                    "group_key",
                    "max_fires",
                    "completed_at",
                    "fired_at",
                    "tags",
                ];
                if let Some(clear_list) = updates.get("clear_fields").and_then(|v| v.as_array()) {
                    for item in clear_list {
                        if let Some(col) = item.as_str()
                            && CLEAR_FIELDS_ALLOWED.contains(&col)
                        {
                            sets.push(format!("{col} = NULL"));
                        }
                    }
                }
                macro_rules! maybe_set {
                    ($field:expr, $col:expr) => {
                        if let Some(v) = updates.get($field)
                            && !v.is_null()
                        {
                            if let Some(s) = v.as_str() {
                                sets.push(format!("{} = ?{}", $col, idx));
                                params.push(Box::new(s.to_string()));
                                idx += 1;
                            } else if let Some(i) = v.as_i64() {
                                sets.push(format!("{} = ?{}", $col, idx));
                                params.push(Box::new(i));
                                idx += 1;
                            } else if let Some(b) = v.as_bool() {
                                sets.push(format!("{} = ?{}", $col, idx));
                                params.push(Box::new(b as i32));
                                idx += 1;
                            }
                        }
                    };
                }
                maybe_set!("title", "title");
                maybe_set!("body", "body");
                maybe_set!("priority", "priority");
                maybe_set!("status", "status");
                maybe_set!("due_date", "due_date");
                // trigger_at_unix: ROOT window-snap before write. A windowed
                // recurring intent updated here (notably the Telegram snooze
                // ACTIVE path, which writes the snooze target through update_task)
                // must land in its allowed window. snap_trigger_for_task is a no-op
                // for non-windowed rows, so ordinary task edits are unaffected.
                if let Some(v) = updates.get("trigger_at_unix")
                    && let Some(raw) = v.as_i64()
                {
                    let snapped = snap_trigger_for_task(conn, id, raw);
                    sets.push(format!("trigger_at_unix = ?{idx}"));
                    params.push(Box::new(snapped));
                    idx += 1;
                }
                maybe_set!("trigger_type", "trigger_type");
                maybe_set!("event_key", "event_key");
                maybe_set!("execution_mode", "execution_mode");
                maybe_set!("context_text", "context_text");
                maybe_set!("interval_seconds", "interval_seconds");
                maybe_set!("group_key", "group_key");
                maybe_set!("max_fires", "max_fires");
                maybe_set!("pin_on_fire", "pin_on_fire");
                maybe_set!("expires_at_unix", "expires_at_unix");
                maybe_set!("completed_at", "completed_at");
                // `null` values are skipped by design (absent = preserve), so a
                // reopen asks for the clear explicitly.
                if updates
                    .get("clear_completed_at")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    sets.push("completed_at = NULL".to_string());
                }
                maybe_set!("fired_at", "fired_at");
                // `tags` is serialized as a JSON string. Same null-skip rule as maybe_set!.
                if let Some(tags) = updates.get("tags")
                    && !tags.is_null()
                {
                    sets.push(format!("tags = ?{}", idx));
                    params.push(Box::new(tags.to_string()));
                    idx += 1;
                }
                let _ = idx; // suppress unused

                // When this generic update transitions a task INTO 'archived',
                // stamp archived_at = now. ALL manual archive paths route through
                // here (manage_tasks `delete` action, Telegram confirmdel button,
                // panel task_archive), so this single site covers the whole class.
                // Without it, archived_at stays NULL and the DESTRUCTIVE retention
                // sweep's COALESCE basis falls through to the original finished-at
                // (e.g. an old `completed_at`), which for a long-finished task is
                // already past the retention cutoff → it would be deleted on the
                // very next sweep, skipping the entire archived grace window.
                // Mirrors the auto-archive ArchiveOldIntents path.
                if updates.get("status").and_then(|v| v.as_str()) == Some("archived") {
                    sets.push("archived_at = datetime('now')".to_string());
                }

                if sets.is_empty() {
                    return Ok(false);
                }
                sets.push("updated_at = datetime('now')".to_string());

                let sql = format!(
                    "UPDATE tasks SET {} WHERE id = ?{} AND chat_id = ?{}",
                    sets.join(", "),
                    params.len() + 1,
                    params.len() + 2,
                );
                params.push(Box::new(id));
                params.push(Box::new(chat_id));

                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();
                let changed = conn.execute(&sql, param_refs.as_slice())?;
                Ok(changed > 0)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::ClearFinishedTasks { chat_id, reply } => {
            let result = (|| -> Result<usize> {
                let changed = conn.execute(
                    "DELETE FROM tasks WHERE chat_id = ?1 AND status IN ('done', 'archived')",
                    rusqlite::params![chat_id],
                )?;
                Ok(changed)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::DeleteTask { id, chat_id, reply } => {
            let result = (|| -> Result<bool> {
                let changed = conn.execute(
                        "DELETE FROM tasks WHERE id = ?1 AND chat_id = ?2 AND status IN ('done', 'archived')",
                        rusqlite::params![id, chat_id],
                    )?;
                Ok(changed > 0)
            })();
            mem_reply!(reply, result);
        }
        MemoryCommand::ListTasks {
            chat_id,
            user_did,
            status,
            limit,
            reply,
        } => {
            let tasks = (|| -> Vec<Task> {
                let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = {
                    let mut conditions = vec!["chat_id = ?1".to_string()];
                    let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(chat_id)];
                    let mut idx = 2;
                    if let Some(ref ud) = user_did {
                        conditions.push(format!("user_did = ?{}", idx));
                        p.push(Box::new(ud.clone()));
                        idx += 1;
                    }
                    if let Some(ref s) = status {
                        conditions.push(format!("status = ?{}", idx));
                        p.push(Box::new(s.clone()));
                        idx += 1;
                    } else {
                        // Default scope EXCLUDES archived tasks: auto-archived
                        // long-finished tasks must not surface in normal listings.
                        // An explicit status='archived' request still returns them
                        // (so "明确要查" works). Note: done/cancelled are NOT excluded
                        // here — archiving is the mechanism that removes finished
                        // tasks from normal scope, not the done/cancelled status.
                        conditions.push("status != 'archived'".to_string());
                    }
                    let _ = idx;
                    (format!(
                            "SELECT id, chat_id, user_did, title, body, priority, status, tags, due_date, source,
                                    trigger_at_unix, trigger_type, event_key, execution_mode, context_text,
                                    interval_seconds, group_key, max_fires, fire_count, fired_at,
                                    retry_count, chain_depth, pin_on_fire, expires_at_unix, is_callback_origin,
                                    created_at, updated_at, completed_at,
                                    recur_weekdays, recur_window_start, recur_window_end, recur_timezone
                             FROM tasks WHERE {} ORDER BY
                                CASE priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 WHEN 'normal' THEN 2 ELSE 3 END,
                                id DESC LIMIT ?{}",
                            conditions.join(" AND "),
                            p.len() + 1,
                        ), p)
                };
                let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = params_vec;
                all_params.push(Box::new(limit));
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    all_params.iter().map(|p| p.as_ref()).collect();
                let mut stmt = match conn.prepare(&sql) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoryActor] query failed");
                        return Vec::new();
                    }
                };
                stmt.query_map(param_refs.as_slice(), map_task_row)
                    .map(|rows| rows.filter_map(row_ok).collect())
                    .unwrap_or_default()
            })();
            mem_reply!(reply, tasks);
        }

        MemoryCommand::ListAllScheduledTasks { limit, reply } => {
            let sql = "SELECT id, chat_id, user_did, title, body, priority, status, tags, due_date, source,
                                  trigger_at_unix, trigger_type, event_key, execution_mode, context_text,
                                  interval_seconds, group_key, max_fires, fire_count, fired_at,
                                  retry_count, chain_depth, pin_on_fire, expires_at_unix, is_callback_origin,
                                  created_at, updated_at, completed_at,
                                  recur_weekdays, recur_window_start, recur_window_end, recur_timezone
                           FROM tasks WHERE status = 'active' AND trigger_type IS NOT NULL
                           ORDER BY trigger_at_unix ASC LIMIT ?1";
            let tasks = match conn.prepare(sql) {
                Ok(mut stmt) => stmt
                    .query_map(rusqlite::params![limit], map_task_row)
                    .map(|rows| rows.filter_map(row_ok).collect())
                    .unwrap_or_default(),
                Err(_) => Vec::new(),
            };
            mem_reply!(reply, tasks);
        }

        _ => {
            tracing::error!("BUG: non-task command routed to handle_task_cmd");
        }
    }
}
