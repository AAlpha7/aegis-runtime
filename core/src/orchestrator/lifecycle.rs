use super::*;

impl Orchestrator {
    /// Create a new orchestrator.
    ///
    /// `openrouter_api_key`: The API key for OpenRouter.
    /// `model`: e.g. "anthropic/claude-3.5-sonnet" or "google/gemini-2.0-flash-exp".
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        api_key: &str,
        api_base: &str,
        model: &str,
        sandbox: Arc<SandboxEngine>,
        identity: Arc<AgentIdentity>,
        memory: MemoryHandle,
        embedder: EmbeddingClient,
        settings: Arc<RuntimeSettings>,
    ) -> Self {
        let llm_client = llm_client::create_llm_client(api_key, api_base);

        let http_client = reqwest::Client::builder()
            .pool_max_idle_per_host(4)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .build()
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "reqwest client builder failed (TLS backend unavailable?), using default client");
                reqwest::Client::new()
            });

        Self {
            llm_client,
            http_client,
            model: model.to_string(),
            api_base: api_base.to_string(),
            api_key: api_key.to_string(),
            sandbox,
            identity,
            memory,
            embedder,
            system_prompt: default_system_prompt(),
            settings,
            services: ServiceEngines {
                intent_event_tx: None,
                chronos: parking_lot::Mutex::new(None),
                mcp_client: None,
                tts: {
                    let t = crate::multimodal::TtsSynthesizer::new();
                    if t.is_configured() {
                        Some(Arc::new(t))
                    } else {
                        None
                    }
                },
                telegram_bot: None,
                hercules: parking_lot::Mutex::new(None),
                external_executors: parking_lot::Mutex::new(None),
                secure_cu: parking_lot::Mutex::new(None),
            },
            credentials: CredentialState {
                credential_db_path: None,
                credential_aes_key: None,
                public_url: None,
            },
            caches: CacheState {
                embedding_caches: EmbeddingCaches::new(),
                tool_effectiveness_cache: Arc::new(parking_lot::Mutex::new(String::new())),
                prompt_overrides: Arc::new(parking_lot::Mutex::new(HashMap::new())),
                heartbeat_config: Arc::new(parking_lot::RwLock::new(
                    crate::heartbeat_config::HeartbeatConfig::default(),
                )),
                deferred_tool_schemas: parking_lot::Mutex::new(HashMap::new()),
                alert_policies_loaded: std::sync::atomic::AtomicBool::new(false),
            },
            progress_tx: None,
            agent_skills: Vec::new(),
            activated_skills: std::sync::Mutex::new(std::collections::HashSet::new()),
            tool_gate: crate::tool_gate::ToolGate::new(),
            tool_exec: ToolExecutionState::new(),
            llm_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            reembed_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            notification_state: Arc::new(NotificationState::new()),
            menu_lock: Arc::new(tokio::sync::Mutex::new(())),
            google_self_addr: tokio::sync::OnceCell::new(),
            last_health_probe_at: parking_lot::Mutex::new(None),
            health_probe_state: parking_lot::Mutex::new(std::collections::HashMap::new()),
            subcall_registry: Arc::new(crate::subcall::SubcallRegistry::new()),
            llm_degraded: Arc::new(parking_lot::RwLock::new(None)),
            embedder_degraded: Arc::new(parking_lot::RwLock::new(None)),
        }
    }

    /// Accessor for the subcall registry. Used by the hourly safety-net sweep
    /// task in `main.rs` and by `run_subcall` callers (Phase 2).
    pub fn subcall_registry(&self) -> &Arc<crate::subcall::SubcallRegistry> {
        &self.subcall_registry
    }

    /// Snapshot of the current LLM-degraded signal (`None` when healthy).
    /// Read by the webapp status projection so the owner can see when all
    /// providers are exhausted and switch provider from the settings page.
    pub fn llm_degraded_snapshot(&self) -> Option<LlmDegraded> {
        self.llm_degraded.read().clone()
    }

    /// Snapshot of the current embedder-degraded signal (`None` when healthy).
    /// Read by the webapp status projection so the owner can see when the
    /// primary embedder is down and semantic dedup is paused (duplicate
    /// duplicate memory writes may leak until it recovers).
    pub fn embedder_degraded_snapshot(&self) -> Option<EmbedderDegraded> {
        self.embedder_degraded.read().clone()
    }

    /// Set the embedder-degraded snapshot on the healthy→degraded transition.
    /// Idempotent: preserves the original `since` if a snapshot already exists
    /// (so the episode keeps its start time even if called again). The runtime
    /// health probe calls this only on the `DegradedEdge::Entered` edge.
    pub fn mark_embedder_degraded(&self, primary_model: String, backup_model: Option<String>) {
        let mut guard = self.embedder_degraded.write();
        match guard.as_mut() {
            // Repeat mark within the same episode (e.g. BackupServing →
            // Unreachable after the backup also fails): PRESERVE `since` but
            // UPDATE the models, so the snapshot never keeps reporting a backup
            // as serving after it has gone down (Codex).
            Some(existing) => {
                existing.primary_model = primary_model;
                existing.backup_model = backup_model;
            }
            None => {
                *guard = Some(EmbedderDegraded {
                    since: chrono::Utc::now().to_rfc3339(),
                    primary_model,
                    backup_model,
                });
            }
        }
    }

    /// Clear the embedder-degraded snapshot on recovery. Idempotent (a no-op
    /// when already healthy).
    pub fn clear_embedder_degraded(&self) {
        *self.embedder_degraded.write() = None;
    }

    /// Get a clone of the LLM busy flag for use by the True Heartbeat timer.
    pub fn llm_busy_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.llm_busy.clone()
    }

    /// Clone of the corpus-reembed single-flight guard. Shared with the
    /// startup `reembed_stale_facts` path so the two sibling re-embedders
    /// cannot race within the ~10s startup window (CTO round-1 WARN #4).
    pub fn reembed_in_progress_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.reembed_in_progress.clone()
    }

    /// Set the MCP client manager for external server connections.
    pub fn set_mcp_client(&mut self, mcp: Arc<crate::mcp_client::McpClientManager>) {
        self.services.mcp_client = Some(mcp);
    }

    /// Get the memory handle reference.
    pub fn memory(&self) -> &MemoryHandle {
        &self.memory
    }

    // ── Stored approval helpers (granular approval framework) ────────────

    /// Load stored approvals for a specific tool.
    pub(crate) async fn load_tool_approvals(
        &self,
        tool_name: &str,
    ) -> Vec<crate::tool_gate::StoredApproval> {
        let key = format!("tool_approvals:{}", tool_name);
        match self.memory.load_skill_state("hitl", &key).await {
            Ok(Some(data)) if !data.is_empty() => serde_json::from_str(&data).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Store a new tool approval record (deduplicates by scope).
    pub(crate) async fn store_tool_approval(&self, record: &crate::tool_gate::StoredApproval) {
        let key = format!("tool_approvals:{}", record.tool_name);
        let mut approvals = self.load_tool_approvals(&record.tool_name).await;

        // Dedup: don't store duplicate scopes
        approvals.retain(|a| a.scope != record.scope);
        approvals.push(record.clone());

        if let Ok(json) = serde_json::to_string(&approvals)
            && let Err(e) = self.memory.store_skill_state("hitl", &key, &json).await
        {
            tracing::error!(%e, key, "[ToolGate] Failed to store approval");
        }
    }

    /// Clear all intent-scoped approvals for a given group_key.
    pub async fn clear_intent_approvals(&self, group_key: &str) {
        // We'd need to scan all tool approvals — use a known key pattern
        let key = format!("tool_approvals_by_intent:{}", group_key);
        if let Ok(Some(tools_json)) = self.memory.load_skill_state("hitl", &key).await {
            let tool_names: Vec<String> = serde_json::from_str(&tools_json).unwrap_or_default();
            for tool_name in tool_names {
                let mut approvals = self.load_tool_approvals(&tool_name).await;
                let before = approvals.len();
                approvals.retain(|a| !matches!(&a.scope, crate::tool_gate::ApprovalScope::Intent { group_key: gk } if gk == group_key));
                if approvals.len() < before {
                    let store_key = format!("tool_approvals:{}", tool_name);
                    if let Ok(json) = serde_json::to_string(&approvals)
                        && let Err(e) = self
                            .memory
                            .store_skill_state("hitl", &store_key, &json)
                            .await
                    {
                        tracing::warn!(error = %e, key = %store_key, "[HITL] Failed to persist updated approvals after intent cleanup");
                    }
                }
            }
            if let Err(e) = self.memory.store_skill_state("hitl", &key, "").await {
                tracing::warn!(error = %e, key = %key, "[HITL] Failed to clear intent approval index");
            }
        }
    }

    /// Maximum age for pending HITL approval entries (24 hours).
    pub(crate) const HITL_EXPIRY_SECS: i64 = 24 * SECS_PER_HOUR;

    /// Expire stale HITL approval requests older than 24 hours.
    /// Returns the number of entries expired.
    pub async fn expire_stale_hitl_entries(&self) -> usize {
        let entries = match self.memory.list_hitl_pending().await {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(error = %err, "[HITL] Failed to list pending entries for expiry check");
                return 0;
            }
        };

        let now = chrono::Utc::now();
        let mut expired_count = 0;

        for (key, value) in &entries {
            // Skip tool_approvals entries (these are granted approvals, not pending requests)
            if key.starts_with("tool_approvals:") || key.starts_with("tool_approvals_by_intent:") {
                continue;
            }

            // Parse the JSON value to extract created_at
            let parsed: serde_json::Value = match serde_json::from_str(value) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let created_at_str = match parsed.get("created_at").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => continue, // No created_at, skip (legacy entry)
            };

            let created_at = match chrono::DateTime::parse_from_rfc3339(created_at_str) {
                Ok(dt) => dt.with_timezone(&chrono::Utc),
                Err(_) => continue,
            };

            let age_secs = (now - created_at).num_seconds();
            if age_secs > Self::HITL_EXPIRY_SECS {
                tracing::info!(
                    key,
                    age_hours = age_secs / SECS_PER_HOUR,
                    "[HITL] Expiring stale approval request (>24h)"
                );
                // Clear the entry (same as reject — sets value to empty string)
                if let Err(e) = self.memory.store_skill_state("hitl", key, "").await {
                    tracing::warn!(error = %e, key = %key, "[HITL] Failed to clear expired approval request");
                }
                expired_count += 1;
            }
        }

        if expired_count > 0 {
            tracing::info!(expired_count, "[HITL] Expired stale approval requests");
        }
        expired_count
    }

    /// Wire the Hercules delegation client (`delegate_task`).
    pub fn set_hercules_client(&self, client: Arc<crate::hercules_client::HerculesClient>) {
        *self.services.hercules.lock() = Some(client);
    }

    /// Override the default system prompt.
    pub fn set_system_prompt(&mut self, prompt: &str) {
        self.system_prompt = prompt.to_string();
    }

    /// Set the deferred-intent event sender (shared with the intent engine).
    pub fn set_intent_event_tx(
        &mut self,
        tx: tokio::sync::mpsc::Sender<crate::intent::IntentEvent>,
    ) {
        self.services.intent_event_tx = Some(tx);
    }

    /// Set a progress channel for real-time execution feedback.
    pub fn set_progress_tx(&mut self, tx: ProgressSender) {
        self.progress_tx = Some(tx);
    }

    /// Mid-flight owner-visible phase (WebApp SSE `action_phase`).
    pub(crate) fn emit_action_phase(
        &self,
        chat_id: Option<i64>,
        phase: &str,
        tool_name: Option<&str>,
        detail: Option<&str>,
    ) {
        let (Some(tx), Some(cid)) = (&self.progress_tx, chat_id) else {
            return;
        };
        let _ = tx.try_send(crate::progress_router::ProgressEvent::new(
            cid,
            crate::progress_router::ProgressPayload::ActionPhase {
                phase: phase.to_string(),
                tool_name: tool_name.map(str::to_string),
                detail: detail.map(str::to_string),
            },
        ));
    }

    /// Push one piece of owner-visible assistant text. No-op unless this task
    /// is inside the owner-reply stream scope and a Web turn id is set.
    /// Fragments are not written to L1.
    pub(crate) fn emit_assistant_delta(&self, text: &str) {
        if text.is_empty()
            || !crate::progress_router::owner_reply_stream_enabled()
            || crate::progress_router::current_turn_id().is_none()
        {
            return;
        }
        self.emit_webapp_only_progress(crate::progress_router::ProgressPayload::AssistantDelta {
            text: text.to_string(),
        });
    }

    /// Clear provisional assistant text on the Web console. Used when the
    /// streamed words will not be the stored reply, including after the
    /// owner-reply scope has already ended.
    pub(crate) fn emit_assistant_draft_reset(&self) {
        if crate::progress_router::current_turn_id().is_none() {
            return;
        }
        self.emit_webapp_only_progress(
            crate::progress_router::ProgressPayload::AssistantDraftReset,
        );
    }

    fn emit_webapp_only_progress(&self, payload: crate::progress_router::ProgressPayload) {
        let Some(tx) = &self.progress_tx else {
            return;
        };
        let Some(turn_id) = crate::progress_router::current_turn_id() else {
            return;
        };
        let _ = tx.try_send(crate::progress_router::ProgressEvent::with_turn(
            0,
            Some(turn_id),
            payload,
        ));
    }

    /// Get a clone of the progress sender for use by external components
    /// (e.g., daily summary in True Heartbeat timer).
    pub fn progress_sender(&self) -> Option<ProgressSender> {
        self.progress_tx.clone()
    }

    pub(super) fn assistant_event_priority(
        event: &crate::assistant_events::AssistantEvent,
    ) -> NotifyPriority {
        match event.decision {
            crate::assistant_events::AssistantEventDecision::NotifyNow
                if event.urgency_hint == crate::assistant_events::EventUrgencyHint::High =>
            {
                NotifyPriority::Urgent
            }
            crate::assistant_events::AssistantEventDecision::NotifyNow
            | crate::assistant_events::AssistantEventDecision::CallbackNow => {
                NotifyPriority::Important
            }
            crate::assistant_events::AssistantEventDecision::DigestLater
            | crate::assistant_events::AssistantEventDecision::Taskify
            | crate::assistant_events::AssistantEventDecision::MemoryOnly
            | crate::assistant_events::AssistantEventDecision::Ignore => NotifyPriority::Routine,
            // Unknown decision: treat as Important (fail-open — surface to owner).
            crate::assistant_events::AssistantEventDecision::Unknown => NotifyPriority::Important,
        }
    }

    pub(super) fn notify_priority_to_str(priority: NotifyPriority) -> &'static str {
        match priority {
            NotifyPriority::Urgent => "urgent",
            NotifyPriority::Important => "important",
            NotifyPriority::Routine => "routine",
        }
    }

    pub(super) fn highest_notification_priority(
        notifications: &[NotifyUserRequest],
        fallback: NotifyPriority,
    ) -> NotifyPriority {
        notifications
            .iter()
            .fold(fallback, |current, notif| match (current, notif.priority) {
                (NotifyPriority::Urgent, _) | (_, NotifyPriority::Urgent) => NotifyPriority::Urgent,
                (NotifyPriority::Important, _) | (_, NotifyPriority::Important) => {
                    NotifyPriority::Important
                }
                _ => NotifyPriority::Routine,
            })
    }

    pub(super) fn proposal_title_from_text(text: &str) -> Option<String> {
        let normalized = text.trim().trim_end_matches(['.', '!', '?']).trim();
        (!normalized.is_empty()).then(|| normalized.to_string())
    }

    pub(super) fn build_deferred_intent_proposed_task(
        intent_context: &serde_json::Value,
        internal_summary: &str,
        user_message: Option<&str>,
    ) -> Option<crate::assistant_work_items::AssistantProposedTask> {
        let title = intent_context
            .get("context_text")
            .and_then(|v| v.as_str())
            .and_then(Self::proposal_title_from_text)?;
        let notes = user_message
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .or_else(|| {
                let summary = internal_summary.trim();
                (!summary.is_empty()).then(|| summary.to_string())
            });
        Some(crate::assistant_work_items::AssistantProposedTask {
            title,
            notes,
            anchor_kind: intent_context
                .get("anchor_kind")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
            anchor_ref: intent_context
                .get("anchor_ref")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
        })
    }

    pub(super) fn build_deferred_intent_proposed_reminder(
        intent_context: &serde_json::Value,
        internal_summary: &str,
        user_message: Option<&str>,
    ) -> Option<crate::assistant_work_items::AssistantProposedReminder> {
        let title = intent_context
            .get("context_text")
            .and_then(|v| v.as_str())
            .and_then(Self::proposal_title_from_text)?;
        let trigger_at = intent_context
            .get("trigger_at_unix")
            .and_then(|v| v.as_i64())
            .and_then(|ts| chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0))
            .map(|dt| dt.to_rfc3339());
        if trigger_at.is_none()
            && intent_context
                .get("interval_seconds")
                .and_then(|v| v.as_i64())
                .is_none()
        {
            return None;
        }
        let notes = user_message
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .or_else(|| {
                let summary = internal_summary.trim();
                (!summary.is_empty()).then(|| summary.to_string())
            });
        Some(crate::assistant_work_items::AssistantProposedReminder {
            title,
            notes,
            trigger_at,
            anchor_kind: intent_context
                .get("anchor_kind")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
            anchor_ref: intent_context
                .get("anchor_ref")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
        })
    }

    pub(super) fn build_assistant_event_proposed_task(
        event: &crate::assistant_events::StoredAssistantEvent,
        internal_summary: &str,
        user_message: Option<&str>,
    ) -> Option<crate::assistant_work_items::AssistantProposedTask> {
        let raw_title = event.event.title.trim();
        if raw_title.is_empty() {
            return None;
        }
        let title = match event.event.actionability_hint {
            crate::assistant_events::EventActionabilityHint::Review => {
                format!("Review: {raw_title}")
            }
            crate::assistant_events::EventActionabilityHint::Act => {
                format!("Act on: {raw_title}")
            }
            _ => raw_title.to_string(),
        };
        let notes = user_message
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .or_else(|| {
                let summary = internal_summary.trim();
                (!summary.is_empty()).then(|| summary.to_string())
            });
        Some(crate::assistant_work_items::AssistantProposedTask {
            title,
            notes,
            anchor_kind: event
                .event
                .thread_key
                .as_ref()
                .map(|_| "email_thread".to_string()),
            anchor_ref: event.event.thread_key.clone(),
        })
    }

    pub(super) fn build_work_item_proposed_task(
        work_item: &crate::assistant_work_items::AssistantWorkItem,
        internal_summary: &str,
        user_message: Option<&str>,
    ) -> Option<crate::assistant_work_items::AssistantProposedTask> {
        let title = Self::proposal_title_from_text(&work_item.goal)?;
        let notes = user_message
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .or_else(|| {
                let summary = internal_summary.trim();
                (!summary.is_empty()).then(|| summary.to_string())
            });
        Some(crate::assistant_work_items::AssistantProposedTask {
            title,
            notes,
            anchor_kind: work_item.anchor_kind.map(|v| v.as_str().to_string()),
            anchor_ref: work_item.anchor_ref.clone(),
        })
    }

    /// Formats an RFC3339 trigger timestamp in the owner's timezone for
    /// delivery text ("2026-06-28 22:27 PDT"). Falls back to the raw input
    /// when the timestamp or timezone does not parse (fail-safe: never drop
    /// the information, only skip the prettification).
    pub(super) fn format_delivery_trigger_time(raw: &str, tz_name: &str) -> String {
        let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(raw) else {
            return raw.to_string();
        };
        let Ok(tz) = tz_name.parse::<chrono_tz::Tz>() else {
            return raw.to_string();
        };
        parsed
            .with_timezone(&tz)
            .format("%Y-%m-%d %H:%M %Z")
            .to_string()
    }

    pub(super) fn render_assistant_turn_delivery_message(
        outcome: &crate::assistant_work_items::AssistantTurnOutcome,
        tz_name: &str,
    ) -> Option<String> {
        // Telegram-bound text path: this string flows through
        // `markdown_to_telegram_html` which forwards `Event::Html` /
        // `Event::InlineHtml` raw (see `core/src/telegram_render.rs:365-368`).
        // Any user-controlled fields composed in here (proposed task/reminder
        // titles + notes, derived from raw email subject / event summary in
        // `build_assistant_event_proposed_task`) MUST be passed through
        // `sanitize_for_telegram_markup` to neutralize `<...>` tags before
        // they reach the renderer. Mirrors the C4 fix in
        // `assistant_events.rs::render_*`.
        //
        // Labels are zh (owner-facing chrome is zh throughout the backend, cf.
        // "[待确认]"), and the reminder trigger renders in the owner's local
        // timezone instead of the raw RFC3339 UTC string (IMG_3939: the raw
        // "Suggested reminder … When: 2026-06-28T05:27:19+00:00" internal
        // format leaked into the chat timeline).
        use crate::assistant_events::sanitize_for_telegram_markup;
        let mut parts = Vec::new();
        let mut appended_proposal_notes: Vec<&str> = Vec::new();
        let user_message = outcome
            .user_message
            .as_deref()
            .map(str::trim)
            .unwrap_or_default();
        if let Some(message) = outcome.user_message.as_deref().map(str::trim)
            && !message.is_empty()
        {
            parts.push(message.to_string());
        }
        if let Some(task) = &outcome.proposed_task {
            let safe_title = sanitize_for_telegram_markup(&task.title);
            let mut section = format!("建议任务：{safe_title}");
            if let Some(notes) = task
                .notes
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .filter(|v| !Self::delivery_fragment_duplicates(v, &[user_message]))
            {
                section.push('\n');
                section.push_str(&sanitize_for_telegram_markup(notes));
                appended_proposal_notes.push(notes);
            }
            parts.push(section);
        }
        if let Some(reminder) = &outcome.proposed_reminder {
            let safe_title = sanitize_for_telegram_markup(&reminder.title);
            let mut section = format!("建议提醒：{safe_title}");
            if let Some(trigger_at) = reminder
                .trigger_at
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                let local_trigger = Self::format_delivery_trigger_time(trigger_at, tz_name);
                let safe_trigger = sanitize_for_telegram_markup(&local_trigger);
                section.push_str(&format!("\n时间：{safe_trigger}"));
            }
            if let Some(notes) = reminder
                .notes
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .filter(|v| {
                    let mut previous = Vec::with_capacity(appended_proposal_notes.len() + 1);
                    previous.push(user_message);
                    previous.extend(appended_proposal_notes.iter().copied());
                    !Self::delivery_fragment_duplicates(v, &previous)
                })
            {
                section.push('\n');
                section.push_str(&sanitize_for_telegram_markup(notes));
            }
            parts.push(section);
        }
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }

    pub(super) fn delivery_fragment_duplicates(candidate: &str, previous: &[&str]) -> bool {
        let normalized_candidate = Self::normalize_delivery_fragment(candidate);
        if normalized_candidate.is_empty() {
            return true;
        }
        previous
            .iter()
            .map(|fragment| Self::normalize_delivery_fragment(fragment))
            .any(|fragment| fragment == normalized_candidate)
    }

    pub(super) fn normalize_delivery_fragment(value: &str) -> String {
        value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string()
    }

    pub(super) fn callback_result_to_work_item_outcome(
        event: &crate::assistant_events::StoredAssistantEvent,
        result: &OrchestratorResult,
    ) -> crate::assistant_work_items::AssistantTurnOutcome {
        let user_message = if result.notifications.is_empty() {
            None
        } else {
            Some(
                result
                    .notifications
                    .iter()
                    .map(|notif| notif.message.trim())
                    .filter(|msg| !msg.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            )
        };

        let fallback_priority = Self::assistant_event_priority(&event.event);
        let effective_priority =
            Self::highest_notification_priority(&result.notifications, fallback_priority);

        let internal_summary = if !result.reply.trim().is_empty() {
            result.reply.trim().to_string()
        } else if let Some(message) = user_message.as_deref() {
            message.to_string()
        } else {
            format!(
                "Reviewed external update and kept it silent: {}",
                event.event.summary
            )
        };
        let proposed_task = Self::build_assistant_event_proposed_task(
            event,
            &internal_summary,
            user_message.as_deref(),
        );

        crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: chrono::Utc::now().to_rfc3339(),
            internal_summary,
            user_message,
            delivery: match effective_priority {
                NotifyPriority::Routine => {
                    crate::assistant_work_items::AssistantTurnDelivery::DigestOnly
                }
                NotifyPriority::Important | NotifyPriority::Urgent => {
                    if result.notifications.is_empty() {
                        crate::assistant_work_items::AssistantTurnDelivery::Silent
                    } else {
                        crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage
                    }
                }
            },
            notify_priority: Some(Self::notify_priority_to_str(effective_priority).to_string()),
            proposed_task,
            proposed_reminder: None,
            follow_up_anchor_kind: event
                .event
                .thread_key
                .as_ref()
                .map(|_| "email_thread".to_string()),
            follow_up_anchor_ref: event
                .event
                .thread_key
                .clone()
                .or_else(|| Some(format!("assistant_event:{}", event.id))),
            needs_user_reply: matches!(
                event.event.actionability_hint,
                crate::assistant_events::EventActionabilityHint::Review
                    | crate::assistant_events::EventActionabilityHint::Act
            ),
            tools_called: result.tools_called.clone(),
            rounds_used: result.rounds_used,
        }
    }

    pub(super) fn deferred_intent_result_to_work_item_outcome(
        intent_context: &serde_json::Value,
        result: &OrchestratorResult,
    ) -> crate::assistant_work_items::AssistantTurnOutcome {
        let user_message = if result.notifications.is_empty() {
            None
        } else {
            Some(
                result
                    .notifications
                    .iter()
                    .map(|notif| notif.message.trim())
                    .filter(|msg| !msg.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            )
        };

        let effective_priority =
            Self::highest_notification_priority(&result.notifications, NotifyPriority::Routine);

        let fallback_summary = intent_context
            .get("context_text")
            .and_then(|v| v.as_str())
            .map(|text| format!("Completed scheduled follow-up silently: {text}"))
            .unwrap_or_else(|| "Completed scheduled follow-up silently.".to_string());
        let internal_summary = if !result.reply.trim().is_empty() {
            result.reply.trim().to_string()
        } else if let Some(message) = user_message.as_deref() {
            message.to_string()
        } else {
            fallback_summary
        };
        // A RECURRING intent re-proposing ITSELF as a suggested task/reminder on
        // every delivery is pure noise (IMG_3939: "建议任务/建议提醒: 总结本日
        // 学习…" appended to each daily run): the scheduler already owns the
        // next recurrence, and these fields are consumed only by the rendered
        // delivery text — suppressing them for recurring intents loses nothing.
        // One-shot intents keep the suggestion (a fired one-shot follow-up
        // proposing its continuation is genuinely useful).
        let is_recurring = intent_context
            .get("interval_seconds")
            .and_then(|v| v.as_i64())
            .is_some();
        let proposed_task = (!is_recurring)
            .then(|| {
                Self::build_deferred_intent_proposed_task(
                    intent_context,
                    &internal_summary,
                    user_message.as_deref(),
                )
            })
            .flatten();
        let proposed_reminder = (!is_recurring)
            .then(|| {
                Self::build_deferred_intent_proposed_reminder(
                    intent_context,
                    &internal_summary,
                    user_message.as_deref(),
                )
            })
            .flatten();

        crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: chrono::Utc::now().to_rfc3339(),
            internal_summary,
            user_message,
            delivery: if result.notifications.is_empty() {
                crate::assistant_work_items::AssistantTurnDelivery::Silent
            } else {
                crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage
            },
            notify_priority: Some(Self::notify_priority_to_str(effective_priority).to_string()),
            proposed_task,
            proposed_reminder,
            follow_up_anchor_kind: intent_context
                .get("anchor_kind")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
            follow_up_anchor_ref: intent_context
                .get("anchor_ref")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
            needs_user_reply: true,
            tools_called: result.tools_called.clone(),
            rounds_used: result.rounds_used,
        }
    }

    pub(super) fn heartbeat_follow_up_result_to_work_item_outcome(
        work_item: &crate::assistant_work_items::AssistantWorkItem,
        result: &OrchestratorResult,
    ) -> crate::assistant_work_items::AssistantTurnOutcome {
        let user_message = if result.notifications.is_empty() {
            None
        } else {
            Some(
                result
                    .notifications
                    .iter()
                    .map(|notif| notif.message.trim())
                    .filter(|msg| !msg.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            )
        };

        let effective_priority =
            Self::highest_notification_priority(&result.notifications, NotifyPriority::Routine);

        let internal_summary = if !result.reply.trim().is_empty() {
            result.reply.trim().to_string()
        } else if let Some(message) = user_message.as_deref() {
            message.to_string()
        } else {
            format!("Heartbeat follow-up stayed silent: {}", work_item.goal)
        };
        let proposed_task = Self::build_work_item_proposed_task(
            work_item,
            &internal_summary,
            user_message.as_deref(),
        );

        crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: chrono::Utc::now().to_rfc3339(),
            internal_summary,
            user_message: if work_item.delivery_policy
                == crate::assistant_work_items::AssistantWorkDeliveryPolicy::Silent
            {
                None
            } else {
                user_message
            },
            delivery: if work_item.delivery_policy
                == crate::assistant_work_items::AssistantWorkDeliveryPolicy::Silent
                || result.notifications.is_empty()
            {
                crate::assistant_work_items::AssistantTurnDelivery::Silent
            } else {
                crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage
            },
            notify_priority: Some(Self::notify_priority_to_str(effective_priority).to_string()),
            proposed_task,
            proposed_reminder: None,
            follow_up_anchor_kind: work_item.anchor_kind.map(|v| v.as_str().to_string()),
            follow_up_anchor_ref: work_item.anchor_ref.clone(),
            needs_user_reply: true,
            tools_called: result.tools_called.clone(),
            rounds_used: result.rounds_used,
        }
    }

    pub(super) fn background_review_tool_allowlist() -> BTreeSet<String> {
        [
            "notify_user",
            "get_current_time",
            "recall_memory",
            "knowledge_base",
            "manage_tasks",
            "calendar_reader",
            "web_search",
            "search_tools",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    pub(super) fn background_action_tool_allowlist() -> BTreeSet<String> {
        let mut allowed = Self::background_review_tool_allowlist();
        for tool in ["calendar_create_event", "calendar_update_event"] {
            allowed.insert(tool.to_string());
        }
        allowed
    }

    pub(super) fn heartbeat_synthesis_tool_allowlist() -> BTreeSet<String> {
        [
            "notify_user",
            "get_current_time",
            "recall_memory",
            "knowledge_base",
            "manage_tasks",
            "calendar_reader",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    pub(super) fn heartbeat_digest_result_to_work_item_outcome(
        summary: impl Into<String>,
        message: Option<String>,
    ) -> crate::assistant_work_items::AssistantTurnOutcome {
        crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: chrono::Utc::now().to_rfc3339(),
            internal_summary: summary.into(),
            user_message: message,
            delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
            notify_priority: Some("important".to_string()),
            proposed_task: None,
            proposed_reminder: None,
            follow_up_anchor_kind: None,
            follow_up_anchor_ref: None,
            needs_user_reply: false,
            tools_called: Vec::new(),
            rounds_used: 0,
        }
    }

    pub(super) async fn commit_digest_cursor_for_work_item(
        &self,
        user_did: &str,
        work_item: &crate::assistant_work_items::AssistantWorkItem,
    ) -> Result<()> {
        let consumer = work_item
            .context
            .get("assistant_event_consumer")
            .and_then(|v| v.as_str())
            .unwrap_or("mail_notifier_digest");
        let Some(last_event_id) = work_item
            .context
            .get("last_event_id")
            .and_then(|v| v.as_i64())
        else {
            anyhow::bail!("digest work item missing last_event_id");
        };
        self.memory
            .commit_assistant_event_cursor(user_did, consumer, last_event_id)
            .await
    }

    pub(super) async fn run_callback_work_item_with_budget(
        &self,
        work_item: &crate::assistant_work_items::StoredAssistantWorkItem,
        chat_id: i64,
        prompt: &str,
        tool_allowlist: &BTreeSet<String>,
    ) -> Result<OrchestratorResult> {
        let budget = ReactLoopBudgetOverride {
            max_rounds: work_item.item.budget_profile.max_rounds as usize,
            max_tool_calls: work_item.item.budget_profile.max_tool_calls as usize,
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(work_item.item.budget_profile.timeout_secs as u64),
            self.process_callback_message_with_persistence_and_tool_allowlist(
                &work_item.item.user_did,
                prompt,
                Some(chat_id),
                UserMessagePersistence::Ephemeral,
                Some(tool_allowlist),
                Some(budget),
            ),
        )
        .await
        .with_context(|| {
            format!(
                "assistant work item timed out after {}s: {}",
                work_item.item.budget_profile.timeout_secs, work_item.item.dedup_key
            )
        })?
    }

    pub(super) async fn emit_proactive_notification(
        &self,
        chat_id: i64,
        label: &str,
        message: &str,
        priority: NotifyPriority,
    ) -> Result<ProactiveNotificationOutcome> {
        let Some(tx) = &self.progress_tx else {
            return Ok(ProactiveNotificationOutcome::Deferred);
        };

        if self.notification_state.in_quiet_hours(&self.settings)
            && priority != NotifyPriority::Urgent
        {
            tracing::info!(
                chat_id,
                label,
                "[AssistantEvent] suppressed during quiet hours"
            );
            return Ok(ProactiveNotificationOutcome::Deferred);
        }

        if self
            .notification_state
            .over_daily_cap(chat_id, &self.settings)
            && priority != NotifyPriority::Urgent
        {
            tracing::info!(chat_id, label, "[AssistantEvent] suppressed by daily cap");
            return Ok(ProactiveNotificationOutcome::Deferred);
        }

        let now = chrono::Utc::now();
        if priority != NotifyPriority::Urgent {
            let label_already_sent = self
                .notification_state
                .label_dedup
                .lock()
                .get(&(chat_id, label.to_string()))
                .map(|sent_at| (now - *sent_at).num_hours() < 6)
                .unwrap_or(false);
            if label_already_sent {
                tracing::debug!(chat_id, label, "[AssistantEvent] label-dedup suppressed");
                return Ok(ProactiveNotificationOutcome::AlreadySatisfied);
            }
        }

        // Content-dedup for Important: reserve the slot UNDER the lock (so a
        // concurrent call is suppressed and we never double-send), but remember
        // the key so we can roll the reservation back if the send below fails —
        // otherwise a failed send would leave a false dedup marker that wrongly
        // suppresses the retry, dropping the Telegram fallback copy entirely.
        //
        // Known limitation (deferred): an in-flight reservation is indistinguishable
        // from a completed delivery, so a concurrent same-key contender is treated as
        // satisfied even though this send might fail+roll back. This is a strict
        // improvement over the prior code (which never rolled back, so a failed send
        // suppressed ALL retries) and is low-stakes here: the owner-facing copy was
        // already delivered to the primary web surface via push_l1 before this point;
        // only the redundant Telegram fallback is affected. Full in-flight/marker
        // separation belongs to the larger notification-commit redesign.
        let reserved_dedup_key: Option<(i64, u64)> = if priority == NotifyPriority::Important {
            use std::hash::{Hash, Hasher};
            let normalized = normalize_for_dedup(message);
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            normalized.hash(&mut hasher);
            let content_hash = hasher.finish();
            let dedup_window = {
                let last_notif = self.notification_state.last_at.lock();
                let silence_hours = last_notif
                    .get(&chat_id)
                    .map(|t| (now - *t).num_hours())
                    .unwrap_or(999);
                if silence_hours >= 48 {
                    chrono::Duration::hours(12)
                } else {
                    chrono::Duration::hours(24)
                }
            };
            let mut dedup = self.notification_state.dedup.lock();
            let key = (chat_id, content_hash);
            if let Some((sent_at, _)) = dedup.get(&key)
                && now - *sent_at < dedup_window
            {
                tracing::debug!(chat_id, label, "[AssistantEvent] content-dedup suppressed");
                return Ok(ProactiveNotificationOutcome::AlreadySatisfied);
            }
            dedup.insert(key, (now, priority));
            Some(key)
        } else {
            None
        };

        if let Err(e) = tx.try_send(ProgressEvent::new(
            chat_id,
            ProgressPayload::OwnerNotice {
                text: message.to_string(),
                kind: OwnerNoticeKind::AssistantEvent,
            },
        )) {
            // Send failed: undo the dedup reservation so the retry is not suppressed.
            if let Some(key) = reserved_dedup_key {
                self.notification_state.dedup.lock().remove(&key);
            }
            return Err(anyhow::anyhow!("assistant-event progress send failed: {e}"));
        }
        self.notification_state.increment_daily_count(chat_id);
        self.notification_state
            .last_at
            .lock()
            .insert(chat_id, chrono::Utc::now());
        self.notification_state
            .label_dedup
            .lock()
            .insert((chat_id, label.to_string()), chrono::Utc::now());
        Ok(ProactiveNotificationOutcome::Delivered)
    }

    pub(crate) async fn emit_owner_timeline_notice_with_optional_telegram(
        &self,
        user_did: &str,
        chat_id: i64,
        label: &str,
        message: &str,
        priority: NotifyPriority,
        anchor_kind: Option<String>,
        anchor_ref: Option<String>,
    ) -> Result<ProactiveNotificationOutcome> {
        let telegram_fallback_allowed =
            Self::owner_timeline_notice_allows_telegram_fallback(priority, anchor_kind.as_deref());
        self.memory
            .push_l1(
                user_did,
                Message::text_with_anchor(
                    "assistant",
                    message,
                    chrono::Utc::now().timestamp(),
                    anchor_kind,
                    anchor_ref,
                ),
            )
            .await?;

        if self.try_owner_web_push_notice(user_did, message).await {
            tracing::info!(
                chat_id,
                label,
                "[Delivery] Web Push delivered owner timeline notice; Telegram fallback suppressed"
            );
            return Ok(ProactiveNotificationOutcome::Delivered);
        }

        if !telegram_fallback_allowed {
            tracing::info!(
                chat_id,
                label,
                priority = ?priority,
                "[Delivery] Owner timeline notice recorded; Telegram fallback suppressed"
            );
            return Ok(ProactiveNotificationOutcome::Delivered);
        }

        match self
            .emit_proactive_notification(chat_id, label, message, priority)
            .await?
        {
            ProactiveNotificationOutcome::Deferred => Ok(ProactiveNotificationOutcome::Delivered),
            outcome => Ok(outcome),
        }
    }

    pub(super) fn owner_timeline_notice_allows_telegram_fallback(
        priority: NotifyPriority,
        anchor_kind: Option<&str>,
    ) -> bool {
        if priority == NotifyPriority::Urgent {
            return true;
        }
        if priority == NotifyPriority::Routine {
            return false;
        }
        anchor_kind != Some("deferred_intent_group")
    }

    pub(super) async fn deliver_assistant_turn_outcome(
        &self,
        chat_id: i64,
        work_item: &crate::assistant_work_items::StoredAssistantWorkItem,
        outcome: &crate::assistant_work_items::AssistantTurnOutcome,
    ) -> Result<ProactiveNotificationOutcome> {
        match outcome.delivery {
            crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage => {
                let tz_name = self
                    .settings
                    .get("user_timezone")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "UTC".to_string());
                let Some(message) = Self::render_assistant_turn_delivery_message(outcome, &tz_name)
                else {
                    return Ok(ProactiveNotificationOutcome::AlreadySatisfied);
                };
                let priority = outcome
                    .notify_priority
                    .as_deref()
                    .map(NotifyPriority::from_str_or_default)
                    .unwrap_or(NotifyPriority::Important);
                let anchor_kind = outcome
                    .follow_up_anchor_kind
                    .clone()
                    .or_else(|| work_item.item.anchor_kind.map(|v| v.as_str().to_string()));
                let anchor_ref = outcome
                    .follow_up_anchor_ref
                    .clone()
                    .or_else(|| work_item.item.anchor_ref.clone());

                let label = format!("assistant_work_item:{}", work_item.item.dedup_key);

                // Durable exact dedup before the unconditional L1 push inside
                // emit_owner_timeline_notice: without it, a stored-outcome
                // replay or an LLM that ignores the stay-silent contract can
                // append the identical message to the Web timeline every fire
                // (Codex round-3). Scope guards, both fail-open toward
                // delivery:
                // - Urgent is exempt (a repeated urgent alert must never be
                //   silenced — matches every other dedup gate).
                // - RECURRING intents (interval_seconds present) are exempt:
                //   an owner-scheduled cadence repeats identical reminder text
                //   BY DESIGN; suppressing the 2nd "喝水提醒" of the day would
                //   break explicit owner intent. Their no-op-noise problem is
                //   handled upstream (prompt contract + self-reproposal
                //   suppression), and daily cadences sit outside the 12h
                //   window anyway.
                let is_recurring_intent = work_item
                    .item
                    .context
                    .get("interval_seconds")
                    .and_then(|v| v.as_i64())
                    .is_some();
                // group_key is LLM-chosen and NOT unique per obligation: two
                // DISTINCT one-shot intents may legitimately share it (and the
                // same text). Only dedup deferred intents whose anchor is the
                // auto-generated unique deferred_intent:{id} — a group_key'd
                // replay fails open (double post beats a lost reminder,
                // Codex round-6 HIGH).
                let anchor_unique_per_obligation = match work_item.item.source_kind {
                    crate::assistant_work_items::AssistantWorkSourceKind::DeferredIntent => {
                        work_item
                            .item
                            .context
                            .get("group_key")
                            .and_then(|v| v.as_str())
                            .is_none()
                    }
                    _ => true,
                };
                // Anchor-scoped: two DISTINCT one-shot obligations that render
                // identical text have different anchor identities and must
                // both deliver; only a same-anchor replay is a duplicate.
                // Missing either stable anchor component → fail open (deliver).
                if priority != NotifyPriority::Urgent
                    && !is_recurring_intent
                    && anchor_unique_per_obligation
                    && match (anchor_kind.as_deref(), anchor_ref.as_deref()) {
                        (Some(kind), Some(anchor)) => {
                            assistant_l1_exact_duplicate_with_anchor_within(
                                &self.memory,
                                &work_item.item.user_did,
                                &message,
                                12,
                                kind,
                                anchor,
                            )
                            .await
                        }
                        _ => false,
                    }
                {
                    tracing::info!(
                        label,
                        "[Delivery] durable L1 exact-dedup suppressed work-item delivery"
                    );
                    return Ok(ProactiveNotificationOutcome::AlreadySatisfied);
                }

                self.emit_owner_timeline_notice_with_optional_telegram(
                    &work_item.item.user_did,
                    chat_id,
                    &label,
                    &message,
                    priority,
                    anchor_kind,
                    anchor_ref,
                )
                .await
            }
            crate::assistant_work_items::AssistantTurnDelivery::DigestOnly
            | crate::assistant_work_items::AssistantTurnDelivery::Silent => {
                Ok(ProactiveNotificationOutcome::AlreadySatisfied)
            }
        }
    }

    pub(super) async fn try_owner_web_push_notice(&self, user_did: &str, message: &str) -> bool {
        crate::admin::try_owner_web_push_notice(&self.memory, user_did, message).await
    }

    pub async fn process_assistant_event_notifications(
        &self,
        user_did: &str,
        chat_id: i64,
        limit: usize,
    ) -> Result<usize> {
        // Fetch all decisions except the known non-immediate set so unknown/future
        // decisions (fail-open) are not silently dropped by a SQL IN-list filter.
        // CallbackNow, DigestLater, Taskify, MemoryOnly, and Ignore are handled by
        // their own delivery paths; they are skipped in the loop below.
        // SQL NOT IN excludes known non-immediate decisions so they do not consume
        // the LIMIT ahead of notify_now rows — prevents starvation when there are
        // more digest_later/promotional rows than the fetch limit.
        // Unknown/future decisions are not excluded (fail-open: they are fetched and
        // passed to the conversation gate below, same as notify_now).
        // The in-loop skip below is kept as a defensive guard.
        let immediate_events = self
            .memory
            .peek_new_assistant_events_for_consumer(
                user_did,
                "mail_notifier_immediate",
                limit,
                None,
                Some(&[
                    "callback_now",
                    "digest_later",
                    "taskify",
                    "memory_only",
                    "ignore",
                ]),
            )
            .await?;
        if immediate_events.is_empty() {
            return Ok(0);
        }

        let mut processed = 0usize;
        let mut suppressed = 0usize;
        let mut committed_immediate_id = None;
        for row in &immediate_events {
            // Skip decisions handled by other delivery paths (callback work-item
            // queue, digest assembly, task creation, memory-only store, ignore).
            // The cursor advances so these events are not re-processed next tick.
            // Unknown/future decisions fall through here → fail-open delivery.
            if matches!(
                row.event.decision,
                crate::assistant_events::AssistantEventDecision::CallbackNow
                    | crate::assistant_events::AssistantEventDecision::DigestLater
                    | crate::assistant_events::AssistantEventDecision::Taskify
                    | crate::assistant_events::AssistantEventDecision::MemoryOnly
                    | crate::assistant_events::AssistantEventDecision::Ignore
            ) {
                committed_immediate_id = Some(row.id);
                suppressed += 1;
                continue;
            }
            // Gate: routine Google/Calendar inbox notifications do not enter the
            // owner conversation (they are already visible in the Mail view and
            // drive the unread badge). The cursor still advances so the event is
            // not re-processed on the next tick.
            if !crate::assistant_events::assistant_event_enters_conversation(&row.event) {
                tracing::debug!(
                    source = row.event.source.as_str(),
                    kind = row.event.kind.as_str(),
                    dedup_key = %row.event.dedup_key,
                    "[AssistantEvent] routine inbox notification suppressed from chat"
                );
                committed_immediate_id = Some(row.id);
                suppressed += 1;
                continue;
            }
            if let Some(message) =
                crate::assistant_events::render_assistant_event_notification(&row.event)
            {
                let priority = Self::assistant_event_priority(&row.event);
                let label = format!(
                    "assistant_event:{}:{}",
                    row.event.source.as_str(),
                    row.event.dedup_key
                );
                match self
                    .emit_owner_timeline_notice_with_optional_telegram(
                        user_did,
                        chat_id,
                        &label,
                        &message,
                        priority,
                        Some("assistant_event".to_string()),
                        Some(format!("assistant_event:{}", row.id)),
                    )
                    .await?
                {
                    ProactiveNotificationOutcome::Delivered
                    | ProactiveNotificationOutcome::AlreadySatisfied => {
                        committed_immediate_id = Some(row.id);
                        processed += 1;
                    }
                    ProactiveNotificationOutcome::Deferred => break,
                }
            }
        }
        if let Some(last_id) = committed_immediate_id {
            self.memory
                .commit_assistant_event_cursor(user_did, "mail_notifier_immediate", last_id)
                .await?;
        }

        tracing::info!(
            chat_id,
            user_did,
            count = processed,
            suppressed,
            immediate_seen = immediate_events.len(),
            "[AssistantEvent] processed notification batch"
        );
        Ok(processed)
    }

    pub(crate) async fn run_bounded_assistant_work_item(
        &self,
        work_item: &crate::assistant_work_items::StoredAssistantWorkItem,
        chat_id: i64,
    ) -> Result<crate::assistant_work_items::AssistantTurnOutcome> {
        match work_item.item.source_kind {
            crate::assistant_work_items::AssistantWorkSourceKind::AssistantEvent => {
                let assistant_event_id = work_item
                    .item
                    .assistant_event_id
                    .context("assistant work item missing assistant_event_id")?;
                let assistant_event = self
                    .memory
                    .get_assistant_event_by_id(assistant_event_id)
                    .await?
                    .context("assistant work item references missing assistant event")?;

                anyhow::ensure!(
                    work_item.item.mode
                        == crate::assistant_work_items::AssistantWorkMode::BackgroundReview,
                    "unsupported assistant-event work item mode: {}",
                    work_item.item.mode.as_str()
                );
                anyhow::ensure!(
                    assistant_event.event.decision
                        == crate::assistant_events::AssistantEventDecision::CallbackNow,
                    "bounded assistant work runner currently only supports callback_now assistant events"
                );

                let callback_context = crate::assistant_work_items::render_callback_review_prompt(
                    &work_item.item,
                    &assistant_event,
                );
                let tool_allowlist = Self::background_review_tool_allowlist();
                let result = self
                    .run_callback_work_item_with_budget(
                        work_item,
                        chat_id,
                        &callback_context,
                        &tool_allowlist,
                    )
                    .await?;
                Ok(Self::callback_result_to_work_item_outcome(
                    &assistant_event,
                    &result,
                ))
            }
            crate::assistant_work_items::AssistantWorkSourceKind::DeferredIntent => {
                anyhow::ensure!(
                    work_item.item.mode
                        == crate::assistant_work_items::AssistantWorkMode::BackgroundAction,
                    "unsupported deferred-intent work item mode: {}",
                    work_item.item.mode.as_str()
                );
                let rendered_prompt =
                    crate::assistant_work_items::render_deferred_intent_callback_prompt(
                        &work_item.item,
                    );
                let chain_depth = work_item
                    .item
                    .context
                    .get("chain_depth")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let callback_context = format!(
                    "[INTENT_CALLBACK:depth={}] {}",
                    chain_depth, rendered_prompt
                );
                let group_key = work_item
                    .item
                    .context
                    .get("group_key")
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string());
                let intent_id = work_item
                    .item
                    .context
                    .get("intent_id")
                    .and_then(|v| v.as_i64())
                    .context("deferred-intent work item missing intent_id")?;
                let source_chat_id = work_item
                    .item
                    .context
                    .get("chat_id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(chat_id);
                let is_recurring = work_item
                    .item
                    .context
                    .get("interval_seconds")
                    .and_then(|v| v.as_i64())
                    .is_some();

                *self.tool_exec.callback_group_key.lock().await = group_key;
                if is_recurring {
                    *self.tool_exec.callback_task_info.lock().await =
                        Some((intent_id, source_chat_id));
                }

                let tool_allowlist = Self::background_action_tool_allowlist();
                let result = self
                    .run_callback_work_item_with_budget(
                        work_item,
                        source_chat_id,
                        &callback_context,
                        &tool_allowlist,
                    )
                    .await;

                *self.tool_exec.callback_group_key.lock().await = None;
                *self.tool_exec.callback_task_info.lock().await = None;

                let result = result?;
                Ok(Self::deferred_intent_result_to_work_item_outcome(
                    &work_item.item.context,
                    &result,
                ))
            }
            crate::assistant_work_items::AssistantWorkSourceKind::HeartbeatFollowUp => {
                match work_item.item.mode {
                    crate::assistant_work_items::AssistantWorkMode::BackgroundReview => {
                        let rendered_prompt =
                            crate::assistant_work_items::render_heartbeat_follow_up_prompt(
                                &work_item.item,
                            );
                        let tool_allowlist = Self::heartbeat_synthesis_tool_allowlist();
                        let result = self
                            .run_callback_work_item_with_budget(
                                work_item,
                                chat_id,
                                &rendered_prompt,
                                &tool_allowlist,
                            )
                            .await?;
                        Ok(Self::heartbeat_follow_up_result_to_work_item_outcome(
                            &work_item.item,
                            &result,
                        ))
                    }
                    crate::assistant_work_items::AssistantWorkMode::DigestAssembly => {
                        let consumer = work_item
                            .item
                            .context
                            .get("assistant_event_consumer")
                            .and_then(|v| v.as_str())
                            .unwrap_or("mail_notifier_digest");
                        let last_event_id = work_item
                            .item
                            .context
                            .get("last_event_id")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(i64::MAX);
                        let limit = work_item
                            .item
                            .context
                            .get("event_count")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize)
                            .unwrap_or(25)
                            .max(1);
                        let digest_events = self
                            .memory
                            .peek_new_assistant_events_for_consumer(
                                &work_item.item.user_did,
                                consumer,
                                limit,
                                Some(&["digest_later"]),
                                None,
                            )
                            .await?;
                        let digest_events: Vec<_> = digest_events
                            .into_iter()
                            .take_while(|row| row.id <= last_event_id)
                            .collect();
                        let Some(message) =
                            crate::assistant_events::render_assistant_event_digest_message(
                                &digest_events,
                                6,
                            )
                        else {
                            return Ok(crate::assistant_work_items::AssistantTurnOutcome::silent(
                                "Heartbeat digest assembly found no remaining eligible low-signal mail.",
                            ));
                        };
                        Ok(Self::heartbeat_digest_result_to_work_item_outcome(
                            format!(
                                "Assembled digest covering {} lower-signal email(s).",
                                digest_events.len()
                            ),
                            Some(message),
                        ))
                    }
                    other => anyhow::bail!(
                        "unsupported heartbeat follow-up work item mode: {}",
                        other.as_str()
                    ),
                }
            }
            other => anyhow::bail!(
                "unsupported assistant work item source kind: {}",
                other.as_str()
            ),
        }
    }

    /// Give up on a bounded work item after this many failed attempts
    /// (one attempt per periodic tick).
    const MAX_BOUNDED_WORK_ITEM_ATTEMPTS: u64 = 3;

    pub(super) async fn process_work_items_matching(
        &self,
        user_did: &str,
        chat_id: i64,
        filter: &crate::assistant_work_items::AssistantWorkClaimFilter,
        limit: usize,
    ) -> Result<usize> {
        let mut completed = 0usize;
        for _ in 0..limit {
            let Some(work_item) = self
                .memory
                .claim_next_assistant_work_item_matching(user_did, filter)
                .await?
            else {
                break;
            };

            let outcome = match work_item.item.result.clone().and_then(|value| {
                serde_json::from_value::<crate::assistant_work_items::AssistantTurnOutcome>(value)
                    .ok()
            }) {
                Some(outcome) => outcome,
                None => match self
                    .run_bounded_assistant_work_item(&work_item, chat_id)
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(e) => {
                        // Count failures across ticks so a permanently failing
                        // item cannot be re-claimed forever (it would otherwise
                        // stay the lowest (priority, id) row and starve its lane).
                        let attempts = work_item
                            .item
                            .result
                            .as_ref()
                            .and_then(|v| v.get("attempts"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0)
                            + 1;
                        let give_up = attempts >= Self::MAX_BOUNDED_WORK_ITEM_ATTEMPTS;
                        let next_status = if give_up {
                            crate::assistant_work_items::AssistantWorkStatus::Cancelled
                        } else {
                            crate::assistant_work_items::AssistantWorkStatus::Queued
                        };
                        tracing::warn!(
                            error = %e,
                            work_item_id = work_item.id,
                            attempts,
                            give_up,
                            "[AssistantWorkItem] bounded runner failed"
                        );
                        let error_result = serde_json::json!({
                            "generated_at": chrono::Utc::now().to_rfc3339(),
                            "error": format!("{e:#}"),
                            "attempts": attempts,
                        });
                        self.memory
                            .update_assistant_work_item(
                                work_item.id,
                                next_status,
                                Some(&error_result),
                            )
                            .await?;
                        // One attempt per tick: do not re-claim in the same
                        // pass, the periodic tick provides the backoff.
                        break;
                    }
                },
            };

            let delivery = self
                .deliver_assistant_turn_outcome(chat_id, &work_item, &outcome)
                .await?;
            let outcome_json = serde_json::to_value(&outcome)
                .context("serializing assistant turn outcome for work item")?;
            let next_status = match delivery {
                ProactiveNotificationOutcome::Delivered
                | ProactiveNotificationOutcome::AlreadySatisfied => {
                    crate::assistant_work_items::AssistantWorkStatus::Done
                }
                ProactiveNotificationOutcome::Deferred => {
                    crate::assistant_work_items::AssistantWorkStatus::Queued
                }
            };
            if next_status == crate::assistant_work_items::AssistantWorkStatus::Done
                && work_item.item.source_kind
                    == crate::assistant_work_items::AssistantWorkSourceKind::HeartbeatFollowUp
                && work_item.item.mode
                    == crate::assistant_work_items::AssistantWorkMode::DigestAssembly
                && let Err(e) = self
                    .commit_digest_cursor_for_work_item(&work_item.item.user_did, &work_item.item)
                    .await
            {
                tracing::warn!(
                    error = %e,
                    work_item_id = work_item.id,
                    "[AssistantWorkItem] failed to commit digest cursor; requeueing"
                );
                self.memory
                    .update_assistant_work_item(
                        work_item.id,
                        crate::assistant_work_items::AssistantWorkStatus::Queued,
                        Some(&outcome_json),
                    )
                    .await?;
                continue;
            }
            self.memory
                .update_assistant_work_item(work_item.id, next_status, Some(&outcome_json))
                .await?;
            if next_status == crate::assistant_work_items::AssistantWorkStatus::Done {
                completed += 1;
            }
        }
        Ok(completed)
    }

    pub async fn process_callback_review_work_items(
        &self,
        user_did: &str,
        chat_id: i64,
        limit: usize,
    ) -> Result<usize> {
        let completed = self
            .process_work_items_matching(
                user_did,
                chat_id,
                &crate::assistant_work_items::AssistantWorkClaimFilter::callback_review(),
                limit,
            )
            .await?;
        if completed > 0 {
            tracing::info!(
                user_did,
                chat_id,
                completed,
                "[AssistantWorkItem] processed bounded callback-review items"
            );
        }
        Ok(completed)
    }

    pub async fn process_deferred_intent_work_items(
        &self,
        user_did: &str,
        chat_id: i64,
        limit: usize,
    ) -> Result<usize> {
        let completed = self
            .process_work_items_matching(
                user_did,
                chat_id,
                &crate::assistant_work_items::AssistantWorkClaimFilter::deferred_intent_callback(),
                limit,
            )
            .await?;
        if completed > 0 {
            tracing::info!(
                user_did,
                chat_id,
                completed,
                "[AssistantWorkItem] processed deferred-intent callback items"
            );
        }
        Ok(completed)
    }

    pub async fn process_heartbeat_follow_up_work_items(
        &self,
        user_did: &str,
        chat_id: i64,
        limit: usize,
    ) -> Result<usize> {
        let completed = self
            .process_work_items_matching(
                user_did,
                chat_id,
                &crate::assistant_work_items::AssistantWorkClaimFilter::heartbeat_follow_up(),
                limit,
            )
            .await?;
        if completed > 0 {
            tracing::info!(
                user_did,
                chat_id,
                completed,
                "[AssistantWorkItem] processed heartbeat follow-up items"
            );
        }
        Ok(completed)
    }

    pub async fn process_heartbeat_digest_work_items(
        &self,
        user_did: &str,
        chat_id: i64,
        limit: usize,
    ) -> Result<usize> {
        let completed = self
            .process_work_items_matching(
                user_did,
                chat_id,
                &crate::assistant_work_items::AssistantWorkClaimFilter::heartbeat_digest_assembly(),
                limit,
            )
            .await?;
        if completed > 0 {
            tracing::info!(
                user_did,
                chat_id,
                completed,
                "[AssistantWorkItem] processed heartbeat digest-assembly items"
            );
        }
        Ok(completed)
    }

    pub(crate) async fn persist_assistant_event(
        &self,
        raw_event: &crate::assistant_events::RawExternalEvent,
        assistant_event: &crate::assistant_events::AssistantEvent,
    ) -> Result<()> {
        let raw_id = self.memory.store_raw_external_event(raw_event).await?;
        let mut assistant_event = assistant_event.clone();
        assistant_event.raw_event_id = Some(raw_id);
        let assistant_event_id = self.memory.store_assistant_event(&assistant_event).await?;

        if assistant_event.decision == crate::assistant_events::AssistantEventDecision::CallbackNow
        {
            let work_item = crate::assistant_work_items::build_callback_review_work_item(
                &crate::assistant_events::StoredAssistantEvent {
                    id: assistant_event_id,
                    event: assistant_event.clone(),
                },
            );
            self.memory.store_assistant_work_item(&work_item).await?;
        }

        let dispatch = crate::assistant_events::build_assistant_event_dispatch(&assistant_event);
        if let Some(observed) = dispatch.observed_event {
            self.memory
                .record_observed_event(
                    &observed.source,
                    &observed.severity,
                    &observed.summary,
                    Some(&observed.payload),
                )
                .await?;
        }
        Ok(())
    }

    /// Set discovered external Agent Skills.
    pub fn set_agent_skills(&mut self, skills: Vec<AgentSkill>) {
        self.agent_skills = skills;
    }

    /// Get a reference to loaded agent skills (for registry initialization).
    pub fn agent_skills(&self) -> &[AgentSkill] {
        &self.agent_skills
    }

    /// Set the Chronos engine reference for monitoring intent pseudo-tools.
    /// Uses interior mutability so it can be called after Arc wrapping.
    pub fn set_chronos(&self, chronos: Arc<ChronosEngine>) {
        *self.services.chronos.lock() = Some(chronos);
    }

    /// Lazily load persisted alert policies from SQLite into ChronosEngine.
    /// Called once on first alert-policy pseudo-tool use (avoids startup ordering issues).
    pub(crate) async fn ensure_alert_policies_loaded(&self) {
        if self
            .caches
            .alert_policies_loaded
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        if let Ok(all_settings) = self.memory.load_all_settings().await
            && let Some(Value::String(json)) = all_settings.get("alert_policies")
            && let Ok(policies) = serde_json::from_str::<
                std::collections::HashMap<String, crate::chronos::AlertPolicy>,
            >(json)
            && let Some(chronos) = self.services.chronos.lock().as_ref()
        {
            let count = policies.len();
            for (skill, policy) in policies {
                chronos.set_policy(&skill, policy);
            }
            tracing::info!(count, "Loaded persisted alert policies from SQLite");
        }
        self.caches
            .alert_policies_loaded
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Set the Telegram bot handle for sending photos and voice messages.
    pub fn set_telegram_bot(&mut self, bot: teloxide::Bot) {
        self.services.telegram_bot = Some(bot);
    }

    /// Set credential store info for Calendar auto-setup.
    pub fn set_credential_store(&mut self, db_path: String, aes_key: [u8; 32]) {
        self.credentials.credential_db_path = Some(db_path);
        self.credentials.credential_aes_key = Some(aes_key);
    }

    /// Set the public URL for OAuth callback.
    pub fn set_public_url(&mut self, url: String) {
        self.credentials.public_url = Some(url);
    }

    /// Open a fresh connection to the encrypted credential store.
    /// Errors when `set_credential_store` was never called (e.g. headless
    /// test orchestrators) — callers surface that as a structured error.
    pub(crate) fn open_credential_store(&self) -> Result<crate::credential_store::CredentialStore> {
        let (db_path, aes_key) = match (
            &self.credentials.credential_db_path,
            &self.credentials.credential_aes_key,
        ) {
            (Some(p), Some(k)) => (p.clone(), *k),
            _ => anyhow::bail!("Credential store not configured"),
        };
        crate::credential_store::CredentialStore::open(
            crate::encrypted_sqlite::open(&db_path)?,
            aes_key,
        )
    }

    pub(crate) fn jev_active_key(&self) -> Option<String> {
        let resolved = if self.credentials.credential_db_path.is_some() {
            self.open_credential_store()
                .and_then(|store| crate::jev_config::resolve(Some(&store)))
        } else {
            crate::jev_config::resolve(None)
        };
        match resolved {
            Ok(config) => {
                if config.enabled && config.api_key.is_none() {
                    crate::behavior_notice::record(
                        "jev_key_unavailable",
                        "已启用的 Jev 缺少可用密钥，本次使用本地检查和必要确认。",
                        "Enabled Jev has no usable key. Local checks and required approvals apply.",
                    );
                }
                config.active_key()
            }
            Err(_) => {
                crate::behavior_notice::record(
                    "jev_config_unavailable",
                    "Jev 配置暂时无法读取，本次未调用 Jev；本地检查和必要确认继续生效。",
                    "Jev configuration could not be read. Jev was not called; local checks and required approvals remain.",
                );
                None
            }
        }
    }

    /// Store Google OAuth client credentials in the credential store.    /// Store Google OAuth client credentials in the credential store.
    ///
    /// Used by the Telegram credential-paste handler to persist client_id/secret
    /// without exposing them to the LLM.
    pub fn store_google_client_credentials(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<()> {
        self.store_google_client_credentials_named_for_chat(
            client_id,
            client_secret,
            "primary",
            None,
        )
    }

    /// Store pending OAuth credentials with an account name for multi-account support.
    pub fn store_google_client_credentials_named(
        &self,
        client_id: &str,
        client_secret: &str,
        account_name: &str,
    ) -> Result<()> {
        self.store_google_client_credentials_named_for_chat(
            client_id,
            client_secret,
            account_name,
            None,
        )
    }

    pub fn store_google_client_credentials_named_for_chat(
        &self,
        client_id: &str,
        client_secret: &str,
        account_name: &str,
        chat_id: Option<i64>,
    ) -> Result<()> {
        let (db_path, aes_key) = match (
            &self.credentials.credential_db_path,
            &self.credentials.credential_aes_key,
        ) {
            (Some(p), Some(k)) => (p.clone(), *k),
            _ => anyhow::bail!("Credential store not configured"),
        };
        let cred_store = crate::credential_store::CredentialStore::open(
            crate::encrypted_sqlite::open(&db_path)?,
            aes_key,
        )?;
        let pending_service = crate::google_auth::pending_cred_service(chat_id);
        if let Err(e) =
            crate::google_auth::delete_scope_upgrade_audit_context(&cred_store, &pending_service)
        {
            tracing::debug!(error = %e, "no stale Google scope upgrade audit context to clear");
        }
        cred_store.store(&pending_service, "client_id", client_id)?;
        cred_store.store(&pending_service, "client_secret", client_secret)?;
        // Store account name so exchange_google_auth_code knows which account this is for
        cred_store.store(&pending_service, "account_name", account_name)?;
        // Timestamp the staging so abandoned flows can be cleaned up before
        // they poison the next setup/upgrade-scopes call for a different
        // account. See cleanup_stale_pending_oauth.
        cred_store.store(
            &pending_service,
            "staged_at",
            &chrono::Utc::now().to_rfc3339(),
        )?;
        Ok(())
    }

    pub(crate) fn store_google_scope_upgrade_audit_context_for_chat(
        &self,
        context: &crate::google_auth::GoogleScopeUpgradeAuditContext,
        chat_id: Option<i64>,
    ) -> Result<()> {
        let (db_path, aes_key) = match (
            &self.credentials.credential_db_path,
            &self.credentials.credential_aes_key,
        ) {
            (Some(p), Some(k)) => (p.clone(), *k),
            _ => anyhow::bail!("Credential store not configured"),
        };
        let cred_store = crate::credential_store::CredentialStore::open(
            crate::encrypted_sqlite::open(&db_path)?,
            aes_key,
        )?;
        let pending_service = crate::google_auth::pending_cred_service(chat_id);
        crate::google_auth::store_scope_upgrade_audit_context(
            &cred_store,
            &pending_service,
            context,
        )
    }

    pub(crate) fn stage_pending_oauth_flow_for_chat(
        &self,
        account_name: &str,
        chat_id: Option<i64>,
    ) -> Result<String> {
        let chat_id =
            chat_id.ok_or_else(|| anyhow::anyhow!("chat-scoped OAuth flow requires chat_id"))?;
        self.migrate_legacy_pending_oauth_to_chat(chat_id)?;
        let (db_path, aes_key) = match (
            &self.credentials.credential_db_path,
            &self.credentials.credential_aes_key,
        ) {
            (Some(p), Some(k)) => (p.clone(), *k),
            _ => anyhow::bail!("Credential store not configured"),
        };
        let cred_store = crate::credential_store::CredentialStore::open(
            crate::encrypted_sqlite::open(&db_path)?,
            aes_key,
        )?;
        let base_pending_service = crate::google_auth::pending_cred_service(Some(chat_id));
        let client_id = cred_store
            .get(&base_pending_service, "client_id")?
            .ok_or_else(|| anyhow::anyhow!("No staged OAuth client_id for chat {chat_id}"))?;
        let client_secret = cred_store
            .get(&base_pending_service, "client_secret")?
            .ok_or_else(|| anyhow::anyhow!("No staged OAuth client_secret for chat {chat_id}"))?;
        let flow_id = crate::google_auth::generate_setup_flow_id();
        let flow_pending_service =
            crate::google_auth::pending_cred_service_for_flow(chat_id, &flow_id);
        cred_store.store(&flow_pending_service, "client_id", &client_id)?;
        cred_store.store(&flow_pending_service, "client_secret", &client_secret)?;
        cred_store.store(&flow_pending_service, "account_name", account_name)?;
        cred_store.store(
            &flow_pending_service,
            "staged_at",
            &chrono::Utc::now().to_rfc3339(),
        )?;
        if let Some(context) = crate::google_auth::load_scope_upgrade_audit_context(
            &cred_store,
            &base_pending_service,
        )? {
            crate::google_auth::store_scope_upgrade_audit_context(
                &cred_store,
                &flow_pending_service,
                &context,
            )?;
        }
        cred_store.store(&base_pending_service, "active_flow_id", &flow_id)?;
        Ok(flow_id)
    }

    pub(super) fn migrate_legacy_pending_oauth_to_chat(&self, chat_id: i64) -> Result<bool> {
        let (db_path, aes_key) = match (
            &self.credentials.credential_db_path,
            &self.credentials.credential_aes_key,
        ) {
            (Some(p), Some(k)) => (p.clone(), *k),
            _ => return Ok(false),
        };
        let cred_store = crate::credential_store::CredentialStore::open(
            crate::encrypted_sqlite::open(&db_path)?,
            aes_key,
        )?;
        let legacy_service = crate::google_auth::pending_cred_service(None);
        let chat_service = crate::google_auth::pending_cred_service(Some(chat_id));

        let legacy_client_id = cred_store.get(&legacy_service, "client_id")?;
        let legacy_client_secret = cred_store.get(&legacy_service, "client_secret")?;
        let legacy_account_name = cred_store.get(&legacy_service, "account_name")?;
        let legacy_staged_at = cred_store.get(&legacy_service, "staged_at")?;

        let has_legacy_payload = legacy_client_id.is_some()
            || legacy_client_secret.is_some()
            || legacy_account_name.is_some()
            || legacy_staged_at.is_some();
        if !has_legacy_payload {
            return Ok(false);
        }

        let chat_has_pending = cred_store.get(&chat_service, "client_id")?.is_some()
            || cred_store.get(&chat_service, "client_secret")?.is_some()
            || cred_store.get(&chat_service, "account_name")?.is_some()
            || cred_store.get(&chat_service, "staged_at")?.is_some();

        if !chat_has_pending {
            if let Some(client_id) = legacy_client_id.as_deref() {
                cred_store.store(&chat_service, "client_id", client_id)?;
            }
            if let Some(client_secret) = legacy_client_secret.as_deref() {
                cred_store.store(&chat_service, "client_secret", client_secret)?;
            }
            if let Some(account_name) = legacy_account_name.as_deref() {
                cred_store.store(&chat_service, "account_name", account_name)?;
            }
            if let Some(staged_at) = legacy_staged_at.as_deref() {
                cred_store.store(&chat_service, "staged_at", staged_at)?;
            }
            tracing::info!(
                chat_id,
                "migrated legacy pending OAuth staging into chat-scoped slot"
            );
        } else {
            tracing::info!(
                chat_id,
                "dropping legacy pending OAuth staging because chat-scoped staging already exists"
            );
        }

        for key in &["client_id", "client_secret", "account_name", "staged_at"] {
            if let Err(e) = cred_store.delete(&legacy_service, key) {
                tracing::warn!(
                    error = %e,
                    key,
                    "failed to delete legacy pending OAuth key during migration cleanup"
                );
            }
        }
        Ok(true)
    }

    pub(super) fn active_pending_oauth_flow_for_chat(
        &self,
        chat_id: Option<i64>,
    ) -> Result<Option<String>> {
        let chat_id = match chat_id {
            Some(chat_id) => chat_id,
            None => return Ok(None),
        };
        self.migrate_legacy_pending_oauth_to_chat(chat_id)?;
        let (db_path, aes_key) = match (
            &self.credentials.credential_db_path,
            &self.credentials.credential_aes_key,
        ) {
            (Some(p), Some(k)) => (p.clone(), *k),
            _ => return Ok(None),
        };
        let cred_store = crate::credential_store::CredentialStore::open(
            crate::encrypted_sqlite::open(&db_path)?,
            aes_key,
        )?;
        cred_store.get(
            &crate::google_auth::pending_cred_service(Some(chat_id)),
            "active_flow_id",
        )
    }

    /// Clear CRED_SERVICE_PENDING entries if the existing staging is older
    /// than `ttl_secs` (abandonment cleanup). Also clears if the staged
    /// account_name differs from `intended_account` — prevents an abandoned
    /// PENDING for account A from redirecting account B's callback.
    ///
    /// Returns Ok(true) if PENDING was wiped, Ok(false) if it was absent or
    /// still fresh enough to keep.
    ///
    /// This is a best-effort cleanup: errors writing to the store log and
    /// continue, they do not abort the upstream operation.
    pub fn cleanup_stale_pending_oauth(
        &self,
        intended_account: &str,
        ttl_secs: i64,
    ) -> Result<bool> {
        self.cleanup_stale_pending_oauth_for_chat(intended_account, ttl_secs, None)
    }

    pub fn cleanup_stale_pending_oauth_for_chat(
        &self,
        intended_account: &str,
        ttl_secs: i64,
        chat_id: Option<i64>,
    ) -> Result<bool> {
        let (db_path, aes_key) = match (
            &self.credentials.credential_db_path,
            &self.credentials.credential_aes_key,
        ) {
            (Some(p), Some(k)) => (p.clone(), *k),
            _ => return Ok(false),
        };
        let cred_store = crate::credential_store::CredentialStore::open(
            crate::encrypted_sqlite::open(&db_path)?,
            aes_key,
        )?;
        let pending_service = crate::google_auth::pending_cred_service(chat_id);

        let existing_account = cred_store
            .get(&pending_service, "account_name")?
            .unwrap_or_default();
        let staged_at_raw = cred_store
            .get(&pending_service, "staged_at")?
            .unwrap_or_default();

        if existing_account.is_empty() && staged_at_raw.is_empty() {
            return Ok(false);
        }

        let age_too_old = match chrono::DateTime::parse_from_rfc3339(&staged_at_raw) {
            Ok(staged_at) => {
                (chrono::Utc::now() - staged_at.with_timezone(&chrono::Utc)).num_seconds()
                    > ttl_secs
            }
            Err(_) => {
                // Missing or unparseable timestamp on an otherwise-present
                // staging means the entry pre-dates the `staged_at` field;
                // err on the side of wiping so we re-stage with a real
                // timestamp. Log so the drift is visible.
                tracing::warn!(
                    existing_account,
                    staged_at_raw,
                    "pending OAuth entry has missing/invalid staged_at — wiping to re-stage cleanly"
                );
                true
            }
        };

        let different_account =
            !existing_account.is_empty() && existing_account != intended_account;

        if !age_too_old && !different_account {
            return Ok(false);
        }

        if different_account {
            tracing::warn!(
                existing_account,
                intended_account,
                staged_at_raw,
                "wiping stale pending OAuth entry belonging to a different account before re-staging — prior flow was abandoned"
            );
        } else {
            tracing::warn!(
                existing_account,
                staged_at_raw,
                ttl_secs,
                "wiping stale pending OAuth entry past TTL"
            );
        }

        for key in &[
            "client_id",
            "client_secret",
            "account_name",
            "staged_at",
            "active_flow_id",
        ] {
            if let Err(e) = cred_store.delete(&pending_service, key) {
                tracing::warn!(
                    error = %e,
                    key,
                    "failed to delete stale pending OAuth key during cleanup"
                );
            }
        }

        // Clean up *every* flow-scoped pending OAuth slot for this chat,
        // not just the one referenced by the (now-deleted) `active_flow_id`.
        // Each `setup_google` retry stages a fresh
        // `google_oauth_pending:{chat_id}:{flow_id}` row; if a flow is
        // abandoned mid-way, its row is otherwise never cleaned up, leaving
        // stale `client_secret` ciphertext in the credential store and a
        // window where a late-arriving callback for that flow_id could
        // still find a usable staging row. Scanning by service-key prefix
        // ensures the cleanup is exhaustive even when `active_flow_id`
        // has already been overwritten.
        if let Some(chat_id) = chat_id {
            let flow_prefix = format!(
                "{base}:{chat_id}:",
                base = crate::google_auth::CRED_SERVICE_PENDING
            );
            match cred_store.list_services() {
                Ok(rows) => {
                    let mut wiped_flows: std::collections::BTreeSet<String> =
                        std::collections::BTreeSet::new();
                    for (service, key) in rows {
                        if !service.starts_with(&flow_prefix) {
                            continue;
                        }
                        if let Err(e) = cred_store.delete(&service, &key) {
                            tracing::warn!(
                                error = %e,
                                service = %service,
                                key = %key,
                                "failed to delete stale flow-scoped pending OAuth key during cleanup"
                            );
                        } else {
                            wiped_flows.insert(service);
                        }
                    }
                    if !wiped_flows.is_empty() {
                        tracing::info!(
                            chat_id,
                            count = wiped_flows.len(),
                            "wiped stale flow-scoped pending OAuth slots during cleanup"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "failed to enumerate credential store for flow-scoped cleanup"
                    );
                }
            }
        }
        Ok(true)
    }

    /// Exchange a Google OAuth authorization code (Scenario C: manual paste).
    ///
    /// Returns Ok(user-facing completion message — includes the granted scope
    /// list and, for a runtime-added NEW account while the cron scheduler is
    /// running, the restart-transparency hint) after storing tokens and
    /// hot-reloading. Uses the pending account_name to save to the correct
    /// credential service key and register in the AccountRegistry for
    /// multi-account support.
    pub async fn exchange_google_auth_code(&self, code: &str) -> Result<String> {
        self.exchange_google_auth_code_for_chat(code, None).await
    }

    pub async fn exchange_google_auth_code_for_chat(
        &self,
        code: &str,
        chat_id: Option<i64>,
    ) -> Result<String> {
        if let Some(chat_id) = chat_id {
            self.migrate_legacy_pending_oauth_to_chat(chat_id)?;
        }
        let (db_path, aes_key) = match (
            &self.credentials.credential_db_path,
            &self.credentials.credential_aes_key,
        ) {
            (Some(p), Some(k)) => (p.clone(), *k),
            _ => anyhow::bail!("Credential store not configured"),
        };

        // Read pending client credentials + account name
        let db_path_read = db_path.clone();
        let active_flow_id = self.active_pending_oauth_flow_for_chat(chat_id)?;
        let pending_service = match (chat_id, active_flow_id.as_deref()) {
            (Some(chat_id), Some(flow_id)) => {
                crate::google_auth::pending_cred_service_for_flow(chat_id, flow_id)
            }
            _ => crate::google_auth::pending_cred_service(chat_id),
        };
        let (client_id, client_secret, account_name) =
            tokio::task::spawn_blocking(move || -> Result<(String, String, String)> {
                let cred_store = crate::credential_store::CredentialStore::open(
                    crate::encrypted_sqlite::open(&db_path_read)?,
                    aes_key,
                )?;
                let cid = cred_store
                    .get(&pending_service, "client_id")?
                    .ok_or_else(|| {
                        anyhow::anyhow!("No pending OAuth setup — run setup_google first")
                    })?;
                let cs = cred_store
                    .get(&pending_service, "client_secret")?
                    .ok_or_else(|| anyhow::anyhow!("No pending OAuth client_secret"))?;
                let name = cred_store
                    .get(&pending_service, "account_name")?
                    .unwrap_or_else(|| "primary".to_string());
                Ok((cid, cs, name))
            })
            .await
            .context("spawn_blocking")??;

        // For Scenario C (manual paste), redirect_uri is the localhost callback
        // that Google shows the code on. We use "postmessage" for copy-paste flow.
        let redirect_uri = if let Some(ref public_url) = self.credentials.public_url {
            format!("{public_url}/auth/callback/google")
        } else {
            format!("http://localhost:{}/callback", 8765)
        };

        let cid = client_id.clone();
        let cs = client_secret.clone();
        let code_owned = code.to_string();
        let ruri = redirect_uri.clone();
        let (manager, granted_scopes) = tokio::task::spawn_blocking(move || {
            crate::google_auth::GoogleAuthManager::exchange_code_with_redirect(
                &cid,
                &cs,
                &code_owned,
                &ruri,
            )
        })
        .await
        .context("exchange task")??;

        let manager = Arc::new(manager);

        let cred_service = crate::google_auth::google_oauth_cred_service(&account_name);
        let manager_save = manager.clone();
        let db_path_save = db_path.clone();
        let cred_svc = cred_service.clone();
        let scope_list = granted_scopes
            .iter()
            .map(|s| s.rsplit('/').next().unwrap_or(s).to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let pending_service = match (chat_id, active_flow_id.as_deref()) {
            (Some(chat_id), Some(flow_id)) => {
                crate::google_auth::pending_cred_service_for_flow(chat_id, flow_id)
            }
            _ => crate::google_auth::pending_cred_service(chat_id),
        };
        let scope_audit_context = tokio::task::spawn_blocking(move || -> Result<Option<crate::google_auth::GoogleScopeUpgradeAuditContext>> {
            let cred_store = crate::credential_store::CredentialStore::open(
                crate::encrypted_sqlite::open(&db_path_save)?,
                aes_key,
            )?;
            let scope_audit_context =
                crate::google_auth::load_scope_upgrade_audit_context(&cred_store, &pending_service)?;
            manager_save.save_to_credential_store_named(&cred_store, &cred_svc)?;
            for key in &["client_id", "client_secret", "account_name", "staged_at"] {
                if let Err(e) = cred_store.delete(&pending_service, key) {
                    tracing::warn!(error = %e, key, "Failed to clean up pending credential after OAuth exchange");
                }
            }
            if let Err(e) =
                crate::google_auth::delete_scope_upgrade_audit_context(&cred_store, &pending_service)
            {
                tracing::warn!(error = %e, "Failed to clean up scope upgrade audit context");
            }
            if let Some(chat_id) = chat_id {
                let base_pending_service = crate::google_auth::pending_cred_service(Some(chat_id));
                if let Err(e) = cred_store.delete(&base_pending_service, "active_flow_id") {
                    tracing::warn!(error = %e, "Failed to clean up active_flow_id after OAuth exchange");
                }
                if let Err(e) = crate::google_auth::delete_scope_upgrade_audit_context(
                    &cred_store,
                    &base_pending_service,
                ) {
                    tracing::debug!(error = %e, "no base scope upgrade audit context to clear");
                }
            }
            Ok(scope_audit_context)
        })
        .await
        .context("save task")??;

        self.sandbox
            .set_dynamic_auth(&["www.googleapis.com"], manager);
        tracing::info!(account = %account_name, scopes = ?granted_scopes, "Google OAuth2 setup completed via manual code exchange");

        if let Some(context) = scope_audit_context {
            let record = crate::google_auth::build_scope_upgrade_audit_record(
                &context,
                &granted_scopes,
                "manual",
            );
            let key = format!(
                "scope-upgrade-{}-{:016x}",
                chrono::Utc::now().timestamp_millis(),
                rand::random::<u64>()
            );
            if let Err(e) = self
                .memory
                .store_skill_state("google_auth_audit", &key, &record.to_string())
                .await
            {
                tracing::warn!(error = %e, key, "failed to store Google scope upgrade audit record");
            }
        }

        Ok(format!(
            "Google Calendar/Drive connected for `{account_name}` (scopes: {scope_list})."
        ))
    }

    /// Check if Google OAuth is currently configured and working.
    ///
    /// Returns structured status so callers can distinguish missing setup,
    /// pending authorization, and token refresh failures without treating every
    /// non-ready state as "not configured".
    pub fn google_auth_status(&self) -> serde_json::Value {
        if self
            .sandbox
            .get_auth_headers()
            .contains_key("www.googleapis.com")
        {
            serde_json::json!({
                "configured": true,
                "status_kind": "ready",
                "auth_surface": "sandbox",
                "token_status": "valid",
                "message": "Google OAuth2 is configured and active."
            })
        } else if self.has_pending_oauth_setup() {
            serde_json::json!({
                "configured": false,
                "pending_setup": true,
                "status_kind": "pending_authorization",
                "auth_surface": "pending_credentials",
                "message": "Google OAuth2 client credentials received but NOT yet authorized. The user has already provided client_id and client_secret. Next step: call setup_google with action='setup' to generate the authorization URL, then have the user complete the OAuth flow."
            })
        } else {
            serde_json::json!({
                "configured": false,
                "status_kind": "missing_credentials",
                "auth_surface": "sandbox",
                "message": "Google OAuth2 is not configured. Use setup_google to connect."
            })
        }
    }

    /// Check if there's a pending OAuth setup (client credentials stored but not yet authorized).
    pub fn has_pending_oauth_setup(&self) -> bool {
        self.has_pending_oauth_setup_for_chat(None)
    }

    pub fn has_pending_oauth_setup_for_chat(&self, chat_id: Option<i64>) -> bool {
        if let Some(chat_id) = chat_id
            && let Err(e) = self.migrate_legacy_pending_oauth_to_chat(chat_id)
        {
            tracing::warn!(
                error = %e,
                chat_id,
                "failed to migrate legacy pending OAuth staging before checking chat-scoped pending setup"
            );
        }
        let (db_path, aes_key) = match (
            &self.credentials.credential_db_path,
            &self.credentials.credential_aes_key,
        ) {
            (Some(p), Some(k)) => (p.clone(), *k),
            _ => return false,
        };
        let Ok(conn) = crate::encrypted_sqlite::open(&db_path) else {
            return false;
        };
        let Ok(store) = crate::credential_store::CredentialStore::open(conn, aes_key) else {
            return false;
        };
        let pending_service = crate::google_auth::pending_cred_service(chat_id);
        store
            .get(&pending_service, "client_id")
            .ok()
            .flatten()
            .is_some()
    }

    /// Load persisted bot commands for Telegram startup, falling back to defaults.
    pub async fn get_startup_bot_commands(&self) -> Vec<teloxide::types::BotCommand> {
        // Hold menu_lock throughout startup so a concurrent menu sync cannot
        // race with our cleanup + save.
        let _guard = self.menu_lock.lock().await;

        // Sync skill-sourced menu commands: ensure all loaded skills with menu_command
        // are registered in custom commands, and remove entries for unloaded skills.
        let mut custom = self.load_custom_commands().await;
        let loaded = self.sandbox.loaded_skills();
        let core_names = crate::telegram::core_command_names();

        // ── Phase 1a: One-time cleanup — remove duplicate panel + stale custom command ──
        // Delete user-sourced "list_all_tasks" (was manually created, causes LLM fabrication)
        let before_cleanup = custom.len();
        custom.retain(|c| !(c.source.is_user() && c.command == "list_all_tasks"));
        if custom.len() < before_cleanup {
            tracing::info!("Startup cleanup: removed stale 'list_all_tasks' user command");
        }

        // Add menu commands from loaded skills that aren't already registered
        for cap in &loaded {
            if let Some(ref mc) = cap.menu_command {
                if core_names.contains(mc.as_str()) {
                    continue;
                }
                let expected_source = CommandSource::Skill(cap.skill_name.clone());
                let already = custom
                    .iter()
                    .any(|c| c.command == *mc && c.source == expected_source);
                if !already {
                    custom.push(CustomCommand::new(
                        mc.as_str(),
                        cap.menu_description.as_deref().unwrap_or(&cap.description),
                        CommandSource::Skill(cap.skill_name.clone()),
                    ));
                    tracing::info!(skill = %cap.skill_name, command = %mc, "Startup: registered skill menu command");
                }
            }
        }

        // Remove skill-sourced entries for skills that are no longer loaded
        let loaded_names: std::collections::HashSet<&str> =
            loaded.iter().map(|s| s.skill_name.as_str()).collect();
        custom.retain(|c| match &c.source {
            CommandSource::Skill(name) => loaded_names.contains(name.as_str()),
            _ => true,
        });

        self.save_custom_commands(&custom).await;

        // Use adaptive ordering: most-used commands float to the top.
        // At startup we use a generic user_id — per-user menus aren't supported
        // by Telegram's setMyCommands (it's global), but we use the aggregate.
        let usage = self.memory.get_command_usage_stats("__all__").await;
        let full_menu = self.merge_core_and_custom_ranked(&custom, &usage);
        let parsed: Vec<teloxide::types::BotCommand> = full_menu
            .iter()
            .filter_map(|c| {
                let name = c.get("command").and_then(|v| v.as_str())?;
                let desc = c.get("description").and_then(|v| v.as_str())?;
                Some(teloxide::types::BotCommand::new(name, desc))
            })
            .collect();
        if !parsed.is_empty() {
            tracing::info!(
                count = parsed.len(),
                "Built startup menu (core + custom, adaptive order)"
            );
            return parsed;
        }
        crate::telegram::core_commands()
    }

    /// Returns the currently active LLM model, reading from runtime settings.
    /// Empty `llm_model` means the provider default (`codex_model` /
    /// `xai_subscription_model`); OpenRouter with an empty model is handled by
    /// callers that must surface `console.model.choose`.
    pub(crate) fn active_model(&self) -> String {
        let stored = self
            .settings
            .get("llm_model")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| self.model.clone());
        let provider = self
            .settings
            .get("llm_provider")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "codex_oauth".to_string());
        crate::model_catalog::resolve_effective_llm_model(&self.settings, &stored, &provider)
    }

    /// Returns the configured fallback LLM model.
    pub(super) fn fallback_model(&self) -> String {
        self.settings
            .get("fallback_model")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default()
    }

    /// Whether the user has authorized automatic fallback to backup model.
    pub(super) fn fallback_authorized(&self) -> bool {
        self.settings
            .get("fallback_model_authorized")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Build a ChatCompletion request from components.
    pub(super) fn build_chat_request(
        model: &str,
        messages: Vec<ChatCompletionRequestMessage>,
        tools: Option<Vec<ChatCompletionTools>>,
    ) -> Result<CreateChatCompletionRequest> {
        let mut builder = CreateChatCompletionRequestArgs::default();
        builder.model(model).messages(messages);
        if let Some(t) = tools {
            builder.tools(t).parallel_tool_calls(true);
        }
        Ok(builder.build()?)
    }

    pub(super) fn task_class_for_llm_label(label: &str) -> LlmTaskClass {
        if label == "react_round" || label == "fallback_drill" || label.starts_with("subcall:") {
            LlmTaskClass::ForegroundUserRequest
        } else if label.starts_with("agent_") {
            LlmTaskClass::CodingAgentForeground
        } else if label.contains("summar") {
            LlmTaskClass::Summarization
        } else if label.contains("review") {
            LlmTaskClass::ReviewGuard
        } else if label.contains("learn") || label.contains("extract") {
            LlmTaskClass::LearningExtraction
        } else if label.contains("research") {
            LlmTaskClass::ScheduledResearch
        } else if label.contains("heartbeat") {
            LlmTaskClass::BackgroundHeartbeat
        } else if label.contains("reflection") || label.contains("narrative") {
            LlmTaskClass::MemoryConsolidation
        } else {
            LlmTaskClass::OtherBackground
        }
    }

    pub(super) fn route_chat_for_legacy_path(
        &self,
        model: &str,
        label: &str,
        chat_id: Option<i64>,
    ) -> Result<crate::llm::RouteDecision> {
        self.route_chat_for_legacy_path_with_effort(model, label, chat_id, None)
    }

    pub(super) fn route_chat_for_legacy_path_with_effort(
        &self,
        model: &str,
        label: &str,
        chat_id: Option<i64>,
        reasoning_effort: Option<&str>,
    ) -> Result<crate::llm::RouteDecision> {
        let task_class = Self::task_class_for_llm_label(label);
        self.route_chat_for_task_class_with_effort(
            model,
            label,
            chat_id,
            task_class,
            true,
            reasoning_effort,
        )
    }

    pub(crate) fn route_chat_for_task_class(
        &self,
        model: &str,
        label: &str,
        chat_id: Option<i64>,
        task_class: LlmTaskClass,
        legacy_default: bool,
    ) -> Result<crate::llm::RouteDecision> {
        self.route_chat_for_task_class_with_effort(
            model,
            label,
            chat_id,
            task_class,
            legacy_default,
            None,
        )
    }

    pub(super) fn route_chat_for_task_class_with_effort(
        &self,
        model: &str,
        label: &str,
        chat_id: Option<i64>,
        task_class: LlmTaskClass,
        legacy_default: bool,
        reasoning_effort: Option<&str>,
    ) -> Result<crate::llm::RouteDecision> {
        let config = if legacy_default {
            LlmRouterConfig::from_runtime_settings_for_legacy_chat_path(&self.settings)
        } else {
            LlmRouterConfig::from_runtime_settings(&self.settings)
        }
        .map_err(anyhow::Error::new)?;
        let backend = crate::llm::OpenAiCompatBackend::new(self.llm_client.clone(), model);
        let mut router = LlmRouter::new(Arc::new(backend), config);
        if self.codex_enabled() {
            router = router.with_codex(Arc::new(CodexOAuthBackend::auth_required(
                self.codex_model(model),
            )));
        }
        if let Some(xai_backend) = self.xai_subscription_backend(model)? {
            router = router.with_xai_subscription(Arc::new(xai_backend));
        }
        let request = AegisChatRequest {
            model: model.to_string(),
            messages: Vec::new(),
            temperature: None,
            max_tokens: None,
            tools: Vec::new(),
            tool_choice: None,
            metadata: AegisRequestMetadata {
                task_class,
                request_id: None,
                chat_id,
                user_visible: task_class.is_foreground(),
                reasoning_effort: reasoning_effort.map(str::to_string),
            },
        };
        let decision = router.route_chat(&request).map_err(anyhow::Error::new)?;
        if decision.is_fallback {
            crate::behavior_notice::record(
                "provider_fallback",
                &format!(
                    "首选服务不可用，已按设置切换到 {}；能力可能不同，付费 API 服务会独立计费。",
                    decision.provider
                ),
                &format!(
                    "The preferred service is unavailable. Switching to {} as configured; capabilities may differ and paid API services bill separately.",
                    decision.provider
                ),
            );
        }
        tracing::info!(
            label,
            provider = %decision.provider,
            model = %decision.model,
            task_class = ?task_class,
            chat_id,
            effort = reasoning_effort.unwrap_or("default"),
            is_fallback = decision.is_fallback,
            paid_api_provider = decision.provider.is_paid_api(),
            paid_api_fallback_allowed = self
                .settings
                .get("paid_api_fallback_enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            "[LLM Route]"
        );
        Ok(decision)
    }

    pub(crate) async fn create_chat_completion(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse> {
        self.create_chat_completion_with_policy(request, true, None)
            .await
    }

    pub(super) async fn create_chat_completion_with_policy(
        &self,
        request: CreateChatCompletionRequest,
        allow_codex_cli_adapter: bool,
        reasoning_effort: Option<&str>,
    ) -> Result<CreateChatCompletionResponse> {
        let raw_plan = raw_multimodal_request_plan(&request);
        let needs_openrouter_compat = self.openrouter_response_needs_compat(&request.model);
        let provider = self.provider_for_model_hint(&request.model)?;
        if allow_codex_cli_adapter
            && let Some(response) = self
                .try_create_codex_oauth_chat_completion(&request, provider)
                .await?
        {
            return Ok(response);
        }
        if let Some(response) = self
            .try_create_xai_subscription_chat_completion(&request, provider, reasoning_effort)
            .await?
        {
            return Ok(response);
        }
        let stream_owner = crate::progress_router::owner_reply_stream_enabled()
            && crate::progress_router::current_turn_id().is_some();
        let use_llm_client = !raw_plan.needs_raw_http() && !needs_openrouter_compat;
        if use_llm_client && !stream_owner {
            return self.create_via_llm_client(request).await;
        }

        let mut body = serde_json::to_value(&request)
            .context("failed to serialize raw chat completion request")?;
        if raw_plan.has_video_part {
            normalize_openrouter_video_parts(&mut body);
        }
        if raw_plan.has_pdf_part {
            ensure_openrouter_pdf_parser_plugin(&mut body);
        }
        if stream_owner && let Some(obj) = body.as_object_mut() {
            obj.insert("stream".to_string(), serde_json::json!(true));
        }

        let url = format!("{}/chat/completions", self.api_base);
        let resp = self
            .post_chat_completion(&url, &body, stream_owner)
            .await
            .context("raw chat completion HTTP request failed")?;
        if stream_owner {
            match self.finish_owner_chat_stream(resp).await {
                Ok(Some(response)) => return Ok(response),
                Ok(None) if use_llm_client => {
                    tracing::info!(
                        "[LLM Stream] provider rejected stream; this turn stays blocking"
                    );
                    return self.create_via_llm_client(request).await;
                }
                Ok(None) => {
                    tracing::info!(
                        "[LLM Stream] provider rejected stream; retrying this raw turn blocking"
                    );
                    if let Some(obj) = body.as_object_mut() {
                        obj.remove("stream");
                    }
                    let resp = self
                        .post_chat_completion(&url, &body, false)
                        .await
                        .context("raw chat completion HTTP request failed")?;
                    return self.read_blocking_chat_completion(resp).await;
                }
                Err(error) => {
                    self.emit_assistant_draft_reset();
                    return Err(error);
                }
            }
        }
        self.read_blocking_chat_completion(resp).await
    }

    async fn create_via_llm_client(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse> {
        self.llm_client
            .chat()
            .create(request)
            .await
            .context("chat completion API call failed")
    }

    async fn post_chat_completion(
        &self,
        url: &str,
        body: &serde_json::Value,
        stream_owner: bool,
    ) -> Result<reqwest::Response> {
        self.http_client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header(
                "Accept",
                if stream_owner {
                    "text/event-stream, application/json"
                } else {
                    "application/json"
                },
            )
            .json(body)
            .send()
            .await
            .context("chat completion HTTP request failed")
    }

    async fn read_blocking_chat_completion(
        &self,
        resp: reqwest::Response,
    ) -> Result<CreateChatCompletionResponse> {
        let status = resp.status();
        let text = resp
            .text()
            .await
            .context("raw chat completion response read failed")?;
        if !status.is_success() {
            anyhow::bail!(
                "raw chat completion returned {}: {}",
                status,
                truncate_str(&text, TRUNCATE_PREVIEW_MEDIUM)
            );
        }
        parse_chat_completion_response(&text).context("raw chat completion response parse failed")
    }

    /// `Ok(None)` means the provider refused `stream` before any tokens.
    pub(super) async fn finish_owner_chat_stream(
        &self,
        resp: reqwest::Response,
    ) -> Result<Option<CreateChatCompletionResponse>> {
        match crate::chat_stream::read_openai_chat_response(resp, |delta| {
            self.emit_assistant_delta(delta);
        })
        .await
        {
            Ok(crate::chat_stream::ChatStreamRead::Completed(value)) => {
                let text = serde_json::to_string(&value)
                    .context("failed to serialize streamed chat completion")?;
                parse_chat_completion_response(&text)
                    .context("streamed chat completion response parse failed")
                    .map(Some)
            }
            Ok(crate::chat_stream::ChatStreamRead::Rejected { status, body }) => {
                tracing::info!(
                    status,
                    body = %truncate_str(&body, TRUNCATE_PREVIEW_MEDIUM),
                    "[LLM Stream] provider rejected the stream flag"
                );
                Ok(None)
            }
            Err(error) => Err(anyhow::Error::new(error).context("owner chat stream failed")),
        }
    }

    /// The provider that owns `model`: the cached model catalog first, then
    /// the model family (grok-* is the Grok subscription), else the selected
    /// provider. Used so a fallback model reaches its own adapter instead of
    /// the selected provider's (2026-09-12: grok-4.6 was served by Codex).
    pub(super) fn provider_for_model_hint(&self, model: &str) -> Result<LlmProviderKind> {
        let selected = self.selected_llm_provider()?;
        let owner = self.provider_owning_model(model, selected);
        if owner == selected || self.provider_adapter_enabled(owner) {
            return Ok(owner);
        }
        tracing::warn!(
            model,
            owner = %owner,
            selected = %selected,
            "[LLM Route] model belongs to a provider that is not connected; serving it through the selected provider"
        );
        Ok(selected)
    }

    /// The provider that owns `model` by catalog or model family, regardless
    /// of whether that provider is connected (used to judge the fallback drill).
    pub(super) fn provider_owning_model(
        &self,
        model: &str,
        selected: LlmProviderKind,
    ) -> LlmProviderKind {
        let catalog_provider = crate::model_catalog::cached_model_catalog()
            .as_ref()
            .and_then(|catalog| catalog.provider_for_model(model).map(str::to_owned));
        provider_hint_for_model(model, catalog_provider.as_deref(), selected)
    }

    /// Whether the adapter for `kind` can serve a request right now.
    pub(super) fn provider_adapter_enabled(&self, kind: LlmProviderKind) -> bool {
        match kind {
            LlmProviderKind::CodexOAuth => self.codex_enabled(),
            LlmProviderKind::XaiSubscription => {
                self.xai_subscription_enabled()
                    && matches!(self.load_xai_subscription_credential(), Ok(Some(_)))
            }
            _ => true,
        }
    }

    /// Extra Codex app-server models to try after the requested one, from the
    /// `codex_retry_models` setting (comma-separated; empty by default — the
    /// old hardcoded gpt-5.4 / gpt-5.4-mini retries are rejected by ChatGPT
    /// accounts and only added two failed calls per outage).
    pub(super) fn codex_retry_models(&self) -> Vec<String> {
        self.settings
            .get("codex_retry_models")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
            .split(',')
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect()
    }

    pub(super) fn selected_llm_provider(&self) -> Result<LlmProviderKind> {
        let provider = self
            .settings
            .get_if_stored("llm_provider")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "openai_compat".to_string())
            .parse::<LlmProviderKind>()
            .map_err(anyhow::Error::new)?;
        Ok(provider)
    }

    pub(super) fn codex_enabled(&self) -> bool {
        self.settings
            .get("codex_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    pub(super) fn codex_model(&self, fallback: &str) -> String {
        self.settings
            .get("codex_model")
            .and_then(|v| v.as_str().map(str::to_owned))
            .map(|value| normalize_codex_cli_model(&value))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| fallback.to_string())
    }

    pub(super) fn codex_reasoning_effort(&self) -> Option<String> {
        self.settings
            .get("codex_reasoning_effort")
            .and_then(|v| v.as_str().map(str::to_owned))
            .map(|value| value.trim().to_string())
            .filter(|value| matches!(value.as_str(), "low" | "medium" | "high" | "xhigh"))
    }

    pub(super) fn codex_cli_binary(&self) -> String {
        self.settings
            .get("codex_cli_binary")
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "/usr/bin/codex".to_string())
    }

    pub(super) fn codex_cli_home(&self) -> String {
        self.settings
            .get("codex_cli_home")
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(crate::settings::default_codex_home)
    }

    pub(super) fn codex_cli_workdir(&self) -> String {
        self.settings
            .get("codex_cli_workdir")
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "/tmp".to_string())
    }

    pub(super) fn codex_cli_timeout(&self) -> Duration {
        let secs = self
            .settings
            .get("codex_cli_timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(90)
            .clamp(15, 300);
        Duration::from_secs(secs)
    }

    pub(super) fn xai_subscription_enabled(&self) -> bool {
        self.settings
            .get("xai_subscription_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// `xai_subscription_reasoning_effort` setting (low|medium|high); empty = omit.
    pub(super) fn xai_subscription_reasoning_effort(&self) -> Option<String> {
        self.settings
            .get("xai_subscription_reasoning_effort")
            .and_then(|v| v.as_str().map(str::trim).map(str::to_owned))
            .filter(|value| !value.is_empty())
    }

    /// Casual/short-message reasoning effort (default `low`).
    pub(super) fn chat_reasoning_effort_casual(&self) -> String {
        self.settings
            .get("chat_reasoning_effort_casual")
            .and_then(|v| v.as_str().map(str::trim).map(str::to_owned))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "low".to_string())
    }

    /// Prompt history window in messages (default 24, clamped 6–200).
    pub(super) fn chat_history_window_messages(&self) -> usize {
        self.settings
            .get("chat_history_window_messages")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
            })
            .unwrap_or(24)
            .clamp(6, 200) as usize
    }

    pub(super) fn xai_subscription_model(&self, fallback: &str) -> String {
        self.settings
            .get("xai_subscription_model")
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| fallback.to_string())
    }

    pub(super) fn xai_subscription_api_base(&self) -> String {
        self.settings
            .get("xai_subscription_api_base")
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "https://api.x.ai/v1".to_string())
    }

    pub(super) fn load_xai_subscription_credential(
        &self,
    ) -> Result<Option<crate::llm::XaiSubscriptionCredential>> {
        let (db_path, aes_key) = match (
            &self.credentials.credential_db_path,
            &self.credentials.credential_aes_key,
        ) {
            (Some(path), Some(key)) => (path, key),
            _ => return Ok(None),
        };
        let store = crate::credential_store::CredentialStore::open(
            crate::encrypted_sqlite::open(db_path)?,
            *aes_key,
        )?;
        XaiSubscriptionAuthManager::load_credential(&store).map_err(anyhow::Error::new)
    }

    pub(super) fn store_xai_subscription_credential(
        &self,
        credential: &crate::llm::XaiSubscriptionCredential,
    ) -> Result<()> {
        let (db_path, aes_key) = match (
            &self.credentials.credential_db_path,
            &self.credentials.credential_aes_key,
        ) {
            (Some(path), Some(key)) => (path, key),
            _ => anyhow::bail!("credential store is not configured"),
        };
        let store = crate::credential_store::CredentialStore::open(
            crate::encrypted_sqlite::open(db_path)?,
            *aes_key,
        )?;
        XaiSubscriptionAuthManager::store_credential(&store, credential).map_err(anyhow::Error::new)
    }
}

/// Pure provider resolution for a model id: catalog answer when it parses,
/// then the model family, else the selected provider.
pub(crate) fn provider_hint_for_model(
    model: &str,
    catalog_provider: Option<&str>,
    selected: LlmProviderKind,
) -> LlmProviderKind {
    if let Some(kind) = catalog_provider.and_then(|id| id.parse::<LlmProviderKind>().ok()) {
        return kind;
    }
    let lower = model.trim().to_ascii_lowercase();
    if lower.starts_with("grok") {
        return LlmProviderKind::XaiSubscription;
    }
    selected
}
