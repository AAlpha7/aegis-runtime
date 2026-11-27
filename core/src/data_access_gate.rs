//! Sensitive read policy primitives for the Web Console trust layer.
//!
//! Phase 1 is intentionally pure policy: no Google, Calendar, memory, LLM, or
//! grant-storage integration happens here. Callers must classify the resource
//! before fetching private content, then ask this module what authorization is
//! required.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::web_console_auth::MfaLevel;

pub const DEFAULT_MFA_FRESHNESS_SECS: u64 = 600;
pub const READ_GRANT_SCHEMA_VERSION: i64 = 1;
pub const DEFAULT_READ_GRANT_TTL_SECS: u64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSensitivity {
    S0Public,
    S1LowPersonal,
    S2Personal,
    S3Sensitive,
    S4HighlySensitive,
}

impl DataSensitivity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::S0Public => "S0Public",
            Self::S1LowPersonal => "S1LowPersonal",
            Self::S2Personal => "S2Personal",
            Self::S3Sensitive => "S3Sensitive",
            Self::S4HighlySensitive => "S4HighlySensitive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataAction {
    ListMetadata,
    ReadBody,
    ReadAttachment,
    AnalyzeImage,
    SummarizeWithLlm,
    StoreInMemory,
    Export,
    DisplayInUi,
    NotifyUser,
}

