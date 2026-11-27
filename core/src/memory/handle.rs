use super::*;

impl MemoryHandle {
    /// Spawn the memory actor. Returns a handle for sending commands.
    ///
    /// `max_db_size` controls the maximum database file size in bytes.
    /// When exceeded, L2 writes are rejected with a warning.
    /// Pass `0` to use the default (20 GB).
    pub fn spawn(db_path: &Path) -> Result<Self> {
        Self::spawn_with_limit(db_path, DEFAULT_MAX_DB_SIZE)
    }

    /// Spawn with a custom database size limit.
    pub fn spawn_with_limit(db_path: &Path, max_db_size: u64) -> Result<Self> {
        Self::spawn_with_config(
            db_path,
            MemoryConfig {
                max_db_size: if max_db_size == 0 {
                    DEFAULT_MAX_DB_SIZE
                } else {
                    max_db_size
                },
                ..Default::default()
            },
        )
    }

    /// Spawn with full configuration.
    pub fn spawn_with_config(db_path: &Path, config: MemoryConfig) -> Result<Self> {
        let max_db_size = config.max_db_size;
        let conn = crate::encrypted_sqlite::open(db_path)
            .with_context(|| format!("opening SQLite at {}", db_path.display()))?;

        // Enable WAL mode for concurrent reads during writes, and set busy_timeout
        // to wait up to 5s instead of immediately returning SQLITE_BUSY.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;",
        )
        .context("setting SQLite pragmas")?;

        crate::memory_migrations::run_all_migrations(&conn)?;

        let (tx, rx) = mpsc::channel(256);
        let (cb_tx, cb_rx) = mpsc::channel(32);

        let l1_max_size = config.l1_max_size;
        let db_path_str = db_path.to_string_lossy().to_string();
        let db_path_for_handle = db_path_str.clone();

        // Spawn the actor on a blocking thread (SQLite is sync).
        // HNSW index is passed per-query via MemoryCommand, not here.
        let handle = tokio::task::spawn_blocking(move || {
            memory_actor_loop(conn, rx, max_db_size, l1_max_size, db_path_str, cb_tx);
        });

