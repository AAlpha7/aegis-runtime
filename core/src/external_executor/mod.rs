//! Stdio JSON-RPC client that runs one task on grok build (ACP) or Codex
//! (app-server) and returns a receipt — the delegation link decided by the owner on
//! 2026-09-07 (the plan record is archived outside the repo). This module owns process
//! transport, the JSON-RPC line protocol, and the two executor
//! implementations; orchestrator wiring (pseudo-tool, settings, receipts)
//! lives elsewhere.

mod codex_app_server;
mod grok_acp;
mod jsonrpc;
mod transport;

pub use codex_app_server::CodexAppServerExecutor;
pub use grok_acp::GrokAcpExecutor;
pub use transport::{AgentCommand, AgentIo, AgentTransport, ProcessTransport};

use std::pin::Pin;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorKind {
    GrokAcp,
    CodexAppServer,
}

#[derive(Debug, Clone)]
pub struct ExecutorRequest {
    pub prompt: String,
    pub cwd: std::path::PathBuf,
    pub timeout: std::time::Duration,
    /// Model id override; `None` = the executor's configured default.
    pub model: Option<String>,
    /// "low" | "medium" | "high"; `None` = the executor's configured default.
    pub reasoning_effort: Option<String>,
    /// Search-only runs enable live search and use a read-only sandbox.
    pub web_search: bool,
    /// Exact app id or display name; verified through app/list before any turn.
    pub required_connector: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExecutorUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExecutorToolCall {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutorReceipt {
    pub kind: ExecutorKind,
    pub session_id: Option<String>,
    pub model: Option<String>,
    /// "end_turn" | "cancelled" | "timeout" | "error:<short>" — never empty.
    pub stop_reason: String,
    /// Concatenated agent message text (thoughts excluded).
    pub final_text: String,
    pub tool_calls: Vec<ExecutorToolCall>,
    pub usage: ExecutorUsage,
    pub elapsed_ms: u64,
    /// Last 4 KiB of agent stderr; `None` when empty or unavailable.
    pub stderr_tail: Option<String>,
    /// Agent-reported failure detail (e.g. a Codex `turn.error.message`);
    /// `None` when the turn ended normally.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorUpdate {
    Thought(String),
    Message(String),
    ToolCall {
        id: String,
        title: String,
        status: String,
    },
    Plan(String),
}

pub type UpdateSink<'a> = &'a (dyn Fn(ExecutorUpdate) + Send + Sync);
pub type ExecutorFuture<'a> =
    Pin<Box<dyn std::future::Future<Output = anyhow::Result<ExecutorReceipt>> + Send + 'a>>;

pub trait ExternalExecutor: Send + Sync {
    fn kind(&self) -> ExecutorKind;
    fn run<'a>(&'a self, request: ExecutorRequest, on_update: UpdateSink<'a>)
    -> ExecutorFuture<'a>;
}

/// Binary/model defaults, read once from the environment (never from settings — the orchestrator layer decides policy).
///
/// Every environment variable this module reads (all optional; empty = unset):
///
/// | Variable | Default | Purpose |
/// |---|---|---|
/// | `AEGIS_EXECUTOR_GROK_BIN` | `grok` | grok build CLI binary (name on `PATH` or absolute path) |
/// | `AEGIS_EXECUTOR_GROK_MODEL` | `grok-4.7` | model passed to grok build (ACP) |
/// | `AEGIS_EXECUTOR_GROK_EFFORT` | `high` | reasoning effort `low` / `medium` / `high` |
/// | `AEGIS_EXECUTOR_GROK_SANDBOX` | unset | value for grok's `--sandbox` flag; unset = no flag |
/// | `AEGIS_EXECUTOR_CODEX_BIN` | `codex` | Codex CLI binary (app-server mode) |
/// | `AEGIS_EXECUTOR_CODEX_MODEL` | `gpt-6-astra` | model passed to Codex |
/// | `AEGIS_EXECUTOR_CODEX_EFFORT` | `high` | reasoning effort `low` / `medium` / `high` |
/// | `AEGIS_EXECUTOR_CODEX_HOME` | unset | `CODEX_HOME` exported to the Codex child (auth/config dir); unset = inherit |
///
/// The scratch-directory parent (`AEGIS_EXECUTOR_RUNS_DIR`) is read by the
/// `run_external` tool handler in `orchestrator_tools_external`, and the
/// binary resolves its default in `main.rs` (`resolve_executor_runs_dir`):
/// `<dir of MEMORY_DB_PATH>/executor-runs`, else the legacy
/// `/opt/aegis-runtime/shared/executor-runs` when that directory exists, else
/// `./executor-runs`.
///
/// Child processes do NOT inherit the host environment: [`ProcessTransport`]
/// re-exports only `PATH`, `HOME`, `LANG`, `LC_ALL`, `TERM`, `XDG_RUNTIME_DIR`,
/// `TMPDIR` and every variable starting with `GROK_`, `CODEX_` or `XAI_`
/// except API-key variables (so application secrets and paid API keys do not
/// automatically reach the agent).
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    pub grok_bin: String,             // AEGIS_EXECUTOR_GROK_BIN, default "grok"
    pub grok_model: String,           // AEGIS_EXECUTOR_GROK_MODEL, default "grok-4.7"
    pub grok_effort: String,          // AEGIS_EXECUTOR_GROK_EFFORT, default "high"
    pub grok_sandbox: Option<String>, // AEGIS_EXECUTOR_GROK_SANDBOX, default None (no --sandbox flag)
    pub codex_bin: String,            // AEGIS_EXECUTOR_CODEX_BIN, default "codex"
    pub codex_model: String,          // AEGIS_EXECUTOR_CODEX_MODEL, default "gpt-6-astra"
    pub codex_effort: String,         // AEGIS_EXECUTOR_CODEX_EFFORT, default "high"
    pub codex_home: Option<String>,   // AEGIS_EXECUTOR_CODEX_HOME, default None (inherit)
}

impl ExecutorConfig {
    pub fn from_env() -> Self {
        let vars: Vec<(String, String)> = std::env::vars().collect();
        Self::from_env_map(&vars)
    }

