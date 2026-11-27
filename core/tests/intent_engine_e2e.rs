//! Engine-level end-to-end tests for the Deferred Intent System.
//!
//! These tests start the *actual* `DeferredIntentEngine` background tasks
//! (time poller, event matcher, expiry cleanup) and verify the full proactive
//! execution lifecycle:
//!
//!   schedule intent → engine detects trigger → executes → user receives result
//!
//! Unlike `intent_integration.rs` (which only tests orchestrator tool interception),
//! these tests exercise the engine's core capability: autonomous background action.
//!
//! Mock servers:
//! - **Mock Telegram Bot API**: Captures `sendMessage` calls for assertion.
//! - **Mock LLM API**: Returns pre-canned chat completions for `llm_callback` mode.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

mod common;

use aegis_runtime_core::embedding::EmbeddingClient;
use aegis_runtime_core::identity::AgentIdentity;
use aegis_runtime_core::intent::{DeferredIntentEngine, IntentEvent};
use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::orchestrator::Orchestrator;
use aegis_runtime_core::settings::RuntimeSettings;

// ── Mock Telegram Bot API ────────────────────────────────────────────────────

/// Captured sendMessage call from the bot.
#[derive(Debug, Clone)]
struct CapturedMessage {
    chat_id: i64,
    text: String,
}

struct TelegramMockState {
    messages: Mutex<Vec<CapturedMessage>>,
    call_count: AtomicUsize,
}

impl TelegramMockState {
    fn new() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            call_count: AtomicUsize::new(0),
        }
    }
}

/// Telegram Bot API returns a `Message` object on success.
fn telegram_ok_response(chat_id: i64, text: &str) -> Value {
    json!({
        "ok": true,
        "result": {
            "message_id": 1,
            "from": {"id": 123, "is_bot": true, "first_name": "TestBot"},
            "chat": {"id": chat_id, "type": "private"},
            "date": 1700000000_u64,
            "text": text
        }
    })
}

#[derive(Deserialize)]
struct SendMessageBody {
    chat_id: serde_json::Value,
    text: String,
}

/// Unified handler for ALL Telegram API methods.
async fn handle_telegram_any(
    Path((_bot_token, method)): Path<(String, String)>,
    State(state): State<Arc<TelegramMockState>>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let resp = if method == "SendMessage" || method == "sendMessage" {
        // Parse sendMessage body
        if let Ok(msg) = serde_json::from_slice::<SendMessageBody>(&body) {
            state.call_count.fetch_add(1, Ordering::SeqCst);
            let chat_id = match &msg.chat_id {
                serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
                serde_json::Value::String(s) => s.parse().unwrap_or(0),
                _ => 0,
            };
            {
                let mut msgs = state.messages.lock().unwrap();
                msgs.push(CapturedMessage {
                    chat_id,
                    text: msg.text.clone(),
                });
            }
            telegram_ok_response(chat_id, &msg.text)
        } else {
            json!({"ok": false, "description": "parse error"})
        }
    } else if method == "GetMe" || method == "getMe" {
        json!({
            "ok": true,
            "result": {
                "id": 123, "is_bot": true, "first_name": "TestBot",
                "username": "test_bot"
            }
        })
    } else {
        json!({"ok": true, "result": true})
    };
    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(resp.to_string()))
        .unwrap()
}

async fn spawn_telegram_mock(
    state: Arc<TelegramMockState>,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route("/{bot_token}/{method}", post(handle_telegram_any))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

fn make_bot(tg_addr: SocketAddr) -> teloxide::Bot {
    let api_url = format!("http://{}/", tg_addr);
    teloxide::Bot::new("test-token-000").set_api_url(reqwest::Url::parse(&api_url).unwrap())
}

// ── Mock LLM API ─────────────────────────────────────────────────────────────

struct LlmMockState {
    chat_responses: Mutex<Vec<String>>,
    chat_call_count: AtomicUsize,
}

impl LlmMockState {
    fn new(responses: Vec<String>) -> Self {
        Self {
            chat_responses: Mutex::new(responses),
            chat_call_count: AtomicUsize::new(0),
        }
    }
}

fn llm_text_response(content: &str) -> String {
    json!({
        "id": "chatcmpl-engine",
        "provider": "Mock",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock-model",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content,
                "refusal": null,
                "reasoning": null,
                "reasoning_details": []
            },
            "logprobs": null,
            "finish_reason": "stop",
            "native_finish_reason": "STOP"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 10,
            "total_tokens": 20,
            "prompt_tokens_details": {},
            "completion_tokens_details": {},
            "cost_details": {}
        }
    })
    .to_string()
}

