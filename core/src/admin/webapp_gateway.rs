use super::*;

// ── WebApp SDUI API ─────────────────────────────────────────────────────────
//
// These endpoints serve the Telegram Mini App (WebApp). They are on the public
// router because Telegram embeds the WebApp in an iframe from their servers.
// Authentication is done via Telegram initData HMAC verification.

/// Extract and verify Telegram WebApp initData from the Authorization header.
/// Expects: `Authorization: TelegramWebApp <initData>`
pub(super) fn extract_webapp_auth(
    state: &AdminState,
    headers: &axum::http::HeaderMap,
) -> Result<webapp_auth::VerifiedInitData, (StatusCode, String)> {
    let bot_token = state.bot_token.as_deref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "WebApp not configured (no bot token)".into(),
        )
    })?;

    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "Missing Authorization header".into(),
            )
        })?;

    let init_data = auth_header.strip_prefix("TelegramWebApp ").ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            "Expected 'TelegramWebApp <initData>' header".into(),
        )
    })?;

    let verified = webapp_auth::verify_init_data(init_data, bot_token).map_err(|e| {
        tracing::warn!(error = %e, "WebApp auth failed");
        (StatusCode::UNAUTHORIZED, "Authentication failed".into())
    })?;

    // Verify admin access — fail-closed: if admin_chat_id is not configured,
    // reject all requests (prevents unintended open access).
    let admin_id = state.admin_chat_id.ok_or_else(|| {
        tracing::error!("WebApp request but ADMIN_CHAT_ID not configured — rejecting");
        (StatusCode::FORBIDDEN, "WebApp access not configured".into())
    })?;
    webapp_auth::verify_admin_access(verified.user.id, admin_id).map_err(|_e| {
        tracing::warn!(user_id = verified.user.id, admin_id, "WebApp access denied");
        (StatusCode::FORBIDDEN, "Access denied".into())
    })?;

    Ok(verified)
}

pub(super) enum WebConsolePrincipal {
    Telegram(webapp_auth::VerifiedInitData),
    WebConsole(web_console_auth::WebConsoleSession),
    #[cfg(test)]
    DebugLease(WebappDebugLeasePrincipal),
}

impl WebConsolePrincipal {
    pub(super) fn log_subject(&self) -> String {
        match self {
            Self::Telegram(verified) => format!("telegram:{}", verified.user.id),
            Self::WebConsole(session) => format!("web_console:{}", session.email),
            #[cfg(test)]
            Self::DebugLease(lease) => format!("debug_lease:{}", lease.id),
        }
    }

    pub(super) fn data_access_principal(
        &self,
        user_did: String,
    ) -> crate::data_access_gate::AuthPrincipal {
        match self {
            Self::Telegram(verified) => crate::data_access_gate::AuthPrincipal {
                user_did,
                principal_type: "telegram_webapp".into(),
                principal_id: verified.user.id.to_string(),
                mfa_level: web_console_auth::MfaLevel::None,
                mfa_verified_at: None,
            },
            Self::WebConsole(session) => crate::data_access_gate::AuthPrincipal {
                user_did,
                principal_type: "web_console".into(),
                principal_id: session.email.clone(),
                mfa_level: session.mfa_level,
                mfa_verified_at: session.mfa_verified_at,
            },
            #[cfg(test)]
            Self::DebugLease(lease) => crate::data_access_gate::AuthPrincipal {
                user_did,
                principal_type: "debug_lease".into(),
                principal_id: lease.id.clone(),
                mfa_level: web_console_auth::MfaLevel::None,
                mfa_verified_at: None,
            },
        }
    }
}

pub(super) async fn extract_web_console_auth(
    state: &AdminState,
    headers: &axum::http::HeaderMap,
) -> Result<WebConsolePrincipal, (StatusCode, String)> {
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("TelegramWebApp "))
    {
        return extract_webapp_auth(state, headers).map(WebConsolePrincipal::Telegram);
    }

    web_console_session_from_request(state, headers)
        .await
        .map(WebConsolePrincipal::WebConsole)
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "Authentication required".to_string(),
            )
        })
}

/// Owner chat id for a principal that is the actual mailbox OWNER (a verified
/// Telegram user or a Web Console session), or `None` for a scoped
/// `DebugLease` inspect grant.
///
/// Used to fail CLOSED on background work that reads the owner's profile or
/// writes to the store. A read-only debug-lease inbox inspection must stay
/// read-only: it must not cause owner-profile data to be sent to the LLM, nor
/// trigger a store writeback. Distinct from `console_chat_id_for_principal`,
/// which intentionally grants a chat context to debug leases for read APIs.
#[cfg(test)]
pub(super) fn owner_chat_id_for_principal(
    state: &AdminState,
    principal: &WebConsolePrincipal,
) -> Option<i64> {
    match principal {
        WebConsolePrincipal::Telegram(verified) => {
            Some(state.admin_chat_id.unwrap_or(verified.user.id))
        }
        // A Web Console owner session is an owner, but only if a chat context is
        // configured (mirrors `console_chat_id_for_principal`'s fail-closed arm).
        WebConsolePrincipal::WebConsole(_) => state.admin_chat_id,
        #[cfg(test)]
        WebConsolePrincipal::DebugLease(_) => None,
    }
}

pub(super) fn console_chat_id_for_principal(
    state: &AdminState,
    principal: &WebConsolePrincipal,
) -> Result<i64, (StatusCode, String)> {
    match principal {
        WebConsolePrincipal::Telegram(verified) => {
            Ok(state.admin_chat_id.unwrap_or(verified.user.id))
        }
        WebConsolePrincipal::WebConsole(session) => {
            tracing::debug!(
                email = %session.email,
                "Web Console session authorized for console API"
            );
            state.admin_chat_id.ok_or_else(|| {
                (
                    StatusCode::FORBIDDEN,
                    "Workbench chat context is not configured".to_string(),
                )
            })
        }
        #[cfg(test)]
        WebConsolePrincipal::DebugLease(lease) => {
            tracing::debug!(
                lease_id = %lease.id,
                owner = %lease.owner,
                "Debug lease authorized for scoped console API"
            );
            state.admin_chat_id.ok_or_else(|| {
                (
                    StatusCode::FORBIDDEN,
                    "Workbench chat context is not configured".to_string(),
                )
            })
        }
    }
}

