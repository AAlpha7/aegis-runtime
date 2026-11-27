//! Policy Broker for Secure Computer Use.
//!
//! Production path: TypeSafe Jev via OpenRouter Decisions API (Choice/Score/Noul).
//! Tests / offline: [`DeterministicPolicyBroker`].

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::computer_use::decisions::{
    DecisionsAnswer, DecisionsClient, DecisionsResponse, JevModel,
};

/// Final gate outcome after composing Jev answers (and any hard rules).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolicyVerdict {
    Allow,
    #[default]
    Ask,
    Deny,
}

impl PolicyVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "allow" => Some(Self::Allow),
            "ask" => Some(Self::Ask),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// Broker output recorded into the signed intent and audit chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyDecision {
    pub verdict: PolicyVerdict,
    pub model: String,
    /// Opaque evidence (Jev answers, rule hits). Safe to log; no secrets.
    pub evidence: Value,
    pub reason: String,
}

/// Input the broker judges.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyInput {
    pub tool: String,
    pub args: Value,
    pub device_id: String,
    /// Optional free-text objective from the owner / planner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
}

/// Hard deny list for P0 (no OS adapters yet). Keeps obviously dangerous
/// tool names out even if Jev is unreachable or misconfigured.
fn hard_deny_tool(tool: &str) -> bool {
    let t = tool.trim().to_ascii_lowercase();
    t.contains("shell.exec")
        || t.contains("eval")
        || t.contains("doshellscript")
        || t.starts_with("osascript.")
        || t.contains("delete_all")
}

/// Offline / test broker with explicit allow|ask|deny mapping by tool prefix.
#[derive(Debug, Clone, Default)]
pub struct DeterministicPolicyBroker {
    pub default_verdict: PolicyVerdict,
    /// When true, `default_verdict` applies to every non-hard-denied tool
    /// (diagnostics / Oracle hand-test). Production live path leaves this false.
    pub force_all: bool,
}

impl DeterministicPolicyBroker {
    pub fn forced(verdict: PolicyVerdict) -> Self {
        Self {
            default_verdict: verdict,
            force_all: true,
        }
    }

    pub async fn decide(&self, input: &PolicyInput) -> Result<PolicyDecision> {
        if hard_deny_tool(&input.tool) {
            return Ok(PolicyDecision {
                verdict: PolicyVerdict::Deny,
                model: "deterministic/hard-deny".into(),
                evidence: json!({"rule": "hard_deny_tool"}),
                reason: format!("tool `{}` is hard-denied in P0", input.tool),
            });
        }
        let verdict = if self.force_all {
            self.default_verdict
        } else if input.tool == "system.session_probe" {
            // Read-only adapter probe — safe to allow offline without Jev.
            PolicyVerdict::Allow
        } else if input.tool.starts_with("system.") || input.tool.starts_with("browser.") {
            // Other read-ish tools default to ask until richer evidence exists.
            PolicyVerdict::Ask
        } else {
            self.default_verdict
        };
        Ok(PolicyDecision {
            verdict,
            model: "deterministic/v1".into(),
            evidence: json!({"mode": "deterministic"}),
            reason: format!("deterministic verdict {}", verdict.as_str()),
        })
    }
}

/// Production broker: OpenRouter Decisions + Jev.
pub struct JevPolicyBroker {
    client: DecisionsClient,
    /// If Jev Choice confidence is below this, downgrade Allow → Ask.
    min_confidence: f64,
    /// If high_risk noul exceeds this, escalate to Deny.
    high_risk_deny_threshold: f64,
}

impl JevPolicyBroker {
    pub fn new(api_key: impl Into<String>, model: JevModel) -> Result<Self> {
        Ok(Self {
            client: DecisionsClient::new(api_key, model)?,
            min_confidence: 0.55,
            high_risk_deny_threshold: 0.85,
        })
    }

    pub fn from_env() -> Result<Self> {
        let key = crate::computer_use::resolve_openrouter_api_key().context(
            "Jev needs AEGIS_JEV_ENABLED=1 and AEGIS_JEV_API_KEY or OPENROUTER_API_KEY (OpenRouter Decisions; subscription login is not sufficient)",
        )?;
        Self::new(key, JevModel::from_env())
    }

    pub fn with_client(client: DecisionsClient) -> Self {
        Self {
            client,
            min_confidence: 0.55,
            high_risk_deny_threshold: 0.85,
        }
    }

