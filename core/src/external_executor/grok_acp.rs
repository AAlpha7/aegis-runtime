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

pub struct GrokAcpExecutor {
    pub config: ExecutorConfig,
    pub transport: Arc<dyn AgentTransport>,
}

impl GrokAcpExecutor {
    pub fn new(config: ExecutorConfig, transport: Arc<dyn AgentTransport>) -> Self {
        Self { config, transport }
    }

    fn command(&self, request: &ExecutorRequest) -> AgentCommand {
        let model = request.model.as_deref().unwrap_or(&self.config.grok_model);
        let effort = request
            .reasoning_effort
            .as_deref()
            .unwrap_or(&self.config.grok_effort);
        let mut args = vec![
            "agent".to_string(),
            "--always-approve".to_string(),
            "--model".to_string(),
            model.to_string(),
            "--reasoning-effort".to_string(),
            effort.to_string(),
        ];
        if let Some(sandbox) = &self.config.grok_sandbox {
            args.push("--sandbox".to_string());
            args.push(sandbox.clone());
        }
        args.push("stdio".to_string());
        AgentCommand {
            program: self.config.grok_bin.clone(),
            args,
            cwd: request.cwd.clone(),
            env: Vec::new(),
        }
    }

    async fn run_inner(
        &self,
        request: ExecutorRequest,
        on_update: UpdateSink<'_>,
    ) -> anyhow::Result<ExecutorReceipt> {
        let started = std::time::Instant::now();
        let deadline = Instant::now() + request.timeout;
        let command = self.command(&request);
        let io = self
            .transport
            .spawn(&command)
            .with_context(|| format!("spawn grok ACP agent '{}'", command.program))?;
        let mut rpc = LineRpc::from_io(io);

        let mut session_id = None;
        let mut model = None;
        let mut final_text = String::new();
        let mut tool_calls: Vec<ExecutorToolCall> = Vec::new();
        let mut usage = ExecutorUsage::default();
        let mut on_agent_request =
            |_method: &str, _params: &Value| json!({"outcome": {"outcome": "cancelled"}});

        let init = rpc
            .request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientInfo": {
                        "name": "aegis-runtime",
                        "title": "Aegis Runtime",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "clientCapabilities": {
                        "fs": {
                            "readTextFile": false,
                            "writeTextFile": false,
                        }
                    },
                }),
                deadline,
                &mut |method, params| {
                    apply_grok_notification(
                        method,
                        params,
                        &mut final_text,
                        &mut tool_calls,
                        on_update,
                    );
                },
                &mut on_agent_request,
            )
            .await;
        if let Err(err) = init {
            return finish_or_timeout(
                &mut rpc, err, started, session_id, model, final_text, tool_calls, usage,
            )
            .await;
        }

        let session = match rpc
            .request(
                "session/new",
                json!({
                    "cwd": cwd_string(&request.cwd),
                    "mcpServers": [],
                }),
                deadline,
                &mut |method, params| {
                    apply_grok_notification(
                        method,
                        params,
                        &mut final_text,
                        &mut tool_calls,
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
                    &mut rpc, err, started, session_id, model, final_text, tool_calls, usage,
                )
                .await;
            }
        };
        session_id = json_str(&session, "sessionId")
            .or_else(|| json_str(&session, "session_id"))
            .map(str::to_string);
        model = session
            .pointer("/models/currentModelId")
            .and_then(Value::as_str)
            .map(str::to_string);

        let prompt = match rpc
            .request(
                "session/prompt",
                json!({
                    "sessionId": session_id,
                    "prompt": [{
                        "type": "text",
                        "text": request.prompt,
                    }],
                }),
                deadline,
                &mut |method, params| {
                    apply_grok_notification(
                        method,
                        params,
                        &mut final_text,
                        &mut tool_calls,
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
                    &mut rpc, err, started, session_id, model, final_text, tool_calls, usage,
                )
                .await;
            }
        };

        if let Some(meta) = prompt.get("_meta") {
            if let Some(model_id) = json_str(meta, "modelId") {
                model = Some(model_id.to_string());
            }
            usage = ExecutorUsage {
                input_tokens: json_u64(meta, "inputTokens"),
                output_tokens: json_u64(meta, "outputTokens"),
                reasoning_tokens: json_u64(meta, "reasoningTokens"),
                total_tokens: json_u64(meta, "totalTokens"),
            };
        }

        let stop_reason = never_empty_stop_reason(json_str(&prompt, "stopReason"), "end_turn");
        Ok(ExecutorReceipt {
            kind: ExecutorKind::GrokAcp,
            session_id,
            model,
            stop_reason,
            final_text,
            tool_calls,
            usage,
            elapsed_ms: elapsed_ms(started),
            stderr_tail: rpc.stderr_tail(),
            error: None,
        })
    }
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
            kind: ExecutorKind::GrokAcp,
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

