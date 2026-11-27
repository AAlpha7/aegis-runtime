//! Config-proposal and action-routing handlers for the Orchestrator.
//!
//! Extracted from `orchestrator.rs` for maintainability. The methods
//! (`create_config_proposal`, `handle_panel_submission`, `handle_admin_panel_action`)
//! remain on `impl Orchestrator` — they simply live in a separate file.
//!
//! `handle_panel_submission` outlived the Smart Panel / SDUI feature it was
//! named after (retired 2026-09-16): it is now the `[WEBAPP_SUBMISSION …]`
//! action router used by the web console's approval endpoint
//! (`admin/console_handlers.rs`, `admin_approve_hitl` / `admin_reject_hitl`)
//! and by `orchestrator/message_flow.rs` for task quick-actions. The
//! panel-store actions (`pin_panel`, `lock_panel`, `submit_feedback`,
//! `trigger_heartbeat`) were removed with the panels; nothing produces them.

use anyhow::Result;
use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
};
use serde_json::Value;

use crate::orchestrator::{Orchestrator, OrchestratorResult, SECS_PER_HOUR, push_tool_result};

impl Orchestrator {
    /// Create a config change proposal (HITL). Generates an approval ID,
    /// stores the proposal, sends an inline-button approval message to Telegram,
    /// and suspends the ReAct loop. Returns `Ok(false)` to signal budget exhaustion.
    ///
    /// Thin shim around [`create_config_proposal_ex`] for non-gate call sites
    /// (model-initiated tool calls, admin HTTP, panel submissions, etc.) that
    /// always want the full two-layer approval UX.
    ///
    /// `#[allow(dead_code)]` because production call sites now pass the
    /// `gate_pre_approved` flag via [`create_config_proposal_ex`] directly;
    /// the shim is kept as a test-fixture entry point and a stable API for
    /// any future caller that does not care about Gate pre-approval.
    #[allow(dead_code)]
    pub(crate) async fn create_config_proposal(
        &self,
        tool_call_id: &str,
        key: &str,
        proposed_value: Value,
        current_value: Value,
        user_did: &str,
        chat_id: Option<i64>,
        messages: &mut Vec<ChatCompletionRequestMessage>,
    ) -> Result<bool> {
        self.create_config_proposal_ex(
            tool_call_id,
            key,
            proposed_value,
            current_value,
            user_did,
            chat_id,
            messages,
            false, // default: Config proposal requires its own user approval
        )
        .await
    }

