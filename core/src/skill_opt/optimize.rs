use serde::{Deserialize, Serialize};

use crate::llm::LlmProviderErrorKind;
use crate::prompt_opt::{
    Budget, BudgetStop, EvolutionSignalKind, Example, OptPhase, OptimizationTask,
    OptimizationTrigger, PromptOptError, PromptRegistry, RunPolicy, StopReason, parse_evaluation,
    parse_labeled_block, proposal_diff, resolve_proposal, trajectory_digest,
};

use super::llm::{SkillOptChat, SkillOptRequest};
use super::model::SkillOptError;
use super::pilot::{PILOT_TASK_NAME, proposal_passes_structural_gate};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillOptReport {
    pub skill_name: String,
    pub rounds: Vec<crate::prompt_opt::RoundTrace>,
    pub stop: StopReason,
    pub train_ids: Vec<String>,
    pub validation_ids: Vec<String>,
    pub before_markdown: String,
    pub proposed_markdown: String,
    pub proposed_version_id: u64,
    pub proposed_score: Option<f64>,
    pub diff: String,
    /// True only after the owner accepts the proposed version.
    pub accepted: bool,
    /// False when the run stopped on the hard budget. Those versions must not
    /// be written to the live SKILL.md.
    pub live_write_allowed: bool,
    pub calls_used: u32,
    pub cost_used_micros: u64,
    pub tokens_used: u64,
    pub trigger: String,
}

#[derive(Debug)]
pub struct SkillOptLoopResult {
    pub registry: PromptRegistry,
    pub report: SkillOptReport,
    pub new_version_ids: Vec<u64>,
}

pub async fn run_skill_optimization(
    task: &OptimizationTask,
    mut registry: PromptRegistry,
    chat: &dyn SkillOptChat,
    mut budget: Budget,
    max_rounds: u32,
    policy: RunPolicy,
    dir_name: &str,
) -> Result<SkillOptLoopResult, SkillOptError> {
    if policy.trigger != OptimizationTrigger::Explicit {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            "skill evolution starts only from an explicit trigger",
        ));
    }
    if task.name != PILOT_TASK_NAME {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            "pilot scope is the pilot-web-search skill only",
        ));
    }
    if max_rounds == 0 {
        return Err(SkillOptError::invalid("max_rounds must be at least 1"));
    }
    let (train, validation) =
        crate::prompt_opt::split_train_validation(&task.examples, task.validation_ratio);
    let mut rounds = Vec::new();
    let mut stop = StopReason::CompletedRounds;
    let mut new_version_ids = Vec::new();
    let mut budget_stopped = false;

    for round in 1..=max_rounds {
        let mut trace = crate::prompt_opt::RoundTrace {
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
            &propose_system(policy.max_patches_per_round, dir_name),
            &propose_user(task, &train, &current, dir_name),
        )
        .await?
        {
            Control::Stop(reason) => {
                stop = reason;
                budget_stopped = true;
                rounds.push(trace);
                break;
            }
            Control::Ok(text) => text,
        };
        trace.phases.push(OptPhase::Propose);
        let instruction = resolve_proposal(&proposal_text, &current, policy.max_patches_per_round)
            .map_err(map_prompt_err)?;
        trace.proposal = Some(instruction.clone());

        if !proposal_passes_structural_gate(&instruction, dir_name) {
            let version_id = push_version(
                &mut registry,
                &mut new_version_ids,
                instruction,
                parent_id,
                None,
                Vec::new(),
                "structural gate rejected the proposal",
            );
            registry.record_signal(
                EvolutionSignalKind::ValidationRejected,
                version_id,
                "proposal failed the pilot SKILL.md structural gate",
            );
            trace.version_id = Some(version_id);
            rounds.push(trace);
            continue;
        }

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
                budget_stopped = true;
                rounds.push(trace);
                break;
            }
            Control::Ok(text) => text,
        };
        trace.phases.push(OptPhase::Evaluate);
        let (score, dimension_scores, notes) = parse_evaluation(&evaluation_text, task);
        trace.aggregate_score = score.or_else(|| task.rubric.weighted_score(&dimension_scores));
        trace.dimension_scores = dimension_scores.clone();
        let version_id = push_version(
            &mut registry,
            &mut new_version_ids,
            instruction,
            parent_id,
            trace.aggregate_score,
            dimension_scores,
            notes.clone(),
        );
        trace.version_id = Some(version_id);
        trace.validation_promoted = registry
            .promote_if_strictly_better(version_id)
            .map_err(map_prompt_err)?;
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
                budget_stopped = true;
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
                budget_stopped = true;
                rounds.push(trace);
                break;
            }
            Control::Ok(text) => text,
        };
        trace.phases.push(OptPhase::SuccessAnalyst);
        let success = parse_labeled_block(&success_text, "SUCCESS");
        let reflection = combine_reflection(&failure, &success, trace.validation_promoted);
        registry
            .annotate(version_id, reflection.clone())
            .map_err(map_prompt_err)?;
        trace.reflection = Some(reflection);
        if let Some(reason) = maybe_distill(
            task,
            &mut registry,
            chat,
            &mut budget,
            &mut trace,
            &mut new_version_ids,
            &validation,
            policy,
            dir_name,
        )
        .await?
        {
            stop = reason;
            budget_stopped = true;
            rounds.push(trace);
            break;
        }
        rounds.push(trace);
    }

    let live_write_allowed = !budget_stopped && matches!(stop, StopReason::CompletedRounds);
    let report = finish(
        &registry,
        rounds,
        stop,
        &train,
        &validation,
        &budget,
        policy,
        live_write_allowed,
    );
    Ok(SkillOptLoopResult {
        registry,
        report,
        new_version_ids,
    })
}

