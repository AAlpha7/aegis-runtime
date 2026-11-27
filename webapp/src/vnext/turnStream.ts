// Per-turn live-progress SSE client (Slice 3 of the webapp transport/UX project).
//
// The backend (`core/src/admin.rs::webapp_vnext_turn_events` +
// `core/src/progress_router.rs::progress_event_to_sse_json`) exposes a per-turn
// Server-Sent-Events stream of typed progress frames. Each frame's SSE `event:`
// name equals its `type`, and the `data:` line is a JSON object that always
// carries `{ type, turn_id, ... }`. This module opens that stream, parses each
// frame into a typed {@link TurnEvent}, and drives a caller-supplied callback so
// the chat working-message can show LIVE progress instead of a static spinner.
//
// Scope (Slice 3): the caller handles `tool_start` / `tool_done` /
// `vision_routed` / `done` / `error`. `need_approval` and `owner_notice` frames
// are still parsed into typed events (so the stream never crashes on them) but
// the Slice-3 caller ignores them — the approval card is Slice 4.
//
// `assistant_delta` paints provisional reply text while the model is still
// generating. The command poll remains the stored answer: on success the
// working bubble (including any partial text) is replaced by that one reply.
// A mid-stream failure clears the partial via `assistant_draft_reset` and the
// existing unanswered/retry card. If the EventSource never connects, the
// working message stays generic.

const API_BASE = import.meta.env.VITE_API_BASE ?? '';

/** Maximum reconnect attempts after a network-level EventSource error. */
export const TURN_STREAM_MAX_RECONNECTS = 3;
/** Base backoff (ms) for the first reconnect; doubles each subsequent attempt. */
export const TURN_STREAM_BACKOFF_BASE_MS = 500;

/**
 * A typed progress frame from the per-turn SSE stream. The discriminant `type`
 * matches the backend `SseFrame.event_type` (= the SSE `event:` name), and the
 * field names mirror `progress_event_to_sse_json` EXACTLY (snake_case:
 * `tool_name`, `approval_id`, …). Every frame also carries `turn_id`.
 */
export type TurnEvent =
  | { type: 'tool_start'; turn_id: string | null; tool_name: string }
  | { type: 'tool_done'; turn_id: string | null; tool_name: string }
  | { type: 'vision_routed'; turn_id: string | null; model: string }
  | {
      type: 'need_approval';
      turn_id: string | null;
      approval_id: string | null;
      tool_name: string | null;
      summary: string | null;
    }
  | { type: 'owner_notice'; turn_id: string | null; text: string; kind: string }
  | {
      type: 'intent_resolved';
      turn_id: string | null;
      intent_id: string;
      label: string;
      confidence: number;
      needs_review: boolean;
    }
  | {
      type: 'action_phase';
      turn_id: string | null;
      phase: string;
      tool_name: string | null;
      detail: string | null;
    }
  | { type: 'assistant_delta'; turn_id: string | null; text: string }
  | { type: 'assistant_draft_reset'; turn_id: string | null }
  | { type: 'done'; turn_id: string | null; text?: string }
  // The server does not currently emit an `error` frame (a closed channel yields
  // `done`); this variant exists for forward-compatibility and so a future
  // backend error frame parses into a typed event rather than crashing.
  | { type: 'error'; turn_id: string | null; message: string };

/** The SSE `event:` names the client subscribes to. */
const TURN_EVENT_NAMES = [
  'tool_start',
  'tool_done',
  'vision_routed',
  'need_approval',
  'owner_notice',
  'intent_resolved',
  'action_phase',
  'assistant_delta',
  'assistant_draft_reset',
  'done',
  'error',
] as const;

/**
 * Parse one raw SSE `data` payload (a JSON string) plus its SSE event name into a
 * typed {@link TurnEvent}, or `null` if the payload is not valid JSON / not a
 * recognized frame. Tolerant by design: a malformed frame is dropped, never
 * thrown, so one bad frame cannot tear down the stream.
 *
 * The SSE event `name` is authoritative for the discriminant; the JSON `type`
 * field (when present) is used only as a fallback so the parser still works if a
 * frame arrives over the default `message` event without a named `event:` line.
 */
export function parseTurnEventData(name: string, raw: string): TurnEvent | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof parsed !== 'object' || parsed === null) return null;
  const obj = parsed as Record<string, unknown>;
  const type = (TURN_EVENT_NAMES as readonly string[]).includes(name)
    ? name
    : typeof obj.type === 'string'
      ? obj.type
      : '';
  const turn_id = typeof obj.turn_id === 'string' ? obj.turn_id : null;
  const str = (v: unknown): string => (typeof v === 'string' ? v : '');
  const strOrNull = (v: unknown): string | null => (typeof v === 'string' ? v : null);

  switch (type) {
    case 'tool_start':
      return { type: 'tool_start', turn_id, tool_name: str(obj.tool_name) };
    case 'tool_done':
      return { type: 'tool_done', turn_id, tool_name: str(obj.tool_name) };
    case 'vision_routed':
      return { type: 'vision_routed', turn_id, model: str(obj.model) };
    case 'need_approval':
      return {
        type: 'need_approval',
        turn_id,
        approval_id: strOrNull(obj.approval_id),
        tool_name: strOrNull(obj.tool_name),
        summary: strOrNull(obj.summary),
      };
    case 'owner_notice':
      return { type: 'owner_notice', turn_id, text: str(obj.text), kind: str(obj.kind) };
    case 'intent_resolved':
      return {
        type: 'intent_resolved',
        turn_id,
        intent_id: str(obj.intent_id),
        label: str(obj.label),
        confidence: typeof obj.confidence === 'number' ? obj.confidence : 0,
        needs_review: Boolean(obj.needs_review),
      };
    case 'action_phase':
      return {
        type: 'action_phase',
        turn_id,
        phase: str(obj.phase),
        tool_name: strOrNull(obj.tool_name),
        detail: strOrNull(obj.detail),
      };
    case 'assistant_delta':
      return { type: 'assistant_delta', turn_id, text: str(obj.text) };
    case 'assistant_draft_reset':
      return { type: 'assistant_draft_reset', turn_id };
    case 'done':
      return typeof obj.text === 'string'
        ? { type: 'done', turn_id, text: obj.text }
        : { type: 'done', turn_id };
    case 'error':
      return { type: 'error', turn_id, message: str(obj.message) };
    default:
      return null;
  }
}

