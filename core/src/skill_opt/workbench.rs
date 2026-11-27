use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::llm::LlmProviderErrorKind;
use crate::prompt_opt::{Budget, PromptRegistry, RunPolicy};

use super::llm::SkillOptChat;
use super::model::SkillOptError;
use super::optimize::{SkillOptReport, run_skill_optimization};
use super::pilot::{
    DEFAULT_COST_PER_CALL_MICROS, DEFAULT_MAX_CALLS, DEFAULT_MAX_COST_MICROS, DEFAULT_MAX_ROUNDS,
    PILOT_SKILL_NAME, assert_pilot_skill_path, pilot_task, validate_pilot_markdown,
};

/// Owner-console routes this module is ready to back. They are not mounted, so
/// process boot cannot start a round. Handlers should call [`SkillWorkbench`]
/// and require the existing authenticated session plus CSRF body on mutations.
pub const DEFERRED_OWNER_ROUTES: &[&str] = &[
    "GET /api/skill-opt/pilot",
    "POST /api/skill-opt/run",
    "GET /api/skill-opt/proposal",
    "POST /api/skill-opt/approve",
    "POST /api/skill-opt/reject",
    "POST /api/skill-opt/rollback",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillOptSchedule {
    /// `0` disables the scheduled entry. Nothing polls this value on its own.
    pub interval_secs: u64,
    #[serde(default)]
    pub last_run_unix: Option<i64>,
}

/// True only when an operator set a positive interval and that interval has
/// elapsed. A zero interval is never due. This does not start a task.
pub fn schedule_is_due(schedule: &SkillOptSchedule, now_unix: i64) -> bool {
    if schedule.interval_secs == 0 {
        return false;
    }
    let interval = i64::try_from(schedule.interval_secs).unwrap_or(i64::MAX);
    match schedule.last_run_unix {
        Some(last) => now_unix.saturating_sub(last) >= interval,
        None => true,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DiskLedger {
    registry: PromptRegistry,
    #[serde(default)]
    blocked_version_ids: Vec<u64>,
    schedule: SkillOptSchedule,
    live_path: String,
}

#[derive(Debug)]
pub struct SkillWorkbench {
    live_path: PathBuf,
    proposals_dir: PathBuf,
    dir_name: String,
    registry: PromptRegistry,
    budget: Budget,
    max_rounds: u32,
    blocked_version_ids: Vec<u64>,
    schedule: SkillOptSchedule,
    last_report: Option<SkillOptReport>,
}

impl SkillWorkbench {
    pub fn open(
        live_path: impl Into<PathBuf>,
        proposals_dir: impl Into<PathBuf>,
    ) -> Result<Self, SkillOptError> {
        let live_path = live_path.into();
        let proposals_dir = proposals_dir.into();
        let dir_name = assert_pilot_skill_path(&live_path)?;
        let markdown = std::fs::read_to_string(&live_path).map_err(|err| {
            SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                format!("could not read {}: {err}", live_path.display()),
            )
        })?;
        validate_pilot_markdown(&markdown, &dir_name)?;
        std::fs::create_dir_all(&proposals_dir).map_err(|err| {
            SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                format!("could not create proposals dir: {err}"),
            )
        })?;
        let ledger_path = proposals_dir.join("ledger.json");
        let (registry, blocked_version_ids, schedule) = if ledger_path.is_file() {
            let disk = load_ledger(&ledger_path)?;
            if !same_file(&PathBuf::from(&disk.live_path), &live_path) {
                return Err(SkillOptError::fail_closed(
                    LlmProviderErrorKind::UnsupportedFeature,
                    "evolution ledger belongs to a different skill path",
                ));
            }
            if disk.registry.live().instruction != markdown {
                return Err(SkillOptError::fail_closed(
                    LlmProviderErrorKind::UnsupportedFeature,
                    "live SKILL.md does not match the evolution ledger",
                ));
            }
            (disk.registry, disk.blocked_version_ids, disk.schedule)
        } else {
            (
                PromptRegistry::new(markdown),
                Vec::new(),
                SkillOptSchedule::default(),
            )
        };
        Ok(Self {
            live_path,
            proposals_dir,
            dir_name,
            registry,
            budget: default_budget(),
            max_rounds: DEFAULT_MAX_ROUNDS,
            blocked_version_ids,
            schedule,
            last_report: None,
        })
    }

    pub fn skill_name(&self) -> &str {
        PILOT_SKILL_NAME
    }

    pub fn is_due(&self, now_unix: i64) -> bool {
        schedule_is_due(&self.schedule, now_unix)
    }

    pub fn set_schedule_interval(&mut self, interval_secs: u64) -> Result<(), SkillOptError> {
        self.schedule.interval_secs = interval_secs;
        self.persist()
    }

    pub fn schedule(&self) -> &SkillOptSchedule {
        &self.schedule
    }

    pub fn blocked_version_ids(&self) -> &[u64] {
        &self.blocked_version_ids
    }

    pub fn registry(&self) -> &PromptRegistry {
        &self.registry
    }

    pub fn last_report(&self) -> Option<&SkillOptReport> {
        self.last_report.as_ref()
    }

    pub async fn run_once(
        &mut self,
        chat: &dyn SkillOptChat,
    ) -> Result<SkillOptReport, SkillOptError> {
        self.run_once_at(chat, unix_now()).await
    }

    pub async fn run_once_at(
        &mut self,
        chat: &dyn SkillOptChat,
        now_unix: i64,
    ) -> Result<SkillOptReport, SkillOptError> {
        assert_pilot_skill_path(&self.live_path)?;
        self.schedule.last_run_unix = Some(now_unix);
        self.persist()?;
        let task = pilot_task(self.registry.baseline().instruction.as_str());
        let result = run_skill_optimization(
            &task,
            self.registry.clone(),
            chat,
            self.budget.start_run(),
            self.max_rounds,
            RunPolicy::explicit(),
            &self.dir_name,
        )
        .await?;
        self.registry = result.registry;
        if !result.report.live_write_allowed {
            for version_id in result.new_version_ids.iter().copied() {
                if !self.blocked_version_ids.contains(&version_id) {
                    self.blocked_version_ids.push(version_id);
                }
            }
        }
        self.write_drafts(&result.new_version_ids)?;
        self.last_report = Some(result.report.clone());
        self.persist()?;
        std::fs::write(
            self.proposals_dir.join("report.json"),
            serde_json::to_string_pretty(&result.report).map_err(|err| {
                SkillOptError::invalid(format!("could not encode the proposal report: {err}"))
            })?,
        )
        .map_err(|err| {
            SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                format!("could not write proposal report: {err}"),
            )
        })?;
        Ok(result.report)
    }

    /// Write one accepted version to the live `SKILL.md`. The run itself never
    /// calls this.
    pub fn approve(&mut self, version_id: u64) -> Result<(), SkillOptError> {
        assert_pilot_skill_path(&self.live_path)?;
        if self.blocked_version_ids.contains(&version_id) {
            return Err(SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                "budget fail-closed; refusing to write this version to live SKILL.md",
            ));
        }
        let markdown = self
            .registry
            .get(version_id)
            .ok_or_else(|| SkillOptError::invalid(format!("unknown skill version {version_id}")))?
            .instruction
            .clone();
        validate_pilot_markdown(&markdown, &self.dir_name)?;
        self.write_live(&markdown, version_id)?;
        self.registry.accept(version_id).map_err(registry_err)?;
        if let Some(report) = self.last_report.as_mut()
            && report.proposed_version_id == version_id
        {
            report.accepted = true;
        }
        self.persist()
    }

    pub fn reject(&mut self, version_id: u64) -> Result<(), SkillOptError> {
        let before = std::fs::read_to_string(&self.live_path).map_err(|err| {
            SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                format!("could not read live skill: {err}"),
            )
        })?;
        self.registry.reject(version_id).map_err(registry_err)?;
        let after = std::fs::read_to_string(&self.live_path).map_err(|err| {
            SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                format!("could not read live skill: {err}"),
            )
        })?;
        if before != after {
            return Err(SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                "reject changed the live SKILL.md",
            ));
        }
        self.persist()
    }

    pub fn rollback(&mut self, version_id: u64) -> Result<(), SkillOptError> {
        let markdown = self
            .registry
            .get(version_id)
            .ok_or_else(|| SkillOptError::invalid(format!("unknown skill version {version_id}")))?
            .instruction
            .clone();
        let accepted = self
            .registry
            .get(version_id)
            .is_some_and(|version| version.accepted);
        if version_id != self.registry.baseline_id() && !accepted {
            return Err(SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                "owner must approve a version before it can go live",
            ));
        }
        validate_pilot_markdown(&markdown, &self.dir_name)?;
        self.write_live(&markdown, version_id)?;
        self.registry.rollback(version_id).map_err(registry_err)?;
        self.persist()
    }

    fn write_drafts(&self, version_ids: &[u64]) -> Result<(), SkillOptError> {
        let drafts = self.proposals_dir.join("drafts");
        std::fs::create_dir_all(&drafts).map_err(|err| {
            SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                format!("could not create drafts dir: {err}"),
            )
        })?;
        for version_id in version_ids {
            let Some(version) = self.registry.get(*version_id) else {
                continue;
            };
            std::fs::write(
                drafts.join(format!("v{version_id}.md")),
                &version.instruction,
            )
            .map_err(|err| {
                SkillOptError::fail_closed(
                    LlmProviderErrorKind::UnsupportedFeature,
                    format!("could not write draft v{version_id}: {err}"),
                )
            })?;
        }
        Ok(())
    }

    fn write_live(&self, markdown: &str, version_id: u64) -> Result<(), SkillOptError> {
        let rollback_dir = self.proposals_dir.join("rollbacks");
        std::fs::create_dir_all(&rollback_dir).map_err(|err| {
            SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                format!("could not create rollback dir: {err}"),
            )
        })?;
        let backup = rollback_dir.join(format!("v{version_id}-pre.md"));
        if self.live_path.is_file() {
            std::fs::copy(&self.live_path, &backup).map_err(|err| {
                SkillOptError::fail_closed(
                    LlmProviderErrorKind::UnsupportedFeature,
                    format!("could not snapshot live SKILL.md: {err}"),
                )
            })?;
        }
        if let Err(err) = write_atomic(&self.live_path, markdown) {
            restore_backup(&backup, &self.live_path);
            return Err(err);
        }
        match std::fs::read_to_string(&self.live_path) {
            Ok(written) if written == markdown => Ok(()),
            Ok(_) | Err(_) => {
                restore_backup(&backup, &self.live_path);
                Err(SkillOptError::fail_closed(
                    LlmProviderErrorKind::UnsupportedFeature,
                    "live SKILL.md write did not match the approved version; restored the previous file",
                ))
            }
        }
    }

    fn persist(&self) -> Result<(), SkillOptError> {
        let disk = DiskLedger {
            registry: self.registry.clone(),
            blocked_version_ids: self.blocked_version_ids.clone(),
            schedule: self.schedule.clone(),
            live_path: self.live_path.display().to_string(),
        };
        let text = serde_json::to_string_pretty(&disk)
            .map_err(|err| SkillOptError::invalid(format!("could not encode the ledger: {err}")))?;
        write_atomic(&self.proposals_dir.join("ledger.json"), &text)
    }
}

