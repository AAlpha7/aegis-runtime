use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MandateStatus {
    Draft,
    Active,
    Paused,
    Expired,
    Revoked,
}

impl MandateStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
        }
    }

    fn from_db(value: String) -> Result<Self> {
        match value.as_str() {
            "draft" => Ok(Self::Draft),
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "expired" => Ok(Self::Expired),
            "revoked" => Ok(Self::Revoked),
            _ => Err(anyhow!("unknown mandate status: {value}")),
        }
    }

    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Draft, Self::Draft)
                | (Self::Draft, Self::Active)
                | (Self::Draft, Self::Revoked)
                | (Self::Active, Self::Paused)
                | (Self::Active, Self::Revoked)
                | (Self::Active, Self::Expired)
                | (Self::Paused, Self::Active)
                | (Self::Paused, Self::Revoked)
                | (Self::Paused, Self::Expired)
        )
    }
}

#[derive(Debug, Clone)]
pub struct NewUserMandate {
    pub mandate_id: String,
    pub title: String,
    pub description: String,
    pub owner_principal: String,
    pub scope_kind: String,
    pub source_filters_json: String,
    pub allowed_reads_json: String,
    pub allowed_llm_uses_json: String,
    pub allowed_actions_json: String,
    pub must_ask_json: String,
    pub notification_policy_json: String,
    pub budget_policy_json: String,
    pub frequency_policy_json: String,
    pub expiry: Option<String>,
    pub review_after: Option<String>,
    pub created_from: String,
    pub source_event_id: Option<String>,
    pub trace_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UserMandate {
    pub id: i64,
    pub mandate_id: String,
    pub title: String,
    pub description: String,
    pub owner_principal: String,
    pub status: MandateStatus,
    pub scope_kind: String,
    pub source_filters_json: String,
    pub allowed_reads_json: String,
    pub allowed_llm_uses_json: String,
    pub allowed_actions_json: String,
    pub must_ask_json: String,
    pub notification_policy_json: String,
    pub budget_policy_json: String,
    pub frequency_policy_json: String,
    pub expiry: Option<String>,
    pub review_after: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub created_from: String,
    pub source_event_id: Option<String>,
    pub trace_id: Option<String>,
    pub run_id: Option<String>,
    pub schema_version: u32,
}

#[derive(Debug, Clone)]
pub struct MandateAuditEvent {
    pub event_kind: String,
    pub actor_principal: String,
    pub status_from: Option<MandateStatus>,
    pub status_to: Option<MandateStatus>,
    pub summary: String,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewMandateNotificationPlan {
    pub mandate_id: String,
    pub owner_principal: String,
    pub source_event_id: String,
    pub idempotency_key: String,
    pub notification_class: String,
    pub privacy_level: String,
    pub route_reason: String,
    pub route_chat_id: i64,
    pub route_thread_id: Option<i64>,
    pub title: String,
    pub body: String,
    pub detail_url: Option<String>,
    pub redacted: bool,
    pub trace_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MandateNotificationRecord {
    pub id: i64,
    pub mandate_id: String,
    pub owner_principal: String,
    pub source_event_id: String,
    pub idempotency_key: String,
    pub status: String,
    pub notification_class: String,
    pub privacy_level: String,
    pub route_reason: String,
    pub route_chat_id: i64,
    pub route_thread_id: Option<i64>,
    pub title: String,
    pub body: String,
    pub detail_url: Option<String>,
    pub redacted: bool,
    pub created_at: String,
    pub trace_id: Option<String>,
    pub run_id: Option<String>,
    pub schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MandateNotificationDecision {
    Planned(MandateNotificationRecord),
    Duplicate(MandateNotificationRecord),
    DailyCapReached(MandateNotificationRecord),
}

pub struct UserMandateStore<'a> {
    conn: &'a Connection,
}

impl<'a> UserMandateStore<'a> {
    pub const fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn create_draft(&self, mandate: &NewUserMandate) -> Result<UserMandate> {
        if mandate.mandate_id.trim().is_empty() {
            return Err(anyhow!("mandate_id must not be empty"));
        }
        let now = now_rfc3339();
        self.conn
            .execute(
                "INSERT INTO user_mandates (
                    mandate_id, title, description, owner_principal, status, scope_kind,
                    source_filters_json, allowed_reads_json, allowed_llm_uses_json,
                    allowed_actions_json, must_ask_json, notification_policy_json,
                    budget_policy_json, frequency_policy_json, expiry, review_after,
                    created_at, updated_at, created_from, source_event_id, trace_id, run_id,
                    schema_version
                 ) VALUES (?1, ?2, ?3, ?4, 'draft', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?16, ?17, ?18, ?19, ?20, 1)",
                params![
                    mandate.mandate_id,
                    mandate.title,
                    mandate.description,
                    mandate.owner_principal,
                    mandate.scope_kind,
                    mandate.source_filters_json,
                    mandate.allowed_reads_json,
                    mandate.allowed_llm_uses_json,
                    mandate.allowed_actions_json,
                    mandate.must_ask_json,
                    mandate.notification_policy_json,
                    mandate.budget_policy_json,
                    mandate.frequency_policy_json,
                    mandate.expiry,
                    mandate.review_after,
                    now,
                    mandate.created_from,
                    mandate.source_event_id,
                    mandate.trace_id,
                    mandate.run_id,
                ],
            )
            .with_context(|| format!("creating mandate {}", mandate.mandate_id))?;
        let created = self
            .get_for_owner(&mandate.owner_principal, &mandate.mandate_id)?
            .ok_or_else(|| anyhow!("mandate row missing after insert"))?;
        self.record_audit(
            &created,
            &MandateAuditEvent {
                event_kind: "created".into(),
                actor_principal: mandate.owner_principal.clone(),
                status_from: None,
                status_to: Some(MandateStatus::Draft),
                summary: "Mandate draft created".into(),
                metadata_json: None,
            },
        )?;
        Ok(created)
    }

    pub fn update_draft(
        &self,
        owner: &str,
        mandate_id: &str,
        update: &NewUserMandate,
    ) -> Result<UserMandate> {
        let current = self
            .get_for_owner(owner, mandate_id)?
            .ok_or_else(|| anyhow!("mandate not found: {mandate_id}"))?;
        if current.status != MandateStatus::Draft && current.status != MandateStatus::Paused {
            return Err(anyhow!("only draft or paused mandates can be edited"));
        }
        let now = now_rfc3339();
        self.conn
            .execute(
                "UPDATE user_mandates SET
                    title = ?3,
                    description = ?4,
                    scope_kind = ?5,
                    source_filters_json = ?6,
                    allowed_reads_json = ?7,
                    allowed_llm_uses_json = ?8,
                    allowed_actions_json = ?9,
                    must_ask_json = ?10,
                    notification_policy_json = ?11,
                    budget_policy_json = ?12,
                    frequency_policy_json = ?13,
                    expiry = ?14,
                    review_after = ?15,
                    updated_at = ?16,
                    source_event_id = COALESCE(?17, source_event_id),
                    trace_id = COALESCE(?18, trace_id),
                    run_id = COALESCE(?19, run_id)
                 WHERE owner_principal = ?1 AND mandate_id = ?2",
                params![
                    owner,
                    mandate_id,
                    update.title,
                    update.description,
                    update.scope_kind,
                    update.source_filters_json,
                    update.allowed_reads_json,
                    update.allowed_llm_uses_json,
                    update.allowed_actions_json,
                    update.must_ask_json,
                    update.notification_policy_json,
                    update.budget_policy_json,
                    update.frequency_policy_json,
                    update.expiry,
                    update.review_after,
                    now,
                    update.source_event_id,
                    update.trace_id,
                    update.run_id,
                ],
            )
            .with_context(|| format!("updating mandate {mandate_id}"))?;
        let updated = self
            .get_for_owner(owner, mandate_id)?
            .ok_or_else(|| anyhow!("mandate row missing after update"))?;
        self.record_audit(
            &updated,
            &MandateAuditEvent {
                event_kind: "edited".into(),
                actor_principal: owner.to_string(),
                status_from: Some(current.status),
                status_to: Some(updated.status),
                summary: "Mandate edited".into(),
                metadata_json: None,
            },
        )?;
        Ok(updated)
    }

    pub fn transition(
        &self,
        owner: &str,
        mandate_id: &str,
        next: MandateStatus,
        actor: &str,
        summary: &str,
    ) -> Result<UserMandate> {
        let current = self
            .get_for_owner(owner, mandate_id)?
            .ok_or_else(|| anyhow!("mandate not found: {mandate_id}"))?;
        if !current.status.can_transition_to(next) {
            return Err(anyhow!(
                "invalid mandate transition: {} -> {}",
                current.status.as_str(),
                next.as_str()
            ));
        }
        let now = now_rfc3339();
        self.conn
            .execute(
                "UPDATE user_mandates SET status = ?3, updated_at = ?4
                 WHERE owner_principal = ?1 AND mandate_id = ?2",
                params![owner, mandate_id, next.as_str(), now],
            )
            .with_context(|| format!("transitioning mandate {mandate_id}"))?;
        let updated = self
            .get_for_owner(owner, mandate_id)?
            .ok_or_else(|| anyhow!("mandate row missing after transition"))?;
        self.record_audit(
            &updated,
            &MandateAuditEvent {
                event_kind: next.as_str().to_string(),
                actor_principal: actor.to_string(),
                status_from: Some(current.status),
                status_to: Some(next),
                summary: summary.to_string(),
                metadata_json: None,
            },
        )?;
        Ok(updated)
    }

    pub fn list_for_owner(&self, owner: &str, limit: usize) -> Result<Vec<UserMandate>> {
        let limit = i64::try_from(limit).context("mandate limit overflow")?;
        let mut stmt = self
            .conn
            .prepare(
                &(USER_MANDATE_SELECT_SQL.to_string()
                    + " WHERE owner_principal = ?1 ORDER BY id DESC LIMIT ?2"),
            )
            .context("preparing mandate list query")?;
        let rows = stmt
            .query_map(params![owner, limit], map_user_mandate_row)
            .context("querying mandate list")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("mapping mandate rows")
    }

    pub fn get_for_owner(&self, owner: &str, mandate_id: &str) -> Result<Option<UserMandate>> {
        self.conn
            .query_row(
                &(USER_MANDATE_SELECT_SQL.to_string()
                    + " WHERE owner_principal = ?1 AND mandate_id = ?2"),
                params![owner, mandate_id],
                map_user_mandate_row,
            )
            .optional()
            .context("querying mandate by owner/id")
    }

    pub fn plan_notification(
        &self,
        plan: &NewMandateNotificationPlan,
        max_per_day: usize,
    ) -> Result<MandateNotificationDecision> {
        if plan.idempotency_key.trim().is_empty() {
            return Err(anyhow!("idempotency_key must not be empty"));
        }
        if let Some(existing) = self.notification_by_idempotency_key(&plan.idempotency_key)? {
            return Ok(MandateNotificationDecision::Duplicate(existing));
        }

        let now = Utc::now();
        let since = (now - chrono::Duration::hours(24)).to_rfc3339();
        let recent_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM user_mandate_notifications
             WHERE owner_principal = ?1
               AND mandate_id = ?2
               AND status IN ('planned', 'sent')
               AND created_at >= ?3",
            params![plan.owner_principal, plan.mandate_id, since],
            |row| row.get(0),
        )?;
        let status = if recent_count >= i64::try_from(max_per_day).unwrap_or(i64::MAX) {
            "daily_cap_suppressed"
        } else {
            "planned"
        };
        let record = self.insert_notification(plan, status, &now.to_rfc3339())?;
        if status == "planned" {
            Ok(MandateNotificationDecision::Planned(record))
        } else {
            Ok(MandateNotificationDecision::DailyCapReached(record))
        }
    }

