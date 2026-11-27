//! Telegram bot menu management — custom commands, merging, ranking, and sync.
//!
//! Implemented as `impl Orchestrator` methods in a separate file for code organization.
//! Also defines `CommandSource`, `CustomCommand`, and the `retain_non_user_commands` helper.

use anyhow::Result;
use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestToolMessageArgs,
};
use serde_json::Value;

use crate::orchestrator::Orchestrator;

// ── Menu command types ──────────────────────────────────────────────────────

/// Source of a custom bot command — typed enum instead of raw strings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(into = "String")]
pub enum CommandSource {
    User,
    Skill(String),
}

impl CommandSource {
    pub fn is_skill(&self) -> bool {
        matches!(self, Self::Skill(_))
    }
    pub fn is_user(&self) -> bool {
        matches!(self, Self::User)
    }
    pub fn skill_name(&self) -> Option<&str> {
        match self {
            Self::Skill(n) => Some(n),
            _ => None,
        }
    }
}

impl From<CommandSource> for String {
    fn from(s: CommandSource) -> String {
        match s {
            CommandSource::User => "user".to_string(),
            CommandSource::Skill(n) => format!("skill:{n}"),
        }
    }
}

impl<'de> serde::Deserialize<'de> for CommandSource {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(CommandSource::parse(&s))
    }
}

/// Prefix of the `panel:<name>` sources written by the Smart Panel feature
/// (retired 2026-09-16). No variant exists for them any more; persisted rows
/// carrying this prefix are dropped by [`CustomCommand::from_value`].
const RETIRED_PANEL_SOURCE_PREFIX: &str = "panel:";

impl CommandSource {
    /// Parse a persisted source string. Unknown prefixes (including the
    /// retired `panel:` one) degrade to [`CommandSource::User`].
    pub(crate) fn parse(s: &str) -> Self {
        if let Some(name) = s.strip_prefix("skill:") {
            Self::Skill(name.to_string())
        } else {
            Self::User
        }
    }
}

/// A custom bot command with typed source — replaces raw `serde_json::Value`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CustomCommand {
    pub command: String,
    pub description: String,
    pub source: CommandSource,
}

impl CustomCommand {
    pub(crate) fn new(
        command: impl Into<String>,
        description: impl Into<String>,
        source: CommandSource,
    ) -> Self {
        Self {
            command: command.into(),
            description: description.into(),
            source,
        }
    }

    /// Convert to the Value format used by merge/display functions.
    pub(crate) fn to_menu_value(&self) -> Value {
        serde_json::json!({
            "command": self.command,
            "description": self.description,
        })
    }

    /// Parse from a JSON Value (typed rows and legacy DB rows share this shape).
    ///
    /// Returns `None` for rows without a command and for rows sourced from
    /// the retired Smart Panel feature (`"source": "panel:…"`): nothing
    /// handles those slash commands any more, so they are dropped instead of
    /// being re-advertised as user commands.
    pub(crate) fn from_value(v: &Value) -> Option<Self> {
        let command = v.get("command").and_then(|v| v.as_str())?.to_string();
        let source_str = v.get("source").and_then(|v| v.as_str());
        if let Some(s) = source_str
            && s.starts_with(RETIRED_PANEL_SOURCE_PREFIX)
        {
            tracing::info!(command = %command, source = s, "Dropping retired panel menu command");
            return None;
        }
        let description = v
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let source = source_str
            .map(CommandSource::parse)
            .unwrap_or(CommandSource::User);
        Some(Self {
            command,
            description,
            source,
        })
    }
}

/// Helper: filter out user-sourced commands, keep skill-sourced ones.
pub(crate) fn retain_non_user_commands(cmds: &mut Vec<CustomCommand>) {
    cmds.retain(|c| !c.source.is_user());
}

// ── Orchestrator menu methods ───────────────────────────────────────────────