fn llm_fallback_response() -> String {
    llm_text_response("(mock LLM fallback)")
}

/// Response with a tool call (so `tools_called` is non-empty and the fabrication
/// guard doesn't suppress the final reply).
fn llm_tool_call_response(tool_name: &str, args: &str) -> String {
    json!({
        "id": "chatcmpl-tool",
        "provider": "Mock",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock-model",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_mock_1",
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": args
                    }
                }],
                "refusal": null,
                "reasoning": null,
                "reasoning_details": []
            },
            "logprobs": null,
            "finish_reason": "tool_calls",
            "native_finish_reason": "TOOL_CALLS"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 10,
            "total_tokens": 20,
            "prompt_tokens_details": {},
            "completion_tokens_details": {},
            "cost_details": {}
        }
    })
    .to_string()
}

async fn handle_llm_chat(
    State(state): State<Arc<LlmMockState>>,
    _body: String,
) -> axum::response::Response {
    state.chat_call_count.fetch_add(1, Ordering::SeqCst);
    let body = {
        let mut queue = state.chat_responses.lock().unwrap();
        if queue.is_empty() {
            llm_fallback_response()
        } else {
            queue.remove(0)
        }
    };
    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap()
}

#[derive(Deserialize)]
struct EmbedRequest {
    #[allow(dead_code)]
    input: String,
    dimensions: Option<u32>,
}

const EMBED_DIMS: usize = 256;

