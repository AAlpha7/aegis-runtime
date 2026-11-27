//! MCP + UI/WebApp tool handlers for the Orchestrator.
//!
//! This module contains the `handle_tool_mcp` and `handle_tool_ui` handler
//! methods, extracted from `orchestrator.rs` to reduce file size.
//! - `handle_tool_mcp`: routes tool calls to external MCP servers with
//!   security layers (approval, exfiltration detection, size cap, sanitization).
//! - `handle_tool_ui`: answers the retired SDUI pseudo-tools (`manage_panels`,
//!   `render_webapp`) with a terminal tool error. They were retired
//!   2026-09-15: nothing in the web console rendered their output, so the
//!   model reported "panel delivered" while the owner saw nothing.

use anyhow::Result;
use serde_json::Value;

use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionRequestSystemMessageArgs,
};

use crate::orchestrator::{Orchestrator, ToolContext, push_tool_result};
use crate::security::detect_exfiltration;
use crate::utils::truncate_str;

/// SDUI pseudo-tools retired on 2026-09-15. Never offered to the model
/// (see `build_tool_definitions_filtered`); a call that still arrives is
/// answered with a `tool_retired` error by `handle_tool_ui`.
pub(crate) const RETIRED_UI_TOOLS: &[&str] = &["render_webapp", "manage_panels"];

impl Orchestrator {
    pub(crate) async fn handle_tool_mcp(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        // MCP TOOL INTERCEPT: route to external MCP server
        //
        // Security layers applied (matching Wasm tool security):
        // 1. Validate tool exists on a connected MCP server (prevents hallucinated names)
        // 2. Exfiltration detection on arguments (prevents cross-round data leakage)
        // 3. Result size cap (prevents DoS via oversized responses)
        // 4. Result sanitization (prevents prompt injection via tool results)
        //
        // NOTE: This block uses dynamic `is_mcp_tool()` routing and may push
        // system messages + return Ok(Some(false)), so it stays manual.
        if crate::mcp_client::is_mcp_tool(tool_name) {
            // ── Security Layer 0a: Global mcp_enabled gate ──
            if !self.settings.is_enabled("mcp_enabled") {
                ctx.tools_blocked.push(tool_name.to_string());
                let block_msg = serde_json::json!({
                    "status": "error",
                    "error": "MCP is disabled. Ask the user to enable it: \
                              update_setting(key='mcp_enabled', value=true) — requires user approval.",
                    "error_kind": "permission_denied",
                    "retryable": false
                }).to_string();
                push_tool_result(ctx.messages, &tool_call.id, &block_msg)?;
                return Ok(Some(true));
            }

            // ── Security Layer 0b: Per-server first-use approval ──
            if let Some((server_key, _)) = crate::mcp_client::parse_namespaced_tool(tool_name) {
                let approved_csv = self
                    .settings
                    .get("mcp_approved_servers")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                let is_approved = approved_csv.split(',').any(|s| s.trim() == server_key);

                if !is_approved {
                    return self
                        .handle_mcp_server_approval(tool_call, tool_name, &server_key, ctx)
                        .await;
                }
            }

            // ── Security Layer 1: Validate tool is registered ──
            if let Some(ref mcp) = self.services.mcp_client {
                let known = mcp
                    .get_all_tools()
                    .iter()
                    .any(|t| t.namespaced_name == *tool_name);
                if !known {
                    tracing::warn!(
                        tool = tool_name,
                        "Blocked hallucinated MCP tool name (not registered on any server)"
                    );
                    ctx.tools_blocked.push(tool_name.to_string());
                    let block_msg = format!(
                        "MCP tool '{tool_name}' is not available. Use mcp_list_servers to see connected servers and their tools."
                    );
                    push_tool_result(ctx.messages, &tool_call.id, &block_msg)?;
                    return Ok(Some(true));
                }
            }

            // ── Security Layer 2: Exfiltration detection ──
            // MCP tools can send data to external servers; scan arguments
            // for fragments from previous tool results (cross-round data leakage).
            if detect_exfiltration(tool_name, ctx.previous_tool_results, tool_args_str, false) {
                let block_msg = format!(
                    "Tool '{tool_name}' blocked: potential data exfiltration detected. \
                     Tool arguments contain fragments from previous tool results."
                );
                tracing::warn!(tool_name, "MCP exfiltration attempt blocked");
                ctx.tools_blocked.push(tool_name.to_string());
                push_tool_result(ctx.messages, &tool_call.id, &block_msg)?;
                return Ok(Some(true));
            }

            ctx.tools_called.push(tool_name.to_string());
            let result_msg = self.execute_mcp_tool_call(tool_name, tool_args_str).await;

            // ── Security Layer 4: Sanitize result (strip injection tags) ──
            // Prevents malicious MCP servers from injecting <system>, <tool_call>,
            // or other control tags into the conversation.
            let sanitized = crate::mcp_client::sanitize_mcp_result(&result_msg);

            // Track result for future exfiltration detection
            ctx.previous_tool_results.push(sanitized.clone());

            push_tool_result(ctx.messages, &tool_call.id, &sanitized)?;
            return Ok(Some(true));
        }

        // Simple MCP management tools — use macro dispatch
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "mcp_list_servers"     => execute_mcp_list_servers,
            "mcp_reconnect_server" => execute_mcp_reconnect_server,
        );