impl ExternalExecutor for GrokAcpExecutor {
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::GrokAcp
    }

    fn run<'a>(
        &'a self,
        request: ExecutorRequest,
        on_update: UpdateSink<'a>,
    ) -> ExecutorFuture<'a> {
        Box::pin(self.run_inner(request, on_update))
    }
}

fn apply_grok_notification(
    method: &str,
    params: &Value,
    final_text: &mut String,
    tool_calls: &mut Vec<ExecutorToolCall>,
    on_update: UpdateSink<'_>,
) {
    if method != "session/update" {
        return;
    }
    let Some(update) = params.get("update") else {
        return;
    };
    let Some(kind) = json_str(update, "sessionUpdate") else {
        return;
    };
    match kind {
        "agent_message_chunk" => {
            if let Some(text) = content_text(update) {
                final_text.push_str(&text);
                on_update(ExecutorUpdate::Message(text));
            }
        }
        "agent_thought_chunk" => {
            if let Some(text) = content_text(update) {
                on_update(ExecutorUpdate::Thought(text));
            }
        }
        "tool_call" => {
            let id = json_str(update, "toolCallId").unwrap_or("").to_string();
            let title = json_str(update, "title").unwrap_or("").to_string();
            let kind = json_str(update, "kind").unwrap_or("").to_string();
            let status = json_str(update, "status").unwrap_or("").to_string();
            tool_calls.push(ExecutorToolCall {
                id: id.clone(),
                title: title.clone(),
                kind,
                status: status.clone(),
            });
            on_update(ExecutorUpdate::ToolCall { id, title, status });
        }
        "tool_call_update" => {
            let Some(id) = json_str(update, "toolCallId") else {
                return;
            };
            let status = json_str(update, "status").unwrap_or("").to_string();
            let title = json_str(update, "title").map(str::to_string);
            if let Some(existing) = tool_calls.iter_mut().find(|call| call.id == id) {
                existing.status = status.clone();
                if let Some(title) = title.clone() {
                    existing.title = title;
                }
            }
            on_update(ExecutorUpdate::ToolCall {
                id: id.to_string(),
                title: title
                    .or_else(|| {
                        tool_calls
                            .iter()
                            .find(|call| call.id == id)
                            .map(|call| call.title.clone())
                    })
                    .unwrap_or_default(),
                status,
            });
        }
        "plan" => {
            if let Some(entries) = update.get("entries") {
                on_update(ExecutorUpdate::Plan(entries.to_string()));
            }
        }
        _ => {}
    }
}

fn content_text(update: &Value) -> Option<String> {
    if let Some(text) = update.pointer("/content/text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    json_str(update, "content").map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    fn cfg() -> ExecutorConfig {
        ExecutorConfig::from_env_map(&[])
    }

    fn req() -> ExecutorRequest {
        ExecutorRequest {
            prompt: "ping".to_string(),
            cwd: PathBuf::from("/work"),
            timeout: Duration::from_secs(1),
            model: None,
            reasoning_effort: None,
            web_search: false,
            required_connector: None,
        }
    }

    #[test]
    fn grok_agent_command_argument_order() {
        let exec = GrokAcpExecutor::new(cfg(), Arc::new(super::super::ProcessTransport));
        let cmd = exec.command(&req());
        assert_eq!(cmd.program, "grok");
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
        assert!(cmd.env.is_empty());
        assert_eq!(cmd.cwd, PathBuf::from("/work"));

        let mut sandbox_cfg = cfg();
        sandbox_cfg.grok_sandbox = Some("all".to_string());
        let exec = GrokAcpExecutor::new(sandbox_cfg, Arc::new(super::super::ProcessTransport));
        let mut request = req();
        request.model = Some("grok-4".to_string());
        request.reasoning_effort = Some("low".to_string());
        let cmd = exec.command(&request);
        assert_eq!(
            cmd.args,
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
    }
}
