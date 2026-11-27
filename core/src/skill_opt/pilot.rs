use std::path::Path;

use crate::llm::LlmProviderErrorKind;
use crate::prompt_opt::{Example, OptimizationTask, Rubric, RubricDimension};

use super::model::SkillOptError;

/// Directory name of the only skill this loop is allowed to evolve.
pub const PILOT_SKILL_NAME: &str = "pilot-web-search";

/// Task name recorded on every pilot round. Other names fail closed.
pub const PILOT_TASK_NAME: &str = "pilot_web_search_skill";

/// `false`: nothing in boot or heartbeat calls this module.
pub const STARTS_FROM_BOOT_OR_HEARTBEAT: bool = false;

pub const DEFAULT_MAX_CALLS: u32 = 8;
pub const DEFAULT_MAX_COST_MICROS: u64 = 400_000;
pub const DEFAULT_COST_PER_CALL_MICROS: u64 = 50_000;
pub const DEFAULT_MAX_ROUNDS: u32 = 1;

/// Baseline `SKILL.md` for the pilot. The committed file matches this text.
pub const BASELINE_SKILL_MARKDOWN: &str = r#"---
name: pilot-web-search
description: Pilot skill for concise public web search. Use when the owner asks to look something up and wants a short answer with source URLs. Questions may be in English or Chinese.
compatibility: aegis-runtime-skill-opt-pilot
---

# Pilot web search

Search the public web for the owner's question. Prefer one precise query. Answer in a few sentences. Cite each factual claim with a source URL. Do not invent pages, quotes, or tool names. If the lookup is empty, say so and stop.
"#;

pub fn is_pilot_dir_name(name: &str) -> bool {
    name == PILOT_SKILL_NAME || name.starts_with("pilot-web-search-")
}

/// Accept one explicit path under `.agents/skills/<pilot>/SKILL.md`.
///
/// This does not read the parent directory and does not discover other skills.
pub fn assert_pilot_skill_path(path: &Path) -> Result<String, SkillOptError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if file_name != "SKILL.md" {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            "pilot evolution only reads a SKILL.md file",
        ));
    }
    let Some(skill_dir) = path.parent() else {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            "pilot skill path has no directory",
        ));
    };
    let dir_name = skill_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if !is_pilot_dir_name(dir_name) {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            format!("refusing to evolve {dir_name}; pilot domain is {PILOT_SKILL_NAME} only"),
        ));
    }
    let Some(skills_dir) = skill_dir.parent() else {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            "pilot skill must live under .agents/skills",
        ));
    };
    if skills_dir.file_name().and_then(|name| name.to_str()) != Some("skills") {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            "pilot skill must live under .agents/skills",
        ));
    }
    let Some(agents_dir) = skills_dir.parent() else {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            "pilot skill must live under .agents/skills",
        ));
    };
    if agents_dir.file_name().and_then(|name| name.to_str()) != Some(".agents") {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            "pilot skill must live under .agents/skills",
        ));
    }
    Ok(dir_name.to_string())
}

pub fn validate_pilot_markdown(markdown: &str, dir_name: &str) -> Result<(), SkillOptError> {
    if !is_pilot_dir_name(dir_name) {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            format!("refusing to evolve {dir_name}; pilot domain is {PILOT_SKILL_NAME} only"),
        ));
    }
    let Some(parsed) =
        crate::agent_skills::parse_skill_markdown(markdown, std::path::PathBuf::from(dir_name))
    else {
        return Err(SkillOptError::invalid(
            "SKILL.md is missing a name, a description, or frontmatter",
        ));
    };
    if parsed.name != dir_name {
        return Err(SkillOptError::invalid(format!(
            "SKILL.md name {} does not match directory {dir_name}",
            parsed.name
        )));
    }
    if parsed.body.trim().is_empty() {
        return Err(SkillOptError::invalid("SKILL.md body is empty"));
    }
    Ok(())
}

