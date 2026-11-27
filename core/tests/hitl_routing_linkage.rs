//! Sensor 3 — HITL routing-linkage exhaustive test.
//!
//! The Telegram callback layer parses every button tap through the canonical
//! `callback_routes::CALLBACK_FAMILIES` registry into an `(action, cmd_type,
//! id)` triple and (for most families) dispatches the command string
//! `"{action} {cmd_type} {id}"` through `process_message`, where
//! `intercept_hitl_command` must claim it with a kind-specific arm.
//!
//! Failure class this kills (4-way audit 2026-07-08): a new callback family
//! lands in the registry (buttons render, taps parse) but its dispatcher arm
//! is never wired — the tap silently falls through to the LLM router or is
//! swallowed by the GENERIC draft fallback (`approve <id>` / `reject <id>`),
//! and no test notices. Here every `(action, cmd_type)` pair the registry can
//! produce is driven through the REAL `process_message` path with a
//! nonexistent approval id; the arm must claim it deterministically:
//!
//! - zero LLM round-trips (the mock LLM's chat counter must not move), and
//! - the reply must NOT be the generic draft-fallback not-found message
//!   (each arm's kind-specific not-found/parse reply proves the arm exists).
//!
//! Families that the Telegram layer routes BEFORE building a process_message
//! command are exempted explicitly below, with the file/flow that handles
//! them — NOT because they are untestable, but because their dispatch does
//! not travel through `intercept_hitl_command`.

mod mock_helpers;

use std::sync::Arc;

use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::settings::RuntimeSettings;

use mock_helpers::*;

/// `(action, cmd_type)` pairs that the Telegram callback handler dispatches
/// directly (never as a `"{action} {cmd_type} {id}"` command through
/// `process_message`). Keep in sync with the early-return branches in
/// `telegram.rs`'s callback_query handler.
const TELEGRAM_LAYER_ROUTED: &[(&str, &str, &str)] = &[
    // telegram.rs: `if cmd_type == "notification"` — direct DB feedback write.
    ("helpful", "notification", "direct DB write in telegram.rs"),
    (
        "unhelpful",
        "notification",
        "direct DB write in telegram.rs",
    ),
    // telegram.rs: `if cmd_type == "reminder_action"` — deterministic
    // handle_reminder_action_callback, no LLM command string.
    ("done", "reminder_action", "handle_reminder_action_callback"),
    (
        "snooze",
        "reminder_action",
        "handle_reminder_action_callback",
    ),
    (
        "snz10",
        "reminder_action",
        "handle_reminder_action_callback",
    ),
    (
        "snz60",
        "reminder_action",
        "handle_reminder_action_callback",
    ),
    (
        "snzeve",
        "reminder_action",
        "handle_reminder_action_callback",
    ),
    (
        "snztom",
        "reminder_action",
        "handle_reminder_action_callback",
    ),
    (
        "snzmon",
        "reminder_action",
        "handle_reminder_action_callback",
    ),
    (
        "delete",
        "reminder_action",
        "handle_reminder_action_callback",
    ),
    (
        "confirmdel",
        "reminder_action",
        "handle_reminder_action_callback",
    ),
    (
        "cancel",
        "reminder_action",
        "handle_reminder_action_callback",
    ),
    (
        "details",
        "reminder_action",
        "handle_reminder_action_callback",
    ),
    // telegram.rs: circuit_breaker actions are TRANSLATED into
    // natural-language LLM commands ("Deactivate skill '…'"), intentionally
    // LLM-routed rather than intercept-arm routed.
    (
        "deactivate",
        "circuit_breaker",
        "translated to an LLM command in telegram.rs",
    ),
    (
        "retry",
        "circuit_breaker",
        "translated to an LLM command in telegram.rs",
    ),
];

fn is_telegram_layer_routed(action: &str, cmd_type: &str) -> bool {
    TELEGRAM_LAYER_ROUTED
        .iter()
        .any(|(a, c, _)| *a == action && *c == cmd_type)
}

