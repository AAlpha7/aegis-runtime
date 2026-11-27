import React from 'react';
import { createRoot } from 'react-dom/client';
import '../src/vnext/styles.css';
import '../src/vnext/design/index.css';
import { ChatHomeView } from '../src/vnext/ChatHomeView';
import { WorkView } from '../src/vnext/WorkView';
import { VNextIcon } from '../src/vnext/icons';
import { VNextButton } from '../src/vnext/VNextButton';
import { VNextSegmented } from '../src/vnext/VNextSegmented';
import { VNextHeaderMenu } from '../src/vnext/VNextHeaderMenu';
import { VNextChip } from '../src/vnext/VNextChip';
import { StatusView } from '../src/vnext/StatusView';
import { SettingsView } from '../src/vnext/SettingsView';
import { RichText } from '../src/vnext/RichText';
import { AssistantResponseView } from '../src/vnext/AssistantResponseView';
import { LlmDegradedBanner } from '../src/vnext/LlmDegradedBanner';
import { LocaleProvider } from '../src/i18n/LocaleContext';
import { workProps, workDecideProps, workDetailProps, statusProps, settingsProps } from './fixtures';

// Multi-section assistant answer (all five tones) + source evidence + a warning,
// to verify the answer-card stack: flattening, section dividers, tone labels,
// evidence-row text, and .vnext-warning colour — all in light + dark.
const assistantResult: any = {
  response_kind: 'answer',
  visible_answer: [
    'Answer: Three things need your decision this morning; I handled the rest.',
    'Next actions: Reply to the billing email, then confirm the 2pm with Dr. Lin.',
    'Sources: Based on your Gmail inbox and Google Calendar.',
    'Caveats: Calendar access needs a refresh before I can reschedule anything.',
    'Done: Archived 4 newsletters and saved a draft reply for your review.',
  ].join('\n'),
  severity: 'important',
  receipt_expectation: 'Reading alone never authorizes an outside change.',
  citations: ['Gmail', 'Google Calendar'],
  source_evidence: [
    { kind: 'tool', label: 'Billing statement', summary: 'Maple Lane Music School — statement ready', detail: 'Received 8:02 AM via the Gmail connector (Codex). Metadata only; body not read.' },
    { kind: 'calendar', label: '2pm with Dr. Lin', summary: 'Reschedule request pending your call', detail: 'Organizer proposed a new time.' },
  ],
  warnings: ['Calendar token expires soon — reconnect to keep syncing.'],
};

// Kitchen-sink markdown to verify EVERY rich-text element is legible in both
// themes (the assistant renders into .vnext-rich-*; the contrast fix lives in
// design/surfaces/rich-text.css).
const richKitchenSink = [
  '# Heading three',
  '',
  '## Heading four',
  '',
  '### Section label',
  '',
  'A paragraph with **strong text**, *emphasis*, a [link to settings](https://aegis.example.org), and inline `code()`.',
  '',
  '- First bullet point',
  '- Second bullet with a longer line so wrapping is visible too',
  '',
  '1. Ordered step one',
  '2. Ordered step two',
  '',
  '- [x] Confirmed the 2pm with Dr. Lin',
  '- [ ] Reply to the billing email',
  '',
  '> A blockquote: reading alone never authorizes an outside change.',
  '',
  '```ts',
  'const receipt = await aegis.confirm(action);',
  '```',
  '',
  '| Source | Status | When |',
  '| --- | --- | --- |',
  '| Gmail | Ready | 8:02 AM |',
  '| Calendar | Needs attention | 8:03 AM |',
].join('\n');

const noop = () => {};
const asyncOk = async () => ({ ok: true, message: 'Receipt saved.', nextStatus: 'archived' as const });

const avatarProfile = { aegisAvatarUrl: './aegis-avatar.svg', ownerAvatarUrl: '' };

