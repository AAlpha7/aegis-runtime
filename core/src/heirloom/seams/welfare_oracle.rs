//! W7 — WelfareOracle trait + NoopWelfareOracle dev impl.
//!
//! Spec §5 Q5.6. Evaluates marginal welfare impact of a proposed action on
//! the owner. Dev impl returns zero delta; prod impl (`TimeSeriesWelfareOracle`)
//! trains on historical `WelfareObservation` events.

use serde::{Deserialize, Serialize};

use crate::heirloom::types::{Did, WitnessChannel};

use super::types::EventHash;

// ── Value types ───────────────────────────────────────────────────────────

/// Numeric welfare verdict. Range is normative — impls MUST clamp to [-1, +1]
/// and MUST report confidence in [0, 1].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WelfareScore {
    /// Expected change in owner welfare, -1.0 (maximally harmful) to +1.0
    /// (maximally beneficial). Exactly 0.0 means "no marginal impact".
    pub expected_delta: f32,
    /// Impl confidence in its own estimate, 0.0 (no signal) to 1.0 (certain).
    pub confidence: f32,
    /// If the estimate came from an LLM reasoning chain, point at the event
    /// that recorded it. `None` for rule-based or no-signal impls.
    pub reasoning_ref: Option<EventHash>,
    /// Prior observations the estimate relied on. Empty for noop impl.
    pub observational_basis: Vec<EventHash>,
}

impl WelfareScore {
    /// Welfare verdict meaning "no marginal impact either way". NoopOracle
    /// returns this for every call.
    pub const fn neutral() -> Self {
        Self {
            expected_delta: 0.0,
            confidence: 0.0,
            reasoning_ref: None,
            observational_basis: Vec::new(),
        }
    }
}

/// A single welfare indicator the oracle observed. Free-form category + value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WelfareIndicator {
    /// Kind tag — `"mood"`, `"sleep"`, `"goal_progress"`, `"physical_health"`.
    pub kind: String,
    /// Numeric reading in [-1.0, +1.0], directionally consistent (higher = better).
    pub reading: f32,
    /// Free-form note explaining why this reading was assigned.
    pub note: String,
}

/// A welfare observation record — produced by the AI whenever it notices a
/// mood/state shift in the owner. Submitted via `WelfareOracle::submit_observation`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WelfareObservation {
    pub target_did: Did,
    pub observed_channel: WitnessChannel,
    pub indicators: Vec<WelfareIndicator>,
    /// If the AI flagged the observation as anomalous, the score in [0, 1].
    /// Higher = more anomalous. `None` when not computed.
    pub anomaly_score: Option<f32>,
    /// Wall-clock instant (ISO-8601 string for forward compat) — separate
    /// from the envelope HLC the event log will assign.
    pub observed_at: String,
}

// ── Trait ─────────────────────────────────────────────────────────────────

/// Evaluates welfare impact of AI actions on the owner. Spec §5 Q5.6.
pub trait WelfareOracle: Send + Sync {
    /// Estimate the marginal welfare delta of a proposed action on
    /// `target_did`. Called by the W10 writer before every proactive AI
    /// action when `OwnerState == Alive`.
    fn marginal_welfare(
        &self,
        action: &crate::heirloom::types::ProposedAction,
        target_did: &Did,
    ) -> WelfareScore;

    /// Record a spontaneous welfare observation (the AI noticed something).
    /// Returns a placeholder hash in dev; in Phase C the controller will
    /// append a real `ai:welfare_observation` event and return that hash.
    fn submit_observation(&self, obs: WelfareObservation) -> Result<EventHash, WelfareError>;
}

/// Errors from a WelfareOracle. Kept minimal — Phase C adds more.
#[derive(Debug, thiserror::Error)]
pub enum WelfareError {
    #[error("welfare oracle refused observation: {0}")]
    Refused(String),
}

// ── NoopWelfareOracle ─────────────────────────────────────────────────────

/// Dev impl that never blocks anything. Returns `WelfareScore::neutral()`
/// for every call. Observations get a content-address placeholder hash
/// (not a real event log write — W10 does that in Phase C).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopWelfareOracle;

impl NoopWelfareOracle {
    pub const fn new() -> Self {
        Self
    }
}

