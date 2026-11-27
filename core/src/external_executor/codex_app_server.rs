use std::sync::Arc;

use anyhow::Context;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::jsonrpc::LineRpc;
use super::transport::{AgentCommand, AgentTransport};
use super::{
    ExecutorConfig, ExecutorFuture, ExecutorKind, ExecutorReceipt, ExecutorRequest,
    ExecutorToolCall, ExecutorUpdate, ExecutorUsage, ExternalExecutor, UpdateSink, cwd_string,
    elapsed_ms, is_timeout_error, json_str, json_u64, never_empty_stop_reason,
};

const CODEX_BASE_INSTRUCTIONS: &str = "You are an execution agent working for Aegis Runtime. Do the task in the working directory and finish with a concise report of what you did and what you found.";

pub struct CodexAppServerExecutor {
    pub config: ExecutorConfig,
    pub transport: Arc<dyn AgentTransport>,
}

impl CodexAppServerExecutor {
    pub fn new(config: ExecutorConfig, transport: Arc<dyn AgentTransport>) -> Self {
        Self { config, transport }
    }

    fn command(&self, request: &ExecutorRequest) -> AgentCommand {
        let mut env = Vec::new();
        if let Some(codex_home) = &self.config.codex_home {
            env.push(("CODEX_HOME".to_string(), codex_home.clone()));
            if let Some(parent) = std::path::Path::new(codex_home).parent() {
                env.push(("HOME".to_string(), parent.display().to_string()));
            }
        }
        AgentCommand {
            program: self.config.codex_bin.clone(),
            args: vec![
                "app-server".to_string(),
                "--listen".to_string(),
                "stdio://".to_string(),
            ],
            cwd: request.cwd.clone(),
            env,
        }
    }

