//! Single source of truth for Telegram callback-button families.
//!
//! Historically the 15 callback prefix families were duplicated across
//! three sites in `telegram.rs`:
//!
//!   1. The `CALLBACK_ROUTES` table inside the callback_query handler
//!      (maps `prefix_action:` → `(action, cmd_type)`).
//!   2. The `cb_prefix` if/else chain inside `spawn_progress_listener`
//!      (maps `approval_id` prefix → button callback prefix).
//!   3. The whitelist inside `extract_approval_id` (approval_id prefix filter).
//!
//! A new approval family could land in routes (1) without anyone remembering
//! to update (2) and (3), resulting in silent "button never shows" bugs.
//! This module centralises the families so all three views derive from
//! the same data.

use std::borrow::Cow;

/// A single action within a callback family (e.g. `approve` in `sysop_approve:`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct CallbackAction {
    /// The suffix used in the raw `callback_data` string (e.g. `"approve"`
    /// for `sysop_approve:`).
    pub callback_suffix: &'static str,
    /// The logical action name returned by `match_callback_data` and
    /// consumed by the dispatcher (e.g. `"approve"`, `"cancel"`).
    /// Usually identical to `callback_suffix` but kept separate to
    /// decouple wire format from handler logic.
    pub action_name: &'static str,
}

/// A callback family shares a single `cmd_type` and (optionally) an
/// `approval_id_prefix`. For example the `gate_*` family has
/// `callback_prefix = "gate"`, `cmd_type = "gate"`,
/// `approval_id_prefix = Some("gate-")` and six actions
/// (`once`, `intent`, `domain`, `always`, `reject`, `clarify`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct CallbackFamily {
    /// Prefix in raw callback_data (e.g. `"sysop"` for `"sysop_approve:..."`).
    pub callback_prefix: &'static str,
    /// cmd_type returned by `match_callback_data` to route to a handler.
    /// Sometimes differs from `callback_prefix` — e.g. `cb_*` → `"circuit_breaker"`.
    pub cmd_type: &'static str,
    /// Approval-id prefix used in the HITL / `spawn_progress_listener` flow
    /// (e.g. `"menu-"` for `approval_id = "menu-abc123"`).
    /// `None` for families whose approvals bypass `spawn_progress_listener`
    /// (circuit_breaker, notification).
    pub approval_id_prefix: Option<&'static str>,
    pub actions: &'static [CallbackAction],
}

const APPROVE_REJECT: &[CallbackAction] = &[
    CallbackAction {
        callback_suffix: "approve",
        action_name: "approve",
    },
    CallbackAction {
        callback_suffix: "reject",
        action_name: "reject",
    },
];

/// Canonical callback family registry. **Adding a new approval family
/// requires only one entry here** — all derived views stay consistent.
pub(crate) const CALLBACK_FAMILIES: &[CallbackFamily] = &[
    CallbackFamily {
        callback_prefix: "sysop",
        cmd_type: "sysop",
        approval_id_prefix: Some("sysop-"),
        actions: APPROVE_REJECT,
    },
    CallbackFamily {
        callback_prefix: "config",
        cmd_type: "config",
        approval_id_prefix: Some("config-"),
        actions: APPROVE_REJECT,
    },
    CallbackFamily {
        callback_prefix: "menu",
        cmd_type: "menu",
        approval_id_prefix: Some("menu-"),
        actions: APPROVE_REJECT,
    },
    CallbackFamily {
        callback_prefix: "mcp",
        cmd_type: "mcp",
        approval_id_prefix: Some("mcp-"),
        actions: APPROVE_REJECT,
    },
    CallbackFamily {
        callback_prefix: "revoke",
        cmd_type: "revoke",
        approval_id_prefix: Some("revoke-google-"),
        actions: APPROVE_REJECT,
    },
    CallbackFamily {
        callback_prefix: "cb",
        cmd_type: "circuit_breaker",
        approval_id_prefix: None,
        actions: &[
            CallbackAction {
                callback_suffix: "deactivate",
                action_name: "deactivate",
            },
            CallbackAction {
                callback_suffix: "retry",
                action_name: "retry",
            },
        ],
    },
    CallbackFamily {
        callback_prefix: "gate",
        cmd_type: "gate",
        approval_id_prefix: Some("gate-"),
        actions: &[
            CallbackAction {
                callback_suffix: "once",
                action_name: "once",
            },
            CallbackAction {
                callback_suffix: "intent",
                action_name: "intent",
            },
            CallbackAction {
                callback_suffix: "domain",
                action_name: "domain",
            },
            CallbackAction {
                callback_suffix: "always",
                action_name: "always",
            },
            CallbackAction {
                callback_suffix: "reject",
                action_name: "reject",
            },
            CallbackAction {
                callback_suffix: "clarify",
                action_name: "clarify",
            },
        ],
    },
    CallbackFamily {
        callback_prefix: "notif",
        cmd_type: "notification",
        approval_id_prefix: None,
        actions: &[
            CallbackAction {
                callback_suffix: "helpful",
                action_name: "helpful",
            },
            CallbackAction {
                callback_suffix: "unhelpful",
                action_name: "unhelpful",
            },
        ],
    },
    CallbackFamily {
        callback_prefix: "rem",
        cmd_type: "reminder_action",
        approval_id_prefix: None,
        actions: &[
            CallbackAction {
                callback_suffix: "done",
                action_name: "done",
            },
            CallbackAction {
                callback_suffix: "snooze",
                action_name: "snooze",
            },
            CallbackAction {
                callback_suffix: "snz10",
                action_name: "snz10",
            },
            CallbackAction {
                callback_suffix: "snz60",
                action_name: "snz60",
            },
            CallbackAction {
                callback_suffix: "snzeve",
                action_name: "snzeve",
            },
            CallbackAction {
                callback_suffix: "snztom",
                action_name: "snztom",
            },
            CallbackAction {
                callback_suffix: "snzmon",
                action_name: "snzmon",
            },
            CallbackAction {
                callback_suffix: "delete",
                action_name: "delete",
            },
            CallbackAction {
                callback_suffix: "confirmdel",
                action_name: "confirmdel",
            },
            CallbackAction {
                callback_suffix: "cancel",
                action_name: "cancel",
            },
            CallbackAction {
                callback_suffix: "details",
                action_name: "details",
            },
        ],
    },
];

