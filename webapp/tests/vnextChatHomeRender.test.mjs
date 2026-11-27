import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

// The static-render fixtures assert UTC clock labels when no owner timezone
// provider is mounted. Keep those expectations independent of the CI host.
process.env.TZ = 'UTC';

const modulePath = process.env.VNEXT_CHAT_HOME_RENDER_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-chat-home-render-test/ChatHomeView.mjs', import.meta.url).href;
// The bundle (tests/fixtures/chatHomeRenderEntry.ts) re-exports both the view and
// the provider so they share a single LocaleContext instance.
import { readFileSync } from 'node:fs';
function chatHomeViewSource() { return readFileSync(new URL('../src/vnext/ChatHomeView.tsx', import.meta.url), 'utf8'); }
const { ChatHomeView, LocaleProvider, tf, t, appendApprovalMessage, replaceApprovalMessagesFromServer, setApprovalState, setApprovalProgress, composerSizeActionLabel, chatDateDividerLabels, collapsedChatHomeMessages, messageBodyText, messageCiteToken, messagePublicRef, isGatePendingOwnerAck, omitGatePendingOwnerAcks, ownerVisibleSearchOutcome, searchHoldIdentity } = await import(modulePath);

const noop = () => {};

function baseProps(overrides = {}) {
  return {
    email: 'owner@example.com',
    messages: [],
    brief: 'Needs attention: Review calendar access.',
    value: '',
    busy: false,
    snapshotMessage: '',
    snapshotAvailable: true,
    snapshotBusy: false,
    contextSignals: [
      {
        id: 'decision',
        label: 'Needs decision',
        summary: 'Review calendar access',
        tone: 'warn',
        targetItemId: 'approval:calendar',
      },
      {
        id: 'private',
        label: 'Needs detail',
        summary: 'Open Work to review hidden details',
        tone: 'warn',
        targetItemId: 'task:42',
      },
    ],
    workBadgeCount: 2,
    workBadgeLabel: 'Work, 2 decisions waiting',
    workBadgeTone: 'warn',
    costSavingPaused: false,
    avatarProfile: {
      aegisAvatarUrl: '/aegis-avatar.svg',
      ownerAvatarUrl: '',
    },
    onChange: noop,
    onSubmit: noop,
    onSuggestion: noop,
    onClearConversation: noop,
    onOpenWork: noop,
    onOpenStatus: noop,
    onOpenSettings: noop,
    onStartVoiceNote: noop,
    onStopVoiceNote: noop,
    onCancelVoiceNote: noop,
    onRetry: noop,
    onAskWorkItem: noop,
    onWorkItemAction: async () => ({ ok: true, message: 'Receipt saved.', nextStatus: 'archived' }),
    onOpenWorkDetail: async () => ({ ok: false, message: 'Detail unavailable in static render.' }),
    onNoticeAction: async () => ({ ok: true, message: 'Reminder updated.' }),
    onAttachContext: noop,
    onClearAttachedContext: noop,
    onCreateSnapshot: noop,
    onPauseSelfStartedWork: noop,
    onResumeBackgroundWork: noop,
    installActionLabel: 'Install app',
    onInstallOwnerApp: noop,
    onLogout: noop,
    ...overrides,
  };
}

function assistantResponse(overrides = {}) {
  return {
    schema_version: 1,
    response_kind: 'answer',
    visible_answer: 'Answer: I found a mail item that may need attention.',
    draft: null,
    action_draft: null,
    suggested_intent: null,
    authority: null,
    required_gate: null,
    receipt_expectation: 'No external action was executed.',
    citations: [],
    warnings: [],
    ...overrides,
  };
}

