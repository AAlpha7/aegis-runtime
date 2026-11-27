//! Unified tool approval gate.
//!
//! Every tool call passes through [`ToolGate::evaluate`] which returns a
//! [`GateVerdict`] based on the tool's [`ToolPolicy`].  This replaces scattered
//! ad-hoc HITL checks with a single, declarative permission system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Domain-scoped approvals expire after this many minutes.
const DOMAIN_APPROVAL_TTL_MINUTES: i64 = 30;

// ---------------------------------------------------------------------------
// Approval scope types (granular approval framework)
// ---------------------------------------------------------------------------

/// Scope of a user's approval decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ApprovalScope {
    /// Approve only this specific tool call (one-shot, not stored).
    Once,
    /// Approve this tool for the lifetime of a specific intent/callback group.
    /// Stored with the intent's group_key; auto-expires when intent is removed.
    Intent { group_key: String },
    /// Approve this tool for a specific URL domain (e.g. "maps.google.com").
    /// Only meaningful for web-related tools.
    Domain { host: String },
    /// Permanently approve this tool (stored in settings, survives restart).
    Always,
}

/// A stored approval record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredApproval {
    pub tool_name: String,
    pub scope: ApprovalScope,
    pub approved_at: String, // ISO 8601
    pub approved_by: String, // user_did
}

// ---------------------------------------------------------------------------
// Argument-level patterns (parameter-level validation)
// ---------------------------------------------------------------------------

/// A pattern that can match against tool arguments to allow fine-grained
/// approval bypasses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ArgPattern {
    /// Allow specific values of the "action" field without approval.
    /// Matches if args.action is one of the listed values (read-only operations).
    /// A missing / null / empty `action` never matches: tools whose bare call
    /// is not read-only (setup_google, sysadmin_ops) rely on that to send `{}`
    /// to approval.
    ActionWhitelist(Vec<String>),
    /// [`ArgPattern::ActionWhitelist`] for tools whose executor reads a
    /// missing `action` as a fixed read-only default (check_api_usage →
    /// "summary"). A missing / null / empty-string `action` is evaluated as
    /// `default_action`, so the bare call `{}` is allowed exactly when the
    /// default is itself whitelisted; any other non-whitelisted action (and a
    /// non-string discriminator) still needs approval.
    ActionWhitelistOrDefault {
        actions: Vec<String>,
        default_action: String,
    },
}

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Declarative policy for a tool — determines how the gate handles each call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolPolicy {
    /// Always allow execution (read-only or safe tools).
    Allow,
    /// Always block execution.
    Block { reason: String },
    /// Allow in interactive (user-present) mode; block in callback AND heartbeat.
    InteractiveOnly,
    /// Always require explicit user approval before execution.
    RequireApproval,
    /// Require approval UNLESS args match a whitelisted pattern.
    /// If matched -> Allow; if not matched -> NeedApproval (same as RequireApproval).
    RequireApprovalUnless { patterns: Vec<ArgPattern> },
    /// Require approval only in callback mode; allow when user is present.
    /// Under `Heartbeat`, blocks — heartbeat has its own scoped allow-list.
    ApprovalInCallback,
    /// Allowed only when caller is `Heartbeat`; blocks otherwise.
    /// Used by heartbeat-scoped tools (noop, propose_config_change,
    /// send_message, write_next_tick_note).
    HeartbeatAllowed,
    /// Allowed only inside a subcall (i.e., when called with
    /// `CallerContext::Subcall`). All other callers see Block. Used by
    /// `final_answer`, which is how a subcall reports its structured
    /// result back to the parent loop — see docs/subcall-primitive-design.md §6.5.
    SubcallOnly,
}

/// Caller context for a tool invocation — replaces the prior `is_callback: bool`.
///
/// The gate branches on this to decide Allow/Block/NeedApproval. Every call
/// site must pass an explicit variant (no `From<bool>` conversion is provided
/// intentionally — we want every site to state what kind of caller it is).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerContext {
    /// User present in a live Telegram turn — interactive, user can answer
    /// approval prompts synchronously.
    Interactive,
    /// Deferred intent execution, callback-style — user not present, approvals
    /// are queued.
    Callback,
    /// Cron-triggered heartbeat tick — scoped allow-list from design §4.3.
    Heartbeat,
    /// Caller is a subcall — restricted tool surface; `final_answer`
    /// is the only way to exit. See docs/subcall-primitive-design.md §6.5.
    Subcall,
}

/// The gate's verdict for a single tool invocation.
#[derive(Debug, Clone)]
pub enum GateVerdict {
    /// Proceed with execution.
    Allow,
    /// Block execution — return error to LLM.
    Block { reason: String },
    /// Pause execution — store approval request, notify user, return pending.
    NeedApproval {
        approval_id: String,
        tool_name: String,
        tool_args: String,
        summary: String,
    },
}

/// Data-egress capability of a NATIVE (macro / manual dispatched) tool.
///
/// This is the structural classification the exfiltration guard uses for native
/// tools, which — unlike WASM *sandbox* skills and MCP tools — carry no
/// capability manifest. Genuine WASM sandbox skills like `http_api` and MCP
/// tools are guarded at their own dispatch sites.
///
/// The classification is *destination-aware*, not "is it outbound": a tool that
/// only ever sends to the owner is `OwnerDirected` (not a sink); a tool whose
/// destination is unconditionally caller-chosen-external is
/// `ArbitraryExternalSink`; and a tool that is external only under specific
/// args/settings (e.g. calendar invites — self-only by default, external once
/// `calendar_invite_others_enabled` is on) is `ConditionalExternalSink`, refined
/// per-call by [`ToolGate::conditional_sink_is_external`] so the safe default
/// mode is never false-blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataEgress {
    /// Pulls data inward, or has no outbound payload (readers, config, internal
    /// pseudo-tools). Never an exfiltration sink. Safe default for any tool not
    /// explicitly classified below.
    Inbound,
    /// Sends data, but only to the owner's own channels (owner Telegram chat,
    /// the owner's own cloud storage). Not a third-party exfiltration sink.
    OwnerDirected,
    /// Can transmit caller-supplied content to an ARBITRARY, caller-chosen
    /// external recipient (email `to`/`cc`/`bcc`, …). A real exfiltration
    /// sink: the destination is unconditionally attacker-influenceable.
    ArbitraryExternalSink,
    /// External only under tool-specific argument/settings conditions (e.g.
    /// calendar invites reach third parties only when `calendar_invite_others_
    /// enabled` is on AND non-self attendees are supplied). The chokepoint must
    /// call [`ToolGate::conditional_sink_is_external`] to decide per-call.
    ConditionalExternalSink,
}

// ---------------------------------------------------------------------------
// ToolGate
// ---------------------------------------------------------------------------

pub struct ToolGate {
    /// Static policies (populated from [`default_policies`]).
    policies: HashMap<String, ToolPolicy>,
}

impl Default for ToolGate {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolGate {
    pub fn new() -> Self {
        Self {
            policies: default_policies(),
        }
    }

    /// Structural per-tool data-egress classification — see [`DataEgress`].
    ///
    /// Associated function (no policy state needed): the classification is a
    /// fixed property of the tool, not of an approval decision. Anything not
    /// listed here is [`DataEgress::Inbound`] — a SAFE default that adds no
    /// restriction (the dynamic WASM/MCP network surface is guarded separately).
    ///
    /// HOW TO ADD A TOOL: a new native tool that can send caller-supplied
    /// content to a *caller-chosen external recipient* MUST be classified as
    /// `ArbitraryExternalSink` (unconditional) or `ConditionalExternalSink`
    /// (only under specific args/settings). The `exfil_sink_set_is_pinned` /
    /// `arbitrary_external_sinks_are_approval_gated` tests below pin the sets so
    /// any change is a conscious, reviewed diff.
    pub fn data_egress(tool_name: &str) -> DataEgress {
        match tool_name {
            // ── Arbitrary-external sinks ──────────────────────────────────
            // Destination is UNCONDITIONALLY caller-chosen-external.
            // Mail send tools were removed with the self-built mail client.

            // ── Conditional external sinks ────────────────────────────────
            // External only when args/settings make them so — refined per-call by
            // `conditional_sink_is_external`. calendar invites are self-only by
            // default; they reach third parties only when invite-others is on AND
            // non-self attendees are supplied. Static-sink labelling here would
            // false-block legit self-only autonomous events.
            "calendar_create_event" | "calendar_update_event" => {
                DataEgress::ConditionalExternalSink
            }

            // ── Owner-directed outbound (not exfil sinks) ─────────────────
            //   send_message / notify_user — deliver to the owner's chat.
            //   google_drive_upload        — writes to the owner's own Drive.
            "send_message" | "notify_user" | "google_drive_upload" => DataEgress::OwnerDirected,

            // Everything else native = inbound / config / control.
            //
            // KNOWN RESIDUAL (accepted): `web_search` sends a caller-influenced
            // query string outbound to the search provider, but it is read-
            // oriented and low-bandwidth (a short query, not a document body), so
            // it is classified `Inbound`. Exfiltrating meaningful data through a
            // search query is impractical, and treating it as a sink would
            // false-block ordinary autonomous searches. Revisit if a
            // high-bandwidth query-carrying tool is ever added.
            _ => DataEgress::Inbound,
        }
    }

    /// `true` iff the tool is an *unconditional* arbitrary-external exfiltration
    /// sink (see [`DataEgress::ArbitraryExternalSink`]). Conditional sinks are
    /// decided per-call by [`Self::conditional_sink_is_external`].
    pub fn is_arbitrary_external_sink(tool_name: &str) -> bool {
        matches!(
            Self::data_egress(tool_name),
            DataEgress::ArbitraryExternalSink
        )
    }

    /// For a [`DataEgress::ConditionalExternalSink`] tool, decide whether THIS
    /// call actually reaches a third party, given parsed `args` and the relevant
    /// setting. Keeps the conditional egress policy testable and co-located with
    /// the classifier; the chokepoint reads settings and passes them in.
    ///
    /// Conservative by design: for calendar invites we treat the call as external
    /// whenever invite-others is enabled AND any attendee is supplied (we do not
    /// resolve the owner's own address here to filter self-only attendees — a
    /// rare over-approximation that only ever applies once the owner has opted
    /// into inviting others). When invite-others is off, the attendee-policy gate
    /// already forces self-only, so it can never be external.
    pub fn conditional_sink_is_external(
        tool_name: &str,
        args: &serde_json::Value,
        calendar_invite_others_enabled: bool,
    ) -> bool {
        match tool_name {
            "calendar_create_event" | "calendar_update_event" => {
                calendar_invite_others_enabled
                    && args
                        .get("attendees")
                        .and_then(|a| a.as_array())
                        .is_some_and(|a| !a.is_empty())
            }
            _ => false,
        }
    }

