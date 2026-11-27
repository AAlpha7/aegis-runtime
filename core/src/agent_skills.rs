use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// An external Agent Skill discovered from SKILL.md files.
/// These are NOT Wasm skills — they are instruction-based skills
/// whose content is injected into the LLM context on activation.
#[derive(Debug, Clone)]
pub struct AgentSkill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub skill_dir: PathBuf,
    pub compatibility: Option<String>,
}

/// Scan standard Agent Skills directories and return discovered skills.
/// Scans: .agents/skills/ (project-level) and ~/.agents/skills/ (user-level)
/// Project-level skills override user-level on name collision.
pub fn discover_agent_skills(project_root: &Path) -> Vec<AgentSkill> {
    let mut skills_by_name: HashMap<String, AgentSkill> = HashMap::new();

    // 1. User-level: ~/.agents/skills/*/SKILL.md (lowest priority)
    if let Some(home) = dirs_path() {
        let user_skills_dir = home.join(".agents").join("skills");
        scan_skills_dir(&user_skills_dir, &mut skills_by_name);
    }

    // 2. Project-level: <project_root>/.agents/skills/*/SKILL.md (highest priority)
    let project_skills_dir = project_root.join(".agents").join("skills");
    scan_skills_dir(&project_skills_dir, &mut skills_by_name);

    let mut skills: Vec<AgentSkill> = skills_by_name.into_values().collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Scan a single skills directory for SKILL.md files.
fn scan_skills_dir(dir: &Path, skills: &mut HashMap<String, AgentSkill>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return, // directory doesn't exist — skip silently
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Skip common non-skill directories
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if skill_md.exists()
            && let Some(skill) = parse_skill_md(&skill_md)
        {
            tracing::info!(
                name = %skill.name,
                dir = %path.display(),
                "Discovered external Agent Skill"
            );
            skills.insert(skill.name.clone(), skill);
        }
    }
}

/// Parse a single SKILL.md file into an AgentSkill.
/// Returns None if YAML frontmatter is missing or description is empty.
pub fn parse_skill_md(path: &Path) -> Option<AgentSkill> {
    let content = std::fs::read_to_string(path).ok()?;
    let skill_dir = path.parent()?.to_path_buf();
    parse_skill_markdown(&content, skill_dir)
}

/// Parse SKILL.md text already in memory.
///
/// `skill_dir` is the directory that contains the file. The evolution pilot
/// uses this to validate a proposal before any live write.
pub fn parse_skill_markdown(content: &str, skill_dir: PathBuf) -> Option<AgentSkill> {
    // Parse YAML frontmatter (delimited by --- lines)
    if !content.starts_with("---") {
        return None;
    }

    let after_first = &content[3..];
    let end_idx = after_first.find("\n---")?;
    let frontmatter = &after_first[..end_idx];
    let body = after_first[end_idx + 4..].trim().to_string();

    // Simple YAML key extraction (avoid adding a YAML dependency)
    let name = extract_yaml_value(frontmatter, "name")?;
    let description = extract_yaml_value(frontmatter, "description")?;

    if name.is_empty() || description.is_empty() || description.len() > 1024 {
        return None;
    }

    let compatibility = extract_yaml_value(frontmatter, "compatibility");

    Some(AgentSkill {
        name,
        description,
        body,
        skill_dir,
        compatibility,
    })
}

/// Extract a simple string value from YAML frontmatter.
/// Handles: `key: value` and `key: "value"` (single line only).
fn extract_yaml_value(yaml: &str, key: &str) -> Option<String> {
    let prefix = format!("{}:", key);
    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&prefix) {
            let value = trimmed[prefix.len()..].trim();
            // Strip surrounding quotes if present
            let unquoted = if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                &value[1..value.len() - 1]
            } else {
                value
            };
            if !unquoted.is_empty() {
                return Some(unquoted.to_string());
            }
        }
    }
    None
}

