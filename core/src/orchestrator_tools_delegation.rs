//! Hercules delegation tool handler (`delegate_task`).
//!
//! Aegis Runtime hands *tasks* to Hercules and reads back their state; Hercules
//! owns attempts, provider sessions, evidence and verification
//! (`docs/2026-09-05-hercules-delegation-design.md`).
//!
//! Actions: `draft` (intent → task draft + lint, read-only), `create`
//! (creates the Hercules task; runs on the owner's subscription runtimes, no
//! per-call confirmation — owner decision 2026-09-05), `status` (read-only). Linkage is kept in
//! skill state `hercules_delegation` keyed by Hercules task id.

use anyhow::Result;
use serde_json::Value;

use async_openai::types::chat::ChatCompletionMessageToolCall;

use crate::hercules_client::{
    CreateTaskInput, HerculesClient, HerculesTask, is_terminal_state, task_id_for,
};
use crate::orchestrator::{Orchestrator, ToolContext};

/// Skill-state namespace holding one linkage record per Hercules task id.
/// Listed in `memory_migrations::INTERNAL_SKILL_STATE_NAMES_SQL`, otherwise
/// the startup orphan sweep deletes it (it is not a Wasm skill).
pub const HERCULES_LINKAGE_SKILL: &str = "hercules_delegation";
/// Adapter sent on create when the caller names none.
pub const DEFAULT_ADAPTER: &str = "grok";