    pub(crate) fn from_env_map(vars: &[(String, String)]) -> Self {
        let get = |key: &str| -> Option<String> {
            vars.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .filter(|v| !v.is_empty())
        };
        let required = |key: &str, default: &str| get(key).unwrap_or_else(|| default.to_string());
        Self {
            grok_bin: required("AEGIS_EXECUTOR_GROK_BIN", "grok"),
            grok_model: required("AEGIS_EXECUTOR_GROK_MODEL", "grok-4.7"),
            grok_effort: required("AEGIS_EXECUTOR_GROK_EFFORT", "high"),
            grok_sandbox: get("AEGIS_EXECUTOR_GROK_SANDBOX"),
            codex_bin: required("AEGIS_EXECUTOR_CODEX_BIN", "codex"),
            codex_model: required("AEGIS_EXECUTOR_CODEX_MODEL", "gpt-6-astra"),
            codex_effort: required("AEGIS_EXECUTOR_CODEX_EFFORT", "high"),
            codex_home: get("AEGIS_EXECUTOR_CODEX_HOME"),
        }
    }
}

pub(crate) fn json_u64(value: &serde_json::Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            value
                .get(key)
                .and_then(serde_json::Value::as_i64)
                .and_then(|n| u64::try_from(n).ok())
        })
        .unwrap_or(0)
}

pub(crate) fn json_str<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(serde_json::Value::as_str)
}

pub(crate) fn is_timeout_error(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| cause.to_string().contains("timeout waiting for "))
}

