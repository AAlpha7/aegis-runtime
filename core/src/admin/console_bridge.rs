use super::*;

pub(super) enum WebappConsoleDetailAccess {
    Allow {
        grant_id: Option<String>,
        reason: String,
        user_did: String,
        chat_id: i64,
    },
    RequireMfa {
        required_level: String,
        reason: String,
        freshness_secs: u64,
    },
}

pub(super) async fn webapp_console_check_detail_access(
    state: &AdminState,
    headers: &axum::http::HeaderMap,
    resource_type: &str,
    resource_id: &str,
    purpose: &str,
) -> Result<WebappConsoleDetailAccess, (StatusCode, String)> {
    let principal = extract_web_console_auth(state, headers).await?;
    let chat_id = console_chat_id_for_principal(state, &principal)?;
    let user_did = format!("did:aegis:telegram:{chat_id}");

    if state.memory.db_path() == ":memory:" {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Sensitive read gate storage is unavailable".into(),
        ));
    }

    let conn = crate::encrypted_sqlite::open(state.memory.db_path()).map_err(|error| {
        tracing::warn!(
            error = %error,
            resource_type,
            purpose,
            "DataAccessGate storage open failed for Workbench detail"
        );
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Sensitive read gate storage is unavailable".to_string(),
        )
    })?;
    crate::data_access_gate::init_sensitive_read_schema(&conn).map_err(|error| {
        tracing::warn!(
            error = %error,
            resource_type,
            purpose,
            "DataAccessGate schema init failed for Workbench detail"
        );
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Sensitive read gate storage is unavailable".to_string(),
        )
    })?;

    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let mut access_principal = principal.data_access_principal(user_did.clone());
    if let WebConsolePrincipal::WebConsole(session) = &principal
        && session.google_sub == crate::web_console_auth::LOCAL_LOGIN_SUB
    {
        // The local login token already proved control of this host. That
        // proof is enough to open the task body. It stays off the stored
        // session, so a high-risk approval still asks for a second confirmation.
        access_principal.mfa_level = crate::web_console_auth::MfaLevel::Host;
        access_principal.mfa_verified_at = Some(now);
    }
    let decision = crate::data_access_gate::check_or_request_access(
        &conn,
        &crate::data_access_gate::DataAccessRequest {
            principal: access_principal,
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
            sensitivity: crate::data_access_gate::DataSensitivity::S3Sensitive,
            actions: vec![
                crate::data_access_gate::DataAction::ReadBody,
                crate::data_access_gate::DataAction::DisplayInUi,
            ],
            purpose: purpose.into(),
            now,
            grant_ttl_secs: crate::data_access_gate::DEFAULT_READ_GRANT_TTL_SECS,
            mfa_freshness_secs: Some(WEB_CONSOLE_PROTECTED_READ_FRESHNESS_SECS),
            model: None,
            trace_id: None,
        },
    );

    match decision {
        crate::data_access_gate::DataAccessDecision::Allow { grant, reason } => {
            Ok(WebappConsoleDetailAccess::Allow {
                grant_id: grant.map(|grant| grant.grant_id),
                reason,
                user_did,
                chat_id,
            })
        }
        crate::data_access_gate::DataAccessDecision::RequireMfa {
            required_level,
            reason,
            freshness_secs,
        } => Ok(WebappConsoleDetailAccess::RequireMfa {
            required_level: match required_level {
                crate::data_access_gate::RequiredMfa::None => "none",
                crate::data_access_gate::RequiredMfa::TotpOrWebAuthn => "totp_or_web_authn",
                crate::data_access_gate::RequiredMfa::WebAuthn => "web_authn",
            }
            .into(),
            reason,
            freshness_secs,
        }),
        crate::data_access_gate::DataAccessDecision::Deny { reason } => Err((
            StatusCode::FORBIDDEN,
            format!("Sensitive read denied: {reason}"),
        )),
    }
}