pub(super) fn webapp_canonical_user_did(chat_id: i64) -> String {
    webapp_canonical_user_did_from_env(
        chat_id,
        std::env::var("OWNER_DID").ok().as_deref(),
        std::env::var("ADMIN_CHAT_ID").ok().as_deref(),
    )
}

pub(super) fn webapp_canonical_user_did_from_env(
    chat_id: i64,
    owner_did: Option<&str>,
    admin_chat_id: Option<&str>,
) -> String {
    owner_did
        .map(str::trim)
        .filter(|did| !did.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            admin_chat_id
                .and_then(|raw| raw.parse::<i64>().ok())
                .filter(|admin_chat_id| *admin_chat_id != 0)
                .map(|admin_chat_id| format!("did:aegis:telegram:{admin_chat_id}"))
        })
        .unwrap_or_else(|| format!("did:aegis:telegram:{chat_id}"))
}

pub(super) fn require_web_console_csrf_for_principal(
    headers: &axum::http::HeaderMap,
    principal: &WebConsolePrincipal,
) -> Result<(), (StatusCode, String)> {
    if let WebConsolePrincipal::WebConsole(session) = principal {
        require_web_console_csrf(headers, session)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WebAppRateLimitExceeded {
    pub(super) retry_after_secs: u64,
}

#[derive(Default)]
pub(super) struct WebAppRateLimiter {
    hits: parking_lot::Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl WebAppRateLimiter {
    pub(super) fn check(
        &self,
        key: &str,
        limit: usize,
        window: Duration,
        now: Instant,
    ) -> Option<WebAppRateLimitExceeded> {
        if limit == 0 {
            return None;
        }
        let cutoff = now.checked_sub(window).unwrap_or(now);
        let mut hits = self.hits.lock();
        let bucket = hits.entry(key.to_string()).or_default();
        while bucket.front().is_some_and(|hit| *hit <= cutoff) {
            bucket.pop_front();
        }
        if bucket.len() >= limit {
            let oldest = bucket.front().copied().unwrap_or(now);
            let retry_after_secs = window
                .saturating_sub(now.saturating_duration_since(oldest))
                .as_secs()
                .max(1);
            return Some(WebAppRateLimitExceeded { retry_after_secs });
        }
        bucket.push_back(now);
        None
    }
}

fn webapp_rate_limiter() -> &'static WebAppRateLimiter {
    static LIMITER: OnceLock<WebAppRateLimiter> = OnceLock::new();
    LIMITER.get_or_init(WebAppRateLimiter::default)
}

pub(super) fn check_webapp_rate_limit(
    principal: &WebConsolePrincipal,
    endpoint: &str,
    callback_id: &str,
    limit: usize,
) -> Result<(), (StatusCode, String)> {
    let key = format!("{}:{endpoint}:{callback_id}", principal.log_subject());
    if let Some(exceeded) =
        webapp_rate_limiter().check(&key, limit, WEBAPP_RATE_LIMIT_WINDOW, Instant::now())
    {
        tracing::warn!(
            principal = %principal.log_subject(),
            endpoint,
            callback_id,
            retry_after_secs = exceeded.retry_after_secs,
            "WebApp API rate limit exceeded"
        );
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "Rate limited; retry after {} seconds",
                exceeded.retry_after_secs
            ),
        ));
    }
    Ok(())
}

// ── Console API (Phase 1b — Main Mini App) ──────────────────────────────────

/// Query parameters for SSE stream.
#[derive(serde::Deserialize)]
pub(super) struct StreamQuery {
    pub(super) stream_token: Option<String>,
}

#[derive(Clone)]
pub(super) struct WebAppStreamTokenGrant {
    callback_id: String,
    pub(super) chat_id: i64,
    pub(super) expires_at: u64,
}

pub(super) fn webapp_stream_token_store()
-> &'static parking_lot::Mutex<HashMap<String, WebAppStreamTokenGrant>> {
    static STORE: OnceLock<parking_lot::Mutex<HashMap<String, WebAppStreamTokenGrant>>> =
        OnceLock::new();
    STORE.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

pub(super) fn webapp_stream_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn mint_webapp_stream_token(callback_id: String, chat_id: i64) -> (String, u64) {
    let token = hex::encode(rand::random::<[u8; 32]>());
    let now = webapp_stream_now_secs();
    let expires_at = now + WEBAPP_STREAM_TOKEN_TTL_SECS;
    let mut store = webapp_stream_token_store().lock();
    store.retain(|_, grant| grant.expires_at >= now);
    store.insert(
        token.clone(),
        WebAppStreamTokenGrant {
            callback_id,
            chat_id,
            expires_at,
        },
    );
    (token, expires_at)
}

pub(super) fn validate_webapp_stream_token(
    token: &str,
    callback_id: &str,
) -> Result<WebAppStreamTokenGrant, (StatusCode, String)> {
    let now = webapp_stream_now_secs();
    let mut store = webapp_stream_token_store().lock();
    store.retain(|_, grant| grant.expires_at >= now);
    let grant = store
        .get(token)
        .cloned()
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Invalid stream token".to_string()))?;
    if grant.expires_at < now {
        return Err((StatusCode::UNAUTHORIZED, "Expired stream token".to_string()));
    }
    if grant.callback_id != callback_id {
        return Err((
            StatusCode::FORBIDDEN,
            "Stream token does not match task".to_string(),
        ));
    }
    Ok(grant)
}

#[derive(Serialize)]
pub(super) struct UnifiedInboxProjection {
    pub(super) schema_version: u32,
    pub(super) generated_at: String,
    pub(super) owner: String,
    pub(super) sources: Vec<UnifiedInboxSourceStatus>,
    pub(super) counts: UnifiedInboxCounts,
    pub(super) items: Vec<UnifiedInboxItem>,
}

