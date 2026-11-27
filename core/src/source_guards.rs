//! Guards for persisting facts derived from partial or unconfirmed sources.

pub(crate) const UNCONFIRMED_EMAIL_SOURCE_ERROR: &str = "unconfirmed email preview/truncated source must not be stored as a fact; retrieve the full body/attachments or record only an explicit follow-up";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EmailSourceWriteKind {
    DeterministicFact,
    FollowUpAllowed,
}

pub(crate) fn validate_email_source_write(
    fields: &[&str],
    kind: EmailSourceWriteKind,
) -> Result<(), &'static str> {
    if fields_contain_any(fields, UNREAD_ATTACHMENT_MARKERS) {
        if fields_contain_any(fields, CONFIRMED_ATTACHMENT_MARKERS) {
            return Ok(());
        }
        if kind == EmailSourceWriteKind::FollowUpAllowed
            && fields_contain_any(fields, EMAIL_FOLLOW_UP_OR_UNCERTAINTY_MARKERS)
        {
            return Ok(());
        }
        return Err(UNCONFIRMED_EMAIL_SOURCE_ERROR);
    }
    if fields_contain_any(fields, HIDDEN_EMAIL_CONTENT_MARKERS) {
        if kind == EmailSourceWriteKind::FollowUpAllowed
            && fields_contain_any(fields, EMAIL_FOLLOW_UP_OR_UNCERTAINTY_MARKERS)
        {
            return Ok(());
        }
        return Err(UNCONFIRMED_EMAIL_SOURCE_ERROR);
    }
    if !fields_contain_any(fields, UNCONFIRMED_EMAIL_MARKERS) {
        return Ok(());
    }
    if fields_contain_any(fields, CONFIRMED_EMAIL_MARKERS) {
        return Ok(());
    }
    if kind == EmailSourceWriteKind::FollowUpAllowed
        && fields_contain_any(fields, EMAIL_FOLLOW_UP_OR_UNCERTAINTY_MARKERS)
    {
        return Ok(());
    }
    Err(UNCONFIRMED_EMAIL_SOURCE_ERROR)
}

fn fields_contain_any(fields: &[&str], needles: &[&str]) -> bool {
    fields.iter().any(|field| text_contains_any(field, needles))
}

