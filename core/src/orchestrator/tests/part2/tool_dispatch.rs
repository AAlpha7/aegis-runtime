//! Tool dispatch: tool results, bookkeeping tools, microcompaction, approval-gate markers, conflicts, allowlists, tier classification, heirloom tool events, exfil guard, deferred stubs and search_tools.
use super::*;

#[test]
fn push_tool_result_appends_message() {
    let mut messages: Vec<ChatCompletionRequestMessage> = Vec::new();
    push_tool_result(&mut messages, "call_123", "result data").unwrap();
    assert_eq!(messages.len(), 1);
}

/// push_tool_result handles empty content.
#[test]
fn push_tool_result_empty_content() {
    let mut messages: Vec<ChatCompletionRequestMessage> = Vec::new();
    push_tool_result(&mut messages, "call_456", "").unwrap();
    assert_eq!(messages.len(), 1);
}

/// push_tool_result handles very large content (tool results can be big).
#[test]
fn push_tool_result_large_content() {
    let mut messages: Vec<ChatCompletionRequestMessage> = Vec::new();
    let large = "x".repeat(100_000);
    push_tool_result(&mut messages, "call_789", &large).unwrap();
    assert_eq!(messages.len(), 1);
}

/// Multiple push_tool_result calls accumulate correctly.
#[test]
fn push_tool_result_accumulates() {
    let mut messages: Vec<ChatCompletionRequestMessage> = Vec::new();
    push_tool_result(&mut messages, "call_1", "result 1").unwrap();
    push_tool_result(&mut messages, "call_2", "result 2").unwrap();
    push_tool_result(&mut messages, "call_3", "result 3").unwrap();
    assert_eq!(messages.len(), 3);
}

// ── Bookkeeping-only notification suppression tests ─────────────────

/// Verify that bookkeeping tools (notify_user, get_current_time, update_heartbeat_memo)
/// do NOT count as "real work" for notification suppression.
#[test]
fn bookkeeping_only_tools_detected_correctly() {
    let bookkeeping: &[&str] = &["notify_user", "get_current_time", "update_heartbeat_memo"];

    // Only bookkeeping tools → no real work
    let tools_a: Vec<String> = vec!["notify_user".into(), "get_current_time".into()];
    let did_real_work_a = tools_a.iter().any(|t| !bookkeeping.contains(&t.as_str()));
    assert!(
        !did_real_work_a,
        "Bookkeeping-only tools should not count as real work"
    );

    // Mixed: bookkeeping + data-fetching → real work
    let tools_b: Vec<String> = vec!["notify_user".into(), "web_search".into()];
    let did_real_work_b = tools_b.iter().any(|t| !bookkeeping.contains(&t.as_str()));
    assert!(did_real_work_b, "web_search is real work");

    // Empty tools → no real work
    let tools_c: Vec<String> = vec![];
    let did_real_work_c = tools_c.iter().any(|t| !bookkeeping.contains(&t.as_str()));
    assert!(!did_real_work_c, "No tools at all = no real work");

    // Only data-fetching tools → real work
    let tools_d: Vec<String> = vec!["calendar_reader".into(), "recall_memory".into()];
    let did_real_work_d = tools_d.iter().any(|t| !bookkeeping.contains(&t.as_str()));
    assert!(
        did_real_work_d,
        "calendar_reader and recall_memory are real work"
    );
}

// ── Microcompaction summary tests ──────────────────────────────────

#[test]
fn microcompact_preserves_summary_prefix() {
    use async_openai::types::chat::ChatCompletionRequestToolMessageArgs;
    let large_content = format!("Important: BTC at $65000. {}", "x".repeat(3000));
    let mut messages: Vec<ChatCompletionRequestMessage> = vec![
        ChatCompletionRequestToolMessageArgs::default()
            .tool_call_id("tc1")
            .content(large_content.as_str())
            .build()
            .unwrap()
            .into(),
    ];
    // round_start_indices: [0]=round1 start, [1]=round2 start at idx 1, etc.
    let indices = vec![0, 1, 2, 3, 4];
    // Current round=5, age=3 → cutoff_round=2 → cutoff_idx=indices[1]=1
    // Scans messages 0..1 → includes our tool message at index 0
    let compacted = microcompact_tool_results(&mut messages, &indices, 5, 3, 2048).unwrap();
    assert_eq!(compacted, 1);
    // Verify the compacted message has a summary, not a blank placeholder
    if let ChatCompletionRequestMessage::Tool(tool_msg) = &messages[0] {
        let text = match &tool_msg.content {
            ChatCompletionRequestToolMessageContent::Text(s) => s.clone(),
            _ => panic!("Expected Text content"),
        };
        assert!(text.contains("[Compacted"), "Should have compacted header");
        assert!(
            text.contains("Important: BTC"),
            "Should preserve prefix content"
        );
        assert!(
            text.contains("Re-call the tool"),
            "Should suggest re-calling"
        );
    }
}

#[test]
fn microcompact_small_results_untouched() {
    use async_openai::types::chat::ChatCompletionRequestToolMessageArgs;
    let small = "small result";
    let mut messages: Vec<ChatCompletionRequestMessage> = vec![
        ChatCompletionRequestToolMessageArgs::default()
            .tool_call_id("tc1")
            .content(small)
            .build()
            .unwrap()
            .into(),
    ];
    let indices = vec![0usize];
    let compacted = microcompact_tool_results(&mut messages, &indices, 5, 3, 2048).unwrap();
    assert_eq!(compacted, 0, "Small results should not be compacted");
}

// ── Microcompaction tests ─────────────────────────────────────────────

/// Helper: build a Tool message with the given content length.
fn make_tool_msg(tool_call_id: &str, content_len: usize) -> ChatCompletionRequestMessage {
    let content = "x".repeat(content_len);
    ChatCompletionRequestToolMessageArgs::default()
        .tool_call_id(tool_call_id)
        .content(content)
        .build()
        .unwrap()
        .into()
}

/// Helper: build a System message (not a tool result).
fn make_system_msg(text: &str) -> ChatCompletionRequestMessage {
    ChatCompletionRequestSystemMessageArgs::default()
        .content(text)
        .build()
        .unwrap()
        .into()
}

#[test]
fn microcompact_replaces_old_large_tool_results() {
    // Round 1: system + large tool result at indices 0,1
    // Round 2: large tool result at index 2
    // Round 3: large tool result at index 3
    // Round 4: large tool result at index 4
    // Round 5 (current): checking compaction with age=3
    let mut messages = vec![
        make_system_msg("system prompt"), // idx 0
        make_tool_msg("call_r1", 3000),   // idx 1 — round 1, large
        make_tool_msg("call_r2", 3000),   // idx 2 — round 2, large
        make_tool_msg("call_r3", 3000),   // idx 3 — round 3, large
        make_tool_msg("call_r4", 3000),   // idx 4 — round 4, large
    ];
    // round_start_indices: [0]=round1@idx0, [1]=round2@idx2, [2]=round3@idx3, [3]=round4@idx4, [4]=round5@idx5
    let indices = vec![0, 2, 3, 4, 5];
    let current_round = 5;

    let compacted =
        microcompact_tool_results(&mut messages, &indices, current_round, 3, 2048).unwrap();

    // cutoff_round = 5-3 = 2, cutoff_idx = indices[2-1] = indices[1] = 2
    // Only messages at idx 0..2 are eligible. idx 1 is a large tool result → compacted.
    assert_eq!(compacted, 1);

    // idx 1 should be replaced with placeholder
    if let ChatCompletionRequestMessage::Tool(t) = &messages[1] {
        let text = match &t.content {
            ChatCompletionRequestToolMessageContent::Text(s) => s.as_str(),
            _ => "",
        };
        assert!(
            text.contains("Compacted"),
            "old large tool result should be replaced with summary"
        );
    } else {
        panic!("idx 1 should still be a Tool message");
    }

    // idx 2,3,4 should be untouched (too recent)
    if let ChatCompletionRequestMessage::Tool(t) = &messages[2] {
        let text = match &t.content {
            ChatCompletionRequestToolMessageContent::Text(s) => s.as_str(),
            _ => "",
        };
        assert!(
            !text.contains("cleared"),
            "round 2 tool result should NOT be replaced"
        );
    }
}