#[derive(Clone, Serialize)]
pub(super) struct UnifiedInboxSourceStatus {
    pub(super) source: String,
    pub(super) status: String,
    pub(super) reason: Option<String>,
}

#[derive(Clone, Default, Serialize)]
pub(super) struct UnifiedInboxCounts {
    pub(super) total: usize,
    pub(super) urgent: usize,
    pub(super) high: usize,
    pub(super) normal: usize,
    pub(super) low: usize,
    pub(super) silent: usize,
    pub(super) approvals: usize,
    pub(super) email: usize,
    pub(super) calendar: usize,
    pub(super) tasks: usize,
    pub(super) receipts: usize,
    pub(super) alerts: usize,
    pub(super) memory_reviews: usize,
    pub(super) sensitive: usize,
}

#[derive(Clone, Serialize)]
pub(super) struct UnifiedInboxAction {
    pub(super) label: String,
    /// Stable structural identifier for the action label so the webapp can
    /// localize it (i18n catalog key `work.actionLabel.<code>`). `label` stays
    /// the English prose fallback (and the Telegram surface). Serialized only
    /// when set; absent/unknown codes fall back to `label` on the FE.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) label_code: Option<String>,
    pub(super) action_kind: String,
    pub(super) route: Option<String>,
    pub(super) requires_gate: bool,
    pub(super) requires_mfa: bool,
}

/// One hop in a work-card source chain. Smaller hops come first
/// (`occurrence`, then `intent`, then `origin`) so the owner can follow
/// the card upward. `detail` is the owner-facing evidence for that hop
/// (who, when, what was said, and the task) — not stored field names.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(super) struct WorkLineageNode {
    pub(super) layer: String,
    pub(super) kind: String,
    pub(super) id: String,
    pub(super) label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) label_code: Option<String>,
    pub(super) detail: String,
    /// Receipt actions that belong on this hop. Empty for evidence-only hops.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) action_ids: Vec<String>,
    /// Webapp turn id for a 「对话消息」 hop. The console turns this plus
    /// `message_role` into the stable `#` cite. Absent for records that are
    /// not one archived webapp turn (including private tasks, which never
    /// receive a transcript hop).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) turn_id: Option<String>,
    /// `owner` or `aegis`, matching the chat cite. Set only together with
    /// `turn_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) message_role: Option<String>,
}

#[derive(Clone, Serialize)]
pub(super) struct UnifiedInboxItem {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) title: String,
    /// Structural code for `title` (i18n catalog key `work.decide.<code>`).
    /// Set only where the title is static prose; `None` when the title is
    /// derived from user/task content. Serialized only when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) title_code: Option<String>,
    pub(super) summary: String,
    pub(super) owner_visible_reason: String,
    /// Stable structural identifier for `owner_visible_reason` (i18n catalog
    /// key `work.reasonCode.<code>`). One code per construction-site MEANING;
    /// `None` for branches whose prose interpolates dynamic values (the prose
    /// stays the fallback). Serialized only when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) owner_visible_reason_code: Option<String>,
    pub(super) authority_basis: String,
    pub(super) source: String,
    pub(super) source_id: String,
    pub(super) source_event_id: Option<String>,
    pub(super) status: String,
    pub(super) urgency: String,
    pub(super) sensitivity: String,
    pub(super) privacy_level: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) due_at: Option<String>,
    pub(super) primary_action: Option<UnifiedInboxAction>,
    pub(super) secondary_actions: Vec<UnifiedInboxAction>,
    pub(super) next_safe_action: String,
    /// Stable structural identifier for `next_safe_action` (i18n catalog key
    /// `work.nextCode.<code>`). Same contract as `owner_visible_reason_code`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) next_safe_action_code: Option<String>,
    pub(super) blocked_by: Option<String>,
    /// Structural code for `blocked_by` (i18n catalog key `work.reasonCode.<code>`).
    /// Same contract as `owner_visible_reason_code`. Serialized only when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) blocked_by_code: Option<String>,
    pub(super) receipt_expectation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) authority: Option<ContextAuthorityEnvelope>,
    pub(super) detail_route: Option<String>,
    pub(super) external_url_policy: String,
    pub(super) related_action_ids: Vec<String>,
    pub(super) related_receipt_ids: Vec<String>,
    pub(super) route_reason: String,
    pub(super) schema_version: u32,
    /// Source chain for tracking cards. Empty for items that are not a
    /// follow-up, and omitted from the wire when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) lineage: Vec<WorkLineageNode>,
}

#[derive(Serialize)]
pub(super) struct VNextHomeProjection {
    pub(super) schema_version: u32,
    pub(super) generated_at: String,
    pub(super) brief: String,
    pub(super) top_now: Vec<VNextWorkItem>,
    pub(super) decision_previews: Vec<VNextWorkItem>,
    pub(super) follow_up_highlights: Vec<VNextWorkItem>,
    pub(super) status_entry: VNextStatusSummary,
}

#[derive(Serialize)]
pub(super) struct VNextWorkProjection {
    pub(super) schema_version: u32,
    pub(super) generated_at: String,
    pub(super) sources: Vec<UnifiedInboxSourceStatus>,
    pub(super) counts: UnifiedInboxCounts,
    pub(super) lanes: Vec<VNextWorkLane>,
}

#[derive(Serialize)]
pub(super) struct VNextWorkLane {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) collapsed: bool,
    pub(super) items: Vec<VNextWorkItem>,
}

