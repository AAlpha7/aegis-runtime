import type { OwnerWorkQueue, VNextStatusProjection, VNextStatusSummary } from '../api';
import { t, tf } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';
import { useLocale } from '../i18n/LocaleContext';
import { itemNeedsDecision, localizedSourceLabel, sourceStatusView } from './model';
import { ownerMessageForProjectionSection, ownerNextStepForProjectionSection } from './ownerProjectionText';
import { formatInUserTz } from './time';
import { useUserTimezone } from './UserTimezoneContext';
import { VNextButton } from './VNextButton';
import { nightlyStatusLine, nowUnixSeconds, readNightlyStatus, readTelegramRail, telegramRailRow } from './statusRails';
import type { TelegramRailRow } from './statusRails';

function classNames(...values: Array<string | false | null | undefined>): string {
  return values.filter(Boolean).join(' ');
}

function ownerHealthLabel(health: string, locale: Locale): string {
  if (health === 'attention') return t('status.health.attention', locale);
  if (health === 'quiet') return t('status.health.quiet', locale);
  if (health === 'ready') return t('status.health.ready', locale);
  return health;
}

/**
 * Maps a backend-provided english section label to a localized DISPLAY string,
 * falling back to the raw label when unknown. CRITICAL: the english `label` is
 * the identity/comparison/React-key value everywhere in the logic — this helper
 * is for rendering only and must never be used where a label is compared (`===`)
 * or used as a `key`.
 */
const SECTION_LABEL_DISPLAY_KEY: Record<string, string> = {
  Work: 'status.sectionLabel.Work',
  Decisions: 'status.sectionLabel.Decisions',
  Context: 'status.sectionLabel.Context',
  'Cost controls': 'status.sectionLabel.CostControls',
  Routines: 'status.sectionLabel.Routines',
  Tracking: 'status.sectionLabel.Tracking',
  Receipts: 'status.sectionLabel.Receipts',
  Connections: 'status.sectionLabel.Connections',
};

function displaySectionLabel(label: string, locale: Locale): string {
  const key = SECTION_LABEL_DISPLAY_KEY[label];
  return key ? t(key, locale) : label;
}

// Source-label display localization now lives in model.ts (localizedSourceLabel,
// keyed by the English SOURCE_LABELS identity and covering all 9 sources) so the
// Status disclosure and Settings services cards share one source of truth. Same
// rule as displaySectionLabel: the English value stays the identity/key; this
// only changes what the owner sees, never a comparison or a React key.