    /// Evaluate a tool call against the policy registry.
    ///
    /// * `tool_name` — canonical tool name (e.g. `"google_drive_upload"`).
    /// * `tool_args` — JSON-encoded arguments (stored in approval request).
    /// * `caller`    — which runtime context issued the call. See
    ///   [`CallerContext`].
    pub fn evaluate(&self, tool_name: &str, tool_args: &str, caller: CallerContext) -> GateVerdict {
        let policy = self.policies.get(tool_name);

        match policy {
            None => {
                // Unknown tool: default to InteractiveOnly behaviour.
                // Safe default — allow only when user is present; block in
                // callback, heartbeat, and subcall (each has its own scoped
                // list / restricted surface and unknown tools must not leak
                // through).
                match caller {
                    CallerContext::Interactive => GateVerdict::Allow,
                    CallerContext::Callback => GateVerdict::Block {
                        reason: format!(
                            "Tool '{}' has no explicit policy and is blocked in callback mode.",
                            tool_name
                        ),
                    },
                    CallerContext::Heartbeat => GateVerdict::Block {
                        reason: format!(
                            "Tool '{}' has no explicit policy and is blocked in heartbeat mode.",
                            tool_name
                        ),
                    },
                    CallerContext::Subcall => GateVerdict::Block {
                        reason: format!(
                            "Tool '{}' has no explicit policy and is blocked in subcall mode.",
                            tool_name,
                        ),
                    },
                }
            }
            Some(ToolPolicy::Allow)
                if tool_name == "run_external" && caller == CallerContext::Subcall =>
            {
                // Owner decision 2026-09-05/07: grok/codex runs need no
                // per-call confirmation in Interactive/Callback, but a
                // subcall has no owner present and must not spawn an
                // unbounded external agent. Mirror cancel_intent's Subcall block.
                GateVerdict::Block {
                    reason: format!("Tool '{tool_name}' is not available in subcall mode."),
                }
            }
            Some(ToolPolicy::Allow) => GateVerdict::Allow,
            Some(ToolPolicy::Block { reason }) => GateVerdict::Block {
                reason: reason.clone(),
            },
            Some(ToolPolicy::InteractiveOnly) => match caller {
                CallerContext::Interactive => GateVerdict::Allow,
                CallerContext::Callback | CallerContext::Heartbeat | CallerContext::Subcall => {
                    GateVerdict::Block {
                        reason: format!("Tool '{}' requires direct user interaction.", tool_name),
                    }
                }
            },
            Some(ToolPolicy::HeartbeatAllowed) => match caller {
                CallerContext::Heartbeat => GateVerdict::Allow,
                CallerContext::Interactive | CallerContext::Callback | CallerContext::Subcall => {
                    GateVerdict::Block {
                        reason: format!(
                            "Tool '{}' is only available in heartbeat scope.",
                            tool_name
                        ),
                    }
                }
            },
            Some(ToolPolicy::SubcallOnly) => match caller {
                CallerContext::Subcall => GateVerdict::Allow,
                CallerContext::Interactive | CallerContext::Callback | CallerContext::Heartbeat => {
                    GateVerdict::Block {
                        reason: format!(
                            "Tool '{}' is only available inside a subcall (subcall-exit-only).",
                            tool_name,
                        ),
                    }
                }
            },
            Some(ToolPolicy::RequireApproval) => {
                // Subcalls run without a user present, so a deferred approval
                // would hang forever. Block deterministically instead.
                if caller == CallerContext::Subcall {
                    return GateVerdict::Block {
                        reason: format!(
                            "Tool '{}' requires human approval and is not available in subcall mode.",
                            tool_name,
                        ),
                    };
                }
                // When the Jev non-CU gate is on, Interactive calls skip this
                // ToolGate HITL so Jev allow|ask|deny is the owner prompt
                // (flag off = existing RequireApproval, no behavior change).
                if caller == CallerContext::Interactive
                    && crate::computer_use::jev_non_cu_gate_enabled()
                    && crate::computer_use::non_cu_gate::is_jev_non_cu_gated_tool(tool_name)
                {
                    return GateVerdict::Allow;
                }
                let approval_id = format!("gate-{:016x}", rand::random::<u64>());
                // Include a brief args summary so the user understands WHY
                // approval is needed, not just which tool.
                let args_hint = summarize_tool_args(tool_name, tool_args);
                let summary = if args_hint.is_empty() {
                    format!("Tool '{}' requires your approval.", tool_name)
                } else {
                    format!(
                        "Tool '{}' requires your approval.\n{}",
                        tool_name, args_hint
                    )
                };
                GateVerdict::NeedApproval {
                    approval_id,
                    tool_name: tool_name.to_string(),
                    tool_args: tool_args.to_string(),
                    summary,
                }
            }
            Some(ToolPolicy::RequireApprovalUnless { patterns }) => {
                // Subcalls may still take the fast-path Allow if the args
                // match the whitelist (safe-subset), but must Block on the
                // approval side — no user to answer a deferred approval.
                if args_match_patterns(tool_args, patterns) {
                    GateVerdict::Allow
                } else if caller == CallerContext::Subcall {
                    GateVerdict::Block {
                        reason: format!(
                            "Tool '{}' would require human approval and is not available in subcall mode.",
                            tool_name,
                        ),
                    }
                } else {
                    let approval_id = format!("gate-{:016x}", rand::random::<u64>());
                    let args_hint = summarize_tool_args(tool_name, tool_args);
                    let summary = if args_hint.is_empty() {
                        format!("Tool '{}' requires your approval.", tool_name)
                    } else {
                        format!(
                            "Tool '{}' requires your approval.\n{}",
                            tool_name, args_hint
                        )
                    };
                    GateVerdict::NeedApproval {
                        approval_id,
                        tool_name: tool_name.to_string(),
                        tool_args: tool_args.to_string(),
                        summary,
                    }
                }
            }
            Some(ToolPolicy::ApprovalInCallback) => match caller {
                CallerContext::Interactive => GateVerdict::Allow,
                CallerContext::Callback => {
                    let approval_id = format!("gate-{:016x}", rand::random::<u64>());
                    let args_hint = summarize_tool_args(tool_name, tool_args);
                    let summary = if args_hint.is_empty() {
                        format!("Tool '{}' needs approval in background mode.", tool_name)
                    } else {
                        format!(
                            "Tool '{}' needs approval in background mode.\n{}",
                            tool_name, args_hint
                        )
                    };
                    GateVerdict::NeedApproval {
                        approval_id,
                        tool_name: tool_name.to_string(),
                        tool_args: tool_args.to_string(),
                        summary,
                    }
                }
                // Heartbeat has its own scoped allow-list; tools with
                // ApprovalInCallback are NOT heartbeat-scoped and must block.
                CallerContext::Heartbeat => GateVerdict::Block {
                    reason: format!("Tool '{}' is not available in heartbeat mode.", tool_name),
                },
                // Subcalls run a restricted tool surface (final_answer +
                // explicit Allow/SubcallOnly policies). Tools that would
                // normally prompt for approval have no user to notify inside
                // a subcall, so block deterministically rather than defer.
                CallerContext::Subcall => GateVerdict::Block {
                    reason: format!("Tool '{}' is not available in subcall mode.", tool_name),
                },
            },
        }
    }

    /// Check if a tool call is pre-approved by a stored approval.
    /// Returns true if a matching approval exists.
    pub fn check_stored_approval(
        &self,
        tool_name: &str,
        approvals: &[StoredApproval],
        intent_group_key: Option<&str>,
        url_host: Option<&str>,
    ) -> bool {
        approvals.iter().any(|a| {
            if a.tool_name != tool_name {
                return false;
            }
            match &a.scope {
                ApprovalScope::Always => true,
                ApprovalScope::Intent { group_key } => {
                    intent_group_key.is_some_and(|k| k == group_key)
                }
                ApprovalScope::Domain { host } => {
                    // Domain approvals expire after DOMAIN_APPROVAL_TTL_MINUTES
                    let expired = chrono::DateTime::parse_from_rfc3339(&a.approved_at)
                        .map(|t| {
                            let age = chrono::Utc::now().signed_duration_since(t);
                            age.num_minutes() > DOMAIN_APPROVAL_TTL_MINUTES
                        })
                        .unwrap_or(true); // malformed timestamp → treat as expired (safe default)
                    if expired {
                        return false;
                    }
                    url_host.is_some_and(|h| h == host || h.ends_with(&format!(".{}", host)))
                }
                ApprovalScope::Once => false, // Once is never stored
            }
        })
    }

    /// Check if a tool has any policy registered.
    pub fn has_policy(&self, tool_name: &str) -> bool {
        self.policies.contains_key(tool_name)
    }

    /// Get the policy for a tool (for inspection / logging).
    pub fn get_policy(&self, tool_name: &str) -> Option<&ToolPolicy> {
        self.policies.get(tool_name)
    }