impl Orchestrator {
    /// Handle the `delegate_task` pseudo-tool.
    pub(crate) async fn handle_tool_delegation(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "delegate_task" => execute_delegate_task,
        );
        Ok(None)
    }

    async fn execute_delegate_task(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let Some(client) = self.services.hercules.lock().clone() else {
            return delegation_error(
                "not_configured",
                "Hercules delegation is not configured on this instance",
                false,
                "set HERCULES_API_BASE and HERCULES_TOKEN and restart; do not pretend the task was delegated",
            );
        };
        match action {
            "draft" => self.delegate_draft(&client, &args).await,
            "create" => self.delegate_create(&client, &args, ctx.user_did).await,
            "status" => self.delegate_status(&client, &args).await,
            _ => delegation_error(
                "invalid_input",
                &format!("unknown action '{action}'"),
                false,
                "valid actions: draft, create, status",
            ),
        }
    }

    async fn delegate_draft(&self, client: &HerculesClient, args: &Value) -> String {
        let intent = args
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if intent.is_empty() {
            return delegation_error(
                "invalid_input",
                "intent is required for draft",
                false,
                "describe the change wanted in plain language",
            );
        }
        let repo = arg_repo(args);
        match client.draft_from_intent(intent, &repo).await {
            Ok(draft) => serde_json::json!({
                "status": "drafted",
                "repo": repo,
                "draft": draft,
                "production": client.is_production(),
                "hint": "Show the draft (title, body, lint findings) to the user. When they agree, call create with the reviewed title and body (edit the body if needed). Nothing has been created yet."
            })
            .to_string(),
            Err(e) => upstream_error(&e),
        }
    }

    async fn delegate_create(
        &self,
        client: &HerculesClient,
        args: &Value,
        user_did: &str,
    ) -> String {
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
        if title.is_empty() || body.trim().is_empty() {
            return delegation_error(
                "invalid_input",
                "title and body are required for create",
                false,
                "run draft first and pass its title/body",
            );
        }
        if !body.contains("## Definition of Done") {
            return delegation_error(
                "acceptance_lint_failed",
                "body has no '## Definition of Done' section",
                false,
                "run draft first; the body must carry a Definition of Done so Hercules can verify the task",
            );
        }
        match client.lint_acceptance(body).await {
            Ok(lint) if !lint.ok => {
                return serde_json::json!({
                    "status": "error",
                    "error": "acceptance criteria failed Hercules lint",
                    "error_kind": "acceptance_lint_failed",
                    "retryable": false,
                    "findings": lint.findings,
                    "hint": "fix the Definition of Done per the findings, then call create again"
                })
                .to_string();
            }
            Ok(_) => {}
            Err(e) => return upstream_error(&e),
        }
        // Owner decision 2026-09-05: runs on the subscription runtimes (grok by
        // default) need no per-call confirmation, on staging or production.
        let repo = arg_repo(args);
        let work_item_id = args.get("work_item_id").and_then(|v| v.as_i64());
        if let Some(wid) = work_item_id
            && let Some(open) = self.open_delegation_for_work_item(wid).await
        {
            return serde_json::json!({
                "status": "error",
                "error": format!("work item {wid} already has an open delegation ({open})"),
                "error_kind": "already_delegated",
                "retryable": false,
                "hercules_task_id": open,
                "hint": "poll it with status; one open Hercules task per work item"
            })
            .to_string();
        }
        let mut labels: Vec<String> = args
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(id) = work_item_id {
            labels.push(format!("work_item_id={id}"));
        }
        labels.push(format!("principal={user_did}"));
        labels.push("source=aegis-runtime".to_string());
        labels.dedup();
        let attempt = args
            .get("attempt")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(1, 99) as u32)
            .unwrap_or(1);
        let task_id = task_id_for(user_did, title, body, attempt);
        // Adapter is a top-level create field. Hercules falls back to its
        // daemon default when it is null (staging: fake), so always send one;
        // grok is the owner's subscription runtime (see DEFAULT_ADAPTER).
        let adapter = args
            .get("adapter")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_ADAPTER)
            .to_string();
        let input = CreateTaskInput {
            id: Some(task_id.clone()),
            title: title.to_string(),
            repo: repo.clone(),
            body: body.to_string(),
            priority: args.get("priority").and_then(|v| v.as_i64()),
            labels,
            adapter: Some(adapter.clone()),
        };
        let task = match client.create_task_confirmed(&input).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                return serde_json::json!({
                    "status": "error",
                    "error": "Hercules did not answer the create and the task is not visible yet",
                    "error_kind": "unknown_outcome",
                    "retryable": false,
                    "hercules_task_id": task_id,
                    "hint": "Hercules may still create it. Check with status using this task_id later, or retry create with the same title and body: the id is deterministic, so a duplicate is refused rather than created twice."
                })
                .to_string();
            }
            Err(e) if e.to_string().contains("HTTP 409") => {
                return serde_json::json!({
                    "status": "error",
                    "error": "Hercules already has a task with this id",
                    "error_kind": "already_exists",
                    "retryable": false,
                    "hercules_task_id": task_id,
                    "hint": "the same title/body was delegated before (a cancelled id stays taken); poll it with status, or pass attempt=2 to mint a fresh id"
                })
                .to_string();
            }
            Err(e) => return upstream_error(&e),
        };
        let adapter_warning = match task.adapter.as_deref() {
            Some(a) if a == adapter => None,
            other => Some(format!(
                "Hercules reported adapter {other:?} instead of {adapter:?}; the task may run on the daemon default"
            )),
        };
        if let Some(w) = &adapter_warning {
            tracing::warn!(task_id = %task.id, "delegate_task: {w}");
        }
        let now = chrono::Utc::now().to_rfc3339();
        let linkage = serde_json::json!({
            "hercules_task_id": task.id,
            "adapter": task.adapter,
            "work_item_id": work_item_id,
            "principal": user_did,
            "repo": repo,
            "title": title,
            "last_state": task.state,
            "hercules_base": client.base(),
            "production": client.is_production(),
            "created_at": now,
            "updated_at": now,
        });
        if let Err(e) = self
            .memory
            .store_skill_state(HERCULES_LINKAGE_SKILL, &task.id, &linkage.to_string())
            .await
        {
            tracing::warn!(task_id = %task.id, error = %e, "delegate_task: failed to store linkage");
        }
        serde_json::json!({
            "status": "created",
            "hercules_task_id": task.id,
            "state": task.state,
            "repo": repo,
            "title": title,
            "adapter": task.adapter,
            "warning": adapter_warning,
            "production": client.is_production(),
            "hint": "Hercules now owns this task. Poll with status (task_id) before reporting progress; do not claim completion until state is done."
        })
        .to_string()
    }

    async fn delegate_status(&self, client: &HerculesClient, args: &Value) -> String {
        let task_id = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if task_id.is_empty() {
            let entries = self
                .memory
                .list_skill_state_entries(HERCULES_LINKAGE_SKILL)
                .await
                .unwrap_or_default();
            let delegations: Vec<Value> = entries
                .iter()
                .filter_map(|(_, v)| serde_json::from_str::<Value>(v).ok())
                .collect();
            return serde_json::json!({
                "status": "ok",
                "count": delegations.len(),
                "delegations": delegations,
                "hint": "pass task_id to refresh one task from Hercules"
            })
            .to_string();
        }
        let task = match client.get_task(task_id).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                return delegation_error(
                    "not_found",
                    &format!("Hercules has no task '{task_id}'"),
                    false,
                    "check the id; staging tasks are disposable and may have been reset",
                );
            }
            Err(e) => return upstream_error(&e),
        };
        if let Ok(Some(existing)) = self
            .memory
            .load_skill_state(HERCULES_LINKAGE_SKILL, task_id)
            .await
        {
            let mut link: Value =
                serde_json::from_str(&existing).unwrap_or_else(|_| serde_json::json!({}));
            apply_task_to_linkage(&mut link, &task);
            if let Err(e) = self
                .memory
                .store_skill_state(HERCULES_LINKAGE_SKILL, task_id, &link.to_string())
                .await
            {
                tracing::warn!(task_id, error = %e, "delegate_task: failed to update linkage");
            }
        }
        let hold = task.hold_code().map(|c| c.to_string());
        serde_json::json!({
            "status": "ok",
            "task": task,
            "dispatch_hold": hold,
            "hint": if hold.is_some() {
                "the scheduler is not admitting this task yet (see dispatch_hold); tell the user why instead of promising progress"
            } else {
                "states: todo/in_progress/review/done/cancelled/retrying/failed/deleted; review means a PR is waiting for a human decision"
            }
        })
        .to_string()
    }
}

