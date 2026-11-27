use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionStatus {
    Proposed,
    PendingApproval,
    Approved,
    Executing,
    Executed,
    Failed,
    Skipped,
    Cancelled,
    Superseded,
}

impl ActionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::PendingApproval => "pending_approval",
            Self::Approved => "approved",
            Self::Executing => "executing",
            Self::Executed => "executed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
            Self::Superseded => "superseded",
        }
    }

    fn from_db(value: String) -> Result<Self> {
        match value.as_str() {
            "proposed" => Ok(Self::Proposed),
            "pending_approval" => Ok(Self::PendingApproval),
            "approved" => Ok(Self::Approved),
            "executing" => Ok(Self::Executing),
            "executed" => Ok(Self::Executed),
            "failed" => Ok(Self::Failed),
            "skipped" => Ok(Self::Skipped),
            "cancelled" => Ok(Self::Cancelled),
            "superseded" => Ok(Self::Superseded),
            _ => Err(anyhow!("unknown action ledger status: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind {
    TelegramSend,
    EmailSend,
    EmailDraft,
    EmailForward,
    EmailUpdate,
    CalendarCreate,
    CalendarUpdate,
    CalendarDelete,
    ReminderCreate,
    ReminderUpdate,
    ReminderDelete,
    TaskCreate,
    KbMerge,
    KbUpdate,
    KbDelete,
    ApprovalRequest,
    ResearchStart,
    RuntimeSettingChange,
    SkillCreate,
    SkillDeploy,
    CodeCommit,
    PullRequestCreate,
    Other,
}

impl ActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TelegramSend => "telegram_send",
            Self::EmailSend => "email_send",
            Self::EmailDraft => "email_draft",
            Self::EmailForward => "email_forward",
            Self::EmailUpdate => "email_update",
            Self::CalendarCreate => "calendar_create",
            Self::CalendarUpdate => "calendar_update",
            Self::CalendarDelete => "calendar_delete",
            Self::ReminderCreate => "reminder_create",
            Self::ReminderUpdate => "reminder_update",
            Self::ReminderDelete => "reminder_delete",
            Self::TaskCreate => "task_create",
            Self::KbMerge => "kb_merge",
            Self::KbUpdate => "kb_update",
            Self::KbDelete => "kb_delete",
            Self::ApprovalRequest => "approval_request",
            Self::ResearchStart => "research_start",
            Self::RuntimeSettingChange => "runtime_setting_change",
            Self::SkillCreate => "skill_create",
            Self::SkillDeploy => "skill_deploy",
            Self::CodeCommit => "code_commit",
            Self::PullRequestCreate => "pull_request_create",
            Self::Other => "other",
        }
    }

    fn from_db(value: String) -> Result<Self> {
        match value.as_str() {
            "telegram_send" => Ok(Self::TelegramSend),
            "email_send" => Ok(Self::EmailSend),
            "email_draft" => Ok(Self::EmailDraft),
            "email_forward" => Ok(Self::EmailForward),
            "email_update" => Ok(Self::EmailUpdate),
            "calendar_create" => Ok(Self::CalendarCreate),
            "calendar_update" => Ok(Self::CalendarUpdate),
            "calendar_delete" => Ok(Self::CalendarDelete),
            "reminder_create" => Ok(Self::ReminderCreate),
            "reminder_update" => Ok(Self::ReminderUpdate),
            "reminder_delete" => Ok(Self::ReminderDelete),
            "task_create" => Ok(Self::TaskCreate),
            "kb_merge" => Ok(Self::KbMerge),
            "kb_update" => Ok(Self::KbUpdate),
            "kb_delete" => Ok(Self::KbDelete),
            "approval_request" => Ok(Self::ApprovalRequest),
            "research_start" => Ok(Self::ResearchStart),
            "runtime_setting_change" => Ok(Self::RuntimeSettingChange),
            "skill_create" => Ok(Self::SkillCreate),
            "skill_deploy" => Ok(Self::SkillDeploy),
            "code_commit" => Ok(Self::CodeCommit),
            "pull_request_create" => Ok(Self::PullRequestCreate),
            "other" => Ok(Self::Other),
            _ => Err(anyhow!("unknown action ledger kind: {value}")),
        }
    }

    const fn requires_receipt_when_executed(self) -> bool {
        !matches!(self, Self::Other)
    }
}

pub fn runtime_setting_action_id(approval_id: &str) -> String {
    format!("runtime-setting:{approval_id}")
}