const contextSignals = [
  { id: 'decision', label: 'Needs decision', summary: 'Review calendar access before Calendar can run', tone: 'warn', targetItemId: 'approval:calendar' },
  { id: 'calendar', label: 'Coming up', summary: '2pm with Dr. Lin — reschedule request pending', tone: 'normal', targetItemId: 'cal:open' },
  { id: 'private', label: 'Needs detail', summary: 'Insurance follow-up is waiting on you', tone: 'warn', targetItemId: 'task:42' },
];

const messages = [
  { id: 'm1', role: 'owner', text: 'What needs my attention this morning?', createdAt: '2026-05-31T08:00:00Z' },
  { id: 'm2', role: 'aegis', text: 'Three things need your decision. I handled the rest — archived 4 newsletters, drafted a reply to the billing email for your review, and confirmed your 2pm is still on.', createdAt: '2026-05-31T08:00:04Z' },
];

const chatHomeProps = {
  email: 'owner@example.com', messages, brief: '3 things need your decision. I handled the rest.',
  value: '', busy: false, snapshotMessage: '', snapshotAvailable: true, snapshotBusy: false,
  contextSignals, workBadgeCount: 3, workBadgeLabel: 'Work, 3 decisions waiting', workBadgeTone: 'warn',
  costSavingPaused: false, avatarProfile,
  onChange: noop, onSubmit: noop, onSuggestion: noop, onClearConversation: noop,
  onOpenWork: noop, onOpenStatus: noop,
  onOpenSettings: noop, onVoiceReady: noop, onRetry: noop, onAskWorkItem: noop,
  onWorkItemAction: asyncOk,
  onOpenWorkDetail: async () => ({ ok: false, message: 'Detail unavailable in static render.' }),
  onNoticeAction: async () => ({ ok: true, message: 'Reminder updated.' }),
  onAttachContext: noop, onClearAttachedContext: noop, onCreateSnapshot: noop,
  onPauseSelfStartedWork: noop, onResumeBackgroundWork: noop,
  installActionLabel: 'Install app', onInstallOwnerApp: noop, onLogout: noop,
};

function ChatShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="vnext-app vnext-app-chat-home">
      <div className="vnext-shell">
        <main className="vnext-main">{children}</main>
      </div>
    </div>
  );
}

// Faithful non-home console shell: stacked brand-rail + header + main (matches
// App.tsx structure so the same CSS applies).
function ConsoleShell({ view, title, subtitle, children }: { view: string; title: string; subtitle: string; children: React.ReactNode }) {
  return (
    <div className={`vnext-app vnext-app-${view}`}>
      <aside className="vnext-brand-rail" aria-label="Aegis owner navigation">
        <div className="vnext-brand-lockup">
          <span className="vnext-shield-mark vnext-avatar">A</span>
          <div><strong>Aegis</strong><span>Your private secretary.</span><span>Your time, protected.</span></div>
        </div>
        <p className="vnext-brand-copy">Organizes what needs attention, asks before important changes, and keeps receipts.</p>
        <nav className="vnext-rail-nav" aria-label="Primary desktop">
          {/* Match production ownerPrimaryNavItemIds: Chat / Work (aria-current
              marks the active destination, as App.tsx NavButton does). */}
          <button type="button" className={view === 'home' ? 'vnext-nav-button is-active' : 'vnext-nav-button'} aria-current={view === 'home' ? 'page' : undefined}>Chat</button>
          <button type="button" className={view === 'work' ? 'vnext-nav-button is-active' : 'vnext-nav-button'} aria-current={view === 'work' ? 'page' : undefined}>Work</button>
        </nav>
      </aside>
      <div className="vnext-shell">
        <header className="vnext-header">
          <div>
            <p className="vnext-kicker">Private secretary</p>
            <h1>{title}</h1>
            <p className="vnext-header-subtitle">{subtitle}</p>
          </div>
          <div className="vnext-header-actions">
            <button type="button" className="vnext-secondary vnext-header-system" aria-label="Status" title="Status"><VNextIcon name="status" /><span>Status</span></button>
            <button type="button" className="vnext-secondary vnext-header-system" aria-label="Settings" title="Settings"><VNextIcon name="settings" /><span>Settings</span></button>
            <button type="button" className="vnext-secondary vnext-header-snapshot" aria-label="Snapshot" title="Snapshot"><VNextIcon name="camera" /><span>Snapshot</span></button>
          </div>
        </header>
        <main className="vnext-main">{children}</main>
      </div>
      {/* Bottom nav (mobile) — production Chat / Work; forced display for shots. */}
      <nav className="vnext-nav" style={{ display: 'grid' }} aria-label="Primary mobile">
        <button type="button" className={view === 'home' ? 'active' : ''} aria-current={view === 'home' ? 'page' : undefined}>Chat</button>
        <button type="button" className={view === 'work' ? 'active' : ''} aria-current={view === 'work' ? 'page' : undefined}>Work</button>
      </nav>
    </div>
  );
}