pub(crate) fn never_empty_stop_reason(reason: Option<&str>, fallback: &str) -> String {
    let trimmed = reason.map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn cwd_string(cwd: &std::path::Path) -> String {
    cwd.to_string_lossy().into_owned()
}

pub(crate) fn elapsed_ms(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_executor::transport::child_env;
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    struct FakeTransport {
        script: Arc<dyn Fn(&Value) -> Vec<Value> + Send + Sync>,
        inbound: Arc<Mutex<Vec<Value>>>,
        last_command: Arc<Mutex<Option<AgentCommand>>>,
        stderr_lines: Vec<String>,
    }

    impl FakeTransport {
        fn new(script: impl Fn(&Value) -> Vec<Value> + Send + Sync + 'static) -> Arc<Self> {
            Self::with_stderr(script, &[])
        }

        fn with_stderr(
            script: impl Fn(&Value) -> Vec<Value> + Send + Sync + 'static,
            lines: &[&str],
        ) -> Arc<Self> {
            Arc::new(Self {
                script: Arc::new(script),
                inbound: Arc::new(Mutex::new(Vec::new())),
                last_command: Arc::new(Mutex::new(None)),
                stderr_lines: lines.iter().map(|line| (*line).to_string()).collect(),
            })
        }

        fn inbound(&self) -> Vec<Value> {
            self.inbound.lock().expect("inbound mutex").clone()
        }

        fn last_command(&self) -> Option<AgentCommand> {
            self.last_command.lock().expect("command mutex").clone()
        }
    }

    impl AgentTransport for FakeTransport {
        fn spawn(&self, command: &AgentCommand) -> anyhow::Result<AgentIo> {
            *self.last_command.lock().expect("command mutex") = Some(command.clone());
            let (client_to_server, server_from_client) = tokio::io::duplex(64 * 1024);
            let (server_to_client, client_from_server) = tokio::io::duplex(64 * 1024);
            let script = Arc::clone(&self.script);
            let inbound = Arc::clone(&self.inbound);
            let stderr = if self.stderr_lines.is_empty() {
                None
            } else {
                let (client, mut server) = tokio::io::duplex(64 * 1024);
                let mut payload = String::new();
                for line in &self.stderr_lines {
                    payload.push_str(line);
                    payload.push('\n');
                }
                write_duplex_now(&mut server, payload.as_bytes());
                drop(server);
                Some(Box::new(client) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)
            };
            tokio::spawn(async move {
                let mut reader = BufReader::new(server_from_client);
                let mut writer = server_to_client;
                let mut buf = String::new();
                loop {
                    buf.clear();
                    match reader.read_line(&mut buf).await {
                        Ok(0) => break,
                        Ok(_) => {
                            let trimmed = buf.trim();
                            let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
                                continue;
                            };
                            inbound.lock().expect("inbound mutex").push(value.clone());
                            for outgoing in script(&value) {
                                let encoded = if let Some(raw) = outgoing.as_str() {
                                    raw.to_string()
                                } else {
                                    let Ok(line) = serde_json::to_string(&outgoing) else {
                                        continue;
                                    };
                                    line
                                };
                                if writer.write_all(encoded.as_bytes()).await.is_err() {
                                    return;
                                }
                                if writer.write_all(b"\n").await.is_err() {
                                    return;
                                }
                                if writer.flush().await.is_err() {
                                    return;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
            Ok(AgentIo {
                stdin: Box::new(client_to_server),
                stdout: Box::new(BufReader::new(client_from_server)),
                child: None,
                stderr,
            })
        }
    }

    fn default_config() -> ExecutorConfig {
        ExecutorConfig::from_env_map(&[])
    }

    fn request_with_timeout(timeout: Duration) -> ExecutorRequest {
        ExecutorRequest {
            prompt: "ping".to_string(),
            cwd: PathBuf::from("/work"),
            timeout,
            model: None,
            reasoning_effort: None,
            web_search: false,
            required_connector: None,
        }
    }

    fn rpc_result(req: &Value, result: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": req.get("id").cloned().unwrap_or(Value::Null),
            "result": result,
        })
    }

    fn rpc_error(req: &Value, error: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": req.get("id").cloned().unwrap_or(Value::Null),
            "error": error,
        })
    }

    fn grok_update(session_update: &str, extra: Value) -> Value {
        let mut update = extra;
        if let Some(obj) = update.as_object_mut() {
            obj.insert(
                "sessionUpdate".to_string(),
                Value::String(session_update.to_string()),
            );
        }
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": { "update": update },
        })
    }

    fn silent_sink() -> impl Fn(ExecutorUpdate) + Send + Sync {
        |_| {}
    }

    fn collect_sink(
        into: &Arc<Mutex<Vec<ExecutorUpdate>>>,
    ) -> impl Fn(ExecutorUpdate) + Send + Sync {
        let into = Arc::clone(into);
        move |update| into.lock().expect("updates mutex").push(update)
    }

    fn method_of(value: &Value) -> Option<&str> {
        value.get("method").and_then(Value::as_str)
    }

    fn write_duplex_now(writer: &mut tokio::io::DuplexStream, data: &[u8]) {
        use std::pin::Pin;
        use std::task::{Context, Poll};
        use tokio::io::AsyncWrite;
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut rest = data;
        while !rest.is_empty() {
            match Pin::new(&mut *writer).poll_write(&mut cx, rest) {
                Poll::Ready(Ok(0)) | Poll::Pending | Poll::Ready(Err(_)) => break,
                Poll::Ready(Ok(n)) => rest = &rest[n..],
            }
        }
        let _ = Pin::new(&mut *writer).poll_flush(&mut cx);
    }

    #[test]
    fn executor_config_from_env_defaults() {
        let cfg = ExecutorConfig::from_env_map(&[]);
        assert_eq!(cfg.grok_bin, "grok");
        assert_eq!(cfg.grok_model, "grok-4.7");
        assert_eq!(cfg.grok_effort, "high");
        assert_eq!(cfg.grok_sandbox, None);
        assert_eq!(cfg.codex_bin, "codex");
        assert_eq!(cfg.codex_model, "gpt-6-astra");
        assert_eq!(cfg.codex_effort, "high");
        assert_eq!(cfg.codex_home, None);
    }

    #[test]
    fn child_env_drops_aegis_secrets_keeps_allow_list() {
        let host = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOME".to_string(), "/home/owner".to_string()),
            ("LANG".to_string(), "C".to_string()),
            ("AEGIS_SECRET".to_string(), "nope".to_string()),
            ("TELEGRAM_BOT_TOKEN".to_string(), "nope".to_string()),
            ("HERCULES_TOKEN".to_string(), "nope".to_string()),
            ("OPENROUTER_API_KEY".to_string(), "nope".to_string()),
            ("ADMIN_API_TOKEN".to_string(), "nope".to_string()),
            ("GROK_API_KEY".to_string(), "keep-grok".to_string()),
            ("CODEX_API_KEY".to_string(), "keep-codex".to_string()),
            ("XAI_API_KEY".to_string(), "keep-xai".to_string()),
        ];
        let extra = vec![("CODEX_HOME".to_string(), "/tmp/codex".to_string())];
        let env = child_env(&host, &extra);
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"PATH"));
        assert!(keys.contains(&"HOME"));
        assert!(keys.contains(&"LANG"));
        assert!(!keys.contains(&"GROK_API_KEY"));
        assert!(!keys.contains(&"CODEX_API_KEY"));
        assert!(!keys.contains(&"XAI_API_KEY"));
        assert!(keys.contains(&"CODEX_HOME"));
        assert!(!keys.iter().any(|k| k.starts_with("AEGIS_")));
        assert!(!keys.iter().any(|k| k.starts_with("TELEGRAM_")));
        assert!(!keys.iter().any(|k| k.starts_with("HERCULES_")));
        assert!(!keys.contains(&"OPENROUTER_API_KEY"));
        assert!(!keys.contains(&"ADMIN_API_TOKEN"));
    }

    #[tokio::test]
    async fn grok_happy_path_receipt_fields() {
        let fake = FakeTransport::new(|req| match method_of(req) {
            Some("initialize") => vec![rpc_result(
                req,
                json!({"protocolVersion": 1, "agentCapabilities": {}}),
            )],
            Some("session/new") => vec![rpc_result(
                req,
                json!({
                    "sessionId": "sess-1",
                    "models": {"currentModelId": "grok-4.5"}
                }),
            )],
            Some("session/prompt") => vec![
                grok_update(
                    "agent_thought_chunk",
                    json!({"content": {"text": "thinking"}}),
                ),
                grok_update("agent_message_chunk", json!({"content": {"text": "PONG"}})),
                grok_update(
                    "tool_call",
                    json!({
                        "toolCallId": "tc-1",
                        "title": "read README",
                        "kind": "read",
                        "status": "pending"
                    }),
                ),
                grok_update(
                    "tool_call_update",
                    json!({
                        "toolCallId": "tc-1",
                        "status": "completed",
                        "title": "read README.md"
                    }),
                ),
                rpc_result(
                    req,
                    json!({
                        "stopReason": "end_turn",
                        "_meta": {
                            "sessionId": "sess-1",
                            "modelId": "grok-4.6",
                            "inputTokens": 10,
                            "outputTokens": 4,
                            "reasoningTokens": 2,
                            "totalTokens": 16
                        }
                    }),
                ),
            ],
            _ => Vec::new(),
        });
        let exec = GrokAcpExecutor::new(default_config(), fake.clone());
        let updates = Arc::new(Mutex::new(Vec::new()));
        let sink = collect_sink(&updates);
        let receipt = exec
            .run(request_with_timeout(Duration::from_secs(2)), &sink)
            .await
            .expect("grok happy path");
        assert_eq!(receipt.kind, ExecutorKind::GrokAcp);
        assert_eq!(receipt.session_id.as_deref(), Some("sess-1"));
        assert_eq!(receipt.model.as_deref(), Some("grok-4.6"));
        assert_eq!(receipt.stop_reason, "end_turn");
        assert_eq!(receipt.final_text, "PONG");
        assert_eq!(
            receipt.tool_calls,
            vec![ExecutorToolCall {
                id: "tc-1".to_string(),
                title: "read README.md".to_string(),
                kind: "read".to_string(),
                status: "completed".to_string(),
            }]
        );
        assert_eq!(
            receipt.usage,
            ExecutorUsage {
                input_tokens: 10,
                output_tokens: 4,
                reasoning_tokens: 2,
                total_tokens: 16,
            }
        );
        assert!(!receipt.stop_reason.is_empty());
        assert_eq!(
            fake.inbound()
                .iter()
                .find(|msg| method_of(msg) == Some("initialize"))
                .and_then(|msg| msg.pointer("/params/clientCapabilities/fs/readTextFile"))
                .and_then(Value::as_bool),
            Some(false)
        );
        let updates = updates.lock().expect("updates");
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, ExecutorUpdate::Thought(t) if t == "thinking"))
        );
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, ExecutorUpdate::Message(t) if t == "PONG"))
        );
        let cmd = fake.last_command().expect("spawned command");
        assert_eq!(
            cmd.args,
            vec![
                "agent",
                "--always-approve",
                "--model",
                "grok-4.7",
                "--reasoning-effort",
                "high",
                "stdio"
            ]
        );
    }

    #[tokio::test]
    async fn grok_answers_permission_request_with_cancelled() {
        let prompt_id = Arc::new(Mutex::new(None::<Value>));
        let prompt_id_script = Arc::clone(&prompt_id);
        let fake = FakeTransport::new(move |req| match method_of(req) {
            Some("initialize") => vec![rpc_result(req, json!({"protocolVersion": 1}))],
            Some("session/new") => {
                vec![rpc_result(req, json!({"sessionId": "sess-perm"}))]
            }
            Some("session/prompt") => {
                *prompt_id_script.lock().expect("prompt id") = req.get("id").cloned();
                vec![json!({
                    "jsonrpc": "2.0",
                    "id": 9001,
                    "method": "session/request_permission",
                    "params": {"sessionId": "sess-perm"}
                })]
            }
            _ => {
                if req.get("id") == Some(&json!(9001))
                    && req
                        .pointer("/result/outcome/outcome")
                        .and_then(Value::as_str)
                        == Some("cancelled")
                {
                    let id = prompt_id_script
                        .lock()
                        .expect("prompt id")
                        .clone()
                        .unwrap_or(json!(3));
                    return vec![json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {"stopReason": "end_turn"}
                    })];
                }
                Vec::new()
            }
        });
        let exec = GrokAcpExecutor::new(default_config(), fake.clone());
        let sink = silent_sink();
        let receipt = exec
            .run(request_with_timeout(Duration::from_secs(2)), &sink)
            .await
            .expect("permission round-trip");
        assert_eq!(receipt.stop_reason, "end_turn");
        assert_eq!(receipt.session_id.as_deref(), Some("sess-perm"));
        assert!(
            fake.inbound().iter().any(|msg| {
                msg.get("id") == Some(&json!(9001))
                    && msg
                        .pointer("/result/outcome/outcome")
                        .and_then(Value::as_str)
                        == Some("cancelled")
            }),
            "expected cancelled permission result, inbound={:?}",
            fake.inbound()
        );
    }

    #[tokio::test]
    async fn grok_timeout_returns_timeout_receipt() {
        let fake = FakeTransport::new(|req| match method_of(req) {
            Some("initialize") => vec![rpc_result(req, json!({"protocolVersion": 1}))],
            Some("session/new") => vec![rpc_result(req, json!({"sessionId": "sess-to"}))],
            Some("session/prompt") => vec![grok_update(
                "agent_message_chunk",
                json!({"content": {"text": "partial"}}),
            )],
            _ => Vec::new(),
        });
        let exec = GrokAcpExecutor::new(default_config(), fake);
        let sink = silent_sink();
        let started = std::time::Instant::now();
        let receipt = exec
            .run(request_with_timeout(Duration::from_millis(200)), &sink)
            .await
            .expect("timeout is a receipt");
        assert_eq!(receipt.stop_reason, "timeout");
        assert_eq!(receipt.final_text, "partial");
        assert_eq!(receipt.session_id.as_deref(), Some("sess-to"));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(receipt.elapsed_ms < 5_000);
    }

    #[tokio::test]
    async fn grok_error_response_is_err() {
        let fake = FakeTransport::new(|req| match method_of(req) {
            Some("initialize") => vec![rpc_result(req, json!({"protocolVersion": 1}))],
            Some("session/new") => vec![rpc_result(req, json!({"sessionId": "sess-err"}))],
            Some("session/prompt") => vec![rpc_error(
                req,
                json!({"code": -32000, "message": "model overloaded"}),
            )],
            _ => Vec::new(),
        });
        let exec = GrokAcpExecutor::new(default_config(), fake);
        let sink = silent_sink();
        let err = exec
            .run(request_with_timeout(Duration::from_secs(2)), &sink)
            .await
            .expect_err("error response");
        let text = err.to_string();
        assert!(
            text.contains("model overloaded") || text.contains("-32000"),
            "error should include JSON-RPC error text, got {text}"
        );
    }

    #[tokio::test]
    async fn stderr_tail_on_receipt_and_error_context() {
        let script_error = |req: &Value| match method_of(req) {
            Some("initialize") => vec![rpc_result(req, json!({"protocolVersion": 1}))],
            Some("session/new") => vec![rpc_result(req, json!({"sessionId": "sess-err"}))],
            Some("session/prompt") => vec![rpc_error(
                req,
                json!({"code": -32000, "message": "model overloaded"}),
            )],
            _ => Vec::new(),
        };
        let fake = FakeTransport::with_stderr(script_error, &["runner boom", "hint: sandbox"]);
        let exec = GrokAcpExecutor::new(default_config(), fake);
        let sink = silent_sink();
        let err = exec
            .run(request_with_timeout(Duration::from_secs(2)), &sink)
            .await
            .expect_err("error response with stderr");
        let text = err.to_string();
        assert!(
            text.contains("stderr:"),
            "error context should mention stderr, got {text}"
        );
        assert!(
            text.contains("runner boom"),
            "error context should include first stderr line, got {text}"
        );
        assert!(
            text.contains("hint: sandbox"),
            "error context should include second stderr line, got {text}"
        );

        let script_timeout = |req: &Value| match method_of(req) {
            Some("initialize") => vec![rpc_result(req, json!({"protocolVersion": 1}))],
            Some("session/new") => vec![rpc_result(req, json!({"sessionId": "sess-to"}))],
            Some("session/prompt") => Vec::new(),
            _ => Vec::new(),
        };
        let fake = FakeTransport::with_stderr(script_timeout, &["runner boom", "hint: sandbox"]);
        let exec = GrokAcpExecutor::new(default_config(), fake);
        let sink = silent_sink();
        let receipt = exec
            .run(request_with_timeout(Duration::from_millis(250)), &sink)
            .await
            .expect("timeout receipt");
        assert_eq!(receipt.stop_reason, "timeout");
        let tail = receipt.stderr_tail.expect("stderr_tail filled on timeout");
        assert!(
            tail.contains("runner boom"),
            "receipt stderr_tail missing first line: {tail}"
        );
        assert!(
            tail.contains("hint: sandbox"),
            "receipt stderr_tail missing second line: {tail}"
        );
    }

    #[tokio::test]
    async fn grok_skips_malformed_line() {
        let fake = FakeTransport::new(|req| match method_of(req) {
            Some("initialize") => vec![rpc_result(req, json!({"protocolVersion": 1}))],
            Some("session/new") => vec![rpc_result(req, json!({"sessionId": "sess-bad"}))],
            Some("session/prompt") => vec![
                json!("this is not a json-rpc object"),
                rpc_result(req, json!({"stopReason": "end_turn"})),
            ],
            _ => Vec::new(),
        });
        let exec = GrokAcpExecutor::new(default_config(), fake);
        let sink = silent_sink();
        let receipt = exec
            .run(request_with_timeout(Duration::from_secs(2)), &sink)
            .await
            .expect("malformed line skipped");
        assert_eq!(receipt.stop_reason, "end_turn");
        assert_eq!(receipt.session_id.as_deref(), Some("sess-bad"));
    }

    fn codex_initialize_and_thread(req: &Value) -> Vec<Value> {
        match method_of(req) {
            Some("account/read") => vec![rpc_result(req, json!({"account":{"type":"chatgpt"}}))],
            Some("initialize") => vec![rpc_result(req, json!({"ok": true}))],
            Some("thread/start") => {
                assert!(
                    matches!(
                        req["params"]["sandbox"].as_str(),
                        Some("read-only" | "workspace-write")
                    ),
                    "use the real app-server SandboxMode wire values"
                );
                vec![rpc_result(req, json!({"thread": {"id": "thread-1"}}))]
            }
            _ => Vec::new(),
        }
    }

    fn codex_turn_events(req: &Value) -> Vec<Value> {
        vec![
            json!({
                "jsonrpc": "2.0",
                "method": "item/agentMessage/delta",
                "params": {"delta": "Hel"}
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "item/agentMessage/delta",
                "params": {"delta": "lo"}
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "item/completed",
                "params": {
                    "item": {
                        "id": "cmd-1",
                        "type": "commandExecution",
                        "command": "ls",
                        "status": "completed"
                    }
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "thread/tokenUsage/updated",
                "params": {
                    "tokenUsage": {
                        "last": {
                            "inputTokens": 11,
                            "outputTokens": 5,
                            "reasoningOutputTokens": 3,
                            "totalTokens": 19
                        }
                    }
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "turn/completed",
                "params": {
                    "turn": {
                        "status": "completed",
                        "items": [{
                            "type": "agentMessage",
                            "id": "msg-1",
                            "text": "Hello"
                        }]
                    }
                }
            }),
            rpc_result(req, json!({"turn": {"id": "turn-1"}})),
        ]
    }

    #[tokio::test]
    async fn codex_search_enables_live_read_only_and_records_tool_evidence() {
        let fake = FakeTransport::new(|req| {
            let mut lines = codex_initialize_and_thread(req);
            if method_of(req) == Some("turn/start") {
                lines.push(json!({"jsonrpc":"2.0", "method":"item/completed", "params":{"item":{"id":"s1","type":"webSearch","query":"test"}}}));
                lines.extend(codex_turn_events(req));
            }
            lines
        });
        let mut request = request_with_timeout(Duration::from_secs(2));
        request.web_search = true;
        let receipt = CodexAppServerExecutor::new(default_config(), fake.clone())
            .run(request, &silent_sink())
            .await
            .unwrap();
        assert!(
            receipt
                .tool_calls
                .iter()
                .any(|t| t.kind == "webSearch" && t.status == "completed")
        );
        let inbound = fake.inbound();
        let thread = inbound
            .iter()
            .find(|v| method_of(v) == Some("thread/start"))
            .unwrap();
        assert_eq!(thread["params"]["sandbox"], "read-only");
        assert_eq!(thread["params"]["modelProvider"], "openai");
        assert_eq!(thread["params"]["config"]["web_search"], "live");
        assert_eq!(thread["params"]["config"]["apps._default.enabled"], false);
        assert_eq!(thread["params"]["config"]["features.shell_tool"], false);
    }

    #[tokio::test]
    async fn codex_connector_checks_access_before_turn_and_supports_pagination() {
        for (accessible, enabled) in [(false, true), (true, false), (true, true)] {
            let fake = FakeTransport::new(move |req| {
                let mut lines = codex_initialize_and_thread(req);
                match method_of(req) {
                    Some("app/list") => {
                        let response = if req["params"]["cursor"].is_null() {
                            json!({"data":[], "nextCursor":"page2"})
                        } else {
                            json!({"data":[{"id":"mail-app","name":"Gmail","isAccessible":accessible,"isEnabled":enabled}],"nextCursor":null})
                        };
                        lines.push(rpc_result(req, response));
                    }
                    Some("turn/start") => lines.extend(codex_turn_events(req)),
                    _ => {}
                }
                lines
            });
            let mut request = request_with_timeout(Duration::from_secs(2));
            request.required_connector = Some("Gmail".into());
            let result = CodexAppServerExecutor::new(default_config(), fake.clone())
                .run(request, &silent_sink())
                .await;
            let inbound = fake.inbound();
            let turn = inbound.iter().find(|v| method_of(v) == Some("turn/start"));
            assert_eq!(result.is_ok(), accessible && enabled);
            assert_eq!(turn.is_some(), accessible && enabled);
            if let Some(turn) = turn {
                assert_eq!(turn["params"]["input"][1]["path"], "app://mail-app");
            }
        }
    }

    #[tokio::test]
    async fn codex_api_key_account_never_starts_a_task() {
        let fake = FakeTransport::new(|req| {
            if method_of(req) == Some("account/read") {
                vec![rpc_result(req, json!({"account":{"type":"apiKey"}}))]
            } else {
                codex_initialize_and_thread(req)
            }
        });
        let result = CodexAppServerExecutor::new(default_config(), fake.clone())
            .run(request_with_timeout(Duration::from_secs(2)), &silent_sink())
            .await;
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("subscription login")
        );
        assert!(
            !fake
                .inbound()
                .iter()
                .any(|v| method_of(v) == Some("thread/start"))
        );
    }

    #[tokio::test]
    async fn codex_happy_path_receipt() {
        let fake = FakeTransport::new(|req| {
            let mut lines = codex_initialize_and_thread(req);
            if method_of(req) == Some("turn/start") {
                lines.extend(codex_turn_events(req));
            }
            lines
        });
        let exec = CodexAppServerExecutor::new(default_config(), fake.clone());
        let sink = silent_sink();
        let receipt = exec
            .run(request_with_timeout(Duration::from_secs(2)), &sink)
            .await
            .expect("codex happy path");
        assert_eq!(receipt.kind, ExecutorKind::CodexAppServer);
        assert_eq!(receipt.session_id.as_deref(), Some("thread-1"));
        assert_eq!(receipt.model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(receipt.stop_reason, "completed");
        assert_eq!(receipt.final_text, "Hello");
        assert_eq!(
            receipt.tool_calls,
            vec![ExecutorToolCall {
                id: "cmd-1".to_string(),
                title: "ls".to_string(),
                kind: "commandExecution".to_string(),
                status: "completed".to_string(),
            }]
        );
        assert_eq!(
            receipt.usage,
            ExecutorUsage {
                input_tokens: 11,
                output_tokens: 5,
                reasoning_tokens: 3,
                total_tokens: 19,
            }
        );
        let cmd = fake.last_command().expect("spawned command");
        assert_eq!(cmd.args, vec!["app-server", "--listen", "stdio://"]);
        assert!(cmd.env.is_empty());
    }

    #[tokio::test]
    async fn codex_failed_turn_reports_error() {
        let fake = FakeTransport::new(|req| {
            let mut lines = codex_initialize_and_thread(req);
            if method_of(req) == Some("turn/start") {
                lines.push(json!({
                    "jsonrpc": "2.0",
                    "method": "error",
                    "params": {"error": {"message": "model requires a newer Codex"}, "willRetry": false}
                }));
                lines.push(json!({
                    "jsonrpc": "2.0",
                    "method": "turn/completed",
                    "params": {"turn": {"status": "failed", "items": [], "error": {"message": "model requires a newer Codex"}}}
                }));
                lines.push(rpc_result(req, json!({"turn": {"id": "turn-x"}})));
            }
            lines
        });
        let exec = CodexAppServerExecutor::new(default_config(), fake);
        let sink = silent_sink();
        let receipt = exec
            .run(request_with_timeout(Duration::from_secs(2)), &sink)
            .await
            .expect("failed turn is still a receipt");
        assert_eq!(receipt.stop_reason, "failed");
        assert_eq!(
            receipt.error.as_deref(),
            Some("model requires a newer Codex")
        );
        assert!(receipt.final_text.is_empty());
    }

    #[tokio::test]
    async fn codex_answers_approval_with_cancel() {
        let turn_id = Arc::new(Mutex::new(None::<Value>));
        let turn_id_script = Arc::clone(&turn_id);
        let fake = FakeTransport::new(move |req| {
            let mut lines = codex_initialize_and_thread(req);
            if method_of(req) == Some("turn/start") {
                *turn_id_script.lock().expect("turn id") = req.get("id").cloned();
                lines.push(json!({
                    "jsonrpc": "2.0",
                    "id": 42,
                    "method": "item/commandExecution/requestApproval",
                    "params": {"item": {"id": "cmd-pending"}}
                }));
                return lines;
            }
            if req.get("id") == Some(&json!(42))
                && req.pointer("/result/decision").and_then(Value::as_str) == Some("cancel")
            {
                let id = turn_id_script
                    .lock()
                    .expect("turn id")
                    .clone()
                    .unwrap_or(json!(3));
                let mut rest = Vec::new();
                rest.extend(codex_turn_events(&json!({"id": id})));
                return rest;
            }
            lines
        });
        let exec = CodexAppServerExecutor::new(default_config(), fake.clone());
        let sink = silent_sink();
        let receipt = exec
            .run(request_with_timeout(Duration::from_secs(2)), &sink)
            .await
            .expect("codex approval round-trip");
        assert_eq!(receipt.stop_reason, "completed");
        assert_eq!(receipt.final_text, "Hello");
        assert!(
            fake.inbound().iter().any(|msg| {
                msg.get("id") == Some(&json!(42))
                    && msg.pointer("/result/decision").and_then(Value::as_str) == Some("cancel")
            }),
            "expected cancel decision, inbound={:?}",
            fake.inbound()
        );
    }

    #[tokio::test]
    async fn grok_and_codex_command_argument_order() {
        let grok_cfg = ExecutorConfig {
            grok_sandbox: Some("all".to_string()),
            ..default_config()
        };
        let grok_fake = FakeTransport::new(|req| match method_of(req) {
            Some("initialize") => vec![rpc_result(req, json!({"protocolVersion": 1}))],
            Some("session/new") => vec![rpc_result(req, json!({"sessionId": "s"}))],
            Some("session/prompt") => vec![rpc_result(req, json!({"stopReason": "end_turn"}))],
            _ => Vec::new(),
        });
        let grok = GrokAcpExecutor::new(grok_cfg, grok_fake.clone());
        let mut req = request_with_timeout(Duration::from_secs(2));
        req.model = Some("grok-4".to_string());
        req.reasoning_effort = Some("low".to_string());
        let sink = silent_sink();
        grok.run(req, &sink).await.expect("grok sandbox run");
        let grok_cmd = grok_fake.last_command().expect("grok command");
        assert_eq!(grok_cmd.program, "grok");
        assert_eq!(
            grok_cmd.args,
            vec![
                "agent",
                "--always-approve",
                "--model",
                "grok-4",
                "--reasoning-effort",
                "low",
                "--sandbox",
                "all",
                "stdio"
            ]
        );
        assert!(grok_cmd.env.is_empty());

        let mut codex_cfg = default_config();
        codex_cfg.codex_home = Some("/tmp/codex-home".to_string());
        let codex_fake = FakeTransport::new(|req| {
            let mut lines = codex_initialize_and_thread(req);
            if method_of(req) == Some("turn/start") {
                lines.extend(codex_turn_events(req));
            }
            lines
        });
        let codex = CodexAppServerExecutor::new(codex_cfg, codex_fake.clone());
        let sink = silent_sink();
        codex
            .run(request_with_timeout(Duration::from_secs(2)), &sink)
            .await
            .expect("codex home run");
        let codex_cmd = codex_fake.last_command().expect("codex command");
        assert_eq!(codex_cmd.program, "codex");
        assert_eq!(codex_cmd.args, vec!["app-server", "--listen", "stdio://"]);
        assert!(
            codex_cmd
                .env
                .iter()
                .any(|(k, v)| k == "CODEX_HOME" && v == "/tmp/codex-home")
        );
        assert!(
            codex_cmd
                .env
                .iter()
                .any(|(k, v)| k == "HOME" && v == "/tmp")
        );
    }
}
