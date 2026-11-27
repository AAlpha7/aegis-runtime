import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

const settingsSource = readFileSync(new URL('../src/vnext/SettingsView.tsx', import.meta.url), 'utf8');

const statusModulePath = process.env.VNEXT_STATUS_RENDER_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-status-settings-render-test/StatusView.mjs', import.meta.url).href;
const settingsModulePath = process.env.VNEXT_SETTINGS_RENDER_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-status-settings-render-test/SettingsView.mjs', import.meta.url).href;
const localeContextModulePath = process.env.VNEXT_LOCALE_CONTEXT_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-status-settings-render-test/LocaleContext.mjs', import.meta.url).href;
// StatusView is bundled in its own esbuild graph, so its LocaleContext instance
// is distinct from the Settings bundle's. The provider that feeds StatusView
// must come from the SAME bundle as StatusView, otherwise the locale never
// propagates and StatusView stays on the zh default. Fall back to the shared
// path when the dedicated env is absent (e.g. ad-hoc runs).
const statusLocaleContextModulePath = process.env.VNEXT_STATUS_LOCALE_CONTEXT_MODULE
  ?? statusModulePath;
const { StatusView } = await import(statusModulePath);
const { SettingsView, applyDisplayLanguageChoice, LlmDegradedBanner, EmbedderDegradedBanner, SETTINGS_TIER_ORDER, settingsTierSections, settingsOwnerDisplayName } = await import(settingsModulePath);
const { LocaleProvider } = await import(localeContextModulePath);
const { LocaleProvider: StatusLocaleProvider } = await import(statusLocaleContextModulePath);

const noop = () => {};

function workItem(overrides = {}) {
  return {
    id: 'approval:calendar',
    lane: 'decide',
    kind: 'approval',
    source_kind: 'approval',
    source: 'calendar',
    source_id: 'calendar-access',
    source_event_id: null,
    title: 'Review calendar access',
    summary: 'Aegis needs your decision before Calendar can run.',
    why_it_matters: 'Calendar connection needs your call.',
    owner_visible_reason: 'Calendar connection needs your call.',
    state: 'waiting_user',
    status: 'waiting_user',
    urgency: 'high',
    sensitivity: 'S2',
    privacy_level: 'S2',
    created_at: '2026-05-05T00:00:00Z',
    updated_at: '2026-05-05T00:00:00Z',
    due_at: null,
    authority_basis: 'manual_review',
    authority: null,
    primary_action: null,
    contextual_actions: [],
    next_safe_action: 'Review calendar access.',
    blocked_by: '',
    detail_route: '',
    external_url_policy: '',
    route_reason: 'Owner decision required.',
    related_work_item_ids: [],
    related_action_ids: [],
    related_receipt_ids: [],
    receipt_expectation: 'Aegis cannot claim external work is done without a receipt.',
    ...overrides,
  };
}

function queue(overrides = {}) {
  return {
    generated_at: '2026-05-17T00:00:00Z',
    counts: { total: 2, mail: 0, calendar: 1, tasks: 1, receipts: 0 },
    sources: [
      { source: 'gmail', status: 'available', reason: 'ready' },
      { source: 'calendar', status: 'needs_attention', reason: 'refresh with Google' },
    ],
    items: [
      workItem(),
      workItem({
        id: 'task:42',
        lane: 'follow_up',
        kind: 'task',
        source_kind: 'task',
        source: 'tasks',
        source_id: '42',
        title: 'Insurance follow-up',
        summary: 'Waiting on insurer.',
        status: 'waiting',
        urgency: 'normal',
      }),
    ],
    ...overrides,
  };
}

function statusProjection(overrides = {}) {
  return {
    schema_version: 1,
    generated_at: '2026-05-17T00:00:00Z',
    health: 'attention',
    sections: [
      {
        label: 'Cost controls',
        health: 'quiet',
        owner_message: 'heartbeat and initiative internals should be rewritten for owner language.',
        owner_next_step: 'Resume heartbeat and initiative when ready.',
      },
    ],
    system_events: [
      {
        id: 'system:restart:1779491535',
        kind: 'restart',
        severity: 'quiet',
        title: 'System restarted',
        summary: 'Aegis restarted and returned to service.',
        occurred_at: 1779491535,
        window_start_at: 1779486963,
        count: 9,
        details: ['系统已重启 / System restarted 当前重要信息 活跃技能 / Active skills: 22'],
      },
    ],
    advanced_diagnostics_available: true,
    ...overrides,
  };
}

function settingsProjection(overrides = {}) {
  return {
    generated_at: '2026-05-17T00:00:00Z',
    signed_in_owner: 'owner@example.com',
    cost_saving_paused: true,
    connected_services: [
      { label: 'Mail', health: 'ready', owner_message: 'Mail is ready.', owner_next_step: null },
      { label: 'Calendar', health: 'attention', owner_message: 'Refresh with Google.', owner_next_step: 'Reconnect Calendar.' },
    ],
    security: [
      { label: 'Signed-in session', health: 'ready', owner_message: 'Google verifies your configured account.', owner_next_step: null },
      { label: 'Item access', health: 'ready', owner_message: 'Private bodies stay tied to the selected item.', owner_next_step: null },
    ],
    notification_preferences: [
      { label: 'Telegram', health: 'ready', owner_message: 'Telegram is for timely alerts and links back to the WebApp.', owner_next_step: null },
      { label: 'Cost controls', health: 'quiet', owner_message: 'heartbeat and initiative internals should be rewritten for owner language.', owner_next_step: 'Resume background work when ready.' },
    ],
    ...overrides,
  };
}

function debugJob(overrides = {}) {
  return {
    id: 'debug-job-latest',
    owner: 'owner@example.com',
    principal_type: 'web_console',
    kind: 'webapp_e2e_suite',
    status: 'passed',
    summary: 'WebApp E2E support suite passed.',
    log_text: 'PASSED: WebApp production public/auth smoke OK',
    exit_code: 0,
    lease_id: 'lease-debug-job',
    created_at_unix: 1779500000,
    started_at_unix: 1779500001,
    completed_at_unix: 1779500008,
    expires_at_unix: 1779503600,
    evidence: {
      schema_version: 1,
      verdict: 'deploy_ready',
      owner_summary: 'Production public/auth smoke and mail live smoke passed.',
      phases: [
        {
          id: 'production',
          label: 'Production public/auth smoke',
          status: 'passed',
          summary: 'Public WebApp and auth boundaries passed.',
        },
        {
          id: 'mail',
          label: 'Mail live smoke',
          status: 'passed',
          summary: 'Scoped debug grant created and cleaned up a smoke mailbox read.',
        },
      ],
    },
    ...overrides,
  };
}

function renderStatus(props = {}) {
  // `locale` (optional) wraps StatusView in a LocaleProvider so catalog-backed
  // strings render in the requested language. Omitting it renders with no
  // provider, whose context default is zh (matching the backend default).
  const { locale, ...componentProps } = props;
  const element = React.createElement(StatusView, {
    queue: queue(),
    projection: statusProjection(),
    onOpenWork: noop,
    onOpenSettings: noop,
    ...componentProps,
  });
  const tree = locale
    ? React.createElement(StatusLocaleProvider, { displayLanguage: locale }, element)
    : element;
  return renderToStaticMarkup(tree);
}

function renderSettings(props = {}) {
  // `locale` (optional) wraps SettingsView in a LocaleProvider so catalog-backed
  // strings render in the requested language. Omitting it renders with no
  // provider, whose context default is zh (matching the backend default).
  const { locale, ...componentProps } = props;
  const element = React.createElement(SettingsView, {
    email: 'owner@example.com',
    sources: queue().sources,
    projection: settingsProjection(),
    costSavingBusy: false,
    snapshotMessage: '',
    snapshotAvailable: true,
    snapshotBusy: false,
    webPushStatus: {
      schema_version: 1,
      configured: true,
      send_configured: true,
      subscribed: false,
      subscription_count: 0,
      public_key: 'B'.repeat(87),
      last_delivery: null,
      owner_message: 'Push notifications can be enabled on this device.',
      owner_next_step: null,
    },
    webPushDeviceSubscribed: false,
    webPushBusy: false,
    webPushMessage: '',
    debugLeases: [],
    avatarProfile: {
      aegisAvatarUrl: '/aegis-avatar.svg',
      ownerAvatarUrl: '',
    },
    onPauseCostSaving: noop,
    onResumeCostSaving: noop,
    onEnableWebPush: noop,
    onDisableWebPush: noop,
    onTestWebPush: noop,
    onCreateSnapshot: noop,
    onLogout: noop,
    onCreateDebugLease: noop,
    onRevokeDebugLease: noop,
    onRunDebugJob: noop,
    onSetAvatar: noop,
    onResetAvatar: noop,
    onSetDisplayLanguage: noop,
    onSetLlmProvider: noop,
    onSetLlmModel: noop,
    onSetPaidApiFallback: noop,
    ...componentProps,
  });
  const tree = locale
    ? React.createElement(LocaleProvider, { displayLanguage: locale }, element)
    : element;
  return renderToStaticMarkup(tree);
}