export function StatusView({
  queue,
  projection,
  onOpenWork,
  onOpenSettings,
}: {
  queue: OwnerWorkQueue;
  projection: VNextStatusProjection | null;
  onOpenWork: () => void;
  onOpenSettings: () => void;
}) {
  const { locale } = useLocale();
  const nowUnix = nowUnixSeconds();
  const sourceViews = queue.sources.map((s) => sourceStatusView(s, locale));
  // The Telegram rail is a connection like the others (incident 2026-09-15: a
  // revoked bot token was invisible here for hours); an unhealthy rail counts
  // toward the Connections attention the same way a source does.
  const telegramRow = telegramRailRow(readTelegramRail(projection), nowUnix, locale);
  const nightlyLine = nightlyStatusLine(readNightlyStatus(projection), nowUnix, locale);
  const sourceIssueCount = sourceViews.filter((source) => !source.healthy).length
    + (telegramRow && !telegramRow.healthy ? 1 : 0);
  const connectionCount = queue.sources.length + (telegramRow ? 1 : 0);
  const activeItems = queue.items.filter((item) => !['done', 'executed', 'fired', 'cancelled', 'canceled', 'archived', 'deleted'].includes(item.status.toLowerCase()));
  const followUpCount = activeItems.filter((item) => ['tracking', 'waiting', 'scheduled', 'deferred', 'snoozed'].includes(item.status.toLowerCase())).length;
  const decisionCount = activeItems.filter((item) => itemNeedsDecision(item)).length;
  const statusSections = ownerStatusSections(
    activeItems.length,
    decisionCount,
    followUpCount,
    sourceIssueCount,
    projection,
    locale,
  );
  // CEO contract §6: Status is health only — do not treat Work decisions as a
  // second "needs you" desk. Decisions stay listed under quiet operations; the
  // owner acts in Chat/Work.
  const systemAttentionSections = statusSections.filter(
    (section) => section.health === 'attention' && section.label !== 'Decisions',
  );
  const attentionSections = systemAttentionSections;
  const normalSections = statusSections.filter(
    (section) => section.health !== 'attention' || section.label === 'Decisions',
  );
  const readyCount = statusSections.filter((section) => section.health === 'ready' || section.health === 'quiet').length;
  const attentionMetric = attentionSections.length;
  const systemEvents = projection?.system_events ?? [];
  const openAttention = attentionMetric > 0 ? onOpenSettings : undefined;

  return (
    <section className="vnext-stack">
      <div className="vnext-hero vnext-status-hero">
        <div>
          <p className="vnext-kicker">{t('status.kicker', locale)}</p>
          <h2>{attentionMetric > 0
            ? tf(attentionMetric === 1 ? 'status.hero.needsYouOne' : 'status.hero.needsYouMany', locale, { count: String(attentionMetric) })
            : t('status.hero.clear', locale)}</h2>
          <p>{t('status.hero.description', locale)}</p>
          <p className="vnext-status-trust-line">{t('status.trust.line', locale)}</p>
        </div>
        <div className="vnext-status-metrics" aria-label={t('status.metrics.label', locale)}>
          <StatusMetric label={t('status.metric.needsYou', locale)} value={attentionMetric} tone={attentionMetric > 0 ? 'warn' : 'ok'} onClick={openAttention} />
          <StatusMetric label={t('status.metric.quiet', locale)} value={readyCount} tone="ok" />
          <StatusMetric label={t('status.metric.connections', locale)} value={connectionCount} tone={sourceIssueCount > 0 ? 'warn' : 'neutral'} onClick={onOpenSettings} />
        </div>
      </div>

      {attentionSections.length > 0 ? (
        <section className="vnext-status-attention" aria-label={t('status.attention.label', locale)}>
          <div className="vnext-status-section-heading">
            <p className="vnext-kicker">{t('status.attention.kicker', locale)}</p>
            <h2>{t('status.attention.heading', locale)}</h2>
          </div>
          {attentionSections.map((section) => (
            <StatusAttentionCard key={section.label} section={section} locale={locale} onOpenWork={onOpenWork} onOpenSettings={onOpenSettings} />
          ))}
        </section>
      ) : (
        <section className="vnext-panel vnext-status-calm">
          <span className="vnext-focus-chip">{t('status.calm.chip', locale)}</span>
          <h2>{t('status.calm.heading', locale)}</h2>
          <p>{t('status.calm.description', locale)}</p>
        </section>
      )}

      {/* The three disclosures are rows of ONE card (owner 2026-09-16: no
          card-in-card; label left, plain chevron right). */}
      <div className="vnext-status-disclosures">
        <StatusOperationsDisclosure sections={normalSections} locale={locale} />
        <SystemEventsDisclosure events={systemEvents} locale={locale} />
        <SourceDetailsDisclosure sourceViews={sourceViews} telegramRow={telegramRow} sourceIssueCount={sourceIssueCount} locale={locale} />
      </div>
      {/* A failed nightly is developer information, not an owner alarm: one
          muted line, no banner, nothing when green or unknown. */}
      {nightlyLine && <p className="vnext-muted vnext-status-nightly" data-nightly-line="true">{nightlyLine}</p>}
    </section>
  );
}

function ownerStatusSections(
  activeCount: number,
  decisionCount: number,
  followUpCount: number,
  sourceIssueCount: number,
  projection: VNextStatusProjection | null,
  locale: Locale,
): VNextStatusSummary[] {
  const sections = fallbackStatusSections(activeCount, decisionCount, followUpCount, sourceIssueCount, locale);
  const projected = projection?.sections ?? [];
  const projectedCostControls = projected.find(
    (section) => section.label.trim().toLowerCase() === 'cost controls',
  );
  const merged = projectedCostControls
    ? sections.map((section) => section.label === 'Cost controls' ? projectedCostControls : section)
    : sections;
  const known = new Set(merged.map((section) => section.label));
  const extras = projected.filter(
    (section) => section.health === 'attention' && !known.has(section.label) && section.label !== 'System Health',
  );
  return merged.concat(extras);
}

