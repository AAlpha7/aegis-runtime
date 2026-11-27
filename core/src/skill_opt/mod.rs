//! Skill-file evolution for Agent Skill markdown (`SKILL.md`).
//!
//! This is not the Wasm skill sandbox and it does not read or write Heirloom
//! memory. The loop follows the same shape as [`crate::prompt_opt`]: propose,
//! score on a held-out split, two analysts, optional distillation, and a
//! version ledger. A round starts only from an explicit caller. The runnable
//! pilot is the `pilot-web-search` skill under `.agents/skills/`.
//!
//! The run writes a proposal and scores. It does not modify the live
//! `SKILL.md`. [`SkillWorkbench::approve`] is the only live write. A hard
//! budget stop fail-closes that write for versions produced by the stopped run.
//!
//! Model calls go through [`llm::RoutedSkillLlm`], which uses
//! [`crate::llm::LlmRouter::route_chat`] with
//! [`crate::llm::LlmTaskClass::SkillOptimization`]. Paid API spend requires
//! `skillfile_optimization_allow_paid_api` (default false).
//! `paid_api_fallback_enabled` does not grant this class.

mod cli;
mod llm;
mod model;
mod optimize;
mod pilot;
mod subscription;
mod workbench;

pub use cli::{
    SkillOptCommand, SkillOptPaths, execute_cli, execute_cli_with, parse_skill_opt_args,
};
pub use llm::{RoutedSkillLlm, SkillOptChat, SkillOptRequest};
pub use model::SkillOptError;
pub use optimize::{SkillOptReport, run_skill_optimization};
pub use pilot::{
    BASELINE_SKILL_MARKDOWN, DEFAULT_COST_PER_CALL_MICROS, DEFAULT_MAX_CALLS,
    DEFAULT_MAX_COST_MICROS, DEFAULT_MAX_ROUNDS, PILOT_SKILL_NAME, PILOT_TASK_NAME,
    STARTS_FROM_BOOT_OR_HEARTBEAT, assert_pilot_skill_path,
};
pub use subscription::{SkillSubscriptionPlan, skill_subscription_plan};
pub use workbench::{DEFERRED_OWNER_ROUTES, SkillOptSchedule, SkillWorkbench, schedule_is_due};