#[allow(clippy::too_many_arguments)]
pub fn record_runtime_setting_proposed(
    conn: &Connection,
    approval_id: &str,
    key: &str,
    current_value: &serde_json::Value,
    proposed_value: &serde_json::Value,
    requested_by: &str,
    chat_id: Option<i64>,
    tool_call_id: Option<&str>,
    gate_pre_approved: bool,
) -> Result<ActionLedgerEntry> {
    let action_id = runtime_setting_action_id(approval_id);
    let store = ActionLedgerStore::new(conn);
    let entry = store.create_proposed(&NewAction {
        action_id: action_id.clone(),
        action_kind: ActionKind::RuntimeSettingChange,
        requested_by: Some(requested_by.to_string()),
        chat_id,
        input_summary: Some(truncate_action_summary(&format!(
            "{key}: {} -> {}",
            compact_json(current_value),
            compact_json(proposed_value)
        ))),
        user_visible_summary: Some(format!("Runtime setting change proposed for `{key}`")),
        idempotency_key: Some(action_id.clone()),
        metadata_json: Some(
            serde_json::json!({
                "approval_id": approval_id,
                "setting_key": key,
                "current_value": current_value,
                "proposed_value": proposed_value,
                "gate_pre_approved": gate_pre_approved,
            })
            .to_string(),
        ),
        ..Default::default()
    })?;

    if gate_pre_approved {
        return Ok(entry);
    }

    store.transition(
        &action_id,
        ActionStatus::PendingApproval,
        &ActionUpdate {
            tool_call_id: tool_call_id.map(str::to_string),
            external_event_id: Some(approval_id.to_string()),
            ..Default::default()
        },
    )
}

pub fn transition_runtime_setting_action(
    conn: &Connection,
    approval_id: &str,
    status: ActionStatus,
    update: &ActionUpdate,
) -> Result<ActionLedgerEntry> {
    ActionLedgerStore::new(conn).transition(&runtime_setting_action_id(approval_id), status, update)
}

#[derive(Debug, Clone)]
pub struct NewAction {
    pub action_id: String,
    pub action_kind: ActionKind,
    pub requested_by: Option<String>,
    pub chat_id: Option<i64>,
    pub input_summary: Option<String>,
    pub user_visible_summary: Option<String>,
    pub assistant_event_id: Option<i64>,
    pub provider_request_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub trace_id: Option<String>,
    pub run_id: Option<String>,
    pub mandate_id: Option<String>,
    pub source_event_id: Option<String>,
    pub metadata_json: Option<String>,
}