impl DataAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ListMetadata => "ListMetadata",
            Self::ReadBody => "ReadBody",
            Self::ReadAttachment => "ReadAttachment",
            Self::AnalyzeImage => "AnalyzeImage",
            Self::SummarizeWithLlm => "SummarizeWithLlm",
            Self::StoreInMemory => "StoreInMemory",
            Self::Export => "Export",
            Self::DisplayInUi => "DisplayInUi",
            Self::NotifyUser => "NotifyUser",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredMfa {
    None,
    TotpOrWebAuthn,
    WebAuthn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataAccessPolicy {
    pub requires_authenticated_session: bool,
    pub required_mfa: RequiredMfa,
    pub freshness_secs: u64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataAccessPolicyDecision {
    Satisfied,
    RequireMfa {
        required_level: RequiredMfa,
        reason: String,
        freshness_secs: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewReadGrant {
    pub user_did: String,
    pub principal_type: String,
    pub principal_id: String,
    pub resource_type: String,
    pub resource_id: String,
    pub purpose: String,
    pub sensitivity: DataSensitivity,
    pub allowed_actions: Vec<DataAction>,
    pub mfa_level: Option<MfaLevel>,
    pub mfa_verified_at: Option<u64>,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadGrant {
    pub grant_id: String,
    pub user_did: String,
    pub principal_type: String,
    pub principal_id_hash: String,
    pub resource_type: String,
    pub resource_id_hash: String,
    pub purpose: String,
    pub sensitivity: DataSensitivity,
    pub allowed_actions: Vec<DataAction>,
    pub mfa_level: Option<MfaLevel>,
    pub mfa_verified_at: Option<u64>,
    pub created_at: u64,
    pub expires_at: u64,
    pub consumed_at: Option<u64>,
    pub revoked_at: Option<u64>,
    pub schema_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadGrantLookup {
    pub user_did: String,
    pub principal_type: String,
    pub principal_id: String,
    pub resource_type: String,
    pub resource_id: String,
    pub purpose: String,
    pub requested_actions: Vec<DataAction>,
    pub now: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitiveReadAuditStatus {
    Allowed,
    RequireMfa,
    Denied,
    Error,
}

impl SensitiveReadAuditStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::RequireMfa => "require_mfa",
            Self::Denied => "denied",
            Self::Error => "error",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "allowed" => Self::Allowed,
            "require_mfa" => Self::RequireMfa,
            "denied" => Self::Denied,
            "error" => Self::Error,
            _ => Self::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSensitiveReadAuditEvent {
    pub user_did: String,
    pub principal_type: String,
    pub principal_id: String,
    pub resource_type: String,
    pub resource_id: String,
    pub purpose: String,
    pub sensitivity: DataSensitivity,
    pub actions: Vec<DataAction>,
    pub mfa_level: Option<MfaLevel>,
    pub mfa_verified_at: Option<u64>,
    pub grant_id: Option<String>,
    pub status: SensitiveReadAuditStatus,
    pub model: Option<String>,
    pub details: Option<serde_json::Value>,
    pub trace_id: Option<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensitiveReadAuditEvent {
    pub audit_id: String,
    pub user_did: String,
    pub principal_type: String,
    pub principal_id_hash: String,
    pub resource_type: String,
    pub resource_id_hash: String,
    pub purpose: String,
    pub sensitivity: DataSensitivity,
    pub actions: Vec<DataAction>,
    pub mfa_level: Option<MfaLevel>,
    pub mfa_verified_at: Option<u64>,
    pub grant_id: Option<String>,
    pub status: SensitiveReadAuditStatus,
    pub model: Option<String>,
    pub details: Option<serde_json::Value>,
    pub trace_id: Option<String>,
    pub created_at: u64,
    pub schema_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPrincipal {
    pub user_did: String,
    pub principal_type: String,
    pub principal_id: String,
    pub mfa_level: MfaLevel,
    pub mfa_verified_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataAccessRequest {
    pub principal: AuthPrincipal,
    pub resource_type: String,
    pub resource_id: String,
    pub sensitivity: DataSensitivity,
    pub actions: Vec<DataAction>,
    pub purpose: String,
    pub now: u64,
    pub grant_ttl_secs: u64,
    pub mfa_freshness_secs: Option<u64>,
    pub model: Option<String>,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataAccessDecision {
    Allow {
        grant: Option<ReadGrant>,
        reason: String,
    },
    RequireMfa {
        required_level: RequiredMfa,
        reason: String,
        freshness_secs: u64,
    },
    Deny {
        reason: String,
    },
}

pub fn policy_for_request(
    sensitivity: DataSensitivity,
    actions: &[DataAction],
) -> DataAccessPolicy {
    let effective_sensitivity = effective_sensitivity(sensitivity, actions);
    let required_mfa = match effective_sensitivity {
        DataSensitivity::S0Public
        | DataSensitivity::S1LowPersonal
        | DataSensitivity::S2Personal => RequiredMfa::None,
        DataSensitivity::S3Sensitive => RequiredMfa::TotpOrWebAuthn,
        DataSensitivity::S4HighlySensitive => RequiredMfa::WebAuthn,
    };
    let requires_authenticated_session =
        !matches!(effective_sensitivity, DataSensitivity::S0Public);
    let reason = policy_reason(effective_sensitivity, actions);

    DataAccessPolicy {
        requires_authenticated_session,
        required_mfa,
        freshness_secs: DEFAULT_MFA_FRESHNESS_SECS,
        reason,
    }
}

pub fn evaluate_mfa(
    policy: &DataAccessPolicy,
    current_level: MfaLevel,
    mfa_verified_at: Option<u64>,
    now: u64,
) -> DataAccessPolicyDecision {
    if policy.required_mfa == RequiredMfa::None {
        return DataAccessPolicyDecision::Satisfied;
    }
    if !is_recent_mfa(
        policy.required_mfa,
        current_level,
        mfa_verified_at,
        now,
        policy.freshness_secs,
    ) {
        return DataAccessPolicyDecision::RequireMfa {
            required_level: policy.required_mfa,
            reason: policy.reason.clone(),
            freshness_secs: policy.freshness_secs,
        };
    }
    DataAccessPolicyDecision::Satisfied
}

pub fn is_recent_mfa(
    required: RequiredMfa,
    current_level: MfaLevel,
    mfa_verified_at: Option<u64>,
    now: u64,
    freshness_secs: u64,
) -> bool {
    if required == RequiredMfa::None {
        return true;
    }
    let Some(verified_at) = mfa_verified_at else {
        return false;
    };
    if verified_at > now || now.saturating_sub(verified_at) > freshness_secs {
        return false;
    }
    match required {
        RequiredMfa::None => true,
        RequiredMfa::TotpOrWebAuthn => matches!(
            current_level,
            MfaLevel::GoogleFresh | MfaLevel::Totp | MfaLevel::WebAuthn | MfaLevel::Host
        ),
        RequiredMfa::WebAuthn => matches!(current_level, MfaLevel::WebAuthn),
    }
}

pub fn init_sensitive_read_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS sensitive_read_grants (
            grant_id TEXT PRIMARY KEY,
            user_did TEXT NOT NULL,
            principal_type TEXT NOT NULL,
            principal_id_hash TEXT NOT NULL,
            resource_type TEXT NOT NULL,
            resource_id_hash TEXT NOT NULL,
            purpose TEXT NOT NULL,
            sensitivity TEXT NOT NULL,
            allowed_actions_json TEXT NOT NULL,
            mfa_level TEXT,
            mfa_verified_at INTEGER,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            consumed_at INTEGER,
            revoked_at INTEGER,
            schema_version INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS idx_sensitive_read_grants_user_expires
            ON sensitive_read_grants(user_did, expires_at, revoked_at);
        CREATE INDEX IF NOT EXISTS idx_sensitive_read_grants_lookup
            ON sensitive_read_grants(
                user_did,
                principal_type,
                principal_id_hash,
                resource_type,
                resource_id_hash,
                purpose,
                expires_at
            );

        CREATE TABLE IF NOT EXISTS sensitive_read_audit (
            audit_id TEXT PRIMARY KEY,
            user_did TEXT NOT NULL,
            principal_type TEXT NOT NULL,
            principal_id_hash TEXT NOT NULL,
            resource_type TEXT NOT NULL,
            resource_id_hash TEXT NOT NULL,
            purpose TEXT NOT NULL,
            sensitivity TEXT NOT NULL,
            actions_json TEXT NOT NULL,
            mfa_level TEXT,
            mfa_verified_at INTEGER,
            grant_id TEXT,
            status TEXT NOT NULL,
            model TEXT,
            details_json TEXT,
            trace_id TEXT,
            created_at INTEGER NOT NULL,
            schema_version INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS idx_sensitive_read_audit_user_created
            ON sensitive_read_audit(user_did, created_at);
        "#,
    )
}

pub fn create_read_grant(conn: &Connection, request: &NewReadGrant) -> rusqlite::Result<ReadGrant> {
    let grant_id = format!("rg_{}", hex::encode(rand::random::<[u8; 16]>()));
    let principal_id_hash = hash_identifier(&request.principal_id);
    let resource_id_hash = hash_identifier(&request.resource_id);
    let allowed_actions_json = actions_to_json(&request.allowed_actions)?;
    conn.execute(
        r#"
        INSERT INTO sensitive_read_grants (
            grant_id,
            user_did,
            principal_type,
            principal_id_hash,
            resource_type,
            resource_id_hash,
            purpose,
            sensitivity,
            allowed_actions_json,
            mfa_level,
            mfa_verified_at,
            created_at,
            expires_at,
            schema_version
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
        params![
            grant_id,
            request.user_did,
            request.principal_type,
            principal_id_hash,
            request.resource_type,
            resource_id_hash,
            request.purpose,
            request.sensitivity.as_str(),
            allowed_actions_json,
            request.mfa_level.map(MfaLevel::as_str),
            opt_u64_to_i64(request.mfa_verified_at)?,
            u64_to_i64(request.created_at)?,
            u64_to_i64(request.expires_at)?,
            READ_GRANT_SCHEMA_VERSION,
        ],
    )?;

    Ok(ReadGrant {
        grant_id,
        user_did: request.user_did.clone(),
        principal_type: request.principal_type.clone(),
        principal_id_hash,
        resource_type: request.resource_type.clone(),
        resource_id_hash,
        purpose: request.purpose.clone(),
        sensitivity: request.sensitivity,
        allowed_actions: request.allowed_actions.clone(),
        mfa_level: request.mfa_level,
        mfa_verified_at: request.mfa_verified_at,
        created_at: request.created_at,
        expires_at: request.expires_at,
        consumed_at: None,
        revoked_at: None,
        schema_version: READ_GRANT_SCHEMA_VERSION,
    })
}

pub fn find_valid_read_grant(
    conn: &Connection,
    lookup: &ReadGrantLookup,
) -> rusqlite::Result<Option<ReadGrant>> {
    let principal_id_hash = hash_identifier(&lookup.principal_id);
    let resource_id_hash = hash_identifier(&lookup.resource_id);
    let mut stmt = conn.prepare(
        r#"
        SELECT *
        FROM sensitive_read_grants
        WHERE user_did = ?1
          AND principal_type = ?2
          AND principal_id_hash = ?3
          AND resource_type = ?4
          AND resource_id_hash = ?5
          AND purpose = ?6
          AND expires_at >= ?7
          AND consumed_at IS NULL
          AND revoked_at IS NULL
        ORDER BY expires_at DESC
        LIMIT 8
        "#,
    )?;
    let rows = stmt.query_map(
        params![
            lookup.user_did,
            lookup.principal_type,
            principal_id_hash,
            lookup.resource_type,
            resource_id_hash,
            lookup.purpose,
            u64_to_i64(lookup.now)?,
        ],
        read_grant_from_row,
    )?;

    for grant in rows {
        let grant = grant?;
        if actions_cover(&grant.allowed_actions, &lookup.requested_actions) {
            return Ok(Some(grant));
        }
    }
    Ok(None)
}

pub fn record_sensitive_read_audit(
    conn: &Connection,
    event: &NewSensitiveReadAuditEvent,
) -> rusqlite::Result<SensitiveReadAuditEvent> {
    if let Some(details) = &event.details {
        reject_sensitive_audit_details(details)?;
    }
    let audit_id = format!("sra_{}", hex::encode(rand::random::<[u8; 16]>()));
    let principal_id_hash = hash_identifier(&event.principal_id);
    let resource_id_hash = hash_identifier(&event.resource_id);
    let actions_json = actions_to_json(&event.actions)?;
    let details_json = event
        .details
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| to_sql_error(anyhow::anyhow!(e)))?;

    conn.execute(
        r#"
        INSERT INTO sensitive_read_audit (
            audit_id,
            user_did,
            principal_type,
            principal_id_hash,
            resource_type,
            resource_id_hash,
            purpose,
            sensitivity,
            actions_json,
            mfa_level,
            mfa_verified_at,
            grant_id,
            status,
            model,
            details_json,
            trace_id,
            created_at,
            schema_version
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
        "#,
        params![
            audit_id,
            event.user_did,
            event.principal_type,
            principal_id_hash,
            event.resource_type,
            resource_id_hash,
            event.purpose,
            event.sensitivity.as_str(),
            actions_json,
            event.mfa_level.map(MfaLevel::as_str),
            opt_u64_to_i64(event.mfa_verified_at)?,
            event.grant_id,
            event.status.as_str(),
            event.model,
            details_json,
            event.trace_id,
            u64_to_i64(event.created_at)?,
            READ_GRANT_SCHEMA_VERSION,
        ],
    )?;

    Ok(SensitiveReadAuditEvent {
        audit_id,
        user_did: event.user_did.clone(),
        principal_type: event.principal_type.clone(),
        principal_id_hash,
        resource_type: event.resource_type.clone(),
        resource_id_hash,
        purpose: event.purpose.clone(),
        sensitivity: event.sensitivity,
        actions: event.actions.clone(),
        mfa_level: event.mfa_level,
        mfa_verified_at: event.mfa_verified_at,
        grant_id: event.grant_id.clone(),
        status: event.status,
        model: event.model.clone(),
        details: event.details.clone(),
        trace_id: event.trace_id.clone(),
        created_at: event.created_at,
        schema_version: READ_GRANT_SCHEMA_VERSION,
    })
}

pub fn latest_sensitive_read_audit(
    conn: &Connection,
    user_did: &str,
) -> rusqlite::Result<Option<SensitiveReadAuditEvent>> {
    conn.query_row(
        r#"
        SELECT *
        FROM sensitive_read_audit
        WHERE user_did = ?1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
        params![user_did],
        sensitive_read_audit_from_row,
    )
    .optional()
}

pub fn check_or_request_access(
    conn: &Connection,
    request: &DataAccessRequest,
) -> DataAccessDecision {
    let mut policy = policy_for_request(request.sensitivity, &request.actions);
    if let Some(freshness_secs) = request.mfa_freshness_secs {
        policy.freshness_secs = freshness_secs;
    }
    if policy.requires_authenticated_session
        && (request.principal.user_did.trim().is_empty()
            || request.principal.principal_type.trim().is_empty()
            || request.principal.principal_id.trim().is_empty())
    {
        audit_gate_decision(conn, request, None, SensitiveReadAuditStatus::Denied);
        return DataAccessDecision::Deny {
            reason: "authenticated session required".into(),
        };
    }

    let existing = find_valid_read_grant(
        conn,
        &ReadGrantLookup {
            user_did: request.principal.user_did.clone(),
            principal_type: request.principal.principal_type.clone(),
            principal_id: request.principal.principal_id.clone(),
            resource_type: request.resource_type.clone(),
            resource_id: request.resource_id.clone(),
            purpose: request.purpose.clone(),
            requested_actions: request.actions.clone(),
            now: request.now,
        },
    );
    match existing {
        Ok(Some(grant)) => {
            audit_gate_decision(
                conn,
                request,
                Some(grant.grant_id.clone()),
                SensitiveReadAuditStatus::Allowed,
            );
            return DataAccessDecision::Allow {
                grant: Some(grant),
                reason: "existing grant".into(),
            };
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(error = %error, "DataAccessGate grant lookup failed closed");
            audit_gate_decision(conn, request, None, SensitiveReadAuditStatus::Error);
            return DataAccessDecision::Deny {
                reason: "data access gate storage error".into(),
            };
        }
    }

    match evaluate_mfa(
        &policy,
        request.principal.mfa_level,
        request.principal.mfa_verified_at,
        request.now,
    ) {
        DataAccessPolicyDecision::RequireMfa {
            required_level,
            reason,
            freshness_secs,
        } => {
            audit_gate_decision(conn, request, None, SensitiveReadAuditStatus::RequireMfa);
            DataAccessDecision::RequireMfa {
                required_level,
                reason,
                freshness_secs,
            }
        }
        DataAccessPolicyDecision::Satisfied => {
            if !policy.requires_authenticated_session {
                audit_gate_decision(conn, request, None, SensitiveReadAuditStatus::Allowed);
                return DataAccessDecision::Allow {
                    grant: None,
                    reason: "public access".into(),
                };
            }

            let ttl = if request.grant_ttl_secs == 0 {
                DEFAULT_READ_GRANT_TTL_SECS
            } else {
                request.grant_ttl_secs
            };
            match create_read_grant(
                conn,
                &NewReadGrant {
                    user_did: request.principal.user_did.clone(),
                    principal_type: request.principal.principal_type.clone(),
                    principal_id: request.principal.principal_id.clone(),
                    resource_type: request.resource_type.clone(),
                    resource_id: request.resource_id.clone(),
                    purpose: request.purpose.clone(),
                    sensitivity: request.sensitivity,
                    allowed_actions: request.actions.clone(),
                    mfa_level: Some(request.principal.mfa_level),
                    mfa_verified_at: request.principal.mfa_verified_at,
                    created_at: request.now,
                    expires_at: request.now.saturating_add(ttl),
                },
            ) {
                Ok(grant) => {
                    audit_gate_decision(
                        conn,
                        request,
                        Some(grant.grant_id.clone()),
                        SensitiveReadAuditStatus::Allowed,
                    );
                    DataAccessDecision::Allow {
                        grant: Some(grant),
                        reason: policy.reason,
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %error, "DataAccessGate grant creation failed closed");
                    audit_gate_decision(conn, request, None, SensitiveReadAuditStatus::Error);
                    DataAccessDecision::Deny {
                        reason: "data access gate storage error".into(),
                    }
                }
            }
        }
    }
}

fn effective_sensitivity(sensitivity: DataSensitivity, actions: &[DataAction]) -> DataSensitivity {
    let mut effective = sensitivity;
    for action in actions {
        match action {
            DataAction::AnalyzeImage | DataAction::Export => {
                effective = effective.max(DataSensitivity::S4HighlySensitive);
            }
            DataAction::ReadBody
            | DataAction::ReadAttachment
            | DataAction::SummarizeWithLlm
            | DataAction::StoreInMemory
            | DataAction::DisplayInUi
            | DataAction::NotifyUser
            | DataAction::ListMetadata => {}
        }
    }
    effective
}

fn policy_reason(sensitivity: DataSensitivity, actions: &[DataAction]) -> String {
    let mut parts = vec![sensitivity.as_str().to_string()];
    parts.extend(actions.iter().map(|action| action.as_str().to_string()));
    parts.join(" + ")
}

fn audit_gate_decision(
    conn: &Connection,
    request: &DataAccessRequest,
    grant_id: Option<String>,
    status: SensitiveReadAuditStatus,
) {
    if let Err(error) = record_sensitive_read_audit(
        conn,
        &NewSensitiveReadAuditEvent {
            user_did: request.principal.user_did.clone(),
            principal_type: request.principal.principal_type.clone(),
            principal_id: request.principal.principal_id.clone(),
            resource_type: request.resource_type.clone(),
            resource_id: request.resource_id.clone(),
            purpose: request.purpose.clone(),
            sensitivity: request.sensitivity,
            actions: request.actions.clone(),
            mfa_level: Some(request.principal.mfa_level),
            mfa_verified_at: request.principal.mfa_verified_at,
            grant_id,
            status,
            model: request.model.clone(),
            details: None,
            trace_id: request.trace_id.clone(),
            created_at: request.now,
        },
    ) {
        tracing::warn!(error = %error, status = status.as_str(), "DataAccessGate audit write failed");
    }
}

pub fn hash_identifier(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn actions_to_json(actions: &[DataAction]) -> rusqlite::Result<String> {
    serde_json::to_string(actions).map_err(|e| to_sql_error(anyhow::anyhow!(e)))
}

fn actions_from_json(value: String) -> rusqlite::Result<Vec<DataAction>> {
    serde_json::from_str(&value).map_err(|e| to_sql_error(anyhow::anyhow!(e)))
}

fn actions_cover(allowed: &[DataAction], requested: &[DataAction]) -> bool {
    requested.iter().all(|action| allowed.contains(action))
}

fn read_grant_from_row(row: &Row<'_>) -> rusqlite::Result<ReadGrant> {
    let sensitivity: String = row.get("sensitivity")?;
    let allowed_actions_json: String = row.get("allowed_actions_json")?;
    let mfa_level: Option<String> = row.get("mfa_level")?;
    Ok(ReadGrant {
        grant_id: row.get("grant_id")?,
        user_did: row.get("user_did")?,
        principal_type: row.get("principal_type")?,
        principal_id_hash: row.get("principal_id_hash")?,
        resource_type: row.get("resource_type")?,
        resource_id_hash: row.get("resource_id_hash")?,
        purpose: row.get("purpose")?,
        sensitivity: parse_sensitivity(&sensitivity)?,
        allowed_actions: actions_from_json(allowed_actions_json)?,
        mfa_level: mfa_level.as_deref().map(MfaLevel::parse),
        mfa_verified_at: row_opt_u64(row, "mfa_verified_at")?,
        created_at: row_u64(row, "created_at")?,
        expires_at: row_u64(row, "expires_at")?,
        consumed_at: row_opt_u64(row, "consumed_at")?,
        revoked_at: row_opt_u64(row, "revoked_at")?,
        schema_version: row.get("schema_version")?,
    })
}

fn sensitive_read_audit_from_row(row: &Row<'_>) -> rusqlite::Result<SensitiveReadAuditEvent> {
    let sensitivity: String = row.get("sensitivity")?;
    let actions_json: String = row.get("actions_json")?;
    let mfa_level: Option<String> = row.get("mfa_level")?;
    let status: String = row.get("status")?;
    let details_json: Option<String> = row.get("details_json")?;
    let details = details_json
        .map(|value| serde_json::from_str(&value).map_err(|e| to_sql_error(anyhow::anyhow!(e))))
        .transpose()?;
    Ok(SensitiveReadAuditEvent {
        audit_id: row.get("audit_id")?,
        user_did: row.get("user_did")?,
        principal_type: row.get("principal_type")?,
        principal_id_hash: row.get("principal_id_hash")?,
        resource_type: row.get("resource_type")?,
        resource_id_hash: row.get("resource_id_hash")?,
        purpose: row.get("purpose")?,
        sensitivity: parse_sensitivity(&sensitivity)?,
        actions: actions_from_json(actions_json)?,
        mfa_level: mfa_level.as_deref().map(MfaLevel::parse),
        mfa_verified_at: row_opt_u64(row, "mfa_verified_at")?,
        grant_id: row.get("grant_id")?,
        status: SensitiveReadAuditStatus::parse(&status),
        model: row.get("model")?,
        details,
        trace_id: row.get("trace_id")?,
        created_at: row_u64(row, "created_at")?,
        schema_version: row.get("schema_version")?,
    })
}

fn parse_sensitivity(value: &str) -> rusqlite::Result<DataSensitivity> {
    match value {
        "S0Public" => Ok(DataSensitivity::S0Public),
        "S1LowPersonal" => Ok(DataSensitivity::S1LowPersonal),
        "S2Personal" => Ok(DataSensitivity::S2Personal),
        "S3Sensitive" => Ok(DataSensitivity::S3Sensitive),
        "S4HighlySensitive" => Ok(DataSensitivity::S4HighlySensitive),
        _ => Err(to_sql_error(anyhow::anyhow!(
            "unknown data sensitivity: {value}"
        ))),
    }
}

fn reject_sensitive_audit_details(details: &serde_json::Value) -> rusqlite::Result<()> {
    const FORBIDDEN_KEYS: &[&str] = &[
        "body",
        "content",
        "raw_content",
        "attachment_content",
        "image_description",
        "oauth_token",
        "access_token",
        "refresh_token",
        "secret",
    ];

    fn visit(value: &serde_json::Value, forbidden: &[&str]) -> bool {
        match value {
            serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
                let lower = key.to_ascii_lowercase();
                forbidden.contains(&lower.as_str()) || visit(value, forbidden)
            }),
            serde_json::Value::Array(values) => values.iter().any(|value| visit(value, forbidden)),
            _ => false,
        }
    }

    if visit(details, FORBIDDEN_KEYS) {
        return Err(to_sql_error(anyhow::anyhow!(
            "sensitive read audit details contain forbidden raw-content keys"
        )));
    }
    Ok(())
}

fn u64_to_i64(value: u64) -> rusqlite::Result<i64> {
    i64::try_from(value).map_err(|_| to_sql_error(anyhow::anyhow!("timestamp out of range")))
}

fn opt_u64_to_i64(value: Option<u64>) -> rusqlite::Result<Option<i64>> {
    value.map(u64_to_i64).transpose()
}

fn row_u64(row: &Row<'_>, column: &str) -> rusqlite::Result<u64> {
    let value: i64 = row.get(column)?;
    u64::try_from(value).map_err(|_| to_sql_error(anyhow::anyhow!("negative timestamp in DB")))
}

fn row_opt_u64(row: &Row<'_>, column: &str) -> rusqlite::Result<Option<u64>> {
    let value: Option<i64> = row.get(column)?;
    value
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| to_sql_error(anyhow::anyhow!("negative timestamp in DB")))
        })
        .transpose()
}

fn to_sql_error(error: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(error.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_principal(mfa_level: MfaLevel, mfa_verified_at: Option<u64>) -> AuthPrincipal {
        AuthPrincipal {
            user_did: "did:aegis:telegram:123".into(),
            principal_type: "web_console".into(),
            principal_id: "owner@example.com".into(),
            mfa_level,
            mfa_verified_at,
        }
    }

    fn test_access_request(mfa_level: MfaLevel, mfa_verified_at: Option<u64>) -> DataAccessRequest {
        DataAccessRequest {
            principal: test_principal(mfa_level, mfa_verified_at),
            resource_type: "calendar_event".into(),
            resource_id: "msg_123".into(),
            sensitivity: DataSensitivity::S3Sensitive,
            actions: vec![DataAction::ReadBody, DataAction::SummarizeWithLlm],
            purpose: "summarize_this_email".into(),
            now: 1_000,
            grant_ttl_secs: 300,
            mfa_freshness_secs: None,
            model: Some("x-ai/grok-4.1-fast".into()),
            trace_id: Some("trace-test".into()),
        }
    }

    #[test]
    fn s0_public_read_requires_no_auth_or_mfa() {
        let policy = policy_for_request(DataSensitivity::S0Public, &[DataAction::DisplayInUi]);
        assert!(!policy.requires_authenticated_session);
        assert_eq!(policy.required_mfa, RequiredMfa::None);
        assert_eq!(
            evaluate_mfa(&policy, MfaLevel::None, None, 1_000),
            DataAccessPolicyDecision::Satisfied
        );
    }

    #[test]
    fn s2_metadata_requires_session_but_no_mfa_by_default() {
        let policy = policy_for_request(DataSensitivity::S2Personal, &[DataAction::ListMetadata]);
        assert!(policy.requires_authenticated_session);
        assert_eq!(policy.required_mfa, RequiredMfa::None);
    }

    #[test]
    fn s3_read_body_requires_recent_totp_or_webauthn() {
        let policy = policy_for_request(DataSensitivity::S3Sensitive, &[DataAction::ReadBody]);
        assert_eq!(policy.required_mfa, RequiredMfa::TotpOrWebAuthn);
        assert_eq!(
            evaluate_mfa(&policy, MfaLevel::GoogleOnly, None, 1_000),
            DataAccessPolicyDecision::RequireMfa {
                required_level: RequiredMfa::TotpOrWebAuthn,
                reason: "S3Sensitive + ReadBody".into(),
                freshness_secs: DEFAULT_MFA_FRESHNESS_SECS,
            }
        );
        assert_eq!(
            evaluate_mfa(&policy, MfaLevel::GoogleFresh, Some(950), 1_000),
            DataAccessPolicyDecision::Satisfied
        );
        assert_eq!(
            evaluate_mfa(&policy, MfaLevel::Totp, Some(950), 1_000),
            DataAccessPolicyDecision::Satisfied
        );
        assert_eq!(
            evaluate_mfa(&policy, MfaLevel::WebAuthn, Some(950), 1_000),
            DataAccessPolicyDecision::Satisfied
        );
        assert_eq!(
            evaluate_mfa(&policy, MfaLevel::Host, Some(950), 1_000),
            DataAccessPolicyDecision::Satisfied
        );
    }

    #[test]
    fn stale_mfa_fails_sensitive_read_policy() {
        let policy = policy_for_request(DataSensitivity::S3Sensitive, &[DataAction::ReadBody]);
        assert!(matches!(
            evaluate_mfa(&policy, MfaLevel::WebAuthn, Some(300), 1_000),
            DataAccessPolicyDecision::RequireMfa { .. }
        ));
    }

    #[test]
    fn s4_image_analysis_requires_webauthn_and_rejects_totp() {
        let policy = policy_for_request(
            DataSensitivity::S4HighlySensitive,
            &[DataAction::AnalyzeImage],
        );
        assert_eq!(policy.required_mfa, RequiredMfa::WebAuthn);
        assert!(matches!(
            evaluate_mfa(&policy, MfaLevel::Totp, Some(950), 1_000),
            DataAccessPolicyDecision::RequireMfa { .. }
        ));
        assert!(matches!(
            evaluate_mfa(&policy, MfaLevel::GoogleFresh, Some(950), 1_000),
            DataAccessPolicyDecision::RequireMfa { .. }
        ));
        assert_eq!(
            evaluate_mfa(&policy, MfaLevel::WebAuthn, Some(950), 1_000),
            DataAccessPolicyDecision::Satisfied
        );
    }

    #[test]
    fn analyze_image_escalates_lower_sensitivity_to_s4() {
        let policy = policy_for_request(DataSensitivity::S2Personal, &[DataAction::AnalyzeImage]);
        assert_eq!(policy.required_mfa, RequiredMfa::WebAuthn);
        assert!(policy.reason.contains("S4HighlySensitive"));
    }

    #[test]
    fn recovery_code_does_not_satisfy_sensitive_read_mfa() {
        let policy =
            policy_for_request(DataSensitivity::S3Sensitive, &[DataAction::ReadAttachment]);
        assert!(matches!(
            evaluate_mfa(&policy, MfaLevel::RecoveryCode, Some(990), 1_000),
            DataAccessPolicyDecision::RequireMfa { .. }
        ));
    }

    #[test]
    fn read_grant_is_resource_bound_and_hashed() {
        let conn = Connection::open_in_memory().unwrap();
        init_sensitive_read_schema(&conn).unwrap();
        let grant = create_read_grant(
            &conn,
            &NewReadGrant {
                user_did: "did:aegis:telegram:123".into(),
                principal_type: "web_console".into(),
                principal_id: "owner@example.com".into(),
                resource_type: "calendar_event".into(),
                resource_id: "msg_123".into(),
                purpose: "summarize_this_email".into(),
                sensitivity: DataSensitivity::S3Sensitive,
                allowed_actions: vec![DataAction::ReadBody, DataAction::SummarizeWithLlm],
                mfa_level: Some(MfaLevel::WebAuthn),
                mfa_verified_at: Some(1_000),
                created_at: 1_000,
                expires_at: 1_600,
            },
        )
        .unwrap();

        assert_ne!(grant.principal_id_hash, "owner@example.com");
        assert_ne!(grant.resource_id_hash, "msg_123");

        let matching = find_valid_read_grant(
            &conn,
            &ReadGrantLookup {
                user_did: "did:aegis:telegram:123".into(),
                principal_type: "web_console".into(),
                principal_id: "owner@example.com".into(),
                resource_type: "calendar_event".into(),
                resource_id: "msg_123".into(),
                purpose: "summarize_this_email".into(),
                requested_actions: vec![DataAction::ReadBody],
                now: 1_100,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(matching.grant_id, grant.grant_id);

        let other_resource = find_valid_read_grant(
            &conn,
            &ReadGrantLookup {
                resource_id: "msg_456".into(),
                requested_actions: vec![DataAction::ReadBody],
                now: 1_100,
                user_did: "did:aegis:telegram:123".into(),
                principal_type: "web_console".into(),
                principal_id: "owner@example.com".into(),
                resource_type: "calendar_event".into(),
                purpose: "summarize_this_email".into(),
            },
        )
        .unwrap();
        assert!(other_resource.is_none());
    }

    #[test]
    fn expired_or_too_narrow_grant_is_not_valid() {
        let conn = Connection::open_in_memory().unwrap();
        init_sensitive_read_schema(&conn).unwrap();
        create_read_grant(
            &conn,
            &NewReadGrant {
                user_did: "did:aegis:telegram:123".into(),
                principal_type: "web_console".into(),
                principal_id: "owner@example.com".into(),
                resource_type: "calendar_event".into(),
                resource_id: "msg_123".into(),
                purpose: "summarize_this_email".into(),
                sensitivity: DataSensitivity::S3Sensitive,
                allowed_actions: vec![DataAction::ReadBody],
                mfa_level: Some(MfaLevel::WebAuthn),
                mfa_verified_at: Some(1_000),
                created_at: 1_000,
                expires_at: 1_100,
            },
        )
        .unwrap();

        let expired = find_valid_read_grant(
            &conn,
            &ReadGrantLookup {
                user_did: "did:aegis:telegram:123".into(),
                principal_type: "web_console".into(),
                principal_id: "owner@example.com".into(),
                resource_type: "calendar_event".into(),
                resource_id: "msg_123".into(),
                purpose: "summarize_this_email".into(),
                requested_actions: vec![DataAction::ReadBody],
                now: 1_101,
            },
        )
        .unwrap();
        assert!(expired.is_none());

        let too_narrow = find_valid_read_grant(
            &conn,
            &ReadGrantLookup {
                user_did: "did:aegis:telegram:123".into(),
                principal_type: "web_console".into(),
                principal_id: "owner@example.com".into(),
                resource_type: "calendar_event".into(),
                resource_id: "msg_123".into(),
                purpose: "summarize_this_email".into(),
                requested_actions: vec![DataAction::ReadBody, DataAction::SummarizeWithLlm],
                now: 1_050,
            },
        )
        .unwrap();
        assert!(too_narrow.is_none());
    }

    #[test]
    fn sensitive_read_audit_omits_raw_ids_and_rejects_content_details() {
        let conn = Connection::open_in_memory().unwrap();
        init_sensitive_read_schema(&conn).unwrap();
        let event = record_sensitive_read_audit(
            &conn,
            &NewSensitiveReadAuditEvent {
                user_did: "did:aegis:telegram:123".into(),
                principal_type: "web_console".into(),
                principal_id: "owner@example.com".into(),
                resource_type: "calendar_event".into(),
                resource_id: "msg_123".into(),
                purpose: "summarize_this_email".into(),
                sensitivity: DataSensitivity::S3Sensitive,
                actions: vec![DataAction::ReadBody],
                mfa_level: Some(MfaLevel::WebAuthn),
                mfa_verified_at: Some(1_000),
                grant_id: Some("rg_test".into()),
                status: SensitiveReadAuditStatus::Allowed,
                model: Some("x-ai/grok-4.1-fast".into()),
                details: Some(serde_json::json!({"reason": "user_requested_summary"})),
                trace_id: Some("trace-test".into()),
                created_at: 1_200,
            },
        )
        .unwrap();

        assert_ne!(event.principal_id_hash, "owner@example.com");
        assert_ne!(event.resource_id_hash, "msg_123");
        let loaded = latest_sensitive_read_audit(&conn, "did:aegis:telegram:123")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.audit_id, event.audit_id);
        assert_eq!(
            loaded.details,
            Some(serde_json::json!({"reason": "user_requested_summary"}))
        );

        let rejected = record_sensitive_read_audit(
            &conn,
            &NewSensitiveReadAuditEvent {
                details: Some(serde_json::json!({"body": "raw private email body"})),
                created_at: 1_201,
                user_did: "did:aegis:telegram:123".into(),
                principal_type: "web_console".into(),
                principal_id: "owner@example.com".into(),
                resource_type: "calendar_event".into(),
                resource_id: "msg_123".into(),
                purpose: "summarize_this_email".into(),
                sensitivity: DataSensitivity::S3Sensitive,
                actions: vec![DataAction::ReadBody],
                mfa_level: Some(MfaLevel::WebAuthn),
                mfa_verified_at: Some(1_000),
                grant_id: None,
                status: SensitiveReadAuditStatus::Allowed,
                model: None,
                trace_id: None,
            },
        );
        assert!(rejected.is_err());
    }

    #[test]
    fn check_or_request_access_requires_mfa_before_grant() {
        let conn = Connection::open_in_memory().unwrap();
        init_sensitive_read_schema(&conn).unwrap();
        let request = test_access_request(MfaLevel::GoogleOnly, None);

        let decision = check_or_request_access(&conn, &request);
        assert!(matches!(
            decision,
            DataAccessDecision::RequireMfa {
                required_level: RequiredMfa::TotpOrWebAuthn,
                ..
            }
        ));
        let audit = latest_sensitive_read_audit(&conn, "did:aegis:telegram:123")
            .unwrap()
            .unwrap();
        assert_eq!(audit.status, SensitiveReadAuditStatus::RequireMfa);
        assert!(audit.grant_id.is_none());
    }

    #[test]
    fn check_or_request_access_creates_and_reuses_resource_grant() {
        let conn = Connection::open_in_memory().unwrap();
        init_sensitive_read_schema(&conn).unwrap();
        let request = test_access_request(MfaLevel::WebAuthn, Some(950));

        let first = check_or_request_access(&conn, &request);
        let first_grant_id = match first {
            DataAccessDecision::Allow {
                grant: Some(grant), ..
            } => {
                assert_eq!(grant.expires_at, 1_300);
                grant.grant_id
            }
            other => panic!("expected allow with grant, got {other:?}"),
        };

        let second = check_or_request_access(&conn, &request);
        let second_grant_id = match second {
            DataAccessDecision::Allow {
                grant: Some(grant),
                reason,
            } => {
                assert_eq!(reason, "existing grant");
                grant.grant_id
            }
            other => panic!("expected existing grant, got {other:?}"),
        };
        assert_eq!(second_grant_id, first_grant_id);

        let audit = latest_sensitive_read_audit(&conn, "did:aegis:telegram:123")
            .unwrap()
            .unwrap();
        assert_eq!(audit.status, SensitiveReadAuditStatus::Allowed);
        assert_eq!(audit.grant_id, Some(first_grant_id));
    }

    #[test]
    fn check_or_request_access_fails_closed_on_db_error() {
        let conn = Connection::open_in_memory().unwrap();
        let request = test_access_request(MfaLevel::WebAuthn, Some(950));

        let decision = check_or_request_access(&conn, &request);
        assert_eq!(
            decision,
            DataAccessDecision::Deny {
                reason: "data access gate storage error".into()
            }
        );
    }
}