// The english `label` stays the identity (compared above + localized for display
// via displaySectionLabel). owner_message / owner_next_step render directly, so
// they localize here.
function fallbackStatusSections(activeCount: number, decisionCount: number, followUpCount: number, sourceIssueCount: number, locale: Locale): VNextStatusSummary[] {
  return [
    {
      label: 'Work',
      health: activeCount > 0 ? 'ready' : 'quiet',
      owner_message: activeCount > 0 ? t('work.statusFallback.work.organized', locale) : t('work.statusFallback.work.quiet', locale),
    },
    {
      // Health stays quiet so Status never competes with Chat/Work as "needs you".
      label: 'Decisions',
      health: 'quiet',
      owner_message: decisionCount > 0
        ? tf(decisionCount === 1 ? 'work.statusFallback.decisions.waitingOne' : 'work.statusFallback.decisions.waitingMany', locale, { count: String(decisionCount) })
        : t('work.statusFallback.decisions.none', locale),
      owner_next_step: decisionCount > 0 ? t('work.statusFallback.decisions.nextStep', locale) : null,
    },
    {
      label: 'Context',
      health: 'ready',
      owner_message: t('work.statusFallback.context', locale),
    },
    {
      label: 'Cost controls',
      health: 'ready',
      owner_message: t('work.statusFallback.costControls', locale),
    },
    {
      label: 'Routines',
      health: 'ready',
      owner_message: t('work.statusFallback.routines', locale),
    },
    {
      label: 'Tracking',
      health: followUpCount > 0 ? 'ready' : 'quiet',
      owner_message: followUpCount > 0
        ? tf(followUpCount === 1 ? 'work.statusFallback.tracking.countOne' : 'work.statusFallback.tracking.countMany', locale, { count: String(followUpCount) })
        : t('work.statusFallback.tracking.none', locale),
    },
    {
      label: 'Receipts',
      health: 'ready',
      owner_message: t('work.statusFallback.receipts', locale),
    },
    {
      label: 'Connections',
      health: sourceIssueCount > 0 ? 'attention' : 'ready',
      owner_message: sourceIssueCount > 0
        ? tf(sourceIssueCount === 1 ? 'work.statusFallback.connections.attentionOne' : 'work.statusFallback.connections.attentionMany', locale, { count: String(sourceIssueCount) })
        : t('work.statusFallback.connections.ready', locale),
      owner_next_step: sourceIssueCount > 0 ? t('work.statusFallback.connections.nextStep', locale) : null,
    },
  ];
}

function SourceDetailsDisclosure({ sourceViews, telegramRow, sourceIssueCount, locale }: { sourceViews: ReturnType<typeof sourceStatusView>[]; telegramRow: TelegramRailRow | null; sourceIssueCount: number; locale: Locale }) {
  return (
    <details className={classNames('vnext-panel', 'vnext-disclosure', sourceIssueCount > 0 && 'warn')}>
      <summary className="vnext-disclosure-summary">
        <span className="vnext-disclosure-copy">
          <strong>{t('status.connections.summary', locale)}</strong>
          <em className="vnext-disclosure-hint">{sourceIssueCount > 0
            ? tf(sourceIssueCount === 1 ? 'status.connections.needsAttentionOne' : 'status.connections.needsAttentionMany', locale, { count: String(sourceIssueCount) })
            : t('status.connections.ready', locale)}</em>
        </span>
      </summary>
      {telegramRow && (
        <div className={classNames('vnext-status-row', 'vnext-status-row-telegram', !telegramRow.healthy && 'warn')} data-telegram-rail-row="true">
          <span>{telegramRow.label}</span>
          <strong>{telegramRow.statusLabel}</strong>
          {telegramRow.reason && <em>{telegramRow.reason}</em>}
        </div>
      )}
      {sourceViews.map((view) => {
        return (
          <div key={view.label} className={classNames('vnext-status-row', !view.healthy && 'warn')}>
            <span>{localizedSourceLabel(view.label, locale)}</span>
            <strong>{view.statusLabel}</strong>
            {view.reason && <em>{view.reason}</em>}
            {view.nextStep && <em className="vnext-next-step">{view.nextStep}</em>}
          </div>
        );
      })}
    </details>
  );
}

