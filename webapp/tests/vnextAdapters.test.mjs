import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_ADAPTERS_MODULE;
if (!modulePath) {
  throw new Error('VNEXT_ADAPTERS_MODULE is required');
}

const {
  actionSuccessMessage,
  contextualResponseFromVNext,
  ownerQueueFromVNextWork,
} = await import(modulePath);

function workItem(overrides = {}) {
  return {
    id: 'email:42',
    lane: 'now',
    source_kind: 'mail',
    title: 'Email: Billing statement available',
    summary: 'A billing statement is ready.',
    why_it_matters: 'This may need owner review.',
    source: 'gmail',
    source_id: 'gmail-event-42',
    source_event_id: 'gmail-source-42',
    state: 'new',
    urgency: 'high',
    sensitivity: 'S2',
    created_at: '2026-05-11T00:00:00Z',
    updated_at: '2026-05-11T00:01:00Z',
    due_at: null,
    next_safe_action: 'Open message',
    blocked_by: null,
    authority: {
      basis: 'direct_user_request',
      visible_data: ['sender', 'subject'],
      locked_data: ['body'],
      allowed_now: ['open detail', 'draft'],
      requires_data_grant: false,
      requires_mfa: false,
      requires_approval: false,
      expected_receipt: 'Mailbox changes return a provider receipt.',
      source_mandate_id: null,
      audit_refs: ['source:gmail-source-42'],
    },
    recommended_action: {
      id: 'mail.open',
      label: 'Open message',
      kind: 'open_detail',
      route: null,
      requires_data_gate: false,
      requires_mfa: false,
      expected_receipt: 'No external action.',
      tone: 'primary',
    },
    contextual_actions: [
      {
        id: 'mail.archive',
        label: 'Archive',
        kind: 'mail.archive',
        route: null,
        requires_data_gate: false,
        requires_mfa: false,
        expected_receipt: 'Gmail receipt.',
        tone: 'secondary',
      },
    ],
    detail_route: null,
    external_url_policy: 'gmail_link',
    related_work_item_ids: ['task:7'],
    related_receipt_ids: ['receipt:9'],
    receipt_expectation: null,
    route_reason: 'Billing keyword matched.',
    ...overrides,
  };
}