/// Attempt to parse a raw `callback_data` string into
/// `(action_name, cmd_type, id_suffix)`.
///
/// `callback_data` format: `<callback_prefix>_<callback_suffix>:<id>` where
/// `<id>` is opaque (typically an approval_id or entity key). Returns `None`
/// if no family matches.
pub(crate) fn match_callback_data(
    data: &str,
) -> Option<(&'static str, &'static str, Cow<'_, str>)> {
    for family in CALLBACK_FAMILIES {
        let Some(rest) = data.strip_prefix(family.callback_prefix) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix('_') else {
            continue;
        };
        for action in family.actions {
            if let Some(rest2) = rest.strip_prefix(action.callback_suffix)
                && let Some(id) = rest2.strip_prefix(':')
            {
                return Some((action.action_name, family.cmd_type, Cow::Borrowed(id)));
            }
        }
    }
    None
}

/// Exhaustive `(action_name, cmd_type)` matrix derived from the canonical
/// [`CALLBACK_FAMILIES`] registry — every pair the Telegram callback parse
/// table can hand to the dispatcher. Exposed (doc-hidden) for the S1
/// routing-linkage coverage sensor (`tests/hitl_routing_linkage.rs`), which
/// drives each pair through the REAL `process_message` dispatch path and
/// fails CI when a cmd_type arm is unwired.
#[doc(hidden)]
pub fn callback_route_matrix() -> Vec<(&'static str, &'static str)> {
    CALLBACK_FAMILIES
        .iter()
        .flat_map(|family| {
            family
                .actions
                .iter()
                .map(|action| (action.action_name, family.cmd_type))
        })
        .collect()
}

/// Approval_id prefixes for families that deliver HITL approvals via
/// `spawn_progress_listener`. Used by `extract_approval_id` to whitelist.
pub(crate) fn approval_id_prefixes() -> impl Iterator<Item = &'static str> {
    CALLBACK_FAMILIES
        .iter()
        .filter_map(|f| f.approval_id_prefix)
}

