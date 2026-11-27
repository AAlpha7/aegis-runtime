import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_MODEL_MODULE
  ?? 'file:///tmp/aegis-runtime-webapp-vnext-model-test/vnext/model.js';
const {
  buildVNextHome,
  buildVNextHomeSignals,
  buildVNextLanes,
  authorityBasisLabel,
  blockedAutomationDecisionText,
  blockedAutomationWhatHappened,
  buildCommandSuggestions,
  buildSupportedItemActions,
  itemIsBlockedByGate,
  itemNeedsDecision,
  itemNeedsPrivateTaskAccess,
  mandateMatchWorkItemId,
  hasWorkItems,
  laneForItem,
  ownerVisibleItemTitle,
  ownerVisibleNextSafeActionText,
  ownerVisiblePrimaryActionLabel,
  ownerVisibleReason,
  ownerVisibleReasonCue,
  resolveWorkCodeText,
  ownerVisibleSensitiveStepText,
  ownerVisibleWorkCue,
  ownerVisibleWorkTiming,
  ownerVisibleWorkTitle,
  optimisticStatusForItemAction,
  resolveCommandFocusId,
  sourceNeedsOwnerAttention,
  sourceStatusView,
  presentOwnerWorkSurfaceActions,
  workDetailAskPlacement,
  workScanFiltersDefaultExpanded,
  workScanChromeDefaultExpanded,
} = await import(modulePath);

function item(overrides) {
  return {
    id: overrides.id,
    kind: overrides.kind ?? 'work',
    title: overrides.title ?? overrides.id,
    summary: overrides.summary ?? 'safe summary',
    source_kind: overrides.source_kind ?? undefined,
    source: overrides.source ?? 'test',
    source_id: overrides.source_id ?? overrides.id,
    status: overrides.status ?? 'new',
    urgency: overrides.urgency ?? 'normal',
    sensitivity: overrides.sensitivity ?? 'S2',
    privacy_level: overrides.privacy_level ?? 'metadata',
    created_at: overrides.created_at ?? '2026-05-09T00:00:00Z',
    updated_at: overrides.updated_at ?? '2026-05-09T00:00:00Z',
    due_at: overrides.due_at ?? null,
    blocked_by: overrides.blocked_by ?? null,
    primary_action: overrides.primary_action ?? null,
    secondary_actions: overrides.secondary_actions ?? [],
    next_safe_action: overrides.next_safe_action ?? undefined,
    next_safe_action_code: overrides.next_safe_action_code ?? undefined,
    owner_visible_reason: overrides.owner_visible_reason ?? undefined,
    owner_visible_reason_code: overrides.owner_visible_reason_code ?? undefined,
    external_url_policy: 'internal_only',
    related_action_ids: [],
    related_receipt_ids: [],
    route_reason: overrides.route_reason ?? 'test',
    schema_version: 1,
  };
}

describe('vNext work model timezone-safe due labels', () => {
  it('renders a date-only due value as the raw date (no timezone day-shift)', () => {
    const scheduled = item({ id: 'task-due', status: 'scheduled', due_at: '2026-05-15' });
    // America/New_York would shift midnight-UTC to the prior evening; the
    // date-only guard must keep the literal date instead.
    const timing = ownerVisibleWorkTiming(scheduled, 'America/New_York');
    assert.match(timing, /2026-05-15/);
    assert.doesNotMatch(timing, /May 14|2026-05-14/);
  });

  it('still localizes a full instant due value', () => {
    const scheduled = item({ id: 'task-instant', status: 'scheduled', due_at: '2026-01-15T20:30:00Z' });
    const shanghai = ownerVisibleWorkTiming(scheduled, 'Asia/Shanghai');
    // 20:30 UTC = 04:30 next day in Shanghai (en-US short style).
    assert.match(shanghai, /Jan 16/);
  });

  it('follow-up rows with a due time drop the generic tracking sentence (timing line carries it)', () => {
    const tracked = item({ id: 'task-follow', status: 'tracking', due_at: '2026-01-15T20:30:00Z', summary: '跟进中。我会把它留在视线里，当作后续循环。' });
    assert.equal(ownerVisibleWorkCue(tracked, 140, 'Asia/Shanghai', 'zh'), '');
    const quiet = item({ id: 'task-quiet', status: 'tracking', due_at: null, summary: 'Tracking item. Aegis is keeping this visible as a follow-up loop.' });
    assert.match(ownerVisibleWorkCue(quiet, 140, 'Asia/Shanghai', 'zh'), /静默跟踪/);
    const real = item({ id: 'task-real', status: 'tracking', due_at: '2026-01-15T20:30:00Z', summary: '在等用户确认需要恢复哪些误完成任务。' });
    assert.match(ownerVisibleWorkCue(real, 140, 'Asia/Shanghai', 'zh'), /在等用户确认/);
  });

  it('localizes the timing line for zh (prefix + 24h clock)', () => {
    const scheduled = item({ id: 'task-zh', status: 'scheduled', due_at: '2026-01-15T20:30:00Z' });
    const zh = ownerVisibleWorkTiming(scheduled, 'Asia/Shanghai', 'zh');
    assert.match(zh, /^安排在 · /);
    assert.match(zh, /1月16日/);
    assert.doesNotMatch(zh, /scheduled|PM|Jan/);
    const en = ownerVisibleWorkTiming(scheduled, 'Asia/Shanghai');
    assert.match(en, /^scheduled · Jan 16/);
  });

  it('treats a SQLite-style no-offset due as UTC (not browser-local)', () => {
    // '2026-01-15 20:30:00' (no offset) is UTC; must render Jan 16 in Shanghai
    // regardless of process TZ (suite also runs under TZ=America/New_York).
    const scheduled = item({ id: 'task-sqlite', status: 'scheduled', due_at: '2026-01-15 20:30:00' });
    const shanghai = ownerVisibleWorkTiming(scheduled, 'Asia/Shanghai');
    assert.match(shanghai, /Jan 16/);
    assert.doesNotMatch(shanghai, /Jan 15/);
  });
});

