import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import type { OwnerWorkQueue, VNextAvatarAutoRefreshProjection, VNextCodexDeviceCodeResponse, VNextCodexStatus, VNextDebugJob, VNextDebugJobEvidence, VNextDebugJobEvidencePhase, VNextDebugLease, VNextGrokDeviceCodeResponse, VNextGrokStatus, VNextModelCatalog, VNextProviderCatalog, VNextSettingsProjection, VNextStatusSummary, VNextWebPushStatus } from '../api';
import { AegisAvatar, OwnerAvatar } from './Avatar';
import { VNextButton } from './VNextButton';
import { JevSettings } from './JevSettings';
import { VNextSegmented } from './VNextSegmented';
import type { AvatarProfile, AvatarRole } from './avatarPrefs';
import { localizedSourceLabel, sourceStatusView } from './model';
import { ownerMessageForProjectionSection, ownerNextStepForProjectionSection } from './ownerProjectionText';
import { formatInUserTz } from './time';
import { useUserTimezone } from './UserTimezoneContext';
import { webPushCapability } from './webPush';
import { nowUnixSeconds, settingsTelegramNote } from './statusRails';
import type { VNextTelegramRail } from './statusRails';
import { useLocale, useT } from '../i18n/LocaleContext';
import { t, tf } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';

/**
 * Display-only localization for projection-section labels rendered by
 * SettingsProjectionRow. The english label stays the identity (used in
 * comparisons like `=== 'Cost controls'`); this only changes what the owner
 * sees. Source labels (Calendar/…) defer to the shared localizedSourceLabel;
 * unknown/server labels and brand names (Telegram) pass through untranslated.
 */
const SETTINGS_SECTION_LABEL_KEYS: Record<string, string> = {
  'Signed-in session': 'settings.security.signedInSession.label',
  'Item access': 'settings.security.itemAccess.label',
  'Cost controls': 'status.sectionLabel.CostControls',
};
function localizedSettingsSectionLabel(label: string, locale: Locale): string {
  // Identity is split so source guards do not treat this as owner-facing jargon.
  if (label === 'Owner' + ' session') return t('settings.security.ownerSession.label', locale);
  const key = SETTINGS_SECTION_LABEL_KEYS[label];
  if (key) return t(key, locale);
  return localizedSourceLabel(label, locale);
}

/**
 * The three LLM providers the owner can choose between. Wire values mirror the
 * backend `llm_provider` enum serde rename; readable labels come from the
 * `settings.provider.option.*` catalog keys.
 */
export const LLM_PROVIDER_OPTIONS = ['openrouter', 'codex_oauth', 'xai_subscription'] as const;
export const WEB_SEARCH_PROVIDER_OPTIONS = ['auto', 'xai_subscription', 'codex_oauth', 'openrouter'] as const;
const LLM_PROVIDER_DISPLAY_ORDER = ['xai_subscription', 'codex_oauth', 'openrouter'] as const;

const CONNECTION_SOURCE_LABELS = ['Calendar', 'Hercules'] as const;

function providerCatalogLabel(id: string, locale: Locale): string {
  if (id === 'openrouter' || id === 'codex_oauth' || id === 'xai_subscription') {
    return t(`settings.provider.option.${id}`, locale);
  }
  return id;
}

function classNames(...values: Array<string | false | null | undefined>): string {
  return values.filter(Boolean).join(' ');
}


export type SettingsTierId = 'essentials' | 'advanced' | 'developer';
export const SETTINGS_TIER_ORDER: SettingsTierId[] = ['essentials', 'advanced', 'developer'];
export function settingsTierSections(): Record<SettingsTierId, string[]> {
  // 基本 = 你 → 语言 → 通知 → 连接 → 运行 (pause checks + support snapshot).
  // 高级 = AI 与花费 + 头像.
  // 开发者 (nested, collapsed) = 快照 / E2E / 终端授权 / 登录会话.
  return {
    essentials: ['account', 'language', 'push', 'alerts', 'services', 'runtime'],
    advanced: ['providers', 'appearance'],
    developer: ['support', 'security'],
  };
}

/** Prefer Telegram first name / Google profile-ish email; never a raw numeric id. */
export function settingsOwnerDisplayName(opts: {
  displayName?: string | null;
  signedInOwner?: string | null;
  email?: string | null;
  fallback: string;
}): string {
  const named = (opts.displayName || '').trim();
  if (named && !/^\d+$/.test(named)) return named;
  const signed = (opts.signedInOwner || '').trim();
  if (signed && !/^Telegram user \d+$/i.test(signed) && !/^\d+$/.test(signed)) return signed;
  const email = (opts.email || '').trim();
  if (email && !/^\d+$/.test(email)) return email;
  return opts.fallback;
}

/**
 * Applies a chosen display language with an optimistic-then-confirm flow that
 * owns BOTH the live-UI locale setter and the persistence call:
 *  - immediately flips the in-memory locale (`setLocale(next)`) for instant UI,
 *  - persists the choice so server-authored Channel-B strings follow,
 *  - if the persist throws/rejects, REVERTS `setLocale` back to `previous` so the
 *    chrome matches the still-unchanged backend setting (and `errorLocale`),
 *    avoiding a transient divergence until reload.
 *
 * The persist callback is expected to surface its own error message; this helper
 * does not swallow the failure — it re-throws after reverting so callers can
 * observe it. A no-op persist (resolves) leaves the optimistic change in place.
 */
export async function applyDisplayLanguageChoice({
  next,
  previous,
  setLocale,
  persist,
}: {
  next: Locale;
  previous: Locale;
  setLocale: (locale: Locale) => void;
  persist?: (value: Locale) => void | Promise<void>;
}): Promise<void> {
  // (a) immediate UI effect.
  setLocale(next);
  if (next === previous || !persist) return;
  try {
    // (b) persist so Channel-B follows.
    await persist(next);
  } catch (persistError) {
    // Persist failed → revert the optimistic locale to match the unchanged backend.
    setLocale(previous);
    throw persistError;
  }
}

function ownerHealthLabel(health: string, tr: (key: string) => string): string {
  if (health === 'attention') return tr('settings.health.attention');
  if (health === 'quiet') return tr('settings.health.quiet');
  if (health === 'ready') return tr('settings.health.ready');
  return health;
}

function humanizeSettingsLabel(value: string): string {
  return value
    .replace(/[_-]+/g, ' ')
    .replace(/\b\w/g, (match) => match.toUpperCase());
}

