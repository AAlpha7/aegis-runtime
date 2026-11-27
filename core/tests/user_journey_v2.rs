//! User Journey v2 — multi-turn real-world scenarios from production L3 data.
//!
//! Each test simulates a complete user workflow as observed in production
//! (3,577 messages over 40 days). Multi-turn tests use response queues
//! to simulate realistic LLM tool-calling sequences.
//!
//! All mock-based, no real APIs.

mod mock_helpers;

use std::sync::Arc;

use serde_json::json;

use aegis_runtime_core::memory::{MemoryHandle, Message};
use aegis_runtime_core::settings::RuntimeSettings;

use mock_helpers::*;

async fn pre_approve_tool(memory: &MemoryHandle, tool_name: &str) {
    let approvals = json!([{
        "tool_name": tool_name,
        "scope": "Always",
        "approved_at": "2025-01-01T00:00:00Z",
        "approved_by": "did:test:admin"
    }]);
    memory
        .store_skill_state(
            "hitl",
            &format!("tool_approvals:{tool_name}"),
            &approvals.to_string(),
        )
        .await
        .expect("pre-approve tool");
}

// ── 1. "周末提醒给门铃充电" — Intent scheduling (121 uses in production) ─────

/// User asks for a reminder → LLM calls schedule_intent → stored.
/// Then simulate the callback firing.
#[tokio::test]
async fn journey_reminder_schedule_and_callback() {
    // Turn 1: User asks for reminder → LLM schedules intent
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "description": "周末Ring门铃充电提醒",
                "trigger_type": "time",
                "delay_seconds": 86400,
                "execution_mode": "notify",
                "context_text": "周末Ring门铃充电提醒"
            }),
        ),
        text_response("已设置提醒：周末给Ring门铃充电。"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let r1 = orch
        .process_message("did:test:user1", "周末提醒给门铃充电", Some(100))
        .await
        .unwrap();

    assert!(
        r1.tools_called.contains(&"schedule_intent".to_string()),
        "should call schedule_intent: {:?}",
        r1.tools_called
    );
    let calls_after_t1 = state.chat_calls();
    assert!(
        calls_after_t1 >= 2,
        "at least 2 LLM calls for tool+response"
    );

    // Turn 2: Simulate callback firing
    state.push_responses(vec![text_response("提醒：请给Ring门铃充电。周末别忘了！")]);

    let r2 = orch
        .process_callback_message(
            "did:test:user1",
            "[INTENT_CALLBACK:depth=0] 周末Ring门铃充电提醒",
            Some(100),
        )
        .await
        .unwrap();

    assert!(!r2.reply.is_empty(), "callback should produce a reply");
    assert!(
        state.chat_calls() > calls_after_t1,
        "callback should make additional LLM calls"
    );
}

// ── 2. HITL approval → tool execution (108 uses in production) ───────────────

/// Pre-store approval state → user sends "approve" → tool executes.
#[tokio::test]
async fn journey_hitl_approval_flow() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "update_setting",
            json!({"key": "mcp_enabled", "value": false}),
        ),
        text_response("已关闭 MCP 工具。"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "update_setting").await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "关闭 MCP 工具", Some(100))
        .await
        .unwrap();

    assert!(result.tools_called.contains(&"update_setting".to_string()));
    assert!(result.rounds_used >= 1);
}

// ── 3. Knowledge store → later recall (319 KB entries in production) ─────────

