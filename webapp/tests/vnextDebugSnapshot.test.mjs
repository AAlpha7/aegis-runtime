import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_DEBUG_SNAPSHOT_MODULE;

if (!modulePath) {
  throw new Error('VNEXT_DEBUG_SNAPSHOT_MODULE must point at the compiled debugSnapshot.js file');
}

const {
  buildVNextDebugSnapshotPayload,
  reviewVNextDebugSnapshot,
} = await import(modulePath);

const workItem = {
  id: 'email:42',
  kind: 'email',
  title: 'Billing statement available',
  summary: 'Maple Lane Music School sent a billing statement.',
  source: 'gmail',
  source_id: '42',
  source_event_id: 'gmail-event:42',
  status: 'unread',
  urgency: 'high',
  sensitivity: 'S2',
  privacy_level: 'private',
  created_at: '2026-05-10T00:00:00Z',
  updated_at: '2026-05-10T00:00:00Z',
  due_at: null,
  primary_action: null,
  secondary_actions: [],
  next_safe_action: 'Open email',
  blocked_by: null,
  receipt_expectation: 'No external action completed yet.',
  authority: null,
  detail_route: '/ui/work/email:42',
  external_url_policy: 'none',
  related_action_ids: [],
  related_receipt_ids: [],
  route_reason: 'Important sender',
  schema_version: 1,
  owner_visible_reason: 'Important sender',
  authority_basis: 'manual_review',
};

function queueWithSelectedItem() {
  return {
    schema_version: 1,
    generated_at: '2026-05-10T00:00:00Z',
    owner: 'did:aegis:telegram:123',
    sources: [{ source: 'gmail', status: 'available', reason: null }],
    counts: { total: 1, email: 1, sensitive: 1 },
    items: [workItem],
  };
}

