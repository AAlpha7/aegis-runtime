//! Jev provenance extracted from tool outputs that actually hit the policy broker.
//!
//! Never invent a verdict. A plain chat reply / tool result without `policy.verdict`
//! or `jev.verdict` yields `None`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::computer_use::policy::{PolicyDecision, PolicyVerdict};

/// Owner-visible Jev fields for the WebApp「来源」panel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct JevProvenance {
    /// `allow` | `ask` | `deny` — only present when the broker actually ran.
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_tip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl JevProvenance {
    pub fn from_decision(
        decision: &PolicyDecision,
        owner_decision: Option<&str>,
        approval_id: Option<&str>,
        receipt: Option<&str>,
        audit_tip: Option<&str>,
    ) -> Self {
        Self {
            verdict: decision.verdict.as_str().to_string(),
            owner_decision: owner_decision
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            approval_id: approval_id
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            receipt: receipt
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            audit_tip: audit_tip
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            risk: risk_from_evidence(&decision.evidence),
            model: Some(decision.model.clone()).filter(|s| !s.trim().is_empty()),
            reason: Some(decision.reason.clone()).filter(|s| !s.trim().is_empty()),
        }
    }
}

fn parse_verdict_str(raw: &str) -> Option<String> {
    PolicyVerdict::parse(raw).map(|v| v.as_str().to_string())
}

fn risk_from_evidence(evidence: &Value) -> Option<String> {
    let noul = evidence
        .get("high_risk_noul")
        .and_then(Value::as_f64)
        .or_else(|| {
            evidence
                .pointer("/answers/high_risk/noul")
                .and_then(Value::as_f64)
        });
    noul.map(|n| format!("high_risk={n:.2}"))
}

fn first_str(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = value.get(*key).and_then(Value::as_str) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn receipt_from_value(value: &Value) -> Option<String> {
    if let Some(s) = value.get("receipt").and_then(Value::as_str) {
        let t = s.trim();
        if !t.is_empty() {
            return Some(t.chars().take(160).collect());
        }
    }
    let receipt = value.get("receipt")?;
    if receipt.is_null() {
        return None;
    }
    if let Some(id) = receipt.get("intent_id").and_then(Value::as_str) {
        return Some(id.trim().chars().take(160).collect());
    }
    if let Some(accepted) = receipt.get("accepted").and_then(Value::as_bool) {
        return Some(format!("accepted={accepted}"));
    }
    None
}

fn owner_decision_from_value(value: &Value) -> Option<String> {
    if let Some(s) = value
        .get("jev")
        .and_then(|j| j.get("owner_decision"))
        .and_then(Value::as_str)
    {
        let t = s.trim().to_ascii_lowercase();
        if t == "approved" || t == "rejected" {
            return Some(t);
        }
    }
    if let Some("owner_approved_ask") = value.get("status").and_then(Value::as_str) {
        return Some("approved".into());
    }
    if let Some("ask_rejected") = value.get("error").and_then(Value::as_str) {
        return Some("rejected".into());
    }
    None
}

/// Extract Jev provenance from a tool-result JSON object. `None` if the
/// broker never recorded a verdict — callers must not invent one.
pub fn jev_provenance_from_tool_value(value: &Value) -> Option<JevProvenance> {
    if !value.is_object() {
        return None;
    }
    let jev_obj = value.get("jev").filter(|v| v.is_object());
    let policy = value.get("policy").filter(|v| v.is_object());

    let verdict = policy
        .and_then(|p| p.get("verdict"))
        .and_then(Value::as_str)
        .and_then(parse_verdict_str)
        .or_else(|| {
            jev_obj
                .and_then(|j| j.get("verdict"))
                .and_then(Value::as_str)
                .and_then(parse_verdict_str)
        })?;

    let model = policy
        .and_then(|p| first_str(p, &["model"]))
        .or_else(|| jev_obj.and_then(|j| first_str(j, &["model"])));
    let reason = policy
        .and_then(|p| first_str(p, &["reason"]))
        .or_else(|| jev_obj.and_then(|j| first_str(j, &["reason"])))
        .or_else(|| first_str(value, &["message"]));

    let risk = policy
        .and_then(|p| p.get("evidence"))
        .and_then(risk_from_evidence)
        .or_else(|| jev_obj.and_then(|j| first_str(j, &["risk"])))
        .or_else(|| value.get("policy").and_then(|p| risk_from_evidence(p)));

    Some(JevProvenance {
        verdict,
        owner_decision: owner_decision_from_value(value)
            .or_else(|| jev_obj.and_then(|j| first_str(j, &["owner_decision"]))),
        approval_id: first_str(value, &["approval_id"])
            .or_else(|| jev_obj.and_then(|j| first_str(j, &["approval_id"]))),
        receipt: receipt_from_value(value)
            .or_else(|| jev_obj.and_then(|j| first_str(j, &["receipt"]))),
        audit_tip: first_str(value, &["audit_tip"])
            .or_else(|| jev_obj.and_then(|j| first_str(j, &["audit_tip"]))),
        risk,
        model,
        reason,
    })
}

/// Merge a Jev object onto a tool-result JSON value (does not invent fields).
pub fn stamp_jev_on_value(mut value: Value, jev: &JevProvenance) -> Value {
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "jev".into(),
            serde_json::to_value(jev).unwrap_or_else(|_| json!({})),
        );
        if obj.get("policy").and_then(|p| p.get("verdict")).is_none() {
            let mut policy = obj
                .get("policy")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            policy.insert("verdict".into(), json!(jev.verdict));
            if let Some(model) = &jev.model {
                policy.entry("model").or_insert_with(|| json!(model));
            }
            if let Some(reason) = &jev.reason {
                policy.entry("reason").or_insert_with(|| json!(reason));
            }
            obj.insert("policy".into(), Value::Object(policy));
        }
        if let Some(id) = &jev.approval_id {
            obj.entry("approval_id").or_insert_with(|| json!(id));
        }
        if let Some(tip) = &jev.audit_tip {
            obj.entry("audit_tip").or_insert_with(|| json!(tip));
        }
    }
    value
}

