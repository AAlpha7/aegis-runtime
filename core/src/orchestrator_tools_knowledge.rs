//! Knowledge base + Google account tool handlers extracted from `orchestrator.rs`.
//!
//! Contains `handle_tool_knowledge` (knowledge_base) and
//! `handle_tool_email` (setup_google, google_drive_upload).

use anyhow::Result;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use async_openai::types::chat::ChatCompletionMessageToolCall;

use crate::assistant_events::build_drive_file_uploaded_event;
use crate::memory;
use crate::orchestrator::{Orchestrator, ToolContext};
use crate::source_guards::{EmailSourceWriteKind, validate_email_source_write};
use crate::utils::{kb_entry_to_json, sanitized_tool_error, truncate_str};

const KB_DASHBOARD_RECENT_DAYS: i64 = 14;
const KB_HYGIENE_STALE_L2_DAYS: i64 = 30;

fn normalize_dashboard_text(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect()
}

fn strings_substantially_overlap(left: &str, right: &str) -> bool {
    let left = normalize_dashboard_text(left);
    let right = normalize_dashboard_text(right);
    if left.len() < 6 || right.len() < 6 {
        return false;
    }
    left.contains(&right) || right.contains(&left)
}

fn task_and_entry_look_duplicate(task: &memory::Task, entry: &memory::KnowledgeEntry) -> bool {
    // Only compare subject/content fields (title, body). We deliberately skip:
    // - `entry.waiting_on` — it holds the *person* (e.g. "Sample Person") not
    //   the subject of the waiting item, so matching it against task.title
    //   would dedup every unrelated entry that happens to mention the same
    //   person.
    // - `tags` (both sides) — tags are categories, not content. A task tagged
    //   "legalreview" must not silently suppress any entry whose title/body
    //   happens to contain the substring "legalreview".
    let task_fields = [task.title.as_str(), task.body.as_str()];
    let entry_fields = [entry.title.as_str(), entry.body.as_str()];
    task_fields
        .iter()
        .filter(|s| !s.is_empty())
        .any(|task_text| {
            entry_fields
                .iter()
                .filter(|s| !s.is_empty())
                .any(|entry_text| strings_substantially_overlap(task_text, entry_text))
        })
}

fn duplicate_task_entry_samples(
    tasks: &[memory::Task],
    entries: &[memory::KnowledgeEntry],
    limit: usize,
) -> Vec<Value> {
    let mut samples = Vec::new();
    for entry in entries {
        for task in tasks {
            if !task_and_entry_look_duplicate(task, entry) {
                continue;
            }
            samples.push(serde_json::json!({
                "task": {
                    "id": task.id,
                    "status": &task.status,
                    "priority": &task.priority,
                },
                "entry": {
                    "id": entry.id,
                    "category": &entry.category,
                    "status": &entry.status,
                    "priority": &entry.priority,
                },
                "reason": "task and KB entry title/body substantially overlap",
                "review_actions": ["keep_both", "archive_entry", "restore_entry"],
            }));
            if samples.len() >= limit {
                return samples;
            }
            break;
        }
    }
    samples
}

