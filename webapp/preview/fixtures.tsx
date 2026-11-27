/*
 * Representative fixtures for the screenshot harness, lifted from the vnext
 * render tests so previews exercise the real components with realistic data.
 * Not shipped (preview/ is excluded from the app build).
 */
import { t } from '../src/i18n/catalog';
import type { Locale } from '../src/i18n/catalog';

const noop = () => {};

// Base work item: a tracked bill (knowledge / waiting_for) that surfaced from
// a Gmail connector run through the external executor. Every other fixture
// item is derived from it via overrides.
export function trackedBillItem(overrides: any = {}): any {
  return {
    id: 'knowledge:42', lane: 'now', kind: 'waiting_for', source_kind: 'tracking', source: 'knowledge',
    source_id: '42', source_event_id: null,
    title: 'Billing statement available',
    summary: 'Maple Lane Music School; open the item for the full detail',
    why_it_matters: 'A tracked bill surfaced from safe metadata.',
    owner_visible_reason: 'Sender and subject matched a bill you track.',
    state: 'unread', status: 'unread', urgency: 'normal',
    sensitivity: 'S2', privacy_level: 'S2',
    created_at: '2026-05-05T00:00:00Z', updated_at: '2026-05-05T00:00:00Z', due_at: null,
    authority_basis: 'manual_review',
    authority: {
      basis: 'manual_review', authority_basis: 'manual_review',
      owner_visible_reason: 'Tracked bill matched.',
      visible_data: ['title', 'summary', 'sender', 'status'],
      visible_fields: ['title', 'summary', 'sender', 'status'],
      locked_data: ['body', 'html', 'attachments'],
      locked_fields: ['body', 'html', 'attachments'],
      allowed_now: ['brief', 'draft'],
      requires_mfa: false, requires_approval: false, requires_data_grant: false,
      expected_receipt: 'Tracked-item changes keep an action ledger receipt.',
      source_mandate_id: null, audit_refs: [], audit_links: [],
    },
    primary_action: {
      id: 'open', label: 'Open detail', kind: 'navigate',
      route: '/webapp/console/knowledge/42',
      requires_confirmation: false, requires_mfa: false, requires_data_gate: false,
      expected_receipt: 'Reading alone does not execute an outside action.',
    },
    contextual_actions: [
      { id: 'done', label: 'Mark done', confirm_label: 'Tap again to mark done', receipt_hint: 'Aegis will record a completion receipt.', tone: 'normal' },
      { id: 'follow_up', label: 'Follow up later', confirm_label: 'Tap again to follow up later', receipt_hint: 'Aegis will record a follow-up receipt.', tone: 'normal' },
    ],
    secondary_actions: [
      { label: 'Mark done', action_kind: 'done', route: '/webapp/console/knowledge/42', requires_gate: false, requires_mfa: false },
      { label: 'Follow up later', action_kind: 'follow_up', route: '/webapp/console/knowledge/42', requires_gate: false, requires_mfa: false },
    ],
    next_safe_action: 'Open the item, then brief, draft, or mark it done.',
    blocked_by: '', detail_route: '/webapp/console/knowledge/42',
    external_url_policy: 'available_after_open',
    route_reason: 'A tracked bill surfaced from safe metadata.',
    related_work_item_ids: [], related_action_ids: [], related_receipt_ids: [],
    receipt_expectation: 'Tracked-item changes keep an action ledger receipt.',
    ...overrides,
  };
}

function decisionChoice(overrides: any = {}): any {
  return {
    id: 'revise_rule', label: 'Revise rule',
    consequence: 'Discuss a narrower or clearer rule with Aegis before any run continues.',
    risk_level: 'low', applies_to: 'rule',
    assistant_prompt: 'Help me revise this paused automation rule.',
    requires_confirmation: false, enabled: true, tone: 'normal', ...overrides,
  };
}