fn push_version(
    registry: &mut PromptRegistry,
    new_version_ids: &mut Vec<u64>,
    instruction: String,
    parent_id: u64,
    score: Option<f64>,
    dimension_scores: Vec<(String, f64)>,
    notes: impl Into<String>,
) -> u64 {
    let version_id =
        registry.push_candidate(instruction, Some(parent_id), score, dimension_scores, notes);
    new_version_ids.push(version_id);
    version_id
}

enum Control {
    Stop(StopReason),
    Ok(String),
}

async fn charge_and_complete(
    chat: &dyn SkillOptChat,
    budget: &mut Budget,
    phase: OptPhase,
    system: &str,
    user: &str,
) -> Result<Control, SkillOptError> {
    match budget.try_charge() {
        Err(BudgetStop::Calls) => return Ok(Control::Stop(StopReason::BudgetCalls)),
        Err(BudgetStop::Cost) => return Ok(Control::Stop(StopReason::BudgetCost)),
        Err(BudgetStop::Tokens) => return Ok(Control::Stop(StopReason::BudgetTokens)),
        Ok(()) => {}
    }
    let text = chat
        .complete(SkillOptRequest {
            phase,
            system: system.to_string(),
            user: user.to_string(),
        })
        .await?;
    Ok(Control::Ok(text))
}

fn map_prompt_err(err: PromptOptError) -> SkillOptError {
    match err.provider_kind() {
        Some(kind) => SkillOptError::fail_closed(kind, err.to_string()),
        None => {
            SkillOptError::fail_closed(LlmProviderErrorKind::UnsupportedFeature, err.to_string())
        }
    }
}

fn finish(
    registry: &PromptRegistry,
    rounds: Vec<crate::prompt_opt::RoundTrace>,
    stop: StopReason,
    train: &[Example],
    validation: &[Example],
    budget: &Budget,
    policy: RunPolicy,
    live_write_allowed: bool,
) -> SkillOptReport {
    let before_id = registry.baseline_id();
    let after_id = registry.validation_best_id();
    let before = registry.baseline().instruction.clone();
    let after = registry.validation_best().instruction.clone();
    let accepted = registry.live_id() == after_id
        && after_id != before_id
        && registry
            .get(after_id)
            .is_some_and(|version| version.accepted);
    SkillOptReport {
        skill_name: super::pilot::PILOT_SKILL_NAME.to_string(),
        rounds,
        stop,
        train_ids: train.iter().map(|example| example.id.clone()).collect(),
        validation_ids: validation
            .iter()
            .map(|example| example.id.clone())
            .collect(),
        before_markdown: before.clone(),
        proposed_markdown: after.clone(),
        proposed_version_id: after_id,
        proposed_score: registry.validation_best().score,
        diff: proposal_diff(&before, &after),
        accepted,
        live_write_allowed,
        calls_used: budget.calls_used,
        cost_used_micros: budget.cost_used_micros,
        tokens_used: budget.tokens_used,
        trigger: policy.trigger.as_str().to_string(),
    }
}