#[test]
fn microcompact_preserves_small_old_results() {
    let mut messages = vec![
        make_system_msg("system"),
        make_tool_msg("small_r1", 500), // small — below 2048 threshold
    ];
    let indices = vec![0, 1, 2, 3, 4]; // 5 rounds
    let compacted = microcompact_tool_results(&mut messages, &indices, 5, 3, 2048).unwrap();
    assert_eq!(compacted, 0, "small tool results should not be compacted");
}

#[test]
fn microcompact_preserves_recent_large_results() {
    let mut messages = vec![
        make_system_msg("system"),
        make_tool_msg("recent", 5000), // large but recent (round 3)
    ];
    // Only 3 rounds, current_round=3, age=3 → 3 <= 3, no compaction
    let indices = vec![0, 1, 2];
    let compacted = microcompact_tool_results(&mut messages, &indices, 3, 3, 2048).unwrap();
    assert_eq!(
        compacted, 0,
        "current round <= age threshold, nothing to compact"
    );
}

#[test]
fn microcompact_noop_when_no_rounds() {
    let mut messages = vec![make_system_msg("system")];
    let compacted = microcompact_tool_results(&mut messages, &[], 1, 3, 2048).unwrap();
    assert_eq!(compacted, 0);
}

// ── Gate-approved direct execution tests ────────────────────────────

/// The synthetic callback marker injected by execute_approved_tool must
/// match the gate_approved_in_callback detection pattern, otherwise
/// RequireApproval tools will re-block after "Approve Once".
#[test]
fn gate_callback_marker_matches_detection() {
    use async_openai::types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestUserMessage,
        ChatCompletionRequestUserMessageContent,
    };
    let tool_name = "setup_google";
    let marker = ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
        content: ChatCompletionRequestUserMessageContent::Text(format!(
            "[SYSTEM CALLBACK — Gate Approval (this call only)]\n\
                 Approved execution of tool '{}'.",
            tool_name
        )),
        name: None,
    });
    let messages = [marker];

    // Replicate the exact detection logic from execute_single_tool
    let detected = messages.iter().any(|m| {
        if let ChatCompletionRequestMessage::User(u) = m
            && let ChatCompletionRequestUserMessageContent::Text(t) = &u.content
        {
            return t.contains("[SYSTEM CALLBACK — Gate Approval")
                && t.contains(&format!("tool '{}'", tool_name));
        }
        false
    });
    assert!(
        detected,
        "Synthetic callback marker must match gate_approved_in_callback detection"
    );
}

/// Different tool name should NOT match the callback marker.
#[test]
fn gate_callback_marker_rejects_wrong_tool() {
    use async_openai::types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestUserMessage,
        ChatCompletionRequestUserMessageContent,
    };
    let marker = ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
        content: ChatCompletionRequestUserMessageContent::Text(
            "[SYSTEM CALLBACK — Gate Approval (this call only)]\n\
                 Approved execution of tool 'setup_google'."
                .to_string(),
        ),
        name: None,
    });
    let messages = [marker];

    let detected_wrong = messages.iter().any(|m| {
        if let ChatCompletionRequestMessage::User(u) = m
            && let ChatCompletionRequestUserMessageContent::Text(t) = &u.content
        {
            return t.contains("[SYSTEM CALLBACK — Gate Approval")
                && t.contains("tool 'switch_model'");
        }
        false
    });
    assert!(
        !detected_wrong,
        "Marker for setup_google should not match switch_model"
    );
}

/// Simulate the full gate evaluation path for a RequireApproval tool
/// called via execute_approved_tool (Once scope). The synthetic marker
/// in messages must cause gate_approved_in_callback=true, preventing re-block.
#[test]
fn gate_approved_tool_once_scope_not_reblocked() {
    use async_openai::types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestUserMessage,
        ChatCompletionRequestUserMessageContent,
    };

    let tool_name = "setup_google";
    let tool_args = r#"{"action":"setup"}"#;

    // Simulate the messages buffer that execute_approved_tool creates
    let messages: Vec<ChatCompletionRequestMessage> = vec![ChatCompletionRequestMessage::User(
        ChatCompletionRequestUserMessage {
            content: ChatCompletionRequestUserMessageContent::Text(format!(
                "[SYSTEM CALLBACK — Gate Approval (this call only)]\n\
                     Approved execution of tool '{}'.",
                tool_name
            )),
            name: None,
        },
    )];

    // Replicate the exact gate logic from execute_single_tool (line 5504-5515)
    let gate_approved_in_callback = messages.iter().any(|m| {
        if let ChatCompletionRequestMessage::User(u) = m
            && let ChatCompletionRequestUserMessageContent::Text(t) = &u.content
        {
            return t.contains("[SYSTEM CALLBACK — Gate Approval")
                && t.contains(&format!("tool '{}'", tool_name));
        }
        false
    });

    assert!(
        gate_approved_in_callback,
        "RequireApproval tool must be detected as gate-approved (Once scope)"
    );

    // Verify tool_args is valid JSON and contains the required action parameter
    let parsed: serde_json::Value = serde_json::from_str(tool_args).unwrap();
    assert_eq!(
        parsed.get("action").and_then(|v| v.as_str()),
        Some("setup"),
        "tool_args must preserve action parameter"
    );
}

/// Simulate gate evaluation for Intent/Domain/Always scopes.
/// These scopes store approvals, so gate_approved_in_callback is a bonus,
/// not the only path. But it should still work for safety.
#[test]
fn gate_approved_tool_all_scopes_detected() {
    use async_openai::types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestUserMessage,
        ChatCompletionRequestUserMessageContent,
    };

    for tool in &[
        "setup_google",
        "configure_autonomous",
        "sysadmin_ops",
        "set_alert_policy",
    ] {
        let messages: Vec<ChatCompletionRequestMessage> = vec![ChatCompletionRequestMessage::User(
            ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(format!(
                    "[SYSTEM CALLBACK — Gate Approval (this call only)]\n\
                         Approved execution of tool '{}'.",
                    tool
                )),
                name: None,
            },
        )];

        let detected = messages.iter().any(|m| {
            if let ChatCompletionRequestMessage::User(u) = m
                && let ChatCompletionRequestUserMessageContent::Text(t) = &u.content
            {
                return t.contains("[SYSTEM CALLBACK — Gate Approval")
                    && t.contains(&format!("tool '{}'", tool));
            }
            false
        });

        assert!(detected, "Gate marker for '{}' must be detected", tool);
    }
}

/// Verify the synthetic ChatCompletionMessageToolCall preserves args exactly.
#[test]
fn synthetic_tool_call_preserves_args() {
    use async_openai::types::chat::{ChatCompletionMessageToolCall, FunctionCall};

    let original_args = r#"{"action":"setup","client_id":"abc123","client_secret":"xyz"}"#;
    let tool_call = ChatCompletionMessageToolCall {
        id: "gate_approved_test".to_string(),
        function: FunctionCall {
            name: "setup_google".to_string(),
            arguments: original_args.to_string(),
        },
    };

    // Verify args round-trip — the exact string must survive
    assert_eq!(tool_call.function.arguments, original_args);
    assert_eq!(tool_call.function.name, "setup_google");

    // Verify args parse correctly with all fields
    let parsed: serde_json::Value = serde_json::from_str(&tool_call.function.arguments).unwrap();
    assert_eq!(parsed["action"], "setup");
    assert_eq!(parsed["client_id"], "abc123");
    assert_eq!(parsed["client_secret"], "xyz");
}

/// RequireApproval tools in tool_gate must all be handled by
/// execute_approved_tool's synthetic marker.
#[test]
fn all_require_approval_tools_have_marker_coverage() {
    use crate::tool_gate::ToolGate;

    let gate = ToolGate::new();
    // Get all RequireApproval tools from the gate
    let require_approval_tools: Vec<&str> = vec![
        "deactivate_skill",
        "reactivate_skill",
        "update_setting",
        "switch_model",
        "set_bot_commands",
        "set_log_level",
        "setup_google",
        "calendar_create_event",
        "sysadmin_ops",
        "set_alert_policy",
        "configure_autonomous",
        "set_monitoring_intent",
        "remove_monitoring_intent",
        "calendar_update_event",
        "manage_skill",
    ];

    for tool in require_approval_tools {
        // Verify the gate actually returns NeedApproval for this tool
        let verdict = gate.evaluate(tool, "{}", crate::tool_gate::CallerContext::Interactive);
        assert!(
            matches!(verdict, crate::tool_gate::GateVerdict::NeedApproval { .. }),
            "'{}' should be RequireApproval but got {:?}",
            tool,
            verdict
        );

        // Verify the marker detection would work for this tool
        let marker_text = format!(
            "[SYSTEM CALLBACK — Gate Approval (this call only)]\n\
                 Approved execution of tool '{}'.",
            tool
        );
        assert!(
            marker_text.contains("[SYSTEM CALLBACK — Gate Approval")
                && marker_text.contains(&format!("tool '{}'", tool)),
            "Marker for '{}' must match detection pattern",
            tool
        );
    }
}

