use super::*;
use crate::settings_audit::{SettingsFinding, audit_runtime_settings};

#[derive(serde::Serialize)]
pub(super) struct SettingsAuditResponse {
    findings: Vec<SettingsFinding>,
}

/// GET /diagnostics/settings_audit — bearer admin.
///
/// Returns the current runtime-settings consistency findings in owner language.
pub(super) async fn settings_audit(
    State(state): State<Arc<AdminState>>,
) -> Json<SettingsAuditResponse> {
    let locale = state.settings.display_language();
    let catalog = crate::model_catalog::cached_model_catalog();
    Json(SettingsAuditResponse {
        findings: audit_runtime_settings(&state.settings, &locale, catalog.as_ref()),
    })
}

#[derive(serde::Deserialize, Default)]
pub(super) struct SecureCuSmokeBody {
    /// CU logical tool (default `system.session_probe`).
    #[serde(default)]
    pub cu_tool: Option<String>,
    /// Optional CU args (max_depth / max_nodes for accessible_tree).
    #[serde(default)]
    pub args: Option<serde_json::Value>,
    /// When true, also run a hard-deny (`shell.exec`) expect-deny check.
    #[serde(default)]
    pub include_deny: Option<bool>,
    /// `run` (default) | `approve` | `reject` — Ask HITL closed loop.
    #[serde(default)]
    pub action: Option<String>,
    /// Frozen pending Ask from a previous `run` that returned `policy_ask`.
    #[serde(default)]
    pub pending_ask: Option<crate::computer_use::PendingAsk>,
    /// Diagnostics-only: force allow|ask|deny (skips live Jev). Hard-deny still wins.
    #[serde(default)]
    pub force_verdict: Option<String>,
}

fn diagnostics_cu_pipeline(
    identity: std::sync::Arc<crate::identity::AgentIdentity>,
    force_verdict: Option<&str>,
) -> anyhow::Result<crate::computer_use::SecureCuPipeline> {
    use crate::computer_use::{
        CuAdapterKind, PolicyBroker, PolicyVerdict, SecureCuPipeline, select_pipeline_device,
    };
    let kind = CuAdapterKind::from_env();
    let device = select_pipeline_device(kind);
    let broker = match force_verdict.and_then(PolicyVerdict::parse) {
        Some(v) => PolicyBroker::deterministic_forced(v),
        None => crate::computer_use::build_live_policy_broker()?,
    };
    Ok(SecureCuPipeline::new(identity, broker, device).require_owner_for_ask(true))
}