const which = new URLSearchParams(location.search).get('view') || 'home';
const root = createRoot(document.getElementById('root')!);
const SUPPORTED = ['home', 'empty', 'degraded', 'degraded-zh', 'chat-progress-zh', 'chat-approval-zh', 'chat-attachment-zh', 'work', 'work-decide', 'work-detail', 'work-zh', 'work-en', 'work-detail-zh', 'work-detail-en', 'work-decide-zh', 'status', 'status-zh', 'status-en', 'settings', 'settings-zh', 'settings-en', 'richtext', 'assistant', 'loading', 'error', 'button-system', 'controls-system', 'header-menu-system', 'planC-nav'];

// VNextButton gallery (SP1): every variant × size × state, rendered inside a
// settings view-root shell so the real --btn-*/--b-* tokens resolve. Mirrors the
// approved L3 mock; lets a screenshot verify obsidian/champagne/garnet material
// + labels in both themes.
function ButtonGallery() {
  const plus = <VNextIcon name="camera" />;
  const Row = ({ label, children }: { label: string; children: React.ReactNode }) => (
    <div style={{ display: 'grid', gap: 8, marginBottom: 22 }}>
      <p className="vnext-kicker" style={{ margin: 0 }}>{label}</p>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 12, alignItems: 'center' }}>{children}</div>
    </div>
  );
  return (
    <div style={{ padding: 28, maxWidth: 980 }}>
      <Row label="Variants (md)">
        <VNextButton variant="primary">主要操作</VNextButton>
        <VNextButton variant="cta">确认部署</VNextButton>
        <VNextButton variant="secondary">次要操作</VNextButton>
        <VNextButton variant="ghost">幽灵按钮</VNextButton>
        <VNextButton variant="destructive">退出登录</VNextButton>
      </Row>
      <Row label="Sizes (primary)">
        <VNextButton variant="primary" size="sm">Small 36</VNextButton>
        <VNextButton variant="primary" size="md">Medium 44</VNextButton>
        <VNextButton variant="primary" size="lg">Large 52</VNextButton>
      </Row>
      <Row label="CTA sizes">
        <VNextButton variant="cta" size="sm">Send</VNextButton>
        <VNextButton variant="cta" size="md">发送</VNextButton>
        <VNextButton variant="cta" size="lg">确认部署</VNextButton>
      </Row>
      <Row label="With icons">
        <VNextButton variant="primary" leadingIcon={plus}>新建</VNextButton>
        <VNextButton variant="secondary" trailingIcon={plus}>下一步</VNextButton>
        <VNextButton variant="cta" leadingIcon={plus}>发送</VNextButton>
      </Row>
      <Row label="Icon-only (size=icon)">
        <VNextButton variant="primary" iconOnly size="icon" aria-label="主要">{plus}</VNextButton>
        <VNextButton variant="secondary" iconOnly size="icon" aria-label="次要">{plus}</VNextButton>
        <VNextButton variant="ghost" iconOnly size="icon" aria-label="幽灵">{plus}</VNextButton>
        <VNextButton variant="destructive" iconOnly size="icon" aria-label="删除">{plus}</VNextButton>
      </Row>
      <Row label="States">
        <VNextButton variant="primary" loading>加载中</VNextButton>
        <VNextButton variant="cta" loading>发送中</VNextButton>
        <VNextButton variant="primary" disabled>禁用</VNextButton>
        <VNextButton variant="secondary" disabled>禁用</VNextButton>
      </Row>
      <Row label="Full width">
        <div style={{ width: 320 }}>
          <VNextButton variant="cta" fullWidth>确认部署</VNextButton>
        </div>
      </Row>
    </div>
  );
}