#[tokio::test]
async fn every_callback_route_reaches_a_kind_specific_dispatch_arm() {
    // Empty response queue: any command that leaks past the HITL intercept
    // consumes a mock LLM completion, incrementing chat_calls — the primary
    // "this route is unwired" signal.
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let locale = settings.display_language();
    // No Wasm skills: the HITL intercept path never touches the sandbox, and
    // an empty skill list keeps this sensor runnable in fresh worktrees where
    // the compiled_skills/ artifacts have not been built.
    let orch = build_orchestrator_with_skills(addr, memory, settings, &[]);

    let matrix = aegis_runtime_core::callback_route_matrix();
    // Registry sanity: mail-client families were removed on 2026-09-07,
    // leaving 31 routes. If this shrinks below the floor the accessor (or
    // registry) has rotted and the loop below would pass vacuously.
    assert!(
        matrix.len() >= 30,
        "callback_route_matrix returned only {} routes",
        matrix.len()
    );

    // Exemption hygiene: every exempted pair must still exist in the registry
    // (stale exemptions must be pruned when a family is removed/renamed).
    for (action, cmd_type, reason) in TELEGRAM_LAYER_ROUTED {
        assert!(
            matrix.iter().any(|(a, c)| a == action && c == cmd_type),
            "stale TELEGRAM_LAYER_ROUTED exemption ({action}, {cmd_type}) — {reason}; \
             the pair no longer exists in CALLBACK_FAMILIES, remove it"
        );
    }

    let approval_id = "nonexistent-approval-id-s1-sensor";
    let mut failures: Vec<String> = Vec::new();

    for &(action, cmd_type) in &matrix {
        if is_telegram_layer_routed(action, cmd_type) {
            continue;
        }
        let command = format!("{action} {cmd_type} {approval_id}");
        let calls_before = state.chat_calls();
        let result = orch
            .process_message("did:test:user1", &command, Some(100))
            .await;
        let calls_after = state.chat_calls();

        if calls_after != calls_before {
            failures.push(format!(
                "({action}, {cmd_type}): `{command}` fell through to the LLM \
                 router — no intercept_hitl_command arm claimed it"
            ));
            continue;
        }

        // A deterministic `Err` still proves an arm claimed the command (the
        // zero-LLM-round assertion above already held), so only `Ok` replies
        // need the fallback inspection below.
        if let Ok(res) = result {
            if res.rounds_used != 0 {
                failures.push(format!(
                    "({action}, {cmd_type}): rounds_used = {} (expected a \
                     deterministic 0-round HITL reply)",
                    res.rounds_used
                ));
                continue;
            }
            if res.reply.trim().is_empty() {
                failures.push(format!(
                    "({action}, {cmd_type}): empty reply from the dispatch arm"
                ));
                continue;
            }
            // The GENERIC fallback arms (`approve <id>` / `reject <id>` →
            // draft handlers) swallow ANY "approve …"/"reject …" command,
            // so an unwired approve/reject family would still return a
            // 0-round reply — but it would be the DRAFT not-found message
            // with "{cmd_type} {id}" glued in as the supposed draft id.
            // That reply proves the kind-specific arm is MISSING.
            let swallowed_id = format!("{cmd_type} {approval_id}");
            let generic_fallback = match action {
                "approve" => Some(aegis_runtime_core::i18n::tf(
                    "hitl.draft_not_found",
                    &locale,
                    &swallowed_id,
                )),
                "reject" => Some(aegis_runtime_core::i18n::tf(
                    "hitl.draft.not_found_delete",
                    &locale,
                    &swallowed_id,
                )),
                _ => None,
            };
            if let Some(fallback) = generic_fallback
                && res.reply == fallback
            {
                failures.push(format!(
                    "({action}, {cmd_type}): swallowed by the GENERIC draft \
                     fallback arm — add a kind-specific `{action} {cmd_type} ` \
                     arm to intercept_hitl_command"
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "Unwired HITL callback route(s) — buttons parse but nothing \
         kind-specific handles the tap:\n  {}",
        failures.join("\n  ")
    );
}