describe('vNext adapters', () => {
  it('maps vNext work projections into owner queue items without dropping authority', () => {
    const queue = ownerQueueFromVNextWork(
      {
        schema_version: 1,
        generated_at: '2026-05-11T00:02:00Z',
        sources: [{ source: 'tasks', status: 'available' }],
        counts: { total: 1 },
        lanes: [{ id: 'now', title: 'Now', subtitle: 'Needs attention', collapsed: false, items: [workItem()] }],
      },
    );

    assert.equal(queue.owner, 'owner');
    assert.equal(queue.generated_at, '2026-05-11T00:02:00Z');
    assert.deepEqual(queue.sources, [{ source: 'tasks', status: 'available' }]);
    assert.deepEqual(queue.counts, { total: 1 });
    assert.equal(queue.items.length, 1);
    assert.equal(queue.items[0].kind, 'mail');
    assert.equal(queue.items[0].source_kind, 'mail');
    assert.equal(queue.items[0].privacy_level, 'private');
    assert.deepEqual(queue.items[0].authority.visible_fields, ['sender', 'subject']);
    assert.deepEqual(queue.items[0].authority.locked_fields, ['body']);
    assert.equal(queue.items[0].authority.expected_receipt, 'Mailbox changes return a provider receipt.');
    assert.equal(queue.items[0].primary_action.label, 'Open message');
    assert.equal(queue.items[0].secondary_actions[0].action_kind, 'mail.archive');
  });

  it('maps structural i18n codes through to owner work items (and null when absent)', () => {
    const coded = ownerQueueFromVNextWork({
      schema_version: 1,
      generated_at: '2026-05-11T00:02:00Z',
      sources: [{ source: 'tasks', status: 'available' }],
      counts: { total: 1 },
      lanes: [{
        id: 'now',
        title: 'Now',
        subtitle: 'Needs attention',
        collapsed: false,
        items: [workItem({
          why_it_matters_code: 'email_basics_review',
          next_safe_action_code: 'email_unread_review',
          blocked_by: 'Reading or classifying email content needs owner permission.',
          blocked_by_code: 'blocked_by_mail_permission',
          title_code: 'approval_title_gmail_read',
          recommended_action: { ...workItem().recommended_action, label_code: 'open_email' },
          contextual_actions: [{ ...workItem().contextual_actions[0], label_code: 'archive' }],
        })],
      }],
    });
    assert.equal(coded.items[0].owner_visible_reason_code, 'email_basics_review');
    assert.equal(coded.items[0].next_safe_action_code, 'email_unread_review');
    assert.equal(coded.items[0].blocked_by_code, 'blocked_by_mail_permission');
    assert.equal(coded.items[0].title_code, 'approval_title_gmail_read');
    assert.equal(coded.items[0].primary_action.label_code, 'open_email');
    assert.equal(coded.items[0].secondary_actions[0].label_code, 'archive');

    const uncoded = ownerQueueFromVNextWork({
      schema_version: 1,
      generated_at: '2026-05-11T00:02:00Z',
      sources: [{ source: 'tasks', status: 'available' }],
      counts: { total: 1 },
      lanes: [{ id: 'now', title: 'Now', subtitle: 'Needs attention', collapsed: false, items: [workItem()] }],
    });
    assert.equal(uncoded.items[0].owner_visible_reason_code, null);
    assert.equal(uncoded.items[0].next_safe_action_code, null);
    assert.equal(uncoded.items[0].blocked_by_code, null);
    assert.equal(uncoded.items[0].title_code, null);
    assert.equal(uncoded.items[0].primary_action.label_code, null);
  });

  it('ignores extra mail_ref on owner work items', () => {
    const queue = ownerQueueFromVNextWork({
      schema_version: 1,
      generated_at: '2026-05-11T00:02:00Z',
      sources: [{ source: 'gmail', status: 'available' }],
      counts: { total: 1 },
      lanes: [{
        id: 'now',
        title: 'Now',
        subtitle: 'Needs attention',
        collapsed: false,
        items: [workItem({ mail_ref: { account: 'work', message_id: 'gmail-msg-42' } })],
      }],
    });

    assert.equal(queue.items[0].mail_ref, undefined);
  });

  it('leaves mail_ref undefined for non-gmail work items (omitted by backend)', () => {
    const queue = ownerQueueFromVNextWork({
      schema_version: 1,
      generated_at: '2026-05-11T00:02:00Z',
      sources: [{ source: 'tasks', status: 'available' }],
      counts: { total: 1 },
      lanes: [{
        id: 'now',
        title: 'Now',
        subtitle: 'Needs attention',
        collapsed: false,
        items: [workItem({ id: 'task:7', source_kind: 'reminder' })],
      }],
    });

    assert.equal(queue.items[0].mail_ref, undefined);
  });

  it('filters retired action-draft items and draft actions out of the main owner queue', () => {
    const queue = ownerQueueFromVNextWork({
      schema_version: 1,
      generated_at: '2026-05-11T00:02:00Z',
      sources: [{ source: 'tasks', status: 'available' }],
      counts: { total: 2 },
      lanes: [{
        id: 'now',
        title: 'Now',
        subtitle: 'Needs attention',
        collapsed: false,
        items: [
          workItem({
            id: 'receipt:647',
            source_kind: 'action_draft',
            title: 'Review reminder draft',
          }),
          workItem({
            id: 'email:43',
            contextual_actions: [
              {
                id: 'draft.save_reminder_task',
                label: 'Save reminder',
                kind: 'draft.save_reminder_task',
                route: null,
                requires_data_gate: false,
                requires_mfa: false,
                expected_receipt: 'Draft receipt.',
                tone: 'secondary',
              },
              {
                id: 'mail.archive',
                label: 'Archive',
                kind: 'mail.archive',
                route: null,
                requires_data_gate: false,
                requires_mfa: false,
                expected_receipt: 'Gmail receipt.',
                tone: 'secondary',
              },
            ],
          }),
        ],
      }],
    });

    assert.deepEqual(queue.items.map((item) => item.id), ['email:43']);
    assert.deepEqual(queue.items[0].secondary_actions.map((action) => action.action_kind), ['mail.archive']);
  });

  it('keeps closed and archived work out of the main owner queue even if a backend lane contains it', () => {
    const queue = ownerQueueFromVNextWork({
      schema_version: 1,
      generated_at: '2026-05-11T00:02:00Z',
      sources: [{ source: 'tasks', status: 'available' }],
      counts: { total: 3 },
      lanes: [{
        id: 'done',
        title: 'Done',
        subtitle: 'Handled',
        collapsed: true,
        items: [
          workItem({ id: 'task:archived', source_kind: 'reminder', state: 'archived', urgency: 'normal' }),
          workItem({ id: 'task:done', source_kind: 'reminder', state: 'done', urgency: 'high' }),
          workItem({ id: 'task:silent', source_kind: 'reminder', state: 'active', urgency: 'silent' }),
        ],
      }],
    });

    assert.deepEqual(queue.items.map((item) => item.id), []);
  });

  it('filters legacy status rollup tasks if an old backend projection returns one', () => {
    const queue = ownerQueueFromVNextWork({
      schema_version: 1,
      generated_at: '2026-05-11T00:02:00Z',
      sources: [{ source: 'tasks', status: 'available' }],
      counts: { total: 2 },
      lanes: [{
        id: 'follow_up',
        title: 'Follow Up',
        subtitle: 'Tracked quietly',
        collapsed: false,
        items: [
          workItem({
            id: 'task:status-rollup',
            source_kind: 'reminder',
            title: '日终状态：磁盘77% / gmail电路 / Nationwide 5/9',
            summary: 'Waiting-for item from the knowledge base. Waiting on: Fix skills / disk cleanup / payment.',
            why_it_matters: 'Waiting on a dependency.',
            state: 'tracking',
            urgency: 'high',
          }),
          workItem({
            id: 'task:commute',
            source_kind: 'reminder',
            title: '回家通勤监控',
            summary: 'Check commute when traffic drops below one hour.',
            why_it_matters: 'Aegis is tracking this because it is part of an open follow-up loop.',
            state: 'tracking',
            urgency: 'high',
          }),
        ],
      }],
    });

    assert.deepEqual(queue.items.map((item) => item.id), ['task:commute']);
  });

  it('normalizes command responses while preserving trace metadata and warnings', () => {
    const result = contextualResponseFromVNext({
      schema_version: 1,
      generated_at: '2026-05-11T00:03:00Z',
      outcome: 'answer_now',
      owner_message: 'Fallback message',
      assistant_response: {
        schema_version: 1,
        response_kind: 'explanation',
        visible_answer: 'Use the usual commute route.',
        turn_id: 'turn-commute',
        latency_ms: 33365,
        draft: null,
        suggested_intent: null,
        authority: { basis: 'direct_user_request' },
        required_gate: null,
        receipt_expectation: 'No external action.',
        citations: ['memory:commute'],
        source_evidence: [{
          kind: 'tool',
          label: 'Memory',
          summary: 'Relevant saved context was used.',
          detail: 'Aegis used recalled commute context.',
          jev: {
            verdict: 'allow',
            audit_tip: 'tip-abc',
          },
        }],
        warnings: ['no_live_traffic'],
        evidence: [{
          id: 'E1',
          tool: 'commute_traffic_monitor',
          label: 'Commute check',
          excerpt: 'About 44 minutes.',
          full_len: 80,
        }],
        unsupported_numbers: ['60–80'],
        trace_id: 'trace-1',
        run_id: 'run-1',
        provider_request_id: 'provider-1',
      },
    });

    assert.equal(result.visible_answer, 'Use the usual commute route.');
    assert.equal(result.turn_id, 'turn-commute');
    assert.equal(result.latency_ms, 33365);
    assert.deepEqual(result.citations, ['memory:commute']);
    assert.equal(result.source_evidence?.[0]?.label, 'Memory');
    assert.equal(result.source_evidence?.[0]?.jev?.verdict, 'allow');
    assert.equal(result.source_evidence?.[0]?.jev?.audit_tip, 'tip-abc');
    assert.equal(result.evidence?.[0]?.id, 'E1');
    assert.deepEqual(result.unsupported_numbers, ['60–80']);
    assert.deepEqual(result.warnings, ['no_live_traffic']);
    assert.equal(result.trace_id, 'trace-1');
    assert.equal(result.run_id, 'run-1');
    assert.equal(result.provider_request_id, 'provider-1');
  });

  it('falls back to owner_message for malformed assistant payloads', () => {
    const result = contextualResponseFromVNext({
      schema_version: 1,
      generated_at: '2026-05-11T00:03:00Z',
      outcome: 'request_clarification',
      owner_message: 'What time should I remind you?',
      assistant_response: { visible_answer: 42 },
      receipt: { id: 'receipt:1', kind: 'receipt', label: 'No action receipt', created_at: null },
    });

    assert.equal(result.response_kind, 'request_clarification');
    assert.equal(result.visible_answer, 'What time should I remind you?');
    assert.equal(result.receipt_expectation, 'No action receipt');
  });

  it('keeps item action success copy receipt-oriented', () => {
    assert.equal(actionSuccessMessage('task.complete'), 'Task updated. Aegis recorded the receipt.');
    assert.equal(actionSuccessMessage('approval.approve'), 'Approval decision sent through the existing approval path.');
  });
});