function StatusAttentionCard({
  section,
  locale,
  onOpenWork,
  onOpenSettings,
}: {
  section: VNextStatusSummary;
  locale: Locale;
  onOpenWork: () => void;
  onOpenSettings: () => void;
}) {
  const action = statusAttentionAction(section, locale, onOpenWork, onOpenSettings);
  return (
    <article className="vnext-status-attention-card">
      {/* A server section whose label is a whole sentence (no display key) gets
          the generic chip; the sentence renders once, as the body. */}
      <span className="vnext-focus-chip warn">{SECTION_LABEL_DISPLAY_KEY[section.label] ? displaySectionLabel(section.label, locale) : t('status.metric.needsYou', locale)}</span>
      <p>{ownerMessageForProjectionSection(section, locale)}</p>
      {action && (
        <VNextButton variant="secondary" className="vnext-status-next-step" onClick={action.onClick}>
          {action.label}
        </VNextButton>
      )}
    </article>
  );
}

function statusAttentionAction(
  section: VNextStatusSummary,
  locale: Locale,
  onOpenWork: () => void,
  onOpenSettings: () => void,
): { label: string; onClick: () => void } | null {
  if (section.label === 'Decisions') return { label: t('status.action.openWork', locale), onClick: onOpenWork };
  if (section.label === 'Connections') return { label: t('status.action.openSettings', locale), onClick: onOpenSettings };
  if (section.label === 'Cost controls') return { label: t('status.action.openSettings', locale), onClick: onOpenSettings };
  if (!ownerNextStepForProjectionSection(section, locale)) return null;
  return null;
}

function StatusOperationsDisclosure({ sections, locale }: { sections: VNextStatusSummary[]; locale: Locale }) {
  if (sections.length === 0) return null;
  const readyCount = sections.filter((section) => section.health === 'ready').length;
  const quietCount = sections.filter((section) => section.health === 'quiet').length;
  const summaryLabel = [
    readyCount > 0 ? tf('status.operations.readyCount', locale, { count: String(readyCount) }) : null,
    quietCount > 0 ? tf('status.operations.quietCount', locale, { count: String(quietCount) }) : null,
  ].filter(Boolean).join(' · ') || tf('status.operations.quietCount', locale, { count: String(sections.length) });

  return (
    <details className="vnext-panel vnext-disclosure vnext-status-operations">
      <summary className="vnext-disclosure-summary">
        <span className="vnext-disclosure-copy">
          <strong>{t('status.operations.summary', locale)}</strong>
          <em className="vnext-disclosure-hint">{summaryLabel}</em>
        </span>
      </summary>
      <div className="vnext-status-operation-pills" aria-label={t('status.operations.pillsLabel', locale)}>
        {sections.map((section) => (
          <span key={section.label} className={classNames('vnext-status-operation-pill', section.health)}>
            <strong>{displaySectionLabel(section.label, locale)}</strong>
            <em>{ownerHealthLabel(section.health, locale)}</em>
          </span>
        ))}
      </div>
      {/* Cards only where they add something beyond the pill: a section that
          needs attention or has a next step. Healthy sections' cards restated
          the pill with boilerplate. */}
      {sections.some((section) => section.health === 'attention' || ownerNextStepForProjectionSection(section, locale)) && (
        <section className="vnext-status-grid" aria-label={t('status.operations.gridLabel', locale)}>
          {sections
            .filter((section) => section.health === 'attention' || ownerNextStepForProjectionSection(section, locale))
            .map((section) => (
              <StatusOperationCard key={section.label} section={section} locale={locale} />
            ))}
        </section>
      )}
    </details>
  );
}