    /// Like [`create_config_proposal`] but threads a `gate_pre_approved` flag.
    ///
    /// When `gate_pre_approved` is `true`, the Gate layer (tool_gate
    /// RequireApproval) has already collected the user's approval for this
    /// exact tool+args combination. Instead of asking the user a second time
    /// (which is UX friction for a single intent), the proposal row is still
    /// written for audit, the setting is applied synchronously via
    /// [`Orchestrator::apply_approved_config`], a short bilingual
    /// confirmation is sent to the user, and a success tool_result is pushed
    /// so the ReAct loop can continue.
    ///
    /// MUST only be `true` when invoked from [`Orchestrator::execute_approved_tool`]
    /// (set via `ToolContext::gate_pre_approved`). Every other pathway —
    /// LLM-initiated tool calls in the normal loop, admin HTTP, heartbeat,
    /// MCP server, panel submissions — MUST pass `false` so the default
    /// two-layer HITL continues to guard against silent config drift.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn create_config_proposal_ex(
        &self,
        tool_call_id: &str,
        key: &str,
        proposed_value: Value,
        current_value: Value,
        user_did: &str,
        chat_id: Option<i64>,
        messages: &mut Vec<ChatCompletionRequestMessage>,
        gate_pre_approved: bool,
    ) -> Result<bool> {
        // Deduplicate: if this message history already contains a pending config
        // proposal (from a previous tool call in the same round), skip the duplicate.
        let already_pending = messages.iter().any(|m| {
            if let ChatCompletionRequestMessage::Tool(tool_msg) = m {
                let text = match &tool_msg.content {
                    async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(t) => {
                        t.as_str()
                    }
                    async_openai::types::chat::ChatCompletionRequestToolMessageContent::Array(
                        _,
                    ) => "",
                };
                text.contains("pending_human_approval") && text.contains(key)
            } else {
                false
            }
        });

        if already_pending {
            let dup_msg = serde_json::json!({
                "status": "duplicate",
                "message": format!("A config change for '{key}' is already pending approval. Please wait for the user to approve or reject it."),
            }).to_string();
            push_tool_result(messages, tool_call_id, &dup_msg)?;
            tracing::info!(key, "[HITL] Duplicate config proposal suppressed");
            return Ok(false);
        }

        let approval_id = format!("config-{:016x}", rand::random::<u64>());

        // Mark gate-pre-approved proposals in the stored record so the audit
        // trail reflects the shortcut (proposal was written but never sent for
        // user approval because the Gate already collected it).
        let proposal = serde_json::json!({
            "approval_id": approval_id,
            "key": key,
            "proposed_value": proposed_value,
            "current_value": current_value,
            "user_did": user_did,
            "chat_id": chat_id,
            "created_at": chrono::Utc::now().to_rfc3339(),
            "gate_pre_approved": gate_pre_approved,
        });
        if let Err(e) = self
            .memory
            .store_skill_state("hitl", &approval_id, &proposal.to_string())
            .await
        {
            tracing::warn!(error = %e, approval_id, "failed to store settings change HITL approval state");
        }
        if let Err(e) = record_runtime_setting_proposal_ledger(
            self.memory.db_path(),
            &approval_id,
            key,
            &current_value,
            &proposed_value,
            user_did,
            chat_id,
            tool_call_id,
            gate_pre_approved,
        ) {
            tracing::warn!(
                error = %e,
                approval_id,
                key,
                "failed to record runtime setting proposal in action ledger"
            );
        }

        // Format human-readable values for the approval message
        let fmt_val = |v: &Value| -> String {
            match v {
                Value::Bool(b) => {
                    if *b {
                        "ON".into()
                    } else {
                        "OFF".into()
                    }
                }
                Value::String(s) => s.clone(),
                other => other.to_string(),
            }
        };

        // ── Gate-pre-approved shortcut (Task #12) ─────────────────────────
        // Gate already collected the user's approval for this exact tool+args.
        // Apply synchronously via the shared helper (same code path as
        // handle_config_approval), notify the user with a single-line
        // confirmation (no buttons), push a success tool_result, and return
        // Ok(true) so the ReAct loop can continue — the action completed,
        // this is not a "budget exhausted / suspend loop" scenario.
        if gate_pre_approved {
            let apply_result = self
                .apply_approved_config(&approval_id, key, &proposed_value)
                .await;

            tracing::info!(
                approval_id = %approval_id,
                key = %key,
                "[HITL] Config proposal pre-approved via Gate — applied synchronously"
            );

            // Single-line confirmation to Telegram (no buttons).
            // Channel B — direct owner display, no LLM in the loop → localize
            // both the prefix AND the apply result to the owner's
            // display_language (spec §2/§6). `key` is a setting identifier;
            // the apply result's arbitrary args are tfp-rendered inside
            // `localized()`. Template: "[prefix] — setting 'key': current → proposed"
            let locale = self.settings.display_language();
            let prefix = crate::i18n::t("hitl.config_gate_pre_approved", &locale);
            let confirmation = format!(
                "{}\n\n⚙️ `{}`: {} → {}\n{}",
                prefix,
                key,
                fmt_val(&current_value),
                fmt_val(&proposed_value),
                apply_result.localized(&locale),
            );
            if let (Some(tx), Some(cid)) = (&self.progress_tx, chat_id) {
                let _ = tx.try_send(crate::progress_router::ProgressEvent::new(
                    cid,
                    crate::progress_router::ProgressPayload::OwnerNotice {
                        text: confirmation,
                        kind: crate::progress_router::OwnerNoticeKind::System,
                    },
                ));
            }

            // Push a success tool_result so the ReAct loop (or gate replay
            // caller) has a concrete result string to surface back to the
            // user / LLM. Status uses "ok" (not "pending_human_approval") so
            // downstream ReAct loops don't wait on a non-existent prompt.
            // Channel A — this string is LLM-bound; keep the raw English
            // callback form (do NOT localize).
            let tool_result = serde_json::json!({
                "status": "ok",
                "approval_id": approval_id,
                "key": key,
                "result": apply_result.callback_string(),
                "gate_pre_approved": true,
            })
            .to_string();
            push_tool_result(messages, tool_call_id, &tool_result)?;

            // Clear the HITL row — the proposal has been applied, there is
            // no pending approval to resolve. Audit trail is preserved in
            // the initial write + the tracing::info! above.
            if let Err(e) = self
                .memory
                .store_skill_state("hitl", &approval_id, "")
                .await
            {
                tracing::warn!(
                    error = %e,
                    approval_id,
                    "Failed to clear gate-pre-approved HITL state"
                );
            }

            return Ok(true);
        }

        // Paired-Fix (CTO round-3): escape LLM/user-authored substitutions
        // before insertion into the Telegram HTML approval template.
        // `key` is a setting name (bounded charset) but escape for defense-
        // in-depth. `fmt_val` returns serialized JSON which may contain
        // `<`/`&`/`>` in string values. `description_line` can reference
        // skill names from LLM input.
        let description_line = crate::settings::describe_system_setting(key)
            .map(|d| format!("**What it controls:** {}\n", crate::telegram_render::escape_html(&d)))
            .unwrap_or_else(|| {
                if key.starts_with("skill_") {
                    format!(
                        "**What it controls:** enable/disable the `{}` skill (autonomous + LLM callback modes)\n",
                        crate::telegram_render::escape_html(
                            key.strip_prefix("skill_").unwrap_or(key)
                        )
                    )
                } else {
                    String::new()
                }
            });
        let approval_id_esc = crate::telegram_render::escape_html(&approval_id);
        let key_esc = crate::telegram_render::escape_html(key);
        let current_esc = crate::telegram_render::escape_html(&fmt_val(&current_value));
        let proposed_esc = crate::telegram_render::escape_html(&fmt_val(&proposed_value));

        let approval_msg = format!(
            "⚙️ **Config Change Approval Request**\n\n\
             Approval ID: `{}`\n\n\
             Origin: LLM tool call `update_setting` (ReAct orchestrator)\n\n\
             **Setting:** `{}`\n\
             {}\
             **Current value:** {}\n\
             **Proposed value:** {}\n\n\
             **Revert:** if you change your mind after approving, run \
             `update_setting` again with `value={}` (or use `/settings` to reset to default).\n\n\
             Reply with:\n\
             • `approve config {0}` to apply\n\
             • `reject config {0}` to cancel",
            approval_id_esc, key_esc, description_line, current_esc, proposed_esc, current_esc,
        );

        if let (Some(tx), Some(cid)) = (&self.progress_tx, chat_id) {
            let _ = tx.try_send(crate::progress_router::ProgressEvent::new(
                cid,
                crate::progress_router::ProgressPayload::NeedApproval {
                    approval_id: Some(approval_id.clone()),
                    tool_name: Some("update_setting".to_string()),
                    summary: None,
                    approval_html: approval_msg,
                },
            ));
        }

        tracing::info!(
            approval_id = %approval_id,
            key = %key,
            "[HITL] Config change proposal created — suspending ReAct loop"
        );

        let pending_result = format!(
            "[TOOL_RESULT — HITL approval required]\n\
             Status: pending_human_approval\n\
             Approval ID: {}\n\
             Setting: {} (current: {} → proposed: {})\n\
             The proposed configuration change has been sent to the user for review via Telegram.\n\
             [/TOOL_RESULT]",
            approval_id,
            key,
            fmt_val(&current_value),
            fmt_val(&proposed_value)
        );
        push_tool_result(messages, tool_call_id, &pending_result)?;
        messages.push(
            ChatCompletionRequestSystemMessageArgs::default()
                .content(
                    "[SYSTEM] A configuration change requiring user approval has been \
                     submitted via Telegram. You MUST NOT call any more tools. Tell the user \
                     that the proposed change has been sent for review and they can approve or \
                     reject it.",
                )
                .build()?
                .into(),
        );

        Ok(false)
    }

    /// Handle silent panel actions from WebApp submissions.
    /// Returns Some(result) if the submission was handled, None to fall through.
    pub(crate) async fn handle_panel_submission(
        &self,
        message: &str,
        chat_id: i64,
    ) -> Option<OrchestratorResult> {
        // Parse the action from the message
        // Format: [WEBAPP_SUBMISSION callback_id=xxx action=submit]\n{data}
        let data_start = message.find('\n')?;
        let data_str = &message[data_start + 1..];
        let data: Value = serde_json::from_str(data_str).ok()?;

        // Check if any button/input action matches panel patterns
        let obj = data.as_object()?;

        let action = obj
            .keys()
            .find(|k| {
                k.starts_with("mark_done:")
                    || k.starts_with("task_done:")
                    || k.starts_with("task_archive:")
                    || k.starts_with("task_skip:")
                    || k.starts_with("task_delete:")
                    || k.starts_with("task_restore:")
                    || k.starts_with("task_clear_finished:")
                    || k.starts_with("dismiss:")
                    || k.starts_with("admin_")
            })?
            .clone();

        let (action_type, id_str) = action.split_once(':')?;

        // Admin actions use string identifiers (skill names, HITL keys), not numeric IDs
        if action_type.starts_with("admin_") {
            return self
                .handle_admin_panel_action(action_type, id_str, chat_id)
                .await;
        }

        let target_id: i64 = id_str.parse().ok()?;

        // Channel B — panel-button action results render in the owner's
        // display_language (spec §2/§6). DISPLAY text only; action decisions,
        // match arms, callback_data, ids, and state transitions are unchanged.
        let locale = self.settings.display_language();

        match action_type {
            "mark_done" => {
                if let Err(e) = self
                    .memory
                    .update_knowledge_entry(
                        target_id,
                        chat_id,
                        crate::memory::KnowledgeEntryUpdate {
                            status: Some("done".into()),
                            ..Default::default()
                        },
                    )
                    .await
                {
                    tracing::warn!(error = %e, target_id, "Failed to mark knowledge entry as done");
                }
                Some(OrchestratorResult {
                    reply: crate::i18n::t("panel.action.done", &locale).into(),
                    ..Default::default()
                })
            }
            "task_done" => {
                let now = chrono::Utc::now().to_rfc3339();
                // Scheduler + panel queries filter by status='active', so trigger_*
                // columns on a done task are inert; explicit clearing is unnecessary.
                let updates = serde_json::json!({
                    "status": "done",
                    "completed_at": now,
                });
                match self
                    .memory
                    .update_task(target_id, chat_id, &updates.to_string())
                    .await
                {
                    Ok(true) => Some(OrchestratorResult {
                        reply: crate::i18n::t("panel.action.done", &locale).into(),
                        ..Default::default()
                    }),
                    Ok(false) => Some(OrchestratorResult {
                        reply: crate::i18n::t("panel.action.task_not_found_or_done", &locale)
                            .into(),
                        ..Default::default()
                    }),
                    Err(e) => {
                        tracing::warn!(task_id = target_id, error = %e, "task_done update failed");
                        Some(OrchestratorResult {
                            reply: format!(
                                "{}{e}",
                                crate::i18n::t("panel.action.complete_failed", &locale)
                            ),
                            ..Default::default()
                        })
                    }
                }
            }
            "task_archive" => {
                // Scheduler filters by status='active'; trigger_* on an archived task
                // is inert, no explicit clear needed.
                let updates = serde_json::json!({
                    "status": "archived",
                });
                match self
                    .memory
                    .update_task(target_id, chat_id, &updates.to_string())
                    .await
                {
                    Ok(true) => Some(OrchestratorResult {
                        reply: crate::i18n::t("panel.action.archived", &locale).into(),
                        ..Default::default()
                    }),
                    Ok(false) => Some(OrchestratorResult {
                        reply: crate::i18n::t("panel.action.task_not_found", &locale).into(),
                        ..Default::default()
                    }),
                    Err(e) => {
                        tracing::warn!(task_id = target_id, error = %e, "task_archive failed");
                        Some(OrchestratorResult {
                            reply: format!(
                                "{}{e}",
                                crate::i18n::t("panel.action.archive_failed", &locale)
                            ),
                            ..Default::default()
                        })
                    }
                }
            }
            "task_skip" => {
                // Skip one occurrence of a recurring task: reschedule to next interval.
                if let Some(task) = self.memory.get_task(target_id).await {
                    if task.chat_id != chat_id {
                        return Some(OrchestratorResult {
                            reply: crate::i18n::t("panel.action.task_not_found", &locale).into(),
                            ..Default::default()
                        });
                    }
                    if let Some(interval) = task.interval_seconds {
                        let now = chrono::Utc::now().timestamp();
                        let next = task
                            .trigger_at_unix
                            .map(|t| t + interval)
                            .unwrap_or(now + interval);
                        let expires = next + crate::intent::MAX_INTENT_LIFETIME_SECS;
                        match self
                            .memory
                            .reschedule_recurring_intent(target_id, next, expires)
                            .await
                        {
                            Ok(_) => Some(OrchestratorResult {
                                reply: crate::i18n::t("panel.action.skipped", &locale).into(),
                                ..Default::default()
                            }),
                            Err(e) => {
                                tracing::warn!(task_id = target_id, error = %e, "task_skip reschedule failed");
                                Some(OrchestratorResult {
                                    reply: format!(
                                        "{}{e}",
                                        crate::i18n::t("panel.action.skip_failed", &locale)
                                    ),
                                    ..Default::default()
                                })
                            }
                        }
                    } else {
                        // Not recurring — treat as done. Same rationale as task_done:
                        // scheduler filters by status so trigger_* clears are inert.
                        let now = chrono::Utc::now().to_rfc3339();
                        let updates = serde_json::json!({
                            "status": "done",
                            "completed_at": now,
                        });
                        if let Err(e) = self
                            .memory
                            .update_task(target_id, chat_id, &updates.to_string())
                            .await
                        {
                            tracing::warn!(error = %e, target_id, "Failed to mark task as done");
                        }
                        Some(OrchestratorResult {
                            reply: crate::i18n::t("panel.action.done_not_recurring", &locale)
                                .into(),
                            ..Default::default()
                        })
                    }
                } else {
                    Some(OrchestratorResult {
                        reply: crate::i18n::t("panel.action.task_not_found", &locale).into(),
                        ..Default::default()
                    })
                }
            }
            "task_restore" => {
                // Restoring must actually clear completed_at — if we leave it set
                // the task will appear both active and "completed" downstream.
                // Uses the clear_fields convention (allow-listed nullable columns
                // in memory.rs maybe_set!).
                let updates = serde_json::json!({
                    "status": "active",
                    "clear_fields": ["completed_at"],
                });
                match self
                    .memory
                    .update_task(target_id, chat_id, &updates.to_string())
                    .await
                {
                    Ok(true) => Some(OrchestratorResult {
                        reply: crate::i18n::t("panel.action.restored", &locale).into(),
                        ..Default::default()
                    }),
                    Ok(false) => Some(OrchestratorResult {
                        reply: crate::i18n::t("panel.action.task_not_found", &locale).into(),
                        ..Default::default()
                    }),
                    Err(e) => {
                        tracing::warn!(task_id = target_id, error = %e, "task_restore failed");
                        Some(OrchestratorResult {
                            reply: format!(
                                "{}{e}",
                                crate::i18n::t("panel.action.restore_failed", &locale)
                            ),
                            ..Default::default()
                        })
                    }
                }
            }
            "task_clear_finished" => {
                // Delete all done/archived tasks for this chat
                match self.memory.clear_finished_tasks(chat_id).await {
                    Ok(count) => Some(OrchestratorResult {
                        // count is a non-negative integer (digits only, no braces),
                        // but route through tfp (single-pass) per the arbitrary-arg
                        // safety rule rather than tf's re-scanning replace.
                        reply: crate::i18n::tfp(
                            "panel.action.cleared_finished",
                            &locale,
                            &[&count.to_string()],
                        ),
                        ..Default::default()
                    }),
                    Err(e) => {
                        tracing::warn!(error = %e, "task_clear_finished failed");
                        Some(OrchestratorResult {
                            reply: format!(
                                "{}{e}",
                                crate::i18n::t("panel.action.clear_failed", &locale)
                            ),
                            ..Default::default()
                        })
                    }
                }
            }
            "task_delete" => match self.memory.delete_task(target_id, chat_id).await {
                Ok(true) => Some(OrchestratorResult {
                    reply: crate::i18n::t("panel.action.deleted", &locale).into(),
                    ..Default::default()
                }),
                Ok(false) => Some(OrchestratorResult {
                    reply: crate::i18n::t("panel.action.delete_only_finished", &locale).into(),
                    ..Default::default()
                }),
                Err(e) => {
                    tracing::warn!(task_id = target_id, error = %e, "task_delete failed");
                    Some(OrchestratorResult {
                        reply: format!(
                            "{}{e}",
                            crate::i18n::t("panel.action.delete_failed", &locale)
                        ),
                        ..Default::default()
                    })
                }
            },
            "dismiss" => {
                if let Err(e) = self
                    .memory
                    .update_knowledge_entry(
                        target_id,
                        chat_id,
                        crate::memory::KnowledgeEntryUpdate {
                            status: Some("archived".into()),
                            ..Default::default()
                        },
                    )
                    .await
                {
                    tracing::warn!(error = %e, target_id, "Failed to archive knowledge entry");
                }
                Some(OrchestratorResult {
                    reply: crate::i18n::t("panel.action.archived", &locale).into(),
                    ..Default::default()
                })
            }
            _ => None,
        }
    }

    /// Handle admin panel actions (skill management, proposal control, intent cancel, HITL).
    async fn handle_admin_panel_action(
        &self,
        action_type: &str,
        id_str: &str,
        chat_id: i64,
    ) -> Option<OrchestratorResult> {
        let reply = match action_type {
            "admin_deactivate_skill" => {
                if self.deactivate_skill_by_name(id_str).await {
                    format!("⏹ 已停用技能 {id_str}")
                } else {
                    format!("技能 {id_str} 未找到")
                }
            }
            "admin_reactivate_skill" => match self.reactivate_skill_by_name(id_str).await {
                Ok(msg) => format!("▶️ 已激活技能 {id_str}: {msg}"),
                Err(e) => {
                    tracing::warn!(skill = %id_str, error = %format!("{e:#}"), "admin reactivate_skill failed");
                    format!("激活失败: {e}")
                }
            },
            "admin_reset_breaker" => {
                self.memory.reset_circuit_breaker(id_str).await;
                format!("🔄 已重置 {id_str} 熔断器")
            }
            "admin_cancel_intent" => {
                if let Ok(intent_id) = id_str.parse::<i64>() {
                    let user_did = format!("did:aegis:telegram:{}", chat_id);
                    match self.memory.cancel_intent(intent_id, &user_did).await {
                        Ok(true) => format!("❌ 已取消任务 #{intent_id}"),
                        Ok(false) => format!("任务 #{intent_id} 未找到"),
                        Err(e) => {
                            tracing::warn!(id = intent_id, error = %e, "admin cancel_intent failed");
                            format!("取消失败: {e}")
                        }
                    }
                } else {
                    "无效ID".into()
                }
            }
            "admin_view_intent" => {
                if let Ok(intent_id) = id_str.parse::<i64>() {
                    let user_did = format!("did:aegis:telegram:{}", chat_id);
                    let intents = self
                        .memory
                        .list_user_intents(&user_did)
                        .await
                        .unwrap_or_default();
                    if let Some(intent) = intents.iter().find(|i| i.id == intent_id) {
                        let trigger = if let Some(ts) = intent.trigger_at_unix {
                            chrono::DateTime::from_timestamp(ts, 0)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                                .unwrap_or_else(|| "—".into())
                        } else if let Some(ref ek) = intent.event_key {
                            format!("on event: {ek}")
                        } else {
                            "—".into()
                        };
                        let interval = intent
                            .interval_seconds
                            .map(|s| format!("every {}", format_interval(s)))
                            .unwrap_or_else(|| "一次性".into());
                        format!(
                            "📋 Intent #{}\n\
                             类型: {}\n\
                             模式: {}\n\
                             触发: {trigger}\n\
                             间隔: {interval}\n\
                             触发次数: {}/{}\n\
                             ---\n\
                             {}",
                            intent.id,
                            intent.trigger_type,
                            intent.execution_mode,
                            intent.fire_count,
                            intent.max_fires.unwrap_or(0),
                            intent.context_text,
                        )
                    } else {
                        format!("任务 #{intent_id} 未找到")
                    }
                } else {
                    "无效ID".into()
                }
            }
            "admin_approve_hitl" | "admin_reject_hitl" | "admin_gate_once"
            | "admin_gate_intent" | "admin_gate_domain" | "admin_gate_always" => {
                match self.route_hitl_action(action_type, id_str, chat_id).await {
                    Some(result) => return Some(result),
                    None => "HITL action failed unexpectedly".to_string(),
                }
            }
            "remove_monitoring" => {
                // id_str is a task ID (from unified tasks table)
                let intent_id: i64 = id_str.parse().unwrap_or(0);
                if intent_id == 0 {
                    format!("无效的监控ID: {id_str}")
                } else {
                    // Look up the skill name before deleting (for chronos uncache)
                    let user_did = format!("did:aegis:telegram:{}", chat_id);
                    let intents = self
                        .memory
                        .list_monitoring_intents(&user_did)
                        .await
                        .unwrap_or_default();
                    let skill_name = intents
                        .iter()
                        .find(|r| r.id == intent_id)
                        .map(|r| r.monitoring_skill.clone());

                    match self.memory.delete_monitoring_intent(intent_id).await {
                        Ok(()) => {
                            if let Some(skill) = &skill_name
                                && let Some(chronos) = self.services.chronos.lock().as_ref()
                            {
                                chronos.uncache_intent(&user_did, skill);
                            }
                            format!("❌ 已取消监控 #{intent_id}")
                        }
                        Err(e) => {
                            tracing::warn!(intent_id = intent_id, error = %e, "admin remove_monitoring failed");
                            format!("取消监控失败: {e}")
                        }
                    }
                }
            }
            _ => return None,
        };

        Some(OrchestratorResult {
            reply,
            ..Default::default()
        })
    }
}