describe('vNext debug snapshot replay', () => {
  it('builds replayable selected-item payloads without requiring owner login', () => {
    const payload = buildVNextDebugSnapshotPayload({
      view: 'work',
      selectedId: 'email:42',
      queue: queueWithSelectedItem(),
      home: null,
      statusProjection: { schema_version: 1, generated_at: '2026-05-10T00:00:00Z', advanced_diagnostics_available: true },
      settingsProjection: { schema_version: 1, generated_at: '2026-05-10T00:00:00Z', advanced_diagnostics_available: true },
      commandScope: 'item',
      capturedAt: '2026-05-10T00:00:00Z',
      viewport: { width: 390, height: 844, device_pixel_ratio: 3, scroll_y: 0 },
      layout: [
        { selector: '.vnext-detail', top: 120, left: 12, width: 366, height: 520, overflow: 'visible', display: 'grid' },
      ],
      controls: [
        { selector: '.vnext-detail', buttons: ['Open message', 'Ask Aegis', 'Mark read'] },
      ],
    });

    assert.equal(payload.selected_item?.id, 'email:42');
    assert.equal(payload.command_scope, 'item');
    assert.equal(payload.selected_item?.title, 'Billing statement available');
    assert.deepEqual(payload.controls[0].buttons, ['Open message', 'Ask Aegis', 'Mark read']);
    assert.deepEqual(reviewVNextDebugSnapshot(payload), { ok: true, issues: [] });
  });

  it('flags selected-item snapshots that force the owner into another workspace', () => {
    const payload = buildVNextDebugSnapshotPayload({
      view: 'work',
      selectedId: 'email:42',
      queue: queueWithSelectedItem(),
      home: null,
      statusProjection: null,
      settingsProjection: null,
      commandScope: 'item',
      capturedAt: '2026-05-10T00:00:00Z',
      viewport: { width: 390, height: 844, device_pixel_ratio: 3, scroll_y: 0 },
      layout: [
        { selector: '.vnext-detail', top: 120, left: 12, width: 366, height: 520, overflow: 'visible', display: 'grid' },
      ],
      controls: [
        { selector: '.vnext-detail', buttons: ['Ask Aegis', 'Go to approvals'] },
      ],
    });

    const review = reviewVNextDebugSnapshot(payload);

    assert.equal(review.ok, false);
    assert.ok(review.issues.includes('selected_item_requires_manual_redirect'));
  });

  it('replays production-shaped snapshots and flags UX/focus invariants', () => {
    const replay = {
      schema_version: 1,
      captured_at: '2026-05-14T08:45:07.297Z',
      view: 'home',
      selected_id: 'task:5',
      selected_item: {
        id: 'task:5',
        kind: 'task',
        status: 'blocked_by_gate',
        urgency: 'normal',
        sensitivity: 'S2',
        blocked_by: 'Aegis could not run this standing instruction because a required gate blocked the tool or data access.',
        route_reason: "task surfaced from the owner's task queue",
        title: 'Check primary account unread mail with gmail digest processor',
      },
      command_scope: 'item',
      viewport: { width: 402, height: 714, device_pixel_ratio: 3, scroll_y: 0 },
      queue: {
        counts: { total: 71, tasks: 21, sensitive: 71 },
        item_count: 71,
        source_statuses: [{ source: 'gmail', status: 'available', reason: null }],
      },
      today: { decisions: 2, ready: 0, follow_ups: 3, source_issues: 0 },
      diagnostics: { status_available: true, settings_available: true },
      layout: [
        { selector: '.vnext-detail', top: 558, left: 25, width: 352, height: 2580, overflow: 'hidden', display: 'grid' },
      ],
      controls: [],
    };

    const review = reviewVNextDebugSnapshot(replay);

    assert.equal(review.ok, false);
    assert.ok(review.issues.includes('work_detail_taller_than_viewport'));
    assert.ok(review.issues.includes('blocked_item_flow_too_tall'));
    assert.ok(review.issues.includes('work_detail_may_clip_content'));
    assert.ok(review.issues.includes('selected_item_controls_missing'));
    assert.ok(!review.issues.includes('selected_item_without_item_command_scope'));
    assert.ok(!review.issues.includes('selected_item_mismatch'));
  });

  it('flags current production-shaped blocked Work snapshots as flow regressions', () => {
    const replay = {
      schema_version: 1,
      captured_at: '2026-05-17T22:08:31.697Z',
      view: 'work',
      selected_id: 'task:5',
      selected_item: {
        id: 'task:5',
        kind: 'task',
        status: 'blocked_by_gate',
        urgency: 'normal',
        sensitivity: 'S2',
        blocked_by: 'Aegis could not run this standing instruction because a required gate blocked the tool or data access.',
        route_reason: "task surfaced from the owner's task queue",
        title: 'Standing instruction paused before private access',
      },
      command_scope: 'item',
      viewport: { width: 402, height: 714, device_pixel_ratio: 3, scroll_y: 0 },
      queue: {
        counts: { total: 81, high: 20, tasks: 21, sensitive: 81 },
        item_count: 81,
        source_statuses: [{ source: 'gmail', status: 'available', reason: null }],
      },
      today: { decisions: 2, ready: 3, follow_ups: 3, source_issues: 0 },
      diagnostics: { status_available: true, settings_available: true },
      layout: [
        { selector: '.vnext-header', top: 18, left: 12, width: 378, height: 40, overflow: 'visible', display: 'flex' },
        { selector: '.vnext-detail', top: 70, left: 12, width: 378, height: 2105, overflow: 'visible', display: 'grid' },
        { selector: '.vnext-work-brief', top: 2189, left: 12, width: 378, height: 190, overflow: 'visible', display: 'grid' },
        { selector: '.vnext-nav', top: 661, left: 0, width: 402, height: 53, overflow: 'visible', display: 'grid' },
      ],
      controls: [],
    };

    const review = reviewVNextDebugSnapshot(replay);

    assert.equal(review.ok, false);
    assert.ok(review.issues.includes('work_detail_taller_than_viewport'));
    assert.ok(review.issues.includes('blocked_item_flow_too_tall'));
    assert.ok(review.issues.includes('selected_item_controls_missing'));
  });

  it('accepts compact blocked Work snapshots after inline pause controls render', () => {
    const replay = {
      schema_version: 1,
      captured_at: '2026-05-17T23:29:08.000Z',
      view: 'work',
      selected_id: 'task:5',
      selected_item: {
        id: 'task:5',
        kind: 'task',
        status: 'blocked_by_gate',
        urgency: 'high',
        sensitivity: 'S2',
        blocked_by: 'Aegis could not run this standing instruction because a required gate blocked the tool or data access.',
        route_reason: "task surfaced from the owner's task queue",
        title: 'Monitor Gmail',
      },
      command_scope: 'item',
      viewport: { width: 402, height: 714, device_pixel_ratio: 3, scroll_y: 0 },
      queue: {
        counts: { total: 8, high: 1, tasks: 1, sensitive: 8 },
        item_count: 8,
        source_statuses: [{ source: 'gmail', status: 'available', reason: null }],
      },
      today: { decisions: 0, ready: 1, follow_ups: 0, source_issues: 0 },
      diagnostics: { status_available: true, settings_available: true },
      layout: [
        { selector: '.vnext-header', top: 18, left: 12, width: 378, height: 40, overflow: 'visible', display: 'flex' },
        { selector: '.vnext-detail', top: 70, left: 12, width: 378, height: 559, overflow: 'visible', display: 'grid' },
        { selector: '.vnext-work-brief', top: 645, left: 12, width: 378, height: 82, overflow: 'visible', display: 'grid' },
        { selector: '.vnext-nav', top: 661, left: 0, width: 402, height: 53, overflow: 'visible', display: 'grid' },
      ],
      controls: [
        { selector: '.vnext-detail', buttons: ['Explain pause', 'Needed choice', 'Revise rule', 'Open details', 'Close without running', 'Review tomorrow'] },
      ],
    };

    assert.deepEqual(reviewVNextDebugSnapshot(replay), { ok: true, issues: [] });
  });

  it('flags selected item snapshots that show controls but no inline next step', () => {
    const payload = buildVNextDebugSnapshotPayload({
      view: 'work',
      selectedId: 'email:42',
      queue: queueWithSelectedItem(),
      home: null,
      statusProjection: null,
      settingsProjection: null,
      commandScope: 'item',
      capturedAt: '2026-05-10T00:00:00Z',
      viewport: { width: 390, height: 844, device_pixel_ratio: 3, scroll_y: 0 },
      layout: [
        { selector: '.vnext-detail', top: 120, left: 12, width: 366, height: 520, overflow: 'visible', display: 'grid' },
      ],
      controls: [
        { selector: '.vnext-detail', buttons: ['Copy id', 'Open Work'] },
      ],
    });

    const review = reviewVNextDebugSnapshot(payload);

    assert.equal(review.ok, false);
    assert.ok(review.issues.includes('selected_item_requires_manual_redirect'));
    assert.ok(review.issues.includes('selected_item_without_inline_next_step'));
  });

  it('flags item scope regressions from replayed snapshots', () => {
    const payload = buildVNextDebugSnapshotPayload({
      view: 'home',
      selectedId: 'email:42',
      queue: queueWithSelectedItem(),
      home: null,
      statusProjection: null,
      settingsProjection: null,
      commandScope: 'general',
      capturedAt: '2026-05-10T00:00:00Z',
      viewport: { width: 390, height: 844, device_pixel_ratio: 3, scroll_y: 0 },
      layout: [],
    });

    const review = reviewVNextDebugSnapshot(payload);

    assert.equal(review.ok, false);
    assert.ok(review.issues.includes('selected_item_without_item_command_scope'));
  });
});