/// One state change observed by `process_delegation_sync`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DelegationTransition {
    pub hercules_task_id: String,
    pub title: Option<String>,
    pub from: String,
    pub to: String,
    pub work_item_id: Option<i64>,
    pub pr_url: Option<String>,
}

impl DelegationTransition {
    /// Owner-facing line for a state change. States that need the owner
    /// (review = PR waiting, done, failed) are `Important`; the rest are
    /// `Routine` timeline entries with no Telegram fallback.
    pub fn owner_message(&self) -> String {
        let title = self
            .title
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or(self.hercules_task_id.as_str());
        let what = match self.to.as_str() {
            "review" => "已完成实现，PR 等你审阅",
            "done" => "已完成并通过验证（合并到 main 是 Hercules 里的另一步）",
            "failed" => "失败了，需要你看一下",
            "cancelled" => "已取消",
            "missing" => "在 Hercules 上找不到了（可能被重置）",
            "in_progress" => "开始执行",
            "retrying" => "正在重试",
            other => other,
        };
        let mut line = format!(
            "Hercules 委派任务「{title}」{what}（{} → {}）",
            self.from, self.to
        );
        if let Some(pr) = self.pr_url.as_deref().filter(|p| !p.is_empty()) {
            line.push_str(&format!("\n{pr}"));
        }
        line
    }

    pub fn owner_priority(&self) -> crate::orchestrator::NotifyPriority {
        match self.to.as_str() {
            "review" | "done" | "failed" => crate::orchestrator::NotifyPriority::Important,
            _ => crate::orchestrator::NotifyPriority::Routine,
        }
    }
}

impl Orchestrator {
    pub fn hercules_configured(&self) -> bool {
        self.services.hercules.lock().is_some()
    }

    async fn open_delegation_for_work_item(&self, work_item_id: i64) -> Option<String> {
        let entries = self
            .memory
            .list_skill_state_entries(HERCULES_LINKAGE_SKILL)
            .await
            .ok()?;
        entries.into_iter().find_map(|(key, raw)| {
            let link: Value = serde_json::from_str(&raw).ok()?;
            let same = link["work_item_id"].as_i64() == Some(work_item_id);
            let open = !is_terminal_state(link["last_state"].as_str().unwrap_or(""));
            (same && open).then_some(key)
        })
    }