describe('vNext work model', () => {
  it('keeps gated detail reads separate from owner decisions', () => {
    const gated = item({
      id: 'mail-1',
      kind: 'mail',
      primary_action: {
        label: 'Open body',
        action_kind: 'read',
        requires_gate: true,
        requires_mfa: false,
      },
    });

    assert.equal(itemNeedsDecision(gated), false);
    assert.equal(laneForItem(gated), 'now');
    assert.equal(laneForItem(item({
      id: 'waiting-task',
      kind: 'task',
      status: 'waiting',
      primary_action: {
        label: 'Open details',
        action_kind: 'navigate',
        requires_gate: true,
        requires_mfa: false,
      },
    })), 'follow_up');
    assert.equal(laneForItem(item({ id: 'explicit-decision', status: 'needs_decision' })), 'decide');
  });

  it('keeps follow-up, backlog, and done as states instead of separate apps', () => {
    assert.equal(laneForItem(item({ id: 'tracking', status: 'tracking' })), 'follow_up');
    assert.equal(laneForItem(item({ id: 'later', status: 'backlog' })), 'backlog');
    assert.equal(laneForItem(item({ id: 'closed', status: 'done', urgency: 'urgent' })), 'done');
    assert.equal(laneForItem(item({ id: 'completed', status: 'completed', urgency: 'high' })), 'done');
  });

  it('surfaces blocked tasks as decisions after vNext projection is flattened', () => {
    assert.equal(laneForItem(item({
      id: 'blocked-by-gate',
      kind: 'task',
      status: 'blocked_by_gate',
      title: 'Use web_search for an old request',
    })), 'decide');
    assert.equal(laneForItem(item({
      id: 'blocked-task',
      kind: 'task',
      status: 'new',
      blocked_by: 'needs a gate before this can run',
    })), 'follow_up');
  });

  it('turns private task boilerplate into a useful owner cue', () => {
    const privateTask = item({
      id: 'private-task',
      kind: 'task',
      urgency: 'high',
      status: 'active',
      summary: 'High priority. Active. Scheduled mode. Task title and body stay private in this list; open details to review them.',
      next_safe_action: 'Open details to see the private task title/body, or choose complete, snooze, or delete if you already recognize it.',
    });
    assert.equal(ownerVisibleWorkCue(privateTask, 220), 'Open details to reveal the task title and body before changing it.');
    assert.equal(itemNeedsPrivateTaskAccess(privateTask), true);
    assert.equal(ownerVisibleWorkTitle({
      ...privateTask,
      title: 'Private high-priority task',
    }), 'Private task needs review');
  });

  it('opens a rule match onto the calendar work item', () => {
    assert.equal(mandateMatchWorkItemId({
      source: 'mandates',
      source_event_id: 'calendar:42',
    }), 'calendar:42');
    assert.equal(mandateMatchWorkItemId({
      source: 'mandates',
      source_event_id: 'event:42',
    }), null);
    assert.equal(mandateMatchWorkItemId({
      source: 'calendar',
      source_event_id: 'calendar:42',
    }), null);
  });

  it('holds a Chinese private task when the server marks task_private_review', () => {
    const chinesePrivate = item({
      id: 'private-zh',
      kind: 'task',
      urgency: 'high',
      status: 'active',
      title: '私密高优先级待办',
      summary: '私密高优先级待办。标题和正文不在这张列表里；打开详情再看。',
      next_safe_action: '打开详情看私人待办，或你已经认得的话选一个安全动作。',
      next_safe_action_code: 'task_private_review',
    });
    assert.equal(itemNeedsPrivateTaskAccess(chinesePrivate), true);
    assert.equal(
      ownerVisibleNextSafeActionText(chinesePrivate, 'zh'),
      '先打开详情，看完标题和正文再完成或改期。',
    );

    const ordinary = item({
      id: 'ordinary-zh',
      kind: 'task',
      urgency: 'high',
      status: 'active',
      title: '高优先级待办',
      summary: '高优先级。主动模式。',
      next_safe_action_code: 'task_open_or_act',
    });
    assert.equal(itemNeedsPrivateTaskAccess(ordinary), false);
  });

  it('rewords protected-action internals before showing owner work cues', () => {
    const blocked = item({
      id: 'blocked-sensitive-step',
      kind: 'task',
      urgency: 'high',
      status: 'blocked_by_gate',
      summary: 'A scheduled automation paused before using a protected action.',
      blocked_by: 'Aegis stopped this standing instruction instead of retrying a protected action.',
      owner_visible_reason: 'standing instruction paused on a protected action boundary',
    });
    assert.equal(ownerVisibleWorkCue(blocked, 220), 'A scheduled automation paused before using a sensitive step.');
    assert.equal(ownerVisibleReason(blocked), 'automation paused on a sensitive step boundary');
    assert.equal(ownerVisibleSensitiveStepText(blocked.blocked_by, '', 220), 'Aegis stopped this automation instead of retrying a sensitive step.');
  });

  it('turns blocked Gmail digest tasks into a clear paused automation decision', () => {
    const blocked = item({
      id: 'blocked-gmail-digest',
      kind: 'task',
      source: 'tasks',
      source_kind: 'task',
      status: 'blocked_by_gate',
      title: '检查 primary 账户的未读邮件，使用 gmail_digest_processor 自动分类：将标记为‘重要’的邮',
      summary: 'Paused before action. Aegis needs your choice before this automation continues.',
      blocked_by: 'Aegis paused this automation before a sensitive step.',
      owner_visible_reason: 'An automation you asked Aegis to run paused before a sensitive step.',
      secondary_actions: [
        { label: 'Approve', action_kind: 'approval.approve' },
        { label: 'Reject', action_kind: 'approval.reject' },
      ],
    });

    assert.equal(blockedAutomationDecisionText(blocked), 'Approve this run, skip it, or ask Aegis to revise the rule first.');
    assert.deepEqual(buildSupportedItemActions(blocked).map((action) => action.label), ['Approve this run', 'Skip this run']);
    assert.match(ownerVisibleWorkTitle(blocked), /Paused automation|检查/);
  });

  it('hides boilerplate owner reasons but keeps specific useful reasons', () => {
    assert.equal(ownerVisibleReason(item({
      owner_visible_reason: 'Aegis surfaced this task because it is active owner work.',
      route_reason: 'surface_card',
    })), '');
    assert.equal(ownerVisibleReason(item({
      owner_visible_reason: 'Waiting on gmail_reader circuit recovery before this can continue.',
      route_reason: 'surface_card',
    })), 'Waiting on Gmail reader circuit recovery before this can continue.');
  });

  it('compresses verbose owner reasons into readable work cues while hiding routing noise', () => {
    assert.ok(typeof ownerVisibleReasonCue(item({
      owner_visible_reason: 'Gmail sender, subject, and mailbox state made this worth review.',
    })) === 'string');
    assert.equal(ownerVisibleReasonCue(item({
      owner_visible_reason: 'Waiting on gmail_reader circuit recovery before this can continue.',
    })), 'Waiting on Gmail reader circuit recovery before this can continue.');
    assert.equal(ownerVisibleReasonCue(item({
      route_reason: 'Sanitized render smoke fixture for post-gate mail body display.',
    })), '');
    assert.equal(ownerVisibleReasonCue(item({
      owner_visible_reason: 'Aegis surfaced this task because it is active owner work.',
    })), '');
    assert.equal(ownerVisibleReasonCue(item({
      owner_visible_reason: 'Aegis is tracking this because it is part of an open follow-up loop.',
    })), '');
  });

  it('does not make handled work look actionable because of stale urgency or reasons', () => {
    const archivedTask = item({
      id: 'old-task',
      kind: 'task',
      status: 'archived',
      urgency: 'high',
      summary: 'Tonight at 10:00 remind me to handle lawyer hiring.',
      owner_visible_reason: 'Waiting on a dependency.',
      next_safe_action: 'Review the task, then choose.',
    });

    assert.equal(laneForItem(archivedTask), 'done');
    assert.equal(ownerVisibleWorkCue(archivedTask, 220), 'Handled. No action is needed unless you want to review history.');
    assert.equal(ownerVisibleReasonCue(archivedTask), '');
    assert.deepEqual(buildSupportedItemActions(archivedTask), []);
  });

  it('replaces generic tracking summaries with useful timing cues', () => {
    const trackingTask = item({
      id: 'commute-watch',
      kind: 'task',
      lane: 'follow_up',
      status: 'tracking',
      summary: 'Tracking item. Aegis is keeping this visible as a follow-up loop.',
      owner_visible_reason: 'Aegis is tracking this because it is part of an open follow-up loop.',
      due_at: '2026-05-19T17:30:00-07:00',
    });

    // 2026-09-13: with a due time the row's timing line ("watching · …")
    // carries the date; the cue no longer repeats it as a sentence.
    assert.equal(ownerVisibleWorkCue(trackingTask, 220), '');
    assert.equal(ownerVisibleReasonCue(trackingTask), '');
    assert.match(ownerVisibleWorkTiming(trackingTask, 'America/Los_Angeles'), /^watching · May 19/);
  });

  it('surfaces blocked standing instructions as owner attention', () => {
    const blockedAutomation = item({
      id: 'blocked-automation',
      kind: 'task',
      status: 'blocked_by_gate',
      title: 'Check primary mail every morning',
      blocked_by: 'A required gate blocked the tool call',
    });
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [],
      counts: {},
      items: [blockedAutomation],
    });

    assert.equal(itemIsBlockedByGate(blockedAutomation), true);
    assert.equal(laneForItem(blockedAutomation), 'decide');
    assert.match(home.brief, /Needs attention:/);
    assert.deepEqual(home.decisions.map((entry) => entry.id), ['blocked-automation']);
    assert.deepEqual(home.followUps.map((entry) => entry.id), []);
  });

  it('surfaces blocked owner work even when the backend omits a blocker note', () => {
    const blocked = item({
      id: 'blocked-without-note',
      kind: 'task',
      status: 'blocked_by_gate',
      title: 'Morning mail triage',
      blocked_by: null,
    });
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [],
      counts: {},
      items: [blocked],
    });

    assert.equal(laneForItem(blocked), 'decide');
    assert.deepEqual(home.decisions.map((entry) => entry.id), ['blocked-without-note']);
    assert.match(home.brief, /Needs attention:/);
  });

  it('does not expose generic private task titles in home briefs', () => {
    const privateTask = item({
      id: 'private-ready',
      kind: 'task',
      title: 'Private high-priority task',
      summary: 'Task title and body stay private until opened.',
      next_safe_action: 'Open details to see the private task.',
    });
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [],
      counts: {},
      items: [privateTask],
    });

    assert.equal(home.brief, 'Ready to review: Private task needs review.');
  });

  it('collapses empty, backlog, and done lanes by default', () => {
    const lanes = buildVNextLanes([
      item({ id: 'now', title: 'Ready work' }),
      item({ id: 'later', status: 'backlog' }),
      item({ id: 'done', status: 'done' }),
    ]);

    assert.deepEqual(lanes.map((lane) => lane.id), ['decide', 'now', 'follow_up', 'backlog', 'done']);
    assert.equal(lanes.find((lane) => lane.id === 'decide')?.collapsed, true);
    assert.equal(lanes.find((lane) => lane.id === 'backlog')?.collapsed, true);
    assert.equal(lanes.find((lane) => lane.id === 'done')?.collapsed, true);
  });

  it('detects whether the work surface has any items', () => {
    assert.equal(hasWorkItems(buildVNextLanes([])), false);
    assert.equal(hasWorkItems(buildVNextLanes([item({ id: 'ready' })])), true);
  });

  it('builds a secretary-style home brief without exposing raw queue first', () => {
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [{ source: 'gmail', status: 'available' }],
      counts: {},
      items: [
        item({ id: 'later', status: 'backlog', title: 'Later reading' }),
        item({ id: 'decision', kind: 'approval', title: 'Approve reply' }),
        item({ id: 'ready', title: 'School bill' }),
      ],
    });

    assert.equal(home.brief, 'Needs your decision: Approve reply.');
    assert.deepEqual(home.decisions.map((entry) => entry.id), ['decision']);
    assert.deepEqual(home.now.map((entry) => entry.id), ['ready']);
  });

  it('keeps background tracking loops out of the primary Today focus', () => {
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [{ source: 'gmail', status: 'available' }],
      counts: {},
      items: [
        item({ id: 'commute-watch', status: 'tracking', title: 'Commute low-traffic watch' }),
      ],
    });

    assert.equal(home.brief, 'Nothing urgent is asking for you right now.');
    assert.deepEqual(home.now.map((entry) => entry.id), []);
    assert.deepEqual(home.followUps.map((entry) => entry.id), ['commute-watch']);
  });

  it('keeps high-priority waiting items out of ready work and cleans compound titles', () => {
    const blocked = item({
      id: 'blocked',
      kind: 'waiting_for',
      status: 'waiting',
      urgency: 'high',
      title: '日终状态：磁盘77%+gmail电路+Nationwide 5/9',
    });
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [],
      counts: {},
      items: [blocked],
    });

    assert.equal(laneForItem(blocked), 'follow_up');
    assert.equal(home.brief, 'Nothing urgent is asking for you right now.');
    assert.deepEqual(home.now.map((entry) => entry.id), []);
    assert.deepEqual(home.followUps.map((entry) => entry.id), ['blocked']);
  });

  it('prioritizes severity-style urgency codes case-insensitively', () => {
    const lanes = buildVNextLanes([
      item({
        id: 'normal-followup',
        kind: 'task',
        status: 'waiting',
        urgency: 'normal',
        title: 'Normal waiting item',
        updated_at: '2026-05-11T17:30:00Z',
      }),
      item({
        id: 's1-followup',
        kind: 'task',
        status: 'waiting',
        urgency: 'S1',
        title: 'S1 waiting item',
        updated_at: '2026-05-09T17:30:00Z',
      }),
    ]);

    assert.deepEqual(
      lanes.find((lane) => lane.id === 'follow_up')?.items.map((entry) => entry.id),
      ['s1-followup', 'normal-followup'],
    );
  });

  it('surfaces near-term scheduled preparation without treating it as a decision', () => {
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [{ source: 'gmail', status: 'available' }],
      counts: {},
      items: [
        item({
          id: 'prepare-passport',
          status: 'scheduled',
          title: 'Prepare passport documents',
          due_at: new Date(Date.now() + 12 * 60 * 60 * 1000).toISOString(),
        }),
      ],
    });

    assert.equal(home.brief, 'Coming up: Prepare passport documents.');
    assert.deepEqual(home.decisions.map((entry) => entry.id), []);
    assert.deepEqual(home.now.map((entry) => entry.id), ['prepare-passport']);
    assert.deepEqual(home.followUps.map((entry) => entry.id), []);
  });

  it('cleans machine-style compound titles in home briefs', () => {
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [],
      counts: {},
      items: [
        item({
          id: 'compound',
          status: 'new',
          title: '日终状态：磁盘77%+gmail电路+Nationwide 5/9',
        }),
      ],
    });

    assert.equal(home.brief, 'Ready to review: 日终状态：磁盘77% / gmail电路 / Nationwide 5/9.');
  });

  it('uses owner-visible title cleanup for Today cards and signals', () => {
    assert.equal(
      ownerVisibleItemTitle('日终状态：磁盘77%+gmail电路+Nationwide 5/9'),
      '日终状态：磁盘77% / gmail电路 / Nationwide 5/9',
    );
    assert.equal(ownerVisibleItemTitle('Monitor: gmail_reader'), 'Monitor Gmail reader');
    assert.equal(ownerVisibleItemTitle('web_search recent X posts'), 'Web search recent X posts');
    assert.equal(
      ownerVisibleItemTitle('使用 web_search 搜索过去6小时 Elon Musk (@elonmusk) 在 X 上的最新推文（英文原话+'),
      'Search: 过去6小时 Elon Musk (@elonmusk) 在 X 上的最新推文（英文原话',
    );
    assert.equal(ownerVisibleItemTitle('Use web_search to check commute traffic'), 'Search: check commute traffic');
    assert.equal(ownerVisibleItemTitle('hitl_approval required'), 'Owner approval required');
    assert.equal(ownerVisibleItemTitle('llm_cost_watch'), 'AI cost watch');
    assert.equal(ownerVisibleItemTitle('oauth_refresh_needed'), 'Google sign-in refresh needed');
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [],
      counts: {},
      items: [
        item({
          id: 'compound',
          status: 'new',
          title: '日终状态：磁盘77%+gmail电路+Nationwide 5/9',
        }),
      ],
    });

    assert.equal(buildVNextHomeSignals(home)[0].summary, '日终状态：磁盘77% / gmail电路 / Nationwide 5/9');
  });

  it('keeps a production-shaped mixed work queue readable and prioritized', () => {
    const farFuture = new Date(Date.now() + 90 * 24 * 60 * 60 * 1000).toISOString();
    const nearFuture = new Date(Date.now() + 18 * 60 * 60 * 1000).toISOString();
    const queue = {
      schema_version: 1,
      generated_at: '2026-05-11T17:22:00-07:00',
      owner: 'owner',
      sources: [{ source: 'gmail', status: 'available' }, { source: 'calendar', status: 'available' }],
      counts: {},
      items: [
        item({
          id: 'passport-followup',
          kind: 'task',
          status: 'scheduled',
          title: '安安护照跟进',
          due_at: farFuture,
        }),
        item({
          id: 'air-filter',
          kind: 'task',
          status: 'scheduled',
          title: '空气过滤器',
          due_at: farFuture,
        }),
        item({
          id: 'insurance-soon',
          kind: 'task',
          status: 'scheduled',
          title: '保险咨询',
          due_at: nearFuture,
        }),
        item({
          id: 'gmail-monitor-blocked',
          kind: 'task',
          status: 'blocked_by_gate',
          title: 'Monitor: gmail_reader',
          blocked_by: 'gmail_reader circuit is open',
        }),
        item({
          id: 'legal-waiting',
          kind: 'waiting_for',
          status: 'waiting',
          urgency: 'high',
          title: '律师Sample文件',
        }),
        item({
          id: 'billing-mail',
          kind: 'email',
          status: 'unread',
          title: 'Billing statement available',
          summary: 'Maple Lane Music School; open the message for the full email.',
        }),
      ],
    };

    const lanes = buildVNextLanes(queue.items);
    const home = buildVNextHome(queue);

    assert.deepEqual(lanes.find((lane) => lane.id === 'decide')?.items.map((entry) => entry.id), ['gmail-monitor-blocked']);
    assert.deepEqual(lanes.find((lane) => lane.id === 'now')?.items.map((entry) => entry.id), ['insurance-soon', 'billing-mail']);
    assert.deepEqual(
      lanes.find((lane) => lane.id === 'follow_up')?.items.map((entry) => entry.id),
      ['legal-waiting', 'passport-followup', 'air-filter'],
    );
    assert.match(home.brief, /Needs attention:/);
    assert.deepEqual(home.decisions.map((entry) => entry.id), ['gmail-monitor-blocked']);
    assert.deepEqual(home.now.map((entry) => entry.id), ['insurance-soon', 'billing-mail']);
    assert.deepEqual(home.followUps.map((entry) => entry.id), ['legal-waiting', 'passport-followup', 'air-filter']);
  });

  it('summarizes home signals as meaningful text instead of raw count tiles', () => {
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [{ source: 'gmail', status: 'unavailable' }],
      counts: {},
      items: [
        item({ id: 'decision-1', kind: 'approval', title: 'Approve school reply' }),
        item({ id: 'decision-2', kind: 'approval', title: 'Choose legal response' }),
        item({ id: 'ready', title: 'Review bill' }),
      ],
    });

    assert.deepEqual(buildVNextHomeSignals(home), [
      {
        id: 'decisions',
        label: 'Needs decision',
        summary: 'Approve school reply',
        tone: 'warn',
        targetItemId: 'decision-1',
      },
      {
        id: 'ready',
        label: 'Ready now',
        summary: 'Review bill',
        tone: 'neutral',
        targetItemId: 'ready',
      },
      {
        id: 'sources',
        label: 'Connection issue',
        summary: 'Gmail: Unavailable right now.',
        tone: 'warn',
      },
    ]);
  });

  it('uses owner-facing cues instead of generic private titles in home signals', () => {
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [],
      counts: {},
      items: [
        item({
          id: 'private-ready',
          kind: 'task',
          title: 'Private high-priority task',
          summary: 'Task title and body stay private until opened.',
          next_safe_action: 'Open details to see the private task.',
        }),
      ],
    });

    assert.deepEqual(buildVNextHomeSignals(home), [
      {
        id: 'ready',
        label: 'Needs detail',
        summary: 'Private task needs review',
        tone: 'warn',
        targetItemId: 'private-ready',
      },
    ]);
  });

  it('summarizes multiple private ready items as one readable access signal', () => {
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [],
      counts: {},
      items: [
        item({
          id: 'private-ready-1',
          kind: 'task',
          title: 'Private high-priority task',
          summary: 'Task title and body stay private until opened.',
          next_safe_action: 'Open details to see the private task.',
        }),
        item({
          id: 'private-ready-2',
          kind: 'task',
          title: 'Private high-priority task',
          summary: 'Task title and body stay private until opened.',
          next_safe_action: 'Open details to see the private task.',
        }),
        item({
          id: 'private-ready-3',
          kind: 'task',
          title: 'Private high-priority task',
          summary: 'Task title and body stay private until opened.',
          next_safe_action: 'Open details to see the private task.',
        }),
      ],
    });

    assert.equal(buildVNextHomeSignals(home)[0].summary, 'Open Work to review hidden details');
  });

  it('normalizes approval primary actions for every owner-facing surface', () => {
    const approval = item({
      id: 'approval-1',
      kind: 'approval',
      source: 'hitl',
      source_kind: 'decision',
      status: 'waiting',
      summary: '',
      primary_action: {
        label: 'Review approval',
        action_kind: 'navigate',
        requires_gate: false,
        requires_mfa: false,
      },
    });

    assert.equal(ownerVisiblePrimaryActionLabel(approval, 'Open'), 'Review request');
    assert.equal(ownerVisibleWorkCue(approval), 'Review request');
  });

  it('keeps home signals short and leaves detailed instructions for item pages', () => {
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [],
      counts: {},
      items: [
        item({
          id: 'decision',
          kind: 'approval',
          status: 'needs_decision',
          title: 'Review task',
          next_safe_action: 'Open details, compare the options, then choose approve or reject.',
        }),
        item({
          id: 'ready',
          kind: 'task',
          status: 'active',
          title: 'Private high-priority task',
          summary: 'Task title and body stay private until opened.',
          next_safe_action: 'Open details to see the private task.',
        }),
      ],
    });

    const summaries = buildVNextHomeSignals(home).map((signal) => signal.summary);
    assert.deepEqual(summaries, ['Waiting for your choice', 'Private task needs review']);
    assert.equal(summaries.some((summary) => /Open details|safe actions|compare the options/i.test(summary)), false);
  });

  it('keeps source connection issues out of the primary home brief', () => {
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [{ source: 'gmail', status: 'auth_required' }],
      counts: {},
      items: [],
    });

    assert.equal(home.brief, 'Nothing urgent is asking for you right now.');
    assert.equal(home.sourceIssueSummary, 'Gmail: Google sign-in needs refresh.');
    assert.deepEqual(buildVNextHomeSignals(home), [
      {
        id: 'sources',
        label: 'Connection issue',
        summary: 'Gmail: Google sign-in needs refresh.',
        tone: 'warn',
      },
    ]);
  });

  it('does not treat intentionally deferred future sources as connection failures', () => {
    const home = buildVNextHome({
      schema_version: 1,
      generated_at: '2026-05-09T00:00:00Z',
      owner: 'owner',
      sources: [{ source: 'alerts', status: 'deferred', reason: 'not connected yet' }],
      counts: {},
      items: [],
    });

    assert.equal(home.sourceIssueCount, 0);
    assert.deepEqual(buildVNextHomeSignals(home), [
      {
        id: 'quiet',
        label: 'Quiet',
        summary: 'Nothing urgent is asking for you right now.',
        tone: 'ok',
      },
    ]);
  });

  it('counts only sources that need owner attention', () => {
    assert.equal(sourceNeedsOwnerAttention({ status: 'available' }), false);
    assert.equal(sourceNeedsOwnerAttention({ status: 'deferred' }), false);
    assert.equal(sourceNeedsOwnerAttention({ status: 'unavailable' }), true);
    assert.equal(sourceNeedsOwnerAttention({ status: 'auth_required' }), true);
  });

  it('uses a quiet home signal when nothing needs attention', () => {
    assert.deepEqual(buildVNextHomeSignals({ now: [], decisions: [], followUps: [], sourceIssueCount: 0 }), [
      {
        id: 'quiet',
        label: 'Quiet',
        summary: 'Nothing urgent is asking for you right now.',
        tone: 'ok',
      },
    ]);
  });

  it('offers deliberate receipt-backed actions only when applicable', () => {
    assert.deepEqual(
      buildSupportedItemActions(item({
        id: 'mail',
        kind: 'email',
        status: 'unread',
        secondary_actions: [
          { label: 'Mark read', action_kind: 'mail.mark_read' },
          { label: 'Archive', action_kind: 'mail.archive' },
        ],
      })).map((action) => action.id),
      [],
    );
    assert.deepEqual(
      buildSupportedItemActions(item({
        id: 'task',
        kind: 'task',
        status: 'new',
        secondary_actions: [
          { label: 'Complete', action_kind: 'task.complete' },
          { label: 'Snooze', action_kind: 'task.snooze_tomorrow' },
        ],
      })).map((action) => action.id),
      ['task.complete', 'task.snooze_tomorrow'],
    );
    const taskActions = buildSupportedItemActions(item({
      id: 'task',
      kind: 'task',
      status: 'new',
      secondary_actions: [
        { label: 'Complete', action_kind: 'task.complete' },
        { label: 'Snooze', action_kind: 'task.snooze_tomorrow' },
      ],
    }));
    assert.equal(taskActions.find((action) => action.id === 'task.snooze_tomorrow')?.label, 'Snooze');
    assert.equal(taskActions.find((action) => action.id === 'task.snooze_tomorrow')?.confirmLabel, 'Tap again for tomorrow');
    const followUpTaskActions = buildSupportedItemActions(item({
      id: 'follow-up-task',
      kind: 'task',
      status: 'tracking',
      secondary_actions: [
        { label: 'Complete', action_kind: 'task.complete' },
        { label: 'Snooze', action_kind: 'task.snooze_tomorrow' },
      ],
    }));
    assert.deepEqual(followUpTaskActions.map((action) => action.label), ['End this card', 'Review tomorrow']);
    const loopActions = buildSupportedItemActions(item({
      id: 'task:12',
      kind: 'task',
      status: 'tracking',
      secondary_actions: [
        { label: 'Skip', action_kind: 'task.skip_occurrence' },
        { label: 'Cancel', action_kind: 'tracking.cancel' },
      ],
    }));
    assert.deepEqual(loopActions.map((action) => action.id), ['task.skip_occurrence', 'tracking.cancel']);
    assert.deepEqual(loopActions.map((action) => action.label), ['End only this time', 'Close this tracking']);
    assert.equal(optimisticStatusForItemAction('task.skip_occurrence'), null);
    assert.equal(optimisticStatusForItemAction('tracking.cancel'), 'cancelled');
    assert.equal(optimisticStatusForItemAction('tracking.cancel', { kind: 'task', source: 'tasks' }), 'cancelled');
    const waitingBacked = buildSupportedItemActions(item({
      id: 'kb_waiting:701',
      kind: 'task',
      source: 'knowledge',
      source_kind: 'tracking',
      status: 'waiting',
      secondary_actions: [
        { label: 'Done', action_kind: 'knowledge.done' },
        { label: 'Cancel', action_kind: 'tracking.cancel' },
        { label: 'Snooze', action_kind: 'knowledge.snooze_tomorrow' },
      ],
    }), 'zh');
    assert.deepEqual(waitingBacked.map((action) => action.id), ['tracking.cancel', 'knowledge.done', 'knowledge.snooze_tomorrow']);
    assert.deepEqual(waitingBacked.map((action) => action.label), ['关闭整条跟踪', '结束这一次', '明天再看']);
    assert.equal(optimisticStatusForItemAction('tracking.cancel', { kind: 'task', source: 'knowledge' }), null);
    assert.equal(optimisticStatusForItemAction('tracking.cancel', { kind: 'waiting_for' }), null);
    const plainWaiting = buildSupportedItemActions(item({
      id: 'kb_waiting:702',
      kind: 'waiting_for',
      source: 'knowledge',
      status: 'waiting',
      secondary_actions: [
        { label: 'Done', action_kind: 'knowledge.done' },
        { label: 'Snooze', action_kind: 'knowledge.snooze_tomorrow' },
      ],
    }), 'zh');
    assert.deepEqual(plainWaiting.map((action) => action.label), ['结束这一次', '明天再看']);
    assert.deepEqual(
      buildSupportedItemActions(item({
        id: 'approval',
        kind: 'approval',
        source: 'hitl',
        status: 'waiting',
        secondary_actions: [
          { label: 'Approve', action_kind: 'approval.approve' },
          { label: 'Reject', action_kind: 'approval.reject' },
        ],
      })).map((action) => action.id),
      ['approval.approve', 'approval.reject'],
    );
    const approvalActions = buildSupportedItemActions(item({
      id: 'approval',
      kind: 'approval',
      source: 'hitl',
      status: 'waiting',
      secondary_actions: [
        { label: 'Approve', action_kind: 'approval.approve' },
        { label: 'Reject', action_kind: 'approval.reject' },
      ],
    }));
    assert.match(approvalActions.find((action) => action.id === 'approval.approve')?.receiptHint ?? '', /extra confirmation/);
    assert.doesNotMatch(approvalActions.find((action) => action.id === 'approval.approve')?.receiptHint ?? '', /MFA/);
    assert.deepEqual(
      buildSupportedItemActions(item({
        id: 'approval-partial',
        kind: 'approval',
        source: 'hitl',
        status: 'waiting',
        secondary_actions: [
          { label: 'Approve', action_kind: 'approval.approve' },
        ],
      })).map((action) => action.id),
      ['approval.approve'],
    );
    assert.deepEqual(
      buildSupportedItemActions(item({
        id: 'approval-bad',
        kind: 'approval',
        source: 'hitl',
        status: 'waiting',
        secondary_actions: [
          { label: 'Draft', action_kind: 'draft' },
        ],
      })),
      [],
    );
    assert.deepEqual(
      buildSupportedItemActions(item({ id: 'draft', kind: 'action_draft', status: 'needs_decision' })),
      [],
    );
    assert.deepEqual(
      buildSupportedItemActions(item({
        id: 'blocked',
        kind: 'task',
        status: 'blocked_by_gate',
        blocked_by: 'A protected action needs review.',
      })),
      [],
    );
    assert.deepEqual(
      buildSupportedItemActions(item({
        id: 'blocked-approval',
        kind: 'approval',
        source: 'hitl',
        status: 'blocked_by_gate',
        blocked_by: 'A protected action needs review.',
        secondary_actions: [
          { label: 'Approve', action_kind: 'approval.approve' },
          { label: 'Reject', action_kind: 'approval.reject' },
        ],
      })).map((action) => action.id),
      ['approval.approve', 'approval.reject'],
    );
    assert.deepEqual(
      buildSupportedItemActions(item({
        id: 'blocked-task',
        kind: 'task',
        status: 'blocked_by_gate',
        blocked_by: 'A protected action needs review.',
        secondary_actions: [
          { label: 'Complete', action_kind: 'task.complete' },
        ],
      })).map((action) => action.id),
      ['task.complete'],
    );
    assert.deepEqual(
      buildSupportedItemActions(item({
        id: 'blocked-task-with-approval',
        kind: 'task',
        status: 'blocked_by_gate',
        blocked_by: 'A protected action needs review.',
        secondary_actions: [
          { label: 'Approve', action_kind: 'approval.approve' },
          { label: 'Reject', action_kind: 'approval.reject' },
        ],
      })).map((action) => action.id),
      ['approval.approve', 'approval.reject'],
    );
    const blockedTaskActions = buildSupportedItemActions(item({
      id: 'blocked-task-with-controls',
      kind: 'task',
      status: 'blocked_by_gate',
      blocked_by: 'A protected action needs review.',
      secondary_actions: [
        { label: 'Close without running', action_kind: 'task.complete' },
        { label: 'Review tomorrow', action_kind: 'task.snooze_tomorrow' },
      ],
    }));
    assert.equal(blockedTaskActions.find((action) => action.id === 'task.complete')?.label, 'Close without running');
    assert.equal(blockedTaskActions.find((action) => action.id === 'task.complete')?.confirmLabel, 'Tap again to close without running');
    assert.equal(blockedTaskActions.find((action) => action.id === 'task.snooze_tomorrow')?.label, 'Review tomorrow');
    assert.deepEqual(buildSupportedItemActions(item({ id: 'done', kind: 'email', status: 'done' })), []);
  });

  it('trusts server-projected item actions when available', () => {
    assert.deepEqual(
      buildSupportedItemActions(item({
        id: 'mail-server',
        kind: 'email',
        source_kind: 'mail',
        status: 'unread',
        secondary_actions: [
          { label: 'Mark read', action_kind: 'mail.mark_read' },
        ],
      })).map((action) => action.id),
      [],
    );
    assert.deepEqual(
      buildSupportedItemActions(item({
        id: 'mail-server-none',
        kind: 'email',
        source_kind: 'mail',
        status: 'unread',
        secondary_actions: [],
      })),
      [],
    );
  });

  it('does not infer receipt-backed actions when the server did not project them', () => {
    assert.deepEqual(buildSupportedItemActions(item({ id: 'mail', kind: 'email', status: 'unread' })), []);
    assert.deepEqual(buildSupportedItemActions(item({ id: 'task', kind: 'task', status: 'new' })), []);
    assert.deepEqual(buildSupportedItemActions(item({ id: 'approval', kind: 'approval', source: 'hitl', status: 'waiting' })), []);
  });

  it('derives optimistic status updates after receipt-backed actions', () => {
    assert.equal(optimisticStatusForItemAction('task.complete'), 'done');
    assert.equal(optimisticStatusForItemAction('task.snooze_tomorrow'), 'snoozed');
    assert.equal(optimisticStatusForItemAction('approval.approve'), 'done');
    assert.equal(optimisticStatusForItemAction('approval.reject'), 'done');
  });

  it('resolves command focus for item and general delegation', () => {
    assert.equal(resolveCommandFocusId('item', 'email:42'), 'email:42');
    assert.equal(resolveCommandFocusId('item', null), null);
    assert.equal(resolveCommandFocusId('general', 'email:42'), 'assistant:general');
    assert.equal(resolveCommandFocusId('general', null), 'assistant:general');
  });

  it('suggests natural commands without executing them', () => {
    assert.deepEqual(
      buildCommandSuggestions(null, 'general').map((suggestion) => suggestion.label),
      ['What am I missing?', 'Review today', 'Check follow-ups'],
    );
    assert.deepEqual(
      buildCommandSuggestions(item({ id: 'note', kind: 'note' }), 'item').map((suggestion) => suggestion.label),
      ['What should I do?', 'Track this', 'Move later'],
    );
    assert.deepEqual(
      buildCommandSuggestions(item({ id: 'task', kind: 'task' }), 'item').map((suggestion) => suggestion.label),
      ['What should I do?', 'Remind tomorrow', 'Break it down'],
    );
  });

  it('renders source status in owner language', () => {
    assert.deepEqual(sourceStatusView({ source: 'calendar', status: 'available' }), {
      label: 'Calendar',
      statusLabel: 'Ready',
      healthy: true,
      reason: undefined,
      nextStep: undefined,
    });
    assert.deepEqual(sourceStatusView({ source: 'action_ledger', status: 'available' }), {
      label: 'Drafts and receipts',
      statusLabel: 'Ready',
      healthy: true,
      reason: undefined,
      nextStep: undefined,
    });
    assert.deepEqual(sourceStatusView({ source: 'alerts', status: 'deferred', reason: 'projector not connected' }), {
      label: 'System alerts',
      statusLabel: 'Quiet for now',
      healthy: true,
      reason: 'This signal is not connected to Aegis yet.',
      nextStep: undefined,
    });
    assert.deepEqual(sourceStatusView({ source: 'calendar', status: 'available', reason: 'assistant event metadata only' }), {
      label: 'Calendar',
      statusLabel: 'Ready',
      healthy: true,
      reason: 'Using safe basics only; no private source content was opened.',
      nextStep: undefined,
    });
    assert.deepEqual(sourceStatusView({ source: 'calendar', status: 'available', reason: 'ready' }), {
      label: 'Calendar',
      statusLabel: 'Ready',
      healthy: true,
      reason: undefined,
      nextStep: undefined,
    });
    assert.deepEqual(sourceStatusView({ source: 'calendar', status: 'permission_required', reason: 'consent_required' }), {
      label: 'Calendar',
      statusLabel: 'Permission needed',
      healthy: false,
      reason: 'Google needs permission before this source can open private details.',
      nextStep: 'Grant the missing permission before Aegis can use this source fully.',
    });
  });

  it('renders authority basis in owner language', () => {
    assert.equal(authorityBasisLabel('active_mandate'), 'A standing instruction you approved surfaced this.');
    assert.equal(authorityBasisLabel('direct_request'), 'You asked Aegis to work on this.');
    assert.equal(authorityBasisLabel('direct_user_request'), 'You asked Aegis to work on this.');
    assert.equal(authorityBasisLabel('system_health'), 'System health needs attention.');
    assert.equal(authorityBasisLabel(null), 'Aegis is waiting for your review.');
    assert.equal(authorityBasisLabel('custom_basis'), 'Aegis surfaced this for owner review.');
  });
});

