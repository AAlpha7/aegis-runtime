use super::*;

// ── Actor loop (runs on a blocking thread) ──────────────────────────────────

pub(super) fn memory_actor_loop(
    conn: Connection,
    mut rx: mpsc::Receiver<MemoryCommand>,
    max_db_size: u64,
    l1_max_size: usize,
    db_path_str: String,
    circuit_breaker_tx: mpsc::Sender<CircuitBreakerEvent>,
) {
    use std::collections::HashMap;

    // L1 caches: per-DID VecDeque
    let mut l1_caches: HashMap<String, VecDeque<Message>> = HashMap::new();

    // Track whether we've already warned about DB size (avoid spam)
    let mut size_warning_issued = false;

    // Graceful shutdown: when set, we drain remaining queued commands before flushing.
    // Holds the oneshot reply sender to signal shutdown completion.
    let mut shutdown_reply: Option<oneshot::Sender<Result<()>>> = None;

    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            #[cfg(test)]
            MemoryCommand::SettingsTestBarrier { entered, release } => {
                let _ = entered.send(());
                let _ = release.recv();
            }
            // ── Memory core commands (delegated) ────────────────────────────
            cmd @ (MemoryCommand::PushL1 { .. }
            | MemoryCommand::GetL1 { .. }
            | MemoryCommand::StoreL2 { .. }
            | MemoryCommand::StoreL2WithMeta { .. }
            | MemoryCommand::QueryL2Semantic { .. }
            | MemoryCommand::GetAllL2 { .. }
            | MemoryCommand::GetL2ById { .. }
            | MemoryCommand::TouchL2ById { .. }
            | MemoryCommand::DeleteL2 { .. }
            | MemoryCommand::UpdateL2 { .. }
            | MemoryCommand::SetL2Pinned { .. }
            | MemoryCommand::CheckDbSize { .. }
            | MemoryCommand::Healthcheck { .. }
            | MemoryCommand::CountEmbeddedFacts { .. }
            | MemoryCommand::QueryL3Fulltext { .. }
            | MemoryCommand::ListRecentL3 { .. }
            | MemoryCommand::ListTriggerArchive { .. }) => {
                handle_memory_core_cmd(
                    &conn,
                    cmd,
                    &db_path_str,
                    max_db_size,
                    &mut size_warning_issued,
                    &mut l1_caches,
                    l1_max_size,
                );
            }

            cmd @ (MemoryCommand::StoreL2Provenance { .. }
            | MemoryCommand::ListL2Contradictions { .. }
            | MemoryCommand::ListL2FactHistory { .. }
            | MemoryCommand::GetL2Provenance { .. }
            | MemoryCommand::ConsolidateL2Observations { .. }) => {
                partial_ref::handle_partial_ref_cmd(&conn, cmd);
            }

            // ── Reminder commands (delegated) ──────────────────────────────
            cmd @ (MemoryCommand::StoreReminder { .. }
            | MemoryCommand::GetDueReminders { .. }
            | MemoryCommand::MarkReminderFired { .. }
            | MemoryCommand::GetPendingReminders { .. }
            | MemoryCommand::ListRemindersForChat { .. }
            | MemoryCommand::CancelReminder { .. }) => {
                handle_reminder_cmd(
                    &conn,
                    cmd,
                    &db_path_str,
                    max_db_size,
                    &mut size_warning_issued,
                );
            }

            // ── User settings (delegated) ───────────────────────────────────
            cmd @ (MemoryCommand::CommitRuntimeSettings { .. }
            | MemoryCommand::StoreSetting { .. }
            | MemoryCommand::DeleteSetting { .. }
            | MemoryCommand::LoadAllSettings { .. }
            | MemoryCommand::InsertConfigAudit { .. }
            | MemoryCommand::ListConfigAudit { .. }) => {
                handle_setting_cmd(
                    &conn,
                    cmd,
                    &db_path_str,
                    max_db_size,
                    &mut size_warning_issued,
                );
            }

            // ── L2 topic-prefix query (delegated) ───────────────────────────
            cmd @ (MemoryCommand::QueryL2ByTopicPrefix { .. }
            | MemoryCommand::QueryL2ByTopicPrefixSince { .. }
            | MemoryCommand::QueryL2TextByTopicPrefixSince { .. }
            | MemoryCommand::GetL2ByIdIfFresh { .. }) => {
                handle_l2_topic_query_cmd(&conn, cmd);
            }

            // ── Skill registry commands (delegated) ──────────────────────────
            cmd @ (MemoryCommand::CreateSkillProposal { .. }
            | MemoryCommand::UpdateSkillProposal { .. }
            | MemoryCommand::GetSkillProposal { .. }
            | MemoryCommand::ListSkillProposals { .. }
            | MemoryCommand::CountDailyProposals { .. }
            | MemoryCommand::DeleteSkillProposal { .. }) => {
                handle_skill_proposal_cmd(
                    &conn,
                    cmd,
                    &db_path_str,
                    max_db_size,
                    &mut size_warning_issued,
                );
            }

            // ── Skill metrics commands (delegated) ──────────────────────────
            cmd @ (MemoryCommand::UpdateSkillMetrics { .. }
            | MemoryCommand::GetSkillMetrics { .. }
            | MemoryCommand::GetAllSkillMetrics { .. }
            | MemoryCommand::CountUnhealthySkillMetricsUpdatedSince { .. }
            | MemoryCommand::GetDeactivatedSkillNames { .. }
            | MemoryCommand::ResetCircuitBreaker { .. }
            | MemoryCommand::CheckCircuitBreaker { .. }) => {
                handle_skill_metrics_cmd(
                    &conn,
                    cmd,
                    &db_path_str,
                    max_db_size,
                    &mut size_warning_issued,
                    &circuit_breaker_tx,
                );
            }

            // ── Deferred intent commands (delegated) ──────────────────────────
            cmd @ (MemoryCommand::StoreDeferredIntent { .. }
            | MemoryCommand::GetDueTimeIntents { .. }
            | MemoryCommand::GetMatchingEventIntents { .. }
            | MemoryCommand::MarkIntentFired { .. }
            | MemoryCommand::MarkIntentTriggerAt { .. }
            | MemoryCommand::RescheduleRecurringIntent { .. }
            | MemoryCommand::FireAndAdvanceRecurring { .. }
            | MemoryCommand::ExpireOldIntents { .. }
            | MemoryCommand::ListIntentHistory { .. }
            | MemoryCommand::ArchiveOldIntents { .. }
            | MemoryCommand::DeleteOldArchivedTasks { .. }
            | MemoryCommand::DismissIntent { .. }
            | MemoryCommand::RescheduleIntent { .. }
            | MemoryCommand::ListUserIntents { .. }
            | MemoryCommand::ListRecurringIntents { .. }
            | MemoryCommand::CancelIntent { .. }
            | MemoryCommand::CountActiveIntents { .. }
            | MemoryCommand::CountDailyIntents { .. }
            | MemoryCommand::IncrementIntentRetry { .. }) => {
                handle_intent_cmd(
                    &conn,
                    cmd,
                    &db_path_str,
                    max_db_size,
                    &mut size_warning_issued,
                );
            }
            // ── Monitoring intent commands (delegated) ─────────────────
            cmd @ (MemoryCommand::StoreMonitoringIntent { .. }
            | MemoryCommand::ListMonitoringIntents { .. }
            | MemoryCommand::DeleteMonitoringIntent { .. }
            | MemoryCommand::GetAllMonitoringIntents { .. }) => {
                handle_monitoring_cmd(
                    &conn,
                    cmd,
                    &db_path_str,
                    max_db_size,
                    &mut size_warning_issued,
                );
            }

            // ── Task commands (delegated, includes StoreTask) ─────────────
            cmd @ (MemoryCommand::StoreTask { .. }
            | MemoryCommand::GetTask { .. }
            | MemoryCommand::UpdateTask { .. }
            | MemoryCommand::ClearFinishedTasks { .. }
            | MemoryCommand::DeleteTask { .. }
            | MemoryCommand::ListTasks { .. }
            | MemoryCommand::ListAllScheduledTasks { .. }) => {
                handle_task_cmd(
                    &conn,
                    cmd,
                    &db_path_str,
                    max_db_size,
                    &mut size_warning_issued,
                );
            }

            // ── Replay nonce + skill state commands (delegated) ──────────
            cmd @ (MemoryCommand::PersistReplayNonce { .. }
            | MemoryCommand::LoadReplayNonces { .. }
            | MemoryCommand::DeleteExpiredReplayNonces { .. }
            | MemoryCommand::StoreSkillState { .. }
            | MemoryCommand::LoadSkillState { .. }
            | MemoryCommand::DeleteSkillState { .. }
            | MemoryCommand::ListSkillStateKeys { .. }
            | MemoryCommand::StoreHitlApproval { .. }
            | MemoryCommand::SweepExpiredHitl { .. }
            | MemoryCommand::ListPendingHitlWithEmbeddings { .. }
            | MemoryCommand::CountHitlPendingByKind { .. }
            | MemoryCommand::CountHitlPendingUpdatedSince { .. }) => {
                handle_replay_state_cmd(&conn, cmd);
            }
            // ── Context / L2 topic / fact aging / command usage (delegated) ──
            cmd @ (MemoryCommand::QueryL2ByTopic { .. }
            | MemoryCommand::DeleteL2ByTopic { .. }
            | MemoryCommand::StoreSummary { .. }
            | MemoryCommand::StoreTurnEvidence { .. }
            | MemoryCommand::TouchL2Facts { .. }
            | MemoryCommand::TrackCommandUsage { .. }
            | MemoryCommand::GetCommandUsageStats { .. }
            | MemoryCommand::QueryStaleFacts { .. }) => {
                handle_context_cmd(
                    &conn,
                    cmd,
                    &db_path_str,
                    max_db_size,
                    &mut size_warning_issued,
                );
            }
            #[cfg(test)]
            cmd @ MemoryCommand::TestBackdateL2LastAccessed { .. } => {
                handle_context_cmd(
                    &conn,
                    cmd,
                    &db_path_str,
                    max_db_size,
                    &mut size_warning_issued,
                );
            }

            // ── Analytics / review / heartbeat / credit (delegated) ────────
            cmd @ (MemoryCommand::SnapshotDailyMetrics
            | MemoryCommand::GetDailyMetricsTrend { .. }
            | MemoryCommand::StoreReviewRecord { .. }
            | MemoryCommand::GetReviewStats { .. }
            | MemoryCommand::GetRecentFlaggedReviews { .. }
            | MemoryCommand::StoreHeartbeatAudit { .. }
            | MemoryCommand::SubcallLogInsert { .. }
            | MemoryCommand::SubcallTraceInsert { .. }
            | MemoryCommand::SubcallFetchTrace { .. }
            | MemoryCommand::SubcallTouchAccessed { .. }
            | MemoryCommand::SubcallTtlSweep { .. }
            | MemoryCommand::SubcallEnforceLruCap { .. }
            | MemoryCommand::RecordObservedEvent { .. }
            | MemoryCommand::TakeUnconsumedObservedEvents { .. }
            | MemoryCommand::StoreRawExternalEvent { .. }
            | MemoryCommand::StoreAssistantEvent { .. }
            | MemoryCommand::StoreAssistantWorkItem { .. }
            | MemoryCommand::ListAssistantWorkItems { .. }
            | MemoryCommand::ClaimNextAssistantWorkItem { .. }
            | MemoryCommand::UpdateAssistantWorkItem { .. }
            | MemoryCommand::ListRecentAssistantEvents { .. }
            | MemoryCommand::ListRecentTurnEvidence { .. }
            | MemoryCommand::GetAssistantEventById { .. }
            | MemoryCommand::PeekNewAssistantEventsForConsumer { .. }
            | MemoryCommand::CommitAssistantEventCursor { .. }
            | MemoryCommand::QueryHeartbeatAudit { .. }
            | MemoryCommand::SumTodaysHeartbeatTokens { .. }
            | MemoryCommand::QueryHeartbeatAuditRecent { .. }
            | MemoryCommand::StoreCreditAnalysis { .. }
            | MemoryCommand::QueryLatestCreditAnalysis { .. }
            | MemoryCommand::StoreLlmCallLog { .. }
            | MemoryCommand::QueryCostByLabel { .. }
            | MemoryCommand::QueryCostByModel { .. }
            | MemoryCommand::QueryDailyCostTotals { .. }
            | MemoryCommand::QueryCostCoverage { .. }
            | MemoryCommand::SnapshotDailyCostRollup
            | MemoryCommand::BackfillCostUsd { .. }) => {
                handle_analytics_cmd(&conn, cmd);
            }

            // ── Self-audit query commands ─────────────────────────────────
            cmd @ (MemoryCommand::AuditHeartbeatNoopRatio { .. }
            | MemoryCommand::AuditRecentHeartbeatNoopCount { .. }
            | MemoryCommand::AuditRecentToolBlocks { .. }
            | MemoryCommand::AuditRecentLlmCalls { .. }
            | MemoryCommand::AuditRecentHeartbeatCount { .. }
            | MemoryCommand::AuditRecentLlmEligibleHeartbeatCount { .. }
            | MemoryCommand::AuditDailyCacheRates { .. }
            | MemoryCommand::AuditDailyTokenTotals { .. }) => {
                handle_self_audit_cmd(&conn, cmd);
            }

            // ── Test-only: heartbeat_audit timestamp backdate ─────────────
            #[cfg(test)]
            cmd @ MemoryCommand::TestBackdateHeartbeatAuditByOutcome { .. } => {
                handle_self_audit_cmd(&conn, cmd);
            }

            // ── Knowledge Base operations (delegated) ────────────────────────
            cmd @ (MemoryCommand::StoreKnowledgeEntry { .. }
            | MemoryCommand::UpdateKnowledgeEntry { .. }
            | MemoryCommand::QueryKnowledgeEntries { .. }
            | MemoryCommand::GetKnowledgeEntry { .. }
            | MemoryCommand::SearchKnowledgeEntries { .. }
            | MemoryCommand::CountKnowledgeEntries { .. }
            | MemoryCommand::KnowledgeDashboardSummary { .. }
            | MemoryCommand::KbHygieneSummary { .. }) => {
                handle_knowledge_cmd(
                    &conn,
                    cmd,
                    &db_path_str,
                    max_db_size,
                    &mut size_warning_issued,
                );
            }

            // NOTE: StoreReviewRecord through QueryLatestCreditAnalysis are now
            // dispatched above via handle_analytics_cmd. The remaining old inline
            // arms below should have been removed. If this comment appears in
            // production code, the migration is incomplete.
            //
            // ── Prompt version commands (delegated) ─────────────────────────
            cmd @ (MemoryCommand::StorePromptVersion { .. }
            | MemoryCommand::QueryActivePromptOverrides { .. }
            | MemoryCommand::UpdatePromptVersionStatus { .. }
            | MemoryCommand::QueryPromptVersionHistory { .. }) => {
                handle_prompt_version_cmd(&conn, cmd);
            }

            // ── G1: Relationship State ────────────────────────────────────
            MemoryCommand::GetRelationshipState { did, reply } => {
                let result = get_relationship_state(&conn, &did);
                mem_reply!(reply, result);
            }
            MemoryCommand::UpdateRelationshipInteraction {
                did,
                topic,
                user_tz,
                reply,
            } => {
                let result = update_relationship_interaction(&conn, &did, &topic, &user_tz);
                mem_reply!(reply, result);
            }
            // ── Tier 0: User Profile ──────────────────────────────────────
            MemoryCommand::GetUserProfile { did, reply } => {
                let result = get_user_profile(&conn, &did);
                mem_reply!(reply, result);
            }
            MemoryCommand::UpsertProfileEntry { entry, reply } => {
                let result = upsert_profile_entry(&conn, &entry);
                mem_reply!(reply, result);
            }
            MemoryCommand::DeleteProfileEntry {
                did,
                category,
                key,
                reply,
            } => {
                let result = delete_profile_entry(&conn, &did, &category, &key);
                mem_reply!(reply, result);
            }
            MemoryCommand::ListEphemeralSummaries { did, limit, reply } => {
                let result = (|| -> Result<Vec<crate::memory::EphemeralSummary>> {
                    let mut stmt = conn.prepare(
                        "SELECT topic, fact_summary, source, created_at FROM l2_ephemeral \
                         WHERE agent_did = ?1 AND source IN ('context_summary', 'compaction_summary') \
                         AND (expires_at IS NULL OR expires_at > datetime('now')) \
                         ORDER BY created_at DESC LIMIT ?2",
                    )?;
                    let rows = stmt
                        .query_map(rusqlite::params![did, limit.clamp(1, 200)], |r| {
                            Ok(crate::memory::EphemeralSummary {
                                topic: r.get(0)?,
                                fact_summary: r.get(1)?,
                                source: r.get(2)?,
                                created_at: r.get(3)?,
                            })
                        })?
                        .filter_map(|r| r.ok())
                        .collect();
                    Ok(rows)
                })();
                mem_reply!(reply, result);
            }
            MemoryCommand::CountEphemeral { did, source, reply } => {
                let result = if let Some(src) = &source {
                    conn.query_row(
                        "SELECT COUNT(*) FROM l2_ephemeral WHERE agent_did = ?1 AND source = ?2",
                        rusqlite::params![did, src],
                        |r| r.get(0),
                    )
                } else {
                    conn.query_row(
                        "SELECT COUNT(*) FROM l2_ephemeral WHERE agent_did = ?1",
                        [&did],
                        |r| r.get(0),
                    )
                };
                mem_reply!(reply, result.map_err(anyhow::Error::from));
            }
            MemoryCommand::StoreEphemeral {
                did,
                topic,
                fact,
                embedding,
                source,
                ttl_days,
                reply,
            } => {
                let decision = l2_write_guard(
                    &conn,
                    &did,
                    &topic,
                    &fact,
                    embedding.as_deref(),
                    MemoryTier::Ephemeral,
                );
                if decision == WriteDecision::Skip {
                    tracing::debug!(
                        topic = %topic,
                        source = %source,
                        "store_ephemeral: Skip (duplicate)"
                    );
                    mem_reply!(reply, Ok(-1i64));
                    return;
                }
                // Externalize overly long fact bodies (consistent with l2_experience path).
                let externalized = externalize_long_fact(&fact);
                // Negative TTL is allowed — produces a pre-expired row. Useful for
                // tests and for callers that want an entry to exist briefly.
                let ttl_expr = if ttl_days >= 0 {
                    format!("datetime('now', '+{} days')", ttl_days)
                } else {
                    format!("datetime('now', '{} days')", ttl_days)
                };
                let sql = format!(
                    "INSERT INTO l2_ephemeral \
                     (agent_did, topic, fact_summary, embedding, source, created_at, expires_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), {})",
                    ttl_expr
                );
                let result = conn
                    .execute(
                        &sql,
                        rusqlite::params![did, topic, externalized, embedding, source],
                    )
                    .map(|_| conn.last_insert_rowid())
                    .map_err(anyhow::Error::from);
                mem_reply!(reply, result);
            }
            MemoryCommand::CleanupExpiredEphemeral { reply } => {
                let result = conn
                    .execute(
                        "DELETE FROM l2_ephemeral WHERE expires_at < datetime('now')",
                        [],
                    )
                    .map_err(anyhow::Error::from);
                mem_reply!(reply, result);
            }

            // ── G5: Goal Tracking ──────────────────────────────────────────
            MemoryCommand::CreateGoal { did, goal, reply } => {
                let result = create_goal(&conn, &did, &goal);
                mem_reply!(reply, result);
            }
            MemoryCommand::ListGoals { did, status, reply } => {
                let result = list_goals(&conn, &did, status.as_ref());
                mem_reply!(reply, result);
            }
            MemoryCommand::GetGoal { id, reply } => {
                let result = get_goal(&conn, id);
                mem_reply!(reply, result);
            }
            MemoryCommand::UpdateGoal { id, updates, reply } => {
                let result = update_goal(&conn, id, &updates);
                mem_reply!(reply, result);
            }
            MemoryCommand::AddMilestone {
                goal_id,
                title,
                reply,
            } => {
                let result = add_milestone(&conn, goal_id, &title);
                mem_reply!(reply, result);
            }
            MemoryCommand::CompleteMilestone {
                goal_id,
                milestone_idx,
                reply,
            } => {
                let result = complete_milestone(&conn, goal_id, milestone_idx);
                mem_reply!(reply, result);
            }

            MemoryCommand::LinkFactToGoal {
                goal_id,
                fact_id,
                similarity,
                reply,
            } => {
                let result = link_fact_to_goal(&conn, goal_id, fact_id, similarity);
                mem_reply!(reply, result);
            }

            // ── Knowledge Linting commands ──────────────────────────────────
            MemoryCommand::StoreLintFinding {
                finding_type,
                fact_id_a,
                fact_id_b,
                similarity,
                llm_verdict,
                reply,
            } => {
                let result = conn.execute(
                    "INSERT INTO l2_lint_findings (finding_type, fact_id_a, fact_id_b, similarity, llm_verdict)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![finding_type, fact_id_a, fact_id_b, similarity, llm_verdict],
                ).map(|_| conn.last_insert_rowid()).map_err(anyhow::Error::from);
                mem_reply!(reply, result);
            }
            MemoryCommand::HasPendingLintFinding {
                finding_type,
                fact_id_a,
                fact_id_b,
                reply,
            } => {
                let exists = conn.query_row(
                    "SELECT COUNT(*) FROM l2_lint_findings
                     WHERE finding_type = ?1 AND status = 'pending'
                       AND ((fact_id_a = ?2 AND fact_id_b = ?3) OR (fact_id_a = ?3 AND fact_id_b = ?2))",
                    rusqlite::params![finding_type, fact_id_a, fact_id_b],
                    |r| r.get::<_, i64>(0),
                ).unwrap_or(0) > 0;
                mem_reply!(reply, exists);
            }
            MemoryCommand::ListLintFindings {
                finding_type,
                status,
                limit,
                reply,
            } => {
                let mut sql = String::from(
                    "SELECT id, finding_type, fact_id_a, fact_id_b, similarity, llm_verdict, status, user_action, created_at, resolved_at
                     FROM l2_lint_findings WHERE 1=1"
                );
                let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
                if let Some(ref ft) = finding_type {
                    params.push(Box::new(ft.clone()));
                    sql.push_str(&format!(" AND finding_type = ?{}", params.len()));
                }
                if let Some(ref st) = status {
                    params.push(Box::new(st.clone()));
                    sql.push_str(&format!(" AND status = ?{}", params.len()));
                }
                params.push(Box::new(limit));
                sql.push_str(&format!(
                    " ORDER BY created_at DESC LIMIT ?{}",
                    params.len()
                ));

                let pr: Vec<&dyn rusqlite::ToSql> = params.iter().map(|x| x.as_ref()).collect();
                let findings = (|| -> Vec<LintFinding> {
                    let mut stmt = match conn.prepare(&sql) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(error = %e, "ListLintFindings prepare failed");
                            return Vec::new();
                        }
                    };
                    let rows = match stmt.query_map(pr.as_slice(), |row| {
                        Ok(LintFinding {
                            id: row.get(0)?,
                            finding_type: row.get(1)?,
                            fact_id_a: row.get(2)?,
                            fact_id_b: row.get(3)?,
                            similarity: row.get(4)?,
                            llm_verdict: row.get(5)?,
                            status: row.get(6)?,
                            user_action: row.get(7)?,
                            created_at: row.get(8)?,
                            resolved_at: row.get(9)?,
                        })
                    }) {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!(error = %e, "ListLintFindings query failed");
                            return Vec::new();
                        }
                    };
                    rows.filter_map(|r| r.ok()).collect()
                })();
                mem_reply!(reply, findings);
            }
            MemoryCommand::ListLintFindingsForDid {
                user_did,
                status,
                limit,
                reply,
            } => {
                let mut sql = String::from(
                    "SELECT f.id, f.finding_type, f.fact_id_a, f.fact_id_b, f.similarity,
                            f.llm_verdict, f.status, f.user_action, f.created_at, f.resolved_at
                     FROM l2_lint_findings f
                     LEFT JOIN l2_experience a ON a.id = f.fact_id_a
                     LEFT JOIN l2_experience b ON b.id = f.fact_id_b
                     WHERE (a.agent_did = ?1 OR b.agent_did = ?1)",
                );
                let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(user_did)];
                if let Some(ref st) = status {
                    params.push(Box::new(st.clone()));
                    sql.push_str(&format!(" AND f.status = ?{}", params.len()));
                }
                params.push(Box::new(limit));
                sql.push_str(&format!(
                    " ORDER BY f.created_at DESC LIMIT ?{}",
                    params.len()
                ));
                let pr: Vec<&dyn rusqlite::ToSql> = params.iter().map(|x| x.as_ref()).collect();
                let findings = (|| -> Vec<LintFinding> {
                    let mut stmt = match conn.prepare(&sql) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(error = %e, "ListLintFindingsForDid prepare failed");
                            return Vec::new();
                        }
                    };
                    let rows = match stmt.query_map(pr.as_slice(), |row| {
                        Ok(LintFinding {
                            id: row.get(0)?,
                            finding_type: row.get(1)?,
                            fact_id_a: row.get(2)?,
                            fact_id_b: row.get(3)?,
                            similarity: row.get(4)?,
                            llm_verdict: row.get(5)?,
                            status: row.get(6)?,
                            user_action: row.get(7)?,
                            created_at: row.get(8)?,
                            resolved_at: row.get(9)?,
                        })
                    }) {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!(error = %e, "ListLintFindingsForDid query failed");
                            return Vec::new();
                        }
                    };
                    rows.filter_map(|r| r.ok()).collect()
                })();
                mem_reply!(reply, findings);
            }
            MemoryCommand::UpdateLintFindingStatus {
                id,
                status,
                user_action,
                reply,
            } => {
                let result = conn.execute(
                    "UPDATE l2_lint_findings SET status = ?1, user_action = ?2, resolved_at = datetime('now') WHERE id = ?3",
                    rusqlite::params![status, user_action, id],
                ).map(|_| ()).map_err(anyhow::Error::from);
                mem_reply!(reply, result);
            }
            MemoryCommand::UpdateLintFindingStatusForDid {
                id,
                user_did,
                status,
                user_action,
                reply,
            } => {
                let result = conn
                    .execute(
                        "UPDATE l2_lint_findings AS f
                     SET status = ?1, user_action = ?2, resolved_at = datetime('now')
                     WHERE f.id = ?3
                       AND f.status = 'pending'
                       AND (
                         EXISTS (
                           SELECT 1 FROM l2_experience a
                           WHERE a.id = f.fact_id_a AND a.agent_did = ?4
                         )
                         OR EXISTS (
                           SELECT 1 FROM l2_experience b
                           WHERE b.id = f.fact_id_b AND b.agent_did = ?4
                         )
                       )",
                        rusqlite::params![status, user_action, id, user_did],
                    )
                    .map(|affected| affected > 0)
                    .map_err(anyhow::Error::from);
                mem_reply!(reply, result);
            }

            // ── Layer C: Companion Growth ──────────────────────────────────
            MemoryCommand::GetCompanionIdentity { reply } => {
                let result = get_companion_identity(&conn);
                mem_reply!(reply, result);
            }
            MemoryCommand::SetCompanionName { name, reply } => {
                let result = set_companion_name(&conn, &name);
                mem_reply!(reply, result);
            }
            MemoryCommand::UpdatePersonalityWeights { weights, reply } => {
                let result = update_personality_weights(&conn, &weights);
                mem_reply!(reply, result);
            }
            MemoryCommand::UpdateCommunicationNotes { notes, reply } => {
                let result = update_communication_notes(&conn, &notes);
                mem_reply!(reply, result);
            }
            MemoryCommand::UpdateGrowthStage { stage, reply } => {
                let result = update_growth_stage(&conn, stage);
                mem_reply!(reply, result);
            }
            MemoryCommand::ListGrowthDimensions { reply } => {
                let result = list_growth_dimensions(&conn);
                mem_reply!(reply, result);
            }
            MemoryCommand::ApplyGrowthEvidence {
                dimension_key,
                delta,
                detail,
                metadata,
                reply,
            } => {
                let result =
                    apply_growth_evidence(&conn, &dimension_key, delta, &detail, &metadata);
                mem_reply!(reply, result);
            }
            MemoryCommand::LogGrowthEvent { event, reply } => {
                let result = log_growth_event(&conn, &event);
                mem_reply!(reply, result);
            }
            MemoryCommand::QueryGrowthLog {
                event_type,
                limit,
                reply,
            } => {
                let result = query_growth_log(&conn, event_type.as_deref(), limit);
                mem_reply!(reply, result);
            }
            MemoryCommand::AdjustPersonalityTrait {
                trait_key,
                value,
                reply,
            } => {
                let result = adjust_personality_trait(&conn, &trait_key, value);
                mem_reply!(reply, result);
            }

            // ── Lifecycle commands ──────────────────────────────────────────
            MemoryCommand::FlushL1ToDb { reply } => {
                let result = flush_l1_to_db(&conn, &l1_caches);
                mem_reply!(reply, result);
            }
            MemoryCommand::RestoreL1FromDb { reply } => {
                let result = restore_l1_from_db(&conn, &mut l1_caches, l1_max_size);
                mem_reply!(reply, result);
            }
            MemoryCommand::RebuildHnsw { hnsw, reply } => {
                tracing::info!(
                    deleted = hnsw.deleted_count(),
                    total = hnsw.len(),
                    "[MemoryActor] Rebuilding HNSW index (deleted ratio exceeded threshold)"
                );
                let result = hnsw.build_from_db(&conn, "");
                if result.is_ok() {
                    hnsw.clear_deleted();
                }
                mem_reply!(reply, result);
            }
            MemoryCommand::LogNotificationFeedback {
                notif_id,
                helpful,
                reply,
            } => {
                let result = conn
                    .execute(
                        "INSERT OR IGNORE INTO notification_feedback (notif_id, helpful) VALUES (?1, ?2)",
                        rusqlite::params![notif_id, helpful as i32],
                    )
                    .map(|_| ())
                    .map_err(|e| anyhow::anyhow!("log notification feedback: {e}"));
                mem_reply!(reply, result);
            }
            MemoryCommand::Shutdown { reply } => {
                if shutdown_reply.is_some() {
                    // Nested Shutdown during drain — just ack it
                    mem_reply!(reply, Ok(()));
                    continue;
                }
                // Close the channel so no new commands can be enqueued
                rx.close();
                // Save the reply sender — we'll respond after draining + flushing
                shutdown_reply = Some(reply);
                tracing::info!("shutdown requested, draining queued commands");
                // Continue the loop to process remaining queued commands
                continue;
            }
        }
    }

    // The loop exited: either shutdown drain completed or channel was dropped.
    // Perform the final flush + WAL checkpoint.
    let flush_result = flush_l1_to_db(&conn, &l1_caches);
    if let Err(ref e) = flush_result {
        tracing::warn!(error = %e, "L1 flush failed during shutdown");
    }
    // WAL checkpoint: force all WAL data into the main database file
    if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
        tracing::warn!(error = %e, "WAL checkpoint failed");
    }
    if let Some(reply) = shutdown_reply {
        mem_reply!(reply, flush_result.map(|_| ()));
    }
}