pub(super) fn trust_console_approval_type(approval_id: &str, value: &str) -> String {
    let parsed =
        serde_json::from_str::<serde_json::Value>(value).unwrap_or(serde_json::Value::Null);
    parsed
        .get("approval_type")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            parsed
                .get("tool_name")
                .and_then(serde_json::Value::as_str)
                .map(|tool| format!("tool:{tool}"))
        })
        .or_else(|| {
            parsed
                .get("server_key")
                .and_then(serde_json::Value::as_str)
                .map(|_| "mcp_server".to_string())
        })
        .or_else(|| {
            parsed
                .get("key")
                .and_then(serde_json::Value::as_str)
                .map(|_| "setting_change".to_string())
        })
        .unwrap_or_else(|| {
            approval_id
                .split_once('-')
                .map(|(prefix, _)| prefix.to_string())
                .unwrap_or_else(|| "approval".to_string())
        })
}

pub(super) fn trust_console_approval_risk(approval_type: &str) -> &'static str {
    if approval_type.contains("sysadmin_ops")
        || approval_type.contains("credential")
        || approval_type.contains("oauth")
    {
        "R4 Critical"
    } else if approval_type.contains("send")
        || approval_type.contains("delete")
        || approval_type.contains("setting_change")
        || approval_type.contains("mcp_server")
    {
        "R3 High"
    } else if approval_type.contains("tool:") {
        "R2 Medium"
    } else {
        "R1 Low"
    }
}

pub(super) fn trust_console_receipt_status(
    entry: &crate::action_ledger::ActionLedgerEntry,
) -> &'static str {
    if entry.external_receipt.is_some()
        || entry.external_message_id.is_some()
        || entry.external_event_id.is_some()
        || entry.tool_call_id.is_some()
        || entry.provider_request_id.is_some()
    {
        "Present"
    } else {
        "Missing"
    }
}

pub(super) fn trust_console_action_status_label(
    entry: &crate::action_ledger::ActionLedgerEntry,
) -> &'static str {
    if entry.status == crate::action_ledger::ActionStatus::Executed
        && trust_console_receipt_status(entry) == "Missing"
    {
        "needs_receipt"
    } else {
        entry.status.as_str()
    }
}

#[cfg(test)]
pub(super) fn trust_console_action_kind_label(
    kind: crate::action_ledger::ActionKind,
) -> &'static str {
    match kind {
        crate::action_ledger::ActionKind::TelegramSend => "Notification sent",
        crate::action_ledger::ActionKind::EmailSend => "Email send",
        crate::action_ledger::ActionKind::EmailDraft => "Email draft",
        crate::action_ledger::ActionKind::EmailForward => "Email forward",
        crate::action_ledger::ActionKind::EmailUpdate => "Email update",
        crate::action_ledger::ActionKind::CalendarCreate => "Calendar create",
        crate::action_ledger::ActionKind::CalendarUpdate => "Calendar update",
        crate::action_ledger::ActionKind::CalendarDelete => "Calendar delete",
        crate::action_ledger::ActionKind::ReminderCreate => "Reminder create",
        crate::action_ledger::ActionKind::ReminderUpdate => "Reminder update",
        crate::action_ledger::ActionKind::ReminderDelete => "Reminder delete",
        crate::action_ledger::ActionKind::TaskCreate => "Task create",
        crate::action_ledger::ActionKind::KbMerge => "Memory merge",
        crate::action_ledger::ActionKind::KbUpdate => "Memory update",
        crate::action_ledger::ActionKind::KbDelete => "Memory delete",
        crate::action_ledger::ActionKind::ApprovalRequest => "Approval request",
        crate::action_ledger::ActionKind::ResearchStart => "Research start",
        crate::action_ledger::ActionKind::RuntimeSettingChange => "Runtime setting change",
        crate::action_ledger::ActionKind::SkillCreate => "Skill create",
        crate::action_ledger::ActionKind::SkillDeploy => "Skill deploy",
        crate::action_ledger::ActionKind::CodeCommit => "Code commit",
        crate::action_ledger::ActionKind::PullRequestCreate => "Pull request create",
        crate::action_ledger::ActionKind::Other => "Action",
    }
}

pub(super) fn assistant_event_entity_string<'a>(
    event: &'a crate::assistant_events::StoredAssistantEvent,
    key: &str,
) -> Option<&'a str> {
    event.event.entities.get(key)?.as_str()
}

