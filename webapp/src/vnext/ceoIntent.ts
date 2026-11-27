/**
 * CEO Intent Contract helpers (client slice).
 * Spec: docs/design/2026-07-25-ceo-intent-contract.md
 *
 * Pure functions only — unit-tested. The server remains source of truth for
 * tools and gates; this layer pre-expands high-frequency owner shorthand so a
 * busy CEO can say one word and still get a usable prompt + visible cue.
 */
import type { Locale } from '../i18n/catalog';
import { catalogHas, t, tf } from '../i18n/catalog';

export type CeoRiskClass =
  | 'read'
  | 'organize'
  | 'draft'
  | 'external_write'
  | 'money'
  | 'privacy_upgrade'
  | 'irreversible'
  | 'mandate_change'
  | 'commitment';

export type CeoShorthandResolution = {
  /** Stable id for tests and future server alignment. */
  intentId: string;
  /** Owner-facing short label (shown in "我理解成：…"). */
  label: string;
  /** Expanded natural-language prompt sent to the secretary runtime. */
  expansion: string;
  /** Client-side confidence for the shorthand hit (not model confidence). */
  confidence: number;
  riskClass: CeoRiskClass;
  /** True when acting on this intent would typically require review before external side effects. */
  needsReviewHint: boolean;
};

type ShorthandEntry = {
  intentId: string;
  /** Exact match keys (normalized). */
  keys: readonly string[];
  labelKey: string;
  expansionKey: string;
  riskClass: CeoRiskClass;
  needsReviewHint: boolean;
  confidence?: number;
};