fn propose_system(max_patches: u32, dir_name: &str) -> String {
    format!(
        "You improve one Agent Skill markdown file named {dir_name}. \
Reply with exactly one of these blocks:\nINSTRUCTION:\n<the full replacement SKILL.md>\n\
or up to {max_patches} PATCH blocks:\nPATCH:\n<<<< SEARCH\n<exact text from the current SKILL.md>\n====\n<replacement text>\n>>>>\n\
Keep YAML frontmatter. The name field must stay {dir_name}. \
Do not mention other skills, intents, envelopes, or sandboxes."
    )
}

fn propose_user(
    task: &OptimizationTask,
    train: &[Example],
    current: &str,
    dir_name: &str,
) -> String {
    format!(
        "Pilot skill directory: {dir_name}\nTask: {name}\nInput/output: {io}\nTaste: {taste}\nRubric:\n{rubric}\nTrain examples:\n{train}\nCurrent SKILL.md:\n{current}",
        name = task.name,
        io = task.io_description,
        taste = task.rubric.taste,
        rubric = format_rubric(task),
        train = format_examples(train),
    )
}

fn evaluate_system() -> String {
    "Score the candidate SKILL.md against the rubric. Reply with JSON only: {\"dimensions\":{\"<dimension id>\":0.0},\"notes\":\"...\"}. Scores are from 0 to 1.".to_string()
}

fn evaluate_user(task: &OptimizationTask, validation: &[Example], markdown: &str) -> String {
    format!(
        "Taste: {taste}\nRubric:\n{rubric}\nValidation examples:\n{validation}\nCandidate SKILL.md:\n{markdown}",
        taste = task.rubric.taste,
        rubric = format_rubric(task),
        validation = format_examples(validation),
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
    )
}

fn distill_system(dir_name: &str) -> String {
    format!(
        "Compress the SKILL.md. Keep the same meaning in fewer words and keep valid frontmatter. The name field must stay {dir_name}. Reply with:\nINSTRUCTION:\n<shorter SKILL.md>"
    )
}

fn distill_user(current: &str, ledger: &str) -> String {
    format!("Current SKILL.md:\n{current}\n{ledger}")
}

