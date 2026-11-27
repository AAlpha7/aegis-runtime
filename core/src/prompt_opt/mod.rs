//! General prompt optimization (propose → evaluate → dual analyst).
//!
//! The loop is implemented here. It follows the GEPA idea of proposing a
//! candidate, scoring it, and reflecting, with a Pareto archive of
//! non-dominated candidates. Reflection is two charged analyst calls over one
//! deterministic trajectory digest: a failure analyst and a success analyst.
//! A candidate replaces `best_skill` only when its validation score is
//! strictly higher than the current winner. No DSPy, GEPA, or SkillOpt crate
//! is vendored or called.
//!
//! Optimized text is instruction text only. This module does not construct or
//! modify intent envelopes and does not touch the Wasm sandbox. A candidate
//! stays off the live pointer until the owner approves it. The validation
//! winner is a registry artifact (`best_skill_markdown`), not a live write.
//!
//! Rounds start only from an explicit caller (`OptimizationTrigger::Explicit`).
//! Nothing in boot or heartbeat invokes this module. The runnable pilot is
//! the healing-style image prompt.
//!
//! Every model call goes through [`llm::RoutedPromptLlm`], which calls
//! [`crate::llm::LlmRouter::route_chat`] with
//! [`crate::llm::LlmTaskClass::PromptOptimization`]. Subscription background
//! use is denied until the owner grants that class. Paid API spend also
//! requires `prompt_optimization_allow_paid_api` (default false).
//! `paid_api_fallback_enabled` does not grant this class. Denied calls fail
//! closed and do not spend paid API credits.

mod healing;
mod llm;
mod model;
mod optimize;
mod registry;
mod workbench;

pub use healing::{
    HealingSubscriptionPlan, LiveCredentialPaths, healing_image_task, healing_subscription_plan,
    live_credential_blocker, run_healing_subscription_round_from_files,
};
pub use llm::{CountingPaidBackend, PromptOptChat, PromptOptRequest, RoutedPromptLlm};
pub use model::{
    Budget, BudgetStop, DEFAULT_MAX_TOKENS, DEFAULT_TOKENS_PER_CALL, Example, OptPhase,
    OptimizationTask, OptimizationTrigger, PairwiseJudgment, PromptOptError, Rubric,
    RubricDimension, RunPolicy, split_train_validation,
};
pub use optimize::{
    InstructionComparison, OptimizationReport, RoundTrace, StopReason, parse_evaluation,
    parse_labeled_block, proposal_diff, resolve_proposal, run_optimization, trajectory_digest,
};
pub use registry::{
    ContradictionItem, EvolutionSignal, EvolutionSignalKind, FeedbackOutcome, PromptRegistry,
    PromptVersion,
};
pub use workbench::{DEFERRED_OWNER_ROUTES, PromptWorkbench, WorkbenchCommand, WorkbenchSnapshot};