function SystemEventsDisclosure({ events, locale }: { events: NonNullable<VNextStatusProjection['system_events']>; locale: Locale }) {
  const latest = events[0];
  const noticeCount = events.filter((event) => event.severity !== 'quiet').length;
  // Warning/critical events are an owner-facing cue that must survive the
  // collapsed state: keep the warm count so a calm neutral summary never hides
  // a real problem until expanded (informational "notice" severity stays calm).
  const hasAlert = events.some((event) => event.severity === 'critical' || event.severity === 'warning');
  const summary = events.length === 0
    ? t('status.systemEvents.none', locale)
    : noticeCount > 0
      ? tf(noticeCount === 1 ? 'status.systemEvents.noticeOne' : 'status.systemEvents.noticeMany', locale, { count: String(noticeCount) })
      : tf('status.systemEvents.quietCount', locale, { count: String(events.length) });
  return (
    <details className={classNames('vnext-panel', 'vnext-disclosure', 'vnext-system-events', hasAlert && 'warn')}>
      <summary className="vnext-disclosure-summary">
        <span className="vnext-disclosure-copy">
          <strong>{t('status.systemEvents.summary', locale)}</strong>
          <em className="vnext-disclosure-hint">{summary}</em>
        </span>
      </summary>
      {events.length === 0 ? (
        <p className="vnext-muted">{t('status.systemEvents.emptyDetail', locale)}</p>
      ) : (
        <div className="vnext-system-event-list">
          {events.map((event) => (
            <SystemEventCard key={event.id} event={event} latest={event.id === latest?.id} locale={locale} />
          ))}
        </div>
      )}
    </details>
  );
}

function SystemEventCard({ event, latest, locale }: { event: VNextStatusProjection['system_events'][number]; latest: boolean; locale: Locale }) {
  const userTimezone = useUserTimezone();
  const occurred = formatSystemEventTime(event.occurred_at, locale, userTimezone);
  const windowStart = formatSystemEventTime(event.window_start_at, locale, userTimezone);
  const repeated = event.count > 1;
  return (
    <article className={classNames('vnext-system-event-card', event.severity, latest && 'latest')}>
      <div className="vnext-system-event-head">
        <span className={classNames('vnext-system-event-dot', event.severity)} aria-hidden="true" />
        <div>
          <strong>{event.title}</strong>
          <em>{repeated ? tf('status.systemEvents.repeated', locale, { count: String(event.count), start: windowStart, occurred }) : occurred}</em>
        </div>
      </div>
      <p>{event.summary}</p>
      {event.details.length > 0 && (
        <details className="vnext-system-event-details vnext-disclosure">
          <summary className="vnext-disclosure-summary">{t('status.systemEvents.eventLines', locale)}</summary>
          <ul>
            {event.details.map((detail, index) => (
              <li key={`${event.id}:${index}`}>{detail}</li>
            ))}
          </ul>
        </details>
      )}
    </article>
  );
}

function formatSystemEventTime(value: number, locale: Locale, userTimezone?: string | null): string {
  if (!Number.isFinite(value) || value <= 0) return t('status.systemEvents.recently', locale);
  return formatInUserTz(new Date(value * 1000), userTimezone, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

function StatusOperationCard({ section, locale }: { section: VNextStatusSummary; locale: Locale }) {
  const ownerMessage = ownerMessageForProjectionSection(section, locale);
  const ownerNextStep = ownerNextStepForProjectionSection(section, locale);
  return (
    <article className={classNames('vnext-status-card', section.health === 'attention' && 'warn')}>
      <div>
        <span>{displaySectionLabel(section.label, locale)}</span>
        <strong>{ownerHealthLabel(section.health, locale)}</strong>
      </div>
      <p>{ownerMessage}</p>
      {ownerNextStep && <em>{ownerNextStep}</em>}
    </article>
  );
}

function StatusMetric({
  label,
  value,
  tone,
  onClick,
}: {
  label: string;
  value: number;
  tone: 'neutral' | 'warn' | 'ok';
  onClick?: () => void;
}) {
  const content = (
    <>
      <span>{label}</span>
      <strong>{value}</strong>
    </>
  );
  if (onClick) {
    return (
      <button type="button" className={classNames('vnext-status-metric', tone, 'actionable')} onClick={onClick}>
        {content}
        <em aria-hidden="true">&gt;</em>
      </button>
    );
  }
  return <div className={classNames('vnext-status-metric', tone)}>{content}</div>;
}
