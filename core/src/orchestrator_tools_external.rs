//! External executor tool handler (`run_external`).
//!
//! Runs one task on the selected subscription in a fresh scratch directory.
//! Automatic routing follows Grok/Codex chat; cross-provider fallback is off
//! by default. Connector requests require Codex access discovery.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_openai::types::chat::ChatCompletionMessageToolCall;
use serde_json::Value;

use crate::external_executor::{
    CodexAppServerExecutor, ExecutorConfig, ExecutorKind, ExecutorReceipt, ExecutorRequest,
    ExecutorUpdate, ExternalExecutor, GrokAcpExecutor, ProcessTransport,
};
use crate::orchestrator::{Orchestrator, ToolContext};
use crate::progress_router::{OwnerNoticeKind, ProgressEvent, ProgressPayload};
use crate::utils::truncate_str;

/// Skill-state namespace holding one run record per `run_id`.
/// Listed in `memory_migrations::INTERNAL_SKILL_STATE_NAMES_SQL`, otherwise
/// the startup orphan sweep deletes it (it is not a Wasm skill).
pub const EXTERNAL_RUN_SKILL_STATE: &str = "external_executor";

const MAX_TASK_CHARS: usize = 20_000;
const MAX_FINAL_TEXT_CHARS: usize = 12_000;
const MAX_ERROR_CHARS: usize = 1_000;
const TASK_PREVIEW_CHARS: usize = 500;
const LAST_TOOL_CALLS: usize = 10;
const TIMEOUT_MIN_SECS: u64 = 30;
const TIMEOUT_MAX_SECS: u64 = 3600;
const DEFAULT_RUNS_DIR: &str = "/opt/aegis-runtime/shared/executor-runs";

pub struct ExternalExecutors {
    pub grok: Arc<dyn ExternalExecutor>,
    pub codex: Arc<dyn ExternalExecutor>,
    /// Parent of per-run scratch directories (`<runs_dir>/<run_id>`).
    pub runs_dir: PathBuf,
    pub grok_bin: String,
    pub codex_bin: String,
}

impl ExternalExecutors {
    /// Real binaries via `ProcessTransport` + `ExecutorConfig::from_env()`; `runs_dir` from
    /// `AEGIS_EXECUTOR_RUNS_DIR` (default `/opt/aegis-runtime/shared/executor-runs`).
    pub fn from_env() -> Self {
        Self::from_config(ExecutorConfig::from_env())
    }

    pub fn from_settings(settings: &crate::settings::RuntimeSettings) -> Self {
        let mut config = ExecutorConfig::from_env();
        // Reuse the login directory selected by setup-auth. An explicit executor
        // environment override remains authoritative.
        for (env, setting, target) in [
            (
                "AEGIS_EXECUTOR_CODEX_BIN",
                "codex_cli_binary",
                &mut config.codex_bin,
            ),
            (
                "AEGIS_EXECUTOR_CODEX_MODEL",
                "codex_model",
                &mut config.codex_model,
            ),
        ] {
            if std::env::var(env).ok().is_none_or(|v| v.trim().is_empty())
                && let Some(value) = settings
                    .get_if_stored(setting)
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .filter(|v| !v.is_empty())
            {
                *target = value;
            }
        }
        if config.codex_home.is_none() {
            config.codex_home = settings
                .get("codex_cli_home")
                .and_then(|v| v.as_str().map(str::to_owned))
                .filter(|v| !v.is_empty());
        }
        Self::from_config(config)
    }

    fn from_config(config: ExecutorConfig) -> Self {
        let transport: Arc<dyn crate::external_executor::AgentTransport> =
            Arc::new(ProcessTransport);
        let runs_dir = std::env::var("AEGIS_EXECUTOR_RUNS_DIR")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_RUNS_DIR));
        Self {
            grok_bin: config.grok_bin.clone(),
            codex_bin: config.codex_bin.clone(),
            grok: Arc::new(GrokAcpExecutor::new(config.clone(), Arc::clone(&transport))),
            codex: Arc::new(CodexAppServerExecutor::new(config, transport)),
            runs_dir,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NamedExecutor {
    Grok,
    Codex,
}

impl NamedExecutor {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "grok" => Some(Self::Grok),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Grok => "grok",
            Self::Codex => "codex",
        }
    }

    fn pick(self, execs: &ExternalExecutors) -> Arc<dyn ExternalExecutor> {
        match self {
            Self::Grok => Arc::clone(&execs.grok),
            Self::Codex => Arc::clone(&execs.codex),
        }
    }
}

impl Orchestrator {
    pub fn set_external_executors(&self, executors: Arc<ExternalExecutors>) {
        *self.services.external_executors.lock() = Some(executors);
    }