// VNextSegmented + VNextChip gallery (SP1.5): every selection state, rendered
// inside a settings view-root shell so the real --btn-*/--seg-*/--chip-*/--b-*
// tokens resolve. Interactive (useState) so the screenshot shows a real
// selected segment/chip; both themes verify obsidian-selected vs quiet
// unselected contrast.
function HeaderMenuGallery() {
  const items = [
    { id: 'status', label: '状态', icon: 'status' as const, onSelect() {} },
    { id: 'settings', label: '设置', icon: 'settings' as const, onSelect() {} },
    { id: 'snapshot', label: '保存支持快照', icon: 'camera' as const, onSelect() {} },
    { id: 'logout', label: '退出登录', tone: 'destructive' as const, onSelect() {} },
  ];
  return (
    <div style={{ padding: 28, maxWidth: 980, minHeight: 460 }}>
      <p className="vnext-kicker" style={{ margin: '0 0 10px' }}>Header utility menu — closed trigger</p>
      <div style={{ display: 'flex', gap: 18, alignItems: 'flex-start', marginBottom: 40 }}>
        <VNextHeaderMenu triggerLabel="打开菜单" items={items} />
      </div>
      <p className="vnext-kicker" style={{ margin: '0 0 10px' }}>Header utility menu — open (status / settings / snapshot / logout)</p>
      <div style={{ display: 'flex', gap: 18, alignItems: 'flex-start', position: 'relative', height: 320 }}>
        <VNextHeaderMenu triggerLabel="打开菜单" items={items} defaultOpen align="start" />
      </div>
    </div>
  );
}

function ControlsGallery() {
  const Row = ({ label, children }: { label: string; children: React.ReactNode }) => (
    <div style={{ display: 'grid', gap: 8, marginBottom: 22 }}>
      <p className="vnext-kicker" style={{ margin: 0 }}>{label}</p>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 12, alignItems: 'center' }}>{children}</div>
    </div>
  );
  const [lane, setLane] = React.useState('now');
  const [lang, setLang] = React.useState('zh');
  const [scope, setScope] = React.useState('all');
  const [source, setSource] = React.useState<string | null>(null);
  const [scan, setScan] = React.useState<Record<string, boolean>>({ unread: true, flagged: false });
  return (
    <div style={{ padding: 28, maxWidth: 980 }}>
      <Row label="Segmented — tabs (work lanes)">
        <VNextSegmented
          ariaLabel="泳道"
          mode="tab"
          value={lane}
          onChange={setLane}
          options={[
            { value: 'decide', label: '待决定' },
            { value: 'now', label: '现在' },
            { value: 'follow_up', label: '跟进' },
            { value: 'backlog', label: '待办' },
          ]}
        />
      </Row>
      <Row label="Segmented — radio (settings language, md)">
        <VNextSegmented
          ariaLabel="界面语言"
          mode="radio"
          value={lang}
          onChange={setLang}
          options={[
            { value: 'zh', label: '中文' },
            { value: 'en', label: 'English' },
          ]}
        />
      </Row>
      <Row label="Segmented — radio sm (work scan scope)">
        <VNextSegmented
          ariaLabel="范围"
          mode="radio"
          size="sm"
          value={scope}
          onChange={setScope}
          options={[
            { value: 'all', label: '全部' },
            { value: 'needs_me', label: '需要我' },
            { value: 'quiet', label: '安静' },
          ]}
        />
      </Row>
      <Row label="Segmented — full width + a disabled segment">
        <div style={{ width: 360 }}>
          <VNextSegmented
            ariaLabel="排序"
            mode="radio"
            fullWidth
            value={scope}
            onChange={setScope}
            options={[
              { value: 'all', label: '泳道' },
              { value: 'needs_me', label: '优先级' },
              { value: 'quiet', label: '更新', disabled: true },
            ]}
          />
        </div>
      </Row>
      <Row label="Chips — source filter (sm; one selected)">
        <VNextChip size="sm" selected={source === null} onClick={() => setSource(null)}>全部来源</VNextChip>
        <VNextChip size="sm" selected={source === 'calendar'} onClick={() => setSource('calendar')}>日历</VNextChip>
        <VNextChip size="sm" selected={source === 'tasks'} onClick={() => setSource('tasks')}>任务</VNextChip>
        <VNextChip size="sm" selected={source === 'reminders'} onClick={() => setSource('reminders')}>提醒</VNextChip>
      </Row>
      <Row label="Chips — toggles (md; independent on/off)">
        <VNextChip
          leadingIcon={<VNextIcon name="check" />}
          selected={scan.unread}
          onClick={() => setScan((s) => ({ ...s, unread: !s.unread }))}
        >未读</VNextChip>
        <VNextChip
          selected={scan.flagged}
          onClick={() => setScan((s) => ({ ...s, flagged: !s.flagged }))}
        >已标记</VNextChip>
        <VNextChip selected={false} disabled>已归档</VNextChip>
      </Row>
    </div>
  );
}