    /// Evaluate a tool call in autonomous mode (heartbeat tick or subcall).
    ///
    /// Layered filtering:
    /// 1. Check autonomous denylist (absolute block)
    /// 2. Check autonomous allowlist (only listed tools pass)
    /// 3. Apply normal ToolGate policy with the supplied `caller` context —
    ///    `CallerContext::Heartbeat` for cron ticks, `CallerContext::Subcall`
    ///    for LLM-directed subcalls (Task 2.6)
    /// 4. Downgrade `NeedApproval` to `Block` (no user present to approve)
    pub fn evaluate_autonomous(
        &self,
        tool_name: &str,
        tool_args: &str,
        config: &crate::chronos::AutonomousConfig,
        caller: CallerContext,
    ) -> GateVerdict {
        // Layer 1: Denylist takes absolute precedence
        if config.denied_tools.iter().any(|t| t == tool_name) {
            return GateVerdict::Block {
                reason: format!(
                    "Tool '{}' is explicitly denied in autonomous mode",
                    tool_name
                ),
            };
        }

        // Layer 2: Allowlist check — use configured list, fall back to defaults
        let effective_allowed = if config.allowed_tools.is_empty() {
            crate::chronos::default_autonomous_allowed_tools()
        } else {
            config.allowed_tools.clone()
        };
        if !effective_allowed.iter().any(|t| t == tool_name) {
            return GateVerdict::Block {
                reason: format!("Tool '{}' is not in the autonomous allowlist", tool_name),
            };
        }

        // Layer 3: Normal ToolGate evaluation under the supplied caller.
        // For heartbeat ticks this is `Heartbeat`; for LLM-directed subcalls
        // this is `Subcall`, which activates the Task 2.1 Subcall-Block
        // match arms (InteractiveOnly / HeartbeatAllowed / ApprovalInCallback /
        // RequireApproval / unsafe-args RequireApprovalUnless → Block).
        let verdict = self.evaluate(tool_name, tool_args, caller);

        // Layer 4: Downgrade NeedApproval to Block
        // Critical safety: no human is present to approve in autonomous mode.
        // Note: for Subcall caller, Task 2.1's match arms already early-Block
        // on RequireApproval / unsafe-args RequireApprovalUnless, so this
        // downgrade fires only for the Heartbeat caller in practice.
        match verdict {
            GateVerdict::NeedApproval {
                tool_name: tn,
                summary,
                ..
            } => GateVerdict::Block {
                reason: format!(
                    "Tool '{}' requires human approval which cannot be obtained \
                     in autonomous mode: {}",
                    tn, summary
                ),
            },
            other => other,
        }
    }
}

// ---------------------------------------------------------------------------
// Argument pattern helpers
// ---------------------------------------------------------------------------

/// Check if tool args match any of the given argument patterns.
/// The action discriminator of a tool call. Skills name it `action` or
/// `action_type` (sysadmin_ops uses the latter); both are read.
fn action_field(v: &serde_json::Value) -> Option<&serde_json::Value> {
    v.get("action").or_else(|| v.get("action_type"))
}

fn args_match_patterns(tool_args: &str, patterns: &[ArgPattern]) -> bool {
    for pattern in patterns {
        match pattern {
            ArgPattern::ActionWhitelist(actions) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(tool_args)
                    && let Some(action) = action_field(&v).and_then(|a| a.as_str())
                    && actions.iter().any(|a| a == action)
                {
                    return true;
                }
            }
            ArgPattern::ActionWhitelistOrDefault {
                actions,
                default_action,
            } => {
                // Only a parseable JSON object may take the default: a bare
                // call is `{}`; unparseable / non-object args never match.
                let Ok(v) = serde_json::from_str::<serde_json::Value>(tool_args) else {
                    continue;
                };
                if !v.is_object() {
                    continue;
                }
                let effective = match action_field(&v) {
                    None | Some(serde_json::Value::Null) => Some(default_action.as_str()),
                    Some(serde_json::Value::String(s)) if s.is_empty() => {
                        Some(default_action.as_str())
                    }
                    Some(serde_json::Value::String(s)) => Some(s.as_str()),
                    Some(_) => None,
                };
                if let Some(action) = effective
                    && actions.iter().any(|a| a == action)
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Extract a brief human-readable summary from tool args so the approval
/// message tells the user *why* the tool is being invoked, not just which one.
///
/// Design goals (motivation + entity + scope):
/// - Name the concrete target (email address, file path, setting key, intent id)
/// - Indicate destructive-vs-read-only where discernible
/// - Call out whitelist / scope annotations when relevant
/// - Keep output Telegram-markdown-safe (plain text, no backticks)
fn summarize_tool_args(tool_name: &str, tool_args: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(tool_args) else {
        return String::new();
    };
    // Pick the most informative fields depending on the tool.
    let action = v.get("action").and_then(|a| a.as_str());
    let key = v.get("key").and_then(|k| k.as_str());
    let value = v.get("value").and_then(|val| val.as_str());
    let skill = v
        .get("skill_name")
        .or_else(|| v.get("skill"))
        .and_then(|s| s.as_str());

    match tool_name {
        "update_setting" => match (key, value) {
            (Some(k), Some(val)) => format!("Setting: {} → {}", k, val),
            (Some(k), None) => format!("Setting: {}", k),
            _ => String::new(),
        },
        "manage_skill" | "deactivate_skill" | "reactivate_skill" => {
            let act = action.unwrap_or("modify");
            match skill {
                Some(s) => format!("Action: {} skill '{}'", act, s),
                None => format!("Action: {}", act),
            }
        }
        "setup_google" => {
            let act = action.unwrap_or("");
            match act {
                "revoke" => "\u{52a8}\u{4f5c} / Action: revoke Google OAuth (all connected accounts)\n\
                     \u{8303}\u{56f4} / Scope: disconnects Calendar / Drive; deletes stored tokens\n\
                     \u{53ef}\u{9006}\u{6027} / Reversibility: can re-authorize via setup_google(action=\"setup\")"
                    .to_string(),
                "setup" => "Action: begin Google OAuth setup (consent flow)".to_string(),
                "upgrade-scopes" => "\u{52a8}\u{4f5c} / Action: incremental scope upgrade for already-connected account\n\
                     \u{8303}\u{56f4} / Scope: generates a new Google OAuth consent URL; Google will show the full consent screen (prompt=consent is required to return a fresh refresh_token), but the issued token carries the UNION of old + new scopes (include_granted_scopes=true)\n\
                     \u{53ef}\u{9006}\u{6027} / Reversibility: user must still click Allow on Google's consent screen; no token change until then"
                    .to_string(),
                other if !other.is_empty() => format!("Action: {}", other),
                _ => String::new(),
            }
        }
        "sysadmin_ops" => action.map(|a| format!("Action: {}", a)).unwrap_or_default(),
        "cancel_intent" => {
            let intent_id = v
                .get("intent_id")
                .or_else(|| v.get("id"))
                .and_then(|s| s.as_str())
                .unwrap_or("?");
            let reason = v.get("reason").and_then(|s| s.as_str());
            let mut parts = vec![format!("Target intent: {}", intent_id)];
            if let Some(r) = reason {
                parts.push(format!("Reason: {}", r));
            }
            parts.push(
                "\u{8303}\u{56f4} / Scope: disables the monitoring intent (reversible via re-create)"
                    .to_string(),
            );
            parts.join("\n")
        }
        "set_monitoring_intent"
        | "remove_monitoring_intent"
        | "set_alert_policy"
        | "configure_autonomous" => {
            // Generic: action + target id if present
            let id = v
                .get("intent_id")
                .or_else(|| v.get("policy_id"))
                .or_else(|| v.get("id"))
                .and_then(|s| s.as_str());
            match (action, id) {
                (Some(a), Some(i)) => format!("Action: {} (target: {})", a, i),
                (Some(a), None) => format!("Action: {}", a),
                (None, Some(i)) => format!("Target: {}", i),
                _ => String::new(),
            }
        }
        _ => {
            // Generic: show action if present
            action.map(|a| format!("Action: {}", a)).unwrap_or_default()
        }
    }
}

// ---------------------------------------------------------------------------
// Default policy table
// ---------------------------------------------------------------------------

fn default_policies() -> HashMap<String, ToolPolicy> {
    let mut m = HashMap::new();

    // -- Always Allow (read-only / display / internal pseudo-tools) ------------
    for name in &[
        "web_search",
        // x_search: read-only public X posts via Grok/xAI (no user OAuth / no write).
        "x_search",
        "web_reader",
        "browser_fetch",
        "us_stock_quote",
        "calendar_reader",
        "google_drive_reader",
        "file_summarizer",
        "knowledge_base",
        "manage_tasks",
        "manage_goals",
        "news_digest",
        "server_monitor",
        "http_api", // Wasm skill — generic HTTP access for skills
        "code_assistant",
        // Internal pseudo-tools that must work in callback mode.
        // notify_user is the ONLY way for callbacks to deliver messages to
        // users — blocking it silently breaks the entire intent callback flow.
        "notify_user",
        "get_current_time",
        "recall_memory",
        // FU#2 Phase 6: update_profile writes to Tier-0 via the Profile Slot
        // Registry. Schema-enforced (oneOf over canonical slots) + registry
        // validator at write time. Safe to auto-allow.
        "update_profile",
        "schedule_intent",
        // list_intents is a read-only pseudo-tool that enumerates scheduled
        // intents for the user. It was missing from default_policies, which
        // caused ToolGate to block it in callback mode with "no explicit
        // policy" — wasting completion tokens because the LLM had already
        // decided to call it (it's in the heartbeat allowlist + offered as
        // a Tier-1b "always-promoted" tool schema). See CLAUDE.md
        // anti-pattern checklist: heartbeat allow-list MUST be reflected in
        // default_policies for callback mode.
        "list_intents",
        // 2026-04-17 paired-fix: same heartbeat-allowlist-but-missing-policy
        // class as list_intents. list_monitoring_intents / list_alert_policies
        // are read-only pseudo-tools already promoted to the top-level schema
        // and the heartbeat allowlist (see orchestrator.rs:14419 Promoted
        // Tier 1b). Without explicit Allow policies here they'd block in
        // callback mode.
        "list_monitoring_intents",
        "list_alert_policies",
        // 2026-04-17 sweep: additional read-only pseudo-tools flagged by the
        // CTO review of 42d5568. All are strictly read-only (DB query /
        // registry list / cache read / journalctl -u aegis-runtime) and are
        // offered to the LLM via `tool_schemas.rs` mcp_pseudo_tool_schemas().
        // Without an explicit Allow they fall through to the "no explicit
        // policy → blocked in callback mode" branch, wasting completion
        // tokens. Keep them grouped here so the pairing with tool_schemas.rs
        // stays visible.
        //
        // Deliberately NOT included:
        //   - check_api_usage — has `backfill` / `update_pricing` actions
        //     that mutate pricing data and cost-log rows.
        //   - view_growth    — handler not wired; schema commented out in
        //     mcp_pseudo_tool_schemas() so the LLM is never offered it.
        "view_logs",
        "view_heartbeat_audit",
        "search_tools",
        "mcp_list_servers",
        // Heartbeat memo must be writable by autonomous heartbeat cycles;
        // blocking it forces a redundant memo_eval LLM call each cycle.
        "update_heartbeat_memo",
    ] {
        m.insert(name.to_string(), ToolPolicy::Allow);
    }

    // Foreground manual product-loop reports. Keep this out of callback and
    // heartbeat contexts so Phase 2 does not expand background autonomy.
    m.insert(
        "generate_personal_brief".to_string(),
        ToolPolicy::InteractiveOnly,
    );

    // -- RequireApproval (all system-modifying tools need user confirmation) --
    // Alert/monitoring policy changes MUST go through HITL: the LLM must not
    // be able to unilaterally suppress alerts (e.g. muting its own error
    // notifications to hide failures from the user).
    for name in &[
        "deactivate_skill",
        "reactivate_skill",
        "update_setting",
        "switch_model",
        "set_bot_commands",
        "set_log_level",
        "google_drive_upload",
        "set_alert_policy",
        "configure_autonomous",
        "set_monitoring_intent",
        "remove_monitoring_intent",
        // Clearing a persisted 24h embedding-primary promotion is a durable
        // config change (user_settings row delete) that requires a service
        // restart to take full effect; never auto-allow.
        "unset_embedding_primary_promotion",
        // Nulling every vector in `l2_experience` and recomputing via the
        // embedding API is destructive (destroys the existing semantic
        // corpus) AND expensive (paid API calls per row). Requires explicit
        // human approval on every invocation; no safe-subset fast-path.
        "trigger_corpus_reembed",
        // HITL every event create/update/delete per calendar-write-capability-plan
        "calendar_create_event",
        // HITL every event create/update/delete per calendar-write-capability-plan
        "calendar_update_event",
        // HITL every event create/update/delete per calendar-write-capability-plan
        // (irreversible; calendar_delete_event is destructive)
        "calendar_delete_event",
    ] {
        m.insert(name.to_string(), ToolPolicy::RequireApproval);
    }

    m.insert(
        "manage_skill".to_string(),
        ToolPolicy::RequireApprovalUnless {
            patterns: vec![ArgPattern::ActionWhitelist(vec![
                "health".to_string(),
                "affected_intents".to_string(),
            ])],
        },
    );

    // check_api_usage: read-only views run freely; `backfill` and
    // `update_pricing` rewrite cost rows and need approval (before, the tool
    // had no policy at all and the unknown-tool default allowed everything
    // in interactive mode). execute_check_api_usage reads a missing `action`
    // as "summary", and the model's usual call is the bare `{}` — that is the
    // read-only default and must not end the turn in approval.
    m.insert(
        "check_api_usage".to_string(),
        ToolPolicy::RequireApprovalUnless {
            patterns: vec![ArgPattern::ActionWhitelistOrDefault {
                actions: vec![
                    "balance".to_string(),
                    "activity".to_string(),
                    "summary".to_string(),
                    "local".to_string(),
                    "reconcile".to_string(),
                ],
                default_action: "summary".to_string(),
            }],
        },
    );

    // sysadmin_ops (owner decision 2026-09-12): `diagnose` is read-only — the
    // executor only runs its hardcoded whitelist of inspection commands and
    // rejects shell metacharacters — so it runs without approval; every
    // `propose_changes` (mutating) call still goes through HITL.
    m.insert(
        "sysadmin_ops".to_string(),
        ToolPolicy::RequireApprovalUnless {
            patterns: vec![ArgPattern::ActionWhitelist(vec!["diagnose".to_string()])],
        },
    );

    // setup_google: "status" is read-only (auto-allow); "setup" / "upgrade-scopes" / "revoke" need approval
    m.insert(
        "setup_google".to_string(),
        ToolPolicy::RequireApprovalUnless {
            patterns: vec![ArgPattern::ActionWhitelist(vec!["status".into()])],
        },
    );

    // delegate_task: draft/status are read-only; create hands a task to
    // Hercules, which runs it on the owner's subscription runtimes (grok by
    // default). Owner decision 2026-09-05: no per-call approval for that.
    m.insert("delegate_task".to_string(), ToolPolicy::Allow);
    // run_external: owner decision 2026-09-05/07 — grok/codex subscription
    // runs need no per-call confirmation. Subcall is blocked in evaluate().
    m.insert("run_external".to_string(), ToolPolicy::Allow);

    // Secure Computer Use — env-gated schema; interactive chat only (device
    // actuation still goes through CU Policy Broker + Device Agent).
    m.insert(
        "secure_computer_use".to_string(),
        ToolPolicy::InteractiveOnly,
    );

    // -- ApprovalInCallback (safe interactive, gated in background) -----------
    m.insert("cancel_intent".to_string(), ToolPolicy::ApprovalInCallback); // can disable monitoring

    // -- HeartbeatAllowed (Step 8a + 9b) — heartbeat-scoped tools --------------
    for name in &[
        "noop",
        "write_memory",
        "write_next_tick_note",
        "propose_config_change",
        // Step 10 — the one permitted proactive outbound from heartbeat.
        "send_message",
    ] {
        m.insert(name.to_string(), ToolPolicy::HeartbeatAllowed);
    }

    // -- Subcall primitive (Phase 2 Task 2.2) ----------------------------------
    // spawn_subcall / recall_subcall are Allow under every caller — nesting
    // protection is enforced at the execute_tool handler layer (Task 2.3),
    // not here, since the gate has no visibility into the subcall registry.
    m.insert("spawn_subcall".to_string(), ToolPolicy::Allow);
    m.insert("recall_subcall".to_string(), ToolPolicy::Allow);
    // final_answer is exit-only for a subcall's own loop — never legal in
    // Interactive/Callback/Heartbeat scope.
    m.insert("final_answer".to_string(), ToolPolicy::SubcallOnly);

    m
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chronos::AutonomousConfig;

    fn gate() -> ToolGate {
        ToolGate::new()
    }
    // Helpers to assert verdict variants.
    fn assert_allow(v: &GateVerdict) {
        assert!(
            matches!(v, GateVerdict::Allow),
            "expected Allow, got {:?}",
            v
        );
    }

    fn assert_block(v: &GateVerdict) {
        assert!(
            matches!(v, GateVerdict::Block { .. }),
            "expected Block, got {:?}",
            v
        );
    }

    fn assert_need_approval(v: &GateVerdict) {
        assert!(
            matches!(v, GateVerdict::NeedApproval { .. }),
            "expected NeedApproval, got {:?}",
            v
        );
    }

    // 1. Read-only tools always allowed.
    #[test]
    fn test_allow_policy() {
        let g = gate();
        for name in &[
            "calendar_reader",
            "web_search",
            "knowledge_base",
            "code_assistant",
        ] {
            assert_allow(&g.evaluate(name, "{}", CallerContext::Interactive));
            assert_allow(&g.evaluate(name, "{}", CallerContext::Callback));
        }
    }

    // 2. All system-modifying tools require approval in both modes.
    #[test]
    fn test_system_modifying_tools_require_approval() {
        let g = gate();
        for name in &[
            "deactivate_skill",
            "reactivate_skill",
            "update_setting",
            "switch_model",
            "set_bot_commands",
            "set_log_level",
            // Alert/monitoring policy tools — HITL required to prevent
            // the LLM from silently suppressing its own error alerts.
            "set_alert_policy",
            "configure_autonomous",
            "set_monitoring_intent",
            "remove_monitoring_intent",
        ] {
            assert_need_approval(&g.evaluate(name, "{}", CallerContext::Interactive));
            assert_need_approval(&g.evaluate(name, "{}", CallerContext::Callback));
        }
    }

    // 5. ApprovalInCallback allows when user is present.
    #[test]
    fn test_approval_in_callback_allows_interactive() {
        let g = gate();
        assert_allow(&g.evaluate("cancel_intent", "{}", CallerContext::Interactive));
    }

    // 6. ApprovalInCallback returns NeedApproval in callback mode.
    #[test]
    fn test_approval_in_callback_gates_in_callback() {
        let g = gate();
        let v = g.evaluate(
            "cancel_intent",
            r#"{"to":"a@b.com"}"#,
            CallerContext::Callback,
        );
        assert_need_approval(&v);
        if let GateVerdict::NeedApproval {
            approval_id,
            tool_args,
            ..
        } = &v
        {
            assert!(approval_id.starts_with("gate-"));
            assert_eq!(tool_args, r#"{"to":"a@b.com"}"#);
        }
    }

    // 7. Unknown tools: allow interactive, block callback.
    #[test]
    fn test_unknown_tool_defaults() {
        let g = gate();
        assert_allow(&g.evaluate("totally_new_skill", "{}", CallerContext::Interactive));
        assert_block(&g.evaluate("totally_new_skill", "{}", CallerContext::Callback));
    }
    // 9b. Read-only pseudo-tools must be Allow in every caller mode.
    // These are offered to the LLM via `tool_schemas.rs`
    // mcp_pseudo_tool_schemas() (and a subset — list_intents /
    // list_monitoring_intents / list_alert_policies — are also in the
    // heartbeat allowlist / promoted tier-1b at ~orchestrator.rs:14419).
    // ToolGate must allow them in every caller context — otherwise
    // callback-mode execution wastes completion tokens and logs
    // "[ToolGate] Blocked" WARN. See CLAUDE.md anti-pattern checklist.
    //
    // This list MUST stay in sync with the "read-only pseudo-tools"
    // Allow block in default_policies() above.
    #[test]
    fn promoted_tier1b_read_only_tools_allowed_in_callback_mode() {
        let g = gate();
        for tool in &[
            // Tier-1b promoted (heartbeat allowlist + top-level schema)
            "list_intents",
            "list_monitoring_intents",
            "list_alert_policies",
            // 2026-04-17 sweep — read-only pseudo-tools offered as schemas
            "view_logs",
            "view_heartbeat_audit",
            "search_tools",
            "mcp_list_servers",
        ] {
            assert!(
                matches!(g.get_policy(tool), Some(ToolPolicy::Allow)),
                "missing ToolPolicy::Allow for {tool} — pairs with an \
                 LLM-offered tool schema, ToolGate will otherwise block \
                 execution in callback mode"
            );
            assert_allow(&g.evaluate(tool, "{}", CallerContext::Interactive));
            assert_allow(&g.evaluate(tool, "{}", CallerContext::Callback));
            assert_allow(&g.evaluate(tool, "{}", CallerContext::Heartbeat));
        }
    }

    #[test]
    fn generate_personal_brief_is_interactive_only() {
        let g = gate();
        assert!(matches!(
            g.get_policy("generate_personal_brief"),
            Some(ToolPolicy::InteractiveOnly)
        ));
        assert_allow(&g.evaluate("generate_personal_brief", "{}", CallerContext::Interactive));
        assert_block(&g.evaluate("generate_personal_brief", "{}", CallerContext::Callback));
        assert_block(&g.evaluate("generate_personal_brief", "{}", CallerContext::Heartbeat));
        assert_block(&g.evaluate("generate_personal_brief", "{}", CallerContext::Subcall));
    }

    // Verify has_policy / get_policy accessors.
    #[test]
    fn test_has_policy_and_get_policy() {
        let g = gate();
        assert!(g.has_policy("calendar_reader"));
        assert!(!g.has_policy("nonexistent_tool"));
        assert!(g.get_policy("nonexistent_tool").is_none());
    }

    // NeedApproval carries correct fields.
    #[test]
    fn test_need_approval_fields() {
        let g = gate();
        let v = g.evaluate(
            "google_drive_upload",
            r#"{"filename":"notes.txt"}"#,
            CallerContext::Interactive,
        );
        if let GateVerdict::NeedApproval {
            approval_id,
            tool_name,
            tool_args,
            summary,
        } = v
        {
            assert!(approval_id.starts_with("gate-"));
            assert_eq!(approval_id.len(), 5 + 16); // "gate-" + 16 hex chars
            assert_eq!(tool_name, "google_drive_upload");
            assert_eq!(tool_args, r#"{"filename":"notes.txt"}"#);
            assert!(summary.contains("google_drive_upload"));
        } else {
            panic!("expected NeedApproval");
        }
    }

    // Block policy carries reason.
    #[test]
    fn test_block_policy() {
        let mut g = ToolGate::new();
        g.policies.insert(
            "dangerous_tool".to_string(),
            ToolPolicy::Block {
                reason: "deprecated".to_string(),
            },
        );
        let v = g.evaluate("dangerous_tool", "{}", CallerContext::Interactive);
        if let GateVerdict::Block { reason } = v {
            assert_eq!(reason, "deprecated");
        } else {
            panic!("expected Block");
        }
    }

    // ── Granular approval (StoredApproval / check_stored_approval) tests ──

    fn make_approval(tool: &str, scope: ApprovalScope) -> StoredApproval {
        StoredApproval {
            tool_name: tool.to_string(),
            scope,
            approved_at: chrono::Utc::now().to_rfc3339(),
            approved_by: "did:aegis:test".to_string(),
        }
    }

    #[test]
    fn test_always_approval_matches() {
        let g = gate();
        let approvals = vec![make_approval("web_reader", ApprovalScope::Always)];
        assert!(g.check_stored_approval("web_reader", &approvals, None, None));
    }

    #[test]
    fn test_always_approval_persists_across_calls() {
        let g = gate();
        let approvals = vec![make_approval("web_reader", ApprovalScope::Always)];
        // Should match regardless of context
        assert!(g.check_stored_approval(
            "web_reader",
            &approvals,
            Some("any_intent"),
            Some("any.host")
        ));
        assert!(g.check_stored_approval("web_reader", &approvals, None, None));
    }

    #[test]
    fn test_once_approval_never_matches() {
        let g = gate();
        let approvals = vec![make_approval("web_reader", ApprovalScope::Once)];
        // Once is ephemeral and should never be stored/matched
        assert!(!g.check_stored_approval("web_reader", &approvals, None, None));
    }

    #[test]
    fn test_intent_approval_matches_same_group_key() {
        let g = gate();
        let approvals = vec![make_approval(
            "web_reader",
            ApprovalScope::Intent {
                group_key: "commute_check".to_string(),
            },
        )];
        assert!(g.check_stored_approval("web_reader", &approvals, Some("commute_check"), None));
    }

    #[test]
    fn test_intent_approval_rejects_different_group_key() {
        let g = gate();
        let approvals = vec![make_approval(
            "web_reader",
            ApprovalScope::Intent {
                group_key: "commute_check".to_string(),
            },
        )];
        assert!(!g.check_stored_approval("web_reader", &approvals, Some("other_intent"), None));
    }

    #[test]
    fn test_intent_approval_rejects_no_group_key() {
        let g = gate();
        let approvals = vec![make_approval(
            "web_reader",
            ApprovalScope::Intent {
                group_key: "commute_check".to_string(),
            },
        )];
        assert!(!g.check_stored_approval("web_reader", &approvals, None, None));
    }

    #[test]
    fn test_domain_approval_matches_exact_host() {
        let g = gate();
        let approvals = vec![make_approval(
            "web_reader",
            ApprovalScope::Domain {
                host: "maps.google.com".to_string(),
            },
        )];
        assert!(g.check_stored_approval("web_reader", &approvals, None, Some("maps.google.com")));
    }

    #[test]
    fn test_domain_approval_matches_subdomain() {
        let g = gate();
        let approvals = vec![make_approval(
            "web_reader",
            ApprovalScope::Domain {
                host: "google.com".to_string(),
            },
        )];
        assert!(g.check_stored_approval("web_reader", &approvals, None, Some("maps.google.com")));
    }

    #[test]
    fn test_domain_approval_rejects_different_host() {
        let g = gate();
        let approvals = vec![make_approval(
            "web_reader",
            ApprovalScope::Domain {
                host: "maps.google.com".to_string(),
            },
        )];
        assert!(!g.check_stored_approval("web_reader", &approvals, None, Some("evil.com")));
    }

    #[test]
    fn test_domain_approval_rejects_no_host() {
        let g = gate();
        let approvals = vec![make_approval(
            "web_reader",
            ApprovalScope::Domain {
                host: "google.com".to_string(),
            },
        )];
        assert!(!g.check_stored_approval("web_reader", &approvals, None, None));
    }

    #[test]
    fn test_domain_approval_expires_after_ttl() {
        let g = gate();
        // Create an approval from 2 hours ago — should be expired
        let old_approval = StoredApproval {
            tool_name: "web_reader".to_string(),
            scope: ApprovalScope::Domain {
                host: "claude.ai".to_string(),
            },
            approved_at: (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339(),
            approved_by: "did:aegis:test".to_string(),
        };
        assert!(!g.check_stored_approval("web_reader", &[old_approval], None, Some("claude.ai")));

        // Fresh approval should still work
        let fresh = make_approval(
            "web_reader",
            ApprovalScope::Domain {
                host: "claude.ai".to_string(),
            },
        );
        assert!(g.check_stored_approval("web_reader", &[fresh], None, Some("claude.ai")));
    }

    #[test]
    fn test_wrong_tool_name_never_matches() {
        let g = gate();
        let approvals = vec![make_approval("google_drive_upload", ApprovalScope::Always)];
        assert!(!g.check_stored_approval("web_reader", &approvals, None, None));
    }

    #[test]
    fn test_empty_approvals_never_matches() {
        let g = gate();
        assert!(!g.check_stored_approval("web_reader", &[], None, None));
    }

    #[test]
    fn test_multiple_approvals_any_match() {
        let g = gate();
        let approvals = vec![
            make_approval(
                "web_reader",
                ApprovalScope::Domain {
                    host: "example.com".to_string(),
                },
            ),
            make_approval(
                "web_reader",
                ApprovalScope::Intent {
                    group_key: "daily_check".to_string(),
                },
            ),
        ];
        // Should match on intent even though domain doesn't match
        assert!(g.check_stored_approval(
            "web_reader",
            &approvals,
            Some("daily_check"),
            Some("other.com")
        ));
        // Should match on domain even though intent doesn't match
        assert!(g.check_stored_approval(
            "web_reader",
            &approvals,
            Some("other_intent"),
            Some("example.com")
        ));
    }

    #[test]
    fn test_approval_scope_serialization() {
        let scopes = vec![
            ApprovalScope::Once,
            ApprovalScope::Intent {
                group_key: "test".to_string(),
            },
            ApprovalScope::Domain {
                host: "example.com".to_string(),
            },
            ApprovalScope::Always,
        ];
        for scope in &scopes {
            let json = serde_json::to_string(scope).unwrap();
            let parsed: ApprovalScope = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, scope);
        }
    }

    #[test]
    fn test_stored_approval_serialization() {
        let record = make_approval(
            "web_reader",
            ApprovalScope::Domain {
                host: "test.com".to_string(),
            },
        );
        let json = serde_json::to_string(&record).unwrap();
        let parsed: StoredApproval = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tool_name, "web_reader");
        assert_eq!(
            parsed.scope,
            ApprovalScope::Domain {
                host: "test.com".to_string()
            }
        );
    }

    #[test]
    fn test_domain_partial_match_not_confused() {
        // "notgoogle.com" should NOT match "google.com"
        let g = gate();
        let approvals = vec![make_approval(
            "web_reader",
            ApprovalScope::Domain {
                host: "google.com".to_string(),
            },
        )];
        assert!(!g.check_stored_approval("web_reader", &approvals, None, Some("notgoogle.com")));
    }

    // ── ArgPattern / RequireApprovalUnless tests ──

    #[test]
    fn test_argpattern_serialization() {
        let pat = ArgPattern::ActionWhitelist(vec!["status".into(), "list".into()]);
        let json = serde_json::to_string(&pat).unwrap();
        let parsed: ArgPattern = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, pat);
    }

    #[test]
    fn test_require_approval_unless_serialization() {
        let policy = ToolPolicy::RequireApprovalUnless {
            patterns: vec![ArgPattern::ActionWhitelist(vec!["status".into()])],
        };
        let json = serde_json::to_string(&policy).unwrap();
        let parsed: ToolPolicy = serde_json::from_str(&json).unwrap();
        // Can't derive PartialEq on ToolPolicy without changing existing code,
        // so just verify it round-trips by checking the JSON matches
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }

    #[test]
    fn test_other_require_approval_tools_unchanged() {
        let g = gate();
        // These tools should still use plain RequireApproval
        for name in &[
            "deactivate_skill",
            "reactivate_skill",
            "update_setting",
            "switch_model",
            "set_bot_commands",
            "set_log_level",
        ] {
            assert!(
                matches!(g.get_policy(name), Some(ToolPolicy::RequireApproval)),
                "expected RequireApproval for {}, got {:?}",
                name,
                g.get_policy(name)
            );
        }
    }
    #[test]
    fn delegate_task_all_actions_auto_allowed() {
        // Owner decision 2026-09-05: delegation runs on subscription runtimes;
        // no per-call approval, in interactive or callback context.
        let g = gate();
        for args in [
            r#"{"action":"draft","intent":"x"}"#,
            r#"{"action":"status"}"#,
            r#"{"action":"create","title":"t","repo":"aegis","body":"x"}"#,
        ] {
            assert_allow(&g.evaluate("delegate_task", args, CallerContext::Interactive));
            assert_allow(&g.evaluate("delegate_task", args, CallerContext::Callback));
        }
    }

    #[test]
    fn run_external_auto_allowed_interactive_and_callback() {
        // Owner decision 2026-09-05/07: grok/codex runs need no per-call
        // confirmation in Interactive or Callback.
        let g = gate();
        let args = r#"{"task":"summarize this directory"}"#;
        assert_allow(&g.evaluate("run_external", args, CallerContext::Interactive));
        assert_allow(&g.evaluate("run_external", args, CallerContext::Callback));
        let blocked = g.evaluate("run_external", args, CallerContext::Subcall);
        assert!(
            matches!(blocked, GateVerdict::Block { .. }),
            "run_external must Block under Subcall, got {blocked:?}"
        );
    }

    #[test]
    fn browser_fetch_is_allow() {
        let g = gate();
        let args = r#"{"url":"https://example.com"}"#;
        assert_allow(&g.evaluate("browser_fetch", args, CallerContext::Interactive));
        assert_allow(&g.evaluate("browser_fetch", args, CallerContext::Callback));
    }
    // ── ActionWhitelist pattern ──

    #[test]
    fn sysadmin_ops_diagnose_is_read_only_and_needs_no_approval() {
        let g = gate();
        let diagnose = r#"{"action_type":"diagnose","command":"df -h"}"#;
        assert_allow(&g.evaluate("sysadmin_ops", diagnose, CallerContext::Interactive));
        assert_allow(&g.evaluate("sysadmin_ops", diagnose, CallerContext::Callback));
        // `action` spelling is accepted too.
        let diagnose_alt = r#"{"action":"diagnose","command":"uptime"}"#;
        assert_allow(&g.evaluate("sysadmin_ops", diagnose_alt, CallerContext::Interactive));
        // Mutations and bare calls still need the owner.
        let mutate = r#"{"action_type":"propose_changes","commands":["apt-get install x"]}"#;
        assert_need_approval(&g.evaluate("sysadmin_ops", mutate, CallerContext::Interactive));
        assert_need_approval(&g.evaluate("sysadmin_ops", "{}", CallerContext::Interactive));
    }

    #[test]
    fn action_whitelist_serialization() {
        let pat = ArgPattern::ActionWhitelist(vec!["status".into(), "list".into()]);
        let json = serde_json::to_string(&pat).unwrap();
        let parsed: ArgPattern = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, pat);
    }

    // ── ActionWhitelistOrDefault pattern (check_api_usage) ──

    #[test]
    fn check_api_usage_bare_call_is_the_read_only_default() {
        // execute_check_api_usage reads a missing `action` as "summary", and
        // the model's usual call is `{}`; that must not end the turn in
        // "awaiting your approval" (journey_api_usage_check).
        let g = gate();
        for args in [
            "{}",
            r#"{"action":null}"#,
            r#"{"action":""}"#,
            r#"{"days":30}"#,
        ] {
            assert_allow(&g.evaluate("check_api_usage", args, CallerContext::Interactive));
            assert_allow(&g.evaluate("check_api_usage", args, CallerContext::Callback));
        }
    }

    #[test]
    fn check_api_usage_explicit_read_only_actions_are_allowed() {
        let g = gate();
        for action in ["balance", "activity", "summary", "local", "reconcile"] {
            let args = format!(r#"{{"action":"{action}"}}"#);
            assert_allow(&g.evaluate("check_api_usage", &args, CallerContext::Interactive));
        }
    }

    #[test]
    fn check_api_usage_mutating_actions_need_approval() {
        let g = gate();
        for action in ["backfill", "update_pricing"] {
            let args = format!(r#"{{"action":"{action}"}}"#);
            assert_need_approval(&g.evaluate("check_api_usage", &args, CallerContext::Interactive));
            assert_need_approval(&g.evaluate("check_api_usage", &args, CallerContext::Callback));
        }
        // A non-string discriminator is neither whitelisted nor the default.
        assert_need_approval(&g.evaluate(
            "check_api_usage",
            r#"{"action":5}"#,
            CallerContext::Interactive,
        ));
    }

    #[test]
    fn check_api_usage_subcall_default_allowed_mutation_blocked() {
        let g = gate();
        assert_allow(&g.evaluate("check_api_usage", "{}", CallerContext::Subcall));
        assert_block(&g.evaluate(
            "check_api_usage",
            r#"{"action":"backfill"}"#,
            CallerContext::Subcall,
        ));
    }

    #[test]
    fn plain_action_whitelist_still_sends_bare_calls_to_approval() {
        // Negative control: the default applies only to
        // ActionWhitelistOrDefault. setup_google (plain ActionWhitelist)
        // keeps "missing action → approval" — its bare call is not read-only.
        let g = gate();
        assert_need_approval(&g.evaluate("setup_google", "{}", CallerContext::Interactive));
        assert_need_approval(&g.evaluate(
            "setup_google",
            r#"{"action":""}"#,
            CallerContext::Interactive,
        ));
        assert_block(&g.evaluate("setup_google", "{}", CallerContext::Subcall));
    }

    #[test]
    fn action_whitelist_or_default_only_opens_a_whitelisted_default() {
        let open = [ArgPattern::ActionWhitelistOrDefault {
            actions: vec!["summary".into()],
            default_action: "summary".into(),
        }];
        assert!(super::args_match_patterns("{}", &open));
        assert!(super::args_match_patterns(r#"{"action":"summary"}"#, &open));
        assert!(!super::args_match_patterns(
            r#"{"action":"backfill"}"#,
            &open
        ));
        // A default that is not itself whitelisted does not open the bare call.
        let closed = [ArgPattern::ActionWhitelistOrDefault {
            actions: vec!["summary".into()],
            default_action: "backfill".into(),
        }];
        assert!(!super::args_match_patterns("{}", &closed));
        // Unparseable / non-object args never take the default.
        for args in ["", "null", "[]", "\"summary\"", "not json"] {
            assert!(!super::args_match_patterns(args, &open), "args={args:?}");
        }
    }

    #[test]
    fn action_whitelist_or_default_serialization() {
        let pat = ArgPattern::ActionWhitelistOrDefault {
            actions: vec!["summary".into(), "local".into()],
            default_action: "summary".into(),
        };
        let json = serde_json::to_string(&pat).unwrap();
        let parsed: ArgPattern = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, pat);
        // Existing ActionWhitelist values keep their wire shape.
        let legacy: ArgPattern = serde_json::from_str(r#"{"ActionWhitelist":["status"]}"#).unwrap();
        assert_eq!(legacy, ArgPattern::ActionWhitelist(vec!["status".into()]));
    }

    // ── Autonomous heartbeat gating tests ──

    #[test]
    fn autonomous_denylist_blocks() {
        let gate = ToolGate::new();
        let config = AutonomousConfig {
            enabled: true,
            denied_tools: vec!["web_search".into()],
            ..Default::default()
        };
        let v = gate.evaluate_autonomous("web_search", "{}", &config, CallerContext::Heartbeat);
        assert!(matches!(v, GateVerdict::Block { .. }));
    }

    #[test]
    fn autonomous_allowlist_blocks_unlisted() {
        let gate = ToolGate::new();
        let config = AutonomousConfig {
            enabled: true,
            allowed_tools: vec!["notify_user".into()],
            ..Default::default()
        };
        // web_search not in allowlist
        let v = gate.evaluate_autonomous("web_search", "{}", &config, CallerContext::Heartbeat);
        assert!(matches!(v, GateVerdict::Block { .. }));
        // notify_user IS in allowlist and has Allow policy
        let v2 = gate.evaluate_autonomous("notify_user", "{}", &config, CallerContext::Heartbeat);
        assert!(matches!(v2, GateVerdict::Allow));
    }

    #[test]
    fn autonomous_need_approval_downgraded_to_block() {
        let gate = ToolGate::new();
        let config = AutonomousConfig {
            enabled: true,
            allowed_tools: vec!["update_setting".into()],
            ..Default::default()
        };
        // update_setting has RequireApproval policy — should be downgraded to Block
        let v = gate.evaluate_autonomous("update_setting", "{}", &config, CallerContext::Heartbeat);
        assert!(matches!(v, GateVerdict::Block { .. }));
    }

    #[test]
    fn autonomous_default_allowlist_permits_safe_tools() {
        let gate = ToolGate::new();
        let config = AutonomousConfig::default(); // empty allowed_tools -> uses default list
        // notify_user is in the default list and has Allow policy
        let v = gate.evaluate_autonomous("notify_user", "{}", &config, CallerContext::Heartbeat);
        assert!(matches!(v, GateVerdict::Allow));
    }

    #[test]
    fn autonomous_denylist_overrides_allowlist() {
        let gate = ToolGate::new();
        let config = AutonomousConfig {
            enabled: true,
            allowed_tools: vec!["notify_user".into()],
            denied_tools: vec!["notify_user".into()], // same tool in both!
            ..Default::default()
        };
        let v = gate.evaluate_autonomous("notify_user", "{}", &config, CallerContext::Heartbeat);
        assert!(matches!(v, GateVerdict::Block { .. }));
    }

    // ── summarize_tool_args tests ─────────────────────────────────────

    #[test]
    fn summarize_update_setting() {
        let s = super::summarize_tool_args(
            "update_setting",
            r#"{"key":"user_timezone","value":"Asia/Shanghai"}"#,
        );
        assert!(s.contains("user_timezone"), "should contain key: {s}");
        assert!(s.contains("Asia/Shanghai"), "should contain value: {s}");
    }

    #[test]
    fn summarize_manage_skill() {
        let s = super::summarize_tool_args(
            "manage_skill",
            r#"{"action":"health","skill_name":"calendar_reader"}"#,
        );
        assert!(s.contains("health"), "{s}");
        assert!(s.contains("calendar_reader"), "{s}");
    }

    #[test]
    fn manage_skill_read_only_actions_are_allowed() {
        let g = gate();
        for action in ["health", "affected_intents"] {
            let args = format!(r#"{{"action":"{action}","skill_name":"calendar_reader"}}"#);
            assert_allow(&g.evaluate("manage_skill", &args, CallerContext::Interactive));
            assert_allow(&g.evaluate("manage_skill", &args, CallerContext::Callback));
            assert_allow(&g.evaluate("manage_skill", &args, CallerContext::Subcall));
        }
    }

    #[test]
    fn manage_skill_unknown_action_still_requires_approval() {
        let g = gate();
        assert_need_approval(&g.evaluate(
            "manage_skill",
            r#"{"action":"mutate_everything","skill_name":"calendar_reader"}"#,
            CallerContext::Interactive,
        ));
        assert_block(&g.evaluate(
            "manage_skill",
            r#"{"action":"mutate_everything","skill_name":"calendar_reader"}"#,
            CallerContext::Subcall,
        ));
    }

    #[test]
    fn summarize_sysadmin_ops() {
        let s = super::summarize_tool_args("sysadmin_ops", r#"{"action":"propose_changes"}"#);
        assert!(s.contains("propose_changes"), "{s}");
    }

    #[test]
    fn summarize_unknown_tool_with_action() {
        let s = super::summarize_tool_args("some_new_tool", r#"{"action":"do_thing"}"#);
        assert!(s.contains("do_thing"), "{s}");
    }

    #[test]
    fn summarize_invalid_json_returns_empty() {
        let s = super::summarize_tool_args("update_setting", "not json");
        assert!(s.is_empty(), "invalid JSON should return empty: {s}");
    }

    #[test]
    fn summarize_missing_fields_returns_empty() {
        let s = super::summarize_tool_args("update_setting", r#"{"unrelated":"data"}"#);
        assert!(s.is_empty(), "missing key/value should return empty: {s}");
    }
    #[test]
    fn summarize_cancel_intent_shows_id() {
        let s = super::summarize_tool_args(
            "cancel_intent",
            r#"{"intent_id":"intent-42","reason":"no longer needed"}"#,
        );
        assert!(s.contains("intent-42"), "{s}");
        assert!(s.contains("no longer needed"), "{s}");
    }

    #[test]
    fn summarize_deactivate_skill_names_the_skill() {
        let s = super::summarize_tool_args("deactivate_skill", r#"{"skill_name":"my_skill"}"#);
        assert!(s.contains("my_skill"), "{s}");
    }
    // -----------------------------------------------------------------------
    // Step 7 — CallerContext table-driven policy tests
    // -----------------------------------------------------------------------

    const ALL_CALLERS: [CallerContext; 4] = [
        CallerContext::Interactive,
        CallerContext::Callback,
        CallerContext::Heartbeat,
        CallerContext::Subcall,
    ];

    #[test]
    fn caller_context_variants_exist() {
        // Lock in the four canonical variants and their PartialEq/Copy behaviour.
        let a = CallerContext::Interactive;
        let b = a; // Copy
        assert_eq!(a, b);
        assert_ne!(CallerContext::Interactive, CallerContext::Callback);
        assert_ne!(CallerContext::Callback, CallerContext::Heartbeat);
        assert_ne!(CallerContext::Interactive, CallerContext::Heartbeat);
        assert_ne!(CallerContext::Subcall, CallerContext::Interactive);
        assert_ne!(CallerContext::Subcall, CallerContext::Callback);
        assert_ne!(CallerContext::Subcall, CallerContext::Heartbeat);
    }

    // Helper: build a gate with a single custom policy for the test tool.
    fn gate_with(policy: ToolPolicy) -> ToolGate {
        let mut g = ToolGate::new();
        g.policies.insert("step7_probe".to_string(), policy);
        g
    }

    #[test]
    fn policy_allow_allows_every_caller() {
        let g = gate_with(ToolPolicy::Allow);
        for c in ALL_CALLERS {
            assert_allow(&g.evaluate("step7_probe", "{}", c));
        }
    }

    #[test]
    fn policy_block_blocks_every_caller() {
        let g = gate_with(ToolPolicy::Block {
            reason: "nope".to_string(),
        });
        for c in ALL_CALLERS {
            assert_block(&g.evaluate("step7_probe", "{}", c));
        }
    }

    #[test]
    fn policy_interactive_only_matrix() {
        let g = gate_with(ToolPolicy::InteractiveOnly);
        assert_allow(&g.evaluate("step7_probe", "{}", CallerContext::Interactive));
        assert_block(&g.evaluate("step7_probe", "{}", CallerContext::Callback));
        assert_block(&g.evaluate("step7_probe", "{}", CallerContext::Heartbeat));
        assert_block(&g.evaluate("step7_probe", "{}", CallerContext::Subcall));
    }

    #[test]
    fn policy_require_approval_matrix() {
        let g = gate_with(ToolPolicy::RequireApproval);
        // Interactive / Callback / Heartbeat defer the approval to a human.
        assert_need_approval(&g.evaluate("step7_probe", "{}", CallerContext::Interactive));
        assert_need_approval(&g.evaluate("step7_probe", "{}", CallerContext::Callback));
        assert_need_approval(&g.evaluate("step7_probe", "{}", CallerContext::Heartbeat));
        // Subcall has no user to answer the approval — Block deterministically.
        assert_block(&g.evaluate("step7_probe", "{}", CallerContext::Subcall));
    }

    #[test]
    fn policy_approval_in_callback_matrix() {
        let g = gate_with(ToolPolicy::ApprovalInCallback);
        assert_allow(&g.evaluate("step7_probe", "{}", CallerContext::Interactive));
        assert_need_approval(&g.evaluate("step7_probe", "{}", CallerContext::Callback));
        // Heartbeat has its own scoped allow-list; ApprovalInCallback tools
        // are not heartbeat-scoped and must block.
        assert_block(&g.evaluate("step7_probe", "{}", CallerContext::Heartbeat));
        // Subcalls have no user to notify — block deterministically.
        assert_block(&g.evaluate("step7_probe", "{}", CallerContext::Subcall));
    }

    #[test]
    fn policy_heartbeat_allowed_matrix() {
        let g = gate_with(ToolPolicy::HeartbeatAllowed);
        assert_block(&g.evaluate("step7_probe", "{}", CallerContext::Interactive));
        assert_block(&g.evaluate("step7_probe", "{}", CallerContext::Callback));
        assert_allow(&g.evaluate("step7_probe", "{}", CallerContext::Heartbeat));
        assert_block(&g.evaluate("step7_probe", "{}", CallerContext::Subcall));
    }

    #[test]
    fn unknown_tool_defaults_matrix() {
        let g = gate();
        // Interactive: allow (preserve existing default)
        assert_allow(&g.evaluate("truly_unknown_xyz", "{}", CallerContext::Interactive));
        // Callback: block
        assert_block(&g.evaluate("truly_unknown_xyz", "{}", CallerContext::Callback));
        // Heartbeat: block (safer default — unknown tools must not leak
        // through the heartbeat scope).
        assert_block(&g.evaluate("truly_unknown_xyz", "{}", CallerContext::Heartbeat));
        // Subcall: block — restricted tool surface, unknown tools never leak in.
        assert_block(&g.evaluate("truly_unknown_xyz", "{}", CallerContext::Subcall));
    }

    #[test]
    fn step8a_tools_are_heartbeat_allowed() {
        // Step 8a — the three new tools must block outside heartbeat scope
        // and allow inside it. Verifies policy registration in default_policies().
        let g = ToolGate::new();
        for name in &["noop", "write_memory", "write_next_tick_note"] {
            assert_block(&g.evaluate(name, "{}", CallerContext::Interactive));
            assert_block(&g.evaluate(name, "{}", CallerContext::Callback));
            assert_allow(&g.evaluate(name, "{}", CallerContext::Heartbeat));
        }
    }

    #[test]
    fn step9b_tools_are_heartbeat_allowed() {
        // Step 9b — propose_config_change must block outside heartbeat
        // scope and allow inside.
        let g = ToolGate::new();
        let name = "propose_config_change";
        assert_block(&g.evaluate(name, "{}", CallerContext::Interactive));
        assert_block(&g.evaluate(name, "{}", CallerContext::Callback));
        assert_allow(&g.evaluate(name, "{}", CallerContext::Heartbeat));
        // Sync with heartbeat allow-list.
        let allow_list = crate::chronos::default_autonomous_allowed_tools();
        assert!(
            allow_list.iter().any(|n| n == name),
            "{name} missing from default_autonomous_allowed_tools"
        );
    }

    #[test]
    fn step10_send_message_is_heartbeat_allowed_and_synced() {
        // Step 10 — send_message must block outside heartbeat scope and allow inside.
        let g = ToolGate::new();
        assert_block(&g.evaluate("send_message", "{}", CallerContext::Interactive));
        assert_block(&g.evaluate("send_message", "{}", CallerContext::Callback));
        assert_allow(&g.evaluate("send_message", "{}", CallerContext::Heartbeat));
        let allow_list = crate::chronos::default_autonomous_allowed_tools();
        assert!(
            allow_list.iter().any(|n| n == "send_message"),
            "send_message missing from default_autonomous_allowed_tools"
        );
    }

    #[test]
    fn step8a_tools_are_in_autonomous_allow_list() {
        // Sync check: the three tools listed by default_autonomous_allowed_tools
        // must also have an Allow-compatible policy under Heartbeat.
        let allow_list = crate::chronos::default_autonomous_allowed_tools();
        for name in &["noop", "write_memory", "write_next_tick_note"] {
            assert!(
                allow_list.iter().any(|n| n == name),
                "{name} missing from default_autonomous_allowed_tools"
            );
        }
    }

    #[test]
    fn evaluate_autonomous_passes_heartbeat_context() {
        // An Allow-policy tool that's on the heartbeat allow-list should pass.
        let g = ToolGate::new();
        let cfg = crate::chronos::AutonomousConfig {
            allowed_tools: vec!["web_search".to_string()],
            denied_tools: vec![],
            ..Default::default()
        };
        let v = g.evaluate_autonomous("web_search", "{}", &cfg, CallerContext::Heartbeat);
        assert_allow(&v);
    }

    #[test]
    fn evaluate_autonomous_routes_subcall_context_through_layer3() {
        // Task 2.6 — when a subcall runs inside run_autonomous_react_loop,
        // Layer 3 must see CallerContext::Subcall so Task 2.1's Subcall-Block
        // match arms fire for HeartbeatAllowed tools (e.g. send_message).
        let g = ToolGate::new();
        let cfg = crate::chronos::AutonomousConfig {
            allowed_tools: vec!["send_message".to_string()],
            denied_tools: vec![],
            ..Default::default()
        };
        let v_heartbeat =
            g.evaluate_autonomous("send_message", "{}", &cfg, CallerContext::Heartbeat);
        assert_allow(&v_heartbeat);
        let v_subcall = g.evaluate_autonomous("send_message", "{}", &cfg, CallerContext::Subcall);
        assert_block(&v_subcall);
    }

    #[test]
    fn evaluate_autonomous_subcall_still_enforces_denylist() {
        // Layer 1 denylist wins over caller-level Allow under Subcall.
        let g = ToolGate::new();
        let cfg = crate::chronos::AutonomousConfig {
            allowed_tools: vec!["spawn_subcall".to_string()],
            denied_tools: vec!["spawn_subcall".to_string()],
            ..Default::default()
        };
        let v = g.evaluate_autonomous("spawn_subcall", "{}", &cfg, CallerContext::Subcall);
        assert_block(&v);
    }

    #[test]
    fn evaluate_autonomous_subcall_enforces_allowlist() {
        // A Subcall caller invoking a tool outside the allowlist Blocks at Layer 2
        // even if Task 2.1's Subcall match arms would otherwise Allow.
        let g = ToolGate::new();
        let cfg = crate::chronos::AutonomousConfig {
            allowed_tools: vec!["recall_subcall".to_string()],
            denied_tools: vec![],
            ..Default::default()
        };
        // final_answer is SubcallOnly (Allow under Subcall at Layer 3) but
        // not in the allow-list → Layer 2 Blocks.
        let v = g.evaluate_autonomous("final_answer", "{}", &cfg, CallerContext::Subcall);
        assert_block(&v);
    }

    // -----------------------------------------------------------------------
    // Task 2.1 — CallerContext::Subcall gating
    // -----------------------------------------------------------------------

    #[test]
    fn interactive_only_tool_blocked_under_subcall_context() {
        // No tool in default_policies is registered as InteractiveOnly, so
        // install a custom InteractiveOnly policy and verify the Subcall
        // branch Blocks. This mirrors the gate_with() pattern used elsewhere
        // in this module for exercising specific ToolPolicy variants.
        let g = gate_with(ToolPolicy::InteractiveOnly);
        let verdict = g.evaluate("step7_probe", "{}", CallerContext::Subcall);
        assert!(
            matches!(verdict, GateVerdict::Block { .. }),
            "InteractiveOnly tools must Block under Subcall, got {:?}",
            verdict,
        );
    }

    #[test]
    fn unknown_tool_blocked_under_subcall_context() {
        let gate = ToolGate::new();
        let verdict = gate.evaluate("tool_that_does_not_exist", "{}", CallerContext::Subcall);
        assert!(matches!(verdict, GateVerdict::Block { .. }));
    }

    #[test]
    fn heartbeat_allowed_tool_blocked_under_subcall_context() {
        // HeartbeatAllowed tools (e.g. noop) must Block under Subcall —
        // the Subcall branch is explicit in the match arm.
        let g = ToolGate::new();
        for name in &["noop", "write_memory", "send_message"] {
            let verdict = g.evaluate(name, "{}", CallerContext::Subcall);
            assert!(
                matches!(verdict, GateVerdict::Block { .. }),
                "{} should Block under Subcall, got {:?}",
                name,
                verdict,
            );
        }
    }

    #[test]
    fn approval_in_callback_tool_blocked_under_subcall_context() {
        // cancel_intent (the remaining ApprovalInCallback tool) must Block
        // under Subcall — no user present to approve inside a subcall.
        let g = ToolGate::new();
        let verdict = g.evaluate("cancel_intent", "{}", CallerContext::Subcall);
        assert!(
            matches!(verdict, GateVerdict::Block { .. }),
            "cancel_intent should Block under Subcall, got {:?}",
            verdict,
        );
    }

    #[test]
    fn require_approval_tool_blocked_under_subcall_context() {
        // Real RequireApproval-registered tools (destructive system ops) must
        // Block under Subcall rather than return NeedApproval — the subcall
        // runner has no user to deliver the approval prompt to.
        let g = ToolGate::new();
        for name in &["deactivate_skill", "switch_model", "sysadmin_ops"] {
            let verdict = g.evaluate(name, "{}", CallerContext::Subcall);
            assert!(
                matches!(verdict, GateVerdict::Block { .. }),
                "{} should Block (not NeedApproval) under Subcall, got {:?}",
                name,
                verdict,
            );
        }
    }

    #[test]
    fn require_approval_unless_tool_blocked_under_subcall_context_when_args_unsafe() {
        // setup_google has RequireApprovalUnless with an action whitelist
        // ("status"). Under Subcall: unsafe args must Block (not NeedApproval);
        // safe-subset args still Allow (fast path unchanged).
        let g = ToolGate::new();
        let unsafe_args = r#"{"action":"revoke"}"#;
        let unsafe_verdict = g.evaluate("setup_google", unsafe_args, CallerContext::Subcall);
        assert!(
            matches!(unsafe_verdict, GateVerdict::Block { .. }),
            "setup_google with unsafe args should Block under Subcall, got {:?}",
            unsafe_verdict,
        );
        assert_allow(&g.evaluate(
            "setup_google",
            r#"{"action":"status"}"#,
            CallerContext::Subcall,
        ));
    }

    // -----------------------------------------------------------------------
    // Task 2.2 — ToolPolicy::SubcallOnly + subcall tool registration
    // -----------------------------------------------------------------------

    #[test]
    fn policy_subcall_only_matrix() {
        let g = gate_with(ToolPolicy::SubcallOnly);
        assert_block(&g.evaluate("step7_probe", "{}", CallerContext::Interactive));
        assert_block(&g.evaluate("step7_probe", "{}", CallerContext::Callback));
        assert_block(&g.evaluate("step7_probe", "{}", CallerContext::Heartbeat));
        assert_allow(&g.evaluate("step7_probe", "{}", CallerContext::Subcall));
    }

    #[test]
    fn final_answer_blocked_under_interactive() {
        let g = ToolGate::new();
        assert_block(&g.evaluate("final_answer", "{}", CallerContext::Interactive));
    }

    #[test]
    fn final_answer_blocked_under_callback() {
        let g = ToolGate::new();
        assert_block(&g.evaluate("final_answer", "{}", CallerContext::Callback));
    }

    #[test]
    fn final_answer_blocked_under_heartbeat() {
        let g = ToolGate::new();
        assert_block(&g.evaluate("final_answer", "{}", CallerContext::Heartbeat));
    }

    #[test]
    fn final_answer_allowed_under_subcall() {
        let g = ToolGate::new();
        assert_allow(&g.evaluate("final_answer", "{}", CallerContext::Subcall));
    }

    #[test]
    fn spawn_subcall_allowed_in_non_subcall_contexts() {
        // spawn_subcall is Allow everywhere at the gate layer; nesting
        // protection (blocking spawn from inside a subcall) is enforced
        // in the execute_tool dispatch layer in Task 2.3.
        let g = ToolGate::new();
        for ctx in [
            CallerContext::Interactive,
            CallerContext::Callback,
            CallerContext::Heartbeat,
        ] {
            assert_allow(&g.evaluate("spawn_subcall", "{}", ctx));
        }
    }

    #[test]
    fn recall_subcall_allowed_in_every_context() {
        // recall_subcall is safe from any caller — it's a pure read against
        // the registry / DB trace. No nesting protection needed.
        let g = ToolGate::new();
        for ctx in ALL_CALLERS {
            assert_allow(&g.evaluate("recall_subcall", "{}", ctx));
        }
    }

    // ── #9a — data-egress classification ─────────────────────────────────

    #[test]
    fn exfil_sink_set_is_pinned() {
        // The egress sink sets are pinned so any change is a conscious, reviewed
        // diff. If you add/remove a sink in `data_egress`, update these lists AND
        // confirm the guard wiring (orchestrator.rs `GateVerdict::Allow` arm)
        // still covers it.
        let unconditional: Vec<&str> = Vec::new();
        let conditional: Vec<&str> = ["calendar_create_event", "calendar_update_event"]
            .into_iter()
            .collect();
        for name in &unconditional {
            assert_eq!(
                ToolGate::data_egress(name),
                DataEgress::ArbitraryExternalSink,
                "{name} should be an unconditional arbitrary-external sink"
            );
            assert!(ToolGate::is_arbitrary_external_sink(name));
        }
        for name in &conditional {
            assert_eq!(
                ToolGate::data_egress(name),
                DataEgress::ConditionalExternalSink,
                "{name} should be a conditional external sink"
            );
            // Conditional sinks are NOT unconditional sinks.
            assert!(!ToolGate::is_arbitrary_external_sink(name));
        }
        // Nothing OUTSIDE the pinned sets may classify as any kind of external
        // sink. Sweep the entire policy registry so a future tool silently
        // turning into a sink (or a typo'd extra arm) breaks this test.
        for name in default_policies().keys() {
            let egress = ToolGate::data_egress(name);
            let is_sink = matches!(
                egress,
                DataEgress::ArbitraryExternalSink | DataEgress::ConditionalExternalSink
            );
            let expected =
                unconditional.contains(&name.as_str()) || conditional.contains(&name.as_str());
            assert_eq!(
                is_sink, expected,
                "egress classification drift for '{name}' ({egress:?}): only \
                 {unconditional:?} (unconditional) + {conditional:?} (conditional) may be sinks"
            );
        }
    }

    #[test]
    fn arbitrary_external_sinks_are_approval_gated() {
        // Invariant: every exfil sink (conditional or not) must ALSO require
        // explicit approval in the policy table. The autonomous exfil guard is a
        // backstop for the stored-"Always"-approval / heartbeat-allow paths; the
        // front line is still HITL. A sink that were merely `Allow` would be a hole.
        let g = ToolGate::new();
        for name in ["calendar_create_event", "calendar_update_event"] {
            assert!(
                matches!(
                    g.get_policy(name),
                    Some(&ToolPolicy::RequireApproval)
                        | Some(&ToolPolicy::RequireApprovalUnless { .. })
                ),
                "sink '{name}' must be approval-gated, got {:?}",
                g.get_policy(name)
            );
        }
    }

    #[test]
    fn owner_directed_and_readers_are_not_sinks() {
        for name in ["send_message", "notify_user", "google_drive_upload"] {
            assert_eq!(
                ToolGate::data_egress(name),
                DataEgress::OwnerDirected,
                "{name} should be owner-directed, not a sink"
            );
            assert!(!ToolGate::is_arbitrary_external_sink(name));
        }
        for name in [
            "web_search",
            "recall_memory",
            "web_reader",
            "calendar_reader",
            "update_setting",
            // genuine WASM sandbox skill — guarded by its manifest, not here
            "http_api",
        ] {
            assert_eq!(
                ToolGate::data_egress(name),
                DataEgress::Inbound,
                "{name} must be classified Inbound (not any kind of sink)"
            );
        }
    }

    #[test]
    fn conditional_calendar_sink_only_external_when_invites_enabled() {
        use serde_json::json;
        let with_attendees = json!({"summary": "x", "attendees": ["a@b.com"]});
        let no_attendees = json!({"summary": "x"});
        for tool in ["calendar_create_event", "calendar_update_event"] {
            // invite-others OFF → never external (attendee-policy forces self-only)
            assert!(!ToolGate::conditional_sink_is_external(
                tool,
                &with_attendees,
                false
            ));
            // invite-others ON but no attendees supplied → not external
            assert!(!ToolGate::conditional_sink_is_external(
                tool,
                &no_attendees,
                true
            ));
            // invite-others ON + attendees supplied → external sink
            assert!(ToolGate::conditional_sink_is_external(
                tool,
                &with_attendees,
                true
            ));
        }
        // Non-conditional tools never count as conditional sinks.
        assert!(!ToolGate::conditional_sink_is_external(
            "google_drive_upload",
            &with_attendees,
            true
        ));
    }
}