const SHORTHAND: readonly ShorthandEntry[] = [
  {
    intentId: 'commute_status',
    keys: ['通勤', '路况', '出门', 'commute', 'traffic'],
    labelKey: 'ceoIntent.shorthand.commute.label',
    expansionKey: 'ceoIntent.shorthand.commute.expansion',
    riskClass: 'read',
    needsReviewHint: false,
  },
  {
    intentId: 'what_needs_me',
    keys: ['现在', '忙', '优先', 'now', 'busy'],
    labelKey: 'ceoIntent.shorthand.now.label',
    expansionKey: 'ceoIntent.shorthand.now.expansion',
    riskClass: 'read',
    needsReviewHint: false,
  },
  {
    intentId: 'lawyer_salience',
    keys: ['律师', '法务', 'lawyer', 'legal'],
    labelKey: 'ceoIntent.shorthand.lawyer.label',
    expansionKey: 'ceoIntent.shorthand.lawyer.expansion',
    riskClass: 'read',
    needsReviewHint: false,
  },
  {
    intentId: 'billing_salience',
    keys: ['账单', '发票', '付款', 'bill', 'invoice', 'billing'],
    labelKey: 'ceoIntent.shorthand.billing.label',
    expansionKey: 'ceoIntent.shorthand.billing.expansion',
    riskClass: 'read',
    needsReviewHint: false,
  },
  {
    intentId: 'draft_reply_focus',
    keys: ['回他', '回复', '回信', 'reply'],
    labelKey: 'ceoIntent.shorthand.reply.label',
    expansionKey: 'ceoIntent.shorthand.reply.expansion',
    riskClass: 'draft',
    needsReviewHint: true,
  },
  {
    intentId: 'approve_pending',
    keys: ['批', '批准', '就这样', '好', 'ok', 'approve'],
    labelKey: 'ceoIntent.shorthand.approve.label',
    expansionKey: 'ceoIntent.shorthand.approve.expansion',
    riskClass: 'commitment',
    needsReviewHint: false,
  },
  {
    intentId: 'reject_pending',
    keys: ['不用', '先不要', '否', '拒绝', '别', 'no', 'reject'],
    labelKey: 'ceoIntent.shorthand.reject.label',
    expansionKey: 'ceoIntent.shorthand.reject.expansion',
    riskClass: 'read',
    needsReviewHint: false,
  },
  {
    intentId: 'show_draft',
    keys: ['草稿', 'draft'],
    labelKey: 'ceoIntent.shorthand.draft.label',
    expansionKey: 'ceoIntent.shorthand.draft.expansion',
    riskClass: 'draft',
    needsReviewHint: true,
  },
  {
    intentId: 'watch_focus',
    keys: ['盯着', '盯', 'watch'],
    labelKey: 'ceoIntent.shorthand.watch.label',
    expansionKey: 'ceoIntent.shorthand.watch.expansion',
    riskClass: 'mandate_change',
    needsReviewHint: true,
  },
  {
    intentId: 'today_brief',
    keys: ['今天', '今日', 'today'],
    labelKey: 'ceoIntent.shorthand.today.label',
    expansionKey: 'ceoIntent.shorthand.today.expansion',
    riskClass: 'read',
    needsReviewHint: false,
  },
  {
    intentId: 'archive_focus',
    keys: ['归档', 'archive'],
    labelKey: 'ceoIntent.shorthand.archive.label',
    expansionKey: 'ceoIntent.shorthand.archive.expansion',
    riskClass: 'organize',
    needsReviewHint: false,
  },
  {
    intentId: 'send_draft',
    keys: ['发', '发送', 'send'],
    labelKey: 'ceoIntent.shorthand.send.label',
    expansionKey: 'ceoIntent.shorthand.send.expansion',
    riskClass: 'external_write',
    needsReviewHint: true,
  },
  {
    intentId: 'delete_focus',
    keys: ['删掉', '删除', 'delete'],
    labelKey: 'ceoIntent.shorthand.delete.label',
    expansionKey: 'ceoIntent.shorthand.delete.expansion',
    riskClass: 'irreversible',
    needsReviewHint: true,
  },
  {
    intentId: 'triage_open_loops',
    keys: ['帮我处理', '处理', 'handle', 'triage'],
    labelKey: 'ceoIntent.shorthand.handle.label',
    expansionKey: 'ceoIntent.shorthand.handle.expansion',
    riskClass: 'organize',
    needsReviewHint: true,
  },
  {
    intentId: 'quiet_mode',
    keys: ['安静', '勿扰', 'quiet', 'dnd'],
    labelKey: 'ceoIntent.shorthand.quiet.label',
    expansionKey: 'ceoIntent.shorthand.quiet.expansion',
    riskClass: 'mandate_change',
    needsReviewHint: false,
  },
];

/** Normalize owner input for shorthand matching. */
export function normalizeCeoUtterance(raw: string): string {
  return raw.trim().replace(/\s+/g, ' ').toLowerCase();
}

/** Surfaces the WebApp can navigate to from CEO tools / short utterances. */
export type CeoUiSurface = 'home' | 'work' | 'status' | 'settings';

/**
 * Pure UI navigation intent (open a shell surface). Returns null for
 * free-text that is not pure navigation.
 */
export function resolveUiNavigationIntent(raw: string): CeoUiSurface | null {
  const key = normalizeCeoUtterance(raw);
  if (!key) return null;

  // Strip common polite / filler prefixes so "请帮我打开工作" still hits.
  // Loop: "请" then "帮我" may both appear.
  let stripped = key;
  for (let i = 0; i < 4; i += 1) {
    const next = stripped
      .replace(/^(请你|请|麻烦|帮我|给我|帮忙|直接|先)\s*/, '')
      .replace(/^(please|can you|could you|help me)\s+/i, '')
      .trim();
    if (next === stripped) break;
    stripped = next;
  }

  const candidates = [key, stripped];

  for (const c of candidates) {
    // Work queue
    if (
      /^(打开|开启|切换到|去|到|看|看看|看一下|显示|进入)?\s*(工作|待办|事项|队列)$/.test(c)
      || /^(open|show|go to|switch to)\s*(the\s+)?(work|queue|tasks?)$/.test(c)
      || c === 'work'
    ) {
      return 'work';
    }
    // Home desk
    if (
      /^(打开|开启|切换到|去|到|回|回到|看|看看|显示|进入)?\s*(首页|主页|桌面|home)$/.test(c)
      || /^(open|show|go to|switch to)\s*(the\s+)?(home|desk)$/.test(c)
      || c === 'home'
    ) {
      return 'home';
    }
    // Settings
    if (
      /^(打开|开启|切换到|去|到|看|看看|显示|进入)?\s*(设置|设定)$/.test(c)
      || /^(open|show|go to|switch to)\s*(the\s+)?settings?$/.test(c)
      || c === 'settings'
    ) {
      return 'settings';
    }
    // Status
    if (
      /^(打开|开启|切换到|去|到|看|看看|显示|进入)?\s*(状态|系统状态)$/.test(c)
      || /^(open|show|go to|switch to)\s*(the\s+)?status$/.test(c)
      || c === 'status'
    ) {
      return 'status';
    }
  }
  return null;
}