fn kb_source_entry_ids(args: &Value) -> Vec<i64> {
    args.get("source_entry_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect())
        .unwrap_or_default()
}

fn kb_not_found_json(entry_id: i64) -> String {
    serde_json::json!({
        "status": "error",
        "error": format!("Entry #{} not found for this chat.", entry_id),
        "error_kind": "not_found",
        "retryable": false,
    })
    .to_string()
}

fn kb_cleanup_entry_preview(entry: &memory::KnowledgeEntry) -> Value {
    serde_json::json!({
        "entry_id": entry.id,
        "category": &entry.category,
        "priority": &entry.priority,
        "status": &entry.status,
        "title": &entry.title,
        "body_preview": truncate_str(&entry.body, 240),
        "tags": &entry.tags,
        "related_ids": &entry.related_ids,
        "superseded_by": entry.superseded_by,
        "created_at": &entry.created_at,
        "updated_at": &entry.updated_at,
        "resolved_at": &entry.resolved_at,
    })
}

fn kb_cleanup_preview_token(action: &str, chat_id: i64, preview: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(action.as_bytes());
    hasher.update(b"\0");
    hasher.update(chat_id.to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(preview.to_string().as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
    format!("kbp_{hex}")
}

fn kb_cleanup_confirm_token_error(action: &str, expected_token: &str) -> String {
    serde_json::json!({
        "status": "error",
        "error": format!("{action} requires preview_token from the matching preview action plus confirm=true."),
        "error_kind": "preview_token_required",
        "retryable": false,
        "expected_preview_token": expected_token,
    })
    .to_string()
}

fn text_contains_ambiguous_date(text: &str) -> bool {
    // Structural detector (no keyword lists — see feedback_no_static_matching):
    // an "ambiguous date placeholder" is a date-shaped token paired with a
    // literal `?`/`？` somewhere in the text, signalling the author was unsure.
    // Matches NN/NN, NN-NN, NN/NN/NN, NN-NN-NN, NNNN/NN/NN, NNNN-NN-NN, or
    // `数字月数字日` (Chinese date). Never triggers without the `?` marker,
    // so plain dates in confirmed knowledge entries stay unflagged.
    if !(text.contains('?') || text.contains('？')) {
        return false;
    }
    use std::sync::OnceLock;
    static DATE_SHAPE: OnceLock<regex::Regex> = OnceLock::new();
    let re = DATE_SHAPE.get_or_init(|| {
        regex::Regex::new(r"\b\d{1,4}[/-]\d{1,2}(?:[/-]\d{1,4})?\b|\d{1,2}月\d{1,2}日")
            .expect("ambiguous-date regex compiles")
    });
    re.is_match(text)
}

fn entry_has_ambiguous_date(entry: &memory::KnowledgeEntry) -> bool {
    text_contains_ambiguous_date(&entry.title)
        || text_contains_ambiguous_date(&entry.body)
        || entry
            .waiting_on
            .as_deref()
            .map(text_contains_ambiguous_date)
            .unwrap_or(false)
}

fn task_has_ambiguous_date(task: &memory::Task) -> bool {
    text_contains_ambiguous_date(&task.title) || text_contains_ambiguous_date(&task.body)
}

fn goal_has_ambiguous_date(goal: &memory::Goal) -> bool {
    text_contains_ambiguous_date(&goal.title)
        || text_contains_ambiguous_date(&goal.description)
        || goal
            .target_date
            .as_deref()
            .map(text_contains_ambiguous_date)
            .unwrap_or(false)
}

fn retain_recent_entries(
    entries: Vec<memory::KnowledgeEntry>,
    max_age_days: i64,
) -> Vec<memory::KnowledgeEntry> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(max_age_days);
    entries
        .into_iter()
        .filter(|entry| {
            chrono::DateTime::parse_from_rfc3339(&entry.created_at)
                .map(|dt| dt.with_timezone(&chrono::Utc) >= cutoff)
                .unwrap_or(true)
        })
        .collect()
}

impl Orchestrator {
    /// Handle knowledge domain tools: knowledge_base.
    #[allow(unused_variables)]
    pub(crate) async fn handle_tool_knowledge(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "knowledge_base"     => execute_knowledge_base,
        );
        // PSEUDO-TOOL INTERCEPT: check_api_usage (OpenRouter credits + activity)
        Ok(None)
    }

    /// Handle Google account domain tools: setup_google, google_drive_upload.
    #[allow(unused_variables)]
    pub(crate) async fn handle_tool_email(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "setup_google"        => execute_setup_google,
            "google_drive_upload" => execute_google_drive_upload,
        );
        // PSEUDO-TOOL INTERCEPT: activate_agent_skill (load external skill content)
        Ok(None)
    }

    // ──────────────────────────────────────────────────────────────────────
    //  Knowledge tool handlers
    // ──────────────────────────────────────────────────────────────────────

    async fn execute_knowledge_base(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let cid = ctx.chat_id.unwrap_or(0);
        match action {
            "extract" => self.kb_action_extract(&args, cid, ctx.user_did).await,
            "query" => {
                // "all"/"any" = no filter (a literal `status = 'all'` matched nothing).
                fn unfiltered(v: Option<&str>) -> Option<&str> {
                    v.map(str::trim).filter(|s| {
                        !s.is_empty() && !matches!(s.to_ascii_lowercase().as_str(), "all" | "any" | "*")
                    })
                }
                let cat = unfiltered(args.get("category").and_then(|v| v.as_str()));
                let status = unfiltered(args.get("status").and_then(|v| v.as_str()));
                let prio = unfiltered(args.get("priority").and_then(|v| v.as_str()));
                let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(5);
                let entries = self.memory.query_knowledge_entries(
                    cid, cat, status, prio, limit.min(20),
                ).await;
                let items: Vec<Value> = entries.iter().map(kb_entry_to_json).collect();
                serde_json::json!({
                    "status": "ok",
                    "count": items.len(),
                    "entries": items,
                }).to_string()
            }
            "search" => {
                let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                if q.is_empty() {
                    serde_json::json!({"status": "error", "error": "query required"}).to_string()
                } else {
                    let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(5);
                    let entries = self.memory.search_knowledge_entries(cid, q, limit.min(20)).await;
                    let items: Vec<Value> = entries.iter().map(kb_entry_to_json).collect();
                    serde_json::json!({
                        "status": "ok",
                        "count": items.len(),
                        "entries": items,
                    }).to_string()
                }
            }
            "update" => {
                let entry_id = args.get("entry_id").and_then(|v| v.as_i64()).unwrap_or(0);
                if entry_id == 0 {
                    serde_json::json!({"status": "error", "error": "entry_id required"}).to_string()
                } else {
                    let mut updates = memory::KnowledgeEntryUpdate::default();
                    if let Some(s) = args.get("status").and_then(|v| v.as_str()) {
                        updates.status = Some(s.to_string());
                        if s == "done" || s == "archived" {
                            updates.resolved_at = Some(Some(chrono::Utc::now().to_rfc3339()));
                        }
                    }
                    if let Some(p) = args.get("priority").and_then(|v| v.as_str()) {
                        updates.priority = Some(p.to_string());
                    }
                    if let Some(t) = args.get("title").and_then(|v| v.as_str()) {
                        updates.title = Some(t.to_string());
                    }
                    if let Some(b) = args.get("body").and_then(|v| v.as_str()) {
                        updates.body = Some(b.to_string());
                    }
                    if let Some(d) = args.get("due_date") {
                        updates.due_date = Some(d.as_str().map(String::from));
                    }
                    if let Some(w) = args.get("waiting_on") {
                        updates.waiting_on = Some(w.as_str().map(String::from));
                    }
                    if let Some(tags) = args.get("tags").and_then(|v| v.as_array()) {
                        updates.tags = Some(tags.iter().filter_map(|v| v.as_str().map(String::from)).collect());
                    }
                    if let Some(parent_id) = self.callback_tracking_parent_for_chat(cid).await
                        && let Some(existing) = self.memory.get_knowledge_entry(entry_id, cid).await
                        && existing.category == "waiting_for"
                        && !updates.status.as_deref().is_some_and(|status| {
                            matches!(status, "done" | "archived" | "superseded")
                        })
                    {
                        let mut tags = updates.tags.clone().unwrap_or_else(|| existing.tags.clone());
                        memory::ensure_tracking_parent_tag(&mut tags, parent_id);
                        updates.tags = Some(tags);
                    }
                    match self.memory.update_knowledge_entry(entry_id, cid, updates).await {
                        Ok(true) => serde_json::json!({"status": "ok", "message": format!("Entry #{entry_id} updated.")}).to_string(),
                        Ok(false) => serde_json::json!({"status": "error", "error": format!("Entry #{entry_id} not found.")}).to_string(),
                        Err(e) => {
                            tracing::warn!(tool = "knowledge_base", entry_id, error = %e, "Failed to update KB entry");
                            serde_json::json!({
                                "status": "error",
                                "error": format!("Failed to update entry #{} / 更新条目失败", entry_id),
                                "error_kind": "database_error",
                                "retryable": true,
                                "hint": "The entry may not exist. Use action='query' to check."
                            }).to_string()
                        }
                    }
                }
            }
            "dashboard" => self.kb_action_dashboard(cid, ctx.user_did).await,
            "keep_entry" => self.kb_action_keep_entry(&args, cid).await,
            "archive_entry" => self.kb_action_archive_entry(&args, cid).await,
            "restore_entry" => self.kb_action_restore_entry(&args, cid).await,
            "preview_merge_entries" => self.kb_action_preview_merge_entries(&args, cid).await,
            "merge_entries" => {
                self.kb_action_merge_entries(&args, cid, ctx.user_did).await
            }
            "preview_delete_entry" => self.kb_action_preview_delete_entry(&args, cid).await,
            "delete_entry" => self.kb_action_delete_entry(&args, cid, ctx.user_did).await,
            "dismiss_lint_finding" | "resolve_lint_finding" => {
                self.kb_action_review_lint_finding(&args, ctx.user_did, action)
                    .await
            }
            _ => serde_json::json!({
                "status": "error",
                "error": format!("Unknown action '{action}'. Use: extract, query, search, update, dashboard, keep_entry, archive_entry, restore_entry, preview_merge_entries, merge_entries, preview_delete_entry, delete_entry, dismiss_lint_finding, resolve_lint_finding"),
            }).to_string(),
        }
    }

    // ── KB action helpers (extracted from execute_knowledge_base) ──────────

    /// Recurring callback that is writing into this chat, if any.
    /// `callback_task_info` is set only when the deferred intent has an interval.
    async fn callback_tracking_parent_for_chat(&self, chat_id: i64) -> Option<i64> {
        let info = *self.tool_exec.callback_task_info.lock().await;
        info.and_then(|(task_id, source_chat)| {
            (task_id > 0 && source_chat == chat_id).then_some(task_id)
        })
    }

    /// Handle `knowledge_base` action="extract": store structured entries from conversation.
    async fn kb_action_extract(&self, args: &Value, cid: i64, user_did: &str) -> String {
        let entries = args.get("entries").and_then(|v| v.as_array());
        match entries {
            Some(arr) if !arr.is_empty() => {
                let parent_task = self.callback_tracking_parent_for_chat(cid).await;
                let mut stored = Vec::new();
                let mut errors = Vec::new();
                for entry in arr.iter().take(10) {
                    let cat = entry
                        .get("category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("insight");
                    // Redirect action_item to unified tasks table
                    if cat == "action_item" {
                        let title = entry.get("title").and_then(|v| v.as_str()).unwrap_or("");
                        let body = entry.get("body").and_then(|v| v.as_str()).unwrap_or("");
                        let prio = entry
                            .get("priority")
                            .and_then(|v| v.as_str())
                            .unwrap_or("normal");
                        let due = entry.get("due_date").and_then(|v| v.as_str());
                        let tags: Vec<String> = entry
                            .get("tags")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let src = entry.get("source_summary").and_then(|v| v.as_str());
                        if !title.is_empty() {
                            let tags_text = tags.join(" ");
                            if let Err(e) = validate_email_source_write(
                                &[title, body, src.unwrap_or(""), tags_text.as_str()],
                                EmailSourceWriteKind::FollowUpAllowed,
                            ) {
                                errors.push(e.into());
                                continue;
                            }
                            match self.memory.store_task(
                                cid, user_did, title, body, prio, &tags, due, "llm",
                                None, None, None, "notify", None, None, None, None, false, None, false, 0,
                                None, None, None, None,
                            ).await {
                                Ok(id) => stored.push(serde_json::json!({"id": id, "title": title, "redirected": "tasks"})),
                                Err(e) => {
                                    tracing::warn!(tool = "knowledge_base", title, error = %e, "Failed to store action item");
                                    errors.push("Failed to store action item / 存储待办事项失败".into());
                                }
                            }
                        }
                        continue;
                    }
                    if !memory::KB_CATEGORIES.contains(&cat) {
                        errors.push(format!("invalid category '{cat}'"));
                        continue;
                    }
                    let prio = entry
                        .get("priority")
                        .and_then(|v| v.as_str())
                        .unwrap_or("normal");
                    let title = entry.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let body = entry.get("body").and_then(|v| v.as_str()).unwrap_or("");
                    if title.is_empty() || body.is_empty() {
                        errors.push("title and body required".into());
                        continue;
                    }
                    let mut tags: Vec<String> = entry
                        .get("tags")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let due = entry.get("due_date").and_then(|v| v.as_str());
                    let waiting = entry.get("waiting_on").and_then(|v| v.as_str());
                    let src = entry.get("source_summary").and_then(|v| v.as_str());
                    let tags_text = tags.join(" ");
                    let write_kind = match cat {
                        "decision" | "insight" | "metric" => {
                            EmailSourceWriteKind::DeterministicFact
                        }
                        _ => EmailSourceWriteKind::FollowUpAllowed,
                    };
                    if let Err(e) = validate_email_source_write(
                        &[
                            title,
                            body,
                            src.unwrap_or(""),
                            waiting.unwrap_or(""),
                            tags_text.as_str(),
                        ],
                        write_kind,
                    ) {
                        errors.push(e.into());
                        continue;
                    }
                    if cat == "waiting_for"
                        && let Some(task_id) = parent_task
                    {
                        memory::ensure_tracking_parent_tag(&mut tags, task_id);
                    }

                    match self
                        .memory
                        .store_knowledge_entry(
                            cid,
                            cat,
                            prio,
                            title,
                            body,
                            &tags,
                            &[],
                            None,
                            src,
                            due,
                            waiting,
                            None,
                        )
                        .await
                    {
                        Ok(id) => {
                            tracing::info!(id, category = cat, title, "KB entry stored");
                            stored.push(serde_json::json!({"id": id, "title": title}));
                        }
                        Err(e) => {
                            tracing::warn!(tool = "knowledge_base", title, error = %e, "Failed to store KB entry");
                            errors
                                .push("Failed to store knowledge entry / 存储知识条目失败".into());
                        }
                    }
                }
                serde_json::json!({
                    "status": "ok",
                    "stored": stored.len(),
                    "entries": stored,
                    "errors": errors,
                })
                .to_string()
            }
            _ => serde_json::json!({"status": "error", "error": "entries array required"})
                .to_string(),
        }
    }

    /// Handle `knowledge_base` action="dashboard": aggregate KB summary, tasks, goals, intents.
    async fn kb_action_dashboard(&self, cid: i64, user_did: &str) -> String {
        // Derive timezone from the same source as the formatter below so the
        // JSON `timezone` label mirrors what `parse_user_tz` actually applied,
        // rather than duplicating the lookup path with its own unreachable
        // fallback (settings.get("user_timezone") always returns the registered
        // default per settings.rs §1280).
        let user_tz = crate::settings::parse_user_tz(&self.settings);
        let timezone = user_tz.name().to_string();
        let now_utc = chrono::Utc::now();
        let generated_at = now_utc
            .with_timezone(&user_tz)
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string();
        let local_date = now_utc
            .with_timezone(&user_tz)
            .format("%Y-%m-%d")
            .to_string();

        let summary = self.memory.knowledge_dashboard_summary(cid).await;
        // Action items now come from unified tasks table
        let task_items = self.memory.list_tasks(cid, None, Some("active"), 15).await;
        let action_items_json: Vec<Value> = task_items
            .iter()
            .filter(|t| t.trigger_type.is_none()) // plain to-dos only
            .take(10)
            .map(|t| {
                let mut entry = serde_json::json!({
                    "id": t.id, "title": t.title, "priority": t.priority,
                    "status": t.status, "source": "tasks",
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
                entry
            })
            .collect();
        let waiting_raw = self
            .memory
            .query_knowledge_entries(cid, Some("waiting_for"), Some("active"), None, 20)
            .await;
        let duplicate_suppression_samples =
            duplicate_task_entry_samples(&task_items, &waiting_raw, 5);
        let suppressed_waiting = waiting_raw
            .iter()
            .filter(|entry| {
                task_items
                    .iter()
                    .any(|task| task_and_entry_look_duplicate(task, entry))
            })
            .count();
        let waiting: Vec<_> = waiting_raw
            .into_iter()
            .filter(|entry| {
                !task_items
                    .iter()
                    .any(|task| task_and_entry_look_duplicate(task, entry))
            })
            .take(10)
            .collect();
        let hygiene_summary = self.memory.kb_hygiene_summary(cid, user_did).await;
        let pending_lint_findings = self
            .memory
            .list_lint_findings_for_did(user_did, Some("pending"), 50)
            .await;
        let mut pending_lint_by_type: BTreeMap<String, usize> = BTreeMap::new();
        for finding in &pending_lint_findings {
            *pending_lint_by_type
                .entry(finding.finding_type.clone())
                .or_insert(0) += 1;
        }
        let urgent_active_entries = self
            .memory
            .query_knowledge_entries(cid, None, Some("active"), Some("urgent"), 50)
            .await;
        let stale_l2_facts = self
            .memory
            .query_stale_facts(user_did, KB_HYGIENE_STALE_L2_DAYS)
            .await
            .unwrap_or_default();
        let kb_hygiene_status = if hygiene_summary.pending_lint_findings == 0
            && hygiene_summary.urgent_kb_items == 0
            && stale_l2_facts.is_empty()
            && suppressed_waiting == 0
        {
            "clean"
        } else {
            "needs_review"
        };
        let recent_decisions = retain_recent_entries(
            self.memory
                .query_knowledge_entries(cid, Some("decision"), Some("active"), None, 20)
                .await,
            KB_DASHBOARD_RECENT_DAYS,
        );
        let recent_insights = retain_recent_entries(
            self.memory
                .query_knowledge_entries(cid, Some("insight"), Some("active"), None, 20)
                .await,
            KB_DASHBOARD_RECENT_DAYS,
        );
        let active_goals = self
            .memory
            .list_goals(user_did, Some(memory::GoalStatus::Active))
            .await
            .unwrap_or_default();
        let active_goals_json: Vec<Value> = active_goals
            .iter()
            .take(10)
            .map(|goal| {
                let mut item = serde_json::json!({
                    "id": goal.id,
                    "title": goal.title.clone(),
                    "status": goal.status.as_str(),
                    "progress_pct": goal.progress_pct,
                    "source": goal.source.as_str(),
                });
                if !goal.description.is_empty() {
                    item["description"] =
                        serde_json::json!(goal.description.chars().take(200).collect::<String>());
                }
                if let Some(target_date) = goal.target_date.as_ref() {
                    item["target_date"] = serde_json::json!(target_date);
                }
                if !goal.milestones.is_empty() {
                    let done = goal.milestones.iter().filter(|m| m.completed).count();
                    item["milestones"] =
                        serde_json::json!(format!("{}/{}", done, goal.milestones.len()));
                }
                item
            })
            .collect();

        // Include upcoming scheduled intents (reminders + callbacks)
        let upcoming_intents = self
            .memory
            .list_user_intents(user_did)
            .await
            .unwrap_or_default();
        let now = now_utc.timestamp();
        let intent_entries: Vec<_> = upcoming_intents
            .iter()
            .filter(|i| i.status == "pending")
            .take(15)
            .map(|i| {
                let trigger = i.trigger_at_unix.map(|t| {
                    let dt = chrono::DateTime::from_timestamp(t, 0);
                    let relative = if t > now {
                        let diff = t - now;
                        if diff < 3600 {
                            format!("{}m", diff / 60)
                        } else if diff < 86400 {
                            format!("{}h", diff / 3600)
                        } else {
                            format!("{}d", diff / 86400)
                        }
                    } else {
                        "overdue".into()
                    };
                    serde_json::json!({
                        "time": dt.map(|d| d.with_timezone(&user_tz).format("%Y-%m-%dT%H:%M:%S%:z").to_string()),
                        "relative": relative,
                    })
                });
                let recurring = i.interval_seconds.map(|s| {
                    if s < 3600 {
                        format!("every {}m", s / 60)
                    } else if s < 86400 {
                        format!("every {}h", s / 3600)
                    } else {
                        format!("every {}d", s / 86400)
                    }
                });
                serde_json::json!({
                    "id": i.id,
                    "context": i.context_text.chars().take(80).collect::<String>(),
                    "mode": i.execution_mode,
                    "trigger": trigger,
                    "recurring": recurring,
                    "group_key": i.group_key,
                })
            })
            .collect();

        let ambiguous_date_count = task_items
            .iter()
            .filter(|task| task_has_ambiguous_date(task))
            .count()
            + waiting
                .iter()
                .filter(|entry| entry_has_ambiguous_date(entry))
                .count()
            + recent_decisions
                .iter()
                .filter(|entry| entry_has_ambiguous_date(entry))
                .count()
            + recent_insights
                .iter()
                .filter(|entry| entry_has_ambiguous_date(entry))
                .count()
            + active_goals
                .iter()
                .filter(|goal| goal_has_ambiguous_date(goal))
                .count();
        let mut warnings = vec![String::from(
            "Calendar not queried in knowledge_base dashboard; call calendar_reader before claiming there are no events today.",
        )];
        if suppressed_waiting > 0 {
            warnings.push(format!(
                "Suppressed {suppressed_waiting} waiting_for item(s) that substantially duplicated active tasks."
            ));
        }
        if ambiguous_date_count > 0 {
            warnings.push(format!(
                "{ambiguous_date_count} item(s) contain ambiguous date text; treat them as unconfirmed until the exact date is resolved."
            ));
        }

        serde_json::json!({
            "status": "ok",
            "generated_at": generated_at,
            "timezone": timezone,
            "local_date": local_date,
            "summary": summary.iter().map(|(c, s, n)| {
                serde_json::json!({"category": c, "status": s, "count": n})
            }).collect::<Vec<_>>(),
            "active_tasks": action_items_json,
            "active_goals": active_goals_json,
            "waiting_for": waiting.iter().map(kb_entry_to_json).collect::<Vec<_>>(),
            "recent_decisions": recent_decisions.iter().take(5).map(kb_entry_to_json).collect::<Vec<_>>(),
            "recent_insights": recent_insights.iter().take(5).map(kb_entry_to_json).collect::<Vec<_>>(),
            "upcoming_intents": intent_entries,
            "kb_hygiene": {
                "status": kb_hygiene_status,
                "l2_total": hygiene_summary.l2_total,
                "pending_lint_findings_total": hygiene_summary.pending_lint_findings,
                "pending_contradictions": hygiene_summary.pending_contradictions,
                "pending_connections": hygiene_summary.pending_connections,
                "urgent_kb_items": hygiene_summary.urgent_kb_items,
                "stale_low_recall_candidates": hygiene_summary.stale_low_recall_candidates,
                "pending_lint_findings": {
                    "total_sampled": hygiene_summary.pending_lint_findings,
                    "sample_limit": 50,
                    "by_type": pending_lint_by_type.iter().map(|(finding_type, count)| {
                        serde_json::json!({"type": finding_type, "count": count})
                    }).collect::<Vec<_>>(),
                    "recent": pending_lint_findings.iter().take(5).map(|finding| {
                        serde_json::json!({
                            "id": finding.id,
                            "type": &finding.finding_type,
                            "fact_id_a": finding.fact_id_a,
                            "fact_id_b": finding.fact_id_b,
                            "similarity": finding.similarity,
                            "created_at": &finding.created_at,
                        })
                    }).collect::<Vec<_>>(),
                },
                "urgent_active_entries": {
                    "total_sampled": hygiene_summary.urgent_kb_items,
                    "sample_limit": 50,
                    "items": urgent_active_entries.iter().take(5).map(kb_entry_to_json).collect::<Vec<_>>(),
                },
                "duplicate_suppression": {
                    "waiting_for_suppressed": suppressed_waiting,
                    "sample_limit": 5,
                    "samples": duplicate_suppression_samples,
                },
                "stale_l2_facts": {
                    "total_sampled": stale_l2_facts.len(),
                    "sample_limit": 50,
                    "threshold_days": KB_HYGIENE_STALE_L2_DAYS,
                    "items": stale_l2_facts.iter().take(5).map(|fact| {
                        serde_json::json!({
                            "id": fact.id,
                            "topic": &fact.topic,
                            "fact_summary": truncate_str(&fact.fact_summary, 200),
                            "created_at": &fact.created_at,
                            "last_accessed": &fact.last_accessed,
                            "days_stale": fact.days_stale,
                        })
                    }).collect::<Vec<_>>(),
                },
                "low_recall_facts": {
                    "status": "not_available",
                    "reason": "dashboard does not yet expose per-DID recall-score counts"
                },
            },
            "today_calendar": {
                "status": "not_queried",
                "date": local_date.clone(),
                "events": Value::Null,
                "reason": "knowledge_base dashboard does not call calendar_reader"
            },
            "warnings": warnings,
        })
        .to_string()
    }

    async fn kb_action_keep_entry(&self, args: &Value, cid: i64) -> String {
        let entry_id = args.get("entry_id").and_then(|v| v.as_i64()).unwrap_or(0);
        if entry_id <= 0 {
            return serde_json::json!({
                "status": "error",
                "error": "entry_id required",
            })
            .to_string();
        }

        let before = self.memory.get_knowledge_entry(entry_id, cid).await;
        let Some(entry) = before else {
            return serde_json::json!({
                "status": "error",
                "error": format!("Entry #{entry_id} not found for this chat."),
                "error_kind": "not_found",
                "retryable": false,
            })
            .to_string();
        };

        let previous_status = entry.status.clone();
        let updates = memory::KnowledgeEntryUpdate {
            status: Some("active".to_string()),
            resolved_at: Some(None),
            ..Default::default()
        };

        match self
            .memory
            .update_knowledge_entry(entry_id, cid, updates)
            .await
        {
            Ok(true) => serde_json::json!({
                "status": "ok",
                "message": format!("Entry #{entry_id} kept active."),
                "entry_id": entry_id,
                "entry_status": "active",
                "previous_status": previous_status,
                "category": entry.category,
                "title": entry.title,
                "destructive": false,
            })
            .to_string(),
            Ok(false) => serde_json::json!({
                "status": "error",
                "error": format!("Entry #{entry_id} not found for this chat."),
                "error_kind": "not_found",
                "retryable": false,
            })
            .to_string(),
            Err(e) => {
                tracing::warn!(
                    tool = "knowledge_base",
                    entry_id,
                    chat_id = cid,
                    error = %e,
                    "Failed to keep KB entry active"
                );
                serde_json::json!({
                    "status": "error",
                    "error": format!("Failed to keep entry #{} / 保留条目失败", entry_id),
                    "error_kind": "database_error",
                    "retryable": true,
                })
                .to_string()
            }
        }
    }

    async fn kb_action_archive_entry(&self, args: &Value, cid: i64) -> String {
        let entry_id = args.get("entry_id").and_then(|v| v.as_i64()).unwrap_or(0);
        if entry_id <= 0 {
            return serde_json::json!({
                "status": "error",
                "error": "entry_id required",
            })
            .to_string();
        }

        let before = self.memory.get_knowledge_entry(entry_id, cid).await;
        if before.is_none() {
            return serde_json::json!({
                "status": "error",
                "error": format!("Entry #{entry_id} not found for this chat."),
                "error_kind": "not_found",
                "retryable": false,
            })
            .to_string();
        }

        let updates = memory::KnowledgeEntryUpdate {
            status: Some("archived".to_string()),
            resolved_at: Some(Some(chrono::Utc::now().to_rfc3339())),
            ..Default::default()
        };

        match self
            .memory
            .update_knowledge_entry(entry_id, cid, updates)
            .await
        {
            Ok(true) => {
                let entry = before.expect("checked above");
                serde_json::json!({
                    "status": "ok",
                    "message": format!("Entry #{entry_id} archived."),
                    "entry_id": entry_id,
                    "entry_status": "archived",
                    "category": entry.category,
                    "title": entry.title,
                    "destructive": false,
                })
                .to_string()
            }
            Ok(false) => serde_json::json!({
                "status": "error",
                "error": format!("Entry #{entry_id} not found for this chat."),
                "error_kind": "not_found",
                "retryable": false,
            })
            .to_string(),
            Err(e) => {
                tracing::warn!(
                    tool = "knowledge_base",
                    entry_id,
                    chat_id = cid,
                    error = %e,
                    "Failed to archive KB entry"
                );
                serde_json::json!({
                    "status": "error",
                    "error": format!("Failed to archive entry #{} / 归档条目失败", entry_id),
                    "error_kind": "database_error",
                    "retryable": true,
                })
                .to_string()
            }
        }
    }

    async fn kb_action_restore_entry(&self, args: &Value, cid: i64) -> String {
        let entry_id = args.get("entry_id").and_then(|v| v.as_i64()).unwrap_or(0);
        if entry_id <= 0 {
            return serde_json::json!({
                "status": "error",
                "error": "entry_id required",
            })
            .to_string();
        }

        let before = self.memory.get_knowledge_entry(entry_id, cid).await;
        let Some(entry) = before else {
            return serde_json::json!({
                "status": "error",
                "error": format!("Entry #{entry_id} not found for this chat."),
                "error_kind": "not_found",
                "retryable": false,
            })
            .to_string();
        };

        let previous_status = entry.status.clone();
        let updates = memory::KnowledgeEntryUpdate {
            status: Some("active".to_string()),
            resolved_at: Some(None),
            ..Default::default()
        };

        match self
            .memory
            .update_knowledge_entry(entry_id, cid, updates)
            .await
        {
            Ok(true) => serde_json::json!({
                "status": "ok",
                "message": format!("Entry #{entry_id} restored."),
                "entry_id": entry_id,
                "entry_status": "active",
                "previous_status": previous_status,
                "category": entry.category,
                "title": entry.title,
                "destructive": false,
            })
            .to_string(),
            Ok(false) => serde_json::json!({
                "status": "error",
                "error": format!("Entry #{entry_id} not found for this chat."),
                "error_kind": "not_found",
                "retryable": false,
            })
            .to_string(),
            Err(e) => {
                tracing::warn!(
                    tool = "knowledge_base",
                    entry_id,
                    chat_id = cid,
                    error = %e,
                    "Failed to restore KB entry"
                );
                serde_json::json!({
                    "status": "error",
                    "error": format!("Failed to restore entry #{} / 恢复条目失败", entry_id),
                    "error_kind": "database_error",
                    "retryable": true,
                })
                .to_string()
            }
        }
    }

    async fn kb_action_preview_merge_entries(&self, args: &Value, cid: i64) -> String {
        let source_ids = kb_source_entry_ids(args);
        let target_id = args
            .get("target_entry_id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        match self
            .kb_build_merge_preview(cid, &source_ids, target_id)
            .await
        {
            Ok(preview) => {
                let preview_token =
                    kb_cleanup_preview_token("preview_merge_entries", cid, &preview);
                serde_json::json!({
                    "status": "ok",
                    "action": "preview_merge_entries",
                    "destructive": false,
                    "mutated": false,
                    "requires_confirm": true,
                    "preview_token": preview_token,
                    "confirm_instruction": "Call merge_entries with the same source_entry_ids, target_entry_id, preview_token, and confirm=true only after explicit user confirmation.",
                    "preview": preview,
                })
                .to_string()
            }
            Err(err) => err,
        }
    }

    async fn kb_action_merge_entries(&self, args: &Value, cid: i64, user_did: &str) -> String {
        if !args
            .get("confirm")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return serde_json::json!({
                "status": "error",
                "error": "merge_entries requires confirm=true after explicit user confirmation of preview_merge_entries.",
                "error_kind": "confirmation_required",
                "retryable": false,
            })
            .to_string();
        }

        let source_ids = kb_source_entry_ids(args);
        let target_id = args
            .get("target_entry_id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let preview = match self
            .kb_build_merge_preview(cid, &source_ids, target_id)
            .await
        {
            Ok(preview) => preview,
            Err(err) => return err,
        };
        let expected_preview_token =
            kb_cleanup_preview_token("preview_merge_entries", cid, &preview);
        if args.get("preview_token").and_then(|v| v.as_str()) != Some(&expected_preview_token) {
            return kb_cleanup_confirm_token_error("merge_entries", &expected_preview_token);
        }
        let Some(target) = self.memory.get_knowledge_entry(target_id, cid).await else {
            return kb_not_found_json(target_id);
        };
        let now = chrono::Utc::now().to_rfc3339();
        let merged_title = args
            .get("merged_title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&target.title)
            .to_string();
        let merged_body = args
            .get("merged_body")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&target.body)
            .to_string();
        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("user confirmed KB merge");
        let source_entries = self.kb_load_entries_for_chat(cid, &source_ids).await;
        let mut tags = target.tags.clone();
        for entry in &source_entries {
            for tag in &entry.tags {
                if !tags.contains(tag) {
                    tags.push(tag.clone());
                }
            }
        }
        let mut related_ids = target.related_ids.clone();
        for id in &source_ids {
            if !related_ids.contains(id) {
                related_ids.push(*id);
            }
        }

        let action_id = self.best_effort_kb_cleanup_action_started(
            crate::action_ledger::ActionKind::KbMerge,
            cid,
            user_did,
            "merge_entries",
            &format!(
                "Merge KB entries {:?} into #{target_id}: {reason}",
                source_ids
            ),
            serde_json::json!({
                "source_entry_ids": source_ids,
                "target_entry_id": target_id,
                "reason": reason,
                "preview_token": expected_preview_token,
                "preview": preview,
                "schema_version": 1,
            }),
        );

        let target_update = memory::KnowledgeEntryUpdate {
            title: Some(merged_title.clone()),
            body: Some(merged_body.clone()),
            tags: Some(tags),
            related_ids: Some(related_ids),
            status: Some("active".to_string()),
            resolved_at: Some(None),
            ..Default::default()
        };
        match self
            .memory
            .update_knowledge_entry(target_id, cid, target_update)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                let err = anyhow::anyhow!("target entry #{target_id} disappeared during merge");
                self.best_effort_kb_cleanup_action_failed(
                    action_id.as_deref(),
                    "merge_entries",
                    &err,
                );
                return kb_not_found_json(target_id);
            }
            Err(e) => {
                self.best_effort_kb_cleanup_action_failed(
                    action_id.as_deref(),
                    "merge_entries",
                    &e,
                );
                return serde_json::json!({
                    "status": "error",
                    "error": format!("Failed to update merge target #{} / 合并目标更新失败", target_id),
                    "error_kind": "database_error",
                    "retryable": true,
                })
                .to_string();
            }
        }

        let mut archived_sources = Vec::new();
        for source_id in &source_ids {
            let update = memory::KnowledgeEntryUpdate {
                status: Some("superseded".to_string()),
                superseded_by: Some(target_id),
                resolved_at: Some(Some(now.clone())),
                ..Default::default()
            };
            match self
                .memory
                .update_knowledge_entry(*source_id, cid, update)
                .await
            {
                Ok(true) => archived_sources.push(*source_id),
                Ok(false) => {
                    let err = anyhow::anyhow!("source entry #{source_id} disappeared during merge");
                    self.best_effort_kb_cleanup_action_failed(
                        action_id.as_deref(),
                        "merge_entries",
                        &err,
                    );
                    return serde_json::json!({
                        "status": "error",
                        "error": format!("Entry #{} not found during merge.", source_id),
                        "error_kind": "not_found",
                        "retryable": false,
                    })
                    .to_string();
                }
                Err(e) => {
                    self.best_effort_kb_cleanup_action_failed(
                        action_id.as_deref(),
                        "merge_entries",
                        &e,
                    );
                    return serde_json::json!({
                        "status": "error",
                        "error": format!("Failed to supersede entry #{} / 源条目标记失败", source_id),
                        "error_kind": "database_error",
                        "retryable": true,
                    })
                    .to_string();
                }
            }
        }

        self.best_effort_kb_cleanup_action_executed(
            action_id.as_deref(),
            "merge_entries",
            &format!("kb_merge:{target_id}:{}", archived_sources.len()),
            serde_json::json!({
                "target_entry_id": target_id,
                "source_entry_ids": archived_sources,
                "reason": reason,
                "schema_version": 1,
            }),
        );

        serde_json::json!({
            "status": "ok",
            "action": "merge_entries",
            "destructive": true,
            "hard_delete": false,
            "target_entry_id": target_id,
            "source_entry_ids": source_ids,
            "source_status": "superseded",
            "target_status": "active",
            "action_ledger_id": action_id,
            "message": format!("Merged {} source entry(s) into #{}.", source_ids.len(), target_id),
        })
        .to_string()
    }

    async fn kb_action_preview_delete_entry(&self, args: &Value, cid: i64) -> String {
        let entry_id = args.get("entry_id").and_then(|v| v.as_i64()).unwrap_or(0);
        if entry_id <= 0 {
            return serde_json::json!({"status": "error", "error": "entry_id required"})
                .to_string();
        }
        let Some(entry) = self.memory.get_knowledge_entry(entry_id, cid).await else {
            return kb_not_found_json(entry_id);
        };
        let preview = kb_cleanup_entry_preview(&entry);
        let preview_token = kb_cleanup_preview_token("preview_delete_entry", cid, &preview);
        serde_json::json!({
            "status": "ok",
            "action": "preview_delete_entry",
            "destructive": false,
            "mutated": false,
            "requires_confirm": true,
            "preview_token": preview_token,
            "recommended_action": "archive_entry",
            "hard_delete_supported": false,
            "confirm_instruction": "Call delete_entry with the same entry_id, preview_token, and confirm=true only after explicit user confirmation. This implementation archives instead of hard deleting.",
            "preview": preview,
        })
        .to_string()
    }

    async fn kb_action_delete_entry(&self, args: &Value, cid: i64, user_did: &str) -> String {
        if !args
            .get("confirm")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return serde_json::json!({
                "status": "error",
                "error": "delete_entry requires confirm=true after explicit user confirmation of preview_delete_entry.",
                "error_kind": "confirmation_required",
                "retryable": false,
            })
            .to_string();
        }
        if args
            .get("hard_delete")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return serde_json::json!({
                "status": "error",
                "error": "hard_delete is not supported in the first KB cleanup implementation; use archive semantics.",
                "error_kind": "unsupported_hard_delete",
                "retryable": false,
            })
            .to_string();
        }

        let entry_id = args.get("entry_id").and_then(|v| v.as_i64()).unwrap_or(0);
        if entry_id <= 0 {
            return serde_json::json!({"status": "error", "error": "entry_id required"})
                .to_string();
        }
        let Some(entry) = self.memory.get_knowledge_entry(entry_id, cid).await else {
            return kb_not_found_json(entry_id);
        };
        let preview = kb_cleanup_entry_preview(&entry);
        let expected_preview_token =
            kb_cleanup_preview_token("preview_delete_entry", cid, &preview);
        if args.get("preview_token").and_then(|v| v.as_str()) != Some(&expected_preview_token) {
            return kb_cleanup_confirm_token_error("delete_entry", &expected_preview_token);
        }
        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("user confirmed KB archive/delete");
        let action_id = self.best_effort_kb_cleanup_action_started(
            crate::action_ledger::ActionKind::KbDelete,
            cid,
            user_did,
            "delete_entry",
            &format!("Archive KB entry #{entry_id}: {reason}"),
            serde_json::json!({
                "entry": preview,
                "reason": reason,
                "preview_token": expected_preview_token,
                "hard_delete": false,
                "schema_version": 1,
            }),
        );

        let update = memory::KnowledgeEntryUpdate {
            status: Some("archived".to_string()),
            resolved_at: Some(Some(chrono::Utc::now().to_rfc3339())),
            ..Default::default()
        };
        match self
            .memory
            .update_knowledge_entry(entry_id, cid, update)
            .await
        {
            Ok(true) => {
                self.best_effort_kb_cleanup_action_executed(
                    action_id.as_deref(),
                    "delete_entry",
                    &format!("kb_archive:{entry_id}"),
                    serde_json::json!({
                        "entry_id": entry_id,
                        "reason": reason,
                        "hard_delete": false,
                        "schema_version": 1,
                    }),
                );
                serde_json::json!({
                    "status": "ok",
                    "action": "delete_entry",
                    "entry_id": entry_id,
                    "entry_status": "archived",
                    "destructive": true,
                    "hard_delete": false,
                    "action_ledger_id": action_id,
                    "message": format!("Entry #{} archived after confirmation.", entry_id),
                })
                .to_string()
            }
            Ok(false) => kb_not_found_json(entry_id),
            Err(e) => {
                self.best_effort_kb_cleanup_action_failed(action_id.as_deref(), "delete_entry", &e);
                serde_json::json!({
                    "status": "error",
                    "error": format!("Failed to archive entry #{} / 归档条目失败", entry_id),
                    "error_kind": "database_error",
                    "retryable": true,
                })
                .to_string()
            }
        }
    }

    async fn kb_load_entries_for_chat(
        &self,
        cid: i64,
        entry_ids: &[i64],
    ) -> Vec<memory::KnowledgeEntry> {
        let mut entries = Vec::new();
        for id in entry_ids {
            if let Some(entry) = self.memory.get_knowledge_entry(*id, cid).await {
                entries.push(entry);
            }
        }
        entries
    }

    async fn kb_build_merge_preview(
        &self,
        cid: i64,
        source_ids: &[i64],
        target_id: i64,
    ) -> Result<Value, String> {
        if target_id <= 0 {
            return Err(serde_json::json!({
                "status": "error",
                "error": "target_entry_id required",
                "error_kind": "invalid_input",
                "retryable": false,
            })
            .to_string());
        }
        if source_ids.is_empty() {
            return Err(serde_json::json!({
                "status": "error",
                "error": "source_entry_ids required",
                "error_kind": "invalid_input",
                "retryable": false,
            })
            .to_string());
        }
        let mut unique = source_ids.to_vec();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() != source_ids.len() || source_ids.contains(&target_id) {
            return Err(serde_json::json!({
                "status": "error",
                "error": "source_entry_ids must be unique and must not include target_entry_id",
                "error_kind": "invalid_input",
                "retryable": false,
            })
            .to_string());
        }

        let Some(target) = self.memory.get_knowledge_entry(target_id, cid).await else {
            return Err(kb_not_found_json(target_id));
        };
        if target.status != "active" {
            return Err(serde_json::json!({
                "status": "error",
                "error": format!("Target entry #{} must be active before merge.", target_id),
                "error_kind": "invalid_state",
                "retryable": false,
            })
            .to_string());
        }

        let mut sources = Vec::new();
        let mut missing = Vec::new();
        let mut invalid_state = Vec::new();
        for source_id in source_ids {
            match self.memory.get_knowledge_entry(*source_id, cid).await {
                Some(entry) if entry.status == "active" => sources.push(entry),
                Some(entry) => invalid_state.push(serde_json::json!({
                    "entry_id": entry.id,
                    "status": entry.status,
                })),
                None => missing.push(*source_id),
            }
        }
        if !missing.is_empty() || !invalid_state.is_empty() {
            return Err(serde_json::json!({
                "status": "error",
                "error": "All source entries must exist in this chat and be active before merge.",
                "error_kind": "invalid_merge_sources",
                "retryable": false,
                "missing_entry_ids": missing,
                "invalid_state": invalid_state,
            })
            .to_string());
        }

        let mut proposed_tags = target.tags.clone();
        for entry in &sources {
            for tag in &entry.tags {
                if !proposed_tags.contains(tag) {
                    proposed_tags.push(tag.clone());
                }
            }
        }
        let mut proposed_related_ids = target.related_ids.clone();
        for entry in &sources {
            if !proposed_related_ids.contains(&entry.id) {
                proposed_related_ids.push(entry.id);
            }
        }

        Ok(serde_json::json!({
            "target": kb_cleanup_entry_preview(&target),
            "sources": sources.iter().map(kb_cleanup_entry_preview).collect::<Vec<_>>(),
            "proposed_target": {
                "entry_id": target.id,
                "title": target.title,
                "body_preview": truncate_str(&target.body, 240),
                "tags": proposed_tags,
                "related_ids": proposed_related_ids,
            },
            "source_result": {
                "status": "superseded",
                "superseded_by": target_id,
                "hard_delete": false,
            },
        }))
    }

    fn best_effort_kb_cleanup_action_started(
        &self,
        action_kind: crate::action_ledger::ActionKind,
        chat_id: i64,
        user_did: &str,
        tool_call_id: &str,
        summary: &str,
        metadata: Value,
    ) -> Option<String> {
        let action_id = format!(
            "kb-cleanup:{}:{}:{:08x}",
            chat_id,
            chrono::Utc::now().timestamp_millis(),
            rand::random::<u32>()
        );
        let result = (|| -> anyhow::Result<()> {
            let conn = crate::encrypted_sqlite::open(self.memory.db_path())?;
            let store = crate::action_ledger::ActionLedgerStore::new(&conn);
            store.create_proposed(&crate::action_ledger::NewAction {
                action_id: action_id.clone(),
                action_kind,
                requested_by: Some(user_did.to_string()),
                chat_id: Some(chat_id),
                input_summary: Some(truncate_str(summary, 500).to_string()),
                user_visible_summary: Some(truncate_str(summary, 240).to_string()),
                idempotency_key: Some(action_id.clone()),
                metadata_json: Some(metadata.to_string()),
                ..Default::default()
            })?;
            store.transition(
                &action_id,
                crate::action_ledger::ActionStatus::Executing,
                &crate::action_ledger::ActionUpdate {
                    tool_call_id: Some(tool_call_id.to_string()),
                    ..Default::default()
                },
            )?;
            Ok(())
        })();
        if let Err(e) = result {
            tracing::warn!(error = %e, "failed to start KB cleanup action ledger row");
            None
        } else {
            Some(action_id)
        }
    }

    fn best_effort_kb_cleanup_action_executed(
        &self,
        action_id: Option<&str>,
        tool_call_id: &str,
        receipt: &str,
        metadata: Value,
    ) {
        let Some(action_id) = action_id else {
            return;
        };
        if let Err(e) = (|| -> anyhow::Result<()> {
            let conn = crate::encrypted_sqlite::open(self.memory.db_path())?;
            let store = crate::action_ledger::ActionLedgerStore::new(&conn);
            store.transition(
                action_id,
                crate::action_ledger::ActionStatus::Executed,
                &crate::action_ledger::ActionUpdate {
                    external_receipt: Some(receipt.to_string()),
                    tool_call_id: Some(tool_call_id.to_string()),
                    metadata_json: Some(metadata.to_string()),
                    ..Default::default()
                },
            )?;
            Ok(())
        })() {
            tracing::warn!(error = %e, action_id, "failed to mark KB cleanup action executed");
        }
    }

    fn best_effort_kb_cleanup_action_failed(
        &self,
        action_id: Option<&str>,
        tool_call_id: &str,
        error: &anyhow::Error,
    ) {
        let Some(action_id) = action_id else {
            return;
        };
        if let Err(e) = (|| -> anyhow::Result<()> {
            let conn = crate::encrypted_sqlite::open(self.memory.db_path())?;
            let store = crate::action_ledger::ActionLedgerStore::new(&conn);
            store.transition(
                action_id,
                crate::action_ledger::ActionStatus::Failed,
                &crate::action_ledger::ActionUpdate {
                    tool_call_id: Some(tool_call_id.to_string()),
                    error_kind: Some("kb_cleanup_error".to_string()),
                    error_message: Some(truncate_str(&error.to_string(), 500).to_string()),
                    ..Default::default()
                },
            )?;
            Ok(())
        })() {
            tracing::warn!(error = %e, action_id, "failed to mark KB cleanup action failed");
        }
    }

    async fn kb_action_review_lint_finding(
        &self,
        args: &Value,
        user_did: &str,
        action: &str,
    ) -> String {
        let finding_id = args
            .get("lint_finding_id")
            .or_else(|| args.get("finding_id"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if finding_id <= 0 {
            return serde_json::json!({
                "status": "error",
                "error": "lint_finding_id required",
            })
            .to_string();
        }

        let (new_status, user_action, message) = match action {
            "dismiss_lint_finding" => (
                "dismissed",
                "dismissed_by_user",
                format!("Lint finding #{finding_id} dismissed."),
            ),
            "resolve_lint_finding" => (
                "resolved",
                "resolved_by_user",
                format!("Lint finding #{finding_id} resolved."),
            ),
            _ => unreachable!("caller restricts lint review actions"),
        };

        match self
            .memory
            .update_lint_finding_status_for_did(finding_id, user_did, new_status, Some(user_action))
            .await
        {
            Ok(true) => serde_json::json!({
                "status": "ok",
                "message": message,
                "lint_finding_id": finding_id,
                "lint_status": new_status,
                "user_action": user_action,
            })
            .to_string(),
            Ok(false) => serde_json::json!({
                "status": "error",
                "error": format!("Lint finding #{finding_id} not found for this user."),
                "error_kind": "not_found",
                "retryable": false,
            })
            .to_string(),
            Err(e) => {
                tracing::warn!(
                    tool = "knowledge_base",
                    finding_id,
                    error = %e,
                    "Failed to update KB lint finding status"
                );
                serde_json::json!({
                    "status": "error",
                    "error": format!("Failed to update lint finding #{} / 更新知识库检查项失败", finding_id),
                    "error_kind": "database_error",
                    "retryable": true,
                })
                .to_string()
            }
        }
    }

    async fn execute_setup_google(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        // One hour is long enough that a user who stepped away and came back
        // can still resume consent (Google's own auth URL stays valid for
        // hours), but short enough that an abandoned flow for a different
        // account does not block another setup indefinitely.
        const PENDING_OAUTH_TTL_SECS: i64 = 3600;

        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let account_name = args
            .get("account_name")
            .and_then(|v| v.as_str())
            .unwrap_or("primary");
        let chat_id = ctx.chat_id;
        match action {
            "status" => {
                // Show all registered accounts if registry exists
                let mut status = self.google_auth_status();
                if !status
                    .get("configured")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                    && self.has_pending_oauth_setup_for_chat(chat_id)
                {
                    status["pending_setup"] = serde_json::json!(true);
                    status["message"] = serde_json::json!(
                        "Google OAuth2 client credentials received but NOT yet authorized. The user has already provided client_id and client_secret. Next step: call setup_google with action='setup' to generate the authorization URL, then have the user complete the OAuth flow."
                    );
                }
                status.to_string()
            }
            "setup" => {
                let rate_limit_key = format!("{}:{}", chat_id.unwrap_or(0), account_name.trim());
                if let Some(limit) =
                    crate::google_auth::check_google_oauth_setup_rate_limit(&rate_limit_key)
                {
                    return serde_json::json!({
                        "status": "error",
                        "error": format!(
                            "OAuth setup attempts are rate-limited. Try again in {} seconds.",
                            limit.retry_after_secs
                        ),
                        "error_kind": "rate_limited",
                        "retryable": true,
                        "retry_after_seconds": limit.retry_after_secs,
                    })
                    .to_string();
                }

                // Abandonment cleanup: wipe PENDING if it's older than the TTL
                // or was staged for a different account. Prevents an abandoned
                // OAuth flow (user opened URL, never completed) from
                // misrouting the next setup's callback. Best-effort — errors
                // are logged, not fatal.
                if let Err(e) = self.cleanup_stale_pending_oauth_for_chat(
                    account_name,
                    PENDING_OAUTH_TTL_SECS,
                    chat_id,
                ) {
                    tracing::warn!(error = %e, "pending OAuth cleanup failed before setup — continuing");
                }

                // For multi-account: skip "already_configured" check if account_name is specified
                // and different from primary — user is adding a second account
                if account_name == "primary" {
                    let primary_already_configured = self
                        .sandbox
                        .get_auth_headers()
                        .contains_key("www.googleapis.com");
                    if primary_already_configured {
                        return serde_json::json!({
                            "status": "already_configured",
                            "message": "Primary Google account is already configured. To add another account, specify account_name (e.g. 'work')."
                        }).to_string();
                    }
                }

                // Try to load from credential store (pre-stored via env or Telegram paste)
                if !self.has_pending_oauth_setup_for_chat(chat_id) {
                    // Check env vars as fallback
                    let env_cid = std::env::var("GOOGLE_CLIENT_ID").ok();
                    let env_cs = std::env::var("GOOGLE_CLIENT_SECRET").ok();
                    if let (Some(cid), Some(cs)) = (env_cid, env_cs)
                        && let Err(e) = self.store_google_client_credentials_named_for_chat(
                            &cid,
                            &cs,
                            account_name,
                            chat_id,
                        )
                    {
                        tracing::warn!(error = %e, "failed to store Google client credentials from env vars");
                    }
                }

                if !self.has_pending_oauth_setup_for_chat(chat_id) {
                    return serde_json::json!({
                        "status": "need_credentials",
                        "message": "Google OAuth client credentials not found. The user needs to:\n1. Create an OAuth app at https://console.cloud.google.com/apis/credentials\n2. Send the credentials in a SEPARATE Telegram message using this exact format:\n   client_id: xxx\n   client_secret: yyy\n   (The system auto-detects and securely stores them — the message is deleted immediately, credentials never reach the AI)\n3. Then say 'connect Google' to continue setup.\n\nIMPORTANT: Do NOT ask the user to tell you the credentials in conversation. They must paste them in the specific format above so the system intercepts them before they reach the AI."
                    }).to_string();
                }

                // Build auth URL. Must stay in sync with
                // `google_auth::DEFAULT_SCOPES` — any drift between the
                // constant and this hard-coded list means re-setup re-asks
                // for an outdated scope combo. calendar.events (full r/w) is
                // required for the calendar write tools landed as part of
                // docs/calendar-write-capability-plan.md.
                let default_scopes: Vec<String> = crate::google_auth::DEFAULT_SCOPES
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect();

                // Read client_id for URL building
                let (db_path, aes_key) = match (
                    &self.credentials.credential_db_path,
                    &self.credentials.credential_aes_key,
                ) {
                    (Some(p), Some(k)) => (p.clone(), *k),
                    _ => {
                        return serde_json::json!({
                            "status": "error",
                            "error": "凭证存储未配置 / Credential store not configured",
                            "error_kind": "credential_store_unconfigured",
                            "retryable": false,
                            "hint": "The credential_db_path or credential_aes_key is missing at startup. Check the aegis-runtime service configuration — the store is required for any Google OAuth action.",
                        })
                        .to_string();
                    }
                };
                let client_id = match (|| -> Result<String> {
                    let store = crate::credential_store::CredentialStore::open(
                        crate::encrypted_sqlite::open(&db_path)?,
                        aes_key,
                    )?;
                    Ok(store
                        .get(
                            &crate::google_auth::pending_cred_service(chat_id),
                            "client_id",
                        )?
                        .unwrap_or_default())
                })() {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to read Google client_id from credential store");
                        return serde_json::json!({
                            "status": "error",
                            "error": "读取凭证失败 / Failed to read credentials from store",
                            "error_kind": "credential_store_read_failed",
                            "retryable": true,
                            "hint": "Credential store may be locked or corrupted. Retry in a few seconds; if the problem persists, check the aegis-runtime service logs for SQLite errors.",
                        })
                        .to_string();
                    }
                };

                if let Some(ref public_url) = self.credentials.public_url {
                    // Scenario A: Public callback URL
                    let redirect_uri = format!("{public_url}/auth/callback/google");
                    let flow_id = match self
                        .stage_pending_oauth_flow_for_chat(account_name, chat_id)
                    {
                        Ok(flow_id) => flow_id,
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to create Google OAuth setup flow");
                            return serde_json::json!({
                                "status": "error",
                                "error": "创建授权流程失败 / Failed to create OAuth setup flow",
                                "error_kind": "oauth_flow_stage_failed",
                                "retryable": true,
                            })
                            .to_string();
                        }
                    };
                    let state = crate::google_auth::generate_oauth_state_for_flow(
                        chat_id.unwrap_or(0),
                        &flow_id,
                        &self.identity.signing_key_bytes(),
                    );
                    let auth_url =
                        crate::google_auth::GoogleAuthManager::build_auth_url_with_redirect(
                            &client_id,
                            &default_scopes,
                            &redirect_uri,
                            &state,
                        );
                    serde_json::json!({
                        "status": "auth_url_ready",
                        "flow_type": "callback",
                        "auth_url": auth_url,
                        "redirect_uri": redirect_uri,
                        "message": "Google Calendar/Drive OAuth 授权设置",
                        "steps": [
                            "1️⃣ 先确认 Redirect URI 已配置：\n打开 Google Cloud Console → APIs & Services → Credentials\nhttps://console.cloud.google.com/apis/credentials".to_string(),
                            "2️⃣ 找到你的 OAuth 2.0 Client，点击编辑（铅笔图标）".to_string(),
                            format!("3️⃣ 在 \"Authorized redirect URIs\" 中添加：\n{redirect_uri}\n（如果已添加则跳过此步）"),
                            "4️⃣ 点击 Save 保存".to_string(),
                            "5️⃣ 然后点击下方授权链接，登录 Google 账号并授权：".to_string(),
                        ],
                    }).to_string()
                } else {
                    // Scenario C: Manual code paste (localhost callback)
                    let redirect_uri = "http://localhost:8765/callback";
                    let flow_id = match self
                        .stage_pending_oauth_flow_for_chat(account_name, chat_id)
                    {
                        Ok(flow_id) => flow_id,
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to create manual Google OAuth setup flow");
                            return serde_json::json!({
                                "status": "error",
                                "error": "创建授权流程失败 / Failed to create OAuth setup flow",
                                "error_kind": "oauth_flow_stage_failed",
                                "retryable": true,
                            })
                            .to_string();
                        }
                    };
                    let state = crate::google_auth::generate_oauth_state_for_flow(
                        chat_id.unwrap_or(0),
                        &flow_id,
                        &self.identity.signing_key_bytes(),
                    );
                    let auth_url =
                        crate::google_auth::GoogleAuthManager::build_auth_url_with_redirect(
                            &client_id,
                            &default_scopes,
                            redirect_uri,
                            &state,
                        );
                    serde_json::json!({
                        "status": "auth_url_ready",
                        "flow_type": "manual",
                        "auth_url": auth_url,
                        "message": "Send this authorization URL to the user. After they authorize, they will see an authorization code. Ask them to copy and paste the code back here in Telegram."
                    }).to_string()
                }
            }
            "upgrade-scopes" => {
                let rate_limit_key =
                    format!("upgrade:{}:{}", chat_id.unwrap_or(0), account_name.trim());
                if let Some(limit) =
                    crate::google_auth::check_google_oauth_setup_rate_limit(&rate_limit_key)
                {
                    return serde_json::json!({
                        "status": "error",
                        "error": format!(
                            "OAuth scope-upgrade attempts are rate-limited. Try again in {} seconds.",
                            limit.retry_after_secs
                        ),
                        "error_kind": "rate_limited",
                        "retryable": true,
                        "retry_after_seconds": limit.retry_after_secs,
                    })
                    .to_string();
                }

                // Non-destructive: only generates an auth URL for incremental
                // authorization on an already-connected account. The user
                // completes consent on Google's side, which is its own
                // confirmation gate — gate-level approval (tool_gate
                // RequireApprovalUnless) still fires, no HITL dance here.
                //
                // See google_auth::build_auth_url_with_redirect (already sets
                // `include_granted_scopes=true`). Google merges the new scopes
                // with the previously granted ones and issues a token whose
                // scope set is the union.

                // Abandonment cleanup — symmetrical with the `setup` arm so
                // an abandoned upgrade flow cannot misroute a later setup
                // on a different account.
                if let Err(e) = self.cleanup_stale_pending_oauth_for_chat(
                    account_name,
                    PENDING_OAUTH_TTL_SECS,
                    chat_id,
                ) {
                    tracing::warn!(error = %e, "pending OAuth cleanup failed before upgrade-scopes — continuing");
                }

                // Optional `scopes` override — lets the LLM request only a
                // specific subset rather than re-aligning to DEFAULT_SCOPES.
                // The subset must be drawn from a known allowlist so the LLM
                // cannot escalate to an arbitrary Google scope.
                let scopes_override: Option<Vec<String>> =
                    args.get("scopes").and_then(|v| v.as_array()).map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    });
                let scope_allowlist: std::collections::HashSet<&'static str> = {
                    let mut s = std::collections::HashSet::new();
                    for sc in crate::google_auth::DEFAULT_SCOPES {
                        s.insert(*sc);
                    }
                    for sc in crate::google_auth::READONLY_SCOPES {
                        s.insert(*sc);
                    }
                    s
                };
                if let Some(ref list) = scopes_override {
                    if list.is_empty() {
                        return serde_json::json!({
                            "status": "error",
                            "error": "scopes 数组为空 / `scopes` override must contain at least one scope",
                            "error_kind": "invalid_scopes_override",
                            "retryable": false,
                            "hint": "Either omit `scopes` to request the DEFAULT_SCOPES set, or supply a non-empty list drawn from the allowlist.",
                        })
                        .to_string();
                    }
                    let disallowed: Vec<&String> = list
                        .iter()
                        .filter(|s| !scope_allowlist.contains(s.as_str()))
                        .collect();
                    if !disallowed.is_empty() {
                        let allowed: Vec<&&str> = scope_allowlist.iter().collect();
                        return serde_json::json!({
                            "status": "error",
                            "error": format!("scope 不在允许列表 / scopes {:?} are not in the allowlist", disallowed),
                            "error_kind": "scope_not_allowed",
                            "retryable": false,
                            "hint": format!("Allowed scopes: {:?}. Add to DEFAULT_SCOPES / READONLY_SCOPES in google_auth.rs before you can request anything outside them.", allowed),
                        })
                        .to_string();
                    }
                }

                let (db_path, aes_key) = match (
                    &self.credentials.credential_db_path,
                    &self.credentials.credential_aes_key,
                ) {
                    (Some(p), Some(k)) => (p.clone(), *k),
                    _ => {
                        return serde_json::json!({
                            "status": "error",
                            "error": "凭证存储未配置 / Credential store not configured",
                            "error_kind": "credential_store_unconfigured",
                            "retryable": false,
                            "hint": "The credential_db_path or credential_aes_key is missing at startup. Check the aegis-runtime service configuration — the store is required for any Google OAuth action.",
                        })
                        .to_string();
                    }
                };

                let cred_service = crate::google_auth::google_oauth_cred_service(account_name);

                // Load existing client_id / client_secret / scopes from the
                // FINALIZED credential-service key for this account. Pending
                // keys were wiped after initial setup, so for upgrade we must
                // re-hydrate them from the authoritative location.
                let load_result: Result<Option<(String, String, Vec<String>)>, anyhow::Error> =
                    (|| {
                        let store = crate::credential_store::CredentialStore::open(
                            crate::encrypted_sqlite::open(&db_path)?,
                            aes_key,
                        )?;
                        let client_id = match store.get(&cred_service, "client_id")? {
                            Some(v) if !v.is_empty() => v,
                            _ => return Ok(None),
                        };
                        let client_secret = match store.get(&cred_service, "client_secret")? {
                            Some(v) if !v.is_empty() => v,
                            _ => return Ok(None),
                        };
                        // Existence of refresh_token is what "connected" means.
                        match store.get(&cred_service, "refresh_token")? {
                            Some(v) if !v.is_empty() => {}
                            _ => return Ok(None),
                        }
                        // A connected account always has `scopes` populated by
                        // `save_to_credential_store_named`; if we find an empty
                        // or missing value alongside a refresh_token, the
                        // cred-store schema has drifted — log it so the bug
                        // surfaces instead of silently claiming every scope
                        // is "new".
                        let scopes_raw = store.get(&cred_service, "scopes")?;
                        let scopes: Vec<String> = scopes_raw
                            .as_deref()
                            .map(|s| {
                                s.split(',')
                                    .map(|p| p.trim().to_string())
                                    .filter(|p| !p.is_empty())
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        if scopes.is_empty() {
                            tracing::warn!(
                                account = account_name,
                                cred_service = %cred_service,
                                scopes_raw = ?scopes_raw,
                                "connected account has empty/missing scopes column — cred-store schema drift suspected; treating every DEFAULT_SCOPE as new"
                            );
                        }
                        Ok(Some((client_id, client_secret, scopes)))
                    })();

                let (client_id, client_secret, current_scopes) = match load_result {
                    Ok(Some(tuple)) => tuple,
                    Ok(None) => {
                        return serde_json::json!({
                            "status": "not_configured",
                            "error": format!("账户 '{account_name}' 尚未连接 / account '{account_name}' is not connected"),
                            "error_kind": "not_configured",
                            "retryable": false,
                            "account_name": account_name,
                            "hint": "Run setup_google(action=\"setup\") first to complete the initial OAuth flow for this account. upgrade-scopes only widens the scope set on an already-connected account.",
                            "message": format!(
                                "账户 '{account_name}' 尚未连接——请先运行 setup_google(action=\"setup\")。\n\
                                 Account '{account_name}' is not connected yet. \
                                 Run setup_google(action=\"setup\") first to perform \
                                 the initial OAuth flow — upgrade-scopes only works \
                                 for an already-connected account."
                            )
                        })
                        .to_string();
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, account = account_name, "failed to read finalized Google OAuth credentials for scope upgrade");
                        return serde_json::json!({
                            "status": "error",
                            "error": "读取凭证失败 / Failed to read credentials from store",
                            "error_kind": "credential_store_read_failed",
                            "retryable": true,
                            "hint": "Credential store may be locked or corrupted. Retry in a few seconds; if the problem persists, check the aegis-runtime service logs for SQLite errors.",
                        })
                        .to_string();
                    }
                };

                // Stage client_id / client_secret / account_name into
                // CRED_SERVICE_PENDING so the existing callback / manual-paste
                // flow (admin.rs google_oauth_public_callback +
                // Orchestrator::exchange_google_auth_code) can complete the
                // incremental exchange unchanged.
                if let Err(e) = self.store_google_client_credentials_named_for_chat(
                    &client_id,
                    &client_secret,
                    account_name,
                    chat_id,
                ) {
                    tracing::warn!(error = %e, account = account_name, "failed to stage pending Google OAuth credentials for scope upgrade");
                    return serde_json::json!({
                        "status": "error",
                        "error": "暂存凭证失败 / Failed to stage credentials for scope upgrade",
                        "error_kind": "credential_stage_failed",
                        "retryable": true,
                        "hint": "Writing to the pending credential service failed. Check that the credential store is not read-only and that the credential_aes_key is initialized.",
                    })
                    .to_string();
                }

                // If the LLM passed a `scopes` override (validated above),
                // use it as the request set; otherwise default to the full
                // DEFAULT_SCOPES. The final token scope is still the UNION
                // of current_scopes and requested_scopes (because
                // include_granted_scopes=true), so the override is a
                // "what to request on the URL" control, not a trim.
                let default_scopes: Vec<String> = crate::google_auth::DEFAULT_SCOPES
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect();
                let requested_scopes: Vec<String> = scopes_override
                    .clone()
                    .unwrap_or_else(|| default_scopes.clone());
                let scopes_origin = if scopes_override.is_some() {
                    "caller_override"
                } else {
                    "default"
                };
                let new_scopes: Vec<String> = requested_scopes
                    .iter()
                    .filter(|s| !current_scopes.contains(s))
                    .cloned()
                    .collect();
                let scope_audit_context = crate::google_auth::GoogleScopeUpgradeAuditContext {
                    account_name: account_name.to_string(),
                    current_scopes: current_scopes.clone(),
                    requested_scopes: requested_scopes.clone(),
                    new_scopes: new_scopes.clone(),
                    scopes_origin: scopes_origin.to_string(),
                };
                if let Err(e) = self.store_google_scope_upgrade_audit_context_for_chat(
                    &scope_audit_context,
                    chat_id,
                ) {
                    tracing::warn!(error = %e, account = account_name, "failed to stage Google scope upgrade audit context");
                }

                // Path-specific message. With scopes_origin=default, the
                // consent screen shows EVERY scope in requested_scopes
                // (which equals DEFAULT_SCOPES and is ≥ current_scopes), so
                // "including previously-granted ones" is accurate. With a
                // caller-supplied override, the consent screen shows ONLY
                // the override — it does NOT show already-granted scopes
                // outside the override. include_granted_scopes=true operates
                // at the token level, not the consent-screen display, so the
                // old grants still end up in the final token even though the
                // user never re-sees them.
                let upgrade_message = if scopes_override.is_some() {
                    "Scope upgrade ready. Send the auth URL to the user. Because the URL sets prompt=consent, Google will display a consent screen listing ONLY the scopes in `requested_scopes` (your caller-supplied override) — previously-granted scopes outside the override are not re-shown. include_granted_scopes=true still applies at the token level, so the issued token will carry the UNION of `current_scopes` and `requested_scopes`; no existing grant is lost."
                } else {
                    "Scope upgrade ready. Send the auth URL to the user. Because the URL sets prompt=consent (required to reliably obtain a fresh refresh_token) and requested_scopes equals the DEFAULT_SCOPES set, Google WILL display the full consent screen listing every scope in `requested_scopes` — including previously-granted ones. The `new_scopes` field tells the user which permissions are actually new this time. Existing grant is preserved: the issued token will carry the UNION of old + new scopes, not just the subset shown on screen."
                };

                if let Some(ref public_url) = self.credentials.public_url {
                    // Scenario A: Public callback URL (Telegram-driven flow).
                    let redirect_uri = format!("{public_url}/auth/callback/google");
                    let flow_id = match self
                        .stage_pending_oauth_flow_for_chat(account_name, chat_id)
                    {
                        Ok(flow_id) => flow_id,
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to create Google OAuth upgrade flow");
                            return serde_json::json!({
                                "status": "error",
                                "error": "创建授权流程失败 / Failed to create OAuth upgrade flow",
                                "error_kind": "oauth_flow_stage_failed",
                                "retryable": true,
                            })
                            .to_string();
                        }
                    };
                    let state = crate::google_auth::generate_oauth_state_for_flow(
                        chat_id.unwrap_or(0),
                        &flow_id,
                        &self.identity.signing_key_bytes(),
                    );
                    let auth_url =
                        crate::google_auth::GoogleAuthManager::build_auth_url_with_redirect(
                            &client_id,
                            &requested_scopes,
                            &redirect_uri,
                            &state,
                        );
                    serde_json::json!({
                        "status": "auth_url_ready",
                        "flow_type": "callback",
                        "auth_url": auth_url,
                        "redirect_uri": redirect_uri,
                        "upgrade": true,
                        "account_name": account_name,
                        "current_scopes": current_scopes,
                        "requested_scopes": requested_scopes,
                        "scopes_origin": scopes_origin,
                        "new_scopes": new_scopes,
                        "message": upgrade_message,
                    })
                    .to_string()
                } else {
                    // Scenario C: Manual code paste (localhost callback).
                    let redirect_uri = "http://localhost:8765/callback";
                    let flow_id = match self
                        .stage_pending_oauth_flow_for_chat(account_name, chat_id)
                    {
                        Ok(flow_id) => flow_id,
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to create manual Google OAuth upgrade flow");
                            return serde_json::json!({
                                "status": "error",
                                "error": "创建授权流程失败 / Failed to create OAuth upgrade flow",
                                "error_kind": "oauth_flow_stage_failed",
                                "retryable": true,
                            })
                            .to_string();
                        }
                    };
                    let state = crate::google_auth::generate_oauth_state_for_flow(
                        chat_id.unwrap_or(0),
                        &flow_id,
                        &self.identity.signing_key_bytes(),
                    );
                    let auth_url =
                        crate::google_auth::GoogleAuthManager::build_auth_url_with_redirect(
                            &client_id,
                            &requested_scopes,
                            redirect_uri,
                            &state,
                        );
                    serde_json::json!({
                        "status": "auth_url_ready",
                        "flow_type": "manual",
                        "auth_url": auth_url,
                        "upgrade": true,
                        "account_name": account_name,
                        "current_scopes": current_scopes,
                        "requested_scopes": requested_scopes,
                        "scopes_origin": scopes_origin,
                        "new_scopes": new_scopes,
                        "message": format!("{upgrade_message} After authorizing, the user copies the authorization code and pastes it back in Telegram."),
                    })
                    .to_string()
                }
            }
            "revoke" => {
                // HITL: credential revocation is destructive and irreversible
                let revoke_id = format!("revoke-google-{:016x}", rand::random::<u64>());
                let revoke_data = serde_json::json!({
                    "approval_id": revoke_id,
                    "action": "revoke_google",
                    "account_name": account_name,
                    "user_did": ctx.user_did,
                    "chat_id": chat_id,
                    "created_at": chrono::Utc::now().to_rfc3339(),
                });
                if let Err(e) = self
                    .memory
                    .store_skill_state("hitl", &revoke_id, &revoke_data.to_string())
                    .await
                {
                    tracing::warn!(error = %e, revoke_id, "failed to store Google OAuth revocation HITL state");
                }

                if let (Some(tx), Some(cid)) = (&self.progress_tx, chat_id) {
                    let msg = format!(
                        "⚠️ **Google Access Revocation — Approval Request**\n\n\
                         Approval ID: `{revoke_id}`\n\n\
                         **\u{6765}\u{6e90} / Origin:** LLM tool call `setup_google(action=\"revoke\")`\n\n\
                         **Account affected:** `{account_name}`\n\n\
                         **\u{8303}\u{56f4} / Scope of change:**\n\
                         • Deletes stored OAuth tokens (access + refresh) for the selected account\n\
                         • Disconnects Google Calendar and Drive integrations for that account\n\
                         • Does NOT revoke the upstream Google app grant — revisit \
                         https://myaccount.google.com/permissions to revoke at Google's side\n\n\
                         **\u{53ef}\u{9006}\u{6027} / Reversibility:** you can re-authorize any \
                         time via `setup_google(action=\"setup\")` (you will need to complete the \
                         Google consent flow again)."
                    );
                    if let Err(e) = tx.try_send(crate::progress_router::ProgressEvent::new(
                        cid,
                        crate::progress_router::ProgressPayload::NeedApproval {
                            approval_id: Some(revoke_id.clone()),
                            tool_name: Some("setup_google".to_string()),
                            summary: None,
                            approval_html: msg,
                        },
                    )) {
                        tracing::warn!(error = %e, "Progress channel full, dropping Google OAuth revocation approval notification");
                    }
                }

                serde_json::json!({
                    "status": "pending_human_approval",
                    "approval_id": revoke_id,
                    "message": "Google access revocation requires user confirmation. Approval request sent via Telegram."
                }).to_string()
            }
            _ => serde_json::json!({
                "status": "error",
                "error": format!("未知 action '{action}' / unknown action '{action}'"),
                "error_kind": "unknown_action",
                "retryable": false,
                "hint": "Valid actions: status, setup, upgrade-scopes, revoke.",
            })
            .to_string(),
        }
    }

    async fn execute_google_drive_upload(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        let jev_decision = match self
            .apply_jev_non_cu_gate("google_drive_upload", &args, ctx)
            .await
        {
            crate::orchestrator_tools_computer_use::JevNonCuApply::Early(early) => return early,
            crate::orchestrator_tools_computer_use::JevNonCuApply::Bypass => None,
            crate::orchestrator_tools_computer_use::JevNonCuApply::Allow(decision) => {
                Some(decision)
            }
        };
        let owner_approved = crate::computer_use::non_cu_gate::args_owner_approved(&args);
        let args = crate::computer_use::non_cu_gate::strip_owner_approved_marker(args);
        let filename = args
            .get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let content_text = args
            .get("content_text")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mime_type = args
            .get("mime_type")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("text/plain");
        let folder_id = args
            .get("folder_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let account = args
            .get("account")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        if filename.is_empty() || filename.len() > 255 || filename.contains('/') {
            return serde_json::json!({
                "status": "error",
                "error": "filename must be 1-255 characters and must not contain '/'"
            })
            .to_string();
        }
        if mime_type.contains('\r') || mime_type.contains('\n') {
            return serde_json::json!({
                "status": "error",
                "error": "mime_type must not contain CR/LF"
            })
            .to_string();
        }

        let auth_token = match self.resolve_google_auth_header(
            account,
            "www.googleapis.com",
            "google_drive_upload auth resolution failed",
        ) {
            Ok(token) => token,
            Err(e) => {
                return sanitized_tool_error(
                    "google_drive_upload",
                    "Google auth unavailable / Google 授权不可用",
                    &e,
                    "auth_unavailable",
                    true,
                    Some(
                        "Run setup_google(action=\"upgrade-scopes\") and include https://www.googleapis.com/auth/drive.file if the token lacks upload permissions.",
                    ),
                );
            }
        };

        let mut metadata = serde_json::json!({ "name": filename });
        if let Some(folder) = folder_id {
            metadata["parents"] = serde_json::json!([folder]);
        }
        let metadata_str = metadata.to_string();
        let boundary = format!("aegis-runtime-drive-{:016x}", rand::random::<u64>());
        let multipart_body = format!(
            "--{b}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n\
             --{b}\r\nContent-Type: {mime}\r\n\r\n{content}\r\n--{b}--\r\n",
            b = boundary,
            meta = metadata_str,
            mime = mime_type,
            content = content_text,
        );
        self.emit_action_phase(
            ctx.chat_id,
            "executing",
            Some("google_drive_upload"),
            Some("executing google_drive_upload"),
        );
        let url = "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id,name,mimeType,webViewLink,size";
        let resp = self
            .http_client
            .post(url)
            .header("Authorization", auth_token)
            .header(
                "Content-Type",
                format!("multipart/related; boundary={boundary}"),
            )
            .body(multipart_body)
            .send()
            .await;

        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                return sanitized_tool_error(
                    "google_drive_upload",
                    "Drive upload request failed / Drive 上传请求失败",
                    &e,
                    "network_error",
                    true,
                    None,
                );
            }
        };
        let status = resp.status();
        let resp_text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return sanitized_tool_error(
                    "google_drive_upload",
                    "Failed to read Drive response / 读取 Drive 响应失败",
                    &e,
                    "response_read_failed",
                    true,
                    None,
                );
            }
        };
        if !status.is_success() {
            let hint = if status.as_u16() == 403 {
                Some(
                    "Drive upload usually needs drive.file. Run setup_google(action=\"upgrade-scopes\") to widen the token.",
                )
            } else {
                None
            };
            return serde_json::json!({
                "status": "error",
                "error": format!("HTTP {}: {}", status, &resp_text[..resp_text.len().min(500)]),
                "error_kind": "drive_api_error",
                "retryable": status.is_server_error(),
                "hint": hint,
            })
            .to_string();
        }

        let parsed: Value = match serde_json::from_str(&resp_text) {
            Ok(v) => v,
            Err(e) => {
                return sanitized_tool_error(
                    "google_drive_upload",
                    "Failed to parse Drive response / 解析 Drive 响应失败",
                    &e,
                    "response_parse_failed",
                    true,
                    None,
                );
            }
        };

        let file_id = parsed
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let parsed_name = parsed
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(filename);
        let parsed_mime_type = parsed
            .get("mimeType")
            .and_then(|v| v.as_str())
            .unwrap_or(mime_type);
        let web_view_link = parsed.get("webViewLink").and_then(|v| v.as_str());
        let observed_at = chrono::Utc::now().to_rfc3339();
        let (raw_event, assistant_event) = build_drive_file_uploaded_event(
            ctx.user_did,
            &observed_at,
            account.unwrap_or("primary"),
            file_id,
            parsed_name,
            parsed_mime_type,
            web_view_link,
            folder_id,
        );
        if let Err(e) = self
            .persist_assistant_event(&raw_event, &assistant_event)
            .await
        {
            tracing::warn!(error = %e, file_id, "[drive] failed to persist assistant event");
        }

        let raw = serde_json::json!({
            "status": "ok",
            "file_id": file_id,
            "name": parsed_name,
            "mime_type": parsed_mime_type,
            "web_view_link": web_view_link,
            "size": parsed.get("size"),
            "message": "File uploaded to Google Drive successfully. Delete is not supported by this tool.",
        })
        .to_string();
        if let Some(decision) = jev_decision {
            crate::computer_use::stamp_jev_on_json(
                &raw,
                &crate::computer_use::JevProvenance::from_decision(
                    &decision,
                    owner_approved.then_some("approved"),
                    None,
                    None,
                    None,
                ),
            )
        } else {
            raw
        }
    }
}
// ══════════════════════════════════════════════════════════════════════════════
//  Tests
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_openai::types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestToolMessageContent,
    };

    use crate::memory::{
        Goal, GoalSource, GoalStatus, KnowledgeEntry, KnowledgeEntryUpdate, MemoryHandle,
    };
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

    fn last_tool_json(state: &CtxState) -> serde_json::Value {
        let last = state.messages.last().expect("tool message");
        let content = match last {
            ChatCompletionRequestMessage::Tool(t) => match &t.content {
                ChatCompletionRequestToolMessageContent::Text(text) => text.as_str(),
                other => panic!("unexpected tool content variant: {other:?}"),
            },
            other => panic!("expected tool message, got: {other:?}"),
        };
        serde_json::from_str(content).expect("tool output must be valid json")
    }

    // ── Routing ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn knowledge_unknown_tool_passes_through() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("quantum_knowledge", "{}");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(&tc, "quantum_knowledge", "{}", &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, None, "unknown tool must pass through");
    }

    #[tokio::test]
    async fn email_unknown_tool_passes_through() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("send_telegram", "{}");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_email(&tc, "send_telegram", "{}", &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ── knowledge_base error paths ───────────────────────────────────────────

    #[tokio::test]
    async fn knowledge_base_bad_json_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("knowledge_base", "INVALID JSON!!!");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(&tc, "knowledge_base", "INVALID JSON!!!", &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("JSON") || content.contains("parse") || content.contains("error"),
            "must report parse error, got: {content}"
        );
    }

    #[tokio::test]
    async fn knowledge_base_empty_action_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        // Empty action falls through to unrecognized action error
        let args = r#"{}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(&tc, "knowledge_base", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("error") || content.contains("action"),
            "must report action error, got: {content}"
        );
    }

    // ── email tool error paths ───────────────────────────────────────────────

    #[tokio::test]
    async fn setup_google_bad_json_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("setup_google", "{{bad");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_email(&tc, "setup_google", "{{bad", &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("JSON") || content.contains("parse") || content.contains("error"));
    }

    // ── knowledge_base action=query (empty DB) ──────────────────────────────

    #[tokio::test]
    async fn knowledge_base_query_empty_db_returns_zero() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"query","category":"insight","limit":10}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(&tc, "knowledge_base", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        // Debug formatting escapes inner quotes: "count":0 becomes \"count\":0
        assert!(
            content.contains("count") && content.contains(":0"),
            "empty DB query must return count=0, got: {content}"
        );
    }

    // ── knowledge_base action=query with seeded data ────────────────────────

    #[tokio::test]
    async fn knowledge_base_query_returns_seeded_entries() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let cid = 42i64;
        // Seed two entries: one insight, one decision
        mem.store_knowledge_entry(
            cid,
            "insight",
            "high",
            "Bitcoin analysis methodology",
            "Use RSI divergence with volume confirmation for entry signals",
            &["crypto".to_string(), "trading".to_string()],
            &[],
            None,
            Some("Morning research session"),
            None,
            None,
            None,
        )
        .await
        .expect("store insight");
        mem.store_knowledge_entry(
            cid,
            "decision",
            "normal",
            "Switch to DCA strategy",
            "Dollar-cost averaging reduces timing risk for long-term holds",
            &["strategy".to_string()],
            &[],
            None,
            None,
            Some("2026-06-01"),
            None,
            None,
        )
        .await
        .expect("store decision");

        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        // Query insights only
        let args = r#"{"action":"query","category":"insight","limit":50}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(
                &tc,
                "knowledge_base",
                args,
                &mut state.ctx_with_chat_id(cid),
            )
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("Bitcoin analysis"),
            "must return the seeded insight"
        );
        assert!(
            !content.contains("Switch to DCA"),
            "must NOT return decision when filtering insight"
        );
    }

    #[test]
    fn retain_recent_entries_filters_out_old_items() {
        let recent = KnowledgeEntry {
            id: 1,
            chat_id: 9,
            category: "decision".into(),
            priority: "normal".into(),
            status: "active".into(),
            title: "Recent".into(),
            body: "new".into(),
            tags: vec![],
            related_ids: vec![],
            source_msg_id: None,
            source_summary: None,
            due_date: None,
            waiting_on: None,
            superseded_by: None,
            resolved_at: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        let stale = KnowledgeEntry {
            id: 2,
            title: "Old".into(),
            body: "old".into(),
            created_at: (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339(),
            updated_at: (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339(),
            ..recent.clone()
        };

        let filtered = super::retain_recent_entries(vec![recent.clone(), stale], 14);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, recent.title);
    }

    #[tokio::test]
    async fn task_and_entry_dedup_ignores_tags_and_waiting_on_person_fields() {
        // Regression guard: dedup must compare only title/body (subject),
        // never tags or waiting_on. Otherwise a "legalreview" tag on the task
        // silently suppresses every waiting item about legal matters, and a
        // "Sample Person" in waiting_on suppresses every entry mentioning the
        // same person regardless of subject.
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let cid = 7777i64;
        let task_id = mem
            .store_task(
                cid,
                "did:test:user",
                "Follow up with Sample Person",
                "Confirm legal email follow-up",
                "high",
                &["legalreview".into()],
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
            .expect("store task");
        mem.store_knowledge_entry(
            cid,
            "waiting_for",
            "normal",
            "Legal Review Next Month",
            "Scope for KuiperAI Q3 review",
            &[],
            &[],
            None,
            None,
            None,
            Some("Sample Person"),
            None,
        )
        .await
        .expect("store unrelated waiting");
        mem.store_knowledge_entry(
            cid,
            "waiting_for",
            "normal",
            "Follow up with Sample Person",
            "duplicate title / same subject",
            &[],
            &[],
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("store real dup");

        let tasks = mem.list_tasks(cid, None, Some("active"), 10).await;
        let entries = mem
            .query_knowledge_entries(cid, Some("waiting_for"), Some("active"), None, 10)
            .await;
        let task = tasks.iter().find(|t| t.id == task_id).expect("task row");
        let unrelated = entries
            .iter()
            .find(|e| e.title == "Legal Review Next Month")
            .expect("unrelated entry");
        let real_dup = entries
            .iter()
            .find(|e| e.title == "Follow up with Sample Person")
            .expect("dup entry");

        // tag "legalreview" normalized is a substring of "legalreviewnextmonth",
        // but tags are excluded from dedup — must NOT suppress.
        // waiting_on "Sample Person" matches task.title via substring but
        // waiting_on is excluded too — must NOT suppress.
        assert!(
            !super::task_and_entry_look_duplicate(task, unrelated),
            "tag and waiting_on must not trigger dedup — subject differs"
        );

        // Positive case (sanity): title-on-title exact match DOES dedup.
        assert!(
            super::task_and_entry_look_duplicate(task, real_dup),
            "title-on-title exact match must dedup"
        );
    }

    #[tokio::test]
    async fn knowledge_base_dashboard_returns_structured_snapshot_and_warnings() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let _ = settings.set(
            "user_timezone",
            serde_json::Value::String("Europe/London".to_string()),
        );
        let orch = test_orchestrator(mem.clone(), settings);
        let cid = 4242i64;

        mem.store_task(
            cid,
            "did:test:user",
            "Follow up with Sample Person",
            "Confirm legal email follow-up",
            "high",
            &["legal".into()],
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
        .expect("store task");
        mem.store_knowledge_entry(
            cid,
            "waiting_for",
            "high",
            "Follow up with Sample Person",
            "Confirm legal email follow-up",
            &["legal".into()],
            &[],
            None,
            None,
            None,
            Some("Sample Person"),
            None,
        )
        .await
        .expect("store duplicate waiting");
        mem.store_knowledge_entry(
            cid,
            "waiting_for",
            "normal",
            "Discuss filing date",
            "Need to confirm next Wednesday (4/23?) with Sample Person",
            &[],
            &[],
            None,
            None,
            None,
            Some("Sample Person"),
            None,
        )
        .await
        .expect("store ambiguous waiting");
        mem.store_knowledge_entry(
            cid,
            "decision",
            "normal",
            "Calendar scope approved",
            "Upgrade Google scopes after OAuth confirmation",
            &[],
            &[],
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("store decision");
        mem.create_goal(
            "did:test:user",
            &Goal {
                id: 0,
                user_did: "did:test:user".into(),
                title: "Prepare KuiperAI case".into(),
                description: "Track evidence archive and legal discussion".into(),
                status: GoalStatus::Active,
                parent_goal_id: None,
                progress_pct: 0.25,
                milestones: vec![],
                created_at: chrono::Utc::now().to_rfc3339(),
                target_date: None,
                last_progress_at: None,
                source: GoalSource::Inferred,
            },
        )
        .await
        .expect("create goal");

        let args = r#"{"action":"dashboard"}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("dashboard should succeed");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["timezone"], "Europe/London");
        assert!(json["generated_at"].as_str().unwrap().contains("+"));
        assert_eq!(json["today_calendar"]["status"], "not_queried");
        assert_eq!(json["active_goals"].as_array().unwrap().len(), 1);
        assert_eq!(json["active_tasks"].as_array().unwrap().len(), 1);
        assert_eq!(
            json["waiting_for"].as_array().unwrap().len(),
            1,
            "duplicate waiting entry should be suppressed"
        );
        assert_eq!(
            json["kb_hygiene"]["duplicate_suppression"]["waiting_for_suppressed"],
            1
        );
        assert_eq!(
            json["kb_hygiene"]["duplicate_suppression"]["samples"][0]["task"]["status"],
            "active"
        );
        assert_eq!(
            json["kb_hygiene"]["duplicate_suppression"]["samples"][0]["entry"]["category"],
            "waiting_for"
        );
        assert!(
            json["kb_hygiene"]["duplicate_suppression"]["samples"][0]["task"]
                .get("title")
                .is_none(),
            "duplicate samples must not expose private task titles"
        );
        assert!(
            json["kb_hygiene"]["duplicate_suppression"]["samples"][0]["entry"]
                .get("title")
                .is_none(),
            "duplicate samples must not expose private KB entry titles"
        );
        let warnings = json["warnings"].as_array().unwrap();
        assert!(
            warnings
                .iter()
                .any(|w| { w.as_str().unwrap_or("").contains("Calendar not queried") }),
            "calendar status warning must be present"
        );
        assert!(
            warnings
                .iter()
                .any(|w| { w.as_str().unwrap_or("").contains("duplicated active tasks") }),
            "dedupe warning must be present"
        );
        assert!(
            warnings
                .iter()
                .any(|w| { w.as_str().unwrap_or("").contains("ambiguous date text") }),
            "ambiguous date warning must be present"
        );
    }

    #[tokio::test]
    async fn knowledge_base_dashboard_includes_kb_hygiene_summary() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let cid = 4243i64;
        let fact_a = mem
            .store_l2(
                "did:test:user",
                "client:legal",
                "The legal filing deadline is Monday",
                None,
            )
            .await
            .expect("store fact a");
        let fact_b = mem
            .store_l2(
                "did:test:user",
                "client:legal",
                "The legal filing deadline is Wednesday",
                None,
            )
            .await
            .expect("store fact b");

        mem.store_lint_finding(
            "contradiction",
            Some(fact_a),
            Some(fact_b),
            Some(0.91),
            Some("facts disagree"),
        )
        .await
        .expect("store lint finding");
        mem.store_knowledge_entry(
            cid,
            "reference",
            "urgent",
            "Review stale legal filing note",
            "Needs human review before reuse",
            &["hygiene".into()],
            &[],
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("store urgent KB entry");

        let args = r#"{"action":"dashboard"}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("dashboard should succeed");

        let json = last_tool_json(&state);
        let hygiene = &json["kb_hygiene"];
        assert_eq!(hygiene["status"], "needs_review");
        assert_eq!(hygiene["l2_total"], 2);
        assert_eq!(hygiene["pending_lint_findings_total"], 1);
        assert_eq!(hygiene["pending_contradictions"], 1);
        assert_eq!(hygiene["pending_connections"], 0);
        assert_eq!(hygiene["urgent_kb_items"], 1);
        assert_eq!(hygiene["pending_lint_findings"]["total_sampled"], 1);
        assert_eq!(
            hygiene["pending_lint_findings"]["by_type"][0],
            serde_json::json!({"type": "contradiction", "count": 1})
        );
        assert_eq!(
            hygiene["pending_lint_findings"]["recent"][0]["fact_id_a"],
            fact_a
        );
        assert_eq!(hygiene["urgent_active_entries"]["total_sampled"], 1);
        assert_eq!(
            hygiene["urgent_active_entries"]["items"][0]["title"],
            "Review stale legal filing note"
        );
    }

    #[tokio::test]
    async fn knowledge_base_dashboard_does_not_leak_cross_did_lint_findings() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let cid = 4246i64;
        let other_fact_a = mem
            .store_l2(
                "did:test:other",
                "client:legal",
                "Other user's legal filing deadline is Monday",
                None,
            )
            .await
            .expect("store other fact a");
        let other_fact_b = mem
            .store_l2(
                "did:test:other",
                "client:legal",
                "Other user's legal filing deadline is Wednesday",
                None,
            )
            .await
            .expect("store other fact b");
        mem.store_lint_finding(
            "contradiction",
            Some(other_fact_a),
            Some(other_fact_b),
            Some(0.91),
            Some("other user's facts disagree"),
        )
        .await
        .expect("store other DID lint finding");

        let args = r#"{"action":"dashboard"}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("dashboard should succeed");

        let json = last_tool_json(&state);
        let hygiene = &json["kb_hygiene"];
        assert_eq!(hygiene["status"], "clean");
        assert_eq!(hygiene["pending_lint_findings_total"], 0);
        assert_eq!(hygiene["pending_lint_findings"]["total_sampled"], 0);
        assert_eq!(
            hygiene["pending_lint_findings"]["recent"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn knowledge_base_dismiss_lint_finding_is_did_scoped() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let cid = 4247i64;
        let fact_a = mem
            .store_l2(
                "did:test:user",
                "client:legal",
                "The legal filing deadline is Monday",
                None,
            )
            .await
            .expect("store fact a");
        let fact_b = mem
            .store_l2(
                "did:test:user",
                "client:legal",
                "The legal filing deadline is Wednesday",
                None,
            )
            .await
            .expect("store fact b");
        let finding_id = mem
            .store_lint_finding(
                "contradiction",
                Some(fact_a),
                Some(fact_b),
                Some(0.91),
                Some("facts disagree"),
            )
            .await
            .expect("store lint finding");

        let args = format!(r#"{{"action":"dismiss_lint_finding","lint_finding_id":{finding_id}}}"#);
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("dismiss lint should succeed");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["lint_status"], "dismissed");
        let findings = mem
            .list_lint_findings_for_did("did:test:user", Some("dismissed"), 5)
            .await;
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].id, finding_id);
        assert_eq!(
            findings[0].user_action.as_deref(),
            Some("dismissed_by_user")
        );
    }

    #[tokio::test]
    async fn knowledge_base_lint_review_rejects_cross_did_finding() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let cid = 4248i64;
        let other_fact_a = mem
            .store_l2(
                "did:test:other",
                "client:legal",
                "Other user's filing deadline is Monday",
                None,
            )
            .await
            .expect("store other fact a");
        let other_fact_b = mem
            .store_l2(
                "did:test:other",
                "client:legal",
                "Other user's filing deadline is Wednesday",
                None,
            )
            .await
            .expect("store other fact b");
        let finding_id = mem
            .store_lint_finding(
                "contradiction",
                Some(other_fact_a),
                Some(other_fact_b),
                Some(0.91),
                Some("other facts disagree"),
            )
            .await
            .expect("store other lint finding");

        let args = format!(r#"{{"action":"resolve_lint_finding","lint_finding_id":{finding_id}}}"#);
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("cross-DID lint review should return structured error");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "error");
        assert_eq!(json["error_kind"], "not_found");
        let findings = mem
            .list_lint_findings_for_did("did:test:other", Some("pending"), 5)
            .await;
        assert_eq!(findings.len(), 1, "other user's finding remains pending");
        assert_eq!(findings[0].id, finding_id);
    }

    #[tokio::test]
    async fn knowledge_base_dashboard_marks_empty_kb_hygiene_clean() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);
        let cid = 4244i64;

        let args = r#"{"action":"dashboard"}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("dashboard should succeed");

        let json = last_tool_json(&state);
        assert_eq!(json["kb_hygiene"]["status"], "clean");
        assert_eq!(
            json["kb_hygiene"]["pending_lint_findings"]["total_sampled"],
            0
        );
        assert_eq!(
            json["kb_hygiene"]["urgent_active_entries"]["total_sampled"],
            0
        );
        assert_eq!(
            json["kb_hygiene"]["duplicate_suppression"]["waiting_for_suppressed"],
            0
        );
        assert_eq!(json["kb_hygiene"]["stale_l2_facts"]["total_sampled"], 0);
        assert_eq!(
            json["kb_hygiene"]["low_recall_facts"]["status"],
            "not_available"
        );
    }

    #[tokio::test]
    async fn knowledge_base_dashboard_includes_stale_l2_hygiene_sample() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let cid = 4245i64;
        let emb = vec![0.5_f32; 4];
        let stale_id = mem
            .store_l2(
                "did:test:user",
                "client:legal",
                "Old legal filing note that should be reviewed before reuse",
                Some(&emb),
            )
            .await
            .expect("store L2 fact");
        mem.test_backdate_l2_last_accessed(stale_id, 45)
            .await
            .expect("backdate L2 fact");

        let args = r#"{"action":"dashboard"}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("dashboard should succeed");

        let json = last_tool_json(&state);
        let stale = &json["kb_hygiene"]["stale_l2_facts"];
        assert_eq!(json["kb_hygiene"]["status"], "needs_review");
        assert_eq!(stale["total_sampled"], 1);
        assert_eq!(stale["threshold_days"], super::KB_HYGIENE_STALE_L2_DAYS);
        assert_eq!(stale["items"][0]["id"], stale_id);
        assert_eq!(stale["items"][0]["topic"], "client:legal");
        assert!(
            stale["items"][0]["days_stale"].as_i64().unwrap() >= super::KB_HYGIENE_STALE_L2_DAYS
        );
    }

    // ── knowledge_base action=search with empty query ───────────────────────

    #[tokio::test]
    async fn knowledge_base_search_empty_query_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"search","query":""}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(&tc, "knowledge_base", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("error") && content.contains("query required"),
            "empty search query must return error, got: {content}"
        );
    }

    // ── knowledge_base action=search with valid query ───────────────────────

    #[tokio::test]
    async fn knowledge_base_search_returns_matching_entries() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let cid = 55i64;
        mem.store_knowledge_entry(
            cid,
            "reference",
            "normal",
            "Kubernetes pod scheduling",
            "K8s uses a scheduler to assign pods to nodes based on resource requests",
            &["devops".to_string()],
            &[],
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("store entry");

        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"search","query":"Kubernetes","limit":10}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(
                &tc,
                "knowledge_base",
                args,
                &mut state.ctx_with_chat_id(cid),
            )
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        // Debug formatting escapes inner quotes; check for unquoted substrings
        assert!(
            content.contains("status") && content.contains("ok"),
            "search must return ok status, got: {content}"
        );
    }

    // ── knowledge_base action=update with missing entry_id ──────────────────

    #[tokio::test]
    async fn knowledge_base_update_missing_entry_id_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"update","status":"done"}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(&tc, "knowledge_base", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("entry_id required"),
            "missing entry_id must return error, got: {content}"
        );
    }

    // ── knowledge_base action=update nonexistent entry ──────────────────────

    #[tokio::test]
    async fn knowledge_base_update_nonexistent_entry_returns_not_found() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"update","entry_id":999999,"status":"done"}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(&tc, "knowledge_base", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("not found") || content.contains("error"),
            "nonexistent entry_id must return not-found error, got: {content}"
        );
    }

    // ── knowledge_base action=update success path ───────────────────────────

    #[tokio::test]
    async fn knowledge_base_update_success() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let cid = 77i64;
        let entry_id = mem
            .store_knowledge_entry(
                cid,
                "insight",
                "normal",
                "Original title",
                "Original body",
                &[],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("store entry");

        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = format!(
            r#"{{"action":"update","entry_id":{},"status":"done","priority":"high","title":"Updated title"}}"#,
            entry_id
        );
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(
                &tc,
                "knowledge_base",
                &args,
                &mut state.ctx_with_chat_id(cid),
            )
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("updated") || content.contains("ok"),
            "successful update must confirm, got: {content}"
        );
    }

    #[tokio::test]
    async fn knowledge_base_archive_entry_marks_entry_archived() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let cid = 4249i64;
        let entry_id = mem
            .store_knowledge_entry(
                cid,
                "reference",
                "normal",
                "Old reference",
                "Reference that has been reviewed and can leave the active view",
                &[],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("store entry");

        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let args = format!(r#"{{"action":"archive_entry","entry_id":{entry_id}}}"#);
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("archive should succeed");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["entry_status"], "archived");
        assert_eq!(json["destructive"], false);

        let archived = mem
            .get_knowledge_entry(entry_id, cid)
            .await
            .expect("entry remains present");
        assert_eq!(archived.status, "archived");
        assert!(archived.resolved_at.is_some());
    }

    #[tokio::test]
    async fn knowledge_base_keep_entry_marks_entry_active_without_deleting() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let cid = 4248i64;
        let entry_id = mem
            .store_knowledge_entry(
                cid,
                "reference",
                "normal",
                "Keep reference",
                "Reference that was reviewed and should stay active",
                &[],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("store entry");
        mem.update_knowledge_entry(
            entry_id,
            cid,
            KnowledgeEntryUpdate {
                status: Some("archived".to_string()),
                resolved_at: Some(Some(chrono::Utc::now().to_rfc3339())),
                ..Default::default()
            },
        )
        .await
        .expect("seed archived entry");

        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let args = format!(r#"{{"action":"keep_entry","entry_id":{entry_id}}}"#);
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("keep should succeed");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["entry_status"], "active");
        assert_eq!(json["previous_status"], "archived");
        assert_eq!(json["destructive"], false);

        let kept = mem
            .get_knowledge_entry(entry_id, cid)
            .await
            .expect("entry remains present");
        assert_eq!(kept.status, "active");
        assert!(kept.resolved_at.is_none());
    }

    #[tokio::test]
    async fn knowledge_base_keep_entry_is_chat_scoped() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let owner_cid = 4246i64;
        let other_cid = 4247i64;
        let entry_id = mem
            .store_knowledge_entry(
                owner_cid,
                "reference",
                "normal",
                "Owner reference",
                "This row belongs to a different chat",
                &[],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("store entry");

        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let args = format!(r#"{{"action":"keep_entry","entry_id":{entry_id}}}"#);
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(other_cid),
        )
        .await
        .expect("cross-chat keep should return structured error");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "error");
        assert_eq!(json["error_kind"], "not_found");

        let owner_entry = mem
            .get_knowledge_entry(entry_id, owner_cid)
            .await
            .expect("owner entry remains present");
        assert_eq!(owner_entry.status, "active");
    }

    #[tokio::test]
    async fn knowledge_base_archive_entry_is_chat_scoped() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let owner_cid = 4250i64;
        let other_cid = 4251i64;
        let entry_id = mem
            .store_knowledge_entry(
                owner_cid,
                "reference",
                "normal",
                "Owner reference",
                "This row belongs to a different chat",
                &[],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("store entry");

        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let args = format!(r#"{{"action":"archive_entry","entry_id":{entry_id}}}"#);
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(other_cid),
        )
        .await
        .expect("cross-chat archive should return structured error");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "error");
        assert_eq!(json["error_kind"], "not_found");

        let owner_entry = mem
            .get_knowledge_entry(entry_id, owner_cid)
            .await
            .expect("owner entry remains present");
        assert_eq!(owner_entry.status, "active");
    }

    #[tokio::test]
    async fn knowledge_base_restore_entry_restores_archived_entry() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let cid = 4252i64;
        let entry_id = mem
            .store_knowledge_entry(
                cid,
                "reference",
                "normal",
                "Restorable reference",
                "Reference that was archived and should return to the active view",
                &[],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("store entry");
        mem.update_knowledge_entry(
            entry_id,
            cid,
            KnowledgeEntryUpdate {
                status: Some("archived".to_string()),
                resolved_at: Some(Some(chrono::Utc::now().to_rfc3339())),
                ..Default::default()
            },
        )
        .await
        .expect("archive seed entry");

        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let args = format!(r#"{{"action":"restore_entry","entry_id":{entry_id}}}"#);
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("restore should succeed");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["entry_status"], "active");
        assert_eq!(json["previous_status"], "archived");
        assert_eq!(json["destructive"], false);

        let restored = mem
            .get_knowledge_entry(entry_id, cid)
            .await
            .expect("entry remains present");
        assert_eq!(restored.status, "active");
        assert!(restored.resolved_at.is_none());
    }

    #[tokio::test]
    async fn knowledge_base_restore_entry_is_chat_scoped() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let owner_cid = 4253i64;
        let other_cid = 4254i64;
        let entry_id = mem
            .store_knowledge_entry(
                owner_cid,
                "reference",
                "normal",
                "Owner archived reference",
                "This archived row belongs to a different chat",
                &[],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("store entry");
        mem.update_knowledge_entry(
            entry_id,
            owner_cid,
            KnowledgeEntryUpdate {
                status: Some("archived".to_string()),
                resolved_at: Some(Some(chrono::Utc::now().to_rfc3339())),
                ..Default::default()
            },
        )
        .await
        .expect("archive seed entry");

        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let args = format!(r#"{{"action":"restore_entry","entry_id":{entry_id}}}"#);
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(other_cid),
        )
        .await
        .expect("cross-chat restore should return structured error");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "error");
        assert_eq!(json["error_kind"], "not_found");

        let owner_entry = mem
            .get_knowledge_entry(entry_id, owner_cid)
            .await
            .expect("owner entry remains present");
        assert_eq!(owner_entry.status, "archived");
        assert!(owner_entry.resolved_at.is_some());
    }

    #[tokio::test]
    async fn knowledge_base_preview_merge_entries_is_read_only() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let cid = 4255i64;
        let target = mem
            .store_knowledge_entry(
                cid,
                "reference",
                "normal",
                "Target",
                "Keep this row",
                &["target".into()],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("target");
        let source = mem
            .store_knowledge_entry(
                cid,
                "reference",
                "normal",
                "Source",
                "Merge this row",
                &["source".into()],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("source");

        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let args = format!(
            r#"{{"action":"preview_merge_entries","source_entry_ids":[{source}],"target_entry_id":{target}}}"#
        );
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("preview should succeed");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["mutated"], false);
        assert!(json["preview_token"].as_str().unwrap().starts_with("kbp_"));
        assert_eq!(
            mem.get_knowledge_entry(source, cid).await.unwrap().status,
            "active"
        );
    }

    #[tokio::test]
    async fn knowledge_base_merge_requires_confirm() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let cid = 4256i64;
        let target = mem
            .store_knowledge_entry(
                cid,
                "reference",
                "normal",
                "Target",
                "Keep",
                &[],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let source = mem
            .store_knowledge_entry(
                cid,
                "reference",
                "normal",
                "Source",
                "Merge",
                &[],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);
        let args = format!(
            r#"{{"action":"merge_entries","source_entry_ids":[{source}],"target_entry_id":{target}}}"#
        );
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("merge should return structured error");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "error");
        assert_eq!(json["error_kind"], "confirmation_required");
    }

    #[tokio::test]
    async fn knowledge_base_merge_requires_matching_preview_token() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let cid = 4259i64;
        let target = mem
            .store_knowledge_entry(
                cid,
                "reference",
                "normal",
                "Target",
                "Keep",
                &[],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let source = mem
            .store_knowledge_entry(
                cid,
                "reference",
                "normal",
                "Source",
                "Merge",
                &[],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);
        let args = format!(
            r#"{{"action":"merge_entries","source_entry_ids":[{source}],"target_entry_id":{target},"confirm":true,"preview_token":"kbp_wrong"}}"#
        );
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("merge should return structured error");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "error");
        assert_eq!(json["error_kind"], "preview_token_required");
        assert_eq!(
            mem.get_knowledge_entry(source, cid).await.unwrap().status,
            "active"
        );
    }

    #[tokio::test]
    async fn knowledge_base_merge_supersedes_sources_and_records_ledger() {
        let (db, orch) = file_backed_test_orchestrator();
        let cid = 4257i64;
        let mem = orch.memory.clone();
        let target = mem
            .store_knowledge_entry(
                cid,
                "reference",
                "normal",
                "Target",
                "Keep",
                &["target".into()],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let source = mem
            .store_knowledge_entry(
                cid,
                "reference",
                "normal",
                "Source",
                "Merge",
                &["source".into()],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();

        let preview_args = format!(
            r#"{{"action":"preview_merge_entries","source_entry_ids":[{source}],"target_entry_id":{target}}}"#
        );
        let tc = make_tool_call("knowledge_base", &preview_args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &preview_args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("preview should succeed");
        let preview_token = last_tool_json(&state)["preview_token"]
            .as_str()
            .unwrap()
            .to_string();

        let args = format!(
            r#"{{"action":"merge_entries","source_entry_ids":[{source}],"target_entry_id":{target},"confirm":true,"preview_token":"{preview_token}","merged_body":"Merged body","reason":"duplicate"}}"#
        );
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("merge should succeed");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["hard_delete"], false);
        let archived = mem.get_knowledge_entry(source, cid).await.unwrap();
        assert_eq!(archived.status, "superseded");
        assert_eq!(archived.superseded_by, Some(target));
        let merged = mem.get_knowledge_entry(target, cid).await.unwrap();
        assert_eq!(merged.body, "Merged body");
        assert!(merged.tags.contains(&"source".to_string()));

        let conn = crate::encrypted_sqlite::open(db.path()).expect("open db");
        let ledger = crate::action_ledger::ActionLedgerStore::new(&conn)
            .list_recent_by_status(crate::action_ledger::ActionStatus::Executed, 5)
            .expect("ledger list");
        assert!(ledger.iter().any(|entry| {
            entry.action_kind == crate::action_ledger::ActionKind::KbMerge
                && entry
                    .external_receipt
                    .as_deref()
                    .unwrap_or("")
                    .starts_with("kb_merge:")
        }));
    }

    #[tokio::test]
    async fn knowledge_base_delete_requires_confirm_then_archives_with_ledger() {
        let (db, orch) = file_backed_test_orchestrator();
        let cid = 4258i64;
        let mem = orch.memory.clone();
        let entry_id = mem
            .store_knowledge_entry(
                cid,
                "reference",
                "normal",
                "Delete preview",
                "Archive instead of hard delete",
                &[],
                &[],
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();

        let missing_confirm = format!(r#"{{"action":"delete_entry","entry_id":{entry_id}}}"#);
        let tc = make_tool_call("knowledge_base", &missing_confirm);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &missing_confirm,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .unwrap();
        assert_eq!(
            last_tool_json(&state)["error_kind"],
            "confirmation_required"
        );

        let preview_args = format!(r#"{{"action":"preview_delete_entry","entry_id":{entry_id}}}"#);
        let tc = make_tool_call("knowledge_base", &preview_args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &preview_args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .unwrap();
        let preview_token = last_tool_json(&state)["preview_token"]
            .as_str()
            .unwrap()
            .to_string();

        let wrong_token = format!(
            r#"{{"action":"delete_entry","entry_id":{entry_id},"confirm":true,"preview_token":"kbp_wrong"}}"#
        );
        let tc = make_tool_call("knowledge_base", &wrong_token);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &wrong_token,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .unwrap();
        assert_eq!(
            last_tool_json(&state)["error_kind"],
            "preview_token_required"
        );

        let args = format!(
            r#"{{"action":"delete_entry","entry_id":{entry_id},"confirm":true,"preview_token":"{preview_token}","reason":"stale duplicate"}}"#
        );
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(cid),
        )
        .await
        .expect("delete should archive");

        let json = last_tool_json(&state);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["hard_delete"], false);
        assert_eq!(
            mem.get_knowledge_entry(entry_id, cid).await.unwrap().status,
            "archived"
        );

        let conn = crate::encrypted_sqlite::open(db.path()).expect("open db");
        let ledger = crate::action_ledger::ActionLedgerStore::new(&conn)
            .list_recent_by_status(crate::action_ledger::ActionStatus::Executed, 5)
            .expect("ledger list");
        assert!(ledger.iter().any(|entry| {
            entry.action_kind == crate::action_ledger::ActionKind::KbDelete
                && entry.external_receipt.as_deref() == Some(&format!("kb_archive:{entry_id}"))
        }));
    }

    // ── knowledge_base action=extract with invalid category ─────────────────

    #[tokio::test]
    async fn knowledge_base_extract_invalid_category_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"extract","entries":[{"category":"nonexistent_cat","title":"T","body":"B","priority":"normal"}]}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(&tc, "knowledge_base", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("invalid category"),
            "invalid category must be reported in errors, got: {content}"
        );
    }

    // ── knowledge_base action=extract with empty entries array ───────────────

    #[tokio::test]
    async fn knowledge_base_extract_empty_entries_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"extract","entries":[]}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(&tc, "knowledge_base", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("entries array required"),
            "empty entries array must return error, got: {content}"
        );
    }

    // ── knowledge_base action=extract with missing title/body ───────────────

    #[tokio::test]
    async fn knowledge_base_extract_missing_title_body_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"extract","entries":[{"category":"insight","title":"","body":"","priority":"normal"}]}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(&tc, "knowledge_base", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("title and body required"),
            "empty title/body must be reported, got: {content}"
        );
    }

    // ── knowledge_base action=extract success path ──────────────────────────

    #[tokio::test]
    async fn knowledge_base_extract_valid_entry_succeeds() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings);

        let args = r#"{"action":"extract","entries":[{"category":"decision","title":"Adopt Rust for backend","body":"Rust offers memory safety without GC, reducing production crash rate by 95%","priority":"high","tags":["architecture","backend"]}]}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(&tc, "knowledge_base", args, &mut state.ctx_with_chat_id(88))
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        // Debug formatting escapes inner quotes; check for unquoted substrings
        assert!(
            content.contains("stored") && content.contains(":1"),
            "must report 1 entry stored, got: {content}"
        );
        assert!(content.contains("Adopt Rust"), "must echo back the title");
    }

    // ── knowledge_base unknown action ───────────────────────────────────────

    #[tokio::test]
    async fn knowledge_base_unknown_action_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"teleport"}"#;
        let tc = make_tool_call("knowledge_base", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_knowledge(&tc, "knowledge_base", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("Unknown action") && content.contains("teleport"),
            "unknown action must be reported, got: {content}"
        );
    }

    #[tokio::test]
    async fn google_drive_upload_rejects_bad_filename_locally() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"filename":"nested/path.txt","content_text":"hello"}"#;
        let tc = make_tool_call("google_drive_upload", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_email(&tc, "google_drive_upload", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("filename must be 1-255 characters"),
            "bad filename must be rejected locally, got: {content}"
        );
    }

    #[tokio::test]
    async fn google_drive_upload_reports_not_configured_without_google_auth() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"filename":"report.md","content_text":"hello"}"#;
        let tc = make_tool_call("google_drive_upload", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_email(&tc, "google_drive_upload", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        // Impl now emits error_kind="auth_unavailable" with the bilingual
        // "Google auth unavailable / Google 授权不可用" message when no
        // Google OAuth is configured. Accept either the new wording or the
        // legacy "not_configured" one for robustness.
        assert!(
            content.contains("auth_unavailable")
                || content.contains("not_configured")
                || content.contains("Google OAuth2 not configured")
                || content.contains("Google auth unavailable"),
            "missing auth must surface a structured auth-unavailable error, got: {content}"
        );
    }

    /// Abandonment cleanup — if an old upgrade-scopes PENDING exists for a
    /// different account with a stale timestamp, the next call wipes it.
    /// Uses cleanup_stale_pending_oauth directly because the arm-level call
    /// is fire-and-forget (uses account_name from args).
    #[tokio::test]
    async fn cleanup_stale_pending_oauth_different_account() {
        use crate::credential_store::CredentialStore;
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let mut orch = test_orchestrator(mem, settings);
        let tmp = tempfile::NamedTempFile::new().expect("tmp");
        let db_path = tmp.path().to_str().unwrap().to_string();
        let aes_key = [42u8; 32];
        // Seed PENDING for "abandoned-account" with a 2-hour-old timestamp.
        {
            let store =
                CredentialStore::open(crate::encrypted_sqlite::open(&db_path).unwrap(), aes_key)
                    .unwrap();
            let pending_service = crate::google_auth::pending_cred_service(None);
            store
                .store(&pending_service, "client_id", "abandoned-cid")
                .unwrap();
            store
                .store(&pending_service, "client_secret", "abandoned-cs")
                .unwrap();
            store
                .store(&pending_service, "account_name", "abandoned-account")
                .unwrap();
            let two_hours_ago = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
            store
                .store(&pending_service, "staged_at", &two_hours_ago)
                .unwrap();
        }
        orch.set_credential_store(db_path.clone(), aes_key);

        // Cleanup with 1-hour TTL for a different account.
        let wiped = orch
            .cleanup_stale_pending_oauth("intended-account", 3600)
            .expect("cleanup must not error");
        assert!(wiped, "stale PENDING for different account should be wiped");

        // PENDING should now be empty.
        let store =
            CredentialStore::open(crate::encrypted_sqlite::open(&db_path).unwrap(), aes_key)
                .unwrap();
        let pending_service = crate::google_auth::pending_cred_service(None);
        assert!(store.get(&pending_service, "client_id").unwrap().is_none());
        assert!(
            store
                .get(&pending_service, "account_name")
                .unwrap()
                .is_none()
        );
    }

    /// Cleanup is a no-op when PENDING is fresh AND belongs to the intended
    /// account. Prevents the cleanup from clobbering an in-flight OAuth flow
    /// for the same account.
    #[tokio::test]
    async fn cleanup_stale_pending_oauth_fresh_same_account_no_op() {
        use crate::credential_store::CredentialStore;
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let mut orch = test_orchestrator(mem, settings);
        let tmp = tempfile::NamedTempFile::new().expect("tmp");
        let db_path = tmp.path().to_str().unwrap().to_string();
        let aes_key = [42u8; 32];
        {
            let store =
                CredentialStore::open(crate::encrypted_sqlite::open(&db_path).unwrap(), aes_key)
                    .unwrap();
            let pending_service = crate::google_auth::pending_cred_service(None);
            store
                .store(&pending_service, "client_id", "fresh-cid")
                .unwrap();
            store
                .store(&pending_service, "account_name", "primary")
                .unwrap();
            store
                .store(
                    &pending_service,
                    "staged_at",
                    &chrono::Utc::now().to_rfc3339(),
                )
                .unwrap();
        }
        orch.set_credential_store(db_path.clone(), aes_key);

        let wiped = orch
            .cleanup_stale_pending_oauth("primary", 3600)
            .expect("cleanup must not error");
        assert!(!wiped, "fresh same-account PENDING must be preserved");

        let store =
            CredentialStore::open(crate::encrypted_sqlite::open(&db_path).unwrap(), aes_key)
                .unwrap();
        let pending_service = crate::google_auth::pending_cred_service(None);
        assert_eq!(
            store.get(&pending_service, "client_id").unwrap().as_deref(),
            Some("fresh-cid")
        );
    }

    /// Unknown action error must list upgrade-scopes so the LLM can discover it.
    #[tokio::test]
    async fn setup_google_unknown_action_lists_upgrade_scopes() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"bogus"}"#;
        let tc = make_tool_call("setup_google", args);
        let mut state = CtxState::new();
        orch.handle_tool_email(&tc, "setup_google", args, &mut state.ctx())
            .await
            .expect("should not error");

        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("upgrade-scopes"),
            "unknown-action error must list upgrade-scopes, got: {content}"
        );
    }

    async fn extract_waiting(orch: &Orchestrator, chat_id: i64, category: &str) -> i64 {
        let args = format!(
            r#"{{"action":"extract","entries":[{{"category":"{category}","priority":"normal","title":"治疗提醒","body":"等确认时间","waiting_on":"确认预约时间","tags":["visit"]}}]}}"#
        );
        let tc = make_tool_call("knowledge_base", &args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &args,
            &mut state.ctx_with_chat_id(chat_id),
        )
        .await
        .expect("extract");
        let parsed = last_tool_json(&state);
        assert_eq!(parsed["status"], "ok", "{parsed}");
        parsed["entries"][0]["id"].as_i64().expect("stored id")
    }

    #[tokio::test]
    async fn recurring_callback_stamps_waiting_rows_only() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));
        let chat_id = 12345_i64;
        *orch.tool_exec.callback_task_info.lock().await = Some((36, chat_id));

        let waiting_id = extract_waiting(&orch, chat_id, "waiting_for").await;
        let waiting = mem
            .get_knowledge_entry(waiting_id, chat_id)
            .await
            .expect("waiting row");
        assert!(waiting.tags.iter().any(|tag| tag == "tracking_task:36"));
        assert!(waiting.tags.iter().any(|tag| tag == "visit"));

        let insight_id = extract_waiting(&orch, chat_id, "insight").await;
        let insight = mem
            .get_knowledge_entry(insight_id, chat_id)
            .await
            .expect("insight row");
        assert!(
            insight
                .tags
                .iter()
                .all(|tag| !tag.starts_with("tracking_task:"))
        );

        let plain_id = mem
            .store_knowledge_entry(
                chat_id,
                "waiting_for",
                "normal",
                "未盖章的等待",
                "等确认时间",
                &["visit".into()],
                &[],
                None,
                None,
                None,
                Some("确认预约时间"),
                None,
            )
            .await
            .expect("seed");
        let update_args =
            format!(r#"{{"action":"update","entry_id":{plain_id},"title":"刷新后的等待"}}"#);
        let tc = make_tool_call("knowledge_base", &update_args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &update_args,
            &mut state.ctx_with_chat_id(chat_id),
        )
        .await
        .expect("update");
        let refreshed = mem
            .get_knowledge_entry(plain_id, chat_id)
            .await
            .expect("refreshed");
        assert!(refreshed.tags.iter().any(|tag| tag == "tracking_task:36"));

        let closing_id = mem
            .store_knowledge_entry(
                chat_id,
                "waiting_for",
                "normal",
                "即将结束",
                "等确认时间",
                &["visit".into()],
                &[],
                None,
                None,
                None,
                Some("确认预约时间"),
                None,
            )
            .await
            .expect("seed closing");
        let close_args =
            format!(r#"{{"action":"update","entry_id":{closing_id},"status":"done"}}"#);
        let tc = make_tool_call("knowledge_base", &close_args);
        let mut state = CtxState::new();
        orch.handle_tool_knowledge(
            &tc,
            "knowledge_base",
            &close_args,
            &mut state.ctx_with_chat_id(chat_id),
        )
        .await
        .expect("close");
        let closed = mem
            .get_knowledge_entry(closing_id, chat_id)
            .await
            .expect("closed");
        assert!(
            closed
                .tags
                .iter()
                .all(|tag| !tag.starts_with("tracking_task:"))
        );
    }

    #[tokio::test]
    async fn waiting_extract_without_this_chats_callback_has_no_parent() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));
        let chat_id = 12345_i64;
        let plain_id = extract_waiting(&orch, chat_id, "waiting_for").await;
        let plain = mem
            .get_knowledge_entry(plain_id, chat_id)
            .await
            .expect("plain");
        assert!(
            plain
                .tags
                .iter()
                .all(|tag| !tag.starts_with("tracking_task:"))
        );

        *orch.tool_exec.callback_task_info.lock().await = Some((36, 99_999));
        let mismatch_id = extract_waiting(&orch, chat_id, "waiting_for").await;
        let mismatch = mem
            .get_knowledge_entry(mismatch_id, chat_id)
            .await
            .expect("mismatch");
        assert!(
            mismatch
                .tags
                .iter()
                .all(|tag| !tag.starts_with("tracking_task:"))
        );
    }
}