    /// Reconciliation tick: refresh every non-terminal delegation from
    /// Hercules and record what moved. Tasks Hercules no longer knows are
    /// marked `missing` so they stop being polled. Never fails the tick on a
    /// single task; upstream errors are logged and retried next tick.
    pub async fn process_delegation_sync(&self) -> Result<Vec<DelegationTransition>> {
        let Some(client) = self.services.hercules.lock().clone() else {
            return Ok(Vec::new());
        };
        let entries = self
            .memory
            .list_skill_state_entries(HERCULES_LINKAGE_SKILL)
            .await?;
        let mut transitions = Vec::new();
        for (key, raw) in entries {
            let Ok(mut link) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            let from = link["last_state"].as_str().unwrap_or("").to_string();
            if is_terminal_state(&from) {
                continue;
            }
            let to = match client.get_task(&key).await {
                Ok(Some(task)) => {
                    apply_task_to_linkage(&mut link, &task);
                    task.state
                }
                Ok(None) => {
                    link["last_state"] = Value::String("missing".into());
                    link["updated_at"] = Value::String(chrono::Utc::now().to_rfc3339());
                    "missing".to_string()
                }
                Err(e) => {
                    tracing::warn!(task_id = %key, error = %e, "[delegation-sync] refresh failed");
                    continue;
                }
            };
            if let Err(e) = self
                .memory
                .store_skill_state(HERCULES_LINKAGE_SKILL, &key, &link.to_string())
                .await
            {
                tracing::warn!(task_id = %key, error = %e, "[delegation-sync] store failed");
                continue;
            }
            if to != from {
                tracing::info!(task_id = %key, %from, %to, "[delegation-sync] state changed");
                transitions.push(DelegationTransition {
                    title: link["title"].as_str().map(|s| s.to_string()),
                    hercules_task_id: key,
                    from,
                    to,
                    work_item_id: link["work_item_id"].as_i64(),
                    pr_url: link["pr_url"].as_str().map(|s| s.to_string()),
                });
            }
        }
        Ok(transitions)
    }
}

impl Orchestrator {
    /// Tell the owner about delegation state changes (timeline entry; Web
    /// Push / Telegram fallback per priority). Returns how many were sent.
    pub async fn notify_delegation_transitions(
        &self,
        user_did: &str,
        chat_id: i64,
        transitions: &[DelegationTransition],
    ) -> usize {
        let mut sent = 0;
        for t in transitions {
            let label = format!("hercules_delegation:{}:{}", t.hercules_task_id, t.to);
            match self
                .emit_owner_timeline_notice_with_optional_telegram(
                    user_did,
                    chat_id,
                    &label,
                    &t.owner_message(),
                    t.owner_priority(),
                    Some("hercules_delegation".to_string()),
                    Some(t.hercules_task_id.clone()),
                )
                .await
            {
                Ok(_) => sent += 1,
                Err(e) => {
                    tracing::warn!(task_id = %t.hercules_task_id, error = %e, "[delegation-sync] owner notice failed");
                }
            }
        }
        sent
    }
}

/// Project the Hercules task fields Aegis Runtime tracks onto a linkage record.
fn apply_task_to_linkage(link: &mut Value, task: &HerculesTask) {
    let opt = |v: &Option<String>| v.clone().map(Value::String).unwrap_or(Value::Null);
    link["last_state"] = Value::String(task.state.clone());
    link["pr_url"] = opt(&task.pr_url);
    link["pr_status"] = opt(&task.pr_status);
    link["verified_sha"] = opt(&task.verified_sha);
    link["hold_code"] = task
        .hold_code()
        .map(|c| Value::String(c.to_string()))
        .unwrap_or(Value::Null);
    link["updated_at"] = Value::String(chrono::Utc::now().to_rfc3339());
}

fn arg_repo(args: &Value) -> String {
    args.get("repo")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("aegis")
        .to_string()
}

fn delegation_error(kind: &str, error: &str, retryable: bool, hint: &str) -> String {
    serde_json::json!({
        "status": "error",
        "error": error,
        "error_kind": kind,
        "retryable": retryable,
        "hint": hint
    })
    .to_string()
}