/// Given a HITL approval_id (e.g. `"gate-abc"`), return the callback button
/// prefix used to build approve/reject buttons (e.g. `"gate"`).
/// Returns `None` if no family claims this approval_id — callers should
/// fall back to `"sysop"` for historical reasons.
pub(crate) fn cb_prefix_for_approval_id(approval_id: &str) -> Option<&'static str> {
    CALLBACK_FAMILIES.iter().find_map(|f| {
        let prefix = f.approval_id_prefix?;
        if approval_id.starts_with(prefix) {
            Some(f.callback_prefix)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: every route that existed as a hardcoded tuple in the
    /// pre-refactor `CALLBACK_ROUTES` must be parseable from `match_callback_data`.
    /// The reverse direction (no surprise routes) is trivially ensured
    /// because the module is the source.
    #[test]
    fn match_callback_data_recovers_all_historical_routes() {
        // (input_data, expected_action, expected_cmd_type, expected_id)
        let cases: &[(&str, &str, &str, &str)] = &[
            ("sysop_approve:123", "approve", "sysop", "123"),
            ("sysop_reject:abc", "reject", "sysop", "abc"),
            ("config_approve:x", "approve", "config", "x"),
            ("config_reject:y", "reject", "config", "y"),
            ("menu_approve:m1", "approve", "menu", "m1"),
            ("menu_reject:m2", "reject", "menu", "m2"),
            ("mcp_approve:p1", "approve", "mcp", "p1"),
            ("mcp_reject:p2", "reject", "mcp", "p2"),
            ("revoke_approve:r", "approve", "revoke", "r"),
            ("revoke_reject:r", "reject", "revoke", "r"),
            ("cb_deactivate:z", "deactivate", "circuit_breaker", "z"),
            ("cb_retry:z", "retry", "circuit_breaker", "z"),
            ("gate_once:g", "once", "gate", "g"),
            ("gate_intent:g", "intent", "gate", "g"),
            ("gate_domain:g", "domain", "gate", "g"),
            ("gate_always:g", "always", "gate", "g"),
            ("gate_reject:g", "reject", "gate", "g"),
            ("gate_clarify:g", "clarify", "gate", "g"),
            ("notif_helpful:n", "helpful", "notification", "n"),
            ("notif_unhelpful:n", "unhelpful", "notification", "n"),
            ("rem_done:42", "done", "reminder_action", "42"),
            ("rem_snooze:42", "snooze", "reminder_action", "42"),
            ("rem_snz10:42", "snz10", "reminder_action", "42"),
            ("rem_snz60:42", "snz60", "reminder_action", "42"),
            ("rem_snzeve:42", "snzeve", "reminder_action", "42"),
            ("rem_snztom:42", "snztom", "reminder_action", "42"),
            ("rem_snzmon:42", "snzmon", "reminder_action", "42"),
            ("rem_delete:42", "delete", "reminder_action", "42"),
            ("rem_confirmdel:42", "confirmdel", "reminder_action", "42"),
            ("rem_cancel:42", "cancel", "reminder_action", "42"),
            ("rem_details:42", "details", "reminder_action", "42"),
        ];
        assert_eq!(cases.len(), 31, "route count changed — update registry");
        for (input, exp_action, exp_cmd, exp_id) in cases {
            let (act, cmd, id) =
                match_callback_data(input).unwrap_or_else(|| panic!("no match for {input:?}"));
            assert_eq!(act, *exp_action, "action mismatch for {input}");
            assert_eq!(cmd, *exp_cmd, "cmd_type mismatch for {input}");
            assert_eq!(id.as_ref(), *exp_id, "id mismatch for {input}");
        }
    }

    #[test]
    fn match_callback_data_rejects_unknown_and_malformed() {
        assert!(match_callback_data("").is_none());
        assert!(match_callback_data("bogus:x").is_none());
        assert!(match_callback_data("sysop_approve").is_none()); // missing colon
        assert!(match_callback_data("sysop_nonsense:x").is_none());
    }

    #[test]
    fn cb_prefix_covers_all_historical_approval_id_prefixes() {
        // These are the exact prefixes the pre-refactor `extract_approval_id`
        // whitelist + cb_prefix if/else chain recognised. The derived
        // `cb_prefix_for_approval_id` must recover the same button prefix.
        let cases: &[(&str, &str)] = &[
            ("sysop-abc", "sysop"),
            ("config-abc", "config"),
            ("menu-abc", "menu"),
            ("mcp-abc", "mcp"),
            ("revoke-google-abc", "revoke"),
            ("gate-abc", "gate"),
        ];
        for (approval_id, expected) in cases {
            let got = cb_prefix_for_approval_id(approval_id)
                .unwrap_or_else(|| panic!("no cb_prefix for {approval_id:?}"));
            assert_eq!(got, *expected, "cb_prefix mismatch for {approval_id}");
        }
    }

    #[test]
    fn approval_id_prefixes_are_unique_and_non_empty() {
        let prefixes: Vec<&str> = approval_id_prefixes().collect();
        let mut sorted = prefixes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            prefixes.len(),
            sorted.len(),
            "duplicate approval_id_prefix in CALLBACK_FAMILIES"
        );
        for p in &prefixes {
            assert!(!p.is_empty(), "empty approval_id_prefix");
            assert!(
                p.ends_with('-'),
                "approval_id_prefix {p} should end with '-'"
            );
        }
    }
}