    /// Handle the `run_external` pseudo-tool.
    pub(crate) async fn handle_tool_external(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "run_external" => execute_run_external,
        );
        Ok(None)
    }

    async fn execute_run_external(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        if !self.settings.is_enabled("external_executor_enabled") {
            return external_error(
                "not_configured",
                "external executor is disabled",
                false,
                "ask the owner to enable external_executor_enabled",
            );
        }
        let Some(executors) = self.services.external_executors.lock().clone() else {
            return external_error(
                "not_available",
                "external executor is not available on this instance",
                false,
                "restart after wiring run_external; do not claim the task ran",
            );
        };

        let task = args
            .get("task")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if task.is_empty() {
            return external_error(
                "invalid_input",
                "task is required and must be non-empty",
                false,
                "pass the full self-contained instruction in task",
            );
        }
        if task.len() > MAX_TASK_CHARS {
            return external_error(
                "invalid_input",
                &format!("task exceeds {MAX_TASK_CHARS} characters"),
                false,
                "shorten the task; put extra material in context only if needed",
            );
        }
        let executor_arg = args
            .get("executor")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("auto");
        if !matches!(executor_arg, "auto" | "grok" | "codex") {
            return external_error(
                "invalid_input",
                &format!("unknown executor '{executor_arg}'"),
                false,
                "valid executor values: auto, grok, codex",
            );
        }
        let default_timeout = self.settings.get_u64_clamped(
            "external_executor_timeout_secs",
            600,
            TIMEOUT_MIN_SECS,
            TIMEOUT_MAX_SECS,
        );
        let timeout_secs = match args.get("timeout_secs") {
            None | Some(Value::Null) => default_timeout,
            Some(v) => {
                let parsed = v
                    .as_u64()
                    .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()));
                match parsed {
                    Some(n) => n.clamp(TIMEOUT_MIN_SECS, TIMEOUT_MAX_SECS),
                    None => {
                        return external_error(
                            "invalid_input",
                            "timeout_secs must be an integer",
                            false,
                            "pass timeout_secs between 30 and 3600, or omit it",
                        );
                    }
                }
            }
        };

        let primary_setting = self
            .settings
            .get("external_executor_primary")
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "grok".to_string());
        let fallback_setting = self
            .settings
            .get("external_executor_fallback")
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "codex".to_string());
        let connector = args
            .get("connector")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if connector.is_some() && executor_arg == "grok" {
            return external_error(
                "invalid_input",
                "Connector discovery requires Codex",
                false,
                "Use executor=codex or auto with connector",
            );
        }
        let executor_arg = if connector.is_some() {
            crate::behavior_notice::record(
                "connector_codex",
                "此连接器任务将交给 Codex，并先检查实际访问权限；首版只允许读取和生成文字草稿，不会发送或删除内容。",
                "This connector task is routed to Codex, with access checked first. This release permits reads and text drafts only; sending and deletion are disabled.",
            );
            "codex"
        } else {
            executor_arg
        };
        let explicit_executor = matches!(executor_arg, "grok" | "codex");
        let chosen = match executor_arg {
            "grok" => NamedExecutor::Grok,
            "codex" => NamedExecutor::Codex,
            _ => {
                let provider = self
                    .settings
                    .get("llm_provider")
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                let primary = if primary_setting == "auto" {
                    match provider.as_str() {
                        "codex_oauth" => "codex",
                        "xai_subscription" => "grok",
                        _ => {
                            return external_error(
                                "not_configured",
                                "Choose an authenticated external executor in settings",
                                false,
                                "API-key chat does not imply an installed subscription executor",
                            );
                        }
                    }
                } else {
                    primary_setting.trim()
                };
                match NamedExecutor::parse(primary) {
                    Some(agent) => agent,
                    None => {
                        return external_error(
                            "not_configured",
                            "Unknown external executor",
                            false,
                            "Select auto, grok or codex",
                        );
                    }
                }
            }
        };

        let run_id = new_run_id();
        if let Err(e) = std::fs::create_dir_all(&executors.runs_dir) {
            return external_error(
                "executor_failed",
                &format!("could not create runs_dir: {e}"),
                true,
                "the agent process failed before finishing; retry once, then report the failure honestly",
            );
        }
        let run_dir = executors.runs_dir.join(&run_id);
        match std::fs::create_dir(&run_dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return external_error(
                    "executor_failed",
                    &format!("run_dir already exists: {}", run_dir.display()),
                    true,
                    "the agent process failed before finishing; retry once, then report the failure honestly",
                );
            }
            Err(e) => {
                return external_error(
                    "executor_failed",
                    &format!("could not create run_dir: {e}"),
                    true,
                    "the agent process failed before finishing; retry once, then report the failure honestly",
                );
            }
        }

        tracing::debug!(
            run_id = %run_id,
            executor = chosen.label(),
            task_preview = %truncate_str(task, 200),
            "run_external starting"
        );
        tracing::info!(
            run_id = %run_id,
            executor = chosen.label(),
            timeout_secs,
            run_dir = %run_dir.display(),
            "run_external starting"
        );
        self.emit_action_phase(
            ctx.chat_id,
            "calling_executor",
            Some("run_external"),
            Some(match chosen {
                NamedExecutor::Grok => "calling Grok Build",
                NamedExecutor::Codex => "calling Codex",
            }),
        );

        let context = args
            .get("context")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let prompt = build_external_prompt(task, context);
        let request = ExecutorRequest {
            prompt,
            cwd: run_dir.clone(),
            timeout: Duration::from_secs(timeout_secs),
            model: None,
            reasoning_effort: None,
            web_search: false,
            required_connector: connector.map(str::to_owned),
        };

        let started_at = chrono::Utc::now().to_rfc3339();
        let task_preview = truncate_str(task, TASK_PREVIEW_CHARS).to_string();
        self.persist_run_record(
            &run_id,
            json_run_record(
                &run_id,
                chosen,
                None,
                &task_preview,
                &run_dir,
                &started_at,
                None,
                "running",
                None,
                0,
                false,
                None,
                None,
                "",
            ),
        )
        .await;
        let ledger_action_id =
            self.best_effort_external_action_started(&run_id, ctx, &task_preview, chosen);

        let progress_tx = self.progress_tx.clone();
        let chat_id = ctx.chat_id;
        let primary_sink = progress_sink(progress_tx.clone(), chat_id, chosen.label());
        let primary_agent = chosen.pick(&executors);
        let primary_result = primary_agent.run(request.clone(), &primary_sink).await;

        let (receipt, fallback_used, primary_error) = match primary_result {
            Ok(receipt) => (Ok(receipt), false, None),
            Err(err) => {
                let primary_error = truncate_error(&err);
                // Explicit grok/codex stays on that agent. `auto` falls back
                // only when the primary never started (spawn / initialize),
                // not after a session exists.
                let fallback = if explicit_executor {
                    None
                } else {
                    fallback_of(&fallback_setting, chosen)
                        .filter(|_| is_executor_startup_failure(&primary_error))
                };
                if let Some(fb) = fallback {
                    crate::behavior_notice::record(
                        "executor_fallback",
                        &format!(
                            "{} 未能启动，正在尝试你配置的备用执行器 {}；任务会交给另一服务，能力和结果可能不同。",
                            chosen.label(),
                            fb.label()
                        ),
                        &format!(
                            "{} could not start. Trying your configured fallback executor {}; the task goes to another service, whose capabilities and results may differ.",
                            chosen.label(),
                            fb.label()
                        ),
                    );
                    let detail = fallback_phase_detail(chosen, fb, &primary_error);
                    tracing::warn!(
                        run_id = %run_id,
                        primary = chosen.label(),
                        fallback = fb.label(),
                        error = %primary_error,
                        "run_external primary failed to start; trying fallback"
                    );
                    self.emit_action_phase(
                        ctx.chat_id,
                        "calling_executor",
                        Some("run_external"),
                        Some(&detail),
                    );
                    let fb_sink = progress_sink(progress_tx, chat_id, fb.label());
                    let fb_agent = fb.pick(&executors);
                    match fb_agent.run(request, &fb_sink).await {
                        Ok(receipt) => (Ok(receipt), true, Some(primary_error)),
                        Err(fb_err) => (Err(fb_err), true, Some(primary_error)),
                    }
                } else {
                    (Err(err), false, Some(primary_error))
                }
            }
        };

        let finished_at = chrono::Utc::now().to_rfc3339();
        match receipt {
            Ok(receipt) => {
                let used = if fallback_used {
                    match receipt.kind {
                        ExecutorKind::GrokAcp => NamedExecutor::Grok,
                        ExecutorKind::CodexAppServer => NamedExecutor::Codex,
                    }
                } else {
                    chosen
                };
                self.persist_run_record(
                    &run_id,
                    json_run_record(
                        &run_id,
                        used,
                        receipt.model.as_deref(),
                        &task_preview,
                        &run_dir,
                        &started_at,
                        Some(&finished_at),
                        &receipt.stop_reason,
                        Some(&receipt.usage),
                        receipt.tool_calls.len(),
                        fallback_used,
                        primary_error.as_deref(),
                        receipt.error.as_deref(),
                        &receipt.final_text,
                    ),
                )
                .await;
                if let Some(action_id) = ledger_action_id.as_deref() {
                    self.best_effort_external_action_executed(
                        action_id,
                        used,
                        &receipt.stop_reason,
                    );
                }
                ok_receipt_json(
                    &run_id,
                    &receipt,
                    &run_dir,
                    fallback_used,
                    primary_error.as_deref(),
                )
            }
            Err(err) => {
                let message = truncate_error(&err);
                self.persist_run_record(
                    &run_id,
                    json_run_record(
                        &run_id,
                        chosen,
                        None,
                        &task_preview,
                        &run_dir,
                        &started_at,
                        Some(&finished_at),
                        "error",
                        None,
                        0,
                        fallback_used,
                        primary_error.as_deref().or(Some(message.as_str())),
                        Some(message.as_str()),
                        "",
                    ),
                )
                .await;
                if let Some(action_id) = ledger_action_id.as_deref() {
                    self.best_effort_external_action_failed(action_id, &message);
                }
                external_error(
                    "executor_failed",
                    &message,
                    true,
                    "the agent process failed before finishing; retry once, then report the failure honestly",
                )
            }
        }
    }

    async fn persist_run_record(&self, run_id: &str, record: Value) {
        if let Err(e) = self
            .memory
            .store_skill_state(EXTERNAL_RUN_SKILL_STATE, run_id, &record.to_string())
            .await
        {
            tracing::warn!(run_id, error = %e, "run_external: failed to store run record");
        }
    }

    fn best_effort_external_action_started(
        &self,
        run_id: &str,
        ctx: &ToolContext<'_>,
        task_preview: &str,
        chosen: NamedExecutor,
    ) -> Option<String> {
        if self.memory.db_path() == ":memory:" {
            return None;
        }
        let result = (|| -> anyhow::Result<String> {
            let conn = crate::encrypted_sqlite::open(self.memory.db_path())?;
            let store = crate::action_ledger::ActionLedgerStore::new(&conn);
            match store.create_proposed(&crate::action_ledger::NewAction {
                action_id: run_id.to_string(),
                action_kind: crate::action_ledger::ActionKind::Other,
                requested_by: Some(ctx.user_did.to_string()),
                chat_id: ctx.chat_id,
                input_summary: Some(task_preview.to_string()),
                user_visible_summary: Some(format!(
                    "External executor run requested ({})",
                    chosen.label()
                )),
                idempotency_key: Some(run_id.to_string()),
                run_id: Some(run_id.to_string()),
                metadata_json: Some(
                    serde_json::json!({
                        "tool_name": "run_external",
                        "executor": chosen.label(),
                        "schema_version": 1,
                    })
                    .to_string(),
                ),
                ..Default::default()
            }) {
                Ok(_) => {}
                Err(e) => {
                    if store.get_by_action_id(run_id)?.is_none() {
                        return Err(e);
                    }
                }
            }
            store.transition(
                run_id,
                crate::action_ledger::ActionStatus::Executing,
                &crate::action_ledger::ActionUpdate {
                    tool_call_id: Some("run_external".to_string()),
                    metadata_json: Some(
                        serde_json::json!({
                            "tool_name": "run_external",
                            "status": "executing",
                            "schema_version": 1,
                        })
                        .to_string(),
                    ),
                    ..Default::default()
                },
            )?;
            Ok(run_id.to_string())
        })();
        match result {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::warn!(error = %e, run_id, "failed to record run_external action start");
                None
            }
        }
    }

    fn best_effort_external_action_executed(
        &self,
        action_id: &str,
        used: NamedExecutor,
        stop_reason: &str,
    ) {
        if self.memory.db_path() == ":memory:" {
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            let conn = crate::encrypted_sqlite::open(self.memory.db_path())?;
            let store = crate::action_ledger::ActionLedgerStore::new(&conn);
            store.transition(
                action_id,
                crate::action_ledger::ActionStatus::Executed,
                &crate::action_ledger::ActionUpdate {
                    external_receipt: Some(format!("{}:{stop_reason}", used.label())),
                    tool_call_id: Some("run_external".to_string()),
                    user_visible_summary: Some(format!(
                        "External executor {} finished ({stop_reason})",
                        used.label()
                    )),
                    metadata_json: Some(
                        serde_json::json!({
                            "tool_name": "run_external",
                            "executor": used.label(),
                            "stop_reason": stop_reason,
                            "status": "executed",
                            "schema_version": 1,
                        })
                        .to_string(),
                    ),
                    ..Default::default()
                },
            )?;
            Ok(())
        })();
        if let Err(e) = result {
            tracing::warn!(error = %e, action_id, "failed to record run_external action execution");
        }
    }

    fn best_effort_external_action_failed(&self, action_id: &str, error: &str) {
        if self.memory.db_path() == ":memory:" {
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            let conn = crate::encrypted_sqlite::open(self.memory.db_path())?;
            let store = crate::action_ledger::ActionLedgerStore::new(&conn);
            store.transition(
                action_id,
                crate::action_ledger::ActionStatus::Failed,
                &crate::action_ledger::ActionUpdate {
                    tool_call_id: Some("run_external".to_string()),
                    error_kind: Some("executor_failed".to_string()),
                    error_message: Some(truncate_str(error, 500).to_string()),
                    metadata_json: Some(
                        serde_json::json!({
                            "tool_name": "run_external",
                            "status": "failed",
                            "schema_version": 1,
                        })
                        .to_string(),
                    ),
                    ..Default::default()
                },
            )?;
            Ok(())
        })();
        if let Err(e) = result {
            tracing::warn!(error = %e, action_id, "failed to record run_external action failure");
        }
    }
}