/// Format discovered skills as a catalog section for the system prompt.
/// Only includes name + description (Tier 1: ~50-100 tokens per skill).
pub fn format_skill_catalog(skills: &[AgentSkill]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    lines.push("<agent_skills>".to_string());
    lines.push(
        "The following external Agent Skills are available. When a task matches a skill's \
         description, you can request its full instructions by calling `activate_agent_skill` \
         with the skill name."
            .to_string(),
    );
    lines.push(String::new());
    for skill in skills {
        lines.push(format!("- {}: {}", skill.name, skill.description));
    }
    lines.push("</agent_skills>".to_string());
    lines.join("\n")
}

/// Get home directory path (cross-platform).
fn dirs_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_test_skill(dir: &Path, name: &str, description: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let content = format!(
            "---\nname: {name}\ndescription: {description}\ncompatibility: test\n---\n\n# {name}\n\nTest skill body."
        );
        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn test_parse_skill_md_valid() {
        let dir = tempfile::tempdir().unwrap();
        create_test_skill(dir.path(), "test-greeting", "A greeting skill for testing");

        let skill = parse_skill_md(&dir.path().join("test-greeting/SKILL.md"));
        assert!(skill.is_some(), "valid SKILL.md should parse");
        let skill = skill.unwrap();
        assert_eq!(skill.name, "test-greeting");
        assert_eq!(skill.description, "A greeting skill for testing");
        assert_eq!(skill.compatibility, Some("test".to_string()));
        assert!(skill.body.contains("Test skill body"));
    }

    #[test]
    fn test_parse_skill_md_missing_description() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("bad-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: bad-skill\n---\n\nBody.",
        )
        .unwrap();

        let skill = parse_skill_md(&skill_dir.join("SKILL.md"));
        assert!(skill.is_none(), "missing description should return None");
    }

    #[test]
    fn test_parse_skill_md_no_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("no-fm");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "# Just a markdown file\n\nNo frontmatter.",
        )
        .unwrap();

        let skill = parse_skill_md(&skill_dir.join("SKILL.md"));
        assert!(skill.is_none(), "no frontmatter should return None");
    }

    #[test]
    fn test_discover_agent_skills() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join(".agents").join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        create_test_skill(&skills_dir, "test-alpha", "Alpha skill");
        create_test_skill(&skills_dir, "test-beta", "Beta skill");

        let skills = discover_agent_skills(dir.path());
        assert_eq!(skills.len(), 2, "should discover 2 skills");
        // Sorted by name
        assert_eq!(skills[0].name, "test-alpha");
        assert_eq!(skills[1].name, "test-beta");
    }

    #[test]
    fn test_skill_catalog_format() {
        let skills = vec![
            AgentSkill {
                name: "web-scraping".to_string(),
                description: "Advanced web scraping".to_string(),
                body: String::new(),
                skill_dir: PathBuf::new(),
                compatibility: None,
            },
            AgentSkill {
                name: "data-analysis".to_string(),
                description: "Analyze datasets".to_string(),
                body: String::new(),
                skill_dir: PathBuf::new(),
                compatibility: None,
            },
        ];

        let catalog = format_skill_catalog(&skills);
        assert!(catalog.contains("<agent_skills>"), "must have opening tag");
        assert!(catalog.contains("</agent_skills>"), "must have closing tag");
        assert!(catalog.contains("- web-scraping: Advanced web scraping"));
        assert!(catalog.contains("- data-analysis: Analyze datasets"));
        assert!(catalog.contains("activate_agent_skill"));
    }

    #[test]
    fn test_skill_catalog_empty() {
        let catalog = format_skill_catalog(&[]);
        assert!(
            catalog.is_empty(),
            "empty skills should produce empty catalog"
        );
    }

    #[test]
    fn test_extract_yaml_quoted_value() {
        let yaml = "name: \"my-skill\"\ndescription: 'A test skill'";
        assert_eq!(
            extract_yaml_value(yaml, "name"),
            Some("my-skill".to_string())
        );
        assert_eq!(
            extract_yaml_value(yaml, "description"),
            Some("A test skill".to_string())
        );
    }

    #[test]
    fn test_discover_skips_dotdirs() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join(".agents").join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        create_test_skill(&skills_dir, ".hidden", "Should be skipped");
        create_test_skill(&skills_dir, "visible", "Should be found");

        let skills = discover_agent_skills(dir.path());
        assert_eq!(skills.len(), 1, "hidden dirs should be skipped");
        assert_eq!(skills[0].name, "visible");
    }
}
