//! Built-in Wasm skill management tool handlers.
//!
//! Contains `handle_tool_skills` — `deactivate_skill`, `reactivate_skill` and
//! `manage_skill`. These operate on the `skill_proposals` table, which is the
//! persisted registry of the built-in Wasm skills shipped in `compiled_skills/`
//! (populated at startup by `auto_register_builtin_skills`). Deactivating a
//! skill unloads it from the sandbox and flips its registry row to
//! `deactivated`, which `main.rs` honours on the next boot.

use anyhow::{Context, Result};
use async_openai::types::chat::ChatCompletionMessageToolCall;
use serde_json::Value;

use crate::memory::{STATUS_ACTIVE, STATUS_DEACTIVATED, SkillProposalUpdate};
use crate::models::SkillCapability;
use crate::orchestrator::{Orchestrator, ToolContext};
use crate::utils::sanitized_tool_error;

impl Orchestrator {
    /// Handle skill-management tools: deactivate_skill, reactivate_skill, manage_skill.
    #[allow(unused_variables)]
    pub(crate) async fn handle_tool_skills(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "deactivate_skill"   => execute_deactivate_skill,
            "reactivate_skill"   => execute_reactivate_skill,
            "manage_skill"       => execute_manage_skill,
        );
        Ok(None)
    }

    /// Unload a skill from the sandbox and mark its registry row `deactivated`
    /// so it stays disabled across restarts. Returns false when the skill was
    /// neither loaded nor present in the registry.
    pub(crate) async fn deactivate_skill_by_name(&self, skill_name: &str) -> bool {
        // Read menu_command before deactivation (skill will be unloaded)
        let menu_info: Option<(String, Option<String>)> = self
            .sandbox
            .loaded_skills()
            .iter()
            .find(|s| s.skill_name == skill_name)
            .and_then(|cap| {
                cap.menu_command
                    .as_ref()
                    .map(|mc| (mc.clone(), cap.menu_description.clone()))
            });

        let sandbox_removed = self.sandbox.unload_skill(skill_name);

        // Persist the deactivated status. Target the *active* registry row so a
        // stale failed row cannot shadow the real one.
        let mut db_updated = false;
        if let Ok(proposals) = self
            .memory
            .list_skill_proposals(None, Some(STATUS_ACTIVE))
            .await
        {
            for p in proposals.iter().filter(|p| p.skill_name == skill_name) {
                let updates = SkillProposalUpdate {
                    status: Some(STATUS_DEACTIVATED.to_string()),
                    ..Default::default()
                };
                if self
                    .memory
                    .update_skill_proposal(p.id, updates)
                    .await
                    .is_ok()
                {
                    db_updated = true;
                }
            }
        }

        if !sandbox_removed && !db_updated {
            return false;
        }

        // Sync menu: remove the skill's command
        if let Some((mc, md)) = menu_info {
            self.sync_menu_for_skill(skill_name, Some(&mc), md.as_deref(), false)
                .await;
        }
        true
    }

    async fn execute_deactivate_skill(&self, args: Value, _ctx: &mut ToolContext<'_>) -> String {
        let skill_name = args
            .get("skill_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if skill_name.is_empty() {
            return serde_json::json!({"status": "error", "error": "Missing 'skill_name' field"})
                .to_string();
        }
        if self.deactivate_skill_by_name(skill_name).await {
            serde_json::json!({
                "status": "ok",
                "message": format!("Skill '{}' has been deactivated.", skill_name),
            })
            .to_string()
        } else {
            serde_json::json!({
                "status": "error",
                "error": format!("Skill '{}' was not found or already deactivated.", skill_name),
            })
            .to_string()
        }
    }

    /// Reload a deactivated skill's Wasm module from `compiled_skills/` and flip
    /// its registry row back to `active`.
    pub(crate) async fn reactivate_skill_by_name(&self, skill_name: &str) -> Result<String> {
        let proposals = self
            .memory
            .list_skill_proposals(None, Some(STATUS_DEACTIVATED))
            .await
            .context("listing deactivated skill registry rows")?;
        let proposal = proposals
            .iter()
            .filter(|p| p.skill_name == skill_name)
            .max_by_key(|p| p.version)
            .ok_or_else(|| anyhow::anyhow!("No deactivated skill '{skill_name}' found."))?;

        let wasm_path = std::path::Path::new("compiled_skills").join(format!("{skill_name}.wasm"));
        let wasm_bytes = tokio::fs::read(&wasm_path).await.with_context(|| {
            format!(
                "No compiled Wasm found for skill '{skill_name}' at {} — cannot reactivate",
                wasm_path.display()
            )
        })?;

        // Prefer the manifest stored in the registry row; fall back to the
        // on-disk manifest that ships with the skill.
        let manifest_json = match proposal.generated_manifest.clone() {
            Some(json) => Some(json),
            None => tokio::fs::read_to_string(
                std::path::Path::new("skills")
                    .join(skill_name)
                    .join("manifest.json"),
            )
            .await
            .ok(),
        };
        let manifest_json = manifest_json.ok_or_else(|| {
            anyhow::anyhow!("No manifest found for skill '{skill_name}' — cannot reactivate.")
        })?;
        let capability: SkillCapability = serde_json::from_str(&manifest_json)
            .with_context(|| format!("manifest for skill '{skill_name}' is invalid"))?;

        self.sandbox
            .hot_load_skill(&wasm_bytes, capability.clone())
            .with_context(|| format!("reloading Wasm for skill '{skill_name}'"))?;

        let updates = SkillProposalUpdate {
            status: Some(STATUS_ACTIVE.to_string()),
            ..Default::default()
        };
        self.memory
            .update_skill_proposal(proposal.id, updates)
            .await
            .context("marking skill registry row active")?;

        // Freshly loaded skill — clear any tripped circuit breaker.
        self.memory.reset_circuit_breaker(skill_name).await;

        // Re-register the skill_<name> toggle and re-add its menu command.
        let default = crate::settings::legacy_skill_default(skill_name).unwrap_or(true);
        self.settings
            .register_skill(skill_name, &capability.description, default);
        self.sync_menu_for_skill(
            skill_name,
            capability.menu_command.as_deref(),
            capability.menu_description.as_deref(),
            true,
        )
        .await;

        Ok(format!(
            "Skill '{skill_name}' has been reactivated and is ready to use."
        ))
    }

    async fn execute_reactivate_skill(&self, args: Value, _ctx: &mut ToolContext<'_>) -> String {
        let skill_name = args
            .get("skill_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if skill_name.is_empty() {
            return serde_json::json!({"status": "error", "error": "Missing 'skill_name' field"})
                .to_string();
        }
        match self.reactivate_skill_by_name(skill_name).await {
            Ok(message) => serde_json::json!({"status": "ok", "message": message}).to_string(),
            Err(e) => sanitized_tool_error(
                "reactivate_skill",
                "Skill reactivation failed / 技能重新激活失败",
                &e,
                "skill_reactivation_error",
                true,
                None,
            ),
        }
    }

    /// Inspect a skill: runtime health from `skill_metrics`, or the deferred
    /// intents that reference it.
    async fn execute_manage_skill(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let skill_name = args
            .get("skill_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if skill_name.is_empty() {
            return serde_json::json!({"status": "error", "error": "Missing 'skill_name'"})
                .to_string();
        }
        match action {
            "health" => match self.memory.get_skill_metrics(skill_name).await {
                Ok(Some(metrics)) => {
                    let error_rate = if metrics.total_calls > 0 {
                        metrics.error_count as f64 / metrics.total_calls as f64 * 100.0
                    } else {
                        0.0
                    };
                    let health_status = if error_rate > 50.0 {
                        "unhealthy"
                    } else if error_rate > 20.0 {
                        "degraded"
                    } else {
                        "healthy"
                    };
                    let health_status = if metrics.circuit_open {
                        "circuit_breaker_open"
                    } else {
                        health_status
                    };
                    serde_json::json!({
                        "status": "ok",
                        "skill_name": skill_name,
                        "health": health_status,
                        "total_calls": metrics.total_calls,
                        "success_count": metrics.success_count,
                        "error_count": metrics.error_count,
                        "error_rate_pct": format!("{:.1}%", error_rate),
                        "avg_latency_ms": format!("{:.0}", metrics.avg_latency_ms),
                        "last_error": metrics.last_error,
                        "error_categories": metrics.error_categories,
                        "circuit_open": metrics.circuit_open,
                        "consecutive_failures": metrics.consecutive_failures,
                    })
                    .to_string()
                }
                Ok(None) => serde_json::json!({
                    "status": "ok",
                    "skill_name": skill_name,
                    "health": "no_data",
                    "message": "No execution metrics recorded yet for this skill",
                })
                .to_string(),
                Err(e) => sanitized_tool_error(
                    "manage_skill",
                    "Failed to query skill health / 技能健康查询失败",
                    &e,
                    "database_error",
                    true,
                    None,
                ),
            },
            "affected_intents" => {
                let user_did = ctx.user_did;
                match self.memory.list_user_intents(user_did).await {
                    Ok(intents) => {
                        let matching: Vec<_> = intents
                            .iter()
                            .filter(|i| i.context_text.contains(skill_name))
                            .map(|i| {
                                serde_json::json!({
                                    "id": i.id,
                                    "trigger_type": i.trigger_type,
                                    "context_text": &i.context_text,
                                    "execution_mode": i.execution_mode,
                                })
                            })
                            .collect();
                        serde_json::json!({
                            "status": "ok",
                            "skill_name": skill_name,
                            "affected_intents": matching,
                            "count": matching.len(),
                        })
                        .to_string()
                    }
                    Err(e) => sanitized_tool_error(
                        "manage_skill",
                        "Failed to query affected intents / 受影响意图查询失败",
                        &e,
                        "database_error",
                        true,
                        None,
                    ),
                }
            }
            _ => serde_json::json!({
                "status": "error",
                "error": format!("Unknown action '{}'. Use: health, affected_intents", action),
            })
            .to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::memory::MemoryHandle;
    use crate::settings::RuntimeSettings;
    use crate::test_tool_helpers::{CtxState, make_tool_call, test_orchestrator};

    #[tokio::test]
    async fn skills_unknown_tool_passes_through() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("deploy_nuclear_missile", "{}");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_skills(&tc, "deploy_nuclear_missile", "{}", &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, None, "unknown tool must pass through");
    }

    #[tokio::test]
    async fn manage_skill_rejects_missing_skill_name() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"health"}"#;
        let tc = make_tool_call("manage_skill", args);
        let mut state = CtxState::new();
        orch.handle_tool_skills(&tc, "manage_skill", args, &mut state.ctx())
            .await
            .expect("should not error");
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("skill_name"), "got: {content}");
    }

    #[tokio::test]
    async fn manage_skill_health_reports_no_data_for_unknown_skill() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"action":"health","skill_name":"web_reader"}"#;
        let tc = make_tool_call("manage_skill", args);
        let mut state = CtxState::new();
        orch.handle_tool_skills(&tc, "manage_skill", args, &mut state.ctx())
            .await
            .expect("should not error");
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("no_data"), "got: {content}");
    }

    #[tokio::test]
    async fn manage_skill_rejects_removed_pipeline_actions() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        for action in ["list_versions", "rollback", "regression_test"] {
            let args = format!(r#"{{"action":"{action}","skill_name":"web_reader"}}"#);
            let tc = make_tool_call("manage_skill", &args);
            let mut state = CtxState::new();
            orch.handle_tool_skills(&tc, "manage_skill", &args, &mut state.ctx())
                .await
                .expect("should not error");
            let content = format!("{:?}", state.messages.last().unwrap());
            assert!(
                content.contains("Unknown action"),
                "action {action} must be rejected, got: {content}"
            );
        }
    }

    #[tokio::test]
    async fn deactivate_skill_reports_missing_skill() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"skill_name":"nonexistent_skill"}"#;
        let tc = make_tool_call("deactivate_skill", args);
        let mut state = CtxState::new();
        orch.handle_tool_skills(&tc, "deactivate_skill", args, &mut state.ctx())
            .await
            .expect("should not error");
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("not found"), "got: {content}");
    }

    #[tokio::test]
    async fn reactivate_skill_reports_missing_deactivated_row() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"skill_name":"nonexistent_skill"}"#;
        let tc = make_tool_call("reactivate_skill", args);
        let mut state = CtxState::new();
        orch.handle_tool_skills(&tc, "reactivate_skill", args, &mut state.ctx())
            .await
            .expect("should not error");
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("skill_reactivation_error"),
            "got: {content}"
        );
    }
}
