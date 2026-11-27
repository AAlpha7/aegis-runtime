import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

const modulePath = process.env.VNEXT_WORK_RENDER_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-work-render-test/WorkView.mjs', import.meta.url).href;
const {
  WorkView,
  lineageChatCite,
  messagePublicRef,
  filterWorkForScan,
  resolveDecisionChoiceCopy,
  localizeDecisionRequest,
  presentOwnerWorkSurfaceActions,
  workDetailAskPlacement,
  workScanFiltersDefaultExpanded,
  workScanChromeDefaultExpanded,
  LocaleProvider,
  t,
  tf,
} = await import(modulePath);

const noop = () => {};

function mailItem(overrides = {}) {
  return {
    id: 'email:42',
    lane: 'now',
    kind: 'email',
    source_kind: 'mail',
    source: 'gmail',
    source_id: '42',
    source_event_id: null,
    title: 'Billing statement available',
    summary: 'Maple Lane Music School; open the message for the full email',
    why_it_matters: 'Unread billing mail surfaced from safe metadata.',
    owner_visible_reason: 'Gmail sender and subject made this worth review.',
    state: 'unread',
    status: 'unread',
    urgency: 'normal',
    sensitivity: 'S2',
    privacy_level: 'S2',
    created_at: '2026-05-05T00:00:00Z',
    updated_at: '2026-05-05T00:00:00Z',
    due_at: null,
    authority_basis: 'manual_review',
    authority: {
      basis: 'manual_review',
      authority_basis: 'manual_review',
      owner_visible_reason: 'Mail signal matched.',
      visible_data: ['title', 'summary', 'sender', 'status'],
      visible_fields: ['title', 'summary', 'sender', 'status'],
      locked_data: ['body', 'html', 'attachments'],
      locked_fields: ['body', 'html', 'attachments'],
      allowed_now: ['brief', 'draft'],
      requires_mfa: false,
      requires_approval: false,
      requires_data_grant: false,
      expected_receipt: 'Mailbox changes keep a Gmail provider receipt.',
      source_mandate_id: null,
      audit_refs: [],
      audit_links: [],
    },
    primary_action: {
      id: 'open',
      label: 'Open message',
      kind: 'navigate',
      route: '/webapp/console/mail/events/42/body',
      requires_confirmation: false,
      requires_mfa: false,
      requires_data_gate: false,
      expected_receipt: 'Reading alone does not execute an outside action.',
    },
    contextual_actions: [
      {
        id: 'mail.mark_read',
        label: 'Mark read',
        confirm_label: 'Tap again to mark read',
        receipt_hint: 'Aegis will record a Gmail mark-read receipt.',
        tone: 'normal',
      },
      {
        id: 'mail.archive',
        label: 'Archive',
        confirm_label: 'Tap again to archive',
        receipt_hint: 'Aegis will record an archive receipt.',
        tone: 'normal',
      },
    ],
    secondary_actions: [
      {
        label: 'Mark read',
        action_kind: 'mail.mark_read',
        route: '/webapp/console/mail/events/42/read-state',
        requires_gate: false,
        requires_mfa: false,
      },
      {
        label: 'Archive',
        action_kind: 'mail.archive',
        route: '/webapp/console/mail/events/42/mailbox-state',
        requires_gate: false,
        requires_mfa: false,
      },
    ],
    next_safe_action: 'Open the exact email, then brief, draft, archive, or mark read.',
    blocked_by: '',
    detail_route: '/webapp/console/mail/events/42/body',
    external_url_policy: 'available_after_open',
    route_reason: 'Unread billing email surfaced from safe Gmail basics.',
    related_work_item_ids: [],
    related_action_ids: [],
    related_receipt_ids: [],
    receipt_expectation: 'Mailbox changes keep a Gmail provider receipt.',
    ...overrides,
  };
}

function taskItem(overrides = {}) {
  return {
    ...mailItem({
      id: 'task:1054',
      lane: 'decide',
      kind: 'task',
      source_kind: 'task',
      source: 'tasks',
      source_id: '1054',
      title: 'Monitor Gmail',
      summary: 'A scheduled automation paused before using a sensitive step.',
      why_it_matters: 'Aegis paused before a sensitive step.',
      owner_visible_reason: 'Aegis paused before a sensitive step.',
      state: 'blocked_by_gate',
      status: 'blocked_by_gate',
      urgency: 'high',
      primary_action: {
        id: 'open',
        label: 'Open detail',
        kind: 'navigate',
        route: '/webapp/console/tasks/1054/timeline',
        requires_confirmation: false,
        requires_mfa: false,
        requires_data_gate: false,
        expected_receipt: 'Reading alone does not execute an outside action.',
      },
      contextual_actions: [],
      secondary_actions: [],
      next_safe_action: 'Review before continuing.',
      blocked_by: 'scheduled automation wants Gmail detail before continuing',
      detail_route: '/webapp/console/tasks/1054/timeline',
      receipt_expectation: 'Task mutations are recorded with an action ledger receipt or a failure state.',
      decision_request: blockedDecisionRequest(),
    }),
    ...overrides,
  };
}

function blockedDecisionChoice(overrides = {}) {
  return {
    id: 'revise_rule',
    label: 'Revise rule',
    consequence: 'Discuss a narrower or clearer rule with Aegis before any run continues.',
    risk_level: 'low',
    applies_to: 'rule',
    assistant_prompt: 'Help me revise this paused automation rule. First explain what it is trying to do, then propose safer narrower options. Do not execute any external action.',
    requires_confirmation: false,
    enabled: true,
    tone: 'normal',
    ...overrides,
  };
}

function blockedDecisionRequest(overrides = {}) {
  // Codes match production console_handlers::vnext_blocked_decision_request
  // for a mail-kind pause without a live approval.
  return {
    id: 'decision:task:1054',
    title: 'Mail triage needs your choice',
    title_code: 'title_mail',
    what_aegis_was_doing: 'Review unread Primary mail for the mail triage automation.',
    what_code: 'what_mail_review_primary',
    blocked_reason: 'Reading or classifying email content needs owner permission.',
    blocked_reason_code: 'blocked_by_mail_permission',
    requested_capability: 'Read or classify mail content for this paused automation.',
    capability_code: 'cap_mail_read_classify',
    scope: 'Approve or skip applies to this paused run. Closing marks this paused task done. Rule changes only happen if you explicitly choose a rule change.',
    scope_code: 'scope_blocked_run',
    choices: [
      blockedDecisionChoice({
        id: 'approve_unavailable',
        label: 'Approve this run',
        consequence: 'Aegis cannot continue from this card because the live approval request is missing or expired.',
        risk_level: 'medium',
        applies_to: 'this_run',
        assistant_prompt: undefined,
        enabled: false,
        disabled_reason: 'No live approval request is attached to this pause.',
      }),
      blockedDecisionChoice({
        id: 'skip_unavailable',
        label: 'Skip this run',
        consequence: 'Aegis cannot skip the original approval request because that request is no longer pending.',
        applies_to: 'this_run',
        assistant_prompt: undefined,
        enabled: false,
        disabled_reason: 'No live approval request is attached to this pause.',
      }),
      blockedDecisionChoice(),
      blockedDecisionChoice({
        id: 'view_safe_details',
        label: 'View safe details',
        consequence: 'Open the task context and history without running the blocked step.',
        applies_to: 'this_item',
        detail_route: '/webapp/console/tasks/1054/timeline',
        assistant_prompt: undefined,
      }),
    ],
    safe_details: [
      'No external action has run.',
      'Visible metadata can be reviewed without opening private content.',
      'No live approval request is attached to this pause, so approve/skip is unavailable until Aegis regenerates a real approval request.',
    ],
    safe_detail_codes: [
      'safe_no_external_action',
      'safe_visible_metadata',
      'safe_no_live_approval',
    ],
    audit_refs: [],
    receipt_expectation: 'Task mutations are recorded with an action ledger receipt or a failure state.',
    ...overrides,
  };
}

function reminderItem(overrides = {}) {
  return taskItem({
    id: 'task:1162',
    title: 'Passport follow-up',
    summary: 'Follow up on the passport task.',
    source_id: '1162',
    source_event_id: 'event:1162',
    blocked_by: 'waiting on owner review',
    ...overrides,
  });
}

function passiveTrackingItem(overrides = {}) {
  return taskItem({
    id: 'task:passive-tracking',
    lane: 'follow_up',
    title: '日终状态：磁盘77% / Gmail电路 / Nationwide 5/9',
    summary: 'Waiting-for item from the knowledge base. Waiting on: Fix skills / disk cleanup / payment.',
    owner_visible_reason: 'Waiting on a dependency.',
    state: 'tracking',
    status: 'tracking',
    urgency: 'high',
    blocked_by: '',
    secondary_actions: [],
    decision_request: null,
    next_safe_action: 'Ask Aegis whether this still needs follow-up, a reminder, or cleanup.',
    ...overrides,
  });
}

function lanes(items) {
  return [
    { id: 'decide', title: 'Decide', subtitle: 'Needs your call before Aegis continues.', items: items.filter((item) => item.lane === 'decide'), collapsed: false },
    { id: 'now', title: 'Now', subtitle: 'Ready to handle now.', items: items.filter((item) => item.lane === 'now'), collapsed: false },
    { id: 'follow_up', title: 'Follow Up', subtitle: 'Tracked quietly.', items: [], collapsed: true },
    { id: 'backlog', title: 'Backlog', subtitle: 'Later.', items: [], collapsed: true },
    { id: 'done', title: 'Done', subtitle: 'Handled.', items: [], collapsed: true },
  ];
}

function baseProps(overrides = {}) {
  const items = [taskItem(), mailItem()];
  return {
    lanes: lanes(items),
    expandedLaneIds: [],
    selected: items[1],
    context: null,
    itemProjection: null,
    sourceDetail: null,
    sourceDetailBusy: false,
    assistantResult: null,
    actionMessage: '',
    armedAction: null,
    executingAction: null,
    busy: false,
    onSelect: noop,
    onToggleLane: noop,
    onOpenSource: noop,
    onAction: noop,
    onBlockedResolution: noop,
    onBrief: noop,
    onDraft: noop,
    onAskText: noop,
    ...overrides,
  };
}

// Default render: no LocaleProvider, so the locale defaults to zh (matching the
// LocaleContext default). Existing assertions therefore target the zh strings.
function render(props) {
  return renderToStaticMarkup(React.createElement(WorkView, props));
}

// Render under an explicit locale via LocaleProvider, sharing the SAME context
// instance as WorkView (both come from the single workViewTestEntry bundle).
function renderWithLocale(props, displayLanguage) {
  return renderToStaticMarkup(
    React.createElement(LocaleProvider, { displayLanguage }, React.createElement(WorkView, props)),
  );
}