pub fn stamp_jev_on_json(raw: &str, jev: &JevProvenance) -> String {
    match serde_json::from_str::<Value>(raw) {
        Ok(value) => stamp_jev_on_value(value, jev).to_string(),
        Err(_) => stamp_jev_on_value(json!({ "message": raw }), jev).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer_use::policy::PolicyVerdict;

    fn decision(verdict: PolicyVerdict) -> PolicyDecision {
        PolicyDecision {
            verdict,
            model: "jev-1.13.0".into(),
            evidence: json!({"high_risk_noul": 0.12}),
            reason: "Jev verdict=allow high_risk=0.12 confidence=0.90".into(),
        }
    }

    #[test]
    fn extracts_cu_allow_fields() {
        let value = json!({
            "ok": true,
            "policy": {
                "verdict": "allow",
                "model": "jev-1.13.0",
                "reason": "deterministic verdict allow"
            },
            "receipt": {"accepted": true, "intent_id": "int-1"},
            "audit_tip": "abc123",
            "intent_id": "int-1"
        });
        let jev = jev_provenance_from_tool_value(&value).expect("verdict present");
        assert_eq!(jev.verdict, "allow");
        assert_eq!(jev.audit_tip.as_deref(), Some("abc123"));
        assert_eq!(jev.receipt.as_deref(), Some("int-1"));
        assert_eq!(jev.model.as_deref(), Some("jev-1.13.0"));
        assert!(jev.owner_decision.is_none());
    }

    #[test]
    fn extracts_ask_with_approval_id() {
        let value = json!({
            "ok": false,
            "status": "pending_human_approval",
            "approval_id": "jev-ask-xyz",
            "policy": {"verdict": "ask", "model": "forced/env", "reason": "ask"},
        });
        let jev = jev_provenance_from_tool_value(&value).expect("ask");
        assert_eq!(jev.verdict, "ask");
        assert_eq!(jev.approval_id.as_deref(), Some("jev-ask-xyz"));
        assert!(jev.owner_decision.is_none());
    }

    #[test]
    fn extracts_owner_approved_ask() {
        let value = json!({
            "ok": true,
            "status": "owner_approved_ask",
            "policy": {"verdict": "ask", "model": "jev", "reason": "ask then approved"},
            "audit_tip": "tip-9",
        });
        let jev = jev_provenance_from_tool_value(&value).expect("approved");
        assert_eq!(jev.verdict, "ask");
        assert_eq!(jev.owner_decision.as_deref(), Some("approved"));
        assert_eq!(jev.audit_tip.as_deref(), Some("tip-9"));
    }

    #[test]
    fn extracts_owner_rejected() {
        let value = json!({
            "ok": false,
            "error": "ask_rejected",
            "policy": {"verdict": "ask"},
            "message": "owner said no",
        });
        let jev = jev_provenance_from_tool_value(&value).expect("rejected");
        assert_eq!(jev.owner_decision.as_deref(), Some("rejected"));
    }

    #[test]
    fn plain_chat_or_task_output_has_no_jev() {
        let task = json!({
            "status": "ok",
            "intent_id": 1903,
            "title": "remind me",
        });
        assert!(jev_provenance_from_tool_value(&task).is_none());
        assert!(jev_provenance_from_tool_value(&json!("hello")).is_none());
        assert!(jev_provenance_from_tool_value(&json!({"message": "hi"})).is_none());
    }

    #[test]
    fn does_not_invent_unknown_verdict() {
        let value = json!({"policy": {"verdict": "maybe"}});
        assert!(jev_provenance_from_tool_value(&value).is_none());
    }

    #[test]
    fn stamp_roundtrip_preserves_verdict() {
        let jev = JevProvenance::from_decision(
            &decision(PolicyVerdict::Allow),
            None,
            None,
            None,
            Some("tip"),
        );
        let stamped = stamp_jev_on_value(json!({"status": "ok", "file_id": "f1"}), &jev);
        let extracted = jev_provenance_from_tool_value(&stamped).expect("stamped");
        assert_eq!(extracted.verdict, "allow");
        assert_eq!(extracted.audit_tip.as_deref(), Some("tip"));
        assert_eq!(extracted.risk.as_deref(), Some("high_risk=0.12"));
        assert_eq!(stamped["status"], "ok");
    }
}