/// Format a duration in seconds into a human-readable string.
fn format_interval(secs: i64) -> String {
    if secs <= 0 {
        return "—".into();
    }
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{}d", days)
    } else if hours > 0 {
        format!("{}h", hours)
    } else {
        format!("{}min", mins.max(1))
    }
}

#[allow(clippy::too_many_arguments)]
fn record_runtime_setting_proposal_ledger(
    db_path: &str,
    approval_id: &str,
    key: &str,
    current_value: &Value,
    proposed_value: &Value,
    requested_by: &str,
    chat_id: Option<i64>,
    tool_call_id: &str,
    gate_pre_approved: bool,
) -> Result<()> {
    let conn = crate::encrypted_sqlite::open(db_path)?;
    crate::action_ledger::record_runtime_setting_proposed(
        &conn,
        approval_id,
        key,
        current_value,
        proposed_value,
        requested_by,
        chat_id,
        Some(tool_call_id),
        gate_pre_approved,
    )?;
    Ok(())
}

impl Orchestrator {
    // ── HITL approval routing (extracted from handle_admin_panel_action) ──

    /// Record one owner decision and clear every pending row of that hold.
    ///
    /// Returns true when `hitl_key` is a tool hold. Other HITL rows stay on
    /// the single-key clear in the caller. Sibling `gate-` rows of the same
    /// egress facts (or the same canonical tool args) are cleared together so
    /// the work list cannot keep an identical card after this one is decided.
    async fn settle_owner_tool_hold(&self, hitl_key: &str, decision: &str) -> bool {
        let Ok(Some(raw)) = self.memory.load_skill_state("hitl", hitl_key).await else {
            return false;
        };
        if raw.trim().is_empty() {
            return false;
        }
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return false;
        };
        self.settle_loaded_tool_hold(hitl_key, decision, &parsed)
            .await
    }

    /// Record `decision` for a tool hold and clear every pending sibling.
    ///
    /// `parsed` is the row as it was before replay. Gate replay clears the
    /// approved key itself, so the caller keeps this snapshot.
    async fn settle_loaded_tool_hold(
        &self,
        hitl_key: &str,
        decision: &str,
        parsed: &serde_json::Value,
    ) -> bool {
        let tool_name = parsed
            .get("tool_name")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if tool_name.is_empty() {
            return false;
        }
        let tool_args = parsed
            .get("tool_args")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        let summary = parsed
            .get("approval_summary")
            .and_then(|value| value.as_str())
            .or_else(|| parsed.get("summary").and_then(|value| value.as_str()))
            .unwrap_or("")
            .to_string();
        let identity = crate::egress_gate::owner_hold_identity(&tool_name, &tool_args, &summary);
        let visible_summary = if crate::egress_gate::parse_card_summary(&summary).is_some() {
            summary.chars().take(2_000).collect::<String>()
        } else {
            String::new()
        };
        let record = serde_json::json!({
            "approval_id": hitl_key,
            "tool_name": tool_name,
            "tool_args": tool_args,
            "approval_summary": visible_summary,
            "state": decision,
            "resolved_at": chrono::Utc::now().to_rfc3339(),
            "user_did": parsed.get("user_did").cloned().unwrap_or(serde_json::Value::Null),
            "requested_by": parsed.get("requested_by").cloned().unwrap_or(serde_json::Value::Null),
            "chat_id": parsed.get("chat_id").cloned().unwrap_or(serde_json::Value::Null),
            "created_at": parsed.get("created_at").cloned().unwrap_or(serde_json::Value::Null),
        });
        if let Err(error) = self
            .memory
            .store_skill_state(
                crate::egress_gate::HITL_RESOLVED_SKILL,
                hitl_key,
                &record.to_string(),
            )
            .await
        {
            tracing::warn!(error = %error, key = %hitl_key, "failed to record resolved tool hold");
        }
        let pending = self.memory.list_hitl_pending().await.unwrap_or_default();
        for (key, value) in pending {
            let same = key == hitl_key
                || serde_json::from_str::<serde_json::Value>(&value)
                    .ok()
                    .is_some_and(|row| {
                        let row_tool = row
                            .get("tool_name")
                            .and_then(|item| item.as_str())
                            .unwrap_or("");
                        let row_args = row
                            .get("tool_args")
                            .and_then(|item| item.as_str())
                            .unwrap_or("");
                        let row_summary = row
                            .get("approval_summary")
                            .and_then(|item| item.as_str())
                            .or_else(|| row.get("summary").and_then(|item| item.as_str()))
                            .unwrap_or("");
                        !row_tool.trim().is_empty()
                            && crate::egress_gate::owner_hold_identity(
                                row_tool,
                                row_args,
                                row_summary,
                            ) == identity
                    });
            if !same {
                continue;
            }
            if let Err(error) = self.memory.store_skill_state("hitl", &key, "").await {
                tracing::warn!(error = %error, key = %key, "failed to clear duplicate tool hold");
            }
        }
        true
    }

    /// Replay a stored tool call the way the Telegram gate does, then keep
    /// one approved card. Returns `None` when the row is not a stored call,
    /// so the caller can clear a non-tool approval without executing anything.
    /// Rejection never reaches this function.
    async fn replay_approved_tool_hold(
        &self,
        hitl_key: &str,
        chat_id: i64,
    ) -> Option<OrchestratorResult> {
        let raw = self
            .memory
            .load_skill_state("hitl", hitl_key)
            .await
            .ok()
            .flatten()?;
        if raw.trim().is_empty() {
            return None;
        }
        let parsed = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
        let tool_name = parsed
            .get("tool_name")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();
        let tool_args = parsed
            .get("tool_args")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if tool_name.is_empty() || tool_args.trim().is_empty() {
            return None;
        }
        let user_did = parsed
            .get("user_did")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("did:aegis:telegram:{chat_id}"));
        let search_hold = crate::egress_gate::is_search_egress_tool(tool_name);
        let outcome = self
            .handle_gate_approval_scoped(
                hitl_key,
                crate::tool_gate::ApprovalScope::Once,
                &user_did,
                Some(chat_id),
            )
            .await;
        let dispatched = self
            .memory
            .load_skill_state(crate::egress_gate::HITL_DISPATCH_SKILL, hitl_key)
            .await
            .ok()
            .flatten()
            .is_some_and(|value| !value.trim().is_empty());
        if dispatched {
            // The gate already cleared this key. Record the card from the
            // snapshot and drop sibling rows of the same egress identity.
            self.settle_loaded_tool_hold(hitl_key, "approved", &parsed)
                .await;
        }
        match outcome {
            Ok(result) => {
                if search_hold {
                    self.attach_search_hold_outcome(hitl_key, &result.reply)
                        .await;
                }
                Some(result)
            }
            Err(error) => {
                tracing::warn!(error = %error, key = %hitl_key, "web tool-hold replay failed");
                let reply = format!("审批失败: {error}");
                if search_hold {
                    self.attach_search_hold_outcome(hitl_key, &reply).await;
                }
                Some(OrchestratorResult {
                    reply,
                    ..Default::default()
                })
            }
        }
    }

    /// Keep the replayed search on the resolved card so a refresh still shows
    /// what happened after 已批准.
    async fn attach_search_hold_outcome(&self, hitl_key: &str, reply: &str) {
        let Ok(Some(raw)) = self
            .memory
            .load_skill_state(crate::egress_gate::HITL_RESOLVED_SKILL, hitl_key)
            .await
        else {
            return;
        };
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return;
        };
        let Some(object) = value.as_object_mut() else {
            return;
        };
        object.insert(
            "outcome".to_string(),
            serde_json::Value::String(crate::search_turn::owner_search_outcome_text(reply)),
        );
        if let Err(error) = self
            .memory
            .store_skill_state(
                crate::egress_gate::HITL_RESOLVED_SKILL,
                hitl_key,
                &value.to_string(),
            )
            .await
        {
            tracing::warn!(error = %error, key = %hitl_key, "failed to store search hold outcome");
        }
    }

    /// Route a HITL approval/rejection action to the appropriate handler.
    /// Returns `Some(OrchestratorResult)` with the reply, or `None` on unexpected failure.
    async fn route_hitl_action(
        &self,
        action_type: &str,
        hitl_key: &str,
        chat_id: i64,
    ) -> Option<OrchestratorResult> {
        let is_approve = !action_type.contains("reject");

        // Check if the pending approval has expired (>24h)
        if is_approve
            && let Ok(Some(data)) = self.memory.load_skill_state("hitl", hitl_key).await
            && !data.is_empty()
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data)
            && let Some(created_str) = parsed.get("created_at").and_then(|v| v.as_str())
            && let Ok(created) = chrono::DateTime::parse_from_rfc3339(created_str)
        {
            let age_secs = (chrono::Utc::now() - created.with_timezone(&chrono::Utc)).num_seconds();
            if age_secs > Self::HITL_EXPIRY_SECS {
                if let Err(e) = self.memory.store_skill_state("hitl", hitl_key, "").await {
                    tracing::warn!(error = %e, key = %hitl_key, "failed to clear expired HITL entry");
                }
                let hours = age_secs / SECS_PER_HOUR;
                return Some(OrchestratorResult {
                    reply: format!(
                        "⏰ 此审批请求已过期（已等待 {}h，超过 24h 上限）。\n请重新触发操作以生成新的审批请求。",
                        hours
                    ),
                    ..Default::default()
                });
            }
        }

        let reply = if !is_approve {
            // Audit Secure CU / Jev non-CU Ask rejects (never sign / never execute).
            if let Ok(Some(data)) = self.memory.load_skill_state("hitl", hitl_key).await
                && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data)
            {
                let kind = parsed.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                if kind == "secure_cu_ask"
                    && let Ok(pending) = serde_json::from_value::<crate::computer_use::PendingAsk>(
                        parsed.get("pending_ask").cloned().unwrap_or_default(),
                    )
                    && let Ok(pipeline) = self.secure_cu_pipeline()
                {
                    let _ = pipeline.reject_ask(&pending, "owner rejected via HITL");
                }
            }
            if !self.settle_owner_tool_hold(hitl_key, "rejected").await
                && let Err(e) = self.memory.store_skill_state("hitl", hitl_key, "").await
            {
                tracing::warn!(error = %e, key = %hitl_key, "failed to clear rejected HITL entry");
            }
            format!("{} 已拒绝 {hitl_key}", crate::search_turn::NOT_SENT)
        } else if hitl_key.starts_with("cu-ask-") || hitl_key.starts_with("jev-ask-") {
            let user_did = format!("did:aegis:telegram:{}", chat_id);
            match self
                .handle_gate_approval_scoped(
                    hitl_key,
                    crate::tool_gate::ApprovalScope::Once,
                    &user_did,
                    Some(chat_id),
                )
                .await
            {
                Ok(result) => return Some(result),
                Err(e) => {
                    tracing::warn!(error = %e, key = %hitl_key, "ask HITL approval failed");
                    format!("审批失败: {e}")
                }
            }
        } else if action_type.starts_with("admin_gate_") {
            let scope_str = action_type.strip_prefix("admin_gate_").unwrap_or("once");
            let scope = match scope_str {
                "once" => crate::tool_gate::ApprovalScope::Once,
                "always" => crate::tool_gate::ApprovalScope::Always,
                "intent" => {
                    let data = self
                        .memory
                        .load_skill_state("hitl", hitl_key)
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    let parsed: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();
                    let gk = parsed
                        .get("group_key")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    crate::tool_gate::ApprovalScope::Intent { group_key: gk }
                }
                "domain" => {
                    let data = self
                        .memory
                        .load_skill_state("hitl", hitl_key)
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    let parsed: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();
                    let host = parsed
                        .get("host")
                        .or_else(|| parsed.get("domain"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    crate::tool_gate::ApprovalScope::Domain { host }
                }
                _ => crate::tool_gate::ApprovalScope::Once,
            };
            let user_did = format!("did:aegis:telegram:{}", chat_id);
            match self
                .handle_gate_approval_scoped(hitl_key, scope, &user_did, Some(chat_id))
                .await
            {
                Ok(result) => return Some(result),
                Err(e) => {
                    tracing::warn!(error = %e, key = %hitl_key, "admin gate approval failed");
                    format!("审批失败: {e}")
                }
            }
        } else if hitl_key.contains("sysop") || hitl_key.starts_with("sysop_") {
            match self.handle_sysop_approval(hitl_key, Some(chat_id)).await {
                Ok(result) => return Some(result),
                Err(e) => format!("审批失败: {e}"),
            }
        } else if hitl_key.contains("config") || hitl_key.starts_with("config_") {
            match self.handle_config_approval(hitl_key, Some(chat_id)).await {
                Ok(result) => return Some(result),
                Err(e) => format!("审批失败: {e}"),
            }
        } else if hitl_key.contains("menu") || hitl_key.starts_with("menu_") {
            match self.handle_menu_approval(hitl_key, Some(chat_id)).await {
                Ok(result) => return Some(result),
                Err(e) => format!("审批失败: {e}"),
            }
        } else if hitl_key.contains("revoke") || hitl_key.starts_with("revoke_") {
            match self.handle_revoke_approval(hitl_key, Some(chat_id)).await {
                Ok(result) => return Some(result),
                Err(e) => format!("审批失败: {e}"),
            }
        } else if let Some(result) = self.replay_approved_tool_hold(hitl_key, chat_id).await {
            return Some(result);
        } else {
            if !self.settle_owner_tool_hold(hitl_key, "approved").await
                && let Err(e) = self.memory.store_skill_state("hitl", hitl_key, "").await
            {
                tracing::warn!(error = %e, key = %hitl_key, "failed to clear HITL entry after generic approval");
            }
            format!("✅ 已审批 {hitl_key}")
        };

        Some(OrchestratorResult {
            reply,
            ..Default::default()
        })
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::format_interval;
    use crate::memory::MemoryHandle;
    use crate::settings::RuntimeSettings;
    use crate::test_tool_helpers::test_orchestrator;
    use std::sync::Arc;

    /// Build a panel-submission message with the right header format.
    fn webapp_msg(json: &str) -> String {
        format!("[WEBAPP_SUBMISSION callback_id=test action=submit]\n{json}")
    }

    /// Helper: create an Orchestrator with in-memory DB.
    async fn setup() -> crate::orchestrator::Orchestrator {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let settings = Arc::new(RuntimeSettings::new());
        test_orchestrator(mem, settings)
    }

    // ── handle_panel_submission: parse failures ────────────────────────────

    #[tokio::test]
    async fn submission_no_newline_returns_none() {
        let orch = setup().await;
        assert!(
            orch.handle_panel_submission("no-newline-here", 1)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn submission_invalid_json_returns_none() {
        let orch = setup().await;
        let msg = webapp_msg("not json");
        assert!(orch.handle_panel_submission(&msg, 1).await.is_none());
    }

    #[tokio::test]
    async fn submission_empty_object_returns_none() {
        let orch = setup().await;
        let msg = webapp_msg("{}");
        assert!(orch.handle_panel_submission(&msg, 1).await.is_none());
    }

    #[tokio::test]
    async fn submission_unknown_action_returns_none() {
        let orch = setup().await;
        let msg = webapp_msg(r#"{"something_random:99": true}"#);
        assert!(orch.handle_panel_submission(&msg, 1).await.is_none());
    }

    // ── handle_panel_submission: task actions ──────────────────────────────

    /// Store a minimal task and return its ID.
    async fn seed_task(mem: &MemoryHandle, chat_id: i64) -> i64 {
        mem.store_task(
            chat_id,
            "did:test:user",
            "Test Task",
            "body",
            "normal",
            &[],
            None,
            "test",
            None,
            None,
            None,
            "autonomous",
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
        .unwrap()
    }

    #[tokio::test]
    async fn task_done_marks_task() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 100;
        let tid = seed_task(&mem, cid).await;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        let msg = webapp_msg(&format!(r#"{{"task_done:{tid}": true}}"#));
        let result = orch.handle_panel_submission(&msg, cid).await.unwrap();
        // Default display_language is zh.
        assert_eq!(result.reply, "✅ 已完成");

        // Verify task status in DB
        let task = mem.get_task(tid).await.unwrap();
        assert_eq!(task.status, "done");
    }

    /// Channel-B i18n: the SAME panel-button action renders its owner-facing
    /// result in the owner's display_language, while the action decision and DB
    /// state transition are identical regardless of locale. Exercises the real
    /// entry path (handle_panel_submission) for both zh and en.
    #[tokio::test]
    async fn panel_action_results_localize_under_display_language() {
        for (loc, done, archived, deleted) in [
            ("zh", "✅ 已完成", "🗑 已归档", "🔴 已永久删除"),
            ("en", "✅ Done", "🗑 Archived", "🔴 Permanently deleted"),
        ] {
            let mem = MemoryHandle::spawn_in_memory().unwrap();
            let cid = 4242;
            let settings = Arc::new(RuntimeSettings::new());
            settings
                .set("display_language", serde_json::json!(loc))
                .unwrap();
            let orch = test_orchestrator(mem.clone(), settings);

            // task_done — result localized, DB transition identical.
            let tid = seed_task(&mem, cid).await;
            let msg = webapp_msg(&format!(r#"{{"task_done:{tid}": true}}"#));
            let r = orch.handle_panel_submission(&msg, cid).await.unwrap();
            assert_eq!(r.reply, done, "task_done reply ({loc})");
            assert_eq!(mem.get_task(tid).await.unwrap().status, "done");

            // task_archive
            let tid2 = seed_task(&mem, cid).await;
            let msg = webapp_msg(&format!(r#"{{"task_archive:{tid2}": true}}"#));
            let r = orch.handle_panel_submission(&msg, cid).await.unwrap();
            assert_eq!(r.reply, archived, "task_archive reply ({loc})");
            assert_eq!(mem.get_task(tid2).await.unwrap().status, "archived");

            // task_delete (delete the archived one — only done/archived deletable)
            let msg = webapp_msg(&format!(r#"{{"task_delete:{tid2}": true}}"#));
            let r = orch.handle_panel_submission(&msg, cid).await.unwrap();
            assert_eq!(r.reply, deleted, "task_delete reply ({loc})");
            assert!(mem.get_task(tid2).await.is_none());
        }
    }

    #[tokio::test]
    async fn task_done_not_found() {
        let orch = setup().await;
        let msg = webapp_msg(r#"{"task_done:999999": true}"#);
        let result = orch.handle_panel_submission(&msg, 1).await.unwrap();
        // Default display_language is zh: "任务未找到或已完成。"
        assert!(result.reply.contains("未找到") || result.reply.contains("已完成"));
    }

    #[tokio::test]
    async fn task_archive_marks_task() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 200;
        let tid = seed_task(&mem, cid).await;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        let msg = webapp_msg(&format!(r#"{{"task_archive:{tid}": true}}"#));
        let result = orch.handle_panel_submission(&msg, cid).await.unwrap();
        assert_eq!(result.reply, "🗑 已归档");

        let task = mem.get_task(tid).await.unwrap();
        assert_eq!(task.status, "archived");
    }

    #[tokio::test]
    async fn task_restore_restores_task() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 300;
        let tid = seed_task(&mem, cid).await;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        // First mark done WITH completed_at set (so restore has something to clear).
        let updates = serde_json::json!({
            "status": "done",
            "completed_at": "2026-01-01T00:00:00Z",
        });
        mem.update_task(tid, cid, &updates.to_string())
            .await
            .unwrap();
        // Sanity: completed_at stuck
        let before = mem.get_task(tid).await.unwrap();
        assert_eq!(before.completed_at.as_deref(), Some("2026-01-01T00:00:00Z"));

        // Then restore via panel
        let msg = webapp_msg(&format!(r#"{{"task_restore:{tid}": true}}"#));
        let result = orch.handle_panel_submission(&msg, cid).await.unwrap();
        // Default display_language is zh: "↩ 已恢复"
        assert!(result.reply.contains("已恢复"));

        // Both status active AND completed_at cleared (2026-04-17 regression: the
        // panel handler was relying on `"completed_at": null` to clear the field
        // but maybe_set! started skipping nulls, leaving the restored task with a
        // stale completed_at.
        let task = mem.get_task(tid).await.unwrap();
        assert_eq!(task.status, "active");
        assert!(
            task.completed_at.is_none(),
            "task_restore must clear completed_at via clear_fields; was: {:?}",
            task.completed_at
        );
    }

    #[tokio::test]
    async fn task_delete_completed_task() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 400;
        let tid = seed_task(&mem, cid).await;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        // Mark as done first (only done/archived can be deleted)
        let updates = serde_json::json!({"status": "done", "completed_at": "2026-01-01T00:00:00Z"});
        mem.update_task(tid, cid, &updates.to_string())
            .await
            .unwrap();

        let msg = webapp_msg(&format!(r#"{{"task_delete:{tid}": true}}"#));
        let result = orch.handle_panel_submission(&msg, cid).await.unwrap();
        assert!(result.reply.contains("deleted") || result.reply.contains("删除"));

        // Verify it's gone
        assert!(mem.get_task(tid).await.is_none());
    }

    #[tokio::test]
    async fn task_delete_active_rejected() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 410;
        let tid = seed_task(&mem, cid).await;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        // Try to delete an active task — should be rejected
        let msg = webapp_msg(&format!(r#"{{"task_delete:{tid}": true}}"#));
        let result = orch.handle_panel_submission(&msg, cid).await.unwrap();
        assert!(result.reply.contains("only") || result.reply.contains("只能"));

        // Task should still exist
        assert!(mem.get_task(tid).await.is_some());
    }

    #[tokio::test]
    async fn task_clear_finished() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 500;
        let tid = seed_task(&mem, cid).await;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        // Mark as done
        let updates = serde_json::json!({"status": "done", "completed_at": "2026-01-01T00:00:00Z"});
        mem.update_task(tid, cid, &updates.to_string())
            .await
            .unwrap();

        // The target_id in this case is ignored — it clears ALL finished tasks for the chat
        let msg = webapp_msg(&format!(r#"{{"task_clear_finished:{cid}": true}}"#));
        let result = orch.handle_panel_submission(&msg, cid).await.unwrap();
        assert!(result.reply.contains("清空") || result.reply.contains("1"));
    }

    #[tokio::test]
    async fn task_skip_nonrecurring_marks_done() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 600;
        let tid = seed_task(&mem, cid).await;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        let msg = webapp_msg(&format!(r#"{{"task_skip:{tid}": true}}"#));
        let result = orch.handle_panel_submission(&msg, cid).await.unwrap();
        // Default display_language is zh: "✅ 已完成（非循环任务）"
        assert!(result.reply.contains("已完成") || result.reply.contains("非循环"));
    }

    #[tokio::test]
    async fn task_skip_not_found() {
        let orch = setup().await;
        let msg = webapp_msg(r#"{"task_skip:999999": true}"#);
        let result = orch.handle_panel_submission(&msg, 1).await.unwrap();
        // Default display_language is zh: "任务未找到。"
        assert!(result.reply.contains("未找到"));
    }

    // ── handle_panel_submission: knowledge entry actions ───────────────────

    #[tokio::test]
    async fn mark_done_knowledge_entry() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 700;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        let kid = mem
            .store_knowledge_entry(
                cid,
                "insight",
                "normal",
                "Test insight",
                "Test body",
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

        let msg = webapp_msg(&format!(r#"{{"mark_done:{kid}": true}}"#));
        let result = orch.handle_panel_submission(&msg, cid).await.unwrap();
        assert_eq!(result.reply, "✅ 已完成");
    }

    #[tokio::test]
    async fn dismiss_knowledge_entry() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 800;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        let kid = mem
            .store_knowledge_entry(
                cid,
                "insight",
                "normal",
                "Test insight",
                "body",
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

        let msg = webapp_msg(&format!(r#"{{"dismiss:{kid}": true}}"#));
        let result = orch.handle_panel_submission(&msg, cid).await.unwrap();
        assert_eq!(result.reply, "🗑 已归档");
    }

    // ── handle_panel_submission: retired panel-store actions ───────────────

    /// The Smart Panel store actions were removed with the SDUI pipeline.
    /// Negative control: with the old `pin_panel` / `lock_panel` /
    /// `submit_feedback` arms present these returned `Some("📌 已置顶")` etc.
    /// (and `submit_feedback` pushed a `manage_panels` instruction into L1);
    /// now the router must not recognise them at all and fall through.
    #[tokio::test]
    async fn retired_panel_store_actions_fall_through() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 900;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));
        let user_did = format!("did:aegis:telegram:{cid}");

        for action in [
            r#"{"pin_panel:1": true}"#,
            r#"{"unpin_panel:1": true}"#,
            r#"{"lock_panel:1": true}"#,
            r#"{"unlock_panel:1": true}"#,
            r#"{"submit_feedback:1": true, "panel_feedback:1": "add dark mode"}"#,
            r#"{"trigger_heartbeat": true}"#,
        ] {
            let result = orch.handle_panel_submission(&webapp_msg(action), cid).await;
            assert!(
                result.is_none(),
                "retired panel action must fall through, got {result:?} for {action}"
            );
        }
        // submit_feedback used to leave a manage_panels instruction in L1.
        let l1 = mem.get_l1(&user_did).await.unwrap();
        assert!(
            l1.is_empty(),
            "retired feedback action must not push anything into L1: {l1:?}"
        );

        // Sanity: the live task action on the same router still answers.
        let tid = seed_task(&mem, cid).await;
        let msg = webapp_msg(&format!(r#"{{"task_done:{tid}": true}}"#));
        assert!(orch.handle_panel_submission(&msg, cid).await.is_some());
    }

    #[test]
    fn format_interval_buckets() {
        assert_eq!(format_interval(0), "—");
        assert_eq!(format_interval(30), "1min");
        assert_eq!(format_interval(3 * 60), "3min");
        assert_eq!(format_interval(2 * 3600 + 5), "2h");
        assert_eq!(format_interval(3 * 86400 + 3600), "3d");
    }

    // ── handle_admin_panel_action: via handle_panel_submission ─────────────

    #[tokio::test]
    async fn admin_deactivate_skill_unknown_skill_reports_not_found() {
        let orch = setup().await;
        let msg = webapp_msg(r#"{"admin_deactivate_skill:test_skill": true}"#);
        let result = orch.handle_panel_submission(&msg, 1).await.unwrap();
        assert!(
            result.reply.contains("未找到") && result.reply.contains("test_skill"),
            "got: {}",
            result.reply
        );
    }

    #[tokio::test]
    async fn admin_reactivate_skill_unknown_skill_reports_failure() {
        let orch = setup().await;
        let msg = webapp_msg(r#"{"admin_reactivate_skill:test_skill": true}"#);
        let result = orch.handle_panel_submission(&msg, 1).await.unwrap();
        assert!(
            result.reply.contains("激活失败") && result.reply.contains("test_skill"),
            "got: {}",
            result.reply
        );
    }

    #[tokio::test]
    async fn admin_reset_breaker() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        let msg = webapp_msg(r#"{"admin_reset_breaker:test_skill": true}"#);
        let result = orch.handle_panel_submission(&msg, 1).await.unwrap();
        assert!(result.reply.contains("重置") && result.reply.contains("test_skill"));
    }

    #[tokio::test]
    async fn admin_reject_hitl_clears_entry() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        // Store a HITL entry first
        mem.store_skill_state("hitl", "test_key", r#"{"some":"data"}"#)
            .await
            .unwrap();

        let msg = webapp_msg(r#"{"admin_reject_hitl:test_key": true}"#);
        let result = orch.handle_panel_submission(&msg, 1).await.unwrap();
        assert!(result.reply.contains("拒绝") && result.reply.contains("test_key"));

        // Verify HITL entry was cleared (empty string)
        let data = mem.load_skill_state("hitl", "test_key").await.unwrap();
        assert!(data.is_none() || data.unwrap().is_empty());
    }

    #[tokio::test]
    async fn admin_cancel_intent_success() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 1100;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        // Store a task/intent
        let user_did = format!("did:aegis:telegram:{cid}");
        let tid = mem
            .store_task(
                cid,
                &user_did,
                "Intent Task",
                "ctx",
                "normal",
                &[],
                None,
                "user",
                Some(chrono::Utc::now().timestamp() + 3600),
                Some("time"),
                None,
                "autonomous",
                Some("do something"),
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
            .unwrap();

        let msg = webapp_msg(&format!(r#"{{"admin_cancel_intent:{tid}": true}}"#));
        let result = orch.handle_panel_submission(&msg, cid).await.unwrap();
        assert!(result.reply.contains("取消") && result.reply.contains(&format!("#{tid}")));
    }

    #[tokio::test]
    async fn admin_cancel_intent_invalid_id() {
        let orch = setup().await;
        let msg = webapp_msg(r#"{"admin_cancel_intent:not_a_number": true}"#);
        let result = orch.handle_panel_submission(&msg, 1).await.unwrap();
        assert!(result.reply.contains("无效"));
    }

    #[tokio::test]
    async fn admin_view_intent_not_found() {
        let orch = setup().await;
        let msg = webapp_msg(r#"{"admin_view_intent:99999": true}"#);
        let result = orch.handle_panel_submission(&msg, 1).await.unwrap();
        assert!(result.reply.contains("未找到"));
    }

    #[tokio::test]
    async fn admin_unknown_action_returns_none() {
        let orch = setup().await;
        // admin_ prefix matches the filter, but "admin_nonexistent" doesn't match handle_admin_panel_action
        let msg = webapp_msg(r#"{"admin_nonexistent:test": true}"#);
        assert!(orch.handle_panel_submission(&msg, 1).await.is_none());
    }

    // ── create_config_proposal ────────────────────────────────────────────

    #[tokio::test]
    async fn config_proposal_normal_flow() {
        let db = tempfile::NamedTempFile::new().expect("temp db");
        let mem = MemoryHandle::spawn(db.path()).unwrap();
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        let mut messages = Vec::new();
        let result = orch
            .create_config_proposal(
                "call_1",
                "daily_budget_usd",
                serde_json::json!(5.0),
                serde_json::json!(2.0),
                "did:test:user",
                Some(12345),
                &mut messages,
            )
            .await
            .unwrap();

        // Should return false (budget exhaustion signal to suspend ReAct loop)
        assert!(!result);
        // Should push tool result + system message (2 messages)
        assert_eq!(messages.len(), 2);
        // Tool result should contain "pending_human_approval"
        let tool_msg = &messages[0];
        if let async_openai::types::chat::ChatCompletionRequestMessage::Tool(t) = tool_msg {
            let text = match &t.content {
                async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(s) => {
                    s.clone()
                }
                _ => String::new(),
            };
            assert!(text.contains("pending_human_approval"));
            assert!(text.contains("daily_budget_usd"));
        } else {
            panic!("Expected Tool message");
        }

        let conn = crate::encrypted_sqlite::open(db.path()).expect("open action ledger db");
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        let entries = store
            .list_recent_by_status(crate::action_ledger::ActionStatus::PendingApproval, 1)
            .expect("query pending action ledger rows");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(
            entry.action_kind,
            crate::action_ledger::ActionKind::RuntimeSettingChange
        );
        assert_eq!(entry.tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(entry.chat_id, Some(12345));
        assert_eq!(
            entry
                .external_event_id
                .as_deref()
                .map(|id| id.starts_with("config-")),
            Some(true)
        );
    }

    /// Task #12 — Gate pre-approved path applies the setting synchronously,
    /// skips the approve/reject buttons, and leaves no pending HITL row.
    ///
    /// Simulates the ReAct loop replay that `execute_approved_tool` does:
    /// build a messages buffer, call `create_config_proposal_ex` with
    /// `gate_pre_approved=true`, assert (1) the setting landed in live
    /// state, (2) the HITL state row is cleared (no second prompt pending),
    /// (3) the tool_result carries `status: "ok"`, not
    /// `pending_human_approval`, and (4) the function returns `Ok(true)` so
    /// the ReAct loop continues instead of suspending.
    #[tokio::test]
    async fn config_proposal_gate_pre_approved_applies_synchronously() {
        let db = tempfile::NamedTempFile::new().expect("temp db");
        let mem = MemoryHandle::spawn(db.path()).unwrap();
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings.clone());

        let key = "embedding_primary_model";
        let proposed = serde_json::json!("qwen/qwen3-embedding-8b");
        let current = settings.get(key).unwrap_or(serde_json::Value::Null);

        let mut messages = Vec::new();
        let result = orch
            .create_config_proposal_ex(
                "call_gate_preapproved_1",
                key,
                proposed.clone(),
                current,
                "did:test:user",
                Some(12345),
                &mut messages,
                true, // ← Gate pre-approved: apply immediately, no second prompt
            )
            .await
            .unwrap();

        // ReAct loop should keep going (action completed, not suspended).
        assert!(
            result,
            "gate_pre_approved=true must return Ok(true) so ReAct continues"
        );

        // Exactly one tool_result was pushed, no pending_human_approval
        // system-message suspend marker.
        assert_eq!(
            messages.len(),
            1,
            "pre-approved path pushes ONLY a tool_result (no suspend system msg)"
        );
        let tool_text = if let async_openai::types::chat::ChatCompletionRequestMessage::Tool(t) =
            &messages[0]
        {
            match &t.content {
                async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(s) => {
                    s.clone()
                }
                _ => String::new(),
            }
        } else {
            panic!("Expected Tool message at position 0");
        };
        assert!(
            tool_text.contains("\"status\":\"ok\""),
            "tool_result must carry status:ok, not pending_human_approval — got {}",
            tool_text
        );
        assert!(
            tool_text.contains("\"gate_pre_approved\":true"),
            "tool_result must advertise gate_pre_approved for LLM/caller introspection — got {}",
            tool_text
        );
        assert!(
            !tool_text.contains("pending_human_approval"),
            "pre-approved path MUST NOT emit pending_human_approval (that would suspend ReAct) — got {}",
            tool_text
        );

        // Live setting landed.
        let live = settings.get(key).unwrap_or(serde_json::Value::Null);
        assert_eq!(
            live, proposed,
            "setting must be applied synchronously when gate_pre_approved=true"
        );

        // HITL row cleared — the approval_id is random per-call, but the
        // pre-approved path writes then immediately clears the state row.
        // Extract the approval_id from the tool_result JSON and verify
        // load returns None (or empty string — both count as cleared per
        // `clear_approval_state`'s contract).
        let tool_json: serde_json::Value = serde_json::from_str(&tool_text).unwrap();
        let approval_id = tool_json
            .get("approval_id")
            .and_then(|v| v.as_str())
            .expect("tool_result must carry approval_id for audit trail");
        let hitl_row = mem.load_skill_state("hitl", approval_id).await;
        let cleared =
            matches!(hitl_row, Ok(None)) || matches!(hitl_row, Ok(Some(ref s)) if s.is_empty());
        assert!(
            cleared,
            "HITL state row must be cleared after gate-pre-approved apply (approval_id={approval_id}) — got {:?}",
            hitl_row
        );

        let conn = crate::encrypted_sqlite::open(db.path()).expect("open action ledger db");
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        let entry = store
            .get_by_action_id(&crate::action_ledger::runtime_setting_action_id(
                approval_id,
            ))
            .expect("query runtime setting action")
            .expect("runtime setting action row");
        assert_eq!(entry.status, crate::action_ledger::ActionStatus::Executed);
        let expected_receipt = format!("runtime_setting:{key}:{approval_id}");
        assert_eq!(
            entry.external_receipt.as_deref(),
            Some(expected_receipt.as_str())
        );
        assert!(entry.executing_at.is_some());
        assert!(entry.executed_at.is_some());
    }

    /// Task #12 — ensure the non-pre-approved path (default) is unaffected:
    /// still emits pending_human_approval + suspend marker, still returns
    /// Ok(false) to pause the ReAct loop. Protects against accidental
    /// regression of the default two-layer UX.
    #[tokio::test]
    async fn config_proposal_default_path_still_two_layer() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem.clone(), settings.clone());

        let mut messages = Vec::new();
        let result = orch
            .create_config_proposal_ex(
                "call_default_1",
                "daily_budget_usd",
                serde_json::json!(5.0),
                serde_json::json!(2.0),
                "did:test:user",
                Some(12345),
                &mut messages,
                false, // ← default path: require a second approval
            )
            .await
            .unwrap();

        assert!(
            !result,
            "non-pre-approved path must suspend ReAct (Ok(false))"
        );
        assert_eq!(
            messages.len(),
            2,
            "default path pushes tool_result + suspend system message"
        );
        if let async_openai::types::chat::ChatCompletionRequestMessage::Tool(t) = &messages[0] {
            let text = match &t.content {
                async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(s) => {
                    s.clone()
                }
                _ => String::new(),
            };
            assert!(
                text.contains("pending_human_approval"),
                "default path must still carry pending_human_approval — got {}",
                text
            );
        } else {
            panic!("Expected Tool message at position 0");
        }
    }

    #[tokio::test]
    async fn config_proposal_duplicate_suppressed() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        let mut messages = Vec::new();
        // First proposal
        orch.create_config_proposal(
            "call_1",
            "daily_budget_usd",
            serde_json::json!(5.0),
            serde_json::json!(2.0),
            "did:test:user",
            Some(12345),
            &mut messages,
        )
        .await
        .unwrap();

        // Second proposal for the same key — should be suppressed
        let result = orch
            .create_config_proposal(
                "call_2",
                "daily_budget_usd",
                serde_json::json!(10.0),
                serde_json::json!(2.0),
                "did:test:user",
                Some(12345),
                &mut messages,
            )
            .await
            .unwrap();

        assert!(!result);
        // Should have 3 messages now (2 from first + 1 duplicate response)
        assert_eq!(messages.len(), 3);
        // The third message should mention "duplicate"
        if let async_openai::types::chat::ChatCompletionRequestMessage::Tool(t) = &messages[2] {
            let text = match &t.content {
                async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(s) => {
                    s.clone()
                }
                _ => String::new(),
            };
            assert!(text.contains("duplicate"));
        }
    }

    // ── Cross-chat isolation ──────────────────────────────────────────────

    #[tokio::test]
    async fn task_skip_wrong_chat_returns_not_found() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 1200;
        let tid = seed_task(&mem, cid).await;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        // Try to skip task from a different chat_id
        let msg = webapp_msg(&format!(r#"{{"task_skip:{tid}": true}}"#));
        let result = orch.handle_panel_submission(&msg, cid + 1).await.unwrap();
        // Default display_language is zh: "任务未找到。"
        assert!(result.reply.contains("未找到"));
    }

    // ── route_hitl_action tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn hitl_reject_clears_entry_and_returns_reply() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        // Store a HITL entry with realistic data
        let hitl_data = serde_json::json!({
            "approval_id": "config-test123",
            "key": "daily_budget_usd",
            "proposed_value": 10.0,
            "current_value": 2.0,
            "user_did": "did:aegis:telegram:999",
            "chat_id": 999,
            "created_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string();
        mem.store_skill_state("hitl", "config-test123", &hitl_data)
            .await
            .unwrap();

        let msg = webapp_msg(r#"{"admin_reject_hitl:config-test123": true}"#);
        let result = orch.handle_panel_submission(&msg, 999).await.unwrap();
        assert!(
            result.reply.contains("config-test123"),
            "reply must reference the key"
        );

        // Verify the entry was cleared
        let loaded = mem
            .load_skill_state("hitl", "config-test123")
            .await
            .unwrap();
        assert!(
            loaded.is_none() || loaded.unwrap().is_empty(),
            "HITL entry must be cleared after rejection"
        );
    }

    #[tokio::test]
    async fn hitl_approve_generic_key_clears_and_approves() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        // Store a HITL entry with a key that doesn't match config/email/menu patterns
        let hitl_data = serde_json::json!({
            "approval_id": "custom-action-xyz",
            "created_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string();
        mem.store_skill_state("hitl", "custom-action-xyz", &hitl_data)
            .await
            .unwrap();

        let msg = webapp_msg(r#"{"admin_approve_hitl:custom-action-xyz": true}"#);
        let result = orch.handle_panel_submission(&msg, 100).await.unwrap();
        // Generic approval returns a message containing the key
        assert!(
            result.reply.contains("custom-action-xyz"),
            "reply must reference the key"
        );

        // Verify the entry was cleared
        let loaded = mem
            .load_skill_state("hitl", "custom-action-xyz")
            .await
            .unwrap();
        assert!(
            loaded.is_none() || loaded.unwrap().is_empty(),
            "HITL entry must be cleared after generic approval"
        );
    }

    // ── Expired HITL detection ───────────────────────────────────────────────

    #[tokio::test]
    async fn hitl_approve_expired_returns_expiry_message() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        // Store a HITL entry created >24h ago
        let old_time = chrono::Utc::now() - chrono::Duration::hours(25);
        let hitl_data = serde_json::json!({
            "approval_id": "config-expired",
            "key": "llm_model",
            "proposed_value": "test-model",
            "current_value": "default-model",
            "user_did": "did:aegis:telegram:200",
            "chat_id": 200,
            "created_at": old_time.to_rfc3339(),
        })
        .to_string();
        mem.store_skill_state("hitl", "config-expired", &hitl_data)
            .await
            .unwrap();

        let msg = webapp_msg(r#"{"admin_approve_hitl:config-expired": true}"#);
        let result = orch.handle_panel_submission(&msg, 200).await.unwrap();
        assert!(
            result.reply.contains("过期")
                || result.reply.contains("expired")
                || result.reply.contains("24h"),
            "expired HITL must indicate expiry, got: {}",
            result.reply
        );

        // Verify the expired entry was cleared
        let loaded = mem
            .load_skill_state("hitl", "config-expired")
            .await
            .unwrap();
        assert!(
            loaded.is_none() || loaded.unwrap().is_empty(),
            "expired HITL entry must be cleared"
        );
    }

    // ── Panel submission with Unicode action keys ────────────────────────────

    #[tokio::test]
    async fn submission_with_multiple_action_keys_uses_first_match() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 1300;
        let tid = seed_task(&mem, cid).await;
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        // Submission with a matching task action and an unrelated key
        let msg = webapp_msg(&format!(
            r#"{{"unrelated_field":"ignored","task_done:{tid}":true}}"#
        ));
        let result = orch.handle_panel_submission(&msg, cid).await.unwrap();
        // Default display_language is zh: "✅ 已完成"
        assert_eq!(result.reply, "\u{2705} 已完成");

        // Verify task was marked done
        let task = mem.get_task(tid).await.unwrap();
        assert_eq!(task.status, "done");
    }

    // ── admin_view_intent with valid intent ──────────────────────────────────

    #[tokio::test]
    async fn admin_view_intent_shows_details() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let cid = 1400;
        let user_did = format!("did:aegis:telegram:{cid}");
        let tid = mem
            .store_task(
                cid,
                &user_did,
                "Reminder: Check server logs",
                "Review error rates and latency spikes from last 24 hours",
                "high",
                &["ops".to_string()],
                None,
                "user",
                Some(chrono::Utc::now().timestamp() + 7200),
                Some("time"),
                None,
                "llm_callback",
                Some("Check server health metrics and alert if error rate > 1%"),
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
            .unwrap();

        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        let msg = webapp_msg(&format!(r#"{{"admin_view_intent:{tid}": true}}"#));
        let result = orch.handle_panel_submission(&msg, cid).await.unwrap();
        assert!(
            result.reply.contains(&format!("#{tid}")),
            "must show intent ID"
        );
        assert!(result.reply.contains("time"), "must show trigger type");
        assert!(
            result.reply.contains("llm_callback"),
            "must show execution mode"
        );
    }

    // ── remove_monitoring admin action ───────────────────────────────────────

    #[tokio::test]
    async fn admin_remove_monitoring_invalid_id() {
        let orch = setup().await;
        // "remove_monitoring" is not in the action key whitelist (not admin_ prefixed),
        // so handle_panel_submission returns None before reaching the admin handler.
        let msg = webapp_msg(r#"{"remove_monitoring:not_a_number": true}"#);
        let result = orch.handle_panel_submission(&msg, 1).await;
        assert!(
            result.is_none(),
            "remove_monitoring is not a recognized panel action prefix, must return None"
        );
    }

    #[tokio::test]
    async fn admin_remove_monitoring_nonexistent_id() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let orch = test_orchestrator(mem.clone(), Arc::new(RuntimeSettings::new()));

        // "remove_monitoring" is not in the action key whitelist (not admin_ prefixed),
        // so handle_panel_submission returns None before reaching the admin handler.
        let msg = webapp_msg(r#"{"remove_monitoring:99999": true}"#);
        let result = orch.handle_panel_submission(&msg, 1).await;
        assert!(
            result.is_none(),
            "remove_monitoring is not a recognized panel action prefix, must return None"
        );
    }
}
