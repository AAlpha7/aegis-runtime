//! Host-side orchestration: Intent → Policy → Sign → Device → Audit.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::computer_use::audit::{AuditChain, AuditEvent};
use crate::computer_use::device::{DeviceAgent, DeviceReceipt, LoopbackDeviceAgent};
use crate::computer_use::intent::{CuIntentBody, CuIntentEnvelope};
use crate::computer_use::linux::LinuxAtspiDeviceAgent;
use crate::computer_use::policy::{PolicyBroker, PolicyDecision, PolicyInput, PolicyVerdict};
use crate::computer_use::signer::CuSigner;
use crate::identity::AgentIdentity;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineOutcome {
    pub intent: CuIntentEnvelope,
    pub policy: PolicyDecision,
    pub receipt: DeviceReceipt,
    pub audit_tip: String,
}

/// Frozen Ask payload — owner must approve before CuSigner runs.
///
/// Stored in the existing HITL channel (`skill_state` / InteractiveOnly approval
/// UX). Resume via [`SecureCuPipeline::continue_after_owner_approval`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingAsk {
    pub intent_id: String,
    pub tool: String,
    pub args: serde_json::Value,
    pub device_id: String,
    pub objective: Option<String>,
    pub policy: PolicyDecision,
    pub proposed_at: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("secure computer use is disabled (set AEGIS_SECURE_CU_ENABLED=1)")]
    Disabled,
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    /// Policy returned Ask; nothing was signed. Owner confirmation required.
    #[error("policy ask requires owner approval before signing: {}", .0.tool)]
    AskRequired(Box<PendingAsk>),
    /// Owner rejected a pending Ask; nothing was signed.
    #[error("owner rejected pending ask: {0}")]
    AskRejected(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Which Device Agent the pipeline submits signed intents to.
#[derive(Clone)]
pub enum PipelineDevice {
    Loopback(Arc<LoopbackDeviceAgent>),
    LinuxAtspi(Arc<LinuxAtspiDeviceAgent>),
}

impl PipelineDevice {
    pub fn device_id(&self) -> &str {
        match self {
            Self::Loopback(d) => d.device_id(),
            Self::LinuxAtspi(d) => d.device_id(),
        }
    }

    pub fn adapter_name(&self) -> &'static str {
        match self {
            Self::Loopback(_) => "loopback",
            Self::LinuxAtspi(_) => "linux_atspi",
        }
    }

    pub fn execute(&self, envelope: &CuIntentEnvelope) -> Result<DeviceReceipt> {
        match self {
            Self::Loopback(d) => d.execute(envelope),
            Self::LinuxAtspi(d) => d.execute(envelope),
        }
    }
}

pub struct SecureCuPipeline {
    identity: Arc<AgentIdentity>,
    broker: PolicyBroker,
    device: PipelineDevice,
    lease_secs: u64,
    /// When true, `Ask` stops before device submission (owner confirmation required).
    require_owner_for_ask: bool,
    /// `None` → read `AEGIS_SECURE_CU_ENABLED`; `Some(true/false)` overrides (tests).
    enabled_override: Option<bool>,
    pub(crate) audit: parking_lot::Mutex<AuditChain>,
}

impl SecureCuPipeline {
    pub fn new(identity: Arc<AgentIdentity>, broker: PolicyBroker, device: PipelineDevice) -> Self {
        Self {
            identity,
            broker,
            device,
            lease_secs: 120,
            require_owner_for_ask: true,
            enabled_override: None,
            audit: parking_lot::Mutex::new(AuditChain::new()),
        }
    }

    pub fn loopback(
        identity: Arc<AgentIdentity>,
        broker: PolicyBroker,
        device_id: impl Into<String>,
    ) -> Self {
        Self::new(
            identity,
            broker,
            PipelineDevice::Loopback(Arc::new(LoopbackDeviceAgent::new(device_id))),
        )
    }

    pub fn linux_atspi(
        identity: Arc<AgentIdentity>,
        broker: PolicyBroker,
        device_id: impl Into<String>,
    ) -> Self {
        Self::new(
            identity,
            broker,
            PipelineDevice::LinuxAtspi(Arc::new(LinuxAtspiDeviceAgent::new(device_id))),
        )
    }