/** Minimal structural subtype of the DOM `EventSource` this module depends on. */
export interface TurnEventSourceLike {
  addEventListener(type: string, listener: (event: { data: string }) => void): void;
  onerror: ((this: unknown, ev: unknown) => unknown) | null;
  readyState: number;
  close(): void;
}

/** Factory the caller can override in tests; defaults to the DOM `EventSource`. */
export type TurnEventSourceFactory = (url: string) => TurnEventSourceLike;

const defaultEventSourceFactory: TurnEventSourceFactory = (url) =>
  new EventSource(url) as unknown as TurnEventSourceLike;

export interface OpenTurnStreamOptions {
  /** Override the EventSource constructor (tests inject a fake). */
  eventSourceFactory?: TurnEventSourceFactory;
  /** Override the backoff scheduler (tests run it synchronously). */
  scheduleReconnect?: (fn: () => void, delayMs: number) => void;
}

/**
 * Open the per-turn progress SSE stream for `turnId`, authorized by the
 * short-lived `streamToken` (minted by `mintVNextTurnStreamToken`). Each parsed
 * {@link TurnEvent} is delivered to `onEvent`. On a terminal `done`/`error`
 * frame the stream closes and `onClose` fires once. On a network-level error the
 * stream reconnects with exponential backoff up to {@link TURN_STREAM_MAX_RECONNECTS}
 * attempts; if all reconnects are exhausted, `onClose` fires once.
 *
 * Returns a cleanup function that closes the EventSource and guarantees `onClose`
 * has fired exactly once (idempotent: calling it after a natural close is a
 * no-op). The caller MUST invoke it in its finally/teardown so the stream is
 * always torn down with the turn (no leaked EventSource, no busy-stuck).
 */
/** Append one model delta. Empty pieces leave the provisional text unchanged. */
export function appendAssistantDelta(current: string, delta: string): string {
  if (!delta) return current;
  return current + delta;
}

export function openTurnStream(
  turnId: string,
  streamToken: string,
  onEvent: (e: TurnEvent) => void,
  onClose: () => void,
  options: OpenTurnStreamOptions = {},
): () => void {
  const factory = options.eventSourceFactory ?? defaultEventSourceFactory;
  const scheduleReconnect =
    options.scheduleReconnect ??
    ((fn, delayMs) => {
      setTimeout(fn, delayMs);
    });

  const url = `${API_BASE}/webapp/vnext/turns/${encodeURIComponent(
    turnId,
  )}/events?stream_token=${encodeURIComponent(streamToken)}`;

  let source: TurnEventSourceLike | null = null;
  let reconnects = 0;
  let closed = false;

  // Fire onClose at most once, regardless of which path settled the stream
  // (terminal frame, exhausted reconnects, or caller cleanup).
  const settle = () => {
    if (closed) return;
    closed = true;
    if (source) {
      source.close();
      source = null;
    }
    onClose();
  };

  const handleFrame = (name: string, data: string) => {
    if (closed) return;
    const event = parseTurnEventData(name, data);
    if (!event) return;
    onEvent(event);
    if (event.type === 'done' || event.type === 'error') {
      settle();
    }
  };

  const connect = () => {
    if (closed) return;
    const es = factory(url);
    source = es;
    for (const name of TURN_EVENT_NAMES) {
      es.addEventListener(name, (ev) => handleFrame(name, ev.data));
    }
    // The default (unnamed) `message` event — covers a frame sent without an
    // explicit `event:` line; the JSON `type` then drives the discriminant.
    es.addEventListener('message', (ev) => handleFrame('message', ev.data));
    es.onerror = () => {
      if (closed) return;
      // A live (OPEN) EventSource auto-reconnects internally on a transient
      // blip; only act on hard failures. readyState 2 === CLOSED.
      if (es.readyState !== 2) return;
      es.close();
      if (source === es) source = null;
      if (reconnects >= TURN_STREAM_MAX_RECONNECTS) {
        settle();
        return;
      }
      const delay = TURN_STREAM_BACKOFF_BASE_MS * 2 ** reconnects;
      reconnects += 1;
      scheduleReconnect(() => {
        connect();
      }, delay);
    };
  };

  connect();

  return () => {
    settle();
  };
}