/**
 * When the utterance starts with pure navigation then continues with work
 * ("打开工作，看看 OpenRouter"), return surface + remainder so the client can
 * switch the view first, then run the rest through the secretary runtime.
 */
export function splitUiNavigationPrefix(
  raw: string,
): { surface: CeoUiSurface; remainder: string } | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  // If the whole utterance is pure navigation, no remainder.
  if (resolveUiNavigationIntent(trimmed)) return null;

  // Match on original case so remainder keeps owner wording (OpenRouter etc.).
  let working = trimmed;
  for (let i = 0; i < 4; i += 1) {
    const next = working
      .replace(/^(请你|请|麻烦|帮我|给我|帮忙|直接|先)\s*/u, '')
      .replace(/^(please|can you|could you|help me)\s+/i, '')
      .trim();
    if (next === working) break;
    working = next;
  }

  const patterns: Array<{ surface: CeoUiSurface; re: RegExp }> = [
    {
      surface: 'work',
      re: /^(打开|开启|切换到|去|看|看看|进入)\s*(工作|待办|事项|队列)(?:界面|页面|视图)?[，,。!！\s：:]+(.+)$/su,
    },
  ];

  for (const { surface, re } of patterns) {
    const m = working.match(re);
    if (!m) continue;
    const remainder = (m[m.length - 1] || '').trim();
    if (!remainder) continue;
    return { surface, remainder };
  }
  return null;
}

/**
 * True when the utterance is short enough that CEO shorthand / one-word doctrine
 * should apply (contract §5). Longer free text skips the client map.
 * CJK often has no spaces — treat dense strings over 8 code units as free text
 * unless they are exact dictionary keys (checked by resolveCeoShorthand).
 */
export function isShortCeoUtterance(raw: string): boolean {
  const trimmed = raw.trim();
  if (!trimmed) return false;
  // Allow dictionary exact-key lookup path for any short-ish token stream.
  if (trimmed.length <= 8) return true;
  const tokens = trimmed.split(/\s+/).filter(Boolean);
  // Latin multi-word: up to 3 short words (e.g. "handle this").
  if (tokens.length >= 2 && tokens.length <= 3 && trimmed.length <= 20) return true;
  // Single token longer than 8 is free text (CJK sentences, long English words).
  return false;
}

/**
 * Resolve a high-frequency CEO shorthand. Returns null when no map hit — the
 * model path handles free text. Locale only affects labels/expansions, not keys.
 */
/**
 * Approve / decline while a voice search gate is waiting.
 * Trailing punctuation is not part of the decision: 「批准。」 is 批准.
 */
export function voicePendingDecision(raw: string): 'approve' | 'reject' | null {
  const stripped = raw.trim().replace(/[。.!！？?，,]+$/u, '').trim();
  if (!stripped) return null;
  const intent = resolveCeoShorthand(stripped);
  if (intent?.intentId === 'approve_pending') return 'approve';
  if (intent?.intentId === 'reject_pending') return 'reject';
  return null;
}

