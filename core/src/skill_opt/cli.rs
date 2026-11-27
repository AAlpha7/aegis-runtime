use std::path::PathBuf;

use super::llm::SkillOptChat;
use super::model::SkillOptError;
use super::subscription::{
    SkillCredentialPaths, open_subscription_chat, settings_from_db, skill_subscription_plan,
};
use super::workbench::SkillWorkbench;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillOptPaths {
    pub skill: PathBuf,
    pub proposals: PathBuf,
    pub database: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillOptCommand {
    /// One explicit round. Does not loop. `--if-due` is the scheduled entry
    /// and no-ops when the interval is zero or has not elapsed.
    Run {
        paths: SkillOptPaths,
        if_due: bool,
    },
    Approve {
        paths: SkillOptPaths,
        version_id: u64,
    },
    Reject {
        paths: SkillOptPaths,
        version_id: u64,
    },
    Rollback {
        paths: SkillOptPaths,
        version_id: u64,
    },
    Status {
        paths: SkillOptPaths,
    },
    /// Records the interval. Does not start a background task.
    Schedule {
        paths: SkillOptPaths,
        interval_secs: u64,
    },
}

pub fn parse_skill_opt_args<I, S>(args: I) -> Result<Option<SkillOptCommand>, SkillOptError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect();
    if args.first().map(String::as_str) != Some("skill-opt") {
        return Ok(None);
    }
    let Some(subcommand) = args.get(1).map(String::as_str) else {
        return Err(SkillOptError::invalid(
            "skill-opt: missing subcommand (run|approve|reject|rollback|status|schedule)",
        ));
    };
    let mut paths = default_paths();
    let mut if_due = false;
    let mut version_id = None;
    let mut interval_secs = None;
    let mut index = 2;
    while index < args.len() {
        let arg = args[index].as_str();
        index += 1;
        match arg {
            "--if-due" => if_due = true,
            "--skill" => {
                paths.skill = PathBuf::from(take_value(&args, &mut index, "--skill")?);
            }
            "--proposals" => {
                paths.proposals = PathBuf::from(take_value(&args, &mut index, "--proposals")?);
            }
            "--db" => {
                paths.database = PathBuf::from(take_value(&args, &mut index, "--db")?);
            }
            "--version" => {
                let raw = take_value(&args, &mut index, "--version")?;
                version_id = Some(raw.parse::<u64>().map_err(|_| {
                    SkillOptError::invalid("skill-opt: --version must be an integer")
                })?);
            }
            "--interval-secs" => {
                let raw = take_value(&args, &mut index, "--interval-secs")?;
                interval_secs = Some(raw.parse::<u64>().map_err(|_| {
                    SkillOptError::invalid("skill-opt: --interval-secs must be an integer")
                })?);
            }
            other if let Some(value) = other.strip_prefix("--skill=") => {
                paths.skill = PathBuf::from(value);
            }
            other if let Some(value) = other.strip_prefix("--proposals=") => {
                paths.proposals = PathBuf::from(value);
            }
            other if let Some(value) = other.strip_prefix("--db=") => {
                paths.database = PathBuf::from(value);
            }
            other if let Some(raw) = other.strip_prefix("--version=") => {
                version_id = Some(raw.parse::<u64>().map_err(|_| {
                    SkillOptError::invalid("skill-opt: --version must be an integer")
                })?);
            }
            other if let Some(raw) = other.strip_prefix("--interval-secs=") => {
                interval_secs = Some(raw.parse::<u64>().map_err(|_| {
                    SkillOptError::invalid("skill-opt: --interval-secs must be an integer")
                })?);
            }
            other => {
                return Err(SkillOptError::invalid(format!(
                    "skill-opt: unknown argument `{other}`"
                )));
            }
        }
    }
    let command = match subcommand {
        "run" => SkillOptCommand::Run { paths, if_due },
        "approve" => SkillOptCommand::Approve {
            paths,
            version_id: require_version(version_id)?,
        },
        "reject" => SkillOptCommand::Reject {
            paths,
            version_id: require_version(version_id)?,
        },
        "rollback" => SkillOptCommand::Rollback {
            paths,
            version_id: require_version(version_id)?,
        },
        "status" => SkillOptCommand::Status { paths },
        "schedule" => SkillOptCommand::Schedule {
            paths,
            interval_secs: interval_secs.ok_or_else(|| {
                SkillOptError::invalid("skill-opt: schedule requires --interval-secs")
            })?,
        },
        other => {
            return Err(SkillOptError::invalid(format!(
                "skill-opt: unknown subcommand `{other}`"
            )));
        }
    };
    Ok(Some(command))
}

pub async fn execute_cli(command: SkillOptCommand) -> Result<i32, SkillOptError> {
    execute_cli_with(command, None).await
}