function degraded(overrides = {}) {
  return {
    reason: 'OpenRouter 返回 429',
    since: '2026-06-08T10:00:00Z',
    provider: 'openrouter',
    ...overrides,
  };
}

function renderBanner(props = {}) {
  const { locale, ...componentProps } = props;
  const element = React.createElement(LlmDegradedBanner, {
    degraded: degraded(),
    dismissedSince: null,
    onDismiss: noop,
    ...componentProps,
  });
  const tree = locale
    ? React.createElement(LocaleProvider, { displayLanguage: locale }, element)
    : element;
  return renderToStaticMarkup(tree);
}

function embedderDegraded(overrides = {}) {
  return {
    since: '2026-07-08T00:00:00Z',
    primary_model: 'text-embedding-primary',
    backup_model: 'text-embedding-backup',
    ...overrides,
  };
}

function renderEmbedderBanner(props = {}) {
  const { locale, ...componentProps } = props;
  const element = React.createElement(EmbedderDegradedBanner, {
    degraded: embedderDegraded(),
    dismissedSince: null,
    onDismiss: noop,
    ...componentProps,
  });
  const tree = locale
    ? React.createElement(LocaleProvider, { displayLanguage: locale }, element)
    : element;
  return renderToStaticMarkup(tree);
}

describe('vNext Status and Settings render', () => {
  it('renders Status as an owner-facing health summary in default zh chrome', () => {
    // No LocaleProvider → context default locale is zh, so migrated catalog
    // chrome renders in Chinese. Backend/model-authored projection data
    // (owner_message, system-event titles/summaries) is not migrated and stays
    // as supplied. The headline pluralizes from attentionMetric (2 here).
    const html = renderStatus();

    // Status is system health only (not a second "needs you" desk).
    assert.match(html, /系统健康/);
    assert.match(html, /要拍板的事请回对话或工作/);
    assert.ok((html.match(/要拍板/g) || []).length <= 1, '要拍板 must appear at most once');

    assert.doesNotMatch(html, /Aegis health/);
    // Decisions no longer inflate the attention metric — only Connections here.
    assert.match(html, /有 1 处系统需要关注/);
    assert.doesNotMatch(html, /有 2 件事需要你/);
    assert.match(html, /1 个需要关注/); // connections disclosure summary (sourceIssueCount = 1)
    assert.match(html, /需关注/); // health attention metric label
    assert.match(html, /<em aria-hidden="true">&gt;<\/em>/);
    assert.match(html, /我在安静处理的事/); // secretary quiet-ops summary
    assert.match(html, /Aegis 最近动静/); // soft system-events label
    assert.match(html, /System restarted/); // backend-authored event title (not migrated)
    assert.match(html, /9 个事件/); // "9 events ·" repeated-event line
    assert.match(html, /连接/); // "Connections"
    assert.match(html, /打开设置/); // "Open Settings"
    // owner_message cost-saving copy is now localized via the catalog (SP3);
    // default chrome is zh, so it renders the zh values.
    assert.match(html, /暂停计划检查、你批准的例程、跟进项以及其他自动检查。/);
    assert.match(html, /想让安琪恢复自动检查时，请到「设置」操作。/);
    assert.doesNotMatch(html, /heartbeat/i);
    assert.doesNotMatch(html, /probe/i);
    assert.doesNotMatch(html, /initiative/i);
    assert.doesNotMatch(html, /assistant event metadata/i);
    // No missing-key sentinel leaks in the default render.
    assert.doesNotMatch(html, /\[MISSING:status\./);
  });

  it('renders the same Status chrome in English when a LocaleProvider selects en', () => {
    const html = renderStatus({ locale: 'en' });

    assert.match(html, /System health/);
    assert.match(html, /Decisions belong in Chat or Work/);

    assert.match(html, /1 system area needs attention/);
    assert.match(html, /Attention/);
    assert.match(html, /I am handling quietly/);
    assert.match(html, /What Aegis noticed/);
    assert.match(html, /Open Settings/);
    // Display-mapped section label renders in English under en too.
    assert.match(html, /Connections/);
    assert.doesNotMatch(html, /\[MISSING:status\./);
  });

  it('keeps section-label identity logic working independent of display locale', () => {
    // A Cost-controls attention section must still be DETECTED (the comparison
    // uses the english label) and route to Settings, while the DISPLAY chip is
    // localized. Run under zh so a localized chip cannot accidentally satisfy
    // the english comparison.
    const html = renderStatus({
      queue: queue({
        sources: [{ source: 'gmail', status: 'available', reason: 'ready' }],
        items: [],
      }),
      projection: statusProjection({
        sections: [
          {
            label: 'Cost controls',
            health: 'attention',
            owner_message: 'Automatic checks paused to save spend.',
            owner_next_step: 'Resume when ready.',
          },
        ],
      }),
    });

    // Headline counts the one system-attention section → identity-derived metric works.
    assert.match(html, /有 1 处系统需要关注/);
    // The cost-controls comparison fired → action is Open Settings (localized).
    assert.match(html, /打开设置/);
    assert.doesNotMatch(html, /打开工作/);
    // The attention chip DISPLAY is localized to the Cost-controls mapping.
    assert.match(html, /花费与自动检查/);
  });

  it('renders Settings with security, cost, connection, snapshot, and logout controls (default zh)', () => {
    // No LocaleProvider → context default locale is zh, so migrated catalog
    // strings render in Chinese. Backend-authored projection data (owner@…,
    // 'Refresh with Google.', etc.) is not localized and stays as supplied.
    const html = renderSettings();

    assert.match(html, />你</);
    assert.doesNotMatch(html, /Control Aegis/);
    assert.match(html, /owner@example.com/);
    assert.match(html, /退出登录/);
    // P0: support/debug lives in the nested developer tier (after essentials + advanced).
    // Avatars are advanced; essentials (account/language/push/…) come first.
    assert.ok(html.indexOf('data-settings-tier="essentials"') < html.indexOf('data-settings-tier="advanced"'));
    assert.ok(html.indexOf('data-settings-tier="advanced"') < html.indexOf('data-settings-tier="developer"'));
    assert.ok(html.indexOf('后端 E2E') > html.indexOf('data-settings-tier="developer"'),
      'support debug controls should live inside the developer tier');
    assert.ok(html.indexOf('头像') > html.indexOf('data-settings-tier="advanced"'),
      'avatar settings should live inside the advanced tier');
    assert.match(html, /暂停自动检查/);
    assert.doesNotMatch(html, /Background paused/);
    assert.match(html, /自动检查已暂停，只在你要求时工作/);
    assert.doesNotMatch(html, /vnext-settings-summary/);
    assert.match(html, /连接/);
    assert.match(html, /Refresh with Google/);
    assert.match(html, /登录会话|已登录会话/);
    assert.match(html, /通知/);
    assert.doesNotMatch(html, /Telegram 照常提醒/);
    assert.match(html, /启用推送通知/);
    assert.match(html, /启用推送<\/span><\/button>/);
    assert.match(html, /头像/);
    assert.match(html, /选择图片/);
    assert.match(html, /\/aegis-avatar.svg/);
    assert.match(html, /后端 E2E 套件/);
    assert.match(html, /运行后端 E2E/);
    assert.match(html, /保存快照/);
    assert.match(html, /终端授权：无有效授权/);
    assert.match(html, /终端授权/);
    assert.match(html, /终端授权时长/);
    assert.match(html, />1h<\/button>/);
    assert.match(html, />8h<\/button>/);
    assert.match(html, />24h<\/button>/);
    assert.doesNotMatch(html, /15 min/);
    assert.match(html, /开启终端授权/);
    assert.doesNotMatch(html, /Revoke debug access/);
    assert.doesNotMatch(html, /Allow avatar debug/);
    assert.match(html, /当前没有有效的终端授权/);
    assert.doesNotMatch(html, /heartbeat/i);
    assert.doesNotMatch(html, /initiative/i);
    assert.doesNotMatch(html, /memory consolidation/i);
    assert.doesNotMatch(html, /X（Twitter）|即将推出/);
    assert.doesNotMatch(html, /ToolGate|ReAct|app-server/);
  });


  it('orders essentials before advanced and developer tiers; developer defaults collapsed', () => {
    const html = renderSettings();
    const essentials = html.indexOf('data-settings-tier="essentials"');
    const advanced = html.indexOf('data-settings-tier="advanced"');
    const developer = html.indexOf('data-settings-tier="developer"');
    assert.ok(essentials >= 0, 'essentials tier missing');
    assert.ok(advanced > essentials, 'advanced should follow essentials');
    assert.ok(developer > advanced, 'developer should follow advanced');

    // One collapsed 高级 <details> holds advanced + nested developer (debug + session).
    const advTagMatch = html.match(/<details data-settings-tier="advanced"[^>]*>/);
    assert.ok(advTagMatch, 'advanced details tag missing');
    assert.doesNotMatch(advTagMatch[0], /\sopen[\s>]/);
    const devTagMatch = html.match(/<details data-settings-tier="developer"[^>]*>/);
    assert.ok(devTagMatch, 'developer details tag missing');
    assert.doesNotMatch(devTagMatch[0], /\sopen[\s>]/);
    assert.ok(
      html.indexOf('data-settings-tier="developer"') > html.indexOf('data-settings-tier="advanced"'),
      'developer block should nest after advanced opens',
    );

    // Backend E2E content still in DOM (collapsed developer section).
    assert.match(html, /Backend E2E suite|后端 E2E/);
    assert.match(html, /data-settings-section="logout"/);

    // Pure helper exports
    assert.deepEqual(SETTINGS_TIER_ORDER, ['essentials', 'advanced', 'developer']);
    const sections = settingsTierSections();
    assert.ok(sections.essentials.includes('account'));
    assert.ok(sections.essentials.includes('services'));
    assert.ok(!sections.essentials.includes('summary'));
    assert.ok(sections.advanced.includes('appearance'));
    assert.ok(sections.advanced.includes('providers'));
    assert.ok(sections.developer.includes('support'));
    assert.ok(sections.developer.includes('security'));
    assert.ok(!sections.developer.includes('logout'));
  });

  it('prefers a human display name and never shows a raw numeric owner id', () => {
    assert.equal(settingsOwnerDisplayName({
      displayName: 'Evidence', signedInOwner: 'Telegram user 12345678', email: 'x@y.z', fallback: '你',
    }), 'Evidence');
    assert.equal(settingsOwnerDisplayName({
      displayName: '12345678', signedInOwner: 'Telegram user 99', email: 'owner@example.com', fallback: '你',
    }), 'owner@example.com');
    assert.equal(settingsOwnerDisplayName({
      displayName: null, signedInOwner: '42', email: '', fallback: '你',
    }), '你');
  });

  it('renders the same Settings controls in English when a LocaleProvider selects en', () => {
    const html = renderSettings({ locale: 'en' });

    assert.match(html, />You</);
    assert.match(html, /Log out/);
    assert.match(html, /Backend E2E suite/);
    assert.match(html, /Pause automatic checks/);
    assert.match(html, /Save support snapshot/);
    assert.match(html, /Automatic checks paused/);
    assert.match(html, /Connections|Calendar/);
    assert.match(html, /Sign-in|Signed-in session/);
    assert.match(html, /Notifications/);
    assert.doesNotMatch(html, /Telegram reminders stay on/);
    assert.match(html, /Enable push notifications/);
    assert.match(html, /Enable push<\/span><\/button>/);
    assert.match(html, /Avatars/);
    assert.match(html, /Choose image/);
    assert.match(html, /Run backend E2E/);
    assert.match(html, /Save snapshot/);
    assert.match(html, /Terminal grant: none active/);
    assert.match(html, /Terminal grant duration/);
    assert.match(html, /Open terminal grant/);
    assert.match(html, /No terminal grant is active/);
    // The migrated strings must not leak the missing-key sentinel in either locale.
    assert.doesNotMatch(html, /\[MISSING:settings\./);
  });

  it('does not leak the [MISSING:settings.*] sentinel in the default zh render', () => {
    const html = renderSettings();
    assert.doesNotMatch(html, /\[MISSING:settings\./);
  });

  it('renders a zh/en language control labelled via the i18n catalog', () => {
    const html = renderSettings();

    // The control offers both languages. Rendered outside a LocaleProvider the
    // default locale is zh, so the catalog-sourced labels render in Chinese.
    assert.match(html, /语言/);
    assert.match(html, />中文</);
    assert.match(html, />English</);
    // The language toggle now renders via VNextSegmented (mode="radio"): the
    // track is a radiogroup; the active language (default zh) is the checked
    // segment carrying is-selected.
    assert.match(html, /role="radiogroup"/);
    assert.match(html, /aria-checked="true">\s*<span class="vnext-seg-label">中文/);
  });

  it('persists the chosen language via onSetDisplayLanguage and the immediate locale setter', () => {
    // Choosing a language must (a) flip the in-memory locale for immediate UI
    // effect and (b) persist the setting so server-authored Channel-B strings
    // follow. renderToStaticMarkup cannot fire click handlers, so assert the
    // wiring against the component source (sibling pattern: source-text checks
    // used elsewhere for handler wiring).
    assert.match(settingsSource, /onSetDisplayLanguage/);
    assert.match(settingsSource, /setLocale\b/);
    // The toggle routes through applyDisplayLanguageChoice, handing it the chosen
    // value, the current locale (to revert to), the immediate setter, and the
    // persistence callback.
    assert.match(
      settingsSource,
      /applyDisplayLanguageChoice\(\{[\s\S]{0,200}next:\s*value[\s\S]{0,200}previous:\s*locale[\s\S]{0,200}persist:\s*onSetDisplayLanguage/,
      'language toggle should delegate to applyDisplayLanguageChoice with next/previous/persist',
    );
  });

  it('flips the locale optimistically and persists on success', async () => {
    const calls = [];
    const persisted = [];
    await applyDisplayLanguageChoice({
      next: 'en',
      previous: 'zh',
      setLocale: (value) => calls.push(value),
      persist: async (value) => { persisted.push(value); },
    });
    // Optimistic flip happened; no revert on success.
    assert.deepEqual(calls, ['en']);
    assert.deepEqual(persisted, ['en']);
  });

  it('reverts setLocale to the previous value when persistence fails, still surfacing the error', async () => {
    const calls = [];
    const persistError = new Error('persist failed');
    await assert.rejects(
      () => applyDisplayLanguageChoice({
        next: 'en',
        previous: 'zh',
        setLocale: (value) => calls.push(value),
        persist: async () => { throw persistError; },
      }),
      // The failure is re-thrown so the owner-facing error (surfaced by the
      // persist callback) is not swallowed.
      (error) => error === persistError,
    );
    // First the optimistic flip to the chosen locale, then a revert to the
    // previous locale so the chrome matches the unchanged backend setting.
    assert.deepEqual(calls, ['en', 'zh']);
  });

  it('distinguishes saved WebApp notification devices from this browser', () => {
    const html = renderSettings({
      webPushStatus: {
        schema_version: 1,
        configured: true,
        send_configured: true,
        subscribed: true,
        subscription_count: 1,
        public_key: 'B'.repeat(87),
        last_delivery: null,
        owner_message: 'One browser is already registered.',
        owner_next_step: null,
      },
      webPushDeviceSubscribed: false,
    });

    assert.match(html, /已在其他设备启用推送通知/);
    assert.match(html, /启用推送<\/span><\/button>/);
    assert.match(html, /发送测试/);
  });

  it('shows why Enable push is disabled when this environment cannot receive Web Push', () => {
    const zh = renderSettings({ locale: 'zh' });
    assert.match(zh, /启用推送<\/span><\/button>/);
    assert.match(zh, /Web 应用通知只能在浏览器里使用。/);
    const en = renderSettings({ locale: 'en' });
    assert.match(en, /Web Push is only available in a browser\./);
  });

  it('frames Web notification settings as push controls while keeping the timeline mandatory', () => {
    const html = renderSettings({
      webPushStatus: {
        schema_version: 1,
        configured: true,
        send_configured: true,
        subscribed: true,
        subscription_count: 1,
        public_key: 'B'.repeat(87),
        last_delivery: null,
        owner_message: null,
        owner_next_step: null,
      },
      webPushDeviceSubscribed: true,
      telegramRail: { state: 'reachable', since_unix: Math.floor(Date.now() / 1000) - 60, last_error: null, degraded: false },
    });

    assert.match(html, /已在此设备启用推送通知/);
    assert.match(html, /Telegram 照常提醒/);
    assert.match(html, /关闭推送<\/span><\/button>/);
    assert.doesNotMatch(html, /Turn off<\/span><\/button>/);
  });

  it('shows active terminal grants with visible revoke controls and token recovery guidance', () => {
    const html = renderSettings({
      debugLeases: [{
        id: 'lease-12345678',
        owner: 'owner@example.com',
        scope: 'general',
        reason: 'Owner granted temporary diagnostic access for WebApp debugging.',
        item_id: null,
        permissions: ['read_safe_snapshot'],
        created_at_unix: 1779400000,
        expires_at_unix: 1779428800,
      }],
    });

    assert.match(html, /终端授权：1 个有效授权/);
    // humanizeSettingsLabel('general') stays 'General' (a backend scope id), the
    // 'terminal grant' suffix is localized.
    assert.match(html, /General 终端授权/);
    assert.match(html, /Owner granted temporary diagnostic access/);
    assert.match(html, /令牌仅显示过一次且无法恢复/);
    assert.doesNotMatch(html, /开启终端授权/);
    assert.doesNotMatch(html, /终端授权时长/);
    assert.doesNotMatch(html, /Allow avatar debug/);
    assert.match(html, /撤销<\/span><\/button>/);
    assert.doesNotMatch(html, /当前没有有效的终端授权/);
  });

  it('keeps backend E2E evidence compact with latest-first history and hidden raw logs', () => {
    const html = renderSettings({
      debugJobs: [
        debugJob(),
        debugJob({
          id: 'debug-job-previous',
          completed_at_unix: 1779490000,
          summary: 'Previous run passed.',
          evidence: {
            schema_version: 1,
            verdict: 'deploy_ready',
            owner_summary: 'Previous production public/auth smoke passed.',
            phases: [],
          },
        }),
      ],
    });

    // Verdict label is migrated (zh default); phase/summary text is backend data.
    assert.match(html, /最新 E2E：可部署/);
    assert.match(html, /Production public\/auth smoke and mail live smoke passed/);
    assert.match(html, /Production public\/auth smoke/);
    assert.match(html, /历史 E2E 运行（1）/);
    assert.match(html, /Previous production public\/auth smoke passed/);
    assert.match(html, /显示原始日志/);
    assert.doesNotMatch(html, /PASSED: WebApp production public\/auth smoke OK/);
  });

  it('hides support snapshots when server diagnostics are disabled', () => {
    const html = renderSettings({ snapshotAvailable: false });

    assert.doesNotMatch(html, /Support snapshot/);
    assert.doesNotMatch(html, /Save support snapshot/);
    assert.doesNotMatch(html, /保存支持快照/);
    assert.match(html, />你</);
    assert.match(html, /退出登录/);
  });

  it('keeps account and cost controls independent from assistant work', () => {
    const html = renderSettings({ costSavingBusy: false, logoutBusy: false });

    assert.match(html, /退出登录<\/span><\/button>/);
    assert.match(html, /暂停自动检查/);
    assert.doesNotMatch(html, /正在退出/);
    // Logout is the destructive account action; when not busy it is not disabled.
    assert.match(html, /vnext-btn-destructive/);
    assert.doesNotMatch(html, /暂停自动检查[\s\S]{0,400}?type="checkbox"[^>]*disabled/);
  });

  it('uses a dedicated pending state for cost controls', () => {
    const html = renderSettings({ costSavingBusy: true });

    assert.match(html, /正在更新自动检查/);
    assert.match(html, /type="checkbox" disabled="" checked=""/);
  });

  it('routes cost-control attention to Settings instead of a generic Work path', () => {
    const html = renderStatus({
      queue: queue({
        sources: [{ source: 'gmail', status: 'available', reason: 'ready' }],
        items: [],
      }),
      projection: statusProjection({
        sections: [
          {
            label: 'Cost controls',
            health: 'attention',
            owner_message: 'heartbeat and initiative internals should be rewritten for owner language.',
            owner_next_step: 'Resume heartbeat and initiative when ready.',
          },
        ],
      }),
    });

    // Default zh chrome: chrome strings + the Cost-controls display chip render
    // in Chinese, but the english 'Cost controls' label still drives the
    // attention detection + Settings routing (identity untouched).
    assert.match(html, /有 1 处系统需要关注/);
    assert.match(html, /花费与自动检查/); // display-mapped Cost-controls chip (zh)
    assert.match(html, /打开设置/); // Open Settings (zh)
    assert.doesNotMatch(html, /打开工作/); // Open Work must NOT appear
    assert.doesNotMatch(html, /heartbeat/i);
    assert.doesNotMatch(html, /initiative/i);
  });

  it('uses fallback owner-language settings when projection data is absent', () => {
    const html = renderSettings({ projection: null });

    // The client-built fallback security/notification sections are now localized
    // (SP3); default chrome is zh, so they render the zh copy.
    assert.match(html, /Google 会在打开私密工作前先核验你配置的账户。/);
    assert.match(html, /私密正文、附件以及对外写入都与所选条目和操作绑定。/);
    assert.doesNotMatch(html, /Telegram 照常提醒/);
    // The fallback section labels localize too (the english label stays the identity).
    assert.match(html, /已登录会话/);
    assert.match(html, /条目访问/);
    // The cost-control title is migrated chrome → zh by default.
    assert.match(html, /暂停自动检查/);
    assert.match(html, /打开后暂停后台自动检查，只在你发消息时工作。/);
    assert.doesNotMatch(html, /Private access ready/);
    assert.doesNotMatch(html, /Aegis Runtime/);
  });

  it('sanitizes raw backend projection messages before rendering Settings and Status', () => {
    const settingsHtml = renderSettings({
      projection: settingsProjection({
        connected_services: [
          {
            label: 'Mail',
            health: 'attention',
            owner_message: 'gmail_reader FuelExhausted circuit open',
            owner_next_step: 'restart gmail_reader after credits recover',
          },
        ],
        security: [
          {
            label: 'Item access',
            health: 'attention',
            owner_message: 'data_access_gate mfa_required for protected action',
            owner_next_step: 'complete mfa_required flow',
          },
        ],
      }),
    });
    // Projection copy is now localized (SP3); default chrome is zh. The server
    // tokens are still stripped (doesNotMatch below), only the owner-facing copy changed.
    assert.match(settingsHtml, /模型额度已用尽。/);
    assert.match(settingsHtml, /补充模型额度后重试。/);
    assert.match(settingsHtml, /在安琪继续之前，需要你额外确认。/);
    assert.doesNotMatch(settingsHtml, /gmail_reader/i);
    assert.doesNotMatch(settingsHtml, /FuelExhausted/i);
    assert.doesNotMatch(settingsHtml, /data_access_gate/i);
    assert.doesNotMatch(settingsHtml, /mfa_required/i);

    const statusHtml = renderStatus({
      queue: queue({
        sources: [
          { source: 'calendar', status: 'permission_required', reason: 'calendar_reader consent_required' },
        ],
        items: [],
      }),
    });
    // sourceStatusView reason/nextStep now localize (SP3); default chrome is zh.
    // Server tokens still stripped (doesNotMatch below).
    assert.match(statusHtml, /Google 需要授权后，此来源才能打开私密详情。/);
    assert.match(statusHtml, /授予缺失的权限后，安琪才能完整使用此来源。/);
    assert.doesNotMatch(statusHtml, /calendar_reader/i);
    assert.doesNotMatch(statusHtml, /consent_required/i);
  });

  it('renders the LLM provider selector with all three options and a paid-API fallback toggle', () => {
    const html = renderSettings({
      locale: 'en',
      projection: settingsProjection({ llm_provider: 'codex_oauth', paid_api_fallback_enabled: true }),
    });
    // All three providers are offered as segmented values.
    assert.match(html, /data-seg-value="openrouter"/);
    assert.match(html, /data-seg-value="codex_oauth"/);
    assert.match(html, /data-seg-value="xai_subscription"/);
    // Readable, localized labels (en) — not raw enum slugs.
    assert.match(html, />OpenRouter</);
    assert.match(html, />Codex</);
    assert.match(html, />Grok</);
    // The selector reflects the projection's current provider.
    assert.match(html, /data-seg-value="codex_oauth"[^>]*aria-checked="true"|aria-checked="true"[^>]*data-seg-value="codex_oauth"/);
    // The paid-API fallback toggle renders checked from the projection.
    assert.match(html, /type="checkbox" checked/);
    assert.match(html, /Paid API fallback/);
  });

  it('defaults the provider selector to openrouter when the projection omits llm_provider', () => {
    const html = renderSettings({ locale: 'en', projection: settingsProjection() });
    // openrouter option exists AND is the selected one by default.
    assert.match(html, /data-seg-value="openrouter"[^>]*aria-checked="true"|aria-checked="true"[^>]*data-seg-value="openrouter"/);
    // Unchecked paid-API fallback when paid_api_fallback_enabled is absent.
    assert.match(html, /Paid API fallback[\s\S]{0,320}?<input type="checkbox"(?! checked)/);
  });

  it('renders web search on the Grok subscription and explains a missing sign-in', () => {
    const signedOut = renderSettings({
      locale: 'zh',
      projection: settingsProjection({
        web_search_provider: 'xai_subscription',
        web_search_model: 'grok-4.7',
        web_search_model_options: ['grok-4.7', 'grok-4.6', 'grok-4.5'],
      }),
      onSetWebSearchProvider: noop,
      onSetWebSearchModel: noop,
      grokStatus: {
        schema_version: 1,
        provider: 'xai_subscription',
        enabled: false,
        selected_provider: 'codex_oauth',
        active_for_chat: false,
        connected: false,
        pending_device_code: false,
        model: 'grok-4.7',
        foreground_only: true,
        daily_request_limit: 100,
        owner_message: '需要登录 Grok',
      },
    });
    assert.match(signedOut, /id="vnext-settings-search"/);
    assert.match(signedOut, /vnext-settings-search-provider-toggle[\s\S]*data-seg-value="xai_subscription"[^>]*aria-checked="true"/);
    assert.match(signedOut, /id="vnext-settings-search-model-select"[\s\S]*<option value="grok-4\.7" selected/);
    assert.match(signedOut, /data-web-search-reason="not-signed-in"/);
    assert.match(signedOut, /当前无法搜索/);
    assert.doesNotMatch(signedOut, /x-ai\/grok-4\.7:online/);

    const openrouter = renderSettings({
      locale: 'en',
      projection: settingsProjection({
        web_search_provider: 'openrouter',
        web_search_model: 'x-ai/grok-4.7:online',
        web_search_model_options: ['x-ai/grok-4.7:online'],
      }),
      onSetWebSearchProvider: noop,
      onSetWebSearchModel: noop,
      grokStatus: {
        schema_version: 1,
        provider: 'xai_subscription',
        enabled: false,
        selected_provider: 'openrouter',
        active_for_chat: false,
        connected: false,
        pending_device_code: false,
        model: 'grok-4.7',
        foreground_only: true,
        daily_request_limit: 100,
        owner_message: 'Sign in',
      },
    });
    assert.match(openrouter, /vnext-settings-search-provider-toggle[\s\S]*data-seg-value="openrouter"[^>]*aria-checked="true"/);
    assert.match(openrouter, /Grok subscription/);
    assert.match(openrouter, /<option value="x-ai\/grok-4\.7:online" selected/);
    assert.match(openrouter, /Selecting OpenRouter uses paid search/);
    assert.doesNotMatch(openrouter, /data-web-search-reason="not-signed-in"/);
  });

  it('renders subscription search without a misleading Grok model or secondary service', () => {
    for (const provider of ['auto', 'codex_oauth']) {
      const html = renderSettings({locale: 'en', projection: settingsProjection({
        web_search_provider: provider, web_search_model: '', web_search_model_options: [],
      }), onSetWebSearchProvider: noop});
      assert.match(html, new RegExp(`data-seg-value="${provider}"[^>]*aria-checked="true"`));
      assert.doesNotMatch(html, /id="vnext-settings-search-model-select"/);
      assert.match(html, /without switching services/);
    }
  });

  it('preserves an unsupported legacy search setting as a visible repair choice', () => {
    const html = renderSettings({locale: 'en', projection: settingsProjection({
      web_search_provider: 'duckduckgo', web_search_model: '', web_search_model_options: [],
    }), onSetWebSearchProvider: noop});
    assert.match(html, /data-seg-value="unsupported"[^>]*aria-checked="true"/);
    assert.match(html, /saved search service is no longer supported/);
    assert.doesNotMatch(html, /data-seg-value="duckduckgo"/);
    assert.doesNotMatch(html, /data-seg-value="auto"[^>]*aria-checked="true"/);
  });

  it('wires the provider selector + fallback toggle to their setter props', () => {
    // Static-source wiring assertions (mirrors the display-language wiring test):
    // renderToStaticMarkup cannot dispatch onChange, so verify the handlers
    // forward to the setter props at the source level.
    assert.match(settingsSource, /await onSetLlmProvider\?\.\(llmProvider, currentLlmModel\)/);
    assert.match(settingsSource, /onSetPaidApiFallback\?\.\(checked\)/);
    // Provider switch must drop the model filter and force-reload the catalog
    // so the default-option label and option list follow the new provider.
    assert.match(settingsSource, /async function ensureCatalog\(force\?: boolean\)/);
    assert.match(settingsSource, /setModelFilter\(''\)/);
    assert.match(settingsSource, /void ensureCatalog\(true\)/);
  });

  it('renders the LLM model select from the provider catalog (zh)', () => {
    const html = renderSettings({
      locale: 'zh',
      projection: settingsProjection({ llm_provider: 'openrouter', llm_model: 'deepseek/deepseek-v4-pro' }),
      onSetLlmModel: noop,
      onSetFallbackModel: noop,
      modelCatalog: {
        schema_version: 1,
        providers: [
          {
            id: 'openrouter',
            connected: true,
            source: 'openrouter_models_api',
            models: [
              { id: 'deepseek/deepseek-v4-pro', display_name: 'DeepSeek V4 Pro' },
              { id: 'x-ai/grok-4', display_name: 'Grok 4' },
            ],
          },
        ],
        current: { provider: 'openrouter', model: 'deepseek/deepseek-v4-pro' },
        fallback: { provider: null, model: '' },
      },
    });
    assert.match(html, /class="[^"]*\bvnext-model-select\b[^"]*"/);
    assert.match(html, /value="deepseek\/deepseek-v4-pro"/);
    assert.match(html, /DeepSeek V4 Pro/);
    assert.match(html, />模型</);
    // The select carries its own accessible name (label linked by id), so the
    // filter <input> that appears for long catalogs cannot steal it.
    assert.match(html, /<label id="vnext-settings-model-label" for="vnext-settings-model-select">模型<\/label>/);
    assert.match(html, /<select id="vnext-settings-model-select" class="[^"]*\bvnext-model-select\b[^"]*" aria-labelledby="vnext-settings-model-label"/);
    assert.match(html, /没有备用模型/);
    assert.doesNotMatch(html, /gemini/);
    assert.doesNotMatch(html, /class="vnext-model-input"/);
    assert.doesNotMatch(html, /常用模型/);
  });

  it('disables the model select and shows an empty-list line when the catalog has no models', () => {
    const html = renderSettings({
      locale: 'zh',
      projection: settingsProjection({ llm_provider: 'openrouter' }),
      onSetLlmModel: noop,
      modelCatalog: {
        schema_version: 1,
        providers: [{ id: 'openrouter', connected: true, source: 'unavailable', models: [] }],
        current: { provider: 'openrouter', model: '' },
        fallback: { provider: null, model: '' },
      },
    });
    assert.match(html, /class="[^"]*\bvnext-model-select\b[^"]*"/);
    assert.match(html, /disabled/);
    assert.match(html, /这个服务商还没拿到模型列表/);
  });

  it('wires the model select to the setter prop and no longer has presets', () => {
    assert.match(settingsSource, /onChange=\{\(event\) => submitLlmModel\(event.currentTarget.value\)\}/);
    assert.match(settingsSource, /function submitLlmModel\(value: string\)/);
    assert.match(settingsSource, /setModelDraft\(value.trim\(\)\)/);
    assert.doesNotMatch(settingsSource, /LLM_MODEL_PRESETS/);
    assert.doesNotMatch(settingsSource, /gemini/);
  });

  it('renders the backup summary from the catalog fallback and offers connected providers', () => {
    const html = renderSettings({
      locale: 'zh',
      projection: settingsProjection({ llm_provider: 'openrouter', llm_model: 'deepseek/deepseek-v4-pro' }),
      onSetLlmModel: noop,
      onSetFallbackModel: noop,
      modelCatalog: {
        schema_version: 1,
        providers: [
          {
            id: 'openrouter',
            connected: true,
            source: 'openrouter_models_api',
            models: [{ id: 'deepseek/deepseek-v4-pro', display_name: 'DeepSeek V4 Pro' }],
          },
          {
            id: 'xai_subscription',
            connected: true,
            source: 'xai_models_api',
            models: [{ id: 'grok-4.6', display_name: 'Grok 4.6' }],
          },
        ],
        current: { provider: 'openrouter', model: 'deepseek/deepseek-v4-pro' },
        fallback: { provider: 'xai_subscription', model: 'grok-4.6' },
      },
    });
    assert.match(html, /备用模型/);
    assert.match(html, /主模型失败时用：Grok · grok-4\.6/);
    assert.match(html, /value="xai_subscription"/);
    assert.match(html, /value="grok-4\.6"/);
    assert.match(html, />不用备用</);
    assert.match(html, />选择模型</);
    // Backup empty option must not reuse the primary "服务商默认" label.
    assert.match(html, /vnext-fallback-selects[\s\S]*?>选择模型</);
  });

  it('wires the backup selects to onSetFallbackModel and keeps the summary on draft state', () => {
    assert.match(settingsSource, /function submitFallbackModel\(provider: string, model: string\)/);
    assert.match(settingsSource, /await validateCallSetting\(\(\) => onSetFallbackModel\(provider \? trimmed : ''\), !!provider\)/);
    assert.match(settingsSource, /submitFallbackModel\(fallbackProvider, nextModel\)/);
    assert.match(settingsSource, /const fallbackSummaryProvider = catalog\?\.fallback.provider \|\| '';/);
    assert.match(settingsSource, /const fallbackSummaryModel = catalog\?\.fallback.model \|\| '';/);
    assert.doesNotMatch(settingsSource, /catalog\?\.fallback\.model \|\| fallbackModelDraft/);
    assert.doesNotMatch(settingsSource, /catalog\?\.fallback\.provider \|\| fallbackProvider/);
    assert.match(settingsSource, /tr\('settings\.fallback\.chooseModel'\)/);
  });

  // The three titles below predate the catalog work. Hercules' test-integrity
  // gate fails closed on a removed or renamed title, so they stay and now
  // assert the catalog-driven behaviour that replaced the free-text input.
  it('renders the LLM model free-text input pre-filled with the current model (zh) plus presets', () => {
    const html = renderSettings({
      locale: 'zh',
      projection: settingsProjection({ llm_provider: 'openrouter', llm_model: 'deepseek/deepseek-v4-pro' }),
      onSetLlmModel: noop,
      onSetFallbackModel: noop,
      modelCatalog: {
        schema_version: 1,
        providers: [{ id: 'openrouter', connected: true, source: 'openrouter_models_api', models: [{ id: 'deepseek/deepseek-v4-pro', display_name: 'DeepSeek V4 Pro' }] }],
        current: { provider: 'openrouter', model: 'deepseek/deepseek-v4-pro' },
        fallback: { provider: null, model: '' },
      },
    });
    // The free-text input and presets are gone; the current model is selected from the catalog.
    assert.doesNotMatch(html, /class="vnext-model-input"/);
    assert.doesNotMatch(html, /常用模型/);
    assert.match(html, /class="[^"]*\bvnext-model-select\b[^"]*"/);
    assert.match(html, /value="deepseek\/deepseek-v4-pro"/);
  });

  it('renders an empty model input when the projection omits llm_model', () => {
    const html = renderSettings({
      locale: 'zh',
      projection: settingsProjection({ llm_provider: 'openrouter' }),
      onSetLlmModel: noop,
      modelCatalog: {
        schema_version: 1,
        providers: [{ id: 'openrouter', connected: true, source: 'openrouter_models_api', models: [{ id: 'deepseek/deepseek-v4-pro', display_name: 'DeepSeek V4 Pro' }] }],
        current: { provider: 'openrouter', model: '' },
        fallback: { provider: null, model: '' },
      },
    });
    // No model chosen: the select renders without a selected catalog value.
    assert.match(html, /class="[^"]*\bvnext-model-select\b[^"]*"/);
    assert.doesNotMatch(html, /value="deepseek\/deepseek-v4-pro" selected/);
  });

  it('wires the model input + presets to the setter prop', () => {
    // Presets are gone; the select's onChange is what reaches the setter now.
    assert.match(settingsSource, /onChange=\{\(event\) => submitLlmModel\(event.currentTarget.value\)\}/);
    assert.match(settingsSource, /await onSetLlmProvider\?\.\(llmProvider, currentLlmModel\)/);
    assert.doesNotMatch(settingsSource, /LLM_MODEL_PRESETS/);
  });

  it('labels the provider-default option from the selected provider, not a stale effective_model', () => {
    // After switching OpenRouter → Grok, catalog.current can still name the
    // previous provider's effective gpt id until a force-reload lands.
    const html = renderSettings({
      locale: 'zh',
      projection: settingsProjection({ llm_provider: 'xai_subscription', llm_model: '' }),
      onSetLlmModel: noop,
      modelCatalog: {
        schema_version: 1,
        providers: [
          {
            id: 'openrouter',
            connected: true,
            source: 'openrouter_models_api',
            models: [{ id: 'gpt-5.6-sol', display_name: 'GPT-5.6 Sol' }],
            default_model: 'gpt-5.6-sol',
          },
          {
            id: 'xai_subscription',
            connected: true,
            source: 'xai_models_api',
            models: [{ id: 'grok-4.6', display_name: 'Grok 4.6' }],
            default_model: 'grok-4.6',
          },
        ],
        current: {
          provider: 'openrouter',
          model: 'gpt-5.6-sol',
          effective_model: 'gpt-5.6-sol',
        },
        fallback: { provider: null, model: '' },
      },
    });
    assert.match(html, /服务商默认 · grok-4\.6/);
    assert.doesNotMatch(html, /服务商默认 · gpt-5\.6-sol/);
    assert.match(html, /Grok 4\.6/);
    assert.doesNotMatch(html, /GPT-5\.6 Sol/);
  });

  it('labels the provider-default option with the effective model when the catalog gives one', () => {
    const named = renderSettings({
      locale: 'zh',
      projection: settingsProjection({ llm_provider: 'openrouter', llm_model: '' }),
      onSetLlmModel: noop,
      modelCatalog: {
        schema_version: 1,
        providers: [{
          id: 'openrouter',
          connected: true,
          source: 'openrouter_models_api',
          models: [{ id: 'deepseek/deepseek-v4-pro', display_name: 'DeepSeek V4 Pro' }],
        }],
        current: {
          provider: 'openrouter',
          model: '',
          effective_model: 'deepseek/deepseek-v4-pro',
        },
        fallback: { provider: null, model: '' },
      },
    });
    assert.match(named, /服务商默认 · deepseek\/deepseek-v4-pro/);

    const plain = renderSettings({
      locale: 'zh',
      projection: settingsProjection({ llm_provider: 'openrouter', llm_model: '' }),
      onSetLlmModel: noop,
      modelCatalog: {
        schema_version: 1,
        providers: [{
          id: 'openrouter',
          connected: true,
          source: 'openrouter_models_api',
          models: [{ id: 'deepseek/deepseek-v4-pro', display_name: 'DeepSeek V4 Pro' }],
        }],
        current: { provider: 'openrouter', model: '' },
        fallback: { provider: null, model: '' },
      },
    });
    assert.match(plain, />服务商默认</);
    assert.doesNotMatch(plain, /服务商默认 ·/);
  });

  it('every select in the settings view carries the shared select class', () => {
    const html = renderSettings({
      locale: 'zh',
      projection: settingsProjection({ llm_provider: 'openrouter' }),
      onSetLlmModel: noop,
      onSetFallbackModel: noop,
      modelCatalog: {
        schema_version: 1,
        providers: [{
          id: 'openrouter',
          connected: true,
          source: 'openrouter_models_api',
          models: [{ id: 'deepseek/deepseek-v4-pro', display_name: 'DeepSeek V4 Pro' }],
        }],
        current: { provider: 'openrouter', model: '' },
        fallback: { provider: null, model: '' },
      },
      snapshotAvailable: true,
      advancedDiagnosticsAvailable: true,
    });
    const selects = html.match(/<select\b[^>]*>/g) || [];
    assert.ok(selects.length >= 4, `expected ≥4 selects, got ${selects.length}`);
    for (const tag of selects) {
      assert.match(tag, /\bvnext-select\b/, `missing vnext-select on ${tag}`);
    }
  });

  it('segmented controls render a visible caption', () => {
    const html = renderSettings({
      locale: 'zh',
      projection: settingsProjection({ llm_provider: 'codex_oauth' }),
      onSetCodexModel: noop,
      codexStatus: {
        schema_version: 1,
        provider: 'codex_oauth',
        enabled: true,
        selected_provider: 'codex_oauth',
        active_for_chat: true,
        connected: true,
        binary_available: true,
        model: 'codex-default',
        reasoning_effort: 'medium',
        foreground_only: true,
        daily_request_limit: 100,
        owner_message: '',
        model_options_source: 'test',
        tool_calls_supported: true,
        tool_routing_policy: 'default',
        cli_binary: 'codex',
        cli_home: '',
        cli_workdir: '',
        model_options: [{
          slug: 'codex-default',
          display_name: 'Codex default',
          default_reasoning_level: 'medium',
          supported_reasoning_levels: ['low', 'medium', 'high', 'xhigh'],
        }],
      },
    });
    assert.match(html, /class="vnext-segmented-label"[^>]*>界面语言</);
    assert.match(html, /class="vnext-segmented-label"[^>]*>服务商</);
    assert.match(html, /class="vnext-segmented-label"[^>]*>主模型思考深度</);
    assert.match(html, /只对主模型 Codex 生效，备用模型不受影响；越高越仔细，也更慢、更花钱。/);
  });

  it('renders the LLM-degraded banner with the reason in zh and en, and switches the dismiss key', () => {
    const zh = renderBanner({ locale: 'zh', degraded: degraded({ reason: 'OpenRouter 返回 429' }) });
    assert.match(zh, /AI 服务暂时不可用/);
    assert.match(zh, /OpenRouter 返回 429/);
    assert.match(zh, /可在设置里切换/);
    assert.match(zh, /忽略/); // dismiss button

    const en = renderBanner({ locale: 'en', degraded: degraded({ reason: 'OpenRouter returned 429' }) });
    assert.match(en, /AI service temporarily unavailable/);
    assert.match(en, /OpenRouter returned 429/);
    assert.match(en, /switch provider in Settings/);
    assert.match(en, /Dismiss/);
  });

  it('hides the banner when healthy (degraded null/undefined)', () => {
    assert.equal(renderBanner({ degraded: null }), '');
    assert.equal(renderBanner({ degraded: undefined }), '');
  });

  it('hides the banner once the owner dismissed THIS outage, but re-shows on a new since', () => {
    const since = '2026-06-08T10:00:00Z';
    // Dismissed the current outage → banner stays hidden.
    const dismissed = renderBanner({ degraded: degraded({ since }), dismissedSince: since });
    assert.equal(dismissed, '');

    // A NEW outage carries a different `since` → banner RE-SHOWS even though the
    // previous one was dismissed.
    const newOutage = renderBanner({
      locale: 'en',
      degraded: degraded({ since: '2026-06-08T12:30:00Z', reason: 'xAI quota exhausted' }),
      dismissedSince: since,
    });
    assert.match(newOutage, /AI service temporarily unavailable/);
    assert.match(newOutage, /xAI quota exhausted/);
  });

  it('wires the dismiss button to onDismiss with the current outage since', () => {
    // renderToStaticMarkup discards event handlers, so assert the wiring at the
    // source level: the dismiss button forwards `degraded.since` to onDismiss.
    const bannerSource = readFileSync(new URL('../src/vnext/LlmDegradedBanner.tsx', import.meta.url), 'utf8');
    assert.match(bannerSource, /onClick=\{\(\) => onDismiss\(degraded\.since\)\}/);
    // And the visibility guard is keyed to `since` (dismiss + re-show contract).
    assert.match(bannerSource, /dismissedSince === degraded\.since/);
  });

  it('renders the embedder-degraded banner: backup variant vs unreachable variant, zh and en', () => {
    // With a backup model → the "serving from backup, dedup paused" variant.
    const zhBackup = renderEmbedderBanner({ locale: 'zh' });
    assert.match(zhBackup, /嵌入服务降级/);
    assert.match(zhBackup, /text-embedding-primary/);
    assert.match(zhBackup, /text-embedding-backup/);
    assert.match(zhBackup, /语义去重暂停/);
    assert.match(zhBackup, /忽略/);

    const enBackup = renderEmbedderBanner({ locale: 'en' });
    assert.match(enBackup, /Embedding service degraded/);
    assert.match(enBackup, /serving from backup/);
    assert.match(enBackup, /text-embedding-backup/);
    assert.match(enBackup, /Semantic dedup is paused/);
    assert.match(enBackup, /Dismiss/);

    // Without a backup model → the "unavailable" variant, and the backup
    // model name must not leak in from anywhere.
    const zhNoBackup = renderEmbedderBanner({
      locale: 'zh',
      degraded: embedderDegraded({ backup_model: null }),
    });
    assert.match(zhNoBackup, /无备用模型/);
    assert.doesNotMatch(zhNoBackup, /text-embedding-backup/);

    const enNoBackup = renderEmbedderBanner({
      locale: 'en',
      degraded: embedderDegraded({ backup_model: null }),
    });
    assert.match(enNoBackup, /no backup is configured/);
    assert.doesNotMatch(enNoBackup, /text-embedding-backup/);
  });

  it('renders NOTHING for the embedder banner when healthy (advisory honesty)', () => {
    assert.equal(renderEmbedderBanner({ degraded: null }), '');
    assert.equal(renderEmbedderBanner({ degraded: undefined }), '');
  });

  it('embedder banner: hides once dismissed for THIS outage, re-shows on a new since', () => {
    const since = '2026-07-08T00:00:00Z';
    const dismissed = renderEmbedderBanner({
      degraded: embedderDegraded({ since }),
      dismissedSince: since,
    });
    assert.equal(dismissed, '');

    const newOutage = renderEmbedderBanner({
      locale: 'en',
      degraded: embedderDegraded({ since: '2026-07-08T09:00:00Z' }),
      dismissedSince: since,
    });
    assert.match(newOutage, /Embedding service degraded/);
  });

  it('embedder banner wires dismiss to onDismiss(degraded.since) at the source level', () => {
    const bannerSource = readFileSync(new URL('../src/vnext/EmbedderDegradedBanner.tsx', import.meta.url), 'utf8');
    assert.match(bannerSource, /onClick=\{\(\) => onDismiss\(degraded\.since\)\}/);
    assert.match(bannerSource, /dismissedSince === degraded\.since/);
    // Shares the styled llm-degraded CSS surface (no unstyled-component trap).
    assert.match(bannerSource, /vnext-llm-degraded-banner/);
  });

  it('renders the advanced and developer tiers as disclosure rows with their hints', () => {
    const html = renderSettings();
    assert.match(html, /data-settings-tier="advanced"[^>]*class="[^"]*\bvnext-disclosure\b/);
    assert.match(html, /data-settings-tier="developer"[^>]*class="[^"]*\bvnext-disclosure\b/);
    assert.match(html, /class="vnext-disclosure-summary"/);
    assert.match(html, /高级/);
    assert.match(html, /AI、花费与头像/);
    assert.match(html, /开发者/);
    assert.match(html, /快照、E2E 与登录会话/);
    const en = renderSettings({ locale: 'en' });
    assert.match(en, /AI, spending, and avatars/);
    assert.match(en, /Snapshots, E2E, and sign-in/);
  });

  it('names the reasoning control as the primary model thinking depth', () => {
    const html = renderSettings({
      locale: 'zh',
      projection: settingsProjection({ llm_provider: 'codex_oauth' }),
      onSetCodexModel: noop,
      codexStatus: {
        schema_version: 1,
        provider: 'codex_oauth',
        enabled: true,
        selected_provider: 'codex_oauth',
        active_for_chat: true,
        connected: true,
        binary_available: true,
        model: 'codex-default',
        reasoning_effort: 'medium',
        foreground_only: true,
        daily_request_limit: 100,
        owner_message: '',
        model_options_source: 'test',
        tool_calls_supported: true,
        tool_routing_policy: 'default',
        cli_binary: 'codex',
        cli_home: '',
        cli_workdir: '',
        model_options: [{
          slug: 'codex-default',
          display_name: 'Codex default',
          default_reasoning_level: 'medium',
          supported_reasoning_levels: ['low', 'medium', 'high', 'xhigh'],
        }],
      },
    });
    assert.match(html, /主模型思考深度/);
    assert.match(html, /只对主模型 Codex 生效，备用模型不受影响；越高越仔细，也更慢、更花钱。/);
    const en = renderSettings({
      locale: 'en',
      projection: settingsProjection({ llm_provider: 'codex_oauth' }),
      onSetCodexModel: noop,
      codexStatus: {
        schema_version: 1,
        provider: 'codex_oauth',
        enabled: true,
        selected_provider: 'codex_oauth',
        active_for_chat: true,
        connected: true,
        binary_available: true,
        model: 'codex-default',
        reasoning_effort: 'medium',
        foreground_only: true,
        daily_request_limit: 100,
        owner_message: '',
        model_options_source: 'test',
        tool_calls_supported: true,
        tool_routing_policy: 'default',
        cli_binary: 'codex',
        cli_home: '',
        cli_workdir: '',
        model_options: [{
          slug: 'codex-default',
          display_name: 'Codex default',
          default_reasoning_level: 'medium',
          supported_reasoning_levels: ['low', 'medium', 'high', 'xhigh'],
        }],
      },
    });
    assert.match(en, /Primary model thinking depth/);
    assert.match(en, /Applies only to the primary Codex model, not the backup/);
  });
});

// ── Runtime rails on Status/Settings (incident 2026-09-15: a revoked Telegram
// token was invisible in the console for hours) ──────────────────────────────
describe('Status renders the Telegram rail and the nightly verdict', () => {
  const nowUnix = Math.floor(Date.now() / 1000);
  const invalidToken = { state: 'invalid_token', since_unix: nowUnix - 120, last_error: 'A Telegram\'s error: Invalid bot token', degraded: true };

  it('shows the Telegram row in the Connections list with the invalid-token reason (zh + en) and counts it as attention', () => {
    const zh = renderStatus({ locale: 'zh', projection: statusProjection({ telegram_rail: invalidToken }) });
    assert.match(zh, /data-telegram-rail-row="true"/);
    assert.match(zh, /class="vnext-status-row vnext-status-row-telegram warn"/);
    assert.match(zh, /Telegram 通知/);
    assert.match(zh, /token 已失效/);
    assert.match(zh, /TELOXIDE_TOKEN/);
    // The Connections disclosure counts the rail exactly like an unhealthy
    // source would: the fixture queue already has one (日历 needs attention),
    // so the dead bot makes it two, and the Connections metric counts the
    // rail as a connection (2 sources + Telegram = 3).
    assert.match(zh, /<em class="vnext-disclosure-hint">2 个需要关注<\/em>/);
    assert.match(zh, /<span>数据来源<\/span><strong>3<\/strong>/);
    // Negative control: without the rail the same fixture reports one issue
    // over two connections — proves the increments come from the rail.
    const withoutRail = renderStatus({ locale: 'zh' });
    assert.match(withoutRail, /<em class="vnext-disclosure-hint">1 个需要关注<\/em>/);
    assert.match(withoutRail, /<span>数据来源<\/span><strong>2<\/strong>/);

    const en = renderStatus({ locale: 'en', projection: statusProjection({ telegram_rail: invalidToken }) });
    assert.match(en, /Telegram notifications/);
    assert.match(en, /Token invalid/);
    assert.match(en, /Set a new TELOXIDE_TOKEN in \.env and restart Aegis\./);
  });

  it('renders a healthy Telegram row without warn and no row at all when the server does not report it', () => {
    const reachable = renderStatus({
      locale: 'zh',
      projection: statusProjection({ telegram_rail: { state: 'reachable', since_unix: nowUnix - 10, last_error: null, degraded: false } }),
    });
    assert.match(reachable, /data-telegram-rail-row="true"/);
    assert.doesNotMatch(reachable, /vnext-status-row-telegram warn/);
    assert.doesNotMatch(reachable, /TELOXIDE_TOKEN/);
    assert.match(reachable, /Telegram 通知/);

    // Negative control: an old server (no field) renders exactly the old page.
    const absent = renderStatus({ locale: 'zh' });
    assert.doesNotMatch(absent, /data-telegram-rail-row/);
    assert.doesNotMatch(absent, /Telegram 通知/);
  });

  it('a fresh unreachable is a blip (no warn); a long one warns with the elapsed minutes', () => {
    const blip = renderStatus({
      locale: 'zh',
      projection: statusProjection({ telegram_rail: { state: 'unreachable', since_unix: nowUnix - 30, last_error: 'dns', degraded: false } }),
    });
    assert.match(blip, /连不上/);
    assert.doesNotMatch(blip, /vnext-status-row-telegram warn/);

    const long = renderStatus({
      locale: 'en',
      projection: statusProjection({ telegram_rail: { state: 'unreachable', since_unix: nowUnix - 15 * 60, last_error: 'dns', degraded: true } }),
    });
    assert.match(long, /vnext-status-row-telegram warn/);
    assert.match(long, /Unreachable/);
    assert.match(long, /For 15 minutes now; Aegis keeps retrying\./);
  });

  it('shows one muted nightly line for a RED verdict (zh first) and nothing for GREEN or absent', () => {
    const red = { verdict: 'RED', sha: '02c0d462', finished_at_unix: nowUnix - 3 * 3600 - 60, failed_steps: ['cargo test'], report: null };
    const zh = renderStatus({ locale: 'zh', projection: statusProjection({ nightly: red }) });
    assert.match(zh, /class="vnext-muted vnext-status-nightly" data-nightly-line="true"/);
    assert.match(zh, /夜间测试 RED（02c0d462，3 小时前）/);
    // Muted line only — never the banner surface.
    assert.doesNotMatch(zh, /vnext-llm-degraded-banner/);

    const en = renderStatus({ locale: 'en', projection: statusProjection({ nightly: red }) });
    assert.match(en, /Nightly test run RED \(02c0d462, 3 hours ago\)/);

    const browserRed = renderStatus({
      locale: 'zh',
      projection: statusProjection({ nightly: { ...red, verdict: 'BROWSER_RED', finished_at_unix: nowUnix - 600 } }),
    });
    assert.match(browserRed, /夜间测试 BROWSER_RED（02c0d462，不到 1 小时前）/);

    // Negative controls: green and absent render no line.
    assert.doesNotMatch(renderStatus({ locale: 'zh', projection: statusProjection({ nightly: { ...red, verdict: 'GREEN' } }) }), /data-nightly-line/);
    assert.doesNotMatch(renderStatus({ locale: 'zh', projection: statusProjection({ nightly: null }) }), /data-nightly-line/);
    assert.doesNotMatch(renderStatus({ locale: 'zh' }), /data-nightly-line/);
  });
});

describe('Settings notifications note follows the Telegram rail', () => {
  const nowUnix = Math.floor(Date.now() / 1000);

  it('replaces "Telegram reminders stay on" with the invalid-token reason while the rail is down', () => {
    const zh = renderSettings({
      locale: 'zh',
      telegramRail: { state: 'invalid_token', since_unix: nowUnix - 60, last_error: 'Invalid bot token', degraded: true },
    });
    assert.match(zh, /class="vnext-settings-telegram-note warn" data-telegram-rail="invalid_token"/);
    assert.match(zh, /token 已失效 · 手机端通知已停。请在 \.env 里更新 TELOXIDE_TOKEN 并重启安琪。/);
    assert.doesNotMatch(zh, /Telegram 照常提醒。/);

    const en = renderSettings({
      locale: 'en',
      telegramRail: { state: 'invalid_token', since_unix: nowUnix - 60, last_error: 'Invalid bot token', degraded: true },
    });
    assert.match(en, /Token invalid · Phone notifications have stopped\. Set a new TELOXIDE_TOKEN in \.env and restart Aegis\./);
  });

  it('says reminders stay on only when Telegram is connected', () => {
    const reachable = renderSettings({
      locale: 'zh',
      telegramRail: { state: 'reachable', since_unix: nowUnix - 60, last_error: null, degraded: false },
    });
    assert.match(reachable, /Telegram 照常提醒。/);
    assert.doesNotMatch(reachable, /vnext-settings-telegram-note warn/);

    const off = renderSettings({
      locale: 'zh',
      telegramRail: { state: 'unconfigured', since_unix: nowUnix - 60, last_error: null, degraded: false },
    });
    assert.match(off, /Telegram 未开启。提醒留在这个控制台。/);
    assert.doesNotMatch(off, /Telegram 照常提醒。/);

    const offEn = renderSettings({
      locale: 'en',
      telegramRail: { state: 'unconfigured', since_unix: nowUnix - 60, last_error: null, degraded: false },
    });
    assert.match(offEn, /Telegram is off\. Reminders stay in this console\./);

    const unreported = renderSettings({ locale: 'zh' });
    assert.doesNotMatch(unreported, /Telegram 照常提醒。/);
    assert.doesNotMatch(unreported, /vnext-settings-telegram-note/);

    const starting = renderSettings({
      locale: 'zh',
      telegramRail: { state: 'starting', since_unix: nowUnix - 5, last_error: null, degraded: false },
    });
    assert.match(starting, /正在连接/);
    assert.doesNotMatch(starting, /Telegram 照常提醒。/);

    const blip = renderSettings({
      locale: 'zh',
      telegramRail: { state: 'unreachable', since_unix: nowUnix - 90, last_error: 'dns', degraded: false },
    });
    assert.match(blip, /连不上 · 已持续 1 分钟，安琪会自动重试。/);
    assert.doesNotMatch(blip, /Telegram 照常提醒。/);
    assert.doesNotMatch(blip, /vnext-settings-telegram-note warn/);
  });
});