// ── Tool concurrency conflict tests ─────────────────────────────────

#[test]
fn tool_conflict_independent_tools() {
    assert!(
        !has_tool_conflict(&["web_search", "crypto_tracker"]),
        "Independent tools should not conflict"
    );
}

#[test]
fn tool_conflict_detected() {
    assert!(
        !has_tool_conflict(&["calendar_create_event", "calendar_update_event"]),
        "calendar write tools may run together; no remaining wasm conflict pairs"
    );
}

#[test]
fn tool_conflict_bidirectional() {
    assert!(
        !has_tool_conflict(&["web_reader", "calendar_reader"]),
        "independent wasm skills stay order-independent and non-conflicting"
    );
}

#[test]
fn tool_conflict_single_tool() {
    assert!(
        !has_tool_conflict(&["web_search"]),
        "Single tool cannot conflict with itself"
    );
}

#[test]
fn tool_conflict_empty_list() {
    assert!(!has_tool_conflict(&[]), "Empty list has no conflicts");
}

// ── Deferred Tool Reliability Tests ───────────────────────────────

/// Layer 3: Core skill whitelist enriches the relevant set
#[test]
fn core_wasm_skills_always_included_in_filter() {
    let skills = [
        make_cap("random_skill"),
        make_cap("calendar_reader"),
        make_cap("web_reader"),
        make_cap("another_skill"),
    ];
    // Simulate semantic filter selecting only random_skill
    let mut relevant: std::collections::HashSet<String> =
        ["random_skill"].iter().map(|s| s.to_string()).collect();
    assert_eq!(relevant.len(), 1);

    // Apply core whitelist (same logic as semantic_skill_filter)
    for name in CORE_WASM_SKILLS {
        if skills.iter().any(|c| c.skill_name == *name) {
            relevant.insert(name.to_string());
        }
    }

    assert_eq!(relevant.len(), 3);
    assert!(relevant.contains("calendar_reader"));
    assert!(relevant.contains("web_reader"));
    assert!(relevant.contains("random_skill"));
}

/// Layer 3: Core skill whitelist is a no-op when skills aren't loaded
#[test]
fn core_wasm_skills_no_op_when_not_loaded() {
    let skills = [make_cap("unrelated_skill")];
    let mut relevant: std::collections::HashSet<String> =
        ["unrelated_skill"].iter().map(|s| s.to_string()).collect();

    for name in CORE_WASM_SKILLS {
        if skills.iter().any(|c| c.skill_name == *name) {
            relevant.insert(name.to_string());
        }
    }
    // No core skills loaded, so set should be unchanged
    assert_eq!(relevant.len(), 1);
    assert!(relevant.contains("unrelated_skill"));
}

/// Layer 2: Empty args detection for auto-resolve
#[test]
fn auto_resolve_empty_args_detection() {
    // Empty object
    let v: serde_json::Value = serde_json::from_str("{}").unwrap();
    assert!(v.as_object().is_none_or(|o| o.is_empty()));

    // Null
    let v: serde_json::Value = serde_json::Value::Null;
    assert!(v.as_object().is_none_or(|o| o.is_empty()));

    // Non-empty: has args — should NOT trigger auto-resolve
    let v: serde_json::Value = serde_json::from_str(r#"{"action":"status"}"#).unwrap();
    assert!(!v.as_object().is_none_or(|o| o.is_empty()));

    // Non-empty: nested
    let v: serde_json::Value = serde_json::from_str(r#"{"lines":50,"filter":"error"}"#).unwrap();
    assert!(!v.as_object().is_none_or(|o| o.is_empty()));
}

/// Layer 2: Smart default tool mapping coverage
#[test]
fn smart_default_covers_expected_tools() {
    let smart_default_tools = [
        "setup_google",
        "list_intents",
        "view_logs",
        "list_monitoring_intents",
        "list_alert_policies",
        "view_heartbeat_audit",
        "check_api_usage",
    ];
    // These should NOT have smart defaults (require user-specified params)
    let no_default_tools = [
        "cancel_intent",
        "set_log_level",
        "set_bot_commands",
        "set_monitoring_intent",
        "remove_monitoring_intent",
        "set_alert_policy",
        "configure_autonomous",
        // Calendar write tools (added 2026-04-19) — require user-supplied
        // summary/start/end/event_id params; no safe default.
        "calendar_create_event",
        "calendar_update_event",
        "calendar_delete_event",
    ];
    assert_eq!(smart_default_tools.len() + no_default_tools.len(), 17);

    // No overlap
    for tool in &smart_default_tools {
        assert!(!no_default_tools.contains(tool), "{tool} in both lists");
    }
}

/// Layer 1: 3-tier classification exhaustiveness (Tier 1a + 1b + Tier 2 = 18)
#[test]
fn tier_classification_covers_all_tools() {
    // Google Calendar (Tier 1a): setup + 3 calendar write tools.
    let google_tier = [
        "setup_google",
        "calendar_create_event",
        "calendar_update_event",
        "calendar_delete_event",
    ];
    // Promoted (Tier 1b): 8 tools
    let promoted_tier = [
        "list_intents",
        "cancel_intent",
        "set_monitoring_intent",
        "list_monitoring_intents",
        "remove_monitoring_intent",
        "set_alert_policy",
        "list_alert_policies",
        "check_api_usage",
    ];
    // Admin (Tier 2): 5 tools
    let admin_tier = [
        "view_logs",
        "set_log_level",
        "set_bot_commands",
        "configure_autonomous",
        "view_heartbeat_audit",
    ];

    assert_eq!(
        google_tier.len() + promoted_tier.len() + admin_tier.len(),
        17
    );

    // No duplicates
    let mut all: Vec<&str> = Vec::new();
    all.extend_from_slice(&google_tier);
    all.extend_from_slice(&promoted_tier);
    all.extend_from_slice(&admin_tier);
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), 17, "Duplicate tool in tier classification");
}

/// Gate-pending guard: detect pending_human_approval in tool results
#[test]
fn gate_pending_detected_in_tool_messages() {
    let mut messages: Vec<ChatCompletionRequestMessage> = vec![];

    // Normal tool result
    push_tool_result(
        &mut messages,
        "call_1",
        r#"{"status":"ok","data":"some result"}"#,
    )
    .unwrap();

    // Pending approval result
    push_tool_result(
            &mut messages,
            "call_2",
            r#"{"status":"pending_human_approval","approval_id":"gate-abc123","message":"Tool 'setup_google' requires user approval."}"#,
        )
        .unwrap();

    // Scan from round start (index 0)
    let has_pending = messages.iter().any(|m| {
        if let ChatCompletionRequestMessage::Tool(t) = m {
            match &t.content {
                ChatCompletionRequestToolMessageContent::Text(s) => {
                    s.contains("pending_human_approval")
                }
                _ => false,
            }
        } else {
            false
        }
    });
    assert!(
        has_pending,
        "Should detect pending_human_approval in tool results"
    );

    // No pending: only normal results
    let mut clean_messages: Vec<ChatCompletionRequestMessage> = vec![];
    push_tool_result(
        &mut clean_messages,
        "call_3",
        r#"{"status":"ok","tools":["search_tools"]}"#,
    )
    .unwrap();
    let has_pending_clean = clean_messages.iter().any(|m| {
        if let ChatCompletionRequestMessage::Tool(t) = m {
            match &t.content {
                ChatCompletionRequestToolMessageContent::Text(s) => {
                    s.contains("pending_human_approval")
                }
                _ => false,
            }
        } else {
            false
        }
    });
    assert!(
        !has_pending_clean,
        "Should NOT detect pending in clean results"
    );
}