impl WelfareOracle for NoopWelfareOracle {
    fn marginal_welfare(
        &self,
        _action: &crate::heirloom::types::ProposedAction,
        _target_did: &Did,
    ) -> WelfareScore {
        WelfareScore::neutral()
    }

    fn submit_observation(&self, obs: WelfareObservation) -> Result<EventHash, WelfareError> {
        // Content-address the observation so tests can see determinism.
        // W10 will replace this with a true `ai:welfare_observation` event
        // hash in Phase C.
        let bytes = serde_json::to_vec(&obs)
            .map_err(|e| WelfareError::Refused(format!("serialize: {e}")))?;
        let digest = *blake3::hash(&bytes).as_bytes();
        Ok(EventHash::from_bytes(digest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::types::{Did, ProposedAction, WitnessChannel};

    fn action(event_type: &str, target: Option<&str>) -> ProposedAction {
        ProposedAction {
            event_type: event_type.to_string(),
            author: Did::new("did:aegis:self"),
            target: target.map(Did::new),
            summary: "test".to_string(),
            payload_digest: [0u8; 32],
        }
    }

    #[test]
    fn t7_welfare_score_neutral_is_zero_delta_zero_confidence() {
        let s = WelfareScore::neutral();
        assert_eq!(s.expected_delta, 0.0);
        assert_eq!(s.confidence, 0.0);
        assert!(s.reasoning_ref.is_none());
        assert!(s.observational_basis.is_empty());
    }

    #[test]
    fn t7_noop_marginal_welfare_returns_neutral_for_any_action() {
        let oracle = NoopWelfareOracle::new();
        let target = Did::new("did:aegis:telegram:4242");
        let a = action("ai:message_sent", Some("did:aegis:telegram:4242"));
        let s = oracle.marginal_welfare(&a, &target);
        assert_eq!(s, WelfareScore::neutral());
    }

    #[test]
    fn t7_noop_marginal_welfare_is_stable_across_many_calls() {
        let oracle = NoopWelfareOracle::new();
        let target = Did::new("did:aegis:telegram:4242");
        let a = action("ai:tool_call", None);
        let first = oracle.marginal_welfare(&a, &target);
        for _ in 0..100 {
            let s = oracle.marginal_welfare(&a, &target);
            assert_eq!(s, first);
        }
    }

    #[test]
    fn t7_noop_marginal_welfare_insensitive_to_action_event_type() {
        let oracle = NoopWelfareOracle::new();
        let target = Did::new("did:aegis:telegram:4242");
        let s1 = oracle.marginal_welfare(&action("ai:risky_action", None), &target);
        let s2 = oracle.marginal_welfare(&action("ai:safe_action", None), &target);
        assert_eq!(s1, s2);
    }

    #[test]
    fn t7_noop_submit_observation_returns_deterministic_placeholder_hash() {
        let oracle = NoopWelfareOracle::new();
        let obs = WelfareObservation {
            target_did: Did::new("did:aegis:telegram:4242"),
            observed_channel: WitnessChannel::Telegram,
            indicators: vec![WelfareIndicator {
                kind: "mood".to_string(),
                reading: -0.3,
                note: "irritable".to_string(),
            }],
            anomaly_score: Some(0.6),
            observed_at: "2026-04-17T20:30:00Z".to_string(),
        };
        let h1 = oracle.submit_observation(obs.clone()).unwrap();
        let h2 = oracle.submit_observation(obs).unwrap();
        assert_eq!(
            h1, h2,
            "dev impl must be deterministic so tests are reproducible",
        );
    }

    #[test]
    fn t7_noop_submit_observation_distinct_observations_yield_distinct_hashes() {
        let oracle = NoopWelfareOracle::new();
        let base = WelfareObservation {
            target_did: Did::new("did:aegis:telegram:4242"),
            observed_channel: WitnessChannel::Telegram,
            indicators: vec![],
            anomaly_score: None,
            observed_at: "2026-04-17T20:30:00Z".to_string(),
        };
        let mut other = base.clone();
        other.observed_at = "2026-04-17T20:31:00Z".to_string();
        let h1 = oracle.submit_observation(base).unwrap();
        let h2 = oracle.submit_observation(other).unwrap();
        assert_ne!(h1, h2, "different observations must hash differently");
    }
}