fn fallback_of(setting: &str, used: NamedExecutor) -> Option<NamedExecutor> {
    let fallback = NamedExecutor::parse(setting.trim())?;
    (fallback != used).then_some(fallback)
}

/// Spawn / initialize failures only. A session that already started must
/// surface its own error instead of silently switching agents.
fn is_executor_startup_failure(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    if lower.contains("session/new")
        || lower.contains("session/prompt")
        || lower.contains("thread/start")
        || lower.contains("turn/start")
    {
        return false;
    }
    lower.contains("spawn failed")
        || lower.contains("failed to spawn")
        || lower.contains("spawn grok")
        || lower.contains("spawn codex")
        || lower.contains("initialize")
}

fn executor_phase_name(named: NamedExecutor) -> &'static str {
    match named {
        NamedExecutor::Grok => "Grok Build",
        NamedExecutor::Codex => "Codex",
    }
}

fn fallback_phase_detail(primary: NamedExecutor, fallback: NamedExecutor, error: &str) -> String {
    let one_line = error.lines().next().unwrap_or(error);
    let cleaned: String = one_line
        .chars()
        .map(|c| if c == ';' { ',' } else { c })
        .collect();
    let reason = truncate_str(cleaned.trim(), 160);
    format!(
        "{} failed: {reason}; calling {}",
        executor_phase_name(primary),
        executor_phase_name(fallback)
    )
}