function decisionRequest(): any {
  return {
    id: 'decision:task:1054', title: 'Calendar sweep needs your choice',
    what_aegis_was_doing: 'Review this week\'s events for the calendar sweep automation.',
    blocked_reason: 'Reading event details needs owner permission.',
    requested_capability: 'Read calendar event details for this paused automation.',
    scope: 'This paused run. Rule changes only happen if you explicitly choose a rule change.',
    choices: [
      // Production ids from console_handlers vnext_blocked_decision_request.
      decisionChoice({
        id: 'run_once',
        label: 'Approve this run',
        consequence: 'Aegis will continue this paused run once and record the approval path.',
        risk_level: 'medium',
        applies_to: 'this_run',
      }),
      decisionChoice({
        id: 'skip_once',
        label: 'Skip this run',
        consequence: 'Aegis will skip this paused run and leave the recurring rule unchanged.',
        applies_to: 'this_run',
      }),
      decisionChoice(),
      decisionChoice({
        id: 'view_safe_details',
        label: 'View safe details',
        consequence: 'Open the task context and history without running the blocked step.',
        applies_to: 'this_item',
      }),
    ],
    safe_details: [
      'No external action has run.',
      'Visible metadata can be reviewed without opening private content.',
    ],
    audit_refs: [],
    receipt_expectation: 'Task mutations are recorded with an action ledger receipt or a failure state.',
  };
}

export function taskItem(overrides: any = {}): any {
  return trackedBillItem({
    id: 'task:1054', lane: 'decide', kind: 'task', source_kind: 'task', source: 'tasks', source_id: '1054',
    title: 'Weekly calendar sweep',
    summary: 'A scheduled automation paused before using a sensitive step.',
    why_it_matters: 'Aegis paused before a sensitive step.',
    owner_visible_reason: 'Aegis paused before a sensitive step.',
    state: 'blocked_by_gate', status: 'blocked_by_gate', urgency: 'high',
    primary_action: { id: 'open', label: 'Open detail', kind: 'navigate', route: '/webapp/console/tasks/1054/timeline', requires_confirmation: false, requires_mfa: false, requires_data_gate: false, expected_receipt: 'Reading alone does not execute an outside action.' },
    contextual_actions: [], secondary_actions: [],
    next_safe_action: 'Review before continuing.',
    blocked_by: 'scheduled automation wants calendar detail before continuing',
    detail_route: '/webapp/console/tasks/1054/timeline',
    receipt_expectation: 'Task mutations are recorded with an action ledger receipt or a failure state.',
    decision_request: decisionRequest(),
    ...overrides,
  });
}

// Lane chrome (title/subtitle) localizes via the same catalog keys the real app
// uses (buildVNextLanes); the harness passes the scene locale so screenshots are
// faithful to the zh app (the owner's reported "Now/Decide" leak was here).
export function workLanes(locale: Locale = 'en'): any[] {
  const items = [taskItem(), trackedBillItem()];
  return [
    { id: 'decide', title: t('work.lane.decide.title', locale), subtitle: t('work.lane.decide.subtitle', locale), items: items.filter((i) => i.lane === 'decide'), collapsed: false },
    { id: 'now', title: t('work.lane.now.title', locale), subtitle: t('work.lane.now.subtitle', locale), items: items.filter((i) => i.lane === 'now'), collapsed: false },
    { id: 'follow_up', title: t('work.lane.followUp.title', locale), subtitle: t('work.lane.followUp.subtitle', locale), items: [], collapsed: true },
    { id: 'backlog', title: t('work.lane.backlog.title', locale), subtitle: t('work.lane.backlog.subtitle', locale), items: [], collapsed: true },
    { id: 'done', title: t('work.lane.done.title', locale), subtitle: t('work.lane.done.subtitle', locale), items: [], collapsed: true },
  ];
}

export function workProps(locale: Locale = 'en'): any {
  const lanes = workLanes(locale);
  // Select a non-locked item so the AuthorityPanel (trust display) renders
  // (the default tracked-bill item has locked body content -> "open to review" branch).
  const selected = trackedBillItem({
    id: 'cal:open', lane: 'now', kind: 'calendar', source_kind: 'calendar', source: 'calendar', source_id: 'evt-2pm',
    title: 'Confirm the 2pm with Dr. Lin', summary: 'Reschedule request came in; your call.',
    state: 'unread', status: 'unread',
    primary_action: { id: 'open', label: 'Open event', kind: 'navigate', route: '/webapp/console/calendar/evt-2pm', requires_confirmation: false, requires_mfa: false, requires_data_gate: false, expected_receipt: 'Reading alone does not execute an outside action.' },
    contextual_actions: [], secondary_actions: [],
    next_safe_action: 'Review the event, then brief or reschedule.',
    detail_route: '/webapp/console/calendar/evt-2pm',
    route_reason: 'A reschedule request surfaced this event for your call.',
    receipt_expectation: 'Calendar changes keep a Google Calendar receipt.',
    authority: {
      ...trackedBillItem().authority,
      visible_data: ['title', 'summary', 'time', 'organizer'],
      visible_fields: ['title', 'summary', 'time', 'organizer'],
      locked_data: [], locked_fields: [],
      allowed_now: ['brief', 'reschedule'],
      expected_receipt: 'Calendar changes keep a Google Calendar receipt.',
    },
  });
  lanes[1].items = [selected, lanes[1].items[0]];
  return {
    lanes, expandedLaneIds: [], selected, context: null, itemProjection: null,
    sourceDetail: null, sourceDetailBusy: false, assistantResult: null,
    assistantResultItemId: null, actionMessage: '', workNotice: '', actionsRetired: false,
    armedAction: null, armedActionItemId: null, executingAction: null, busy: false,
    onSelect: noop, onToggleLane: noop, onOpenSource: noop, onAction: noop,
    onBlockedResolution: noop, onBrief: noop, onDraft: noop, onAskText: noop,
  };
}