impl Orchestrator {
    /// Load custom commands from DB (excludes core commands).
    pub(crate) async fn load_custom_commands(&self) -> Vec<CustomCommand> {
        // The typed format (`Vec<CustomCommand>` with string sources) and the
        // legacy raw-Value format share one wire shape, so a single lenient
        // pass through `from_value` covers both — and lets it drop rows that
        // were sourced from the retired Smart Panel feature.
        if let Ok(Some(data)) = self
            .memory
            .load_skill_state("system", "custom_bot_commands")
            .await
            && let Ok(vals) = serde_json::from_str::<Vec<Value>>(&data)
        {
            return vals.iter().filter_map(CustomCommand::from_value).collect();
        }
        // Migration: try old "bot_commands" key and extract non-core entries
        if let Ok(Some(data)) = self.memory.load_skill_state("system", "bot_commands").await
            && let Ok(cmds) = serde_json::from_str::<Vec<Value>>(&data)
        {
            let core_names = crate::telegram::core_command_names();
            return cmds
                .iter()
                .filter_map(CustomCommand::from_value)
                .filter(|c| !core_names.contains(c.command.as_str()))
                .collect();
        }
        Vec::new()
    }

    /// Merge core commands with custom commands into a full menu.
    /// Optionally sorts by usage frequency (most-used core commands first,
    /// most-used custom commands first).
    pub(crate) fn merge_core_and_custom_ranked(
        &self,
        custom: &[CustomCommand],
        usage: &[(String, u64)],
    ) -> Vec<Value> {
        let usage_map: std::collections::HashMap<&str, u64> = usage
            .iter()
            .map(|(cmd, count)| (cmd.as_str(), *count))
            .collect();

        // Core commands sorted by usage (most-used first)
        let mut core: Vec<Value> = crate::telegram::core_commands()
            .into_iter()
            .map(|cmd| {
                serde_json::json!({
                    "command": cmd.command,
                    "description": cmd.description,
                })
            })
            .collect();
        core.sort_by(|a, b| {
            let ca = a.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let cb = b.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let ua = usage_map.get(ca).copied().unwrap_or(0);
            let ub = usage_map.get(cb).copied().unwrap_or(0);
            ub.cmp(&ua) // descending
        });

        let core_names = crate::telegram::core_command_names();

        // Custom commands sorted by usage
        let mut custom_entries: Vec<Value> = custom
            .iter()
            .filter(|c| !c.command.is_empty() && !core_names.contains(c.command.as_str()))
            .map(|c| c.to_menu_value())
            .collect();
        custom_entries.sort_by(|a, b| {
            let ca = a.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let cb = b.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let ua = usage_map.get(ca).copied().unwrap_or(0);
            let ub = usage_map.get(cb).copied().unwrap_or(0);
            ub.cmp(&ua)
        });

        core.extend(custom_entries);
        core
    }

    /// Merge without ranking (for HITL proposals where order shouldn't change).
    pub(crate) fn merge_core_and_custom(&self, custom: &[CustomCommand]) -> Vec<Value> {
        self.merge_core_and_custom_ranked(custom, &[])
    }

    /// Build the full menu: core commands + custom commands from DB.
    pub(crate) async fn build_full_menu(&self) -> Vec<Value> {
        let custom = self.load_custom_commands().await;
        self.merge_core_and_custom(&custom)
    }

    /// Build the full menu with adaptive ordering based on usage frequency.
    #[allow(dead_code)]
    pub(crate) async fn build_adaptive_menu(&self, user_id: &str) -> Vec<Value> {
        let custom = self.load_custom_commands().await;
        let usage = self.memory.get_command_usage_stats(user_id).await;
        self.merge_core_and_custom_ranked(&custom, &usage)
    }

    /// Persist custom commands to DB.
    pub(crate) async fn save_custom_commands(&self, custom: &[CustomCommand]) {
        if let Err(e) = self
            .memory
            .store_skill_state(
                "system",
                "custom_bot_commands",
                &serde_json::to_string(custom).unwrap_or_default(),
            )
            .await
        {
            tracing::error!(error = %e, "Failed to persist custom bot commands");
        }
    }

    /// Apply the full menu to Telegram via API.
    pub(crate) async fn apply_menu_to_telegram(&self, full_menu: &[Value]) -> Result<()> {
        if let Some(ref bot) = self.services.telegram_bot {
            use teloxide::prelude::Requester;
            let bot_commands: Vec<teloxide::types::BotCommand> = full_menu
                .iter()
                .filter_map(|c| {
                    let name = c.get("command").and_then(|v| v.as_str())?;
                    let desc = c.get("description").and_then(|v| v.as_str())?;
                    Some(teloxide::types::BotCommand::new(name, desc))
                })
                .collect();
            bot.set_my_commands(bot_commands).await?;
        }
        Ok(())
    }