    fn compose(input: &PolicyInput, resp: &DecisionsResponse) -> Result<PolicyDecision> {
        if hard_deny_tool(&input.tool) {
            return Ok(PolicyDecision {
                verdict: PolicyVerdict::Deny,
                model: resp.model.clone(),
                evidence: json!({"rule": "hard_deny_tool", "answers": resp.answers}),
                reason: format!("tool `{}` is hard-denied in P0", input.tool),
            });
        }

        let verdict_answer = resp
            .answers
            .get("verdict")
            .context("Decisions response missing `verdict` answer")?;

        let (mut verdict, confidence) = match verdict_answer {
            DecisionsAnswer::Choice {
                choice, confidence, ..
            } => {
                let v = PolicyVerdict::parse(choice)
                    .with_context(|| format!("unknown verdict choice `{choice}`"))?;
                (v, confidence.unwrap_or(1.0))
            }
            other => bail!("verdict must be a Choice answer, got {other:?}"),
        };

        let high_risk = match resp.answers.get("high_risk") {
            Some(DecisionsAnswer::Noul { noul }) => *noul,
            _ => 0.0,
        };

        if high_risk >= 0.85 {
            verdict = PolicyVerdict::Deny;
        } else if high_risk >= 0.6 && matches!(verdict, PolicyVerdict::Allow) {
            verdict = PolicyVerdict::Ask;
        }

        if matches!(verdict, PolicyVerdict::Allow) && confidence < 0.55 {
            verdict = PolicyVerdict::Ask;
        }

        Ok(PolicyDecision {
            verdict,
            model: resp.model.clone(),
            evidence: json!({
                "answers": resp.answers,
                "usage": resp.usage,
                "high_risk_noul": high_risk,
                "choice_confidence": confidence,
            }),
            reason: format!(
                "Jev verdict={} high_risk={high_risk:.2} confidence={confidence:.2}",
                verdict.as_str()
            ),
        })
    }

    /// Local hard deny. Does not call Decisions.
    ///
    /// Destructive names (`shell.exec`, `eval`, `osascript.*`, `delete_all`)
    /// are denied by rule. Waiting on Jev cannot change that, and a slow or
    /// down Decisions endpoint must not delay the deny (client timeout is 30s).
    fn hard_deny_before_decisions(&self, input: &PolicyInput) -> Option<PolicyDecision> {
        if !hard_deny_tool(&input.tool) {
            return None;
        }
        Some(PolicyDecision {
            verdict: PolicyVerdict::Deny,
            model: "hard-deny/pre-decisions".into(),
            evidence: json!({
                "rule": "hard_deny_tool",
                "skipped_decisions": true,
                "configured_model": self.client.model().as_str(),
            }),
            reason: format!("tool `{}` is hard-denied before Decisions", input.tool),
        })
    }

    pub async fn decide(&self, input: &PolicyInput) -> Result<PolicyDecision> {
        if let Some(decision) = self.hard_deny_before_decisions(input) {
            return Ok(decision);
        }
        let state = json!({
            "protocol": "aegis-runtime.secure_computer_use",
            "version": 1,
            "tool": input.tool,
            "args": input.args,
            "device_id": input.device_id,
            "objective": input.objective,
            "constraints": {
                "no_dynamic_script": true,
                "no_shell_exec": true,
                "linux_atspi_shared_path": true,
                "mac_jxa_deferred": true,
                "blade_uia_deferred": true
            }
        });
        let req = self.client.policy_request(state);
        let resp = self.client.decide(req).await?;
        let mut decision = Self::compose(input, &resp)?;
        if let Some(DecisionsAnswer::Choice { confidence, .. }) = resp.answers.get("verdict") {
            let conf = confidence.unwrap_or(1.0);
            if matches!(decision.verdict, PolicyVerdict::Allow) && conf < self.min_confidence {
                decision.verdict = PolicyVerdict::Ask;
                decision.reason.push_str(" (downgraded: low confidence)");
            }
        }
        if let Some(DecisionsAnswer::Noul { noul }) = resp.answers.get("high_risk")
            && *noul >= self.high_risk_deny_threshold
        {
            decision.verdict = PolicyVerdict::Deny;
            decision
                .reason
                .push_str(" (escalated: high_risk threshold)");
        }
        Ok(decision)
    }
}