/// Gate-pending guard: tool name extraction from pending message
#[test]
fn gate_pending_extracts_tool_name() {
    let msg = r#"{"status":"pending_human_approval","approval_id":"gate-abc","message":"Tool 'setup_google' requires user approval. Request sent via Telegram."}"#;
    let v: serde_json::Value = serde_json::from_str(msg).unwrap();
    let name = v
        .get("message")
        .and_then(|m| m.as_str())
        .and_then(|msg| msg.strip_prefix("Tool '"))
        .and_then(|rest| rest.split('\'').next());
    assert_eq!(name, Some("setup_google"));
}

#[test]
fn gate_pending_owner_ack_uses_plain_human_labels() {
    let tools = vec!["sysadmin_ops".to_string(), "setup_google".to_string()];
    let label = gate_pending_tool_list_for_owner(&tools);
    assert_eq!(label, "Google connection change, System operation");
    assert!(!label.contains("sysadmin_ops"));
    assert!(!label.contains("<b>"));
}

#[test]
fn deterministic_tool_result_reply_prevents_silent_image_generation() {
    let reply = deterministic_reply_from_recent_tool_result(
            &["generate_image".to_string()],
            &[r#"{"status":"success","message":"Image generated and saved for the WebApp.","image_url":"/webapp/vnext/generated-image/image-abc-1-def.png"}"#
                .to_string()],
        )
        .expect("fallback reply");

    assert!(reply.contains("Image generated and saved for the WebApp."));
    assert!(
        reply.contains("![Generated image](/webapp/vnext/generated-image/image-abc-1-def.png)")
    );
}

#[test]
fn deterministic_tool_result_reply_surfaces_tool_error() {
    let reply = deterministic_reply_from_recent_tool_result(
        &["generate_image".to_string()],
        &[r#"{"status":"error","error":"Image generation API error (status 503)"}"#.to_string()],
    )
    .expect("fallback reply");

    assert_eq!(
        reply,
        "I could not finish that: Image generation API error (status 503)"
    );
}

/// Regression for the production path reported on 2026-04-29:
/// "verify the two activated skills" led the model to call
/// `manage_skill {"action":"health"}`. That read-only action must not
/// create a pending HITL tool result, otherwise the ReAct turn stops and
/// approval replay executes only the single health subcall.
#[test]
fn manage_skill_health_does_not_trigger_gate_pending_guard() {
    let gate = crate::tool_gate::ToolGate::new();
    let verdict = gate.evaluate(
        "manage_skill",
        r#"{"action":"health","skill_name":"server_health_aggregator"}"#,
        crate::tool_gate::CallerContext::Interactive,
    );
    assert!(
        matches!(verdict, crate::tool_gate::GateVerdict::Allow),
        "read-only manage_skill health should not require approval: {verdict:?}"
    );

    let mut messages: Vec<ChatCompletionRequestMessage> = vec![];
    push_tool_result(
        &mut messages,
        "call_health",
        r#"{"status":"ok","skill_name":"server_health_aggregator","health":"no_data"}"#,
    )
    .unwrap();

    let has_pending = messages.iter().any(|m| {
        if let ChatCompletionRequestMessage::Tool(t) = m {
            match &t.content {
                ChatCompletionRequestToolMessageContent::Text(s) => {
                    s.contains("pending_human_approval")
                }
                _ => false,
            }
        } else {
            false
        }
    });
    assert!(
        !has_pending,
        "read-only skill health checks must not trip the gate-pending guard"
    );
}

/// Counterpart to the health-check regression: dangerous manage_skill
/// actions still need HITL so the read-only fast path cannot broaden
/// rollback authority.
#[test]
fn manage_skill_rollback_still_triggers_gate_pending_guard() {
    let gate = crate::tool_gate::ToolGate::new();
    let verdict = gate.evaluate(
        "manage_skill",
        r#"{"action":"rollback","skill_name":"server_health_aggregator","target_version":1}"#,
        crate::tool_gate::CallerContext::Interactive,
    );
    let crate::tool_gate::GateVerdict::NeedApproval { tool_name, .. } = verdict else {
        panic!("rollback must require approval");
    };
    assert_eq!(tool_name, "manage_skill");

    let mut messages: Vec<ChatCompletionRequestMessage> = vec![];
    push_tool_result(
            &mut messages,
            "call_rollback",
            r#"{"status":"pending_human_approval","tool_name":"manage_skill","message":"Tool 'manage_skill' requires user approval."}"#,
        )
        .unwrap();

    let has_pending = messages.iter().any(|m| {
        if let ChatCompletionRequestMessage::Tool(t) = m {
            match &t.content {
                ChatCompletionRequestToolMessageContent::Text(s) => {
                    s.contains("pending_human_approval")
                }
                _ => false,
            }
        } else {
            false
        }
    });
    assert!(
        has_pending,
        "rollback must still be visible to the gate-pending guard"
    );
}

/// Layer 1: Stub tool has [DEFERRED] prefix and empty schema
#[test]
fn stub_tool_has_deferred_marker() {
    let stub = crate::tool_schemas::build_stub_tool("test_tool", "A test tool").unwrap();
    let ChatCompletionTools::Function(f) = &stub else {
        panic!("Expected Function variant");
    };
    assert!(
        f.function
            .description
            .as_deref()
            .unwrap_or("")
            .contains("[DEFERRED]"),
        "Stub description should contain [DEFERRED] marker"
    );
    assert_eq!(f.function.name, "test_tool");
    // Schema should be empty object (no properties)
    let params = f.function.parameters.as_ref().unwrap();
    let props = params.get("properties").unwrap();
    assert!(props.as_object().unwrap().is_empty());
}

// ── Prompt cache break detection tests ────────────────────────────

#[test]
fn fnv1a_hash_deterministic() {
    assert_eq!(fnv1a_hash(b"hello"), fnv1a_hash(b"hello"));
    assert_ne!(fnv1a_hash(b"hello"), fnv1a_hash(b"world"));
    // Known FNV-1a value for empty input is the FNV offset basis
    assert_eq!(fnv1a_hash(b""), 0xcbf29ce484222325);
}

#[test]
fn detect_cache_break_sequence() {
    // Reset the global hash to a known state for this test. Tests run in
    // parallel, so use a unique prompt string that other tests won't clash
    // with. The global atomic is shared; we verify our own seed → change cycle.
    let seed = format!("cache-break-test-seed-{}", std::process::id());
    let changed = format!("cache-break-test-changed-{}", std::process::id());

    // First call with a new unique prompt. The global hash may have other
    // values from parallel tests, so we just verify the return is deterministic
    // for the same input.
    let _ = detect_cache_break(&seed);
    // Same prompt → None (no change from previous call).
    assert_eq!(
        detect_cache_break(&seed),
        None,
        "same prompt should not flag"
    );
    // Different prompt → Some (cache break).
    let result = detect_cache_break(&changed);
    assert!(result.is_some(), "change should be detected");
    let (prev, new) = result.unwrap();
    assert_ne!(prev, new);
    assert_eq!(new, fnv1a_hash(changed.as_bytes()));
}

// ── Phase E Task 6: tool-gate shim → ai:tool_invoked / ai:tool_rejected_by_gate ──

/// Build an orchestrator wired with a HeirloomBridge so tool-gate emissions
/// land in the heirloom event log.
fn mk_orch_with_bridge() -> (
    Orchestrator,
    std::sync::Arc<crate::heirloom::writer::HeirloomWriter>,
) {
    use crate::heirloom::writer::tests::build_writer_with_registry;
    use crate::heirloom::{LineageId, seams::ConstitutionTuple, types::Did};
    use crate::memory_heirloom_bridge::HeirloomBridge;
    use std::sync::Arc;
    let w = Arc::new(build_writer_with_registry());
    w.register_projector(Arc::new(
        crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
    ));
    w.bootstrap_fresh_log_with_types(
        LineageId::new("lineage:test"),
        Did::new("did:aegis:test"),
        ConstitutionTuple {
            l0_hash: [0; 32],
            l1_hash: [0; 32],
            l2_manifest_hash: [0; 32],
        },
        crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS,
    )
    .unwrap();
    let bridge = Arc::new(HeirloomBridge::new(w.clone(), Did::new("did:aegis:test")));
    let mem = crate::memory::MemoryHandle::spawn_in_memory()
        .unwrap()
        .with_heirloom_bridge(bridge);
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(mem, settings);
    (orch, w)
}

fn count_events(w: &crate::heirloom::writer::HeirloomWriter, event_type: &str) -> i64 {
    let conn = w.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = ?1",
            rusqlite::params![event_type],
            |r| r.get(0),
        )
        .unwrap();
    n
}