    async fn run_inner(
        &self,
        request: ExecutorRequest,
        on_update: UpdateSink<'_>,
    ) -> anyhow::Result<ExecutorReceipt> {
        let started = std::time::Instant::now();
        let deadline = Instant::now() + request.timeout;
        let model = request
            .model
            .as_deref()
            .unwrap_or(&self.config.codex_model)
            .to_string();
        let effort = request
            .reasoning_effort
            .as_deref()
            .unwrap_or(&self.config.codex_effort)
            .to_string();
        let cwd = cwd_string(&request.cwd);
        let command = self.command(&request);
        let io = self
            .transport
            .spawn(&command)
            .with_context(|| format!("spawn Codex app-server '{}'", command.program))?;
        let mut rpc = LineRpc::from_io(io);

        let mut session_id = None;
        let mut final_text = String::new();
        let mut tool_calls: Vec<ExecutorToolCall> = Vec::new();
        let mut usage = ExecutorUsage::default();
        let mut stop_reason = String::new();
        let mut turn_done = false;
        let mut error: Option<String> = None;
        let mut on_agent_request = |method: &str, _params: &Value| match method {
            "mcpServer/elicitation/request" => json!({"action":"cancel", "content":null}),
            "item/permissions/requestApproval" => json!({"permissions":{}, "scope":"turn"}),
            "tool/requestUserInput" | "item/tool/requestUserInput" => json!({"answers":{}}),
            _ => json!({"decision": "cancel"}),
        };

        if let Err(err) = rpc
            .request(
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "aegis-runtime",
                        "title": "Aegis Runtime",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "capabilities": {
                        "experimentalApi": true,
                        "requestAttestation": false,
                        "optOutNotificationMethods": [],
                    }
                }),
                deadline,
                &mut |method, params| {
                    apply_codex_notification(
                        method,
                        params,
                        &mut final_text,
                        &mut tool_calls,
                        &mut usage,
                        &mut stop_reason,
                        &mut turn_done,
                        &mut error,
                        on_update,
                    );
                },
                &mut on_agent_request,
            )
            .await
        {
            return finish_or_timeout(
                &mut rpc,
                err,
                started,
                session_id,
                Some(model),
                final_text,
                tool_calls,
                usage,
            )
            .await;
        }

        if let Err(err) = rpc.notify("initialized", None).await {
            return Err(rpc.wrap_err(err));
        }

        let account = rpc
            .request(
                "account/read",
                json!({"refreshToken":false}),
                deadline,
                &mut |_, _| {},
                &mut on_agent_request,
            )
            .await?;
        anyhow::ensure!(
            account.pointer("/account/type").and_then(Value::as_str) == Some("chatgpt"),
            "Codex requires ChatGPT subscription login; API-key execution is not an automatic fallback"
        );

        let mut config = json!({
            "web_search": if request.web_search { "live" } else { "cached" },
            "apps._default.destructive_enabled": false,
            "apps._default.enabled": request.required_connector.is_some(),
        });
        if request.web_search || request.required_connector.is_some() {
            config["features.shell_tool"] = json!(false);
            config["features.unified_exec"] = json!(false);
        }
        let mut input = vec![json!({"type":"text", "text":request.prompt, "text_elements":[]})];
        if let Some(required) = request.required_connector.as_deref() {
            let mut cursor = Value::Null;
            let mut matched = None;
            for _ in 0..10 {
                let apps = rpc
                    .request(
                        "app/list",
                        json!({
                            "threadId": null, "cursor": cursor, "limit": 50,
                            "forceRefetch": true,
                        }),
                        deadline,
                        &mut |_, _| {},
                        &mut on_agent_request,
                    )
                    .await?;
                if let Some(data) = apps.get("data").and_then(Value::as_array) {
                    matched = data
                        .iter()
                        .find(|app| {
                            let matches = ["id", "name"].iter().any(|field| {
                                app.get(field)
                                    .and_then(Value::as_str)
                                    .is_some_and(|s| s.eq_ignore_ascii_case(required))
                            });
                            matches && app["isAccessible"] == true && app["isEnabled"] == true
                        })
                        .cloned();
                }
                cursor = apps.get("nextCursor").cloned().unwrap_or(Value::Null);
                if matched.is_some() || cursor.is_null() {
                    break;
                }
            }
            let app = matched.context("Required connector is unavailable or disabled. Connect it in Codex before retrying; no task was executed.")?;
            let id = app
                .get("id")
                .and_then(Value::as_str)
                .context("Connector has no app id")?;
            config[format!("apps.{id}.destructive_enabled")] = json!(false);
            config[format!("apps.{id}.default_tools_approval_mode")] = json!("writes");
            config[format!("apps.{id}.tools")] = json!({});
            input.push(json!({"type":"mention", "name":app["name"], "path":format!("app://{id}")}));
        }

        let thread = match rpc
            .request(
                "thread/start",
                json!({
                    "model": model,
                    "modelProvider": "openai",
                    "cwd": cwd,
                    "approvalPolicy": "never",
                    "sandbox": if request.web_search || request.required_connector.is_some() { "read-only" } else { "workspace-write" },
                    "config": config,
                    "ephemeral": true,
                    "baseInstructions": CODEX_BASE_INSTRUCTIONS,
                    "developerInstructions": null,
                    "dynamicTools": [],
                    "environments": [],
                }),
                deadline,
                &mut |method, params| {
                    apply_codex_notification(
                        method,
                        params,
                        &mut final_text,
                        &mut tool_calls,
                        &mut usage,
                        &mut stop_reason,
                        &mut turn_done,
                        &mut error,
                        on_update,
                    );
                },
                &mut on_agent_request,
            )
            .await
        {
            Ok(value) => value,
            Err(err) => {
                return finish_or_timeout(
                    &mut rpc,
                    err,
                    started,
                    session_id,
                    Some(model),
                    final_text,
                    tool_calls,
                    usage,
                )
                .await;
            }
        };
        session_id = thread
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .map(str::to_string);

        if let Err(err) = rpc
            .request(
                "turn/start",
                json!({
                    "threadId": session_id,
                    "input": input,
                    "model": model,
                    "effort": effort,
                    "cwd": cwd,
                    "environments": [],
                }),
                deadline,
                &mut |method, params| {
                    apply_codex_notification(
                        method,
                        params,
                        &mut final_text,
                        &mut tool_calls,
                        &mut usage,
                        &mut stop_reason,
                        &mut turn_done,
                        &mut error,
                        on_update,
                    );
                },
                &mut on_agent_request,
            )
            .await
        {
            return finish_or_timeout(
                &mut rpc,
                err,
                started,
                session_id,
                Some(model),
                final_text,
                tool_calls,
                usage,
            )
            .await;
        }

        if !turn_done {
            let wait = rpc
                .wait_until(
                    deadline,
                    "turn/completed",
                    &mut |method, params| {
                        apply_codex_notification(
                            method,
                            params,
                            &mut final_text,
                            &mut tool_calls,
                            &mut usage,
                            &mut stop_reason,
                            &mut turn_done,
                            &mut error,
                            on_update,
                        );
                        method == "turn/completed"
                    },
                    &mut on_agent_request,
                )
                .await;
            if let Err(err) = wait {
                return finish_or_timeout(
                    &mut rpc,
                    err,
                    started,
                    session_id,
                    Some(model),
                    final_text,
                    tool_calls,
                    usage,
                )
                .await;
            }
        }

        Ok(ExecutorReceipt {
            kind: ExecutorKind::CodexAppServer,
            session_id,
            model: Some(model),
            stop_reason: never_empty_stop_reason(Some(&stop_reason), "end_turn"),
            final_text,
            tool_calls,
            usage,
            elapsed_ms: elapsed_ms(started),
            stderr_tail: rpc.stderr_tail(),
            error,
        })
    }
}