    /// Sync menu after skill activation/deactivation.
    /// Called from skill (de)activation — updates the Telegram menu automatically (no HITL needed
    /// because skill activation itself already went through HITL approval).
    pub async fn sync_menu_for_skill(
        &self,
        skill_name: &str,
        menu_command: Option<&str>,
        menu_description: Option<&str>,
        activate: bool,
    ) {
        let cmd = match menu_command {
            Some(c) if !c.is_empty() => c.trim_start_matches('/'),
            _ => return, // skill has no menu_command — nothing to sync
        };
        let desc = menu_description.unwrap_or("");
        let core_names = crate::telegram::core_command_names();
        if core_names.contains(cmd) {
            tracing::warn!(
                skill = skill_name,
                command = cmd,
                "Skill tried to register a core command name — ignoring"
            );
            return;
        }
        // Phase 3b: validate command name format
        if !cmd
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            || cmd.is_empty()
            || cmd.len() > 32
        {
            tracing::warn!(
                skill = skill_name,
                command = cmd,
                "Skill menu_command has invalid format — ignoring"
            );
            return;
        }

        // Serialize load→modify→save→apply to prevent lost-update races
        // when multiple skills activate/deactivate concurrently.
        let _guard = self.menu_lock.lock().await;

        let mut custom = self.load_custom_commands().await;
        let expected_source = CommandSource::Skill(skill_name.to_string());

        if activate {
            // Remove any existing entry with same command name, then add
            custom.retain(|c| c.command != cmd);
            custom.push(CustomCommand::new(cmd, desc, expected_source));
            tracing::info!(
                skill = skill_name,
                command = cmd,
                "Menu: added skill command"
            );
        } else {
            // Remove only if source matches this skill
            let before = custom.len();
            custom.retain(|c| !(c.command == cmd && c.source == expected_source));
            if custom.len() < before {
                tracing::info!(
                    skill = skill_name,
                    command = cmd,
                    "Menu: removed skill command"
                );
            }
        }

        // Phase 2a: apply to Telegram FIRST, only save on success
        let usage = self.memory.get_command_usage_stats("__all__").await;
        let full_menu = self.merge_core_and_custom_ranked(&custom, &usage);
        if let Err(e) = self.apply_menu_to_telegram(&full_menu).await {
            tracing::warn!(error = %e, "Failed to sync Telegram menu after skill change — DB not updated");
            return;
        }
        self.save_custom_commands(&custom).await;
    }