// Multimodal composer fixture (Slice 5): a zh chat surface where the owner has
// uploaded an image and is holding it as a pending attachment in the composer.
// Shows the attach (clip) button + the pending-attachment PILL (已附加：filename)
// with its remove button. `onAttachFile`/`onClearAttachment` are present so the
// composer renders the file-input + attach button branch.
const chatAttachmentZhProps = {
  ...chatHomeProps,
  busy: false,
  messages: [],
  onAttachFile: noop,
  onClearAttachment: noop,
  pendingAttachment: {
    kind: 'image',
    dataUrl: 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
    filename: '收据.png',
    mimeType: 'image/png',
  },
};

// Live per-turn progress fixture (Slice 3): a zh chat surface mid-turn whose
// working message shows the localized "正在使用 web_search…" text the
// `tool_start` SSE frame drives in App.tsx — the visual confirmation that the
// static spinner is replaced by live progress.
const chatProgressZhProps = {
  ...chatHomeProps,
  busy: true,
  messages: [
    { id: 'p-owner', role: 'owner', text: '帮我查一下今天的天气', createdAt: '2026-06-09T08:00:00Z' },
    { id: 'p-working', role: 'working', text: '正在使用 web_search…', createdAt: '2026-06-09T08:00:02Z' },
  ],
};

// In-chat HITL approval card fixture (Slice 4): a zh chat surface where a
// `need_approval` SSE frame rendered an approval card. Shows the pending card
// (summary + 批准/拒绝) plus a second already-resolved card (已批准 chip) so a
// single screenshot proves both states. The card is independent of the
// working-message lifecycle (busy=false here).
const chatApprovalZhProps = {
  ...chatHomeProps,
  busy: false,
  messages: [
    { id: 'a-owner', role: 'owner', text: '帮我回复账单邮件', createdAt: '2026-06-09T08:00:00Z' },
    {
      id: 'approval:appr-pending',
      role: 'approval',
      text: '',
      createdAt: '2026-06-09T08:00:03Z',
      approvalId: 'appr-pending',
      toolName: 'run_external',
      summary: '通过 Codex 向 billing@northwest.example 发送回复，附上已起草的退款说明。',
      approvalState: 'pending',
    },
    {
      id: 'approval:appr-done',
      role: 'approval',
      text: '',
      createdAt: '2026-06-09T08:01:00Z',
      approvalId: 'appr-done',
      toolName: 'calendar_create_event',
      summary: '在周五 15:00 创建「与会计师复盘」日程。',
      approvalState: 'approved',
    },
  ],
};