function mailWorkItem(overrides = {}) {
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
        route: null,
        requires_gate: false,
        requires_mfa: false,
      },
      {
        label: 'Archive',
        action_kind: 'mail.archive',
        route: null,
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

function blockedAutomationItem(overrides = {}) {
  return {
    ...mailWorkItem({
      id: 'task:1054',
      lane: 'decide',
      kind: 'task',
      source_kind: 'task',
      source: 'tasks',
      source_id: '1054',
      title: 'Monitor Gmail',
      summary: 'Paused before action. Aegis needs your choice before this automation continues.',
      why_it_matters: 'Aegis paused before a sensitive step.',
      owner_visible_reason: 'An automation you asked Aegis to run paused before a sensitive step.',
      state: 'blocked_by_gate',
      status: 'blocked_by_gate',
      urgency: 'high',
      authority_basis: 'manual_review',
      authority: null,
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
      next_safe_action: 'Review this pause, ask Aegis to revise the automation, or close it.',
      blocked_by: 'Aegis paused this automation before a sensitive step.',
      detail_route: '/webapp/console/tasks/1054/timeline',
      external_url_policy: 'none',
      route_reason: 'Aegis paused before continuing an owner automation.',
      receipt_expectation: 'No external action completed yet.',
    }),
    ...overrides,
  };
}

// The component now reads its copy from the i18n catalog via LocaleContext, so
// these render helpers must wrap it in a LocaleProvider. `render()` defaults to
// English so the existing English-copy assertions stay meaningful; the dedicated
// "localizes ... for both locales" suite below proves the zh path renders too.
function renderWithLocale(displayLanguage, props) {
  return renderToStaticMarkup(
    React.createElement(LocaleProvider, { displayLanguage }, React.createElement(ChatHomeView, props)),
  );
}

function render(props) {
  return renderWithLocale('en', props);
}

describe('vNext Chat Home render', () => {
  it('renders the empty AI-first home without old dashboard language', () => {
    const html = render(baseProps());

    assert.match(html, /Tell Angie\./);
    assert.match(html, /Start here/);
    assert.match(html, /Review calendar access/);
    assert.doesNotMatch(html, /Attention: Review calendar access/);
    assert.match(html, /aria-label="Open current Aegis brief"/);
    assert.match(html, /Commute/);
    assert.match(html, /Tasks/);
    assert.doesNotMatch(html, /Mail/);
    assert.match(html, /Focus/);
    assert.match(html, /Review calendar access/);
    assert.match(html, /Open Work to review hidden details/);
    assert.equal((html.match(/Review calendar access/g) || []).length, 1);
    assert.doesNotMatch(html, /Today briefing/);
    assert.doesNotMatch(html, /Decision desk/);
    assert.doesNotMatch(html, /Private access ready/);
    assert.doesNotMatch(html, /Same Aegis as Telegram/);
  });

  // CEO home: peers live in bottom nav / brand rail only — not chat topbar.
  // Empty hero no longer dual-lists Work/Mail destination cards (needs-you is
  // the truth source). Snapshot/clear stay menu-only.
  it('home has no topbar Work/Mail peer strip and no hero destination hub', () => {
    const html = render(baseProps());
    assert.doesNotMatch(html, /vnext-chat-surface-nav/);
    assert.doesNotMatch(html, /vnext-chat-surface-link/);
    assert.doesNotMatch(html, /vnext-chat-destinations/);
    assert.doesNotMatch(html, /vnext-chat-destination/);
    assert.match(html, /vnext-chat-needs-you/);
    assert.match(html, /vnext-chat-hero-hint/);
    assert.doesNotMatch(html, /vnext-chat-work-button/);
    // No peer snapshot/clear chrome — overflow menu only.
    assert.doesNotMatch(html, /vnext-chat-snapshot/);
    assert.doesNotMatch(html, /vnext-chat-clear/);
    assert.match(html, /aria-label="Account menu"/);
  });

  it('empty home renders actionable needs-you work cards when items are provided', () => {
    const decisionItem = {
      id: 'approval:calendar',
      kind: 'approval',
      title: 'Calendar access decision',
      summary: 'Review calendar access before Calendar can run',
      status: 'blocked_by_gate',
      urgency: 'high',
      sensitivity: 'S2',
      privacy_level: 'metadata',
      source: 'calendar',
      source_id: 'cal-1',
      created_at: '2026-05-05T00:00:00Z',
      updated_at: '2026-05-05T00:00:00Z',
      secondary_actions: [],
      external_url_policy: 'none',
      related_action_ids: [],
      related_receipt_ids: [],
      route_reason: 'Needs owner decision.',
      schema_version: 1,
      decision_request: {
        id: 'decision:1',
        title: 'Calendar access',
        what_aegis_was_doing: 'Preparing calendar automation.',
        blocked_reason: 'Needs permission.',
        requested_capability: 'Read calendar content once.',
        scope: 'This run only.',
        choices: [],
        safe_details: [],
      },
    };
    const html = render(baseProps({ needsYouItems: [decisionItem] }));
    assert.match(html, /vnext-chat-needs-you-banner|aegis-needs-you-banner/);
    assert.match(html, /Calendar access decision|需要你/);
    assert.doesNotMatch(html, /vnext-chat-needs-you-cards/);
  });

  it('conversation keeps needs-you desk in rail and mobile inline strip', () => {
    const decisionItem = {
      id: 'approval:calendar',
      kind: 'approval',
      title: 'Calendar access decision',
      summary: 'Needs a decision',
      status: 'blocked_by_gate',
      urgency: 'high',
      sensitivity: 'S2',
      privacy_level: 'metadata',
      source: 'calendar',
      source_id: 'cal-1',
      created_at: '2026-05-05T00:00:00Z',
      updated_at: '2026-05-05T00:00:00Z',
      secondary_actions: [],
      external_url_policy: 'none',
      related_action_ids: [],
      related_receipt_ids: [],
      route_reason: 'Needs owner decision.',
      schema_version: 1,
    };
    const html = render(baseProps({
      needsYouItems: [decisionItem],
      messages: [
        { id: 'owner:1', role: 'owner', text: 'hello', timestamp: Date.UTC(2026, 5, 1, 10, 0) },
      ],
    }));
    assert.match(html, /has-needs-you-desk/);
    assert.match(html, /vnext-chat-needs-you-banner|aegis-needs-you-banner/);
    assert.match(html, /Calendar access decision|需要你/);
    assert.doesNotMatch(html, /vnext-chat-needs-you-inline/);
    // Banner must sit outside the stream (sibling after </main>, above composer).
    // Slice between <main …> and its matching </main> so a post-</main> banner
    // does not falsely match a "inside stream" regex that spans past the close.
    const mainOpen = html.indexOf('<main');
    assert.ok(mainOpen >= 0, 'chat stream <main> missing');
    const mainClose = html.indexOf('</main>', mainOpen);
    assert.ok(mainClose > mainOpen, 'chat stream </main> missing');
    const streamSlice = html.slice(mainOpen, mainClose);
    const afterStream = html.slice(mainClose);
    assert.doesNotMatch(
      streamSlice,
      /data-testid="aegis-needs-you-banner"|vnext-chat-needs-you-banner/,
      'needs-you banner must not render inside the chat stream',
    );
    assert.match(
      afterStream,
      /data-testid="aegis-needs-you-banner"|vnext-chat-needs-you-banner/,
      'needs-you banner must render after the chat stream',
    );
  });

  // Task 5: collapse the >_ icon — the composer's extra action buttons must be
  // gone from the rendered output. The reply button (↩) on both owner messages
  // and aegis answers must survive.
  it('composer extra action buttons are gone; reply buttons survive (Task 5)', () => {
    const html = render(baseProps({
      messages: [
        { id: 'owner:1', role: 'owner', text: 'hello', timestamp: Date.UTC(2026, 5, 1, 10, 0) },
        {
          id: 'aegis:1',
          role: 'aegis',
          text: 'World.',
          result: assistantResponse({ visible_answer: 'World.' }),
          timestamp: Date.UTC(2026, 5, 1, 10, 1),
        },
        // aegis without result (compact heading path)
        {
          id: 'aegis:2',
          role: 'aegis',
          text: 'Plain notice.',
          timestamp: Date.UTC(2026, 5, 1, 10, 2),
        },
      ],
    }));
    // The collapsed composer/per-message action buttons must be absent.
    assert.doesNotMatch(html, /vnext-chat-dev-button/);
    // Reply (quote) button must still be present on owner + aegis messages.
    assert.match(html, /vnext-chat-quote-button/);
  });

  it('gives owner and Aegis messages a stable cite that survives a history id change', () => {
    const turnId = 'idem-cite-turn';
    const owner = { role: 'owner', turnId, text: '这条要迁吗', timestamp: Date.UTC(2026, 8, 22, 5, 54) };
    const reply = {
      role: 'aegis',
      turnId,
      text: '结论：先别迁。',
      timestamp: Date.UTC(2026, 8, 22, 5, 57),
      result: assistantResponse({ visible_answer: '结论：先别迁。', turn_id: turnId }),
    };
    const liveOwnerRef = messagePublicRef({ ...owner, fallbackId: 'local-owner' });
    const historyOwnerRef = messagePublicRef({ ...owner, fallbackId: `server:${turnId}:owner` });
    const replyRef = messagePublicRef({
      role: 'aegis',
      turnId,
      text: reply.text,
      timestamp: reply.timestamp,
      fallbackId: `server:${turnId}:aegis`,
    });
    assert.equal(liveOwnerRef, historyOwnerRef);
    assert.notEqual(liveOwnerRef, replyRef);
    assert.match(liveOwnerRef, /^[0-9abcdefghjkmnpqrstvwxyz]{6}$/);
    assert.equal(messageCiteToken(liveOwnerRef), `#${liveOwnerRef}`);
    const legacyA = messagePublicRef({ role: 'owner', timestamp: 10, text: '同样的话', fallbackId: 'a' });
    const legacyB = messagePublicRef({ role: 'owner', timestamp: 10, text: '同样的话', fallbackId: 'b' });
    const legacyOther = messagePublicRef({ role: 'aegis', timestamp: 10, text: '同样的话', fallbackId: 'a' });
    assert.equal(legacyA, legacyB);
    assert.notEqual(legacyA, legacyOther);

    const html = render(baseProps({
      messages: [
        { id: 'local-owner', ...owner },
        { id: 'local-aegis', ...reply },
        { id: 'working:1', role: 'working', text: '正在查', timestamp: Date.UTC(2026, 8, 22, 5, 55) },
      ],
    }));
    assert.equal((html.match(new RegExp(`data-message-ref="${liveOwnerRef}"`, 'g')) || []).length, 1);
    assert.equal((html.match(new RegExp(`data-message-ref="${replyRef}"`, 'g')) || []).length, 1);
    assert.match(html, new RegExp(`aria-label="Copy reference #${liveOwnerRef}"`));
    assert.equal((html.match(/data-message-ref="/g) || []).length, 2);
  });

  it('keeps quiet home brief concise without making an inert dashboard card', () => {
    const html = render(baseProps({
      brief: 'Nothing urgent is asking for you right now.',
      contextSignals: [
        {
          id: 'quiet',
          label: 'Quiet',
          summary: 'Nothing urgent is asking for you right now.',
          tone: 'ok',
        },
      ],
    }));

    assert.match(html, /Quiet/);
    assert.match(html, /Quiet right now/);
    assert.match(html, /<button type="button" class="vnext-chat-brief-card"/);
    assert.doesNotMatch(html, /Nothing urgent is asking for you right now\./);
  });

  it('renders a working chat turn in-stream with the composer still present', () => {
    const html = render(baseProps({
      busy: true,
      messages: [
        { id: 'owner:1', role: 'owner', text: '通勤', timestamp: Date.UTC(2026, 4, 18, 10, 40) },
        {
          id: 'working:1',
          role: 'working',
          text: 'Checking what I know and what I can safely use...',
          timestamp: Date.UTC(2026, 4, 18, 10, 41),
        },
      ],
      value: 'follow-up typed while waiting',
    }));

    assert.match(html, /通勤/);
    assert.match(html, /Checking what I know and what I can safely use/);
    assert.ok(
      html.indexOf('通勤') < html.indexOf('Checking what I know and what I can safely use'),
      'working status should render after the owner message',
    );
    assert.ok(
      html.indexOf('Review calendar access') === -1
        || html.indexOf('Review calendar access') > html.indexOf('Checking what I know and what I can safely use'),
      'dynamic context should not render above the active chat turn',
    );
    assert.doesNotMatch(html, /vnext-chat-starters compact/);
    assert.doesNotMatch(html, /Active context while chatting/);
    assert.match(html, /dateTime="2026-05-18T10:40:00.000Z"[^>]*>10:40<\/time>/);
    assert.match(html, /dateTime="2026-05-18T10:41:00.000Z"[^>]*>10:41<\/time>/);
    assert.match(html, /title="2026-05-18 10:40:00 UTC"/);
    assert.match(html, /aria-label="Show full time 2026-05-18 10:41:00 UTC"/);
    assert.match(html, /Keep typing while Aegis works/);
    assert.match(html, /follow-up typed while waiting/);
    assert.match(html, />Queue<\/span>/);
    assert.match(html, /aria-label="Queue for Angie"/);
    assert.doesNotMatch(html, /Sent: &quot;通勤&quot;/);
  });

  it('renders the localized live-progress (Slice 3) working text on a tool_start (zh)', () => {
    // App.tsx maps a `tool_start` SSE frame through toolPhrases → human copy
    // (e.g. web_search → "正在搜索…"). The catalog fallback stays generic.
    const usingToolZh = '正在搜索…';
    assert.equal(t('chatHome.working.usingTool', 'zh'), '正在处理…');
    assert.equal(t('chatHome.working.toolDone', 'zh'), '好了，正在整理回复…');
    const html = renderWithLocale('zh', baseProps({
      busy: true,
      messages: [
        { id: 'owner:1', role: 'owner', text: '帮我查天气', timestamp: Date.UTC(2026, 5, 9, 10, 40) },
        {
          id: 'working:1',
          role: 'working',
          text: usingToolZh,
          timestamp: Date.UTC(2026, 5, 9, 10, 41),
        },
      ],
    }));
    assert.match(html, /正在搜索…/);
    assert.ok(
      html.indexOf('帮我查天气') < html.indexOf('正在搜索…'),
      'the live-progress working status should render after the owner message',
    );
    // The image-routing variant uses a non-interpolated key.
    assert.equal(t('chatHome.working.processingImage', 'zh'), '正在处理图片…');
  });

  it('keeps failed chat retry available while Aegis is busy', () => {
    const html = render(baseProps({
      busy: true,
      messages: [
        {
          id: 'aegis:error',
          role: 'aegis',
          text: "I couldn't finish that. Try again.",
          result: assistantResponse({
            response_kind: 'error',
            visible_answer: "I couldn't finish that. Try again.",
            receipt_expectation: 'No action was taken.',
          }),
          retryText: 'auth smoke retry while busy',
          retryLabel: 'Try again',
        },
        { id: 'owner:busy', role: 'owner', text: 'current request' },
        { id: 'working:busy', role: 'working', text: 'Checking what I know and what I can safely use...' },
      ],
    }));

    assert.match(html, /Queue retry/);
    assert.match(html, /Checking what I know and what I can safely use/);
    assert.doesNotMatch(html, /No action was taken/);
    assert.doesNotMatch(html, /<summary class="vnext-disclosure-summary"><span>Receipt<\/span><\/summary>/);
    assert.doesNotMatch(html, /disabled="">[^<]*Queue retry<\/span><\/button>/);
  });

  it('styles the error-retry button with the secondary design-system class (not a bare button)', () => {
    const html = render(baseProps({
      busy: false,
      messages: [
        {
          id: 'aegis:error',
          role: 'aegis',
          text: "I couldn't finish that. Try again.",
          result: assistantResponse({
            response_kind: 'error',
            visible_answer: "I couldn't finish that. Try again.",
            receipt_expectation: 'No action was taken.',
          }),
          retryText: 'retry me',
          retryLabel: 'Try again',
        },
      ],
    }));

    // The retry button must render through VNextButton (design-system secondary)
    // so it does not fall back to the legacy primary-gradient bare <button>.
    assert.match(
      html,
      /<button class="vnext-btn vnext-btn-secondary vnext-btn-sm"[^>]*>.*Try again<\/span><\/button>/,
    );
    // Regression guard: never ship the retry control as a class-less bare button.
    assert.doesNotMatch(html, /<button type="button">Try again<\/button>/);
  });

  it('renders Web-first delivery notices as native action cards instead of command text', () => {
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:notice',
          role: 'aegis',
          text: [
            'SysOp change #42 needs review.',
            '',
            'Web actions: `approve sysop 42` or `reject sysop 42`.',
          ].join('\n'),
          timestamp: Date.UTC(2026, 4, 20, 18, 5),
        },
      ],
    }));

    assert.match(html, /SysOp change #42 needs review/);
    assert.match(html, /<details open="" class="vnext-chat-action-disclosure vnext-chat-notice-action-card vnext-disclosure" aria-label="Aegis actions">/);
    assert.match(html, /aria-label="Aegis actions"/);
    assert.match(html, /<summary class="vnext-disclosure-summary"><span><svg[\s\S]*Actions<\/span><em>2<\/em><\/summary>/);
    assert.match(html, /Approve/);
    assert.match(html, /Reject/);
    assert.match(html, /SysOp #42/);
    assert.doesNotMatch(html, /Web actions:/);
    assert.doesNotMatch(html, /approve sysop 42/);
    assert.doesNotMatch(html, /reject sysop 42/);
  });

  it('renders anchored sensitive mail notices as inline work cards', () => {
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:sensitive-mail',
          role: 'aegis',
          text: '📬 [primary] Sensitive item needs review: Open the Web Console and complete step-up MFA to view details.',
          timestamp: Date.UTC(2026, 4, 20, 18, 7),
          workItem: mailWorkItem({
            id: 'email:42',
            sensitivity: 'S3',
            privacy_level: 'owner_only',
          }),
          workItemAnchorId: 'email:42',
        },
      ],
    }));

    assert.match(html, /Sensitive item needs review\. Open details below when you are ready\./);
    assert.match(html, /Billing statement available/);
    assert.match(html, /Open message/);
    assert.match(html, /aria-expanded="false"/);
    assert.doesNotMatch(html, /Open the Web Console/);
  });

  it('follows the interface language on the approval decision chat card', () => {
    const stored = 'Aegis needs a decision. Open Work to review the approval.';
    const english = render(baseProps({
      messages: [{ id: 'aegis:decision', role: 'aegis', text: stored }],
    }));
    assert.match(english, /Aegis needs a decision\. Open Work to review the approval\./);
    const chinese = renderWithLocale('zh', baseProps({
      messages: [{ id: 'aegis:decision', role: 'aegis', text: stored }],
    }));
    assert.match(chinese, /安琪需要你做决定。打开「工作」查看这次批准。/);
    assert.doesNotMatch(chinese, /Aegis needs a decision/);
    const switched = render(baseProps({
      messages: [{
        id: 'aegis:decision-zh',
        role: 'aegis',
        text: '安琪需要你做决定。打开「工作」查看这次批准。',
      }],
    }));
    assert.match(switched, /Aegis needs a decision\. Open Work to review the approval\./);
    assert.doesNotMatch(switched, /安琪需要你做决定/);
  });

  it('renders reminder notices with collapsed direct action buttons', () => {
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:reminder',
          role: 'aegis',
          text: [
            '⏰ 找时间看看 Jon 发的视频',
            '',
            'Web actions: `done reminder 1894`, `snooze reminder 1894`, `tomorrow reminder 1894`, or `delete reminder 1894`.',
          ].join('\n'),
          timestamp: Date.UTC(2026, 4, 22, 10, 0),
          severity: 'important',
        },
      ],
    }));

    assert.match(html, /找时间看看 Jon 发的视频/);
    assert.match(html, /<details open="" class="vnext-chat-action-disclosure vnext-chat-notice-action-card vnext-disclosure" aria-label="Aegis actions">/);
    assert.match(html, /<summary class="vnext-disclosure-summary"><span><svg[\s\S]*Actions<\/span><em>4<\/em><\/summary>/);
    assert.match(html, /Done/);
    assert.match(html, /Snooze/);
    assert.match(html, /Tomorrow/);
    assert.match(html, /Delete/);
    assert.match(html, /Reminder #1894 · 1h/);
    assert.doesNotMatch(html, /Web actions:/);
    assert.doesNotMatch(html, /done reminder 1894/);
  });

  it('lays Chinese 网页操作 commands out as separate actions', () => {
    const html = renderWithLocale('zh', baseProps({
      messages: [
        {
          id: 'aegis:reminder-zh',
          role: 'aegis',
          text: [
            '提醒 · 未完成',
            '',
            '网页操作：`done reminder 1894`、`snooze reminder 1894`、`tomorrow reminder 1894` 或 `delete reminder 1894`。',
          ].join('\n'),
          timestamp: Date.UTC(2026, 8, 24, 5, 11),
        },
      ],
    }));

    assert.match(html, /提醒 · 未完成/);
    assert.match(html, /完成/);
    assert.match(html, /稍后提醒/);
    assert.match(html, /明天/);
    assert.match(html, /删除/);
    assert.doesNotMatch(html, /网页操作/);
    assert.doesNotMatch(html, /done reminder 1894/);
    assert.doesNotMatch(html, /snooze reminder/);
  });

  it('renders plain chat SOURCES blocks as expandable evidence instead of raw transcript text', () => {
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:sources',
          role: 'aegis',
          text: [
            '已开始关注 Grok Build agent。',
            '我会每 12 小时查一次；有意义的新进展才提醒你。',
            '',
            'SOURCES',
            'Memory',
            'Tasks and reminders',
          ].join('\n'),
          timestamp: Date.UTC(2026, 4, 22, 1, 15),
        },
      ],
    }));

    assert.match(html, /已开始关注 Grok Build agent/);
    assert.match(html, /<summary class="vnext-disclosure-summary"><span>Sources<\/span><\/summary>/);
    assert.match(html, /Relevant saved context was used/);
    assert.match(html, /Task or reminder records were used/);
    assert.doesNotMatch(html, />SOURCES</);
    assert.doesNotMatch(html, /<p>Memory<\/p>/);
    assert.doesNotMatch(html, /<p>Tasks and reminders<\/p>/);
  });

  it('collapses long daily brief notices into a compact summary with expandable original text', () => {
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:daily-brief',
          role: 'aegis',
          severity: 'important',
          text: [
            '# Daily Brief for 2026-05-26 (America/Los_Angeles)',
            '_Generated at: 2026-05-26T09:00:49.859404490-07:00_',
            '',
            '## Calendar',
            'Calendar not queried; do not assume there are no events.',
            '',
            '## Gmail',
            'Gmail not queried; do not assume there are no important emails.',
            '',
            '## Active Tasks',
            '- 完善 Vision Architect [normal]',
            '- 研究 CapCut [normal]',
            '',
            '## Recent Actions',
            '- Executed: Task created: 完善 Vision Architect',
            '',
            '## Warnings',
            '- calendar_not_queried: Calendar not queried.',
            '- gmail_not_queried: Gmail not queried.',
          ].join('\n'),
        },
      ],
    }));

    assert.match(html, /class="vnext-chat-long-notice-summary tone-important"/);
    assert.match(html, /Daily Brief for 2026-05-26/);
    assert.match(html, /2 active tasks/);
    assert.match(html, /1 recent action/);
    assert.match(html, /2 warnings/);
    assert.match(html, /Full brief/);
    assert.match(html, /完善 Vision Architect/);
  });

  it('collapses long suggested work notices and moves inline source URLs into evidence', () => {
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:grok-build',
          role: 'aegis',
          severity: 'important',
          text: [
            'Grok Build 有新进展：xAI 已面向 X Premium Plus 用户推出早期 beta。它是终端里的 coding/agentic CLI，重点能力包括并行 sub-agents、headless mode、MCP server 兼容。',
            '来源： https://x.com/ssict/status/2059301854798876911',
            'Suggested task: Search the web and X/Twitter for the latest news about "Grok Build agent" and related xAI/Grok build-agent announcements, releases, demos, pricing/access changes, GitHub/repo mentions, and credible social posts from the last 12 hours. Summarize only noteworthy updates in Chinese, include raw source URLs, and notify the user only if there is a meaningful new development.',
            'Suggested reminder: Search the web and X/Twitter for the latest news about "Grok Build agent" and related xAI/Grok build-agent announcements, releases, demos, pricing/access changes, GitHub/repo mentions, and credible social posts from the last 12 hours. Summarize only noteworthy updates in Chinese, include raw source URLs, and notify the user only if there is a meaningful new development. When: 2026-05-26T16:01:50+00:00',
          ].join('\n'),
        },
      ],
    }));

    assert.match(html, /class="vnext-chat-long-notice-summary tone-important"/);
    assert.match(html, /Grok Build 有新进展/);
    assert.match(html, /Suggested task \+ reminder attached/);
    assert.match(html, /Full message/);
    assert.match(html, /<summary class="vnext-disclosure-summary"><span>Sources<\/span><\/summary>/);
    assert.match(html, /External link: x\.com/);
    assert.doesNotMatch(html, /来源： https:\/\/x\.com/);
  });

  it('does not treat normal prose after a sources heading as expandable source labels', () => {
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:not-sources',
          role: 'aegis',
          text: [
            'I cannot verify this yet.',
            '',
            'Sources',
            'I do not have a current provider receipt.',
          ].join('\n'),
        },
      ],
    }));

    assert.match(html, /I cannot verify this yet/);
    assert.match(html, /Sources/);
    assert.match(html, /I do not have a current provider receipt/);
    assert.doesNotMatch(html, /class="vnext-source-evidence-row"/);
  });

  it('collapses nearby repeated system restart notices into the latest visible card', () => {
    const restartText = (activeSkills) => [
      '系统已重启 / System restarted',
      '当前重要信息',
      `活跃技能 / Active skills: ${activeSkills}`,
      'HITL 待审批: 0',
      'Initiative 待处理: 33',
    ].join('\n');
    const html = render(baseProps({
      messages: [
        { id: 'restart:1', role: 'aegis', text: restartText(21), timestamp: Date.UTC(2026, 4, 22, 5, 27) },
        { id: 'restart:2', role: 'aegis', text: restartText(22), timestamp: Date.UTC(2026, 4, 22, 5, 50) },
        { id: 'restart:3', role: 'aegis', text: restartText(23), timestamp: Date.UTC(2026, 4, 22, 6, 13) },
      ],
    }));

    assert.match(html, /3 restart notices/);
    assert.match(html, /2 earlier nearby restart notices were collapsed/);
    assert.match(html, /aria-label="Earlier restart notices"/);
    assert.match(html, /活跃技能 \/ Active skills: 21/);
    assert.match(html, /活跃技能 \/ Active skills: 22/);
    assert.match(html, /活跃技能 \/ Active skills: 23/);
    assert.match(html, /dateTime="2026-05-22T05:27:00.000Z"[^>]*>05:27<\/time>/);
    assert.match(html, /dateTime="2026-05-22T05:50:00.000Z"[^>]*>05:50<\/time>/);
    assert.match(html, /dateTime="2026-05-22T06:13:00.000Z"[^>]*>06:13<\/time>/);
    assert.ok(
      html.indexOf('</div><details class="vnext-chat-repeat vnext-disclosure"') > html.indexOf('class="vnext-chat-answer-heading"'),
      'collapsed notices should render as a full-width block below the message heading, not inside the flex heading',
    );
  });

  it('does not collapse repeated restart notices when they are not nearby', () => {
    const restartText = '系统已重启 / System restarted\n当前重要信息';
    const html = render(baseProps({
      messages: [
        { id: 'restart:morning', role: 'aegis', text: restartText, timestamp: Date.UTC(2026, 4, 22, 5, 27) },
        { id: 'restart:night', role: 'aegis', text: restartText, timestamp: Date.UTC(2026, 4, 22, 18, 0) },
      ],
    }));

    assert.equal((html.match(/System restarted/g) || []).length, 2);
    assert.doesNotMatch(html, /restart notices/);
  });

  it('uses the same collapse mechanism for another status-notice category', () => {
    const reportText = [
      'System Integrity Report',
      'Invariants: 3/3 ✅',
      'Rate Monitors: 4/4 ✅',
      'Active alerts: 0',
    ].join('\n');
    const html = render(baseProps({
      messages: [
        { id: 'integrity:1', role: 'aegis', text: reportText, timestamp: Date.UTC(2026, 4, 22, 7, 0) },
        { id: 'integrity:2', role: 'aegis', text: reportText, timestamp: Date.UTC(2026, 4, 22, 7, 30) },
      ],
    }));

    assert.match(html, /2 integrity reports/);
    assert.match(html, /1 earlier nearby integrity reports were collapsed/);
    assert.match(html, /aria-label="Earlier integrity reports"/);
    assert.match(html, /Rate Monitors: 4\/4/);
  });

  it('renders structured severity on plain restored Aegis messages without keyword matching', () => {
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:provider-balance',
          role: 'aegis',
          text: 'Provider balance is below your threshold.',
          severity: 'important',
          timestamp: Date.UTC(2026, 4, 22, 8, 0),
        },
      ],
    }));

    assert.match(html, /class="vnext-rich-callout important"/);
    // `render()` wraps in an English LocaleProvider, so the generic "Important"
    // callout label and Dismiss button resolve to their English catalog values.
    // The dual-locale suite below proves the zh path renders too.
    assert.match(html, /<strong><span aria-hidden="true">‼️<\/span> Important<\/strong>/);
    assert.match(html, /Provider balance is below your threshold/);
    assert.match(html, />Dismiss<\/button>/);
  });

  it('keeps clear chat available while Aegis is busy', () => {
    // Clear is menu-only chrome (not a peer topbar button). SSR does not open
    // the menu, so assert the account menu remains while busy and clear is not
    // a disabled peer button.
    const html = render(baseProps({
      busy: true,
      messages: [
        { id: 'owner:1', role: 'owner', text: 'hello' },
        { id: 'working:1', role: 'working', text: 'Aegis is checking.' },
      ],
    }));
    assert.match(html, /aria-label="Account menu"/);
    assert.doesNotMatch(html, /vnext-chat-clear/);
    assert.match(html, /Aegis is checking/);
  });

  it('renders returned work cards as contextual collaboration instead of navigation-only results', () => {
    const item = mailWorkItem();
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:1',
          role: 'aegis',
          text: 'I found a mail item that may need attention.',
          timestamp: Date.UTC(2026, 4, 18, 10, 42),
          result: assistantResponse(),
          workItem: item,
          workItemThread: [
            {
              id: 'thread:1',
              question: '为什么重要',
              timestamp: Date.UTC(2026, 4, 18, 10, 43),
              responseTimestamp: Date.UTC(2026, 4, 18, 10, 44),
              result: assistantResponse({
                visible_answer: 'Answer: Open the message before deciding whether to archive or mark read.',
              }),
            },
          ],
        },
      ],
    }));

    assert.match(html, /Billing statement available/);
    assert.match(html, /Maple Lane Music School/);
    assert.match(html, /Next/);
    assert.match(html, /Open the exact email, then brief, draft, archive, or mark read/);
    assert.match(html, /Receipt/);
    assert.match(html, /Service receipt kept/);
    assert.match(html, /Ask Angie/);
    assert.match(html, /Open message/);
    assert.doesNotMatch(html, /Mark read/);
    assert.doesNotMatch(html, /Archive/);
    assert.match(html, /为什么重要/);
    assert.match(html, /Open the message before deciding/);
    assert.match(html, /dateTime="2026-05-18T10:42:00.000Z"[^>]*>10:42<\/time>/);
    assert.match(html, /dateTime="2026-05-18T10:43:00.000Z"[^>]*>10:43<\/time>/);
    assert.match(html, /dateTime="2026-05-18T10:44:00.000Z"[^>]*>10:44<\/time>/);
    assert.doesNotMatch(html, /No external action was executed/);
    assert.doesNotMatch(html, /source_content_untrusted/);
    assert.doesNotMatch(html, /visible_fields/);
  });

  it('uses owner-facing service receipt copy in chat work cards', () => {
    const item = mailWorkItem({
      authority: {
        ...mailWorkItem().authority,
        expected_receipt: 'Mailbox changes keep a provider receipt.',
      },
      receipt_expectation: 'Mailbox changes keep a provider receipt.',
    });
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:provider',
          role: 'aegis',
          text: 'I found a mail item.',
          result: assistantResponse(),
          workItem: item,
        },
      ],
    }));

    assert.match(html, /Service receipt kept/);
    assert.doesNotMatch(html, /provider receipt/);
  });

  it('uses owner-facing approval copy in chat work cards', () => {
    const item = mailWorkItem({
      id: 'approval:calendar',
      lane: 'decide',
      kind: 'approval',
      source_kind: 'decision',
      source: 'hitl',
      title: 'Review calendar access',
      summary: 'Aegis needs your choice before reading calendar details.',
      status: 'waiting',
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
        { label: 'Approve', action_kind: 'approval.approve', route: null, requires_gate: false, requires_mfa: false },
        { label: 'Reject', action_kind: 'approval.reject', route: null, requires_gate: false, requires_mfa: false },
      ],
      next_safe_action: '',
      receipt_expectation: 'Reading the request does not execute an outside action.',
    });
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:approval',
          role: 'aegis',
          text: 'Aegis needs your decision.',
          result: assistantResponse(),
          workItem: item,
        },
      ],
    }));

    assert.match(html, /Review request/);
    assert.match(html, /Approve/);
    assert.match(html, /Reject/);
    assert.doesNotMatch(html, /Review approval/);
  });

  it('shows only server-allowed quick actions in chat work cards', () => {
    const item = mailWorkItem({
      kind: 'task',
      source_kind: 'task',
      source: 'tasks',
      secondary_actions: [
        {
          label: 'Complete',
          action_kind: 'task.complete',
          route: null,
          requires_gate: false,
          requires_mfa: false,
        },
      ],
    });
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:server-actions',
          role: 'aegis',
          text: 'I found a work item.',
          result: assistantResponse(),
          workItem: item,
        },
      ],
    }));

    assert.match(html, /Complete|完成/);
    assert.doesNotMatch(html, /Archive/);
  });

  it('hides stale item-local answers after the work item status changes', () => {
    const item = mailWorkItem({ status: 'archived' });
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:stale-thread',
          role: 'aegis',
          text: 'I found a mail item.',
          result: assistantResponse(),
          workItem: item,
          workItemThread: [
            {
              id: 'thread:old',
              question: 'Explain before archive',
              itemStatus: 'unread',
              result: assistantResponse({
                visible_answer: 'This billing statement is unread.',
              }),
            },
            {
              id: 'thread:new',
              question: 'Explain after archive',
              itemStatus: 'archived',
              result: assistantResponse({
                visible_answer: 'This billing statement has been archived.',
              }),
            },
          ],
        },
      ],
    }));

    assert.match(html, /Email · Archived/);
    assert.match(html, /Explain after archive/);
    assert.match(html, /This billing statement has been archived/);
    assert.doesNotMatch(html, /Explain before archive/);
    assert.doesNotMatch(html, /This billing statement is unread/);
  });

  it('renders blocked automations as paused owner work without internal gate language', () => {
    const item = blockedAutomationItem();
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:blocked',
          role: 'aegis',
          text: 'This automation paused before action.',
          result: assistantResponse({
            visible_answer: 'Answer: This automation needs your choice before it continues.',
          }),
          workItem: item,
        },
      ],
    }));

    assert.match(html, /Monitor Gmail/);
    assert.match(html, /Paused before action/);
    assert.match(html, /Ask Angie/);
    assert.doesNotMatch(html, /standing instruction/i);
    assert.doesNotMatch(html, /blocked gate/i);
    assert.doesNotMatch(html, /required gate blocked/i);
    assert.doesNotMatch(html, /Mark read|Archive/);
  });

  it('keeps chat work items inspectable while Aegis is busy and shows queued local questions', () => {
    const item = mailWorkItem();
    const html = render(baseProps({
      busy: true,
      messages: [
        {
          id: 'aegis:1',
          role: 'aegis',
          text: 'I found a mail item that may need attention.',
          result: assistantResponse(),
          workItem: item,
          workItemThread: [
            {
              id: 'thread:queued',
              question: '解释为什么重要',
              state: 'queued',
            },
          ],
        },
      ],
    }));

    assert.match(html, /Billing statement available/);
    assert.match(html, /Open message/);
    assert.match(html, /Ask Angie/);
    assert.doesNotMatch(html, /Mark read/);
    assert.doesNotMatch(html, /Archive/);
    assert.match(html, /解释为什么重要/);
    assert.match(html, /Queued\. Aegis will answer this next\./);
    assert.doesNotMatch(html, /<button type="button" class="vnext-secondary" disabled="">[\s\S]*Open message/);
    assert.doesNotMatch(html, /<button type="button" class="vnext-secondary" disabled="">[\s\S]*Ask Angie/);
    assert.doesNotMatch(html, /<button type="button" class="vnext-secondary" disabled="">[\s\S]*Mark read/);
    assert.doesNotMatch(html, /<button type="button" class="vnext-secondary" disabled="">[\s\S]*Archive/);
  });

  it('hides stale embedded work actions after the refreshed queue no longer contains the item', () => {
    const html = render(baseProps({
      messages: [
        {
          id: 'aegis:stale',
          role: 'aegis',
          text: 'This work item changed.',
          result: assistantResponse(),
          workItem: mailWorkItem({
            status: 'done',
            summary: 'This item changed after it was shown here. Aegis refreshed the work queue and hid the old actions.',
            next_safe_action: 'Ask Aegis if you still need it.',
            secondary_actions: [],
          }),
          workItemUnavailable: true,
        },
      ],
    }));

    assert.match(html, /Billing statement available/);
    assert.match(html, /Aegis refreshed this; old actions are hidden/);
    assert.match(html, /No new action ran/);
    assert.match(html, /Ask Angie/);
    assert.doesNotMatch(html, /Open message/);
    assert.doesNotMatch(html, /Mark read/);
    assert.doesNotMatch(html, /Archive/);
  });
});