fn text_contains_any(text: &str, needles: &[&str]) -> bool {
    let lower = text.to_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

const UNCONFIRMED_EMAIL_MARKERS: &[&str] = &[
    "(no text body)",
    "\"body_status\":\"no_text_body\"",
    "\"body_status\":\"not_requested\"",
    "\"body_status\":\"sanitized_hidden_stripped\"",
    "\"body_status\":\"truncated\"",
    "body_status=no_text_body",
    "body_status=not_requested",
    "body_status=sanitized_hidden_stripped",
    "body_status=truncated",
    "body_status: no_text_body",
    "body_status: not_requested",
    "body_status: sanitized_hidden_stripped",
    "body_status: truncated",
    "hidden_content_stripped",
    "hidden content stripped",
    "email preview",
    "has_attachments",
    "message snippet",
    "no_text_body",
    "snippet preview",
    "snippet_is_preview",
    "snippet_warning",
    "truncated",
    "邮件预览",
    "预览文本",
    "正文为空",
    "未读附件",
    "附件未读",
    "截断",
];

const CONFIRMED_EMAIL_MARKERS: &[&str] = &[
    "\"body_status\":\"complete\"",
    "body_status complete",
    "body_status=complete",
    "body_status: complete",
    "confirmed by user",
    "content_mode=full",
    "content_mode: full",
    "full body",
    "user confirmed",
    "完整正文",
    "完整邮件",
    "用户已确认",
    "用户确认",
];

const UNREAD_ATTACHMENT_MARKERS: &[&str] = &[
    "attachment unread",
    "attachments unread",
    "attachments not read",
    "attachment_unread",
    "unread attachment",
    "unread attachments",
    "未读附件",
    "附件未读",
    "附件未查看",
    "附件未确认",
];

const HIDDEN_EMAIL_CONTENT_MARKERS: &[&str] = &[
    "\"body_status\":\"sanitized_hidden_stripped\"",
    "body_status=sanitized_hidden_stripped",
    "body_status: sanitized_hidden_stripped",
    "hidden_content_stripped",
    "hidden content stripped",
];

const CONFIRMED_ATTACHMENT_MARKERS: &[&str] = &[
    "attachment checked",
    "attachment content read",
    "attachment inspected",
    "attachments checked",
    "attachments inspected",
    "confirmed by user",
    "pdf read",
    "user confirmed",
    "附件内容已读取",
    "附件已查看",
    "附件已确认",
    "用户已确认",
    "用户确认",
];

const EMAIL_FOLLOW_UP_OR_UNCERTAINTY_MARKERS: &[&str] = &[
    "check attachment",
    "do not infer",
    "fetch full",
    "inspect attachment",
    "need full body",
    "needs full body",
    "read full",
    "requires confirmation",
    "retrieve full",
    "uncertain",
    "verify",
    "不确定",
    "不要推断",
    "待确认",
    "未确认",
    "查附件",
    "查看附件",
    "核对附件",
    "获取完整",
    "读取完整",
    "需完整正文",
    "需确认",
    "需要完整正文",
    "需要确认",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    #[test]
    fn rejects_deterministic_fact_from_message_snippet() {
        let err = validate_email_source_write(
            &[
                "Retainer is enough to file claim",
                "source_summary: message snippet preview, body_status=no_text_body",
            ],
            EmailSourceWriteKind::DeterministicFact,
        )
        .unwrap_err();

        assert_eq!(err, UNCONFIRMED_EMAIL_SOURCE_ERROR);
    }

    #[test]
    fn allows_follow_up_from_message_snippet() {
        validate_email_source_write(
            &[
                "查附件和完整正文",
                "邮件预览文本显示 retainer，但未确认；需要完整正文",
            ],
            EmailSourceWriteKind::FollowUpAllowed,
        )
        .unwrap();
    }

    #[test]
    fn allows_confirmed_full_body_fact() {
        validate_email_source_write(
            &[
                "Lawyer requested additional wire",
                "source_summary: message get_message content_mode=full body_status=complete",
            ],
            EmailSourceWriteKind::DeterministicFact,
        )
        .unwrap();
    }

    #[test]
    fn full_body_does_not_confirm_unread_attachments() {
        let err = validate_email_source_write(
            &[
                "PDF comments approve filing",
                "source_summary: content_mode=full body_status=complete; unread attachments present",
            ],
            EmailSourceWriteKind::DeterministicFact,
        )
        .unwrap_err();

        assert_eq!(err, UNCONFIRMED_EMAIL_SOURCE_ERROR);
    }

    #[test]
    fn sanitized_hidden_content_does_not_count_as_verbatim_full_body() {
        let err = validate_email_source_write(
            &[
                "Billing statement is available",
                "source_summary: message get_message content_mode=full body_status=sanitized_hidden_stripped hidden_content_stripped=true",
            ],
            EmailSourceWriteKind::DeterministicFact,
        )
        .unwrap_err();

        assert_eq!(err, UNCONFIRMED_EMAIL_SOURCE_ERROR);
    }

    #[test]
    fn direct_llm_chat_create_calls_are_allowlisted_for_provider_routing_migration() {
        let mut allowed = BTreeMap::new();
        // Save-time configuration probes intentionally bypass provider routing:
        // a working fallback must never certify the requested API path. This
        // owner-triggered call uses fixed test content and a 128-token cap;
        // call_validation_direct_chat_and_search_do_not_accept_fallback_success
        // exercises the actual transport and asserts no fallback is attempted.
        allow_direct_llm(
            &mut allowed,
            "src/orchestrator.rs",
            "let response = self.llm_client.chat().create(request).await?;",
            1,
        );
        // Explicit `setup-auth --provider api-key` validation, before saving.
        // Fixed content, 128-token cap and 70s timeout; deliberately tests the
        // selected API directly so subscriptions/fallback cannot certify it.
        allow_direct_llm(
            &mut allowed,
            "src/main_setup_auth.rs",
            "let response = client.chat().create(request).await?;",
            1,
        );
        allow_direct_llm(
            &mut allowed,
            "src/orchestrator.rs",
            "let response = tokio::time::timeout(Duration::from_secs(30), client.chat().create(request))",
            1,
        );
        allow_direct_llm(
            &mut allowed,
            "src/ux_reviewer.rs",
            "llm_client.chat().create(request),",
            1,
        );
        allow_direct_llm(
            &mut allowed,
            "src/evaluators.rs",
            "tokio::time::timeout(timeout, llm_client.chat().create(request))",
            1,
        );
        allow_direct_llm(
            &mut allowed,
            "src/orchestrator.rs",
            "multiline self.llm_client.chat().create(request)",
            1,
        );

        let mut seen = BTreeMap::<(String, String), usize>::new();
        let src_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        collect_direct_llm_chat_create_calls(&src_root, &src_root, &mut seen)
            .expect("scan source tree");

        let allowed: BTreeMap<(String, String), usize> = allowed
            .into_iter()
            .map(|((path, line), count)| ((path.to_string(), line.to_string()), count))
            .collect();

        assert_eq!(
            seen, allowed,
            "direct async-openai chat calls must be migrated through provider routing or explicitly allowlisted here with migration rationale"
        );

        assert_remaining_direct_llm_calls_are_policy_gated();
    }

    #[test]
    fn raw_llm_chat_http_transports_are_allowlisted_for_provider_routing_migration() {
        let mut allowed = BTreeMap::new();
        allow_direct_llm(
            &mut allowed,
            "src/admin_tests.rs",
            ".route(\"/v1/chat/completions\", axum::routing::post(handle_chat))",
            1,
        );
        allow_direct_llm(
            &mut allowed,
            "src/llm/xai_subscription.rs",
            "\"/chat/completions\",",
            1,
        );
        allow_direct_llm(
            &mut allowed,
            "src/llm/xai_subscription.rs",
            "let url = format!(\"{}/chat/completions\", self.api_base.trim_end_matches('/'));",
            1,
        );
        allow_direct_llm(
            &mut allowed,
            "src/orchestrator.rs",
            "\"{}/chat/completions\",",
            1,
        );
        allow_direct_llm(
            &mut allowed,
            "src/orchestrator.rs",
            ".route(\"/v1/chat/completions\", axum::routing::post(handle_chat))",
            1,
        );
        // Loopback-only mock route in search_setting_probe_validates_normalized_
        // citations_not_executor_json. Exercises the real search adapter and
        // validation path with valid, missing and unsafe citation fixtures;
        // this is a test server, not an additional outbound model transport.
        allow_direct_llm(
            &mut allowed,
            "src/orchestrator.rs",
            "\"/v1/chat/completions\",",
            1,
        );
        allow_direct_llm(
            &mut allowed,
            "src/embedding.rs",
            "let url = format!(\"{}/chat/completions\", self.api_base);",
            1,
        );
        allow_direct_llm(
            &mut allowed,
            "src/orchestrator.rs",
            "let url = format!(\"{}/chat/completions\", self.api_base);",
            3,
        );
        allow_direct_llm(
            &mut allowed,
            "src/orchestrator_tools_web.rs",
            ".post(format!(\"{}/chat/completions\", self.api_base))",
            1,
        );
        // Comment only: names the Decisions path to say it is not a chat
        // completion transport. Present on main before reply streaming.
        allow_direct_llm(
            &mut allowed,
            "src/computer_use/decisions.rs",
            "/// OpenRouter Decisions endpoint (alpha). Not under `/api/v1/chat/completions`.",
            1,
        );

        let mut seen = BTreeMap::<(String, String), usize>::new();
        let src_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        collect_raw_llm_chat_http_transports(&src_root, &src_root, &mut seen)
            .expect("scan source tree");

        let allowed: BTreeMap<(String, String), usize> = allowed
            .into_iter()
            .map(|((path, line), count)| ((path.to_string(), line.to_string()), count))
            .collect();

        assert_eq!(
            seen, allowed,
            "raw /chat/completions transports must be routed, explicitly classified, or kept on this allowlist with migration rationale"
        );

        assert_raw_llm_chat_http_transports_are_classified();
    }

    #[test]
    fn proactive_admin_progress_paths_use_notification_router() {
        let main_rs =
            std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"))
                .expect("read src/main.rs");

        assert!(
            !main_rs.contains("proactive_chat_id(admin_chat_id)"),
            "cron/event-driven admin progress sends must resolve through TelegramNotificationRouter, not topic_router::proactive_chat_id"
        );
        assert!(
            main_rs.contains("fn routed_proactive_progress_chat_id(admin_chat_id: i64) -> i64"),
            "main.rs should keep one explicit helper for proactive progress destination routing"
        );
        assert!(
            main_rs.contains("NotificationClass::RoutineNotification")
                && main_rs.contains("PrivacyLevel::PrivateDmOnly"),
            "proactive progress routing must declare notification class and privacy policy"
        );
    }

    #[test]
    fn persisted_chat_id_sends_route_through_admin_clamp() {
        // Root fix (Codex R3): every outbound Telegram send that reads a
        // PERSISTED chat_id (tasks/deferred_intents — a row
        // created from the defunct linked group carries its negative id) must
        // route through crate::telegram::admin_dm_chat / clamp_chat_to_admin,
        // never a raw ChatId(<row>.chat_id). These are the enumerated sites.
        assert_file_contains_all(
            "src/intent.rs",
            &["crate::telegram::admin_dm_chat(intent.chat_id)"],
        );
        // admin.rs was split into src/admin/ (Phase 0.1, 2026-07-11). Its
        // clamped send site (the SDUI WebApp submission handler in
        // webapp_gateway.rs) was retired with the SDUI pipeline on
        // 2026-09-16, so no positive evidence remains to require there; the
        // negative scan below still covers every submodule of the old file.
        // The raw forms must be gone from those sites.
        let intent_src = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/intent.rs"),
        )
        .expect("read intent.rs");
        assert!(
            !intent_src.contains("ChatId(intent.chat_id)"),
            "intent.rs must not send to a raw persisted ChatId(intent.chat_id)"
        );
        let admin_src = read_admin_module_source();
        assert!(
            !admin_src.contains("ChatId(task.chat_id)"),
            "the admin module must not send to a raw persisted ChatId(task.chat_id)"
        );
        // Also catch the aliased form `let chat_id = task.chat_id; ChatId(chat_id)`
        // (Codex R4): the persisted task id copied into a local must be clamped
        // at the send site, so no raw ChatId(chat_id) send remains in the admin module.
        assert!(
            !admin_src.contains("teloxide::types::ChatId(chat_id)"),
            "the admin module must clamp aliased persisted chat ids via admin_dm_chat, not raw ChatId(chat_id)"
        );
    }

    #[test]
    fn unauthorized_chat_reply_is_gated_to_private_dm_not_groups() {
        // Regression (independent Codex R1, 2026-07-03): after forum routing was
        // removed, resolve_thread_id is always None, so replying "access denied"
        // into an unauthorized GROUP would post plainly into a raw non-admin
        // group (e.g. the defunct linked forum). The rejection branch must gate
        // its reply on msg.chat.is_private() so groups/channels are left
        // silently — never a raw group send.
        assert_file_contains_all(
            "src/telegram.rs",
            &[
                "if msg.chat.is_private() {",
                "[Security] Unauthorized access attempt",
            ],
        );
        // The access_denied reply must sit AFTER the is_private gate in the
        // inbound auth branch — the ONLY error.access_denied send preceding the
        // gate would mean an ungated raw-chat reply.
        let src = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/telegram.rs"),
        )
        .expect("read telegram.rs");
        let gate = src
            .find("if msg.chat.is_private() {")
            .expect("is_private gate present");
        // The inbound-auth access_denied send must come AFTER the gate. The
        // callback-query access_denied elsewhere uses answer_callback_query (a
        // different call) and is unaffected, so keying on send_html_to_topic +
        // error.access_denied uniquely identifies the inbound-auth reply.
        let inbound_denied = src.find(r#"error.access_denied", &locale)"#);
        assert!(
            inbound_denied.map(|pos| pos > gate).unwrap_or(false),
            "the inbound access_denied reply must be inside/after the is_private() gate"
        );
    }

    #[test]
    fn proactive_telegram_sends_do_not_target_raw_admin_chat_id() {
        let src_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut raw_admin_sends = BTreeMap::<(String, String), usize>::new();
        // "admin" is a directory since the Phase 0.1 split (2026-07-11);
        // the collector expands directories to every .rs file inside.
        for file in ["main.rs", "admin", "telegram.rs"] {
            collect_raw_admin_send_message_calls(&src_root, file, &mut raw_admin_sends)
                .expect("scan Telegram send call sites");
        }

        assert!(
            raw_admin_sends.is_empty(),
            "new proactive Telegram sends must resolve through TelegramNotificationRouter or document an explicit reactive/DM exception before targeting admin_chat_id directly: {raw_admin_sends:#?}"
        );
    }

    #[test]
    fn owner_notifications_keep_web_first_mirror_before_telegram_fallback() {
        assert_file_contains_all(
            "src/main.rs",
            &[
                "let alert_memory = memory.clone();",
                "Web action: ask Aegis to run `trigger_corpus_reembed` if you approve re-embedding.",
                "record_owner_web_notice_if_push_delivered(\n                    &alert_memory,",
                "let monitor_memory = memory.clone();",
                "open Web Settings and refresh the Google connection",
                "record_owner_web_notice_if_push_delivered(\n                                &monitor_memory,",
                "record_owner_web_notice_if_push_delivered(\n                                        &promote_alert_memory,",
                "Web actions: ask Aegis to deactivate or retry skill",
                "TelegramHeartbeatHandler::new(tg_bot.bot(), admin_chat_id)\n            .with_memory(memory.clone())",
            ],
        );
        assert_file_contains_all(
            "src/telegram.rs",
            &[
                "pub fn with_memory(mut self, memory: MemoryHandle) -> Self",
                "record_owner_web_notice_if_push_delivered_anchored(\n                    memory,\n                    admin_chat_id,\n                    &alert_msg,",
                "Some(HEARTBEAT_ALERT_ANCHOR_KIND),",
                "[Delivery] Web Push delivered heartbeat alert; Telegram fallback suppressed",
            ],
        );
        assert_file_contains_all(
            "src/intent.rs",
            &[
                "record_owner_web_notice_without_push(\n                    &orchestrator.memory,",
                "crate::admin::try_owner_web_push_notice(",
                "Failed to send error notification for deferred-intent callback",
            ],
        );
        assert_file_contains_all(
            "src/llm_pricing.rs",
            &["record_owner_web_notice_if_push_delivered(\n                        &memory,"],
        );
        assert_file_contains_all(
            "src/self_audit.rs",
            &[
                "record_owner_web_notice_if_push_delivered(&memory, chat_id, msg)",
                "record_owner_web_notice_if_push_delivered(memory, chat_id, &text)",
                "record_owner_web_notice(&ctx.memory, chat_id, &digest)",
            ],
        );
    }

    #[test]
    fn owner_timeline_notices_fan_out_to_web_push() {
        assert_file_contains_all(
            "src/orchestrator.rs",
            &[
                "async fn try_owner_web_push_notice(&self, user_did: &str, message: &str)",
                "crate::admin::try_owner_web_push_notice(&self.memory, user_did, message).await",
                "[Delivery] Web Push delivered owner timeline notice; Telegram fallback suppressed",
                "[Delivery] Web Push delivered notify_user; Telegram fallback suppressed",
            ],
        );
        assert_file_contains_all(
            "src/telegram.rs",
            &[
                "crate::admin::spawn_web_push_owner_notice(memory.clone(), owner_did, text);",
                "crate::admin::try_owner_web_push_notice(memory, &owner_did, &text).await",
            ],
        );
    }

    #[test]
    fn scheduled_briefs_do_not_reintroduce_telegram_webapp_nudges() {
        let main_rs = read_src("main.rs");
        assert!(
            main_rs.contains("finish_scheduled_brief_web_ledger"),
            "scheduled briefs should keep receipt-backed Web owner timeline delivery"
        );
        for forbidden in [
            "scheduled_brief_telegram_nudge",
            "telegram_webapp_fallback_delivery",
            "ready in WebApp",
        ] {
            assert!(
                !main_rs.contains(forbidden),
                "routine scheduled Daily Brief / Evening Review must not create Telegram WebApp nudges; found `{forbidden}`"
            );
        }
    }

    #[test]
    fn profile_reads_retain_heirloom_projector_migration_path() {
        let memory_rs = read_memory_module_source();
        assert!(
            memory_rs.contains("ProfileReadSource::from_env()"),
            "default profile reads must remain source-selectable so Heirloom projection can shadow legacy"
        );
        assert!(
            memory_rs.contains("pub async fn get_user_profile_with_read_source("),
            "profile reads must keep an explicit read-source hook for projector migration tests"
        );
        assert!(
            memory_rs.contains("fn project_user_profile_from_heirloom("),
            "profile reads must keep the Heirloom projection fallback path"
        );
        assert!(
            memory_rs.contains("crate::heirloom::projectors::ProfileReadProjector::new()"),
            "profile reads must derive Heirloom state through the profile projector, not ad hoc legacy SQL"
        );
    }

    #[test]
    fn task_reminder_heirloom_projector_stays_shadow_only() {
        let memory_rs = read_memory_module_source();
        assert!(
            !memory_rs.contains("TaskReminderReadProjector"),
            "task/reminder Heirloom projector must stay out of production task reads until a separate read-migration item"
        );

        let projectors_mod = read_src("heirloom/projectors/mod.rs");
        assert!(
            projectors_mod.contains("pub mod tasks;")
                && projectors_mod.contains("TaskReminderReadProjector"),
            "task/reminder projector should remain available as an explicit shadow projector"
        );
    }

    #[test]
    fn direct_user_profile_sql_reads_are_storage_or_migration_only() {
        let src_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut seen = BTreeMap::<(String, String), usize>::new();
        collect_source_mentions(&src_root, &src_root, "user_profile", &mut seen)
            .expect("scan source tree");

        let allowed: BTreeMap<(String, String), usize> = [
            (
                ("src/profile_registry.rs", "//! Profile Slot Registry — single source of truth for user_profile schema."),
                1,
            ),
            (
                ("src/recall.rs", "///   - topic starts with `\"profile:\"`    → `\"profile\"`  (rare; most profile data lives in user_profile table)"),
                1,
            ),
            (
                ("src/heirloom/projectors/profile.rs", "//! state against the legacy `user_profile` table."),
                1,
            ),
        ]
        .into_iter()
        .map(|((path, line), count)| ((path.to_string(), line.to_string()), count))
        .collect();

        assert_eq!(
            seen, allowed,
            "new user_profile references outside memory storage/migrations must use MemoryHandle profile reads or include an explicit Heirloom projector migration note"
        );

        let tools_rs = read_src("orchestrator_tools_system.rs");
        assert!(
            !tools_rs.contains("FROM user_profile") && !tools_rs.contains("JOIN user_profile"),
            "system tools must not bypass MemoryHandle with direct user_profile SQL reads"
        );
    }

    #[test]
    fn external_action_tools_keep_action_ledger_decisions() {
        for evidence in [
            ActionLedgerDecisionEvidence {
                tool_name: "calendar_create_event",
                schema_builder: "build_calendar_create_event_tool",
                decision: ActionLedgerToolDecision::ReceiptBacked,
                implementation_path: "src/orchestrator_tools_calendar.rs",
                implementation_patterns: &[
                    "\"calendar_create_event\" => execute_calendar_create_event",
                    "ActionKind::CalendarCreate",
                    "calendar_action_id(\"calendar-create\"",
                    "best_effort_calendar_action_started(",
                    "best_effort_calendar_action_executed(",
                    "best_effort_calendar_action_failed(action_id, \"calendar_create_event\"",
                    "external_receipt: Some(format!(",
                    "tool_call_id: Some(format!(\"calendar_{operation}_event\"))",
                ],
            },
            ActionLedgerDecisionEvidence {
                tool_name: "calendar_update_event",
                schema_builder: "build_calendar_update_event_tool",
                decision: ActionLedgerToolDecision::ReceiptBacked,
                implementation_path: "src/orchestrator_tools_calendar.rs",
                implementation_patterns: &[
                    "\"calendar_update_event\" => execute_calendar_update_event",
                    "ActionKind::CalendarUpdate",
                    "calendar_action_id(\"calendar-update\"",
                    "best_effort_calendar_action_started(",
                    "best_effort_calendar_action_executed(",
                    "best_effort_calendar_action_failed(action_id, \"calendar_update_event\"",
                    "external_receipt: Some(format!(",
                    "tool_call_id: Some(format!(\"calendar_{operation}_event\"))",
                ],
            },
            ActionLedgerDecisionEvidence {
                tool_name: "calendar_delete_event",
                schema_builder: "build_calendar_delete_event_tool",
                decision: ActionLedgerToolDecision::ReceiptBacked,
                implementation_path: "src/orchestrator_tools_calendar.rs",
                implementation_patterns: &[
                    "\"calendar_delete_event\" => execute_calendar_delete_event",
                    "ActionKind::CalendarDelete",
                    "calendar_action_id(\"calendar-delete\"",
                    "best_effort_calendar_action_started(",
                    "best_effort_calendar_action_executed(",
                    "best_effort_calendar_action_failed(action_id, \"calendar_delete_event\"",
                    "external_receipt: Some(format!(",
                    "tool_call_id: Some(format!(\"calendar_{operation}_event\"))",
                ],
            },
            ActionLedgerDecisionEvidence {
                tool_name: "send_message",
                schema_builder: "build_send_message_tool",
                decision: ActionLedgerToolDecision::ReceiptBacked,
                implementation_path: "src/telegram.rs",
                implementation_patterns: &[
                    "best_effort_start_telegram_send_ledger(",
                    "best_effort_finish_telegram_send_ledger(",
                    "best_effort_fail_telegram_send_ledger(",
                    "ActionKind::TelegramSend",
                    "external_receipt: Some(format!(\"telegram_message_id:{}\"",
                    "source_event_id: Some(skill_name.to_string())",
                ],
            },
            ActionLedgerDecisionEvidence {
                tool_name: "generate_personal_brief",
                schema_builder: "build_generate_personal_brief_tool",
                decision: ActionLedgerToolDecision::ReadOnlyReport,
                implementation_path: "src/orchestrator_tools_system.rs",
                implementation_patterns: &[
                    "\"generate_personal_brief\" => execute_generate_personal_brief",
                    "build_daily_brief(input, &self.settings.display_language())",
                    "build_evening_review(input, &self.settings.display_language())",
                    "recent_action_ledger_summaries(",
                    "pending_hitl_approval_brief_items(",
                ],
            },
        ] {
            assert_action_ledger_decision(evidence);
        }
    }

    fn allow_direct_llm(
        allowed: &mut BTreeMap<(&'static str, &'static str), usize>,
        path: &'static str,
        line: &'static str,
        count: usize,
    ) {
        allowed.insert((path, line), count);
    }

    fn assert_remaining_direct_llm_calls_are_policy_gated() {
        for evidence in [
            DirectLlmPolicyEvidence {
                path: "src/orchestrator.rs",
                patterns: &[
                    "fn route_summarization_model(",
                    "LlmRouterConfig::from_runtime_settings_for_legacy_chat_path",
                    "LlmTaskClass::Summarization",
                    "let routed_model = route_summarization_model(&client, settings, model)?;",
                ],
            },
            DirectLlmPolicyEvidence {
                path: "src/ux_reviewer.rs",
                patterns: &[
                    "fn route_ux_review_model(",
                    "LlmRouterConfig::from_runtime_settings_for_legacy_chat_path",
                    "LlmTaskClass::ReviewGuard",
                    "async fn create_ux_review_chat_completion(",
                ],
            },
            DirectLlmPolicyEvidence {
                path: "src/evaluators.rs",
                patterns: &[
                    "fn route_review_guard_model(",
                    "LlmRouterConfig::from_runtime_settings_for_legacy_chat_path",
                    "LlmTaskClass::ReviewGuard",
                    "async fn create_review_guard_chat_completion(",
                ],
            },
            DirectLlmPolicyEvidence {
                path: "src/admin_tests.rs",
                patterns: &[
                    "async fn spawn_webapp_mock_runtime(",
                    "WebappMockRuntimeState",
                    ".route(\"/v1/chat/completions\", axum::routing::post(handle_chat))",
                ],
            },
            DirectLlmPolicyEvidence {
                path: "src/llm/xai_subscription.rs",
                patterns: &[
                    "XAI_DEFAULT_API_BASE",
                    "XaiSubscriptionBackendState::Ready",
                    "Grok subscription backend only accepts foreground requests",
                    "embeddings must never route through Grok subscription auth",
                    "xAI OAuth discovery returned untrusted",
                ],
            },
            DirectLlmPolicyEvidence {
                path: "src/orchestrator.rs",
                patterns: &[
                    "try_create_xai_subscription_chat_completion",
                    "provider != LlmProviderKind::XaiSubscription",
                    "load_xai_subscription_credential",
                    "Grok subscription needs owner sign-in. No paid fallback was used.",
                ],
            },
            DirectLlmPolicyEvidence {
                path: "src/orchestrator.rs",
                patterns: &[
                    "fn route_chat_for_legacy_path(",
                    "fn route_chat_for_task_class(",
                    "LlmRouterConfig::from_runtime_settings_for_legacy_chat_path",
                    "pub(crate) async fn create_chat_completion(",
                ],
            },
        ] {
            assert_file_contains_all(evidence.path, evidence.patterns);
        }
    }

    fn assert_raw_llm_chat_http_transports_are_classified() {
        for evidence in [
            DirectLlmPolicyEvidence {
                path: "src/orchestrator.rs",
                patterns: &[
                    "fn route_chat_for_legacy_path(",
                    "fn route_chat_for_task_class(",
                    "LlmRouterConfig::from_runtime_settings_for_legacy_chat_path",
                    "pub(crate) async fn create_chat_completion(",
                    "async fn chat_completion_with_reasoning(",
                ],
            },
            DirectLlmPolicyEvidence {
                path: "src/orchestrator_tools_web.rs",
                patterns: &[
                    "OpenRouter chat completions + modalities",
                    "AEGIS_IMAGEGEN_MODEL",
                    "\"modalities\": [\"image\", \"text\"]",
                ],
            },
            DirectLlmPolicyEvidence {
                path: "src/embedding.rs",
                patterns: &[
                    "Translate a short query to English using a cheap chat model",
                    "Returns `None` if translation fails",
                    "\"google/gemini-2.0-flash-lite-001\"",
                ],
            },
        ] {
            assert_file_contains_all(evidence.path, evidence.patterns);
        }
    }

    struct DirectLlmPolicyEvidence {
        path: &'static str,
        patterns: &'static [&'static str],
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ActionLedgerToolDecision {
        ReceiptBacked,
        ReadOnlyReport,
    }

    struct ActionLedgerDecisionEvidence {
        tool_name: &'static str,
        schema_builder: &'static str,
        decision: ActionLedgerToolDecision,
        implementation_path: &'static str,
        implementation_patterns: &'static [&'static str],
    }

    fn assert_action_ledger_decision(evidence: ActionLedgerDecisionEvidence) {
        let schemas = read_src("tool_schemas.rs");
        assert!(
            schemas.contains(&format!("fn {}(", evidence.schema_builder)),
            "{} must keep an explicit tool schema builder before it can be classified for Action Ledger",
            evidence.tool_name
        );
        assert!(
            schemas.contains(&format!(".name(\"{}\")", evidence.tool_name)),
            "{} must keep a stable tool name for Action Ledger classification",
            evidence.tool_name
        );

        let implementation = read_src(
            evidence
                .implementation_path
                .strip_prefix("src/")
                .unwrap_or(evidence.implementation_path),
        );
        for pattern in evidence.implementation_patterns {
            assert!(
                implementation.contains(pattern),
                "{} classified as {:?} must retain source evidence `{}` in {}",
                evidence.tool_name,
                evidence.decision,
                pattern,
                evidence.implementation_path
            );
        }
    }

    fn assert_file_contains_all(path: &str, patterns: &[&str]) {
        // orchestrator.rs was split into src/orchestrator/ (Phase 0.2,
        // 2026-07-12); guards that scanned the single file scan the whole
        // module directory instead.
        let contents = if path == "src/orchestrator.rs" {
            read_orchestrator_module_source()
        } else if path == "src/admin_tests.rs" {
            read_admin_tests_module_source()
        } else {
            read_src(path.strip_prefix("src/").unwrap_or(path))
        };

        for pattern in patterns {
            assert!(
                contents.contains(pattern),
                "{} must retain provider-routing evidence `{}` for direct async-openai allowlist",
                path,
                pattern
            );
        }
    }

    fn read_src(path: &str) -> String {
        let full_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(path);
        std::fs::read_to_string(&full_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", full_path.display()))
    }

    /// Concatenated source of every file in `src/admin/` — the Phase 0.1
    /// (2026-07-11) split of the old monolithic `admin.rs`. Guards that used
    /// to scan `admin.rs` scan the whole module directory instead.
    fn read_admin_module_source() -> String {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/admin");
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
            .map(|entry| entry.expect("read src/admin entry").path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
            .collect();
        paths.sort();
        let mut sources = String::new();
        for path in paths {
            sources.push_str(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|err| panic!("read {}: {err}", path.display())),
            );
        }
        sources
    }

    /// Concatenated source of every file in `src/orchestrator/` — the Phase 0.2
    /// (2026-07-12) split of the old monolithic `orchestrator.rs`. Guards that
    /// used to scan `orchestrator.rs` scan the whole module directory instead.
    fn read_orchestrator_module_source() -> String {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/orchestrator");
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
            .map(|entry| entry.expect("read src/orchestrator entry").path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
            .collect();
        paths.sort();
        let mut sources = String::new();
        for path in paths {
            sources.push_str(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|err| panic!("read {}: {err}", path.display())),
            );
        }
        sources
    }

    /// Concatenated source of every top-level file in `src/memory/` — the
    /// Phase 0.3 (2026-07-13) split of the old monolithic `memory.rs`. Guards
    /// that used to scan `memory.rs` scan the whole module directory instead.
    fn read_memory_module_source() -> String {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/memory");
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
            .map(|entry| entry.expect("read src/memory entry").path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
            .collect();
        paths.sort();
        let mut sources = String::new();
        for path in paths {
            sources.push_str(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|err| panic!("read {}: {err}", path.display())),
            );
        }
        sources
    }

    /// Concatenated source of every file in `src/admin_tests/` — the Phase 0.5
    /// (2026-07-12) split of the old monolithic `admin_tests.rs`. Guards that
    /// used to scan `admin_tests.rs` scan the whole module directory instead.
    fn read_admin_tests_module_source() -> String {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/admin_tests");
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
            .map(|entry| entry.expect("read src/admin_tests entry").path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
            .collect();
        paths.sort();
        let mut sources = String::new();
        for path in paths {
            sources.push_str(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|err| panic!("read {}: {err}", path.display())),
            );
        }
        sources
    }

    /// Collapse a `src/orchestrator/<sub>.rs` submodule path back to the
    /// historical `src/orchestrator.rs` so LLM-transport allowlists keyed on
    /// the pre-split single-file path still match after the Phase 0.2 split.
    /// Same for `src/admin_tests/<sub>.rs` → `src/admin_tests.rs` after the
    /// Phase 0.5 split (2026-07-12).
    fn normalize_split_module_path(rel: String) -> String {
        if rel.starts_with("src/orchestrator/") && rel.ends_with(".rs") {
            return "src/orchestrator.rs".to_string();
        }
        if rel.starts_with("src/admin_tests/") && rel.ends_with(".rs") {
            return "src/admin_tests.rs".to_string();
        }
        rel
    }

    fn collect_direct_llm_chat_create_calls(
        root: &Path,
        dir: &Path,
        seen: &mut BTreeMap<(String, String), usize>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_direct_llm_chat_create_calls(root, &path, seen)?;
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }

            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel = format!("src/{}", rel.display());
            if rel == "src/source_guards.rs" {
                continue;
            }
            let rel = normalize_split_module_path(rel);
            let contents = std::fs::read_to_string(&path)?;
            let lines: Vec<&str> = contents.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                let trimmed = line.trim();
                if trimmed.contains(".chat().create(") || trimmed.contains(".chat().create_stream(")
                {
                    *seen.entry((rel.clone(), trimmed.to_string())).or_default() += 1;
                }
                if trimmed == ".chat()"
                    && let Some(next) = next_nonempty_trimmed(&lines, index + 1)
                    && (next.starts_with(".create(") || next.starts_with(".create_stream("))
                {
                    let receiver = previous_chat_receiver(&lines, index);
                    let method = next.trim_start_matches('.');
                    let snippet = format!("multiline {receiver}.chat().{method}");
                    *seen.entry((rel.clone(), snippet)).or_default() += 1;
                }
            }
        }
        Ok(())
    }

    fn collect_raw_llm_chat_http_transports(
        root: &Path,
        dir: &Path,
        seen: &mut BTreeMap<(String, String), usize>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_raw_llm_chat_http_transports(root, &path, seen)?;
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }

            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel = format!("src/{}", rel.display());
            if rel == "src/source_guards.rs" {
                continue;
            }
            let rel = normalize_split_module_path(rel);
            let contents = std::fs::read_to_string(&path)?;
            for line in contents.lines() {
                let trimmed = line.trim();
                if trimmed.contains("chat/completions") {
                    *seen.entry((rel.clone(), trimmed.to_string())).or_default() += 1;
                }
            }
        }
        Ok(())
    }

    fn collect_source_mentions(
        root: &Path,
        dir: &Path,
        needle: &str,
        seen: &mut BTreeMap<(String, String), usize>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_source_mentions(root, &path, needle, seen)?;
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }

            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel = format!("src/{}", rel.display());
            if matches!(
                rel.as_str(),
                "src/source_guards.rs" | "src/memory.rs" | "src/memory_migrations.rs"
            ) || rel.starts_with("src/memory/")
                || rel.ends_with("_tests.rs")
            {
                // src/memory/ is the Phase 0.3 (2026-07-13) split of memory.rs;
                // it is the profile storage layer, exempt like the old memory.rs.
                continue;
            }
            let contents = std::fs::read_to_string(&path)?;
            for line in contents.lines() {
                let trimmed = line.trim();
                if trimmed.contains(needle) && !trimmed.contains("get_user_profile") {
                    *seen.entry((rel.clone(), trimmed.to_string())).or_default() += 1;
                }
            }
        }
        Ok(())
    }

    fn collect_raw_admin_send_message_calls(
        src_root: &Path,
        file: &str,
        seen: &mut BTreeMap<(String, String), usize>,
    ) -> std::io::Result<()> {
        let path = src_root.join(file);
        if path.is_dir() {
            // Module directory (e.g. src/admin/ after the Phase 0.1 split):
            // scan every .rs file inside it.
            for entry in std::fs::read_dir(&path)? {
                let entry = entry?;
                if entry.path().extension().and_then(|ext| ext.to_str()) == Some("rs") {
                    let rel = format!("{file}/{}", entry.file_name().to_string_lossy());
                    collect_raw_admin_send_message_calls(src_root, &rel, seen)?;
                }
            }
            return Ok(());
        }
        let contents = std::fs::read_to_string(&path)?;
        let rel = format!("src/{file}");
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.contains("send_message(admin_chat_id")
                || trimmed.contains("send_message(ChatId(admin_chat_id")
                || trimmed.contains(".send_message(admin_chat_id")
                || trimmed.contains(".send_message(ChatId(admin_chat_id")
            {
                *seen.entry((rel.clone(), trimmed.to_string())).or_default() += 1;
            }
        }
        Ok(())
    }

    fn next_nonempty_trimmed(lines: &[&str], start: usize) -> Option<String> {
        lines
            .iter()
            .skip(start)
            .map(|line| line.trim())
            .find(|line| !line.is_empty())
            .map(str::to_string)
    }

    fn previous_chat_receiver(lines: &[&str], chat_line_index: usize) -> String {
        let mut parts = Vec::new();
        for line in lines[..chat_line_index].iter().rev() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(stripped) = trimmed.strip_prefix('.') {
                parts.push(stripped.trim_end_matches(',').to_string());
                continue;
            }
            if trimmed.starts_with("return ") {
                parts.push(trimmed.trim_start_matches("return ").to_string());
            } else {
                parts.push(trimmed.trim_end_matches(',').to_string());
            }
            break;
        }
        parts.reverse();
        parts.join(".")
    }
}