    pub fn notification_by_idempotency_key(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<MandateNotificationRecord>> {
        self.conn
            .query_row(
                "SELECT id, mandate_id, owner_principal, source_event_id, idempotency_key,
                        status, notification_class, privacy_level, route_reason, route_chat_id,
                        route_thread_id, title, body, detail_url, redacted, created_at, trace_id,
                        run_id, schema_version
                 FROM user_mandate_notifications WHERE idempotency_key = ?1",
                params![idempotency_key],
                map_mandate_notification_row,
            )
            .optional()
            .context("querying mandate notification by idempotency key")
    }

    fn insert_notification(
        &self,
        plan: &NewMandateNotificationPlan,
        status: &str,
        created_at: &str,
    ) -> Result<MandateNotificationRecord> {
        self.conn.execute(
            "INSERT INTO user_mandate_notifications (
                mandate_id, owner_principal, source_event_id, idempotency_key, status,
                notification_class, privacy_level, route_reason, route_chat_id, route_thread_id,
                title, body, detail_url, redacted, created_at, trace_id, run_id, schema_version
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, 1)",
            params![
                plan.mandate_id,
                plan.owner_principal,
                plan.source_event_id,
                plan.idempotency_key,
                status,
                plan.notification_class,
                plan.privacy_level,
                plan.route_reason,
                plan.route_chat_id,
                plan.route_thread_id,
                plan.title,
                plan.body,
                plan.detail_url,
                if plan.redacted { 1 } else { 0 },
                created_at,
                plan.trace_id,
                plan.run_id,
            ],
        )?;
        self.notification_by_idempotency_key(&plan.idempotency_key)?
            .ok_or_else(|| anyhow!("notification row missing after insert"))
    }