pub fn default_budget() -> Budget {
    Budget {
        max_calls: DEFAULT_MAX_CALLS,
        max_cost_micros: DEFAULT_MAX_COST_MICROS,
        cost_per_call_micros: DEFAULT_COST_PER_CALL_MICROS,
        max_tokens: crate::prompt_opt::DEFAULT_MAX_TOKENS,
        tokens_per_call: crate::prompt_opt::DEFAULT_TOKENS_PER_CALL,
        calls_used: 0,
        cost_used_micros: 0,
        tokens_used: 0,
    }
}

fn registry_err(err: crate::prompt_opt::PromptOptError) -> SkillOptError {
    SkillOptError::fail_closed(LlmProviderErrorKind::UnsupportedFeature, err.to_string())
}

fn load_ledger(path: &Path) -> Result<DiskLedger, SkillOptError> {
    let text = std::fs::read_to_string(path).map_err(|err| {
        SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            format!("could not read evolution ledger: {err}"),
        )
    })?;
    serde_json::from_str(&text).map_err(|err| {
        SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            format!("evolution ledger is unreadable: {err}"),
        )
    })
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), SkillOptError> {
    let tmp = path.with_extension("tmp");
    if let Err(err) = std::fs::write(&tmp, contents) {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            format!("could not write {}: {err}", tmp.display()),
        ));
    }
    if let Err(err) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            format!("could not replace {}: {err}", path.display()),
        ));
    }
    Ok(())
}