// Degraded-provider verification fixture: stacks the sticky LlmDegradedBanner
// over a chat surface whose last turn is an assistant ERROR (so the localized
// error header + retry button render). Used to confirm zh sessions have NO
// English leak in the banner, the chat error header, or the retry button.
const degradedChatHomeProps = {
  ...chatHomeProps,
  busy: false,
  messages: [
    { id: 'd-owner', role: 'owner', text: '帮我查一下今早的邮件。', createdAt: '2026-06-09T08:00:00Z' },
    {
      id: 'd-aegis-error',
      role: 'aegis',
      text: '我没能完成这件事。请重试。',
      createdAt: '2026-06-09T08:00:04Z',
      result: {
        schema_version: 1,
        response_kind: 'error',
        visible_answer: '我没能完成这件事。请重试。',
        receipt_expectation: 'No action was taken.',
        citations: [],
        warnings: [],
        severity: 'warning',
      },
      retryText: '重新尝试刚才的请求',
      // Dual-use English logic key (set by App.tsx); display must localize.
      retryLabel: 'Try again',
    },
  ],
};

function DegradedHarness({ displayLanguage, reason, retryLabel }: { displayLanguage: string; reason: string; retryLabel: string }) {
  const props = {
    ...degradedChatHomeProps,
    messages: degradedChatHomeProps.messages.map((m: any) =>
      m.id === 'd-aegis-error' ? { ...m, retryLabel } : m),
  };
  return (
    <LocaleProvider displayLanguage={displayLanguage}>
      <div className="vnext-app vnext-app-chat-home">
        {React.createElement(LlmDegradedBanner as any, {
          degraded: { since: '2026-06-09T08:00:00Z', reason },
          dismissedSince: null,
          onDismiss: noop,
        })}
        <div className="vnext-shell">
          <main className="vnext-main">{React.createElement(ChatHomeView as any, props)}</main>
        </div>
      </div>
    </LocaleProvider>
  );
}

