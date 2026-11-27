//! Task and deferred-intent tool handlers.
//!
//! Extracted from `orchestrator.rs` for modularity. Contains:
//! - `handle_tool_tasks` — manage_tasks (CRUD + scheduling), schedule_intent,
//!   list_intents, cancel_intent, and the deprecated manage_reminder shim.

use anyhow::Result;
use async_openai::types::chat::ChatCompletionMessageToolCall;
use serde_json::Value;

use crate::intent;
use crate::orchestrator::{Orchestrator, ToolContext};
use crate::source_guards::{EmailSourceWriteKind, validate_email_source_write};
use crate::utils::{sanitized_tool_error, truncate_str};

impl Orchestrator {
    /// Handle tasks domain tools: manage_tasks, schedule_intent, list_intents, cancel_intent, manage_reminder.
    #[allow(unused_variables)]
    pub(crate) async fn handle_tool_tasks(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "manage_tasks"     => execute_manage_tasks,
            "schedule_intent"  => execute_schedule_intent,
            "list_intents"     => execute_list_intents,
            "cancel_intent"    => execute_cancel_intent,
            "manage_reminder"  => execute_manage_reminder,
        );
        Ok(None)
    }

    // ──────────────────────────────────────────────────────────────────────
    //  Task domain tool handlers
    // ──────────────────────────────────────────────────────────────────────

    async fn execute_manage_tasks(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        let user_did = ctx.user_did;
        let cid = ctx.chat_id.unwrap_or(0);
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        match action {
            "add" => self.task_action_add(&args, cid, user_did).await,
            "update" => {
                let task_id = args.get("task_id").and_then(|v| v.as_i64()).unwrap_or(0);
                if task_id == 0 {
                    serde_json::json!({"status": "error", "error": "task_id is required"}).to_string()
                } else {
                    // Only forward keys the LLM explicitly supplied with non-null values.
                    // Passing `null` for absent fields used to materialize `SET body = NULL`
                    // in memory.rs's maybe_set! macro, violating `body TEXT NOT NULL`.
                    // The set of updatable fields matches memory.rs MemoryCommand::UpdateTask
                    // (except status/completed_at which have dedicated actions).
                    let mut updates = serde_json::Map::new();
                    for key in [
                        "title",
                        "body",
                        "priority",
                        "due_date",
                        "tags",
                        "context_text",
                        "trigger_at_unix",
                        "interval_seconds",
                        "max_fires",
                    ] {
                        if let Some(v) = args.get(key)
                            && !v.is_null()
                        {
                            updates.insert(key.to_string(), v.clone());
                        }
                    }
                    // "把截止日期去掉": explicit clears, validated against the
                    // memory layer's allowlist (null values mean "preserve").
                    if let Some(clear) = args.get("clear_fields").and_then(|v| v.as_array()) {
                        const CLEARABLE: &[&str] = &[
                            "due_date", "trigger_at_unix", "trigger_type", "event_key",
                            "context_text", "interval_seconds", "group_key", "max_fires",
                        ];
                        let wanted: Vec<serde_json::Value> = clear
                            .iter()
                            .filter_map(|v| v.as_str())
                            .filter(|f| CLEARABLE.contains(f))
                            .map(|f| serde_json::Value::String(f.to_string()))
                            .collect();
                        if !wanted.is_empty() {
                            updates.insert("clear_fields".to_string(), serde_json::Value::Array(wanted));
                        }
                    }
                    let updates = serde_json::Value::Object(updates);
                    match self.memory.update_task(task_id, cid, &updates.to_string()).await {
                        Ok(true) => serde_json::json!({"status": "ok", "task_id": task_id}).to_string(),
                        Ok(false) => serde_json::json!({"status": "error", "error": "Task not found or no fields to update"}).to_string(),
                        Err(e) => sanitized_tool_error("task_manager", "Failed to update task / 更新任务失败", &e, "database_error", true, None),
                    }
                }
            }
            "complete" => {
                let task_id = args.get("task_id").and_then(|v| v.as_i64()).unwrap_or(0);
                if task_id == 0 {
                    serde_json::json!({"status": "error", "error": "task_id is required"}).to_string()
                } else {
                    let now = chrono::Utc::now().to_rfc3339();
                    // Note: setting status='done' alone is sufficient — scheduler/panel
                    // queries filter by status='active', so lingering trigger_* fields
                    // are inert. Previously we also passed `trigger_type: null` etc.,
                    // but maybe_set! now skips null values (absent = preserve) so those
                    // explicit clears would be no-ops anyway.
                    let updates = serde_json::json!({
                        "status": "done",
                        "completed_at": now,
                    });
                    match self.memory.update_task(task_id, cid, &updates.to_string()).await {
                        Ok(true) => serde_json::json!({"status": "ok", "message": format!("Task #{} completed.", task_id)}).to_string(),
                        Ok(false) => serde_json::json!({"status": "error", "error": "Task not found"}).to_string(),
                        Err(e) => sanitized_tool_error("task_manager", "Failed to complete task / 完成任务失败", &e, "database_error", true, None),
                    }
                }
            }
            "reopen" => {
                // Undo of `complete`/`delete`: the Work view completes a task with
                // one tap and the owner's 2026-09-13 "点太快" turn had no way back
                // (安琪 offered to restore a task no action could restore).
                let task_id = args.get("task_id").and_then(|v| v.as_i64()).unwrap_or(0);
                if task_id == 0 {
                    serde_json::json!({"status": "error", "error": "task_id is required"}).to_string()
                } else {
                    let updates = serde_json::json!({
                        "status": "active",
                        "clear_completed_at": true,
                    });
                    match self.memory.update_task(task_id, cid, &updates.to_string()).await {
                        Ok(true) => {
                            let note = self.reopen_rearm_schedule(task_id, cid).await;
                            serde_json::json!({
                                "status": "ok",
                                "message": format!("Task #{} reopened (active again).{}", task_id, note),
                            })
                            .to_string()
                        }
                        Ok(false) => serde_json::json!({"status": "error", "error": "Task not found"}).to_string(),
                        Err(e) => sanitized_tool_error("task_manager", "Failed to reopen task / 恢复任务失败", &e, "database_error", true, None),
                    }
                }
            }
            "delete" => {
                let task_id = args.get("task_id").and_then(|v| v.as_i64()).unwrap_or(0);
                if task_id == 0 {
                    serde_json::json!({"status": "error", "error": "task_id is required"}).to_string()
                } else {
                    let updates = serde_json::json!({"status": "archived"});
                    match self.memory.update_task(task_id, cid, &updates.to_string()).await {
                        Ok(true) => serde_json::json!({"status": "ok", "message": format!("Task #{} archived.", task_id)}).to_string(),
                        Ok(false) => serde_json::json!({"status": "error", "error": "Task not found"}).to_string(),
                        Err(e) => sanitized_tool_error("task_manager", "Failed to archive task / 归档任务失败", &e, "database_error", true, None),
                    }
                }
            }
            "list" => self.task_action_list(&args, cid).await,
            "schedule" => self.task_action_schedule(&args, cid, user_did).await,
            _ => serde_json::json!({
                "status": "error",
                "error": format!("Unknown action '{}'. Use: add, update, complete, reopen, delete, list, schedule", action),
            }).to_string(),
        }
    }

    // ── Task action helpers (extracted from execute_manage_tasks) ────────────

    /// Handle manage_tasks action="add" — create a task with optional scheduling.
    async fn task_action_add(&self, args: &Value, cid: i64, user_did: &str) -> String {
        // Recurrence on `add` used to bypass every intent validation (window,
        // weekday, timezone): "工作日 9–18 点每两小时" became a round-the-clock
        // alarm. Route it through the validated schedule path instead.
        if args.get("interval_seconds").is_some_and(|v| !v.is_null())
            || args.get("recur_weekdays").is_some_and(|v| !v.is_null())
            || args.get("recur_window_start").is_some_and(|v| !v.is_null())
        {
            return self.task_action_schedule(args, cid, user_did).await;
        }
        let tool_call_id: &str = "manage_tasks";
        let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
        if title.is_empty() {
            return serde_json::json!({"status": "error", "error": "title is required"})
                .to_string();
        }
        let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let priority = args
            .get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("normal");
        let due_date = args.get("due_date").and_then(|v| v.as_str());
        let tags: Vec<String> = args
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let pin = args.get("pin").and_then(|v| v.as_bool()).unwrap_or(false);
        let trigger_at_str = args.get("trigger_at").and_then(|v| v.as_str());
        let trigger_type = args.get("trigger_type").and_then(|v| v.as_str());
        let event_key = args.get("event_key").and_then(|v| v.as_str());
        // The self-built mail client left on 2026-09-07; nothing emits
        // `email:*` events any more, so an event-triggered "watch the inbox"
        // would sit forever while the owner believes it is watching.
        if let Some(key) = event_key
            && let Some(error) = unsupported_event_trigger_error(key)
        {
            return error;
        }
        let execution_mode = args
            .get("execution_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("notify");
        let context_text = args.get("context_text").and_then(|v| v.as_str());
        // Fallback: if context_text is empty and mode is notify, use title as the
        // notification text. LLM often omits context_text for simple reminders.
        let context_text_fallback;
        let context_text =
            if context_text.is_none_or(|s| s.trim().is_empty()) && execution_mode == "notify" {
                context_text_fallback = format!("⏰ {}", title);
                Some(context_text_fallback.as_str())
            } else {
                context_text
            };
        let interval_seconds = args.get("interval_seconds").and_then(|v| v.as_i64());
        let group_key = args.get("group_key").and_then(|v| v.as_str());
        let max_fires = args.get("max_fires").and_then(|v| v.as_i64());
        let source_summary = args.get("source_summary").and_then(|v| v.as_str());
        let tags_text = tags.join(" ");
        if let Err(e) = validate_email_source_write(
            &[
                title,
                body,
                context_text.unwrap_or(""),
                source_summary.unwrap_or(""),
                tags_text.as_str(),
            ],
            EmailSourceWriteKind::FollowUpAllowed,
        ) {
            return serde_json::json!({
                "status": "error",
                "error": e,
                "error_kind": "unconfirmed_source",
                "retryable": true,
                "hint": "Read the full source (not a preview or truncated snippet) before storing conclusions; create only an explicit follow-up task if the source remains incomplete."
            })
            .to_string();
        }

        let trigger_at_unix = if let Some(ta) = trigger_at_str {
            let mut args_for_tz = args.clone();
            if args_for_tz
                .get("timezone")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .is_empty()
                // `get_if_stored` (not `get`) so the env fallback is reachable.
                // `user_timezone`'s registered default is the host zone (non-empty),
                // so `.get().filter(!empty).or_else(env)` would never reach env.
                // Invariant covered by `settings::tests::stored_keys_starts_empty_after_new`.
                // Terminal `user_tz_default()` restores the registry-default
                // semantics that R2 (commit 10c5007) inadvertently dropped — without
                // it, "no DB + no env" users silently flipped from the host zone to
                // UTC (downstream `intent::parse_trigger_at` falls back to UTC when
                // `timezone` is `None`). Matches the canonical pattern in
                // `main.rs::current_brief_local_clock`.
                && let Some(tz) = self
                    .settings
                    .get_if_stored("user_timezone")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        std::env::var("USER_TIMEZONE")
                            .ok()
                            .filter(|s| !s.is_empty())
                    })
                    .or_else(|| Some(crate::settings::user_tz_default()))
            {
                args_for_tz["timezone"] = serde_json::Value::String(tz);
            }
            let tz = args_for_tz.get("timezone").and_then(|v| v.as_str());
            let now = chrono::Utc::now().timestamp();
            match intent::parse_trigger_at(ta, tz, now) {
                Ok(ts) => Some(ts),
                Err(e) => {
                    return sanitized_tool_error(
                        tool_call_id,
                        "Invalid trigger_at format / 触发时间格式无效",
                        &e,
                        "invalid_input",
                        true,
                        Some("Use ISO 8601 format or relative time like '2h', '30m'."),
                    );
                }
            }
        } else {
            None
        };

        let tt = if trigger_at_unix.is_some() || event_key.is_some() {
            trigger_type.or(if event_key.is_some() {
                Some("event")
            } else {
                Some("time")
            })
        } else {
            None
        };
        let expires = trigger_at_unix.map(|t| t + intent::MAX_INTENT_LIFETIME_SECS);

        match self
            .memory
            .store_task(
                cid,
                user_did,
                title,
                body,
                priority,
                &tags,
                due_date,
                "user",
                trigger_at_unix,
                tt,
                event_key,
                execution_mode,
                context_text,
                interval_seconds,
                group_key,
                max_fires,
                pin,
                expires,
                false,
                0,
                None,
                None,
                None,
                None,
            )
            .await
        {
            Ok(id) => {
                let action_kind = if tt.is_some() {
                    crate::action_ledger::ActionKind::ReminderCreate
                } else {
                    crate::action_ledger::ActionKind::TaskCreate
                };
                self.best_effort_task_creation_action_executed(
                    id,
                    action_kind,
                    cid,
                    user_did,
                    title,
                    body,
                    trigger_at_unix,
                    interval_seconds,
                    group_key,
                );
                let mut resp = serde_json::json!({"status": "ok", "task_id": id, "title": title});
                if trigger_at_unix.is_some() {
                    resp["scheduled"] = serde_json::json!(true);
                    resp["trigger_at_unix"] = serde_json::json!(trigger_at_unix);
                }
                if let Some(interval) = interval_seconds {
                    resp["recurring"] = serde_json::json!(true);
                    resp["interval_seconds"] = serde_json::json!(interval);
                }
                resp.to_string()
            }
            Err(e) => sanitized_tool_error(
                "task_manager",
                "Failed to create task / 创建任务失败",
                &e,
                "database_error",
                true,
                None,
            ),
        }
    }

    /// Handle manage_tasks action="list" — query and format task list.
    async fn task_action_list(&self, args: &Value, cid: i64) -> String {
        let status_filter = args
            .get("status")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("active");
        // "all"/"any"/"*" mean no filter. Before, `status: "all"` became
        // `WHERE status = 'all'` → an empty list → "未找到第 36 项，未恢复"
        // (owner turn 2026-09-13 14:55Z) although the task existed.
        let status_filter = match status_filter.to_ascii_lowercase().as_str() {
            "all" | "any" | "*" => None,
            _ => Some(status_filter.to_string()),
        };
        // Default matches the WebApp /tasks panel limit (50) so a bare
        // manage_tasks(action='list') enumerates the SAME set the panel numbers —
        // otherwise ordinals 31..50 from the visible cards would be unresolvable
        // (the index-divergence bug this fix exists to prevent, at the limit edge).
        let limit = args
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(50)
            .min(50);
        let tasks = self
            .memory
            .list_tasks(cid, None, status_filter.as_deref(), limit)
            .await;
        let entries: Vec<Value> = tasks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                // 1-based ordinal in this returned order (priority bucket, then id DESC).
                // The WebApp /tasks panel renders the SAME list_tasks(active) set with the
                // SAME index, so the owner's eyeballed number and this `index` agree. When
                // the owner says "3完成", resolve 3 → this entry's `id` (see prompts.rs).
                let index = i + 1;
                let mut entry = serde_json::json!({
                    "index": index,
                    "id": t.id, "title": t.title, "priority": t.priority, "status": t.status,
                });
                if !t.body.is_empty() {
                    entry["body"] = serde_json::json!(t.body.chars().take(200).collect::<String>());
                }
                if let Some(ref dd) = t.due_date {
                    entry["due_date"] = serde_json::json!(dd);
                }
                if !t.tags.is_empty() {
                    entry["tags"] = serde_json::json!(t.tags);
                }
                if let Some(ref tt) = t.trigger_type {
                    entry["trigger_type"] = serde_json::json!(tt);
                    entry["trigger_at_unix"] = serde_json::json!(t.trigger_at_unix);
                    entry["execution_mode"] = serde_json::json!(t.execution_mode);
                }
                if let Some(interval) = t.interval_seconds {
                    entry["recurring"] = serde_json::json!(true);
                    entry["interval_seconds"] = serde_json::json!(interval);
                    entry["fire_count"] = serde_json::json!(t.fire_count);
                }
                if let Some(ref gk) = t.group_key {
                    entry["group_key"] = serde_json::json!(gk);
                }
                entry
            })
            .collect();
        let mut result = serde_json::json!({
            "status": "ok",
            "count": entries.len(),
            "status_filter": status_filter.clone().unwrap_or_else(|| "all".to_string()),
            "tasks": entries,
        });
        if result["count"] == 0 {
            // An empty page must say WHICH filter produced it, so the model does
            // not conclude "the task does not exist" from a status mismatch.
            result["hint"] = serde_json::json!(match status_filter.as_deref() {
                Some(s) => format!(
                    "No tasks with status={s}. Other statuses may hold the task: retry with status=\"all\", or act on a known task_id directly (complete/reopen/delete need no list)."
                ),
                None => "No tasks at all for this owner.".to_string(),
            });
        }
        result.to_string()
    }

    /// After `reopen`: a trigger left in the past would fire on the next
    /// scheduler tick (complete/delete keep trigger_* in place). Recurring →
    /// advance to the next occurrence; one-shot → clear the trigger and say
    /// so; an expiry inside the next day is pushed out 30 days so the
    /// expiry sweep does not cancel the task the owner just restored.
    async fn reopen_rearm_schedule(&self, task_id: i64, cid: i64) -> String {
        let Some(task) = self.memory.get_task(task_id).await else {
            return String::new();
        };
        let now = chrono::Utc::now().timestamp();
        let mut notes = Vec::new();
        if let Some(trigger) = task.trigger_at_unix
            && trigger < now
        {
            match task.interval_seconds.filter(|i| *i > 0) {
                Some(interval) => {
                    let steps = (now - trigger) / interval + 1;
                    let next = trigger + steps * interval;
                    let updates = serde_json::json!({ "trigger_at_unix": next });
                    if matches!(
                        self.memory
                            .update_task(task_id, cid, &updates.to_string())
                            .await,
                        Ok(true)
                    ) {
                        notes.push(format!(
                            " Recurring reminder re-armed for the next occurrence (unix {next})."
                        ));
                    }
                }
                None => {
                    let updates =
                        serde_json::json!({ "clear_fields": ["trigger_at_unix", "trigger_type"] });
                    if matches!(
                        self.memory
                            .update_task(task_id, cid, &updates.to_string())
                            .await,
                        Ok(true)
                    ) {
                        notes.push(" Its reminder time had already passed, so the reminder was cleared — schedule a new time if one is still wanted.".to_string());
                    }
                }
            }
        }
        if let Some(expires) = task.expires_at_unix
            && expires < now + 86_400
        {
            let updates = serde_json::json!({ "expires_at_unix": now + 30 * 86_400 });
            if matches!(
                self.memory
                    .update_task(task_id, cid, &updates.to_string())
                    .await,
                Ok(true)
            ) {
                notes.push(" Expiry extended by 30 days.".to_string());
            }
        }
        notes.concat()
    }

    /// Handle manage_tasks action="schedule" — create a scheduled/recurring task.
    async fn task_action_schedule(&self, args: &Value, cid: i64, user_did: &str) -> String {
        let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let context_text = args
            .get("context_text")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if context_text.is_empty() && title.is_empty() {
            return serde_json::json!({"status": "error", "error": "context_text or title is required for scheduling"}).to_string();
        }
        let mut intent_args = args.clone();
        if intent_args.get("trigger_type").is_none() {
            intent_args["trigger_type"] = serde_json::json!("time");
        }
        if intent_args.get("context_text").is_none() {
            intent_args["context_text"] = serde_json::json!(title);
        }
        if intent_args
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .is_empty()
            // `get_if_stored` — see paired-fix comment at first tz resolution above.
            // Terminal `user_tz_default()` restores the registry-default
            // semantics that R2 (commit 10c5007) inadvertently dropped.
            && let Some(tz) = self
                .settings
                .get_if_stored("user_timezone")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    std::env::var("USER_TIMEZONE")
                        .ok()
                        .filter(|s| !s.is_empty())
                })
                .or_else(|| Some(crate::settings::user_tz_default()))
        {
            intent_args["timezone"] = serde_json::Value::String(tz);
        }
        let has_trigger = intent_args
            .get("trigger_at")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
            || intent_args
                .get("delay_seconds")
                .and_then(|v| v.as_i64())
                .is_some();
        if !has_trigger {
            return serde_json::json!({
                "status": "error",
                "error": "action='schedule' requires either 'trigger_at' (e.g., '22:00', '2026-03-28T10:00:00') or 'delay_seconds'. Without timing, use action='add' for a plain task."
            }).to_string();
        }
        match intent::validate_intent_request(&intent_args) {
            Ok(validated) => {
                if let Some(key) = validated.event_key.as_deref()
                    && let Some(error) = unsupported_event_trigger_error(key)
                {
                    return error;
                }
                let pin = args.get("pin").and_then(|v| v.as_bool()).unwrap_or(false);
                let priority = args
                    .get("priority")
                    .and_then(|v| v.as_str())
                    .unwrap_or("normal");
                let tags: Vec<String> = args
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let due_date = args.get("due_date").and_then(|v| v.as_str());
                let task_title = if title.is_empty() {
                    derive_task_title(&validated.context_text, TASK_TITLE_MAX_CHARS)
                } else {
                    title.to_string()
                };
                match self
                    .memory
                    .store_task(
                        cid,
                        user_did,
                        &task_title,
                        &validated.context_text,
                        priority,
                        &tags,
                        due_date,
                        "user",
                        validated.trigger_at_unix,
                        Some(&validated.trigger_type),
                        validated.event_key.as_deref(),
                        &validated.execution_mode,
                        Some(&validated.context_text),
                        validated.interval_seconds,
                        validated.group_key.as_deref(),
                        validated.max_fires,
                        pin,
                        Some(validated.expires_at_unix),
                        false,
                        0,
                        validated.recur_weekdays.clone(),
                        validated.recur_window_start.as_deref(),
                        validated.recur_window_end.as_deref(),
                        validated.recur_timezone.as_deref(),
                    )
                    .await
                {
                    Ok(id) => {
                        self.best_effort_task_creation_action_executed(
                            id,
                            crate::action_ledger::ActionKind::ReminderCreate,
                            cid,
                            user_did,
                            &task_title,
                            &validated.context_text,
                            validated.trigger_at_unix,
                            validated.interval_seconds,
                            validated.group_key.as_deref(),
                        );
                        let mut resp = serde_json::json!({
                            "status": "ok", "task_id": id, "title": task_title,
                            "execution_mode": validated.execution_mode,
                            "trigger_at_unix": validated.trigger_at_unix,
                        });
                        if let Some(interval) = validated.interval_seconds {
                            resp["recurring"] = serde_json::json!(true);
                            resp["interval_seconds"] = serde_json::json!(interval);
                        }
                        if let Some(ref gk) = validated.group_key {
                            resp["group_key"] = serde_json::json!(gk);
                        }
                        if let Some(ref warn) = validated.warning {
                            resp["warning"] = serde_json::json!(warn);
                        }
                        resp.to_string()
                    }
                    Err(e) => sanitized_tool_error(
                        "task_manager",
                        "Failed to schedule task / 定时任务创建失败",
                        &e,
                        "database_error",
                        true,
                        None,
                    ),
                }
            }
            Err(e) => {
                let stale_start = e.to_string().starts_with("trigger_at must be at least");
                sanitized_tool_error(
                    "task_manager",
                    if stale_start {
                        "Schedule start time has passed / 开始时间已过"
                    } else {
                        "Invalid schedule parameters / 定时参数无效"
                    },
                    &e,
                    "validation_error",
                    false,
                    Some(if stale_start {
                        "Choose a new future trigger_at or use delay_seconds >= 10 for a relative start. Do not retry the same expired timestamp."
                    } else {
                        "Check trigger_type, trigger_at or delay_seconds, interval_seconds, group_key, and recurrence window/timezone against the tool schema."
                    }),
                )
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn execute_schedule_intent(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        let user_did = ctx.user_did;
        let chat_id = ctx.chat_id;
        let is_callback = ctx.is_callback;
        let callback_depth = ctx.callback_depth;

        // Bounded chain depth: callbacks can schedule new intents up to MAX_CHAIN_DEPTH
        let chain_depth = if is_callback {
            let new_depth = callback_depth + 1;
            if new_depth > intent::MAX_CHAIN_DEPTH {
                return serde_json::json!({
                    "status": "error",
                    "error": format!(
                        "Maximum chain depth reached ({}/{}). Cannot schedule further intents from callbacks.",
                        new_depth, intent::MAX_CHAIN_DEPTH
                    )
                }).to_string();
            }
            new_depth
        } else {
            0
        };

        // Rate limit: active intents
        let active_count = self
            .memory
            .count_active_intents(user_did)
            .await
            .unwrap_or(0);
        if active_count >= intent::MAX_ACTIVE_INTENTS {
            return serde_json::json!({
                "status": "error",
                "error": format!("Too many active intents ({}/{}). Cancel some before scheduling new ones.", active_count, intent::MAX_ACTIVE_INTENTS)
            }).to_string();
        }
        // Rate limit: daily intents (chain continuations are exempt)
        let daily_count = self.memory.count_daily_intents(user_did).await.unwrap_or(0);
        if daily_count >= intent::MAX_DAILY_INTENTS && chain_depth == 0 {
            return serde_json::json!({
                "status": "error",
                "error": format!("Daily intent limit reached ({}/{}). Try again tomorrow.", daily_count, intent::MAX_DAILY_INTENTS)
            }).to_string();
        }
        // Auto-inject user timezone if not provided by LLM
        let mut args = args;
        if args
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .is_empty()
            // `get_if_stored` — see paired-fix comment at first tz resolution above.
            // Terminal `user_tz_default()` restores the registry-default
            // semantics that R2 (commit 10c5007) inadvertently dropped.
            && let Some(tz) = self
                .settings
                .get_if_stored("user_timezone")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    std::env::var("USER_TIMEZONE")
                        .ok()
                        .filter(|s| !s.is_empty())
                })
                .or_else(|| Some(crate::settings::user_tz_default()))
        {
            args["timezone"] = serde_json::Value::String(tz);
        }
        match intent::validate_intent_request(&args) {
            Ok(validated) => {
                if let Some(key) = validated.event_key.as_deref()
                    && let Some(error) = unsupported_event_trigger_error(key)
                {
                    return error;
                }
                let cid = chat_id.unwrap_or(0);
                let pin_on_fire = args.get("pin").and_then(|v| v.as_bool()).unwrap_or(false);

                // Semantic dedup: embed context_text for recurring intents,
                // compare with existing recurring intents' embeddings.
                //
                // Degraded-mode policy: refuse to register a RECURRING intent
                // while the embedder is on backup. Recurring intents persist
                // their vector for future primary-model dedup comparisons
                // (see `find_similar_recurring_intents` in orchestrator.rs);
                // storing a backup-model vector would poison those future
                // comparisons. Mirrors the `set_monitoring_intent` refusal
                // pattern — LLM gets a retryable error and can retry after
                // primary recovers. One-shot intents (no `interval_seconds`)
                // are unaffected because they never persist an embedding.
                let is_recurring = validated.interval_seconds.is_some();
                if is_recurring && self.embedder.is_degraded() {
                    return sanitized_tool_error(
                        "schedule_intent",
                        "Embedder is on backup model; refusing to register recurring intent to avoid cross-model vector mismatch / 嵌入模型处于备份模式，暂不注册循环意图",
                        &anyhow::anyhow!("embedder degraded"),
                        "embedding_error",
                        true,
                        Some("Retry in a few minutes after primary model recovers."),
                    );
                }
                let new_embedding = if is_recurring {
                    self.embedder
                        .embed(&validated.context_text)
                        .await
                        .map_err(|e| tracing::warn!(error = %e, "Intent embedding failed"))
                        .ok()
                } else {
                    None
                };
                let similar_intents = if is_recurring {
                    if let Some(ref emb) = new_embedding {
                        self.find_similar_recurring_intents(user_did, emb, 0.85)
                            .await
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };

                match self
                    .memory
                    .store_deferred_intent(
                        user_did,
                        cid,
                        &validated.trigger_type,
                        validated.trigger_at_unix,
                        validated.event_key.as_deref(),
                        &validated.execution_mode,
                        &validated.context_text,
                        validated.expires_at_unix,
                        is_callback,
                        validated.interval_seconds,
                        chain_depth,
                        pin_on_fire,
                        validated.group_key.as_deref(),
                        validated.max_fires,
                        new_embedding.as_deref(),
                        validated.recur_weekdays.clone(),
                        validated.recur_window_start.as_deref(),
                        validated.recur_window_end.as_deref(),
                        validated.recur_timezone.as_deref(),
                    )
                    .await
                {
                    Ok(id) => {
                        let desc = match validated.trigger_type.as_str() {
                            "time" => {
                                let delay = validated.trigger_at_unix.unwrap_or(0)
                                    - chrono::Utc::now().timestamp();
                                format!("time trigger in {}s", delay)
                            }
                            "event" => format!(
                                "event trigger on '{}'",
                                validated.event_key.as_deref().unwrap_or("?")
                            ),
                            _ => "unknown trigger".to_string(),
                        };
                        let mut resp = serde_json::json!({
                            "status": "ok",
                            "intent_id": id,
                            "context_text": validated.context_text,
                            "trigger_type": validated.trigger_type,
                            "trigger": desc,
                            "execution_mode": validated.execution_mode,
                            "expires_at_unix": validated.expires_at_unix,
                        });
                        if let Some(trigger_at_unix) = validated.trigger_at_unix {
                            resp["trigger_at_unix"] = serde_json::json!(trigger_at_unix);
                        }
                        if let Some(ref event_key) = validated.event_key {
                            resp["event_key"] = serde_json::json!(event_key);
                        }
                        if let Some(interval) = validated.interval_seconds {
                            resp["recurring"] = serde_json::json!(true);
                            resp["interval_seconds"] = serde_json::json!(interval);
                        }
                        if let Some(ref gk) = validated.group_key {
                            resp["group_key"] = serde_json::json!(gk);
                            resp["note"] = serde_json::json!(
                                "Any previous pending intent with this group_key was auto-replaced."
                            );
                        }
                        if let Some(mf) = validated.max_fires {
                            resp["max_fires"] = serde_json::json!(mf);
                        }
                        if chain_depth > 0 {
                            resp["chain_depth"] = serde_json::json!(chain_depth);
                        }
                        if let Some(ref warn) = validated.warning {
                            resp["warning"] = serde_json::json!(warn);
                        }
                        // Semantic dedup warning: inject similar existing intents
                        if !similar_intents.is_empty() {
                            let similar: Vec<Value> = similar_intents.iter().map(|(sim_id, sim_gk, sim_ctx, sim_score)| {
                                serde_json::json!({
                                    "id": sim_id,
                                    "group_key": sim_gk,
                                    "context_preview": sim_ctx.chars().take(100).collect::<String>(),
                                    "similarity": format!("{:.2}", sim_score),
                                })
                            }).collect();
                            resp["similar_existing_intents"] = serde_json::json!(similar);
                            resp["dedup_warning"] = serde_json::json!(
                                "IMPORTANT: Found semantically similar active recurring intent(s). \
                                 If this is a DUPLICATE, cancel the old intent(s) using cancel_intent. \
                                 If these are genuinely different tasks, no action needed."
                            );
                        }
                        resp.to_string()
                    }
                    Err(e) => sanitized_tool_error(
                        "deferred_intents",
                        "Failed to store intent / 存储意图失败",
                        &e,
                        "database_error",
                        true,
                        None,
                    ),
                }
            }
            Err(e) => serde_json::json!({"status": "error", "error": e.to_string()}).to_string(),
        }
    }

    pub(crate) async fn execute_list_intents(
        &self,
        _args: Value,
        ctx: &mut ToolContext<'_>,
    ) -> String {
        let user_did = ctx.user_did;

        match self.memory.list_user_intents(user_did).await {
            Ok(intents) => {
                let entries: Vec<Value> = intents
                    .iter()
                    .map(|i| {
                        let mut entry = serde_json::json!({
                            "id": i.id,
                            "trigger_type": i.trigger_type,
                            "trigger_at_unix": i.trigger_at_unix,
                            "event_key": i.event_key,
                            "execution_mode": i.execution_mode,
                            "context_text": i.context_text,
                            "created_at": i.created_at,
                            "expires_at_unix": i.expires_at_unix,
                        });
                        if let Some(interval) = i.interval_seconds {
                            entry["interval_seconds"] = serde_json::json!(interval);
                            entry["recurring"] = serde_json::json!(true);
                        }
                        if let Some(ref gk) = i.group_key {
                            entry["group_key"] = serde_json::json!(gk);
                        }
                        if let Some(max) = i.max_fires {
                            entry["max_fires"] = serde_json::json!(max);
                            entry["fire_count"] = serde_json::json!(i.fire_count);
                        }
                        if i.chain_depth > 0 {
                            entry["chain_depth"] = serde_json::json!(i.chain_depth);
                        }
                        entry
                    })
                    .collect();
                serde_json::json!({
                    "status": "ok",
                    "count": entries.len(),
                    "intents": entries,
                })
                .to_string()
            }
            Err(e) => sanitized_tool_error(
                "deferred_intents",
                "Failed to list intents / 查询意图失败",
                &e,
                "database_error",
                true,
                None,
            ),
        }
    }

    async fn execute_cancel_intent(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        let user_did = ctx.user_did;

        let intent_id = args.get("intent_id").and_then(|v| v.as_i64()).unwrap_or(0);
        if intent_id == 0 {
            return serde_json::json!({"status": "error", "error": "Missing or invalid 'intent_id'"}).to_string();
        }
        match self.memory.cancel_intent(intent_id, user_did).await {
            Ok(true) => serde_json::json!({
                "status": "ok",
                "message": format!("Intent #{} cancelled.", intent_id),
            }).to_string(),
            Ok(false) => serde_json::json!({
                "status": "error",
                "error": format!("Intent #{} not found, already cancelled, or belongs to another user.", intent_id),
            }).to_string(),
            Err(e) => sanitized_tool_error("cancel_intent", "Failed to cancel intent / 取消意图失败", &e, "database_error", true, None),
        }
    }

    async fn execute_manage_reminder(&self, _args: Value, _ctx: &mut ToolContext<'_>) -> String {
        serde_json::json!({
            "status": "error",
            "error": "manage_reminder is deprecated. Use manage_tasks(action='list') to see all tasks and scheduled reminders, or manage_tasks(action='complete', task_id=...) to dismiss.",
            "hint": "manage_tasks"
        }).to_string()
    }

    #[allow(clippy::too_many_arguments)]
    fn best_effort_task_creation_action_executed(
        &self,
        task_id: i64,
        action_kind: crate::action_ledger::ActionKind,
        chat_id: i64,
        user_did: &str,
        title: &str,
        body: &str,
        trigger_at_unix: Option<i64>,
        interval_seconds: Option<i64>,
        group_key: Option<&str>,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let conn = crate::encrypted_sqlite::open(self.memory.db_path())?;
            let store = crate::action_ledger::ActionLedgerStore::new(&conn);
            let action_id = match action_kind {
                crate::action_ledger::ActionKind::ReminderCreate => format!("reminder:{task_id}"),
                _ => format!("task:{task_id}"),
            };
            let receipt_prefix = match action_kind {
                crate::action_ledger::ActionKind::ReminderCreate => "reminder",
                _ => "task",
            };
            let title_summary = truncate_str(title, 120);
            let body_summary = (!body.trim().is_empty()).then(|| truncate_str(body, 160));
            let scheduled = trigger_at_unix.is_some() || group_key.is_some();

            store.create_proposed(&crate::action_ledger::NewAction {
                action_id: action_id.clone(),
                action_kind,
                requested_by: Some(user_did.to_string()),
                chat_id: Some(chat_id),
                input_summary: Some(match &body_summary {
                    Some(body) => format!("{title_summary} — {body}"),
                    None => title_summary.to_string(),
                }),
                user_visible_summary: Some(match action_kind {
                    crate::action_ledger::ActionKind::ReminderCreate => {
                        format!("Reminder created: {title_summary}")
                    }
                    _ => format!("Task created: {title_summary}"),
                }),
                idempotency_key: Some(action_id.clone()),
                metadata_json: Some({
                    let mut meta = serde_json::json!({
                        "task_id": task_id,
                        "trigger_at_unix": trigger_at_unix,
                        "interval_seconds": interval_seconds,
                        "group_key": group_key,
                        "scheduled": scheduled,
                        "schema_version": 1,
                    });
                    if let Some(turn_id) = crate::progress_router::current_turn_id() {
                        meta["origin_turn_id"] = serde_json::Value::String(turn_id);
                    }
                    meta.to_string()
                }),
                ..Default::default()
            })?;
            store.transition(
                &action_id,
                crate::action_ledger::ActionStatus::Executing,
                &crate::action_ledger::ActionUpdate {
                    tool_call_id: Some("manage_tasks".to_string()),
                    ..Default::default()
                },
            )?;
            store.transition(
                &action_id,
                crate::action_ledger::ActionStatus::Executed,
                &crate::action_ledger::ActionUpdate {
                    external_receipt: Some(format!("{receipt_prefix}:{task_id}")),
                    external_event_id: Some(task_id.to_string()),
                    tool_call_id: Some("manage_tasks".to_string()),
                    metadata_json: Some(
                        serde_json::json!({
                            "task_id": task_id,
                            "status": "stored",
                            "trigger_at_unix": trigger_at_unix,
                            "interval_seconds": interval_seconds,
                            "group_key": group_key,
                            "scheduled": scheduled,
                            "schema_version": 1,
                        })
                        .to_string(),
                    ),
                    ..Default::default()
                },
            )?;
            Ok(())
        })();

        if let Err(e) = result {
            tracing::warn!(
                error = %e,
                task_id,
                action_kind = action_kind.as_str(),
                "failed to record task creation in action ledger"
            );
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  Tests
// ══════════════════════════════════════════════════════════════════════════════

/// `email:*` event triggers have had no producer since the self-built mail
/// client left (2026-09-07); an event-triggered "watch the inbox" would sit
/// forever while the owner believes it is watching (task #2135, 2026-09-12).
/// Longest derived title. The full request stays in `context_text`/`body`, so
/// the title only has to be recognizable in a list.
pub(crate) const TASK_TITLE_MAX_CHARS: usize = 60;

/// A readable title from the owner's own words: the first sentence when it
/// fits, otherwise a cut at the last word/clause boundary with an ellipsis.
/// A hard `chars().take(60)` produced "…at 123 Example Stre" (a real task,
/// 2026-04): mid-word, and the owner could not tell it was truncated.
pub(crate) fn derive_task_title(context: &str, max_chars: usize) -> String {
    let text = context.trim().replace(['\n', '\r'], " ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max_chars {
        return text;
    }
    // First sentence, when it fits.
    let mut sentence_end: Option<usize> = None;
    let mut count = 0usize;
    for (idx, ch) in text.char_indices() {
        count += 1;
        if matches!(ch, '。' | '！' | '？' | '.' | '!' | '?') {
            let rest = &text[idx + ch.len_utf8()..];
            // CJK full stops are unambiguous; an ASCII '.' only ends a sentence
            // when a space (or the end of the text) follows it.
            let ends_here =
                matches!(ch, '。' | '！' | '？') || rest.is_empty() || rest.starts_with(' ');
            if ends_here && count >= 8 {
                sentence_end = Some(idx);
            }
            break;
        }
        if count > max_chars {
            break;
        }
    }
    if let Some(end) = sentence_end {
        return text[..end].trim_end().to_string();
    }
    // Otherwise cut at the last boundary before the limit.
    let cut_byte = text
        .char_indices()
        .nth(max_chars - 1)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    let head = &text[..cut_byte];
    let boundary = head
        .rfind([
            ' ', '，', '、', ',', ';', '；', ':', '：', '—', '-', '（', '(',
        ])
        .filter(|idx| head[..*idx].chars().count() >= max_chars / 3);
    let kept = match boundary {
        Some(idx) => &head[..idx],
        None => head,
    };
    format!(
        "{}…",
        kept.trim_end()
            .trim_end_matches([',', '，', '、', ';', '；', ':', '：', '-', '—'])
    )
}

pub(crate) fn unsupported_event_trigger_error(event_key: &str) -> Option<String> {
    let key = event_key.trim();
    // No event source exists any more (the mail client was the only producer
    // and left on 2026-09-07): an event task of ANY key would sit "active"
    // forever while the owner believes something is being watched.
    let reason = if key.starts_with("email:") {
        format!(
            "event trigger '{key}' has no producer: Aegis Runtime no longer receives mailbox events"
        )
    } else {
        format!(
            "event trigger '{key}' has no producer: no event source is connected, so it could never fire"
        )
    };
    Some(
        serde_json::json!({
            "status": "error",
            "error": reason,
            "hint": "To watch a mailbox, schedule a TIME trigger (trigger_at + interval_seconds, execution_mode=\"llm_callback\") whose context_text runs run_external(executor=\"codex\", connector=\"Gmail\") with a read-only check. Access must be configured first. Tell the owner it is a periodic check, not instant.",
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::memory::MemoryHandle;
    use crate::orchestrator::Orchestrator;
    use crate::settings::RuntimeSettings;
    use crate::test_tool_helpers::{CtxState, make_tool_call, test_orchestrator};

    fn file_backed_test_orchestrator() -> (tempfile::NamedTempFile, Orchestrator) {
        let db = tempfile::NamedTempFile::new().expect("temp db");
        let mem = MemoryHandle::spawn(db.path()).expect("file-backed DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);
        (db, orch)
    }

    // ── Routing ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn tasks_unknown_tool_passes_through() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("personal_tasks", "{}");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_tasks(&tc, "personal_tasks", "{}", &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ── manage_reminder deprecation ──────────────────────────────────────────

    #[tokio::test]
    async fn manage_reminder_returns_deprecation() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("manage_reminder", "{}");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_tasks(&tc, "manage_reminder", "{}", &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("deprecated") || content.contains("manage_tasks"),
            "must report deprecation, got: {content}"
        );
    }

    // ── manage_tasks error paths ─────────────────────────────────────────────

    #[tokio::test]
    async fn manage_tasks_bad_json_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("manage_tasks", "{{invalid");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_tasks(&tc, "manage_tasks", "{{invalid", &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("JSON") || content.contains("parse") || content.contains("error"));
    }

    #[tokio::test]
    async fn manage_tasks_list_action() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"list"}"#;
        let tc = make_tool_call("manage_tasks", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_tasks(&tc, "manage_tasks", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        // Should return a list (possibly empty) without error
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("ok") || content.contains("tasks") || content.contains("status"),
            "list action should succeed, got: {content}"
        );
    }

    /// Extract the text content of the most recent tool-result message.
    fn last_tool_content(state: &CtxState) -> String {
        use async_openai::types::chat::{
            ChatCompletionRequestMessage, ChatCompletionRequestToolMessageContent,
        };
        match state.messages.last().expect("a tool-result message") {
            ChatCompletionRequestMessage::Tool(m) => match &m.content {
                ChatCompletionRequestToolMessageContent::Text(s) => s.clone(),
                other => panic!("unexpected tool content variant: {other:?}"),
            },
            other => panic!("expected a Tool message, got: {other:?}"),
        }
    }

    /// manage_tasks(action='list') must number entries with a sequential 1-based
    /// `index` in the exact order they are returned. This `index` is the shared,
    /// stable ordinal the owner refers to when they say e.g. "3完成"; the WebApp
    /// /tasks panel renders the same list_tasks(active) order with the same number.
    #[tokio::test]
    async fn manage_tasks_list_includes_sequential_index() {
        let (_db, orch) = file_backed_test_orchestrator();

        // Add three tasks across two priority buckets so the list spans more than
        // one ORDER BY group (priority bucket, then id DESC).
        for args in [
            r#"{"action":"add","title":"Buy underwear","priority":"normal"}"#,
            r#"{"action":"add","title":"Improve Vision Architect","priority":"high"}"#,
            r#"{"action":"add","title":"Pay rent","priority":"normal"}"#,
        ] {
            let tc = make_tool_call("manage_tasks", args);
            let mut state = CtxState::new();
            orch.handle_tool_tasks(&tc, "manage_tasks", args, &mut state.ctx())
                .await
                .expect("add should not error");
        }

        let args = r#"{"action":"list"}"#;
        let tc = make_tool_call("manage_tasks", args);
        let mut state = CtxState::new();
        orch.handle_tool_tasks(&tc, "manage_tasks", args, &mut state.ctx())
            .await
            .expect("list should not error");

        let json: serde_json::Value =
            serde_json::from_str(&last_tool_content(&state)).expect("list output is JSON");
        let tasks = json["tasks"].as_array().expect("tasks array");
        assert_eq!(tasks.len(), 3, "all three tasks listed");

        // `index` must be exactly 1..=N in the returned order, with no gaps/dupes,
        // and each `index` must pair with a real `id`.
        for (pos, entry) in tasks.iter().enumerate() {
            assert_eq!(
                entry["index"].as_u64(),
                Some((pos + 1) as u64),
                "entry at position {pos} must carry index {}",
                pos + 1
            );
            assert!(entry["id"].as_i64().is_some(), "entry must keep its id");
        }

        // The high-priority task sorts ahead of the normal ones, so index 1 is the
        // one the owner would eyeball first — proving the ordinal tracks the real
        // sort order, not insertion order.
        assert_eq!(tasks[0]["title"], "Improve Vision Architect");
        assert_eq!(tasks[0]["index"], 1);
    }

    #[tokio::test]
    async fn manage_tasks_add_records_task_create_action_ledger() {
        let (db, orch) = file_backed_test_orchestrator();

        let args = r#"{"action":"add","title":"Review server logs","body":"Check the overnight error rate","priority":"high"}"#;
        let tc = make_tool_call("manage_tasks", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_tasks(&tc, "manage_tasks", args, &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, Some(true));

        let conn = crate::encrypted_sqlite::open(db.path()).expect("open db");
        let entries = crate::action_ledger::ActionLedgerStore::new(&conn)
            .list_recent_by_status(crate::action_ledger::ActionStatus::Executed, 5)
            .expect("query action ledger");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(
            entry.action_kind,
            crate::action_ledger::ActionKind::TaskCreate
        );
        assert_eq!(entry.requested_by.as_deref(), Some("did:test:user"));
        assert_eq!(entry.chat_id, Some(12345));
        assert_eq!(entry.external_receipt.as_deref(), Some("task:1"));
        assert_eq!(entry.external_event_id.as_deref(), Some("1"));
        assert_eq!(entry.tool_call_id.as_deref(), Some("manage_tasks"));
        assert_eq!(entry.idempotency_key.as_deref(), Some("task:1"));
        assert!(
            entry
                .user_visible_summary
                .as_deref()
                .unwrap_or_default()
                .contains("Review server logs")
        );
    }

    #[tokio::test]
    async fn manage_tasks_schedule_records_reminder_create_action_ledger() {
        let (db, orch) = file_backed_test_orchestrator();

        let args = r#"{"action":"schedule","title":"Stand up","context_text":"Remind me to stand up","delay_seconds":3600,"execution_mode":"notify"}"#;
        let tc = make_tool_call("manage_tasks", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_tasks(&tc, "manage_tasks", args, &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, Some(true));

        let conn = crate::encrypted_sqlite::open(db.path()).expect("open db");
        let entries = crate::action_ledger::ActionLedgerStore::new(&conn)
            .list_recent_by_status(crate::action_ledger::ActionStatus::Executed, 5)
            .expect("query action ledger");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(
            entry.action_kind,
            crate::action_ledger::ActionKind::ReminderCreate
        );
        assert_eq!(entry.external_receipt.as_deref(), Some("reminder:1"));
        assert_eq!(entry.external_event_id.as_deref(), Some("1"));
        let metadata: serde_json::Value =
            serde_json::from_str(entry.metadata_json.as_deref().unwrap_or("{}"))
                .expect("metadata JSON");
        assert_eq!(metadata["task_id"], serde_json::json!(1));
        assert_eq!(metadata["status"], serde_json::json!("stored"));
        assert_eq!(metadata["scheduled"], serde_json::json!(true));
        assert!(metadata["trigger_at_unix"].as_i64().is_some());
        assert_eq!(metadata["schema_version"], serde_json::json!(1));
    }

    // ── manage_tasks update action — NOT NULL preservation ──────────────────
    //
    // Regression: production bug where an LLM call like
    //   {"action":"update","task_id":934,"title":"..."}
    // failed with `NOT NULL constraint failed: tasks.body`. The handler was
    // building updates via `args.get(k)` for every field, materializing
    // `null` for absent keys, which memory.rs then turned into `SET body=NULL`.

    #[tokio::test]
    async fn manage_tasks_update_title_only_preserves_body() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);

        // Seed a task with a real body.
        let chat_id: i64 = 12345;
        let task_id = mem
            .store_task(
                chat_id,
                "did:test:user",
                "Original title",
                "original body",
                "normal",
                &[],
                None,
                "user",
                None,
                None,
                None,
                "notify",
                None,
                None,
                None,
                None,
                false,
                None,
                false,
                0,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("store_task");

        let args = format!(r#"{{"action":"update","task_id":{task_id},"title":"new title"}}"#);
        let tc = make_tool_call("manage_tasks", &args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_tasks(&tc, "manage_tasks", &args, &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, Some(true));

        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("\\\"status\\\":\\\"ok\\\"") || content.contains("status\":\"ok"),
            "update should succeed, got: {content}"
        );

        let task = mem.get_task(task_id).await.expect("task exists");
        assert_eq!(task.title, "new title");
        assert_eq!(
            task.body, "original body",
            "body must be preserved when only title is updated"
        );
    }

    #[test]
    fn derived_titles_cut_on_a_boundary_and_say_so() {
        // A real stored task (2026-04) ended "…at 123 Example Stre": a hard
        // 60-char cut mid-word with no ellipsis. The fixture is synthetic.
        let long = "Weekly physiotherapy session (Sample Person) at 123 Example Street Northeast, Suite 105, Example City, WA. reminder: 1 hour before and at time to leave.";
        let title = super::derive_task_title(long, super::TASK_TITLE_MAX_CHARS);
        assert!(
            title.chars().count() <= super::TASK_TITLE_MAX_CHARS + 1,
            "{title}"
        );
        assert!(title.ends_with('…'), "{title}");
        assert!(!title.ends_with("Nort…"), "must not cut mid-word: {title}");
        assert!(title.starts_with("Weekly physiotherapy session"), "{title}");

        // Short text is kept verbatim.
        assert_eq!(super::derive_task_title("打电话给爸妈", 60), "打电话给爸妈");
        // A first sentence that fits wins over the raw cut.
        let two = "提醒我周四去体检。地址是某某医院门诊楼三层，记得带医保卡和身份证，最好提前半小时到，不要忘记空腹。";
        assert_eq!(super::derive_task_title(two, 30), "提醒我周四去体检");
        // CJK without punctuation still cuts inside the budget with an ellipsis.
        let cjk =
            "每周一上午十点提醒我复盘上周的工作进展并整理成一页纸的要点发给团队所有人抄送经理";
        let cut = super::derive_task_title(cjk, 30);
        assert!(cut.chars().count() <= 31, "{cut}");
        assert!(cut.ends_with('…'), "{cut}");
        // Whitespace is normalized.
        assert_eq!(
            super::derive_task_title("  hello   world  ", 60),
            "hello world"
        );
    }

    #[tokio::test]
    async fn manage_tasks_reopen_restores_a_completed_task() {
        // Owner turn 2026-09-13: "恢复第36项" after a mis-tap on Done. `complete`
        // had no inverse, so 安琪 offered a restore no action could deliver.
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let chat_id: i64 = 12345;
        let task_id = mem
            .store_task(
                chat_id,
                "did:test:user",
                "Weekly physiotherapy session",
                "",
                "normal",
                &[],
                None,
                "user",
                None,
                None,
                None,
                "notify",
                None,
                None,
                None,
                None,
                false,
                None,
                false,
                0,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("store_task");

        let mut state = CtxState::new();
        let args = format!(r#"{{"action":"complete","task_id":{task_id}}}"#);
        let tc = make_tool_call("manage_tasks", &args);
        orch.handle_tool_tasks(&tc, "manage_tasks", &args, &mut state.ctx())
            .await
            .expect("complete should not error");
        let done = mem.get_task(task_id).await.expect("task exists");
        assert_eq!(done.status, "done");
        assert!(done.completed_at.is_some(), "complete stamps completed_at");

        let args = format!(r#"{{"action":"reopen","task_id":{task_id}}}"#);
        let tc = make_tool_call("manage_tasks", &args);
        let result = orch
            .handle_tool_tasks(&tc, "manage_tasks", &args, &mut state.ctx())
            .await
            .expect("reopen should not error");
        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("reopened"),
            "reopen should report success, got: {content}"
        );

        let task = mem.get_task(task_id).await.expect("task exists");
        assert_eq!(task.status, "active");
        assert!(task.completed_at.is_none(), "reopen clears completed_at");
        assert_eq!(task.title, "Weekly physiotherapy session");

        let args = r#"{"action":"reopen","task_id":999999}"#;
        let tc = make_tool_call("manage_tasks", args);
        orch.handle_tool_tasks(&tc, "manage_tasks", args, &mut state.ctx())
            .await
            .expect("missing task should not error");
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("Task not found"), "got: {content}");
    }

    #[tokio::test]
    async fn manage_tasks_update_explicit_null_body_preserved() {
        // Defensive: even if a caller explicitly passes `{"body": null}`, the
        // memory.rs maybe_set! macro must skip it (absent = preserve semantic).
        // This guards against future handler regressions.
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);

        let chat_id: i64 = 12345;
        let task_id = mem
            .store_task(
                chat_id,
                "did:test:user",
                "T",
                "do not clear me",
                "normal",
                &[],
                None,
                "user",
                None,
                None,
                None,
                "notify",
                None,
                None,
                None,
                None,
                false,
                None,
                false,
                0,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("store_task");

        // Bypass the handler guard by talking to memory directly — this tests
        // the memory.rs defensive layer in isolation.
        let updates = serde_json::json!({"title": "updated", "body": null});
        let updated = mem
            .update_task(task_id, chat_id, &updates.to_string())
            .await
            .expect("update_task");
        assert!(updated);

        let task = mem.get_task(task_id).await.expect("task exists");
        assert_eq!(task.title, "updated");
        assert_eq!(
            task.body, "do not clear me",
            "explicit null body must be skipped, not written as NULL"
        );

        // Also sanity-check the handler path: a handler call carrying an
        // explicit null must not reach memory as NULL.
        let args = format!(
            r#"{{"action":"update","task_id":{task_id},"body":null,"title":"via handler"}}"#
        );
        let tc = make_tool_call("manage_tasks", &args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_tasks(&tc, "manage_tasks", &args, &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, Some(true));

        let task2 = mem.get_task(task_id).await.expect("task exists");
        assert_eq!(task2.title, "via handler");
        assert_eq!(task2.body, "do not clear me");
    }

    #[tokio::test]
    async fn manage_tasks_update_empty_string_body_allowed() {
        // Explicit empty string is intentional — caller wants to clear the body
        // visually. Since body is `TEXT NOT NULL DEFAULT ''`, an empty string
        // satisfies the constraint and expresses "erase the body content".
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);

        let chat_id: i64 = 12345;
        let task_id = mem
            .store_task(
                chat_id,
                "did:test:user",
                "T",
                "some body",
                "normal",
                &[],
                None,
                "user",
                None,
                None,
                None,
                "notify",
                None,
                None,
                None,
                None,
                false,
                None,
                false,
                0,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("store_task");

        let args = format!(r#"{{"action":"update","task_id":{task_id},"body":""}}"#);
        let tc = make_tool_call("manage_tasks", &args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_tasks(&tc, "manage_tasks", &args, &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, Some(true));

        let task = mem.get_task(task_id).await.expect("task exists");
        assert_eq!(
            task.body, "",
            "explicit empty string must be applied (distinct from null)"
        );
    }

    // ── schedule_intent bad JSON ─────────────────────────────────────────────

    #[tokio::test]
    async fn schedule_intent_bad_json_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("schedule_intent", "NOT_JSON");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_tasks(&tc, "schedule_intent", "NOT_JSON", &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("JSON") || content.contains("parse") || content.contains("error"));
    }

    #[tokio::test]
    async fn schedule_intent_windowed_persists_recur_fields_for_resched() {
        // Codex F5 regression: the schedule_intent path must PERSIST the recur_*
        // fields, not just snap the first trigger. Otherwise the stored row has
        // NULL recur columns and every SUBSEQUENT fire (rescheduled via
        // next_windowed_trigger, which then sees no constraints) escapes the
        // window. This drove the call site that previously passed None×4 into
        // store_deferred_intent.
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);

        let args = r#"{"trigger_type":"time","delay_seconds":3600,"context_text":"commute estimate","execution_mode":"notify","interval_seconds":1200,"group_key":"commute-f5","recur_weekdays":[2,3,4],"recur_window_start":"17:30","recur_window_end":"18:30","recur_timezone":"America/Los_Angeles"}"#;
        let tc = make_tool_call("schedule_intent", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_tasks(&tc, "schedule_intent", args, &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, Some(true));

        // Pull the stored intent_id out of the tool result and read the row back.
        let content = format!("{:?}", state.messages.last().unwrap());
        let marker = r#"intent_id\":"#;
        let start = content.find(marker).expect("intent_id in result") + marker.len();
        let rest = &content[start..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let id: i64 = rest[..end].parse().expect("numeric intent_id");

        let task = mem.get_task(id).await.expect("stored intent row exists");
        assert_eq!(
            task.recur_weekdays,
            Some(vec![2, 3, 4]),
            "schedule_intent must persist recur_weekdays for subsequent in-window fires"
        );
        assert_eq!(task.recur_window_start.as_deref(), Some("17:30"));
        assert_eq!(task.recur_window_end.as_deref(), Some("18:30"));
        assert_eq!(task.recur_timezone.as_deref(), Some("America/Los_Angeles"));
    }

    #[tokio::test]
    async fn schedule_intent_success_returns_owner_facing_evidence_fields() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"trigger_type":"time","delay_seconds":3600,"context_text":"关注 Grok Build agent；有意义的新进展才提醒","execution_mode":"notify","interval_seconds":43200,"group_key":"grok-build-watch","max_fires":3}"#;
        let tc = make_tool_call("schedule_intent", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_tasks(&tc, "schedule_intent", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains(r#"\"intent_id\":"#), "{content}");
        assert!(
            content.contains(r#"\"context_text\":\"关注 Grok Build agent；有意义的新进展才提醒\""#),
            "{content}"
        );
        assert!(
            content.contains(r#"\"trigger_type\":\"time\""#),
            "{content}"
        );
        assert!(content.contains(r#"\"trigger_at_unix\":"#), "{content}");
        assert!(content.contains(r#"\"expires_at_unix\":"#), "{content}");
        assert!(
            content.contains(r#"\"interval_seconds\":43200"#),
            "{content}"
        );
        assert!(content.contains(r#"\"max_fires\":3"#), "{content}");
    }
    #[tokio::test]
    async fn manage_tasks_recovers_recent_recurring_start_but_rejects_stale_one_shot() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));
        let start = chrono::Utc::now() - chrono::Duration::seconds(48);
        let mut args = serde_json::json!({"title":"Commute monitor", "context_text":"Check commute",
            "trigger_at":start.to_rfc3339(), "interval_seconds":1200, "group_key":"commute-latency"});
        let result: serde_json::Value = serde_json::from_str(
            &orch
                .task_action_schedule(&args, 12345678, "did:aegis:telegram:12345678")
                .await,
        )
        .unwrap();
        assert_eq!(result["status"], "ok");
        assert!(result["warning"].as_str().unwrap().contains("No catch-up"));
        let task = mem
            .get_task(result["task_id"].as_i64().unwrap())
            .await
            .unwrap();
        assert_eq!(task.trigger_at_unix, result["trigger_at_unix"].as_i64());
        assert!(task.trigger_at_unix.unwrap() > chrono::Utc::now().timestamp());
        args.as_object_mut().unwrap().remove("interval_seconds");
        args.as_object_mut().unwrap().remove("group_key");
        let error: serde_json::Value = serde_json::from_str(
            &orch
                .task_action_schedule(&args, 12345678, "did:aegis:telegram:12345678")
                .await,
        )
        .unwrap();
        assert_eq!(error["status"], "error");
        assert!(
            error["hint"]
                .as_str()
                .unwrap()
                .contains("Do not retry the same expired timestamp")
        );
        assert!(error.get("task_id").is_none());
    }
}