pub async fn execute_cli_with(
    command: SkillOptCommand,
    chat: Option<&dyn SkillOptChat>,
) -> Result<i32, SkillOptError> {
    match command {
        SkillOptCommand::Run { paths, if_due } => run_command(&paths, if_due, chat).await,
        SkillOptCommand::Approve { paths, version_id } => {
            let mut bench = SkillWorkbench::open(&paths.skill, &paths.proposals)?;
            bench.approve(version_id)?;
            println!("skill-opt: wrote live SKILL.md from version {version_id}");
            Ok(0)
        }
        SkillOptCommand::Reject { paths, version_id } => {
            let mut bench = SkillWorkbench::open(&paths.skill, &paths.proposals)?;
            bench.reject(version_id)?;
            println!(
                "skill-opt: rejected version {version_id}; live SKILL.md was not modified; ledger signals {}",
                bench.registry().signals().len()
            );
            Ok(0)
        }
        SkillOptCommand::Rollback { paths, version_id } => {
            let mut bench = SkillWorkbench::open(&paths.skill, &paths.proposals)?;
            bench.rollback(version_id)?;
            println!("skill-opt: restored live SKILL.md to version {version_id}");
            Ok(0)
        }
        SkillOptCommand::Status { paths } => {
            let bench = SkillWorkbench::open(&paths.skill, &paths.proposals)?;
            println!(
                "skill-opt: pilot={pilot} live_version={live} best_version={best} blocked={blocked} interval_secs={interval} last_run={last:?}",
                pilot = bench.skill_name(),
                live = bench.registry().live_id(),
                best = bench.registry().validation_best_id(),
                blocked = bench.blocked_version_ids().len(),
                interval = bench.schedule().interval_secs,
                last = bench.schedule().last_run_unix,
            );
            Ok(0)
        }
        SkillOptCommand::Schedule {
            paths,
            interval_secs,
        } => {
            let mut bench = SkillWorkbench::open(&paths.skill, &paths.proposals)?;
            bench.set_schedule_interval(interval_secs)?;
            println!(
                "skill-opt: recorded interval_secs={interval_secs}; no background task was started"
            );
            Ok(0)
        }
    }
}

async fn run_command(
    paths: &SkillOptPaths,
    if_due: bool,
    chat: Option<&dyn SkillOptChat>,
) -> Result<i32, SkillOptError> {
    let mut bench = SkillWorkbench::open(&paths.skill, &paths.proposals)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    if if_due && !bench.is_due(now) {
        println!("skill-opt: schedule disabled or not due; no round started");
        return Ok(0);
    }
    let report = if let Some(chat) = chat {
        bench.run_once_at(chat, now).await?
    } else {
        let settings = settings_from_db(&paths.database)?;
        let plan = skill_subscription_plan(&settings)?;
        let chat = open_subscription_chat(&SkillCredentialPaths::from_env(), plan).await?;
        bench.run_once_at(&chat, now).await?
    };
    println!(
        "skill-opt: trigger={trigger} stop={stop:?} calls={calls} live_write_allowed={allowed} proposed_version={version} score={score:?}",
        trigger = report.trigger,
        stop = report.stop,
        calls = report.calls_used,
        allowed = report.live_write_allowed,
        version = report.proposed_version_id,
        score = report.proposed_score,
    );
    if report.live_write_allowed {
        println!(
            "skill-opt: proposal only; live SKILL.md was not modified. Approve with: skill-opt approve --version {}",
            report.proposed_version_id
        );
    } else {
        println!(
            "skill-opt: budget fail-closed; live SKILL.md was not modified and the new versions cannot be approved"
        );
    }
    Ok(0)
}

pub fn default_paths() -> SkillOptPaths {
    SkillOptPaths {
        skill: PathBuf::from(".agents/skills/pilot-web-search/SKILL.md"),
        proposals: PathBuf::from("skill-opt-proposals"),
        database: std::env::var("MEMORY_DB_PATH")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./aegis_runtime_memory.db")),
    }
}