fn new_run_id() -> String {
    format!(
        "xr-{}-{:08x}",
        chrono::Utc::now().format("%Y%m%d%H%M%S"),
        rand::random::<u32>()
    )
}

fn build_external_prompt(task: &str, context: Option<&str>) -> String {
    let mut prompt = task.to_string();
    if let Some(ctx) = context {
        prompt.push_str("\n\n## Context\n");
        prompt.push_str(ctx);
    }
    prompt.push_str("\n\n");
    prompt.push_str(crate::prompts::GMAIL_UNREAD_SCOPE_GUIDANCE);
    prompt.push_str(
        "\n\nRules: work only inside the current directory; do not modify anything outside it; finish with a concise report of what you did and found.",
    );
    prompt
}

fn progress_sink(
    progress_tx: Option<crate::orchestrator::ProgressSender>,
    chat_id: Option<i64>,
    label: &'static str,
) -> impl Fn(ExecutorUpdate) + Send + Sync {
    move |update| {
        let ExecutorUpdate::ToolCall { title, status, .. } = update else {
            return;
        };
        let Some(tx) = progress_tx.as_ref() else {
            return;
        };
        let Some(cid) = chat_id else {
            return;
        };
        let text = format!("[Executor {label}] {title} ({status})");
        let _ = tx.try_send(ProgressEvent::new(
            cid,
            ProgressPayload::OwnerNotice {
                text,
                kind: OwnerNoticeKind::System,
            },
        ));
    }
}

fn kind_wire(kind: ExecutorKind) -> &'static str {
    match kind {
        ExecutorKind::GrokAcp => "grok_acp",
        ExecutorKind::CodexAppServer => "codex_app_server",
    }
}

fn named_wire(named: NamedExecutor) -> &'static str {
    match named {
        NamedExecutor::Grok => "grok_acp",
        NamedExecutor::Codex => "codex_app_server",
    }
}

#[allow(clippy::too_many_arguments)]
fn json_run_record(
    run_id: &str,
    executor: NamedExecutor,
    model: Option<&str>,
    task_preview: &str,
    run_dir: &std::path::Path,
    started_at: &str,
    finished_at: Option<&str>,
    stop_reason: &str,
    usage: Option<&crate::external_executor::ExecutorUsage>,
    tool_call_count: usize,
    fallback_used: bool,
    primary_error: Option<&str>,
    error: Option<&str>,
    final_text: &str,
) -> Value {
    serde_json::json!({
        "run_id": run_id,
        "executor": named_wire(executor),
        "model": model,
        "task_preview": task_preview,
        "run_dir": run_dir.display().to_string(),
        "started_at": started_at,
        "finished_at": finished_at,
        "stop_reason": stop_reason,
        "usage": usage,
        "tool_call_count": tool_call_count,
        "fallback_used": fallback_used,
        "primary_error": primary_error,
        "error": error,
        "final_text": final_text,
    })
}

fn truncate_final_text(text: &str) -> String {
    if text.len() <= MAX_FINAL_TEXT_CHARS {
        return text.to_string();
    }
    let kept = truncate_str(text, MAX_FINAL_TEXT_CHARS);
    let omitted = text.len() - kept.len();
    format!("{kept}\n…[truncated {omitted} chars; full text in run record]")
}

fn truncate_error(err: &anyhow::Error) -> String {
    let mut text = format!("{err:#}");
    if text.len() > MAX_ERROR_CHARS {
        let kept = truncate_str(&text, MAX_ERROR_CHARS);
        text = kept.to_string();
    }
    text
}

fn ok_receipt_json(
    run_id: &str,
    receipt: &ExecutorReceipt,
    run_dir: &std::path::Path,
    fallback_used: bool,
    primary_error: Option<&str>,
) -> String {
    let status = if receipt.stop_reason == "timeout" {
        "timeout"
    } else if receipt.error.is_some() || matches!(receipt.stop_reason.as_str(), "failed" | "error")
    {
        "failed"
    } else {
        "ok"
    };
    let start = receipt.tool_calls.len().saturating_sub(LAST_TOOL_CALLS);
    let last: Vec<Value> = receipt.tool_calls[start..]
        .iter()
        .map(|tc| {
            serde_json::json!({
                "title": tc.title,
                "status": tc.status,
            })
        })
        .collect();
    let mut body = serde_json::json!({
        "status": status,
        "run_id": run_id,
        "error": receipt.error.as_deref().map(|e| truncate_str(e, MAX_ERROR_CHARS)),
        "stderr_tail": receipt.stderr_tail.as_deref().map(|e| truncate_str(e, MAX_ERROR_CHARS)),
        "executor": kind_wire(receipt.kind),
        "model": receipt.model,
        "stop_reason": receipt.stop_reason,
        "final_text": truncate_final_text(&receipt.final_text),
        "tool_calls": {
            "count": receipt.tool_calls.len(),
            "last": last,
        },
        "usage": receipt.usage,
        "elapsed_ms": receipt.elapsed_ms,
        "run_dir": run_dir.display().to_string(),
        "fallback_used": fallback_used,
    });
    if let Some(err) = primary_error {
        body["primary_error"] = Value::String(err.to_string());
    }
    body.to_string()
}

