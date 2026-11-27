use serde::{Deserialize, Serialize};

use super::healing::{HEALING_TASK_NAME, healing_image_task};
use super::llm::PromptOptChat;
use super::model::{Budget, OptimizationTask, PromptOptError, Rubric, RunPolicy};
use super::optimize::{InstructionComparison, OptimizationReport, run_optimization};
use super::registry::PromptRegistry;

/// Owner-console routes this module is ready to back. The HTTP surface is
/// deferred so this change does not widen the admin server. Handlers should
/// call [`PromptWorkbench`] and require the existing authenticated session
/// plus CSRF body on mutations.
pub const DEFERRED_OWNER_ROUTES: &[&str] = &[
    "GET /api/prompt-opt/tasks",
    "POST /api/prompt-opt/tasks/select",
    "PUT /api/prompt-opt/rubric",
    "PUT /api/prompt-opt/budget",
    "POST /api/prompt-opt/run",
    "GET /api/prompt-opt/comparison",
    "POST /api/prompt-opt/approve",
    "POST /api/prompt-opt/reject",
    "POST /api/prompt-opt/feedback",
    "POST /api/prompt-opt/rollback",
];

#[derive(Debug, Clone, PartialEq)]
pub enum WorkbenchCommand {
    SelectTask { name: String },
    SetRubric { rubric: Rubric },
    SetBudget { budget: Budget },
    Approve { version_id: u64 },
    Reject { version_id: u64 },
    NoteFeedback { feedback: String },
    Rollback { version_id: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkbenchSnapshot {
    pub task_name: String,
    pub rubric: Rubric,
    pub budget: Budget,
    pub live_instruction: String,
    pub baseline_instruction: String,
    pub comparison: Option<InstructionComparison>,
    pub evolution_signal_count: usize,
    pub contradiction_count: usize,
}

pub struct PromptWorkbench {
    task: OptimizationTask,
    budget: Budget,
    max_rounds: u32,
    registry: PromptRegistry,
    last_report: Option<OptimizationReport>,
}

impl PromptWorkbench {
    pub fn healing_default() -> Self {
        let task = healing_image_task();
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        Self {
            task,
            budget: Budget {
                max_calls: 6,
                max_cost_micros: 1_000_000,
                cost_per_call_micros: 20_000,
                max_tokens: 20_000,
                tokens_per_call: 1_200,
                calls_used: 0,
                cost_used_micros: 0,
                tokens_used: 0,
            },
            max_rounds: 1,
            registry,
            last_report: None,
        }
    }

    pub fn apply(
        &mut self,
        command: WorkbenchCommand,
    ) -> Result<WorkbenchSnapshot, PromptOptError> {
        match command {
            WorkbenchCommand::SelectTask { name } => {
                if name != HEALING_TASK_NAME {
                    return Err(PromptOptError::invalid(format!(
                        "unknown optimization task {name}"
                    )));
                }
                self.task = healing_image_task();
                self.registry = PromptRegistry::new(self.task.baseline_instruction.clone());
                self.last_report = None;
            }
            WorkbenchCommand::SetRubric { rubric } => {
                if rubric.dimensions.is_empty() {
                    return Err(PromptOptError::invalid(
                        "rubric needs at least one dimension",
                    ));
                }
                self.task.rubric = rubric;
            }
            WorkbenchCommand::SetBudget { budget } => {
                self.budget = budget.start_run();
            }
            WorkbenchCommand::Approve { version_id } => {
                self.registry.accept(version_id)?;
            }
            WorkbenchCommand::Reject { version_id } => {
                self.registry.reject(version_id)?;
            }
            WorkbenchCommand::NoteFeedback { feedback } => {
                self.registry.consider_feedback(&feedback);
            }
            WorkbenchCommand::Rollback { version_id } => {
                self.registry.rollback(version_id)?;
            }
        }
        Ok(self.snapshot())
    }

    pub async fn run_once(
        &mut self,
        chat: &dyn PromptOptChat,
    ) -> Result<OptimizationReport, PromptOptError> {
        let report = run_optimization(
            &self.task,
            self.registry.clone(),
            chat,
            self.budget.start_run(),
            self.max_rounds,
            RunPolicy::explicit(),
        )
        .await?;
        self.registry = report.registry.clone();
        self.last_report = Some(report.clone());
        Ok(report)
    }

    pub fn snapshot(&self) -> WorkbenchSnapshot {
        WorkbenchSnapshot {
            task_name: self.task.name.clone(),
            rubric: self.task.rubric.clone(),
            budget: self.budget,
            live_instruction: self.registry.live().instruction.clone(),
            baseline_instruction: self.registry.baseline().instruction.clone(),
            comparison: self
                .last_report
                .as_ref()
                .map(|report| report.comparison.clone()),
            evolution_signal_count: self.registry.signals().len(),
            contradiction_count: self.registry.contradictions().len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_opt::llm::PromptOptRequest;
    use crate::prompt_opt::model::OptPhase;

    struct Scripted;

    impl PromptOptChat for Scripted {
        fn complete(
            &self,
            request: PromptOptRequest,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, PromptOptError>> + Send + '_>,
        > {
            let phase = request.phase;
            Box::pin(async move {
                let text = match phase {
                    OptPhase::Propose => {
                        "INSTRUCTION:\nAlive, one scene. 有生气但不乱。".to_string()
                    }
                    OptPhase::Evaluate => {
                        r#"{"dimensions":{"vitality":0.9,"not_chaotic":0.85},"notes":"ok"}"#
                            .to_string()
                    }
                    OptPhase::FailureAnalyst => "FAILURE:\nStill one extra detail.".to_string(),
                    OptPhase::SuccessAnalyst => "SUCCESS:\nShip after owner approval.".to_string(),
                    OptPhase::Distill => "INSTRUCTION:\nShort scene.".to_string(),
                };
                Ok(text)
            })
        }
    }

    #[tokio::test]
    async fn workbench_run_shows_comparison_until_approval() {
        let mut bench = PromptWorkbench::healing_default();
        let report = bench.run_once(&Scripted).await.unwrap();
        assert!(!report.comparison.accepted);
        assert_eq!(
            bench.snapshot().live_instruction,
            bench.snapshot().baseline_instruction
        );
        let after_id = report.comparison.after_version_id;
        bench
            .apply(WorkbenchCommand::Approve {
                version_id: after_id,
            })
            .unwrap();
        assert_eq!(
            bench.snapshot().live_instruction,
            "Alive, one scene. 有生气但不乱。"
        );
        bench
            .apply(WorkbenchCommand::Rollback {
                version_id: bench
                    .snapshot()
                    .comparison
                    .as_ref()
                    .unwrap()
                    .before_version_id,
            })
            .unwrap();
        assert_eq!(bench.snapshot().live_instruction, report.comparison.before);
        assert!(bench.snapshot().evolution_signal_count >= 1);
    }

    #[tokio::test]
    async fn workbench_reject_and_conflict_do_not_retune_live_text() {
        let mut bench = PromptWorkbench::healing_default();
        let report = bench.run_once(&Scripted).await.unwrap();
        let snapshot = bench
            .apply(WorkbenchCommand::Reject {
                version_id: report.comparison.after_version_id,
            })
            .unwrap();
        assert_eq!(snapshot.live_instruction, snapshot.baseline_instruction);
        assert!(snapshot.evolution_signal_count >= 1);
        let noted = bench
            .apply(WorkbenchCommand::NoteFeedback {
                feedback: "冲突：不要改成另一个场景".to_string(),
            })
            .unwrap();
        assert_eq!(noted.contradiction_count, 1);
        assert_eq!(noted.live_instruction, snapshot.baseline_instruction);
    }
}
