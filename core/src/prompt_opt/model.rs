use serde::{Deserialize, Serialize};

use crate::llm::LlmProviderErrorKind;

#[derive(Debug, thiserror::Error)]
pub enum PromptOptError {
    #[error("{kind:?}: {message}")]
    Routing {
        kind: LlmProviderErrorKind,
        message: String,
    },
    #[error("fail closed: {message}")]
    FailClosed {
        kind: LlmProviderErrorKind,
        message: String,
    },
    #[error("invalid prompt optimization input: {message}")]
    Invalid { message: String },
}

impl PromptOptError {
    pub fn provider_kind(&self) -> Option<LlmProviderErrorKind> {
        match self {
            Self::Routing { kind, .. } | Self::FailClosed { kind, .. } => Some(*kind),
            Self::Invalid { .. } => None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            message: message.into(),
        }
    }
}

impl From<crate::llm::LlmProviderError> for PromptOptError {
    fn from(err: crate::llm::LlmProviderError) -> Self {
        Self::Routing {
            kind: err.kind,
            message: err.message,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptPhase {
    Propose,
    Evaluate,
    /// Reads the trajectory digest and names what failed the rubric.
    FailureAnalyst,
    /// Reads the same digest and names what to keep.
    SuccessAnalyst,
    /// Compresses an instruction that exceeded the length threshold.
    Distill,
}

impl OptPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Propose => "propose",
            Self::Evaluate => "evaluate",
            Self::FailureAnalyst => "failure_analyst",
            Self::SuccessAnalyst => "success_analyst",
            Self::Distill => "distill",
        }
    }
}

/// How a round is allowed to start. There is no boot or heartbeat variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationTrigger {
    /// A person or an external scheduler asked for this round.
    Explicit,
}

impl OptimizationTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
        }
    }
}

/// Per-round limits that are not the call/cost/token budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunPolicy {
    pub trigger: OptimizationTrigger,
    /// Maximum SEARCH/REPLACE blocks accepted in one proposal. Default 3.
    pub max_patches_per_round: u32,
    /// When the validation winner is longer than this, run distillation.
    pub distill_when_chars_above: usize,
}

pub const DEFAULT_MAX_PATCHES_PER_ROUND: u32 = 3;
pub const DEFAULT_DISTILL_CHARS: usize = 800;
pub const DEFAULT_TOKENS_PER_CALL: u64 = 1_200;
pub const DEFAULT_MAX_TOKENS: u64 = 20_000;