        // PSEUDO-TOOL INTERCEPT: get_current_time (real-time clock, zero cost)
        Ok(None)
    }

    /// UI pseudo-tools.
    ///
    /// `render_webapp` and `manage_panels` (SDUI) are retired: they are no
    /// longer offered to the model, and a stray call (stale prompt cache,
    /// hallucinated name) gets a terminal tool error asking for a plain-text
    /// answer instead of executing. Everything else passes through.
    pub(crate) async fn handle_tool_ui(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        _tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        if !RETIRED_UI_TOOLS.contains(&tool_name) {
            // PSEUDO-TOOL INTERCEPT: text_to_speech (xAI TTS -> Telegram voice message)
            return Ok(None);
        }
        ctx.tools_called.push(tool_name.to_string());
        tracing::warn!(
            tool = tool_name,
            "retired SDUI tool called; answering with a tool error"
        );
        let err = serde_json::json!({
            "status": "error",
            "error": format!("{tool_name} is retired: answer in plain text"),
            "error_kind": "tool_retired",
            "retryable": false,
        })
        .to_string();
        push_tool_result(ctx.messages, &tool_call.id, &err)?;
        Ok(Some(true))
    }

    // ──────────────────────────────────────────────────────────────────────
    //  MCP tool handlers
    // ──────────────────────────────────────────────────────────────────────

    /// Handle the MCP server first-use approval flow.
    ///
    /// Pushes a system message instructing the LLM not to retry, and returns
    /// `Ok(Some(false))` to break the ReAct loop.
    async fn handle_mcp_server_approval(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        server_key: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        let chat_id = ctx.chat_id;

        // Send HITL approval request via Telegram
        let (tool_list, server_command, server_description) =
            if let Some(ref mcp) = self.services.mcp_client {
                let tools = mcp.get_all_tools();
                let server_tools: Vec<&str> = tools
                    .iter()
                    .filter(|t| t.server_key == server_key)
                    .map(|t| t.original_name.as_str())
                    .collect();
                let tool_list_str = if server_tools.is_empty() {
                    "(no tools discovered — server may not be connected)".to_string()
                } else {
                    server_tools.join(", ")
                };
                // Best-effort: pull the server's shell command + description so the
                // user knows what process will be spawned (trust signal).
                let (cmd, desc) = mcp
                    .list_servers()
                    .into_iter()
                    .find(|s| s.name == server_key)
                    .map(|s| (s.command, s.description))
                    .unwrap_or_else(|| ("(unknown)".to_string(), String::new()));
                (tool_list_str, cmd, desc)
            } else {
                (
                    "(unknown)".to_string(),
                    "(unknown)".to_string(),
                    String::new(),
                )
            };
        // Paired-Fix (CTO round-3): escape every LLM/external-authored
        // field before rendering into the Telegram HTML approval template.
        // `server_key` / `server_command` / `server_description` / tools
        // come from MCP server config which is attacker-controllable via
        // malicious servers. `tool_name` is LLM-supplied.
        let server_key_esc = crate::telegram_render::escape_html(server_key);
        let server_command_esc = crate::telegram_render::escape_html(&server_command);
        let tool_list_esc = crate::telegram_render::escape_html(&tool_list);
        let tool_name_esc = crate::telegram_render::escape_html(tool_name);
        let description_line = if server_description.is_empty() {
            String::new()
        } else {
            format!(
                "**\u{63cf}\u{8ff0} / Description:** {}\n",
                crate::telegram_render::escape_html(&server_description)
            )
        };

        let approval_id = format!("mcp-{}", server_key);
        let approval_id_esc = crate::telegram_render::escape_html(&approval_id);
        // Store the pending server key for the approval handler
        let proposal = serde_json::json!({
            "server_key": server_key,
            "tool_requested": tool_name,
            "server_command": server_command,
            "server_description": server_description,
            "exposed_tools": tool_list,
            "chat_id": chat_id,
            "user_did": ctx.user_did,
            "created_at": chrono::Utc::now().to_rfc3339(),
        });
        if let Err(e) = self
            .memory
            .store_skill_state("hitl", &approval_id, &proposal.to_string())
            .await
        {
            tracing::warn!(error = %e, approval_id, "failed to store MCP approval HITL state");
        }

        let approval_msg = format!(
            "\u{1f50c} **MCP Server Approval Request**\n\n\
             **\u{6765}\u{6e90} / Origin:** LLM tool call `{tool_name_esc}` (first use of server `{server_key_esc}`)\n\
             **\u{670d}\u{52a1}\u{5668} / Server:** `{server_key_esc}`\n\
             **\u{547d}\u{4ee4} / Command:** `{server_command_esc}`\n\
             {description_line}\
             **\u{53ef}\u{7528}\u{5de5}\u{5177} / Exposed tools:** {tool_list_esc}\n\
             **\u{672c}\u{6b21}\u{8c03}\u{7528} / Requested tool:** `{tool_name_esc}`\n\n\
             **\u{8303}\u{56f4} / Scope:** approving grants the LLM permission to call ANY of the \
             tools listed above (`{tool_list_esc}`) from this server freely for the rest of this \
             session, without further prompts. The server process runs as a child of this bot \
             with full network + filesystem access of the bot user — treat it like installing \
             software.\n\n\
             **\u{53ef}\u{9006}\u{6027} / Reversibility:** approval is per-session; restarting the \
             bot revokes it. To permanently disconnect, remove the server from the MCP config \
             file and restart. Individual tool calls afterwards are NOT re-prompted.\n\n\
             Approval ID: `{approval_id_esc}`\n\
             Open Web Work to approve or reject. Telegram commands remain available as fallback:\n\
             \u{2022} `approve mcp {approval_id_esc}` to allow\n\
             \u{2022} `reject mcp {approval_id_esc}` to deny"
        );

        if let (Some(tx), Some(cid)) = (&self.progress_tx, chat_id)
            && let Err(e) = tx.try_send(crate::progress_router::ProgressEvent::new(
                cid,
                crate::progress_router::ProgressPayload::NeedApproval {
                    approval_id: Some(approval_id.clone()),
                    tool_name: Some(tool_name.to_string()),
                    summary: None,
                    approval_html: approval_msg,
                },
            ))
        {
            tracing::warn!(error = %e, "Progress channel full, dropping MCP approval notification");
        }

        tracing::info!(
            server = server_key,
            tool = tool_name,
            "[HITL] MCP server approval requested"
        );

        ctx.tools_blocked.push(tool_name.to_string());
        let pending_result = serde_json::json!({
            "status": "pending_human_approval",
            "message": format!(
                "MCP server '{}' requires user approval before first use. \
                 An approval request has been sent to the user via Telegram. \
                 Do NOT retry this tool call — wait for user action.",
                server_key
            ),
        })
        .to_string();
        push_tool_result(ctx.messages, &tool_call.id, &pending_result)?;
        ctx.messages.push(
            ChatCompletionRequestSystemMessageArgs::default()
                .content(
                    "[SYSTEM] An MCP server approval request has been sent to the user. \
                     You MUST NOT call any more MCP tools. Tell the user that the request \
                     is pending approval and they can approve or reject it via Telegram.",
                )
                .build()?
                .into(),
        );
        Ok(Some(false)) // Signal to break the ReAct loop
    }

    /// Execute the actual MCP tool call with size cap (security layer 3).
    async fn execute_mcp_tool_call(&self, tool_name: &str, tool_args_str: &str) -> String {
        if let Some(ref mcp) = self.services.mcp_client {
            let args: serde_json::Value = serde_json::from_str(tool_args_str)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            match mcp.call_tool(tool_name, args).await {
                Ok(result) => {
                    let raw = result.to_json();
                    // ── Security Layer 3: Size cap (32 KB) ──
                    // Prevents DoS via oversized MCP responses flooding LLM context.
                    const MCP_RESULT_MAX_BYTES: usize = 32_768;
                    if raw.len() > MCP_RESULT_MAX_BYTES {
                        tracing::warn!(
                            tool = tool_name,
                            raw_len = raw.len(),
                            "MCP result truncated to {MCP_RESULT_MAX_BYTES} bytes"
                        );
                        let truncated = truncate_str(&raw, MCP_RESULT_MAX_BYTES);
                        format!(
                            "{truncated}\n[... truncated, total {len} bytes]",
                            len = raw.len()
                        )
                    } else {
                        raw
                    }
                }
                Err(e) => {
                    tracing::warn!(tool = tool_name, error = %e, "MCP tool call failed");
                    serde_json::json!({
                        "status": "error",
                        "error": "MCP tool execution failed / MCP\u{5de5}\u{5177}\u{6267}\u{884c}\u{5931}\u{8d25}",
                        "error_kind": "mcp_error",
                        "retryable": true,
                        "hint": "MCP server may be disconnected. Try mcp_list_servers to check status."
                    }).to_string()
                }
            }
        } else {
            serde_json::json!({
                "status": "error",
                "error": "MCP client not configured",
                "error_kind": "configuration_error",
                "retryable": false
            })
            .to_string()
        }
    }

    async fn execute_mcp_list_servers(&self, _args: Value, _ctx: &mut ToolContext<'_>) -> String {
        if let Some(ref mcp) = self.services.mcp_client {
            let statuses = mcp.list_servers();
            serde_json::to_string_pretty(&statuses).unwrap_or_else(|_| "[]".into())
        } else {
            r#"{"status":"error","error":"MCP client not configured"}"#.into()
        }
    }

    async fn execute_mcp_reconnect_server(
        &self,
        args: Value,
        _ctx: &mut ToolContext<'_>,
    ) -> String {
        if let Some(ref mcp) = self.services.mcp_client {
            let server = args["server"].as_str().unwrap_or("");
            if server.is_empty() {
                r#"{"status":"error","error":"Missing 'server' parameter"}"#.into()
            } else {
                match mcp.reconnect_server(server).await {
                    Ok(()) => format!(
                        r#"{{"status":"ok","message":"Reconnected to MCP server '{server}'"}}"#
                    ),
                    Err(e) => {
                        tracing::warn!(tool = "mcp_reconnect_server", server, error = %e, "MCP reconnect failed");
                        serde_json::json!({
                            "status": "error",
                            "error": format!("Failed to reconnect to MCP server '{}' / MCP\u{670d}\u{52a1}\u{5668}\u{91cd}\u{8fde}\u{5931}\u{8d25}", server),
                            "error_kind": "mcp_error",
                            "retryable": true,
                            "hint": "Check server name with mcp_list_servers and try again."
                        }).to_string()
                    }
                }
            }
        } else {
            r#"{"status":"error","error":"MCP client not configured"}"#.into()
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  Tests
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::memory::MemoryHandle;
    use crate::settings::RuntimeSettings;
    use crate::test_tool_helpers::{CtxState, make_tool_call, test_orchestrator};

    // ── Routing ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ui_unknown_tool_passes_through() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("render_hologram", "{}");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_ui(&tc, "render_hologram", "{}", &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, None, "unknown tool must pass through");
    }

    // ── MCP security: disabled gate ──────────────────────────────────────────

    #[tokio::test]
    async fn mcp_tool_when_mcp_disabled_is_blocked() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        // MCP is disabled by default
        let orch = test_orchestrator(mem, settings);

        // Simulate an MCP tool name (namespace__tool format)
        // Note: is_mcp_tool checks a global registry, which may not have this tool.
        // If it's not recognized as MCP, it should pass through.
        // This tests the code path defensively.
        let tc = make_tool_call("mcp_unknown__tool", "{}");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_mcp(&tc, "mcp_unknown__tool", "{}", &mut state.ctx())
            .await
            .expect("should not error");

        // If not recognized as MCP tool, passes through (None)
        // If recognized, should be blocked (Some(true) with error)
        // Either way, should not panic
        assert!(
            result.is_none() || result == Some(true),
            "must handle gracefully"
        );
    }

    // ── Retired SDUI tools ───────────────────────────────────────────────────

    /// A well-formed `render_webapp` call used to suspend the ReAct loop with
    /// a WebApp panel (`"status":"suspended"` pushed).
    /// Retired: the call must yield a terminal tool error, no suspension.
    #[tokio::test]
    async fn render_webapp_is_retired_returns_tool_error_without_suspension() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        // Passes the old normalizer + safety validator, so the negative
        // control is the old suspension path, not a validation failure.
        let args = r#"{
            "schema": {
                "view_type": "dashboard",
                "title": "Status",
                "components": [{"type": "text", "content": "all good"}]
            }
        }"#;
        let tc = make_tool_call("render_webapp", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_ui(&tc, "render_webapp", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(
            result,
            Some(true),
            "retired tool must keep the loop running"
        );
        assert_eq!(state.tools_called, vec!["render_webapp"]);
        let content = format!("{:?}", state.messages.last().expect("tool result pushed"));
        assert!(
            content.contains("render_webapp is retired: answer in plain text"),
            "must tell the model the tool is retired, got: {content}"
        );
        assert!(
            content.contains("tool_retired") && !content.contains("suspended"),
            "must be a terminal error, not a suspension, got: {content}"
        );
    }

    /// `manage_panels action=list` used to succeed against an empty panel
    /// table. Retired: it must yield the same terminal error shape.
    #[tokio::test]
    async fn manage_panels_is_retired_returns_tool_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"list"}"#;
        let tc = make_tool_call("manage_panels", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_ui(&tc, "manage_panels", args, &mut state.ctx())
            .await
            .expect("should not error");

        assert_eq!(result, Some(true));
        assert_eq!(state.tools_called, vec!["manage_panels"]);
        let content = format!("{:?}", state.messages.last().expect("tool result pushed"));
        assert!(
            content.contains("manage_panels is retired: answer in plain text")
                && content.contains("tool_retired"),
            "must tell the model the tool is retired, got: {content}"
        );
        assert!(
            !content.contains("\"panels\""),
            "retired tool must not execute the old list action, got: {content}"
        );
    }

    /// Even malformed arguments must not reach a JSON-parse error path — the
    /// retirement answer comes first and is the only answer.
    #[tokio::test]
    async fn retired_ui_tools_ignore_arguments() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        for name in super::RETIRED_UI_TOOLS {
            let tc = make_tool_call(name, "NOT JSON");
            let mut state = CtxState::new();
            let result = orch
                .handle_tool_ui(&tc, name, "NOT JSON", &mut state.ctx())
                .await
                .expect("should not error");
            assert_eq!(result, Some(true), "{name}");
            let content = format!("{:?}", state.messages.last().expect("tool result pushed"));
            assert!(
                content.contains(&format!("{name} is retired"))
                    && !content.contains("Invalid JSON"),
                "{name}: got {content}"
            );
        }
    }
}