async fn handle_llm_embed(Json(req): Json<EmbedRequest>) -> axum::response::Response {
    let dims = req.dimensions.unwrap_or(EMBED_DIMS as u32) as usize;
    let embedding: Vec<f32> = (0..dims)
        .map(|i| (i as f32 / dims as f32) * 2.0 - 1.0)
        .collect();
    let body = json!({
        "id": "gen-emb-mock",
        "provider": "Mock",
        "object": "list",
        "data": [{"object": "embedding", "embedding": embedding, "index": 0}],
        "model": "mock-embed",
        "usage": {"prompt_tokens": 5, "total_tokens": 5, "cost": 0.0}
    });
    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

async fn spawn_llm_mock(state: Arc<LlmMockState>) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route("/v1/chat/completions", post(handle_llm_chat))
        .route("/v1/embeddings", post(handle_llm_embed))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn build_orchestrator(
    llm_addr: SocketAddr,
    memory: MemoryHandle,
    settings: Arc<RuntimeSettings>,
) -> Orchestrator {
    let api_base = format!("http://{}/v1", llm_addr);
    let embedder = EmbeddingClient::new(&api_base, "mock-key", "mock-embed", EMBED_DIMS as u32);
    let sandbox = Arc::new(common::shared_sandbox());
    let identity = Arc::new(AgentIdentity::generate());
    let mut orch = Orchestrator::new(
        "mock-key",
        &api_base,
        "mock-model",
        sandbox,
        identity,
        memory,
        embedder,
        settings,
    );
    orch.set_system_prompt("You are a test assistant.");
    orch
}

/// Wait for at least `min_messages` to arrive in the Telegram mock, with timeout.
async fn wait_for_messages(
    tg_state: &Arc<TelegramMockState>,
    min_messages: usize,
    timeout: Duration,
) -> Vec<CapturedMessage> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        {
            let msgs = tg_state.messages.lock().unwrap();
            if msgs.len() >= min_messages {
                return msgs.clone();
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let msgs = tg_state.messages.lock().unwrap();
            return msgs.clone();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// **Scenario 1: Time-based notify intent fires after delay.**
///
/// Schedule a notify intent with trigger_at in the past (already due),
/// start the engine with a 1s poll interval, and verify the user receives
/// the notification message via Telegram within seconds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_time_notify_fires_and_sends_message() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // Store a time intent that's already due (trigger_at = 5s ago)
    let id = memory
        .store_deferred_intent(
            "did:test:user1",
            42, // chat_id
            "time",
            Some(now - 5),
            None,
            "notify",
            "Your BTC price alert: $105,000",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(id > 0);

    // Set up mocks
    let tg_state = Arc::new(TelegramMockState::new());
    let (tg_addr, _tg_handle) = spawn_telegram_mock(tg_state.clone()).await;
    let bot = make_bot(tg_addr);

    let llm_state = Arc::new(LlmMockState::new(vec![]));
    let (llm_addr, _llm_handle) = spawn_llm_mock(llm_state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = Arc::new(build_orchestrator(llm_addr, memory.clone(), settings));

    let (_event_tx, event_rx) = mpsc::channel::<IntentEvent>(256);
    let engine = DeferredIntentEngine::new(memory.clone(), orch, 0, event_rx)
        .with_poll_interval(Duration::from_secs(1));

    let handles = engine.start(bot);

    // Wait for the message to arrive
    let messages = wait_for_messages(&tg_state, 1, Duration::from_secs(5)).await;

    // Clean up
    for h in handles {
        h.abort();
    }

    // Verify
    assert!(
        !messages.is_empty(),
        "Engine should have sent at least 1 message, got 0"
    );
    assert_eq!(messages[0].chat_id, 42);
    assert_eq!(messages[0].text, "Your BTC price alert: $105,000");

    // 0 LLM calls (notify mode doesn't use LLM)
    assert_eq!(llm_state.chat_call_count.load(Ordering::SeqCst), 0);

    // Intent should be marked as fired
    let pending = memory.list_user_intents("did:test:user1").await.unwrap();
    assert!(pending.is_empty(), "Intent should no longer be pending");
}

/// Reminder delivery regression: Web presence must not suppress Telegram alarms.
///
/// The Web UX is the owner workbench, but reminders are time-sensitive push
/// notifications. When Web presence is active, Aegis should mirror the reminder
/// into Web history and still deliver the Telegram notification immediately.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_time_notify_sends_telegram_even_when_web_is_active() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let chat_id = 42;
    let owner_did = aegis_runtime_core::owner_presence::canonical_owner_did_for_chat(chat_id);
    aegis_runtime_core::owner_presence::mark_owner_web_presence(&memory, &owner_did, "web")
        .await
        .unwrap();

    let id = memory
        .store_deferred_intent(
            "did:test:user-web-active",
            chat_id,
            "time",
            Some(now - 5),
            None,
            "notify",
            "Time to scan the lawyer documents",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(id > 0);

    let tg_state = Arc::new(TelegramMockState::new());
    let (tg_addr, _tg_handle) = spawn_telegram_mock(tg_state.clone()).await;
    let bot = make_bot(tg_addr);

    let llm_state = Arc::new(LlmMockState::new(vec![]));
    let (llm_addr, _llm_handle) = spawn_llm_mock(llm_state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = Arc::new(build_orchestrator(llm_addr, memory.clone(), settings));

    let (_event_tx, event_rx) = mpsc::channel::<IntentEvent>(256);
    let engine = DeferredIntentEngine::new(memory.clone(), orch, 0, event_rx)
        .with_poll_interval(Duration::from_secs(1));

    let handles = engine.start(bot);
    let messages = wait_for_messages(&tg_state, 1, Duration::from_secs(5)).await;
    for h in handles {
        h.abort();
    }

    assert_eq!(messages.len(), 1, "reminder must still push to Telegram");
    assert_eq!(messages[0].chat_id, chat_id);
    assert_eq!(messages[0].text, "Time to scan the lawyer documents");

    let l1 = memory.get_l1(&owner_did).await.unwrap();
    assert!(
        l1.iter()
            .any(|msg| msg.content.contains("Time to scan the lawyer documents")),
        "reminder should also be mirrored into Web owner history: {l1:?}"
    );
    assert_eq!(llm_state.chat_call_count.load(Ordering::SeqCst), 0);
}

/// **Scenario 2: Time-based llm_callback intent fires and invokes orchestrator.**
///
/// The engine picks up a due llm_callback intent, calls orchestrator.process_message()
/// (which hits the mock LLM), and sends the LLM's response to the user via Telegram.
/// This is the "5秒后查当前时间" scenario.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_time_llm_callback_fires_and_returns_llm_response() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // Store a callback intent (already due)
    memory
        .store_deferred_intent(
            "did:test:user1",
            99, // chat_id
            "time",
            Some(now - 1),
            None,
            "llm_callback",
            "Check the current time and tell the user.",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Mock LLM: round 1 calls notify_user tool (opt-in notification model),
    // round 2 returns final text (which is NOT sent to user in callback mode).
    let llm_state = Arc::new(LlmMockState::new(vec![
        llm_tool_call_response(
            "notify_user",
            r#"{"message":"The current time is 2026-03-04 02:30 UTC. Have a great day!"}"#,
        ),
        llm_text_response("Done."),
    ]));
    let (llm_addr, _llm_handle) = spawn_llm_mock(llm_state.clone()).await;

    let tg_state = Arc::new(TelegramMockState::new());
    let (tg_addr, _tg_handle) = spawn_telegram_mock(tg_state.clone()).await;
    let bot = make_bot(tg_addr);

    let settings = Arc::new(RuntimeSettings::new());
    let (progress_tx, mut progress_rx) =
        mpsc::channel::<aegis_runtime_core::progress_router::ProgressEvent>(8);
    let mut orch = build_orchestrator(llm_addr, memory.clone(), settings);
    orch.set_progress_tx(
        aegis_runtime_core::progress_router::ProgressRouterSender::direct(progress_tx),
    );
    let orch = Arc::new(orch);

    let (_event_tx, event_rx) = mpsc::channel::<IntentEvent>(256);
    let engine = DeferredIntentEngine::new(memory.clone(), orch, 0, event_rx)
        .with_poll_interval(Duration::from_secs(1));

    let handles = engine.start(bot);

    // Tool dispatch emits ToolStart ("⚙️ Using notify_user …") before the
    // owner notice. Wait for the notice itself; the tool-start line is not
    // the notification body.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut notice = None;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, progress_rx.recv()).await {
            Ok(Some(event)) => {
                let message =
                    aegis_runtime_core::progress_router::progress_event_to_legacy_string(&event);
                if message.contains("2026-03-04") {
                    notice = Some(event);
                    break;
                }
            }
            _ => break,
        }
    }

    for h in handles {
        h.abort();
    }

    let event = notice.expect("Engine should have emitted the notify_user notification");
    let chat_id = event.chat_id;
    let message = aegis_runtime_core::progress_router::progress_event_to_legacy_string(&event);
    assert_eq!(chat_id, 99);
    assert!(
        message.contains("2026-03-04"),
        "Response should contain the time from notify_user: {}",
        message
    );

    // LLM was called 3 times: notify_user tool call + final text + notification evaluator
    assert_eq!(llm_state.chat_call_count.load(Ordering::SeqCst), 3);

    // Intent marked as fired
    let pending = memory.list_user_intents("did:test:user1").await.unwrap();
    assert!(pending.is_empty());
}

/// **Scenario 3: Event-based intent fires when event is emitted.**
///
/// Simulates: user schedules an event intent for `email:new:primary`,
/// then the mail runtime emits that event → engine triggers → user gets notified.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_event_fires_on_channel_emit() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // Store an event intent
    memory
        .store_deferred_intent(
            "did:test:user1",
            77,
            "event",
            None,
            Some("email:new:primary"),
            "notify",
            "Skill #17 is ready for your approval!",
            now + 86400,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let tg_state = Arc::new(TelegramMockState::new());
    let (tg_addr, _tg_handle) = spawn_telegram_mock(tg_state.clone()).await;
    let bot = make_bot(tg_addr);

    let llm_state = Arc::new(LlmMockState::new(vec![]));
    let (llm_addr, _llm_handle) = spawn_llm_mock(llm_state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = Arc::new(build_orchestrator(llm_addr, memory.clone(), settings));

    let (event_tx, event_rx) = mpsc::channel::<IntentEvent>(256);
    let engine = DeferredIntentEngine::new(memory.clone(), orch, 0, event_rx)
        .with_poll_interval(Duration::from_secs(60)); // long poll — event should trigger via channel, not timer

    let handles = engine.start(bot);

    // Emit the event (simulating the mail runtime publishing new mail)
    event_tx
        .send(IntentEvent {
            key: "email:new:primary".to_string(),
            context: Some("Pipeline completed".to_string()),
            user_did: "did:test:user1".to_string(),
        })
        .await
        .unwrap();

    let messages = wait_for_messages(&tg_state, 1, Duration::from_secs(5)).await;

    for h in handles {
        h.abort();
    }

    assert!(
        !messages.is_empty(),
        "Event intent should have triggered a message"
    );
    assert_eq!(messages[0].chat_id, 77);
    assert_eq!(messages[0].text, "Skill #17 is ready for your approval!");

    // No LLM calls (notify mode)
    assert_eq!(llm_state.chat_call_count.load(Ordering::SeqCst), 0);

    // Intent marked as fired
    let pending = memory.list_user_intents("did:test:user1").await.unwrap();
    assert!(pending.is_empty());
}

/// **Scenario 4: Event-based llm_callback — full event notification flow.**
///
/// The canonical use case: LLM schedules an event intent for a mail account.
/// When the pipeline reaches `pending_final_approval`, the event fires, the engine
/// invokes the orchestrator (which calls the mock LLM), and the LLM's response
/// is sent to the user.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_event_llm_callback_full_flow() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    memory
        .store_deferred_intent(
            "did:test:user1",
            55,
            "event",
            None,
            Some("email:new:work"),
            "llm_callback",
            "New mail arrived in the work account. Summarize it for the user and ask what to do next.",
            now + 86400,
            false,
            None, 0,
            false,
            None, None,
            None,
            None, None, None, None,
        )
        .await
        .unwrap();

    // Round 1: LLM calls notify_user (opt-in notification model).
    // Round 2: LLM returns final text (not sent to user in callback mode).
    let llm_state = Arc::new(LlmMockState::new(vec![
        llm_tool_call_response(
            "notify_user",
            r#"{"message":"Great news! Your skill proposal #42 (crypto_alert) has passed all tests and is ready for final approval. Would you like me to activate it?"}"#,
        ),
        llm_text_response("Done."),
    ]));
    let (llm_addr, _llm_handle) = spawn_llm_mock(llm_state.clone()).await;

    let tg_state = Arc::new(TelegramMockState::new());
    let (tg_addr, _tg_handle) = spawn_telegram_mock(tg_state.clone()).await;
    let bot = make_bot(tg_addr);

    let settings = Arc::new(RuntimeSettings::new());
    let (progress_tx, mut progress_rx) =
        mpsc::channel::<aegis_runtime_core::progress_router::ProgressEvent>(8);
    let mut orch = build_orchestrator(llm_addr, memory.clone(), settings);
    orch.set_progress_tx(
        aegis_runtime_core::progress_router::ProgressRouterSender::direct(progress_tx),
    );
    let orch = Arc::new(orch);

    let (event_tx, event_rx) = mpsc::channel::<IntentEvent>(256);
    let engine = DeferredIntentEngine::new(memory.clone(), orch, 0, event_rx)
        .with_poll_interval(Duration::from_secs(60));

    let handles = engine.start(bot);

    // Emit event
    event_tx
        .send(IntentEvent {
            key: "email:new:work".to_string(),
            context: None,
            user_did: "did:test:user1".to_string(),
        })
        .await
        .unwrap();

    // Tool dispatch emits ToolStart ("⚙️ Using notify_user …") before the
    // owner notice. Wait for the notice itself; the tool-start line is not
    // the notification body.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut notice = None;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, progress_rx.recv()).await {
            Ok(Some(event)) => {
                let message =
                    aegis_runtime_core::progress_router::progress_event_to_legacy_string(&event);
                if message.contains("proposal #42") {
                    notice = Some(event);
                    break;
                }
            }
            _ => break,
        }
    }

    for h in handles {
        h.abort();
    }

    let event = notice.expect("Should have received notify_user notification");
    let chat_id = event.chat_id;
    let message = aegis_runtime_core::progress_router::progress_event_to_legacy_string(&event);
    assert_eq!(chat_id, 55);
    assert!(
        message.contains("proposal #42"),
        "Response should mention the proposal: {}",
        message
    );

    // LLM was called 3 times: notify_user tool call + final text + notification evaluator
    assert_eq!(llm_state.chat_call_count.load(Ordering::SeqCst), 3);

    // Fired
    let pending = memory.list_user_intents("did:test:user1").await.unwrap();
    assert!(pending.is_empty());
}

/// **Scenario 5: Non-matching event does NOT fire the intent.**
///
/// An event with a different key should not trigger the stored intent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_event_non_matching_key_ignored() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    memory
        .store_deferred_intent(
            "did:test:user1",
            10,
            "event",
            None,
            Some("email:new:archive"),
            "notify",
            "Should not fire",
            now + 86400,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let tg_state = Arc::new(TelegramMockState::new());
    let (tg_addr, _tg_handle) = spawn_telegram_mock(tg_state.clone()).await;
    let bot = make_bot(tg_addr);

    let llm_state = Arc::new(LlmMockState::new(vec![]));
    let (llm_addr, _llm_handle) = spawn_llm_mock(llm_state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = Arc::new(build_orchestrator(llm_addr, memory.clone(), settings));

    let (event_tx, event_rx) = mpsc::channel::<IntentEvent>(256);
    let engine = DeferredIntentEngine::new(memory.clone(), orch, 0, event_rx)
        .with_poll_interval(Duration::from_secs(60));

    let handles = engine.start(bot);

    // Emit a DIFFERENT event
    event_tx
        .send(IntentEvent {
            key: "email:new:other".to_string(),
            context: None,
            user_did: "did:test:user1".to_string(),
        })
        .await
        .unwrap();

    // Wait briefly — should NOT get any messages
    tokio::time::sleep(Duration::from_secs(2)).await;
    let messages = { tg_state.messages.lock().unwrap().clone() };

    for h in handles {
        h.abort();
    }

    assert!(
        messages.is_empty(),
        "Non-matching event should not trigger intent, but got {} messages",
        messages.len()
    );

    // Intent should still be pending
    let pending = memory.list_user_intents("did:test:user1").await.unwrap();
    assert_eq!(pending.len(), 1);
}

/// **Scenario 6: Expiry cleanup marks old intents as expired.**
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_expiry_cleanup_fires() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // Store 2 intents: one expired, one still valid
    memory
        .store_deferred_intent(
            "did:test:user1",
            10,
            "time",
            Some(now - 100),
            None,
            "notify",
            "old and expired",
            now - 10, // already past expiry
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    memory
        .store_deferred_intent(
            "did:test:user1",
            10,
            "time",
            Some(now + 3600),
            None,
            "notify",
            "still valid",
            now + 7200,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let tg_state = Arc::new(TelegramMockState::new());
    let (tg_addr, _tg_handle) = spawn_telegram_mock(tg_state.clone()).await;
    let bot = make_bot(tg_addr);

    let llm_state = Arc::new(LlmMockState::new(vec![]));
    let (llm_addr, _llm_handle) = spawn_llm_mock(llm_state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = Arc::new(build_orchestrator(llm_addr, memory.clone(), settings));

    let (_event_tx, event_rx) = mpsc::channel::<IntentEvent>(256);
    let engine = DeferredIntentEngine::new(memory.clone(), orch, 0, event_rx)
        .with_poll_interval(Duration::from_secs(60))
        .with_expiry_interval(Duration::from_secs(1)); // fast expiry for test

    let handles = engine.start(bot);

    // Wait for expiry cleanup to run
    tokio::time::sleep(Duration::from_secs(3)).await;

    for h in handles {
        h.abort();
    }

    // Only the valid intent should remain pending
    let pending = memory.list_user_intents("did:test:user1").await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].context_text, "still valid");
}

/// **Scenario 7: Multiple intents fire in the same poll cycle.**
///
/// Two time intents are both due — engine should execute both in one cycle.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_multiple_intents_fire_in_one_cycle() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    memory
        .store_deferred_intent(
            "did:test:user1",
            10,
            "time",
            Some(now - 10),
            None,
            "notify",
            "Message A",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    memory
        .store_deferred_intent(
            "did:test:user1",
            10,
            "time",
            Some(now - 5),
            None,
            "notify",
            "Message B",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let tg_state = Arc::new(TelegramMockState::new());
    let (tg_addr, _tg_handle) = spawn_telegram_mock(tg_state.clone()).await;
    let bot = make_bot(tg_addr);

    let llm_state = Arc::new(LlmMockState::new(vec![]));
    let (llm_addr, _llm_handle) = spawn_llm_mock(llm_state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = Arc::new(build_orchestrator(llm_addr, memory.clone(), settings));

    let (_event_tx, event_rx) = mpsc::channel::<IntentEvent>(256);
    let engine = DeferredIntentEngine::new(memory.clone(), orch, 0, event_rx)
        .with_poll_interval(Duration::from_secs(1));

    let handles = engine.start(bot);

    let messages = wait_for_messages(&tg_state, 2, Duration::from_secs(5)).await;

    for h in handles {
        h.abort();
    }

    assert_eq!(messages.len(), 2, "Both intents should have fired");
    let texts: Vec<&str> = messages.iter().map(|m| m.text.as_str()).collect();
    assert!(texts.contains(&"Message A"));
    assert!(texts.contains(&"Message B"));

    // Both marked as fired
    let pending = memory.list_user_intents("did:test:user1").await.unwrap();
    assert!(pending.is_empty());
}

/// **Scenario 8: Full E2E — orchestrator schedules intent → engine fires it.**
///
/// This is the most realistic test: the LLM (mock) calls schedule_intent,
/// the orchestrator stores it, then the engine picks it up and delivers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_full_e2e_schedule_then_fire() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());

    // Step 1: Use orchestrator to schedule an intent (mock LLM calls schedule_intent)
    let schedule_llm_state = Arc::new(LlmMockState::new(vec![
        // LLM calls schedule_intent
        json!({
            "id": "chatcmpl-1",
            "provider": "Mock",
            "object": "chat.completion",
            "created": 1700000000_u64,
            "model": "mock-model",
            "system_fingerprint": null,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "refusal": null,
                    "reasoning": null,
                    "reasoning_details": [],
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_sched",
                        "type": "function",
                        "function": {
                            "name": "schedule_intent",
                            "arguments": json!({
                                "trigger_type": "time",
                                "delay_seconds": 10,
                                "execution_mode": "notify",
                                "context_text": "Time's up! Check the server."
                            }).to_string()
                        }
                    }]
                },
                "logprobs": null,
                "finish_reason": "tool_calls",
                "native_finish_reason": "completed"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 10,
                "total_tokens": 20,
                "prompt_tokens_details": {},
                "completion_tokens_details": {},
                "cost_details": {}
            }
        })
        .to_string(),
        // LLM final reply
        llm_text_response("I've scheduled a check in 10 seconds."),
    ]));
    let (schedule_llm_addr, _h1) = spawn_llm_mock(schedule_llm_state).await;
    let schedule_orch = build_orchestrator(schedule_llm_addr, memory.clone(), settings.clone());

    let result = schedule_orch
        .process_message(
            "did:test:user1",
            "check the server in 10 seconds",
            Some(200),
        )
        .await
        .unwrap();
    assert_eq!(result.tools_called, vec!["schedule_intent"]);

    // Verify the intent was stored
    let intents = memory.list_user_intents("did:test:user1").await.unwrap();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].context_text, "Time's up! Check the server.");

    // Manually set trigger_at to now so the engine picks it up immediately
    // (We can't wait 10 real seconds in a test. The intent was stored with
    //  trigger_at = now + 10, so we update it directly.)
    let intent_id = intents[0].id;
    let now = chrono::Utc::now().timestamp();
    memory
        .mark_intent_trigger_at(intent_id, now - 1)
        .await
        .unwrap();

    // Step 2: Start the engine and wait for it to fire
    let tg_state = Arc::new(TelegramMockState::new());
    let (tg_addr, _tg_handle) = spawn_telegram_mock(tg_state.clone()).await;
    let bot = make_bot(tg_addr);

    let engine_llm_state = Arc::new(LlmMockState::new(vec![]));
    let (engine_llm_addr, _h2) = spawn_llm_mock(engine_llm_state.clone()).await;
    let orch = Arc::new(build_orchestrator(
        engine_llm_addr,
        memory.clone(),
        settings,
    ));

    let (_event_tx, event_rx) = mpsc::channel::<IntentEvent>(256);
    let engine = DeferredIntentEngine::new(memory.clone(), orch, 0, event_rx)
        .with_poll_interval(Duration::from_secs(1));

    let handles = engine.start(bot);

    let messages = wait_for_messages(&tg_state, 1, Duration::from_secs(5)).await;

    for h in handles {
        h.abort();
    }

    // Verify: the notify message was delivered
    assert!(
        !messages.is_empty(),
        "Engine should have delivered the notification"
    );
    assert_eq!(messages[0].chat_id, 200);
    assert_eq!(messages[0].text, "Time&#39;s up! Check the server.");

    // 0 LLM calls from engine (notify mode)
    assert_eq!(engine_llm_state.chat_call_count.load(Ordering::SeqCst), 0);

    // Intent marked as fired
    let pending = memory.list_user_intents("did:test:user1").await.unwrap();
    assert!(pending.is_empty());
}
