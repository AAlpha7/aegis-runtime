import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.TURN_STREAM_MODULE;
if (!modulePath) throw new Error('TURN_STREAM_MODULE is required');
const {
  appendAssistantDelta,
  openTurnStream,
  parseTurnEventData,
  TURN_STREAM_MAX_RECONNECTS,
  TURN_STREAM_BACKOFF_BASE_MS,
} = await import(modulePath);

// A fake EventSource: records listeners, lets the test drive frames + errors.
class FakeEventSource {
  constructor(url) {
    this.url = url;
    this.listeners = new Map();
    this.onerror = null;
    this.readyState = 0; // CONNECTING
    this.closed = false;
    FakeEventSource.instances.push(this);
  }
  addEventListener(type, fn) {
    const arr = this.listeners.get(type) ?? [];
    arr.push(fn);
    this.listeners.set(type, fn ? arr : arr);
  }
  emit(type, data) {
    for (const fn of this.listeners.get(type) ?? []) fn({ data });
  }
  // Simulate a hard network failure (EventSource transitions to CLOSED=2).
  fail() {
    this.readyState = 2;
    if (this.onerror) this.onerror({});
  }
  // Simulate a transient blip that the browser auto-recovers (stays OPEN=1).
  transientBlip() {
    this.readyState = 1;
    if (this.onerror) this.onerror({});
  }
  close() {
    this.closed = true;
    this.readyState = 2;
  }
}
FakeEventSource.instances = [];

function freshFactory() {
  FakeEventSource.instances = [];
  return {
    factory: (url) => new FakeEventSource(url),
    instances: () => FakeEventSource.instances,
  };
}

describe('parseTurnEventData', () => {
  it('parses each backend frame type into a typed event (snake_case fields)', () => {
    assert.deepEqual(
      parseTurnEventData('tool_start', JSON.stringify({ type: 'tool_start', turn_id: 't1', tool_name: 'web_search' })),
      { type: 'tool_start', turn_id: 't1', tool_name: 'web_search' },
    );
    assert.deepEqual(
      parseTurnEventData('tool_done', JSON.stringify({ type: 'tool_done', turn_id: 't1', tool_name: 'web_search' })),
      { type: 'tool_done', turn_id: 't1', tool_name: 'web_search' },
    );
    assert.deepEqual(
      parseTurnEventData('vision_routed', JSON.stringify({ type: 'vision_routed', turn_id: 't1', model: 'gpt-4o' })),
      { type: 'vision_routed', turn_id: 't1', model: 'gpt-4o' },
    );
    assert.deepEqual(
      parseTurnEventData('need_approval', JSON.stringify({ type: 'need_approval', turn_id: 't1', approval_id: 'a1', tool_name: 'shell', summary: 'rm -rf' })),
      { type: 'need_approval', turn_id: 't1', approval_id: 'a1', tool_name: 'shell', summary: 'rm -rf' },
    );
    assert.deepEqual(
      parseTurnEventData('owner_notice', JSON.stringify({ type: 'owner_notice', turn_id: 't1', text: 'hi', kind: 'system' })),
      { type: 'owner_notice', turn_id: 't1', text: 'hi', kind: 'system' },
    );
    assert.deepEqual(
      parseTurnEventData('intent_resolved', JSON.stringify({
        type: 'intent_resolved',
        turn_id: 't1',
        intent_id: 'commute_status',
        label: 'Commute',
        confidence: 0.9,
        needs_review: false,
      })),
      {
        type: 'intent_resolved',
        turn_id: 't1',
        intent_id: 'commute_status',
        label: 'Commute',
        confidence: 0.9,
        needs_review: false,
      },
    );
    assert.deepEqual(
      parseTurnEventData('action_phase', JSON.stringify({
        type: 'action_phase',
        turn_id: 't1',
        phase: 'jev_ask',
        tool_name: 'google_drive_upload',
        detail: 'Jev ask — waiting for owner approval',
      })),
      {
        type: 'action_phase',
        turn_id: 't1',
        phase: 'jev_ask',
        tool_name: 'google_drive_upload',
        detail: 'Jev ask — waiting for owner approval',
      },
    );
    assert.deepEqual(
      parseTurnEventData('assistant_delta', JSON.stringify({ type: 'assistant_delta', turn_id: 't1', text: '早' })),
      { type: 'assistant_delta', turn_id: 't1', text: '早' },
    );
    assert.deepEqual(
      parseTurnEventData('assistant_draft_reset', JSON.stringify({ type: 'assistant_draft_reset', turn_id: 't1' })),
      { type: 'assistant_draft_reset', turn_id: 't1' },
    );
    assert.equal(appendAssistantDelta('早', '安'), '早安');
    assert.equal(appendAssistantDelta('早安', ''), '早安');
    assert.deepEqual(
      parseTurnEventData('done', JSON.stringify({ type: 'done', turn_id: 't1', text: 'final' })),
      { type: 'done', turn_id: 't1', text: 'final' },
    );
    assert.deepEqual(
      parseTurnEventData('done', JSON.stringify({ type: 'done', turn_id: 't1' })),
      { type: 'done', turn_id: 't1' },
    );
  });

  it('falls back to the JSON type when the SSE event name is the default message', () => {
    assert.deepEqual(
      parseTurnEventData('message', JSON.stringify({ type: 'tool_start', turn_id: null, tool_name: 'x' })),
      { type: 'tool_start', turn_id: null, tool_name: 'x' },
    );
  });

  it('drops malformed / unknown frames without throwing', () => {
    assert.equal(parseTurnEventData('tool_start', 'not json'), null);
    assert.equal(parseTurnEventData('message', JSON.stringify({ type: 'bogus' })), null);
    assert.equal(parseTurnEventData('message', JSON.stringify(42)), null);
  });
});