/// Enum dispatch so the pipeline does not need `async_trait`.
#[derive(Clone)]
pub enum PolicyBroker {
    Deterministic(Arc<DeterministicPolicyBroker>),
    Jev(Arc<JevPolicyBroker>),
}

impl PolicyBroker {
    pub fn deterministic(default_verdict: PolicyVerdict) -> Self {
        Self::Deterministic(Arc::new(DeterministicPolicyBroker {
            default_verdict,
            force_all: false,
        }))
    }

    pub fn deterministic_forced(verdict: PolicyVerdict) -> Self {
        Self::Deterministic(Arc::new(DeterministicPolicyBroker::forced(verdict)))
    }

    pub fn jev(broker: JevPolicyBroker) -> Self {
        Self::Jev(Arc::new(broker))
    }

    pub async fn decide(&self, input: &PolicyInput) -> Result<PolicyDecision> {
        match self {
            Self::Deterministic(b) => b.decide(input).await,
            Self::Jev(b) => {
                let result = b.decide(input).await;
                if result.is_err() {
                    crate::behavior_notice::record(
                        "jev_assessment_failed",
                        "Jev 判断暂时不可用，本次操作已暂停，请重试。",
                        "Jev assessment is temporarily unavailable. This action was paused; please retry.",
                    );
                }
                result
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer_use::decisions::{DecisionsAnswer, DecisionsResponse, DecisionsUsage};
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn deterministic_hard_denies_shell() {
        let broker = DeterministicPolicyBroker::default();
        let d = broker
            .decide(&PolicyInput {
                tool: "shell.exec".into(),
                args: json!({}),
                device_id: "d1".into(),
                objective: None,
            })
            .await
            .unwrap();
        assert_eq!(d.verdict, PolicyVerdict::Deny);
    }

    #[test]
    fn compose_escalates_high_risk() {
        let mut answers = BTreeMap::new();
        answers.insert(
            "verdict".into(),
            DecisionsAnswer::Choice {
                choice: "allow".into(),
                probabilities: BTreeMap::new(),
                confidence: Some(0.9),
            },
        );
        answers.insert("high_risk".into(), DecisionsAnswer::Noul { noul: 0.91 });
        let resp = DecisionsResponse {
            model: "jev-1.13.0".into(),
            answers,
            usage: Some(DecisionsUsage {
                input_tokens: Some(10),
                output_tokens: Some(5),
                total_tokens: None,
            }),
            id: None,
        };
        let input = PolicyInput {
            tool: "notes.create".into(),
            args: json!({}),
            device_id: "d1".into(),
            objective: None,
        };
        let d = JevPolicyBroker::compose(&input, &resp).unwrap();
        assert_eq!(d.verdict, PolicyVerdict::Deny);
    }

    #[test]
    fn compose_downgrades_low_confidence_allow() {
        let mut answers = BTreeMap::new();
        answers.insert(
            "verdict".into(),
            DecisionsAnswer::Choice {
                choice: "allow".into(),
                probabilities: BTreeMap::new(),
                confidence: Some(0.2),
            },
        );
        answers.insert("high_risk".into(), DecisionsAnswer::Noul { noul: 0.1 });
        let resp = DecisionsResponse {
            model: "jev-1.13.0".into(),
            answers,
            usage: None,
            id: None,
        };
        let input = PolicyInput {
            tool: "system.frontmost_app".into(),
            args: json!({}),
            device_id: "d1".into(),
            objective: None,
        };
        let d = JevPolicyBroker::compose(&input, &resp).unwrap();
        assert_eq!(d.verdict, PolicyVerdict::Ask);
    }

    #[tokio::test]
    async fn forced_deterministic_allows_accessible_tree() {
        let broker = PolicyBroker::deterministic_forced(PolicyVerdict::Allow);
        let d = broker
            .decide(&PolicyInput {
                tool: "system.accessible_tree".into(),
                args: json!({}),
                device_id: "d1".into(),
                objective: None,
            })
            .await
            .unwrap();
        assert_eq!(d.verdict, PolicyVerdict::Allow);
    }

    fn percentile_us(samples: &[std::time::Duration], quantile: f64) -> u128 {
        let mut micros: Vec<u128> = samples.iter().map(|d| d.as_micros()).collect();
        micros.sort_unstable();
        let last = micros.len() - 1;
        let idx = ((last as f64) * quantile).round() as usize;
        micros[idx]
    }

    fn report_timing(label: &str, samples: &[std::time::Duration]) -> (u128, u128) {
        let p50 = percentile_us(samples, 0.50);
        let p95 = percentile_us(samples, 0.95);
        println!(
            "JEV_GATE_TIMING {label} n={} p50_us={p50} p95_us={p95}",
            samples.len()
        );
        (p50, p95)
    }

    /// Previous `JevPolicyBroker::decide`: always await Decisions, then compose.
    /// Hard deny used to pay this round-trip (or the 30s timeout).
    async fn legacy_decide_always_http(
        broker: &JevPolicyBroker,
        input: &PolicyInput,
    ) -> Result<PolicyDecision> {
        let state = json!({
            "protocol": "aegis-runtime.secure_computer_use",
            "version": 1,
            "tool": input.tool,
            "args": input.args,
            "device_id": input.device_id,
            "objective": input.objective,
        });
        let req = broker.client.policy_request(state);
        let resp = broker.client.decide(req).await?;
        JevPolicyBroker::compose(input, &resp)
    }

    /// Synthetic gate clock.
    ///
    /// Local mock injects 80ms per Decisions call. No live Jev and no chat model:
    /// the numbers compare the allow path (still pays the mock RTT) with hard
    /// deny before and after the pre-HTTP short-circuit.
    #[tokio::test]
    async fn jev_gate_timing_hard_deny_skips_decisions() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};

        const MOCK_RTT: Duration = Duration::from_millis(80);

        let hits = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hit_state = Arc::clone(&hits);
        let app = axum::Router::new()
            .route(
                "/decisions",
                axum::routing::post(
                    |axum::extract::State(hits): axum::extract::State<Arc<AtomicUsize>>,
                     body: String| async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(MOCK_RTT).await;
                        let tool = serde_json::from_str::<serde_json::Value>(&body)
                            .ok()
                            .and_then(|v| v["state"]["tool"].as_str().map(str::to_string))
                            .unwrap_or_default();
                        let choice = if tool == "system.frontmost_app" {
                            "ask"
                        } else {
                            "allow"
                        };
                        axum::Json(json!({
                            "model": "jev-1.13.0",
                            "answers": {
                                "verdict": {
                                    "type": "choice",
                                    "choice": choice,
                                    "probabilities": {"allow": 0.05, "ask": 0.05, "deny": 0.9},
                                    "confidence": 0.92
                                },
                                "high_risk": {"type": "noul", "noul": 0.04},
                                "risk": {
                                    "type": "score",
                                    "score": 0.2,
                                    "legend": {},
                                    "probabilities": {},
                                    "confidence": 0.8
                                }
                            }
                        }))
                    },
                ),
            )
            .with_state(hit_state);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = DecisionsClient::new("sk-or-test-key-1234567890", JevModel::Pinned)
            .unwrap()
            .with_url(format!("http://{addr}/decisions"));
        let jev = JevPolicyBroker::with_client(client);
        let broker = PolicyBroker::jev(JevPolicyBroker::with_client(
            DecisionsClient::new("sk-or-test-key-1234567890", JevModel::Pinned)
                .unwrap()
                .with_url(format!("http://{addr}/decisions")),
        ));

        let mut det_samples = Vec::new();
        let det = DeterministicPolicyBroker::default();
        for _ in 0..200 {
            let started = Instant::now();
            let d = det
                .decide(&PolicyInput {
                    tool: "system.session_probe".into(),
                    args: json!({}),
                    device_id: "d1".into(),
                    objective: None,
                })
                .await
                .unwrap();
            assert_eq!(d.verdict, PolicyVerdict::Allow);
            det_samples.push(started.elapsed());
        }
        let (det_p50, det_p95) = report_timing("deterministic_allow", &det_samples);
        assert!(
            det_p95 < 5_000,
            "deterministic allow p95 {det_p95}us (p50 {det_p50}us) is not a few ms"
        );

        let hard_names = [
            "shell.exec",
            "Shell.Exec",
            "eval",
            "osascript.display",
            "DoShellScript",
            "delete_all",
        ];
        let hits_before_deny = hits.load(Ordering::SeqCst);
        let mut deny_samples = Vec::new();
        for name in hard_names.iter().cycle().take(24) {
            let started = Instant::now();
            let d = jev
                .decide(&PolicyInput {
                    tool: (*name).into(),
                    args: json!({"cmd": "true"}),
                    device_id: "d1".into(),
                    objective: None,
                })
                .await
                .unwrap();
            assert_eq!(d.verdict, PolicyVerdict::Deny, "{name}");
            assert_eq!(d.model, "hard-deny/pre-decisions");
            assert_eq!(d.evidence["skipped_decisions"], json!(true));
            assert_eq!(d.evidence["rule"], json!("hard_deny_tool"));
            deny_samples.push(started.elapsed());
        }
        let hits_after_deny = hits.load(Ordering::SeqCst);
        let (deny_p50, deny_p95) = report_timing("hard_deny_pre_decisions", &deny_samples);
        assert_eq!(
            hits_after_deny, hits_before_deny,
            "hard deny must not call Decisions"
        );
        assert!(
            deny_p95 < 15_000,
            "hard deny p95 {deny_p95}us still looks like a network wait (p50 {deny_p50}us)"
        );

        let legacy_input = PolicyInput {
            tool: "shell.exec".into(),
            args: json!({"cmd": "true"}),
            device_id: "d1".into(),
            objective: None,
        };
        let mut legacy_samples = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            let d = legacy_decide_always_http(&jev, &legacy_input)
                .await
                .unwrap();
            assert_eq!(d.verdict, PolicyVerdict::Deny);
            legacy_samples.push(started.elapsed());
        }
        let (legacy_p50, legacy_p95) =
            report_timing("hard_deny_legacy_pays_mock_rtt", &legacy_samples);
        assert!(
            legacy_p50 >= 70_000,
            "legacy deny p50 {legacy_p50}us did not pay the 80ms mock"
        );
        assert!(legacy_p95 >= legacy_p50);