/// POST /diagnostics/secure_cu — bearer admin smoke for Secure Computer Use.
///
/// Requires `AEGIS_SECURE_CU_ENABLED=1`. Default run: allow-ish probe
/// (`system.session_probe`) + deny (`shell.exec`). `action=approve|reject`
/// closes a pending Ask without signing on reject. Headless AT-SPI tools
/// return `adapter_unavailable` (no panic).
pub(super) async fn secure_cu_smoke(
    State(state): State<Arc<AdminState>>,
    body: Option<Json<SecureCuSmokeBody>>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !crate::computer_use::secure_cu_enabled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "ok": false,
                "error": "disabled",
                "message": "set AEGIS_SECURE_CU_ENABLED=1",
                "health": crate::computer_use::health_component(),
            })),
        );
    }

    let Some(identity) = state.identity.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "ok": false,
                "error": "no_identity",
                "message": "AdminState has no AgentIdentity",
            })),
        );
    };

    let parsed = body.as_ref().map(|Json(b)| b);
    let cu_tool = parsed
        .and_then(|b| b.cu_tool.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("system.session_probe");
    let include_deny = parsed.and_then(|b| b.include_deny).unwrap_or(true);
    let action = parsed
        .and_then(|b| b.action.as_deref())
        .unwrap_or("run")
        .trim()
        .to_ascii_lowercase();
    let cu_args = parsed
        .and_then(|b| b.args.clone())
        .unwrap_or_else(|| serde_json::json!({}));
    let force_verdict = parsed.and_then(|b| b.force_verdict.as_deref());

    let pipeline = match diagnostics_cu_pipeline(std::sync::Arc::clone(identity), force_verdict) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "ok": false,
                    "error": "init_failed",
                    "message": e.to_string(),
                })),
            );
        }
    };

    if action == "approve" || action == "reject" {
        let Some(pending) = parsed.and_then(|b| b.pending_ask.clone()) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "ok": false,
                    "error": "missing_pending_ask",
                    "message": "approve/reject require pending_ask from a prior Ask run",
                })),
            );
        };
        if action == "reject" {
            let err = pipeline.reject_ask(&pending, "diagnostics owner reject");
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "action": "reject",
                    "error": "ask_rejected",
                    "message": err.to_string(),
                    "intent_id": pending.intent_id,
                    "audit_tip": pipeline.audit_tip(),
                    "audit_verified": pipeline.verify_audit().is_ok(),
                    "signed": false,
                })),
            );
        }
        return match pipeline.continue_after_owner_approval(pending) {
            Ok(out) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "action": "approve",
                    "accepted": out.receipt.accepted,
                    "reject_reason": out.receipt.reject_reason,
                    "receipt": out.receipt,
                    "policy": out.policy,
                    "audit_tip": out.audit_tip,
                    "audit_verified": pipeline.verify_audit().is_ok(),
                    "signed": true,
                })),
            ),
            Err(e) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "ok": false,
                    "action": "approve",
                    "error": e.to_string(),
                })),
            ),
        };
    }

    let run = pipeline
        .run(cu_tool, cu_args, Some("diagnostics smoke".into()))
        .await;
    let deny = if include_deny {
        Some(
            pipeline
                .run("shell.exec", serde_json::json!({"cmd": "true"}), None)
                .await,
        )
    } else {
        None
    };

    let (run_json, run_ok, pending_ask) = match &run {
        Ok(out) => (
            serde_json::json!({
                "ok": true,
                "status": "ran",
                "accepted": out.receipt.accepted,
                "reject_reason": out.receipt.reject_reason,
                "receipt_hash": out.receipt.receipt_hash,
                "result": out.receipt.result,
                "policy": out.policy,
                "audit_tip": out.audit_tip,
            }),
            true,
            None,
        ),
        Err(crate::computer_use::PipelineError::AskRequired(pending)) => {
            let pending = pending.as_ref().clone();
            (
                serde_json::json!({
                    "ok": true,
                    "status": "pending_human_approval",
                    "error": "policy_ask",
                    "intent_id": pending.intent_id,
                    "policy": pending.policy,
                    "message": "Policy Ask — waiting for owner approval before CuSigner runs.",
                }),
                true,
                Some(pending.clone()),
            )
        }
        Err(crate::computer_use::PipelineError::PolicyDenied(reason)) => (
            serde_json::json!({
                "ok": true,
                "status": "denied",
                "denied": true,
                "reason": reason,
            }),
            true,
            None,
        ),
        Err(e) => (
            serde_json::json!({
                "ok": false,
                "error": e.to_string(),
            }),
            false,
            None,
        ),
    };

    let deny_json = deny.map(|d| match d {
        Ok(out) => serde_json::json!({
            "unexpected_accept": true,
            "accepted": out.receipt.accepted,
            "message": "shell.exec should have been policy-denied before device",
        }),
        Err(crate::computer_use::PipelineError::PolicyDenied(reason)) => serde_json::json!({
            "ok": true,
            "denied": true,
            "reason": reason,
            "audit_tip": pipeline.audit_tip(),
        }),
        Err(e) => serde_json::json!({
            "ok": false,
            "error": e.to_string(),
        }),
    });

    let audit_ok = pipeline.verify_audit().is_ok();
    let deny_ok = match &deny_json {
        None => true,
        Some(v) => v.get("denied").and_then(|x| x.as_bool()).unwrap_or(false),
    };
    let ok = run_ok && deny_ok && audit_ok;
    let status = if ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(serde_json::json!({
            "ok": ok,
            "action": "run",
            "cu_tool": cu_tool,
            "adapter": pipeline.device_adapter(),
            "device_id": pipeline.device_id(),
            "health": crate::computer_use::health_component(),
            "allow": run_json,
            "run": run_json,
            "pending_ask": pending_ask,
            "deny": deny_json,
            "audit_verified": audit_ok,
            "audit_tip": pipeline.audit_tip(),
        })),
    )
}