// Work view with an OPENED source detail so the activity narrative
// (.vnext-timeline) + attachment metadata render (info-ux #4).
export function workDetailProps(locale: Locale = 'en'): any {
  const base = workProps(locale);
  const sourceDetail = {
    kind: 'receipt',
    title: 'Billing statement — Maple Lane Music School',
    // Production subtitles come from detail.ts, which localizes (catalog keys /
    // server summary) — mirror that so the zh harness view is not flagged as a
    // mixed-language leak by the UI audit (2026-07-10).
    subtitle: locale === 'zh'
      ? '已打开，方便你从头到尾核查发生了什么。'
      : 'Opened so you can review what happened, end to end.',
    fields: [
      { label: 'From', value: 'billing@maplelane-music.example' },
      { label: 'Received', value: 'May 31, 8:02 AM' },
    ],
    attachments: [
      // Previewable: signed same-origin url + inline preview toggle.
      // Synthetic same-origin-safe data URLs: the harness only needs a
      // non-null url to render the download / preview affordances.
      { filename: 'statement-may.pdf', meta: 'application/pdf · 142 KB', downloadUrl: 'data:application/pdf;base64,JVBERi0xLjQK', previewable: true, mimeType: 'application/pdf' },
      // Download-only (active content forced to download by the server).
      { filename: 'report.docx', meta: 'application/vnd…document · 38 KB', downloadUrl: 'data:application/octet-stream;base64,UEsDBA==', previewable: false, mimeType: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document' },
      // Metadata-only (no signed url): name shown, no actions.
      { filename: 'logo.png', meta: 'image/png · 2 KB', downloadUrl: null, previewable: false, mimeType: 'image/png' },
    ],
    timeline: [
      { label: 'Surfaced to you', status: 'needs your review', at: 'just now' },
      { label: 'Drafted a reply', status: 'saved for your review', at: '8:04 AM' },
      { label: 'Read the statement', status: 'metadata only', at: '8:03 AM' },
      { label: 'Received', status: 'via the Gmail connector', at: '8:02 AM' },
    ],
  };
  return { ...base, sourceDetail };
}

// ---- Status + Settings fixtures (mirrors vnextStatusSettingsRender test) ----
export function statusQueue(): any {
  return {
    generated_at: '2026-05-17T00:00:00Z',
    counts: { total: 2, calendar: 1, tasks: 1, receipts: 0 },
    sources: [
      { source: 'drive', status: 'available', reason: 'ready' },
      { source: 'calendar', status: 'needs_attention', reason: 'refresh with Google' },
    ],
    items: [taskItem(), trackedBillItem()],
  };
}

export function statusProjection(): any {
  return {
    schema_version: 1, generated_at: '2026-05-17T00:00:00Z', health: 'attention',
    sections: [
      { label: 'Cost controls', health: 'quiet', owner_message: 'Heartbeat and initiative are paused to save cost.', owner_next_step: 'Resume heartbeat and initiative when ready.' },
      { label: 'Codex', health: 'ready', owner_message: 'Codex is connected and ready.', owner_next_step: null },
      { label: 'Calendar', health: 'attention', owner_message: 'Calendar needs a refresh with Google.', owner_next_step: 'Reconnect Calendar.' },
    ],
    system_events: [
      { id: 'system:auth:critical', kind: 'auth_failure', severity: 'critical', title: 'Calendar auth expired', summary: 'Calendar stopped syncing — reconnect with Google.', occurred_at: 1779491800, window_start_at: 1779491800, count: 2, details: ['Token refresh failed twice'] },
      { id: 'system:probe:warning', kind: 'probe', severity: 'warning', title: 'Executor probe retried', summary: 'An executor probe retried before succeeding.', occurred_at: 1779491600, window_start_at: 1779491600, count: 1, details: [] },
      { id: 'system:probe:notice', kind: 'probe', severity: 'notice', title: 'Skill refreshed', summary: 'A skill refreshed its cache.', occurred_at: 1779491550, window_start_at: 1779491550, count: 1, details: [] },
      { id: 'system:restart:1779491535', kind: 'restart', severity: 'quiet', title: 'System restarted', summary: 'Aegis restarted and returned to service.', occurred_at: 1779491535, window_start_at: 1779486963, count: 9, details: ['Active skills: 22'] },
    ],
    advanced_diagnostics_available: true,
  };
}

export function statusProps(): any {
  return { queue: statusQueue(), projection: statusProjection(), onOpenWork: noop, onOpenSettings: noop };
}

// Locale-aware where the PRODUCTION path localizes: SettingsView's own fallback
// sections use catalog keys (settings.security.itemAccess.message,
// settings.notifications.telegram.message, settings.push.ownerMessage), so the
// zh harness views must exercise those localized strings, not fixture English —
// otherwise the audit's mixed-lang sensor flags harness-only artifacts.
// Server-sent owner_message strings that production surfaces verbatim in
// English (admin.rs settings projection) are the known-DEFERRED server-payload
// i18n class and are intentionally NOT simulated as localized here.
export function settingsProjection(locale: Locale = 'en'): any {
  return {
    generated_at: '2026-05-17T00:00:00Z', signed_in_owner: 'owner@example.com', cost_saving_paused: true,
    llm_provider: 'codex_oauth', llm_model: 'google/gemini-3.1-flash-lite', paid_api_fallback_enabled: false,
    connected_services: [
      { label: 'Codex', health: 'ready', owner_message: 'Codex is ready.', owner_next_step: null },
      { label: 'Calendar', health: 'attention', owner_message: 'Refresh with Google.', owner_next_step: 'Reconnect Calendar.' },
    ],
    security: [
      { label: 'Signed-in session', health: 'ready', owner_message: 'Google verifies your configured account.', owner_next_step: null },
      { label: 'Item access', health: 'ready', owner_message: t('settings.security.itemAccess.message', locale), owner_next_step: null },
    ],
    notification_preferences: [
      { label: 'Telegram', health: 'ready', owner_message: t('settings.notifications.telegram.message', locale), owner_next_step: null },
      { label: 'Cost controls', health: 'quiet', owner_message: 'Heartbeat and initiative internals are paused.', owner_next_step: 'Resume background work when ready.' },
    ],
  };
}

export function settingsProps(locale: Locale = 'en'): any {
  return {
    email: 'owner@example.com', sources: statusQueue().sources, projection: settingsProjection(locale),
    costSavingBusy: false, snapshotMessage: '', snapshotAvailable: true, snapshotBusy: false,
    webPushStatus: { schema_version: 1, configured: true, send_configured: true, subscribed: false, subscription_count: 0, public_key: 'B'.repeat(87), last_delivery: null, owner_message: t('settings.push.ownerMessage', locale), owner_next_step: null },
    webPushDeviceSubscribed: false, webPushBusy: false, webPushMessage: '', debugLeases: [],
    avatarProfile: { aegisAvatarUrl: './aegis-avatar.svg', ownerAvatarUrl: '' },
    onPauseCostSaving: noop, onResumeCostSaving: noop, onEnableWebPush: noop, onDisableWebPush: noop,
    onTestWebPush: noop, onCreateSnapshot: noop, onLogout: noop, onCreateDebugLease: noop,
    onRevokeDebugLease: noop, onRunDebugJob: noop, onSetAvatar: noop, onResetAvatar: noop,
    onSetLlmProvider: noop, onSetLlmModel: noop, onSetPaidApiFallback: noop,
  };
}

export function workDecideProps(locale: Locale = 'en'): any {
  const lanes = workLanes(locale);
  const selected = lanes[0].items[0]; // the blocked task (decide lane) w/ decision_request
  return {
    lanes, expandedLaneIds: [], selected, context: null, itemProjection: null,
    sourceDetail: null, sourceDetailBusy: false, assistantResult: null,
    assistantResultItemId: null, actionMessage: '', workNotice: '', actionsRetired: false,
    armedAction: null, armedActionItemId: null, executingAction: null, busy: false,
    onSelect: noop, onToggleLane: noop, onOpenSource: noop, onAction: noop,
    onBlockedResolution: noop, onBrief: noop, onDraft: noop, onAskText: noop,
  };
}