/// User stores knowledge → subsequent recall_memory finds it.
#[tokio::test]
async fn journey_knowledge_store_then_recall() {
    // Turn 1: Store knowledge
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "knowledge_base",
            json!({
                "action": "extract",
                "title": "Tesla Model 3 租赁",
                "content": "36个月封闭式租赁，月供$299",
                "category": "reference"
            }),
        ),
        text_response("已记录Tesla租赁信息。"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let r1 = orch
        .process_message(
            "did:test:user1",
            "帮忙记住：Tesla Model 3 租赁 36个月 月供$299",
            Some(100),
        )
        .await
        .unwrap();

    assert!(r1.tools_called.contains(&"knowledge_base".to_string()));

    // Turn 2: Recall
    state.push_responses(vec![
        tool_call_response(
            "call_2",
            "recall_memory",
            json!({"query": "Tesla 租赁 月供", "scope": "all"}),
        ),
        text_response("Tesla Model 3 月供$299。"),
    ]);

    let r2 = orch
        .process_message("did:test:user1", "Tesla月供多少？", Some(100))
        .await
        .unwrap();

    assert!(r2.tools_called.contains(&"recall_memory".to_string()));
    assert!(
        state.chat_calls() >= 4,
        "at least 4 LLM calls across 2 turns"
    );
}

// ── 4. "通勤" — single word triggers full workflow (26 uses) ─────────────────

/// User sends just "通勤" → LLM calls commute skill → report returned.
#[tokio::test]
async fn journey_commute_single_word() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "commute_traffic_monitor",
            json!({"home": "Example City", "office": "Worktown"}),
        ),
        text_response("通勤预估 35 分钟，交通顺畅。"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    // commute_traffic_monitor is a Wasm skill — won't actually execute in test
    // but it should be dispatched and return a structured error
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "通勤", Some(100))
        .await
        .unwrap();

    // The key assertion: the tool was dispatched (even if it fails due to no Wasm)
    assert!(result.rounds_used >= 1);
    assert_eq!(state.chat_calls(), 2);
}

// ── 5. Stock monitoring callback (195 reports in production) ─────────────────

/// Simulates the recurring stock monitoring intent callback.
#[tokio::test]
async fn journey_stock_monitoring_callback() {
    // Owner-authored quote. The journey tests the tool loop, so Jev's Noul
    // is pinned below the hold threshold. The hold itself is a lib test.
    aegis_runtime_core::egress_gate::pin_noul_for_tests(0.05);
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "us_stock_quote", json!({"symbol": "MSFT"})),
        text_response("MSFT: $420.50 (+1.2%)，建议持有。"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_callback_message(
            "did:test:user1",
            "[INTENT_CALLBACK:depth=0] 查询微软(MSFT)股票价格并为你分析最新情况",
            Some(100),
        )
        .await
        .unwrap();

    assert!(!result.reply.is_empty());
    assert_eq!(result.rounds_used, 2);
    assert_eq!(state.chat_calls(), 2);
}

// ── 6. "api费用分析" (45 system uses in production) ──────────────────────────

#[tokio::test]
async fn journey_api_usage_check() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "check_api_usage", json!({})),
        text_response("本月 API 费用 $12.50，使用 20M tokens。"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "api费用分析", Some(100))
        .await
        .unwrap();

    assert!(result.tools_called.contains(&"check_api_usage".to_string()));
    assert_eq!(result.rounds_used, 2);
}

// ── 7. Quote reply with context (36 uses in production) ──────────────────────