impl Default for NewAction {
    fn default() -> Self {
        Self {
            action_id: String::new(),
            action_kind: ActionKind::Other,
            requested_by: None,
            chat_id: None,
            input_summary: None,
            user_visible_summary: None,
            assistant_event_id: None,
            provider_request_id: None,
            idempotency_key: None,
            trace_id: None,
            run_id: None,
            mandate_id: None,
            source_event_id: None,
            metadata_json: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActionUpdate {
    pub external_receipt: Option<String>,
    pub external_message_id: Option<String>,
    pub external_event_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub provider_request_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub user_visible_summary: Option<String>,
    pub error_kind: Option<String>,
    pub error_message: Option<String>,
    pub retry_policy: Option<String>,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionLedgerEntry {
    pub id: i64,
    pub action_id: String,
    pub action_kind: ActionKind,
    pub status: ActionStatus,
    pub requested_by: Option<String>,
    pub chat_id: Option<i64>,
    pub created_at: String,
    pub proposed_at: Option<String>,
    pub approved_at: Option<String>,
    pub executing_at: Option<String>,
    pub executed_at: Option<String>,
    pub failed_at: Option<String>,
    pub cancelled_at: Option<String>,
    pub input_summary: Option<String>,
    pub user_visible_summary: Option<String>,
    pub external_receipt: Option<String>,
    pub external_message_id: Option<String>,
    pub external_event_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub assistant_event_id: Option<i64>,
    pub provider_request_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub error_kind: Option<String>,
    pub error_message: Option<String>,
    pub retry_policy: Option<String>,
    pub trace_id: Option<String>,
    pub run_id: Option<String>,
    pub mandate_id: Option<String>,
    pub source_event_id: Option<String>,
    pub metadata_json: Option<String>,
}

pub struct ActionLedgerStore<'a> {
    conn: &'a Connection,
}

impl<'a> ActionLedgerStore<'a> {
    pub const fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn create_proposed(&self, action: &NewAction) -> Result<ActionLedgerEntry> {
        if action.action_id.trim().is_empty() {
            return Err(anyhow!("action_id must not be empty"));
        }

        let now = now_rfc3339();
        self.conn
            .execute(
                "INSERT INTO action_ledger (
                    action_id, action_kind, status, requested_by, chat_id, created_at, proposed_at,
                    input_summary, user_visible_summary, assistant_event_id, provider_request_id,
                    idempotency_key, trace_id, run_id, mandate_id, source_event_id, metadata_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    action.action_id,
                    action.action_kind.as_str(),
                    ActionStatus::Proposed.as_str(),
                    action.requested_by,
                    action.chat_id,
                    now,
                    action.input_summary,
                    action.user_visible_summary,
                    action.assistant_event_id,
                    action.provider_request_id,
                    action.idempotency_key,
                    action.trace_id,
                    action.run_id,
                    action.mandate_id,
                    action.source_event_id,
                    action.metadata_json,
                ],
            )
            .with_context(|| format!("creating action ledger row {}", action.action_id))?;

        self.get_by_action_id(&action.action_id)?
            .ok_or_else(|| anyhow!("action ledger row missing after insert"))
    }

    pub fn transition(
        &self,
        action_id: &str,
        status: ActionStatus,
        update: &ActionUpdate,
    ) -> Result<ActionLedgerEntry> {
        let current = self
            .get_by_action_id(action_id)?
            .ok_or_else(|| anyhow!("action ledger row not found: {action_id}"))?;

        if status == ActionStatus::Executed && current.action_kind.requires_receipt_when_executed()
        {
            require_execution_receipt(&current, update)?;
        }

        let now = now_rfc3339();
        let proposed_at = if status == ActionStatus::Proposed {
            Some(now.as_str())
        } else {
            None
        };
        let approved_at = if status == ActionStatus::Approved {
            Some(now.as_str())
        } else {
            None
        };
        let executing_at = if status == ActionStatus::Executing {
            Some(now.as_str())
        } else {
            None
        };
        let executed_at = if status == ActionStatus::Executed {
            Some(now.as_str())
        } else {
            None
        };
        let failed_at = if status == ActionStatus::Failed {
            Some(now.as_str())
        } else {
            None
        };
        let cancelled_at = if matches!(status, ActionStatus::Cancelled | ActionStatus::Skipped) {
            Some(now.as_str())
        } else {
            None
        };

        self.conn
            .execute(
                "UPDATE action_ledger SET
                    status = ?2,
                    proposed_at = COALESCE(?3, proposed_at),
                    approved_at = COALESCE(?4, approved_at),
                    executing_at = COALESCE(?5, executing_at),
                    executed_at = COALESCE(?6, executed_at),
                    failed_at = COALESCE(?7, failed_at),
                    cancelled_at = COALESCE(?8, cancelled_at),
                    external_receipt = COALESCE(?9, external_receipt),
                    external_message_id = COALESCE(?10, external_message_id),
                    external_event_id = COALESCE(?11, external_event_id),
                    tool_call_id = COALESCE(?12, tool_call_id),
                    provider_request_id = COALESCE(?13, provider_request_id),
                    idempotency_key = COALESCE(?14, idempotency_key),
                    user_visible_summary = COALESCE(?15, user_visible_summary),
                    error_kind = COALESCE(?16, error_kind),
                    error_message = COALESCE(?17, error_message),
                    retry_policy = COALESCE(?18, retry_policy),
                    metadata_json = COALESCE(?19, metadata_json)
                 WHERE action_id = ?1",
                params![
                    action_id,
                    status.as_str(),
                    proposed_at,
                    approved_at,
                    executing_at,
                    executed_at,
                    failed_at,
                    cancelled_at,
                    update.external_receipt,
                    update.external_message_id,
                    update.external_event_id,
                    update.tool_call_id,
                    update.provider_request_id,
                    update.idempotency_key,
                    update.user_visible_summary,
                    update.error_kind,
                    update.error_message,
                    update.retry_policy,
                    update.metadata_json,
                ],
            )
            .with_context(|| format!("transitioning action ledger row {action_id}"))?;

        self.get_by_action_id(action_id)?
            .ok_or_else(|| anyhow!("action ledger row missing after update"))
    }

    pub fn get_by_action_id(&self, action_id: &str) -> Result<Option<ActionLedgerEntry>> {
        let sql = ACTION_LEDGER_SELECT_SQL.to_string() + " WHERE action_id = ?1";
        self.conn
            .query_row(&sql, params![action_id], map_action_ledger_row)
            .optional()
            .context("querying action ledger by action_id")
    }

    pub fn get_by_id(&self, id: i64) -> Result<Option<ActionLedgerEntry>> {
        let sql = ACTION_LEDGER_SELECT_SQL.to_string() + " WHERE id = ?1";
        self.conn
            .query_row(&sql, params![id], map_action_ledger_row)
            .optional()
            .context("querying action ledger by id")
    }

    pub fn list_recent_by_status(
        &self,
        status: ActionStatus,
        limit: usize,
    ) -> Result<Vec<ActionLedgerEntry>> {
        let limit = i64::try_from(limit).context("action ledger limit overflow")?;
        let sql =
            ACTION_LEDGER_SELECT_SQL.to_string() + " WHERE status = ?1 ORDER BY id DESC LIMIT ?2";
        let mut stmt = self
            .conn
            .prepare(&sql)
            .context("preparing action ledger status query")?;
        let rows = stmt
            .query_map(params![status.as_str(), limit], map_action_ledger_row)
            .context("querying action ledger by status")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("mapping action ledger rows")
    }

    pub fn list_recent(&self, limit: usize) -> Result<Vec<ActionLedgerEntry>> {
        let limit = i64::try_from(limit).context("action ledger limit overflow")?;
        let sql = ACTION_LEDGER_SELECT_SQL.to_string() + " ORDER BY id DESC LIMIT ?1";
        let mut stmt = self
            .conn
            .prepare(&sql)
            .context("preparing recent action ledger query")?;
        let rows = stmt
            .query_map(params![limit], map_action_ledger_row)
            .context("querying recent action ledger rows")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("mapping recent action ledger rows")
    }
}

const ACTION_LEDGER_SELECT_SQL: &str = "SELECT
    id, action_id, action_kind, status, requested_by, chat_id, created_at,
    proposed_at, approved_at, executing_at, executed_at, failed_at, cancelled_at,
    input_summary, user_visible_summary, external_receipt, external_message_id,
    external_event_id, tool_call_id, assistant_event_id, provider_request_id,
    idempotency_key, error_kind, error_message, retry_policy, trace_id, run_id,
    mandate_id, source_event_id, metadata_json
    FROM action_ledger";

fn map_action_ledger_row(row: &Row<'_>) -> rusqlite::Result<ActionLedgerEntry> {
    let action_kind = ActionKind::from_db(row.get::<_, String>(2)?).map_err(to_sql_error)?;
    let status = ActionStatus::from_db(row.get::<_, String>(3)?).map_err(to_sql_error)?;
    Ok(ActionLedgerEntry {
        id: row.get(0)?,
        action_id: row.get(1)?,
        action_kind,
        status,
        requested_by: row.get(4)?,
        chat_id: row.get(5)?,
        created_at: row.get(6)?,
        proposed_at: row.get(7)?,
        approved_at: row.get(8)?,
        executing_at: row.get(9)?,
        executed_at: row.get(10)?,
        failed_at: row.get(11)?,
        cancelled_at: row.get(12)?,
        input_summary: row.get(13)?,
        user_visible_summary: row.get(14)?,
        external_receipt: row.get(15)?,
        external_message_id: row.get(16)?,
        external_event_id: row.get(17)?,
        tool_call_id: row.get(18)?,
        assistant_event_id: row.get(19)?,
        provider_request_id: row.get(20)?,
        idempotency_key: row.get(21)?,
        error_kind: row.get(22)?,
        error_message: row.get(23)?,
        retry_policy: row.get(24)?,
        trace_id: row.get(25)?,
        run_id: row.get(26)?,
        mandate_id: row.get(27)?,
        source_event_id: row.get(28)?,
        metadata_json: row.get(29)?,
    })
}

fn require_execution_receipt(current: &ActionLedgerEntry, update: &ActionUpdate) -> Result<()> {
    if any_present([
        current.external_receipt.as_deref(),
        current.external_message_id.as_deref(),
        current.external_event_id.as_deref(),
        current.tool_call_id.as_deref(),
        current.provider_request_id.as_deref(),
        update.external_receipt.as_deref(),
        update.external_message_id.as_deref(),
        update.external_event_id.as_deref(),
        update.tool_call_id.as_deref(),
        update.provider_request_id.as_deref(),
    ]) {
        Ok(())
    } else {
        Err(anyhow!(
            "executed external action requires receipt-like evidence"
        ))
    }
}

fn any_present<const N: usize>(values: [Option<&str>; N]) -> bool {
    values
        .into_iter()
        .flatten()
        .any(|value| !value.trim().is_empty())
}

fn compact_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn truncate_action_summary(value: &str) -> String {
    const MAX: usize = 240;
    if value.len() <= MAX {
        return value.to_string();
    }
    let mut end = MAX;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &value[..end])
}

fn to_sql_error(error: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::memory_migrations::run_all_migrations(&conn).unwrap();
        conn
    }