#[derive(Clone, Serialize)]
pub(super) struct VNextWorkItem {
    pub(super) id: String,
    pub(super) lane: String,
    pub(super) source_kind: String,
    pub(super) source: String,
    pub(super) source_id: String,
    pub(super) source_event_id: Option<String>,
    pub(super) title: String,
    /// Structural code mirroring `UnifiedInboxItem::title_code`, forwarded only
    /// when `owner_visible_title` left the title untouched (truncation or the
    /// empty-title fallback would make the code describe a different string).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) title_code: Option<String>,
    pub(super) summary: String,
    pub(super) why_it_matters: String,
    /// Structural code mirroring `UnifiedInboxItem::owner_visible_reason_code`
    /// (why_it_matters is that field projected). Serialized only when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) why_it_matters_code: Option<String>,
    pub(super) state: String,
    pub(super) urgency: String,
    pub(super) sensitivity: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) due_at: Option<String>,
    pub(super) authority: VNextAuthorityEnvelope,
    pub(super) recommended_action: Option<VNextActionRef>,
    pub(super) contextual_actions: Vec<VNextActionRef>,
    pub(super) next_safe_action: String,
    /// Structural code mirroring `UnifiedInboxItem::next_safe_action_code`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) next_safe_action_code: Option<String>,
    pub(super) blocked_by: Option<String>,
    /// Structural code mirroring `UnifiedInboxItem::blocked_by_code`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) blocked_by_code: Option<String>,
    pub(super) detail_route: Option<String>,
    pub(super) external_url_policy: String,
    pub(super) route_reason: String,
    pub(super) related_work_item_ids: Vec<String>,
    pub(super) related_receipt_ids: Vec<String>,
    pub(super) receipt_expectation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) decision_request: Option<VNextDecisionRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) lineage: Vec<WorkLineageNode>,
}

#[derive(Clone, Serialize)]
pub(super) struct VNextActionRef {
    pub(super) id: String,
    pub(super) label: String,
    /// Structural code mirroring `UnifiedInboxAction::label_code`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) label_code: Option<String>,
    pub(super) kind: String,
    pub(super) route: Option<String>,
    pub(super) requires_confirmation: bool,
    pub(super) requires_mfa: bool,
    pub(super) requires_data_gate: bool,
    pub(super) expected_receipt: String,
}

/// Owner decision card projected for the Work view.
///
/// A code may be written `"<namespace>:<code>"` to resolve under
/// `work.<namespace>.<code>` instead of the field's default `work.decide`
/// namespace. Allowed namespaces: `decide`, `reasonCode`, `nextCode`,
/// `actionLabel` (kept in lockstep with `resolveWorkCodeText` in
/// `webapp/src/vnext/model.ts`). Anything else (or an unknown code) falls back
/// to the server string.
#[derive(Clone, Serialize)]
pub(super) struct VNextDecisionRequest {
    pub(super) id: String,
    pub(super) title: String,
    /// Structural code for `title` (i18n catalog key `work.decide.<code>`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(super) title_code: Option<String>,
    pub(super) what_aegis_was_doing: String,
    /// Structural code for `what_aegis_was_doing`. Absent when freeform summary.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(super) what_code: Option<String>,
    pub(super) blocked_reason: String,
    /// Structural code for `blocked_reason` (i18n catalog key
    /// `work.reasonCode.<code>`). Chosen at the same site that chooses
    /// `blocked_reason`, so the code and the string can never drift.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(super) blocked_reason_code: Option<String>,
    pub(super) requested_capability: String,
    /// Structural code for `requested_capability`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(super) capability_code: Option<String>,
    pub(super) scope: String,
    /// Structural code for `scope`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(super) scope_code: Option<String>,
    pub(super) choices: Vec<VNextDecisionChoice>,
    pub(super) safe_details: Vec<String>,
    /// Aligned 1:1 with `safe_details`; `""` marks an uncoded (freeform) entry.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(super) safe_detail_codes: Option<Vec<String>>,
    pub(super) audit_refs: Vec<String>,
    pub(super) receipt_expectation: String,
}

#[derive(Clone, Serialize)]
pub(super) struct VNextDecisionChoice {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) consequence: String,
    pub(super) risk_level: String,
    pub(super) applies_to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) action_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) assistant_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) detail_route: Option<String>,
    pub(super) requires_confirmation: bool,
    pub(super) enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) disabled_reason: Option<String>,
    pub(super) tone: String,
}

#[derive(Clone, Serialize)]
pub(super) struct VNextAuthorityEnvelope {
    pub(super) basis: String,
    pub(super) owner_visible_reason: String,
    pub(super) visible_data: Vec<String>,
    pub(super) locked_data: Vec<String>,
    pub(super) allowed_now: Vec<String>,
    pub(super) requires_mfa: bool,
    pub(super) requires_approval: bool,
    pub(super) requires_data_grant: bool,
    pub(super) expected_receipt: Option<String>,
    pub(super) source_mandate_id: Option<String>,
    pub(super) audit_refs: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct VNextItemProjection {
    pub(super) schema_version: u32,
    pub(super) generated_at: String,
    pub(super) item: VNextWorkItem,
    pub(super) safe_source_detail: String,
    pub(super) history: Vec<VNextReceiptRef>,
    pub(super) collaboration: VNextCollaborationEntry,
}

#[derive(Serialize)]
pub(super) struct VNextReceiptRef {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) label: String,
    pub(super) created_at: String,
}

#[derive(Serialize)]
pub(super) struct VNextCollaborationEntry {
    pub(super) focus_id: String,
    pub(super) placeholder: String,
    pub(super) suggested_prompts: Vec<String>,
}

#[derive(Clone, Serialize)]
pub(super) struct VNextStatusSummary {
    pub(super) label: String,
    pub(super) health: String,
    pub(super) owner_message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) owner_next_step: Option<String>,
}

#[derive(Clone, Serialize)]
pub(super) struct VNextSystemEvent {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) severity: String,
    pub(super) title: String,
    pub(super) summary: String,
    pub(super) occurred_at: i64,
    pub(super) window_start_at: i64,
    pub(super) count: usize,
    pub(super) details: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct VNextStatusProjection {
    pub(super) schema_version: u32,
    pub(super) generated_at: String,
    pub(super) sections: Vec<VNextStatusSummary>,
    pub(super) system_events: Vec<VNextSystemEvent>,
    pub(super) advanced_diagnostics_available: bool,
    /// Ephemeral signal that ALL configured LLM providers/fallbacks are
    /// currently exhausted for foreground chat. `None` when healthy. The webapp
    /// app-shell polls this via `fetchVNextStatus` and surfaces a degraded
    /// banner pointing the owner at the provider switcher. Serialized as `null`
    /// when healthy (the frozen contract's `None`).
    pub(super) llm_degraded: Option<crate::orchestrator::LlmDegraded>,
    /// Ephemeral signal that the primary embedder is down and semantic dedup is
    /// PAUSED (duplicate memory writes may leak until it recovers). `None`
    /// when healthy. Additive field (like `llm_degraded`); the app-shell can
    /// surface a degraded-dedup banner. Serialized as `null` when healthy.
    pub(super) embedder_degraded: Option<crate::orchestrator::EmbedderDegraded>,
}