/// User replies to a quoted message — context should be passed to LLM.
#[tokio::test]
async fn journey_quote_reply_context() {
    let state = Arc::new(MockServerState::new(vec![text_response(
        "根据之前的电价信息，每月约$150。",
    )]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    // Simulate quoted reply format (as seen in production L3)
    let result = orch
        .process_message(
            "did:test:user1",
            "[引用消息] Example City (PSE) 家用电价: $0.12/kWh\n\n[用户回复] 那我家一个月电费大概多少",
            Some(100),
        )
        .await
        .unwrap();

    assert!(
        !result.reply.is_empty(),
        "should produce reply from context"
    );
    assert_eq!(result.rounds_used, 1);
}

// ── 8. Multi-tool parallel call — morning briefing pattern ───────────────────

/// LLM calls multiple tools in parallel (time + memory + tasks).
#[tokio::test]
async fn journey_morning_briefing_parallel_tools() {
    let state = Arc::new(MockServerState::new(vec![
        multi_tool_call_response(vec![
            ("call_1", "get_current_time", json!({})),
            (
                "call_2",
                "recall_memory",
                json!({"query": "today schedule", "scope": "all"}),
            ),
            ("call_3", "manage_tasks", json!({"action": "list"})),
        ]),
        text_response("早上好！现在 8:30am，今天有3个任务待办。"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "早上好，今天安排是什么", Some(100))
        .await
        .unwrap();

    assert!(
        result.tools_called.len() >= 3,
        "should dispatch 3 parallel tools: {:?}",
        result.tools_called
    );
    assert_eq!(
        result.rounds_used, 2,
        "parallel tools in 1 round + response"
    );
}

// ── 9. Heartbeat autonomous reasoning (174 uses in production) ───────────────

/// Simulates the autonomous heartbeat callback path.
#[tokio::test]
async fn journey_heartbeat_autonomous_reasoning() {
    let state = Arc::new(MockServerState::new(vec![text_response(
        "系统正常，无需通知用户。",
    )]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_callback_message(
            "did:test:user1",
            "[HEARTBEAT] 检查系统状态，如有异常通知用户",
            Some(100),
        )
        .await
        .unwrap();

    // Heartbeat may or may not produce notifications
    assert_eq!(result.rounds_used, 1);
    assert_eq!(state.chat_calls(), 1);
}

// ── 10. L3 archive fallback (3,577 entries in production) ────────────────────

/// When L2 returns nothing, recall_memory should fall back to L3 FTS5.
#[tokio::test]
async fn journey_l3_archive_fallback() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "recall_memory",
            json!({"query": "Tesla rental agreement", "scope": "all"}),
        ),
        text_response("Found in archive: Tesla Model 3 rental, $299/month."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // Pre-populate L3 via push_l1 (which archives to L3)
    memory
        .push_l1(
            "did:test:user1",
            Message {
                role: "user".into(),
                content: "Tesla Model 3 rental agreement: 36 months, $299/month".into(),
                timestamp: chrono::Utc::now().timestamp(),
                anchor_kind: None,
                anchor_ref: None,
                latency_ms: None,
                attachments: vec![],
            },
        )
        .await
        .unwrap();

    // DON'T store in L2 — force L3 fallback
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "what was my Tesla rental agreement?",
            Some(100),
        )
        .await
        .unwrap();

    assert!(result.tools_called.contains(&"recall_memory".to_string()));
    assert_eq!(state.chat_calls(), 2);
}

// ── 11. Goal lifecycle — dead code coverage ──────────────────────────────────

/// Create → update → list goals across multiple turns.
#[tokio::test]
async fn journey_goal_lifecycle() {
    let state = Arc::new(MockServerState::new(vec![
        // Turn 1: Create goal
        tool_call_response(
            "call_1",
            "manage_goals",
            json!({"action": "create", "title": "学习 Rust", "description": "掌握所有权模型"}),
        ),
        text_response("已创建目标：学习 Rust。"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    // Turn 1
    let r1 = orch
        .process_message("did:test:user1", "我要学 Rust", Some(100))
        .await
        .unwrap();
    assert!(r1.tools_called.contains(&"manage_goals".to_string()));

    // Turn 2 — push fresh responses for next turn
    state.push_responses(vec![
        tool_call_response(
            "call_2",
            "manage_goals",
            json!({"action": "update", "goal_id": 1, "progress": 30}),
        ),
        text_response("进度已更新为 30%。"),
    ]);
    let r2 = orch
        .process_message("did:test:user1", "进度 30%", Some(100))
        .await
        .unwrap();
    assert!(r2.tools_called.contains(&"manage_goals".to_string()));

    // Turn 3 — push fresh responses
    state.push_responses(vec![
        tool_call_response("call_3", "manage_goals", json!({"action": "list"})),
        text_response("你有 1 个目标：学习 Rust (30%)。"),
    ]);
    let r3 = orch
        .process_message("did:test:user1", "列出目标", Some(100))
        .await
        .unwrap();
    assert!(r3.tools_called.contains(&"manage_goals".to_string()));

    assert!(
        state.chat_calls() >= 6,
        "at least 6 LLM calls across 3 turns"
    );
}

// ── 12. Web search retry after failure ───────────────────────────────────────

/// Two empty public lookups do not invent a third chat answer.
/// The turn continues as one research job on the same thread.
#[tokio::test]
async fn journey_web_search_retry_on_failure() {
    aegis_runtime_core::egress_gate::pin_noul_for_tests(0.05);
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "web_search", json!({"query": "latest AI news"})),
        text_response("(no results)"),
        text_response("(no results)"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    // Search HTTP shares the chat mock only when the owner chose OpenRouter.
    settings
        .set("web_search_provider", json!("openrouter"))
        .unwrap();
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let result = orch
        .process_message("did:test:user1", "搜索最新AI新闻", Some(100))
        .await
        .unwrap();

    assert_eq!(
        result.rounds_used, 1,
        "no extra model round after the two lookups"
    );
    assert!(result.reply.contains("latest AI news"), "{}", result.reply);
    assert!(result.reply.contains("不在"), "{}", result.reply);
    assert!(!result.reply.contains("Claude 5"), "{}", result.reply);
    // 1 model call + 2 public lookups + the research job's remaining lookups.
    assert!(
        state.chat_calls() > 3,
        "research continues after two misses, calls={}",
        state.chat_calls()
    );
    assert!(
        state.chat_calls() <= 11,
        "research stays inside eight lookups"
    );
    let jobs = memory
        .list_skill_state_entries("research_job")
        .await
        .unwrap();
    assert_eq!(
        jobs.len(),
        1,
        "the work record is the research job, not each lookup"
    );
}

// ── 13. Model switch → subsequent message uses new model ─────────────────────

/// Switch model, then verify the next message goes through (future-proofing).
#[tokio::test]
async fn journey_model_switch_then_continue() {
    let state = Arc::new(MockServerState::new(vec![
        // Turn 1: switch model
        tool_call_response(
            "call_1",
            "switch_model",
            json!({"model": "google/gemini-2.5-flash"}),
        ),
        text_response("已切换到 Gemini 2.5 Flash。"),
        // Turn 2: normal conversation on new model
        text_response("这是用新模型回答的。"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "switch_model").await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let r1 = orch
        .process_message("did:test:user1", "切换到 Gemini", Some(100))
        .await
        .unwrap();
    assert!(r1.tools_called.contains(&"switch_model".to_string()));

    let r2 = orch
        .process_message("did:test:user1", "你好，新模型", Some(100))
        .await
        .unwrap();
    assert!(!r2.reply.is_empty());
    assert!(
        state.chat_calls() >= 3,
        "at least 3 LLM calls across 2 turns"
    );
}

// ── 14. Concurrent user message safety ───────────────────────────────────────

/// Two concurrent process_message calls on the same DID should not deadlock.
#[tokio::test]
async fn journey_concurrent_messages_no_deadlock() {
    let state = Arc::new(MockServerState::new(vec![
        text_response("Reply A"),
        text_response("Reply B"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = Arc::new(build_orchestrator(addr, memory, settings));

    let orch1 = orch.clone();
    let orch2 = orch.clone();

    let (r1, r2) = tokio::join!(
        async move {
            orch1
                .process_message("did:test:user1", "message A", Some(100))
                .await
        },
        async move {
            orch2
                .process_message("did:test:user1", "message B", Some(100))
                .await
        }
    );

    // Both should complete (one may fail with llm_busy, that's OK)
    let ok_count = [r1.is_ok(), r2.is_ok()].iter().filter(|&&x| x).count();
    assert!(
        ok_count >= 1,
        "at least 1 concurrent message should succeed"
    );
}