async fn maybe_distill(
    task: &OptimizationTask,
    registry: &mut PromptRegistry,
    chat: &dyn SkillOptChat,
    budget: &mut Budget,
    trace: &mut crate::prompt_opt::RoundTrace,
    new_version_ids: &mut Vec<u64>,
    validation: &[Example],
    policy: RunPolicy,
    dir_name: &str,
) -> Result<Option<StopReason>, SkillOptError> {
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
        &distill_system(dir_name),
        &distill_user(&current, &registry.ledger_prompt()),
    )
    .await?
    {
        Control::Stop(reason) => return Ok(Some(reason)),
        Control::Ok(text) => text,
    };
    trace.phases.push(OptPhase::Distill);
    let instruction = resolve_proposal(&distill_text, &current, policy.max_patches_per_round)
        .map_err(map_prompt_err)?;
    if instruction.trim() == current.trim()
        || instruction.chars().count() >= current.chars().count()
    {
        let version_id = push_version(
            registry,
            new_version_ids,
            instruction,
            parent_id,
            None,
            Vec::new(),
            "distill did not shorten the skill",
        );
        registry.record_signal(
            EvolutionSignalKind::DistillRejected,
            version_id,
            "distill did not shorten the skill; refusing the over-long text",
        );
        return Ok(None);
    }
    if !proposal_passes_structural_gate(&instruction, dir_name) {
        let version_id = push_version(
            registry,
            new_version_ids,
            instruction,
            parent_id,
            None,
            Vec::new(),
            "distill failed the structural gate",
        );
        registry.record_signal(
            EvolutionSignalKind::DistillRejected,
            version_id,
            "distill failed the pilot SKILL.md structural gate",
        );
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
    let version_id = push_version(
        registry,
        new_version_ids,
        instruction,
        parent_id,
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
        registry
            .annotate(version_id, "distill held-out gate: worse than current")
            .map_err(map_prompt_err)?;
        return Ok(None);
    }
    let promoted = registry
        .promote_if_strictly_better(version_id)
        .map_err(map_prompt_err)?;
    if !promoted {
        registry
            .annotate(
                version_id,
                "distill held-out gate: not worse; awaiting human accept",
            )
            .map_err(map_prompt_err)?;
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

fn combine_reflection(failure: &str, success: &str, promoted: bool) -> String {
    let mut notes = format!("FAILURE:\n{failure}\nSUCCESS:\n{success}");
    if !promoted {
        notes.push_str("\nvalidation gate: not strictly better than the current best");
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_opt::{Budget, PromptRegistry, RunPolicy};
    use crate::skill_opt::llm::SkillOptFuture;
    use crate::skill_opt::pilot::{BASELINE_SKILL_MARKDOWN, PILOT_SKILL_NAME, pilot_task};

    struct Scripted {
        calls: std::sync::Arc<std::sync::atomic::AtomicU32>,
        eval_index: std::sync::Arc<std::sync::atomic::AtomicU32>,
        propose: String,
        evaluate_scores: Vec<String>,
        distill: String,
    }

    impl Scripted {
        fn improved() -> Self {
            Self {
                calls: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
                eval_index: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
                propose: format!("INSTRUCTION:\n{}", sample_improved()),
                evaluate_scores: vec![high_score()],
                distill: format!("INSTRUCTION:\n{}", sample_short()),
            }
        }
    }

    fn high_score() -> String {
        r#"{"dimensions":{"sources":0.9,"precision":0.85,"brevity":0.8},"notes":"cites urls"}"#
            .to_string()
    }

    fn low_score() -> String {
        r#"{"dimensions":{"sources":0.2,"precision":0.2,"brevity":0.2},"notes":"worse"}"#
            .to_string()
    }

    fn sample_improved() -> String {
        format!(
            "{}\nWhen the question is in Chinese, answer in Chinese and still cite source URLs.\n",
            BASELINE_SKILL_MARKDOWN.trim_end()
        )
    }

    fn sample_short() -> String {
        "---\nname: pilot-web-search\ndescription: Short pilot web search with source URLs.\n---\n\n# Pilot web search\n\nOne query. Cite a source URL.\n".to_string()
    }

    impl SkillOptChat for Scripted {
        fn complete(
            &self,
            request: SkillOptRequest,
        ) -> SkillOptFuture<'_, Result<String, SkillOptError>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let eval_slot = if request.phase == OptPhase::Evaluate {
                self.eval_index
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst) as usize
            } else {
                0
            };
            let propose = self.propose.clone();
            let distill = self.distill.clone();
            let evaluate_scores = self.evaluate_scores.clone();
            let phase = request.phase;
            Box::pin(async move {
                let text = match phase {
                    OptPhase::Propose => propose,
                    OptPhase::Evaluate => evaluate_scores
                        .get(eval_slot)
                        .cloned()
                        .unwrap_or_else(high_score),
                    OptPhase::FailureAnalyst => "FAILURE:\nStill one vague step.".to_string(),
                    OptPhase::SuccessAnalyst => "SUCCESS:\nKeep the source URL rule.".to_string(),
                    OptPhase::Distill => distill,
                };
                Ok(text)
            })
        }
    }

    fn budget(max_calls: u32) -> Budget {
        Budget {
            max_calls,
            max_cost_micros: 1_000_000,
            cost_per_call_micros: 1,
            max_tokens: 20_000,
            tokens_per_call: 1,
            calls_used: 0,
            cost_used_micros: 0,
            tokens_used: 0,
        }
    }

    #[tokio::test]
    async fn explicit_round_proposes_without_accepting() {
        let task = pilot_task(BASELINE_SKILL_MARKDOWN);
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let chat = Scripted::improved();
        let result = run_skill_optimization(
            &task,
            registry,
            &chat,
            budget(8),
            1,
            RunPolicy::explicit(),
            PILOT_SKILL_NAME,
        )
        .await
        .unwrap();
        assert!(result.report.live_write_allowed);
        assert!(!result.report.accepted);
        assert_eq!(result.report.trigger, "explicit");
        assert_eq!(result.report.train_ids, vec!["rust-result", "http-429"]);
        assert_eq!(
            result.report.validation_ids,
            vec!["zh-rust-docs", "axum-repo"]
        );
        assert!(
            result
                .report
                .proposed_markdown
                .contains("answer in Chinese")
        );
        assert_eq!(result.registry.live().instruction, BASELINE_SKILL_MARKDOWN);
        assert_ne!(
            result.registry.validation_best().instruction,
            BASELINE_SKILL_MARKDOWN
        );
    }

    #[tokio::test]
    async fn more_than_three_patches_fail_closed() {
        let task = pilot_task(BASELINE_SKILL_MARKDOWN);
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let mut patches = String::new();
        for index in 0..4 {
            patches.push_str(&format!(
                "PATCH:\n<<<< SEARCH\nline{index}\n====\nline{index}b\n>>>>\n"
            ));
        }
        let chat = Scripted {
            propose: patches,
            ..Scripted::improved()
        };
        let err = run_skill_optimization(
            &task,
            registry,
            &chat,
            budget(8),
            1,
            RunPolicy::explicit(),
            PILOT_SKILL_NAME,
        )
        .await
        .expect_err("four patches exceed the edit budget");
        assert!(err.to_string().contains("edit budget"));
        assert!(err.to_string().contains("fail closed"));
        assert_eq!(chat.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn budget_exhaustion_does_not_keep_an_unscored_proposal() {
        let task = pilot_task(BASELINE_SKILL_MARKDOWN);
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let chat = Scripted::improved();
        let result = run_skill_optimization(
            &task,
            registry,
            &chat,
            budget(1),
            1,
            RunPolicy::explicit(),
            PILOT_SKILL_NAME,
        )
        .await
        .unwrap();
        assert!(!result.report.live_write_allowed);
        assert_eq!(result.report.stop, StopReason::BudgetCalls);
        assert!(result.new_version_ids.is_empty());
        assert_eq!(result.registry.versions().len(), 1);
        assert_eq!(chat.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn over_long_distill_must_pass_the_held_out_gate() {
        let task = pilot_task(BASELINE_SKILL_MARKDOWN);
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let chat = Scripted {
            evaluate_scores: vec![high_score(), low_score()],
            ..Scripted::improved()
        };
        let mut policy = RunPolicy::explicit();
        policy.distill_when_chars_above = 10;
        let result = run_skill_optimization(
            &task,
            registry,
            &chat,
            budget(8),
            1,
            policy,
            PILOT_SKILL_NAME,
        )
        .await
        .unwrap();
        assert!(
            result
                .registry
                .signals()
                .iter()
                .any(|signal| signal.kind == EvolutionSignalKind::DistillRejected)
        );
        assert!(
            result
                .registry
                .validation_best()
                .instruction
                .contains("Chinese")
        );
        assert!(
            !result
                .registry
                .validation_best()
                .instruction
                .contains("One query")
        );
        assert_eq!(result.registry.live().instruction, BASELINE_SKILL_MARKDOWN);
    }

    #[tokio::test]
    async fn distill_that_does_not_shrink_is_rejected() {
        let task = pilot_task(BASELINE_SKILL_MARKDOWN);
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let improved = sample_improved();
        let chat = Scripted {
            distill: format!("INSTRUCTION:\n{improved}\nExtra sentence that makes it longer.\n"),
            ..Scripted::improved()
        };
        let mut policy = RunPolicy::explicit();
        policy.distill_when_chars_above = 10;
        let before_calls_budget = budget(8);
        let result = run_skill_optimization(
            &task,
            registry,
            &chat,
            before_calls_budget,
            1,
            policy,
            PILOT_SKILL_NAME,
        )
        .await
        .unwrap();
        assert!(result.registry.signals().iter().any(|signal| {
            signal.kind == EvolutionSignalKind::DistillRejected
                && signal.detail.contains("did not shorten")
        }));
        assert!(result.report.live_write_allowed);
    }

    #[tokio::test]
    async fn non_pilot_task_name_fails_closed_before_a_call() {
        let mut task = pilot_task(BASELINE_SKILL_MARKDOWN);
        task.name = "typesafe-ai".to_string();
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let chat = Scripted::improved();
        let err = run_skill_optimization(
            &task,
            registry,
            &chat,
            budget(8),
            1,
            RunPolicy::explicit(),
            PILOT_SKILL_NAME,
        )
        .await
        .expect_err("only the pilot task may run");
        assert!(err.to_string().contains("pilot-web-search"));
        assert_eq!(chat.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