fn require_version(version_id: Option<u64>) -> Result<u64, SkillOptError> {
    version_id.ok_or_else(|| SkillOptError::invalid("skill-opt: --version is required"))
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, SkillOptError> {
    let Some(value) = args.get(*index) else {
        return Err(SkillOptError::invalid(format!(
            "skill-opt: {flag} needs a value"
        )));
    };
    *index += 1;
    Ok(value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_opt::OptPhase;
    use crate::skill_opt::llm::{SkillOptFuture, SkillOptRequest};
    use crate::skill_opt::pilot::BASELINE_SKILL_MARKDOWN;
    struct PanicIfCalled;

    impl SkillOptChat for PanicIfCalled {
        fn complete(
            &self,
            _request: SkillOptRequest,
        ) -> SkillOptFuture<'_, Result<String, SkillOptError>> {
            Box::pin(async { Err(SkillOptError::invalid("schedule must not call the model")) })
        }
    }

    struct Scripted;

    impl SkillOptChat for Scripted {
        fn complete(
            &self,
            request: SkillOptRequest,
        ) -> SkillOptFuture<'_, Result<String, SkillOptError>> {
            let phase = request.phase;
            Box::pin(async move {
                let text = match phase {
                    OptPhase::Propose => format!(
                        "INSTRUCTION:\n{}\nCite the official page.\n",
                        BASELINE_SKILL_MARKDOWN.trim_end()
                    ),
                    OptPhase::Evaluate => {
                        r#"{"dimensions":{"sources":0.95,"precision":0.9,"brevity":0.8},"notes":"ok"}"#
                            .to_string()
                    }
                    OptPhase::FailureAnalyst => "FAILURE:\nnone".to_string(),
                    OptPhase::SuccessAnalyst => "SUCCESS:\nkeep sources".to_string(),
                    OptPhase::Distill => "INSTRUCTION:\nshort".to_string(),
                };
                Ok(text)
            })
        }
    }

    fn paths(root: &std::path::Path) -> SkillOptPaths {
        let skill = root.join(".agents/skills/pilot-web-search/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(&skill, BASELINE_SKILL_MARKDOWN).unwrap();
        SkillOptPaths {
            skill,
            proposals: root.join("skill-opt-proposals"),
            database: root.join("missing-settings.db"),
        }
    }

    #[test]
    fn ordinary_boot_args_are_not_a_trigger() {
        assert!(
            parse_skill_opt_args(std::iter::empty::<String>())
                .unwrap()
                .is_none()
        );
        assert!(parse_skill_opt_args(["--print-pubkey"]).unwrap().is_none());
    }

    #[test]
    fn parse_run_and_approve() {
        let command = parse_skill_opt_args([
            "skill-opt",
            "run",
            "--if-due",
            "--skill",
            ".agents/skills/pilot-web-search/SKILL.md",
        ])
        .unwrap()
        .unwrap();
        match command {
            SkillOptCommand::Run { if_due, paths } => {
                assert!(if_due);
                assert_eq!(
                    paths.skill,
                    PathBuf::from(".agents/skills/pilot-web-search/SKILL.md")
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(parse_skill_opt_args(["skill-opt", "approve"]).is_err());
        assert!(parse_skill_opt_args(["skill-opt", "nope"]).is_err());
    }

    #[tokio::test]
    async fn if_due_with_a_zero_interval_does_not_call_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let command = SkillOptCommand::Run {
            paths: paths(dir.path()),
            if_due: true,
        };
        let code = execute_cli_with(command, Some(&PanicIfCalled))
            .await
            .unwrap();
        assert_eq!(code, 0);
        let live =
            std::fs::read_to_string(dir.path().join(".agents/skills/pilot-web-search/SKILL.md"))
                .unwrap();
        assert_eq!(live, BASELINE_SKILL_MARKDOWN);
    }

    #[tokio::test]
    async fn cli_run_with_a_scripted_chat_does_not_touch_the_live_file() {
        let dir = tempfile::tempdir().unwrap();
        let command = SkillOptCommand::Run {
            paths: paths(dir.path()),
            if_due: false,
        };
        let code = execute_cli_with(command, Some(&Scripted)).await.unwrap();
        assert_eq!(code, 0);
        let live =
            std::fs::read_to_string(dir.path().join(".agents/skills/pilot-web-search/SKILL.md"))
                .unwrap();
        assert_eq!(live, BASELINE_SKILL_MARKDOWN);
        assert!(dir.path().join("skill-opt-proposals/report.json").is_file());
    }

    #[tokio::test]
    async fn cli_run_without_a_grant_fails_closed_before_a_live_write() {
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join(".agents/skills/pilot-web-search/SKILL.md");
        let command = SkillOptCommand::Run {
            paths: paths(dir.path()),
            if_due: false,
        };
        let err = execute_cli(command)
            .await
            .expect_err("default settings do not grant skill optimization");
        assert!(err.to_string().contains("failing closed"));
        assert_eq!(
            std::fs::read_to_string(skill_path).unwrap(),
            BASELINE_SKILL_MARKDOWN
        );
    }

    #[tokio::test]
    async fn schedule_command_does_not_start_a_round() {
        let dir = tempfile::tempdir().unwrap();
        let command = SkillOptCommand::Schedule {
            paths: paths(dir.path()),
            interval_secs: 86_400,
        };
        execute_cli(command).await.unwrap();
        let bench = SkillWorkbench::open(
            dir.path().join(".agents/skills/pilot-web-search/SKILL.md"),
            dir.path().join("skill-opt-proposals"),
        )
        .unwrap();
        assert_eq!(bench.schedule().interval_secs, 86_400);
        assert!(bench.schedule().last_run_unix.is_none());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".agents/skills/pilot-web-search/SKILL.md"))
                .unwrap(),
            BASELINE_SKILL_MARKDOWN
        );
    }
}
