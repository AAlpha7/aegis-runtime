//! Calendar write-tool handlers (create/update/delete events).
//!
//! Mirrors the structure of `orchestrator_tools_knowledge.rs` (email+KB). The
//! three pseudo-tools in this module are all HITL-gated at the `ToolGate`
//! layer — every single invocation surfaces an approval button before the
//! handler runs. Beyond the gate, the module enforces a second policy:
//! the "invite others" capability is disabled by default and requires the
//! user to explicitly flip `calendar_invite_others_enabled=true` (a separate
//! HITL-approved setting change) before the LLM can send invites to anyone
//! other than the authenticated user.
//!
//! See `docs/calendar-write-capability-plan.md` for the full design.

use anyhow::Result;
use serde_json::Value;

use async_openai::types::chat::ChatCompletionMessageToolCall;

use crate::assistant_events::build_calendar_event_changed_event;
use crate::orchestrator::{Orchestrator, ToolContext};
use crate::settings::RuntimeSettings;
use crate::utils::{sanitized_tool_error, truncate_str};

/// Calendar API base URL. Calendar endpoints live under `www.googleapis.com`
/// (shared with Drive), not `www.googleapis.com`.
const CALENDAR_DOMAIN_KEY: &str = "www.googleapis.com";

impl Orchestrator {
    /// Handle calendar-domain tools: calendar_create_event, calendar_update_event,
    /// calendar_delete_event. Reads are still served by the `calendar_reader`
    /// Wasm skill — unchanged.
    #[allow(unused_variables)]
    pub(crate) async fn handle_tool_calendar(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "calendar_create_event" => execute_calendar_create_event,
            "calendar_update_event" => execute_calendar_update_event,
            "calendar_delete_event" => execute_calendar_delete_event,
        );
        Ok(None)
    }

    // ─────────────────────────────────────────────────────────────────────
    //  calendar_create_event
    // ─────────────────────────────────────────────────────────────────────

    async fn execute_calendar_create_event(
        &self,
        args: Value,
        ctx: &mut ToolContext<'_>,
    ) -> String {
        let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("");
        let start = args.get("start").and_then(|v| v.as_str()).unwrap_or("");
        let end = args.get("end").and_then(|v| v.as_str()).unwrap_or("");
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let location = args.get("location").and_then(|v| v.as_str()).unwrap_or("");
        let timezone = args.get("timezone").and_then(|v| v.as_str()).unwrap_or("");
        let calendar_id = args
            .get("calendar_id")
            .and_then(|v| v.as_str())
            .unwrap_or("primary");
        let account = args.get("account").and_then(|v| v.as_str()).unwrap_or("");
        let attendees = extract_attendees(&args);

        // Basic input validation
        if summary.is_empty() || summary.len() > 200 {
            return serde_json::json!({
                "status": "error",
                "error": "summary must be 1-200 characters",
                "error_kind": "invalid_input",
                "retryable": true,
            })
            .to_string();
        }
        if start.is_empty() || end.is_empty() {
            return serde_json::json!({
                "status": "error",
                "error": "start and end are required (RFC 3339 datetime)",
                "error_kind": "invalid_input",
                "retryable": true,
            })
            .to_string();
        }
        if description.len() > 5000 {
            return serde_json::json!({
                "status": "error",
                "error": "description exceeds 5000 characters",
                "error_kind": "invalid_input",
                "retryable": true,
            })
            .to_string();
        }

        // Validate attendee format + CRLF injection guard BEFORE any network
        // work. The attendee list is forwarded to Google and lands in To: /
        // Attendees: headers when invites go out, so it's the same injection
        // class as email addresses. Reuse the canonical helper so every
        // site rejects the same shape (audit 2026-04-19 paired-fix rule).
        if let Err(err_json) = validate_attendee_list(&attendees) {
            return err_json;
        }

        // Account resolution via the single source of truth used by every
        // Google send path (audit 2026-04-19 Finding #2).
        let resolved_account_opt: Option<String> = if account.is_empty() {
            None
        } else {
            Some(account.to_string())
        };

        // Attendee-policy check (self-only unless calendar_invite_others_enabled).
        // CTO round-1 refinement: when own_email cannot be resolved AND the LLM
        // supplied attendees, return a distinct `identity_unresolved` error
        // rather than letting `check_attendee_policy` report every attendee
        // as an "invite_others_disabled" offender (misleading — the real
        // problem is auth, not the invite-others toggle).
        let own_email = self.google_self_address().await.unwrap_or_default();
        if own_email.is_empty() && !attendees.is_empty() {
            return serde_json::json!({
                "status": "error",
                "error": "Cannot verify authenticated user email — google_self_address returned empty (likely unconfigured Google auth or revoked token).",
                "error_kind": "identity_unresolved",
                "retryable": true,
                "hint": "Ask the user to run `setup_google action=setup` to refresh OAuth, then retry the event with attendees.",
            }).to_string();
        }
        if let Err(err_json) = check_attendee_policy(&self.settings, &attendees, &own_email) {
            return err_json.to_string();
        }

        // Build the Google Calendar event body. Only include optional fields
        // the LLM actually set — Google treats absent properties as "don't
        // create/overwrite" which keeps the payload minimal.
        let mut event_body = serde_json::json!({
            "summary": summary,
            "start": build_datetime_obj(start, timezone),
            "end": build_datetime_obj(end, timezone),
        });
        if !description.is_empty() {
            event_body["description"] = Value::String(description.to_string());
        }
        if !location.is_empty() {
            event_body["location"] = Value::String(location.to_string());
        }
        if !attendees.is_empty() {
            let attendees_json: Vec<Value> = attendees
                .iter()
                .map(|email| serde_json::json!({"email": email}))
                .collect();
            event_body["attendees"] = Value::Array(attendees_json);
        }

        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{}/events",
            urlencoding_encode(calendar_id)
        );

        let ledger_action_id = self.best_effort_calendar_action_started(
            crate::action_ledger::ActionKind::CalendarCreate,
            &calendar_action_id("calendar-create", calendar_id, &args),
            ctx.chat_id,
            ctx.user_did,
            calendar_id,
            None,
            summary,
            "create",
            &args,
        );

        match self
            .google_api_call(
                reqwest::Method::POST,
                &url,
                Some(&event_body),
                resolved_account_opt.as_deref(),
                CALENDAR_DOMAIN_KEY,
            )
            .await
        {
            Ok(resp) => {
                let event_id = resp.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                let html_link = resp.get("htmlLink").and_then(|v| v.as_str()).unwrap_or("");
                let account_id = resolved_account_opt.as_deref().unwrap_or("primary");
                let observed_at = chrono::Utc::now().to_rfc3339();
                let (raw_event, assistant_event) = build_calendar_event_changed_event(
                    ctx.user_did,
                    &observed_at,
                    account_id,
                    "create",
                    calendar_id,
                    event_id,
                    summary,
                    Some(start),
                    Some(end),
                    attendees.len(),
                );
                if let Err(e) = self
                    .persist_assistant_event(&raw_event, &assistant_event)
                    .await
                {
                    tracing::warn!(error = %e, event_id, "[calendar] failed to persist assistant event");
                }
                if let Some(action_id) = ledger_action_id.as_deref() {
                    self.best_effort_calendar_action_executed(
                        action_id,
                        crate::action_ledger::ActionKind::CalendarCreate,
                        calendar_id,
                        event_id,
                        html_link,
                        summary,
                        "create",
                    );
                }
                serde_json::json!({
                    "status": "ok",
                    "action": "create_event",
                    "event_id": event_id,
                    "calendar_id": calendar_id,
                    "html_link": html_link,
                    "summary": summary,
                    "attendee_count": attendees.len(),
                    "message": format!(
                        "Event '{}' created (ID: {}). {} attendee(s).",
                        summary, event_id, attendees.len()
                    ),
                })
                .to_string()
            }
            Err(e) => {
                if let Some(action_id) = ledger_action_id.as_deref() {
                    self.best_effort_calendar_action_failed(action_id, "calendar_create_event", &e);
                }
                sanitized_tool_error(
                    "calendar_create_event",
                    "Google Calendar API error / 日历接口错误",
                    &e,
                    "api_error",
                    true,
                    Some(
                        "Verify the OAuth scope includes calendar.events and that the time fields are RFC 3339.",
                    ),
                )
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    //  calendar_update_event
    // ─────────────────────────────────────────────────────────────────────

    async fn execute_calendar_update_event(
        &self,
        args: Value,
        ctx: &mut ToolContext<'_>,
    ) -> String {
        let event_id = args.get("event_id").and_then(|v| v.as_str()).unwrap_or("");
        let calendar_id = args
            .get("calendar_id")
            .and_then(|v| v.as_str())
            .unwrap_or("primary");
        let account = args.get("account").and_then(|v| v.as_str()).unwrap_or("");

        if event_id.is_empty() {
            return serde_json::json!({
                "status": "error",
                "error": "event_id required",
                "error_kind": "invalid_input",
                "retryable": true,
            })
            .to_string();
        }

        // Build a PARTIAL payload that contains only the fields the caller
        // actually set. Google Calendar PATCH preserves unspecified fields —
        // unlike PUT which requires the full event object. The manage_tasks
        // UPDATE-preserves-unspecified-fields discipline applies here.
        let mut patch_body = serde_json::Map::new();

        if let Some(summary) = args.get("summary").and_then(|v| v.as_str()) {
            if summary.len() > 200 {
                return serde_json::json!({
                    "status": "error",
                    "error": "summary exceeds 200 characters",
                    "error_kind": "invalid_input",
                    "retryable": true,
                })
                .to_string();
            }
            patch_body.insert("summary".into(), Value::String(summary.to_string()));
        }
        if let Some(description) = args.get("description").and_then(|v| v.as_str()) {
            if description.len() > 5000 {
                return serde_json::json!({
                    "status": "error",
                    "error": "description exceeds 5000 characters",
                    "error_kind": "invalid_input",
                    "retryable": true,
                })
                .to_string();
            }
            patch_body.insert("description".into(), Value::String(description.to_string()));
        }
        if let Some(location) = args.get("location").and_then(|v| v.as_str()) {
            patch_body.insert("location".into(), Value::String(location.to_string()));
        }

        let timezone = args.get("timezone").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(start) = args.get("start").and_then(|v| v.as_str()) {
            patch_body.insert("start".into(), build_datetime_obj(start, timezone));
        }
        if let Some(end) = args.get("end").and_then(|v| v.as_str()) {
            patch_body.insert("end".into(), build_datetime_obj(end, timezone));
        }

        let attendees_supplied = args.get("attendees").is_some();
        let attendees = extract_attendees(&args);

        if attendees_supplied && let Err(err_json) = validate_attendee_list(&attendees) {
            return err_json;
        }

        let resolved_account_opt: Option<String> = if account.is_empty() {
            None
        } else {
            Some(account.to_string())
        };

        // Only run the attendee-policy check when the caller actually supplied
        // an attendees array — otherwise we'd overwrite existing attendees with
        // an empty list, violating the PATCH "preserve unspecified fields"
        // contract.
        if attendees_supplied {
            let own_email = self.google_self_address().await.unwrap_or_default();
            // CTO round-1 refinement — see execute_calendar_create_event for
            // the identity_unresolved rationale.
            if own_email.is_empty() && !attendees.is_empty() {
                return serde_json::json!({
                    "status": "error",
                    "error": "Cannot verify authenticated user email — google_self_address returned empty (likely unconfigured Google auth or revoked token).",
                    "error_kind": "identity_unresolved",
                    "retryable": true,
                    "hint": "Ask the user to run `setup_google action=setup` to refresh OAuth, then retry the event update with attendees.",
                }).to_string();
            }
            if let Err(err_json) = check_attendee_policy(&self.settings, &attendees, &own_email) {
                return err_json.to_string();
            }
            let attendees_json: Vec<Value> = attendees
                .iter()
                .map(|email| serde_json::json!({"email": email}))
                .collect();
            patch_body.insert("attendees".into(), Value::Array(attendees_json));
        }

        if patch_body.is_empty() {
            return serde_json::json!({
                "status": "error",
                "error": "no fields to update — supply at least one of summary/start/end/description/location/attendees",
                "error_kind": "invalid_input",
                "retryable": true,
            })
            .to_string();
        }

        let patch_value = Value::Object(patch_body);
        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{}/events/{}",
            urlencoding_encode(calendar_id),
            urlencoding_encode(event_id),
        );

        let ledger_action_id = self.best_effort_calendar_action_started(
            crate::action_ledger::ActionKind::CalendarUpdate,
            &calendar_action_id("calendar-update", calendar_id, &args),
            ctx.chat_id,
            ctx.user_did,
            calendar_id,
            Some(event_id),
            args.get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or(event_id),
            "update",
            &args,
        );

        match self
            .google_api_call(
                reqwest::Method::PATCH,
                &url,
                Some(&patch_value),
                resolved_account_opt.as_deref(),
                CALENDAR_DOMAIN_KEY,
            )
            .await
        {
            Ok(resp) => {
                let id = resp.get("id").and_then(|v| v.as_str()).unwrap_or(event_id);
                let html_link = resp.get("htmlLink").and_then(|v| v.as_str()).unwrap_or("");
                let account_id = resolved_account_opt.as_deref().unwrap_or("primary");
                let event_summary = resp
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .or_else(|| args.get("summary").and_then(|v| v.as_str()))
                    .unwrap_or(id);
                let start_value = args.get("start").and_then(|v| v.as_str());
                let end_value = args.get("end").and_then(|v| v.as_str());
                let attendee_count = args
                    .get("attendees")
                    .and_then(|v| v.as_array().map(|items| items.len()))
                    .unwrap_or(0);
                let observed_at = chrono::Utc::now().to_rfc3339();
                let (raw_event, assistant_event) = build_calendar_event_changed_event(
                    ctx.user_did,
                    &observed_at,
                    account_id,
                    "update",
                    calendar_id,
                    id,
                    event_summary,
                    start_value,
                    end_value,
                    attendee_count,
                );
                if let Err(e) = self
                    .persist_assistant_event(&raw_event, &assistant_event)
                    .await
                {
                    tracing::warn!(error = %e, event_id = id, "[calendar] failed to persist assistant event");
                }
                if let Some(action_id) = ledger_action_id.as_deref() {
                    self.best_effort_calendar_action_executed(
                        action_id,
                        crate::action_ledger::ActionKind::CalendarUpdate,
                        calendar_id,
                        id,
                        html_link,
                        event_summary,
                        "update",
                    );
                }
                serde_json::json!({
                    "status": "ok",
                    "action": "update_event",
                    "event_id": id,
                    "calendar_id": calendar_id,
                    "html_link": html_link,
                    "message": format!("Event {} updated.", id),
                })
                .to_string()
            }
            Err(e) => {
                if let Some(action_id) = ledger_action_id.as_deref() {
                    self.best_effort_calendar_action_failed(action_id, "calendar_update_event", &e);
                }
                sanitized_tool_error(
                    "calendar_update_event",
                    "Google Calendar API error / 日历接口错误",
                    &e,
                    "api_error",
                    true,
                    Some("Verify event_id exists and the OAuth scope includes calendar.events."),
                )
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    //  calendar_delete_event
    // ─────────────────────────────────────────────────────────────────────

    async fn execute_calendar_delete_event(
        &self,
        args: Value,
        ctx: &mut ToolContext<'_>,
    ) -> String {
        let event_id = args.get("event_id").and_then(|v| v.as_str()).unwrap_or("");
        let calendar_id = args
            .get("calendar_id")
            .and_then(|v| v.as_str())
            .unwrap_or("primary");
        let account = args.get("account").and_then(|v| v.as_str()).unwrap_or("");

        if event_id.is_empty() {
            return serde_json::json!({
                "status": "error",
                "error": "event_id required",
                "error_kind": "invalid_input",
                "retryable": true,
            })
            .to_string();
        }

        let resolved_account_opt: Option<String> = if account.is_empty() {
            None
        } else {
            Some(account.to_string())
        };
        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{}/events/{}",
            urlencoding_encode(calendar_id),
            urlencoding_encode(event_id),
        );
        let ledger_action_id = self.best_effort_calendar_action_started(
            crate::action_ledger::ActionKind::CalendarDelete,
            &calendar_action_id("calendar-delete", calendar_id, &args),
            ctx.chat_id,
            ctx.user_did,
            calendar_id,
            Some(event_id),
            event_id,
            "delete",
            &args,
        );

        // Calendar delete returns 204 No Content; google_api_call normalizes
        // that to {"ok": true}.
        match self
            .google_api_call(
                reqwest::Method::DELETE,
                &url,
                None,
                resolved_account_opt.as_deref(),
                CALENDAR_DOMAIN_KEY,
            )
            .await
        {
            Ok(_) => {
                let account_id = resolved_account_opt.as_deref().unwrap_or("primary");
                let observed_at = chrono::Utc::now().to_rfc3339();
                let (raw_event, assistant_event) = build_calendar_event_changed_event(
                    ctx.user_did,
                    &observed_at,
                    account_id,
                    "delete",
                    calendar_id,
                    event_id,
                    event_id,
                    None,
                    None,
                    0,
                );
                if let Err(e) = self
                    .persist_assistant_event(&raw_event, &assistant_event)
                    .await
                {
                    tracing::warn!(error = %e, event_id, "[calendar] failed to persist assistant event");
                }
                if let Some(action_id) = ledger_action_id.as_deref() {
                    self.best_effort_calendar_action_executed(
                        action_id,
                        crate::action_ledger::ActionKind::CalendarDelete,
                        calendar_id,
                        event_id,
                        "",
                        event_id,
                        "delete",
                    );
                }
                serde_json::json!({
                    "status": "ok",
                    "action": "delete_event",
                    "event_id": event_id,
                    "calendar_id": calendar_id,
                    "message": format!(
                        "Event {} deleted (irreversible). Attendees will receive a cancellation notice if they were invited.",
                        event_id
                    ),
                })
                .to_string()
            }
            Err(e) => {
                if let Some(action_id) = ledger_action_id.as_deref() {
                    self.best_effort_calendar_action_failed(action_id, "calendar_delete_event", &e);
                }
                sanitized_tool_error(
                    "calendar_delete_event",
                    "Google Calendar API error / 日历接口错误",
                    &e,
                    "api_error",
                    true,
                    Some("Verify event_id exists and the OAuth scope includes calendar.events."),
                )
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn best_effort_calendar_action_started(
        &self,
        action_kind: crate::action_ledger::ActionKind,
        action_id: &str,
        chat_id: Option<i64>,
        user_did: &str,
        calendar_id: &str,
        event_id: Option<&str>,
        summary: &str,
        operation: &str,
        args: &Value,
    ) -> Option<String> {
        let result = (|| -> anyhow::Result<String> {
            let conn = crate::encrypted_sqlite::open(self.memory.db_path())?;
            let store = crate::action_ledger::ActionLedgerStore::new(&conn);
            match store.create_proposed(&crate::action_ledger::NewAction {
                action_id: action_id.to_string(),
                action_kind,
                requested_by: Some(user_did.to_string()),
                chat_id,
                input_summary: Some(calendar_input_summary(
                    operation,
                    summary,
                    calendar_id,
                    event_id,
                )),
                user_visible_summary: Some(format!(
                    "Calendar event {} requested: {}",
                    operation,
                    truncate_str(summary, 120)
                )),
                idempotency_key: Some(action_id.to_string()),
                source_event_id: event_id.map(str::to_string),
                metadata_json: Some(
                    serde_json::json!({
                        "operation": operation,
                        "calendar_id": calendar_id,
                        "event_id": event_id,
                        "args_fingerprint": calendar_args_fingerprint(args),
                        "schema_version": 1,
                    })
                    .to_string(),
                ),
                ..Default::default()
            }) {
                Ok(_) => {}
                Err(e) => {
                    if store.get_by_action_id(action_id)?.is_none() {
                        return Err(e);
                    }
                }
            }
            store.transition(
                action_id,
                crate::action_ledger::ActionStatus::Executing,
                &crate::action_ledger::ActionUpdate {
                    tool_call_id: Some(format!("calendar_{operation}_event")),
                    metadata_json: Some(
                        serde_json::json!({
                            "operation": operation,
                            "calendar_id": calendar_id,
                            "event_id": event_id,
                            "status": "executing",
                            "schema_version": 1,
                        })
                        .to_string(),
                    ),
                    ..Default::default()
                },
            )?;
            Ok(action_id.to_string())
        })();

        match result {
            Ok(action_id) => Some(action_id),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    action_kind = action_kind.as_str(),
                    calendar_id,
                    event_id,
                    "failed to record Calendar action start in action ledger"
                );
                None
            }
        }
    }

    fn best_effort_calendar_action_executed(
        &self,
        action_id: &str,
        action_kind: crate::action_ledger::ActionKind,
        calendar_id: &str,
        event_id: &str,
        html_link: &str,
        summary: &str,
        operation: &str,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let conn = crate::encrypted_sqlite::open(self.memory.db_path())?;
            let store = crate::action_ledger::ActionLedgerStore::new(&conn);
            store.transition(
                action_id,
                crate::action_ledger::ActionStatus::Executed,
                &crate::action_ledger::ActionUpdate {
                    external_receipt: Some(format!(
                        "calendar:{calendar_id}:{event_id}:{operation}"
                    )),
                    external_event_id: Some(event_id.to_string()),
                    tool_call_id: Some(format!("calendar_{operation}_event")),
                    user_visible_summary: Some(format!(
                        "Calendar event {}: {}",
                        match action_kind {
                            crate::action_ledger::ActionKind::CalendarCreate => "created",
                            crate::action_ledger::ActionKind::CalendarUpdate => "updated",
                            crate::action_ledger::ActionKind::CalendarDelete => "deleted",
                            _ => operation,
                        },
                        truncate_str(summary, 120)
                    )),
                    metadata_json: Some(
                        serde_json::json!({
                            "operation": operation,
                            "calendar_id": calendar_id,
                            "event_id": event_id,
                            "html_link": html_link,
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
            tracing::warn!(
                error = %e,
                calendar_id,
                event_id,
                "failed to record Calendar action execution in action ledger"
            );
        }
    }

    fn best_effort_calendar_action_failed(
        &self,
        action_id: &str,
        tool_name: &str,
        error: &anyhow::Error,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let conn = crate::encrypted_sqlite::open(self.memory.db_path())?;
            let store = crate::action_ledger::ActionLedgerStore::new(&conn);
            store.transition(
                action_id,
                crate::action_ledger::ActionStatus::Failed,
                &crate::action_ledger::ActionUpdate {
                    tool_call_id: Some(tool_name.to_string()),
                    error_kind: Some("calendar_api_error".to_string()),
                    error_message: Some(truncate_str(&error.to_string(), 500).to_string()),
                    metadata_json: Some(
                        serde_json::json!({
                            "tool_name": tool_name,
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
            tracing::warn!(
                error = %e,
                action_id,
                "failed to record Calendar action failure in action ledger"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Free helpers (crate-visible so tests can exercise them directly)
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `Ok(())` if the attendee list is permitted under current policy,
/// else a structured tool-error JSON `Value` the LLM can reason about (and
/// the caller can `.to_string()` directly into the tool-result slot).
///
/// Self-only rule: when `calendar_invite_others_enabled` is `false` (default),
/// the only allowed attendee is the authenticated user's own email
/// (case-insensitive, trimmed). Empty attendee list is always allowed —
/// that's a self-implicit event.
///
/// **Why Err instead of silent strip?** Per
/// `.claude/memory/feedback_transparent_limits.md`, system-imposed limits
/// must be visible to the LLM; silent enforcement is forbidden. Silent strip
/// would let the LLM create the event without the intended invites and the
/// user would never know the attendees were dropped.
pub(crate) fn check_attendee_policy(
    settings: &RuntimeSettings,
    attendees: &[String],
    own_email: &str,
) -> std::result::Result<(), Value> {
    if attendees.is_empty() {
        return Ok(());
    }
    let invite_ok = settings
        .get("calendar_invite_others_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if invite_ok {
        return Ok(());
    }
    let ow = own_email.trim().to_ascii_lowercase();
    let offenders: Vec<String> = attendees
        .iter()
        .filter(|a| a.trim().to_ascii_lowercase() != ow)
        .cloned()
        .collect();
    if offenders.is_empty() {
        return Ok(());
    }
    Err(serde_json::json!({
        "status": "error",
        "error": format!(
            "Cannot invite others: `calendar_invite_others_enabled` is off (default). \
             Offending attendees: {}. User must explicitly enable the setting \
             (via update_setting, HITL-approved) before you can send invites.",
            offenders.join(", ")
        ),
        "error_kind": "invite_others_disabled",
        "retryable": false,
        "hint": "Ask the user: '要给这些联系人发日历邀请，需要打开 calendar_invite_others_enabled 设置。批准吗?' — on yes, call update_setting, wait for HITL approval, then retry.",
        "offending_attendees": offenders,
    }))
}

/// Pull the `attendees` array out of the tool args as `Vec<String>`. Returns
/// an empty vec when the field is absent or not an array. String trimming is
/// deferred to `validate_attendee_list` / `check_attendee_policy`.
fn extract_attendees(args: &Value) -> Vec<String> {
    args.get("attendees")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Validate each attendee string:
/// 1. CRLF-injection guard (`openrouter::rfc822_header_has_crlf`). Matches
///    the ingress-guard shape at `orchestrator_tools_knowledge.rs:~1030`.
///    On violation: `error_kind: "header_injection"`.
/// 2. Must `contains('@')` — basic "looks like an email" check. On failure:
///    `error_kind: "invalid_attendee"`.
///
/// Returns `Err(String)` containing the ready-to-return tool-result JSON on
/// the first violation, `Ok(())` when all entries pass.
fn validate_attendee_list(attendees: &[String]) -> std::result::Result<(), String> {
    for attendee in attendees {
        if crate::openrouter::rfc822_header_has_crlf(attendee) {
            return Err(serde_json::json!({
                "status": "error",
                "error": format!(
                    "Header-injection attempt: attendee '{}' contains CR/LF",
                    sanitize_for_error(attendee)
                ),
                "error_kind": "header_injection",
                "retryable": false,
                "hint": "Strip \\r and \\n from attendee emails before calling calendar tools."
            })
            .to_string());
        }
        if !attendee.contains('@') {
            return Err(serde_json::json!({
                "status": "error",
                "error": format!(
                    "Attendee '{}' is not a valid email (missing '@').",
                    attendee
                ),
                "error_kind": "invalid_attendee",
                "retryable": true,
                "hint": "Each attendee must be a syntactically-valid email address.",
            })
            .to_string());
        }
    }
    Ok(())
}

/// Tiny CRLF-stripping sanitizer for echoing the offending value back in an
/// error message without re-introducing the injection vector.
fn sanitize_for_error(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '\r' && *c != '\n')
        .take(80)
        .collect()
}

/// Build a Google Calendar `dateTime` object. Always populates `dateTime`;
/// adds `timeZone` only when the caller supplied one — Google defaults to
/// the calendar's primary time zone when absent.
fn build_datetime_obj(dt: &str, timezone: &str) -> Value {
    if timezone.is_empty() {
        serde_json::json!({"dateTime": dt})
    } else {
        serde_json::json!({"dateTime": dt, "timeZone": timezone})
    }
}

fn calendar_action_id(prefix: &str, calendar_id: &str, args: &Value) -> String {
    format!("{prefix}:{calendar_id}:{}", calendar_args_fingerprint(args))
}

fn calendar_args_fingerprint(args: &Value) -> String {
    let serialized = serde_json::to_string(args).unwrap_or_else(|_| args.to_string());
    blake3::hash(serialized.as_bytes()).to_hex()[..16].to_string()
}

fn calendar_input_summary(
    operation: &str,
    summary: &str,
    calendar_id: &str,
    event_id: Option<&str>,
) -> String {
    match event_id {
        Some(event_id) => format!(
            "{operation} calendar event '{}' on {calendar_id}/{event_id}",
            truncate_str(summary, 120)
        ),
        None => format!(
            "{operation} calendar event '{}' on {calendar_id}",
            truncate_str(summary, 120)
        ),
    }
}

/// Minimal percent-encoder for path segments (calendar_id, event_id). Covers
/// the characters Google documents as illegal in calendar / event IDs plus
/// the URL-unsafe classics. Kept inline (no `urlencoding` crate dependency)
/// because the valid input alphabet is narrow.
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let ok = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if ok {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use async_openai::types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestToolMessageContent,
    };

    use crate::memory::MemoryHandle;
    use crate::orchestrator::Orchestrator;
    use crate::settings::RuntimeSettings;
    use crate::test_tool_helpers::{CtxState, make_tool_call, test_orchestrator};

    fn s(v: &str) -> String {
        v.to_string()
    }

    fn file_backed_test_orchestrator() -> (tempfile::NamedTempFile, Orchestrator) {
        let db = tempfile::NamedTempFile::new().expect("temp db");
        let mem = MemoryHandle::spawn(db.path()).expect("file-backed DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);
        (db, orch)
    }

    fn last_tool_json(state: &CtxState) -> serde_json::Value {
        let last = state.messages.last().expect("tool message");
        let content = match last {
            ChatCompletionRequestMessage::Tool(t) => match &t.content {
                ChatCompletionRequestToolMessageContent::Text(text) => text.as_str(),
                other => panic!("unexpected tool content variant: {other:?}"),
            },
            other => panic!("expected tool message, got: {other:?}"),
        };
        serde_json::from_str(content).expect("tool output must be valid json")
    }

    // ── Routing ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn calendar_unknown_tool_passes_through() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let tc = make_tool_call("quantum_calendar", "{}");
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_calendar(&tc, "quantum_calendar", "{}", &mut state.ctx())
            .await
            .expect("should not error");
        assert_eq!(result, None, "unknown tool must pass through");
    }

    // ── check_attendee_policy — 6 combinations ──────────────────────────

    #[test]
    fn attendee_policy_empty_setting_off_ok() {
        let settings = RuntimeSettings::new();
        assert!(check_attendee_policy(&settings, &[], "me@example.com").is_ok());
    }

    #[test]
    fn attendee_policy_empty_setting_on_ok() {
        let settings = RuntimeSettings::new();
        settings
            .set("calendar_invite_others_enabled", Value::Bool(true))
            .expect("set");
        assert!(check_attendee_policy(&settings, &[], "me@example.com").is_ok());
    }

    #[test]
    fn attendee_policy_self_only_setting_off_ok_case_insensitive() {
        let settings = RuntimeSettings::new();
        // Upper-case + trailing whitespace on both sides — trim + lowercase
        // comparison should still pass.
        assert!(
            check_attendee_policy(&settings, &[s("  Me@Example.COM  ")], "me@example.com").is_ok()
        );
    }

    #[test]
    fn attendee_policy_self_plus_other_setting_off_err_names_offender() {
        let settings = RuntimeSettings::new();
        let err = check_attendee_policy(
            &settings,
            &[s("me@example.com"), s("friend@other.com")],
            "me@example.com",
        )
        .expect_err("should error");
        assert_eq!(err["error_kind"], "invite_others_disabled");
        assert_eq!(err["retryable"], false);
        let offenders = err["offending_attendees"].as_array().expect("array");
        assert_eq!(offenders.len(), 1);
        assert_eq!(offenders[0], "friend@other.com");
    }

    #[test]
    fn attendee_policy_other_only_setting_off_err() {
        let settings = RuntimeSettings::new();
        let err = check_attendee_policy(&settings, &[s("friend@other.com")], "me@example.com")
            .expect_err("should error");
        assert_eq!(err["error_kind"], "invite_others_disabled");
        let offenders = err["offending_attendees"].as_array().expect("array");
        assert_eq!(offenders.len(), 1);
    }

    #[test]
    fn attendee_policy_self_plus_other_setting_on_ok() {
        let settings = RuntimeSettings::new();
        settings
            .set("calendar_invite_others_enabled", Value::Bool(true))
            .expect("set");
        assert!(
            check_attendee_policy(
                &settings,
                &[s("me@example.com"), s("friend@other.com")],
                "me@example.com"
            )
            .is_ok()
        );
    }

    // ── validate_attendee_list — CRLF + invalid-email ────────────────────

    #[test]
    fn validate_attendee_rejects_crlf() {
        let bad = [s("victim@example.com\r\nBcc: attacker@evil.com")];
        let err = validate_attendee_list(&bad).expect_err("should reject");
        let parsed: Value = serde_json::from_str(&err).unwrap();
        assert_eq!(parsed["error_kind"], "header_injection");
        assert_eq!(parsed["retryable"], false);
    }

    #[test]
    fn validate_attendee_rejects_lone_lf() {
        let bad = [s("victim@example.com\nMalicious: x")];
        let err = validate_attendee_list(&bad).expect_err("should reject");
        let parsed: Value = serde_json::from_str(&err).unwrap();
        assert_eq!(parsed["error_kind"], "header_injection");
    }

    #[test]
    fn validate_attendee_rejects_missing_at_sign() {
        let bad = [s("not-an-email")];
        let err = validate_attendee_list(&bad).expect_err("should reject");
        let parsed: Value = serde_json::from_str(&err).unwrap();
        assert_eq!(parsed["error_kind"], "invalid_attendee");
        assert_eq!(parsed["retryable"], true);
    }

    #[test]
    fn validate_attendee_accepts_valid_list() {
        let good = [s("a@x.com"), s("b@y.org")];
        assert!(validate_attendee_list(&good).is_ok());
    }

    // ── Dispatch error paths (no Google configured) ───────────────────────

    #[tokio::test]
    async fn calendar_create_event_missing_fields_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"summary":"hi","start":"","end":""}"#;
        let tc = make_tool_call("calendar_create_event", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_calendar(&tc, "calendar_create_event", args, &mut state.ctx())
            .await
            .expect("should not error at dispatch level");
        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("start and end are required"),
            "expected RFC3339 validation, got: {content}"
        );
    }

    #[tokio::test]
    async fn calendar_update_event_missing_event_id_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"event_id":""}"#;
        let tc = make_tool_call("calendar_update_event", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_calendar(&tc, "calendar_update_event", args, &mut state.ctx())
            .await
            .expect("should not error at dispatch level");
        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("event_id required"));
    }

    #[tokio::test]
    async fn calendar_update_event_no_fields_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"event_id":"abc123"}"#;
        let tc = make_tool_call("calendar_update_event", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_calendar(&tc, "calendar_update_event", args, &mut state.ctx())
            .await
            .expect("should not error at dispatch level");
        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("no fields to update"));
    }

    #[tokio::test]
    async fn calendar_delete_event_missing_event_id_returns_error() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = r#"{"calendar_id":"primary"}"#;
        let tc = make_tool_call("calendar_delete_event", args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_calendar(&tc, "calendar_delete_event", args, &mut state.ctx())
            .await
            .expect("should not error at dispatch level");
        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("event_id required"));
    }

    #[tokio::test]
    async fn calendar_create_event_api_failure_records_action_ledger() {
        let (db, orch) = file_backed_test_orchestrator();

        let args = serde_json::json!({
            "summary": "Planning session",
            "start": "2026-05-01T09:00:00+08:00",
            "end": "2026-05-01T10:00:00+08:00",
            "calendar_id": "primary"
        })
        .to_string();
        let args_value: Value = serde_json::from_str(&args).expect("args json");
        let expected_action_id = calendar_action_id("calendar-create", "primary", &args_value);
        let tc = make_tool_call("calendar_create_event", &args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_calendar(&tc, "calendar_create_event", &args, &mut state.ctx())
            .await
            .expect("should not error at dispatch level");

        assert_eq!(result, Some(true));
        let output = last_tool_json(&state);
        assert_eq!(output["status"], "error");

        let conn = crate::encrypted_sqlite::open(db.path()).expect("open action ledger DB");
        let entry = crate::action_ledger::ActionLedgerStore::new(&conn)
            .get_by_action_id(&expected_action_id)
            .expect("query action")
            .expect("calendar create action");
        assert_eq!(
            entry.action_kind,
            crate::action_ledger::ActionKind::CalendarCreate
        );
        assert_eq!(entry.status, crate::action_ledger::ActionStatus::Failed);
        assert_eq!(entry.requested_by.as_deref(), Some("did:test:user"));
        assert_eq!(entry.chat_id, Some(12345));
        assert_eq!(entry.tool_call_id.as_deref(), Some("calendar_create_event"));
        assert_eq!(
            entry.idempotency_key.as_deref(),
            Some(expected_action_id.as_str())
        );
        assert_eq!(entry.error_kind.as_deref(), Some("calendar_api_error"));
        assert!(entry.failed_at.is_some(), "failed_at should be set");
        let metadata: Value =
            serde_json::from_str(entry.metadata_json.as_deref().expect("metadata"))
                .expect("metadata json");
        assert_eq!(metadata["status"], "failed");
        assert_eq!(metadata["schema_version"], 1);
    }

    #[tokio::test]
    async fn calendar_update_event_api_failure_records_action_ledger() {
        let (db, orch) = file_backed_test_orchestrator();

        let args = serde_json::json!({
            "event_id": "event-123",
            "calendar_id": "primary",
            "summary": "Updated planning session"
        })
        .to_string();
        let args_value: Value = serde_json::from_str(&args).expect("args json");
        let expected_action_id = calendar_action_id("calendar-update", "primary", &args_value);
        let tc = make_tool_call("calendar_update_event", &args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_calendar(&tc, "calendar_update_event", &args, &mut state.ctx())
            .await
            .expect("should not error at dispatch level");

        assert_eq!(result, Some(true));
        let output = last_tool_json(&state);
        assert_eq!(output["status"], "error");

        let conn = crate::encrypted_sqlite::open(db.path()).expect("open action ledger DB");
        let entry = crate::action_ledger::ActionLedgerStore::new(&conn)
            .get_by_action_id(&expected_action_id)
            .expect("query action")
            .expect("calendar update action");
        assert_eq!(
            entry.action_kind,
            crate::action_ledger::ActionKind::CalendarUpdate
        );
        assert_eq!(entry.status, crate::action_ledger::ActionStatus::Failed);
        assert_eq!(entry.source_event_id.as_deref(), Some("event-123"));
        assert_eq!(entry.tool_call_id.as_deref(), Some("calendar_update_event"));
        assert_eq!(
            entry.idempotency_key.as_deref(),
            Some(expected_action_id.as_str())
        );
        assert_eq!(entry.error_kind.as_deref(), Some("calendar_api_error"));
        assert!(entry.failed_at.is_some(), "failed_at should be set");
    }

    #[tokio::test]
    async fn calendar_delete_event_api_failure_records_action_ledger() {
        let (db, orch) = file_backed_test_orchestrator();

        let args = serde_json::json!({
            "event_id": "event-456",
            "calendar_id": "primary"
        })
        .to_string();
        let args_value: Value = serde_json::from_str(&args).expect("args json");
        let expected_action_id = calendar_action_id("calendar-delete", "primary", &args_value);
        let tc = make_tool_call("calendar_delete_event", &args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_calendar(&tc, "calendar_delete_event", &args, &mut state.ctx())
            .await
            .expect("should not error at dispatch level");

        assert_eq!(result, Some(true));
        let output = last_tool_json(&state);
        assert_eq!(output["status"], "error");

        let conn = crate::encrypted_sqlite::open(db.path()).expect("open action ledger DB");
        let entry = crate::action_ledger::ActionLedgerStore::new(&conn)
            .get_by_action_id(&expected_action_id)
            .expect("query action")
            .expect("calendar delete action");
        assert_eq!(
            entry.action_kind,
            crate::action_ledger::ActionKind::CalendarDelete
        );
        assert_eq!(entry.status, crate::action_ledger::ActionStatus::Failed);
        assert_eq!(entry.source_event_id.as_deref(), Some("event-456"));
        assert_eq!(entry.tool_call_id.as_deref(), Some("calendar_delete_event"));
        assert_eq!(
            entry.idempotency_key.as_deref(),
            Some(expected_action_id.as_str())
        );
        assert_eq!(entry.error_kind.as_deref(), Some("calendar_api_error"));
        assert!(entry.failed_at.is_some(), "failed_at should be set");
    }

    #[tokio::test]
    async fn calendar_create_event_rejects_crlf_in_attendee() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = serde_json::json!({
            "summary": "Test",
            "start": "2026-05-01T09:00:00+08:00",
            "end": "2026-05-01T10:00:00+08:00",
            "attendees": ["victim@example.com\r\nBcc: attacker@evil.com"],
        })
        .to_string();
        let tc = make_tool_call("calendar_create_event", &args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_calendar(&tc, "calendar_create_event", &args, &mut state.ctx())
            .await
            .expect("should not error at dispatch level");
        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("header_injection"),
            "expected header_injection, got: {content}"
        );
    }

    #[tokio::test]
    async fn calendar_create_event_rejects_invalid_attendee() {
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = serde_json::json!({
            "summary": "Test",
            "start": "2026-05-01T09:00:00+08:00",
            "end": "2026-05-01T10:00:00+08:00",
            "attendees": ["not-an-email"],
        })
        .to_string();
        let tc = make_tool_call("calendar_create_event", &args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_calendar(&tc, "calendar_create_event", &args, &mut state.ctx())
            .await
            .expect("should not error at dispatch level");
        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(content.contains("invalid_attendee"));
    }

    #[tokio::test]
    async fn calendar_create_event_identity_unresolved_when_auth_missing() {
        // CTO round-1 refinement: when google_self_address
        // returns empty (no live Google auth) AND the LLM supplied
        // attendees, we now return `identity_unresolved` instead of
        // `invite_others_disabled`. This surfaces the true root cause
        // (can't verify user identity) rather than misleading the LLM
        // into thinking the user needs to flip the invite-others toggle.
        //
        // The actual invite_others_disabled branch is covered directly at
        // `check_attendee_policy_self_plus_other_setting_off` unit test
        // (bypasses the self-address fetch by calling the helper).
        let mem = MemoryHandle::spawn_in_memory().expect("in-memory DB");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = test_orchestrator(mem, settings);

        let args = serde_json::json!({
            "summary": "Test event",
            "start": "2026-05-01T09:00:00+08:00",
            "end": "2026-05-01T10:00:00+08:00",
            "attendees": ["friend@other.com"],
        })
        .to_string();
        let tc = make_tool_call("calendar_create_event", &args);
        let mut state = CtxState::new();
        let result = orch
            .handle_tool_calendar(&tc, "calendar_create_event", &args, &mut state.ctx())
            .await
            .expect("should not error at dispatch level");
        assert_eq!(result, Some(true));
        let content = format!("{:?}", state.messages.last().unwrap());
        assert!(
            content.contains("identity_unresolved"),
            "expected identity_unresolved (auth missing), got: {content}"
        );
        assert!(
            !content.contains("invite_others_disabled"),
            "must NOT report invite_others_disabled — the real cause is unresolved identity, not the toggle: {content}"
        );
    }

    // ── urlencoding_encode ──────────────────────────────────────────────

    #[test]
    fn urlencoding_encode_passes_through_safe_chars() {
        assert_eq!(urlencoding_encode("primary"), "primary");
        assert_eq!(urlencoding_encode("abc_123"), "abc_123");
        assert_eq!(urlencoding_encode("foo-bar.baz~qux"), "foo-bar.baz~qux");
    }

    #[test]
    fn urlencoding_encode_percent_escapes_slash_and_space() {
        assert_eq!(urlencoding_encode("a/b"), "a%2Fb");
        assert_eq!(urlencoding_encode("a b"), "a%20b");
        assert_eq!(urlencoding_encode("@:#"), "%40%3A%23");
    }
}