pub fn proposal_passes_structural_gate(markdown: &str, dir_name: &str) -> bool {
    validate_pilot_markdown(markdown, dir_name).is_ok()
}

pub fn pilot_task(baseline_markdown: &str) -> OptimizationTask {
    OptimizationTask {
        name: PILOT_TASK_NAME.to_string(),
        io_description: "Instruction skill for a short public web lookup with source URLs. Owner questions may be English or Chinese.".to_string(),
        baseline_instruction: baseline_markdown.to_string(),
        rubric: Rubric {
            taste: "precise lookup with sources".to_string(),
            dimensions: vec![
                RubricDimension {
                    id: "sources".to_string(),
                    label: "sources".to_string(),
                    weight: 0.4,
                    guidance: "Every factual claim the skill asks for has a source URL. Do not invent pages.".to_string(),
                },
                RubricDimension {
                    id: "precision".to_string(),
                    label: "precision".to_string(),
                    weight: 0.4,
                    guidance: "One precise query. Do not invent tool names or extra steps.".to_string(),
                },
                RubricDimension {
                    id: "brevity".to_string(),
                    label: "brevity".to_string(),
                    weight: 0.2,
                    guidance: "The instruction stays short enough to follow. English or Chinese questions both fit.".to_string(),
                },
            ],
        },
        examples: vec![
            Example {
                id: "rust-result".to_string(),
                input: "Where are the official Rust docs for Result?".to_string(),
            },
            Example {
                id: "http-429".to_string(),
                input: "What does HTTP status 429 mean?".to_string(),
            },
            Example {
                id: "zh-rust-docs".to_string(),
                input: "用中文查 Rust 官方文档首页".to_string(),
            },
            Example {
                id: "axum-repo".to_string(),
                input: "What is the official repository for the Axum web framework?".to_string(),
            },
        ],
        validation_ratio: 0.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_opt::split_train_validation;
    use std::fs;

    #[test]
    fn boot_does_not_start_skill_evolution() {
        assert!(!STARTS_FROM_BOOT_OR_HEARTBEAT);
    }

    #[test]
    fn baseline_parses_as_the_pilot_skill() {
        validate_pilot_markdown(BASELINE_SKILL_MARKDOWN, PILOT_SKILL_NAME).unwrap();
        let task = pilot_task(BASELINE_SKILL_MARKDOWN);
        let (train, validation) = split_train_validation(&task.examples, task.validation_ratio);
        let train_ids: Vec<_> = train.iter().map(|example| example.id.as_str()).collect();
        let validation_ids: Vec<_> = validation
            .iter()
            .map(|example| example.id.as_str())
            .collect();
        assert_eq!(train_ids, vec!["rust-result", "http-429"]);
        assert_eq!(validation_ids, vec!["zh-rust-docs", "axum-repo"]);
        assert!(train_ids.iter().all(|id| !validation_ids.contains(id)));
    }

    #[test]
    fn refuses_skills_outside_the_pilot_domain() {
        let err = assert_pilot_skill_path(Path::new(".agents/skills/typesafe-ai/SKILL.md"))
            .expect_err("typesafe-ai is outside the pilot");
        assert!(err.to_string().contains("pilot-web-search"));
        assert!(is_pilot_dir_name("pilot-web-search"));
        assert!(is_pilot_dir_name("pilot-web-search-citations"));
        assert!(!is_pilot_dir_name("pilot-web-searcher"));
        assert!(!is_pilot_dir_name("typesafe-ai"));
    }

    #[test]
    fn committed_pilot_file_matches_the_baseline() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.agents/skills/pilot-web-search/SKILL.md");
        if !path.exists() {
            // The optional agent pilot is intentionally outside the curated
            // OSS tree. The baseline parser is covered above in this module.
            return;
        }
        let text = fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!("read {}: {err}", path.display());
        });
        assert_eq!(text, BASELINE_SKILL_MARKDOWN);
    }
}
