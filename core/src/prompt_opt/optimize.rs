use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::healing::HEALING_TASK_NAME;
use super::llm::{PromptOptChat, PromptOptRequest};
use super::model::{
    Budget, BudgetStop, Example, OptPhase, OptimizationTask, PromptOptError, RunPolicy,
    split_train_validation,
};
use super::registry::{EvolutionSignalKind, PromptRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    BudgetCalls,
    BudgetCost,
    BudgetTokens,
    CompletedRounds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoundTrace {
    pub round: u32,
    pub phases: Vec<OptPhase>,
    pub proposal: Option<String>,
    pub aggregate_score: Option<f64>,
    pub dimension_scores: Vec<(String, f64)>,
    pub reflection: Option<String>,
    pub version_id: Option<u64>,
    /// True when this round's score replaced the validation winner.
    /// The live pointer does not move.
    #[serde(default)]
    pub validation_promoted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstructionComparison {
    pub before: String,
    pub after: String,
    pub before_version_id: u64,
    pub after_version_id: u64,
    pub after_score: Option<f64>,
    pub notes: String,
    /// Unified before/after text. The round stops at this proposal.
    pub diff: String,
    /// True only after the owner accepts the after-version.
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationReport {
    pub registry: PromptRegistry,
    pub rounds: Vec<RoundTrace>,
    pub stop: StopReason,
    pub train_ids: Vec<String>,
    pub validation_ids: Vec<String>,
    pub pareto_front: Vec<u64>,
    pub comparison: InstructionComparison,
    pub calls_used: u32,
    pub cost_used_micros: u64,
    pub tokens_used: u64,
    /// Always `explicit`. Rounds are not started from boot or heartbeat.
    pub trigger: String,
}

pub async fn run_optimization(
    task: &OptimizationTask,
    mut registry: PromptRegistry,
    chat: &dyn PromptOptChat,
    mut budget: Budget,
    max_rounds: u32,
    policy: RunPolicy,
) -> Result<OptimizationReport, PromptOptError> {
    if task.name != HEALING_TASK_NAME {
        return Err(PromptOptError::invalid(
            "pilot scope is the healing-style image prompt only",
        ));
    }
    let (train, validation) = split_train_validation(&task.examples, task.validation_ratio);
    let mut rounds = Vec::new();
    let mut stop = StopReason::CompletedRounds;

    for round in 1..=max_rounds {
        let mut trace = RoundTrace {
            round,
            phases: Vec::new(),
            proposal: None,
            aggregate_score: None,
            dimension_scores: Vec::new(),
            reflection: None,
            version_id: None,
            validation_promoted: false,
        };
        let parent_id = registry.validation_best_id();
        let current = registry.validation_best().instruction.clone();

        let proposal_text = match charge_and_complete(
            chat,
            &mut budget,
            OptPhase::Propose,
            &propose_system(policy.max_patches_per_round),
            &propose_user(task, &train, &current),
        )
        .await?
        {
            Control::Stop(reason) => {
                stop = reason;
                rounds.push(trace);
                break;
            }
            Control::Ok(text) => text,
        };
        trace.phases.push(OptPhase::Propose);
        let instruction = resolve_proposal(&proposal_text, &current, policy.max_patches_per_round)?;
        trace.proposal = Some(instruction.clone());

        let evaluation_text = match charge_and_complete(
            chat,
            &mut budget,
            OptPhase::Evaluate,
            &evaluate_system(),
            &evaluate_user(task, &validation, &instruction),
        )
        .await?
        {
            Control::Stop(reason) => {
                stop = reason;
                rounds.push(trace);
                break;
            }
            Control::Ok(text) => text,
        };
        trace.phases.push(OptPhase::Evaluate);
        let (score, dimension_scores, notes) = parse_evaluation(&evaluation_text, task);
        trace.aggregate_score = score.or_else(|| task.rubric.weighted_score(&dimension_scores));
        trace.dimension_scores = dimension_scores.clone();
        let version_id = registry.push_candidate(
            instruction,
            Some(parent_id),
            trace.aggregate_score,
            dimension_scores,
            notes.clone(),
        );
        trace.version_id = Some(version_id);
        trace.validation_promoted = registry.promote_if_strictly_better(version_id)?;
        if !trace.validation_promoted {
            registry.record_signal(
                EvolutionSignalKind::ValidationRejected,
                version_id,
                "held-out score was not strictly better than the current best",
            );
        }
        let digest = trajectory_digest(&trace, &notes);

        let failure_text = match charge_and_complete(
            chat,
            &mut budget,
            OptPhase::FailureAnalyst,
            &failure_system(),
            &analyst_user(task, &digest, &registry.ledger_prompt()),
        )
        .await?
        {
            Control::Stop(reason) => {
                stop = reason;
                rounds.push(trace);
                break;
            }
            Control::Ok(text) => text,
        };
        trace.phases.push(OptPhase::FailureAnalyst);
        let failure = parse_labeled_block(&failure_text, "FAILURE");

        let success_text = match charge_and_complete(
            chat,
            &mut budget,
            OptPhase::SuccessAnalyst,
            &success_system(),
            &analyst_user(task, &digest, &registry.ledger_prompt()),
        )
        .await?
        {
            Control::Stop(reason) => {
                stop = reason;
                rounds.push(trace);
                break;
            }
            Control::Ok(text) => text,
        };
        trace.phases.push(OptPhase::SuccessAnalyst);
        let success = parse_labeled_block(&success_text, "SUCCESS");
        let reflection = combine_reflection(&failure, &success, trace.validation_promoted);
        registry.annotate(version_id, reflection.clone())?;
        trace.reflection = Some(reflection);
        if let Some(reason) = maybe_distill(
            task,
            &mut registry,
            chat,
            &mut budget,
            &mut trace,
            &validation,
            policy,
        )
        .await?
        {
            stop = reason;
            rounds.push(trace);
            break;
        }
        rounds.push(trace);
    }

    Ok(finish(
        registry,
        rounds,
        stop,
        &train,
        &validation,
        &budget,
        policy,
    ))
}

enum Control {
    Stop(StopReason),
    Ok(String),
}

async fn charge_and_complete(
    chat: &dyn PromptOptChat,
    budget: &mut Budget,
    phase: OptPhase,
    system: &str,
    user: &str,
) -> Result<Control, PromptOptError> {
    match budget.try_charge() {
        Err(BudgetStop::Calls) => return Ok(Control::Stop(StopReason::BudgetCalls)),
        Err(BudgetStop::Cost) => return Ok(Control::Stop(StopReason::BudgetCost)),
        Err(BudgetStop::Tokens) => return Ok(Control::Stop(StopReason::BudgetTokens)),
        Ok(()) => {}
    }
    let text = chat
        .complete(PromptOptRequest {
            phase,
            system: system.to_string(),
            user: user.to_string(),
        })
        .await?;
    Ok(Control::Ok(text))
}

fn finish(
    registry: PromptRegistry,
    rounds: Vec<RoundTrace>,
    stop: StopReason,
    train: &[Example],
    validation: &[Example],
    budget: &Budget,
    policy: RunPolicy,
) -> OptimizationReport {
    let before = registry.baseline().instruction.clone();
    let before_id = registry.baseline_id();
    let after_id = registry.validation_best_id();
    let after_score = registry.validation_best().score;
    let after_text = registry.validation_best().instruction.clone();
    let notes = rounds
        .iter()
        .rev()
        .find_map(|round| round.reflection.clone())
        .unwrap_or_default();
    let diff = proposal_diff(&before, &after_text);
    let accepted = registry.live_id() == after_id
        && registry.get(after_id).is_some_and(|v| v.accepted)
        && after_id != before_id;
    let pareto_front = pareto_front_ids(&registry);
    OptimizationReport {
        registry,
        rounds,
        stop,
        train_ids: train.iter().map(|example| example.id.clone()).collect(),
        validation_ids: validation
            .iter()
            .map(|example| example.id.clone())
            .collect(),
        pareto_front,
        comparison: InstructionComparison {
            before,
            after: after_text,
            before_version_id: before_id,
            after_version_id: after_id,
            after_score,
            notes,
            diff,
            accepted,
        },
        calls_used: budget.calls_used,
        cost_used_micros: budget.cost_used_micros,
        tokens_used: budget.tokens_used,
        trigger: policy.trigger.as_str().to_string(),
    }
}

pub fn proposal_diff(before: &str, after: &str) -> String {
    if before == after {
        "(no change)\n".to_string()
    } else {
        format!("--- current\n{before}\n+++ proposal\n{after}\n")
    }
}

pub fn pareto_front(candidates: &[Vec<(String, f64)>]) -> Vec<usize> {
    let mut front = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let dominated = candidates
            .iter()
            .enumerate()
            .any(|(other_index, other)| other_index != index && dominates(other, candidate));
        if !dominated && !candidate.is_empty() {
            front.push(index);
        }
    }
    front
}

fn pareto_front_ids(registry: &PromptRegistry) -> Vec<u64> {
    let scored: Vec<&super::registry::PromptVersion> = registry
        .versions()
        .iter()
        .filter(|version| !version.dimension_scores.is_empty())
        .collect();
    let vectors: Vec<Vec<(String, f64)>> = scored
        .iter()
        .map(|version| version.dimension_scores.clone())
        .collect();
    pareto_front(&vectors)
        .into_iter()
        .map(|index| scored[index].id)
        .collect()
}

fn dominates(better: &[(String, f64)], worse: &[(String, f64)]) -> bool {
    if better.is_empty() || worse.is_empty() {
        return false;
    }
    let mut strictly = false;
    for (name, worse_score) in worse {
        let Some((_, better_score)) = better.iter().find(|(id, _)| id == name) else {
            return false;
        };
        if better_score + 1e-9 < *worse_score {
            return false;
        }
        if *better_score > worse_score + 1e-9 {
            strictly = true;
        }
    }
    strictly
}

fn propose_system(max_patches: u32) -> String {
    format!(
        "You improve a task instruction. Reply with exactly one of these blocks:\nINSTRUCTION:\n<the full replacement instruction>\nor up to {max_patches} PATCH blocks:\nPATCH:\n<<<< SEARCH\n<exact text from the current best instruction>\n====\n<replacement text>\n>>>>\nThe instruction is link text only. Do not mention intents, envelopes, or sandboxes."
    )
}

fn propose_user(task: &OptimizationTask, train: &[Example], current: &str) -> String {
    format!(
        "Task: {name}\nInput/output: {io}\nTaste: {taste}\nRubric:\n{rubric}\nTrain examples:\n{train}\nCurrent best instruction:\n{current}",
        name = task.name,
        io = task.io_description,
        taste = task.rubric.taste,
        rubric = format_rubric(task),
        train = format_examples(train),
        current = current,
    )
}

fn evaluate_system() -> String {
    "Score the candidate instruction against the rubric. Reply with JSON only: {\"dimensions\":{\"<dimension id>\":0.0},\"notes\":\"...\"}. Scores are from 0 to 1.".to_string()
}

fn evaluate_user(task: &OptimizationTask, validation: &[Example], instruction: &str) -> String {
    format!(
        "Taste: {taste}\nRubric:\n{rubric}\nValidation examples:\n{validation}\nCandidate instruction:\n{instruction}",
        taste = task.rubric.taste,
        rubric = format_rubric(task),
        validation = format_examples(validation),
        instruction = instruction,
    )
}

fn failure_system() -> String {
    "You are the failure analyst. Read the trajectory digest and name what failed the rubric. Reply with:\nFAILURE:\n<short notes the owner can read>".to_string()
}

fn success_system() -> String {
    "You are the success analyst. Read the same trajectory digest and name what to keep. Reply with:\nSUCCESS:\n<short notes the owner can read>".to_string()
}

fn analyst_user(task: &OptimizationTask, digest: &str, ledger: &str) -> String {
    format!(
        "Taste: {taste}\nRubric:\n{rubric}\nTrajectory digest:\n{digest}\n{ledger}",
        taste = task.rubric.taste,
        rubric = format_rubric(task),
        digest = digest,
        ledger = ledger,
    )
}

fn distill_system() -> String {
    "Compress the instruction. Keep the same meaning in fewer words. Reply with:\nINSTRUCTION:\n<shorter instruction>\nDo not mention intents, envelopes, or sandboxes.".to_string()
}

fn distill_user(current: &str, ledger: &str) -> String {
    format!("Current best instruction:\n{current}\n{ledger}")
}

async fn maybe_distill(
    task: &OptimizationTask,
    registry: &mut PromptRegistry,
    chat: &dyn PromptOptChat,
    budget: &mut Budget,
    trace: &mut RoundTrace,
    validation: &[Example],
    policy: RunPolicy,
) -> Result<Option<StopReason>, PromptOptError> {
    let current = registry.validation_best().instruction.clone();
    if current.chars().count() <= policy.distill_when_chars_above {
        return Ok(None);
    }
    let parent_id = registry.validation_best_id();
    let best_score = registry.validation_best().score;
    let distill_text = match charge_and_complete(
        chat,
        budget,
        OptPhase::Distill,
        &distill_system(),
        &distill_user(&current, &registry.ledger_prompt()),
    )
    .await?
    {
        Control::Stop(reason) => return Ok(Some(reason)),
        Control::Ok(text) => text,
    };
    trace.phases.push(OptPhase::Distill);
    let instruction = resolve_proposal(&distill_text, &current, policy.max_patches_per_round)?;
    if instruction.trim() == current.trim() {
        return Ok(None);
    }
    let evaluation_text = match charge_and_complete(
        chat,
        budget,
        OptPhase::Evaluate,
        &evaluate_system(),
        &evaluate_user(task, validation, &instruction),
    )
    .await?
    {
        Control::Stop(reason) => return Ok(Some(reason)),
        Control::Ok(text) => text,
    };
    trace.phases.push(OptPhase::Evaluate);
    let (score, dimension_scores, notes) = parse_evaluation(&evaluation_text, task);
    let aggregate = score.or_else(|| task.rubric.weighted_score(&dimension_scores));
    let version_id = registry.push_candidate(
        instruction,
        Some(parent_id),
        aggregate,
        dimension_scores,
        notes,
    );
    if !score_is_not_worse(aggregate, best_score) {
        registry.record_signal(
            EvolutionSignalKind::DistillRejected,
            version_id,
            "distill held-out score is worse than the current best",
        );
        registry.annotate(version_id, "distill held-out gate: worse than current")?;
        return Ok(None);
    }
    let promoted = registry.promote_if_strictly_better(version_id)?;
    if !promoted {
        registry.annotate(
            version_id,
            "distill held-out gate: not worse; awaiting human accept",
        )?;
    }
    Ok(None)
}

fn score_is_not_worse(candidate: Option<f64>, best: Option<f64>) -> bool {
    match (candidate, best) {
        (Some(candidate), Some(best)) => candidate + 1e-9 >= best,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

fn format_rubric(task: &OptimizationTask) -> String {
    task.rubric
        .dimensions
        .iter()
        .map(|dimension| {
            format!(
                "- {id} ({label}, weight {weight}): {guidance}",
                id = dimension.id,
                label = dimension.label,
                weight = dimension.weight,
                guidance = dimension.guidance,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_examples(examples: &[Example]) -> String {
    if examples.is_empty() {
        return "(none)".to_string();
    }
    examples
        .iter()
        .map(|example| format!("- {id}: {input}", id = example.id, input = example.input))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn parse_instruction(text: &str) -> String {
    let body = if let Some((_, rest)) = text.split_once("INSTRUCTION:") {
        rest
    } else {
        text
    };
    body.lines()
        .take_while(|line| !is_block_label(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

pub fn resolve_proposal(
    text: &str,
    current: &str,
    max_patches: u32,
) -> Result<String, PromptOptError> {
    let instruction = if text.contains("PATCH:") {
        apply_instruction_patches(current, text, max_patches)?
    } else {
        parse_instruction(text)
    };
    if instruction.trim().is_empty() {
        return Err(PromptOptError::invalid(
            "propose step returned an empty instruction",
        ));
    }
    Ok(instruction)
}

pub fn apply_instruction_patches(
    current: &str,
    text: &str,
    max_patches: u32,
) -> Result<String, PromptOptError> {
    let parts: Vec<&str> = text.split("PATCH:").skip(1).collect();
    if parts.is_empty() {
        return Err(PromptOptError::invalid("patch is missing PATCH:"));
    }
    if parts.len() as u32 > max_patches {
        return Err(PromptOptError::invalid(format!(
            "edit budget exceeded: {} patches in one round, max {max_patches}",
            parts.len()
        )));
    }
    let mut cursor = current.to_string();
    for part in parts {
        let block = format!("PATCH:{part}");
        cursor = apply_instruction_patch(&cursor, &block)?;
    }
    Ok(cursor)
}

/// Apply one SEARCH/REPLACE span to `current`. The search text must occur
/// exactly once in the sense of the first match; later copies stay put.
pub fn apply_instruction_patch(current: &str, text: &str) -> Result<String, PromptOptError> {
    let body = match text.split_once("PATCH:") {
        Some((_, rest)) => rest,
        None => text,
    };
    let Some((_, after_marker)) = body.split_once("<<<< SEARCH") else {
        return Err(PromptOptError::invalid("patch is missing <<<< SEARCH"));
    };
    let Some((search_raw, after_search)) = after_marker.split_once("====") else {
        return Err(PromptOptError::invalid("patch is missing ===="));
    };
    let Some((replacement_raw, _)) = after_search.split_once(">>>>") else {
        return Err(PromptOptError::invalid("patch is missing >>>>"));
    };
    let search = unwrap_patch_span(search_raw);
    let replacement = unwrap_patch_span(replacement_raw);
    if search.is_empty() {
        return Err(PromptOptError::invalid("patch search text is empty"));
    }
    if !current.contains(&search) {
        return Err(PromptOptError::invalid("patch search text was not found"));
    }
    let updated = current.replacen(&search, &replacement, 1);
    if updated.trim().is_empty() {
        return Err(PromptOptError::invalid(
            "patch produced an empty instruction",
        ));
    }
    Ok(updated)
}

fn unwrap_patch_span(text: &str) -> String {
    let text = text.strip_prefix('\n').unwrap_or(text);
    text.strip_suffix('\n').unwrap_or(text).to_string()
}

/// Deterministic summary of one scored proposal. This is not a model call.
pub fn trajectory_digest(trace: &RoundTrace, notes: &str) -> String {
    let dimensions = if trace.dimension_scores.is_empty() {
        "none".to_string()
    } else {
        trace
            .dimension_scores
            .iter()
            .map(|(id, score)| format!("{id}={score}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let score = trace
        .aggregate_score
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unscored".to_string());
    format!(
        "round: {round}\nproposal:\n{proposal}\nvalidation_score: {score}\ndimensions: {dimensions}\nnotes: {notes}",
        round = trace.round,
        proposal = trace.proposal.as_deref().unwrap_or(""),
        notes = notes.trim(),
    )
}

fn combine_reflection(failure: &str, success: &str, promoted: bool) -> String {
    let mut notes = format!("FAILURE:\n{failure}\nSUCCESS:\n{success}");
    if !promoted {
        notes.push_str("\nvalidation gate: not strictly better than the current best");
    }
    notes
}

fn is_block_label(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("REFLECTION:")
        || trimmed.starts_with("FAILURE:")
        || trimmed.starts_with("SUCCESS:")
        || trimmed.starts_with("PATCH:")
        || trimmed.starts_with("INSTRUCTION:")
}

pub fn parse_evaluation(
    text: &str,
    task: &OptimizationTask,
) -> (Option<f64>, Vec<(String, f64)>, String) {
    let Some(json) = extract_json_object(text) else {
        return (None, Vec::new(), text.trim().to_string());
    };
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return (None, Vec::new(), text.trim().to_string());
    };
    let mut scores = Vec::new();
    if let Some(dimensions) = value.get("dimensions").and_then(Value::as_object) {
        for dimension in &task.rubric.dimensions {
            if let Some(score) = dimensions.get(&dimension.id).and_then(Value::as_f64) {
                scores.push((dimension.id.clone(), score.clamp(0.0, 1.0)));
            }
        }
    }
    let aggregate = value
        .get("score")
        .and_then(Value::as_f64)
        .map(|score| score.clamp(0.0, 1.0))
        .or_else(|| task.rubric.weighted_score(&scores));
    let notes = value
        .get("notes")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    (aggregate, scores, notes)
}

pub fn parse_labeled_block(text: &str, label: &str) -> String {
    let marker = format!("{label}:");
    let body = if let Some((_, rest)) = text.split_once(&marker) {
        rest
    } else {
        text
    };
    body.lines()
        .take_while(|line| !is_block_label(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        None
    } else {
        Some(&text[start..=end])
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::prompt_opt::healing::healing_image_task;
    use crate::prompt_opt::llm::PromptOptRequest;
    use crate::prompt_opt::model::OptPhase;

    struct ScriptedChat {
        calls: AtomicU32,
    }

    impl PromptOptChat for ScriptedChat {
        fn complete(
            &self,
            request: PromptOptRequest,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, PromptOptError>> + Send + '_>,
        > {
            let phase = request.phase;
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                let _ = call;
                let text = match phase {
                    OptPhase::Propose => {
                        "INSTRUCTION:\nAlive, one scene. 有生气但不乱。\n".to_string()
                    }
                    OptPhase::Evaluate => {
                        r#"{"dimensions":{"vitality":0.86,"not_chaotic":0.8},"notes":"vital without clutter"}"#
                            .to_string()
                    }
                    OptPhase::FailureAnalyst => {
                        "FAILURE:\nOne extra object still competes with the subject.".to_string()
                    }
                    OptPhase::SuccessAnalyst => {
                        "SUCCESS:\nKeep the living light and one readable scene.".to_string()
                    }
                    OptPhase::Distill => "INSTRUCTION:\nShort scene.".to_string(),
                };
                Ok(text)
            })
        }
    }

    fn budget(max_calls: u32, max_cost_micros: u64, cost_per_call_micros: u64) -> Budget {
        Budget {
            max_calls,
            max_cost_micros,
            cost_per_call_micros,
            max_tokens: 1_000_000,
            tokens_per_call: 1,
            calls_used: 0,
            cost_used_micros: 0,
            tokens_used: 0,
        }
    }

    fn policy() -> RunPolicy {
        RunPolicy::explicit()
    }

    #[tokio::test]
    async fn loop_runs_propose_evaluate_reflect() {
        let task = healing_image_task();
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let chat = ScriptedChat {
            calls: AtomicU32::new(0),
        };
        let report = run_optimization(&task, registry, &chat, budget(6, 1_000_000, 1), 1, policy())
            .await
            .unwrap();
        assert_eq!(report.stop, StopReason::CompletedRounds);
        assert_eq!(report.calls_used, 4);
        assert_eq!(
            report.rounds[0].phases,
            vec![
                OptPhase::Propose,
                OptPhase::Evaluate,
                OptPhase::FailureAnalyst,
                OptPhase::SuccessAnalyst
            ]
        );
        assert!(report.rounds[0].validation_promoted);
        assert_eq!(
            report.comparison.after, "Alive, one scene. 有生气但不乱。",
            "optimized text replaces the instruction only"
        );
        assert_eq!(report.comparison.before, task.baseline_instruction);
        assert!(!report.comparison.accepted);
        assert_eq!(
            report.registry.live().instruction,
            task.baseline_instruction
        );
        assert!(
            report
                .registry
                .best_skill_markdown()
                .contains("Alive, one scene. 有生气但不乱。")
        );
        assert!(
            report
                .registry
                .best_skill_markdown()
                .contains("not live until the owner accepts")
        );
        let reflection = report.rounds[0].reflection.as_deref().unwrap();
        assert!(reflection.contains("FAILURE:"));
        assert!(reflection.contains("SUCCESS:"));
        assert!(report.comparison.after_score.unwrap() > 0.8);
        assert_eq!(report.pareto_front.len(), 1);
        assert_eq!(report.train_ids.len(), 2);
        assert_eq!(report.validation_ids.len(), 2);
        assert_eq!(report.trigger, "explicit");
        assert!(report.comparison.diff.contains("+++ proposal"));
        assert!(report.comparison.diff.contains("Alive, one scene"));
        assert!(!report.comparison.accepted);
        assert_eq!(chat.calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn budget_hard_stop_on_max_calls() {
        let task = healing_image_task();
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let chat = ScriptedChat {
            calls: AtomicU32::new(0),
        };
        let report = run_optimization(&task, registry, &chat, budget(2, 1_000_000, 1), 3, policy())
            .await
            .unwrap();
        assert_eq!(report.stop, StopReason::BudgetCalls);
        assert_eq!(report.calls_used, 2);
        assert_eq!(chat.calls.load(Ordering::SeqCst), 2);
        assert!(report.rounds[0].reflection.is_none());
        assert_eq!(
            report.rounds[0].phases,
            vec![OptPhase::Propose, OptPhase::Evaluate]
        );
    }

    #[tokio::test]
    async fn budget_hard_stop_on_max_cost() {
        let task = healing_image_task();
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let chat = ScriptedChat {
            calls: AtomicU32::new(0),
        };
        let report = run_optimization(&task, registry, &chat, budget(9, 10, 10), 3, policy())
            .await
            .unwrap();
        assert_eq!(report.stop, StopReason::BudgetCost);
        assert_eq!(report.calls_used, 1);
        assert_eq!(report.cost_used_micros, 10);
        assert_eq!(chat.calls.load(Ordering::SeqCst), 1);
        assert!(report.rounds[0].proposal.is_some());
        assert!(report.rounds[0].version_id.is_none());
    }

    #[test]
    fn pareto_front_drops_dominated_candidate() {
        let front = pareto_front(&[
            vec![("vitality".into(), 0.9), ("not_chaotic".into(), 0.4)],
            vec![("vitality".into(), 0.5), ("not_chaotic".into(), 0.8)],
            vec![("vitality".into(), 0.4), ("not_chaotic".into(), 0.3)],
        ]);
        assert_eq!(front, vec![0, 1]);
    }

    #[test]
    fn patch_replaces_one_exact_span() {
        let current = "Describe the subject, the light, and the place.";
        let patched = apply_instruction_patch(
            current,
            "PATCH:\n<<<< SEARCH\nthe light\n====\none warm light\n>>>>\n",
        )
        .unwrap();
        assert_eq!(
            patched,
            "Describe the subject, one warm light, and the place."
        );
        let missing = apply_instruction_patch(
            current,
            "PATCH:\n<<<< SEARCH\nmissing span\n====\none warm light\n>>>>\n",
        )
        .unwrap_err();
        assert!(missing.to_string().contains("not found"));
        let broken =
            apply_instruction_patch(current, "PATCH:\n<<<< SEARCH\nthe light\n>>>>\n").unwrap_err();
        assert!(broken.to_string().contains("===="));
        assert_eq!(
            parse_labeled_block("REFLECTION:\nkeep the light", "REFLECTION"),
            "keep the light"
        );
    }

    #[test]
    fn trajectory_digest_is_deterministic_text() {
        let trace = RoundTrace {
            round: 1,
            phases: vec![OptPhase::Evaluate],
            proposal: Some("one scene".to_string()),
            aggregate_score: Some(0.5),
            dimension_scores: vec![("vitality".to_string(), 0.5)],
            reflection: None,
            version_id: Some(2),
            validation_promoted: false,
        };
        let digest = trajectory_digest(&trace, "held out");
        assert!(digest.contains("round: 1"));
        assert!(digest.contains("proposal:\none scene"));
        assert!(digest.contains("validation_score: 0.5"));
        assert!(digest.contains("vitality=0.5"));
        assert!(digest.contains("notes: held out"));
        assert_eq!(digest, trajectory_digest(&trace, "held out"));
    }

    struct TwoRoundChat {
        calls: AtomicU32,
    }

    impl PromptOptChat for TwoRoundChat {
        fn complete(
            &self,
            _request: PromptOptRequest,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, PromptOptError>> + Send + '_>,
        > {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                let text = match call {
                    0 => "INSTRUCTION:\nHigh vitality scene.".to_string(),
                    1 => r#"{"dimensions":{"vitality":0.9,"not_chaotic":0.9},"notes":"strong"}"#
                        .to_string(),
                    2 => "FAILURE:\nminor gap".to_string(),
                    3 => "SUCCESS:\nkeep the scene".to_string(),
                    4 => "INSTRUCTION:\nWeaker cluttered scene.".to_string(),
                    5 => r#"{"dimensions":{"vitality":0.4,"not_chaotic":0.4},"notes":"weaker"}"#
                        .to_string(),
                    6 => "FAILURE:\nclutter".to_string(),
                    _ => "SUCCESS:\nnothing new to keep".to_string(),
                };
                Ok(text)
            })
        }
    }

    #[tokio::test]
    async fn validation_gate_keeps_the_strictly_better_instruction() {
        let task = healing_image_task();
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let chat = TwoRoundChat {
            calls: AtomicU32::new(0),
        };
        let report = run_optimization(&task, registry, &chat, budget(8, 1_000_000, 1), 2, policy())
            .await
            .unwrap();
        assert_eq!(report.stop, StopReason::CompletedRounds);
        assert_eq!(report.calls_used, 8);
        assert!(report.rounds[0].validation_promoted);
        assert!(!report.rounds[1].validation_promoted);
        assert_eq!(report.comparison.after, "High vitality scene.");
        assert!(
            report.rounds[1]
                .reflection
                .as_deref()
                .unwrap()
                .contains("validation gate: not strictly better than the current best")
        );
        let skill = report.registry.best_skill_markdown();
        assert!(skill.contains("High vitality scene."));
        assert!(!skill.contains("Weaker cluttered scene."));
        assert_eq!(
            report.registry.live().instruction,
            task.baseline_instruction
        );
        assert!(!report.comparison.accepted);
        assert!(
            report
                .registry
                .signals()
                .iter()
                .any(|signal| { signal.kind == EvolutionSignalKind::ValidationRejected })
        );
    }

    #[tokio::test]
    async fn budget_hard_stop_on_max_tokens() {
        let task = healing_image_task();
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let chat = ScriptedChat {
            calls: AtomicU32::new(0),
        };
        let mut limited = budget(9, 1_000_000, 1);
        limited.max_tokens = 10;
        limited.tokens_per_call = 10;
        let report = run_optimization(&task, registry, &chat, limited, 3, policy())
            .await
            .unwrap();
        assert_eq!(report.stop, StopReason::BudgetTokens);
        assert_eq!(report.calls_used, 1);
        assert_eq!(report.tokens_used, 10);
        assert_eq!(chat.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn edit_budget_rejects_a_fourth_patch() {
        let current = "Describe the subject, the light, and the place.";
        let one = "PATCH:\n<<<< SEARCH\nthe light\n====\none warm light\n>>>>\n";
        assert!(resolve_proposal(one, current, 3).is_ok());
        let four = "\
PATCH:\n<<<< SEARCH\nthe subject\n====\none subject\n>>>>
PATCH:\n<<<< SEARCH\nthe light\n====\none light\n>>>>
PATCH:\n<<<< SEARCH\nthe place\n====\none place\n>>>>
PATCH:\n<<<< SEARCH\nDescribe\n====\nPaint\n>>>>\n";
        let err = resolve_proposal(four, current, 3).unwrap_err();
        assert!(err.to_string().contains("edit budget exceeded"));
        let three = "\
PATCH:\n<<<< SEARCH\nthe subject\n====\none subject\n>>>>
PATCH:\n<<<< SEARCH\nthe light\n====\none light\n>>>>
PATCH:\n<<<< SEARCH\nthe place\n====\none place\n>>>>\n";
        let updated = resolve_proposal(three, current, 3).unwrap();
        assert_eq!(updated, "Describe one subject, one light, and one place.");
    }

    struct DistillChat;

    impl PromptOptChat for DistillChat {
        fn complete(
            &self,
            request: PromptOptRequest,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, PromptOptError>> + Send + '_>,
        > {
            let phase = request.phase;
            let user = request.user.clone();
            Box::pin(async move {
                let text = match phase {
                    OptPhase::Propose => {
                        "INSTRUCTION:\nA long living scene with light.".to_string()
                    }
                    OptPhase::Evaluate => {
                        if user.contains("A long living scene") {
                            r#"{"dimensions":{"vitality":0.9,"not_chaotic":0.9},"notes":"strong"}"#
                                .to_string()
                        } else {
                            r#"{"dimensions":{"vitality":0.9,"not_chaotic":0.9},"notes":"same"}"#
                                .to_string()
                        }
                    }
                    OptPhase::FailureAnalyst => "FAILURE:\nnone".to_string(),
                    OptPhase::SuccessAnalyst => "SUCCESS:\nkeep it".to_string(),
                    OptPhase::Distill => "INSTRUCTION:\nShort scene.".to_string(),
                };
                Ok(text)
            })
        }
    }

    #[tokio::test]
    async fn distill_must_pass_held_out_and_still_needs_accept() {
        let task = healing_image_task();
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let mut tight = policy();
        tight.distill_when_chars_above = 1;
        let report = run_optimization(
            &task,
            registry,
            &DistillChat,
            budget(8, 1_000_000, 1),
            1,
            tight,
        )
        .await
        .unwrap();
        assert!(report.rounds[0].phases.contains(&OptPhase::Distill));
        assert_eq!(report.comparison.after, "A long living scene with light.");
        assert!(report.registry.versions().iter().any(|version| {
            version.instruction == "Short scene." && version.notes.contains("awaiting human accept")
        }));
        assert_eq!(
            report.registry.live().instruction,
            task.baseline_instruction
        );
        let mut registry = report.registry;
        let distill_id = registry
            .versions()
            .iter()
            .find(|version| version.instruction == "Short scene.")
            .unwrap()
            .id;
        registry.accept(distill_id).unwrap();
        assert_eq!(registry.live().instruction, "Short scene.");
    }

    struct WorseDistillChat;

    impl PromptOptChat for WorseDistillChat {
        fn complete(
            &self,
            request: PromptOptRequest,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, PromptOptError>> + Send + '_>,
        > {
            let phase = request.phase;
            let user = request.user.clone();
            Box::pin(async move {
                let text = match phase {
                    OptPhase::Propose => {
                        "INSTRUCTION:\nA long living scene with light.".to_string()
                    }
                    OptPhase::Evaluate => {
                        if user.contains("Compressed badly") {
                            r#"{"dimensions":{"vitality":0.1,"not_chaotic":0.1},"notes":"worse"}"#
                                .to_string()
                        } else {
                            r#"{"dimensions":{"vitality":0.9,"not_chaotic":0.9},"notes":"strong"}"#
                                .to_string()
                        }
                    }
                    OptPhase::FailureAnalyst => "FAILURE:\nnone".to_string(),
                    OptPhase::SuccessAnalyst => "SUCCESS:\nkeep it".to_string(),
                    OptPhase::Distill => "INSTRUCTION:\nCompressed badly.".to_string(),
                };
                Ok(text)
            })
        }
    }

    #[tokio::test]
    async fn worse_distill_is_archived_and_not_promoted() {
        let task = healing_image_task();
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let mut tight = policy();
        tight.distill_when_chars_above = 1;
        let report = run_optimization(
            &task,
            registry,
            &WorseDistillChat,
            budget(8, 1_000_000, 1),
            1,
            tight,
        )
        .await
        .unwrap();
        assert_eq!(report.comparison.after, "A long living scene with light.");
        assert!(
            report
                .registry
                .signals()
                .iter()
                .any(|signal| { signal.kind == EvolutionSignalKind::DistillRejected })
        );
        assert_eq!(
            report.registry.live().instruction,
            task.baseline_instruction
        );
        assert!(!report.comparison.accepted);
    }

    #[tokio::test]
    async fn non_healing_task_is_outside_the_pilot() {
        let mut task = healing_image_task();
        task.name = "some_other_skill".to_string();
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let err = run_optimization(
            &task,
            registry,
            &ScriptedChat {
                calls: AtomicU32::new(0),
            },
            budget(4, 100, 1),
            1,
            policy(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("healing-style"));
    }

    #[test]
    fn optimizer_is_not_wired_into_boot_or_heartbeat() {
        let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut hits = Vec::new();
        fn visit(dir: &std::path::Path, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().and_then(|name| name.to_str()) == Some("prompt_opt") {
                        continue;
                    }
                    visit(&path, hits);
                    continue;
                }
                if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for needle in [
                    "run_optimization(",
                    "PromptWorkbench",
                    "run_healing_subscription_round",
                ] {
                    if text.contains(needle) {
                        hits.push(format!("{} contains {needle}", path.display()));
                    }
                }
            }
        }
        visit(&src, &mut hits);
        assert!(hits.is_empty(), "{hits:?}");
    }
}