        // Monitor actor for panics — log loudly if it crashes unexpectedly.
        let monitor_handle = handle.abort_handle();
        tokio::spawn(async move {
            // AbortHandle doesn't have a .finished() method, so we poll is_finished
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                if monitor_handle.is_finished() {
                    tracing::error!(
                        "[MemoryActor] Actor thread terminated unexpectedly! \
                         All subsequent DB operations will fail. Restart required."
                    );
                    break;
                }
            }
        });

        Ok(Self {
            tx,
            max_db_size,
            hnsw: None,
            circuit_breaker_rx: std::sync::Arc::new(std::sync::Mutex::new(Some(cb_rx))),
            actor_handle: Some(std::sync::Arc::new(std::sync::Mutex::new(Some(handle)))),
            db_path: db_path_for_handle,
            heirloom_bridge: None,
        })
    }

    /// Phase E (W17): install a `HeirloomBridge` so every shim-covered method
    /// ALSO emits a heirloom event after its legacy SQLite write. `None`
    /// bridge = pre-Phase-E behavior (default in tests). Soft-fail per D1.
    pub fn with_heirloom_bridge(
        mut self,
        bridge: std::sync::Arc<crate::memory_heirloom_bridge::HeirloomBridge>,
    ) -> Self {
        self.heirloom_bridge = Some(bridge);
        self
    }

    /// Path to the underlying SQLite database as passed to [`Self::spawn`]
    /// (or `":memory:"` for in-process tests). Exposed for features that
    /// open their own raw rusqlite connection — WAL mode allows concurrent
    /// reads so this is safe, but writes must still be serialized through
    /// the actor OR run only during single-flight windows.
    pub fn db_path(&self) -> &str {
        &self.db_path
    }

    /// Spawn with an in-memory database (for testing).
    pub fn spawn_in_memory() -> Result<Self> {
        Self::spawn(Path::new(":memory:"))
    }
    /// Check the current database size against the configured limit.
    pub async fn check_db_size(&self) -> Result<DbSizeStatus> {
        mem_send!(self, CheckDbSize {})
    }

    // ── Self-audit query methods ──────────────────────────────────────

    /// Count heartbeat no-op ratio in the last N hours.
    pub async fn audit_heartbeat_noop_ratio(&self, hours: u32) -> Result<(i64, i64)> {
        mem_send!(self, AuditHeartbeatNoopRatio { hours: hours })?
    }

    /// Count heartbeat no-op rows in the last N hours.
    pub async fn audit_recent_heartbeat_noop_count(&self, hours: u32) -> Result<i64> {
        mem_send!(self, AuditRecentHeartbeatNoopCount { hours: hours })?
    }

    /// Test-only: backdate `heartbeat_audit_log.timestamp` for rows matching
    /// `outcome` (e.g. `"noop"`) by `hours_ago` hours. Serialized through the
    /// memory actor so parallel lib-tests never dual-write the same SQLite
    /// file. Returns rows updated.
    #[cfg(test)]
    pub async fn test_backdate_heartbeat_audit_by_outcome(
        &self,
        outcome: &str,
        hours_ago: i64,
    ) -> Result<u64> {
        mem_send!(
            self,
            TestBackdateHeartbeatAuditByOutcome {
                outcome: outcome.to_string(),
                hours_ago: hours_ago,
            }
        )?
    }

    /// Count tool blocks from heartbeat_audit_log in the last N hours.
    pub async fn audit_recent_tool_blocks(&self, hours: u32) -> Result<Vec<(String, i64)>> {
        mem_send!(self, AuditRecentToolBlocks { hours: hours })?
    }

    /// Count LLM call log entries in the last N hours.
    pub async fn audit_recent_llm_calls(&self, hours: u32) -> Result<i64> {
        mem_send!(self, AuditRecentLlmCalls { hours: hours })?
    }

    /// Count heartbeat audit entries in the last N hours.
    pub async fn audit_recent_heartbeat_count(&self, hours: u32) -> Result<i64> {
        mem_send!(self, AuditRecentHeartbeatCount { hours: hours })?
    }

    /// Count heartbeat audit entries that actually entered the LLM path.
    pub async fn audit_recent_llm_eligible_heartbeat_count(&self, hours: u32) -> Result<i64> {
        mem_send!(self, AuditRecentLlmEligibleHeartbeatCount { hours: hours })?
    }

    /// Get daily cache hit rates from llm_call_log for the last N days.
    pub async fn audit_daily_cache_rates(&self, days: u32) -> Result<Vec<(String, f64)>> {
        mem_send!(self, AuditDailyCacheRates { days: days })?
    }

    /// Get daily total token usage from llm_call_log for the last N days.
    pub async fn audit_daily_token_totals(&self, days: u32) -> Result<Vec<(String, i64)>> {
        mem_send!(self, AuditDailyTokenTotals { days: days })?
    }

    // ── G1: Relationship State ────────────────────────────────────────

    /// Get the relationship state for a user (returns default if no record).
    pub async fn get_relationship_state(&self, did: &str) -> Result<RelationshipState> {
        mem_send!(
            self,
            GetRelationshipState {
                did: did.to_string()
            }
        )?
    }

    /// Update relationship state after a user interaction.
    /// Increments interaction_count, updates last_interaction_at, last_topic, streak_days,
    /// and active hours histogram (G6).
    pub async fn update_relationship_interaction(
        &self,
        did: &str,
        topic: &str,
        user_tz: &str,
    ) -> Result<()> {
        mem_send!(
            self,
            UpdateRelationshipInteraction {
                did: did.to_string(),
                topic: topic.to_string(),
                user_tz: user_tz.to_string()
            }
        )?
    }

    /// Format a ~200 token relationship context string for LLM prompt injection.
    /// Includes active goals from the G5 goals table.
    pub async fn get_relationship_context(&self, did: &str) -> Result<String> {
        let state = self.get_relationship_state(did).await?;
        let goals = self
            .list_goals(did, Some(GoalStatus::Active))
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, did, "failed to load G5 goals for relationship context");
                vec![]
            });
        Ok(format_relationship_context(&state, &goals))
    }

    // ── Tier 0: User Profile ──────────────────────────────────────────

    /// Get all profile entries for a user, ordered by category then key.
    pub async fn get_user_profile(&self, did: &str) -> Result<Vec<ProfileEntry>> {
        self.get_user_profile_with_read_source(did, ProfileReadSource::from_env())
            .await
    }

    /// Get all profile entries using an explicit read source.
    ///
    /// The Heirloom path is opt-in and best-effort. Legacy rows are loaded
    /// first, then returned as fallback whenever the bridge is absent, the
    /// event log cannot be decoded, or the projection is empty.
    pub async fn get_user_profile_with_read_source(
        &self,
        did: &str,
        read_source: ProfileReadSource,
    ) -> Result<Vec<ProfileEntry>> {
        let legacy_entries = mem_send!(
            self,
            GetUserProfile {
                did: did.to_string()
            }
        )??;

        if read_source == ProfileReadSource::Legacy {
            return Ok(legacy_entries);
        }
        let projected = self.project_user_profile_from_heirloom(did);
        match projected {
            Ok(projected) if !projected.is_empty() => Ok(projected),
            _ => {
                crate::behavior_notice::record(
                    "heirloom_profile_fallback",
                    "所选 Heirloom 资料暂不可用，本次改用普通记忆；Heirloom 独有资料可能未包含。",
                    "The selected Heirloom profile is unavailable. Standard memory was used; Heirloom-only details may be missing.",
                );
                Ok(legacy_entries)
            }
        }
    }

    fn project_user_profile_from_heirloom(&self, did: &str) -> Result<Vec<ProfileEntry>> {
        let bridge = self
            .heirloom_bridge
            .as_ref()
            .context("Heirloom bridge is not installed")?;
        anyhow::ensure!(bridge.enabled(), "Heirloom bridge is disabled");
        if bridge.did().as_str() != did {
            anyhow::bail!(
                "Heirloom bridge DID {} does not match requested DID {did}",
                bridge.did().as_str()
            );
        }

        let events = load_profile_slot_events_from_heirloom(bridge, did)?;
        if events.is_empty() {
            return Ok(Vec::new());
        }

        let projection = crate::heirloom::projectors::ProfileReadProjector::new()
            .project(&events)
            .context("projecting Heirloom profile events")?;
        Ok(profile_entries_from_projection(did, &projection))
    }

    /// Get the formatted profile prompt for injection into the volatile prompt.
    /// Returns empty string if no profile entries exist.
    /// FU#2 Phase 5: extended formatter that can also emit a MISSING_PROFILE_SLOTS
    /// block listing canonical slots the LLM might want to ask about.
    pub async fn get_user_profile_prompt_ext(
        &self,
        did: &str,
        include_missing_hints: bool,
    ) -> String {
        match self.get_user_profile(did).await {
            Ok(entries) => format_user_profile_prompt_ext(&entries, include_missing_hints),
            Err(e) => {
                tracing::warn!(error = %e, did, "failed to load user profile");
                crate::behavior_notice::record(
                    "profile_unavailable",
                    "本次未能读取个人资料，回答可能缺少你的偏好和背景。",
                    "Your profile could not be read for this reply; preferences and background may be missing.",
                );
                String::new()
            }
        }
    }

    /// Upsert a profile entry. Enforces value max length and per-DID capacity limit.
    /// `source='manual'` entries take priority and cannot be overwritten by `source='auto'`.
    pub async fn upsert_profile_entry(&self, mut entry: ProfileEntry) -> Result<()> {
        // Enforce value length cap
        if entry.value.chars().count() > PROFILE_VALUE_MAX_CHARS {
            let boundary = entry.value.floor_char_boundary(PROFILE_VALUE_MAX_CHARS);
            entry.value.truncate(boundary);
        }
        // Legacy first (G6): delegate to the existing mem_send! path.
        let entry_for_bridge = entry.clone();
        mem_send!(self, UpsertProfileEntry { entry: entry })??;

        // Phase E Task 7: emit profile:slot_upserted from the shim. Uses REAL
        // ProfileEntry fields per memory.rs:218 (FIX 3): category, key, value,
        // confidence, source — NOT slot/sub_slot/value_json (plan draft typo).
        if let Some(bridge) = &self.heirloom_bridge {
            let env = crate::memory_heirloom_bridge::HeirloomBridge::build_profile_upsert_envelope(
                bridge.did(),
                &entry_for_bridge.category,
                &entry_for_bridge.key,
                &entry_for_bridge.value,
                entry_for_bridge.confidence,
                &entry_for_bridge.source,
            );
            bridge.try_emit(env);
        }
        Ok(())
    }

    /// Rolling summaries + compaction facts for recall (newest first).
    pub async fn list_ephemeral_summaries(
        &self,
        did: &str,
        limit: i64,
    ) -> Result<Vec<crate::memory::EphemeralSummary>> {
        mem_send!(
            self,
            ListEphemeralSummaries {
                did: did.to_string(),
                limit: limit
            }
        )?
    }

    /// Count ephemeral entries for a DID, optionally filtered by source.
    pub async fn count_ephemeral(&self, did: &str, source: Option<&str>) -> Result<i64> {
        mem_send!(
            self,
            CountEphemeral {
                did: did.to_string(),
                source: source.map(|s| s.to_string())
            }
        )?
    }

    /// Insert a row into `l2_ephemeral` (Tier 2). Applies the shared
    /// `l2_write_guard` so duplicates/near-duplicates are skipped.
    /// Returns the row id on insert, `-1` when skipped by the guard.
    pub async fn store_ephemeral(
        &self,
        did: &str,
        topic: &str,
        fact: &str,
        embedding: Option<&[f32]>,
        source: &str,
        ttl_days: i64,
    ) -> Result<i64> {
        if did.is_empty() {
            anyhow::bail!("store_ephemeral: agent_did must not be empty");
        }
        let embedding_bytes = embedding.map(|emb| {
            emb.iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<u8>>()
        });
        mem_send!(
            self,
            StoreEphemeral {
                did: did.to_string(),
                topic: topic.to_string(),
                fact: fact.to_string(),
                embedding: embedding_bytes,
                source: source.to_string(),
                ttl_days: ttl_days,
            }
        )?
    }

    /// Delete all `l2_ephemeral` rows where `expires_at < datetime('now')`.
    /// Returns the number of rows deleted.
    pub async fn cleanup_expired_ephemeral(&self) -> Result<usize> {
        mem_send!(self, CleanupExpiredEphemeral {})?
    }

    /// Delete a profile entry. Returns true if a row was deleted.
    pub async fn delete_profile_entry(&self, did: &str, category: &str, key: &str) -> Result<bool> {
        mem_send!(
            self,
            DeleteProfileEntry {
                did: did.to_string(),
                category: category.to_string(),
                key: key.to_string()
            }
        )?
    }

    // ── G5: Goal Tracking ──────────────────────────────────────────────

    /// Create a new goal, returning its ID.
    pub async fn create_goal(&self, did: &str, goal: &Goal) -> Result<i64> {
        mem_send!(
            self,
            CreateGoal {
                did: did.to_string(),
                goal: goal.clone()
            }
        )?
    }

    /// List goals for a user, optionally filtered by status.
    pub async fn list_goals(&self, did: &str, status: Option<GoalStatus>) -> Result<Vec<Goal>> {
        mem_send!(
            self,
            ListGoals {
                did: did.to_string(),
                status: status
            }
        )?
    }

    /// Get a single goal by ID (returns None if not found).
    pub async fn get_goal(&self, id: i64) -> Result<Option<Goal>> {
        mem_send!(self, GetGoal { id: id })?
    }

    /// Apply partial updates to a goal.
    pub async fn update_goal(&self, id: i64, updates: GoalUpdate) -> Result<()> {
        mem_send!(
            self,
            UpdateGoal {
                id: id,
                updates: updates
            }
        )?
    }

    /// Add a milestone to a goal.
    pub async fn add_milestone(&self, goal_id: i64, title: &str) -> Result<()> {
        mem_send!(
            self,
            AddMilestone {
                goal_id: goal_id,
                title: title.to_string()
            }
        )?
    }

    /// Mark a milestone as completed (by 0-based index).
    pub async fn complete_milestone(&self, goal_id: i64, milestone_idx: usize) -> Result<()> {
        mem_send!(
            self,
            CompleteMilestone {
                goal_id: goal_id,
                milestone_idx: milestone_idx
            }
        )?
    }

    /// G5: Link a fact to a goal (with similarity score).
    pub async fn link_fact_to_goal(
        &self,
        goal_id: i64,
        fact_id: i64,
        similarity: f32,
    ) -> Result<()> {
        mem_send!(
            self,
            LinkFactToGoal {
                goal_id: goal_id,
                fact_id: fact_id,
                similarity: similarity
            }
        )?
    }

    // ── Layer C: Companion Growth ────────────────────────────────────

    /// Get the singleton companion identity.
    pub async fn get_companion_identity(&self) -> Result<CompanionIdentity> {
        mem_send!(self, GetCompanionIdentity {})?
    }

    /// Set the companion's user-given name.
    pub async fn set_companion_name(&self, name: &str) -> Result<()> {
        mem_send!(
            self,
            SetCompanionName {
                name: name.to_string()
            }
        )?
    }

    /// Replace all personality weights at once.
    pub async fn update_personality_weights(&self, weights: PersonalityWeights) -> Result<()> {
        mem_send!(self, UpdatePersonalityWeights { weights: weights })?
    }

    /// Update free-text communication style notes.
    pub async fn update_communication_notes(&self, notes: &str) -> Result<()> {
        mem_send!(
            self,
            UpdateCommunicationNotes {
                notes: notes.to_string()
            }
        )?
    }

    /// Transition to a new growth stage.
    pub async fn update_growth_stage(&self, stage: GrowthStage) -> Result<()> {
        mem_send!(self, UpdateGrowthStage { stage: stage })?
    }

    /// List all growth dimensions.
    pub async fn list_growth_dimensions(&self) -> Result<Vec<GrowthDimension>> {
        mem_send!(self, ListGrowthDimensions {})?
    }

    /// Apply evidence to a growth dimension (depth update + log entry, one transaction).
    pub async fn apply_growth_evidence(
        &self,
        dimension_key: &str,
        delta: f64,
        detail: &str,
        metadata: serde_json::Value,
    ) -> Result<()> {
        mem_send!(
            self,
            ApplyGrowthEvidence {
                dimension_key: dimension_key.to_string(),
                delta: delta,
                detail: detail.to_string(),
                metadata: metadata
            }
        )?
    }

    /// Append a free-form event to the growth log.
    pub async fn log_growth_event(&self, event: NewGrowthEvent) -> Result<i64> {
        mem_send!(self, LogGrowthEvent { event: event })?
    }

    /// Query recent growth log entries, optionally filtered by event_type.
    pub async fn query_growth_log(
        &self,
        event_type: Option<&str>,
        limit: i64,
    ) -> Result<Vec<GrowthEvent>> {
        mem_send!(
            self,
            QueryGrowthLog {
                event_type: event_type.map(|s| s.to_string()),
                limit: limit
            }
        )?
    }

    /// Adjust a single personality trait by key.
    pub async fn adjust_personality_trait(&self, trait_key: &str, value: f32) -> Result<()> {
        mem_send!(
            self,
            AdjustPersonalityTrait {
                trait_key: trait_key.to_string(),
                value: value
            }
        )?
    }

    /// Load companion identity + growth dimensions + interaction count,
    /// then format into a ~150-200 token self-awareness string for prompt injection.
    pub async fn get_companion_context(&self, user_did: &str) -> Result<String> {
        let identity = self.get_companion_identity().await?;
        let dimensions = self.list_growth_dimensions().await.unwrap_or_default();
        let interaction_count = self
            .get_relationship_state(user_did)
            .await
            .map(|rs| rs.interaction_count)
            .unwrap_or(0);
        Ok(format_self_awareness(
            &identity,
            &dimensions,
            interaction_count,
        ))
    }

    // ── HNSW vector index ─────────────────────────────────────────────

    /// Attach an HNSW index for O(log N) vector search.
    /// Must be called before queries to enable fast search.
    pub fn set_hnsw_index(&mut self, index: std::sync::Arc<HnswIndex>) {
        self.hnsw = Some(index);
    }

    /// Get a reference to the HNSW index (if configured).
    pub fn hnsw_index(&self) -> Option<&std::sync::Arc<HnswIndex>> {
        self.hnsw.as_ref()
    }

    /// Force a full HNSW rebuild from DB irrespective of the deleted-ratio
    /// threshold. Used by the embedding-corpus consent mechanism after a
    /// successful [`full_corpus_reembed`] — every old-model vector has been
    /// NULL'd, and the index needs to start from scratch so stale entries
    /// don't contaminate future searches.
    ///
    /// [`full_corpus_reembed`]: crate::embedding_corpus::full_corpus_reembed
    ///
    /// Returns `Ok(0)` if no HNSW is attached (headless mode / test harness).
    pub async fn request_full_hnsw_rebuild(&self) -> Result<usize> {
        let Some(idx) = self.hnsw.clone() else {
            return Ok(0);
        };
        let (reply, reply_rx) = oneshot::channel();
        self.tx
            .send(MemoryCommand::RebuildHnsw { hnsw: idx, reply })
            .await
            .map_err(|_| anyhow::anyhow!("memory actor closed"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("memory actor dropped HNSW rebuild reply"))?
    }

    /// Count L2 facts that currently have a non-NULL embedding (excludes
    /// `l1_overflow` transient snapshots). Used by the embedding-corpus
    /// consent check at startup.
    pub async fn count_embedded_facts(&self) -> Result<i64> {
        mem_send!(self, CountEmbeddedFacts {})?
    }

    /// Trigger an HNSW rebuild if the deleted ratio exceeds the rebuild threshold.
    /// Runs asynchronously via the memory actor (which owns the DB connection).
    async fn maybe_rebuild_hnsw(&self, idx: &std::sync::Arc<crate::hnsw_index::HnswIndex>) {
        if idx.needs_rebuild() {
            tracing::info!(
                deleted = idx.deleted_count(),
                total = idx.len(),
                "[MemoryHandle] HNSW rebuild threshold exceeded, requesting rebuild"
            );
            let (reply, reply_rx) = oneshot::channel();
            if self
                .tx
                .send(MemoryCommand::RebuildHnsw {
                    hnsw: idx.clone(),
                    reply,
                })
                .await
                .is_ok()
            {
                match reply_rx.await {
                    Ok(Ok(n)) => tracing::info!(count = n, "[MemoryHandle] HNSW rebuild complete"),
                    Ok(Err(e)) => tracing::warn!(error = %e, "[MemoryHandle] HNSW rebuild failed"),
                    Err(_) => tracing::warn!("[MemoryHandle] HNSW rebuild reply dropped"),
                }
            }
        }
    }

    // ── L1: Recent conversation cache ───────────────────────────────────

    /// Push a message to the L1 cache for a given agent DID.
    pub async fn push_l1(&self, did: &str, msg: Message) -> Result<()> {
        // Phase E D7 / FIX 6 — reply-ack added so the bridge emit ordering
        // (G6: legacy first, bridge second) is enforceable at the method
        // level. Keeps the public signature unchanged; callers see zero
        // difference in Ok()-always observable semantics (actor internal
        // archive/overflow failures remain warn-and-continue per FIX R2-3).
        let role = msg.role.clone();
        let content = msg.content.clone();
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(MemoryCommand::PushL1 {
                did: did.to_string(),
                msg,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("memory actor closed"))?;
        // AWAIT the actor's ack BEFORE bridge emit (G6 legacy-first ordering).
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("memory actor dropped reply"))??;

        // Phase E Task 5: emit paired L1 event (user_stated OR ai:said).
        // Reaches here only after the legacy cache.push_back returned Ok (G6).
        if let Some(bridge) = &self.heirloom_bridge {
            let env = bridge.build_l1_envelope(&role, &content);
            bridge.try_emit(env);
        }
        Ok(())
    }
    /// Get all L1 cached messages for a given agent DID.
    pub async fn get_l1(&self, did: &str) -> Result<Vec<Message>> {
        mem_send!(
            self,
            GetL1 {
                did: did.to_string()
            }
        )
    }

    // ── L2: Persistent experience spine ─────────────────────────────────

    /// Store a fact in L2 with an optional embedding vector. Returns the row ID.
    /// Also inserts into the HNSW index if available and embedding is provided.
    pub async fn store_l2(
        &self,
        did: &str,
        topic: &str,
        fact: &str,
        embedding: Option<&[f32]>,
    ) -> Result<i64> {
        // Guard: reject empty DID — facts with empty DID are invisible to all queries
        if did.is_empty() {
            tracing::error!(
                topic = %topic,
                fact_preview = %fact.chars().take(80).collect::<String>(),
                "BUG: store_l2 called with empty DID — rejecting to prevent orphaned fact"
            );
            anyhow::bail!("store_l2: agent_did must not be empty");
        }
        let emb_clone = embedding.map(|e| e.to_vec());
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(MemoryCommand::StoreL2 {
                did: did.to_string(),
                topic: topic.to_string(),
                fact: fact.to_string(),
                embedding: embedding.map(EmbeddingClient::to_blob),
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("memory actor closed"))?;
        let row_id = reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("memory actor dropped reply"))??;

        // Insert into HNSW index (best-effort, non-blocking)
        if let (Some(idx), Some(emb)) = (&self.hnsw, &emb_clone) {
            idx.insert(row_id, emb);
        }

        // Phase E Task 3: legacy first (G6), then bridge tail. Soft-fail (D1).
        if let Some(bridge) = &self.heirloom_bridge {
            let env = crate::memory_heirloom_bridge::HeirloomBridge::build_learned_envelope(
                &crate::heirloom::types::Did::new(did),
                topic,
                fact,
            );
            bridge.try_emit(env);
        }

        Ok(row_id)
    }

    /// Store a fact in L2 with explicit confidence score and optional expiration.
    /// Used by the learning engine which extracts confidence from LLM output.
    pub async fn store_l2_with_meta(
        &self,
        did: &str,
        topic: &str,
        fact: &str,
        embedding: Option<&[f32]>,
        confidence: f64,
        expires_at: Option<&str>,
    ) -> Result<i64> {
        if did.is_empty() {
            anyhow::bail!("store_l2_with_meta: agent_did must not be empty");
        }
        let emb_clone = embedding.map(|e| e.to_vec());
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(MemoryCommand::StoreL2WithMeta {
                did: did.to_string(),
                topic: topic.to_string(),
                fact: fact.to_string(),
                embedding: embedding.map(EmbeddingClient::to_blob),
                confidence,
                expires_at: expires_at.map(|s| s.to_string()),
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("memory actor closed"))?;
        let row_id = reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("memory actor dropped reply"))??;

        if let (Some(idx), Some(emb)) = (&self.hnsw, &emb_clone) {
            idx.insert(row_id, emb);
        }

        // Phase E Task 3: legacy first (G6), then bridge tail. Soft-fail (D1).
        if let Some(bridge) = &self.heirloom_bridge {
            let env = crate::memory_heirloom_bridge::HeirloomBridge::build_learned_envelope(
                &crate::heirloom::types::Did::new(did),
                topic,
                fact,
            );
            bridge.try_emit(env);
        }

        Ok(row_id)
    }

    /// Store an L2 fact, rewriting `topic` under a `heartbeat:<kind>:` prefix
    /// when `origin` is a heartbeat tick (design §3.1 L1/L2 write discipline).
    ///
    /// User-turn callers keep using `store_l2()` unchanged. Heartbeat-internal
    /// writers (the Step 8 `write_memory` tool handler and any in-loop
    /// persisters) call this method with the
    /// appropriate [`MemoryWriteOrigin::HeartbeatTick`] so a user-fact
    /// `topic="x"` and a heartbeat-fact `topic="x"` land as distinct rows
    /// (`"x"` vs `"heartbeat:learned:x"`) and the heartbeat's inner life
    /// never crowds the user-centric retrieval surfaces.
    ///
    /// Idempotent for already-scoped topics: if the caller already passes
    /// `"heartbeat:…"` the topic is stored verbatim.
    pub async fn store_l2_with_origin(
        &self,
        did: &str,
        topic: &str,
        fact: &str,
        embedding: Option<&[f32]>,
        origin: MemoryWriteOrigin,
    ) -> Result<i64> {
        let effective_topic = apply_write_origin(topic, origin);
        let row_id = self
            .store_l2(did, &effective_topic, fact, embedding)
            .await?;

        // Phase E Task 4: emit observation:user_stated (user origin) or
        // ai:observation (heartbeat origin). BOTH this event AND
        // the learned:* event from the delegated store_l2 above appear —
        // double-event invariant per Q3.9 + Task 4.
        if let Some(bridge) = &self.heirloom_bridge {
            let origin_is_user = matches!(origin, MemoryWriteOrigin::UserTurn);
            let env = crate::memory_heirloom_bridge::HeirloomBridge::build_observation_envelope(
                &crate::heirloom::types::Did::new(did),
                // Use ORIGINAL topic, not effective_topic — the observation
                // captures what the fact was ABOUT, not the write-origin
                // rewrite applied to the l2_experience row.
                topic,
                fact,
                origin_is_user,
            );
            bridge.try_emit(env);
        }
        Ok(row_id)
    }

    /// Store an L2 belief with epistemic kind, source receipt, and optional
    /// `valid_until`. Unverified writes are capped. Contradictions keep both
    /// rows and the previous text.
    pub async fn store_l2_with_provenance(
        &self,
        write: crate::memory::L2ProvenanceWrite,
    ) -> Result<crate::memory::ProvenanceOutcome> {
        let embedding = write.embedding.clone();
        let outcome = mem_send!(self, StoreL2Provenance { write: write })??;
        if let (Some(idx), Some(emb)) = (&self.hnsw, embedding.as_ref())
            && outcome.id > 0
        {
            idx.insert(outcome.id, emb);
        }
        Ok(outcome)
    }

    /// Open contradiction queue for one DID. Both fact rows remain stored.
    pub async fn list_l2_contradictions(
        &self,
        did: &str,
    ) -> Result<Vec<crate::memory::L2Contradiction>> {
        mem_send!(
            self,
            ListL2Contradictions {
                did: did.to_string()
            }
        )?
    }

    /// Previous summaries for one fact, scoped to `did`.
    pub async fn list_l2_fact_history(
        &self,
        did: &str,
        fact_id: i64,
    ) -> Result<Vec<crate::memory::L2FactHistoryEntry>> {
        mem_send!(
            self,
            ListL2FactHistory {
                did: did.to_string(),
                fact_id: fact_id,
            }
        )?
    }

    /// Provenance columns for one L2 row owned by `did`.
    pub async fn get_l2_provenance(
        &self,
        did: &str,
        id: i64,
    ) -> Result<Option<crate::memory::L2ProvenanceView>> {
        mem_send!(
            self,
            GetL2Provenance {
                did: did.to_string(),
                id: id,
            }
        )?
    }

    /// Fold retained `episode` rows on `topic` into one belief. Episodes stay.
    pub async fn consolidate_l2_observations(
        &self,
        did: &str,
        topic: &str,
    ) -> Result<crate::memory::ConsolidationOutcome> {
        mem_send!(
            self,
            ConsolidateL2Observations {
                did: did.to_string(),
                topic: topic.to_string(),
            }
        )?
    }

    /// Semantic search with the pre-MMR lexical, temporal, and topic-token fusion.
    /// MMR λ stays 0.7. This does not switch the ranker to reciprocal rank fusion.
    pub async fn query_l2_semantic_hybrid(
        &self,
        did: &str,
        query_embedding: &[f32],
        query_text: &str,
    ) -> Result<Vec<Experience>> {
        mem_send!(
            self,
            QueryL2Semantic {
                did: did.to_string(),
                query_embedding: EmbeddingClient::to_blob(query_embedding),
                hnsw: self.hnsw.clone(),
                query_text: Some(query_text.to_string()),
            }
        )
    }

    /// Semantic search over L2 facts using vector cosine similarity.
    ///
    /// Returns up to 5 most relevant facts above the similarity threshold.
    pub async fn query_l2_semantic(
        &self,
        did: &str,
        query_embedding: &[f32],
    ) -> Result<Vec<Experience>> {
        mem_send!(
            self,
            QueryL2Semantic {
                did: did.to_string(),
                query_embedding: EmbeddingClient::to_blob(query_embedding),
                hnsw: self.hnsw.clone(),
                query_text: None,
            }
        )
    }
    /// List all active scheduled tasks across all users (admin view).
    pub async fn list_all_active_scheduled_tasks(&self, limit: i64) -> Vec<Task> {
        mem_send_or!(
            self,
            ListAllScheduledTasks { limit: limit },
            Vec::new(),
            "list_all_active_scheduled_tasks"
        )
    }

    // ── Reminders (deprecated — use unified tasks table) ─────────────────

    /// Store a new reminder. Returns the row ID.
    #[deprecated(
        note = "Use store_task() with trigger_type='time' and execution_mode='notify' instead"
    )]
    pub async fn store_reminder(
        &self,
        chat_id: i64,
        message: &str,
        remind_at_unix: i64,
    ) -> Result<i64> {
        mem_send!(
            self,
            StoreReminder {
                chat_id: chat_id,
                message: message.to_string(),
                remind_at_unix: remind_at_unix
            }
        )?
    }
    /// Get all reminders that are due (remind_at_unix <= now, not yet fired).
    #[deprecated(
        note = "Use get_due_time_intents() instead — reminders are now in the unified tasks table"
    )]
    pub async fn get_due_reminders(&self, now_unix: i64) -> Result<Vec<Reminder>> {
        mem_send!(self, GetDueReminders { now_unix: now_unix })
    }
    /// Mark a reminder as fired.
    #[deprecated(
        note = "Use mark_intent_fired() instead — reminders are now in the unified tasks table"
    )]
    pub async fn mark_reminder_fired(&self, id: i64) -> Result<()> {
        mem_send!(self, MarkReminderFired { id: id })?
    }

    // ── L3: Full-text archive ──────────────────────────────────────────────
    /// Full-text search over L3 archive using SQLite FTS5.
    /// Returns up to 10 matching entries, ranked by relevance.
    /// No embedding API needed — purely local.
    pub async fn query_l3_fulltext(&self, did: &str, query: &str) -> Result<Vec<L3Entry>> {
        mem_send!(
            self,
            QueryL3Fulltext {
                did: did.to_string(),
                query: query.to_string()
            }
        )
    }

    /// Recent archived turns for transcript projection.
    ///
    /// L1 is an in-memory short-term cache and may be full of restart/status
    /// notices after deploy churn. User-facing chat history should recover from
    /// the durable L3 append log instead of treating L1 as authoritative.
    pub async fn list_recent_l3(&self, did: &str, limit: usize) -> Result<Vec<L3Entry>> {
        mem_send!(
            self,
            ListRecentL3 {
                did: did.to_string(),
                limit: limit
            }
        )
    }

    /// Archive rows for the conversation that created a work item.
    pub async fn list_trigger_archive(
        &self,
        did: &str,
        created_at_unix: i64,
        task_id: Option<i64>,
    ) -> Result<TriggerArchive> {
        mem_send!(
            self,
            ListTriggerArchive {
                did: did.to_string(),
                created_at_unix: created_at_unix,
                task_id: task_id,
            }
        )
    }

    // ── Lifecycle ──────────────────────────────────────────────────────────
    /// Flush all L1 caches to SQLite for crash-safe persistence.
    /// Returns the number of DIDs flushed.
    pub async fn flush_l1(&self) -> Result<usize> {
        mem_send!(self, FlushL1ToDb {})?
    }
    /// Restore L1 caches from the l1_snapshot table (called at startup).
    /// Returns the number of DIDs restored.
    pub async fn restore_l1(&self) -> Result<usize> {
        mem_send!(self, RestoreL1FromDb {})?
    }
    /// Graceful shutdown: flush L1, WAL checkpoint, stop actor.
    /// After sending the Shutdown command, waits for the actor thread to exit.
    pub async fn shutdown(&self) -> Result<()> {
        if let Err(e) = mem_send!(self, Shutdown {}) {
            tracing::warn!(error = %e, "Shutdown command send/reply failed (actor may have already exited)");
        }
        // Wait for the actor thread to actually finish
        if let Some(ref arc) = self.actor_handle
            && let Some(handle) = arc.lock().ok().and_then(|mut g| g.take())
        {
            match tokio::time::timeout(std::time::Duration::from_secs(5), handle).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::error!("Memory actor thread panicked: {e}"),
                Err(_) => tracing::warn!("Memory actor thread did not exit within 5s"),
            }
        }
        Ok(())
    }

    /// Check if the memory actor is alive and responsive.
    /// Returns true if healthy, false if the actor is dead or stuck (5s timeout).
    pub async fn healthcheck(&self) -> bool {
        let (tx, rx) = oneshot::channel();
        if self
            .tx
            .send(MemoryCommand::Healthcheck { reply: tx })
            .await
            .is_err()
        {
            return false; // channel closed = actor dead
        }
        matches!(
            tokio::time::timeout(std::time::Duration::from_secs(5), rx).await,
            Ok(Ok(true))
        )
    }
    /// Get all L2 experiences for a DID.
    pub async fn get_all_l2(&self, did: &str) -> Result<Vec<Experience>> {
        mem_send!(
            self,
            GetAllL2 {
                did: did.to_string()
            }
        )
    }

    /// Get one L2 experience by DID and row ID without applying list limits.
    pub async fn get_l2_by_id(&self, did: &str, id: i64) -> Result<Option<Experience>> {
        mem_send!(
            self,
            GetL2ById {
                did: did.to_string(),
                id: id
            }
        )?
    }

    /// Mark one L2 fact as reviewed by DID and row ID.
    /// Returns `true` if a row was touched, `false` if no matching row found.
    pub async fn touch_l2_by_id(&self, did: &str, id: i64) -> Result<bool> {
        mem_send!(
            self,
            TouchL2ById {
                did: did.to_string(),
                id: id
            }
        )?
    }

    /// Delete a specific L2 fact by DID and row ID.
    /// Returns `true` if a row was deleted, `false` if no matching row found.
    pub async fn delete_l2(&self, did: &str, id: i64) -> Result<bool> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(MemoryCommand::DeleteL2 {
                did: did.to_string(),
                id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("memory actor closed"))?;
        let deleted = reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("memory actor dropped reply"))??;

        // Mark deleted in HNSW index (lazy prune)
        if deleted && let Some(ref idx) = self.hnsw {
            idx.mark_deleted(id);
            self.maybe_rebuild_hnsw(idx).await;
        }

        Ok(deleted)
    }

    /// Update a specific L2 fact by DID and row ID.
    /// Returns `true` if a row was updated, `false` if no matching row found.
    pub async fn update_l2(&self, did: &str, id: i64, fact: &str, confidence: f64) -> Result<bool> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(MemoryCommand::UpdateL2 {
                did: did.to_string(),
                id,
                fact: fact.to_string(),
                confidence,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("memory actor closed"))?;
        let updated = reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("memory actor dropped reply"))??;

        // The fact text changed, so any old HNSW vector for this row is stale.
        if updated && let Some(ref idx) = self.hnsw {
            idx.mark_deleted(id);
            self.maybe_rebuild_hnsw(idx).await;
        }

        Ok(updated)
    }

    /// Pin or unpin a specific L2 fact by DID and row ID.
    /// Returns `true` if a row was updated, `false` if no matching row found.
    pub async fn set_l2_pinned(&self, did: &str, id: i64, pinned: bool) -> Result<bool> {
        let updated = mem_send!(
            self,
            SetL2Pinned {
                did: did.to_string(),
                id: id,
                pinned: pinned
            }
        )??;
        Ok(updated)
    }
    /// Get all pending (unfired) reminders across all chat IDs.
    #[deprecated(note = "Use list_tasks() instead — reminders are now in the unified tasks table")]
    pub async fn get_pending_reminders(&self) -> Result<Vec<Reminder>> {
        mem_send!(self, GetPendingReminders {})
    }
    /// List pending (unfired) reminders for a specific chat.
    #[deprecated(note = "Use list_tasks() instead — reminders are now in the unified tasks table")]
    pub async fn list_reminders_for_chat(&self, chat_id: i64) -> Result<Vec<Reminder>> {
        mem_send!(self, ListRemindersForChat { chat_id: chat_id } resp)?
    }
    /// Cancel a pending reminder by ID and chat_id. Returns true if deleted.
    #[deprecated(note = "Use update_task() with status='cancelled' instead")]
    pub async fn cancel_reminder(&self, reminder_id: i64, chat_id: i64) -> Result<bool> {
        mem_send!(self, CancelReminder { reminder_id: reminder_id, chat_id: chat_id } resp)?
    }

    // ── L2 topic-prefix query (for proactive learning dedup) ──────────────

    /// Query L2 facts whose topic starts with the given prefix, returning
    /// (fact_summary, embedding) pairs. Used by the learning engine for
    /// embedding-based deduplication against existing learned facts.
    pub async fn query_l2_by_topic_prefix(
        &self,
        did: &str,
        prefix: &str,
    ) -> Result<Vec<(String, Vec<f32>)>> {
        mem_send!(
            self,
            QueryL2ByTopicPrefix {
                did: did.to_string(),
                prefix: prefix.to_string()
            }
        )
    }

    /// Like `query_l2_by_topic_prefix` but only returns facts created after `since`.
    pub async fn query_l2_by_topic_prefix_since(
        &self,
        did: &str,
        prefix: &str,
        since: &str,
    ) -> Result<Vec<(String, Vec<f32>)>> {
        mem_send!(
            self,
            QueryL2ByTopicPrefixSince {
                did: did.to_string(),
                prefix: prefix.to_string(),
                since: since.to_string()
            }
        )
    }

    /// Step 10 (design §5 gate 2): fetch a single L2 row by id, scoped to
    /// `did` and gated by a max-age freshness window.
    ///
    /// Returns `Some(L2RowBrief)` only when:
    /// 1. Row id exists, AND
    /// 2. `agent_did == did` (cross-tenant safety), AND
    /// 3. `created_at >= now - max_age`.
    ///
    /// Returns `None` otherwise. Does NOT re-embed — a cheap existence +
    /// freshness check used by `send_message` to verify the LLM's cited
    /// source actually resolves.
    pub async fn get_l2_by_id_if_fresh(
        &self,
        did: &str,
        row_id: i64,
        max_age: chrono::Duration,
    ) -> Result<Option<L2RowBrief>> {
        let since = (chrono::Utc::now() - max_age).to_rfc3339();
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(MemoryCommand::GetL2ByIdIfFresh {
                did: did.to_string(),
                row_id,
                since_rfc3339: since,
                reply: tx,
            })
            .await
            .map_err(|e| anyhow::anyhow!("memory actor dead: {e}"))?;
        rx.await
            .map_err(|e| anyhow::anyhow!("reply channel dropped: {e}"))
    }

    /// Like `query_l2_by_topic_prefix_since` but returns only fact text.
    /// Does NOT require embeddings — includes rows with NULL embedding.
    pub async fn query_l2_text_by_topic_prefix_since(
        &self,
        did: &str,
        prefix: &str,
        since: &str,
    ) -> Result<Vec<String>> {
        mem_send!(
            self,
            QueryL2TextByTopicPrefixSince {
                did: did.to_string(),
                prefix: prefix.to_string(),
                since: since.to_string()
            }
        )
    }

    // ── Monitoring intent queries ──────────────────────────────────────────

    /// Query L2 facts by exact topic, returning (fact_summary, Option<embedding>) pairs.
    /// Includes rows with NULL embedding (unlike `query_l2_by_topic_prefix`).
    pub async fn query_l2_by_topic(
        &self,
        did: &str,
        topic: &str,
    ) -> Result<Vec<(String, Option<Vec<f32>>)>> {
        mem_send!(
            self,
            QueryL2ByTopic {
                did: did.to_string(),
                topic: topic.to_string()
            }
        )
    }
    /// Delete all L2 facts matching an exact topic for a DID.
    /// Returns the number of rows deleted. Also marks HNSW entries as deleted.
    pub async fn delete_l2_by_topic(&self, did: &str, topic: &str) -> Result<u64> {
        let (count, deleted_ids): (u64, Vec<i64>) = mem_send!(
            self,
            DeleteL2ByTopic {
                did: did.to_string(),
                topic: topic.to_string()
            }
        )??;
        // Mark deleted in HNSW index (lazy prune) — mirrors delete_l2 pattern
        if let Some(ref idx) = self.hnsw {
            for id in &deleted_ids {
                idx.mark_deleted(*id);
            }
            if !deleted_ids.is_empty() {
                self.maybe_rebuild_hnsw(idx).await;
            }
        }
        Ok(count)
    }

    // ── User settings persistence ───────────────────────────────────────────
    /// Deterministically pause the fixture actor before enqueueing an HTTP write.
    #[cfg(test)]
    pub(crate) async fn test_settings_barrier(&self) -> std::sync::mpsc::Sender<()> {
        let (entered, entered_rx) = oneshot::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        self.tx
            .send(MemoryCommand::SettingsTestBarrier {
                entered,
                release: release_rx,
            })
            .await
            .unwrap();
        entered_rx.await.unwrap();
        release
    }

    /// Validate, commit a complete settings group, then publish it to the cache.
    /// None deletes the stored override. Once queued, the actor owns both steps:
    /// cancellation of the waiting HTTP request cannot skip cache publication.
    /// No cache lock is held while waiting for SQLite or the actor queue.
    pub async fn commit_runtime_settings(
        &self,
        settings: std::sync::Arc<crate::settings::RuntimeSettings>,
        updates: Vec<(String, Option<serde_json::Value>)>,
    ) -> Result<std::collections::HashMap<String, Option<serde_json::Value>>> {
        self.commit_runtime_settings_with_embedding(settings, updates, None)
            .await
            .map(|commit| commit.previous)
    }

    /// Own the client's publication on the memory actor, not on the awaiting
    /// HTTP/HITL future. Preparation is local and happens before enqueueing.
    pub(crate) async fn commit_runtime_settings_with_embedding(
        &self,
        settings: std::sync::Arc<crate::settings::RuntimeSettings>,
        updates: Vec<(String, Option<serde_json::Value>)>,
        embedding: Option<EmbeddingClient>,
    ) -> Result<RuntimeSettingsCommit> {
        let write_guard = if updates
            .iter()
            .any(|(key, _)| crate::call_validation::affects_call_validation(key))
        {
            Some(
                crate::call_validation::SETTINGS_WRITE
                    .clone()
                    .lock_owned()
                    .await,
            )
        } else {
            None
        };
        self.commit_runtime_settings_with_guard(settings, updates, write_guard, embedding)
            .await
    }

    /// Keep the validation lock until the actor publishes, including when the
    /// requesting future is cancelled after enqueueing. Otherwise the next
    /// candidate could be verified against the previous configuration.
    pub(crate) async fn commit_runtime_settings_locked(
        &self,
        settings: std::sync::Arc<crate::settings::RuntimeSettings>,
        updates: Vec<(String, Option<serde_json::Value>)>,
        write_guard: crate::call_validation::SettingsWriteGuard,
    ) -> Result<std::collections::HashMap<String, Option<serde_json::Value>>> {
        self.commit_runtime_settings_with_guard(settings, updates, Some(write_guard), None)
            .await
            .map(|commit| commit.previous)
    }

    async fn commit_runtime_settings_with_guard(
        &self,
        settings: std::sync::Arc<crate::settings::RuntimeSettings>,
        updates: Vec<(String, Option<serde_json::Value>)>,
        write_guard: Option<crate::call_validation::SettingsWriteGuard>,
        embedding: Option<EmbeddingClient>,
    ) -> Result<RuntimeSettingsCommit> {
        let mut updates = settings.prepare_updates(updates)?;
        let embedding = embedding
            .map(|client| -> Result<_> {
                let mut plans = Vec::new();
                for update in &mut updates {
                    if update.key.starts_with("embedding_") {
                        let plan = crate::embedding::PreparedEmbeddingUpdate::prepare(
                            &update.key,
                            &update.effective,
                        )
                        .map_err(anyhow::Error::msg)?;
                        update.effective = plan.value();
                        if update.stored.is_some() {
                            update.stored = Some(update.effective.clone());
                        }
                        plans.push((update.key.clone(), plan));
                    }
                }
                Ok(EmbeddingSettingsCommit {
                    client,
                    updates: plans,
                })
            })
            .transpose()?;
        mem_send!(
            self,
            CommitRuntimeSettings {
                updates: updates,
                settings: settings,
                write_guard: write_guard,
                embedding: embedding
            }
        )?
    }

    /// Persist a user setting to SQLite (INSERT OR REPLACE).
    pub async fn store_setting(&self, key: &str, value: serde_json::Value) -> Result<()> {
        mem_send!(
            self,
            StoreSetting {
                key: key.to_string(),
                value: value
            }
        )?
    }
    /// Delete a persisted user setting. Returns `true` if a row was deleted.
    pub async fn delete_setting(&self, key: &str) -> Result<bool> {
        mem_send!(
            self,
            DeleteSetting {
                key: key.to_string()
            }
        )?
    }

    /// Load all persisted user settings from SQLite.
    pub async fn load_all_settings(
        &self,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>> {
        mem_send!(self, LoadAllSettings {})?
    }

    // ── config_audit_log (heartbeat config drift) ──────────────────────────

    /// Append an audit row tracking a change to `heartbeat_config` (or any
    /// future config key we chose to route through this table). Returns the
    /// new row id.
    pub async fn insert_config_audit(
        &self,
        actor: crate::heartbeat_config::ConfigAuditActor,
        old_json: Option<String>,
        new_json: String,
        reason: Option<String>,
    ) -> Result<i64> {
        mem_send!(
            self,
            InsertConfigAudit {
                actor: actor.as_str().to_string(),
                old_json: old_json,
                new_json: new_json,
                reason: reason
            }
        )?
    }

    /// List recent `config_audit_log` rows, newest first.
    pub async fn list_config_audit(
        &self,
        limit: usize,
    ) -> Result<Vec<crate::heartbeat_config::ConfigAuditEntry>> {
        mem_send!(self, ListConfigAudit { limit: limit })?
    }

    // ── Skill registry (skill_proposals) methods ────────────────────────

    /// Create a new skill proposal. Returns the row ID.
    pub async fn create_skill_proposal(
        &self,
        skill_name: &str,
        requested_by: &str,
        user_request: &str,
        proposal_summary: Option<&str>,
    ) -> Result<i64> {
        let user_request = crate::utils::sanitize_stored_text(user_request, 4000);
        let proposal_summary =
            proposal_summary.map(|summary| crate::utils::sanitize_stored_text(summary, 1200));
        mem_send!(
            self,
            CreateSkillProposal {
                skill_name: skill_name.to_string(),
                requested_by: requested_by.to_string(),
                user_request: user_request,
                proposal_summary: proposal_summary
            }
        )?
    }

    /// Update fields on a skill proposal.
    pub async fn update_skill_proposal(
        &self,
        id: i64,
        mut updates: SkillProposalUpdate,
    ) -> Result<()> {
        updates.proposal_summary = match updates.proposal_summary.take() {
            Some(Some(summary)) => Some(Some(crate::utils::sanitize_stored_text(&summary, 1200))),
            Some(None) => Some(None),
            None => None,
        };

        mem_send!(
            self,
            UpdateSkillProposal {
                id: id,
                updates: Box::new(updates)
            }
        )?
    }
    /// Get a skill proposal by ID.
    pub async fn get_skill_proposal(&self, id: i64) -> Result<Option<SkillProposal>> {
        mem_send!(self, GetSkillProposal { id: id })
    }

    /// List skill proposals, optionally filtered by requester and/or status.
    pub async fn list_skill_proposals(
        &self,
        requested_by: Option<&str>,
        status_filter: Option<&str>,
    ) -> Result<Vec<SkillProposal>> {
        mem_send!(
            self,
            ListSkillProposals {
                requested_by: requested_by.map(|s| s.to_string()),
                status_filter: status_filter.map(|s| s.to_string())
            }
        )
    }
    /// Count proposals created today by a specific requester (rate limit check).
    pub async fn count_daily_proposals(&self, requested_by: &str) -> Result<i64> {
        mem_send!(
            self,
            CountDailyProposals {
                requested_by: requested_by.to_string()
            }
        )
    }
    /// Delete a skill proposal from the database.
    pub async fn delete_skill_proposal(&self, id: i64) -> Result<bool> {
        mem_send!(self, DeleteSkillProposal { id: id })
    }

    // ── Skill metrics methods ──────────────────────────────────────────

    /// Update execution metrics for a skill (called after each sandbox execution).
    pub async fn update_skill_metrics(
        &self,
        skill_name: &str,
        success: bool,
        latency_ms: f64,
        error_msg: Option<&str>,
        error_category: Option<&str>,
    ) {
        if let Err(e) = self
            .tx
            .send(MemoryCommand::UpdateSkillMetrics {
                skill_name: skill_name.to_string(),
                success,
                latency_ms,
                error_msg: error_msg.map(|s| s.to_string()),
                error_category: error_category.map(|s| s.to_string()),
            })
            .await
        {
            tracing::warn!(error = %e, "Failed to send UpdateSkillMetrics command — actor may be closed");
        }
    }
    /// Get metrics for a single skill.
    pub async fn get_skill_metrics(&self, skill_name: &str) -> Result<Option<SkillMetrics>> {
        mem_send!(
            self,
            GetSkillMetrics {
                skill_name: skill_name.to_string()
            }
        )
    }
    /// Get metrics for all skills.
    pub async fn get_all_skill_metrics(&self) -> Result<Vec<SkillMetrics>> {
        mem_send!(self, GetAllSkillMetrics {})
    }

    /// Count unhealthy skill metrics whose state changed since `since`.
    pub async fn count_unhealthy_skill_metrics_updated_since(
        &self,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<u32> {
        mem_send!(
            self,
            CountUnhealthySkillMetricsUpdatedSince {
                since_rfc3339: since.to_rfc3339(),
            }
        )?
    }

    /// Get the set of deactivated skill names.
    pub async fn get_deactivated_skill_names(&self) -> std::collections::HashSet<String> {
        mem_send_or!(
            self,
            GetDeactivatedSkillNames {},
            Default::default(),
            "get_deactivated_skill_names"
        )
    }

    // ── Circuit breaker methods ───────────────────────────────────────

    /// Reset the circuit breaker for a skill (e.g. after fix or reactivation).
    pub async fn reset_circuit_breaker(&self, skill_name: &str) {
        if let Err(e) = self
            .tx
            .send(MemoryCommand::ResetCircuitBreaker {
                skill_name: skill_name.to_string(),
            })
            .await
        {
            tracing::warn!(error = %e, "Failed to send ResetCircuitBreaker command");
        }
    }
    /// Check if a skill's circuit breaker is open. Returns `(circuit_open, consecutive_failures, last_error)`.
    pub async fn check_circuit_breaker(
        &self,
        skill_name: &str,
    ) -> Result<Option<(bool, i64, Option<String>)>> {
        mem_send!(
            self,
            CheckCircuitBreaker {
                skill_name: skill_name.to_string()
            }
        )
    }

    /// Take the circuit breaker event receiver (call once in main.rs).
    pub fn take_circuit_breaker_rx(&self) -> Option<mpsc::Receiver<CircuitBreakerEvent>> {
        self.circuit_breaker_rx.lock().ok()?.take()
    }

    // ── Deferred intent methods ────────────────────────────────────────

    /// Store a new deferred intent. Returns the row ID.
    ///
    /// If `group_key` is set and a pending intent with the same group_key + user_did
    /// already exists, the old intent is atomically cancelled before inserting the new one
    /// (upsert semantics — prevents duplicate recurring intents).
    ///
    /// `embedding` is an optional pre-computed embedding of `context_text` for semantic dedup.
    /// Pass `None` if embedding is unavailable (dedup still works via group_key).
    #[allow(clippy::too_many_arguments)]
    pub async fn store_deferred_intent(
        &self,
        user_did: &str,
        chat_id: i64,
        trigger_type: &str,
        trigger_at_unix: Option<i64>,
        event_key: Option<&str>,
        execution_mode: &str,
        context_text: &str,
        expires_at_unix: i64,
        is_callback_origin: bool,
        interval_seconds: Option<i64>,
        chain_depth: i32,
        pin_on_fire: bool,
        group_key: Option<&str>,
        max_fires: Option<i64>,
        embedding: Option<&[f32]>,
        recur_weekdays: Option<Vec<u8>>,
        recur_window_start: Option<&str>,
        recur_window_end: Option<&str>,
        recur_timezone: Option<&str>,
    ) -> Result<i64> {
        // Convert f32 embedding to bytes for storage
        let embedding_bytes = embedding.map(|emb| {
            emb.iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<u8>>()
        });
        mem_send!(
            self,
            StoreDeferredIntent {
                user_did: user_did.to_string(),
                chat_id: chat_id,
                trigger_type: trigger_type.to_string(),
                trigger_at_unix: trigger_at_unix,
                event_key: event_key.map(|s| s.to_string()),
                execution_mode: execution_mode.to_string(),
                context_text: context_text.to_string(),
                expires_at_unix: expires_at_unix,
                is_callback_origin: is_callback_origin,
                interval_seconds: interval_seconds,
                chain_depth: chain_depth,
                pin_on_fire: pin_on_fire,
                group_key: group_key.map(|s| s.to_string()),
                max_fires: max_fires,
                recur_weekdays: recur_weekdays,
                recur_window_start: recur_window_start.map(|s| s.to_string()),
                recur_window_end: recur_window_end.map(|s| s.to_string()),
                recur_timezone: recur_timezone.map(|s| s.to_string()),
                embedding: embedding_bytes,
            }
        )?
    }
    /// List active recurring intents for a user (for prompt injection).
    pub async fn list_recurring_intents(&self, user_did: &str) -> Result<Vec<DeferredIntent>> {
        mem_send!(
            self,
            ListRecurringIntents {
                user_did: user_did.to_string()
            }
        )
    }
    /// Get all pending time-based intents that are due (trigger_at_unix <= now).
    pub async fn get_due_time_intents(&self, now_unix: i64) -> Result<Vec<DeferredIntent>> {
        mem_send!(self, GetDueTimeIntents { now_unix: now_unix })
    }

    /// Get all pending event-based intents matching the given event key, scoped to a user DID.
    ///
    /// The `user_did` filter prevents cross-tenant event spoofing: an event emitted
    /// by one user's pipeline cannot fire another user's intents.
    pub async fn get_matching_event_intents(
        &self,
        event_key: &str,
        user_did: &str,
    ) -> Result<Vec<DeferredIntent>> {
        mem_send!(
            self,
            GetMatchingEventIntents {
                event_key: event_key.to_string(),
                user_did: user_did.to_string()
            }
        )
    }
    /// Mark an intent as fired.
    pub async fn mark_intent_fired(&self, id: i64) -> Result<()> {
        mem_send!(self, MarkIntentFired { id: id })?
    }

    /// Reschedule a recurring intent: update trigger_at, expires_at, reset status to pending.
    pub async fn reschedule_recurring_intent(
        &self,
        id: i64,
        trigger_at_unix: i64,
        expires_at_unix: i64,
    ) -> Result<()> {
        mem_send!(
            self,
            RescheduleRecurringIntent {
                id: id,
                trigger_at_unix: trigger_at_unix,
                expires_at_unix: expires_at_unix
            }
        )?
    }

    /// Atomically fire a recurring intent and advance its trigger time.
    /// Unlike the two-step mark_fired + reschedule, this keeps status='active'
    /// and is crash-safe: if the process dies mid-callback, the poller will
    /// pick up the intent at its next trigger_at.
    pub async fn fire_and_advance_recurring(
        &self,
        id: i64,
        trigger_at_unix: i64,
        expires_at_unix: i64,
    ) -> Result<()> {
        mem_send!(
            self,
            FireAndAdvanceRecurring {
                id: id,
                trigger_at_unix: trigger_at_unix,
                expires_at_unix: expires_at_unix
            }
        )?
    }
    /// Update the trigger_at_unix for a pending intent (useful for testing).
    pub async fn mark_intent_trigger_at(&self, id: i64, trigger_at_unix: i64) -> Result<()> {
        mem_send!(
            self,
            MarkIntentTriggerAt {
                id: id,
                trigger_at_unix: trigger_at_unix
            }
        )?
    }
    /// Expire all pending intents that have passed their TTL.
    /// Returns Vec<(id, user_did, context_text)> of expired items.
    pub async fn expire_old_intents(&self, now_unix: i64) -> Result<Vec<(i64, String, String)>> {
        mem_send!(self, ExpireOldIntents { now_unix: now_unix })?
    }
    /// List all pending intents for a user, ordered by creation (newest first).
    pub async fn list_user_intents(&self, user_did: &str) -> Result<Vec<DeferredIntent>> {
        mem_send!(
            self,
            ListUserIntents {
                user_did: user_did.to_string()
            }
        )
    }
    /// List intent history (fired/cancelled) for a user since a given timestamp.
    pub async fn list_intent_history(
        &self,
        user_did: &str,
        since_unix: i64,
    ) -> Result<Vec<DeferredIntent>> {
        mem_send!(
            self,
            ListIntentHistory {
                user_did: user_did.to_string(),
                since_unix: since_unix
            }
        )?
    }
    /// Archive old fired/cancelled intents older than cutoff.
    /// Returns Vec<(id, user_did, context_text, status)> of archived items.
    pub async fn archive_old_intents(
        &self,
        cutoff_unix: i64,
    ) -> Result<Vec<(i64, String, String, String)>> {
        mem_send!(
            self,
            ArchiveOldIntents {
                cutoff_unix: cutoff_unix
            }
        )?
    }
    /// DESTRUCTIVE retention sweep: permanently delete `archived` tasks whose
    /// finished-at basis (COALESCE(fired_at, completed_at, updated_at, created_at))
    /// is older than `cutoff_unix`. Bounded by `limit` rows per call. Returns
    /// (deleted_count, min_id, max_id) so the caller can log the audited range.
    /// Only ever deletes status='archived'; never active/done/fired/cancelled.
    pub async fn delete_old_archived_tasks(
        &self,
        cutoff_unix: i64,
        limit: i64,
    ) -> Result<(usize, Option<i64>, Option<i64>)> {
        mem_send!(
            self,
            DeleteOldArchivedTasks {
                cutoff_unix: cutoff_unix,
                limit: limit
            }
        )?
    }
    /// Dismiss a fired/cancelled intent (set to archived). DID-gated.
    pub async fn dismiss_intent(&self, id: i64, user_did: &str) -> Result<bool> {
        mem_send!(
            self,
            DismissIntent {
                id: id,
                user_did: user_did.to_string()
            }
        )?
    }
    /// Reschedule a fired/cancelled intent back to active with new timing. DID-gated.
    pub async fn reschedule_intent(
        &self,
        id: i64,
        user_did: &str,
        trigger_at_unix: i64,
        expires_at_unix: i64,
    ) -> Result<bool> {
        mem_send!(
            self,
            RescheduleIntent {
                id: id,
                user_did: user_did.to_string(),
                trigger_at_unix: trigger_at_unix,
                expires_at_unix: expires_at_unix
            }
        )?
    }
    /// Cancel a pending intent (DID-gated). Returns true if cancelled.
    pub async fn cancel_intent(&self, id: i64, user_did: &str) -> Result<bool> {
        mem_send!(
            self,
            CancelIntent {
                id: id,
                user_did: user_did.to_string()
            }
        )?
    }
    /// Count active (pending) intents for a user (rate limit check).
    pub async fn count_active_intents(&self, user_did: &str) -> Result<i64> {
        mem_send!(
            self,
            CountActiveIntents {
                user_did: user_did.to_string()
            }
        )
    }
    /// Count intents created today by a user (daily rate limit check).
    pub async fn count_daily_intents(&self, user_did: &str) -> Result<i64> {
        mem_send!(
            self,
            CountDailyIntents {
                user_did: user_did.to_string()
            }
        )
    }
    /// Increment retry_count for a pending intent. Returns new count.
    /// If max_retries reached, the intent is moved to 'dead_letter' status.
    pub async fn increment_intent_retry(&self, id: i64, max_retries: i32) -> Result<i32> {
        mem_send!(
            self,
            IncrementIntentRetry {
                id: id,
                max_retries: max_retries
            }
        )?
    }

    // ── Monitoring intent API (unified in tasks table) ─────────────

    /// Store a monitoring intent in the tasks table (trigger_type = 'semantic_alert').
    /// Replaces any existing intent for the same skill+user (upsert semantics).
    pub async fn store_monitoring_intent(
        &self,
        user_did: &str,
        skill_name: &str,
        description: &str,
        threshold: f32,
        embedding: Vec<f32>,
    ) -> Result<i64> {
        let embedding_bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
        mem_send!(
            self,
            StoreMonitoringIntent {
                user_did: user_did.to_string(),
                skill_name: skill_name.to_string(),
                description: description.to_string(),
                threshold: threshold,
                embedding: embedding_bytes
            }
        )?
    }
    /// List monitoring intents for a specific user.
    pub async fn list_monitoring_intents(
        &self,
        user_did: &str,
    ) -> Result<Vec<MonitoringIntentRecord>> {
        mem_send!(
            self,
            ListMonitoringIntents {
                user_did: user_did.to_string()
            }
        )?
    }
    /// Delete a monitoring intent by task ID.
    pub async fn delete_monitoring_intent(&self, intent_id: i64) -> Result<()> {
        mem_send!(
            self,
            DeleteMonitoringIntent {
                intent_id: intent_id
            }
        )?
    }
    /// Get all active monitoring intents across all users (for chronos heartbeat matching).
    pub async fn get_all_monitoring_intents(&self) -> Result<Vec<MonitoringIntentRecord>> {
        mem_send!(self, GetAllMonitoringIntents {})?
    }

    // ── Unified task API ────────────────────────────────────────────

    /// Store a new unified task. Returns the task ID.
    #[allow(clippy::too_many_arguments)]
    pub async fn store_task(
        &self,
        chat_id: i64,
        user_did: &str,
        title: &str,
        body: &str,
        priority: &str,
        tags: &[String],
        due_date: Option<&str>,
        source: &str,
        trigger_at_unix: Option<i64>,
        trigger_type: Option<&str>,
        event_key: Option<&str>,
        execution_mode: &str,
        context_text: Option<&str>,
        interval_seconds: Option<i64>,
        group_key: Option<&str>,
        max_fires: Option<i64>,
        pin_on_fire: bool,
        expires_at_unix: Option<i64>,
        is_callback_origin: bool,
        chain_depth: i32,
        recur_weekdays: Option<Vec<u8>>,
        recur_window_start: Option<&str>,
        recur_window_end: Option<&str>,
        recur_timezone: Option<&str>,
    ) -> Result<i64> {
        let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string());
        mem_send!(
            self,
            StoreTask {
                chat_id: chat_id,
                user_did: user_did.to_string(),
                title: title.to_string(),
                body: body.to_string(),
                priority: priority.to_string(),
                tags: tags_json,
                due_date: due_date.map(|s| s.to_string()),
                source: source.to_string(),
                trigger_at_unix: trigger_at_unix,
                trigger_type: trigger_type.map(|s| s.to_string()),
                event_key: event_key.map(|s| s.to_string()),
                execution_mode: execution_mode.to_string(),
                context_text: context_text.map(|s| s.to_string()),
                interval_seconds: interval_seconds,
                group_key: group_key.map(|s| s.to_string()),
                max_fires: max_fires,
                pin_on_fire: pin_on_fire,
                expires_at_unix: expires_at_unix,
                is_callback_origin: is_callback_origin,
                chain_depth: chain_depth,
                recur_weekdays: recur_weekdays,
                recur_window_start: recur_window_start.map(|s| s.to_string()),
                recur_window_end: recur_window_end.map(|s| s.to_string()),
                recur_timezone: recur_timezone.map(|s| s.to_string()),
            }
        )?
    }
    /// Get a task by ID.
    pub async fn get_task(&self, id: i64) -> Option<Task> {
        mem_send_or!(self, GetTask { id: id }, None, "get_task")
    }
    /// Update a task's fields.
    pub async fn update_task(&self, id: i64, chat_id: i64, updates_json: &str) -> Result<bool> {
        mem_send!(
            self,
            UpdateTask {
                id: id,
                chat_id: chat_id,
                updates_json: updates_json.to_string()
            }
        )?
    }

    /// List tasks for a chat, optionally filtered by status.
    pub async fn list_tasks(
        &self,
        chat_id: i64,
        user_did: Option<&str>,
        status: Option<&str>,
        limit: i64,
    ) -> Vec<Task> {
        mem_send_or!(
            self,
            ListTasks {
                chat_id: chat_id,
                user_did: user_did.map(|s| s.to_string()),
                status: status.map(|s| s.to_string()),
                limit: limit
            },
            Vec::new(),
            "list_tasks"
        )
    }
    /// Delete all done/archived tasks for a chat. Returns the count deleted.
    pub async fn clear_finished_tasks(&self, chat_id: i64) -> Result<usize> {
        mem_send!(self, ClearFinishedTasks { chat_id: chat_id })?
    }
    /// Permanently delete a task (only if status is done or archived).
    pub async fn delete_task(&self, id: i64, chat_id: i64) -> Result<bool> {
        mem_send!(
            self,
            DeleteTask {
                id: id,
                chat_id: chat_id
            }
        )?
    }

    /// Persist a replay nonce to SQLite. Retries briefly if channel is full
    /// because nonce loss weakens replay attack protection.
    pub fn persist_replay_nonce(&self, sig_hex: String, expires_at: i64) {
        let cmd = MemoryCommand::PersistReplayNonce {
            sig_hex,
            expires_at,
        };
        match self.tx.try_send(cmd) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(cmd)) => {
                // Security-critical: retry once after brief yield
                tracing::warn!("[Security] Memory channel full on replay nonce — retrying");
                // Use a blocking permit to avoid losing security data
                if let Ok(permit) = self.tx.try_reserve() {
                    permit.send(cmd);
                } else {
                    tracing::error!(
                        "[Security] Failed to persist replay nonce after retry — replay protection degraded"
                    );
                }
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                tracing::error!("[Security] Memory actor closed — replay nonce lost");
            }
        }
    }
    /// Load all unexpired replay nonces from SQLite.
    pub async fn load_replay_nonces(&self, min_expires_at: i64) -> Result<Vec<(String, u64)>> {
        mem_send!(
            self,
            LoadReplayNonces {
                min_expires_at: min_expires_at
            }
        )?
    }

    /// Delete expired replay nonces from SQLite (fire-and-forget, best-effort).
    pub fn delete_expired_replay_nonces(&self, now_unix: i64) {
        if let Err(e) = self
            .tx
            .try_send(MemoryCommand::DeleteExpiredReplayNonces { now_unix })
        {
            tracing::warn!(error = %e, "Memory channel full, dropping DeleteExpiredReplayNonces");
        }
    }

    // ── Skill state persistence ───────────────────────────────────────────
    /// Store a key-value pair for a skill. INSERT OR REPLACE semantics.
    /// Enforces max 100 keys per skill and max 64KB value size.
    pub async fn store_skill_state(&self, skill_name: &str, key: &str, value: &str) -> Result<()> {
        mem_send!(
            self,
            StoreSkillState {
                skill_name: skill_name.to_string(),
                key: key.to_string(),
                value: value.to_string()
            }
        )?
    }
    /// Load a value for a skill key. Returns None if the key does not exist.
    pub async fn load_skill_state(&self, skill_name: &str, key: &str) -> Result<Option<String>> {
        mem_send!(
            self,
            LoadSkillState {
                skill_name: skill_name.to_string(),
                key: key.to_string()
            }
        )?
    }
    /// Delete a skill state key. Succeeds even if the key did not exist.
    pub async fn delete_skill_state(&self, skill_name: &str, key: &str) -> Result<()> {
        mem_send!(
            self,
            DeleteSkillState {
                skill_name: skill_name.to_string(),
                key: key.to_string()
            }
        )?
    }
    /// List all non-empty state entries for a skill namespace.
    pub async fn list_skill_state_entries(
        &self,
        skill_name: &str,
    ) -> Result<Vec<(String, String)>> {
        mem_send!(
            self,
            ListSkillStateKeys {
                skill_name: skill_name.to_string()
            }
        )?
    }

    // ── Fact aging ──────────────────────────────────────────────────────────
    /// Insert/replace an HITL approval row with full extension metadata
    /// (design §6, Step 9). The HITL queue lives in `skill_state` under
    /// `skill_name='hitl'`; Step 9 adds kind/cycle/embedding/TTL columns
    /// without forking a new table.
    ///
    /// Default-bearing callers (non-heartbeat HITL) should keep using
    /// `store_skill_state("hitl", key, value)` — existing rows get
    /// `kind='generic'`, sentinel `expires_at`, and null embedding / cycle.
    /// Heartbeat proposal tools call this directly with explicit values.
    #[allow(clippy::too_many_arguments)]
    pub async fn store_hitl_approval(
        &self,
        key: &str,
        value: &str,
        kind: &str,
        proposed_at_audit_cycle_id: Option<&str>,
        embedding: Option<&[f32]>,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let embedding_json = match embedding {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        mem_send!(
            self,
            StoreHitlApproval {
                key: key.to_string(),
                value: value.to_string(),
                kind: kind.to_string(),
                proposed_at_audit_cycle_id: proposed_at_audit_cycle_id.map(str::to_string),
                embedding_json: embedding_json,
                expires_at: expires_at.to_rfc3339(),
            }
        )?
    }

    /// Delete expired HITL rows (design §6, §15.1). Rows bearing the
    /// non-expiring sentinel `'9999-12-31T00:00:00Z'` are untouched.
    /// Returns number of rows deleted. Idempotent.
    pub async fn sweep_expired_hitl(&self, now: chrono::DateTime<chrono::Utc>) -> Result<u64> {
        mem_send!(
            self,
            SweepExpiredHitl {
                now_rfc3339: now.to_rfc3339(),
            }
        )?
    }

    /// List pending HITL rows of a given `kind` (non-empty state_value,
    /// non-null embedding_json, updated_at >= since). Caller computes
    /// cosine against these to run dedup.
    pub async fn list_pending_hitl_with_embeddings(
        &self,
        kind: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<(String, String)>> {
        mem_send!(
            self,
            ListPendingHitlWithEmbeddings {
                kind: kind.to_string(),
                since_rfc3339: since.to_rfc3339(),
            }
        )?
    }

    /// Count pending HITL rows of the given `kind` (Step 9b). Used for live
    /// budget snapshot rendering.
    pub async fn count_hitl_pending_by_kind(&self, kind: &str) -> Result<u32> {
        mem_send!(
            self,
            CountHitlPendingByKind {
                kind: kind.to_string(),
            }
        )?
    }

    /// Count pending HITL rows whose state changed since `since`.
    pub async fn count_hitl_pending_updated_since(
        &self,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<u32> {
        mem_send!(
            self,
            CountHitlPendingUpdatedSince {
                since_rfc3339: since.to_rfc3339(),
            }
        )?
    }

    /// List pending HITL approval keys and values.
    pub async fn list_hitl_pending(&self) -> Result<Vec<(String, String)>> {
        self.list_skill_state_entries("hitl").await
    }

    /// Fire-and-forget: update last_accessed for the given L2 fact IDs.
    /// Used to refresh facts that are actively being retrieved.
    pub fn touch_l2_facts(&self, ids: Vec<i64>) {
        if let Err(e) = self.tx.try_send(MemoryCommand::TouchL2Facts { ids }) {
            tracing::warn!(error = %e, "Memory channel full, dropping TouchL2Facts");
        }
    }

    /// Fire-and-forget: track that a command was used (for adaptive menu ordering).
    pub fn track_command_usage(&self, command: &str, user_id: &str) {
        if let Err(e) = self.tx.try_send(MemoryCommand::TrackCommandUsage {
            command: command.to_string(),
            user_id: user_id.to_string(),
        }) {
            tracing::warn!(command = %command, error = %e, "Memory channel full, dropping TrackCommandUsage");
        }
    }
    /// Get command usage stats sorted by frequency (most-used first).
    pub async fn get_command_usage_stats(&self, user_id: &str) -> Vec<(String, u64)> {
        mem_send_or!(
            self,
            GetCommandUsageStats {
                user_id: user_id.to_string()
            },
            Vec::new(),
            "get_command_usage_stats"
        )
    }

    /// Store a context summary and structured facts from background summarization.
    /// Used by the orchestrator's rolling summarization to preserve context from pruned messages.
    pub async fn store_summary(
        &self,
        did: &str,
        summary: String,
        facts: Vec<(String, String)>,
    ) -> Result<()> {
        self.tx
            .send(MemoryCommand::StoreSummary {
                did: did.to_string(),
                summary,
                facts,
            })
            .await
            .map_err(|_| anyhow::anyhow!("memory actor closed"))
    }

    /// Fire-and-forget: persist inspectable evidence for one Web console reply.
    pub async fn store_turn_evidence(
        &self,
        user_did: &str,
        reply_key: &str,
        evidence: &[crate::evidence::EvidenceItem],
        unsupported_numbers: &[String],
    ) -> Result<()> {
        let evidence_json = serde_json::to_string(evidence)?;
        let unsupported_json = serde_json::to_string(unsupported_numbers)?;
        self.tx
            .send(MemoryCommand::StoreTurnEvidence {
                user_did: user_did.to_string(),
                reply_key: reply_key.to_string(),
                evidence_json,
                unsupported_json,
            })
            .await
            .map_err(|_| anyhow::anyhow!("memory actor closed"))
    }
    /// Test-only: backdate `last_accessed` (and `created_at`) for an L2 fact so
    /// staleness-based logic can be exercised deterministically.
    #[cfg(test)]
    pub async fn test_backdate_l2_last_accessed(&self, id: i64, days_ago: i64) -> Result<()> {
        mem_send!(
            self,
            TestBackdateL2LastAccessed {
                id: id,
                days_ago: days_ago
            }
        )?
    }

    /// Query stale L2 facts not accessed within the given number of days.
    /// Returns up to 50 facts ordered by staleness (oldest access first).
    pub async fn query_stale_facts(
        &self,
        did: &str,
        days_threshold: i64,
    ) -> Result<Vec<StaleFact>> {
        mem_send!(
            self,
            QueryStaleFacts {
                did: did.to_string(),
                days_threshold: days_threshold
            }
        )?
    }

    /// Snapshot current skill_metrics into daily table (idempotent per day).
    pub async fn snapshot_daily_metrics(&self) {
        if let Err(e) = self.tx.try_send(MemoryCommand::SnapshotDailyMetrics) {
            tracing::warn!(error = %e, "Memory channel full, dropping SnapshotDailyMetrics");
        }
    }

    /// Get daily metrics trend for a skill over the last N days.
    pub async fn get_daily_metrics_trend(
        &self,
        skill_name: &str,
        days: i64,
    ) -> Vec<(String, u64, u64, f64)> {
        mem_send_or!(
            self,
            GetDailyMetricsTrend {
                skill_name: skill_name.to_string(),
                days: days
            },
            Vec::new(),
            "get_daily_metrics_trend"
        )
    }

    // ── Knowledge Linting ──────────────────────────────────────────────

    /// Store a lint finding (contradiction, gap, or connection).
    pub async fn store_lint_finding(
        &self,
        finding_type: &str,
        fact_id_a: Option<i64>,
        fact_id_b: Option<i64>,
        similarity: Option<f64>,
        llm_verdict: Option<&str>,
    ) -> Result<i64> {
        mem_send!(
            self,
            StoreLintFinding {
                finding_type: finding_type.to_string(),
                fact_id_a: fact_id_a,
                fact_id_b: fact_id_b,
                similarity: similarity,
                llm_verdict: llm_verdict.map(|s| s.to_string())
            }
        )?
    }

    /// Check if a pending lint finding already exists for this fact pair.
    pub async fn has_pending_lint_finding(
        &self,
        finding_type: &str,
        fact_id_a: i64,
        fact_id_b: i64,
    ) -> bool {
        mem_send_or!(
            self,
            HasPendingLintFinding {
                finding_type: finding_type.to_string(),
                fact_id_a: fact_id_a,
                fact_id_b: fact_id_b
            },
            false,
            "has_pending_lint_finding"
        )
    }

    /// List lint findings with optional filters.
    pub async fn list_lint_findings(
        &self,
        finding_type: Option<&str>,
        status: Option<&str>,
        limit: i64,
    ) -> Vec<LintFinding> {
        mem_send_or!(
            self,
            ListLintFindings {
                finding_type: finding_type.map(|s| s.to_string()),
                status: status.map(|s| s.to_string()),
                limit: limit
            },
            Vec::new(),
            "list_lint_findings"
        )
    }

    pub async fn list_lint_findings_for_did(
        &self,
        user_did: &str,
        status: Option<&str>,
        limit: i64,
    ) -> Vec<LintFinding> {
        mem_send_or!(
            self,
            ListLintFindingsForDid {
                user_did: user_did.to_string(),
                status: status.map(|s| s.to_string()),
                limit: limit
            },
            Vec::new(),
            "list_lint_findings_for_did"
        )
    }

    /// Update lint finding status (resolve or dismiss).
    pub async fn update_lint_finding_status(
        &self,
        id: i64,
        status: &str,
        user_action: Option<&str>,
    ) -> Result<()> {
        mem_send!(
            self,
            UpdateLintFindingStatus {
                id: id,
                status: status.to_string(),
                user_action: user_action.map(|s| s.to_string())
            }
        )?
    }

    /// Update lint finding status with DID ownership protection.
    ///
    /// Returns `Ok(false)` when the finding is not pending, does not exist, or
    /// is not connected to an L2 fact owned by `user_did`.
    pub async fn update_lint_finding_status_for_did(
        &self,
        id: i64,
        user_did: &str,
        status: &str,
        user_action: Option<&str>,
    ) -> Result<bool> {
        mem_send!(
            self,
            UpdateLintFindingStatusForDid {
                id: id,
                user_did: user_did.to_string(),
                status: status.to_string(),
                user_action: user_action.map(str::to_string)
            }
        )?
    }

    // ── Cost analytics queries ────────────────────────────────────────

    /// Query cost breakdown by label over the last N days.
    pub async fn query_cost_by_label(&self, days: i32) -> Result<Vec<CostByLabel>> {
        mem_send!(self, QueryCostByLabel { days: days })?
    }

    /// Query cost breakdown by model over the last N days.
    pub async fn query_cost_by_model(&self, days: i32) -> Result<Vec<CostByModel>> {
        mem_send!(self, QueryCostByModel { days: days })?
    }

    /// Query daily cost totals over the last N days.
    pub async fn query_daily_cost_totals(&self, days: i32) -> Result<Vec<DailyCostTotal>> {
        mem_send!(self, QueryDailyCostTotals { days: days })?
    }

    /// Query local LLM cost coverage/completeness metadata over the last N days.
    pub async fn query_cost_coverage(&self, days: i32) -> Result<CostCoverage> {
        mem_send!(self, QueryCostCoverage { days: days })?
    }

    /// Snapshot yesterday's LLM call log into the llm_cost_daily rollup table.
    pub async fn snapshot_daily_cost_rollup(&self) {
        if let Err(e) = self.tx.try_send(MemoryCommand::SnapshotDailyCostRollup) {
            tracing::warn!(error = %e, "[CostAnalytics] Failed to send SnapshotDailyCostRollup");
        }
    }

    /// Backfill cost_usd for historical rows missing pricing data.
    pub async fn backfill_cost_usd(&self) -> Result<BackfillResult> {
        mem_send!(self, BackfillCostUsd {})?
    }

    /// Store thumbs-up/down feedback for an autonomous heartbeat notification.
    pub async fn log_notification_feedback(&self, notif_id: &str, helpful: bool) -> Result<()> {
        mem_send!(
            self,
            LogNotificationFeedback {
                notif_id: notif_id.to_string(),
                helpful: helpful,
            }
        )?
    }

    // ── Knowledge Base ──────────────────────────────────────────────────

    /// Store a new knowledge entry. Returns the row ID.
    pub async fn store_knowledge_entry(
        &self,
        chat_id: i64,
        category: &str,
        priority: &str,
        title: &str,
        body: &str,
        tags: &[String],
        related_ids: &[i64],
        source_msg_id: Option<i64>,
        source_summary: Option<&str>,
        due_date: Option<&str>,
        waiting_on: Option<&str>,
        embedding: Option<&[f32]>,
    ) -> Result<i64> {
        let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".into());
        let related_ids_json = serde_json::to_string(related_ids).unwrap_or_else(|_| "[]".into());
        mem_send!(
            self,
            StoreKnowledgeEntry {
                chat_id: chat_id,
                category: category.to_string(),
                priority: priority.to_string(),
                title: title.to_string(),
                body: body.to_string(),
                tags: tags_json,
                related_ids: related_ids_json,
                source_msg_id: source_msg_id,
                source_summary: source_summary.map(String::from),
                due_date: due_date.map(String::from),
                waiting_on: waiting_on.map(String::from),
                embedding: embedding.map(EmbeddingClient::to_blob),
            }
        )?
    }

    /// Update a knowledge entry. Returns true if updated.
    pub async fn update_knowledge_entry(
        &self,
        id: i64,
        chat_id: i64,
        updates: KnowledgeEntryUpdate,
    ) -> Result<bool> {
        mem_send!(
            self,
            UpdateKnowledgeEntry {
                id: id,
                chat_id: chat_id,
                updates: Box::new(updates)
            }
        )?
    }

    /// Query knowledge entries by category, status, and/or priority.
    pub async fn query_knowledge_entries(
        &self,
        chat_id: i64,
        category: Option<&str>,
        status: Option<&str>,
        priority: Option<&str>,
        limit: i64,
    ) -> Vec<KnowledgeEntry> {
        mem_send_or!(
            self,
            QueryKnowledgeEntries {
                chat_id: chat_id,
                category: category.map(String::from),
                status: status.map(String::from),
                priority: priority.map(String::from),
                limit: limit
            },
            Vec::new(),
            "query_knowledge_entries"
        )
    }
    /// Get a single knowledge entry by ID.
    pub async fn get_knowledge_entry(&self, id: i64, chat_id: i64) -> Option<KnowledgeEntry> {
        mem_send_or!(
            self,
            GetKnowledgeEntry {
                id: id,
                chat_id: chat_id
            },
            None,
            "get_knowledge_entry"
        )
    }

    /// Full-text search over knowledge entries (title, body, tags).
    pub async fn search_knowledge_entries(
        &self,
        chat_id: i64,
        query: &str,
        limit: i64,
    ) -> Vec<KnowledgeEntry> {
        mem_send_or!(
            self,
            SearchKnowledgeEntries {
                chat_id: chat_id,
                query: query.to_string(),
                limit: limit
            },
            Vec::new(),
            "search_knowledge_entries"
        )
    }
    /// Count active knowledge entries for a chat.
    pub async fn count_knowledge_entries(&self, chat_id: i64) -> i64 {
        mem_send_or!(
            self,
            CountKnowledgeEntries { chat_id: chat_id },
            0,
            "count_knowledge_entries"
        )
    }
    /// Get dashboard summary: counts grouped by (category, status).
    pub async fn knowledge_dashboard_summary(&self, chat_id: i64) -> Vec<(String, String, i64)> {
        mem_send_or!(
            self,
            KnowledgeDashboardSummary { chat_id: chat_id },
            Vec::new(),
            "knowledge_dashboard_summary"
        )
    }

    /// Get read-only KB hygiene counts for a chat and DID.
    pub async fn kb_hygiene_summary(&self, chat_id: i64, user_did: &str) -> KbHygieneSummary {
        mem_send_or!(
            self,
            KbHygieneSummary {
                chat_id: chat_id,
                user_did: user_did.to_string()
            },
            KbHygieneSummary::default(),
            "kb_hygiene_summary"
        )
    }

    // ── Review guard log methods ─────────────────────────────────────────

    /// Store a review pipeline record (fire-and-forget, no reply).
    pub fn store_review_record(&self, record: &crate::evaluators::ReviewRecord) {
        if let Err(e) = self.tx.try_send(MemoryCommand::StoreReviewRecord {
            layer: record.result.layer.as_str().to_string(),
            passed: record.result.passed,
            tool_matched: record.result.tool_matched.clone(),
            similarity: record.result.similarity,
            reply_preview: record.reply_preview.clone(),
            tools_called: serde_json::to_string(&record.tools_called).unwrap_or_default(),
            latency_ms: record.result.latency_ms,
        }) {
            tracing::warn!(error = %e, "Memory channel full, dropping StoreReviewRecord");
        }
    }
    /// Get aggregate review stats for the last N hours.
    pub async fn get_review_stats(&self, since_hours: u32) -> crate::evaluators::ReviewStats {
        mem_send_or!(
            self,
            GetReviewStats {
                since_hours: since_hours
            },
            Default::default(),
            "get_review_stats"
        )
    }

    /// Get recent flagged review records.
    pub async fn get_recent_flagged_reviews(
        &self,
        limit: u32,
    ) -> Vec<(String, String, String, Option<String>, Option<f64>, i64)> {
        mem_send_or!(
            self,
            GetRecentFlaggedReviews { limit: limit },
            Vec::new(),
            "get_recent_flagged_reviews"
        )
    }

    // ── Heartbeat audit log methods ─────────────────────────────────────

    /// Store a heartbeat audit entry (legacy 16-arg signature).
    ///
    /// Extension fields (design §13: outcome, skip_reason, config_snapshot_json,
    /// prompt_hash, proposals_created_json, next_tick_note_bytes) are left NULL.
    /// New writers should prefer [`store_heartbeat_audit_ext`] to populate them.
    #[allow(clippy::too_many_arguments)]
    pub async fn store_heartbeat_audit(
        &self,
        cycle_id: &str,
        skill_name: &str,
        severity: &str,
        probe_payload: Option<&str>,
        tools_called: &[String],
        tools_blocked: &[String],
        actions_taken: &[String],
        llm_reply: Option<&str>,
        tokens_used: i32,
        rounds_used: i32,
        user_did: &str,
        chat_id: Option<i64>,
        config_snapshot: &str,
        raw_process_log: Option<&str>,
        tool_call_details: Option<&str>,
    ) -> Result<()> {
        self.store_heartbeat_audit_ext(
            cycle_id,
            skill_name,
            severity,
            probe_payload,
            tools_called,
            tools_blocked,
            actions_taken,
            llm_reply,
            tokens_used,
            rounds_used,
            user_did,
            chat_id,
            config_snapshot,
            raw_process_log,
            tool_call_details,
            &HeartbeatAuditExtras::default(),
        )
        .await
    }

    /// Store a heartbeat audit entry with Step 2 extension fields populated.
    ///
    /// See [`HeartbeatAuditExtras`] for the six optional columns (outcome,
    /// skip_reason, config_snapshot_json, prompt_hash, proposals_created,
    /// next_tick_note_bytes) added for the autonomy redesign (design §13).
    #[allow(clippy::too_many_arguments)]
    pub async fn store_heartbeat_audit_ext(
        &self,
        cycle_id: &str,
        skill_name: &str,
        severity: &str,
        probe_payload: Option<&str>,
        tools_called: &[String],
        tools_blocked: &[String],
        actions_taken: &[String],
        llm_reply: Option<&str>,
        tokens_used: i32,
        rounds_used: i32,
        user_did: &str,
        chat_id: Option<i64>,
        config_snapshot: &str,
        raw_process_log: Option<&str>,
        tool_call_details: Option<&str>,
        extras: &HeartbeatAuditExtras,
    ) -> Result<()> {
        let tools_called_json = serde_json::to_string(tools_called).unwrap_or_else(|_| "[]".into());
        let tools_blocked_json =
            serde_json::to_string(tools_blocked).unwrap_or_else(|_| "[]".into());
        let actions_taken_json =
            serde_json::to_string(actions_taken).unwrap_or_else(|_| "[]".into());

        // Enum > free-text: if a typed skip_reason variant was provided, use it;
        // otherwise fall back to the free-text breadcrumb.
        let skip_reason_text = extras
            .skip_reason
            .map(|r| r.as_str().to_string())
            .or_else(|| extras.skip_reason_text.clone());

        let proposals_created_json = if extras.proposals_created.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&extras.proposals_created).unwrap_or_else(|_| "[]".into()))
        };

        mem_send!(
            self,
            StoreHeartbeatAudit {
                cycle_id: cycle_id.to_string(),
                skill_name: skill_name.to_string(),
                severity: severity.to_string(),
                probe_payload: probe_payload.map(String::from),
                tools_called: tools_called_json,
                tools_blocked: tools_blocked_json,
                actions_taken: actions_taken_json,
                llm_reply: llm_reply.map(String::from),
                tokens_used: tokens_used,
                rounds_used: rounds_used,
                user_did: user_did.to_string(),
                chat_id: chat_id,
                config_snapshot: config_snapshot.to_string(),
                raw_process_log: raw_process_log.map(String::from),
                tool_call_details: tool_call_details.map(String::from),
                outcome: extras.outcome.map(|o| o.as_str().to_string()),
                skip_reason: skip_reason_text,
                config_snapshot_json: extras.config_snapshot_json.clone(),
                prompt_hash: extras.prompt_hash.clone(),
                proposals_created_json: proposals_created_json,
                next_tick_note_bytes: extras.next_tick_note_bytes,
            }
        )?
    }

    /// Insert one row into `subcall_log`. See `docs/subcall-primitive-design.md`
    /// §7.4 (cost attribution) + §7.5 (retention). Called by the subcall
    /// runner after a bounded ReAct loop completes.
    #[allow(clippy::too_many_arguments)]
    pub async fn subcall_log_insert(
        &self,
        id: &str,
        parent_session_id: &str,
        origin: &str,
        task_preview: Option<&str>,
        allowed_tools: &[String],
        status: &str,
        rounds_used: Option<i32>,
        tool_calls_made: Option<i32>,
        tokens_used: Option<i32>,
        duration_ms: Option<i64>,
        created_at: &str,
        parent_subcall_id: Option<&str>,
        trace_available: bool,
        summary: Option<&str>,
    ) -> Result<()> {
        let allowed_tools_json = if allowed_tools.is_empty() {
            None
        } else {
            serde_json::to_string(allowed_tools).ok()
        };
        mem_send!(
            self,
            SubcallLogInsert {
                id: id.to_string(),
                parent_session_id: parent_session_id.to_string(),
                origin: origin.to_string(),
                task_preview: task_preview.map(String::from),
                allowed_tools_json: allowed_tools_json,
                status: status.to_string(),
                rounds_used: rounds_used,
                tool_calls_made: tool_calls_made,
                tokens_used: tokens_used,
                duration_ms: duration_ms,
                created_at: created_at.to_string(),
                parent_subcall_id: parent_subcall_id.map(String::from),
                trace_available: trace_available,
                summary: summary.map(String::from),
            }
        )?
    }

    /// Fetch the trace payload for `subcall_id`. Returns
    /// `(trace_available, summary, tool_outputs)`. When the `subcall_log`
    /// row reports `trace_available=0`, returns `(false, None, None)` so
    /// the caller can set `trace_evicted=true` without doing a second
    /// query. See Task 1.7 in
    /// `docs/subcall-primitive-implementation-plan.md`.
    pub async fn subcall_fetch_trace(
        &self,
        subcall_id: &str,
    ) -> Result<(bool, bool, Option<String>, Option<Vec<(String, String)>>)> {
        mem_send!(
            self,
            SubcallFetchTrace {
                subcall_id: subcall_id.to_string(),
            }
        )?
    }

    /// Update `subcall_trace.accessed_at = now()` so LRU sweep (Task 1.8)
    /// treats the row as hot. No-op if the trace row has been evicted.
    pub async fn subcall_touch_accessed(&self, subcall_id: &str) -> Result<()> {
        mem_send!(
            self,
            SubcallTouchAccessed {
                subcall_id: subcall_id.to_string(),
            }
        )?
    }

    /// Daily TTL sweep for `subcall_log` / `subcall_trace` (design §7.5).
    /// Deletes log rows older than 90 days whose trace is either absent
    /// (already evicted) or untouched for 30+ days. The matching trace
    /// row is CASCADE-deleted via the FK on `subcall_trace.subcall_id`.
    /// Returns the number of log rows deleted. Idempotent — safe to run
    /// on every tick.
    pub async fn subcall_ttl_sweep(&self) -> Result<usize> {
        mem_send!(self, SubcallTtlSweep {})?
    }

    /// Enforce the LRU soft cap on `subcall_trace` total size (design §7.5).
    /// If `SUM(size_bytes) > total_bytes_cap`, delete the oldest-by-
    /// `accessed_at` rows until back under cap, and flip the matching
    /// `subcall_log.trace_available` to 0 so recall reports eviction.
    /// Returns the number of trace rows evicted. Idempotent.
    pub async fn subcall_enforce_lru_cap(&self, total_bytes_cap: i64) -> Result<usize> {
        mem_send!(
            self,
            SubcallEnforceLruCap {
                total_bytes_cap: total_bytes_cap,
            }
        )?
    }

    /// Insert one row into `subcall_trace`. Truncation (500 KB cap, per-tool
    /// shrinking) is the caller's responsibility — this helper just writes.
    #[allow(clippy::too_many_arguments)]
    pub async fn subcall_trace_insert(
        &self,
        subcall_id: &str,
        system_prompt: Option<&str>,
        messages_json: &str,
        tool_outputs_json: &str,
        created_at: &str,
        accessed_at: &str,
        size_bytes: i64,
    ) -> Result<()> {
        mem_send!(
            self,
            SubcallTraceInsert {
                subcall_id: subcall_id.to_string(),
                system_prompt: system_prompt.map(String::from),
                messages_json: messages_json.to_string(),
                tool_outputs_json: tool_outputs_json.to_string(),
                created_at: created_at.to_string(),
                accessed_at: accessed_at.to_string(),
                size_bytes: size_bytes,
            }
        )?
    }

    /// Record a probe observation so the next heartbeat tick can fold it
    /// into its prompt. Design §15.2 Step 3.
    ///
    /// `summary` is truncated to 500 chars to bound prompt cost.
    pub async fn record_observed_event(
        &self,
        source: &str,
        severity: &str,
        summary: &str,
        payload: Option<&serde_json::Value>,
    ) -> Result<i64> {
        let summary_bounded: String = summary.chars().take(500).collect();
        let payload_json = payload.map(|v| v.to_string());
        mem_send!(
            self,
            RecordObservedEvent {
                source: source.to_string(),
                severity: severity.to_string(),
                summary: summary_bounded,
                payload_json: payload_json,
            }
        )?
    }

    /// Atomically drain up to `limit` oldest unconsumed observed events and
    /// mark them consumed in a single transaction. A second call with the
    /// same `limit` yields rows strictly newer than those returned here.
    pub async fn take_unconsumed_observed_events(
        &self,
        limit: usize,
    ) -> Result<Vec<ObservedEvent>> {
        mem_send!(self, TakeUnconsumedObservedEvents { limit: limit })?
    }

    pub async fn store_raw_external_event(
        &self,
        event: &crate::assistant_events::RawExternalEvent,
    ) -> Result<i64> {
        mem_send!(
            self,
            StoreRawExternalEvent {
                event: event.clone(),
            }
        )?
    }

    pub async fn store_assistant_event(
        &self,
        event: &crate::assistant_events::AssistantEvent,
    ) -> Result<i64> {
        mem_send!(
            self,
            StoreAssistantEvent {
                event: event.clone(),
            }
        )?
    }

    pub async fn store_assistant_work_item(
        &self,
        item: &crate::assistant_work_items::AssistantWorkItem,
    ) -> Result<i64> {
        mem_send!(self, StoreAssistantWorkItem { item: item.clone() })?
    }

    pub async fn list_assistant_work_items(
        &self,
        user_did: &str,
        statuses: Option<&[&str]>,
        limit: usize,
    ) -> Result<Vec<crate::assistant_work_items::StoredAssistantWorkItem>> {
        mem_send!(
            self,
            ListAssistantWorkItems {
                user_did: user_did.to_string(),
                statuses: statuses.map(|items| items.iter().map(|s| (*s).to_string()).collect()),
                source_kinds: None,
                modes: None,
                caller_contexts: None,
                limit: limit,
            }
        )?
    }

    pub async fn claim_next_assistant_work_item(
        &self,
        user_did: &str,
    ) -> Result<Option<crate::assistant_work_items::StoredAssistantWorkItem>> {
        mem_send!(
            self,
            ClaimNextAssistantWorkItem {
                user_did: user_did.to_string(),
                filter: None,
            }
        )?
    }

    pub async fn claim_next_assistant_work_item_matching(
        &self,
        user_did: &str,
        filter: &crate::assistant_work_items::AssistantWorkClaimFilter,
    ) -> Result<Option<crate::assistant_work_items::StoredAssistantWorkItem>> {
        mem_send!(
            self,
            ClaimNextAssistantWorkItem {
                user_did: user_did.to_string(),
                filter: Some(filter.clone()),
            }
        )?
    }

    pub async fn update_assistant_work_item(
        &self,
        id: i64,
        status: crate::assistant_work_items::AssistantWorkStatus,
        result: Option<&serde_json::Value>,
    ) -> Result<()> {
        mem_send!(
            self,
            UpdateAssistantWorkItem {
                id: id,
                status: status,
                result: result.cloned(),
            }
        )?
    }

    pub async fn list_recent_assistant_events(
        &self,
        user_did: &str,
        limit: usize,
    ) -> Result<Vec<crate::assistant_events::StoredAssistantEvent>> {
        mem_send!(
            self,
            ListRecentAssistantEvents {
                user_did: user_did.to_string(),
                limit: limit,
            }
        )?
    }

    pub async fn list_recent_turn_evidence(
        &self,
        user_did: &str,
        limit: usize,
    ) -> Result<Vec<TurnEvidenceRecord>> {
        mem_send!(
            self,
            ListRecentTurnEvidence {
                user_did: user_did.to_string(),
                limit: limit,
            }
        )?
    }

    pub async fn get_assistant_event_by_id(
        &self,
        id: i64,
    ) -> Result<Option<crate::assistant_events::StoredAssistantEvent>> {
        mem_send!(self, GetAssistantEventById { id: id })?
    }

    pub async fn peek_new_assistant_events_for_consumer(
        &self,
        user_did: &str,
        consumer: &str,
        limit: usize,
        decisions: Option<&[&str]>,
        exclude_decisions: Option<&[&str]>,
    ) -> Result<Vec<crate::assistant_events::StoredAssistantEvent>> {
        mem_send!(
            self,
            PeekNewAssistantEventsForConsumer {
                user_did: user_did.to_string(),
                consumer: consumer.to_string(),
                limit: limit,
                decisions: decisions.map(|items| items.iter().map(|s| (*s).to_string()).collect()),
                exclude_decisions: exclude_decisions
                    .map(|items| items.iter().map(|s| (*s).to_string()).collect()),
            }
        )?
    }

    pub async fn commit_assistant_event_cursor(
        &self,
        user_did: &str,
        consumer: &str,
        last_event_id: i64,
    ) -> Result<()> {
        mem_send!(
            self,
            CommitAssistantEventCursor {
                user_did: user_did.to_string(),
                consumer: consumer.to_string(),
                last_event_id: last_event_id,
            }
        )?
    }

    /// Query heartbeat audit entries, optionally filtered by skill name.
    pub async fn query_heartbeat_audit(
        &self,
        skill_name: Option<&str>,
        limit: usize,
    ) -> Result<Vec<HeartbeatAuditEntry>> {
        mem_send!(
            self,
            QueryHeartbeatAudit {
                skill_name: skill_name.map(String::from),
                limit: limit
            }
        )?
    }
    /// Sum tokens_used from today's heartbeat audit entries (UTC date).
    /// Used to restore the in-memory daily token counter after process restart.
    pub async fn sum_todays_heartbeat_tokens(&self) -> Result<u32> {
        mem_send!(self, SumTodaysHeartbeatTokens {})?
    }
    /// Query heartbeat audit entries from the last N days that have tool_call_details.
    /// Used by credit assignment analysis.
    pub async fn query_heartbeat_audit_recent(
        &self,
        lookback_days: u32,
    ) -> Result<Vec<HeartbeatAuditEntry>> {
        mem_send!(
            self,
            QueryHeartbeatAuditRecent {
                lookback_days: lookback_days
            }
        )?
    }

    /// Store a credit analysis result for one tool on a given date.
    pub async fn store_credit_analysis(
        &self,
        analysis_date: &str,
        tool_name: &str,
        times_called: i32,
        times_cited: i32,
        effectiveness_score: f64,
        wasted_calls: i32,
        avg_latency_ms: f64,
        total_tokens_attributed: i32,
    ) -> Result<()> {
        mem_send!(
            self,
            StoreCreditAnalysis {
                analysis_date: analysis_date.to_string(),
                tool_name: tool_name.to_string(),
                times_called: times_called,
                times_cited: times_cited,
                effectiveness_score: effectiveness_score,
                wasted_calls: wasted_calls,
                avg_latency_ms: avg_latency_ms,
                total_tokens_attributed: total_tokens_attributed,
            }
        )?
    }

    /// Query the latest credit analysis results within a lookback window.
    pub async fn query_latest_credit_analysis(
        &self,
        lookback_days: u32,
    ) -> Result<Vec<crate::chronos::ToolCreditScore>> {
        mem_send!(
            self,
            QueryLatestCreditAnalysis {
                lookback_days: lookback_days
            }
        )?
    }

    /// Store a per-call LLM token usage record (fire-and-forget, non-blocking).
    pub async fn store_llm_call_log(
        &self,
        label: &str,
        model: &str,
        provider: Option<&str>,
        provider_request_id: Option<&str>,
        prompt_tokens: u32,
        completion_tokens: u32,
        cached_tokens: u32,
        total_tokens: u32,
        cost_usd: Option<f64>,
        latency_ms: Option<u64>,
        is_fallback: bool,
    ) {
        if let Err(e) = self
            .tx
            .send(MemoryCommand::StoreLlmCallLog {
                label: label.to_string(),
                model: model.to_string(),
                provider: provider.map(str::to_string),
                provider_request_id: provider_request_id.map(str::to_string),
                prompt_tokens,
                completion_tokens,
                cached_tokens,
                total_tokens,
                cost_usd,
                latency_ms,
                is_fallback,
            })
            .await
        {
            tracing::warn!(error = %e, "Failed to send StoreLlmCallLog command");
        }
    }

    /// Store a prompt version (pending approval).
    pub async fn store_prompt_version(
        &self,
        module_name: &str,
        version: i32,
        original_text: &str,
        modified_text: &str,
        rationale: &str,
        status: &str,
        approval_id: Option<&str>,
    ) -> Result<i64> {
        mem_send!(
            self,
            StorePromptVersion {
                module_name: module_name.to_string(),
                version: version,
                original_text: original_text.to_string(),
                modified_text: modified_text.to_string(),
                rationale: rationale.to_string(),
                status: status.to_string(),
                approval_id: approval_id.map(String::from),
            }
        )?
    }
    /// Query all active prompt overrides.
    pub async fn query_active_prompt_overrides(&self) -> Result<Vec<PromptOverride>> {
        mem_send!(self, QueryActivePromptOverrides {})?
    }
    /// Update the status of a prompt version.
    pub async fn update_prompt_version_status(&self, id: i64, status: &str) -> Result<()> {
        mem_send!(
            self,
            UpdatePromptVersionStatus {
                id: id,
                status: status.to_string()
            }
        )?
    }

    /// Query prompt version history for a specific module.
    pub async fn query_prompt_version_history(
        &self,
        module_name: &str,
        limit: usize,
    ) -> Result<Vec<PromptOverride>> {
        mem_send!(
            self,
            QueryPromptVersionHistory {
                module_name: module_name.to_string(),
                limit: limit
            }
        )?
    }
}