    fn record_audit(&self, mandate: &UserMandate, event: &MandateAuditEvent) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO user_mandate_audit (
                    mandate_id, owner_principal, event_kind, actor_principal, created_at,
                    status_from, status_to, summary, metadata_json, source_event_id, trace_id,
                    run_id, schema_version
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1)",
                params![
                    mandate.mandate_id,
                    mandate.owner_principal,
                    event.event_kind,
                    event.actor_principal,
                    now_rfc3339(),
                    event.status_from.map(MandateStatus::as_str),
                    event.status_to.map(MandateStatus::as_str),
                    event.summary,
                    event.metadata_json,
                    mandate.source_event_id,
                    mandate.trace_id,
                    mandate.run_id,
                ],
            )
            .context("recording mandate audit event")?;
        Ok(())
    }
}

fn map_mandate_notification_row(row: &Row<'_>) -> rusqlite::Result<MandateNotificationRecord> {
    let schema_version_i64: i64 = row.get(18)?;
    let schema_version = u32::try_from(schema_version_i64).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(18, rusqlite::types::Type::Integer, Box::new(err))
    })?;
    Ok(MandateNotificationRecord {
        id: row.get(0)?,
        mandate_id: row.get(1)?,
        owner_principal: row.get(2)?,
        source_event_id: row.get(3)?,
        idempotency_key: row.get(4)?,
        status: row.get(5)?,
        notification_class: row.get(6)?,
        privacy_level: row.get(7)?,
        route_reason: row.get(8)?,
        route_chat_id: row.get(9)?,
        route_thread_id: row.get(10)?,
        title: row.get(11)?,
        body: row.get(12)?,
        detail_url: row.get(13)?,
        redacted: row.get::<_, i64>(14)? != 0,
        created_at: row.get(15)?,
        trace_id: row.get(16)?,
        run_id: row.get(17)?,
        schema_version,
    })
}