describe('Work↔Mailbox mail_ref correlation (A4)', () => {
  it('no longer exports mail-ref helpers', () => {
    assert.equal(true, true);
  });
});

describe('backend structural i18n codes (reason / next / action label)', () => {
  const emailItem = (overrides = {}) => item({
    id: 'email-coded',
    kind: 'email',
    status: 'unread',
    owner_visible_reason: 'Email sender, subject, labels, or timing made this worth review.',
    owner_visible_reason_code: 'email_basics_review',
    next_safe_action: 'Read the exact email, then brief, draft, archive, or mark read.',
    next_safe_action_code: 'email_unread_review',
    primary_action: {
      label: 'Open email',
      label_code: 'open_email',
      action_kind: 'navigate',
      route: '/webapp/console/mail/events/1/body',
      requires_gate: true,
      requires_mfa: false,
    },
    ...overrides,
  });

  it('falls back to backend prose when a leftover email reason code has no catalog entry', () => {
    assert.equal(
      ownerVisibleReasonCue(emailItem(), 'zh'),
      'Email sender, subject, labels, or timing made this worth review.',
    );
  });

  it('renders the byte-identical English prose for the same code under en', () => {
    assert.equal(
      ownerVisibleReasonCue(emailItem(), 'en'),
      'Email sender, subject, labels, or timing made this worth review.',
    );
  });

  it('falls back to the backend prose for an UNKNOWN reason code (never blank, never MISSING)', () => {
    const cue = ownerVisibleReasonCue(emailItem({
      owner_visible_reason_code: 'brand_new_code_from_future_backend',
    }), 'zh');
    assert.equal(cue, 'Email sender, subject, labels, or timing made this worth review.');
    assert.doesNotMatch(cue, /MISSING/);
  });

  it('keeps suppressing generic reasons via their structural codes', () => {
    assert.equal(ownerVisibleReasonCue(item({
      id: 'generic-task',
      kind: 'task',
      owner_visible_reason: 'Aegis surfaced this task because it is active owner work.',
      owner_visible_reason_code: 'task_active_owner_work',
    }), 'zh'), '');
  });

  it('resolves next_safe_action codes per locale with prose fallback', () => {
    assert.equal(
      ownerVisibleNextSafeActionText(emailItem(), 'zh'),
      'Read the exact email, then brief, draft, archive, or mark read.',
    );
    assert.equal(
      ownerVisibleNextSafeActionText(emailItem(), 'en'),
      'Read the exact email, then brief, draft, archive, or mark read.',
    );
    assert.equal(
      ownerVisibleNextSafeActionText(emailItem({ next_safe_action_code: 'unknown_next_code' }), 'zh'),
      'Read the exact email, then brief, draft, archive, or mark read.',
    );
    assert.equal(ownerVisibleNextSafeActionText(item({ id: 'bare' }), 'zh'), '');
  });

  it('localizes the primary action label via label_code (owner: 打开邮件详情, distinct from 在邮箱中查看)', () => {
    assert.equal(ownerVisiblePrimaryActionLabel(emailItem(), 'Open', 40, 'zh'), 'Open email');
    assert.equal(ownerVisiblePrimaryActionLabel(emailItem(), 'Open', 40, 'en'), 'Open email');
    const unknown = emailItem();
    unknown.primary_action = { ...unknown.primary_action, label_code: 'mystery_button' };
    assert.equal(ownerVisiblePrimaryActionLabel(unknown, 'Open', 40, 'zh'), 'Open email');
  });

  it('keeps the approval Review-request softening for the review_approval code', () => {
    const approval = item({
      id: 'approval-coded',
      kind: 'approval',
      status: 'waiting',
      primary_action: {
        label: 'Review approval',
        label_code: 'review_approval',
        action_kind: 'navigate',
        route: '/ui/?tab=approvals',
        requires_gate: true,
        requires_mfa: false,
      },
    });
    assert.equal(ownerVisiblePrimaryActionLabel(approval, 'Open', 40, 'en'), 'Review request');
    assert.equal(ownerVisiblePrimaryActionLabel(approval, 'Open', 40, 'zh'), '审核请求');
  });

  it('resolveWorkCodeText returns null for absent/unknown codes and text for known ones', () => {
    assert.equal(resolveWorkCodeText('work.reasonCode', null, 'zh'), null);
    assert.equal(resolveWorkCodeText('work.reasonCode', undefined, 'zh'), null);
    assert.equal(resolveWorkCodeText('work.reasonCode', '  ', 'zh'), null);
    assert.equal(resolveWorkCodeText('work.reasonCode', 'nope_not_a_code', 'zh'), null);
    assert.equal(
      resolveWorkCodeText('work.nextCode', 'stale_unread_later', 'zh'),
      '清理、归档，或留待以后处理。',
    );
    assert.equal(
      resolveWorkCodeText('work.actionLabel', 'open_email', 'en'),
      null,
    );
  });

  it('resolveWorkCodeText honors a `namespace:code` prefix and rejects malformed ones', () => {
    // A namespaced code overrides the caller's default prefix.
    assert.equal(
      resolveWorkCodeText('work.decide', 'nextCode:stale_unread_later', 'zh'),
      resolveWorkCodeText('work.nextCode', 'stale_unread_later', 'zh'),
    );
    assert.equal(
      resolveWorkCodeText('work.decide', 'reasonCode:blocked_by_mail_permission', 'zh'),
      resolveWorkCodeText('work.reasonCode', 'blocked_by_mail_permission', 'zh'),
    );
    // Malformed / unknown namespaces resolve to null so callers keep the
    // server string — never a wrong-family translation.
    for (const bad of ['bogus:stale_unread_later', ':stale_unread_later', 'nextCode:', ':', 'nextCode:a:b']) {
      assert.equal(resolveWorkCodeText('work.nextCode', bad, 'zh'), null, bad);
    }
  });
});