    pub fn with_lease_secs(mut self, secs: u64) -> Self {
        self.lease_secs = secs.max(1);
        self
    }

    pub fn require_owner_for_ask(mut self, require: bool) -> Self {
        self.require_owner_for_ask = require;
        self
    }

    /// Override the env feature flag (used by unit tests; production leaves this `None`).
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled_override = Some(enabled);
        self
    }

    fn is_enabled(&self) -> bool {
        self.enabled_override
            .unwrap_or_else(crate::computer_use::secure_cu_enabled)
    }

    pub fn device_adapter(&self) -> &'static str {
        self.device.adapter_name()
    }

    pub fn device_id(&self) -> &str {
        self.device.device_id()
    }

    pub fn audit_tip(&self) -> String {
        self.audit.lock().tip_hex()
    }

    pub fn verify_audit(&self) -> Result<()> {
        self.audit.lock().verify()
    }

    pub async fn run(
        &self,
        tool: impl Into<String>,
        args: serde_json::Value,
        objective: Option<String>,
    ) -> Result<PipelineOutcome, PipelineError> {
        Box::pin(self.run_with_broker(tool, args, objective, self.broker.clone())).await
    }

    /// A call owns its policy snapshot; concurrent calls cannot replace it.
    /// The device and audit chain remain shared across settings changes.
    pub(crate) async fn run_with_broker(
        &self,
        tool: impl Into<String>,
        args: serde_json::Value,
        objective: Option<String>,
        broker: PolicyBroker,
    ) -> Result<PipelineOutcome, PipelineError> {
        if !self.is_enabled() {
            return Err(PipelineError::Disabled);
        }

        let tool = tool.into();
        let intent_id = Uuid::new_v4().to_string();
        let now = current_unix_secs();
        let device_id = self.device.device_id().to_string();

        {
            let mut audit = self.audit.lock();
            audit
                .append(
                    AuditEvent::IntentProposed,
                    &intent_id,
                    now,
                    json!({
                        "tool": tool,
                        "device_id": device_id,
                        "adapter": self.device.adapter_name(),
                        "args": args
                    }),
                )
                .map_err(PipelineError::Other)?;
        }

        let policy = broker
            .decide(&PolicyInput {
                tool: tool.clone(),
                args: args.clone(),
                device_id: device_id.clone(),
                objective: objective.clone(),
            })
            .await
            .map_err(PipelineError::Other)?;

        {
            let mut audit = self.audit.lock();
            audit
                .append(
                    AuditEvent::PolicyDecided,
                    &intent_id,
                    current_unix_secs(),
                    json!({
                        "verdict": policy.verdict,
                        "model": policy.model,
                        "reason": policy.reason,
                    }),
                )
                .map_err(PipelineError::Other)?;
        }

        if matches!(policy.verdict, PolicyVerdict::Deny) {
            return Err(PipelineError::PolicyDenied(policy.reason));
        }

        if matches!(policy.verdict, PolicyVerdict::Ask) && self.require_owner_for_ask {
            // Do NOT sign. Hand the frozen decision to the existing HITL /
            // InteractiveOnly approval channel; resume with
            // `continue_after_owner_approval` after the owner nods.
            let pending = PendingAsk {
                intent_id: intent_id.clone(),
                tool,
                args,
                device_id,
                objective,
                policy,
                proposed_at: now,
            };
            {
                let mut audit = self.audit.lock();
                let _ = audit.append(
                    AuditEvent::PipelineFailed,
                    &pending.intent_id,
                    current_unix_secs(),
                    json!({
                        "stage": "awaiting_owner_approval",
                        "verdict": "ask",
                        "reason": pending.policy.reason,
                    }),
                );
            }
            return Err(PipelineError::AskRequired(Box::new(pending)));
        }

        self.sign_and_submit(intent_id, tool, args, device_id, now, policy)
    }

    /// After HITL / InteractiveOnly approval of a pending Ask: sign + device.
    ///
    /// Reuses the frozen [`PolicyDecision`] (does not re-call the broker) so the
    /// owner approved exactly what Jev/deterministic decided.
    pub fn continue_after_owner_approval(
        &self,
        pending: PendingAsk,
    ) -> Result<PipelineOutcome, PipelineError> {
        if !self.is_enabled() {
            return Err(PipelineError::Disabled);
        }
        if !matches!(pending.policy.verdict, PolicyVerdict::Ask) {
            return Err(PipelineError::Other(anyhow!(
                "continue_after_owner_approval requires Ask verdict, got {}",
                pending.policy.verdict.as_str()
            )));
        }
        if pending.device_id != self.device.device_id() {
            return Err(PipelineError::Other(anyhow!(
                "pending ask device_id {} != pipeline device {}",
                pending.device_id,
                self.device.device_id()
            )));
        }

        {
            let mut audit = self.audit.lock();
            audit
                .append(
                    AuditEvent::PolicyDecided,
                    &pending.intent_id,
                    current_unix_secs(),
                    json!({
                        "verdict": pending.policy.verdict,
                        "model": pending.policy.model,
                        "reason": pending.policy.reason,
                        "owner_approved": true,
                    }),
                )
                .map_err(PipelineError::Other)?;
        }

        let now = current_unix_secs();
        self.sign_and_submit(
            pending.intent_id,
            pending.tool,
            pending.args,
            pending.device_id,
            now,
            pending.policy,
        )
    }

    /// Owner rejected a pending Ask — audit only, never sign.
    pub fn reject_ask(&self, pending: &PendingAsk, reason: impl Into<String>) -> PipelineError {
        let reason = reason.into();
        {
            let mut audit = self.audit.lock();
            let _ = audit.append(
                AuditEvent::PipelineFailed,
                &pending.intent_id,
                current_unix_secs(),
                json!({
                    "stage": "owner_rejected_ask",
                    "reason": reason,
                    "tool": pending.tool,
                }),
            );
        }
        PipelineError::AskRejected(reason)
    }

    fn sign_and_submit(
        &self,
        intent_id: String,
        tool: String,
        args: serde_json::Value,
        device_id: String,
        now: u64,
        policy: PolicyDecision,
    ) -> Result<PipelineOutcome, PipelineError> {
        let mut body = CuIntentBody::new(
            intent_id.clone(),
            self.identity.did.clone(),
            device_id,
            tool,
            args,
            now,
            self.lease_secs,
            policy.verdict,
        );
        body.policy_model.replace(policy.model.clone());
        body.policy_evidence.replace(policy.evidence.clone());

        let intent = CuSigner::new(self.identity.as_ref())
            .sign(body)
            .map_err(PipelineError::Other)?;

        {
            let mut audit = self.audit.lock();
            audit
                .append(
                    AuditEvent::IntentSigned,
                    &intent_id,
                    current_unix_secs(),
                    json!({"signature_prefix": intent.signature.chars().take(16).collect::<String>()}),
                )
                .map_err(PipelineError::Other)?;
        }

        let receipt = self.device.execute(&intent).map_err(PipelineError::Other)?;

        {
            let mut audit = self.audit.lock();
            let event = if receipt.accepted {
                AuditEvent::DeviceAccepted
            } else {
                AuditEvent::DeviceRejected
            };
            audit
                .append(
                    event,
                    &intent_id,
                    receipt.observed_at,
                    json!({
                        "accepted": receipt.accepted,
                        "reject_reason": receipt.reject_reason,
                        "receipt_hash": receipt.receipt_hash,
                    }),
                )
                .map_err(PipelineError::Other)?;
        }

        Ok(PipelineOutcome {
            intent,
            policy,
            receipt,
            audit_tip: self.audit_tip(),
        })
    }
}

fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer_use::device::LoopbackDeviceAgent;
    use crate::computer_use::linux::{LinuxSessionProbe, StaticLinuxBackend};
    use crate::computer_use::policy::PolicyVerdict;
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn pipeline_allow_reaches_device() {
        let identity = Arc::new(AgentIdentity::generate());
        let broker = PolicyBroker::deterministic(PolicyVerdict::Allow);
        let device = PipelineDevice::Loopback(Arc::new(
            LoopbackDeviceAgent::new("dev-loop").accept_ask(true),
        ));
        let pipeline = SecureCuPipeline::new(identity, broker, device)
            .with_enabled(true)
            .require_owner_for_ask(false)
            .with_lease_secs(300);

        let out = pipeline
            .run("notes.ping", json!({}), None)
            .await
            .expect("pipeline");
        assert!(out.receipt.accepted);
        assert!(pipeline.verify_audit().is_ok());
    }

    #[tokio::test]
    async fn jev_settings_broker_refresh_preserves_audit_and_requires_next_approval() {
        let pipeline = SecureCuPipeline::loopback(
            Arc::new(AgentIdentity::generate()),
            PolicyBroker::deterministic(PolicyVerdict::Allow),
            "live-config-test",
        )
        .with_enabled(true);
        pipeline.run("notes.ping", json!({}), None).await.unwrap();
        let (ask, deny) = tokio::join!(
            pipeline.run_with_broker(
                "notes.ping",
                json!({}),
                None,
                PolicyBroker::deterministic(PolicyVerdict::Ask)
            ),
            pipeline.run_with_broker(
                "notes.ping",
                json!({}),
                None,
                PolicyBroker::deterministic(PolicyVerdict::Deny)
            ),
        );
        assert!(matches!(ask, Err(PipelineError::AskRequired(_))));
        assert!(matches!(deny, Err(PipelineError::PolicyDenied(_))));
        assert!(pipeline.verify_audit().is_ok());
    }

    #[tokio::test]
    async fn pipeline_deny_stops_before_device() {
        let identity = Arc::new(AgentIdentity::generate());
        let broker = PolicyBroker::deterministic(PolicyVerdict::Deny);
        let pipeline = SecureCuPipeline::loopback(identity, broker, "dev-loop").with_enabled(true);
        let err = pipeline
            .run("shell.exec", json!({"cmd": "rm -rf /"}), None)
            .await
            .expect_err("must deny");
        assert!(matches!(err, PipelineError::PolicyDenied(_)));
    }

    #[tokio::test]
    async fn pipeline_disabled_without_flag() {
        let identity = Arc::new(AgentIdentity::generate());
        let broker = PolicyBroker::deterministic(PolicyVerdict::Ask);
        let pipeline = SecureCuPipeline::loopback(identity, broker, "dev-loop").with_enabled(false);
        let err = pipeline
            .run("system.frontmost_app", json!({}), None)
            .await
            .expect_err("disabled");
        assert!(matches!(err, PipelineError::Disabled));
    }

    #[tokio::test]
    async fn pipeline_linux_adapter_unavailable_is_clean() {
        let identity = Arc::new(AgentIdentity::generate());
        let broker = PolicyBroker::deterministic(PolicyVerdict::Allow);
        let backend = Arc::new(StaticLinuxBackend {
            probe: LinuxSessionProbe {
                available: false,
                reason: "no_display (test)".into(),
                display: None,
                wayland_display: None,
                dbus_session_bus: None,
                atspi_bus_address: None,
                platform: "linux".into(),
            },
            frontmost: Err("no_display (test)".into()),
            tree: Err("no_display (test)".into()),
            focused: Err("no_display (test)".into()),
            screenshot: Err("no_display (test)".into()),
        });
        let device = PipelineDevice::LinuxAtspi(Arc::new(
            LinuxAtspiDeviceAgent::with_backend("dev-linux", backend).accept_ask(true),
        ));
        let pipeline = SecureCuPipeline::new(identity, broker, device)
            .with_enabled(true)
            .require_owner_for_ask(false);
        let out = pipeline
            .run("system.frontmost_app", json!({}), None)
            .await
            .expect("pipeline");
        assert!(!out.receipt.accepted);
        assert_eq!(
            out.receipt.reject_reason,
            Some(crate::computer_use::device::DeviceRejectReason::AdapterUnavailable)
        );
        assert!(pipeline.verify_audit().is_ok());
    }

    #[tokio::test]
    async fn ask_without_owner_approval_does_not_sign() {
        let identity = Arc::new(AgentIdentity::generate());
        let broker = PolicyBroker::deterministic(PolicyVerdict::Ask);
        let device = PipelineDevice::Loopback(Arc::new(
            LoopbackDeviceAgent::new("dev-loop").accept_ask(true),
        ));
        let pipeline = SecureCuPipeline::new(identity, broker, device)
            .with_enabled(true)
            .require_owner_for_ask(true);

        let err = pipeline
            .run("system.frontmost_app", json!({"x": 1}), None)
            .await
            .expect_err("ask must pend");
        let pending = match err {
            PipelineError::AskRequired(p) => *p,
            other => panic!("expected AskRequired, got {other:?}"),
        };
        assert_eq!(pending.tool, "system.frontmost_app");
        assert_eq!(pending.policy.verdict, PolicyVerdict::Ask);
        // Audit has proposal + policy + awaiting — never IntentSigned.
        let events: Vec<_> = pipeline
            .audit
            .lock()
            .records()
            .iter()
            .map(|r| r.event.clone())
            .collect();
        assert!(events.contains(&AuditEvent::IntentProposed));
        assert!(events.contains(&AuditEvent::PolicyDecided));
        assert!(!events.contains(&AuditEvent::IntentSigned));
    }

    #[tokio::test]
    async fn ask_after_owner_approval_signs_and_submits() {
        let identity = Arc::new(AgentIdentity::generate());
        let broker = PolicyBroker::deterministic(PolicyVerdict::Ask);
        let device = PipelineDevice::Loopback(Arc::new(
            LoopbackDeviceAgent::new("dev-loop").accept_ask(true),
        ));
        let pipeline = SecureCuPipeline::new(identity, broker, device)
            .with_enabled(true)
            .require_owner_for_ask(true);

        let err = pipeline
            .run("system.frontmost_app", json!({}), None)
            .await
            .expect_err("ask");
        let pending = match err {
            PipelineError::AskRequired(p) => *p,
            other => panic!("expected AskRequired, got {other:?}"),
        };
        let out = pipeline
            .continue_after_owner_approval(pending)
            .expect("owner-approved ask should sign");
        assert!(out.receipt.accepted);
        assert_eq!(out.policy.verdict, PolicyVerdict::Ask);
        assert!(!out.intent.signature.is_empty());
        assert!(pipeline.verify_audit().is_ok());
    }

    #[tokio::test]
    async fn ask_owner_reject_never_signs() {
        let identity = Arc::new(AgentIdentity::generate());
        let broker = PolicyBroker::deterministic(PolicyVerdict::Ask);
        let pipeline = SecureCuPipeline::loopback(identity, broker, "dev-loop")
            .with_enabled(true)
            .require_owner_for_ask(true);
        let err = pipeline
            .run("system.frontmost_app", json!({}), None)
            .await
            .expect_err("ask");
        let pending = match err {
            PipelineError::AskRequired(p) => *p,
            other => panic!("expected AskRequired, got {other:?}"),
        };
        let rejected = pipeline.reject_ask(&pending, "owner said no");
        assert!(matches!(rejected, PipelineError::AskRejected(_)));
        let events: Vec<_> = pipeline
            .audit
            .lock()
            .records()
            .iter()
            .map(|r| r.event.clone())
            .collect();
        assert!(!events.contains(&AuditEvent::IntentSigned));
    }

    #[tokio::test]
    async fn pipeline_new_readonly_tools_reach_loopback() {
        let identity = Arc::new(AgentIdentity::generate());
        let broker = PolicyBroker::deterministic(PolicyVerdict::Allow);
        let device = PipelineDevice::Loopback(Arc::new(
            LoopbackDeviceAgent::new("dev-loop").accept_ask(true),
        ));
        let pipeline = SecureCuPipeline::new(identity, broker, device)
            .with_enabled(true)
            .require_owner_for_ask(false);
        for tool in [
            "system.accessible_tree",
            "system.focused_control",
            "system.screenshot_digest",
        ] {
            let out = pipeline
                .run(tool, json!({"max_depth": 2}), None)
                .await
                .unwrap_or_else(|e| panic!("{tool}: {e}"));
            assert!(out.receipt.accepted, "{tool}");
            assert!(pipeline.verify_audit().is_ok());
        }
    }

    #[tokio::test]
    async fn flag_off_ask_is_disabled_not_ask() {
        let identity = Arc::new(AgentIdentity::generate());
        let broker = PolicyBroker::deterministic(PolicyVerdict::Ask);
        let pipeline = SecureCuPipeline::loopback(identity, broker, "dev-loop")
            .with_enabled(false)
            .require_owner_for_ask(true);
        let err = pipeline
            .run("system.frontmost_app", json!({}), None)
            .await
            .expect_err("disabled");
        assert!(matches!(err, PipelineError::Disabled));
    }
}