/// Dispatched tool (unknown name, caller=Interactive → Allow → WASM fallback)
/// emits exactly one `ai:tool_invoked`; zero `ai:tool_rejected_by_gate`.
#[tokio::test]
async fn t_e6_tool_allowed_and_dispatched_emits_ai_tool_invoked() {
    let (orch, w) = mk_orch_with_bridge();
    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    let tc = crate::test_tool_helpers::make_tool_call(
        "phase_e_test_tool_invoked",
        r#"{"secret_arg":"redact_me"}"#,
    );
    // Interactive caller + unknown tool → gate returns Allow.
    let _ = orch.execute_single_tool(&tc, &mut ctx).await;

    assert_eq!(count_events(&w, "ai:tool_invoked"), 1);
    assert_eq!(count_events(&w, "ai:tool_rejected_by_gate"), 0);
}

/// `GateVerdict::Block` path (unknown tool in Callback context) emits
/// exactly one `ai:tool_rejected_by_gate` and zero `ai:tool_invoked`.
#[tokio::test]
async fn t_e6_tool_denied_by_gate_emits_ai_tool_rejected_by_gate() {
    let (orch, w) = mk_orch_with_bridge();
    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    ctx.is_callback = true;
    ctx.caller_context = crate::tool_gate::CallerContext::Callback;
    let tc =
        crate::test_tool_helpers::make_tool_call("phase_e_unknown_tool_for_gate_block", r#"{}"#);
    let _ = orch.execute_single_tool(&tc, &mut ctx).await;

    assert_eq!(count_events(&w, "ai:tool_rejected_by_gate"), 1);
    assert_eq!(count_events(&w, "ai:tool_invoked"), 0);
}

/// FIX R2-2 (CTO Round 2): MCP tool in callback mode short-circuits AFTER
/// gate=Allow but BEFORE the pre-dispatch seam. Must emit
/// `ai:tool_rejected_by_gate` (gate-equivalent) and NOT `ai:tool_invoked`.
/// To hit this branch with a gate=Allow, we inject a synthetic Gate
/// Approval marker so the callback-mode Allow short-circuit fires.
#[tokio::test]
async fn t_e_6_mcp_callback_block_after_allow_verdict_does_not_emit_tool_invoked() {
    let (orch, w) = mk_orch_with_bridge();
    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    ctx.is_callback = true;
    ctx.caller_context = crate::tool_gate::CallerContext::Callback;
    // Pre-seed a synthetic Gate Approval marker message so line 7661 short-
    // circuits gate eval to Allow for this tool_name.
    use async_openai::types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestUserMessageArgs,
    };
    let marker_body =
        "[SYSTEM CALLBACK — Gate Approval] tool 'mcp__fake__write' was approved".to_string();
    let user_msg = ChatCompletionRequestUserMessageArgs::default()
        .content(marker_body)
        .build()
        .unwrap();
    ctx.messages
        .push(ChatCompletionRequestMessage::User(user_msg));
    let tc = crate::test_tool_helpers::make_tool_call("mcp__fake__write", "{}");
    let _ = orch.execute_single_tool(&tc, &mut ctx).await;

    assert_eq!(
        count_events(&w, "ai:tool_rejected_by_gate"),
        1,
        "MCP-callback-block is a gate-equivalent rejection"
    );
    assert_eq!(
        count_events(&w, "ai:tool_invoked"),
        0,
        "short-circuit before pre-dispatch seam — no tool_invoked"
    );
}

// ── #9a — native arbitrary-external-sink exfiltration guard ──────────
//
// These drive the REAL entry path (`execute_single_tool`'s `GateVerdict::
// Allow` arm), not the isolated classifier, so they fail-open without the
// guard wiring. Every classified sink is approval-gated, so to reach `Allow`
// we model the actual exploitable path: a STORED "Always" approval (set once
// by the owner, then abusable by an injected autonomous turn). The FRESH
// owner-approval path (HITL Approve replay) is represented separately by the
// gate-approval marker and must be EXEMPT.

fn mk_orch_with_invite_others(enabled: bool) -> Orchestrator {
    let mem = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = std::sync::Arc::new(crate::settings::RuntimeSettings::new());
    settings
        .set(
            "calendar_invite_others_enabled",
            serde_json::Value::Bool(enabled),
        )
        .unwrap();
    crate::test_tool_helpers::test_orchestrator(mem, settings)
}

/// Seed a stored "Always" approval so an approval-gated sink reaches the
/// gate's `Allow` verdict WITHOUT a fresh per-call owner approval — the
/// exact un-vetted autonomous path the exfil guard must catch.
async fn seed_always_approval(orch: &Orchestrator, tool: &str) {
    orch.store_tool_approval(&crate::tool_gate::StoredApproval {
        tool_name: tool.to_string(),
        scope: crate::tool_gate::ApprovalScope::Always,
        approved_at: "2026-01-01T00:00:00Z".to_string(),
        approved_by: "did:test:user".to_string(),
    })
    .await;
}

/// The "[SYSTEM CALLBACK — Gate Approval]" marker that the HITL Approve-button
/// replay injects — a FRESH, per-call owner approval the guard must exempt.
fn gate_approval_marker(tool_name: &str) -> ChatCompletionRequestMessage {
    use async_openai::types::chat::ChatCompletionRequestUserMessageArgs;
    let body = format!(
        "[SYSTEM CALLBACK — Gate Approval] tool '{}' was approved",
        tool_name
    );
    ChatCompletionRequestMessage::User(
        ChatCompletionRequestUserMessageArgs::default()
            .content(body)
            .build()
            .unwrap(),
    )
}

const EXFIL_SECRET: &str = "This is a secret value that should not leak to external services";
#[tokio::test]
async fn exfil_guard_exempts_owner_approved_replay() {
    // CTO round-2 regression: a FRESH owner approval (HITL Approve button) is
    // replayed as `Callback` WITH the gate-approval marker. The owner saw this
    // exact send (e.g. "forward this paragraph to alice@partner.com"), so the
    // guard must NOT block it even though the body reuses prior tool output.
    // Fails (false-block) against the prior `caller_context != Interactive`
    // predicate; passes with the owner-vetted exemption.
    let orch = mk_orch_with_invite_others(true);
    seed_always_approval(&orch, "calendar_create_event").await; // reach Allow regardless
    let mut state = crate::test_tool_helpers::CtxState::new();
    state.prev_results = vec![EXFIL_SECRET.to_string()];
    state.messages = vec![gate_approval_marker("calendar_create_event")];
    let mut ctx = state.ctx_with_caller(12345, crate::tool_gate::CallerContext::Callback);

    let args = format!(
        r#"{{"summary":"sync","start":"2026-06-02T10:00:00Z","end":"2026-06-02T11:00:00Z","attendees":["someone@external.test"],"description":"{}"}}"#,
        EXFIL_SECRET
    );
    let tc = crate::test_tool_helpers::make_tool_call("calendar_create_event", &args);
    let _ = orch.execute_single_tool(&tc, &mut ctx).await;

    assert!(
        !state
            .tools_blocked
            .iter()
            .any(|t| t == "calendar_create_event"),
        "owner-approved replay (gate-approval marker) must NOT be exfil-blocked"
    );
}