pub(super) fn webapp_action_ledger_entry_belongs_to_owner(
    entry: &crate::action_ledger::ActionLedgerEntry,
    chat_id: i64,
    user_did: &str,
) -> bool {
    entry.chat_id == Some(chat_id) || entry.requested_by.as_deref() == Some(user_did)
}

pub(super) fn build_webapp_task_timeline_events(
    task: &crate::memory::Task,
) -> Vec<serde_json::Value> {
    let mut events = vec![serde_json::json!({
        "kind": "created",
        "status": "completed",
        "at": &task.created_at,
        "label": "Task created",
    })];

    if let Some(trigger_at_unix) = task.trigger_at_unix {
        events.push(serde_json::json!({
            "kind": "scheduled",
            "status": if task.fired_at.is_some() { "completed" } else { "waiting" },
            "at_unix": trigger_at_unix,
            "label": "Scheduled trigger",
            "trigger_type": task.trigger_type.as_ref(),
            "event_key": task.event_key.as_ref(),
        }));
    }

    if let Some(fired_at) = &task.fired_at {
        events.push(serde_json::json!({
            "kind": "fired",
            "status": "completed",
            "at": fired_at,
            "label": "Task fired",
            "fire_count": task.fire_count,
        }));
    }

    if task.status == "dead_letter" || task.status == "failed" {
        events.push(serde_json::json!({
            "kind": "failed",
            "status": "failed",
            "at": &task.updated_at,
            "label": "Task failed",
            "retry_count": task.retry_count,
        }));
    } else if let Some(completed_at) = &task.completed_at {
        events.push(serde_json::json!({
            "kind": "completed",
            "status": "completed",
            "at": completed_at,
            "label": "Task completed",
        }));
    } else {
        events.push(serde_json::json!({
            "kind": "current",
            "status": &task.status,
            "at": &task.updated_at,
            "label": "Current state",
        }));
    }

    events
}

pub(super) fn user_facing_notification_route_reason(
    reason: crate::telegram_notification_router::NotificationRouteReason,
) -> &'static str {
    match reason {
        crate::telegram_notification_router::NotificationRouteReason::DirectDmContinuation => {
            "Sent to the current private chat"
        }
        crate::telegram_notification_router::NotificationRouteReason::PrivacyDmOnly
        | crate::telegram_notification_router::NotificationRouteReason::SensitivePreferDm => {
            "Kept in private chat because it may be sensitive"
        }
        crate::telegram_notification_router::NotificationRouteReason::OAuthDeepLinkException => {
            "Sent as a sign-in link"
        }
        crate::telegram_notification_router::NotificationRouteReason::NoLinkedGroupFallbackToDm => {
            "Sent to private chat because no linked notification topic is available"
        }
        crate::telegram_notification_router::NotificationRouteReason::ExplicitStoredChatAllowed => {
            "Sent to the approved chat"
        }
    }
}

// ── SSE Stream (Phase 2a) ───────────────────────────────────────────────────

// ── Per-turn webapp progress SSE (Slice 2) ──────────────────────────────────

/// RAII guard that unregisters a per-turn webapp sink from the shared
/// [`ProgressRouter`] when the SSE stream future is dropped (EventSource close,
/// turn end, or error). Idempotent — `unregister_turn` on an already-removed
/// key is a no-op.
struct TurnSinkGuard {
    router: Arc<crate::progress_router::ProgressRouter>,
    turn_id: String,
}

impl Drop for TurnSinkGuard {
    fn drop(&mut self) {
        self.router.unregister_turn(&self.turn_id);
    }
}

/// POST /webapp/vnext/turns/:turn_id/stream-token — issue a short-lived,
/// chat-bound token the webapp passes on the EventSource query string (which
/// cannot carry auth headers / CSRF). Console auth + CSRF gate the mint; the
/// token then authorizes the matching SSE GET. Keys on `turn_id` (the
/// client's `idempotency_key`), so a token for one turn cannot open another
/// (no enumeration).
pub(super) async fn webapp_vnext_turn_stream_token(
    State(state): State<Arc<AdminState>>,
    Path(turn_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    require_web_console_csrf_for_principal(req.headers(), &principal)?;

    if turn_id.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Missing turn id".to_string()));
    }
    // Rate-limit per principal+turn so a client cannot churn tokens.
    check_webapp_rate_limit(
        &principal,
        "turn-stream-token",
        &turn_id,
        WEBAPP_STREAM_TOKEN_RATE_LIMIT_PER_MINUTE,
    )?;

    // Bind the token to the principal's console chat id; the SSE GET validates
    // token↔turn_id and the grant carries chat_id (no further owner check
    // needed — the token IS the capability, scoped to this chat + this turn).
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let (token, expires_at) = mint_webapp_stream_token(turn_id, chat_id);
    Ok(Json(serde_json::json!({
        "status": "ok",
        "stream_token": token,
        "expires_at": expires_at,
        "expires_in": WEBAPP_STREAM_TOKEN_TTL_SECS,
    })))
}

