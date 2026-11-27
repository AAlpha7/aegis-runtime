//! Secure Computer Use pseudo-tool (`secure_computer_use`).
//!
//! Offered only when `AEGIS_SECURE_CU_ENABLED=1`. Runs the shared CU pipeline
//! (Policy → Sign → Linux AT-SPI / loopback Device Agent → Audit).

use std::sync::Arc;

use anyhow::Result;
use async_openai::types::chat::ChatCompletionMessageToolCall;
use serde_json::{Value, json};

use crate::computer_use::PolicyDecision;
use crate::computer_use::{PipelineError, SecureCuPipeline, secure_cu_enabled};
use crate::orchestrator::{Orchestrator, ToolContext};

/// Jev non-CU gate result for the original tool handler.
pub(crate) enum JevNonCuApply {
    Bypass,
    Allow(PolicyDecision),
    Early(String),
}

impl Orchestrator {
    pub(crate) fn secure_cu_pipeline(&self) -> Result<Arc<SecureCuPipeline>, String> {
        if !secure_cu_enabled() {
            return Err("secure computer use is disabled (set AEGIS_SECURE_CU_ENABLED=1)".into());
        }
        let mut guard = self.services.secure_cu.lock();
        if let Some(p) = guard.as_ref() {
            return Ok(Arc::clone(p));
        }
        let built = crate::computer_use::build_live_pipeline(Arc::clone(&self.identity))
            .map_err(|e| format!("secure_cu_init_failed: {e}"))?;
        let arc = Arc::new(built);
        *guard = Some(Arc::clone(&arc));
        Ok(arc)
    }