#[derive(Serialize)]
pub(super) struct VNextSettingsProjection {
    pub(super) schema_version: u32,
    pub(super) signed_in_owner: String,
    pub(super) connected_services: Vec<VNextStatusSummary>,
    pub(super) security: Vec<VNextStatusSummary>,
    pub(super) notification_preferences: Vec<VNextStatusSummary>,
    pub(super) cost_saving_paused: bool,
    pub(super) advanced_diagnostics_available: bool,
    /// The owner's configured IANA timezone (e.g. `Asia/Shanghai`).
    /// The webapp passes this to `Intl.DateTimeFormat({ timeZone })` so all
    /// user-facing time displays render in the owner's timezone rather than
    /// the browser's. Falls back to the backend default (the host zone; see
    /// `settings::user_tz_default`).
    pub(super) user_timezone: String,
    /// The owner's configured display language (`"zh"` or `"en"`). The webapp
    /// uses this to select its locale (see `webapp/src/i18n`). Falls back to the
    /// backend default (`"zh"`) when no setting is present.
    pub(super) display_language: String,
    /// The active LLM provider backend
    /// (`"openrouter"` | `"codex_oauth"` | `"xai_subscription"` plus other
    /// registered values). The webapp's provider switcher reads this to show the
    /// current selection. Sourced from the `llm_provider` setting; defaults to
    /// `"openrouter"` when unset.
    pub(super) llm_provider: String,
    /// The active LLM model id (an arbitrary provider slug such as
    /// `deepseek/deepseek-v4-flash`). Empty means provider default. The webapp
    /// model field reads this to pre-fill the current value. Sourced from the
    /// `llm_model` setting; defaults to empty when unset.
    pub(super) llm_model: String,
    /// Whether fallback to the paid OpenAI-compatible API is permitted when the
    /// subscription-backed provider is unavailable. Sourced from the
    /// `paid_api_fallback_enabled` setting; defaults to `false`.
    pub(super) paid_api_fallback_enabled: bool,
    /// Web search provider (`xai_subscription` or `openrouter`). Default is the
    /// Grok subscription. Independent of `llm_provider`.
    pub(super) web_search_provider: String,
    /// Effective web search model id for `web_search_provider`. Subscription
    /// default is `grok-4.7`. OpenRouter values keep the `:online` suffix.
    pub(super) web_search_model: String,
    /// Models the settings selector may offer for `web_search_provider`.
    /// Includes the saved model when it is outside the short preset list.
    pub(super) web_search_model_options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebPushSubscriptionKeys {
    pub(super) p256dh: String,
    pub(super) auth: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebPushSubscriptionRecord {
    pub(super) endpoint: String,
    pub(super) keys: WebPushSubscriptionKeys,
    pub(super) user_agent: Option<String>,
    pub(super) created_at_unix: i64,
    pub(super) updated_at_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebPushSubscriptionState {
    pub(super) schema_version: u32,
    pub(super) owner_did: String,
    pub(super) subscriptions: Vec<WebPushSubscriptionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebPushDeliveryReceipt {
    pub(super) id: String,
    pub(super) created_at_unix: i64,
    pub(super) status: String,
    pub(super) tag: String,
    pub(super) title: String,
    pub(super) attempted: usize,
    pub(super) delivered: usize,
    pub(super) failed: usize,
    pub(super) removed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebPushDeliveryReceiptState {
    pub(super) schema_version: u32,
    pub(super) owner_did: String,
    pub(super) receipts: Vec<WebPushDeliveryReceipt>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct WebPushBrowserSubscription {
    pub(super) endpoint: String,
    pub(super) keys: WebPushSubscriptionKeys,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct VNextWebPushSubscribeRequest {
    #[allow(dead_code)]
    pub(super) schema_version: Option<u32>,
    pub(super) subscription: WebPushBrowserSubscription,
    pub(super) user_agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct VNextWebPushUnsubscribeRequest {
    pub(super) endpoint: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct VNextWebPushStatus {
    pub(super) schema_version: u32,
    pub(super) configured: bool,
    pub(super) send_configured: bool,
    pub(super) subscribed: bool,
    pub(super) subscription_count: usize,
    pub(super) public_key: Option<String>,
    pub(super) last_delivery: Option<VNextWebPushDeliverySummary>,
    pub(super) owner_message: String,
    pub(super) owner_next_step: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct VNextWebPushDeliverySummary {
    pub(super) id: String,
    pub(super) status: String,
    pub(super) delivered: usize,
    pub(super) attempted: usize,
    pub(super) removed: usize,
    pub(super) created_at_unix: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct VNextWebPushTestResponse {
    pub(super) schema_version: u32,
    pub(super) status: String,
    pub(super) attempted: usize,
    pub(super) delivered: usize,
    pub(super) failed: usize,
    pub(super) removed: usize,
    pub(super) receipt: String,
    pub(super) owner_message: String,
}

#[derive(Debug, Clone)]
pub(super) struct WebPushNotice {
    pub(super) title: String,
    pub(super) body: String,
    pub(super) tag: String,
    pub(super) url: String,
}

#[derive(Debug)]
pub(super) struct WebPushSendError {
    pub(super) expired_subscription: bool,
    pub(super) message: String,
}

#[derive(Serialize)]
pub(super) struct VNextChatHistoryProjection {
    pub(super) schema_version: u32,
    pub(super) messages: Vec<VNextChatHistoryMessage>,
    /// Older messages exist before `messages[0]`; the console offers 加载更早
    /// and asks again with `?before=<timestamp of messages[0]>`.
    pub(super) has_more: bool,
    /// Pending and recently resolved tool holds for this owner. Not part of
    /// the message window, so a refresh still shows the one approval card.
    pub(super) approvals: Vec<VNextChatApprovalCard>,
}

#[derive(Serialize)]
pub(super) struct VNextChatHistoryMessage {
    pub(super) role: String,
    /// Stable request identity; absent for historical and non-Web messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) turn_id: Option<String>,
    pub(super) text: String,
    pub(super) timestamp: i64,
    pub(super) severity: String,
    /// Reply latency in ms (assistant rows stored after 2026-09-12 only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) latency_ms: Option<u64>,
    /// Consecutive heartbeat alerts of one skill collapse into the latest
    /// copy; `repeat_count` (≥ 2) and `first_timestamp` describe the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) repeat_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) first_timestamp: Option<i64>,
    /// Internal grouping key (`heartbeat_alert:<skill>`); never serialized.
    #[serde(skip)]
    pub(super) alert_group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) work_item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) evidence: Option<Vec<crate::evidence::EvidenceItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) unsupported_numbers: Option<Vec<String>>,
    /// Owner text to send again when this row is the failure hint
    /// (`assistant.turn_unanswered` or `assistant.empty_response`). Absent on
    /// ordinary replies, so the console does not offer retry there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) retry_text: Option<String>,
}

/// One owner tool hold on the 安琪 thread. Duplicate stored rows collapse to
/// this card. `summary` is the egress card (`destination` / `would_send` /
/// `stopped`) or empty when the hold is not an egress card.
#[derive(Serialize)]
pub(super) struct VNextChatApprovalCard {
    pub(super) approval_id: String,
    pub(super) tool_name: String,
    pub(super) summary: String,
    pub(super) state: String,
    pub(super) timestamp: i64,
    /// `result` or `failure` plus the sentence the owner should read after
    /// approving a search. Absent until that replay finishes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) outcome: Option<String>,
}

#[derive(Clone, Serialize)]
pub(super) struct VNextAvatarProfileProjection {
    pub(super) schema_version: u32,
    pub(super) aegis_avatar_url: String,
    pub(super) owner_avatar_url: String,
    pub(super) updated_at_unix: Option<i64>,
    pub(super) auto_refresh: VNextAvatarAutoRefreshProjection,
}

#[derive(Debug, Deserialize)]
pub(super) struct VNextAvatarProfileUpdateRequest {
    pub(super) schema_version: Option<u32>,
    pub(super) aegis_avatar_url: String,
    pub(super) owner_avatar_url: String,
}

#[derive(Clone, Serialize)]
pub(super) struct VNextAvatarAutoRefreshProjection {
    pub(super) enabled: bool,
    pub(super) provider: String,
    pub(super) interval_days: u32,
    pub(super) image_generation_available: bool,
    pub(super) status: String,
    pub(super) owner_message: String,
    pub(super) owner_next_step: Option<String>,
    pub(super) last_refresh_at_unix: Option<i64>,
    pub(super) next_refresh_at_unix: Option<i64>,
    pub(super) prompt_hint: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct VNextAvatarAutoRefreshUpdateRequest {
    pub(super) schema_version: Option<u32>,
    pub(super) enabled: bool,
    pub(super) prompt_hint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct VNextAegisAvatarGenerateRequest {
    pub(super) schema_version: Option<u32>,
    pub(super) prompt_hint: Option<String>,
}

#[derive(Serialize)]
pub(super) struct VNextAegisAvatarGenerateResponse {
    pub(super) schema_version: u32,
    pub(super) status: String,
    pub(super) owner_message: String,
    pub(super) provider: String,
    pub(super) provider_request_id: Option<String>,
    pub(super) profile: VNextAvatarProfileProjection,
}

#[derive(Debug, Deserialize)]
pub(super) struct VNextCommandConversationTurn {
    pub(super) role: String,
    pub(super) text: String,
    pub(super) kind: Option<String>,
    pub(super) source_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct VNextCommandRequest {
    pub(super) schema_version: Option<u32>,
    pub(super) input: String,
    pub(super) mode: String,
    pub(super) intent_kind: Option<String>,
    pub(super) focus_item_id: Option<String>,
    pub(super) client_context: String,
    pub(super) idempotency_key: Option<String>,
    pub(super) conversation_context: Option<Vec<VNextCommandConversationTurn>>,
    /// Slice 5: optional multimodal attachments for the general-chat path. The
    /// browser uploads each file via `/webapp/vnext/upload` (which validates
    /// MIME by header + magic-number and returns a base64 `data_url`) then
    /// includes the resulting descriptors here. Voice is pre-transcribed text.
    /// Only honored on the general-chat (`home`) path — see `webapp_assistant_respond`.
    #[serde(default)]
    pub(super) attachments: Vec<VNextCommandAttachment>,
}

/// Slice 5: a single webapp chat attachment. Tagged enum mirroring the subset
/// of `crate::multimodal::MediaAttachment` reachable from the browser composer.
/// `Image`/`Document` carry an in-memory base64 `data_url` (no disk persistence);
/// `Voice` carries the browser-side transcript (no audio bytes round-trip).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum VNextCommandAttachment {
    Image {
        data_url: String,
    },
    Voice {
        transcript: String,
        #[serde(default)]
        duration_secs: u32,
    },
    Document {
        data_url: String,
        filename: String,
        mime_type: String,
    },
}

/// Slice 5: map a webapp attachment descriptor onto the orchestrator's
/// `MediaAttachment`. Pure + total (no I/O, no validation side effects); the
/// size/MIME checks live in `validate_vnext_command_request`. Voice maps to the
/// transcript variant (browser already did STT), so the orchestrator never
/// re-transcribes a webapp voice turn.
pub(super) fn vnext_attachment_to_media_attachment(
    attachment: &VNextCommandAttachment,
) -> crate::multimodal::MediaAttachment {
    match attachment {
        VNextCommandAttachment::Image { data_url } => crate::multimodal::MediaAttachment::Image {
            data_url: data_url.clone(),
        },
        VNextCommandAttachment::Voice {
            transcript,
            duration_secs,
        } => crate::multimodal::MediaAttachment::Voice {
            transcript: transcript.clone(),
            duration_secs: *duration_secs,
        },
        VNextCommandAttachment::Document {
            data_url,
            filename,
            mime_type,
        } => crate::multimodal::MediaAttachment::Document {
            data_url: data_url.clone(),
            filename: filename.clone(),
            mime_type: mime_type.clone(),
        },
    }
}

/// The message text a webapp command turn actually carries.
///
/// Typed text always wins. When the owner sent an attachment with no caption,
/// the text is derived from the attachment so the orchestrator receives a real
/// message instead of an empty string (which every downstream general-assistant
/// gate rejects):
/// - voice note → its server-side transcript (this IS the message);
/// - image → a localized "the owner sent an image" caption, mirroring the
///   Telegram photo path;
/// - document → its filename, which is the only thing the owner named.
///
/// Pure and total; `locale` is the owner's display language.
pub(super) fn vnext_command_effective_input(
    input: &str,
    attachments: &[VNextCommandAttachment],
    locale: &str,
) -> String {
    let trimmed = input.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    for attachment in attachments {
        let derived = match attachment {
            VNextCommandAttachment::Voice { transcript, .. } => transcript.trim().to_string(),
            VNextCommandAttachment::Image { .. } => {
                crate::i18n::t("webapp.attachment.imageOnly", locale).to_string()
            }
            VNextCommandAttachment::Document { filename, .. } => {
                let filename = filename.trim();
                if filename.is_empty() {
                    crate::i18n::t("webapp.attachment.documentOnly", locale).to_string()
                } else {
                    filename.to_string()
                }
            }
        };
        if !derived.is_empty() {
            return derived;
        }
    }
    String::new()
}

#[derive(Debug, Deserialize)]
pub(super) struct VNextWorkActionRequest {
    pub(super) schema_version: Option<u32>,
    pub(super) action_id: String,
    pub(super) idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct VNextDebugSnapshotRequest {
    pub(super) schema_version: Option<u32>,
    pub(super) surface: String,
    pub(super) path: String,
    pub(super) payload: serde_json::Value,
    pub(super) idempotency_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct VNextDebugSnapshotResponse {
    pub(super) schema_version: u32,
    pub(super) id: String,
    pub(super) expires_at_unix: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct VNextDebugLeaseRequest {
    pub(super) schema_version: Option<u32>,
    pub(super) scope: String,
    pub(super) permissions: Option<Vec<String>>,
    pub(super) duration_minutes: Option<u32>,
    pub(super) item_id: Option<String>,
    pub(super) reason: Option<String>,
    pub(super) idempotency_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct VNextDebugLeaseResponse {
    pub(super) schema_version: u32,
    pub(super) lease: WebappDebugLeaseSummary,
    pub(super) owner_message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) debug_grant_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct VNextDebugJobRequest {
    pub(super) schema_version: Option<u32>,
    pub(super) kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct WebappDebugJobSummary {
    pub(super) id: String,
    pub(super) owner: String,
    pub(super) principal_type: String,
    pub(super) kind: String,
    pub(super) status: String,
    pub(super) summary: String,
    pub(super) log_text: String,
    pub(super) exit_code: Option<i64>,
    pub(super) lease_id: Option<String>,
    pub(super) created_at_unix: i64,
    pub(super) started_at_unix: Option<i64>,
    pub(super) completed_at_unix: Option<i64>,
    pub(super) expires_at_unix: i64,
    pub(super) evidence: WebappDebugJobEvidence,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct WebappDebugJobEvidence {
    pub(super) schema_version: u32,
    pub(super) verdict: String,
    pub(super) owner_summary: String,
    pub(super) phases: Vec<WebappDebugJobEvidencePhase>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct WebappDebugJobEvidencePhase {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) status: String,
    pub(super) summary: String,
}

#[derive(Debug, Clone)]
pub(super) struct WebappDebugJobRuntimeConfig {
    pub(super) db_path: String,
    pub(super) kind: String,
    pub(super) base_url: Option<String>,
    pub(super) repo_root: PathBuf,
}

#[derive(Debug)]
pub(super) struct WebappDebugJobRunOutcome {
    pub(super) passed: bool,
    pub(super) summary: String,
    pub(super) log_text: String,
    pub(super) exit_code: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(super) struct WebappDebugSnapshotSummary {
    pub(super) id: String,
    pub(super) owner: String,
    pub(super) principal_type: String,
    pub(super) surface: String,
    pub(super) path: String,
    pub(super) payload_bytes: i64,
    pub(super) created_at_unix: i64,
    pub(super) expires_at_unix: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct WebappDebugLeaseSummary {
    pub(super) id: String,
    pub(super) owner: String,
    pub(super) principal_type: String,
    pub(super) scope: String,
    pub(super) permissions: Vec<String>,
    pub(super) item_id: Option<String>,
    pub(super) reason: String,
    pub(super) created_at_unix: i64,
    pub(super) expires_at_unix: i64,
    pub(super) revoked_at_unix: Option<i64>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(super) struct WebappDebugLeasePrincipal {
    pub(super) id: String,
    pub(super) owner: String,
    pub(super) principal_type: String,
}

#[derive(Debug, Serialize)]
pub(super) struct WebappDebugSnapshotRecord {
    pub(super) id: String,
    pub(super) owner: String,
    pub(super) principal_type: String,
    pub(super) surface: String,
    pub(super) path: String,
    pub(super) payload: serde_json::Value,
    pub(super) payload_bytes: i64,
    pub(super) created_at_unix: i64,
    pub(super) expires_at_unix: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct ContextualAssistantContextRequest {
    pub(super) focus_id: String,
    pub(super) intent_kind: Option<String>,
    #[allow(dead_code)]
    client_context_hint: Option<serde_json::Value>,
    #[allow(dead_code)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ContextualAssistantRespondRequest {
    pub(super) schema_version: Option<u32>,
    pub(super) focus_id: String,
    pub(super) intent_kind: String,
    pub(super) user_text: Option<String>,
    pub(super) requested_output_kind: Option<String>,
    #[allow(dead_code)]
    pub(super) client_context_hint: Option<serde_json::Value>,
    #[allow(dead_code)]
    pub(super) idempotency_key: Option<String>,
    /// Slice 5: multimodal attachments forwarded from `VNextCommandRequest` by
    /// the `/command` handler (the owner_request JSON it builds). Only the
    /// general-chat branch reads these; focused/contextual paths ignore them.
    #[serde(default)]
    pub(super) attachments: Vec<VNextCommandAttachment>,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
pub(super) struct ContextualAssistantActionDraftRequest {
    #[allow(dead_code)]
    pub(super) schema_version: Option<u32>,
    #[allow(dead_code)]
    pub(super) focus_id: String,
    pub(super) draft_text: String,
    pub(super) requested_action_kind: Option<String>,
    pub(super) suggested_intent_kind: Option<String>,
    pub(super) user_text: Option<String>,
    pub(super) idempotency_key: Option<String>,
}

pub(super) const GENERAL_ASSISTANT_FOCUS_ID: &str = "assistant:general";

#[derive(Debug, Deserialize)]
pub(super) struct WebappMandateDraftRequest {
    pub(super) schema_version: Option<u32>,
    pub(super) title: String,
    pub(super) description: String,
    pub(super) scope_kind: String,
    pub(super) source_filters: Option<serde_json::Value>,
    pub(super) allowed_reads: Vec<String>,
    pub(super) allowed_llm_uses: Option<Vec<String>>,
    pub(super) allowed_actions: Vec<String>,
    pub(super) must_ask: Vec<String>,
    pub(super) notification_policy: Option<serde_json::Value>,
    pub(super) budget_policy: Option<serde_json::Value>,
    pub(super) frequency_policy: Option<serde_json::Value>,
    pub(super) expiry: Option<String>,
    pub(super) review_after: Option<String>,
    pub(super) source_event_id: Option<String>,
    pub(super) trace_id: Option<String>,
    pub(super) run_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WebappTaskActionRequest {
    pub(super) schema_version: Option<u32>,
    pub(super) action: String,
    pub(super) preset: Option<String>,
}

#[derive(Clone, Serialize)]
pub(super) struct ContextEnvelope {
    pub(super) schema_version: u32,
    pub(super) focus_id: String,
    pub(super) focus_kind: String,
    pub(super) focus_source: String,
    pub(super) visible_fields: BTreeMap<String, String>,
    pub(super) redacted_fields: Vec<String>,
    pub(super) authorized_sensitive_scopes: Vec<String>,
    pub(super) denied_sensitive_scopes: Vec<String>,
    pub(super) allowed_intents: Vec<String>,
    pub(super) required_gate: Option<ContextRequiredGate>,
    pub(super) related_action_ids: Vec<String>,
    pub(super) related_receipt_ids: Vec<String>,
    pub(super) source_event_id: Option<String>,
    pub(super) trace_id: Option<String>,
    pub(super) run_id: Option<String>,
}

#[derive(Clone, Serialize)]
pub(super) struct ContextRequiredGate {
    pub(super) gate_kind: String,
    pub(super) reason: String,
    pub(super) route: Option<String>,
    pub(super) requires_mfa: bool,
}

#[derive(Clone, Serialize)]
pub(super) struct ContextAuthorityEnvelope {
    pub(super) item_id: String,
    pub(super) authority_basis: String,
    pub(super) visible_fields: Vec<String>,
    pub(super) locked_fields: Vec<String>,
    pub(super) allowed_now: Vec<String>,
    pub(super) requires_data_gate: bool,
    pub(super) requires_mfa: bool,
    pub(super) requires_approval: bool,
    pub(super) expected_receipt: String,
    pub(super) source_mandate_id: Option<String>,
    pub(super) audit_links: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct ContextualAssistantResponse {
    pub(super) schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) turn_id: Option<String>,
    pub(super) response_kind: String,
    pub(super) visible_answer: String,
    pub(super) severity: String,
    pub(super) draft: Option<String>,
    pub(super) action_draft: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) suggested_intent: Option<OwnerIntentSuggestion>,
    pub(super) authority: ContextAuthorityEnvelope,
    pub(super) required_gate: Option<ContextRequiredGate>,
    pub(super) receipt_expectation: String,
    /// Owner chat turns: ms from the message reaching the orchestrator to the
    /// stored reply (owner 2026-09-12). Absent for non-runtime responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) latency_ms: Option<u64>,
    pub(super) citations: Vec<String>,
    pub(super) source_evidence: Vec<ContextualAssistantSourceEvidence>,
    pub(super) evidence: Vec<crate::evidence::EvidenceItem>,
    pub(super) unsupported_numbers: Vec<String>,
    pub(super) warnings: Vec<String>,
    pub(super) trace_id: Option<String>,
    pub(super) run_id: Option<String>,
    pub(super) provider_request_id: Option<String>,
}

#[derive(Clone, Serialize)]
pub(super) struct ContextualAssistantSourceEvidence {
    pub(super) kind: String,
    pub(super) label: String,
    pub(super) summary: String,
    pub(super) detail: String,
    /// Present only when this tool actually passed the Jev policy broker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) jev: Option<crate::computer_use::JevProvenance>,
}

#[derive(Clone, Serialize)]
pub(super) struct OwnerIntentSuggestion {
    pub(super) kind: String,
    pub(super) label: String,
    pub(super) summary: String,
    pub(super) confidence: f32,
    pub(super) next_action_kind: String,
}

#[cfg(test)]
#[derive(Serialize)]
pub(super) struct ContextualActionDraftResponse {
    pub(super) schema_version: u32,
    pub(super) status: String,
    pub(super) action_draft: serde_json::Value,
    pub(super) required_gate: Option<ContextRequiredGate>,
    pub(super) receipt_expectation: String,
    pub(super) warnings: Vec<String>,
    pub(super) trace_id: Option<String>,
    pub(super) run_id: Option<String>,
}