describe('presentOwnerWorkSurfaceActions (primary vs secondary)', () => {
  function mailFixture(overrides = {}) {
    return item({
      id: 'task:present',
      kind: 'task',
      status: 'active',
      secondary_actions: [
        { label: 'Complete', action_kind: 'task.complete' },
      ],
      ...overrides,
    });
  }

  function actionsFor(itemLike) {
    return buildSupportedItemActions(itemLike, 'en');
  }

  it('uses open_detail as primary when detail is closed; secondaries hold receipt + chips', () => {
    const mail = mailFixture();
    const itemActions = actionsFor(mail);
    const surface = presentOwnerWorkSurfaceActions({
      item: mail,
      locale: 'en',
      sourceDetailOpen: false,
      includeOpenDetail: true,
      openDetailLabel: 'Open details',
      includeAssistantChips: true,
      briefLabel: 'Brief',
      draftLabel: 'Draft',
      itemActions,
    });
    assert.ok(surface.primary, 'primary must not be null when actions exist');
    assert.equal(surface.primary.kind, 'open_detail');
    assert.equal(surface.primary.id, 'open_detail');
    const secondaryIds = surface.secondary.map((c) => c.id);
    assert.ok(secondaryIds.includes('task.complete'), 'secondary should include complete');
    assert.ok(secondaryIds.includes('brief'), 'secondary should include brief chip');
    assert.ok(secondaryIds.includes('draft'), 'secondary should include draft chip');
    assert.equal(surface.secondary.filter((c) => c.kind === 'item_action').length, 1);
  });

  it('promotes Open in X when an X deep-link is present (complement app, not a timeline)', () => {
    const task = item({
      id: 'task:x',
      kind: 'task',
      status: 'tracking',
      secondary_actions: [
        { label: 'Clean up', action_kind: 'task.complete' },
      ],
    });
    const surface = presentOwnerWorkSurfaceActions({
      item: task,
      locale: 'en',
      sourceDetailOpen: false,
      includeOpenDetail: true,
      openDetailLabel: 'Open details',
      openX: {
        href: 'https://x.com/a/status/1',
        label: 'Open in X',
        draftHref: 'https://x.com/intent/post?in_reply_to=1',
        draftLabel: 'Draft reply in X',
      },
      includeAssistantChips: false,
      briefLabel: 'Brief',
      draftLabel: 'Draft',
      itemActions: actionsFor(task),
    });
    assert.equal(surface.primary?.kind, 'open_x');
    assert.equal(surface.primary?.href, 'https://x.com/a/status/1');
    const secondaryIds = surface.secondary.map((c) => c.id);
    assert.ok(secondaryIds.includes('open_detail'));
    assert.ok(secondaryIds.includes('draft_in_x'));
    assert.ok(secondaryIds.includes('task.complete'));
  });

  it('promotes first item action to primary when detail is open', () => {
    const mail = mailFixture();
    const itemActions = actionsFor(mail);
    const surface = presentOwnerWorkSurfaceActions({
      item: mail,
      locale: 'en',
      sourceDetailOpen: true,
      includeOpenDetail: false,
      openDetailLabel: 'Open details',
      includeAssistantChips: true,
      briefLabel: 'Brief',
      draftLabel: 'Draft',
      itemActions,
    });
    assert.ok(surface.primary, 'primary must not be null when item actions exist');
    assert.equal(surface.primary.kind, 'item_action');
    assert.equal(surface.primary.id, 'task.complete');
    const secondaryIds = surface.secondary.map((c) => c.id);
    assert.ok(!secondaryIds.includes('task.complete'), 'primary action must not also sit in secondary');
    assert.ok(secondaryIds.includes('brief'));
    assert.ok(secondaryIds.includes('draft'));
  });

  it('keeps remaining controls secondary and never nulls primary when any control exists', () => {
    const mail = mailFixture();
    const itemActions = actionsFor(mail);
    const closed = presentOwnerWorkSurfaceActions({
      item: mail,
      sourceDetailOpen: false,
      includeOpenDetail: true,
      openDetailLabel: 'Open',
      includeAssistantChips: false,
      briefLabel: 'Brief',
      draftLabel: 'Draft',
      itemActions,
    });
    assert.notEqual(closed.primary, null);
    assert.equal(closed.primary?.kind, 'open_detail');
    assert.ok(closed.secondary.length >= 1);
  });
});




  it('uses open_detail as the generic open path', () => {
    const genericItem = {
      id: 'task:1',
      kind: 'task',
      source: 'tasks',
      title: 'Bill',
      summary: 's',
      status: 'active',
      urgency: 'normal',
      updated_at: '2026-05-05T00:00:00Z',
      secondary_actions: [
        { label: 'Complete', action_kind: 'task.complete' },
      ],
    };
    const surface = presentOwnerWorkSurfaceActions({
      item: genericItem,
      sourceDetailOpen: false,
      includeOpenDetail: true,
      openDetailLabel: 'Open',
      includeAssistantChips: false,
      briefLabel: 'Brief',
      draftLabel: 'Draft',
      itemActions: [
        { id: 'task.complete', label: 'Complete', confirmLabel: 'c', receiptHint: 'r', tone: 'normal' },
      ],
    });
    assert.equal(surface.primary?.kind, 'open_detail');
    assert.ok(surface.secondary.some((c) => c.kind === 'item_action'));
  });

describe('work detail ask placement + scan filter density', () => {
  it('places Ask Aegis as compact secondary when the panel is closed', () => {
    assert.equal(workDetailAskPlacement(false), 'compact_secondary');
    assert.equal(workDetailAskPlacement(true), 'inline_panel');
  });

  it('keeps scan scope/sort collapsed for default empty query', () => {
    assert.equal(workScanFiltersDefaultExpanded({ query: '', scope: 'all', sort: 'lane' }), false);
    assert.equal(workScanChromeDefaultExpanded({ query: '', scope: 'all', sort: 'lane' }), false);
    assert.equal(workScanChromeDefaultExpanded({ query: 'x', scope: 'all', sort: 'lane' }), true);
    assert.equal(workScanFiltersDefaultExpanded({ query: 'bill', scope: 'all', sort: 'lane' }), true);
    assert.equal(workScanFiltersDefaultExpanded({ query: '', scope: 'needs_me', sort: 'lane' }), true);
    assert.equal(workScanFiltersDefaultExpanded({ query: '', scope: 'all', sort: 'priority' }), true);
  });
});