if (which === 'home') {
  root.render(<ChatShell>{React.createElement(ChatHomeView as any, chatHomeProps)}</ChatShell>);
} else if (which === 'degraded' || which === 'degraded-zh') {
  const isZh = which === 'degraded-zh';
  root.render(
    <DegradedHarness
      displayLanguage={isZh ? 'zh' : 'en'}
      reason={isZh ? 'Codex 已达使用上限' : 'Codex usage limit reached'}
      retryLabel="Try again"
    />,
  );
} else if (which === 'chat-progress-zh') {
  root.render(
    <LocaleProvider displayLanguage="zh">
      <ChatShell>{React.createElement(ChatHomeView as any, chatProgressZhProps)}</ChatShell>
    </LocaleProvider>,
  );
} else if (which === 'chat-approval-zh') {
  root.render(
    <LocaleProvider displayLanguage="zh">
      <ChatShell>{React.createElement(ChatHomeView as any, chatApprovalZhProps)}</ChatShell>
    </LocaleProvider>,
  );
} else if (which === 'chat-attachment-zh') {
  root.render(
    <LocaleProvider displayLanguage="zh">
      <ChatShell>{React.createElement(ChatHomeView as any, chatAttachmentZhProps)}</ChatShell>
    </LocaleProvider>,
  );
} else if (which === 'empty') {
  root.render(<ChatShell>{React.createElement(ChatHomeView as any, { ...chatHomeProps, messages: [] })}</ChatShell>);
} else if (which === 'richtext') {
  // NOTE (2026-07-10 UI audit): the stream div must sit inside
  // `.vnext-chat-home.has-conversation` — in the product, ChatHomeView always
  // nests it there, and that scope is what resets the legacy dark glass
  // background on `.vnext-chat-stream` (styles.css mobile block:
  // `background: transparent`). A BARE .vnext-chat-stream kept the dark-era
  // rgba(7,17,31,.52) fill and screenshotted as a dim veil over the whole
  // view — a harness-only artifact, not a product scrim.
  root.render(
    <ChatShell>
      <section className="vnext-chat-home has-conversation">
        <div className="vnext-chat-stream" style={{ display: 'grid', gap: 16, padding: 16 }}>
          <div className="vnext-chat-answer-card" style={{ padding: 16 }}>
            {React.createElement(RichText as any, {
              content: '**Verify the calendar permission before Calendar can act.** Reading alone never authorizes a change.',
              importanceSeverity: 'important',
            })}
          </div>
          <div className="vnext-chat-answer-card" style={{ padding: 16 }}>
            {React.createElement(RichText as any, { content: richKitchenSink })}
          </div>
        </div>
      </section>
    </ChatShell>,
  );
} else if (which === 'assistant') {
  // Same .vnext-chat-home.has-conversation nesting as 'richtext' (see note above).
  root.render(
    <ChatShell>
      <section className="vnext-chat-home has-conversation">
        <div className="vnext-chat-stream" style={{ display: 'grid', gap: 16, padding: 16 }}>
          <div className="vnext-chat-answer-card" style={{ padding: 16 }}>
            {React.createElement(AssistantResponseView as any, { result: assistantResult })}
          </div>
        </div>
      </section>
    </ChatShell>,
  );
} else if (which === 'work') {
  root.render(
    <ConsoleShell view="work" title="Work" subtitle="What needs attention now.">
      {React.createElement(WorkView as any, workProps())}
    </ConsoleShell>,
  );
} else if (which === 'work-decide') {
  root.render(
    <ConsoleShell view="work" title="Work" subtitle="A paused decision needs your call.">
      {React.createElement(WorkView as any, workDecideProps())}
    </ConsoleShell>,
  );
} else if (which === 'work-detail') {
  root.render(
    <ConsoleShell view="work" title="Work" subtitle="What needs attention now.">
      {React.createElement(WorkView as any, workDetailProps())}
    </ConsoleShell>,
  );
} else if (which === 'work-zh' || which === 'work-en' || which === 'work-detail-zh' || which === 'work-detail-en' || which === 'work-decide-zh') {
  // SP3 i18n leak-cleanup screenshots: render the Work view (list + detail +
  // decision) under an explicit LocaleProvider so the logic-layer cues / titles
  // / action buttons / basis text resolve in the chosen locale.
  const locale = which === 'work-en' || which === 'work-detail-en' ? 'en' : 'zh';
  const props =
    which === 'work-detail-zh' || which === 'work-detail-en'
      ? workDetailProps(locale)
      : which === 'work-decide-zh'
        ? workDecideProps(locale)
        : workProps(locale);
  const title = locale === 'zh' ? '工作' : 'Work';
  const subtitle = locale === 'zh' ? '现在需要关注的事项。' : 'What needs attention now.';
  root.render(
    <LocaleProvider displayLanguage={locale}>
      <ConsoleShell view="work" title={title} subtitle={subtitle}>
        {React.createElement(WorkView as any, props)}
      </ConsoleShell>
    </LocaleProvider>,
  );
} else if (which === 'status') {
  root.render(
    <ConsoleShell view="status" title="Status" subtitle="How Aegis is doing right now.">
      {React.createElement(StatusView as any, statusProps())}
    </ConsoleShell>,
  );
} else if (which === 'status-zh' || which === 'status-en') {
  // SP3: Status under an explicit LocaleProvider so the Connections disclosure
  // (sourceStatusView statusLabel/reason/nextStep), the operations-grid fallback
  // copy, and source labels all resolve in the chosen locale.
  const locale = which === 'status-en' ? 'en' : 'zh';
  root.render(
    <LocaleProvider displayLanguage={locale}>
      <ConsoleShell view="status" title={locale === 'zh' ? '状态' : 'Status'} subtitle={locale === 'zh' ? 'Aegis 当前的运行情况。' : 'How Aegis is doing right now.'}>
        {React.createElement(StatusView as any, statusProps())}
      </ConsoleShell>
    </LocaleProvider>,
  );
} else if (which === 'settings-zh' || which === 'settings-en') {
  // SP3: Settings under an explicit LocaleProvider so the projection-section
  // labels (localizedSettingsSectionLabel) + owner copy resolve in the locale.
  const locale = which === 'settings-en' ? 'en' : 'zh';
  root.render(
    <LocaleProvider displayLanguage={locale}>
      <ConsoleShell view="settings" title={locale === 'zh' ? '设置' : 'Settings'} subtitle={locale === 'zh' ? '连接、安全与通知。' : 'Connections, security, and notifications.'}>
        {React.createElement(SettingsView as any, settingsProps(locale))}
      </ConsoleShell>
    </LocaleProvider>,
  );
} else if (which === 'settings') {
  root.render(
    <ConsoleShell view="settings" title="Settings" subtitle="Connections, security, and notifications.">
      {React.createElement(SettingsView as any, settingsProps())}
    </ConsoleShell>,
  );
} else if (which === 'button-system') {
  root.render(
    <ConsoleShell view="settings" title="Buttons" subtitle="VNextButton — L3 obsidian + champagne gallery.">
      <ButtonGallery />
    </ConsoleShell>,
  );
} else if (which === 'controls-system') {
  root.render(
    <ConsoleShell view="settings" title="Controls" subtitle="VNextSegmented + VNextChip — L3 obsidian + champagne selection controls (SP1.5).">
      <ControlsGallery />
    </ConsoleShell>,
  );
} else if (which === 'header-menu-system') {
  root.render(
    <ConsoleShell view="settings" title="Header menu" subtitle="VNextHeaderMenu — L3 obsidian + champagne gear/overflow utility menu (SP2).">
      <HeaderMenuGallery />
    </ConsoleShell>,
  );
} else if (which === 'loading' || which === 'error') {
  const tone = which === 'error' ? 'error' : 'loading';
  root.render(
    <div className="vnext-app vnext-center">
      <div className={`vnext-center-state tone-${tone}`}>
        <span className="vnext-center-state-mark">{tone === 'error' ? '!' : '•'}</span>
        <p className="vnext-kicker">{tone === 'error' ? 'Something went wrong' : 'Getting set up'}</p>
        <h1>{tone === 'error' ? "Couldn't reach Aegis" : 'Loading your day'}</h1>
        <p>{tone === 'error' ? 'Check your connection and try again — nothing was lost.' : 'One moment while Aegis gathers what needs your attention.'}</p>
      </div>
    </div>,
  );
} else if (which === 'planC-nav') {
  // Plan C tab-signals proof: renders the bottom nav (.vnext-nav) with both
  // signals lit — Work count badge (warn tone), Chat unseen dot.
  // Uses the real CSS tokens from styles.css via the bundle.
  // This is a PARTIAL harness (not the full App shell; NavButton is private).
  // It exactly replicates the rendered markup that NavButton produces.
  function NavBadge({ count, warn }: { count: number; warn?: boolean }) {
    return <span className={`vnext-nav-badge${warn ? ' warn' : ''}`} aria-hidden="true">{count}</span>;
  }
  function NavDot() {
    return <span className="vnext-nav-dot" aria-hidden="true" />;
  }
  root.render(
    <LocaleProvider displayLanguage="zh">
      <div className="vnext-app" style={{ minHeight: '200px', background: 'var(--aegis-base)', position: 'relative', paddingBottom: '90px' }}>
        <nav className="vnext-nav" style={{ display: 'grid' }}>
          <button type="button" className="active" aria-label="有新消息" aria-current="page">
            <VNextIcon name="home" />
            <span>对话</span>
            <NavDot />
          </button>
          <button type="button" aria-label="工作，3 个决策待处理">
            <VNextIcon name="work" />
            <span>工作</span>
            <NavBadge count={3} warn />
          </button>
        </nav>
      </div>
    </LocaleProvider>,
  );
} else {
  root.render(
    React.createElement(
      'div',
      { style: { color: '#fff', font: '16px/1.5 monospace', padding: '40px', background: '#300', minHeight: '100vh' } },
      `⚠ preview fixture for "${which}" is not wired yet. Supported: ${SUPPORTED.join(', ')}. Add a real fixture in preview/harness.tsx before screenshotting this surface.`,
    ),
  );
}