describe('vNext Chat Home i18n', () => {
  it('localizes the empty-home chrome for both locales', () => {
    const en = renderWithLocale('en', baseProps());
    const zh = renderWithLocale('zh', baseProps());

    // Hero + composer chrome.
    assert.match(en, /Tell Angie\./);
    assert.match(en, /<p class="vnext-kicker">Ready<\/p>/);
    assert.match(en, /placeholder="Tell Angie…"/);
    assert.match(en, /aria-label="Account menu"/);
    assert.match(en, /aria-label="Record a voice message"/);
    // Snapshot is menu-only — not a peer topbar label.
    assert.doesNotMatch(en, /vnext-chat-action-label">Snapshot</);

    assert.match(zh, /告诉安琪。/);
    assert.match(zh, /<p class="vnext-kicker">就绪<\/p>/);
    assert.match(zh, /placeholder="告诉安琪…"/);
    assert.match(zh, /aria-label="账户菜单"/);
    assert.match(zh, /aria-label="录制语音消息"/);
    assert.doesNotMatch(zh, /vnext-chat-action-label">快照</);
    // English chrome must NOT leak into the zh render.
    assert.doesNotMatch(zh, /Tell Angie\./);
    assert.doesNotMatch(zh, /aria-label="Account menu"/);
  });

  it('localizes the brief card label for both locales', () => {
    const props = baseProps({ brief: 'Needs attention: Review calendar access.' });
    assert.match(renderWithLocale('en', props), /<em>Start here<\/em>/);
    assert.match(renderWithLocale('zh', props), /<em>从这里开始<\/em>/);
  });

  it('localizes work card next-step and receipt copy for both locales', () => {
    const props = baseProps({
      messages: [
        {
          id: 'aegis:1',
          role: 'aegis',
          text: 'I found a mail item that may need attention.',
          result: assistantResponse(),
          workItem: mailWorkItem(),
        },
      ],
    });
    const en = renderWithLocale('en', props);
    const zh = renderWithLocale('zh', props);

    assert.match(en, /Open the exact email, then brief, draft, archive, or mark read/);
    assert.match(en, /Service receipt kept/);
    assert.match(en, /<span class="vnext-btn-label">Ask Angie<\/span>/);
    assert.match(en, /<em>Next<\/em>/);
    assert.match(en, /<em>Receipt<\/em>/);

    assert.match(zh, /Open the exact email, then brief, draft, archive, or mark read/);
    assert.match(zh, /已保留服务记录。/);
    assert.match(zh, /<span class="vnext-btn-label">问安琪<\/span>/);
    assert.match(zh, /<em>下一步<\/em>/);
    assert.match(zh, /<em>记录<\/em>/);
    assert.doesNotMatch(zh, /Service receipt kept/);
  });

  it('localizes the collapsed restart-notice badge for both locales', () => {
    const restartText = (n) => `系统已重启 / System restarted\nActive skills: ${n}`;
    const props = baseProps({
      messages: [
        { id: 'restart:1', role: 'aegis', text: restartText(21), timestamp: Date.UTC(2026, 4, 22, 5, 27) },
        { id: 'restart:2', role: 'aegis', text: restartText(22), timestamp: Date.UTC(2026, 4, 22, 5, 50) },
      ],
    });
    assert.match(renderWithLocale('en', props), /restart notices/);
    assert.match(renderWithLocale('en', props), /aria-label="Earlier restart notices"/);
    assert.match(renderWithLocale('zh', props), /条重启通知/);
  });

  it('localizes the daily-brief summary meta counts for both locales', () => {
    const props = baseProps({
      messages: [
        {
          id: 'aegis:daily-brief',
          role: 'aegis',
          severity: 'important',
          text: [
            '# Daily Brief for 2026-05-26 (America/Los_Angeles)',
            '',
            '## Active Tasks',
            '- 完善 Vision Architect [normal]',
            '- 研究 CapCut [normal]',
            '',
            '## Recent Actions',
            '- Executed: Task created: 完善 Vision Architect',
          ].join('\n'),
        },
      ],
    });
    assert.match(renderWithLocale('en', props), /2 active tasks/);
    assert.match(renderWithLocale('en', props), /Full brief/);
    assert.match(renderWithLocale('en', props), /Daily Brief for 2026-05-26/);
    const zh = renderWithLocale('zh', props);
    assert.match(zh, /2 个活跃任务/);
    assert.match(zh, /完整简报/);
    assert.match(zh, /2026-05-26 的今日摘要/);
    assert.doesNotMatch(zh, /Daily Brief/);
    assert.doesNotMatch(zh, /Active Tasks/);
    assert.match(zh, /进行中的待办/);
  });

  it('localizes the assistant error header and retry button (no English leak in zh)', () => {
    // Owner complaint: the chat error header "I couldn't finish that" and the
    // "Try again" retry button leaked English into a zh session. Both must
    // render in display_language. `retryLabel` stays the English logic key in
    // App.tsx; only the displayed text is localized.
    const props = baseProps({
      messages: [
        {
          id: 'aegis:error',
          role: 'aegis',
          text: 'I couldn\'t finish that. Try again.',
          result: assistantResponse({
            response_kind: 'error',
            visible_answer: 'Something failed. Try again.',
            receipt_expectation: 'No action was taken.',
          }),
          retryText: 'retry the failed chat command',
          retryLabel: 'Try again',
        },
      ],
    });

    const en = render(props);
    assert.match(en, /I couldn&#x27;t finish that|I couldn't finish that/);
    assert.match(en, /Try again<\/span><\/button>/);

    const zh = renderWithLocale('zh', props);
    // Localized header + retry button.
    assert.match(zh, /我没能完成这件事/);
    assert.match(zh, /重试<\/span><\/button>/);
    // The English title/retry phrases must NOT survive in the zh render.
    assert.doesNotMatch(zh, /I couldn(&#x27;|')t finish that/);
    assert.doesNotMatch(zh, /Try again<\/span><\/button>/);
  });

  it('shows a retry button beside a plain failure hint', () => {
    const hint = '这轮没有生成回复。消息已留下，可以再发一次。';
    const props = baseProps({
      messages: [
        { id: 'owner:stuck', role: 'owner', text: '帮我看看这封信', timestamp: 1_700_000_000_000 },
        {
          id: 'aegis:stuck',
          role: 'aegis',
          text: hint,
          severity: 'warning',
          timestamp: 1_700_000_000_000,
          retryText: '帮我看看这封信',
          retryLabel: 'Try again',
        },
        { id: 'aegis:ok', role: 'aegis', text: '信已归档。', timestamp: 1_700_000_030_000 },
      ],
    });
    const zh = renderWithLocale('zh', props);
    assert.match(zh, /这轮没有生成回复/);
    assert.match(zh, /data-testid="aegis-chat-retry"/);
    assert.match(zh, /重试<\/span><\/button>/);
    assert.doesNotMatch(zh, /Try again<\/span><\/button>/);
    const retryButtons = zh.match(/data-testid="aegis-chat-retry"/g) || [];
    assert.equal(retryButtons.length, 1);
  });

  it('localizes the Check-result retry display while keeping the English logic key', () => {
    const props = baseProps({
      messages: [
        {
          id: 'aegis:timeout',
          role: 'aegis',
          text: 'Aegis needs another try.',
          result: assistantResponse({
            response_kind: 'error',
            visible_answer: 'I did not get a complete answer. Try again.',
            receipt_expectation: 'No action was taken.',
          }),
          retryText: 'check the timed-out result',
          retryLabel: 'Check result',
        },
      ],
    });

    assert.match(render(props), /Check result<\/span><\/button>/);
    const zh = renderWithLocale('zh', props);
    assert.match(zh, /查看结果<\/span><\/button>/);
    assert.doesNotMatch(zh, /Check result<\/span><\/button>/);
    // Error header localized too (timeout class → "安琪需要再试一次").
    assert.match(zh, /安琪需要再试一次/);
  });
});

// Slice 4: in-chat HITL approval card. The card renders from a `role:'approval'`
// message (pushed by App.tsx when a `need_approval` SSE frame arrives). Pending
// shows Approve/Reject; once actioned it shows a disabled localized state chip.
// The summary excerpt is the only payload (never raw html/shell).
describe('vNext Chat Home approval card (Slice 4)', () => {
  function approvalMessage(overrides = {}) {
    return {
      id: 'approval:appr-1',
      role: 'approval',
      text: '',
      timestamp: Date.UTC(2026, 5, 9, 10, 45),
      approvalId: 'appr-1',
      toolName: 'send_email',
      summary: 'Send the reply to billing@northwest.example with the drafted refund text.',
      approvalState: 'pending',
      ...overrides,
    };
  }

  it('renders an egress hold with destination, body, and why it stopped (zh)', () => {
    const html = renderWithLocale('zh', baseProps({
      messages: [
        approvalMessage({
          toolName: 'http_api',
          summary: [
            'destination: https://status.example/hook',
            'would_send: 季度营收备注',
            'stopped: untrusted_source',
            'untrusted_source: web_page',
          ].join('\n'),
        }),
      ],
    }));
    assert.match(html, /发往/);
    assert.match(html, /将发送/);
    assert.match(html, /为什么要你看/);
    assert.match(html, /https:\/\/status\.example\/hook/);
    assert.match(html, /季度营收备注/);
    assert.match(html, /来源不可信 \(网页\)/);
    assert.match(html, /vnext-chat-approval-approve/);
    assert.match(html, /vnext-chat-approval-reject/);
    assert.doesNotMatch(html, /http_api/);
  });

  it('renders a pending CEO review card with What/Why and both buttons (zh)', () => {
    const html = renderWithLocale('zh', baseProps({
      messages: [
        { id: 'owner:1', role: 'owner', text: '帮我回复账单邮件', timestamp: Date.UTC(2026, 5, 9, 10, 44) },
        approvalMessage(),
      ],
    }));

    assert.match(html, /需要你确认/);
    assert.match(html, /我准备做/);
    assert.match(html, /为什么要你看/);
    // Summary is the "what" line; raw tool id must not be primary chrome.
    assert.match(html, /Send the reply to billing@northwest\.example/);
    assert.doesNotMatch(html, /工具：send_email/);
    // VNextButton wraps labels in .vnext-btn-label.
    assert.match(html, /vnext-chat-approval-approve/);
    assert.match(html, /vnext-chat-approval-reject/);
    assert.match(html, />批准</);
    assert.match(html, />先不要</);
    assert.doesNotMatch(html, /vnext-chat-approval-state/);
    assert.match(html, /vnext-chat-approval-card/);
    assert.match(html, /vnext-ceo-review-card/);
  });

  it('renders the approved state chip and hides the buttons after approval (zh)', () => {
    const html = renderWithLocale('zh', baseProps({
      messages: [approvalMessage({ approvalState: 'approved' })],
    }));

    assert.match(html, /<span class="vnext-chat-approval-state approved">已批准<\/span>/);
    assert.doesNotMatch(html, /vnext-chat-approval-approve/);
    assert.doesNotMatch(html, /vnext-chat-approval-reject/);
  });

  it('renders the rejected state chip after rejection (zh)', () => {
    const html = renderWithLocale('zh', baseProps({
      messages: [approvalMessage({ approvalState: 'rejected' })],
    }));

    assert.match(html, /<span class="vnext-chat-approval-state rejected">已拒绝<\/span>/);
    assert.doesNotMatch(html, /vnext-chat-approval-approve/);
  });

  it('localizes the approval card for en too (no zh leak)', () => {
    const en = renderWithLocale('en', baseProps({ messages: [approvalMessage()] }));
    assert.match(en, /Needs your review/);
    assert.match(en, /I will/);
    assert.match(en, /Why you/);
    assert.match(en, /vnext-chat-approval-approve/);
    assert.match(en, />Approve</);
    assert.match(en, />Not now</);
    assert.doesNotMatch(en, /需要你确认/);
    assert.doesNotMatch(en, /Tool: send_email/);
  });

  it('disables both buttons when the approvalId is missing (cannot action)', () => {
    const html = renderWithLocale('en', baseProps({
      messages: [approvalMessage({ approvalId: undefined })],
    }));
    assert.match(html, /vnext-chat-approval-approve[^>]*disabled/);
    assert.match(html, /vnext-chat-approval-reject[^>]*disabled/);
  });

  // Pure-reducer logic (the transitions App.tsx drives): push-on-need_approval,
  // optimistic flip, and revert-on-failure.
  it('appendApprovalMessage pushes a pending card and dedups by approvalId', () => {
    const base = [{ id: 'owner:1', role: 'owner', text: 'hi' }];
    const once = appendApprovalMessage(base, { approvalId: 'a1', toolName: 'send_email', summary: 'do it' });
    assert.equal(once.length, 2);
    const card = once[1];
    assert.equal(card.role, 'approval');
    assert.equal(card.approvalId, 'a1');
    assert.equal(card.approvalState, 'pending');
    assert.equal(card.summary, 'do it');
    assert.equal(card.toolName, 'send_email');
    // A replayed/reconnected frame for the same approval does NOT stack a card.
    const twice = appendApprovalMessage(once, { approvalId: 'a1', summary: 'do it again' });
    assert.equal(twice.length, 2);
    assert.strictEqual(twice, once);
  });

  it('replaceApprovalMessagesFromServer keeps one server card and its decision', () => {
    const local = appendApprovalMessage(
      [{ id: 'owner:1', role: 'owner', text: 'hi', timestamp: 1_700_000_000_000 }],
      { approvalId: 'gate-live', toolName: 'send_email', summary: 'live only' },
    );
    const stacked = appendApprovalMessage(local, {
      approvalId: 'gate-old',
      toolName: 'send_email',
      summary: 'destination: old\nwould_send: note\nstopped: sensitive',
    });
    const replaced = replaceApprovalMessagesFromServer(stacked, [{
      approval_id: 'gate-old',
      tool_name: 'send_email',
      summary: 'destination: lawyer@example.com\nwould_send: billing note\nstopped: sensitive',
      state: 'approved',
      timestamp: 1_758_000_000,
    }]);
    const cards = replaced.filter((message) => message.role === 'approval');
    assert.equal(cards.length, 2);
    const decided = cards.find((message) => message.approvalId === 'gate-old');
    assert.equal(decided.approvalState, 'approved');
    assert.match(decided.summary, /lawyer@example.com/);
    assert.equal(cards.filter((message) => message.approvalId === 'gate-old').length, 1);
    assert.equal(cards.find((message) => message.approvalId === 'gate-live').approvalState, 'pending');
  });

  it('setApprovalState flips only the targeted approval card (optimistic + revert)', () => {
    const start = appendApprovalMessage([], { approvalId: 'a1', summary: 's' });
    // Optimistic approve.
    const approved = setApprovalState(start, 'a1', 'approved');
    assert.equal(approved[0].approvalState, 'approved');
    // Revert back to pending (failure path).
    const reverted = setApprovalState(approved, 'a1', 'pending');
    assert.equal(reverted[0].approvalState, 'pending');
    // A different approvalId is untouched.
    const twoCards = appendApprovalMessage(start, { approvalId: 'a2', summary: 's2' });
    const rejectedOne = setApprovalState(twoCards, 'a2', 'rejected');
    assert.equal(rejectedOne.find((m) => m.approvalId === 'a1').approvalState, 'pending');
    assert.equal(rejectedOne.find((m) => m.approvalId === 'a2').approvalState, 'rejected');
  });

  const searchSummary = [
    'destination: search-provider',
    'would_send: Find the latest posts about the launch',
    'stopped: untrusted_source',
    'untrusted_source: tool_result',
    'owner_question: 帮我看看最近的帖子在说什么',
  ].join('\n');

  it('renders one search card with the owner question and hides the tool dump (zh)', () => {
    const html = renderWithLocale('zh', baseProps({
      messages: [
        { id: 'owner:1', role: 'owner', text: '帮我看看最近的帖子在说什么', timestamp: Date.UTC(2026, 5, 9, 10, 44) },
        { id: 'aegis:wait', role: 'aegis', text: '⏳ Tool action · 等待你批准 / awaiting your approval', timestamp: Date.UTC(2026, 5, 9, 10, 45) },
        approvalMessage({
          toolName: 'web_search',
          summary: searchSummary,
        }),
      ],
    }));
    assert.match(html, /你问的/);
    assert.match(html, /帮我看看最近的帖子在说什么/);
    assert.match(html, /来源不可信 \(工具结果\)/);
    assert.equal((html.match(/vnext-chat-approval-card/g) || []).length, 1);
    assert.doesNotMatch(html, /search-provider/);
    assert.doesNotMatch(html, /web_search/);
    assert.doesNotMatch(html, /Tool action/);
    assert.doesNotMatch(html, /等待你批准/);
    assert.doesNotMatch(html, /Find the latest posts/);
  });

  it('keeps 已批准 on the same card while the search runs and then shows the result', () => {
    const running = renderWithLocale('zh', baseProps({
      messages: [approvalMessage({
        toolName: 'web_search',
        summary: searchSummary,
        approvalState: 'approved',
        approvalPhase: 'running',
      })],
    }));
    assert.match(running, /已批准/);
    assert.match(running, /正在继续这次搜索/);
    assert.doesNotMatch(running, /vnext-chat-approval-approve/);
    const done = renderWithLocale('zh', baseProps({
      messages: [approvalMessage({
        toolName: 'web_search',
        summary: searchSummary,
        approvalState: 'approved',
        approvalPhase: 'done',
        approvalOutcome: '最近的帖子提到发布会。',
      })],
    }));
    assert.match(done, /已批准/);
    assert.match(done, /最近的帖子提到发布会。/);
    assert.doesNotMatch(done, /正在继续这次搜索/);
    const failed = renderWithLocale('zh', baseProps({
      messages: [approvalMessage({
        toolName: 'web_search',
        summary: searchSummary,
        approvalState: 'approved',
        approvalPhase: 'failed',
      })],
    }));
    assert.match(failed, /这次搜索没有完成/);
    assert.match(failed, /已批准/);
  });

  it('does not open a second card for the same search outbound', () => {
    const once = appendApprovalMessage(
      [{ id: 'owner:1', role: 'owner', text: '帮我看看最近的帖子在说什么' }],
      { approvalId: 'gate-1', toolName: 'web_search', summary: searchSummary },
    );
    const again = appendApprovalMessage(once, {
      approvalId: 'gate-2',
      toolName: 'web_search',
      summary: searchSummary,
    });
    assert.equal(again.filter((message) => message.role === 'approval').length, 1);
    assert.equal(searchHoldIdentity(searchSummary, 'web_search'), searchHoldIdentity(searchSummary, 'web_search'));
    const other = appendApprovalMessage(once, {
      approvalId: 'gate-3',
      toolName: 'web_search',
      summary: searchSummary.replace('Find the latest posts about the launch', 'a different query'),
    });
    assert.equal(other.filter((message) => message.role === 'approval').length, 2);
  });

  it('keeps the in-progress search card when history still says pending, then shows the stored result', () => {
    const running = setApprovalProgress(
      appendApprovalMessage([], { approvalId: 'gate-1', toolName: 'web_search', summary: searchSummary }),
      'gate-1',
      { approvalState: 'approved', approvalPhase: 'running' },
    );
    const stillPending = replaceApprovalMessagesFromServer(running, [{
      approval_id: 'gate-1',
      tool_name: 'web_search',
      summary: searchSummary,
      state: 'pending',
      timestamp: 1_758_000_100,
    }]);
    assert.equal(stillPending[0].approvalState, 'approved');
    assert.equal(stillPending[0].approvalPhase, 'running');
    const settled = replaceApprovalMessagesFromServer(running, [{
      approval_id: 'gate-1',
      tool_name: 'web_search',
      summary: searchSummary,
      state: 'approved',
      timestamp: 1_758_000_100,
      outcome: 'result\n最近的帖子提到发布会。',
    }]);
    assert.equal(settled[0].approvalPhase, 'done');
    assert.equal(settled[0].approvalOutcome, '最近的帖子提到发布会。');
    assert.equal(ownerVisibleSearchOutcome('{"status":"ok","summary":"最近的帖子提到发布会。","note":"Use only these excerpts."}').text, '最近的帖子提到发布会。');
    assert.equal(ownerVisibleSearchOutcome('{"status":"error","error":"provider down"}').phase, 'failed');
    assert.equal(isGatePendingOwnerAck('⏳ Tool action · 等待你批准 / awaiting your approval'), true);
    assert.equal(isGatePendingOwnerAck('我已经批准了日历，明天再看。'), false);
    const hidden = omitGatePendingOwnerAcks([
      { role: 'owner', text: '帮我看看' },
      { role: 'aegis', text: '⏳ Tool action · 等待你批准 / awaiting your approval' },
      { role: 'approval', text: '' },
    ]);
    assert.equal(hidden.length, 2);
  });
});

describe('composer size toggle (owner 2026-09-12: the "more" menu is gone)', () => {
  it('labels composer size by next action, not cryptic 1L glyphs', () => {
    assert.equal(composerSizeActionLabel('line', 'en'), t('chatHome.composer.expand', 'en'));
    assert.equal(composerSizeActionLabel('half', 'en'), t('chatHome.composer.collapse', 'en'));
    assert.equal(composerSizeActionLabel('line', 'zh'), t('chatHome.composer.expand', 'zh'));
    assert.match(t('chatHome.composer.expand', 'zh'), /[\u4e00-\u9fff]/);
    assert.doesNotMatch(t('chatHome.composer.expand', 'en'), /^1L$/);
  });

  it('renders ONE direct size toggle and no "more" menu trigger (zh)', () => {
    const html = renderWithLocale('zh', baseProps({ onAttachFile: noop, onClearAttachment: noop }));
    assert.match(html, /class="vnext-chat-size-button"[^>]*aria-label="展开输入框"/);
    assert.doesNotMatch(html, /vnext-chat-more-button/);
    assert.doesNotMatch(html, /更多选项/);
    // Attach lives ONLY on the visible paperclip now — no duplicate entry.
    assert.equal((html.match(/aria-label="添加附件"/g) || []).length, 1);
  });

  // The three attach-item behaviours moved from the (deleted) menu item to the
  // visible paperclip; the cases keep their titles so the coverage is traceable.
  it('attach item is disabled while uploading and its onSelect triggers the file input', () => {
    const html = renderWithLocale('zh', baseProps({
      onAttachFile: noop,
      onClearAttachment: noop,
      attachmentUploading: true,
    }));
    const paperclip = html.match(/<button[^>]*class="vnext-chat-attach-button"[^>]*>/)?.[0] || '';
    assert.ok(paperclip, 'paperclip rendered');
    assert.match(paperclip, /\bdisabled\b/, 'paperclip disabled while an upload is in flight');
    // Its click opens the (hidden) file input that the same render carries.
    assert.match(html, /class="vnext-chat-file-input"/);
    assert.match(chatHomeViewSource(), /className="vnext-chat-attach-button"[\s\S]{0,400}onClick=\{\(\) => fileInputRef\.current\?\.click\(\)\}/);
  });

  it('attach item is enabled when not uploading', () => {
    const html = renderWithLocale('zh', baseProps({ onAttachFile: noop, onClearAttachment: noop }));
    const paperclip = html.match(/<button[^>]*class="vnext-chat-attach-button"[^>]*>/)?.[0] || '';
    assert.ok(paperclip, 'paperclip rendered');
    assert.doesNotMatch(paperclip, /\bdisabled\b/);
  });

  it('omits the attach item when there is no attach handler (size still present)', () => {
    const html = render(baseProps());
    assert.doesNotMatch(html, /vnext-chat-attach-button/);
    assert.doesNotMatch(html, /vnext-chat-file-input/);
    assert.match(html, /class="vnext-chat-size-button"/);
  });
});

describe('vNext Chat Home multimodal composer (Slice 5)', () => {
  it('renders the "more" menu + hidden file input when onAttachFile is provided (zh)', () => {
    const html = renderWithLocale('zh', baseProps({ onAttachFile: noop, onClearAttachment: noop }));
    // The composer carries a direct size toggle (no "more" menu any more).
    assert.match(html, /class="vnext-chat-size-button"/);
    // The hidden file input (triggered by either entry) accepts images + PDFs.
    assert.match(html, /class="vnext-chat-file-input"/);
    assert.match(html, /accept="image\/\*,application\/pdf"/);
    // Attach is ALSO a visible paperclip in the composer row: the owner never
    // found it while it lived only inside the "more" menu.
    assert.match(html, /class="vnext-chat-attach-button"[^>]*aria-label="添加附件"/);
  });

  it('does not render the hidden file input when onAttachFile is absent', () => {
    const html = render(baseProps());
    // The size toggle still renders, but with no attach handler there is no
    // file input and no paperclip.
    assert.doesNotMatch(html, /vnext-chat-file-input/);
    assert.match(html, /vnext-chat-size-button/);
    assert.doesNotMatch(html, /vnext-chat-attach-button/);
  });

  it('renders the pending-attachment pill with filename + remove button (zh)', () => {
    const html = renderWithLocale('zh', baseProps({
      onAttachFile: noop,
      onClearAttachment: noop,
      pendingAttachment: {
        kind: 'image',
        dataUrl: 'data:image/png;base64,iVBORw0KGgo=',
        filename: 'receipt.png',
        mimeType: 'image/png',
      },
    }));
    assert.match(html, /class="vnext-chat-attached-file"/);
    // Localized "已附加：{filename}" with the real filename.
    assert.match(html, /已附加：receipt\.png/);
    // Remove button with the localized aria-label.
    assert.match(html, /aria-label="移除附件"/);
    // Composer carries the has-pending-attachment marker class.
    assert.match(html, /has-pending-attachment/);
  });

  it('renders a document attachment pill with its filename (zh)', () => {
    const html = renderWithLocale('zh', baseProps({
      onAttachFile: noop,
      onClearAttachment: noop,
      pendingAttachment: {
        kind: 'document',
        dataUrl: 'data:application/pdf;base64,JVBERi0=',
        filename: 'invoice.pdf',
        mimeType: 'application/pdf',
      },
    }));
    assert.match(html, /已附加：invoice\.pdf/);
  });

  it('renders the uploading state instead of a pill while uploading (zh)', () => {
    const html = renderWithLocale('zh', baseProps({
      onAttachFile: noop,
      onClearAttachment: noop,
      attachmentUploading: true,
    }));
    assert.match(html, /class="vnext-chat-attached-file is-uploading"/);
    assert.match(html, /上传中…/);
    // (The Attach menu item's disabled-while-uploading wiring is a closed-dropdown
    // item, not server-rendered; its `disabled: attachmentUploading` binding is
    // guarded statically in vnextAppStatic.test.mjs instead.)
  });

  it('renders the upload-error state with a remove button (zh)', () => {
    const html = renderWithLocale('zh', baseProps({
      onAttachFile: noop,
      onClearAttachment: noop,
      attachmentError: '上传失败',
    }));
    assert.match(html, /class="vnext-chat-attached-file is-error"/);
    assert.match(html, /上传失败/);
    assert.match(html, /aria-label="移除附件"/);
  });

  it('localizes the "more" menu trigger + pill for en too (no zh leak)', () => {
    const en = renderWithLocale('en', baseProps({
      onAttachFile: noop,
      onClearAttachment: noop,
      pendingAttachment: {
        kind: 'image',
        dataUrl: 'data:image/png;base64,iVBORw0KGgo=',
        filename: 'receipt.png',
        mimeType: 'image/png',
      },
    }));
    // The composer size toggle carries the en aria-label; the "more" menu is gone.
    assert.match(en, /class="vnext-chat-size-button"[^>]*aria-label="Expand composer"/);
    assert.doesNotMatch(en, /More options/);
    assert.match(en, /Attached: receipt\.png/);
    assert.match(en, /aria-label="Remove attachment"/);
    assert.doesNotMatch(en, /更多选项/);
    assert.doesNotMatch(en, /已附加/);
  });
});

// Task 6: the composer attach-context button must use its own distinct glyph —
// NOT the `work` glyph that now means the Work nav tab.
describe('vNext Chat Home attach-context glyph (Task 6)', () => {
  it('attach-context button does not render the work glyph', () => {
    const html = render(baseProps());
    // The work icon is a briefcase: M6 6.5h12v11H6z path. The attach-context
    // button (.vnext-chat-context-button) must NOT contain this path.
    // Strategy: check the HTML segment between the vnext-chat-context-button
    // open tag and the next closing button tag doesn't contain the work SVG path.
    const contextButtonMatch = html.match(/class="vnext-chat-context-button"[^>]*>([\s\S]*?)<\/button>/);
    assert.ok(contextButtonMatch, 'vnext-chat-context-button must be present');
    // The work glyph's distinguishing inner path (briefcase body M6 6.5h12v11H6z).
    assert.doesNotMatch(contextButtonMatch[1], /M6 6\.5h12v11H6z/, 'attach-context button must NOT use the work (briefcase) glyph');
  });

  it('attach-context button renders a distinct link/pin glyph (not the work glyph)', () => {
    const html = render(baseProps());
    const contextButtonMatch = html.match(/class="vnext-chat-context-button"[^>]*>([\s\S]*?)<\/button>/);
    assert.ok(contextButtonMatch, 'vnext-chat-context-button must be present');
    // Must contain an SVG element (any icon).
    assert.match(contextButtonMatch[1], /<svg/, 'attach-context button must contain an SVG icon');
    // Must NOT be the work (briefcase) icon — confirmed by absence of its body path.
    assert.doesNotMatch(contextButtonMatch[1], /M6 6\.5h12v11H6z/);
  });
});

describe('chat date dividers (IMG_3939 cross-day run-together)', () => {
  const TZ = 'America/Los_Angeles';
  // 2026-01-05 20:00 PST and 2026-01-06 09:00 PST (fixed past dates so the
  // labels are formatted dates, deterministic regardless of the real clock).
  const DAY1_EVENING = Date.UTC(2026, 0, 6, 4, 0); // = Jan 5, 20:00 PST
  const DAY2_MORNING = Date.UTC(2026, 0, 6, 17, 0); // = Jan 6, 09:00 PST

  it('labels each day-group start once, in the owner timezone (pure helper)', () => {
    const labels = chatDateDividerLabels(
      [
        { id: 'm1', timestamp: DAY1_EVENING },
        { id: 'm2', timestamp: DAY1_EVENING + 60_000 },
        { id: 'gap' }, // no timestamp: must not break the day group
        { id: 'm3', timestamp: DAY2_MORNING },
      ],
      TZ,
      'zh',
      Date.UTC(2026, 0, 10, 12, 0),
    );
    assert.equal(Object.keys(labels).length, 2);
    assert.match(labels.m1, /1月5日/);
    assert.equal(labels.m2, undefined);
    assert.equal(labels.gap, undefined);
    assert.match(labels.m3, /1月6日/);
  });

  it('uses 今天/昨天 (Today/Yesterday) relative to the injected now', () => {
    const now = Date.UTC(2026, 0, 6, 20, 0); // Jan 6, 12:00 PST
    const zh = chatDateDividerLabels(
      [
        { id: 'y', timestamp: DAY1_EVENING },
        { id: 't', timestamp: DAY2_MORNING },
      ],
      TZ,
      'zh',
      now,
    );
    assert.equal(zh.y, '昨天');
    assert.equal(zh.t, '今天');
    const en = chatDateDividerLabels(
      [{ id: 'y', timestamp: DAY1_EVENING }, { id: 't', timestamp: DAY2_MORNING }],
      TZ,
      'en',
      now,
    );
    assert.equal(en.y, 'Yesterday');
    assert.equal(en.t, 'Today');
  });

  it('does NOT collapse repeated notices across an owner-local day boundary (pure)', () => {
    // Two restart notices 2h apart (well inside the 6h collapse window) but
    // straddling LOCAL midnight in the owner tz: 22:00 PST Jan 5 vs 00:00 PST
    // Jan 6. Merging them would hide the day change inside a time-only
    // collapsed range (the 23:05→01:00 ambiguity this slice removes).
    const restartText = '系统已重启 / System restarted';
    const lateNight = Date.UTC(2026, 0, 6, 6, 0); // Jan 5, 22:00 PST
    const afterMidnight = Date.UTC(2026, 0, 6, 8, 0); // Jan 6, 00:00 PST
    const crossDay = collapsedChatHomeMessages(
      [
        { id: 'restart:1', role: 'aegis', text: restartText, timestamp: lateNight },
        { id: 'restart:2', role: 'aegis', text: restartText, timestamp: afterMidnight },
      ],
      TZ,
    );
    assert.equal(crossDay.length, 2, 'cross-midnight notices must NOT merge');
    // Control: the same 2h gap within ONE owner-local day still collapses.
    const sameDay = collapsedChatHomeMessages(
      [
        { id: 'restart:1', role: 'aegis', text: restartText, timestamp: Date.UTC(2026, 0, 6, 18, 0) },
        { id: 'restart:2', role: 'aegis', text: restartText, timestamp: Date.UTC(2026, 0, 6, 20, 0) },
      ],
      TZ,
    );
    assert.equal(sameDay.length, 1, 'same-day nearby notices keep collapsing');
    assert.ok(sameDay[0].collapsedNotice, 'collapsed card metadata present');

    // An UNDATED duplicate must not bridge the two days: the group's day
    // anchor is its last DATED timestamp, not the (undated) visible card.
    const bridged = collapsedChatHomeMessages(
      [
        { id: 'restart:1', role: 'aegis', text: restartText, timestamp: lateNight },
        { id: 'restart:undated', role: 'aegis', text: restartText },
        { id: 'restart:2', role: 'aegis', text: restartText, timestamp: afterMidnight },
      ],
      TZ,
    );
    assert.equal(bridged.length, 2, 'undated duplicate must not bridge a midnight boundary');
  });

  it('labels a collapsed group whose visible card is undated via its hidden dated anchor', () => {
    // Inverse of the bridge case: Jan5 normal message, Jan6 dated restart,
    // undated duplicate → the collapsed card's VISIBLE timestamp is undefined
    // but its hidden range is dated Jan6; the Jan6 divider must still appear.
    const restartText = '系统已重启 / System restarted';
    const jan5 = Date.UTC(2026, 0, 5, 20, 0); // Jan 5, 12:00 PST
    const jan6 = Date.UTC(2026, 0, 6, 20, 0); // Jan 6, 12:00 PST
    const collapsed = collapsedChatHomeMessages(
      [
        { id: 'owner:1', role: 'owner', text: '普通消息', timestamp: jan5 },
        { id: 'restart:1', role: 'aegis', text: restartText, timestamp: jan6 },
        { id: 'restart:undated', role: 'aegis', text: restartText },
      ],
      TZ,
    );
    assert.equal(collapsed.length, 2);
    assert.equal(collapsed[1].timestamp, undefined, 'precondition: visible card is undated');
    const labels = chatDateDividerLabels(collapsed, TZ, 'zh', Date.UTC(2026, 0, 10, 12, 0));
    assert.equal(Object.keys(labels).length, 2, 'both day groups labeled');
    assert.match(labels[collapsed[0].id], /1月5日/);
    assert.match(labels[collapsed[1].id], /1月6日/);
  });

  it('labels yesterday correctly across the DST spring-forward boundary', () => {
    // DST began 2026-03-08 02:00 in America/Los_Angeles (a 23h day).
    // now = 2026-03-09 00:30 PDT; a message on local 2026-03-08 must be 昨天 —
    // a naive now-24h subtraction lands on 03-07 and mislabels it.
    const now = Date.UTC(2026, 2, 9, 7, 30);
    const marchEighth = Date.UTC(2026, 2, 8, 20, 0); // 13:00 PDT, Mar 8
    const labels = chatDateDividerLabels([{ id: 'm', timestamp: marchEighth }], TZ, 'zh', now);
    assert.equal(labels.m, '昨天');
  });

  it('renders a divider pill per day boundary in the stream (zh)', () => {
    // The component reads the timezone from UserTimezoneContext (null in this
    // harness → runtime default), so use timestamps 32h apart: they land on
    // different calendar days in EVERY timezone (UTC±14), keeping the
    // boundary count deterministic without a provider.
    const t1 = Date.UTC(2026, 0, 5, 12, 0);
    const t2 = Date.UTC(2026, 0, 6, 20, 0);
    const html = renderWithLocale('zh', baseProps({
      messages: [
        { id: 'owner:1', role: 'owner', text: '第一天', timestamp: t1 },
        { id: 'aegis:1', role: 'aegis', text: '第一天回复', timestamp: t1 + 60_000 },
        { id: 'owner:2', role: 'owner', text: '第二天', timestamp: t2 },
      ],
    }));
    assert.equal((html.match(/vnext-chat-date-divider/g) || []).length, 2);
    assert.match(html, /role="separator"/);
  });

  it('renders no divider for an empty conversation', () => {
    const html = renderWithLocale('zh', baseProps({ messages: [] }));
    assert.doesNotMatch(html, /vnext-chat-date-divider/);
  });
});

describe('vNext Chat Home voice notes', () => {
  it('records into the composer row: red dot, elapsed timer, Cancel and Stop (zh)', () => {
    const html = renderWithLocale('zh', baseProps({
      voiceNote: { status: 'recording', elapsedLabel: '0:07' },
    }));
    assert.match(html, /class="vnext-chat-recording"/);
    assert.match(html, /class="vnext-chat-recording-dot"/);
    assert.match(html, /正在录音/);
    assert.match(html, /0:07/);
    assert.match(html, /class="vnext-chat-recording-cancel"[^>]*>取消/);
    assert.match(html, /class="vnext-chat-recording-stop"/);
    // Recording OWNS the row: the textarea and the send button step aside so a
    // stray tap cannot send an empty turn mid-recording.
    assert.doesNotMatch(html, /<textarea/);
    assert.doesNotMatch(html, /type="submit"/);
  });

  it('shows the transcribe wait with no Stop button once recording ended (zh)', () => {
    const html = renderWithLocale('zh', baseProps({ voiceNote: { status: 'uploading' } }));
    assert.match(html, /正在转写/);
    assert.match(html, /class="vnext-chat-recording-cancel"/);
    // Nothing left to stop — the audio is already on its way to the server.
    assert.doesNotMatch(html, /class="vnext-chat-recording-stop"/);
  });

  it('holds the server transcript in a pill and keeps Send live with empty text (zh)', () => {
    const html = renderWithLocale('zh', baseProps({
      value: '',
      voiceNote: { status: 'attached', transcript: '提醒我五点回电话', durationLabel: '0:06' },
    }));
    assert.match(html, /class="vnext-chat-attached-file is-voice"/);
    assert.match(html, /提醒我五点回电话/);
    assert.match(html, /0:06/);
    assert.match(html, /aria-label="移除语音消息"/);
    // The composer row is back, and Send is NOT disabled despite empty text.
    assert.match(html, /<textarea/);
    const submit = html.match(/<button type="submit"[^>]*>/);
    assert.ok(submit, 'submit button must render');
    assert.doesNotMatch(submit[0], /disabled/);
  });

  it('surfaces a voice failure as a visible error pill instead of doing nothing (zh)', () => {
    const html = renderWithLocale('zh', baseProps({
      voiceNote: { status: 'error', message: '无法使用麦克风。' },
    }));
    assert.match(html, /class="vnext-chat-attached-file is-error"[^>]*role="alert"/);
    assert.match(html, /无法使用麦克风。/);
  });

  it('keeps Send live for an image-only turn (the owner-reported silent drop)', () => {
    const html = render(baseProps({
      value: '',
      onAttachFile: noop,
      onClearAttachment: noop,
      pendingAttachment: {
        kind: 'image',
        dataUrl: 'data:image/png;base64,iVBORw0KGgo=',
        filename: 'receipt.png',
        mimeType: 'image/png',
      },
    }));
    const submit = html.match(/<button type="submit"[^>]*>/);
    assert.ok(submit, 'submit button must render');
    assert.doesNotMatch(submit[0], /disabled/);
  });

  it('renders a sent voice note as mic + transcript + duration, without repeating the text', () => {
    const html = renderWithLocale('zh', baseProps({
      messages: [{
        id: 'owner:1',
        role: 'owner',
        text: '提醒我五点回电话',
        timestamp: 1_700_000_000_000,
        attachment: { kind: 'voice', transcript: '提醒我五点回电话', durationSecs: 6.2 },
      }],
    }));
    assert.match(html, /class="vnext-chat-message-voice"/);
    assert.match(html, /aria-label="语音消息"/);
    assert.match(html, /class="vnext-chat-message-voice-duration">0:06/);
    // The transcript renders ONCE (inside the chip), not again as a paragraph.
    const occurrences = html.split('提醒我五点回电话').length - 1;
    assert.equal(occurrences, 1, `transcript must render once, saw ${occurrences}`);
  });

  it('renders a sent image turn as a thumbnail even with no caption', () => {
    const html = render(baseProps({
      messages: [{
        id: 'owner:2',
        role: 'owner',
        text: '',
        timestamp: 1_700_000_000_000,
        attachment: { kind: 'image', dataUrl: 'data:image/png;base64,iVBORw0KGgo=' },
      }],
    }));
    assert.match(html, /class="vnext-chat-message-attachment is-image"/);
    assert.match(html, /<img src="data:image\/png;base64,iVBORw0KGgo="/);
  });

  it('messageBodyText hides a duplicated voice transcript but keeps a typed caption', () => {
    assert.equal(
      messageBodyText({ text: 'hi', attachment: { kind: 'voice', transcript: 'hi', durationSecs: 1 } }),
      '',
    );
    assert.equal(
      messageBodyText({ text: 'look at this', attachment: { kind: 'voice', transcript: 'hi', durationSecs: 1 } }),
      'look at this',
    );
    assert.equal(messageBodyText({ text: 'plain turn' }), 'plain turn');
  });
});