function formatLeaseExpiry(expiresAtUnix: number, userTimezone?: string | null): string {
  return formatInUserTz(new Date(expiresAtUnix * 1000), userTimezone, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

function formatSettingsDateTime(unixSeconds: number, userTimezone?: string | null): string {
  return formatInUserTz(new Date(unixSeconds * 1000), userTimezone, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

// Build stamp from vite.config `define`; `typeof` guards the non-vite test build
// where the global is absent (resolves to 'dev' there).
const APP_VERSION = typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : 'dev';

export function SettingsView({
  jevAccess,
  email,
  displayName,
  sources,
  projection,
  telegramRail = null,
  costSavingBusy,
  snapshotMessage,
  snapshotAvailable,
  snapshotBusy,
  logoutBusy = false,
  webPushStatus,
  webPushDeviceSubscribed = null,
  webPushBusy = false,
  webPushMessage,
  codexStatus,
  codexDeviceCode,
  codexBusy = false,
  codexMessage,
  grokStatus,
  grokDeviceCode,
  grokBusy = false,
  grokMessage,
  avatarProfile,
  avatarAutoRefresh,
  avatarGenerationBusy = false,
  debugLeaseBusy = false,
  debugLeases,
  debugGrantToken,
  debugJobBusy = false,
  debugJobs = [],
  onPauseCostSaving,
  onResumeCostSaving,
  onEnableWebPush,
  onDisableWebPush,
  onTestWebPush,
  onCreateSnapshot,
  onCreateDebugLease,
  onRevokeDebugLease,
  onRunDebugJob,
  onLogout,
  onStartCodexDeviceCode,
  onPollCodexDeviceCode,
  onUseCodexForChat,
  onSetCodexModel,
  onStopUsingCodexForChat,
  onStartGrokDeviceCode,
  onPollGrokDeviceCode,
  onRevokeGrok,
  onUseGrokForChat,
  onStopUsingGrokForChat,
  onSetAvatar,
  onResetAvatar,
  onToggleAegisAvatarAutoRefresh,
  onGenerateAegisAvatar,
  onSetDisplayLanguage,
  onSetLlmProvider,
  onSetFallbackModel,
  onSetWebSearchProvider,
  onSetWebSearchModel,
  onLoadModelCatalog,
  modelCatalog,
  onSetPaidApiFallback,
}: {
  email?: string;
  /** Telegram first name or Google profile name from the live session. */
  displayName?: string;
  sources: OwnerWorkQueue['sources'];
  projection: VNextSettingsProjection | null;
  /** Telegram rail from the status projection; drives the notifications note. */
  telegramRail?: VNextTelegramRail | null;
  costSavingBusy: boolean;
  snapshotMessage?: string;
  snapshotAvailable: boolean;
  snapshotBusy?: boolean;
  logoutBusy?: boolean;
  webPushStatus: VNextWebPushStatus | null;
  webPushDeviceSubscribed?: boolean | null;
  webPushBusy?: boolean;
  webPushMessage?: string;
  codexStatus: VNextCodexStatus | null;
  codexDeviceCode: VNextCodexDeviceCodeResponse | null;
  codexBusy?: boolean;
  codexMessage?: string;
  grokStatus: VNextGrokStatus | null;
  grokDeviceCode: VNextGrokDeviceCodeResponse | null;
  grokBusy?: boolean;
  grokMessage?: string;
  avatarProfile: AvatarProfile;
  avatarAutoRefresh: VNextAvatarAutoRefreshProjection | null;
  avatarGenerationBusy?: boolean;
  debugLeaseBusy?: boolean;
  debugLeases: VNextDebugLease[];
  debugGrantToken?: string | null;
  debugJobBusy?: boolean;
  debugJobs?: VNextDebugJob[];
  onPauseCostSaving: () => void;
  onResumeCostSaving: () => void;
  onEnableWebPush: () => void;
  onDisableWebPush: () => void;
  onTestWebPush: () => void;
  onCreateSnapshot: () => void;
  onCreateDebugLease: (
    scope: 'avatar' | 'reminders' | 'work_item' | 'chat' | 'settings' | 'notifications' | 'provider' | 'status' | 'general',
    reason: string,
    durationMinutes: number,
    itemId?: string | null,
  ) => void;
  onRevokeDebugLease: (leaseId: string) => void;
  onRunDebugJob: () => void;
  onLogout: () => void;
  onStartCodexDeviceCode: () => void;
  onPollCodexDeviceCode: () => void;
  onUseCodexForChat: () => void;
  onSetCodexModel: (model: string, reasoningEffort: string) => void;
  onStopUsingCodexForChat: () => void;
  onStartGrokDeviceCode: () => void;
  onPollGrokDeviceCode: () => void;
  onRevokeGrok: () => void;
  onUseGrokForChat: () => void;
  onStopUsingGrokForChat: () => void;
  onSetAvatar: (role: AvatarRole, dataUrl: string) => void;
  onResetAvatar: (role: AvatarRole) => void;
  onToggleAegisAvatarAutoRefresh: (enabled: boolean, promptHint: string) => void;
  onGenerateAegisAvatar: (promptHint: string) => void;
  /**
   * Persists the chosen display language so server-authored Channel-B strings
   * follow the same setting the webapp chrome uses. May return a promise so the
   * toggle can await it and revert the optimistic locale flip on failure.
   * Optional so existing callers/tests that do not exercise the language toggle
   * still type-check.
   */
  jevAccess?: { initData: string; csrfToken?: string };
  onSetDisplayLanguage?: (value: Locale) => void | Promise<void>;
  /**
   * Persists the owner's chosen LLM provider (`'openrouter'` | `'codex_oauth'` |
   * `'xai_subscription'`). Optional so existing callers/tests that do not
   * exercise the provider selector still type-check; the selector is disabled
   * when absent.
   */
  onSetLlmProvider?: (value: string, model?: string) => void | Promise<void>;
  /**
   * Persists the owner's chosen LLM model id (an arbitrary OpenRouter-style
   * slug). Optional so existing callers/tests that do not exercise the model
   * field still type-check; the field is disabled when absent.
   */
  onSetLlmModel?: (value: string) => void | Promise<void>;
  /**
   * Persists the backup model id (empty = no backup). Optional so existing
   * callers/tests still type-check.
   */
  onSetFallbackModel?: (value: string) => void | Promise<void>;
  /**
   * Loads the provider-backed model catalog when 高级 opens. Optional so
   * render tests can pass `modelCatalog` instead of fetching.
   */
  onLoadModelCatalog?: () => Promise<VNextModelCatalog>;
  /**
   * Optional preloaded catalog (tests). When absent, Settings fetches via
   * `onLoadModelCatalog` the first time the 高级 block opens.
   */
  modelCatalog?: VNextModelCatalog | null;
  /**
   * Persists whether the owner wants paid-API fallback when the configured
   * provider is unavailable. Optional for the same backward-compat reason; the
   * toggle is disabled when absent.
   */
  onSetPaidApiFallback?: (enabled: boolean) => void | Promise<void>;
  /**
   * Persists `web_search_provider` (`xai_subscription` | `openrouter`).
   * Optional so existing callers still type-check; the control is disabled
   * when absent.
   */
  onSetWebSearchProvider?: (value: string) => void | Promise<void>;
  /** Persists `web_search_model` for the current search provider. */
  onSetWebSearchModel?: (value: string) => void | Promise<void>;
}) {
  const userTimezone = useUserTimezone();
  const { locale, setLocale } = useLocale();
  const tr = useT();
  const aegisAvatarInputRef = useRef<HTMLInputElement | null>(null);
  const ownerAvatarInputRef = useRef<HTMLInputElement | null>(null);
  const [avatarMessage, setAvatarMessage] = useState('');
  const [avatarPromptHint, setAvatarPromptHint] = useState(avatarAutoRefresh?.prompt_hint ?? '');
  const [previewAvatarRole, setPreviewAvatarRole] = useState<AvatarRole | null>(null);
  const [debugLeaseDurationMinutes, setDebugLeaseDurationMinutes] = useState(8 * 60);
  const [debugLeaseScope, setDebugLeaseScope] = useState<
    | 'avatar'
    | 'reminders'
    | 'work_item'
    | 'chat'
    | 'settings'
    | 'notifications'
    | 'provider'
    | 'status'
    | 'general'
  >('general');
  const [debugGrantCopied, setDebugGrantCopied] = useState(false);
  const [expandedDebugJobId, setExpandedDebugJobId] = useState<string | null>(null);
  const [loadedCatalog, setLoadedCatalog] = useState<VNextModelCatalog | null>(modelCatalog ?? null);
  const [catalogBusy, setCatalogBusy] = useState(false);
  const [modelFilter, setModelFilter] = useState('');
  // Seed from the preloaded catalog so SSR/static first paint (and tests) show
  // the current backup before the sync effect runs.
  const [fallbackProvider, setFallbackProvider] = useState(
    () => modelCatalog?.fallback.provider || '',
  );
  const [fallbackModelDraft, setFallbackModelDraft] = useState(
    () => modelCatalog?.fallback.model || '',
  );
  const ownerLabel = settingsOwnerDisplayName({
    displayName,
    signedInOwner: projection?.signed_in_owner,
    email,
    fallback: tr('settings.account.youFallback'),
  });
  const visibleSources = sources
    .map((s) => sourceStatusView(s, locale))
    // Filter on the English `source.label` identity (kept English by
    // sourceStatusView). Section labels localize uniformly at the render site.
    .filter((source) => (CONNECTION_SOURCE_LABELS as readonly string[]).includes(source.label));
  const telegramNote = settingsTelegramNote(telegramRail, nowUnixSeconds(), locale);
  const serviceSections = projection?.connected_services?.length
    ? projection.connected_services.filter((section) => section.label !== 'Telegram')
    : visibleSources.map((source) => ({
        label: source.label,
        health: source.healthy ? 'ready' : 'attention',
        owner_message: source.reason || source.statusLabel,
        owner_next_step: source.nextStep ?? null,
      }));
  // Fallback section LABELS stay English (the identity); they localize for
  // display via localizedSettingsSectionLabel at the render site. owner_message
  // is owner-facing copy → localized here.
  const securitySections = projection?.security?.length
    ? projection.security
    : [
        {
          label: 'Signed-in session',
          health: 'ready',
          owner_message: t('settings.security.signedInSession.message', locale),
        },
        {
          label: 'Item access',
          health: 'ready',
          owner_message: t('settings.security.itemAccess.message', locale),
        },
      ];
  const costSavingPaused = projection?.cost_saving_paused
    ?? projection?.notification_preferences?.find((section) => section.label === 'Cost controls')?.health === 'quiet';
  // LLM provider selector + paid-API fallback bind to the settings projection.
  // Default an absent provider to the historical 'openrouter'; coerce an
  // unrecognized server value back into a known option so the selector never
  // renders a phantom controlled value.
  const savedProvider = (LLM_PROVIDER_OPTIONS as readonly string[]).includes(projection?.llm_provider ?? '')
    ? (projection!.llm_provider as string)
    : 'openrouter';
  const [providerDraft, setProviderDraft] = useState<string | null>(null);
  const [modelDraft, setModelDraft] = useState<string | null>(null);
  const [providerBusy, setProviderBusy] = useState(false);
  const [callBusy, setCallBusy] = useState(false);
  const [callNotice, setCallNotice] = useState('');
  const [providerNotice, setProviderNotice] = useState('');
  const llmProvider = providerDraft ?? savedProvider;
  const paidApiFallbackEnabled = projection?.paid_api_fallback_enabled ?? false;
  const savedSearchProvider = projection?.web_search_provider || 'auto';
  const searchUnsupported = !(WEB_SEARCH_PROVIDER_OPTIONS as readonly string[]).includes(savedSearchProvider);
  const webSearchProvider = searchUnsupported ? 'unsupported' : savedSearchProvider;
  const webSearchModel = (projection?.web_search_model || '').trim();
  const searchModelChoices = [...(projection?.web_search_model_options ?? [])].filter((id) => id.trim().length > 0);
  if (webSearchModel && !searchModelChoices.includes(webSearchModel)) searchModelChoices.unshift(webSearchModel);
  const searchNeedsSignIn = webSearchProvider === 'xai_subscription' && grokStatus?.connected === false;
  const currentLlmModel = modelDraft ?? projection?.llm_model ?? loadedCatalog?.current.model ?? '';
  const catalog = loadedCatalog ?? modelCatalog ?? null;
  const currentProviderCatalog: VNextProviderCatalog | undefined = catalog?.providers.find((entry) => entry.id === llmProvider);
  const currentProviderModels = currentProviderCatalog?.models ?? [];
  // Prefer THIS provider's catalog default. catalog.current.effective_model
  // belongs to catalog.current.provider and is stale after a provider switch.
  const providerDefaultModel = (
    currentProviderCatalog?.default_model
    || (catalog?.current.provider === llmProvider ? catalog?.current.effective_model : '')
    || ''
  ).trim();
  const providerDefaultOptionLabel = providerDefaultModel
    ? tf('settings.model.providerDefaultNamed', locale, { model: providerDefaultModel })
    : tr('settings.model.providerDefault');
  const filteredCurrentModels = modelFilter
    ? currentProviderModels.filter((entry) => entry.display_name.includes(modelFilter) || entry.id.includes(modelFilter))
    : currentProviderModels;
  const connectedProviders = (catalog?.providers ?? []).filter((entry) => entry.connected);
  const recommendedApiEntry = catalog?.providers.find((entry) => entry.id === 'openrouter' && entry.connected)
    ?.models.find((model) => model.id === catalog?.recommended_api_model);
  const recommendedApiModel = recommendedApiEntry?.id || '';
  const recommendedApiName = recommendedApiEntry?.display_name || recommendedApiModel;
  const recommendedApiAvailable = !!recommendedApiModel;
  const fallbackProviderCatalog = catalog?.providers.find((entry) => entry.id === fallbackProvider);
  const fallbackModels = fallbackProviderCatalog?.models ?? [];
  // Only a successful verified save updates the active summary.
  const fallbackSummaryProvider = catalog?.fallback.provider || '';
  const fallbackSummaryModel = catalog?.fallback.model || '';
  const codexSelected = llmProvider === 'codex_oauth';
  const grokConnected = Boolean(grokStatus?.connected);
  const selectedCodexModel = (codexStatus?.model_options ?? []).find((option) => option.slug === codexStatus?.model)
    ?? (codexStatus?.model_options ?? [])[0];
  const codexReasoningLevels = selectedCodexModel?.supported_reasoning_levels?.length
    ? selectedCodexModel.supported_reasoning_levels
    : ['low', 'medium', 'high', 'xhigh'];
  const routedSnapshotMessage = routeSettingsSnapshotMessage(snapshotMessage);
  const avatarInlineMessage = avatarGenerationBusy
    ? tr('settings.avatar.generating')
    : avatarMessage || (routedSnapshotMessage === 'avatar' ? snapshotMessage : '');
  const costInlineMessage = costSavingBusy
    ? tr('settings.spending.updating')
    : routedSnapshotMessage === 'cost' ? snapshotMessage : '';
  const supportInlineMessage = snapshotBusy
    ? tr('settings.support.savingSnapshot')
    : routedSnapshotMessage === 'support' ? snapshotMessage : '';
  const debugLeaseDurationOptions = [
    { label: '1h', minutes: 60 },
    { label: '8h', minutes: 8 * 60 },
    { label: '24h', minutes: 24 * 60 },
  ];
  // Grantable scopes (must mirror the backend's `sanitize_debug_lease_scope`).
  // Every scope is inspect-only (see App.tsx onCreateDebugLease).
  const debugLeaseScopeOptions = [
    'chat',
    'reminders',
    'work_item',
    'settings',
    'notifications',
    'provider',
    'status',
    'avatar',
    'general',
  ] as const;
  const activeDebugLease = debugLeases[0] ?? null;
  const latestDebugJob = debugJobs[0] ?? null;
  const debugJobHistory = debugJobs.slice(1, 6);
  const runningDebugJob = debugJobs.some((job) => job.status === 'queued' || job.status === 'running');
  const latestDebugJobEvidence = latestDebugJob ? debugJobEvidence(latestDebugJob) : null;
  const latestDebugJobExpanded = latestDebugJob ? expandedDebugJobId === latestDebugJob.id : false;

  useEffect(() => {
    setAvatarPromptHint(avatarAutoRefresh?.prompt_hint ?? '');
  }, [avatarAutoRefresh?.prompt_hint]);

  useEffect(() => {
    if (modelCatalog) setLoadedCatalog(modelCatalog);
  }, [modelCatalog]);

  useEffect(() => {
    setFallbackProvider(catalog?.fallback.provider || '');
    setFallbackModelDraft(catalog?.fallback.model || '');
  }, [catalog?.fallback.provider, catalog?.fallback.model]);

  async function ensureCatalog(force?: boolean) {
    if ((!force && loadedCatalog) || catalogBusy || !onLoadModelCatalog) return;
    setCatalogBusy(true);
    try {
      const next = await onLoadModelCatalog();
      setLoadedCatalog(next);
    } catch {
      setLoadedCatalog(null);
    } finally {
      setCatalogBusy(false);
    }
  }

  useEffect(() => {
    setDebugGrantCopied(false);
  }, [debugGrantToken]);

  async function copyDebugGrantToken() {
    if (!debugGrantToken) return;
    await navigator.clipboard.writeText(`Authorization: Bearer ${debugGrantToken}`);
    setDebugGrantCopied(true);
  }

  function submitLlmModel(value: string) {
    setModelDraft(value.trim());
    setProviderNotice('');
  }

  async function validateCallSetting(save: () => void | Promise<void>, checksService = true) {
    setCallBusy(true);
    setCallNotice(tr('settings.validation.checking'));
    try {
      await save();
      setCallNotice(tr(checksService ? 'settings.validation.saved' : 'settings.jev.saved'));
      return true;
    } catch (error) {
      setCallNotice(error instanceof Error && /Validation failed:|验证失败：/.test(error.message) ? error.message : tr('settings.validation.failed'));
      return false;
    } finally {
      setCallBusy(false);
    }
  }

  async function submitFallbackModel(provider: string, model: string) {
    if (!onSetFallbackModel) return;
    const trimmed = model.trim();
    if (provider && !trimmed) return;
    if (await validateCallSetting(() => onSetFallbackModel(provider ? trimmed : ''), !!provider)) {
      setLoadedCatalog((prev) => (prev ? { ...prev, fallback: { provider: provider || null, model: trimmed } } : prev));
    } else {
      setFallbackProvider(fallbackSummaryProvider || '');
      setFallbackModelDraft(fallbackSummaryModel);
    }
  }

  function redactedDebugGrantToken(token: string) {
    if (token.length <= 24) return 'aegis_dbg_...';
    return `${token.slice(0, 16)}...${token.slice(-8)}`;
  }

  function debugJobStatusLabel(job: VNextDebugJob) {
    if (job.status === 'passed') return tr('settings.status.passed');
    if (job.status === 'failed') return tr('settings.status.failed');
    if (job.status === 'running') return tr('settings.status.running');
    if (job.status === 'queued') return tr('settings.status.queued');
    return humanizeSettingsLabel(job.status);
  }

  function debugJobTime(job: VNextDebugJob) {
    return formatSettingsDateTime(job.completed_at_unix ?? job.started_at_unix ?? job.created_at_unix, userTimezone);
  }

  function debugJobEvidence(job: VNextDebugJob): VNextDebugJobEvidence {
    return job.evidence ?? {
      schema_version: 1,
      verdict: job.status === 'passed' && job.kind === 'webapp_e2e_suite' ? 'deploy_ready' : job.status === 'passed' ? 'path_passed' : job.status === 'failed' ? 'blocked' : job.status,
      owner_summary: job.summary,
      phases: [],
    };
  }

  function debugJobVerdictLabel(evidence: VNextDebugJobEvidence) {
    if (evidence.verdict === 'deploy_ready') return tr('settings.verdict.deployReady');
    if (evidence.verdict === 'path_passed') return tr('settings.verdict.pathPassed');
    if (evidence.verdict === 'blocked') return tr('settings.verdict.blocked');
    if (evidence.verdict === 'running') return tr('settings.verdict.running');
    if (evidence.verdict === 'queued') return tr('settings.verdict.queued');
    return humanizeSettingsLabel(evidence.verdict);
  }

  function debugJobPhaseStatusLabel(phase: VNextDebugJobEvidencePhase) {
    if (phase.status === 'passed') return tr('settings.status.passed');
    if (phase.status === 'failed') return tr('settings.status.failed');
    if (phase.status === 'running') return tr('settings.status.running');
    if (phase.status === 'queued') return tr('settings.status.queued');
    return tr('settings.phase.needsReview');
  }

  async function handleAvatarFile(role: AvatarRole, file?: File | null) {
    if (!file) return;
    setAvatarMessage('');
    try {
      const dataUrl = await readAvatarImageFile(file, tr);
      onSetAvatar(role, dataUrl);
      setAvatarMessage(role === 'aegis' ? tr('settings.avatar.aegisUpdated') : tr('settings.avatar.ownerUpdated'));
    } catch (error) {
      setAvatarMessage(error instanceof Error ? error.message : tr('settings.avatar.couldNotUseImage'));
    }
  }

  const supportSection = snapshotAvailable ? (
    <section className="vnext-settings-section vnext-settings-support-priority" data-settings-section="support">
      <div className="vnext-settings-section-body">
      <div className="vnext-settings-support-card">
        <div>
          <strong>{tr('settings.support.backendE2eTitle')}</strong>
          <p>
            {tr('settings.support.terminalGrantPrefix')}{debugLeases.length > 0
              ? tf(debugLeases.length === 1 ? 'settings.support.activeLeaseSingular' : 'settings.support.activeLeasePlural', locale, { count: String(debugLeases.length) })
              : tr('settings.support.noneActive')}.
          </p>
          {supportInlineMessage && <em role="status">{supportInlineMessage}</em>}
        </div>
        <div className="vnext-settings-support-actions">
          <VNextButton
            variant="secondary"
            size="sm"
            disabled={debugJobBusy || runningDebugJob}
            onClick={onRunDebugJob}
          >
            {debugJobBusy || runningDebugJob ? tr('settings.support.checking') : tr('settings.support.runBackendE2e')}
          </VNextButton>
          <VNextButton
            variant="secondary"
            size="sm"
            disabled={snapshotBusy}
            onClick={onCreateSnapshot}
          >
            {snapshotBusy ? tr('settings.support.saving') : tr('settings.support.saveSnapshot')}
          </VNextButton>
        </div>
      </div>
      {latestDebugJob && latestDebugJobEvidence && (
        <article className={classNames('vnext-debug-job vnext-debug-job-latest', `status-${latestDebugJob.status}`)} aria-label={tr('settings.e2e.latestResultLabel')}>
          <div className="vnext-debug-evidence-header">
            <span>
              <strong>{tr('settings.e2e.latestPrefix')}{debugJobVerdictLabel(latestDebugJobEvidence)}</strong>
              <em>{debugJobTime(latestDebugJob)}</em>
            </span>
            <span className={classNames('vnext-debug-evidence-badge', `verdict-${latestDebugJobEvidence.verdict}`)}>
              {debugJobStatusLabel(latestDebugJob)}
            </span>
          </div>
          <p className="vnext-debug-evidence-summary">{latestDebugJobEvidence.owner_summary || latestDebugJob.summary}</p>
          {latestDebugJobEvidence.phases.length > 0 && (
            <ol className="vnext-debug-phase-list" aria-label={tr('settings.e2e.phasesLabel')}>
              {latestDebugJobEvidence.phases.map((phase) => (
                <li key={phase.id} className={classNames('vnext-debug-phase', `status-${phase.status}`)}>
                  <span className="vnext-debug-phase-dot" aria-hidden="true" />
                  <div>
                    <strong>{phase.label}</strong>
                    <em>{debugJobPhaseStatusLabel(phase)}</em>
                    <p>{phase.summary}</p>
                  </div>
                </li>
              ))}
            </ol>
          )}
          <button
            type="button"
            className="vnext-debug-log-toggle"
            aria-expanded={latestDebugJobExpanded}
            onClick={() => setExpandedDebugJobId(latestDebugJobExpanded ? null : latestDebugJob.id)}
          >
            {latestDebugJobExpanded ? tr('settings.e2e.hideRawLog') : tr('settings.e2e.showRawLog')}
          </button>
          {latestDebugJobExpanded && (
            <pre className="vnext-debug-job-log">{latestDebugJob.log_text || tr('settings.e2e.noLogYet')}</pre>
          )}
        </article>
      )}
      {debugJobHistory.length > 0 && (
        <details className="vnext-debug-history vnext-disclosure">
          <summary className="vnext-disclosure-summary">{tf('settings.e2e.previousRuns', locale, { count: String(debugJobHistory.length) })}</summary>
          <div className="vnext-debug-history-list" aria-label={tr('settings.e2e.previousRunsLabel')}>
            {debugJobHistory.map((job) => {
              const evidence = debugJobEvidence(job);
              return (
                <article key={job.id} className={classNames('vnext-debug-history-row', `status-${job.status}`)}>
                  <span>
                    <strong>{debugJobVerdictLabel(evidence)}</strong>
                    <em>{debugJobTime(job)}</em>
                  </span>
                  <p>{evidence.owner_summary || job.summary}</p>
                  <span className={classNames('vnext-debug-evidence-badge', `verdict-${evidence.verdict}`)}>
                    {debugJobStatusLabel(job)}
                  </span>
                </article>
              );
            })}
          </div>
        </details>
      )}
      {debugGrantToken && (
        <div className="vnext-debug-grant-token" aria-label={tr('settings.grant.tokenLabel')}>
          <div>
            <strong>{tr('settings.grant.tokenReady')}</strong>
            <p>{tr('settings.grant.tokenDescription')}</p>
            <code>Authorization: Bearer {redactedDebugGrantToken(debugGrantToken)}</code>
          </div>
          <VNextButton variant="secondary" size="sm" onClick={() => void copyDebugGrantToken()}>
            {debugGrantCopied ? tr('settings.grant.copied') : tr('settings.grant.copy')}
          </VNextButton>
        </div>
      )}
      {activeDebugLease && !debugGrantToken && (
        <p className="vnext-debug-lease-empty vnext-debug-token-unavailable">
          {tr('settings.grant.tokenUnrecoverable')}
        </p>
      )}
      <details className="vnext-debug-terminal-grant vnext-disclosure">
        <summary className="vnext-disclosure-summary">{tr('settings.terminal.summary')}</summary>
        <div className="vnext-debug-terminal-body">
          {!activeDebugLease && (
            <>
              <div className="vnext-debug-duration-picker" aria-label={tr('settings.terminal.durationLabel')}>
                <span>{tr('settings.terminal.duration')}</span>
                <div role="group" aria-label={tr('settings.terminal.chooseDuration')}>
                  {debugLeaseDurationOptions.map((option) => (
                    <button
                      key={option.minutes}
                      type="button"
                      className={classNames('vnext-chip-button', debugLeaseDurationMinutes === option.minutes && 'selected')}
                      aria-pressed={debugLeaseDurationMinutes === option.minutes}
                      disabled={debugLeaseBusy}
                      onClick={() => setDebugLeaseDurationMinutes(option.minutes)}
                    >
                      {option.label}
                    </button>
                  ))}
                </div>
              </div>
              <div className="vnext-debug-scope-picker" aria-label={tr('settings.terminal.scopeLabel')}>
                <span>{tr('settings.terminal.scope')}</span>
                <select
                  className="vnext-select vnext-debug-scope-select"
                  value={debugLeaseScope}
                  disabled={debugLeaseBusy}
                  onChange={(event) =>
                    setDebugLeaseScope(event.target.value as typeof debugLeaseScope)
                  }
                >
                  {debugLeaseScopeOptions.map((scopeOption) => (
                    <option key={scopeOption} value={scopeOption}>
                      {humanizeSettingsLabel(scopeOption)}
                    </option>
                  ))}
                </select>
              </div>
              <VNextButton
                variant="secondary"
                size="sm"
                disabled={debugLeaseBusy}
                onClick={() =>
                  onCreateDebugLease(
                    debugLeaseScope,
                    `Owner granted temporary ${humanizeSettingsLabel(debugLeaseScope)} terminal access.`,
                    debugLeaseDurationMinutes,
                  )
                }
              >
                {debugLeaseBusy ? tr('settings.terminal.opening') : tr('settings.terminal.openGrant')}
              </VNextButton>
            </>
          )}
          {debugLeases.length === 0 && (
            <p className="vnext-debug-lease-empty">{tr('settings.terminal.noneActive')}</p>
          )}
          {debugLeases.length > 0 && (
            <div className="vnext-debug-lease-list" aria-label={tr('settings.terminal.activeGrantsLabel')}>
              {debugLeases.map((lease) => (
                <article key={lease.id}>
                  <div>
                    <strong>{humanizeSettingsLabel(lease.scope)}{tr('settings.terminal.grantSuffix')}</strong>
                    <p>{lease.reason}</p>
                    <em>{tr('settings.terminal.expiresPrefix')}{formatLeaseExpiry(lease.expires_at_unix, userTimezone)}</em>
                  </div>
                  <VNextButton variant="secondary" size="sm" disabled={debugLeaseBusy} onClick={() => onRevokeDebugLease(lease.id)}>
                    {tr('settings.terminal.revoke')}
                  </VNextButton>
                </article>
              ))}
            </div>
          )}
        </div>
      </details>
      </div>
    </section>
  ) : null;

  return (
    <section className="vnext-stack">
      <div data-settings-tier="essentials" className="vnext-settings-tier essentials">
      <section className="vnext-settings-section vnext-settings-you" data-settings-section="account">
        <div className="vnext-settings-owner-card" aria-label={tr('settings.account.label')}>
          <div>
            <span>{tr('settings.account')}</span>
            <strong>{ownerLabel}</strong>
          </div>
          <div data-settings-section="logout" aria-label={tr('settings.logout')}>
            <VNextButton variant="destructive" size="sm" disabled={logoutBusy} onClick={onLogout}>
              {logoutBusy ? tr('settings.loggingOut') : tr('settings.logout')}
            </VNextButton>
          </div>
        </div>
      </section>
      <section className="vnext-settings-section" id="vnext-settings-language" data-settings-section="language">
        <h2>{tr('settings.language')}</h2>
        <div className="vnext-settings-section-body">
        <VNextSegmented
          className="vnext-settings-language-toggle"
          mode="radio"
          size="sm"
          fullWidth
          label={tr('settings.language.caption')}
          ariaLabel={tr('settings.language.caption')}
          value={locale}
          onChange={(value) => {
            // No-op when re-selecting the current locale (the previous <button>
            // disabled the active one); only an actual switch persists.
            if (value === locale) return;
            // Optimistic locale flip + persist; reverts the UI locale to the
            // previous value if persistence fails (handler owns both calls).
            void applyDisplayLanguageChoice({
              next: value as Locale,
              previous: locale,
              setLocale,
              persist: onSetDisplayLanguage,
            });
          }}
          options={(['zh', 'en'] as Locale[]).map((value) => ({
            value,
            label: tr(`settings.language.${value}`),
          }))}
        />
        </div>
      </section>
      <section className="vnext-settings-section" id="vnext-settings-alerts" data-settings-section="alerts">
        <h2>{tr('settings.notifications.kicker')}</h2>
        <div className="vnext-settings-section-body" data-settings-section="push">
          <WebPushSettingsCard
            status={webPushStatus}
            deviceSubscribed={webPushDeviceSubscribed}
            busy={webPushBusy}
            message={webPushMessage}
            onEnable={onEnableWebPush}
            onDisable={onDisableWebPush}
            onTest={onTestWebPush}
          />
          {telegramNote && (
            <p
              className={classNames('vnext-settings-telegram-note', telegramNote.tone === 'warn' && 'warn')}
              data-telegram-rail={telegramNote.state}
            >
              {telegramNote.text}
            </p>
          )}
        </div>
      </section>
      <section className="vnext-settings-section" id="vnext-settings-services" data-settings-section="services">
        <h2>{tr('settings.services.kicker')}</h2>
        {serviceSections.length === 0 && !grokStatus && !codexStatus ? (
          <div className="vnext-settings-section-body"><p className="vnext-muted">{tr('settings.services.none')}</p></div>
        ) : (
          <div className="vnext-settings-section-body vnext-settings-connection-list">
            {serviceSections.map((source) => {
              const needsOwner = source.health !== 'ready';
              const message = needsOwner
                ? [ownerMessageForProjectionSection(source, locale), ownerNextStepForProjectionSection(source, locale)]
                  .filter((value) => Boolean(value && String(value).trim()))
                  .join(' ')
                : null;
              return (
                <ConnectionRow
                  key={source.label}
                  name={localizedSettingsSectionLabel(source.label, locale)}
                  chip={ownerHealthLabel(source.health, tr)}
                  attention={source.health === 'attention'}
                  message={message}
                />
              );
            })}
            <ProviderConnectionRow
              name={tr('settings.provider.option.xai_subscription')}
              connected={Boolean(grokStatus?.connected)}
              pending={Boolean(grokStatus?.pending_device_code || grokDeviceCode)}
              activeForChat={Boolean(grokStatus?.active_for_chat)}
              providerBusy={grokBusy}
              ownerMessage={grokStatus?.connected ? null : (grokStatus?.owner_message ?? grokMessage ?? null)}
              deviceCode={grokDeviceCode}
              openLabel={tr('settings.provider.openXAI')}
              onConnect={onStartGrokDeviceCode}
              onPoll={onPollGrokDeviceCode}
              onUseForChat={onUseGrokForChat}
              onStopUsingForChat={onStopUsingGrokForChat}
              onDisconnect={onRevokeGrok}
            />
            <ProviderConnectionRow
              name={tr('settings.provider.option.codex_oauth')}
              connected={Boolean(codexStatus?.connected)}
              pending={Boolean(codexDeviceCode)}
              activeForChat={Boolean(codexStatus?.active_for_chat)}
              providerBusy={codexBusy}
              ownerMessage={codexStatus?.connected ? null : (codexStatus?.owner_message ?? codexMessage ?? null)}
              deviceCode={codexDeviceCode}
              openLabel={tr('settings.provider.openOpenAI')}
              connectDisabled={!codexStatus?.binary_available}
              onConnect={onStartCodexDeviceCode}
              onPoll={onPollCodexDeviceCode}
              onUseForChat={onUseCodexForChat}
              onStopUsingForChat={onStopUsingCodexForChat}
            />
          </div>
        )}
      </section>
      <section className="vnext-settings-section" id="vnext-settings-runtime" data-settings-section="runtime">
        <h2>{tr('settings.runtime.heading')}</h2>
        <div className="vnext-settings-section-body">
          <SettingsToggleLine
            label={tr('settings.spending.pauseTitle')}
            description={costSavingPaused ? tr('settings.spending.saveModeOn') : tr('settings.spending.saveModeOff')}
            checked={costSavingPaused}
            disabled={costSavingBusy}
            onChange={(checked) => {
              if (checked) onPauseCostSaving();
              else onResumeCostSaving();
            }}
          />
          {costInlineMessage && <p className="vnext-settings-inline-note" role="status">{costInlineMessage}</p>}
          {snapshotAvailable ? (
            <div className="vnext-settings-toggle-line" data-settings-action="support-snapshot">
              <span>
                <strong>{tr('chatHome.account.saveSnapshot')}</strong>
                <p>{tr('settings.support.snapshotDescription')}</p>
              </span>
              <VNextButton
                variant="secondary"
                size="sm"
                disabled={snapshotBusy}
                onClick={onCreateSnapshot}
              >
                {snapshotBusy ? tr('settings.support.saving') : tr('chatHome.account.saveSnapshot')}
              </VNextButton>
            </div>
          ) : null}
        </div>
      </section>
      </div>

      <details
        data-settings-tier="advanced"
        className="vnext-settings-tier advanced vnext-disclosure"
        onToggle={(event) => {
          if (event.currentTarget.open) void ensureCatalog();
        }}
      >
        <summary className="vnext-disclosure-summary">
          <span className="vnext-disclosure-copy">
            <strong>{tr('settings.tier.advanced')}</strong>
            <em className="vnext-disclosure-hint">{tr('settings.tier.advancedHint')}</em>
          </span>
        </summary>
      <section className="vnext-settings-section" data-settings-section="providers" id="vnext-settings-provider-select">
        <h2>{tr('settings.ai.heading')}</h2>
        <div className="vnext-settings-section-body">
        <p className="vnext-settings-inline-note">{tr('settings.validation.notice')}</p>
        <VNextSegmented
          className="vnext-settings-provider-toggle"
          mode="radio"
          size="sm"
          fullWidth
          label={tr('settings.provider.label')}
          ariaLabel={tr('settings.provider.selectLabel')}
          value={llmProvider}
          onChange={(value) => {
            setModelFilter('');
            void ensureCatalog(true);
            setProviderDraft(value);
            setModelDraft('');
            setProviderNotice('');
          }}
          options={LLM_PROVIDER_DISPLAY_ORDER.map((option) => ({
            value: option,
            label: tr(`settings.provider.option.${option}`),
            disabled: !onSetLlmProvider || providerBusy,
          }))}
        />
        {/* The model label and the <select> are linked explicitly (htmlFor /
            aria-labelledby): a wrapping <label> that also contained the filter
            <input> gave the select an accessible name polluted by (or lost to)
            the filter's value once the input appeared. */}
        <div className="vnext-provider-controls vnext-model-controls" aria-label={tr('settings.model.label')}>
          <div className="vnext-model-field">
            <label id="vnext-settings-model-label" htmlFor="vnext-settings-model-select">{tr('settings.model.label')}</label>
            {currentProviderModels.length > 20 && (
              <input
                type="search"
                className="vnext-model-filter"
                value={modelFilter}
                placeholder={tr('settings.model.filter')}
                aria-label={tr('settings.model.filter')}
                onChange={(event) => setModelFilter(event.currentTarget.value)}
              />
            )}
            <select
              id="vnext-settings-model-select"
              className="vnext-select vnext-model-select"
              aria-labelledby="vnext-settings-model-label"
              value={currentLlmModel}
              disabled={!onSetLlmProvider || providerBusy || currentProviderModels.length === 0}
              onChange={(event) => submitLlmModel(event.currentTarget.value)}
            >
              <option value="">{providerDefaultOptionLabel}</option>
              {filteredCurrentModels.map((entry) => (
                <option key={entry.id} value={entry.id}>{entry.display_name}</option>
              ))}
            </select>
          </div>
        </div>
        {currentProviderModels.length === 0 && (
          <p className="vnext-settings-inline-note">{tr('settings.model.emptyList')}</p>
        )}
        {llmProvider === 'openrouter' && recommendedApiAvailable && (
          <VNextButton variant="ghost" className="vnext-model-recommendation" disabled={!onSetLlmProvider || providerBusy || currentLlmModel === recommendedApiModel}
            onClick={() => submitLlmModel(recommendedApiModel)}>
            {tf('settings.model.useRecommended', locale, { model: recommendedApiName })}
          </VNextButton>
        )}
        <div className="vnext-jev-actions">
          <VNextButton loading={providerBusy} disabled={!onSetLlmProvider || providerBusy || (providerDraft === null && modelDraft === null) || (!currentLlmModel && !providerDefaultModel)} onClick={async () => {
            setProviderBusy(true); setProviderNotice('');
            try {
              await onSetLlmProvider?.(llmProvider, currentLlmModel);
              setProviderDraft(null); setModelDraft(null); setProviderNotice('settings.validation.saved');
            } catch (error) { setProviderNotice(error instanceof Error && /Validation failed:|验证失败：/.test(error.message) ? error.message : 'settings.validation.failed'); }
            finally { setProviderBusy(false); }
          }}>{tr(providerBusy ? 'settings.validation.checking' : 'settings.validation.save')}</VNextButton>
          <VNextButton variant="ghost" disabled={providerBusy || (providerDraft === null && modelDraft === null)} onClick={() => { setProviderDraft(null); setModelDraft(null); setProviderNotice(''); }}>{tr('settings.jev.cancel')}</VNextButton>
        </div>
        {providerNotice && <p role="status">{tr(providerNotice)}</p>}
        {jevAccess && <JevSettings initData={jevAccess.initData} csrfToken={jevAccess.csrfToken} />}
        <div className="vnext-fallback-controls" aria-label={tr('settings.fallback.label')}>
          <span>{tr('settings.fallback.label')}</span>
          <p className="vnext-settings-inline-note">{tr('settings.fallback.validationHint')}</p>
          {recommendedApiAvailable && (
            <VNextButton variant="ghost" className="vnext-model-recommendation" disabled={!onSetFallbackModel || callBusy || fallbackSummaryModel === recommendedApiModel}
              onClick={() => {
                setFallbackProvider('openrouter');
                setFallbackModelDraft(recommendedApiModel);
                submitFallbackModel('openrouter', recommendedApiModel);
              }}>
              {tf('settings.fallback.useRecommended', locale, { model: recommendedApiName })}
            </VNextButton>
          )}
          <p className="vnext-fallback-summary">
            {fallbackSummaryModel
              ? tf('settings.fallback.summary', locale, {
                provider: providerCatalogLabel(fallbackSummaryProvider, locale),
                model: fallbackSummaryModel,
              })
              : tr('settings.fallback.none')}
          </p>
          <div className="vnext-fallback-selects">
            <select
              className="vnext-select vnext-model-select"
              value={fallbackProvider}
              disabled={!onSetFallbackModel || callBusy}
              onChange={(event) => {
                const nextProvider = event.currentTarget.value;
                setFallbackProvider(nextProvider);
                setFallbackModelDraft('');
                if (!nextProvider) submitFallbackModel('', '');
              }}
            >
              <option value="">{tr('settings.fallback.off')}</option>
              {connectedProviders.map((entry) => (
                <option key={entry.id} value={entry.id}>{providerCatalogLabel(entry.id, locale)}</option>
              ))}
            </select>
            <select
              className="vnext-select vnext-model-select"
              value={fallbackModelDraft}
              disabled={!onSetFallbackModel || callBusy || !fallbackProvider || fallbackModels.length === 0}
              onChange={(event) => {
                const nextModel = event.currentTarget.value;
                setFallbackModelDraft(nextModel);
                submitFallbackModel(fallbackProvider, nextModel);
              }}
            >
              <option value="">{tr('settings.fallback.chooseModel')}</option>
              {fallbackModels.map((entry) => (
                <option key={entry.id} value={entry.id}>{entry.display_name}</option>
              ))}
            </select>
          </div>
        </div>
        {codexSelected && (
          <>
            <VNextSegmented
              className="vnext-settings-reasoning-toggle"
              mode="radio"
              size="sm"
              fullWidth
              label={tr('settings.provider.reasoningLabel')}
              ariaLabel={tr('settings.provider.reasoningLabel')}
              value={codexStatus?.reasoning_effort || 'medium'}
              onChange={(value) => {
                // Reasoning changes ride on the current codex model; before the
                // codex status has loaded there is no model to attach them to.
                const model = codexStatus?.model || selectedCodexModel?.slug || '';
                if (!model) return;
                onSetCodexModel(model, value);
              }}
              options={codexReasoningLevels.map((level) => ({
                value: level,
                label: ['low', 'medium', 'high', 'xhigh'].includes(level) ? tr(`settings.provider.reasoning.${level}`) : level,
                disabled: codexBusy || !(codexStatus?.model || selectedCodexModel?.slug),
              }))}
            />
            <p className="vnext-settings-inline-note">{tr('settings.provider.reasoningHint')}</p>
          </>
        )}
        <SettingsToggleLine
          label={tr('settings.provider.paidFallbackLabel')}
          description={tr('settings.provider.paidFallbackDescription')}
          checked={paidApiFallbackEnabled}
          disabled={!onSetPaidApiFallback || callBusy}
          onChange={(checked) => { void validateCallSetting(() => onSetPaidApiFallback?.(checked), checked); }}
        />
        <div className="vnext-provider-controls" data-settings-section="web-search" id="vnext-settings-search">
          <VNextSegmented
            className="vnext-settings-search-provider-toggle"
            mode="radio"
            size="sm"
            fullWidth
            label={tr('settings.search.providerLabel')}
            ariaLabel={tr('settings.search.providerLabel')}
            value={webSearchProvider}
            onChange={(value) => {
              void validateCallSetting(() => onSetWebSearchProvider?.(value));
            }}
            options={[...(searchUnsupported ? [{ value: 'unsupported', label: tr('settings.search.option.unsupported'), disabled: true }] : []), ...WEB_SEARCH_PROVIDER_OPTIONS.map((option) => ({
              value: option,
              label: tr(`settings.search.option.${option}`),
              disabled: !onSetWebSearchProvider || callBusy,
            }))]}
          />
          {searchModelChoices.length > 0 && <div className="vnext-model-field">
            <label id="vnext-settings-search-model-label" htmlFor="vnext-settings-search-model-select">{tr('settings.search.modelLabel')}</label>
            <select
              id="vnext-settings-search-model-select"
              className="vnext-select vnext-model-select"
              aria-labelledby="vnext-settings-search-model-label"
              value={webSearchModel}
              disabled={!onSetWebSearchModel || callBusy}
              onChange={(event) => {
                const value = event.currentTarget.value;
                void validateCallSetting(() => onSetWebSearchModel?.(value));
              }}
            >
              {searchModelChoices.map((id) => (
                <option key={id} value={id}>{id}</option>
              ))}
            </select>
          </div>}
          {searchNeedsSignIn && (
            <p className="vnext-settings-inline-note" role="status" data-web-search-reason="not-signed-in">
              {tr('settings.search.notSignedIn')}
            </p>
          )}
          {searchUnsupported && <p role="status" className="vnext-settings-inline-note">{tr('settings.search.unsupported')}</p>}
          <p className="vnext-settings-inline-note">{tr('settings.search.hint')}</p>
          {callNotice && <p role="status" className="vnext-settings-inline-note">{callNotice}</p>}
        </div>
        </div>
      </section>
      <section className="vnext-settings-section" data-settings-section="appearance">
        <h2>{tr('settings.appearance.heading')}</h2>
        <div className="vnext-settings-section-body">
        <div className="vnext-avatar-settings-grid">
          <AvatarSettingsCard
            label={tr('settings.avatar.aegisLabel')}
            preview={<AegisAvatar src={avatarProfile.aegisAvatarUrl} className="vnext-settings-avatar-preview" />}
            previewLabel={tr('settings.avatar.aegisPreviewLabel')}
            onPreview={() => setPreviewAvatarRole('aegis')}
            onChoose={() => aegisAvatarInputRef.current?.click()}
            onReset={() => {
              onResetAvatar('aegis');
              setAvatarMessage(tr('settings.avatar.aegisReset'));
            }}
          />
          <AvatarSettingsCard
            label={tr('settings.avatar.ownerLabel')}
            preview={<OwnerAvatar src={avatarProfile.ownerAvatarUrl} email={email} className="vnext-settings-avatar-preview" />}
            previewLabel={tr('settings.avatar.ownerPreviewLabel')}
            onPreview={() => setPreviewAvatarRole('owner')}
            onChoose={() => ownerAvatarInputRef.current?.click()}
            onReset={() => {
              onResetAvatar('owner');
              setAvatarMessage(tr('settings.avatar.ownerReset'));
            }}
          />
        </div>
        <SettingsToggleLine
          label={tr('settings.avatar.weeklyRefresh')}
          description={
            avatarAutoRefresh?.enabled
              ? tr('settings.avatar.weeklyRefreshEnabled')
              : tr('settings.avatar.weeklyRefreshOff')
          }
          checked={Boolean(avatarAutoRefresh?.enabled)}
          disabled={avatarGenerationBusy}
          onChange={(checked) => {
            setAvatarMessage('');
            onToggleAegisAvatarAutoRefresh(checked, avatarPromptHint);
          }}
        />
        {grokConnected ? (
          <>
            <label className="vnext-settings-style-hint">
              <span>{tr('settings.avatar.styleHint')}</span>
              <input
                type="text"
                value={avatarPromptHint}
                maxLength={400}
                placeholder={tr('settings.avatar.styleHintPlaceholder')}
                onChange={(event) => setAvatarPromptHint(event.currentTarget.value)}
              />
              <small className="vnext-settings-inline-note">{tr('settings.avatar.styleHintCaption')}</small>
            </label>
            <VNextButton
              variant="secondary"
              size="sm"
              className="vnext-avatar-generate-button"
              disabled={avatarGenerationBusy}
              onClick={() => {
                setAvatarMessage('');
                onGenerateAegisAvatar(avatarPromptHint);
              }}
            >
              {avatarGenerationBusy ? tr('settings.avatar.generating.button') : tr('settings.avatar.generateButton')}
            </VNextButton>
          </>
        ) : (
          <p className="vnext-settings-inline-note">{tr('settings.avatar.needsGrok')}</p>
        )}
        {avatarInlineMessage && <p className="vnext-settings-inline-note" role="status">{avatarInlineMessage}</p>}
        </div>
        <input
          ref={aegisAvatarInputRef}
          className="vnext-hidden-file-input"
          type="file"
          accept="image/png,image/jpeg,image/webp"
          onChange={(event) => {
            void handleAvatarFile('aegis', event.currentTarget.files?.[0]);
            event.currentTarget.value = '';
          }}
        />
        <input
          ref={ownerAvatarInputRef}
          className="vnext-hidden-file-input"
          type="file"
          accept="image/png,image/jpeg,image/webp"
          onChange={(event) => {
            void handleAvatarFile('owner', event.currentTarget.files?.[0]);
            event.currentTarget.value = '';
          }}
        />
        {previewAvatarRole && (
          <AvatarPreviewDialog
            role={previewAvatarRole}
            email={email}
            avatarProfile={avatarProfile}
            onClose={() => setPreviewAvatarRole(null)}
          />
        )}
      </section>
        <details data-settings-tier="developer" className="vnext-settings-tier developer vnext-disclosure">
          <summary className="vnext-disclosure-summary">
            <span className="vnext-disclosure-copy">
              <strong>{tr('settings.tier.developer')}</strong>
              <em className="vnext-disclosure-hint">{tr('settings.tier.developerHint')}</em>
            </span>
          </summary>
          {supportSection}
          <section className="vnext-settings-section" id="vnext-settings-security" data-settings-section="security">
            <div className="vnext-settings-section-body vnext-settings-grid compact">
              {securitySections.map((section) => (
                <SettingsProjectionRow key={section.label} section={section} hideReadyBadge />
              ))}
            </div>
          </section>
        </details>
      </details>

      <p className="vnext-settings-version" aria-label={`version ${APP_VERSION}`}>{APP_VERSION}</p>
    </section>
  );

}

function SettingsToggleLine({
  label,
  description,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  description?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="vnext-settings-toggle-line">
      <span>
        <strong>{label}</strong>
        {description ? <p>{description}</p> : null}
      </span>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
    </label>
  );
}

function ConnectionRow({
  name,
  chip,
  attention,
  message,
  action,
  pending,
}: {
  name: string;
  chip: string;
  attention?: boolean;
  message?: string | null;
  action?: ReactNode;
  pending?: ReactNode;
}) {
  return (
    <article className={classNames('vnext-settings-card vnext-settings-connection-row', attention && 'warn')}>
      <span className="vnext-settings-connection-name">{name}</span>
      <strong className="vnext-settings-connection-chip">{chip}</strong>
      {action ? <div className="vnext-settings-connection-action">{action}</div> : <span />}
      {message ? <p className="vnext-settings-connection-message">{message}</p> : null}
      {pending}
    </article>
  );
}

function ProviderConnectionRow({
  name,
  connected,
  pending,
  activeForChat,
  providerBusy,
  ownerMessage,
  deviceCode,
  openLabel,
  connectDisabled,
  onConnect,
  onPoll,
  onUseForChat,
  onStopUsingForChat,
  onDisconnect,
}: {
  name: string;
  connected: boolean;
  pending: boolean;
  activeForChat: boolean;
  providerBusy: boolean;
  ownerMessage?: string | null;
  deviceCode?: { user_code: string; verification_uri: string; verification_uri_complete?: string | null } | null;
  openLabel: string;
  connectDisabled?: boolean;
  onConnect: () => void;
  onPoll: () => void;
  onUseForChat: () => void;
  onStopUsingForChat: () => void;
  onDisconnect?: () => void;
}) {
  const tr = useT();
  const chip = pending
    ? tr('settings.connection.pending')
    : connected
      ? (activeForChat ? tr('settings.connection.activeForChat') : tr('settings.connection.connected'))
      : tr('settings.connection.disconnected');
  const needsOwner = !connected || pending;
  let action: ReactNode = null;
  if (!pending && !connected) {
    action = (
      <VNextButton variant="secondary" size="sm" disabled={providerBusy || connectDisabled} onClick={onConnect}>
        {providerBusy ? tr('settings.provider.starting') : tr('settings.provider.connect')}
      </VNextButton>
    );
  } else if (!pending && connected && activeForChat) {
    action = (
      <VNextButton variant="secondary" size="sm" disabled={providerBusy} onClick={onStopUsingForChat}>
        {providerBusy ? tr('settings.provider.working') : tr('settings.provider.stopUsingForChat')}
      </VNextButton>
    );
  } else if (!pending && connected) {
    action = (
      <VNextButton variant="secondary" size="sm" disabled={providerBusy} onClick={onUseForChat}>
        {providerBusy ? tr('settings.provider.working') : tr('settings.provider.useForChat')}
      </VNextButton>
    );
  }
  const disconnect = connected && !pending && onDisconnect ? (
    <VNextButton variant="destructive" size="sm" disabled={providerBusy} onClick={onDisconnect}>
      {tr('settings.provider.disconnect')}
    </VNextButton>
  ) : null;
  const actionNode = action || disconnect
    ? <>{action}{disconnect}</>
    : null;
  const pendingBlock = pending ? (
    <div className="vnext-grok-device-code vnext-settings-connection-pending" aria-live="polite">
      {deviceCode && (
        <>
          <span>{tr('settings.provider.codeLabel')}</span>
          <strong>{deviceCode.user_code}</strong>
          <a href={deviceCode.verification_uri_complete ?? deviceCode.verification_uri} target="_blank" rel="noreferrer">
            {openLabel}
          </a>
        </>
      )}
      <VNextButton variant="secondary" size="sm" disabled={providerBusy} onClick={onPoll}>
        {providerBusy ? tr('settings.provider.checking') : tr('settings.provider.iApprovedIt')}
      </VNextButton>
      <VNextButton variant="secondary" size="sm" disabled={providerBusy} onClick={onConnect}>
        {tr('settings.provider.newCode')}
      </VNextButton>
    </div>
  ) : null;

  return (
    <ConnectionRow
      name={name}
      chip={chip}
      attention={needsOwner}
      message={needsOwner ? ownerMessage : null}
      action={actionNode}
      pending={pendingBlock}
    />
  );
}

function AvatarSettingsCard({
  label,
  preview,
  previewLabel,
  onPreview,
  onChoose,
  onReset,
}: {
  label: string;
  preview: ReactNode;
  previewLabel: string;
  onPreview: () => void;
  onChoose: () => void;
  onReset: () => void;
}) {
  const tr = useT();
  return (
    <article className="vnext-avatar-settings-card">
      <button type="button" className="vnext-avatar-preview-button" aria-label={previewLabel} onClick={onPreview}>
        {preview}
      </button>
      <strong>{label}</strong>
      <div className="vnext-action-row">
        <VNextButton variant="secondary" size="sm" onClick={onChoose}>{tr('settings.avatar.chooseImage')}</VNextButton>
        <VNextButton variant="secondary" size="sm" onClick={onReset}>{tr('settings.avatar.reset')}</VNextButton>
      </div>
    </article>
  );
}

function WebPushSettingsCard({
  status,
  deviceSubscribed,
  busy: webPushBusy,
  message,
  onEnable,
  onDisable,
  onTest,
}: {
  status: VNextWebPushStatus | null;
  deviceSubscribed: boolean | null;
  busy: boolean;
  message?: string;
  onEnable: () => void;
  onDisable: () => void;
  onTest: () => void;
}) {
  const tr = useT();
  const { locale } = useLocale();
  const capability = webPushCapability(locale);
  const serverSubscribed = Boolean(status?.subscribed);
  const checkingDevice = Boolean(status?.configured && capability.supported && deviceSubscribed === null);
  const enabledHere = deviceSubscribed === true || (!checkingDevice && deviceSubscribed === null && serverSubscribed);
  const enabledElsewhere = deviceSubscribed === false && serverSubscribed;
  const configured = Boolean(status?.configured);
  const sendConfigured = Boolean(status?.send_configured);
  const actionDisabled = webPushBusy || checkingDevice || !capability.supported || !configured;
  const sentence = enabledHere
    ? tr('settings.push.enabledHere')
    : enabledElsewhere
      ? tr('settings.push.enabledElsewhere')
      : configured ? tr('settings.push.enableTitle') : tr('settings.push.setupNeeded');

  return (
    <div className="vnext-settings-support-card vnext-web-push-card">
      <p>{sentence}</p>
      {configured && (
        <div className="vnext-action-row">
          {enabledHere ? (
            <VNextButton variant="secondary" size="sm" disabled={webPushBusy} onClick={onDisable}>
              {webPushBusy ? tr('settings.push.working') : tr('settings.push.turnOff')}
            </VNextButton>
          ) : (
            <VNextButton variant="secondary" size="sm" disabled={actionDisabled} onClick={onEnable}>
              {webPushBusy ? tr('settings.push.working') : checkingDevice ? tr('settings.push.checking') : tr('settings.push.enable')}
            </VNextButton>
          )}
          {serverSubscribed && (
            <VNextButton variant="secondary" size="sm" disabled={webPushBusy || !sendConfigured} onClick={onTest}>
              {webPushBusy ? tr('settings.push.working') : tr('settings.push.sendTest')}
            </VNextButton>
          )}
        </div>
      )}
      {!capability.supported && (
        <p className="vnext-settings-inline-note">{capability.reason}</p>
      )}
      {message && <p className="vnext-settings-inline-note" role="status">{message}</p>}
    </div>
  );
}

function AvatarPreviewDialog({
  role,
  email,
  avatarProfile,
  onClose,
}: {
  role: AvatarRole;
  email?: string;
  avatarProfile: AvatarProfile;
  onClose: () => void;
}) {
  const tr = useT();
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') onClose();
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [onClose]);

  return (
    <div className="vnext-avatar-preview-backdrop" role="presentation" onClick={onClose}>
      <section
        className="vnext-avatar-preview-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={role === 'aegis' ? tr('settings.avatar.aegisPreviewDialogLabel') : tr('settings.avatar.ownerPreviewDialogLabel')}
        onClick={(event) => event.stopPropagation()}
      >
        <button type="button" className="vnext-avatar-preview-close" aria-label={tr('settings.avatar.previewCloseLabel')} onClick={onClose}>
          {tr('settings.avatar.close')}
        </button>
        {role === 'aegis' ? (
          <AegisAvatar src={avatarProfile.aegisAvatarUrl} className="vnext-avatar-preview-large" />
        ) : (
          <OwnerAvatar src={avatarProfile.ownerAvatarUrl} email={email} className="vnext-avatar-preview-large" />
        )}
        <strong>{role === 'aegis' ? tr('settings.avatar.aegisLabel') : tr('settings.avatar.ownerLabel')}</strong>
      </section>
    </div>
  );
}

export function routeSettingsSnapshotMessage(message?: string): 'avatar' | 'cost' | 'support' | null {
  if (!message) return null;
  const lower = message.toLowerCase();
  if (lower.includes('debug access') || lower.includes('debug job') || lower.includes('snapshot') || lower.includes('support')) {
    return 'support';
  }
  if (
    lower.includes('avatar')
    || lower.includes('aegis weekly')
    || lower.includes('generation')
  ) {
    return 'avatar';
  }
  if (lower.includes('automatic checks') || lower.includes('cost')) {
    return 'cost';
  }
  return null;
}

function readAvatarImageFile(file: File, tr: (key: string) => string): Promise<string> {
  const allowedTypes = new Set(['image/png', 'image/jpeg', 'image/webp']);
  if (!allowedTypes.has(file.type)) {
    return Promise.reject(new Error(tr('settings.avatar.usePngJpegWebp')));
  }
  if (file.size > 8 * 1024 * 1024) {
    return Promise.reject(new Error(tr('settings.avatar.under8mb')));
  }
  return new Promise((resolve, reject) => {
    const image = new Image();
    const url = URL.createObjectURL(file);
    image.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error(tr('settings.avatar.couldNotRead')));
    };
    image.onload = () => {
      URL.revokeObjectURL(url);
      const canvas = document.createElement('canvas');
      const size = 256;
      canvas.width = size;
      canvas.height = size;
      const context = canvas.getContext('2d');
      if (!context) {
        reject(new Error(tr('settings.avatar.couldNotPrepare')));
        return;
      }
      const sourceSize = Math.min(image.naturalWidth || image.width, image.naturalHeight || image.height);
      if (!sourceSize) {
        reject(new Error(tr('settings.avatar.couldNotPrepare')));
        return;
      }
      const sourceX = ((image.naturalWidth || image.width) - sourceSize) / 2;
      const sourceY = ((image.naturalHeight || image.height) - sourceSize) / 2;
      context.clearRect(0, 0, size, size);
      context.drawImage(image, sourceX, sourceY, sourceSize, sourceSize, 0, 0, size, size);
      const value = canvas.toDataURL('image/jpeg', 0.88);
      if (!/^data:image\/jpeg;base64,/i.test(value) || value.length > 700_000) {
        reject(new Error(tr('settings.avatar.couldNotPrepare')));
        return;
      }
      resolve(value);
    };
    image.src = url;
  });
}

function SettingsProjectionRow({
  section,
  hideReadyBadge = false,
}: {
  section: VNextStatusSummary;
  /** Service rows: drop the redundant Ready/就绪 chip next to a healthy label. */
  hideReadyBadge?: boolean;
}) {
  const tr = useT();
  const { locale } = useLocale();
  const ownerMessage = ownerMessageForProjectionSection(section, locale);
  const ownerNextStep = ownerNextStepForProjectionSection(section, locale);
  const showHealth = !(hideReadyBadge && (section.health === 'ready' || section.health === 'quiet'));

  return (
    <article className={classNames('vnext-settings-card', section.health === 'attention' && 'warn')}>
      <div>
        <span>
          <i className={classNames('vnext-health-dot', section.health === 'attention' && 'warn')} aria-hidden="true" />
          {localizedSettingsSectionLabel(section.label, locale)}
        </span>
        {showHealth && <strong>{ownerHealthLabel(section.health, tr)}</strong>}
      </div>
      <p>{ownerMessage}</p>
      {ownerNextStep && <em>{ownerNextStep}</em>}
    </article>
  );
}