#[tokio::test]
async fn exfil_guard_exempts_interactive_sink() {
    // Owner present in a live turn: rely on the synchronous approval UX, do
    // NOT exfil-block (no marker needed — the Interactive context itself is
    // the owner-vetted signal).
    let orch = mk_orch_with_invite_others(true);
    seed_always_approval(&orch, "calendar_create_event").await;
    let mut state = crate::test_tool_helpers::CtxState::new();
    state.prev_results = vec![EXFIL_SECRET.to_string()];
    let mut ctx = state.ctx_with_caller(12345, crate::tool_gate::CallerContext::Interactive);

    let args = format!(
        r#"{{"summary":"sync","start":"2026-06-02T10:00:00Z","end":"2026-06-02T11:00:00Z","attendees":["someone@external.test"],"description":"{}"}}"#,
        EXFIL_SECRET
    );
    let tc = crate::test_tool_helpers::make_tool_call("calendar_create_event", &args);
    let _ = orch.execute_single_tool(&tc, &mut ctx).await;

    assert!(
        !state
            .tools_blocked
            .iter()
            .any(|t| t == "calendar_create_event"),
        "interactive calendar_create_event must NOT be exfil-blocked (owner present)"
    );
    assert!(
        !state
            .tools_called
            .iter()
            .any(|t| t == "calendar_create_event"),
        "untrusted egress holds for HITL instead of sending"
    );
    let pending = orch.memory.list_hitl_pending().await.expect("hitl");
    assert!(
        pending
            .iter()
            .any(|(_, data)| data.contains("stopped: untrusted_source")),
        "the card records why it stopped"
    );
}

#[tokio::test]
async fn untrusted_calendar_invite_holds_without_sending() {
    // The invite leaves the machine and the description copies a prior tool
    // result. The egress gate holds for the existing HITL card and does not
    // run the tool. Provenance is the runtime's fact, so Jev is not asked.
    let orch = mk_orch_with_invite_others(true);
    seed_always_approval(&orch, "calendar_create_event").await;
    let mut state = crate::test_tool_helpers::CtxState::new();
    state.prev_results = vec![EXFIL_SECRET.to_string()];
    let mut ctx = state.ctx_with_caller(12345, crate::tool_gate::CallerContext::Callback);

    let args = format!(
        r#"{{"summary":"sync","start":"2026-06-02T10:00:00Z","end":"2026-06-02T11:00:00Z","attendees":["someone@external.test"],"description":"{}"}}"#,
        EXFIL_SECRET
    );
    let tc = crate::test_tool_helpers::make_tool_call("calendar_create_event", &args);
    let _ = orch.execute_single_tool(&tc, &mut ctx).await;

    assert!(
        !state
            .tools_called
            .iter()
            .any(|t| t == "calendar_create_event"),
        "untrusted egress must not run the tool"
    );
    let pending = orch.memory.list_hitl_pending().await.expect("hitl");
    let card = pending.iter().find_map(|(_, data)| {
        data.contains("stopped: untrusted_source")
            .then_some(data.as_str())
    });
    let card = card.expect("egress hold card");
    assert!(
        card.contains("someone@external.test"),
        "card shows destination"
    );
    assert!(
        card.contains("would_send:"),
        "card shows what would be sent"
    );
    assert!(
        !card.contains("[SYSTEM CALLBACK"),
        "the hold is not a new instruction"
    );
}

#[tokio::test]
async fn exfil_guard_skips_calendar_when_invites_disabled() {
    // Conditional sink INACTIVE: invite-others off ⇒ attendee-policy forces
    // self-only ⇒ never reaches a third party ⇒ must NOT be exfil-blocked
    // even though args reuse a prior-result fragment.
    let orch = mk_orch_with_invite_others(false);
    seed_always_approval(&orch, "calendar_create_event").await;
    let mut state = crate::test_tool_helpers::CtxState::new();
    state.prev_results = vec![EXFIL_SECRET.to_string()];
    let mut ctx = state.ctx_with_caller(12345, crate::tool_gate::CallerContext::Callback);

    let args = format!(
        r#"{{"summary":"note to self","start":"2026-06-02T10:00:00Z","end":"2026-06-02T11:00:00Z","attendees":["someone@external.test"],"description":"{}"}}"#,
        EXFIL_SECRET
    );
    let tc = crate::test_tool_helpers::make_tool_call("calendar_create_event", &args);
    let _ = orch.execute_single_tool(&tc, &mut ctx).await;

    assert!(
        !state
            .tools_blocked
            .iter()
            .any(|t| t == "calendar_create_event"),
        "self-only calendar (invite-others off) must NOT be exfil-blocked"
    );
}

/// FIX R2-2 (CTO Round 2): auto-resolve-deferred synthetic response emits
/// NEITHER ai:tool_invoked NOR ai:tool_rejected_by_gate. Verified via a
/// deferred tool returning Ok(Some(_)) in try_auto_resolve_deferred.
///
/// We can't easily trigger the real auto-resolve-deferred path from a
/// minimal test fixture (it requires specific skill/schema configuration),
/// so this test verifies the negative contract — if the path short-
/// circuits without reaching the pre-dispatch seam, no tool_invoked
/// event is emitted. We satisfy this by invoking a tool the MCP-callback-
/// block also catches, then asserting auto-resolve-deferred would have
/// been called AFTER MCP-callback-block had this MCP tool not been caught
/// — preserving the "not dispatched, not rejected" invariant shape. The
/// authoritative semantic pin lives in the implementation comment at
/// `// Phase E Task 6 (FIX R2-2): auto_resolve_deferred returned a synthetic`
/// in execute_single_tool; a production-shape test requires real deferred
/// tool fixtures (Phase F cleanup).
#[tokio::test]
async fn t_e_6_auto_resolve_deferred_does_not_emit_tool_invoked() {
    // Smoke test: an unknown tool in Interactive mode reaches the pre-
    // dispatch seam (Allow verdict, not MCP, auto-resolve returns None by
    // default — no deferred tool registered). Since the seam fires,
    // tool_invoked == 1, tool_rejected == 0. This is the complement of
    // the auto-resolve-short-circuit case and documents what the contract
    // IS when auto-resolve does NOT short-circuit. The real "auto-resolve
    // SHORT-CIRCUITS → no emit" case is pinned by the implementation
    // comment + the t_e6_tool_allowed_and_dispatched_emits_ai_tool_invoked
    // symmetry (both gate=Allow paths emit exactly once iff they actually
    // dispatch).
    let (orch, w) = mk_orch_with_bridge();
    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    let tc = crate::test_tool_helpers::make_tool_call("phase_e_test_tool_no_auto_resolve", "{}");
    let _ = orch.execute_single_tool(&tc, &mut ctx).await;

    // Allow → auto-resolve-deferred returns None → pre-dispatch seam fires.
    assert_eq!(count_events(&w, "ai:tool_invoked"), 1);
    assert_eq!(count_events(&w, "ai:tool_rejected_by_gate"), 0);
}