    /// Validate a list of command objects from LLM input.
    /// Returns parsed CustomCommands (source=User) on success, or a JSON error string on failure.
    pub(crate) fn validate_menu_commands(
        &self,
        commands: &[Value],
        core_names: &std::collections::HashSet<&str>,
    ) -> Result<Vec<CustomCommand>, String> {
        let mut parsed = Vec::new();
        for cmd in commands {
            let name = cmd.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let desc = cmd
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if name.is_empty() || desc.is_empty() {
                return Err(serde_json::json!({
                    "status": "error",
                    "error": "each command must have non-empty 'command' and 'description' fields",
                })
                .to_string());
            }
            let clean_name = name.trim_start_matches('/');
            if clean_name.len() > 32
                || !clean_name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                return Err(serde_json::json!({
                    "status": "error",
                    "error": format!("command '{}' is invalid: must be 1-32 lowercase letters, digits, or underscores", clean_name),
                }).to_string());
            }
            if desc.len() > 256 {
                return Err(serde_json::json!({
                    "status": "error",
                    "error": format!("description for '{}' exceeds 256 characters", clean_name),
                })
                .to_string());
            }
            if core_names.contains(clean_name) {
                return Err(serde_json::json!({
                    "status": "error",
                    "error": format!("'{}' is a core command and cannot be overridden. Use a different name.", clean_name),
                }).to_string());
            }
            parsed.push(CustomCommand::new(clean_name, desc, CommandSource::User));
        }
        Ok(parsed)
    }

    /// Create a HITL proposal for changing the Telegram bot command menu.
    /// `proposed_custom` is the new custom commands list; `proposed_full` and `current_full`
    /// are the merged (core + custom) lists for display purposes.
    pub(crate) async fn create_menu_proposal(
        &self,
        tool_call_id: &str,
        proposed_custom: Vec<CustomCommand>,
        proposed_full: Vec<Value>,
        current_full: Vec<Value>,
        user_did: &str,
        chat_id: Option<i64>,
        messages: &mut Vec<ChatCompletionRequestMessage>,
    ) -> Result<bool> {
        let approval_id = format!("menu-{:016x}", rand::random::<u64>());

        let proposal = serde_json::json!({
            "approval_id": approval_id,
            "type": "bot_commands",
            "proposed_custom": proposed_custom,
            "proposed_full": proposed_full,
            "current_full": current_full,
            "user_did": user_did,
            "chat_id": chat_id,
            "created_at": chrono::Utc::now().to_rfc3339(),
        });
        let _ = self
            .memory
            .store_skill_state("hitl", &approval_id, &proposal.to_string())
            .await;

        // Build diff display
        let current_set: std::collections::HashMap<String, String> = current_full
            .iter()
            .filter_map(|c| {
                let name = c.get("command").and_then(|v| v.as_str())?.to_string();
                let desc = c.get("description").and_then(|v| v.as_str())?.to_string();
                Some((name, desc))
            })
            .collect();
        let proposed_set: std::collections::HashMap<String, String> = proposed_full
            .iter()
            .filter_map(|c| {
                let name = c.get("command").and_then(|v| v.as_str())?.to_string();
                let desc = c.get("description").and_then(|v| v.as_str())?.to_string();
                Some((name, desc))
            })
            .collect();

        let mut diff_lines = Vec::new();
        // Added or changed
        for (name, desc) in &proposed_set {
            if let Some(old_desc) = current_set.get(name) {
                if old_desc != desc {
                    diff_lines.push(format!("  ~ /{} — {} (was: {})", name, desc, old_desc));
                }
            } else {
                diff_lines.push(format!("  + /{} — {}", name, desc));
            }
        }
        // Removed
        for (name, desc) in &current_set {
            if !proposed_set.contains_key(name) {
                diff_lines.push(format!("  - /{} — {}", name, desc));
            }
        }

        // Paired-Fix (CTO round-3): escape LLM-authored command names +
        // descriptions before rendering into the Telegram HTML approval
        // template. `diff_section` may also carry LLM content.
        let fmt_cmds = |cmds: &[Value]| -> String {
            cmds.iter()
                .map(|c| {
                    let cmd = c.get("command").and_then(|v| v.as_str()).unwrap_or("?");
                    let desc = c.get("description").and_then(|v| v.as_str()).unwrap_or("?");
                    format!(
                        "  /{} — {}",
                        crate::telegram_render::escape_html(cmd),
                        crate::telegram_render::escape_html(desc),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let diff_section = if diff_lines.is_empty() {
            "  (no changes)".to_string()
        } else {
            diff_lines
                .iter()
                .map(|line| crate::telegram_render::escape_html(line))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let approval_id_esc = crate::telegram_render::escape_html(&approval_id);

        let approval_msg = format!(
            "📋 **Bot Menu Change — Approval Request**\n\n\
             Approval ID: `{}`\n\n\
             **Changes:**\n{}\n\n\
             **Full menu (if approved):**\n{}\n\n\
             Reply with:\n\
             • `approve menu {0}` to apply\n\
             • `reject menu {0}` to cancel",
            approval_id_esc,
            diff_section,
            fmt_cmds(&proposed_full)
        );

        if let (Some(tx), Some(cid)) = (&self.progress_tx, chat_id) {
            let _ = tx.try_send(crate::progress_router::ProgressEvent::new(
                cid,
                crate::progress_router::ProgressPayload::NeedApproval {
                    approval_id: Some(approval_id.clone()),
                    tool_name: None,
                    summary: None,
                    approval_html: approval_msg,
                },
            ));
        }

        tracing::info!(
            approval_id = %approval_id,
            num_commands = proposed_full.len(),
            "[HITL] Bot menu change proposal created — suspending ReAct loop"
        );

        let pending_result = format!(
            "[TOOL_RESULT — HITL approval required]\n\
             Status: pending_human_approval\n\
             Approval ID: {}\n\
             Proposed {} commands for Telegram bot menu.\n\
             The proposal has been sent to the user for review via Telegram.\n\
             [/TOOL_RESULT]",
            approval_id,
            proposed_full.len()
        );
        messages.push(
            ChatCompletionRequestToolMessageArgs::default()
                .tool_call_id(tool_call_id)
                .content(&*pending_result)
                .build()?
                .into(),
        );
        messages.push(
            ChatCompletionRequestSystemMessageArgs::default()
                .content(
                    "[SYSTEM] A bot menu change requiring user approval has been \
                     submitted via Telegram. You MUST NOT call any more tools. Tell the user \
                     that the proposed menu change has been sent for review and they can approve or \
                     reject it."
                )
                .build()?
                .into(),
        );

        Ok(false)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_source_roundtrip() {
        // Serialize → Deserialize round-trip
        for source in &[CommandSource::User, CommandSource::Skill("weather".into())] {
            let json = serde_json::to_string(source).unwrap();
            let back: CommandSource = serde_json::from_str(&json).unwrap();
            assert_eq!(*source, back);
        }
    }

    #[test]
    fn command_source_parse_legacy_strings() {
        assert_eq!(CommandSource::parse("user"), CommandSource::User);
        assert_eq!(
            CommandSource::parse("skill:weather"),
            CommandSource::Skill("weather".into())
        );
        // Retired panel sources (2026-09-16) have no variant: the bare parser
        // degrades them to User; `from_value` is what drops such rows.
        assert_eq!(CommandSource::parse("panel:tasks"), CommandSource::User);
        // Unknown strings default to User
        assert_eq!(CommandSource::parse("bogus"), CommandSource::User);
        assert_eq!(CommandSource::parse(""), CommandSource::User);
    }

    #[test]
    fn custom_command_json_roundtrip() {
        let cmd = CustomCommand::new("tasks", "My tasks", CommandSource::Skill("tasks".into()));
        let json = serde_json::to_string(&cmd).unwrap();
        let back: CustomCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back.command, "tasks");
        assert_eq!(back.description, "My tasks");
        assert_eq!(back.source, CommandSource::Skill("tasks".into()));
    }

    #[test]
    fn custom_command_from_legacy_value() {
        // Old format: raw JSON with string source
        let v = serde_json::json!({
            "command": "weather",
            "description": "Check weather",
            "source": "skill:weather_skill"
        });
        let cmd = CustomCommand::from_value(&v).unwrap();
        assert_eq!(cmd.command, "weather");
        assert_eq!(cmd.source, CommandSource::Skill("weather_skill".into()));

        // Legacy without source field → defaults to User
        let v2 = serde_json::json!({"command": "foo", "description": "bar"});
        let cmd2 = CustomCommand::from_value(&v2).unwrap();
        assert_eq!(cmd2.source, CommandSource::User);

        // Missing command → None
        let v3 = serde_json::json!({"description": "bar"});
        assert!(CustomCommand::from_value(&v3).is_none());

        // Rows sourced from the retired Smart Panel feature (2026-09-16) are
        // dropped on load …
        let v4 = serde_json::json!({
            "command": "tasks_panel",
            "description": "📋 Tasks",
            "source": "panel:tasks_panel"
        });
        assert!(
            CustomCommand::from_value(&v4).is_none(),
            "panel-sourced rows must be dropped"
        );
        // … while the same row with a live source still parses (negative control).
        let v5 = serde_json::json!({
            "command": "tasks_panel",
            "description": "📋 Tasks",
            "source": "skill:tasks"
        });
        assert_eq!(
            CustomCommand::from_value(&v5).unwrap().source,
            CommandSource::Skill("tasks".into())
        );
    }

    #[test]
    fn retain_non_user_commands_keeps_skill_commands() {
        let mut cmds = vec![
            CustomCommand::new("a", "user cmd", CommandSource::User),
            CustomCommand::new("b", "skill cmd", CommandSource::Skill("s1".into())),
            CustomCommand::new("c", "other skill cmd", CommandSource::Skill("s2".into())),
            CustomCommand::new("d", "another user", CommandSource::User),
        ];
        retain_non_user_commands(&mut cmds);
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].command, "b");
        assert_eq!(cmds[1].command, "c");
    }

    #[test]
    fn custom_command_to_menu_value() {
        let cmd = CustomCommand::new("tasks", "📋 Tasks", CommandSource::Skill("tasks".into()));
        let v = cmd.to_menu_value();
        assert_eq!(v.get("command").unwrap().as_str().unwrap(), "tasks");
        assert_eq!(v.get("description").unwrap().as_str().unwrap(), "📋 Tasks");
        // Menu value should NOT include source (it's for Telegram API)
        assert!(v.get("source").is_none());
    }
}