/// GET /webapp/vnext/turns/:turn_id/events?stream_token=<token> — per-turn SSE
/// of typed progress events for the webapp chat surface.
///
/// Auth: the short-lived `stream_token` (minted by the POST above, console
/// auth + CSRF) is validated against `turn_id`; the grant binds the chat id, so
/// no turn enumeration is possible. The stream registers a bounded per-turn
/// sink on the shared [`ProgressRouter`]; the orchestrator's events for this
/// `turn_id` then fan out here (the Telegram sink is unaffected). Each event
/// becomes a typed SSE frame whose `event:` name = the frame type (so the
/// client can `addEventListener(type, …)`). A `FinalReply` (or the channel
/// closing) emits a terminal `done` frame and ends the stream. The sink is
/// unregistered on drop via [`TurnSinkGuard`].
pub(super) async fn webapp_vnext_turn_events(
    State(state): State<Arc<AdminState>>,
    Path(turn_id): Path<String>,
    Query(query): Query<StreamQuery>,
) -> Result<
    axum_sse::Sse<impl futures::Stream<Item = Result<axum_sse::Event, std::convert::Infallible>>>,
    (StatusCode, String),
> {
    let stream_token = query
        .stream_token
        .as_deref()
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Missing stream token".to_string()))?;
    // Validates token↔turn_id match + expiry; the grant binds chat_id.
    let grant = validate_webapp_stream_token(stream_token, &turn_id)?;
    tracing::debug!(
        turn_id = %turn_id,
        chat_id = grant.chat_id,
        "webapp per-turn progress stream opened"
    );

    let router = state.progress_router.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Progress router not initialized".to_string(),
        )
    })?;

    // Bounded per-turn sink. Drops on full are acceptable (advisory progress);
    // the orchestrator's `try_send` already drops + logs when this lags.
    // Assistant deltas share this sink. 512 keeps a burst of tokens from
    // being dropped while the browser paints; a drop is still advisory — the
    // persisted reply replaces the bubble when the command finishes.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(512);
    router.register_turn(turn_id.clone(), tx);
    // Unregister on stream drop (client disconnect / turn end / error).
    let guard = TurnSinkGuard {
        router: router.clone(),
        turn_id: turn_id.clone(),
    };

    let stream = async_stream::stream! {
        // Hold the guard for the lifetime of the stream so the sink is
        // unregistered exactly when the stream future is dropped.
        let _guard = guard;
        let mut emitted_done = false;
        while let Some(event) = rx.recv().await {
            // `FinalReply` serializes to a `done` frame and terminates the turn.
            let is_terminal = matches!(
                &*event.payload,
                crate::progress_router::ProgressPayload::FinalReply { .. }
            );
            let frame = crate::progress_router::progress_event_to_sse_json(&event);
            yield Ok(axum_sse::Event::default()
                .event(frame.event_type)
                .data(frame.data.to_string()));
            if is_terminal {
                emitted_done = true;
                break;
            }
        }
        // Channel closed without an explicit FinalReply (turn ended / sink
        // dropped): emit a terminal `done` so the client stops waiting. Skipped
        // when a `FinalReply` already produced the `done` frame (no duplicate).
        if !emitted_done {
            yield Ok(axum_sse::Event::default()
                .event("done")
                .data(serde_json::json!({ "type": "done", "turn_id": turn_id }).to_string()));
        }
    };

    Ok(axum_sse::Sse::new(stream).keep_alive(axum_sse::KeepAlive::default()))
}