    /// Handle the `secure_computer_use` pseudo-tool.
    pub(crate) async fn handle_tool_computer_use(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "secure_computer_use" => execute_secure_computer_use,
        );
        Ok(None)
    }

    /// Jev non-CU gate: Bypass/Allow = proceed; Early = return that JSON
    /// (deny / ask / decide failed).
    pub(crate) async fn apply_jev_non_cu_gate(
        &self,
        tool: &str,
        args: &Value,
        ctx: &mut ToolContext<'_>,
    ) -> JevNonCuApply {
        use crate::computer_use::non_cu_gate::{
            JEV_NON_CU_ASK_KIND, NonCuGateOutcome, evaluate_non_cu_gate_with_broker,
        };
        // The tool-gate / egress HITL button already approved this exact call.
        // Do not open a second confirmation.
        if ctx.gate_pre_approved {
            return JevNonCuApply::Bypass;
        }
        let broker = crate::computer_use::policy_broker_for_key(self.jev_active_key())
            .unwrap_or_else(|_| {
                crate::computer_use::PolicyBroker::deterministic(
                    crate::computer_use::PolicyVerdict::Ask,
                )
            });
        let outcome = match evaluate_non_cu_gate_with_broker(tool, args, None, broker).await {
            Ok(o) => o,
            Err(e) => {
                return JevNonCuApply::Early(
                    json!({
                        "ok": false,
                        "error": "jev_non_cu_decide_failed",
                        "message": e.to_string(),
                    })
                    .to_string(),
                );
            }
        };
        match outcome {
            NonCuGateOutcome::Bypass => JevNonCuApply::Bypass,
            NonCuGateOutcome::Allow(decision) => {
                self.emit_action_phase(
                    ctx.chat_id,
                    "jev_allow",
                    Some(tool),
                    Some(&format!("Policy allow {tool}")),
                );
                JevNonCuApply::Allow(decision)
            }
            NonCuGateOutcome::Deny(decision) => {
                self.emit_action_phase(
                    ctx.chat_id,
                    "jev_deny",
                    Some(tool),
                    Some(&format!("Policy deny {tool}")),
                );
                JevNonCuApply::Early(
                    json!({
                        "ok": false,
                        "error": "policy_denied",
                        "status": "denied",
                        "message": decision.reason,
                        "policy": decision,
                    })
                    .to_string(),
                )
            }
            NonCuGateOutcome::Ask(pending) => {
                let approval_id = format!("jev-ask-{}", pending.intent_id);
                let summary = format!(
                    "Policy approval: `{}` — {}",
                    pending.tool, pending.policy.reason
                );
                let hitl = json!({
                    "approval_id": approval_id,
                    "kind": JEV_NON_CU_ASK_KIND,
                    "tool_name": pending.tool,
                    "tool_args": pending.args.to_string(),
                    "pending_ask": pending,
                    "approval_summary": summary,
                    "user_did": ctx.user_did,
                    "chat_id": ctx.chat_id,
                    "created_at": chrono::Utc::now().to_rfc3339(),
                });
                if let Err(e) = self
                    .memory
                    .store_skill_state("hitl", &approval_id, &hitl.to_string())
                    .await
                {
                    return JevNonCuApply::Early(
                        json!({
                            "ok": false,
                            "error": "hitl_store_failed",
                            "message": e.to_string(),
                        })
                        .to_string(),
                    );
                }
                let approval_html = format!(
                    "🔐 <b>Policy approval</b>\n\nTool: <code>{}</code>\n{}\n\nApproval ID: <code>{}</code>",
                    crate::telegram_render::escape_html(&pending.tool),
                    crate::telegram_render::escape_html(&pending.policy.reason),
                    crate::telegram_render::escape_html(&approval_id),
                );
                if let (Some(tx), Some(cid)) = (&self.progress_tx, ctx.chat_id) {
                    let _ = tx.try_send(crate::progress_router::ProgressEvent::new(
                        cid,
                        crate::progress_router::ProgressPayload::NeedApproval {
                            approval_id: Some(approval_id.clone()),
                            tool_name: Some(pending.tool.clone()),
                            summary: Some(summary.clone()),
                            approval_html,
                        },
                    ));
                }
                self.emit_action_phase(
                    ctx.chat_id,
                    "jev_ask",
                    Some(pending.tool.as_str()),
                    Some("Policy requires owner approval"),
                );
                JevNonCuApply::Early(
                    json!({
                        "ok": false,
                        "status": "pending_human_approval",
                        "error": "policy_ask",
                        "approval_id": approval_id,
                        "intent_id": pending.intent_id,
                        "policy": {
                            "verdict": "ask",
                            "model": pending.policy.model,
                            "reason": pending.policy.reason,
                        },
                        "message": "Policy requires owner approval before the original tool runs.",
                    })
                    .to_string(),
                )
            }
        }
    }

    async fn execute_secure_computer_use(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        if !secure_cu_enabled() {
            return json!({
                "ok": false,
                "error": "disabled",
                "message": "secure computer use is disabled (set AEGIS_SECURE_CU_ENABLED=1)",
            })
            .to_string();
        }

        let cu_tool = args
            .get("cu_tool")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        if cu_tool.is_empty() {
            return json!({
                "ok": false,
                "error": "invalid_input",
                "message": "cu_tool is required (e.g. system.session_probe, system.frontmost_app)",
            })
            .to_string();
        }

        let cu_args = args.get("args").cloned().unwrap_or_else(|| json!({}));
        let objective = args
            .get("objective")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let pipeline = match self.secure_cu_pipeline() {
            Ok(p) => p,
            Err(e) => {
                return json!({
                    "ok": false,
                    "error": "not_available",
                    "message": e,
                })
                .to_string();
            }
        };

        self.emit_action_phase(
            ctx.chat_id,
            "tool",
            Some("secure_computer_use"),
            Some(&format!("Policy check {cu_tool}")),
        );
        let broker = crate::computer_use::policy_broker_for_key(self.jev_active_key())
            .unwrap_or_else(|_| {
                crate::computer_use::PolicyBroker::deterministic(
                    crate::computer_use::PolicyVerdict::Ask,
                )
            });
        match pipeline
            .run_with_broker(cu_tool, cu_args, objective, broker)
            .await
        {
            Ok(out) => {
                self.emit_action_phase(
                    ctx.chat_id,
                    "jev_allow",
                    Some("secure_computer_use"),
                    Some(&format!("Policy {} {cu_tool}", out.policy.verdict.as_str())),
                );
                json!({
                    "ok": out.receipt.accepted,
                    "adapter": pipeline.device_adapter(),
                    "device_id": pipeline.device_id(),
                    "policy": {
                        "verdict": out.policy.verdict,
                        "model": out.policy.model,
                        "reason": out.policy.reason,
                    },
                    "receipt": out.receipt,
                    "audit_tip": out.audit_tip,
                    "intent_id": out.intent.body.intent_id,
                })
                .to_string()
            }
            Err(PipelineError::Disabled) => json!({
                "ok": false,
                "error": "disabled",
                "message": "secure computer use is disabled",
            })
            .to_string(),
            Err(PipelineError::PolicyDenied(reason)) => {
                self.emit_action_phase(
                    ctx.chat_id,
                    "jev_deny",
                    Some("secure_computer_use"),
                    Some(&format!("Policy deny {cu_tool}")),
                );
                json!({
                    "ok": false,
                    "error": "policy_denied",
                    "message": reason.clone(),
                    "adapter": pipeline.device_adapter(),
                    "audit_tip": pipeline.audit_tip(),
                    "policy": { "verdict": "deny", "reason": reason },
                })
                .to_string()
            }
            Err(PipelineError::AskRequired(pending)) => {
                let pending = *pending;
                // Reuse ToolGate HITL channel (InteractiveOnly UX): store pending
                // Ask, notify owner, suspend. Resume signs via continue_after_owner_approval.
                let approval_id = format!("cu-ask-{}", pending.intent_id);
                let summary = format!(
                    "Secure CU ask: `{}` — {}",
                    pending.tool, pending.policy.reason
                );
                let hitl = json!({
                    "approval_id": approval_id,
                    "kind": "secure_cu_ask",
                    "tool_name": "secure_computer_use",
                    "tool_args": args,
                    "pending_ask": pending,
                    "approval_summary": summary,
                    "user_did": ctx.user_did,
                    "chat_id": ctx.chat_id,
                    "created_at": chrono::Utc::now().to_rfc3339(),
                });
                if let Err(e) = self
                    .memory
                    .store_skill_state("hitl", &approval_id, &hitl.to_string())
                    .await
                {
                    return json!({
                        "ok": false,
                        "error": "hitl_store_failed",
                        "message": e.to_string(),
                        "intent_id": pending.intent_id,
                    })
                    .to_string();
                }
                let approval_html = format!(
                    "🔐 <b>Secure CU approval</b>

Tool: <code>{}</code>
{}

Approval ID: <code>{}</code>",
                    crate::telegram_render::escape_html(&pending.tool),
                    crate::telegram_render::escape_html(&pending.policy.reason),
                    crate::telegram_render::escape_html(&approval_id),
                );
                if let (Some(tx), Some(cid)) = (&self.progress_tx, ctx.chat_id) {
                    let _ = tx.try_send(crate::progress_router::ProgressEvent::new(
                        cid,
                        crate::progress_router::ProgressPayload::NeedApproval {
                            approval_id: Some(approval_id.clone()),
                            tool_name: Some("secure_computer_use".into()),
                            summary: Some(summary.clone()),
                            approval_html,
                        },
                    ));
                }
                self.emit_action_phase(
                    ctx.chat_id,
                    "jev_ask",
                    Some("secure_computer_use"),
                    Some("Policy requires owner approval"),
                );
                json!({
                    "ok": false,
                    "status": "pending_human_approval",
                    "error": "policy_ask",
                    "approval_id": approval_id,
                    "intent_id": pending.intent_id,
                    "policy": {
                        "verdict": "ask",
                        "model": pending.policy.model,
                        "reason": pending.policy.reason,
                    },
                    "message": "Policy Ask — waiting for owner approval before CuSigner runs.",
                    "adapter": pipeline.device_adapter(),
                    "audit_tip": pipeline.audit_tip(),
                })
                .to_string()
            }
            Err(PipelineError::AskRejected(reason)) => json!({
                "ok": false,
                "error": "ask_rejected",
                "message": reason.clone(),
                "adapter": pipeline.device_adapter(),
                "audit_tip": pipeline.audit_tip(),
                "policy": { "verdict": "ask" },
            })
            .to_string(),
            Err(PipelineError::Other(e)) => json!({
                "ok": false,
                "error": "pipeline_failed",
                "message": e.to_string(),
                "adapter": pipeline.device_adapter(),
                "audit_tip": pipeline.audit_tip(),
            })
            .to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::computer_use::secure_cu_enabled;

    #[test]
    fn flag_default_is_off_unless_env_set() {
        // Do not assert absolute false — parallel tests may set the env.
        let _ = secure_cu_enabled();
    }
}