export function resolveCeoShorthand(raw: string, locale: Locale = 'en'): CeoShorthandResolution | null {
  const key = normalizeCeoUtterance(raw);
  if (!key) return null;
  const trimmed = raw.trim();
  // Exact dictionary keys always resolve (even multi-char phrases like 帮我处理).
  for (const entry of SHORTHAND) {
    const hit = entry.keys.some((k) => k === key || k === trimmed || k.toLowerCase() === key);
    if (!hit) continue;
    return {
      intentId: entry.intentId,
      label: t(entry.labelKey, locale),
      expansion: t(entry.expansionKey, locale),
      confidence: entry.confidence ?? 0.9,
      riskClass: entry.riskClass,
      needsReviewHint: entry.needsReviewHint,
    };
  }
  // No exact key: only attempt further matching on short utterances.
  if (!isShortCeoUtterance(raw)) return null;
  return null;
}

/**
 * Whether this risk class requires a CEO Review Card before external effects
 * (contract §4). Client hint only — server enforces.
 */
export function riskClassNeedsReview(risk: CeoRiskClass): boolean {
  return (
    risk === 'external_write'
    || risk === 'money'
    || risk === 'privacy_upgrade'
    || risk === 'irreversible'
    || risk === 'mandate_change'
    || risk === 'commitment'
    || risk === 'draft'
  );
}

/**
 * Map a backend tool / skill name to owner-facing copy for the Review Card.
 * Unknown tools fall back to a generic action label (never raw snake_case if
 * we can avoid it).
 */
export function ownerFacingApprovalAction(toolName: string | null | undefined, locale: Locale = 'en'): string {
  const raw = (toolName ?? '').trim();
  if (!raw) return t('chatHome.approval.whatFallback', locale);
  const key = raw.toLowerCase().replace(/[\s.]+/g, '_');
  const known: Array<{ needle: string; i18nKey: string }> = [
    { needle: 'archive', i18nKey: 'chatHome.approval.tool.archive' },
    { needle: 'delete', i18nKey: 'chatHome.approval.tool.delete' },
    { needle: 'calendar', i18nKey: 'chatHome.approval.tool.calendar' },
    { needle: 'mcp', i18nKey: 'chatHome.approval.tool.external_tool' },
    { needle: 'shell', i18nKey: 'chatHome.approval.tool.shell' },
    { needle: 'run_command', i18nKey: 'chatHome.approval.tool.shell' },
    { needle: 'browser', i18nKey: 'chatHome.approval.tool.browser' },
    { needle: 'web_fetch', i18nKey: 'chatHome.approval.tool.browser' },
    { needle: 'attachment', i18nKey: 'chatHome.approval.tool.read_private' },
  ];
  for (const { needle, i18nKey } of known) {
    if (key === needle || key.includes(needle)) {
      return catalogHas(i18nKey) ? t(i18nKey, locale) : t('chatHome.approval.whatFallback', locale);
    }
  }
  const softened = raw.replace(/[_.-]+/g, ' ').trim();
  return tf('chatHome.approval.whatNamed', locale, { action: softened });
}

/** Contract §4 why-line for review cards when backend only sent a tool name. */
export function ownerFacingApprovalWhy(toolName: string | null | undefined, locale: Locale = 'en'): string {
  const raw = (toolName ?? '').trim().toLowerCase();
  if (!raw) return t('chatHome.approval.whyFallback', locale);
  if (/send|message|telegram/.test(raw)) return t('chatHome.approval.why.external', locale);
  if (/delete|purge|drop/.test(raw)) return t('chatHome.approval.why.irreversible', locale);
  if (/pay|purchase|transfer|charge/.test(raw)) return t('chatHome.approval.why.money', locale);
  if (/read|body|attachment|private/.test(raw)) return t('chatHome.approval.why.privacy', locale);
  if (/mcp|shell|command|browser/.test(raw)) return t('chatHome.approval.why.external_tool', locale);
  return t('chatHome.approval.whyFallback', locale);
}