impl ExternalExecutor for CodexAppServerExecutor {
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::CodexAppServer
    }

    fn run<'a>(
        &'a self,
        request: ExecutorRequest,
        on_update: UpdateSink<'a>,
    ) -> ExecutorFuture<'a> {
        Box::pin(self.run_inner(request, on_update))
    }
}

fn apply_codex_notification(
    method: &str,
    params: &Value,
    final_text: &mut String,
    tool_calls: &mut Vec<ExecutorToolCall>,
    usage: &mut ExecutorUsage,
    stop_reason: &mut String,
    turn_done: &mut bool,
    error: &mut Option<String>,
    on_update: UpdateSink<'_>,
) {
    match method {
        "item/agentMessage/delta" => {
            if let Some(delta) = json_str(params, "delta") {
                final_text.push_str(delta);
                on_update(ExecutorUpdate::Message(delta.to_string()));
            }
        }
        "item/completed" => {
            let Some(item) = params.get("item") else {
                return;
            };
            let Some(kind) = json_str(item, "type") else {
                return;
            };
            // Commentary and the final answer arrive as separate messages.
            // Only the completed final answer is the structured result; joining
            // commentary to it makes valid search JSON impossible to parse.
            if kind == "agentMessage"
                && json_str(item, "phase") == Some("final_answer")
                && let Some(text) = json_str(item, "text")
            {
                *final_text = text.to_string();
            }
            if matches!(
                kind,
                "commandExecution" | "mcpToolCall" | "fileChange" | "webSearch"
            ) {
                let id = json_str(item, "id").unwrap_or("").to_string();
                let title = tool_title(kind, item);
                let status = json_str(item, "status").unwrap_or("completed").to_string();
                tool_calls.push(ExecutorToolCall {
                    id: id.clone(),
                    title: title.clone(),
                    kind: kind.to_string(),
                    status: status.clone(),
                });
                on_update(ExecutorUpdate::ToolCall { id, title, status });
            }
        }
        "thread/tokenUsage/updated" => {
            if let Some(last) = params.pointer("/tokenUsage/last") {
                *usage = ExecutorUsage {
                    input_tokens: json_u64(last, "inputTokens"),
                    output_tokens: json_u64(last, "outputTokens"),
                    reasoning_tokens: json_u64(last, "reasoningOutputTokens"),
                    total_tokens: json_u64(last, "totalTokens"),
                };
            }
        }
        "error" => {
            if let Some(message) = params.pointer("/error/message").and_then(Value::as_str) {
                *error = Some(message.to_string());
            }
        }
        "turn/completed" => {
            *turn_done = true;
            if let Some(status) = params.pointer("/turn/status").and_then(Value::as_str) {
                *stop_reason = status.to_string();
            }
            if let Some(message) = params
                .pointer("/turn/error/message")
                .and_then(Value::as_str)
            {
                *error = Some(message.to_string());
            }
            if final_text.trim().is_empty()
                && let Some(text) = last_agent_message_text(params)
            {
                *final_text = text;
            }
        }
        _ => {}
    }
}