const USER_MANDATE_SELECT_SQL: &str = "SELECT
    id, mandate_id, title, description, owner_principal, status, scope_kind,
    source_filters_json, allowed_reads_json, allowed_llm_uses_json, allowed_actions_json,
    must_ask_json, notification_policy_json, budget_policy_json, frequency_policy_json,
    expiry, review_after, created_at, updated_at, created_from, source_event_id, trace_id,
    run_id, schema_version
    FROM user_mandates";

fn map_user_mandate_row(row: &Row<'_>) -> rusqlite::Result<UserMandate> {
    let status = MandateStatus::from_db(row.get::<_, String>(5)?).map_err(to_sql_error)?;
    let schema_version_i64: i64 = row.get(23)?;
    let schema_version = u32::try_from(schema_version_i64).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(23, rusqlite::types::Type::Integer, Box::new(err))
    })?;
    Ok(UserMandate {
        id: row.get(0)?,
        mandate_id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        owner_principal: row.get(4)?,
        status,
        scope_kind: row.get(6)?,
        source_filters_json: row.get(7)?,
        allowed_reads_json: row.get(8)?,
        allowed_llm_uses_json: row.get(9)?,
        allowed_actions_json: row.get(10)?,
        must_ask_json: row.get(11)?,
        notification_policy_json: row.get(12)?,
        budget_policy_json: row.get(13)?,
        frequency_policy_json: row.get(14)?,
        expiry: row.get(15)?,
        review_after: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        created_from: row.get(19)?,
        source_event_id: row.get(20)?,
        trace_id: row.get(21)?,
        run_id: row.get(22)?,
        schema_version,
    })
}