        let allow_input = PolicyInput {
            tool: "system.session_probe".into(),
            args: json!({"probe": true}),
            device_id: "d1".into(),
            objective: Some("read the frontmost app name".into()),
        };
        let hits_before_allow = hits.load(Ordering::SeqCst);
        let mut allow_samples = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            let d = jev.decide(&allow_input).await.unwrap();
            assert_eq!(d.verdict, PolicyVerdict::Allow);
            assert_ne!(d.model, "hard-deny/pre-decisions");
            allow_samples.push(started.elapsed());
        }
        let (allow_p50, allow_p95) = report_timing("jev_allow_pays_mock_rtt", &allow_samples);
        assert_eq!(
            hits.load(Ordering::SeqCst) - hits_before_allow,
            5,
            "identical allow intents must each call Decisions (no session cache)"
        );
        assert!(
            allow_p50 >= 70_000,
            "allow p50 {allow_p50}us skipped the mock RTT"
        );
        assert!(allow_p95 >= allow_p50);

        let ask = jev
            .decide(&PolicyInput {
                tool: "system.frontmost_app".into(),
                args: json!({}),
                device_id: "d1".into(),
                objective: None,
            })
            .await
            .unwrap();
        assert_eq!(
            ask.verdict,
            PolicyVerdict::Ask,
            "a Jev ask must stay ask; do not hot-allow it"
        );

        let identity = Arc::new(crate::identity::AgentIdentity::generate());
        let pipeline =
            crate::computer_use::SecureCuPipeline::loopback(identity, broker, "dev-loop")
                .with_enabled(true);
        let hits_before_pipe = hits.load(Ordering::SeqCst);
        let started = Instant::now();
        let err = pipeline
            .run("shell.exec", json!({"cmd": "true"}), None)
            .await
            .expect_err("hard deny");
        let pipe_us = started.elapsed().as_micros();
        println!("JEV_GATE_TIMING pipeline_shell_exec_deny_us={pipe_us}");
        assert!(matches!(
            err,
            crate::computer_use::PipelineError::PolicyDenied(_)
        ));
        assert!(
            pipe_us < 40_000,
            "pipeline deny waited {pipe_us}us on Decisions"
        );
        assert_eq!(hits.load(Ordering::SeqCst), hits_before_pipe);
        let events: Vec<_> = pipeline
            .audit
            .lock()
            .records()
            .iter()
            .map(|r| r.event.clone())
            .collect();
        assert!(events.contains(&crate::computer_use::AuditEvent::PolicyDecided));
        assert!(
            !events.contains(&crate::computer_use::AuditEvent::IntentSigned),
            "deny must not sign"
        );

        server.abort();
    }
}