describe('openTurnStream', () => {
  it('builds the per-turn events URL with the encoded token', () => {
    const f = freshFactory();
    openTurnStream('turn/1', 'tok+en', () => {}, () => {}, { eventSourceFactory: f.factory });
    const es = f.instances()[0];
    assert.match(es.url, /\/webapp\/vnext\/turns\/turn%2F1\/events\?stream_token=tok%2Ben$/);
  });

  it('dispatches typed events to onEvent', () => {
    const f = freshFactory();
    const seen = [];
    openTurnStream('t1', 'tok', (e) => seen.push(e), () => {}, { eventSourceFactory: f.factory });
    const es = f.instances()[0];
    es.emit('tool_start', JSON.stringify({ type: 'tool_start', turn_id: 't1', tool_name: 'web_search' }));
    es.emit('vision_routed', JSON.stringify({ type: 'vision_routed', turn_id: 't1', model: 'm' }));
    assert.deepEqual(seen, [
      { type: 'tool_start', turn_id: 't1', tool_name: 'web_search' },
      { type: 'vision_routed', turn_id: 't1', model: 'm' },
    ]);
  });

  it('closes + fires onClose exactly once on a terminal done frame', () => {
    const f = freshFactory();
    let closes = 0;
    openTurnStream('t1', 'tok', () => {}, () => { closes += 1; }, { eventSourceFactory: f.factory });
    const es = f.instances()[0];
    es.emit('done', JSON.stringify({ type: 'done', turn_id: 't1' }));
    es.emit('done', JSON.stringify({ type: 'done', turn_id: 't1' })); // duplicate ignored
    assert.equal(es.closed, true);
    assert.equal(closes, 1);
  });

  it('cleanup fn closes the source and fires onClose once; second call is a no-op', () => {
    const f = freshFactory();
    let closes = 0;
    const cleanup = openTurnStream('t1', 'tok', () => {}, () => { closes += 1; }, { eventSourceFactory: f.factory });
    const es = f.instances()[0];
    cleanup();
    cleanup();
    assert.equal(es.closed, true);
    assert.equal(closes, 1);
  });

  it('does not dispatch events after cleanup', () => {
    const f = freshFactory();
    const seen = [];
    const cleanup = openTurnStream('t1', 'tok', (e) => seen.push(e), () => {}, { eventSourceFactory: f.factory });
    const es = f.instances()[0];
    cleanup();
    es.emit('tool_start', JSON.stringify({ type: 'tool_start', turn_id: 't1', tool_name: 'x' }));
    assert.deepEqual(seen, []);
  });

  it('ignores a transient blip (readyState OPEN) and does not reconnect', () => {
    const f = freshFactory();
    const delays = [];
    openTurnStream('t1', 'tok', () => {}, () => {}, {
      eventSourceFactory: f.factory,
      scheduleReconnect: (fn, d) => delays.push(d),
    });
    f.instances()[0].transientBlip();
    assert.equal(delays.length, 0, 'transient blip must not trigger a reconnect');
    assert.equal(f.instances().length, 1);
  });

  it('reconnects with exponential backoff up to the max, then fires onClose', () => {
    const f = freshFactory();
    const delays = [];
    let closes = 0;
    const pending = [];
    openTurnStream('t1', 'tok', () => {}, () => { closes += 1; }, {
      eventSourceFactory: f.factory,
      scheduleReconnect: (fn, d) => { delays.push(d); pending.push(fn); },
    });
    // Hard-fail the connection MAX+1 times, running the scheduled reconnect each time.
    for (let i = 0; i <= TURN_STREAM_MAX_RECONNECTS; i += 1) {
      const es = f.instances()[f.instances().length - 1];
      es.fail();
      if (pending.length) pending.shift()();
    }
    // Backoff doubles: base, base*2, base*4 (MAX=3 reconnect attempts).
    assert.deepEqual(delays, [
      TURN_STREAM_BACKOFF_BASE_MS,
      TURN_STREAM_BACKOFF_BASE_MS * 2,
      TURN_STREAM_BACKOFF_BASE_MS * 4,
    ]);
    // After exhausting reconnects, the final fail settles the stream.
    assert.equal(closes, 1);
    // One initial + MAX reconnect EventSources were created.
    assert.equal(f.instances().length, TURN_STREAM_MAX_RECONNECTS + 1);
  });
});