impl RunPolicy {
    pub fn explicit() -> Self {
        Self {
            trigger: OptimizationTrigger::Explicit,
            max_patches_per_round: DEFAULT_MAX_PATCHES_PER_ROUND,
            distill_when_chars_above: DEFAULT_DISTILL_CHARS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RubricDimension {
    pub id: String,
    pub label: String,
    pub weight: f64,
    pub guidance: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rubric {
    /// Owner-facing name of the taste this rubric encodes.
    pub taste: String,
    pub dimensions: Vec<RubricDimension>,
}

impl Rubric {
    pub fn weight_sum(&self) -> f64 {
        self.dimensions
            .iter()
            .map(|dimension| dimension.weight)
            .sum()
    }

    /// Nudge one dimension after an owner A/B judgment, then renormalize.
    ///
    /// A positive `delta` means the owner preferred the side that expressed
    /// this dimension more strongly.
    pub fn calibrate_pairwise(&self, judgment: &PairwiseJudgment) -> Result<Self, PromptOptError> {
        if !judgment.delta.is_finite() {
            return Err(PromptOptError::invalid(
                "pairwise weight delta must be a finite number",
            ));
        }
        let mut next = self.clone();
        let Some(dimension) = next
            .dimensions
            .iter_mut()
            .find(|dimension| dimension.id == judgment.dimension_id)
        else {
            return Err(PromptOptError::invalid(format!(
                "unknown rubric dimension {}",
                judgment.dimension_id
            )));
        };
        dimension.weight = (dimension.weight + judgment.delta).max(0.0);
        let sum = next.weight_sum();
        if sum <= f64::EPSILON {
            return Err(PromptOptError::invalid(
                "pairwise calibration removed every rubric weight",
            ));
        }
        for dimension in &mut next.dimensions {
            dimension.weight /= sum;
        }
        Ok(next)
    }

    pub fn weighted_score(&self, scores: &[(String, f64)]) -> Option<f64> {
        let mut numerator = 0.0;
        let mut denominator = 0.0;
        for dimension in &self.dimensions {
            if let Some((_, score)) = scores.iter().find(|(id, _)| id == &dimension.id) {
                numerator += dimension.weight * score.clamp(0.0, 1.0);
                denominator += dimension.weight;
            }
        }
        if denominator <= f64::EPSILON {
            None
        } else {
            Some(numerator / denominator)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PairwiseJudgment {
    pub dimension_id: String,
    pub delta: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Example {
    pub id: String,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationTask {
    pub name: String,
    pub io_description: String,
    pub baseline_instruction: String,
    pub rubric: Rubric,
    pub examples: Vec<Example>,
    /// Fraction of examples held out for evaluation. The split is deterministic.
    pub validation_ratio: f64,
}

fn default_max_tokens() -> u64 {
    DEFAULT_MAX_TOKENS
}

fn default_tokens_per_call() -> u64 {
    DEFAULT_TOKENS_PER_CALL
}

/// Hard budget. The loop charges one call before sending it and stops when the
/// next call would exceed `max_calls`, `max_cost_micros`, or `max_tokens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub max_calls: u32,
    pub max_cost_micros: u64,
    pub cost_per_call_micros: u64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,
    #[serde(default = "default_tokens_per_call")]
    pub tokens_per_call: u64,
    pub calls_used: u32,
    pub cost_used_micros: u64,
    #[serde(default)]
    pub tokens_used: u64,
}

impl Budget {
    pub fn start_run(self) -> Self {
        Self {
            calls_used: 0,
            cost_used_micros: 0,
            tokens_used: 0,
            ..self
        }
    }

    pub fn try_charge(&mut self) -> Result<(), BudgetStop> {
        if self.calls_used >= self.max_calls {
            return Err(BudgetStop::Calls);
        }
        if self
            .cost_used_micros
            .saturating_add(self.cost_per_call_micros)
            > self.max_cost_micros
        {
            return Err(BudgetStop::Cost);
        }
        if self.tokens_used.saturating_add(self.tokens_per_call) > self.max_tokens {
            return Err(BudgetStop::Tokens);
        }
        self.calls_used += 1;
        self.cost_used_micros += self.cost_per_call_micros;
        self.tokens_used += self.tokens_per_call;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStop {
    Calls,
    Cost,
    Tokens,
}

pub fn split_train_validation(
    examples: &[Example],
    validation_ratio: f64,
) -> (Vec<Example>, Vec<Example>) {
    if examples.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if examples.len() == 1 {
        return (examples.to_vec(), examples.to_vec());
    }
    let ratio = if validation_ratio.is_finite() {
        validation_ratio.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let mut validation_count = ((examples.len() as f64) * ratio).ceil() as usize;
    validation_count = validation_count.clamp(1, examples.len() - 1);
    let split_at = examples.len() - validation_count;
    (examples[..split_at].to_vec(), examples[split_at..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rubric() -> Rubric {
        Rubric {
            taste: "有生气但不乱".to_string(),
            dimensions: vec![
                RubricDimension {
                    id: "vitality".to_string(),
                    label: "有生气".to_string(),
                    weight: 0.5,
                    guidance: "alive".to_string(),
                },
                RubricDimension {
                    id: "not_chaotic".to_string(),
                    label: "不乱".to_string(),
                    weight: 0.5,
                    guidance: "one scene".to_string(),
                },
            ],
        }
    }

    #[test]
    fn pairwise_calibration_renormalizes_weights() {
        let updated = rubric()
            .calibrate_pairwise(&PairwiseJudgment {
                dimension_id: "vitality".to_string(),
                delta: 0.25,
            })
            .unwrap();
        let vitality = updated
            .dimensions
            .iter()
            .find(|dimension| dimension.id == "vitality")
            .unwrap();
        let coherence = updated
            .dimensions
            .iter()
            .find(|dimension| dimension.id == "not_chaotic")
            .unwrap();
        assert!(vitality.weight > coherence.weight);
        assert!((updated.weight_sum() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn budget_stops_before_the_call_that_would_exceed_cost() {
        let mut budget = Budget {
            max_calls: 10,
            max_cost_micros: 10,
            cost_per_call_micros: 10,
            max_tokens: DEFAULT_MAX_TOKENS,
            tokens_per_call: 1,
            calls_used: 0,
            cost_used_micros: 0,
            tokens_used: 0,
        };
        budget.try_charge().unwrap();
        assert_eq!(budget.try_charge(), Err(BudgetStop::Cost));
        assert_eq!(budget.calls_used, 1);
    }

    #[test]
    fn budget_stops_before_the_call_that_would_exceed_tokens() {
        let mut budget = Budget {
            max_calls: 10,
            max_cost_micros: 1_000,
            cost_per_call_micros: 1,
            max_tokens: 100,
            tokens_per_call: 100,
            calls_used: 0,
            cost_used_micros: 0,
            tokens_used: 0,
        };
        budget.try_charge().unwrap();
        assert_eq!(budget.try_charge(), Err(BudgetStop::Tokens));
        assert_eq!(budget.calls_used, 1);
        assert_eq!(budget.tokens_used, 100);
    }

    #[test]
    fn explicit_policy_caps_patches_at_three() {
        let policy = RunPolicy::explicit();
        assert_eq!(policy.trigger, OptimizationTrigger::Explicit);
        assert_eq!(policy.max_patches_per_round, 3);
        assert_eq!(policy.trigger.as_str(), "explicit");
    }
}