fn external_error(kind: &str, error: &str, retryable: bool, hint: &str) -> String {
    serde_json::json!({
        "status": "error",
        "error": error,
        "error_kind": kind,
        "retryable": retryable,
        "hint": hint
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};

    use super::{EXTERNAL_RUN_SKILL_STATE, ExternalExecutors, NamedExecutor};

    #[test]
    fn delegated_gmail_count_has_explicit_scope_and_read_only_guidance() {
        let prompt = super::build_external_prompt("Check unread mail", None);
        assert!(prompt.contains(crate::prompts::GMAIL_UNREAD_SCOPE_GUIDANCE));
        assert!(prompt.contains("messagesUnread"));
        assert!(prompt.contains("SPAM and TRASH unless explicitly requested"));
        assert!(
            crate::prompts::prompt_module_external_executor()
                .contains(crate::prompts::GMAIL_UNREAD_SCOPE_GUIDANCE)
        );
    }

    use crate::external_executor::{
        ExecutorFuture, ExecutorKind, ExecutorReceipt, ExecutorRequest, ExecutorToolCall,
        ExecutorUpdate, ExecutorUsage, ExternalExecutor, UpdateSink,
    };
    use crate::memory::MemoryHandle;
    use crate::orchestrator::Orchestrator;
    use crate::settings::RuntimeSettings;
    use crate::test_tool_helpers::{CtxState, make_tool_call, test_orchestrator};
    use crate::tool_gate::{CallerContext, GateVerdict, ToolGate};

    struct FakeExecutor {
        kind: ExecutorKind,
        outcome: Canned,
        updates: Vec<ExecutorUpdate>,
        calls: AtomicUsize,
        last_request: Mutex<Option<ExecutorRequest>>,
    }

    #[derive(Clone)]
    enum Canned {
        Ok(ExecutorReceipt),
        Err(String),
    }

    impl FakeExecutor {
        fn ok(kind: ExecutorKind, receipt: ExecutorReceipt) -> Self {
            Self {
                kind,
                outcome: Canned::Ok(receipt),
                updates: vec![ExecutorUpdate::ToolCall {
                    id: "tc-1".into(),
                    title: "read file".into(),
                    status: "completed".into(),
                }],
                calls: AtomicUsize::new(0),
                last_request: Mutex::new(None),
            }
        }

        fn err(kind: ExecutorKind, message: &str) -> Self {
            Self {
                kind,
                outcome: Canned::Err(message.to_string()),
                updates: Vec::new(),
                calls: AtomicUsize::new(0),
                last_request: Mutex::new(None),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl ExternalExecutor for FakeExecutor {
        fn kind(&self) -> ExecutorKind {
            self.kind
        }

        fn run<'a>(
            &'a self,
            request: ExecutorRequest,
            on_update: UpdateSink<'a>,
        ) -> ExecutorFuture<'a> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                if let Ok(mut slot) = self.last_request.lock() {
                    *slot = Some(request);
                }
                for update in &self.updates {
                    on_update(update.clone());
                }
                match &self.outcome {
                    Canned::Ok(receipt) => Ok(receipt.clone()),
                    Canned::Err(message) => Err(anyhow::anyhow!("{message}")),
                }
            })
        }
    }

    fn fake_receipt(kind: ExecutorKind, stop_reason: &str, final_text: String) -> ExecutorReceipt {
        ExecutorReceipt {
            kind,
            session_id: Some("sess-test".into()),
            model: Some("test-model".into()),
            stop_reason: stop_reason.into(),
            final_text,
            tool_calls: vec![ExecutorToolCall {
                id: "tc-1".into(),
                title: "read file".into(),
                kind: "read".into(),
                status: "completed".into(),
            }],
            usage: ExecutorUsage {
                input_tokens: 10,
                output_tokens: 4,
                reasoning_tokens: 2,
                total_tokens: 16,
            },
            elapsed_ms: 42,
            stderr_tail: None,
            error: None,
        }
    }

    fn last_tool_json(state: &CtxState) -> Value {
        let msg = state.messages.last().expect("tool result message");
        let text = match msg {
            async_openai::types::chat::ChatCompletionRequestMessage::Tool(t) => match &t.content {
                async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(s) => {
                    s.clone()
                }
                other => panic!("unexpected tool content: {other:?}"),
            },
            other => panic!("expected tool message, got {other:?}"),
        };
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("tool result not JSON ({e}): {text}"))
    }

    async fn run(orch: &Orchestrator, args: Value) -> (Option<bool>, CtxState) {
        let args_str = args.to_string();
        let tc = make_tool_call("run_external", &args_str);
        let mut state = CtxState::new();
        let r = orch
            .handle_tool_external(&tc, "run_external", &args_str, &mut state.ctx())
            .await
            .expect("handler must not error");
        (r, state)
    }

    fn enable(orch: &Orchestrator) {
        orch.settings
            .set("external_executor_enabled", json!(true))
            .expect("enable external_executor_enabled");
        // Legacy explicit routing fixture; automatic subscription routing is tested separately.
        orch.settings
            .set("external_executor_primary", json!("grok"))
            .unwrap();
        orch.settings
            .set("external_executor_fallback", json!("codex"))
            .unwrap();
    }

    fn wire(
        orch: &Orchestrator,
        grok: Arc<FakeExecutor>,
        codex: Arc<FakeExecutor>,
        runs_dir: PathBuf,
    ) {
        orch.set_external_executors(Arc::new(ExternalExecutors {
            grok: grok as Arc<dyn ExternalExecutor>,
            codex: codex as Arc<dyn ExternalExecutor>,
            runs_dir,
            grok_bin: "grok".into(),
            codex_bin: "codex".into(),
        }));
    }

    fn orch_mem() -> Orchestrator {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        test_orchestrator(mem, Arc::new(RuntimeSettings::new()))
    }

    fn install_search_fixture(orch: &Orchestrator, output: &str) {
        let encoded: String = output.bytes().map(|b| format!("\\{b:02x}")).collect();
        let wat = format!(
            r#"(module
            (import "aegis" "host_write_output" (func $write (param i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "{encoded}")
            (func (export "run") (call $write (i32.const 0) (i32.const {}))))"#,
            output.len()
        );
        let capability =
            serde_json::from_value(json!({"skill_name":"web_search", "max_memory_mb":4, "allowed_domains":[], "allowed_fs_paths":[], "description":"Retired search trap", "payload_schema":{}})).unwrap();
        orch.sandbox
            .load_skill_from_bytes(&wat::parse_str(wat).unwrap(), capability)
            .unwrap();
    }

    #[tokio::test]
    async fn subscription_auto_routing_and_disabled_executor_behavior() {
        for (provider, expected) in [("codex_oauth", "codex"), ("xai_subscription", "grok")] {
            let env = orch_file();
            env.orch
                .settings
                .set("llm_provider", json!(provider))
                .unwrap();
            env.orch
                .settings
                .set("external_executor_primary", json!("auto"))
                .unwrap();
            env.orch
                .settings
                .set("external_executor_fallback", json!("none"))
                .unwrap();
            let grok = Arc::new(FakeExecutor::err(ExecutorKind::GrokAcp, "failed to spawn"));
            let codex = Arc::new(FakeExecutor::err(
                ExecutorKind::CodexAppServer,
                "failed to spawn",
            ));
            wire(&env.orch, grok.clone(), codex.clone(), env.runs_dir.clone());
            run(&env.orch, json!({"task":"test routing"})).await;
            assert_eq!(grok.calls(), usize::from(expected == "grok"));
            assert_eq!(codex.calls(), usize::from(expected == "codex"));
            env.orch
                .settings
                .set("external_executor_enabled", json!(false))
                .unwrap();
            let (_, state) = run(&env.orch, json!({"task":"must not run"})).await;
            assert!(last_tool_json(&state).to_string().contains("disabled"));
            assert_eq!(grok.calls() + codex.calls(), 1);
        }
    }

    #[tokio::test]
    async fn subscription_search_requires_live_evidence_without_secondary_service() {
        for evidence in [false, true] {
            let env = orch_file();
            env.orch
                .settings
                .set("llm_provider", json!("codex_oauth"))
                .unwrap();
            env.orch.settings.set("codex_enabled", json!(true)).unwrap();
            let result = json!({"status":"ok","summary":"Found", "results":[{"url":"https://example.com/source","title":"Source","snippet":"Fact"}]}).to_string();
            // Even a leftover working Wasm search must never become a fallback.
            install_search_fixture(&env.orch, &result);
            let mut receipt = fake_receipt(ExecutorKind::CodexAppServer, "completed", result);
            receipt.tool_calls.clear();
            if evidence {
                receipt.tool_calls.push(ExecutorToolCall {
                    id: "s1".into(),
                    title: "search".into(),
                    kind: "webSearch".into(),
                    status: "completed".into(),
                });
            }
            let codex = Arc::new(FakeExecutor::ok(ExecutorKind::CodexAppServer, receipt));
            let grok = Arc::new(FakeExecutor::err(ExecutorKind::GrokAcp, "must not run"));
            wire(&env.orch, grok.clone(), codex.clone(), env.runs_dir.clone());
            let (raw, notices) = crate::behavior_notice::collect(
                "en",
                None,
                None,
                env.orch.execute_web_search("public query"),
            )
            .await;
            let payload: Value = serde_json::from_str(&raw).unwrap();
            assert_eq!(notices.is_empty(), evidence);
            assert_eq!(payload["source"], "codex_subscription");
            assert_eq!(payload["status"], if evidence { "ok" } else { "error" });
            assert_eq!(
                crate::search_turn::lookup_from_payload("public query", &raw)
                    .sources
                    .is_empty(),
                !evidence
            );
            assert!(
                !payload
                    .get("fallback_used")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            );
            if !evidence {
                assert!(notices.join(" ").contains("no service was switched"));
            }
            assert_eq!(codex.calls(), 1);
            assert_eq!(grok.calls(), 0);
        }
    }

    #[tokio::test]
    async fn unavailable_or_retired_search_never_executes_leftover_wasm() {
        for provider in ["auto", "duckduckgo", "invalid"] {
            let orch = orch_mem();
            orch.settings
                .set("llm_provider", json!("openrouter"))
                .unwrap();
            orch.settings
                .set("web_search_provider", json!(provider))
                .unwrap();
            install_search_fixture(
                &orch,
                r#"{"status":"ok","results":[{"url":"https://example.com/should-not-run"}]}"#,
            );
            let (raw, notices) =
                crate::behavior_notice::collect("zh", None, None, orch.execute_web_search("query"))
                    .await;
            let payload: Value = serde_json::from_str(&raw).unwrap();
            assert_eq!(payload["status"], "error");
            assert_eq!(payload["reason"], "search_provider_unavailable");
            assert_eq!(payload["fallback_attempted"], false);
            assert!(notices.join(" ").contains("未自动切换服务"));
            assert!(!raw.contains("should-not-run"));
        }
    }

    #[tokio::test]
    async fn approved_search_replay_keeps_failure_notice_in_human_receipt() {
        let orch = orch_mem();
        orch.settings
            .set("llm_provider", json!("codex_oauth"))
            .unwrap();
        orch.settings.set("display_language", json!("en")).unwrap();
        let approval = json!({
            "tool_name":"web_search", "tool_args":json!({"query":"public fact"}).to_string(),
            "user_did":"did:test:replay", "chat_id":71, "created_at":chrono::Utc::now().to_rfc3339(),
        });
        orch.memory
            .store_skill_state("hitl", "test-search-replay", &approval.to_string())
            .await
            .unwrap();
        let result = orch
            .handle_gate_approval_scoped(
                "test-search-replay",
                crate::tool_gate::ApprovalScope::Once,
                "did:test:replay",
                Some(71),
            )
            .await
            .unwrap();
        assert!(
            result.reply.contains("no verifiable sources"),
            "{}",
            result.reply
        );
        assert!(
            result.reply.contains("no service was switched"),
            "{}",
            result.reply
        );
    }

    struct FileOrch {
        orch: Orchestrator,
        _tmp: tempfile::TempDir,
        runs_dir: PathBuf,
    }

    fn orch_file() -> FileOrch {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = tmp.path().join("memory.db");
        let runs_dir = tmp.path().join("runs");
        std::fs::create_dir(&runs_dir).expect("runs dir");
        let mem = MemoryHandle::spawn(&db).expect("file DB");
        let orch = test_orchestrator(mem, Arc::new(RuntimeSettings::new()));
        enable(&orch);
        FileOrch {
            orch,
            _tmp: tmp,
            runs_dir,
        }
    }

    #[tokio::test]
    async fn unknown_tool_passes_through() {
        let orch = orch_mem();
        let tc = make_tool_call("delegate_task", "{}");
        let mut state = CtxState::new();
        let r = orch
            .handle_tool_external(&tc, "delegate_task", "{}", &mut state.ctx())
            .await
            .unwrap();
        assert_eq!(r, None);
    }

    #[tokio::test]
    async fn happy_path_stores_receipt_and_ledger() {
        let env = orch_file();
        let grok = Arc::new(FakeExecutor::ok(
            ExecutorKind::GrokAcp,
            fake_receipt(ExecutorKind::GrokAcp, "end_turn", "done".into()),
        ));
        let codex = Arc::new(FakeExecutor::err(ExecutorKind::CodexAppServer, "unused"));
        wire(
            &env.orch,
            Arc::clone(&grok),
            Arc::clone(&codex),
            env.runs_dir.clone(),
        );

        let (r, state) = run(
            &env.orch,
            json!({"task": "summarize README", "context": "keep it short"}),
        )
        .await;
        assert_eq!(r, Some(true));
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "ok", "{v}");
        let run_id = v["run_id"].as_str().expect("run_id");
        assert!(
            run_id.starts_with("xr-") && run_id.len() == 26,
            "run_id shape: {run_id}"
        );
        assert_eq!(v["executor"], "grok_acp");
        assert_eq!(v["model"], "test-model");
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["final_text"], "done");
        assert_eq!(v["tool_calls"]["count"], 1);
        assert_eq!(v["tool_calls"]["last"][0]["title"], "read file");
        assert_eq!(v["tool_calls"]["last"][0]["status"], "completed");
        assert_eq!(v["usage"]["total_tokens"], 16);
        assert_eq!(v["elapsed_ms"], 42);
        assert_eq!(v["fallback_used"], false);
        let run_dir = std::path::PathBuf::from(v["run_dir"].as_str().expect("run_dir"));
        assert!(
            run_dir.is_dir(),
            "run_dir should exist: {}",
            run_dir.display()
        );
        assert_eq!(grok.calls(), 1);
        assert_eq!(codex.calls(), 0);

        let req = grok.last_request.lock().unwrap().clone().expect("request");
        assert_eq!(req.cwd, run_dir);
        assert!(req.prompt.contains("summarize README"));
        assert!(req.prompt.contains("## Context"));
        assert!(req.prompt.contains("keep it short"));
        assert!(
            req.prompt
                .contains("Rules: work only inside the current directory")
        );

        let stored = env
            .orch
            .memory
            .load_skill_state(EXTERNAL_RUN_SKILL_STATE, run_id)
            .await
            .unwrap()
            .expect("run record stored");
        let rec: Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(rec["stop_reason"], "end_turn");
        assert_eq!(rec["fallback_used"], false);
        assert_eq!(rec["final_text"], "done");

        let conn = crate::encrypted_sqlite::open(env.orch.memory.db_path()).unwrap();
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        let entry = store.get_by_action_id(run_id).unwrap().expect("ledger row");
        assert_eq!(entry.status, crate::action_ledger::ActionStatus::Executed);
        assert_eq!(entry.action_kind, crate::action_ledger::ActionKind::Other);
    }

    #[tokio::test]
    async fn fallback_path_records_primary_error() {
        let env = orch_file();
        let grok = Arc::new(FakeExecutor::err(
            ExecutorKind::GrokAcp,
            "spawn failed: grok missing",
        ));
        let codex = Arc::new(FakeExecutor::ok(
            ExecutorKind::CodexAppServer,
            fake_receipt(
                ExecutorKind::CodexAppServer,
                "completed",
                "from codex".into(),
            ),
        ));
        wire(
            &env.orch,
            Arc::clone(&grok),
            Arc::clone(&codex),
            env.runs_dir.clone(),
        );

        let ((_, state), notices) = crate::behavior_notice::collect(
            "en",
            None,
            None,
            run(&env.orch, json!({"task": "list files"})),
        )
        .await;
        assert!(notices.join(" ").contains("fallback executor codex"));
        assert!(notices.join(" ").contains("another service"));
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "ok", "{v}");
        assert_eq!(v["executor"], "codex_app_server");
        assert_eq!(v["fallback_used"], true);
        assert_eq!(v["final_text"], "from codex");
        assert!(
            v["primary_error"]
                .as_str()
                .unwrap()
                .contains("spawn failed"),
            "receipt must quote why grok did not start: {v}"
        );
        assert_eq!(grok.calls(), 1);
        assert_eq!(codex.calls(), 1);

        let run_id = v["run_id"].as_str().unwrap();
        let stored = env
            .orch
            .memory
            .load_skill_state(EXTERNAL_RUN_SKILL_STATE, run_id)
            .await
            .unwrap()
            .unwrap();
        let rec: Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(rec["fallback_used"], true);
        assert!(
            rec["primary_error"]
                .as_str()
                .unwrap()
                .contains("spawn failed"),
            "{rec}"
        );
    }

    #[tokio::test]
    async fn explicit_grok_does_not_fall_back_to_codex() {
        let env = orch_file();
        let grok = Arc::new(FakeExecutor::err(
            ExecutorKind::GrokAcp,
            "spawn failed: grok missing",
        ));
        let codex = Arc::new(FakeExecutor::ok(
            ExecutorKind::CodexAppServer,
            fake_receipt(
                ExecutorKind::CodexAppServer,
                "completed",
                "should not run".into(),
            ),
        ));
        wire(
            &env.orch,
            Arc::clone(&grok),
            Arc::clone(&codex),
            env.runs_dir.clone(),
        );

        let (_, state) = run(
            &env.orch,
            json!({"task": "write a python script", "executor": "grok"}),
        )
        .await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "error", "{v}");
        assert!(v["error"].as_str().unwrap().contains("spawn failed"), "{v}");
        assert_eq!(grok.calls(), 1);
        assert_eq!(codex.calls(), 0);
    }

    #[tokio::test]
    async fn session_error_does_not_fall_back_to_codex() {
        let env = orch_file();
        let grok = Arc::new(FakeExecutor::err(
            ExecutorKind::GrokAcp,
            "JSON-RPC error on session/prompt: model overloaded",
        ));
        let codex = Arc::new(FakeExecutor::ok(
            ExecutorKind::CodexAppServer,
            fake_receipt(
                ExecutorKind::CodexAppServer,
                "completed",
                "should not run".into(),
            ),
        ));
        wire(
            &env.orch,
            Arc::clone(&grok),
            Arc::clone(&codex),
            env.runs_dir.clone(),
        );

        let (_, state) = run(&env.orch, json!({"task": "write a python script"})).await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "error", "{v}");
        assert!(
            v["error"].as_str().unwrap().contains("session/prompt"),
            "{v}"
        );
        assert_eq!(grok.calls(), 1);
        assert_eq!(codex.calls(), 0);
    }

    #[test]
    fn startup_failure_detects_spawn_not_a_live_session() {
        assert!(super::is_executor_startup_failure(
            "spawn grok ACP agent 'grok': No such file or directory"
        ));
        assert!(super::is_executor_startup_failure(
            "failed to spawn agent binary 'grok'"
        ));
        assert!(super::is_executor_startup_failure(
            "JSON-RPC error on initialize: auth required"
        ));
        assert!(!super::is_executor_startup_failure(
            "JSON-RPC error on session/prompt: model overloaded"
        ));
        assert!(!super::is_executor_startup_failure(
            "JSON-RPC error on thread/start: sandbox refused"
        ));
        let detail = super::fallback_phase_detail(
            NamedExecutor::Grok,
            NamedExecutor::Codex,
            "spawn failed: grok missing; stderr: secret",
        );
        assert_eq!(
            detail,
            "Grok Build failed: spawn failed: grok missing, stderr: secret; calling Codex"
        );
    }

    #[tokio::test]
    async fn agent_reported_error_marks_run_failed() {
        let env = orch_file();
        let mut receipt = fake_receipt(ExecutorKind::CodexAppServer, "failed", String::new());
        receipt.error = Some("The 'gpt-6-astra' model requires a newer version of Codex.".into());
        let grok = Arc::new(FakeExecutor::err(ExecutorKind::GrokAcp, "unused"));
        let codex = Arc::new(FakeExecutor::ok(ExecutorKind::CodexAppServer, receipt));
        wire(&env.orch, grok, Arc::clone(&codex), env.runs_dir.clone());

        let (_, state) = run(
            &env.orch,
            json!({"task": "count mail", "executor": "codex"}),
        )
        .await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "failed", "{v}");
        assert_eq!(v["stop_reason"], "failed");
        assert!(
            v["error"]
                .as_str()
                .unwrap()
                .contains("newer version of Codex"),
            "{v}"
        );
        assert_eq!(codex.calls(), 1);
        let run_id = v["run_id"].as_str().unwrap();
        let stored = env
            .orch
            .memory
            .load_skill_state(EXTERNAL_RUN_SKILL_STATE, run_id)
            .await
            .unwrap()
            .unwrap();
        let rec: Value = serde_json::from_str(&stored).unwrap();
        assert!(
            rec["error"].as_str().unwrap().contains("newer version"),
            "{rec}"
        );
    }

    #[tokio::test]
    async fn timeout_receipt_is_not_retried() {
        let env = orch_file();
        let grok = Arc::new(FakeExecutor::ok(
            ExecutorKind::GrokAcp,
            fake_receipt(ExecutorKind::GrokAcp, "timeout", "partial".into()),
        ));
        let codex = Arc::new(FakeExecutor::ok(
            ExecutorKind::CodexAppServer,
            fake_receipt(
                ExecutorKind::CodexAppServer,
                "completed",
                "should not run".into(),
            ),
        ));
        wire(
            &env.orch,
            Arc::clone(&grok),
            Arc::clone(&codex),
            env.runs_dir.clone(),
        );

        let (_, state) = run(&env.orch, json!({"task": "slow job"})).await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "timeout", "{v}");
        assert_eq!(v["stop_reason"], "timeout");
        assert_eq!(v["fallback_used"], false);
        assert_eq!(grok.calls(), 1);
        assert_eq!(codex.calls(), 0);
    }

    #[tokio::test]
    async fn disabled_setting_is_not_configured_and_creates_no_run_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runs_dir = tmp.path().join("runs");
        std::fs::create_dir(&runs_dir).unwrap();
        let orch = orch_mem();
        let grok = Arc::new(FakeExecutor::ok(
            ExecutorKind::GrokAcp,
            fake_receipt(ExecutorKind::GrokAcp, "end_turn", "nope".into()),
        ));
        let codex = Arc::new(FakeExecutor::err(ExecutorKind::CodexAppServer, "unused"));
        wire(&orch, grok, codex, runs_dir.clone());

        let (r, state) = run(&orch, json!({"task": "do not run"})).await;
        assert_eq!(r, Some(true));
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "error");
        assert_eq!(v["error_kind"], "not_configured");
        assert_eq!(v["retryable"], false);
        assert!(
            v["hint"]
                .as_str()
                .unwrap()
                .contains("external_executor_enabled"),
            "{v}"
        );
        assert!(std::fs::read_dir(&runs_dir).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn enabled_without_executors_is_not_available() {
        let orch = orch_mem();
        enable(&orch);
        let (_, state) = run(&orch, json!({"task": "hello"})).await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "error");
        assert_eq!(v["error_kind"], "not_available");
    }

    #[tokio::test]
    async fn invalid_executor_is_invalid_input() {
        let env = orch_file();
        let grok = Arc::new(FakeExecutor::ok(
            ExecutorKind::GrokAcp,
            fake_receipt(ExecutorKind::GrokAcp, "end_turn", "nope".into()),
        ));
        let codex = Arc::new(FakeExecutor::err(ExecutorKind::CodexAppServer, "unused"));
        wire(&env.orch, grok, codex, env.runs_dir.clone());

        let (_, state) = run(&env.orch, json!({"task": "hello", "executor": "claude"})).await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "error");
        assert_eq!(v["error_kind"], "invalid_input");
        assert!(std::fs::read_dir(&env.runs_dir).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn final_text_is_truncated_at_12000_chars() {
        let env = orch_file();
        let long = "a".repeat(12_050);
        let grok = Arc::new(FakeExecutor::ok(
            ExecutorKind::GrokAcp,
            fake_receipt(ExecutorKind::GrokAcp, "end_turn", long.clone()),
        ));
        let codex = Arc::new(FakeExecutor::err(ExecutorKind::CodexAppServer, "unused"));
        wire(&env.orch, grok, codex, env.runs_dir.clone());

        let (_, state) = run(&env.orch, json!({"task": "write a lot"})).await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "ok", "{v}");
        let returned = v["final_text"].as_str().unwrap();
        assert!(returned.starts_with(&"a".repeat(12_000)));
        assert!(
            returned.contains("truncated 50 chars; full text in run record"),
            "{returned}"
        );
        let run_id = v["run_id"].as_str().unwrap();
        let stored = env
            .orch
            .memory
            .load_skill_state(EXTERNAL_RUN_SKILL_STATE, run_id)
            .await
            .unwrap()
            .unwrap();
        let rec: Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(rec["final_text"].as_str().unwrap().len(), 12_050);
    }

    #[test]
    fn run_external_subcall_is_blocked_by_the_gate() {
        let g = ToolGate::new();
        let args = r#"{"task":"x"}"#;
        assert!(matches!(
            g.evaluate("run_external", args, CallerContext::Interactive),
            GateVerdict::Allow
        ));
        assert!(matches!(
            g.evaluate("run_external", args, CallerContext::Callback),
            GateVerdict::Allow
        ));
        assert!(
            matches!(
                g.evaluate("run_external", args, CallerContext::Subcall),
                GateVerdict::Block { .. }
            ),
            "Subcall must be blocked"
        );
    }

    #[test]
    fn named_executor_fallback_skips_same_agent() {
        assert_eq!(
            super::fallback_of("codex", NamedExecutor::Grok),
            Some(NamedExecutor::Codex)
        );
        assert_eq!(super::fallback_of("grok", NamedExecutor::Grok), None);
        assert_eq!(super::fallback_of("none", NamedExecutor::Grok), None);
    }
}