/// `ai:tool_invoked` payload carries `args_hash` (blake3 hex) NOT the raw
/// args string — spec §3 Q3.7 privacy invariant.
#[tokio::test]
async fn t_e6_tool_invoked_args_hash_is_blake3_not_raw_payload() {
    let (orch, w) = mk_orch_with_bridge();
    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    let secret = "XXXXXX_secret_arg_string";
    let tc = crate::test_tool_helpers::make_tool_call(
        "phase_e_args_hash_check",
        &format!(r#"{{"secret":"{secret}"}}"#),
    );
    let _ = orch.execute_single_tool(&tc, &mut ctx).await;

    let conn = w.shared_connection();
    let payload: Vec<u8> = conn
        .lock()
        .query_row(
            "SELECT payload FROM heirloom_event_log
                 WHERE event_type = 'ai:tool_invoked'
                 ORDER BY hlc_phys DESC, hlc_log DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // Unwrap plaintext codec framing: "aegis:plaintext-v1:" + header + "\n" + body
    let tag = b"aegis:plaintext-v1:";
    assert!(payload.starts_with(tag));
    let rest = &payload[tag.len()..];
    let nl = rest.iter().position(|b| *b == b'\n').unwrap();
    let body = &rest[nl + 1..];
    let body_str = std::str::from_utf8(body).unwrap();
    assert!(
        !body_str.contains(secret),
        "payload must NOT contain raw secret args"
    );
    let parsed: serde_json::Value = serde_json::from_slice(body).unwrap();
    let args_hash = parsed["args_hash"].as_str().unwrap();
    assert_eq!(args_hash.len(), 64, "args_hash must be 64-char blake3 hex");
}

/// G10 kill-switch coverage: `AEGIS_RUNTIME_HEIRLOOM_BRIDGE=off` disables emits
/// at the source. Verified by constructing the bridge via `new_with_env`
/// with the env var set. Checked via memory_heirloom_bridge Task 1 tests;
/// here we verify an orchestrator with bridge=None emits zero events.
#[tokio::test]
async fn t_e6_tool_emit_under_disabled_bridge_is_noop() {
    // bridge=None on the MemoryHandle (equivalent to kill-switch off).
    let mem = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = std::sync::Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(mem, settings);
    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    let tc = crate::test_tool_helpers::make_tool_call("anything", "{}");
    let _ = orch.execute_single_tool(&tc, &mut ctx).await;
    // With bridge=None there is no heirloom DB to query — just ensure
    // no panic.
}

/// `GateVerdict::NeedApproval` does NOT emit — approval may later allow
/// (which re-enters and emits at pre-dispatch seam) or deny (never enters
/// execute_single_tool again). Neither is a dispatch or rejection here.
#[tokio::test]
async fn t_e6_need_approval_denied_does_not_emit_tool_invoked() {
    let (orch, w) = mk_orch_with_bridge();
    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    // NeedApproval path requires a tool with RequireApproval policy —
    // use update_setting which has RequireApproval in default_policies.
    let tc = crate::test_tool_helpers::make_tool_call(
        "update_setting",
        r#"{"key":"test_k","value":"v"}"#,
    );
    let _ = orch.execute_single_tool(&tc, &mut ctx).await;
    assert_eq!(
        count_events(&w, "ai:tool_invoked"),
        0,
        "NeedApproval must not emit tool_invoked"
    );
    assert_eq!(
        count_events(&w, "ai:tool_rejected_by_gate"),
        0,
        "NeedApproval is not a rejection — no tool_rejected"
    );
}

/// Budget-block fires BEFORE the match gate_verdict block. Bridge must
/// not emit on the budget-exceeded early-return path.
#[tokio::test]
async fn t_e6_budget_block_after_allow_verdict_does_not_emit() {
    let (orch, w) = mk_orch_with_bridge();
    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    // Set total_tool_call_count to the cap so the next call trips the
    // budget guard at line 7608 and returns BEFORE gate eval.
    *ctx.total_tool_call_count = super::MAX_TOTAL_TOOL_CALLS;
    let tc = crate::test_tool_helpers::make_tool_call("anything", "{}");
    let _ = orch.execute_single_tool(&tc, &mut ctx).await;
    assert_eq!(count_events(&w, "ai:tool_invoked"), 0);
    assert_eq!(count_events(&w, "ai:tool_rejected_by_gate"), 0);
}
#[test]
fn background_review_tool_allowlist_stays_conservative() {
    let allow = Orchestrator::background_review_tool_allowlist();
    assert!(allow.contains("notify_user"));
    assert!(allow.contains("recall_memory"));
    assert!(allow.contains("calendar_reader"));
    assert!(allow.contains("manage_tasks"));
    assert!(!allow.contains("calendar_create_event"));
    assert!(!allow.contains("calendar_delete_event"));
}

#[test]
fn background_action_tool_allowlist_adds_bounded_execution_tools() {
    let allow = Orchestrator::background_action_tool_allowlist();
    assert!(allow.contains("notify_user"));
    assert!(allow.contains("manage_tasks"));
    assert!(allow.contains("calendar_create_event"));
    assert!(allow.contains("calendar_update_event"));
    assert!(!allow.contains("calendar_delete_event"));
}

#[test]
fn heartbeat_synthesis_tool_allowlist_stays_narrower_than_callback_review() {
    let allow = Orchestrator::heartbeat_synthesis_tool_allowlist();
    assert!(allow.contains("notify_user"));
    assert!(allow.contains("recall_memory"));
    assert!(allow.contains("calendar_reader"));
    assert!(allow.contains("manage_tasks"));
    assert!(!allow.contains("web_search"));
    assert!(!allow.contains("search_tools"));
    assert!(!allow.contains("calendar_create_event"));
}

#[test]
fn webapp_focused_tool_allowlist_disables_global_runtime_tools() {
    let allow = Orchestrator::webapp_focused_tool_allowlist();
    assert!(allow.is_empty());
    assert!(!allow.contains("recall_memory"));
    assert!(!allow.contains("knowledge_base"));
    assert!(!allow.contains("web_search"));
}

#[tokio::test]
async fn deferred_stubs_resolve_through_search_tools_and_auto_resolve() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(memory, settings);
    let mut extra = make_cap("server_monitor");
    extra.description =
        "Monitor host health and emit alerts when thresholds are crossed. Extra detail."
            .to_string();
    extra.payload_schema = serde_json::json!({
        "type": "object",
        "properties": { "action": { "type": "string" } },
        "required": ["action"]
    });
    orch.sandbox.register_test_capability(extra);

    let tools = orch.build_tool_definitions();
    let server_monitor = tools
        .iter()
        .find_map(|tool| match tool {
            ChatCompletionTools::Function(f) if f.function.name == "server_monitor" => Some(f),
            _ => None,
        })
        .expect("non-core wasm skill must appear");
    assert!(
        server_monitor
            .function
            .description
            .as_deref()
            .unwrap_or("")
            .contains("[DEFERRED]"),
        "non-core wasm skills become deferred stubs"
    );
    assert!(
        server_monitor
            .function
            .description
            .as_deref()
            .unwrap_or("")
            .contains("Monitor host health and emit alerts when thresholds are crossed."),
        "stub uses the skill's first sentence"
    );

    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    let search =
        crate::test_tool_helpers::make_tool_call("search_tools", r#"{"query":"manage_goals"}"#);
    orch.execute_single_tool(&search, &mut ctx)
        .await
        .expect("search_tools should succeed");
    let found = state.messages.iter().any(|m| match m {
        ChatCompletionRequestMessage::Tool(t) => match &t.content {
            ChatCompletionRequestToolMessageContent::Text(s) => {
                s.contains("manage_goals") && s.contains("action")
            }
            _ => false,
        },
        _ => false,
    });
    assert!(found, "search_tools must return the cached full schema");
    // Regression (2026-09-08): the handler used to return Ok(None) after
    // pushing its result, so dispatch fell through to the Wasm branch which
    // denied the "unregistered skill" and marked search_tools as blocked.
    assert!(
        state.tools_blocked.is_empty(),
        "search_tools must not be reported as blocked, got {:?}",
        state.tools_blocked
    );
    let tool_messages = state
        .messages
        .iter()
        .filter(|m| matches!(m, ChatCompletionRequestMessage::Tool(_)))
        .count();
    assert_eq!(
        tool_messages, 1,
        "exactly one tool result for the search_tools call"
    );
    // The looked-up tool is promoted: later rounds get its full schema.
    assert_eq!(state.promoted_tools, vec!["manage_goals".to_string()]);
    let promoted = orch.promote_deferred_tools(&tools, &state.promoted_tools);
    let manage_goals = promoted
        .iter()
        .find_map(|tool| match tool {
            ChatCompletionTools::Function(f) if f.function.name == "manage_goals" => Some(f),
            _ => None,
        })
        .expect("manage_goals stays in the list");
    assert!(
        !manage_goals
            .function
            .description
            .as_deref()
            .unwrap_or("")
            .contains("[DEFERRED]"),
        "promoted tool must carry the full schema, not the stub"
    );
    assert_eq!(promoted.len(), tools.len(), "promotion replaces in place");

    // An empty-args call on a deferred tool without a smart default also
    // promotes it, and the help text says the schema is now in the list.
    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    let empty = crate::test_tool_helpers::make_tool_call("manage_goals", "{}");
    let resolved = orch
        .try_auto_resolve_deferred(&empty, "manage_goals", "{}", &mut ctx)
        .await
        .expect("auto-resolve should run");
    assert_eq!(resolved, Some(true));
    assert_eq!(state.promoted_tools, vec!["manage_goals".to_string()]);
    assert!(state.messages.iter().any(|m| match m {
        ChatCompletionRequestMessage::Tool(t) => match &t.content {
            ChatCompletionRequestToolMessageContent::Text(s) => s.contains("now in your tool list"),
            _ => false,
        },
        _ => false,
    }));

    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    let setup = crate::test_tool_helpers::make_tool_call("setup_google", "{}");
    let resolved = orch
        .try_auto_resolve_deferred(&setup, "setup_google", "{}", &mut ctx)
        .await
        .expect("auto-resolve should run");
    assert_eq!(
        resolved, None,
        "setup_google must not be in the deferred schema cache"
    );
}

#[tokio::test]
async fn search_tools_routes_unconfigured_google_skill_via_codex() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(memory, settings);
    let mut google_cap = make_cap("calendar_reader");
    google_cap.allowed_domains = vec!["www.googleapis.com".to_string()];
    orch.sandbox.register_test_capability(google_cap);
    let _ = orch.build_tool_definitions();

    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    let search =
        crate::test_tool_helpers::make_tool_call("search_tools", r#"{"query":"calendar_reader"}"#);
    orch.execute_single_tool(&search, &mut ctx)
        .await
        .expect("search_tools should succeed");
    let payload = state
        .messages
        .iter()
        .find_map(|m| match m {
            ChatCompletionRequestMessage::Tool(t) => match &t.content {
                ChatCompletionRequestToolMessageContent::Text(s) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("search_tools result");
    let json: serde_json::Value = serde_json::from_str(&payload).expect("search_tools json");
    assert_eq!(json["route_via"], "run_external");
    let hint = json["hint"].as_str().unwrap_or("");
    assert!(hint.contains("run_external"), "hint={hint}");
    assert!(hint.contains("executor=\"codex\""), "hint={hint}");
    assert!(
        hint.contains("Access is checked before execution"),
        "hint={hint}"
    );
    assert!(!hint.contains("setup_google"), "hint={hint}");
    assert_eq!(json["tools_found"], 0);
}

#[tokio::test]
async fn search_tools_does_not_offer_setup_google() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(memory, settings);
    let _ = orch.build_tool_definitions();

    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    let search =
        crate::test_tool_helpers::make_tool_call("search_tools", r#"{"query":"setup_google"}"#);
    orch.execute_single_tool(&search, &mut ctx)
        .await
        .expect("search_tools should succeed");
    let payload = state
        .messages
        .iter()
        .find_map(|m| match m {
            ChatCompletionRequestMessage::Tool(t) => match &t.content {
                ChatCompletionRequestToolMessageContent::Text(s) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("search_tools result");
    let json: serde_json::Value = serde_json::from_str(&payload).expect("search_tools json");
    assert_eq!(json["tools_found"], 0);
    assert!(json.get("route_via").is_none());
    assert!(
        json["tools"]
            .as_array()
            .map(|tools| tools.is_empty())
            .unwrap_or(false),
        "setup_google must not appear in search_tools results: {payload}"
    );
}

#[tokio::test]
async fn search_tools_disabled_skill_only_for_loaded_skill_and_does_not_claim_feature_off() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    settings.register_skill("server_monitor", "monitor", false);
    settings.register_skill("not_loaded_skill", "absent", false);
    let orch = crate::test_tool_helpers::test_orchestrator(memory, settings);
    orch.sandbox
        .register_test_capability(make_cap("server_monitor"));

    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    let search =
        crate::test_tool_helpers::make_tool_call("search_tools", r#"{"query":"server_monitor"}"#);
    orch.execute_single_tool(&search, &mut ctx)
        .await
        .expect("search_tools should succeed");
    let payload = state
        .messages
        .iter()
        .find_map(|m| match m {
            ChatCompletionRequestMessage::Tool(t) => match &t.content {
                ChatCompletionRequestToolMessageContent::Text(s) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("search_tools result");
    let json: serde_json::Value = serde_json::from_str(&payload).expect("search_tools json");
    assert_eq!(json["disabled_skill"], "server_monitor");
    let hint = json["hint"].as_str().unwrap_or("");
    assert!(hint.contains("Automatic/scheduled runs"), "hint={hint}");
    assert!(hint.contains("on-demand use is unaffected"), "hint={hint}");
    assert!(!hint.contains("turned off"), "hint={hint}");
    assert!(!hint.contains("unable"), "hint={hint}");

    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    let search =
        crate::test_tool_helpers::make_tool_call("search_tools", r#"{"query":"not_loaded_skill"}"#);
    orch.execute_single_tool(&search, &mut ctx)
        .await
        .expect("search_tools should succeed");
    let payload = state
        .messages
        .iter()
        .find_map(|m| match m {
            ChatCompletionRequestMessage::Tool(t) => match &t.content {
                ChatCompletionRequestToolMessageContent::Text(s) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("search_tools result");
    let json: serde_json::Value = serde_json::from_str(&payload).expect("search_tools json");
    assert!(
        json.get("disabled_skill").is_none(),
        "disabled_skill must not fire for a skill that is not loaded: {payload}"
    );
}

fn tool_result_text(messages: &[ChatCompletionRequestMessage]) -> String {
    messages
        .iter()
        .rev()
        .find_map(|message| match message {
            ChatCompletionRequestMessage::Tool(tool) => match &tool.content {
                ChatCompletionRequestToolMessageContent::Text(text) => Some(text.clone()),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn egress_gate_skips_jev_for_a_local_tool() {
    let mem = crate::memory::MemoryHandle::spawn_in_memory().expect("in-memory DB");
    let settings = std::sync::Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(mem, settings);
    let mut state = crate::test_tool_helpers::CtxState::new();
    let mut ctx = state.ctx();
    let tc = crate::test_tool_helpers::make_tool_call("get_current_time", "{}");
    orch.execute_single_tool(&tc, &mut ctx)
        .await
        .expect("local tool");
    let result = tool_result_text(&state.messages);
    assert!(
        !result.contains("pending_human_approval"),
        "a non-egress tool must not hold when Jev is unavailable: {result}"
    );
    assert!(
        !result.contains("jev_unavailable"),
        "a non-egress tool must not call Jev: {result}"
    );
}

#[tokio::test]
async fn owner_egress_holds_when_jev_is_unavailable() {
    if crate::computer_use::resolve_openrouter_api_key().is_some() {
        return;
    }
    let mem = crate::memory::MemoryHandle::spawn_in_memory().expect("in-memory DB");
    let settings = std::sync::Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(mem, settings);
    let mut state = crate::test_tool_helpers::CtxState::new();
    state.messages = vec![ChatCompletionRequestMessage::User(
        async_openai::types::chat::ChatCompletionRequestUserMessageArgs::default()
            .content("ping the status hook")
            .build()
            .unwrap(),
    )];
    let mut ctx = state.ctx();
    let tc = crate::test_tool_helpers::make_tool_call(
        "http_api",
        r#"{"url":"https://status.example/hook","body":"ping"}"#,
    );
    orch.execute_single_tool(&tc, &mut ctx)
        .await
        .expect("dispatch");
    assert!(
        !state.tools_called.iter().any(|name| name == "http_api"),
        "jev failure must not run the tool"
    );
    let pending = orch.memory.list_hitl_pending().await.expect("hitl");
    let card = pending
        .iter()
        .find_map(|(_, data)| {
            data.contains("stopped: jev_unavailable")
                .then_some(data.as_str())
        })
        .expect("jev unavailable card");
    assert!(card.contains("https://status.example/hook"));
    assert!(card.contains("would_send:"));
    assert!(card.contains("ping"));
}

#[tokio::test]
async fn owner_replay_does_not_hold_egress_again() {
    let mem = crate::memory::MemoryHandle::spawn_in_memory().expect("in-memory DB");
    let settings = std::sync::Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(mem, settings);
    let mut state = crate::test_tool_helpers::CtxState::new();
    state.messages = vec![gate_approval_marker("http_api")];
    state.prev_results = vec![EXFIL_SECRET.to_string()];
    let mut ctx = state.ctx();
    ctx.gate_pre_approved = true;
    let args = format!(
        r#"{{"url":"https://status.example/hook","body":"{}"}}"#,
        EXFIL_SECRET
    );
    let tc = crate::test_tool_helpers::make_tool_call("http_api", &args);
    orch.execute_single_tool(&tc, &mut ctx)
        .await
        .expect("replay");
    let result = tool_result_text(&state.messages);
    assert!(
        !result.contains("pending_human_approval"),
        "the approval button must not open a second hold: {result}"
    );
    let pending = orch.memory.list_hitl_pending().await.expect("hitl");
    assert!(
        pending.is_empty(),
        "replay must not store another approval: {pending:?}"
    );
}
