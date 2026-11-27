//! Goal tracking tool handler (G5 — Long-term Goal Tracking).
//!
//! Contains `handle_tool_goals` dispatching the `manage_goals` pseudo-tool.

use anyhow::Result;
use serde_json::Value;

use async_openai::types::chat::ChatCompletionMessageToolCall;

use crate::memory::{Goal, GoalSource, GoalStatus, GoalUpdate};
use crate::orchestrator::{Orchestrator, ToolContext};
use crate::source_guards::{EmailSourceWriteKind, validate_email_source_write};

impl Orchestrator {
    /// Handle the `manage_goals` pseudo-tool.
    #[allow(unused_variables)]
    pub(crate) async fn handle_tool_goals(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "manage_goals" => execute_manage_goals,
        );
        Ok(None)
    }

    async fn execute_manage_goals(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        let user_did = ctx.user_did;
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        match action {
            "create" => self.goal_action_create(&args, user_did).await,
            "update" => self.goal_action_update(&args).await,
            "list" => self.goal_action_list(&args, user_did).await,
            "get" => self.goal_action_get(&args).await,
            "complete" => self.goal_action_set_status(&args, GoalStatus::Completed).await,
            "pause" => self.goal_action_set_status(&args, GoalStatus::Paused).await,
            "resume" => self.goal_action_set_status(&args, GoalStatus::Active).await,
            "abandon" => self.goal_action_set_status(&args, GoalStatus::Abandoned).await,
            "add_milestone" => self.goal_action_add_milestone(&args).await,
            "complete_milestone" => self.goal_action_complete_milestone(&args).await,
            _ => serde_json::json!({
                "status": "error",
                "error": format!("unknown action '{action}'"),
                "error_kind": "invalid_input",
                "retryable": false,
                "hint": "valid actions: create, update, list, get, complete, pause, resume, abandon, add_milestone, complete_milestone"
            })
            .to_string(),
        }
    }

    // ── Goal action helpers ─────────────────────────────────────────────

    async fn goal_action_create(&self, args: &Value, user_did: &str) -> String {
        let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
        if title.is_empty() {
            return serde_json::json!({
                "status": "error",
                "error": "title is required",
                "error_kind": "invalid_input",
                "retryable": false,
                "hint": "provide a title for the goal"
            })
            .to_string();
        }
        let source = args
            .get("source")
            .and_then(|v| v.as_str())
            .map(GoalSource::from_str_lossy)
            .unwrap_or(GoalSource::Explicit);
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let target_date = args.get("target_date").and_then(|v| v.as_str());
        let source_summary = args.get("source_summary").and_then(|v| v.as_str());
        if let Err(e) = validate_email_source_write(
            &[
                title,
                description,
                target_date.unwrap_or(""),
                source_summary.unwrap_or(""),
            ],
            EmailSourceWriteKind::DeterministicFact,
        ) {
            return serde_json::json!({
                "status": "error",
                "error": e,
                "error_kind": "unconfirmed_source",
                "retryable": true,
                "hint": "Confirm the email with full body/attachments or user confirmation before creating/updating a goal from it."
            })
            .to_string();
        }
        let goal = Goal {
            id: 0,
            user_did: user_did.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            status: GoalStatus::Active,
            parent_goal_id: args.get("parent_goal_id").and_then(|v| v.as_i64()),
            progress_pct: 0.0,
            milestones: vec![],
            created_at: String::new(), // set by DB function
            target_date: target_date.map(String::from),
            last_progress_at: None,
            source,
        };
        match self.memory.create_goal(user_did, &goal).await {
            Ok(id) => serde_json::json!({
                "status": "ok",
                "goal_id": id,
                "title": title,
                "source": source.as_str(),
            })
            .to_string(),
            Err(e) => crate::utils::sanitized_tool_error(
                "manage_goals",
                "goal operation failed",
                &e,
                "internal",
                false,
                None,
            ),
        }
    }

    async fn goal_action_update(&self, args: &Value) -> String {
        let Some(goal_id) = args.get("goal_id").and_then(|v| v.as_i64()) else {
            return serde_json::json!({
                "status": "error",
                "error": "goal_id is required",
                "error_kind": "invalid_input",
                "retryable": false,
                "hint": "provide the goal_id to update"
            })
            .to_string();
        };
        let title = args.get("title").and_then(|v| v.as_str());
        let description = args.get("description").and_then(|v| v.as_str());
        let target_date = args.get("target_date").and_then(|v| v.as_str());
        let source_summary = args.get("source_summary").and_then(|v| v.as_str());
        if let Err(e) = validate_email_source_write(
            &[
                title.unwrap_or(""),
                description.unwrap_or(""),
                target_date.unwrap_or(""),
                source_summary.unwrap_or(""),
            ],
            EmailSourceWriteKind::DeterministicFact,
        ) {
            return serde_json::json!({
                "status": "error",
                "error": e,
                "error_kind": "unconfirmed_source",
                "retryable": true,
                "hint": "Confirm the email with full body/attachments or user confirmation before creating/updating a goal from it."
            })
            .to_string();
        }
        let updates = GoalUpdate {
            title: title.map(String::from),
            description: description.map(String::from),
            status: None,
            progress_pct: args
                .get("progress_pct")
                .and_then(|v| v.as_f64())
                .map(|p| p as f32),
            target_date: args
                .get("target_date")
                .map(|v| v.as_str().map(String::from)),
        };
        match self.memory.update_goal(goal_id, updates).await {
            Ok(()) => serde_json::json!({"status": "ok", "goal_id": goal_id}).to_string(),
            Err(e) => crate::utils::sanitized_tool_error(
                "manage_goals",
                "goal operation failed",
                &e,
                "internal",
                false,
                None,
            ),
        }
    }

    async fn goal_action_list(&self, args: &Value, user_did: &str) -> String {
        let raw_filter = args.get("status_filter").and_then(|v| v.as_str());
        // "all" → None (no filter), default → active
        let filter = match raw_filter.map(|s| s.trim().to_ascii_lowercase()) {
            Some(s) if s == "all" || s == "any" || s == "*" => None,
            Some(s) => {
                // Strict: an unknown status used to fall back to Active and
                // present the wrong set as if it were the requested one.
                let parsed = GoalStatus::from_str_lossy(&s);
                if parsed.as_str() != s {
                    return serde_json::json!({
                        "status": "error",
                        "error": format!("unknown status_filter '{s}'"),
                        "hint": "use active, paused, completed, abandoned, or all",
                    })
                    .to_string();
                }
                Some(parsed)
            }
            None => Some(GoalStatus::Active),
        };
        match self.memory.list_goals(user_did, filter).await {
            Ok(goals) => {
                let items: Vec<Value> = goals
                    .iter()
                    .map(|g| {
                        let mut obj = serde_json::json!({
                            "id": g.id,
                            "title": g.title,
                            "status": g.status.as_str(),
                            "progress_pct": g.progress_pct,
                            "source": g.source.as_str(),
                        });
                        if let Some(ref td) = g.target_date {
                            obj["target_date"] = Value::String(td.clone());
                        }
                        if let Some(pid) = g.parent_goal_id {
                            obj["parent_goal_id"] = Value::Number(pid.into());
                        }
                        if !g.milestones.is_empty() {
                            let done = g.milestones.iter().filter(|m| m.completed).count();
                            obj["milestones"] =
                                serde_json::json!(format!("{}/{}", done, g.milestones.len()));
                        }
                        obj
                    })
                    .collect();
                serde_json::json!({"status": "ok", "count": items.len(), "goals": items})
                    .to_string()
            }
            Err(e) => crate::utils::sanitized_tool_error(
                "manage_goals",
                "goal operation failed",
                &e,
                "internal",
                false,
                None,
            ),
        }
    }

    async fn goal_action_get(&self, args: &Value) -> String {
        let Some(goal_id) = args.get("goal_id").and_then(|v| v.as_i64()) else {
            return serde_json::json!({
                "status": "error",
                "error": "goal_id is required",
                "error_kind": "invalid_input",
                "retryable": false,
                "hint": "provide goal_id"
            })
            .to_string();
        };
        match self.memory.get_goal(goal_id).await {
            Ok(Some(g)) => {
                let milestones: Vec<Value> = g
                    .milestones
                    .iter()
                    .enumerate()
                    .map(|(i, m)| {
                        serde_json::json!({
                            "index": i,
                            "title": m.title,
                            "completed": m.completed,
                            "completed_at": m.completed_at,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "status": "ok",
                    "goal": {
                        "id": g.id,
                        "title": g.title,
                        "description": g.description,
                        "status": g.status.as_str(),
                        "progress_pct": g.progress_pct,
                        "parent_goal_id": g.parent_goal_id,
                        "milestones": milestones,
                        "created_at": g.created_at,
                        "target_date": g.target_date,
                        "last_progress_at": g.last_progress_at,
                        "source": g.source.as_str(),
                    }
                })
                .to_string()
            }
            Ok(None) => serde_json::json!({
                "status": "error",
                "error": format!("goal {goal_id} not found"),
                "error_kind": "not_found",
                "retryable": false,
                "hint": "use action='list' to see available goals"
            })
            .to_string(),
            Err(e) => crate::utils::sanitized_tool_error(
                "manage_goals",
                "goal operation failed",
                &e,
                "internal",
                false,
                None,
            ),
        }
    }

    async fn goal_action_set_status(&self, args: &Value, status: GoalStatus) -> String {
        let Some(goal_id) = args.get("goal_id").and_then(|v| v.as_i64()) else {
            return serde_json::json!({
                "status": "error",
                "error": "goal_id is required",
                "error_kind": "invalid_input",
                "retryable": false,
                "hint": "provide goal_id"
            })
            .to_string();
        };
        let updates = GoalUpdate {
            status: Some(status),
            progress_pct: if status == GoalStatus::Completed {
                Some(1.0)
            } else {
                None
            },
            ..Default::default()
        };
        match self.memory.update_goal(goal_id, updates).await {
            Ok(()) => serde_json::json!({
                "status": "ok",
                "goal_id": goal_id,
                "new_status": status.as_str(),
            })
            .to_string(),
            Err(e) => crate::utils::sanitized_tool_error(
                "manage_goals",
                "goal operation failed",
                &e,
                "internal",
                false,
                None,
            ),
        }
    }

    async fn goal_action_add_milestone(&self, args: &Value) -> String {
        let Some(goal_id) = args.get("goal_id").and_then(|v| v.as_i64()) else {
            return serde_json::json!({
                "status": "error",
                "error": "goal_id is required",
                "error_kind": "invalid_input",
                "retryable": false,
                "hint": "provide goal_id"
            })
            .to_string();
        };
        let title = args
            .get("milestone_title")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if title.is_empty() {
            return serde_json::json!({
                "status": "error",
                "error": "milestone_title is required",
                "error_kind": "invalid_input",
                "retryable": false,
                "hint": "provide milestone_title"
            })
            .to_string();
        }
        let source_summary = args.get("source_summary").and_then(|v| v.as_str());
        if let Err(e) = validate_email_source_write(
            &[title, source_summary.unwrap_or("")],
            EmailSourceWriteKind::FollowUpAllowed,
        ) {
            return serde_json::json!({
                "status": "error",
                "error": e,
                "error_kind": "unconfirmed_source",
                "retryable": true,
                "hint": "Use an explicit confirmation/review milestone if the email source is only a preview or has unread attachments."
            })
            .to_string();
        }
        match self.memory.add_milestone(goal_id, title).await {
            Ok(()) => serde_json::json!({
                "status": "ok",
                "goal_id": goal_id,
                "milestone_title": title,
            })
            .to_string(),
            Err(e) => crate::utils::sanitized_tool_error(
                "manage_goals",
                "goal operation failed",
                &e,
                "internal",
                false,
                None,
            ),
        }
    }

    async fn goal_action_complete_milestone(&self, args: &Value) -> String {
        let Some(goal_id) = args.get("goal_id").and_then(|v| v.as_i64()) else {
            return serde_json::json!({
                "status": "error",
                "error": "goal_id is required",
                "error_kind": "invalid_input",
                "retryable": false,
                "hint": "provide goal_id"
            })
            .to_string();
        };
        let Some(idx) = args.get("milestone_index").and_then(|v| v.as_u64()) else {
            return serde_json::json!({
                "status": "error",
                "error": "milestone_index is required",
                "error_kind": "invalid_input",
                "retryable": false,
                "hint": "provide 0-based milestone_index"
            })
            .to_string();
        };
        match self.memory.complete_milestone(goal_id, idx as usize).await {
            Ok(()) => serde_json::json!({
                "status": "ok",
                "goal_id": goal_id,
                "milestone_index": idx,
            })
            .to_string(),
            Err(e) => crate::utils::sanitized_tool_error(
                "manage_goals",
                "goal operation failed",
                &e,
                "internal",
                false,
                None,
            ),
        }
    }
}