fn upstream_error(e: &anyhow::Error) -> String {
    let msg = e.to_string();
    let auth = msg.contains("HTTP 401") || msg.contains("HTTP 403");
    delegation_error(
        if auth { "auth" } else { "upstream" },
        &msg,
        !auth,
        if auth {
            "HERCULES_TOKEN is rejected; tell the user instead of retrying"
        } else {
            "Hercules did not answer; retry later and tell the user if it keeps failing"
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use super::{DelegationTransition, HERCULES_LINKAGE_SKILL};
    use crate::hercules_client::{HerculesClient, mock};
    use crate::memory::MemoryHandle;
    use crate::orchestrator::Orchestrator;
    use crate::settings::RuntimeSettings;
    use crate::test_tool_helpers::{CtxState, make_tool_call, test_orchestrator};

    fn orch() -> Orchestrator {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        test_orchestrator(mem, Arc::new(RuntimeSettings::new()))
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
        let tc = make_tool_call("delegate_task", &args_str);
        let mut state = CtxState::new();
        let r = orch
            .handle_tool_delegation(&tc, "delegate_task", &args_str, &mut state.ctx())
            .await
            .expect("handler must not error");
        (r, state)
    }

    #[tokio::test]
    async fn unknown_tool_passes_through() {
        let orch = orch();
        let tc = make_tool_call("manage_goals", "{}");
        let mut state = CtxState::new();
        let r = orch
            .handle_tool_delegation(&tc, "manage_goals", "{}", &mut state.ctx())
            .await
            .unwrap();
        assert_eq!(r, None);
    }

    #[tokio::test]
    async fn not_configured_is_a_structured_error() {
        let orch = orch();
        let (r, state) = run(&orch, json!({"action": "status", "task_id": "t-1"})).await;
        assert_eq!(r, Some(true));
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "error");
        assert_eq!(v["error_kind"], "not_configured");
        assert!(state.tools_called.contains(&"delegate_task".to_string()));
    }

    #[tokio::test]
    async fn draft_returns_body_lint_and_next_step() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        let (r, state) = run(
            &orch,
            json!({"action": "draft", "intent": "Add a --version flag to the CLI", "repo": "aegis"}),
        )
        .await;
        assert_eq!(r, Some(true));
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "drafted", "{v}");
        assert!(
            v["draft"]["body"]
                .as_str()
                .unwrap()
                .contains("## Definition of Done")
        );
        assert_eq!(v["draft"]["lint"]["ok"], true);
        assert_eq!(v["draft"]["labels"][0], "needs-plan");
        assert!(
            v["hint"].as_str().unwrap().contains("create"),
            "hint must tell the model how to proceed: {v}"
        );
        assert!(
            m.seen.lock().unwrap().created.is_empty(),
            "draft must not create"
        );
    }

    #[tokio::test]
    async fn draft_requires_intent() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        let (_, state) = run(&orch, json!({"action": "draft"})).await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "error");
        assert_eq!(v["error_kind"], "invalid_input");
    }

    #[tokio::test]
    async fn create_on_staging_posts_task_with_linkage_labels_and_stores_linkage() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        let body = "## Task\nx\n\n## Definition of Done\n- [ ] (reviewer) done\n";
        let (r, state) = run(
            &orch,
            json!({
                "action": "create",
                "title": "Add --version",
                "repo": "aegis",
                "body": body,
                "work_item_id": 7
            }),
        )
        .await;
        assert_eq!(r, Some(true));
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "created", "{v}");
        let task_id = v["hercules_task_id"].as_str().unwrap().to_string();
        assert!(
            task_id.starts_with("aegis-runtime-add-version-"),
            "{task_id}"
        );
        assert_eq!(v["state"], "todo");

        let created = m.seen.lock().unwrap().created.clone();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0]["repo"], "aegis");
        assert_eq!(created[0]["body"], body);
        let labels: Vec<String> = created[0]["labels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap().to_string())
            .collect();
        assert!(labels.contains(&"work_item_id=7".to_string()), "{labels:?}");
        assert!(
            labels.contains(&"principal=did:test:user".to_string()),
            "{labels:?}"
        );
        assert!(
            labels.contains(&"source=aegis-runtime".to_string()),
            "{labels:?}"
        );

        let stored = orch
            .memory
            .load_skill_state(HERCULES_LINKAGE_SKILL, &task_id)
            .await
            .unwrap()
            .expect("linkage stored");
        let link: Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(link["hercules_task_id"], task_id);
        assert_eq!(link["work_item_id"], 7);
        assert_eq!(link["principal"], "did:test:user");
        assert_eq!(link["repo"], "aegis");
        assert_eq!(link["last_state"], "todo");
        assert!(link["created_at"].is_string());
    }

    #[tokio::test]
    async fn create_uses_deterministic_task_id_so_retries_cannot_duplicate() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        let body = "## Definition of Done\n- [ ] (reviewer) done\n";
        let args =
            json!({"action": "create", "title": "Add --version", "repo": "aegis", "body": body});
        let (_, state) = run(&orch, args.clone()).await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "created", "{v}");
        let created = m.seen.lock().unwrap().created.clone();
        let id = created[0]["id"].as_str().expect("id sent").to_string();
        assert!(id.starts_with("aegis-runtime-add-version-"), "{id}");
        assert_eq!(
            v["hercules_task_id"], id,
            "mock echoes the id; linkage keyed by it"
        );
        assert_eq!(
            crate::hercules_client::task_id_for("did:test:user", "Add --version", body, 1),
            id
        );
        assert_eq!(
            created[0]["adapter"], "grok",
            "adapter is a top-level create field, defaulted"
        );
        assert_eq!(v["adapter"], "grok");
        assert!(v["warning"].is_null(), "{v}");
    }

    #[tokio::test]
    async fn create_attempt_mints_fresh_id_and_explicit_adapter_is_kept() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        let body = "## Definition of Done\n- [ ] (reviewer) done\n";
        let (_, s1) = run(
            &orch,
            json!({"action": "create", "title": "T", "repo": "aegis", "body": body}),
        )
        .await;
        let (_, s2) = run(
            &orch,
            json!({"action": "create", "title": "T", "repo": "aegis", "body": body, "attempt": 2, "adapter": "codex"}),
        )
        .await;
        let id1 = last_tool_json(&s1)["hercules_task_id"]
            .as_str()
            .unwrap()
            .to_string();
        let id2 = last_tool_json(&s2)["hercules_task_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(id1, id2);
        let created = m.seen.lock().unwrap().created.clone();
        assert_eq!(created[1]["adapter"], "codex");
    }

    #[tokio::test]
    async fn create_timeout_then_visible_counts_as_created() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(
            HerculesClient::new(&m.base, "test-token", false)
                .with_create_timeout(std::time::Duration::from_millis(200))
                .with_create_confirm(
                    std::time::Duration::from_secs(5),
                    std::time::Duration::from_millis(100),
                ),
        ));
        let body = "## Definition of Done\n- [ ] (reviewer) done\n";
        let (_, state) = run(
            &orch,
            json!({"action": "create", "title": "slow: Add --version", "repo": "aegis", "body": body}),
        )
        .await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "created", "{v}");
        let id = v["hercules_task_id"].as_str().unwrap();
        assert!(id.starts_with("aegis-runtime-slow-add-version-"), "{v}");
        assert!(
            orch.memory
                .load_skill_state(HERCULES_LINKAGE_SKILL, id)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn create_timeout_still_invisible_is_unknown_outcome_not_failure() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(
            HerculesClient::new(&m.base, "test-token", false)
                .with_create_timeout(std::time::Duration::from_millis(200))
                .with_create_confirm(
                    std::time::Duration::from_millis(400),
                    std::time::Duration::from_millis(100),
                ),
        ));
        let body = "## Definition of Done\n- [ ] (reviewer) done\n";
        let (_, state) = run(
            &orch,
            json!({"action": "create", "title": "slow: invisible task", "repo": "aegis", "body": body}),
        )
        .await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "error", "{v}");
        assert_eq!(v["error_kind"], "unknown_outcome");
        assert!(
            v["hercules_task_id"]
                .as_str()
                .unwrap()
                .starts_with("aegis-runtime-slow-invisible-task-"),
            "{v}"
        );
        assert!(
            v["hint"].as_str().unwrap().contains("same title and body"),
            "{v}"
        );
        assert!(
            orch.memory
                .list_skill_state_entries(HERCULES_LINKAGE_SKILL)
                .await
                .unwrap()
                .is_empty(),
            "no linkage for an unconfirmed create"
        );
    }

    #[tokio::test]
    async fn create_refuses_body_without_definition_of_done() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        let (_, state) = run(
            &orch,
            json!({"action": "create", "title": "t", "repo": "aegis", "body": "just do it"}),
        )
        .await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "error", "{v}");
        assert_eq!(v["error_kind"], "acceptance_lint_failed");
        assert!(
            m.seen.lock().unwrap().created.is_empty(),
            "lint failure must not create"
        );
    }

    #[tokio::test]
    async fn create_on_production_needs_no_confirmation() {
        // Owner decision 2026-09-05: grok/codex runs are on subscriptions,
        // so production creates go through without an extra confirmation.
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", true)));
        let body = "## Definition of Done\n- [ ] (reviewer) done\n";
        let (_, state) = run(
            &orch,
            json!({"action": "create", "title": "t", "repo": "aegis", "body": body}),
        )
        .await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "created", "{v}");
        assert_eq!(v["production"], true);
        assert_eq!(m.seen.lock().unwrap().created.len(), 1);
    }

    #[tokio::test]
    async fn status_reads_task_and_updates_linkage() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        orch.memory
            .store_skill_state(
                HERCULES_LINKAGE_SKILL,
                "t-5",
                &json!({"hercules_task_id": "t-5", "last_state": "todo", "principal": "did:test:user"})
                    .to_string(),
            )
            .await
            .unwrap();
        let (_, state) = run(&orch, json!({"action": "status", "task_id": "t-5"})).await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "ok", "{v}");
        assert_eq!(v["task"]["state"], "review");
        assert_eq!(v["task"]["pr_url"], "https://example.invalid/pr/1");
        assert_eq!(v["task"]["verified_sha"], "abc1234");

        let link: Value = serde_json::from_str(
            &orch
                .memory
                .load_skill_state(HERCULES_LINKAGE_SKILL, "t-5")
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(link["last_state"], "review");
        assert_eq!(link["pr_url"], "https://example.invalid/pr/1");
    }

    #[tokio::test]
    async fn status_without_task_id_lists_known_linkages() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        for id in ["t-a", "t-b"] {
            orch.memory
                .store_skill_state(
                    HERCULES_LINKAGE_SKILL,
                    id,
                    &json!({"hercules_task_id": id, "last_state": "todo", "title": id}).to_string(),
                )
                .await
                .unwrap();
        }
        let (_, state) = run(&orch, json!({"action": "status"})).await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "ok", "{v}");
        assert_eq!(v["delegations"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn status_and_sync_surface_dispatch_holds() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        orch.memory
            .store_skill_state(
                HERCULES_LINKAGE_SKILL,
                "t-held",
                &json!({"hercules_task_id": "t-held", "last_state": "todo"}).to_string(),
            )
            .await
            .unwrap();
        let (_, state) = run(&orch, json!({"action": "status", "task_id": "t-held"})).await;
        let v = last_tool_json(&state);
        assert_eq!(v["dispatch_hold"], "repo_readiness_incomplete", "{v}");
        assert!(v["hint"].as_str().unwrap().contains("not admitting"), "{v}");
        orch.process_delegation_sync().await.unwrap();
        let link: Value = serde_json::from_str(
            &orch
                .memory
                .load_skill_state(HERCULES_LINKAGE_SKILL, "t-held")
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(link["hold_code"], "repo_readiness_incomplete");
    }

    #[tokio::test]
    async fn status_of_missing_task_is_not_found() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        let (_, state) = run(&orch, json!({"action": "status", "task_id": "gone"})).await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "error", "{v}");
        assert_eq!(v["error_kind"], "not_found");
    }

    #[tokio::test]
    async fn unknown_action_is_invalid_input() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        let (_, state) = run(&orch, json!({"action": "launch"})).await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "error");
        assert_eq!(v["error_kind"], "invalid_input");
        assert!(v["hint"].as_str().unwrap().contains("draft"));
    }

    #[tokio::test]
    async fn create_refuses_second_open_delegation_for_same_work_item() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        orch.memory
            .store_skill_state(
                HERCULES_LINKAGE_SKILL,
                "t-open",
                &json!({"hercules_task_id": "t-open", "work_item_id": 9, "last_state": "running"})
                    .to_string(),
            )
            .await
            .unwrap();
        let body = "## Definition of Done\n- [ ] (reviewer) done\n";
        let (_, state) = run(
            &orch,
            json!({"action": "create", "title": "t", "repo": "aegis", "body": body, "work_item_id": 9}),
        )
        .await;
        let v = last_tool_json(&state);
        assert_eq!(v["error_kind"], "already_delegated", "{v}");
        assert_eq!(v["hercules_task_id"], "t-open");
        assert!(m.seen.lock().unwrap().created.is_empty());

        // A terminal delegation does not block a new one.
        orch.memory
            .store_skill_state(
                HERCULES_LINKAGE_SKILL,
                "t-open",
                &json!({"hercules_task_id": "t-open", "work_item_id": 9, "last_state": "done"})
                    .to_string(),
            )
            .await
            .unwrap();
        let (_, state) = run(
            &orch,
            json!({"action": "create", "title": "t", "repo": "aegis", "body": body, "work_item_id": 9}),
        )
        .await;
        assert_eq!(last_tool_json(&state)["status"], "created");
    }

    #[tokio::test]
    async fn sync_refreshes_open_linkages_and_reports_transitions() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        for (id, st) in [("t-5", "todo"), ("t-done", "done"), ("t-rev", "review")] {
            orch.memory
                .store_skill_state(
                    HERCULES_LINKAGE_SKILL,
                    id,
                    &json!({"hercules_task_id": id, "work_item_id": 3, "last_state": st})
                        .to_string(),
                )
                .await
                .unwrap();
        }
        let t = orch.process_delegation_sync().await.unwrap();
        // mock answers "review" for every known id: only t-5 moved.
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!(t[0].hercules_task_id, "t-5");
        assert_eq!(t[0].from, "todo");
        assert_eq!(t[0].to, "review");
        assert_eq!(t[0].work_item_id, Some(3));
        assert_eq!(t[0].pr_url.as_deref(), Some("https://example.invalid/pr/1"));
        let done: Value = serde_json::from_str(
            &orch
                .memory
                .load_skill_state(HERCULES_LINKAGE_SKILL, "t-done")
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            done["last_state"], "done",
            "terminal linkages are left alone"
        );
        assert!(done.get("pr_url").is_none());
        // Second tick: nothing moves.
        assert!(orch.process_delegation_sync().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn sync_marks_vanished_tasks_missing_and_stops_polling_them() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        orch.memory
            .store_skill_state(
                HERCULES_LINKAGE_SKILL,
                "gone",
                &json!({"hercules_task_id": "gone", "last_state": "todo"}).to_string(),
            )
            .await
            .unwrap();
        let t = orch.process_delegation_sync().await.unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].to, "missing");
        let before = m.seen.lock().unwrap().auth_headers.len();
        assert!(orch.process_delegation_sync().await.unwrap().is_empty());
        assert_eq!(
            m.seen.lock().unwrap().auth_headers.len(),
            before,
            "missing tasks are not polled again"
        );
    }

    #[tokio::test]
    async fn transitions_become_owner_timeline_notices() {
        let m = mock::start().await;
        let orch = orch();
        orch.set_hercules_client(Arc::new(HerculesClient::new(&m.base, "test-token", false)));
        orch.memory
            .store_skill_state(
                HERCULES_LINKAGE_SKILL,
                "t-5",
                &json!({"hercules_task_id": "t-5", "title": "Add --version", "last_state": "in_progress"})
                    .to_string(),
            )
            .await
            .unwrap();
        let t = orch.process_delegation_sync().await.unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].title.as_deref(), Some("Add --version"));
        assert_eq!(
            t[0].owner_priority(),
            crate::orchestrator::NotifyPriority::Important
        );
        let sent = orch
            .notify_delegation_transitions("did:test:owner", 42, &t)
            .await;
        assert_eq!(sent, 1);
        let l1 = orch.memory.get_l1("did:test:owner").await.unwrap();
        let notice = l1
            .iter()
            .find(|m| m.content.contains("Add --version"))
            .unwrap_or_else(|| panic!("owner timeline must carry the notice: {l1:?}"));
        assert!(notice.content.contains("PR 等你审阅"), "{}", notice.content);
        assert!(
            notice.content.contains("https://example.invalid/pr/1"),
            "{}",
            notice.content
        );
        assert!(
            notice.content.contains("in_progress → review"),
            "{}",
            notice.content
        );
    }

    #[test]
    fn owner_message_and_priority_by_state() {
        let mk = |to: &str| DelegationTransition {
            hercules_task_id: "t-9".into(),
            title: None,
            from: "todo".into(),
            to: to.into(),
            work_item_id: None,
            pr_url: None,
        };
        assert!(
            mk("done")
                .owner_message()
                .contains("「t-9」已完成并通过验证")
        );
        assert!(
            !mk("done").owner_message().contains("已合并"),
            "done is not merged"
        );
        assert_eq!(
            mk("done").owner_priority(),
            crate::orchestrator::NotifyPriority::Important
        );
        assert_eq!(
            mk("failed").owner_priority(),
            crate::orchestrator::NotifyPriority::Important
        );
        assert_eq!(
            mk("in_progress").owner_priority(),
            crate::orchestrator::NotifyPriority::Routine
        );
        assert_eq!(
            mk("missing").owner_priority(),
            crate::orchestrator::NotifyPriority::Routine
        );
    }

    #[tokio::test]
    async fn sync_without_client_is_a_noop() {
        let orch = orch();
        assert!(orch.process_delegation_sync().await.unwrap().is_empty());
        assert!(!orch.hercules_configured());
    }

    #[tokio::test]
    async fn upstream_failure_is_retryable_error() {
        let orch = orch();
        // Nothing listens on port 1.
        orch.set_hercules_client(Arc::new(HerculesClient::new(
            "http://127.0.0.1:1",
            "t",
            false,
        )));
        let (_, state) = run(&orch, json!({"action": "status", "task_id": "t-1"})).await;
        let v = last_tool_json(&state);
        assert_eq!(v["status"], "error");
        assert_eq!(v["error_kind"], "upstream");
        assert_eq!(v["retryable"], true);
    }
}