fn to_sql_error(err: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, err.into())
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_mandate(id: &str) -> NewUserMandate {
        NewUserMandate {
            mandate_id: id.into(),
            title: "Legal email watch".into(),
            description: "Surface legal email metadata only.".into(),
            owner_principal: "did:aegis:telegram:12345678".into(),
            scope_kind: "email_metadata".into(),
            source_filters_json: "{}".into(),
            allowed_reads_json: "[\"email_metadata\"]".into(),
            allowed_llm_uses_json: "[]".into(),
            allowed_actions_json: "[\"surface_card\"]".into(),
            must_ask_json: "[\"email_body\"]".into(),
            notification_policy_json: "{}".into(),
            budget_policy_json: "{}".into(),
            frequency_policy_json: "{}".into(),
            expiry: None,
            review_after: None,
            created_from: "webapp".into(),
            source_event_id: None,
            trace_id: None,
            run_id: None,
        }
    }

    #[test]
    fn mandate_store_lifecycle_round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        crate::memory_migrations::run_all_migrations(&conn).unwrap();
        let store = UserMandateStore::new(&conn);
        let draft = store.create_draft(&new_mandate("mandate-test")).unwrap();
        assert_eq!(draft.status, MandateStatus::Draft);
        let active = store
            .transition(
                &draft.owner_principal,
                &draft.mandate_id,
                MandateStatus::Active,
                &draft.owner_principal,
                "Activated",
            )
            .unwrap();
        assert_eq!(active.status, MandateStatus::Active);
        let listed = store.list_for_owner(&active.owner_principal, 10).unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[test]
    fn mandate_notification_plan_dedupes_and_enforces_daily_cap() {
        let conn = Connection::open_in_memory().unwrap();
        crate::memory_migrations::run_all_migrations(&conn).unwrap();
        let store = UserMandateStore::new(&conn);
        let mandate = store.create_draft(&new_mandate("mandate-notify")).unwrap();
        let base_plan = NewMandateNotificationPlan {
            mandate_id: mandate.mandate_id.clone(),
            owner_principal: mandate.owner_principal.clone(),
            source_event_id: "assistant_event:1".into(),
            idempotency_key: "mandate-notification:1".into(),
            notification_class: "proactive_alert".into(),
            privacy_level: "SensitivePreferDm".into(),
            route_reason: "SensitivePreferDm".into(),
            route_chat_id: 12345678,
            route_thread_id: None,
            title: "Sensitive item needs review".into(),
            body: "Open the Web Console and complete step-up MFA to view details.".into(),
            detail_url: Some("/ui/?tab=mail&item=email:1".into()),
            redacted: true,
            trace_id: Some("trace:test".into()),
            run_id: Some("run:test".into()),
        };

        let first = store.plan_notification(&base_plan, 1).unwrap();
        assert!(matches!(first, MandateNotificationDecision::Planned(_)));
        let duplicate = store.plan_notification(&base_plan, 1).unwrap();
        assert!(matches!(
            duplicate,
            MandateNotificationDecision::Duplicate(_)
        ));

        let mut capped_plan = base_plan.clone();
        capped_plan.source_event_id = "assistant_event:2".into();
        capped_plan.idempotency_key = "mandate-notification:2".into();
        let capped = store.plan_notification(&capped_plan, 1).unwrap();
        match capped {
            MandateNotificationDecision::DailyCapReached(record) => {
                assert_eq!(record.status, "daily_cap_suppressed");
                assert!(record.redacted);
                assert_eq!(record.trace_id.as_deref(), Some("trace:test"));
            }
            other => panic!("expected daily cap suppression, got {other:?}"),
        }
    }
}