    fn new_action(action_id: &str, kind: ActionKind) -> NewAction {
        NewAction {
            action_id: action_id.to_string(),
            action_kind: kind,
            requested_by: Some("user".to_string()),
            chat_id: Some(42),
            input_summary: Some("send status update".to_string()),
            user_visible_summary: Some("Status update queued".to_string()),
            idempotency_key: Some(format!("idem-{action_id}")),
            trace_id: Some("trace-1".to_string()),
            run_id: Some("run-1".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn action_ledger_create_proposed() {
        let conn = test_conn();
        let store = ActionLedgerStore::new(&conn);
        let entry = store
            .create_proposed(&new_action("act-create", ActionKind::TelegramSend))
            .unwrap();

        assert_eq!(entry.action_id, "act-create");
        assert_eq!(entry.action_kind, ActionKind::TelegramSend);
        assert_eq!(entry.status, ActionStatus::Proposed);
        assert!(entry.proposed_at.is_some());
        assert_eq!(entry.trace_id.as_deref(), Some("trace-1"));
    }

    #[test]
    fn action_ledger_transition_to_pending_approval() {
        let conn = test_conn();
        let store = ActionLedgerStore::new(&conn);
        store
            .create_proposed(&new_action("act-approval", ActionKind::ApprovalRequest))
            .unwrap();

        let entry = store
            .transition(
                "act-approval",
                ActionStatus::PendingApproval,
                &ActionUpdate::default(),
            )
            .unwrap();

        assert_eq!(entry.status, ActionStatus::PendingApproval);
        assert!(entry.proposed_at.is_some());
    }

    #[test]
    fn runtime_setting_proposal_records_pending_approval() {
        let conn = test_conn();
        let entry = record_runtime_setting_proposed(
            &conn,
            "config-test",
            "user_timezone",
            &serde_json::json!("America/New_York"),
            &serde_json::json!("Europe/London"),
            "did:aegis:telegram:42",
            Some(42),
            Some("tool-call-1"),
            false,
        )
        .unwrap();

        assert_eq!(entry.action_id, "runtime-setting:config-test");
        assert_eq!(entry.action_kind, ActionKind::RuntimeSettingChange);
        assert_eq!(entry.status, ActionStatus::PendingApproval);
        assert_eq!(entry.chat_id, Some(42));
        assert_eq!(entry.tool_call_id.as_deref(), Some("tool-call-1"));
        assert_eq!(entry.external_event_id.as_deref(), Some("config-test"));
    }

    #[test]
    fn runtime_setting_execution_requires_and_keeps_receipt() {
        let conn = test_conn();
        record_runtime_setting_proposed(
            &conn,
            "config-apply",
            "user_timezone",
            &serde_json::json!("America/New_York"),
            &serde_json::json!("Europe/London"),
            "did:aegis:telegram:42",
            Some(42),
            Some("tool-call-2"),
            true,
        )
        .unwrap();

        transition_runtime_setting_action(
            &conn,
            "config-apply",
            ActionStatus::Executing,
            &ActionUpdate::default(),
        )
        .unwrap();
        let entry = transition_runtime_setting_action(
            &conn,
            "config-apply",
            ActionStatus::Executed,
            &ActionUpdate {
                external_receipt: Some("runtime_setting:user_timezone:config-apply".to_string()),
                external_event_id: Some("config-apply".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(entry.status, ActionStatus::Executed);
        assert_eq!(
            entry.external_receipt.as_deref(),
            Some("runtime_setting:user_timezone:config-apply")
        );
        assert!(entry.executing_at.is_some());
        assert!(entry.executed_at.is_some());
    }

    #[test]
    fn action_ledger_transition_to_executed_requires_receipt_for_external_action() {
        let conn = test_conn();
        let store = ActionLedgerStore::new(&conn);
        store
            .create_proposed(&new_action("act-receipt", ActionKind::TelegramSend))
            .unwrap();

        let err = store
            .transition(
                "act-receipt",
                ActionStatus::Executed,
                &ActionUpdate::default(),
            )
            .unwrap_err();
        assert!(err.to_string().contains("requires receipt-like evidence"));

        let entry = store
            .transition(
                "act-receipt",
                ActionStatus::Executed,
                &ActionUpdate {
                    external_message_id: Some("telegram:123".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(entry.status, ActionStatus::Executed);
        assert_eq!(entry.external_message_id.as_deref(), Some("telegram:123"));
        assert!(entry.executed_at.is_some());
    }

    #[test]
    fn action_ledger_failed_keeps_error() {
        let conn = test_conn();
        let store = ActionLedgerStore::new(&conn);
        store
            .create_proposed(&new_action("act-failed", ActionKind::EmailSend))
            .unwrap();

        let entry = store
            .transition(
                "act-failed",
                ActionStatus::Failed,
                &ActionUpdate {
                    error_kind: Some("smtp_auth".to_string()),
                    error_message: Some("auth failed".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();

        assert_eq!(entry.status, ActionStatus::Failed);
        assert_eq!(entry.error_kind.as_deref(), Some("smtp_auth"));
        assert_eq!(entry.error_message.as_deref(), Some("auth failed"));
        assert!(entry.failed_at.is_some());
    }

    #[test]
    fn action_ledger_retry_preserves_idempotency_key() {
        let conn = test_conn();
        let store = ActionLedgerStore::new(&conn);
        store
            .create_proposed(&new_action("act-retry", ActionKind::CalendarCreate))
            .unwrap();
        store
            .transition(
                "act-retry",
                ActionStatus::Failed,
                &ActionUpdate {
                    error_kind: Some("timeout".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();

        let entry = store
            .transition(
                "act-retry",
                ActionStatus::Executing,
                &ActionUpdate::default(),
            )
            .unwrap();

        assert_eq!(entry.status, ActionStatus::Executing);
        assert_eq!(entry.idempotency_key.as_deref(), Some("idem-act-retry"));
    }

    #[test]
    fn action_ledger_query_recent_by_status() {
        let conn = test_conn();
        let store = ActionLedgerStore::new(&conn);
        store
            .create_proposed(&new_action("act-old", ActionKind::TaskCreate))
            .unwrap();
        store
            .create_proposed(&new_action("act-new", ActionKind::TaskCreate))
            .unwrap();

        let proposed = store
            .list_recent_by_status(ActionStatus::Proposed, 1)
            .unwrap();

        assert_eq!(proposed.len(), 1);
        assert_eq!(proposed[0].action_id, "act-new");
    }

    #[test]
    fn action_ledger_query_recent_across_statuses() {
        let conn = test_conn();
        let store = ActionLedgerStore::new(&conn);
        store
            .create_proposed(&new_action("act-old", ActionKind::TaskCreate))
            .unwrap();
        store
            .create_proposed(&new_action("act-new", ActionKind::TaskCreate))
            .unwrap();
        store
            .transition(
                "act-new",
                ActionStatus::Failed,
                &ActionUpdate {
                    error_kind: Some("task_failed".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();

        let recent = store.list_recent(2).unwrap();

        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].action_id, "act-new");
        assert_eq!(recent[0].status, ActionStatus::Failed);
        assert_eq!(recent[1].action_id, "act-old");
        assert_eq!(recent[1].status, ActionStatus::Proposed);
    }

    #[test]
    fn action_ledger_get_by_id() {
        let conn = test_conn();
        let store = ActionLedgerStore::new(&conn);
        let created = store
            .create_proposed(&new_action("act-by-id", ActionKind::TaskCreate))
            .unwrap();

        let loaded = store.get_by_id(created.id).unwrap().expect("ledger row");

        assert_eq!(loaded.id, created.id);
        assert_eq!(loaded.action_id, "act-by-id");
        assert!(store.get_by_id(created.id + 1).unwrap().is_none());
    }

    #[test]
    fn action_ledger_user_visible_summary_is_available_for_brief() {
        let conn = test_conn();
        let store = ActionLedgerStore::new(&conn);
        store
            .create_proposed(&NewAction {
                user_visible_summary: Some("Drafted reply to Alice".to_string()),
                ..new_action("act-summary", ActionKind::EmailDraft)
            })
            .unwrap();

        let entry = store.get_by_action_id("act-summary").unwrap().unwrap();

        assert_eq!(
            entry.user_visible_summary.as_deref(),
            Some("Drafted reply to Alice")
        );
    }
}