fn restore_backup(backup: &Path, live: &Path) {
    if backup.is_file() {
        let _ = std::fs::copy(backup, live);
    }
}

fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_opt::{EvolutionSignalKind, OptPhase};
    use crate::skill_opt::llm::{SkillOptFuture, SkillOptRequest};
    use crate::skill_opt::pilot::BASELINE_SKILL_MARKDOWN;

    struct Scripted;

    impl SkillOptChat for Scripted {
        fn complete(
            &self,
            request: SkillOptRequest,
        ) -> SkillOptFuture<'_, Result<String, SkillOptError>> {
            let phase = request.phase;
            Box::pin(async move {
                let text = match phase {
                    OptPhase::Propose => format!("INSTRUCTION:\n{}", improved()),
                    OptPhase::Evaluate => {
                        r#"{"dimensions":{"sources":0.9,"precision":0.8,"brevity":0.7},"notes":"ok"}"#
                            .to_string()
                    }
                    OptPhase::FailureAnalyst => "FAILURE:\nCite one more primary source.".to_string(),
                    OptPhase::SuccessAnalyst => "SUCCESS:\nShip after owner approval.".to_string(),
                    OptPhase::Distill => "INSTRUCTION:\nshort".to_string(),
                };
                Ok(text)
            })
        }
    }

    struct CountingChat {
        calls: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }

    impl SkillOptChat for CountingChat {
        fn complete(
            &self,
            _request: SkillOptRequest,
        ) -> SkillOptFuture<'_, Result<String, SkillOptError>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async { Ok("INSTRUCTION:\nunused".to_string()) })
        }
    }

    fn improved() -> String {
        format!(
            "{}\nWhen the question is in Chinese, answer in Chinese and still cite source URLs.\n",
            BASELINE_SKILL_MARKDOWN.trim_end()
        )
    }

    fn tree() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join(".agents/skills/pilot-web-search/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(&skill, BASELINE_SKILL_MARKDOWN).unwrap();
        let sibling = dir.path().join(".agents/skills/typesafe-ai/SKILL.md");
        std::fs::create_dir_all(sibling.parent().unwrap()).unwrap();
        std::fs::write(
            &sibling,
            "---\nname: typesafe-ai\ndescription: leave me alone\n---\n\n# Stay\n",
        )
        .unwrap();
        let proposals = dir.path().join("skill-opt-proposals");
        (dir, skill, proposals)
    }

    #[tokio::test]
    async fn run_writes_a_draft_and_approve_is_the_live_write() {
        let (dir, skill, proposals) = tree();
        let sibling = dir.path().join(".agents/skills/typesafe-ai/SKILL.md");
        let sibling_before = std::fs::read_to_string(&sibling).unwrap();
        let mut bench = SkillWorkbench::open(&skill, &proposals).unwrap();
        let report = bench.run_once(&Scripted).await.unwrap();
        assert!(report.live_write_allowed);
        assert!(!report.accepted);
        assert_eq!(
            std::fs::read_to_string(&skill).unwrap(),
            BASELINE_SKILL_MARKDOWN
        );
        let draft = proposals.join(format!("drafts/v{}.md", report.proposed_version_id));
        assert!(draft.is_file());
        assert!(std::fs::read_to_string(&draft).unwrap().contains("Chinese"));
        bench.approve(report.proposed_version_id).unwrap();
        let live = std::fs::read_to_string(&skill).unwrap();
        assert!(live.contains("answer in Chinese"));
        assert_ne!(live, BASELINE_SKILL_MARKDOWN);
        assert!(
            proposals
                .join("rollbacks")
                .read_dir()
                .unwrap()
                .next()
                .is_some()
        );
        assert_eq!(std::fs::read_to_string(&sibling).unwrap(), sibling_before);
        bench.rollback(bench.registry().baseline_id()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&skill).unwrap(),
            BASELINE_SKILL_MARKDOWN
        );
        let ledger = std::fs::read_to_string(proposals.join("ledger.json")).unwrap();
        assert!(ledger.contains("\"rollback\""));
    }

    #[tokio::test]
    async fn reject_records_the_ledger_and_leaves_the_file() {
        let (_dir, skill, proposals) = tree();
        let mut bench = SkillWorkbench::open(&skill, &proposals).unwrap();
        let report = bench.run_once(&Scripted).await.unwrap();
        bench.reject(report.proposed_version_id).unwrap();
        assert_eq!(
            std::fs::read_to_string(&skill).unwrap(),
            BASELINE_SKILL_MARKDOWN
        );
        assert!(
            bench
                .registry()
                .signals()
                .iter()
                .any(|signal| signal.kind == EvolutionSignalKind::Reject)
        );
        assert_eq!(
            bench.registry().validation_best_id(),
            bench.registry().baseline_id()
        );
    }

    #[tokio::test]
    async fn budget_stop_blocks_the_live_write() {
        let (_dir, skill, proposals) = tree();
        let mut bench = SkillWorkbench::open(&skill, &proposals).unwrap();
        bench.budget.max_calls = 2;
        let report = bench.run_once(&Scripted).await.unwrap();
        assert!(!report.live_write_allowed);
        assert!(!bench.blocked_version_ids().is_empty());
        let blocked = bench.blocked_version_ids()[0];
        let err = bench
            .approve(blocked)
            .expect_err("budget stop blocks approve");
        assert!(err.to_string().contains("fail closed"));
        assert_eq!(
            std::fs::read_to_string(&skill).unwrap(),
            BASELINE_SKILL_MARKDOWN
        );
    }

    #[test]
    fn outside_pilot_path_is_refused_before_any_chat() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join(".agents/skills/typesafe-ai/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(
            &skill,
            "---\nname: typesafe-ai\ndescription: not the pilot\n---\n\n# No\n",
        )
        .unwrap();
        let err = SkillWorkbench::open(&skill, dir.path().join("proposals"))
            .expect_err("typesafe-ai is outside the pilot domain");
        assert!(err.to_string().contains("pilot-web-search"));
    }

    #[tokio::test]
    async fn disabled_schedule_does_not_become_due() {
        let (_dir, skill, proposals) = tree();
        let bench = SkillWorkbench::open(&skill, &proposals).unwrap();
        assert!(!bench.is_due(1_700_000_000));
        assert_eq!(bench.schedule().interval_secs, 0);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let chat = CountingChat {
            calls: std::sync::Arc::clone(&calls),
        };
        assert!(!schedule_is_due(bench.schedule(), 1_700_000_000));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        let _ = chat;
    }
}