fn tool_title(kind: &str, item: &Value) -> String {
    match kind {
        "commandExecution" => json_str(item, "command")
            .map(str::to_string)
            .or_else(|| {
                item.get("command")
                    .and_then(|command| json_str(command, "command"))
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "command".to_string()),
        "mcpToolCall" => json_str(item, "tool")
            .or_else(|| json_str(item, "name"))
            .unwrap_or("mcp")
            .to_string(),
        "fileChange" => "file change".to_string(),
        other => other.to_string(),
    }
}

fn last_agent_message_text(params: &Value) -> Option<String> {
    let items = params.pointer("/turn/items")?.as_array()?;
    items.iter().rev().find_map(|item| {
        if json_str(item, "type") != Some("agentMessage") {
            return None;
        }
        json_str(item, "text")
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

async fn finish_or_timeout(
    rpc: &mut LineRpc,
    err: anyhow::Error,
    started: std::time::Instant,
    session_id: Option<String>,
    model: Option<String>,
    final_text: String,
    tool_calls: Vec<ExecutorToolCall>,
    usage: ExecutorUsage,
) -> anyhow::Result<ExecutorReceipt> {
    if is_timeout_error(&err) {
        rpc.abort().await;
        return Ok(ExecutorReceipt {
            kind: ExecutorKind::CodexAppServer,
            session_id,
            model,
            stop_reason: "timeout".to_string(),
            final_text,
            tool_calls,
            usage,
            elapsed_ms: elapsed_ms(started),
            stderr_tail: rpc.stderr_tail(),
            error: None,
        });
    }
    Err(rpc.wrap_err(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn search_final_json_replaces_streamed_commentary() {
        let mut text = "I will search the web. ".to_string();
        let answer = r#"{"summary":"verified","results":[{"url":"https://example.test/"}]}"#;
        text.push_str(answer);
        apply_codex_notification(
            "item/completed",
            &serde_json::json!({"item":{"type":"agentMessage","phase":"final_answer","text":answer}}),
            &mut text,
            &mut Vec::new(),
            &mut ExecutorUsage::default(),
            &mut String::new(),
            &mut false,
            &mut None,
            &|_| {},
        );
        assert_eq!(text, answer);
        assert!(crate::web_search::parse_search_results(&text).is_some());
    }

    #[test]
    fn codex_agent_command_argument_order() {
        let exec = CodexAppServerExecutor::new(
            ExecutorConfig::from_env_map(&[]),
            Arc::new(super::super::ProcessTransport),
        );
        let request = ExecutorRequest {
            prompt: "ping".to_string(),
            cwd: PathBuf::from("/work"),
            timeout: Duration::from_secs(1),
            model: None,
            reasoning_effort: None,
            web_search: false,
            required_connector: None,
        };
        let cmd = exec.command(&request);
        assert_eq!(cmd.program, "codex");
        assert_eq!(cmd.args, vec!["app-server", "--listen", "stdio://"]);
        assert!(cmd.env.is_empty());

        let mut cfg = ExecutorConfig::from_env_map(&[]);
        cfg.codex_home = Some("/var/codex-home".to_string());
        let exec = CodexAppServerExecutor::new(cfg, Arc::new(super::super::ProcessTransport));
        let cmd = exec.command(&request);
        assert_eq!(
            cmd.env,
            vec![
                ("CODEX_HOME".to_string(), "/var/codex-home".to_string()),
                ("HOME".to_string(), "/var".to_string()),
            ]
        );
    }
}