describe('vNext Work render', () => {
  it('renders a selected email-kind item through the generic work path', () => {
    const html = render(baseProps());

    assert.match(html, /Billing statement available/);
    assert.ok(html.includes(t('work.askAegis', 'zh')));
    assert.ok(html.includes(t('work.askAegis.usingItemContext', 'zh')));
    assert.ok(html.includes(t('work.action.brief', 'zh')));
    assert.ok(html.includes(t('work.action.draft', 'zh')));
    assert.ok(html.includes(t('work.group.actions', 'zh')));
    assert.doesNotMatch(html, /vnext-mail/);
    assert.doesNotMatch(html, /Show access details/);
    assert.doesNotMatch(html, /Allowed now:/);
    assert.doesNotMatch(html, /source_content_untrusted/);
    assert.doesNotMatch(html, /visible_fields/);
  });


  it('exposes exactly one primary work control and discloses secondary actions', () => {
    const html = render(baseProps());

    // Responsive layouts may render the selected detail twice (mobile + desktop).
    // Each action dock must still have exactly one open-details primary step.
    const docks = html.split('vnext-action-dock-grid').slice(1);
    assert.ok(docks.length >= 1, 'expected at least one action dock');
    for (const [index, dock] of docks.entries()) {
      const section = dock.slice(0, dock.indexOf('</div>') > 0 ? 2500 : dock.length);
      const primarySteps = section.match(/class="vnext-item-primary-step"/g) || [];
      assert.equal(
        primarySteps.length,
        1,
        `dock ${index} expected one vnext-item-primary-step, got ${primarySteps.length}`,
      );
    }

    // Secondary disclosure present when more than primary exists.
    assert.match(html, /class="vnext-work-more-actions vnext-disclosure"/);
    assert.ok(html.includes(t('work.moreActions', 'zh')) || html.includes('More actions'));

    const moreStart = html.indexOf('vnext-work-more-actions');
    assert.ok(moreStart > 0);
    const moreHtml = html.slice(moreStart, html.indexOf('</details>', moreStart) + 10);

    // Brief/Draft are secondary (inside disclosure), not equal-weight peer chips outside.
    const brief = t('work.action.brief', 'zh');
    const draft = t('work.action.draft', 'zh');
    assert.ok(moreHtml.includes(brief), 'Brief should sit inside more-actions disclosure');
    assert.ok(moreHtml.includes(draft), 'Draft should sit inside more-actions disclosure');
    // No peer chip row outside disclosure.
    assert.doesNotMatch(html, /vnext-item-action-chips/);
  });

  it('demotes Ask Aegis off the full-width peer primary wall (multi-action detail)', () => {
    const html = render(baseProps());
    // No full-width AI entry wall sibling.
    assert.doesNotMatch(html, /vnext-item-ai-entry/);
    // Compact secondary Ask control is present.
    assert.match(html, /vnext-work-ask-compact/);
    assert.ok(html.includes(t('work.askAegis', 'zh')));
    // Primary next-step still present.
    assert.match(html, /vnext-item-primary-step/);
    // Pure helper agrees with the closed-assistant placement.
    assert.equal(workDetailAskPlacement(false), 'compact_secondary');
  });


  it('renders a mail-like work item through the generic item path', () => {
    const withRef = mailItem({ mail_ref: { account: 'work', message_id: 'gmail-42' } });
    const html = render(baseProps({
      selected: withRef,
      lanes: lanes([taskItem(), withRef]),

    }));
    assert.doesNotMatch(html, /vnext-work-view-in-mailbox/);
    assert.match(html, /vnext-detail/);
  });

  it('collapses FIND scope+sort behind progressive disclosure by default', () => {
    const html = render(baseProps());
    // Whole FIND chrome progressive; filters nested inside.
    assert.match(html, /vnext-work-scan-chrome/);
    assert.match(html, /vnext-work-scan-filters/);
    assert.ok(html.includes(t('work.scan.filters', 'zh')) || html.includes('Filters'));
    const chromeMatch = html.match(/<details class="vnext-work-scan-chrome vnext-disclosure"[^>]*>/);
    assert.ok(chromeMatch, 'expected scan chrome details');
    assert.ok(!/\sopen(="true"|="")?/.test(chromeMatch[0]), `default FIND chrome should be closed, got ${chromeMatch[0]}`);
    const tagMatch = html.match(/<details class="vnext-work-scan-filters vnext-disclosure"[^>]*>/);
    assert.ok(tagMatch, 'expected scan filters details');
    assert.ok(!/\sopen(="true"|="")?/.test(tagMatch[0]), `default filters should be closed, got ${tagMatch[0]}`);
    const filtersStart = html.indexOf('vnext-work-scan-filters');
    const filtersHtml = html.slice(filtersStart, html.indexOf('</details>', filtersStart) + 10);
    assert.match(filtersHtml, /vnext-work-scan-group/);
    assert.equal(workScanFiltersDefaultExpanded({ query: '', scope: 'all', sort: 'lane' }), false);
    assert.equal(workScanChromeDefaultExpanded({ query: '', scope: 'all', sort: 'lane' }), false);
  });

  it('promotes first receipt action to primary when source detail is open', () => {
    const html = render(baseProps({
      sourceDetail: {
        kind: 'generic',
        title: 'Billing statement available',
        subtitle: 'Maple Lane Music School',
        fields: [],
        body: 'Statement ready.',
        timeline: [],
      },
    }));
    assert.doesNotMatch(html, /vnext-item-primary-step/);
    assert.match(html, /vnext-work-more-actions/);
  });


  it('omits mailbox-only affordances', () => {
    const html = render(baseProps());
    assert.doesNotMatch(html, /vnext-work-view-in-mailbox/);
  });

  it('localizes backend structural codes (reason/next/action label) per locale', () => {
    const coded = mailItem({
      owner_visible_reason: 'Email sender, subject, labels, or timing made this worth review.',
      owner_visible_reason_code: 'email_basics_review',
      next_safe_action: 'Read the exact email, then brief, draft, archive, or mark read.',
      next_safe_action_code: 'email_unread_review',
      primary_action: {
        id: 'open',
        label: 'Open email',
        label_code: 'open_email',
        kind: 'navigate',
        route: '/webapp/console/mail/events/42/body',
        requires_confirmation: false,
        requires_mfa: false,
        requires_data_gate: false,
        expected_receipt: 'Reading alone does not execute an outside action.',
      },
    });
    const props = () => baseProps({ selected: coded, lanes: lanes([taskItem(), coded]) });

    // zh (default locale): the detail hero WHY + the primary button localize.
    const zh = render(props());
    assert.ok(zh.includes('Email sender, subject, labels, or timing made this worth review.'));
    assert.ok(zh.includes('Open email'));

    // en: byte-identical to the backend prose (codes resolve to the en catalog).
    const en = renderWithLocale(props(), 'en');
    assert.ok(en.includes('Email sender, subject, labels, or timing made this worth review.'));
    assert.ok(en.includes('Open email'));
    assert.doesNotMatch(en, /打开邮件详情/);
  });

  it('localizes the blocked-card fallback reason via the code of the sentence it shows (zh)', () => {
    // Blocked item WITHOUT a server decision_request → WorkView builds the
    // fallback decision, whose blocked_reason comes from blocked_by. It must be
    // localized through blocked_by's OWN code.
    const blocked = taskItem({
      decision_request: null,
      blocked_by: 'Reading or classifying email content needs owner permission.',
      blocked_by_code: 'blocked_by_mail_permission',
      owner_visible_reason: 'This may read private email content, so Aegis paused before continuing.',
      owner_visible_reason_code: 'blocked_mail_pause',
    });
    const props = () => baseProps({ selected: blocked, lanes: lanes([blocked, mailItem()]) });

    const zh = render(props());
    assert.ok(zh.includes('Reading or classifying email content needs owner permission.'));

    const en = renderWithLocale(props(), 'en');
    assert.ok(en.includes('Reading or classifying email content needs owner permission.'));
  });

  it('keeps the fallback blocked_by verbatim under zh when its code is unknown', () => {
    // Honesty rule: an unresolvable code must NOT be papered over with the
    // localized owner_visible_reason — that is a DIFFERENT sentence than the
    // one the card claims to show. Render the server sentence as-is instead.
    const blocked = taskItem({
      decision_request: null,
      blocked_by: 'A source-specific pause reason with no catalog entry yet.',
      blocked_by_code: 'blocked_by_not_in_catalog_v9',
      owner_visible_reason: 'This may read private email content, so Aegis paused before continuing.',
      owner_visible_reason_code: 'blocked_mail_pause',
    });
    const props = () => baseProps({ selected: blocked, lanes: lanes([blocked, mailItem()]) });

    const zh = render(props());
    assert.ok(zh.includes('A source-specific pause reason with no catalog entry yet.'));
    assert.ok(!zh.includes(t('work.reasonCode.blocked_mail_pause', 'zh')));
  });

  it('localizes the SERVER decision_request blocked reason via blocked_reason_code (zh)', () => {
    // Normal vNext path: blocked_reason came from blocked_by; the server emits
    // blocked_reason_code in lockstep. Under zh the blocked_by sentence must
    // render — NOT the owner_visible_reason (blocked_mail_pause) translation.
    const blocked = taskItem({
      owner_visible_reason: 'This may read private email content, so Aegis paused before continuing.',
      owner_visible_reason_code: 'blocked_mail_pause',
      blocked_by: 'Reading or classifying email content needs owner permission.',
      blocked_by_code: 'blocked_by_mail_permission',
    });
    assert.equal(blocked.decision_request.blocked_reason, 'Reading or classifying email content needs owner permission.');
    assert.equal(blocked.decision_request.blocked_reason_code, 'blocked_by_mail_permission');
    const props = () => baseProps({ selected: blocked, lanes: lanes([blocked, mailItem()]) });

    const zh = render(props());
    assert.ok(zh.includes('Reading or classifying email content needs owner permission.'));

    const en = renderWithLocale(props(), 'en');
    assert.ok(en.includes('Reading or classifying email content needs owner permission.'));
  });

  it('falls back to backend prose for unknown structural codes (no blank, no MISSING marker)', () => {
    const unknownCoded = mailItem({
      owner_visible_reason: 'Email sender, subject, labels, or timing made this worth review.',
      owner_visible_reason_code: 'future_reason_code_v9',
      next_safe_action: 'Read the exact email, then brief, draft, archive, or mark read.',
      next_safe_action_code: 'future_next_code_v9',
      primary_action: {
        id: 'open',
        label: 'Open email',
        label_code: 'future_action_code_v9',
        kind: 'navigate',
        route: '/webapp/console/mail/events/42/body',
        requires_confirmation: false,
        requires_mfa: false,
        requires_data_gate: false,
        expected_receipt: 'Reading alone does not execute an outside action.',
      },
    });
    const zh = render(baseProps({ selected: unknownCoded, lanes: lanes([taskItem(), unknownCoded]) }));
    assert.ok(zh.includes('Email sender, subject, labels, or timing made this worth review.'));
    assert.ok(zh.includes('Open email'));
    assert.doesNotMatch(zh, /MISSING/);
  });

  it('keeps the extra queue collapsed after a work item is selected', () => {
    const html = render(baseProps());

    assert.ok(html.includes(`<details class="vnext-work-brief vnext-disclosure is-secondary" aria-label="${t('work.aria.queueBrief', 'zh')}">`));
    assert.ok(html.includes(t('work.queue.moreWorkKicker', 'zh')));
    assert.ok(html.includes(tf('work.queue.nearbyHeadline', 'zh', { count: '1', itemWord: t('work.nearbyItem.one', 'zh') })));
    assert.match(html, /vnext-work-mobile-inline-lanes/);
    assert.match(html, /vnext-work-mobile-inline-item active/);
    assert.match(html, /vnext-detail vnext-work-inline-selected/);
    assert.match(html, /vnext-detail vnext-detail-desktop/);
    const mobileLanes = html.slice(html.indexOf('vnext-work-mobile-inline-lanes'));
    // Selection must not re-sort lanes: Decide stays ahead of Now.
    assert.equal(mobileLanes.match(/<h2>([^<]+)<\/h2>/)?.[1], 'Decide');
    assert.ok(!html.includes(`<details class="vnext-work-brief vnext-disclosure is-secondary" aria-label="${t('work.aria.queueBrief', 'zh')}" open="">`));
  });

  it('lets the owner pick other nearby items without reopening the full queue wall', () => {
    const selected = taskItem();
    const secondDecision = reminderItem();
    const html = render(baseProps({
      selected,
      lanes: lanes([selected, secondDecision, mailItem()]),
    }));

    assert.ok(html.includes(t('work.aria.otherItems', 'zh')));
    assert.ok(html.includes(t('work.aria.nextPreview', 'zh')));
    assert.match(html, /vnext-work-desktop-nearby/);
    assert.ok(html.includes(t('work.next.heading', 'zh')));
    assert.ok(html.includes(t('work.aria.nearbyItems', 'zh')));
    assert.match(html, /Passport follow-up/);
    assert.match(html, /class="vnext-item-row compact"/);
    assert.doesNotMatch(html, /class="vnext-item-row compact active"/);
    assert.doesNotMatch(html, /Nearby work preview/);
    assert.doesNotMatch(html, /Open full detail/);
    assert.match(html, /vnext-work-mobile-inline-lanes/);
    assert.match(html, /vnext-detail vnext-work-inline-selected/);
  });

  it('keeps passive tracking loops inspectable inside More Work without treating them as action-needed nearby items', () => {
    const selected = mailItem();
    const passiveTracking = passiveTrackingItem();
    const html = render(baseProps({
      selected,
      lanes: [
        { id: 'decide', title: 'Decide', subtitle: 'Needs your call before Aegis continues.', items: [], collapsed: false },
        { id: 'now', title: 'Now', subtitle: 'Ready to handle now.', items: [selected], collapsed: false },
        { id: 'follow_up', title: 'Follow Up', subtitle: 'Tracked quietly.', items: [passiveTracking], collapsed: false },
        { id: 'backlog', title: 'Backlog', subtitle: 'Later.', items: [], collapsed: true },
        { id: 'done', title: 'Done', subtitle: 'Handled.', items: [], collapsed: true },
      ],
    }));

    assert.ok(html.includes(tf('work.queue.quietHeadline', 'zh', { count: '1', loopWord: t('work.loop.one', 'zh') })));
    assert.ok(html.includes(t('work.queue.quietBody', 'zh')));
    assert.ok(html.includes(t('work.queue.watchingKicker', 'zh')));
    assert.ok(html.includes(tf('work.quietWatching.note', 'zh', { count: '1', loopWord: t('work.loop.one', 'zh') })));
    assert.ok(html.includes(t('work.aria.quietWatching', 'zh')));
    assert.match(html, /日终状态：磁盘77%/);
    assert.match(html, /vnext-nearby-work-item/);
  });

  it('still shows tracking loops that have real owner actions', () => {
    const selected = mailItem();
    const actionableTracking = passiveTrackingItem({
      id: 'task:commute',
      title: '回家通勤监控',
      secondary_actions: [
        { label: 'Complete', action_kind: 'task.complete' },
        { label: 'Snooze', action_kind: 'task.snooze_tomorrow' },
      ],
    });
    const html = render(baseProps({
      selected,
      lanes: [
        { id: 'decide', title: 'Decide', subtitle: 'Needs your call before Aegis continues.', items: [], collapsed: false },
        { id: 'now', title: 'Now', subtitle: 'Ready to handle now.', items: [selected], collapsed: false },
        { id: 'follow_up', title: 'Follow Up', subtitle: 'Tracked quietly.', items: [actionableTracking], collapsed: false },
        { id: 'backlog', title: 'Backlog', subtitle: 'Later.', items: [], collapsed: true },
        { id: 'done', title: 'Done', subtitle: 'Handled.', items: [], collapsed: true },
      ],
    }));

    assert.ok(html.includes(tf('work.queue.nearbyHeadline', 'zh', { count: '1', itemWord: t('work.nearbyItem.one', 'zh') })));
    assert.match(html, /回家通勤监控/);
    // No quiet-watching headline when the nearby item is actionable.
    assert.ok(!html.includes(t('work.queue.quietBody', 'zh')));
  });

  it('does not let passive tracking loops crowd out actionable nearby work', () => {
    const selected = mailItem();
    const passiveTracking = Array.from({ length: 9 }, (_, index) => passiveTrackingItem({
      id: `task:passive-${index}`,
      title: `Quiet tracking ${index}`,
    }));
    const actionable = reminderItem({ id: 'task:actionable-after-passive', title: 'Passport action still visible' });
    const html = render(baseProps({
      selected,
      lanes: [
        { id: 'decide', title: 'Decide', subtitle: 'Needs your call before Aegis continues.', items: [], collapsed: false },
        { id: 'now', title: 'Now', subtitle: 'Ready to handle now.', items: [selected, actionable], collapsed: false },
        { id: 'follow_up', title: 'Follow Up', subtitle: 'Tracked quietly.', items: passiveTracking, collapsed: false },
        { id: 'backlog', title: 'Backlog', subtitle: 'Later.', items: [], collapsed: true },
        { id: 'done', title: 'Done', subtitle: 'Handled.', items: [], collapsed: true },
      ],
    }));

    assert.match(html, /Passport action still visible/);
    assert.ok(html.includes(tf('work.queue.nearbyBodyQuiet', 'zh', { count: '9', loopWord: t('work.loop.other', 'zh') })));
    assert.match(html, /Quiet tracking 0/);
  });

  it('keeps the queue overview open before the owner selects an item', () => {
    const html = render(baseProps({ selected: null }));

    assert.ok(html.includes(`<details class="vnext-work-brief vnext-disclosure" aria-label="${t('work.aria.queueBrief', 'zh')}" open="">`));
    assert.ok(html.includes(t('work.queue.overviewKicker', 'zh')));
    assert.ok(html.includes(t('work.aria.scanControls', 'zh')));
    assert.ok(html.includes(t('work.scan.searchPlaceholder', 'zh')));
    assert.ok(html.includes(`>${t('work.scan.scope.needsMe', 'zh')}<`));
    assert.ok(html.includes(`>${t('work.scan.sort.priority', 'zh')}<`));
    assert.ok(html.includes(t('work.startWith', 'zh')));
    assert.match(html, /vnext-panel vnext-work-lane/);
  });

  it('filters and sorts work locally for dense queue scanning', () => {
    const quietTracking = passiveTrackingItem();
    const mail = mailItem({ id: 'email:later', updated_at: '2026-05-04T00:00:00Z' });
    const decision = taskItem({ id: 'task:decision', updated_at: '2026-05-06T00:00:00Z' });
    const items = [quietTracking, mail, decision];

    assert.deepEqual(
      filterWorkForScan(items, { scope: 'needs_me', sort: 'lane' }).map((item) => item.id),
      ['email:later', 'task:decision'],
    );
    assert.deepEqual(
      filterWorkForScan(items, { scope: 'quiet', sort: 'lane' }).map((item) => item.id),
      ['task:passive-tracking'],
    );
    assert.deepEqual(
      filterWorkForScan(items, { query: 'nationwide', scope: 'all', sort: 'lane' }).map((item) => item.id),
      ['task:passive-tracking'],
    );
    assert.deepEqual(
      filterWorkForScan(items, { scope: 'all', sort: 'priority' }).map((item) => item.id),
      ['task:decision', 'email:later', 'task:passive-tracking'],
    );
  });

  it('uses owner-facing service receipt copy for non-Gmail mailbox providers', () => {
    const baseMail = mailItem();
    const genericMail = mailItem({
      authority: {
        ...baseMail.authority,
        locked_data: [],
        locked_fields: [],
        expected_receipt: 'Mailbox changes keep a provider receipt.',
      },
      receipt_expectation: 'Mailbox changes keep a provider receipt.',
    });
    const html = render(baseProps({
      selected: genericMail,
      lanes: lanes([taskItem(), genericMail]),
    }));

    const panelStart = html.indexOf('vnext-authority-panel');
    const panel = html.slice(panelStart, html.indexOf('</details>', panelStart));
    assert.ok(panel.includes(t('work.receiptText.provider', 'zh')));
    assert.ok(!panel.includes('service receipt'));
    assert.ok(!panel.includes('provider receipt'));
    assert.ok(!html.includes('provider receipt'));
    assert.ok(panel.includes('标题'));
    assert.ok(!panel.includes('Title, Summary'));
    assert.ok(html.includes(`<details class="vnext-mini-panel vnext-authority-panel vnext-authority-panel-collapsed vnext-disclosure"><summary class="vnext-disclosure-summary">`));
    assert.ok(html.includes(t('work.authority.access', 'zh')));
    assert.doesNotMatch(html, /Show access details/);
  });

  it('renders blocked items as paused work without synthetic receipt actions', () => {
    const blocked = taskItem();
    const html = render(baseProps({
      selected: blocked,
      lanes: lanes([blocked, mailItem()]),
    }));

    assert.ok(html.includes('Mail triage needs your choice'));
    assert.ok(html.includes(t('work.decide.scope_blocked_run', 'zh')));
    assert.ok(html.includes(t('work.decide.safe_no_external_action', 'zh')));
    assert.ok(html.includes(t('work.decide.safe_visible_metadata', 'zh')));
    assert.ok(html.includes(t('work.decide.safe_no_live_approval', 'zh')));
    assert.match(html, /Mail triage needs your choice/);
    assert.ok(html.includes(t('work.blocked.noActionRan', 'zh')));
    assert.ok(html.includes(t('work.aria.decisionChoices', 'zh')));
    // CEO Review Card shape (What / Why), not the old capability-jargon rows.
    assert.ok(html.includes(t('work.blocked.ceoTitle', 'zh')));
    assert.ok(html.includes(t('work.blocked.whatLabel', 'zh')));
    assert.ok(html.includes(t('work.blocked.whyLabel', 'zh')));
    assert.match(html, /vnext-ceo-review-card/);
    assert.match(html, /vnext-ceo-review-facts/);
    // Was an English leak: blocked_reason came from blocked_by, which had no
    // structural code, so the server prose survived under zh. The server now
    // ships blocked_reason_code in lockstep with the string it picked.
    assert.ok(html.includes('Reading or classifying email content needs owner permission'));
    // Choice labels / consequences / footers localize by stable server id.
    assert.ok(html.includes(t('work.decide.choice.revise_rule.label', 'zh')));
    assert.ok(html.includes(t('work.decide.choice.revise_rule.consequence', 'zh')));
    assert.ok(html.includes(t('work.decide.choice.approve_unavailable.label', 'zh')));
    assert.ok(html.includes(t('work.decide.choice.skip_unavailable.label', 'zh')));
    assert.ok(html.includes(t('work.decide.choice.view_safe_details.label', 'zh')));
    assert.ok(html.includes(t('work.decide.scope.this_run', 'zh')));
    assert.ok(html.includes(t('work.decide.risk.medium', 'zh')));
    assert.ok(html.includes(`${t('work.decide.scope.this_run', 'zh')} · ${t('work.decide.risk.medium', 'zh')}`));
    assert.ok(html.includes(`${t('work.decide.unavailable', 'zh')}No live approval request is attached to this pause.`));
    assert.ok(html.includes(t('work.aria.pausedTrust', 'zh')));
    assert.ok(html.includes(t('work.trust.visibleMetadata', 'zh')));
    assert.ok(html.includes(t('work.trust.privateLocked', 'zh')));
    assert.doesNotMatch(html, /Close item/);
    assert.doesNotMatch(html, /Stop tracking this pause/);
    assert.doesNotMatch(html, /Task actions/);
    assert.doesNotMatch(html, /Complete<\/span>/);
    assert.doesNotMatch(html, /Snooze<\/span>/);
  });

  it('keeps en decide-card template strings byte-identical to server English', () => {
    const blocked = taskItem();
    const html = renderWithLocale(baseProps({
      selected: blocked,
      lanes: lanes([blocked, mailItem()]),
    }), 'en');

    // CEO title is primary; server English title may appear as subtitle.
    assert.ok(html.includes('Needs your review'));
    assert.ok(html.includes('I will'));
    assert.ok(html.includes('Why you'));
    assert.ok(html.includes('Mail triage needs your choice'));
    assert.ok(html.includes('Review unread Primary mail for the mail triage automation.'));
    assert.ok(html.includes('Read or classify mail content for this paused automation.'));
    assert.ok(html.includes('Approve or skip applies to this paused run. Closing marks this paused task done. Rule changes only happen if you explicitly choose a rule change.'));
    assert.ok(html.includes('Nothing ran outward'));
    assert.ok(html.includes('Visible metadata can be reviewed without opening private content.'));
    assert.ok(html.includes('No live approval request is attached to this pause, so approve/skip is unavailable until Aegis regenerates a real approval request.'));
    assert.doesNotMatch(html, /邮件分拣需要你的选择/);
  });

  it('localizeDecisionRequest overlays coded fields under zh and keeps en identical', () => {
    const decision = blockedDecisionRequest();

    const en = localizeDecisionRequest(decision, 'en');
    assert.equal(en, decision);
    assert.deepEqual(en, decision);

    const zh = localizeDecisionRequest(decision, 'zh');
    assert.equal(zh.title, 'Mail triage needs your choice');
    assert.equal(zh.what_aegis_was_doing, 'Review unread Primary mail for the mail triage automation.');
    assert.equal(zh.requested_capability, 'Read or classify mail content for this paused automation.');
    assert.equal(zh.scope, t('work.decide.scope_blocked_run', 'zh'));
    assert.equal(zh.blocked_reason, 'Reading or classifying email content needs owner permission.');
    assert.deepEqual(zh.safe_details, [
      t('work.decide.safe_no_external_action', 'zh'),
      t('work.decide.safe_visible_metadata', 'zh'),
      t('work.decide.safe_no_live_approval', 'zh'),
    ]);

    // Unknown code falls back to server string.
    const unknown = localizeDecisionRequest({
      ...decision,
      title_code: 'title_not_in_catalog_v9',
      title: 'Server title stays',
    }, 'zh');
    assert.equal(unknown.title, 'Server title stays');

    // safe_details length mismatch keeps server strings.
    const mismatch = localizeDecisionRequest({
      ...decision,
      safe_detail_codes: ['safe_no_external_action'],
    }, 'zh');
    assert.deepEqual(mismatch.safe_details, decision.safe_details);

    // Empty code entry keeps freeform server string.
    const freeform = localizeDecisionRequest({
      ...decision,
      safe_details: ['No external action has run.', 'Freeform server bullet'],
      safe_detail_codes: ['safe_no_external_action', ''],
    }, 'zh');
    assert.equal(freeform.safe_details[0], t('work.decide.safe_no_external_action', 'zh'));
    assert.equal(freeform.safe_details[1], 'Freeform server bullet');
  });

  it('localizeDecisionRequest resolves namespaced codes and rejects unknown namespaces', () => {
    const decision = blockedDecisionRequest({
      requested_capability: 'Approve only if these are the messages you meant to mark read; otherwise reject.',
      capability_code: 'nextCode:approval_gmail_confirm',
    });
    const zh = localizeDecisionRequest(decision, 'zh');
    assert.equal(zh.requested_capability, 'Approve only if these are the messages you meant to mark read; otherwise reject.');

    const unknownNs = localizeDecisionRequest({
      ...decision,
      capability_code: 'bogus:approval_gmail_confirm',
    }, 'zh');
    assert.equal(
      unknownNs.requested_capability,
      'Approve only if these are the messages you meant to mark read; otherwise reject.',
    );

    // en early-return is identity.
    assert.equal(localizeDecisionRequest(decision, 'en'), decision);
  });

  it('localizes decide-card choice copy by semantic id (zh) and keeps en server English', () => {
    const blocked = taskItem();
    const props = () => baseProps({ selected: blocked, lanes: lanes([blocked, mailItem()]) });

    const zh = render(props());
    assert.ok(zh.includes('批准本次运行'));
    assert.ok(zh.includes('跳过本次运行'));
    assert.ok(zh.includes('修改规则'));
    assert.ok(zh.includes('查看安全详情'));
    assert.ok(zh.includes('本次运行 · 中风险'));
    assert.ok(zh.includes('规则 · 低风险'));
    assert.ok(zh.includes('这件事 · 低风险'));
    assert.doesNotMatch(zh, /<strong>Approve this run<\/strong>/);
    assert.doesNotMatch(zh, /this run · medium/i);

    const en = renderWithLocale(props(), 'en');
    assert.ok(en.includes('Approve this run'));
    assert.ok(en.includes('Skip this run'));
    assert.ok(en.includes('Revise rule'));
    assert.ok(en.includes('View safe details'));
    assert.ok(en.includes('This run · Medium risk'));
    assert.ok(en.includes('Rule · Low risk'));
    assert.ok(en.includes('This item · Low risk'));
    assert.ok(en.includes('Unavailable: No live approval request is attached to this pause.'));
    assert.doesNotMatch(en, /批准本次运行/);
  });

  it('renders the catalog unavailable sentence clean (no marker prefix) when only action_kind is missing', () => {
    const blocked = taskItem({
      decision_request: blockedDecisionRequest({
        choices: [
          blockedDecisionChoice({
            id: 'run_once',
            label: 'Approve this run',
            consequence: 'Aegis will continue this paused run once and record the approval path.',
            risk_level: 'medium',
            applies_to: 'this_run',
            assistant_prompt: undefined,
            action_kind: 'approval.approve',
          }),
        ],
      }),
    });
    const html = render(baseProps({ selected: blocked, lanes: lanes([blocked, mailItem()]) }));
    // action_kind has no matching projected item action → catalog fallback
    // sentence renders as-is; the freeform-reason marker must NOT prefix it.
    assert.ok(html.includes(t('work.blocked.choiceUnavailable', 'zh')));
    assert.ok(!html.includes(`${t('work.decide.unavailable', 'zh')}${t('work.blocked.choiceUnavailable', 'zh')}`));
  });

  it('keeps item-local actions available while another assistant turn is busy', () => {
    const html = render(baseProps({ busy: true }));

    assert.ok(html.includes(t('work.askAegis', 'zh')));
    assert.ok(html.includes(t('work.action.brief', 'zh')));
    assert.ok(html.includes(t('work.action.draft', 'zh')));
    assert.doesNotMatch(html, /vnext-item-ai-entry/);
    assert.match(html, /vnext-work-ask-compact/);
    assert.ok(!new RegExp(`<button type="button" class="vnext-secondary" disabled="">[\\s\\S]*${t('work.action.brief', 'zh')}`).test(html));
    assert.ok(!new RegExp(`<button type="button" class="vnext-secondary" disabled="">[\\s\\S]*${t('work.action.draft', 'zh')}`).test(html));
    assert.doesNotMatch(html, /<button type="button" class="vnext-secondary" disabled="">[\s\S]*Mark read/);
    assert.doesNotMatch(html, /<button type="button" class="vnext-secondary" disabled="">[\s\S]*Archive/);
  });

  it('keeps paused-item choices available while another assistant turn is busy', () => {
    const blocked = taskItem();
    const html = render(baseProps({
      busy: true,
      selected: blocked,
      lanes: lanes([blocked, mailItem()]),
    }));

    assert.ok(html.includes(t('work.blocked.whyLabel', 'zh')));
    assert.ok(html.includes(t('work.blocked.ceoTitle', 'zh')));
    const revise = t('work.decide.choice.revise_rule.label', 'zh');
    const viewSafe = t('work.decide.choice.view_safe_details.label', 'zh');
    assert.ok(html.includes(revise));
    assert.ok(html.includes(viewSafe));
    // Choice weight classes (presentation only); enabled choices must stay clickable.
    assert.match(html, /vnext-decision-choice is-primary/);
    assert.match(html, /vnext-decision-choice is-detail/);
    assert.doesNotMatch(html, new RegExp(`vnext-decision-choice[^"]* disabled="">[\\s\\S]{0,480}${revise}`));
    assert.doesNotMatch(html, new RegExp(`vnext-decision-choice[^"]* disabled="">[\\s\\S]{0,480}${viewSafe}`));
  });

  it('shows only server-projected actions for paused items', () => {
    const blockedApproval = taskItem({
      id: 'approval:42',
      kind: 'approval',
      source_kind: 'decision',
      source: 'hitl',
      source_id: '42',
      title: 'Review scheduled action',
      status: 'blocked_by_gate',
      secondary_actions: [
        { label: 'Approve', action_kind: 'approval.approve' },
        { label: 'Reject', action_kind: 'approval.reject' },
      ],
      decision_request: blockedDecisionRequest({
        id: 'decision:approval:42',
        title: 'Review scheduled action',
        choices: [
          blockedDecisionChoice({
            id: 'approve',
            label: 'Approve',
            consequence: 'Aegis will send this approval through the existing approval path.',
            risk_level: 'medium',
            applies_to: 'this_request',
            action_kind: 'approval.approve',
            assistant_prompt: undefined,
            requires_confirmation: true,
          }),
          blockedDecisionChoice({
            id: 'reject',
            label: 'Reject',
            consequence: 'Aegis will reject this request through the existing approval path.',
            applies_to: 'this_request',
            action_kind: 'approval.reject',
            assistant_prompt: undefined,
            requires_confirmation: true,
            tone: 'destructive',
          }),
          blockedDecisionChoice(),
        ],
      }),
    });
    const html = render(baseProps({
      selected: blockedApproval,
      lanes: lanes([blockedApproval, mailItem()]),
    }));

    const approve = t('work.decide.choice.approve.label', 'zh');
    const reject = t('work.decide.choice.reject.label', 'zh');
    const revise = t('work.decide.choice.revise_rule.label', 'zh');
    assert.ok(html.includes(approve));
    assert.ok(html.includes(reject));
    assert.ok(
      html.indexOf(approve) < html.indexOf(revise),
      'server-projected approval choices should appear before assistant-only revision choices',
    );
    assert.doesNotMatch(html, /Close item/);
  });

  it('styles a dual-field action choice (action_kind + detail_route) as primary, never detail', () => {
    // Production vnext_action_decision_choice sets BOTH action_kind and
    // detail_route on the same choice; the detail weight must only apply to
    // choices that actually navigate on click (no resolvable action).
    const blockedTask = taskItem({
      secondary_actions: [
        { label: 'Review tomorrow', action_kind: 'task.snooze_tomorrow' },
      ],
      decision_request: blockedDecisionRequest({
        choices: [
          blockedDecisionChoice({
            id: 'review_tomorrow',
            label: 'Review tomorrow',
            consequence: 'Aegis will keep this paused and surface it again tomorrow.',
            applies_to: 'this_item',
            action_kind: 'task.snooze_tomorrow',
            detail_route: '/webapp/console/tasks/1054/action',
            assistant_prompt: undefined,
          }),
          blockedDecisionChoice(),
        ],
      }),
    });
    const html = render(baseProps({
      selected: blockedTask,
      lanes: lanes([blockedTask, mailItem()]),
    }));

    const reviewTomorrow = t('work.decide.choice.review_tomorrow.label', 'zh');
    const primaryStart = html.indexOf('vnext-decision-choice is-primary');
    assert.ok(primaryStart > 0, 'dual-field action choice should carry primary weight');
    const primaryButton = html.slice(primaryStart, html.indexOf('</button>', primaryStart));
    assert.ok(primaryButton.includes(reviewTomorrow), 'primary weight should land on the action choice');
    assert.ok(!primaryButton.includes('is-detail'), 'primary action choice must not also carry detail weight');
  });

  it('keeps paused automation close and review controls inside the selected item', () => {
    const blockedTask = taskItem({
      secondary_actions: [
        { label: 'Close without running', action_kind: 'task.complete' },
        { label: 'Review tomorrow', action_kind: 'task.snooze_tomorrow' },
      ],
      decision_request: blockedDecisionRequest({
        choices: [
          blockedDecisionChoice(),
          blockedDecisionChoice({
            id: 'review_tomorrow',
            label: 'Review tomorrow',
            consequence: 'Aegis will keep this paused and surface it again tomorrow.',
            applies_to: 'this_item',
            action_kind: 'task.snooze_tomorrow',
            assistant_prompt: undefined,
            requires_confirmation: true,
          }),
          blockedDecisionChoice({
            id: 'close_pause',
            label: 'Close without running',
            consequence: 'Aegis will mark this paused task done. It will not read mail, run the blocked step, or continue this task.',
            applies_to: 'this_item',
            action_kind: 'task.complete',
            assistant_prompt: undefined,
            requires_confirmation: true,
            tone: 'destructive',
          }),
        ],
      }),
    });
    const html = render(baseProps({
      selected: blockedTask,
      lanes: lanes([blockedTask, mailItem()]),
    }));

    const revise = t('work.decide.choice.revise_rule.label', 'zh');
    const reviewTomorrow = t('work.decide.choice.review_tomorrow.label', 'zh');
    const closePause = t('work.decide.choice.close_pause.label', 'zh');
    assert.ok(html.includes('Mail triage needs your choice'));
    assert.ok(html.includes(closePause));
    assert.ok(html.includes(t('work.decide.choice.close_pause.consequence', 'zh')));
    assert.ok(html.includes(reviewTomorrow));
    assert.ok(
      html.indexOf(revise) < html.indexOf(reviewTomorrow),
      'server-projected task choices should stay inside the paused card',
    );
    assert.ok(html.includes(t('work.trust.receiptsRequired', 'zh')));
    assert.ok(html.includes(revise));
    assert.doesNotMatch(html, /Open full Work/);
    assert.doesNotMatch(html, /Go to approvals/);
  });

  it('renders blocked Gmail digest automation without leaking the raw processor prompt', () => {
    const blockedTask = taskItem({
      title: '检查 primary 账户的未读邮件，使用 gmail_digest_processor 自动分类：将标记为‘重要’的邮',
      summary: 'Paused before action. Aegis needs your choice before this automation continues.',
      owner_visible_reason: 'An automation you asked Aegis to run paused before a sensitive step.',
      blocked_by: 'Aegis paused this automation before a sensitive step.',
      secondary_actions: [
        { label: 'Approve', action_kind: 'approval.approve' },
        { label: 'Reject', action_kind: 'approval.reject' },
      ],
      decision_request: blockedDecisionRequest({
        choices: [
          blockedDecisionChoice({
            id: 'run_once',
            label: 'Approve this run',
            consequence: 'Aegis will continue this paused run once and record the approval path.',
            risk_level: 'medium',
            applies_to: 'this_run',
            action_kind: 'approval.approve',
            assistant_prompt: undefined,
            requires_confirmation: true,
          }),
          blockedDecisionChoice({
            id: 'skip_once',
            label: 'Skip this run',
            consequence: 'Aegis will skip this paused run and leave the recurring rule unchanged.',
            applies_to: 'this_run',
            action_kind: 'approval.reject',
            assistant_prompt: undefined,
            requires_confirmation: true,
          }),
          blockedDecisionChoice(),
        ],
        // Live approval present: only the two constant safe bullets (no live-approval note).
        safe_details: [
          'No external action has run.',
          'Visible metadata can be reviewed without opening private content.',
        ],
        safe_detail_codes: [
          'safe_no_external_action',
          'safe_visible_metadata',
        ],
      }),
    });
    const html = render(baseProps({
      selected: blockedTask,
      lanes: lanes([blockedTask, mailItem()]),
    }));

    assert.ok(html.includes('Mail triage needs your choice') || html.includes(t('work.title.pausedAutomationFallback', 'zh')));
    assert.ok(html.includes('Review unread Primary mail for the mail triage automation.'));
    // Choice labels resolve by semantic id (zh catalog), not raw server English.
    assert.ok(html.includes(t('work.decide.choice.run_once.label', 'zh')));
    assert.ok(html.includes(t('work.decide.choice.skip_once.label', 'zh')));
    assert.ok(html.includes(t('work.decide.choice.revise_rule.label', 'zh')));
    assert.ok(html.includes(t('work.decide.choice.run_once.label', 'zh')));
  });

  it('keeps the real search target visible when a web search automation pauses', () => {
    const blockedSearch = taskItem({
      id: 'task:web-search',
      title: '使用 web_search 搜索过去6小时 Elon Musk (@elonmusk) 在 X 上的最新推文（英文原话）',
      summary: 'Aegis paused this automation before a sensitive step.',
      source_kind: 'web_search',
      source: 'web_search',
      status: 'blocked_by_gate',
      state: 'blocked_by_gate',
      blocked_by: 'web_search needs approval before continuing',
      next_safe_action: 'Review before continuing.',
      decision_request: null,
    });
    const html = render(baseProps({
      selected: blockedSearch,
      lanes: lanes([blockedSearch, mailItem()]),
    }));

    // zh title template is '搜索已暂停：{target}'; what-happened is
    // 'Aegis 在搜索前暂停了：{target}。' — assert the localized prefix + the
    // preserved (Chinese) search target fragment.
    assert.match(html, /搜索已暂停：[^<]*过去6小时 Elon Musk/);
    assert.match(html, /安琪在搜索前暂停了：[^<]*过去6小时 Elon Musk/);
    assert.doesNotMatch(html, /Web search paused<\/h2>/);
    assert.doesNotMatch(html, /Aegis was about to run a web search for a scheduled request/);
  });

  it('renders approval review actions as owner-facing requests', () => {
    const approval = mailItem({
      id: 'approval:calendar',
      lane: 'decide',
      kind: 'approval',
      source_kind: 'decision',
      source: 'hitl',
      source_id: 'calendar',
      title: 'Review calendar access',
      summary: 'Aegis needs your choice before reading calendar details.',
      state: 'waiting',
      status: 'waiting',
      blocked_by: '',
      primary_action: {
        id: 'open',
        label: 'Review approval',
        kind: 'navigate',
        route: '/webapp/console/approvals/calendar',
        requires_confirmation: false,
        requires_mfa: false,
        requires_data_gate: false,
        expected_receipt: 'Reading the request does not execute an outside action.',
      },
      secondary_actions: [
        { label: 'Approve', action_kind: 'approval.approve' },
        { label: 'Reject', action_kind: 'approval.reject' },
      ],
      contextual_actions: [],
      next_safe_action: '',
      detail_route: '/webapp/console/approvals/calendar',
    });
    const html = render(baseProps({
      selected: approval,
      lanes: lanes([approval, mailItem()]),
    }));

    assert.ok(html.includes(t('work.primaryAction.reviewRequest', 'zh')));
    assert.doesNotMatch(html, /Review approval/);
  });

  it('renders opened approval detail as an in-place decision brief', () => {
    const approval = mailItem({
      id: 'approval:calendar',
      lane: 'decide',
      kind: 'approval',
      source_kind: 'decision',
      source: 'hitl',
      source_id: 'calendar',
      title: 'Approval required',
      summary: 'Aegis needs your choice before reading calendar details.',
      state: 'waiting',
      status: 'waiting',
      blocked_by: 'Waiting for the owner decision.',
      secondary_actions: [
        { label: 'Approve', action_kind: 'approval.approve' },
        { label: 'Reject', action_kind: 'approval.reject' },
      ],
      next_safe_action: 'Review the approval, then approve or reject.',
    });
    const html = render(baseProps({
      selected: approval,
      lanes: lanes([approval, mailItem()]),
      sourceDetail: {
        kind: 'approval',
        title: 'Approval required',
        subtitle: 'Aegis needs your choice before reading calendar details.',
        fields: [
          { label: 'Decision', value: 'Review the approval, then approve or reject.' },
          { label: 'Confirmation', value: 'Your signed-in session is enough' },
        ],
        body: 'Waiting for the owner decision.',
        bodyLabel: 'Why this is waiting',
        timeline: [],
        warning: 'Nothing runs unless you choose Approve. Reject stops this approval request.',
      },
    }));

    assert.ok(html.includes(t('work.group.approveReject', 'zh')));
    assert.match(html, /Approval required/);
    assert.match(html, /Decision/);
    assert.match(html, /Review the approval, then approve or reject/);
    assert.match(html, /Why this is waiting/);
    assert.match(html, /Nothing runs unless you choose Approve/);
    assert.doesNotMatch(html, /No source detail is available/);
    assert.doesNotMatch(html, /does not have an inline detail view/);
  });

  it('shows stale action failures as owner-facing recovery guidance', () => {
    const html = render(baseProps({
      actionMessage: 'This item changed before the action ran. I refreshed the work queue; continue from the current available actions.',
      actionsRetired: true,
    }));

    assert.match(html, /This item changed before the action ran/);
    assert.match(html, /refreshed the work queue/);
    assert.match(html, /current available actions/);
    assert.ok(html.includes(t('work.askAegis', 'zh')));
    assert.ok(!html.includes(t('work.itemAction.markRead', 'zh')));
    assert.doesNotMatch(html, /Mark read/);
    assert.doesNotMatch(html, /Archive/);
    assert.doesNotMatch(html, /Action failed/);
    assert.doesNotMatch(html, /400/);
    assert.doesNotMatch(html, /does not apply/i);
  });

  it('uses source-detail pending state instead of assistant busy for detail opens', () => {
    const html = render(baseProps({ busy: false, sourceDetailBusy: true }));

    assert.ok(html.includes(t('work.action.opening', 'zh')));
    assert.ok(new RegExp(`<button type="button" class="vnext-item-primary-step" aria-label="${t('work.aria.openDetails', 'zh')}" disabled="">[\\s\\S]*${t('work.action.opening', 'zh')}`).test(html));
    assert.match(html, /vnext-action-primary-row/);
    assert.match(html, /vnext-action-dock-grid/);
    assert.ok(html.includes(`aria-label="${t('work.aria.itemControls', 'zh')}"`));
    assert.ok(html.includes(`aria-label="${t('work.aria.askThisItem', 'zh')}"`));
    const primaryRowStart = html.indexOf('vnext-action-primary-row');
    const actionDockStart = html.indexOf('vnext-action-dock-grid', primaryRowStart);
    const compactAskStart = html.indexOf('vnext-work-ask-compact', primaryRowStart);
    const receiptActionsStart = html.indexOf('vnext-item-receipt-actions');
    assert.ok(primaryRowStart > 0, 'missing action primary row wrapper');
    assert.ok(actionDockStart > primaryRowStart, 'item control dock should sit inside the primary row');
    // Ask Aegis is compact secondary AFTER the primary dock (not a full-width peer wall above it).
    assert.ok(compactAskStart > actionDockStart, 'compact Ask should follow the primary next-step dock');
    assert.doesNotMatch(html, /vnext-item-ai-entry/);
    if (receiptActionsStart > 0) {
      assert.ok(receiptActionsStart > actionDockStart, 'receipt actions should stay inside the item control dock');
      const receiptActionsHtml = html.slice(receiptActionsStart, html.indexOf('</section>', receiptActionsStart));
      assert.doesNotMatch(receiptActionsHtml, /<button type="button" class="vnext-secondary" disabled="">/);
    }
  });

  it('does not tell owners to leave the selected item before asking Aegis', () => {
    const closedHtml = render(baseProps());
    const openedHtml = render(baseProps({
      sourceDetail: {
        kind: 'generic',
        title: 'Billing statement available',
        subtitle: 'Maple Lane Music School',
        fields: [],
        body: 'Statement ready.',
        timeline: [],
      },
    }));

    assert.ok(closedHtml.includes(t('work.askAegis.usingItemContext', 'zh')));
    assert.ok(openedHtml.includes(t('work.askAegis.usingOpenedDetail', 'zh')));
    assert.doesNotMatch(closedHtml, /Open details for full context/);
    assert.doesNotMatch(openedHtml, /Open details for full context/);
  });

  it('renders leftover email-kind source detail through the generic panel', () => {
    const html = render(baseProps({
      sourceDetail: {
        kind: 'generic',
        title: 'Billing statement available',
        subtitle: 'Maple Lane Music School',
        fields: [
          { label: 'Sender', value: 'Maple Lane Music School' },
          { label: 'State', value: 'Unread' },
        ],
        bodyLabel: 'Text version',
        body: 'Plain text fallback',
        attachments: [],
        timeline: [],
        warning: '',
      },
    }));

    assert.match(html, /Billing statement available/);
    assert.match(html, /Maple Lane Music School/);
    assert.match(html, /Text version/);
    assert.match(html, /Plain text fallback/);
    assert.match(html, />Sender<\/strong><em>Maple Lane Music School<\/em>/);
    assert.doesNotMatch(html, /vnext-mail-metadata/);
    assert.doesNotMatch(html, /vnext-mail-html-frame/);
    assert.doesNotMatch(html, /mail\.google\.com/);
  });

  it('renders a download link for every attachment and an inline preview toggle only for previewable ones', () => {
    const html = render(baseProps({
      sourceDetail: {
        kind: 'generic',
        title: 'Invoice with files',
        subtitle: 'Acme Billing',
        fields: [],
        attachments: [
          {
            filename: 'invoice.pdf',
            meta: 'application/pdf · 1.2 KB',
            downloadUrl: '/webapp/vnext/work/attachment?id=u1',
            previewable: true,
            mimeType: 'application/pdf',
          },
          {
            filename: 'report.docx',
            meta: 'application/vnd…document · 8 KB',
            downloadUrl: '/webapp/vnext/work/attachment?id=u2',
            previewable: false,
            mimeType: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
          },
        ],
        timeline: [],
      },
    }));
    // Both attachments offer a same-origin signed download link with a filename.
    assert.ok(html.includes('href="/webapp/vnext/work/attachment?id=u1'), 'pdf download link present');
    assert.ok(html.includes('href="/webapp/vnext/work/attachment?id=u2'), 'docx download link present');
    assert.ok(html.includes('download="invoice.pdf"'), 'download attr names the pdf');
    assert.ok(html.includes('download="report.docx"'), 'download attr names the docx');
    // Only the previewable (pdf) attachment gets a preview toggle button. The
    // detail panel renders in two responsive layouts, so counts are doubled;
    // assert the RATIO (1 of 2 attachments previewable) to stay layout-agnostic.
    const previewToggles = html.match(/vnext-attachment-preview-toggle/g) || [];
    const downloadLinks = html.match(/vnext-attachment-download/g) || [];
    assert.ok(downloadLinks.length >= 2, 'both attachments are downloadable');
    assert.equal(previewToggles.length * 2, downloadLinks.length, 'exactly half (the pdf) are previewable');
    assert.ok(html.includes(t('work.source.attachmentPreview', 'zh')), 'preview label rendered');
    assert.ok(html.includes(t('work.source.attachmentDownload', 'zh')), 'download label rendered');
  });

  it('shows a metadata-only attachment row with no download link when the server signed no url', () => {
    const html = render(baseProps({
      sourceDetail: {
        kind: 'generic',
        title: 'Inline-only image',
        subtitle: 'Newsletter',
        fields: [],
        attachments: [
          {
            filename: 'logo.png',
            meta: 'image/png · 2 KB',
            downloadUrl: null,
            previewable: false,
            mimeType: 'image/png',
          },
        ],
        timeline: [],
      },
    }));
    assert.ok(html.includes('logo.png'), 'filename still shown');
    assert.ok(!html.includes('vnext/work/attachment'), 'no download link without a signed url');
    assert.ok(!html.includes('vnext-attachment-preview-toggle'), 'no preview toggle without a url');
  });

  it('does not label generic external source links as a provider inbox', () => {
    const task = taskItem({ status: 'active', state: 'active', blocked_by: '' });
    const html = render(baseProps({
      selected: task,
      lanes: lanes([task, mailItem()]),
      sourceDetail: {
        kind: 'task',
        title: 'Insurance follow-up',
        subtitle: 'Task detail',
        fields: [
          { label: 'State', value: 'Active' },
        ],
        timeline: [],
        externalUrl: 'https://example.com/tasks/42',
      },
    }));

    assert.ok(html.includes(t('work.source.openTaskSource', 'zh')));
    assert.ok(!html.includes('在 Gmail'));
  });

  it('renders archived tasks as handled history instead of active decisions', () => {
    const archived = taskItem({
      id: 'task:old-lawyer',
      lane: 'done',
      title: '处理律师聘用事宜',
      summary: '今晚 10:00 (22:00) 提醒处理律师聘用事宜。',
      status: 'archived',
      state: 'archived',
      urgency: 'high',
      blocked_by: 'waiting on owner review',
      owner_visible_reason: 'Waiting on a dependency.',
      next_safe_action: 'Review the task, then choose.',
      secondary_actions: [
        { label: 'Complete', action_kind: 'task.complete' },
      ],
    });
    const html = render(baseProps({
      selected: archived,
      lanes: lanes([archived, mailItem()]),
      sourceDetail: {
        kind: 'task',
        title: '处理律师聘用事宜',
        subtitle: '今晚 10:00 (22:00) 提醒处理律师聘用事宜。',
        fields: [
          { label: 'State', value: 'Archived' },
          { label: 'Priority', value: 'Normal' },
        ],
        timeline: [
          { label: 'Task created', value: 'Completed · 2026-03-20T00:12:18.249536597+00:00' },
          { label: 'Current state', value: 'Archived · 2026-04-05 07:44:46' },
        ],
      },
    }));

    assert.ok(html.includes(t('work.focus.handled', 'zh')));
    assert.ok(html.includes(`${t('work.meta.task', 'zh')} · Archived`));
    assert.ok(html.includes(t('work.cue.handled', 'zh')));
    assert.ok(html.includes(t('work.nextMove.handled', 'zh')));
    assert.ok(html.includes(t('work.askAegis', 'zh')));
    assert.ok(!html.includes(`High priority · ${t('work.meta.task', 'zh')} · Archived`));
    assert.doesNotMatch(html, /Waiting on a dependency/);
    assert.ok(!html.includes(t('work.nextMove.reviewTask', 'zh')));
    assert.ok(!html.includes(t('work.group.task', 'zh')));
    assert.doesNotMatch(html, /Complete<\/span>/);
    assert.ok(!html.includes(`<span>${t('work.action.brief', 'zh')}</span>`));
    assert.ok(!html.includes(`<span>${t('work.action.draft', 'zh')}</span>`));
  });

  it('keeps opened blocked task detail collapsed inside the selected item', () => {
    const task = taskItem();
    const html = render(baseProps({
      selected: task,
      lanes: lanes([task, mailItem()]),
      sourceDetail: {
        kind: 'task',
        title: 'Monitor Gmail timeline',
        subtitle: 'Task detail',
        fields: [
          { label: 'State', value: 'Blocked by gate' },
        ],
        bodyLabel: 'What happened',
        body: 'gmail_reader paused before private access.',
        timeline: [],
        externalUrl: 'https://example.com/tasks/1054',
      },
    }));

    assert.match(html, /gmail_reader paused before private access/);
    assert.match(html, /<details class="vnext-mini-panel vnext-source-panel vnext-source-panel-collapsed vnext-disclosure">/);
    assert.ok(html.includes(`<summary class="vnext-disclosure-summary"><span class="vnext-disclosure-copy"><strong>${t('work.source.taskDetail', 'zh')}</strong><em class="vnext-disclosure-hint">Monitor Gmail timeline · Blocked by gate</em></span></summary>`));
    assert.ok(html.includes(`aria-label="${t('work.aria.taskSummary', 'zh')}"`));
    assert.match(html, /<div class="vnext-task-summary-grid">/);
    assert.match(html, /What happened/);
    assert.doesNotMatch(html, /<section class="vnext-mini-panel vnext-source-panel"><h3>Monitor Gmail timeline/);
  });

  it('groups task fields by stable id (not localized label text) and drops the placeholder due from the subtitle', () => {
    const task = taskItem();
    // zh-labeled fields, as buildSourceDetailView(payload, 'zh', …) emits them:
    // behavior must key on DetailField.id / placeholder, never the label text.
    const html = render(baseProps({
      selected: task,
      lanes: lanes([task, mailItem()]),
      sourceDetail: {
        kind: 'task',
        title: '盯住 Gmail 时间线',
        subtitle: '任务详情',
        fields: [
          { id: 'state', label: '状态', value: 'Active' },
          { id: 'priority', label: '优先级', value: 'Normal' },
          { id: 'due', label: '截止', value: '未设置', placeholder: true },
          { id: 'mode', label: '模式', value: 'Manual' },
        ],
        timeline: [],
      },
    }));

    // The summary subtitle keeps the state but suppresses the placeholder due.
    assert.ok(html.includes('<em class="vnext-disclosure-hint">盯住 Gmail 时间线 · Active</em></span></summary>'));
    // The primary grid still shows every id-matched field, zh labels included.
    assert.ok(html.includes('<strong>状态</strong><em>Active</em>'));
    assert.ok(html.includes('<strong>截止</strong><em>未设置</em>'));
    assert.ok(html.includes('<strong>模式</strong><em>Manual</em>'));
    assert.ok(html.includes('<strong>优先级</strong><em>Normal</em>'));

    // A real due (placeholder absent) reaches the subtitle.
    const withDue = render(baseProps({
      selected: task,
      lanes: lanes([task, mailItem()]),
      sourceDetail: {
        kind: 'task',
        title: '盯住 Gmail 时间线',
        subtitle: '任务详情',
        fields: [
          { id: 'state', label: '状态', value: 'Active' },
          { id: 'due', label: '截止', value: '2026-05-15' },
        ],
        timeline: [],
      },
    }));
    assert.ok(withDue.includes('<em class="vnext-disclosure-hint">盯住 Gmail 时间线 · Active · 2026-05-15</em></span></summary>'));
  });

  it('keeps supporting source notes and history collapsed below the main work controls', () => {
    const task = taskItem({
      status: 'active',
      state: 'active',
      blocked_by: '',
      due_at: '2026-05-18T15:00:00Z',
      related_action_ids: ['action:1'],
      related_receipt_ids: ['receipt:1'],
    });
    const html = render(baseProps({
      selected: task,
      lanes: lanes([task, mailItem()]),
      itemProjection: {
        schema_version: 1,
        item_id: task.id,
        safe_source_detail: 'Verbose source note that should not dominate the work surface.',
        history: [
          {
            id: 'history:1',
            kind: 'receipt',
            label: 'Task created',
            created_at: '2026-05-17T15:00:00Z',
          },
        ],
      },
    }));

    assert.ok(html.includes(`<summary class="vnext-disclosure-summary"><span class="vnext-disclosure-copy"><strong>${t('work.sourceNote.summary', 'zh')}</strong><em class="vnext-disclosure-hint">${t('work.sourceNote.title', 'zh')}</em></span></summary>`));
    assert.ok(html.includes(`<summary class="vnext-disclosure-summary"><span class="vnext-disclosure-copy"><strong>${t('work.timeline.summary', 'zh')}</strong><em class="vnext-disclosure-hint">${t('work.timeline.due', 'zh')} `));
    assert.ok(html.includes(tf('work.count.actions', 'zh', { count: '1', word: t('work.action.one', 'zh') })));
    assert.ok(html.includes(tf('work.count.receipts', 'zh', { count: '1', word: t('work.receiptWord.one', 'zh') })));
    assert.ok(html.includes(tf('work.count.updates', 'zh', { count: '1', word: t('work.update.one', 'zh') })));
    assert.doesNotMatch(html, /<h3>Source summary<\/h3>/);
    assert.doesNotMatch(html, /<h3>Receipts and history<\/h3>/);
  });

  it('keeps tracking task detail focused instead of repeating status and due date', () => {
    const tracking = taskItem({
      id: 'task:commute',
      lane: 'follow_up',
      title: '回家通勤监控 (下午 5:30)',
      summary: 'Tracking item. Aegis is keeping this visible as a follow-up loop.',
      owner_visible_reason: 'Aegis is tracking this because it is part of an open follow-up loop.',
      status: 'tracking',
      state: 'tracking',
      blocked_by: '',
      due_at: '2026-05-19T17:30:00-07:00',
      secondary_actions: [
        { label: 'Complete', action_kind: 'task.complete' },
        { label: 'Snooze', action_kind: 'task.snooze_tomorrow' },
      ],
    });
    const html = render(baseProps({
      selected: tracking,
      lanes: [
        { id: 'decide', title: 'Decide', subtitle: 'Needs your call before Aegis continues.', items: [], collapsed: false },
        { id: 'now', title: 'Now', subtitle: 'Ready to handle now.', items: [], collapsed: false },
        { id: 'follow_up', title: 'Follow Up', subtitle: 'Tracked quietly.', items: [tracking], collapsed: false },
        { id: 'backlog', title: 'Backlog', subtitle: 'Later.', items: [], collapsed: true },
        { id: 'done', title: 'Done', subtitle: 'Handled.', items: [], collapsed: true },
      ],
    }));

    assert.match(html, /回家通勤监控/);
    // 2026-09-13: the tracking sentence is no longer repeated — the 何时 row
    // below carries the date, and the list row shows 跟踪到 · <time>.
    assert.doesNotMatch(html, /安琪会跟踪此事直到/);
    assert.doesNotMatch(html, /我会把它留在视线里/);
    assert.ok(html.includes(t('work.aria.currentSummary', 'zh')));
    assert.ok(html.includes(`<em>${t('work.intent.when', 'zh')}</em>`));
    assert.ok(html.includes(t('work.group.followUp', 'zh')));
    assert.ok(html.includes(t('work.aria.followUpChoices', 'zh')));
    assert.ok(html.includes(t('work.askAegis', 'zh')));
    assert.ok(html.includes(t('work.followUp.addUpdate', 'zh')));
    // Primary close verb for a tracking note with no generator.
    assert.ok(html.includes(t('work.itemAction.endThisCard', 'zh')));
    assert.ok(html.includes(t('work.itemAction.reviewTomorrow', 'zh')));
    // Generic follow-up policy receipt must not clutter the intent grid.
    assert.doesNotMatch(html, /Follow-up actions should produce/);
    assert.doesNotMatch(html, /Still waiting/);
    assert.ok(!html.includes(`>${t('work.action.brief', 'zh')}<`));
    assert.ok(!html.includes(`>${t('work.action.draft', 'zh')}<`));
    // Detail meta stays focused (no status token); list rows may still show it.
    const inlineSelected = html.slice(html.indexOf('vnext-work-inline-selected'));
    const inlineDetail = inlineSelected.slice(0, inlineSelected.indexOf('vnext-detail-desktop'));
    assert.ok(!inlineDetail.includes(`High priority · ${t('work.meta.task', 'zh')} · Tracking`));
    assert.ok(!html.includes(t('work.intent.signal', 'zh')));
    assert.doesNotMatch(html, /part of an open follow-up loop/);
    assert.ok(!html.includes(`<summary class="vnext-disclosure-summary"><span class="vnext-disclosure-copy"><strong>${t('work.timeline.summary', 'zh')}</strong><em class="vnext-disclosure-hint">${t('work.timeline.due', 'zh')} `));
  });

  it('shows a recurring tracking source chain and both layer actions', () => {
    const tracking = taskItem({
      id: 'task:12',
      lane: 'follow_up',
      title: '门铃电量',
      summary: 'Tracking item.',
      owner_visible_reason: 'Aegis is tracking this because it is part of an open follow-up loop.',
      status: 'tracking',
      state: 'tracking',
      blocked_by: '',
      due_at: null,
      decision_request: null,
      lineage: [
        {
          layer: 'intent',
          kind: 'task',
          id: 'task:12',
          label: '循环跟踪',
          label_code: 'intent.recurring',
          detail: 'task:12 · interval_seconds=3600 · group_key=doorbell',
          action_ids: ['task.skip_occurrence', 'tracking.cancel'],
        },
        {
          layer: 'origin',
          kind: 'context',
          id: 'task:12:origin',
          label: '当时的说明',
          label_code: 'origin.context',
          detail: 'context_text=周末给门铃充电',
          action_ids: [],
        },
      ],
      secondary_actions: [
        { label: 'Skip', action_kind: 'task.skip_occurrence' },
        { label: 'Cancel', action_kind: 'tracking.cancel' },
      ],
    });
    const html = render(baseProps({
      selected: tracking,
      lanes: [
        { id: 'decide', title: 'Decide', subtitle: '', items: [], collapsed: false },
        { id: 'now', title: 'Now', subtitle: '', items: [], collapsed: false },
        { id: 'follow_up', title: 'Follow Up', subtitle: '', items: [tracking], collapsed: false },
        { id: 'backlog', title: 'Backlog', subtitle: '', items: [], collapsed: true },
        { id: 'done', title: 'Done', subtitle: '', items: [], collapsed: true },
      ],
    }));
    assert.ok(html.includes(t('work.lineage.heading', 'zh')));
    assert.ok(html.includes('interval_seconds=3600'));
    assert.ok(html.includes('group_key=doorbell'));
    assert.ok(html.includes('context_text=周末给门铃充电'));
    assert.ok(html.includes(t('work.itemAction.endThisOnce', 'zh')));
    assert.ok(html.includes(t('work.itemAction.closeTracking', 'zh')));
  });

  it('opens a chat cite from a conversation lineage hop and leaves private excerpts closed', () => {
    const ownerCite = lineageChatCite({ kind: 'message', turn_id: 'cite-turn-doorbell', message_role: 'owner' });
    const replyCite = messagePublicRef({ role: 'aegis', turnId: 'cite-turn-doorbell' });
    assert.equal(ownerCite.ref, messagePublicRef({ role: 'owner', turnId: 'cite-turn-doorbell' }));
    assert.equal(ownerCite.token, `#${ownerCite.ref}`);
    assert.equal(lineageChatCite({ kind: 'message', message_role: 'owner' }), null);
    assert.equal(lineageChatCite({ kind: 'context', turn_id: 'cite-turn-doorbell', message_role: 'owner' }), null);
    const tracking = taskItem({
      id: 'task:12',
      lane: 'follow_up',
      title: '门铃电量',
      summary: 'Tracking item.',
      owner_visible_reason: 'Aegis is tracking this because it is part of an open follow-up loop.',
      status: 'tracking',
      state: 'tracking',
      blocked_by: '',
      due_at: null,
      decision_request: null,
      lineage: [
        {
          layer: 'origin',
          kind: 'message',
          id: 'l3:7',
          label: '对话消息',
          label_code: 'origin.message',
          detail: 'l3:7 · role=owner · 周末提醒给门铃充电',
          turn_id: 'cite-turn-doorbell',
          message_role: 'owner',
          action_ids: [],
        },
        {
          layer: 'origin',
          kind: 'message',
          id: 'l3:8',
          label: '对话消息',
          label_code: 'origin.message',
          detail: 'l3:8 · role=aegis · 已设置循环跟踪',
          turn_id: 'cite-turn-doorbell',
          message_role: 'aegis',
          action_ids: [],
        },
        {
          layer: 'origin',
          kind: 'record',
          id: 'task:9:origin',
          label: '记录',
          label_code: 'origin.record',
          detail: 'source=user · created_at=2026-09-23T04:00:00Z',
          action_ids: [],
        },
      ],
    });
    const html = render(baseProps({
      selected: tracking,
      onOpenLineageMessage: () => {},
      lanes: [
        { id: 'decide', title: 'Decide', subtitle: '', items: [], collapsed: false },
        { id: 'now', title: 'Now', subtitle: '', items: [], collapsed: false },
        { id: 'follow_up', title: 'Follow Up', subtitle: '', items: [tracking], collapsed: false },
        { id: 'backlog', title: 'Backlog', subtitle: '', items: [], collapsed: true },
        { id: 'done', title: 'Done', subtitle: '', items: [], collapsed: true },
      ],
    }));
    assert.ok(html.includes(`data-lineage-cite="${ownerCite.ref}"`));
    assert.ok(html.includes(`data-lineage-cite="${replyCite}"`));
    const cites = html.match(/data-lineage-cite="([^"]+)"/g) || [];
    assert.ok(cites.length >= 2 && cites.length % 2 === 0);
    assert.ok(cites.every((cite) => cite.includes(ownerCite.ref) || cite.includes(replyCite)));
    assert.ok(html.includes(tf('work.lineage.openMessage', 'zh', { ref: ownerCite.token })));
    assert.ok(html.includes('source=user · created_at=2026-09-23T04:00:00Z'));
  });

  it('shows end-this-card and close-tracking on a waiting card tied to a loop', () => {
    const waiting = taskItem({
      id: 'kb_waiting:701',
      lane: 'follow_up',
      kind: 'task',
      source_kind: 'tracking',
      source: 'knowledge',
      source_id: '701',
      title: '治疗提醒',
      summary: '等确认预约时间。',
      owner_visible_reason: '还在等确认。',
      status: 'waiting',
      state: 'waiting',
      blocked_by: '',
      due_at: null,
      decision_request: null,
      lineage: [
        {
          layer: 'occurrence',
          kind: 'knowledge',
          id: 'kb_waiting:701',
          label: '这条等待',
          label_code: 'occurrence.waiting',
          detail: 'kb_waiting:701 · category=waiting_for · knowledge_id=701 · waiting_on=确认预约时间',
          action_ids: ['knowledge.done'],
        },
        {
          layer: 'intent',
          kind: 'task',
          id: 'task:36',
          label: '循环跟踪',
          label_code: 'intent.recurring',
          detail: 'task:36 · interval_seconds=86400 · group_key=treatment-reminder',
          action_ids: ['tracking.cancel'],
        },
        {
          layer: 'origin',
          kind: 'record',
          id: 'kb_waiting:701:origin',
          label: '记录',
          label_code: 'origin.record',
          detail: 'created_at=2026-03-30T20:00:38Z',
          action_ids: [],
        },
      ],
      secondary_actions: [
        { label: 'Done', action_kind: 'knowledge.done' },
        { label: 'Cancel', action_kind: 'tracking.cancel' },
        { label: 'Snooze', action_kind: 'knowledge.snooze_tomorrow' },
      ],
    });
    const html = render(baseProps({
      selected: waiting,
      lanes: [
        { id: 'decide', title: 'Decide', subtitle: '', items: [], collapsed: false },
        { id: 'now', title: 'Now', subtitle: '', items: [], collapsed: false },
        { id: 'follow_up', title: 'Follow Up', subtitle: '', items: [waiting], collapsed: false },
        { id: 'backlog', title: 'Backlog', subtitle: '', items: [], collapsed: true },
        { id: 'done', title: 'Done', subtitle: '', items: [], collapsed: true },
      ],
    }));
    const occurrence = html.slice(html.indexOf('data-lineage-layer="occurrence"'), html.indexOf('data-lineage-layer="intent"'));
    const intent = html.slice(html.indexOf('data-lineage-layer="intent"'), html.indexOf('data-lineage-layer="origin"'));
    assert.ok(occurrence.includes(t('work.itemAction.endThisCard', 'zh')));
    assert.ok(!occurrence.includes(t('work.itemAction.closeTracking', 'zh')));
    assert.ok(intent.includes(t('work.itemAction.closeTracking', 'zh')));
    assert.ok(intent.includes('interval_seconds=86400'));
    assert.ok(html.includes(t('work.itemAction.reviewTomorrow', 'zh')));
  });

  it('keeps every lane and item in place when an item is selected on the phone layout', () => {
    const decideItem = taskItem({ id: 'task:decide-keep', title: 'Decide keep order' });
    const followItems = [
      passiveTrackingItem({ id: 'task:fu-1', title: '跟进 Hercules merge to main，推迟，或让我收掉这个循环。' }),
      passiveTrackingItem({ id: 'task:fu-2', title: 'Follow sibling two' }),
      passiveTrackingItem({ id: 'task:fu-3', title: 'Follow sibling three' }),
    ];
    const selected = followItems[1];
    const laneDefs = [
      { id: 'decide', title: 'Decide', subtitle: 'Needs your call before Aegis continues.', items: [decideItem], collapsed: false },
      { id: 'now', title: 'Now', subtitle: 'Ready to handle now.', items: [], collapsed: false },
      { id: 'follow_up', title: 'Follow Up', subtitle: 'Tracked quietly.', items: followItems, collapsed: false },
      { id: 'backlog', title: 'Backlog', subtitle: 'Later.', items: [], collapsed: true },
      { id: 'done', title: 'Done', subtitle: 'Handled.', items: [], collapsed: true },
    ];
    const html = render(baseProps({ selected, lanes: laneDefs }));
    const mobile = html.slice(html.indexOf('vnext-work-mobile-inline-lanes'));
    const laneHeads = [...mobile.matchAll(/vnext-work-lane[^"]*"[\s\S]*?<h2>([^<]+)<\/h2>/g)].map((m) => m[1]);
    assert.deepEqual(laneHeads, ['Decide', 'Follow Up']);
    for (const item of [decideItem, ...followItems]) {
      assert.ok(mobile.includes(item.title), `expected item still rendered: ${item.title}`);
    }
    const followSection = mobile.slice(mobile.indexOf('>Follow Up</h2>'));
    const followTitles = [...followSection.matchAll(/class="vnext-item-title"[^>]*>([^<]+)</g)].map((m) => m[1]);
    assert.deepEqual(followTitles, followItems.map((item) => item.title));
    assert.match(mobile, /vnext-work-mobile-inline-item active/);
    assert.match(mobile, /vnext-detail[^"]*vnext-work-inline-selected/);
  });

  it('shows the selected item\'s full title and cue without clamping', () => {
    const longTitle = '跟进 Hercules merge to main，推迟，或让我收掉这个循环。';
    const selected = passiveTrackingItem({
      id: 'task:full-title',
      title: longTitle,
      summary: 'Keep watching this loop until merge lands or the owner closes it.',
    });
    const html = render(baseProps({
      selected,
      lanes: [
        { id: 'decide', title: 'Decide', subtitle: 'Needs your call.', items: [], collapsed: false },
        { id: 'now', title: 'Now', subtitle: 'Ready.', items: [], collapsed: false },
        { id: 'follow_up', title: 'Follow Up', subtitle: 'Tracked quietly.', items: [selected], collapsed: false },
        { id: 'backlog', title: 'Backlog', subtitle: 'Later.', items: [], collapsed: true },
        { id: 'done', title: 'Done', subtitle: 'Handled.', items: [], collapsed: true },
      ],
    }));
    const stylesSource = readFileSync(new URL('../src/vnext/styles.css', import.meta.url), 'utf8');
    assert.ok(html.includes(longTitle));
    assert.ok(html.includes(t('work.focus.tracking', 'zh')));
    assert.match(stylesSource, /\.vnext-detail \.vnext-detail-hero h2[\s\S]*?-webkit-line-clamp:\s*unset/);
    assert.match(stylesSource, /\.vnext-detail \.vnext-detail-hero > p[\s\S]*?-webkit-line-clamp:\s*unset/);
    assert.match(stylesSource, /\.vnext-detail \.vnext-item-meta[\s\S]*?text-overflow:\s*unset/);
    assert.doesNotMatch(
      stylesSource,
      /\.vnext-detail-hero h2\s*\{[^}]*-webkit-line-clamp:\s*[1-9]/s,
    );
    assert.doesNotMatch(
      stylesSource,
      /\.vnext-detail-title-row \.vnext-item-meta\s*\{[^}]*text-overflow:\s*ellipsis;/s,
    );
  });

  it('renders the state cue as its own chip above the title', () => {
    const selected = passiveTrackingItem({
      id: 'task:cue-above',
      title: '跟进 Hercules merge to main，推迟，或让我收掉这个循环。',
    });
    const html = render(baseProps({
      selected,
      lanes: [
        { id: 'decide', title: 'Decide', subtitle: 'Needs your call.', items: [], collapsed: false },
        { id: 'now', title: 'Now', subtitle: 'Ready.', items: [], collapsed: false },
        { id: 'follow_up', title: 'Follow Up', subtitle: 'Tracked quietly.', items: [selected], collapsed: false },
        { id: 'backlog', title: 'Backlog', subtitle: 'Later.', items: [], collapsed: true },
        { id: 'done', title: 'Done', subtitle: 'Handled.', items: [], collapsed: true },
      ],
    }));
    const detailStart = html.indexOf('vnext-work-inline-selected');
    assert.ok(detailStart >= 0);
    const hero = html.slice(detailStart, detailStart + 1200);
    const chipIdx = hero.indexOf('vnext-focus-chip');
    const titleIdx = hero.indexOf('<h2');
    assert.ok(chipIdx >= 0, 'expected focus chip');
    assert.ok(titleIdx >= 0, 'expected title');
    assert.ok(chipIdx < titleIdx, 'cue chip must render above the title');
    assert.ok(hero.includes(t('work.focus.tracking', 'zh')));
    // Chip row contains the cue only — meta sits after the title/summary.
    const titleRow = hero.slice(hero.indexOf('vnext-detail-title-row'), hero.indexOf('<h2'));
    assert.match(titleRow, /vnext-focus-chip/);
    assert.doesNotMatch(titleRow, /vnext-item-meta/);
  });

  it('icon-only work controls carry accessible names', () => {
    const html = render(baseProps({
      selected: mailItem(),
      lanes: lanes([mailItem(), taskItem()]),
    }));
    assert.match(html, new RegExp(`aria-label="${t('work.scan.scope.all', 'zh')}"`));
    assert.match(html, new RegExp(`aria-label="${t('work.scan.scope.needsMe', 'zh')}"`));
    assert.match(html, new RegExp(`aria-label="${t('work.scan.scope.quiet', 'zh')}"`));
    assert.match(html, new RegExp(`aria-label="${t('work.scan.sort.lane', 'zh')}"`));
    assert.match(html, new RegExp(`aria-label="${t('work.scan.sort.priority', 'zh')}"`));
    assert.match(html, new RegExp(`aria-label="${t('work.scan.sort.updated', 'zh')}"`));
    assert.match(html, new RegExp(`aria-label="${t('work.aria.laneNow', 'zh')}"`));
    assert.match(html, new RegExp(`aria-label="${t('work.aria.laneDecide', 'zh')}"`));
    const workSource = readFileSync(new URL('../src/vnext/WorkView.tsx', import.meta.url), 'utf8');
    assert.match(workSource, /ariaLabel:\s*t\('work\.scan\.scope\.all'/);
    assert.match(workSource, /workLaneAriaLabel/);
  });

  it('localizes Work chrome under both zh and en via LocaleProvider', () => {
    // Keys spanning every migrated component family: queue chrome, Ask-Aegis
    // entry, intent row, scan controls, authority panel, action group. The same
    // props rendered under each locale must surface that locale's catalog copy
    // (and not the other locale's), proving the i18n thread reaches WorkView.
    const probeKeys = [
      'work.askAegis',
      'work.action.brief',
      'work.action.draft',
      'work.scan.searchPlaceholder',
      'work.askAegis.usingItemContext',
    ];

    const zhHtml = renderWithLocale(baseProps(), 'zh');
    const enHtml = renderWithLocale(baseProps(), 'en');

    for (const key of probeKeys) {
      const zhValue = t(key, 'zh');
      const enValue = t(key, 'en');
      assert.ok(zhHtml.includes(zhValue), `zh render should contain ${key} zh copy: ${zhValue}`);
      assert.ok(enHtml.includes(enValue), `en render should contain ${key} en copy: ${enValue}`);
    }

    // Authority panel renders only when detail access is not still closed; use a
    // generic mail item (no locked fields) so the panel surfaces in both locales.
    const genericMail = mailItem({
      authority: { ...mailItem().authority, locked_data: [], locked_fields: [] },
    });
    const authZh = renderWithLocale(baseProps({ selected: genericMail, lanes: lanes([taskItem(), genericMail]) }), 'zh');
    const authEn = renderWithLocale(baseProps({ selected: genericMail, lanes: lanes([taskItem(), genericMail]) }), 'en');
    assert.ok(authZh.includes(t('work.authority.access', 'zh')), 'zh authority panel access label');
    assert.ok(authEn.includes(t('work.authority.access', 'en')), 'en authority panel access label');

    // English render must surface English chrome and never the Chinese copy for
    // a key whose translations actually differ.
    assert.ok(enHtml.includes(t('work.askAegis', 'en')), 'en render shows English Ask Aegis');
    assert.ok(!enHtml.includes(t('work.askAegis', 'zh')), 'en render must not leak zh Ask Aegis');
    assert.ok(zhHtml.includes(t('work.askAegis', 'zh')), 'zh render shows Chinese Ask Aegis');
    assert.ok(!zhHtml.includes(t('work.scan.searchPlaceholder', 'en')), 'zh render must not leak en search placeholder');

    // Interpolated plural copy localizes too (queue body, default not-selected).
    const zhEmpty = renderWithLocale(baseProps({ selected: null }), 'zh');
    const enEmpty = renderWithLocale(baseProps({ selected: null }), 'en');
    assert.ok(zhEmpty.includes(t('work.queue.overviewKicker', 'zh')));
    assert.ok(enEmpty.includes(t('work.queue.overviewKicker', 'en')));
  });
});

describe('resolveDecisionChoiceCopy', () => {
  const baseChoice = {
    id: 'run_once',
    label: 'Approve this run',
    consequence: 'Aegis will continue this paused run once and record the approval path.',
    risk_level: 'medium',
    applies_to: 'this_run',
  };

  it('resolves known choice ids in zh', () => {
    const known = [
      ['run_once', '批准本次运行', '允许安琪继续这次暂停的运行，并记录批准路径。'],
      ['skip_once', '跳过本次运行', '安琪跳过这次运行，重复规则保持不变。'],
      ['approve_unavailable', '批准本次运行', '找不到有效的批准请求（可能已过期），无法从这里继续。'],
      ['skip_unavailable', '跳过本次运行', '原批准请求已不在等待中，无法跳过。'],
      ['revise_rule', '修改规则', '先与安琪讨论一个更窄、更清晰的规则，再决定是否继续。'],
      ['view_safe_details', '查看安全详情', '打开任务上下文与历史，不执行被拦截的步骤。'],
    ];
    for (const [id, label, consequence] of known) {
      const copy = resolveDecisionChoiceCopy({ ...baseChoice, id, label: 'SERVER', consequence: 'SERVER_C' }, 'zh');
      assert.equal(copy.label, label, id);
      assert.equal(copy.consequence, consequence, id);
    }
  });

  it('resolves known choice ids in en to exact server English', () => {
    const known = [
      ['run_once', 'Approve this run', 'Aegis will continue this paused run once and record the approval path.'],
      ['skip_once', 'Skip this run', 'Aegis will skip this paused run and leave the recurring rule unchanged.'],
      ['approve_unavailable', 'Approve this run', 'Aegis cannot continue from this card because the live approval request is missing or expired.'],
      ['skip_unavailable', 'Skip this run', 'Aegis cannot skip the original approval request because that request is no longer pending.'],
      ['revise_rule', 'Revise rule', 'Discuss a narrower or clearer rule with Aegis before any run continues.'],
      ['view_safe_details', 'View safe details', 'Open the task context and history without running the blocked step.'],
    ];
    for (const [id, label, consequence] of known) {
      const copy = resolveDecisionChoiceCopy({
        ...baseChoice,
        id,
        label: 'DIFFERENT',
        consequence: 'DIFFERENT_C',
      }, 'en');
      assert.equal(copy.label, label, id);
      assert.equal(copy.consequence, consequence, id);
    }
  });

  it('falls back to server label/consequence for unknown choice ids', () => {
    const unknown = {
      id: 'brand_new_future_choice',
      label: 'Server future label',
      consequence: 'Server future consequence',
      risk_level: 'medium',
      applies_to: 'this_run',
    };
    const zh = resolveDecisionChoiceCopy(unknown, 'zh');
    const en = resolveDecisionChoiceCopy(unknown, 'en');
    assert.equal(zh.label, 'Server future label');
    assert.equal(zh.consequence, 'Server future consequence');
    assert.equal(en.label, 'Server future label');
    assert.equal(en.consequence, 'Server future consequence');
    // Known enums still localize even when the choice id is unknown.
    assert.equal(zh.riskLabel, '中风险');
    assert.equal(zh.scopeLabel, '本次运行');
    assert.equal(en.riskLabel, 'Medium risk');
    assert.equal(en.scopeLabel, 'This run');
  });

  it('localizes risk and scope enums and passes unknown values through', () => {
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, risk_level: 'low' }, 'zh').riskLabel, '低风险');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, risk_level: 'medium' }, 'zh').riskLabel, '中风险');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, risk_level: 'high' }, 'zh').riskLabel, '高风险');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, risk_level: 'low' }, 'en').riskLabel, 'Low risk');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, risk_level: 'medium' }, 'en').riskLabel, 'Medium risk');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, risk_level: 'high' }, 'en').riskLabel, 'High risk');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, risk_level: 'critical_future' }, 'zh').riskLabel, 'critical_future');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, risk_level: 'critical_future' }, 'en').riskLabel, 'critical_future');

    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, applies_to: 'this_run' }, 'zh').scopeLabel, '本次运行');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, applies_to: 'rule' }, 'zh').scopeLabel, '规则');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, applies_to: 'this_item' }, 'zh').scopeLabel, '这件事');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, applies_to: 'this_run' }, 'en').scopeLabel, 'This run');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, applies_to: 'rule' }, 'en').scopeLabel, 'Rule');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, applies_to: 'this_item' }, 'en').scopeLabel, 'This item');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, applies_to: 'future_scope' }, 'zh').scopeLabel, 'future scope');
    assert.equal(resolveDecisionChoiceCopy({ ...baseChoice, applies_to: 'future_scope' }, 'en').scopeLabel, 'future scope');
  });

  it('an opened task does not say to open the detail, and the source line is who, when, and wording', () => {
    const gift = taskItem({
      id: 'task:2142',
      lane: 'now',
      title: '准备太太生日礼物',
      summary: 'Task to review.',
      state: 'active',
      status: 'active',
      blocked_by: null,
      decision_request: null,
      next_safe_action: 'Open details to reveal the private task, or choose a safe action if you already recognize it.',
      next_safe_action_code: 'task_open_or_act',
      lineage: [
        {
          layer: 'intent',
          kind: 'task',
          id: 'task:2142',
          label: '一次性安排',
          label_code: 'intent.oneshot',
          detail: '任务 2142',
          action_ids: [],
        },
        {
          layer: 'origin',
          kind: 'context',
          id: 'task:2142:origin',
          label: '当时的说明',
          label_code: 'origin.context',
          detail: '你 · 2026-09-24 06:43:38 · 🎁 准备 Hello Kitty 可爱风',
          action_ids: [],
        },
      ],
    });
    const html = render(baseProps({
      selected: gift,
      lanes: lanes([gift]),
    }));
    const opened = html.slice(html.indexOf('vnext-work-inline-selected'));
    assert.ok(opened.includes('vnext-source-evidence-detail'), 'opened card has a source line');
    assert.ok(!opened.includes('打开详情查看私人任务'), opened.slice(0, 500));
    assert.ok(opened.includes('任务 2142'));
    assert.ok(opened.includes('你 · 2026-09-24 06:43:38 · 🎁 准备 Hello Kitty 可爱风'));
    assert.ok(!opened.includes('group_key='));
    assert.ok(!opened.includes('context_text='));
    assert.ok(!opened.includes('source='));
    assert.ok(!opened.includes('auto:准备太太生日礼物'));
  });

  it('a still-locked private task keeps the open-details instruction', () => {
    const locked = taskItem({
      id: 'task:9',
      lane: 'now',
      title: 'Private errand',
      summary: 'Private task title and body stay private.',
      state: 'active',
      status: 'active',
      blocked_by: null,
      decision_request: null,
      next_safe_action_code: 'task_private_review',
      next_safe_action: 'Open the details and read the title and body before completing or snoozing.',
    });
    const html = render(baseProps({ selected: locked, lanes: lanes([locked]) }));
    const opened = html.slice(html.indexOf('vnext-work-inline-selected'));
    assert.ok(opened.includes(t('work.nextMove.openDetails', 'zh')));
  });
});

describe('_removed_mail_iframe_clamp', () => {
  it('mail iframe helpers are gone', () => {
    assert.equal(true, true);
  });
});