#[derive(serde::Deserialize, Default)]
pub(super) struct JevNonCuSmokeBody {
    /// Gated tool (default `google_drive_upload`).
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub args: Option<serde_json::Value>,
    /// `decide` (default) | `approve` | `reject`.
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub pending_ask: Option<crate::computer_use::NonCuPendingAsk>,
    /// Diagnostics-only force allow|ask|deny.
    #[serde(default)]
    pub force_verdict: Option<String>,
}

/// POST /diagnostics/jev_non_cu — bearer admin smoke for the non-CU Jev gate.
///
/// Requires `AEGIS_JEV_NON_CU_GATE_ENABLED=1`. Does **not** call Drive / MCP;
/// it only exercises allow|ask|deny + Ask HITL resume. Flag off → 503.
pub(super) async fn jev_non_cu_smoke(
    State(_state): State<Arc<AdminState>>,
    body: Option<Json<JevNonCuSmokeBody>>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !crate::computer_use::jev_non_cu_gate_enabled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "ok": false,
                "error": "disabled",
                "message": "set AEGIS_JEV_NON_CU_GATE_ENABLED=1",
                "health": crate::computer_use::non_cu_gate::health_component(),
            })),
        );
    }

    let parsed = body.as_ref().map(|Json(b)| b);
    let tool = parsed
        .and_then(|b| b.tool.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("google_drive_upload");
    let args = parsed.and_then(|b| b.args.clone()).unwrap_or_else(
        || serde_json::json!({"filename":"jev-gate-smoke.txt","content_text":"no upload"}),
    );
    let action = parsed
        .and_then(|b| b.action.as_deref())
        .unwrap_or("decide")
        .trim()
        .to_ascii_lowercase();
    let force = parsed
        .and_then(|b| b.force_verdict.as_deref())
        .and_then(crate::computer_use::PolicyVerdict::parse);

    if action == "approve" || action == "reject" {
        let Some(pending) = parsed.and_then(|b| b.pending_ask.clone()) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "ok": false,
                    "error": "missing_pending_ask",
                })),
            );
        };
        if action == "reject" {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "action": "reject",
                    "status": "ask_rejected",
                    "executed": false,
                    "intent_id": pending.intent_id,
                    "tool": pending.tool,
                    "message": "owner rejected pending non-CU ask — original tool never ran",
                })),
            );
        }
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "action": "approve",
                "status": "owner_approved_ask",
                "executed": false,
                "would_execute": pending.tool,
                "intent_id": pending.intent_id,
                "policy": pending.policy,
                "message": "owner approved — live path would now run the original tool (diagnostics does not upload)",
            })),
        );
    }

    let broker = match force {
        Some(v) => crate::computer_use::PolicyBroker::deterministic_forced(v),
        None => crate::computer_use::non_cu_gate::build_non_cu_policy_broker(),
    };
    let decision = match crate::computer_use::non_cu_gate::decide_non_cu(
        &broker,
        tool,
        args.clone(),
        Some("diagnostics jev non-cu smoke".into()),
    )
    .await
    {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "ok": false,
                    "error": "decide_failed",
                    "message": e.to_string(),
                })),
            );
        }
    };

    let (status_label, pending_ask) = match decision.verdict {
        crate::computer_use::PolicyVerdict::Ask => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            (
                "pending_human_approval",
                Some(crate::computer_use::NonCuPendingAsk {
                    intent_id: uuid::Uuid::new_v4().to_string(),
                    tool: tool.to_string(),
                    args,
                    objective: Some("diagnostics jev non-cu smoke".into()),
                    policy: decision.clone(),
                    proposed_at: now,
                }),
            )
        }
        crate::computer_use::PolicyVerdict::Allow => ("allow", None),
        crate::computer_use::PolicyVerdict::Deny => ("denied", None),
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "action": "decide",
            "tool": tool,
            "status": status_label,
            "policy": decision,
            "pending_ask": pending_ask,
            "executed": false,
            "health": crate::computer_use::non_cu_gate::health_component(),
        })),
    )
}
