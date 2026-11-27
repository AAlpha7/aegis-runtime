import { citedSourceFact, missingEvidenceIds, sourceEvidenceText, sourceExcerptWasCut } from './sourceEvidenceText';
import { useUserTimezone } from './UserTimezoneContext';
import { useId, useState } from 'react';
import type { ReactNode } from 'react';
import { RichText } from './RichText';
import { SourceExcerpt } from './SourceExcerpt';
import { useLocale } from '../i18n/LocaleContext';
import { t, tf } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';
import { parseJevProvenance } from './jevProvenance';
import { splitSearchReceipt, type SearchReceipt } from './searchReceipt';
import { SearchReceiptCard } from './SearchReceiptCard';
import type { JevProvenanceView } from './jevProvenance';
import {
  dedupeXLinks,
  extractXUrlsFromText,
  isXUrl,
  openXDeepLink,
  xHandoffTargets,
} from './xDeepLink';

type AssistantResponseTone = 'answer' | 'action' | 'evidence' | 'caution' | 'done' | 'neutral';
type NoticeSeverity = 'info' | 'notice' | 'important' | 'warning' | 'critical';

type AssistantResponseViewResult = {
  response_kind: string;
  visible_answer: string;
  severity?: NoticeSeverity | null;
  receipt_expectation: string;
  citations: string[];
  source_evidence?: SourceEvidenceItem[];
  evidence?: ClaimEvidenceItem[];
  unsupported_numbers?: string[];
  warnings: string[];
};

export type AssistantResponseSection = {
  id: string;
  title: string;
  content: string;
  tone: AssistantResponseTone;
  sourceEvidence?: SourceEvidenceItem[];
  claimEvidence?: ClaimEvidenceItem[];
};

export type ClaimEvidenceItem = {
  id: string;
  tool: string;
  label: string;
  excerpt: string;
  full_len: number;
};

type SectionDraft = {
  title: string;
  lines: string[];
  tone: AssistantResponseTone;
};

const LABEL_RULES: Array<{ title: string; tone: AssistantResponseTone; pattern: RegExp }> = [
  { title: 'Answer', tone: 'answer', pattern: /^(answer|summary|result|reply|explain|explanation|这是什么|what this is|what it is|结论|摘要|结果|回复|概览|解释|说明)\s*[:：]\s*(.*)$/i },
  { title: 'Next actions', tone: 'action', pattern: /^(next|next step|next steps|action|actions|todo|recommendation|recommendations|建议下一步|suggested next|行动|下一步|建议|行动建议|可做)\s*[:：]\s*(.*)$/i },
  { title: 'Sources', tone: 'evidence', pattern: /^(evidence|source|sources|citation|citations|why|because|为什么显示|why (?:it |this )?shows?|依据|证据|来源|原因|为什么)\s*[:：]\s*(.*)$/i },
  { title: 'Caveats', tone: 'caution', pattern: /^(caveat|caveats|warning|warnings|risk|risks|limits|limitations|note|注意|风险|限制|提醒|警告)\s*[:：]\s*(.*)$/i },
  { title: 'Done', tone: 'done', pattern: /^(done|completed|executed|receipt|已完成|已执行|回执)\s*[:：]\s*(.*)$/i },
];

export function AssistantResponseView({
  result,
  extraXLinks = [],
  className,
  ownerQuestion = '',
}: {
  result: AssistantResponseViewResult;
  /** Work-item post/profile already on this surface. Merged into the one strip. */
  extraXLinks?: string[];
  className?: string;
  /** The owner turn this answer is for. Replaces a leaked search-tool instruction. */
  ownerQuestion?: string;
}) {
  const { locale } = useLocale();
  const userTimezone = useUserTimezone();
  const evidenceScope = useId();
  const receiptSplit = splitSearchReceipt(result.visible_answer);
  const sections = assistantResponseSections({ ...result, visible_answer: receiptSplit.body }, locale);
  const answerImportance = ownerFacingAnswerImportance(result);
  const xHandoff = xHandoffTargets([
    ...collectXLinksFromAssistantResult(result),
    ...extraXLinks,
  ]);
  const claimEvidence = sections.find((section) => section.title === 'Sources')?.claimEvidence ?? [];
  const evidenceIds = new Set(claimEvidence.map((item) => item.id));
  const evidenceExcerpts = new Map(claimEvidence.map((item) => [item.id, item.excerpt]));
  const unsupportedNumbers = normalizedUnsupportedNumbers(result.unsupported_numbers);
  const [sourcesOpen, setSourcesOpen] = useState(false);
  const [highlightEstimates, setHighlightEstimates] = useState(false);
  const [citedFact, setCitedFact] = useState<{ id: string; fact: string } | null>(null);

  function selectEvidence(id: string, fact?: string) {
    setSourcesOpen(true);
    setCitedFact(fact ? { id, fact } : null);
    if (typeof window === 'undefined') return;
    window.setTimeout(() => {
      const row = document.getElementById(evidenceRowId(id, evidenceScope));
      if (row instanceof HTMLDetailsElement) row.open = true;
      const escaped = fact && typeof CSS !== 'undefined' && typeof CSS.escape === 'function'
        ? CSS.escape(fact)
        : fact?.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
      const target = escaped
        ? row?.querySelector(`[data-source-fact="${escaped}"]`)
        : null;
      (target ?? row)?.scrollIntoView({ block: 'nearest' });
    }, 0);
  }

  const answerDecoration: AnswerDecoration = {
    evidenceIds,
    evidenceExcerpts,
    evidenceScope,
    sourcesOpen,
    unsupportedNumbers,
    highlightEstimates,
    estimateTitle: t('assistantResponse.estimate.title', locale),
    onSelectEvidence: selectEvidence,
    locale,
    userTimezone,
    citedFact,
  };

  return (
    <div className={classNames('vnext-answer-stack', className)}>
      {sections.map((section) => {
        const collapsed = shouldCollapseSection(section);
        const isSources = section.title === 'Sources';
        const claims = section.claimEvidence ?? [];
        const visibleSources = (section.sourceEvidence ?? []).filter(source =>
          !claims.some(claim => isRepeatedToolSummary(source, claim, locale, userTimezone)));
        const onlyFailedSearch = isSources && !section.content && !visibleSources.length
          && claims.length > 0 && claims.every(item => item.tool === 'web_search' && failedSearchExcerpt(item.excerpt));
        if (collapsed) {
          return (
            <details
              key={section.id}
              className={classNames('vnext-answer-card', 'vnext-disclosure', 'is-collapsible', `tone-${section.tone}`)}
              open={isSources ? sourcesOpen : undefined}
              onToggle={isSources ? (event) => setSourcesOpen(event.currentTarget.open) : undefined}
            >
              <summary className="vnext-disclosure-summary">
                <span>{onlyFailedSearch ? (locale === 'zh' ? '检索状态' : 'Search status') : sectionTitleLabel(section.title, locale)}</span>
              </summary>
              <AssistantSectionContent
                section={section}
                searchReceipt={isSources ? receiptSplit.receipt : null}
                severity={answerImportance}
                locale={locale}
                answerDecoration={answerDecoration}
                highlightEstimates={highlightEstimates}
                onToggleEstimates={() => setHighlightEstimates((current) => !current)}
              />
            </details>
          );
        }
        return (
          <section
            key={section.id}
            className={classNames('vnext-answer-card', `tone-${section.tone}`)}
            aria-label={sectionTitleLabel(section.title, locale)}
          >
            {section.title !== 'Answer' && (
              <header>
                <span>{sectionTitleLabel(section.title, locale)}</span>
              </header>
            )}
            <AssistantSectionContent
              section={section}
              severity={answerImportance}
              locale={locale}
              answerDecoration={answerDecoration}
              highlightEstimates={highlightEstimates}
              onToggleEstimates={() => setHighlightEstimates((current) => !current)}
            />
          </section>
        );
      })}
      {receiptSplit.receipt && receiptSplit.receipt.sources.length > 0 && !sections.some(section => section.title === 'Sources') && (
        <SearchReceiptCard receipt={receiptSplit.receipt} ownerQuestion={ownerQuestion} tr={{ t: (key) => t(key, locale) }} />
      )}
      {xHandoff && (
        <div className="vnext-x-open-strip" role="group" aria-label={t('work.x.openInX', locale)}>
          <span className="vnext-x-open-strip-label">{t('work.x.complementHint', locale)}</span>
          <div className="vnext-x-open-strip-actions">
            <button
              type="button"
              className="vnext-x-open-btn"
              title={xHandoff.openHref}
              onClick={() => openXDeepLink(xHandoff.openHref)}
            >
              {t('work.x.openInX', locale)}
            </button>
            {xHandoff.replyHref && (
              <button
                type="button"
                className="vnext-x-open-btn is-secondary"
                title={xHandoff.replyHref}
                onClick={() => {
                  if (xHandoff.replyHref) openXDeepLink(xHandoff.replyHref);
                }}
              >
                {t('work.x.draftReplyInX', locale)}
              </button>
            )}
          </div>
          {xHandoff.extras.length > 0 && (
            <div className="vnext-x-open-extras">
              {xHandoff.extras.map((extra) => (
                <button
                  key={extra.href}
                  type="button"
                  className="vnext-x-open-extra"
                  title={extra.href}
                  aria-label={`${t('work.x.openInX', locale)} ${extra.caption}`}
                  onClick={() => openXDeepLink(extra.href)}
                >
                  {extra.caption}
                </button>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** Deduped X status/profile URLs from answer body + citation labels. */
export function collectXLinksFromAssistantResult(result: {
  visible_answer?: string;
  citations?: string[];
  source_evidence?: Array<{ label?: string; detail?: string; summary?: string }>;
}): string[] {
  const chunks = [
    result.visible_answer || '',
    ...(result.citations || []),
    ...(result.source_evidence || []).flatMap((s) => [s.label, s.detail, s.summary]),
  ];
  const found: string[] = [];
  for (const chunk of chunks) {
    if (!chunk) continue;
    found.push(...extractXUrlsFromText(chunk, 8));
    const trimmed = chunk.trim();
    if (isXUrl(trimmed)) found.push(trimmed);
  }
  return dedupeXLinks(found);
}

/** Severity shown on the answer body — demotes soft-gate "important" noise. */
export function ownerFacingAnswerImportance(result: Pick<AssistantResponseViewResult, 'severity' | 'warnings'>): NoticeSeverity | null {
  const severity = result.severity ?? null;
  if (!severity || severity === 'info' || severity === 'notice') return null;
  if (severity === 'warning' || severity === 'critical') return severity;
  if (severity !== 'important') return null;
  const noise = new Set(['source_content_untrusted', 'redacted_fields_excluded', 'more_detail_requires_gate']);
  const warnings = Array.isArray(result.warnings) ? result.warnings : [];
  const meaningful = warnings.filter((w) => (
    typeof w === 'string'
    && w.trim()
    && !noise.has(w.trim())
    && !isUnsupportedNumbersWarning(w)
  ));
  // Soft gate alone was stamping ‼️重要 on every follow-up explain card.
  if (warnings.some((w) => w === 'more_detail_requires_gate') && meaningful.length === 0) {
    return null;
  }
  return 'important';
}

function AssistantSectionContent({
  section,
  searchReceipt,
  severity,
  locale,
  answerDecoration,
  highlightEstimates,
  onToggleEstimates,
}: {
  section: AssistantResponseSection;
  searchReceipt?: SearchReceipt | null;
  severity?: NoticeSeverity | null;
  locale: Locale;
  answerDecoration: AnswerDecoration;
  highlightEstimates: boolean;
  onToggleEstimates: () => void;
}) {
  if (section.title === 'Sources') {
    return (
      <>
      {searchReceipt && <SearchReceiptCard receipt={searchReceipt} inline tr={{ t: key => t(key, locale) }} />}
      <SourceEvidenceContent
        content={section.content}
        sources={section.sourceEvidence}
        claimEvidence={section.claimEvidence}
        evidenceScope={answerDecoration.evidenceScope}
        locale={locale}
        citedFact={answerDecoration.citedFact}
        compactSearch={!!searchReceipt?.sources.length}
      />
      </>
    );
  }
  const answerSeverity = section.tone === 'answer' ? severity : null;
  const preview = section.tone === 'answer' ? longAnswerPreview(section.content) : null;
  const remainingEstimates = answerDecoration.unsupportedNumbers.slice();
  const decorateText = (text: string) => annotateAnswerText(text, {
    ...answerDecoration,
    remainingEstimates: section.tone === 'answer' ? remainingEstimates : [],
  });
  const estimateNote = section.tone === 'answer' && answerDecoration.unsupportedNumbers.length > 0
    ? (
        <p className="vnext-estimate-note">
          <span>{tf('assistantResponse.estimate.count', locale, { count: String(answerDecoration.unsupportedNumbers.length) })}</span>
          <button
            type="button"
            className="vnext-btn vnext-btn-ghost vnext-btn-sm vnext-estimate-toggle"
            aria-pressed={highlightEstimates}
            onClick={onToggleEstimates}
          >
            {t('assistantResponse.estimate.toggle', locale)}
          </button>
        </p>
      )
    : null;
  if (!preview) {
    return (
      <>
        <RichText content={section.content} importanceSeverity={answerSeverity} decorateText={decorateText} />
        {estimateNote}
      </>
    );
  }
  return (
    <>
      <RichText content={preview.visible} importanceSeverity={answerSeverity} decorateText={decorateText} />
      <details className="vnext-answer-more vnext-disclosure">
        <summary className="vnext-disclosure-summary">{t('assistantResponse.moreDetail', locale)}</summary>
        <RichText content={preview.remaining} decorateText={decorateText} />
      </details>
      {estimateNote}
    </>
  );
}

const SOURCE_LABELS_ZH: Record<string, string> = {
  Memory: '记忆',
  Tasks: '任务',
  Calendar: '日历',
  Web: '网页',
  Tool: '工具',
  Knowledge: '知识库',
  'Tasks and reminders': '待办和提醒',
  'Web search': '网页搜索',
  'Server monitor': '服务器监控',
  'API cost tracker': 'API 花费',
  'Commute check': '通勤查询',
};

function localizeSourceLabel(label: string, locale: Locale): string {
  if (locale !== 'zh') return label;
  return SOURCE_LABELS_ZH[label.trim()] ?? label;
}

function localizeSourceText(text: string, locale: Locale): string {
  if (locale !== 'zh') return text;
  return text
    .replace(/^Current evidence:/im, '当前证据：')
    .replace(/^(\d+) memory clues:/im, '$1 条记忆线索：')
    .replace(/^Memory:/im, '记忆：')
    .replace(/^Relevant saved context was used\.?/im, '使用了相关的已保存上下文。');
}

// The legacy tool summary and E# excerpt may quote the same output at
// different truncation lengths. Suppress only a summary contained in an E#
// excerpt; distinct facts, URLs, context and separate tool calls stay visible.
function isRepeatedToolSummary(source: SourceEvidenceItem, claim: ClaimEvidenceItem, locale: Locale, userTimezone?: string | null): boolean {
  if (source.kind !== 'tool' || localizeSourceLabel(source.label, locale).toLowerCase() !== localizeSourceLabel(claim.label, locale).toLowerCase()) return false;
  if (claim.tool === 'web_search' && failedSearchExcerpt(claim.excerpt)
    && /Tool returned (?:status: )?error/.test(`${source.summary} ${source.detail}`)) return true;
  const normalized = (text: string) => sourceEvidenceText(text.replace(/^(?:Current evidence|当前证据)[:：]\s*[-*]?\s*/i, '').replace(/^Tool returned (?:ok|success):\s*/i, ''), locale, userTimezone)
    .replace(/(?:…|\.\.\.)$/, '').trim().replace(/\s+/g, ' ');
  const detail = normalized(source.detail);
  const excerpt = normalized(claim.excerpt);
  const summary = normalized(source.summary);
  return detail.length > 0 && excerpt.includes(detail)
    && (excerpt.includes(summary) || isGenericSourceEvidenceSummary(source.summary));
}

function failedSearchExcerpt(excerpt: string): boolean {
  try {
    const result = JSON.parse(excerpt);
    return result?.status === 'error' || result?.status === 'failed';
  } catch { return /^Status:\s*(?:error|failed)\b/im.test(excerpt); }
}

export function SourceEvidenceContent({
  content,
  sources: providedSources,
  locale,
  claimEvidence,
  evidenceScope: providedScope,
  citedFact,
  compactSearch = false,
}: {
  content: string;
  sources?: SourceEvidenceItem[];
  locale: Locale;
  claimEvidence?: ClaimEvidenceItem[];
  evidenceScope?: string;
  citedFact?: { id: string; fact: string } | null;
  compactSearch?: boolean;
}) {
  const localScope = useId();
  const userTimezone = useUserTimezone();
  const evidenceScope = providedScope ?? localScope;
  const claims = claimEvidence ?? [];
  const sources = (providedSources ?? sourceEvidenceItems(content, locale))
    .filter((source) => !claims.some((claim) => isRepeatedToolSummary(source, claim, locale, userTimezone)));
  const prose = providedSources ? content : '';
  const gaps = missingEvidenceIds(claims.map((item) => item.id));
  return (
    <div className="vnext-source-evidence-list">
      {gaps.length > 0 && (
        <p className="vnext-evidence-gaps">
          {tf('assistantResponse.evidence.gaps', locale, {
            ids: gaps.join(locale === 'zh' ? '、' : ', '),
          })}
        </p>
      )}
      {prose && <SourceExcerpt excerpt={prose} locale={locale} />}
      {claims.map((item) => {
        const failedSearch = item.tool === 'web_search' && failedSearchExcerpt(item.excerpt);
        if (compactSearch && item.tool === 'web_search' && !failedSearch) return (
          <details key={item.id} id={evidenceRowId(item.id, evidenceScope)} data-evidence-id={item.id} className="vnext-search-diagnostics">
            <summary>{item.id} · {locale === 'zh' ? '检索记录' : 'Search record'}</summary>
            <SourceExcerpt excerpt={item.excerpt} locale={locale} highlightFact={citedFact?.id === item.id ? citedFact.fact : null} />
          </details>
        );
        return (
          <section
            key={item.id}
            id={evidenceRowId(item.id, evidenceScope)}
            data-evidence-id={item.id}
            aria-label={failedSearch ? (locale === 'zh' ? '未取得可用搜索来源' : 'No usable search sources') : `${item.id} · ${localizeSourceLabel(item.label, locale)}`}
            className="vnext-source-evidence-row"
          >
            <div className="vnext-source-evidence-heading">
              <strong>{failedSearch ? (locale === 'zh' ? '未取得可用搜索来源' : 'No usable search sources') : `${item.id} · ${localizeSourceLabel(item.label, locale)}`}</strong>
            </div>
            <div className="vnext-source-evidence-detail">
              {failedSearch ? <>
                <p>{locale === 'zh' ? '本次检索未能核实资料。回答中的链接不能据此视为已验证来源。' : 'This lookup did not verify information. Links in the answer are not verified by this lookup.'}</p>
                <details className="vnext-search-diagnostics"><summary>{locale === 'zh' ? '诊断详情' : 'Diagnostic details'}</summary>
                  <SourceExcerpt excerpt={item.excerpt} locale={locale} />
                </details>
              </> : <SourceExcerpt
                excerpt={item.excerpt}
                locale={locale}
                highlightFact={citedFact?.id === item.id ? citedFact.fact : null}
              />}
              {!failedSearch && sourceExcerptWasCut(item.excerpt, item.full_len) && (
                <p className="vnext-evidence-full-len">
                  {tf('assistantResponse.evidence.fullLength', locale, { count: formatFullLength(item.full_len) })}
                </p>
              )}
            </div>
          </section>
        );
      })}
      {sources.map((source, index) => {
        const jev = source.jev ? parseJevProvenance(source.jev) : undefined;
        return (
          <section key={`${index}:${source.kind ?? ''}:${source.label}:${source.summary}:${source.detail}`} className="vnext-source-evidence-row">
            <div className="vnext-source-evidence-heading">
                <strong>{localizeSourceLabel(source.label, locale)}</strong>
            </div>
            <div className="vnext-source-evidence-detail">
              {!summaryContainedInDetail(source.summary, source.detail) && (
                <SourceExcerpt excerpt={localizeSourceText(source.summary, locale)} locale={locale} />
              )}
              {/* Evidence is quoted data: never promote it to an alert callout
                  (a recalled "系统已重启" line rendered as a SYSTEM RESTART
                  incident card, owner report 2026-09-06). */}
              <SourceExcerpt excerpt={localizeSourceText(source.detail, locale)} locale={locale} />
              {jev && <JevProvenanceFacts jev={jev} locale={locale} />}
            </div>
          </section>
        );
      })}
    </div>
  );
}

export type SourceEvidenceItem = {
  kind?: string;
  label: string;
  summary: string;
  detail: string;
  /** Wire verdict is a string. Render only after parseJevProvenance. */
  jev?: {
    verdict: string;
    owner_decision?: string | null;
    approval_id?: string | null;
    receipt?: string | null;
    audit_tip?: string | null;
    risk?: string | null;
    model?: string | null;
    reason?: string | null;
  } | null;
};

function JevProvenanceFacts({ jev, locale }: { jev: JevProvenanceView; locale: Locale }) {
  const verdictLabel = t(`assistantResponse.jev.verdict.${jev.verdict}`, locale);
  const ownerLabel = jev.owner_decision
    ? t(`assistantResponse.jev.owner.${jev.owner_decision}`, locale)
    : jev.verdict === 'ask'
      ? t('assistantResponse.jev.owner.pending', locale)
      : '';
  const rows: Array<{ label: string; value: string }> = [
    { label: t('assistantResponse.jev.verdict', locale), value: verdictLabel },
  ];
  if (ownerLabel) {
    rows.push({ label: t('assistantResponse.jev.owner', locale), value: ownerLabel });
  }
  if (jev.approval_id) {
    rows.push({ label: t('assistantResponse.jev.approvalId', locale), value: jev.approval_id });
  }
  if (jev.receipt) {
    rows.push({ label: t('assistantResponse.jev.receipt', locale), value: jev.receipt });
  }
  if (jev.audit_tip) {
    rows.push({ label: t('assistantResponse.jev.auditTip', locale), value: jev.audit_tip });
  }
  if (jev.risk) {
    rows.push({ label: t('assistantResponse.jev.risk', locale), value: jev.risk });
  }
  return (
    <dl className="vnext-jev-provenance" data-jev-verdict={jev.verdict}>
      {rows.map((row) => (
        <div key={row.label} className="vnext-jev-provenance-row">
          <dt>{row.label}</dt>
          <dd>{row.value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function sourceEvidenceItems(content: string, locale: Locale): SourceEvidenceItem[] {
  const labels = content
    .split('\n')
    .map((line) => line.replace(/^\s*[-*]\s*/, '').trim())
    .filter(Boolean);
  return Array.from(new Set(labels)).map((label) => sourceEvidenceItem(label, locale));
}

function sourceEvidenceItem(label: string, locale: Locale): SourceEvidenceItem {
  const urlHost = sourceUrlHost(label);
  if (urlHost) {
    return {
      label,
      summary: tf('assistantResponse.source.externalLink', locale, { host: urlHost }),
      detail: tf('assistantResponse.source.externalDetail', locale, { url: label }),
    };
  }
  const normalized = label.toLowerCase();
  if (normalized === 'memory') {
    return {
      label,
      summary: t('assistantResponse.source.memory.summary', locale),
      detail: t('assistantResponse.source.memory.detail', locale),
    };
  }
  if (normalized === 'tasks and reminders' || normalized === 'reminder scheduler') {
    return {
      label,
      summary: t('assistantResponse.source.tasks.summary', locale),
      detail: t('assistantResponse.source.tasks.detail', locale),
    };
  }
  if (normalized === 'calendar') {
    return {
      label,
      summary: t('assistantResponse.source.calendar.summary', locale),
      detail: t('assistantResponse.source.calendar.detail', locale),
    };
  }
  if (normalized === 'web search') {
    return {
      label,
      summary: t('assistantResponse.source.webSearch.summary', locale),
      detail: t('assistantResponse.source.webSearch.detail', locale),
    };
  }
  if (normalized === 'commute check') {
    return {
      label,
      summary: t('assistantResponse.source.commute.summary', locale),
      detail: t('assistantResponse.source.commute.detail', locale),
    };
  }
  return {
    label,
    summary: t('assistantResponse.source.generic.summary', locale),
    detail: t('assistantResponse.source.generic.detail', locale),
  };
}

function sourceUrlHost(label: string): string | null {
  try {
    const url = new URL(label);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
    return url.host;
  } catch {
    return null;
  }
}

export function assistantResponseSections(result: AssistantResponseViewResult, locale: Locale): AssistantResponseSection[] {
  const answerText = ownerFacingAnswerText(result.visible_answer, locale);
  const parsed = answerText ? textSections(answerText) : [];
  const sourceText = parsed.filter((section) => section.title === 'Sources').map((section) => section.content).join('\n\n');
  const sections = parsed.filter((section) => section.title !== 'Sources');
  const warnings = normalizedWarnings(result.warnings, locale);
  if (warnings.length > 0) {
    sections.push({
      id: 'warnings',
      title: 'Safety',
      content: warnings.map((warning) => `- ${warning}`).join('\n'),
      tone: 'caution',
    });
  }
  const citations = normalizedCitations(result.citations, locale);
  const sourceEvidence = normalizedSourceEvidence(result.source_evidence);
  const claimEvidence = normalizedClaimEvidence(result.evidence);
  // Merge every source representation before rendering. Textual source headings
  // must never win a text-only dedupe over the structured evidence envelope.
  const represented = () => [...sourceEvidence.flatMap((source) => [source.label, source.summary, source.detail]), ...claimEvidence.flatMap((claim) => [claim.label, claim.excerpt])];
  const sourceIsRepresented = (value: string) => claimEvidence.some((claim) => (
    value === localizeSourceLabel(claim.label, locale) || value === runtimeSourceLabel(claim.tool, locale)
  )) || represented().some((text) => (
    text.trim() === value || (/^https?:\/\//i.test(value) && (text.match(/https?:\/\/[^\s<>"\[\]，。；！？]+/g) ?? [])
      .some((url) => url.replace(/[.,;:!?]+$/, '') === value))
  ));
  for (const citation of citations) {
    if (!sourceIsRepresented(citation)) sourceEvidence.push(sourceEvidenceItem(citation, locale));
  }
  const remainingSourceText = sourceText.split('\n').filter((line) => {
    const value = line.replace(/^\s*[-*]\s*/, '').trim();
    if (!value) return true;
    if (sourceIsRepresented(value)) return false;
    if (sourceUrlHost(value)) {
      sourceEvidence.push(sourceEvidenceItem(value, locale));
      return false;
    }
    return true;
  }).join('\n').trim();
  if (claimEvidence.length > 0 || sourceEvidence.length > 0 || remainingSourceText) {
    sections.push({
      id: 'citations',
      title: 'Sources',
      content: remainingSourceText,
      tone: 'evidence',
      sourceEvidence,
      claimEvidence,
    });
  }
  const receipt = result.receipt_expectation.trim();
  if (shouldShowReceiptPolicy(result.response_kind, receipt)) {
    sections.push({
      id: 'receipt',
      title: 'Receipt',
      content: receipt,
      tone: result.response_kind === 'error' ? 'caution' : 'done',
    });
  }
  return dedupeSections(sections);
}

function ownerFacingAnswerText(content: string, locale: Locale): string {
  const lines = content.replace(/\r\n?/g, '\n').split('\n');
  const firstContentIndex = lines.findIndex((line) => line.trim().length > 0);
  if (firstContentIndex < 0) return content;
  const first = lines[firstContentIndex].trim();
  const hasMoreContent = lines.slice(firstContentIndex + 1).some((line) => line.trim().length > 0);
  const withoutClassifier = hasMoreContent && isLowValueClassifierLine(first)
    ? lines.filter((_, index) => index !== firstContentIndex)
    : lines;
  let inCodeFence = false;
  const cleaned = withoutClassifier.flatMap((line) => {
    const fence = line.trim().startsWith('```');
    if (fence || inCodeFence) {
      if (fence) inCodeFence = !inCodeFence;
      return [line];
    }
    const text = stripLowValueRuntimeBoilerplateText(line, locale);
    return isLowValueRuntimeBoilerplateLine(text.trim()) ? [] : [text];
  }).join('\n').trim();
  return cleaned || withoutClassifier.join('\n').trim();
}

function isLowValueClassifierLine(line: string): boolean {
  const normalized = line
    .replace(/\s+/g, ' ')
    .replace(/[.:：。]+$/, '')
    .trim()
    .toLowerCase();
  return [
    'general request',
    'general',
    'task request',
    'task',
    'calendar request',
    'reminder request',
    'owner request',
    'direct request',
    'request',
    'bounded proactive review',
    'proactive review',
    'digest summary',
    'daily brief',
    'evening review',
    'heartbeat escalation',
    'heartbeat',
    'scheduled review',
    'system escalation',
    'system check',
    'receipt policy',
    'explain',
    'explanation',
    '普通请求',
    '一般请求',
    '任务',
    '日历请求',
    '提醒请求',
    '解释',
    '说明',
    '主动回顾',
    '摘要',
    '每日简报',
    '晚间回顾',
    '心跳升级',
    '系统检查',
    '回执策略',
  ].includes(normalized);
}

function isLowValueRuntimeBoilerplateLine(line: string): boolean {
  const normalized = line
    .replace(/\s+/g, ' ')
    .replace(/[.:：。]+$/, '')
    .trim()
    .toLowerCase();
  return [
    'no external action was taken',
    'no external action was executed',
    'no action was taken',
    'no action was executed',
    '未执行外部动作',
    '没有执行外部动作',
    '未采取任何操作',
    '没有采取任何操作',
    'receipt policy',
    '回执策略',
  ].includes(normalized);
}

function stripLowValueRuntimeBoilerplateText(line: string, locale: Locale): string {
  const incomplete = t('assistantResponse.incompleteAnswer', locale);
  return line
    .replace(/\bAegis could not finish\s*[:：]\s*(could not reach Aegis|connection was interrupted)\.?\s*/gi, `${incomplete} `)
    .replace(/\bcould not reach Aegis\.?/gi, incomplete)
    .replace(/\bconnection was interrupted\.?/gi, incomplete)
    .replace(/\bReceipt policy\s*[:：]\s*No external action was (taken|executed)\.?/gi, '')
    .replace(/\bReceipt policy\s*[:：]\s*No action was (taken|executed)\.?/gi, '')
    .replace(/\bNo source item or private content was opened\.?/gi, '')
    .replace(/\bNo source item was opened\.?/gi, '')
    .replace(/\bNo private content was opened\.?/gi, '')
    .replace(/\bNo external action was (taken|executed)\.?/gi, '')
    .replace(/\bNo action was (taken|executed)\.?/gi, '')
    .replace(/\s{2,}/g, ' ')
    .trim();
}

function shouldShowReceiptPolicy(responseKind: string, receipt: string): boolean {
  if (!receipt || responseKind === 'working') return false;
  // FE-authored boilerplate now comes from the catalog per display_language;
  // suppress the zh value exactly like the English regex below suppresses en
  // (otherwise zh sessions would surface a receipt section en never shows).
  if (receipt === t('app.receipt.noActionTaken', 'zh')) {
    return false;
  }
  if (/^No external action was executed; receipts are required before completion claims\.?$/i.test(receipt)) {
    return false;
  }
  if (/^No external action was (executed|taken)\.?$/i.test(receipt)) {
    return false;
  }
  if (/^No action was (executed|taken)\.?$/i.test(receipt)) {
    return false;
  }
  if (/^Handled through the main assistant runtime\.?( External completion claims still require durable receipts\.)?$/i.test(receipt)) {
    return false;
  }
  return true;
}

function shouldCollapseSection(section: AssistantResponseSection): boolean {
  return section.tone === 'evidence' || section.id === 'receipt';
}

function longAnswerPreview(content: string): { visible: string; remaining: string } | null {
  // Splitting a reference list after a fixed line count separates titles from
  // their URLs and replaces a readable streamed answer with a partial card.
  if (/https?:\/\//i.test(content)) return null;
  const lines = content.replace(/\r\n?/g, '\n').split('\n');
  const meaningfulLines = lines.filter((line) => line.trim()).length;
  if (content.length <= 720 && meaningfulLines <= 7) return null;

  let splitIndex = 0;
  let visibleLines = 0;
  let inCodeFence = false;
  for (const line of lines) {
    splitIndex += 1;
    if (line.trim().startsWith('```')) inCodeFence = !inCodeFence;
    if (line.trim()) visibleLines += 1;
    if (visibleLines >= 5 && !inCodeFence) break;
  }
  while (splitIndex < lines.length && !lines[splitIndex]?.trim()) {
    splitIndex += 1;
  }
  const visible = lines.slice(0, splitIndex).join('\n').trim();
  const remaining = lines.slice(splitIndex).join('\n').trim();
  if (!visible || !remaining) return null;
  return { visible, remaining };
}

function normalizedWarnings(warnings: string[], locale: Locale): string[] {
  const normalized = warnings
    .map((warning) => normalizeWarning(warning, locale))
    .filter((warning): warning is string => Boolean(warning));
  return Array.from(new Set(normalized));
}

function normalizeWarning(warning: string, locale: Locale): string | null {
  const value = warning.trim();
  if (!value) return null;
  if (isUnsupportedNumbersWarning(value)) return null;
  const blockedTool = value.match(/^tool_blocked:(.+)$/);
  if (blockedTool) {
    return tf('assistantResponse.warning.toolUnavailable', locale, { tool: runtimeSourceLabel(blockedTool[1], locale) });
  }
  switch (value) {
    case 'source_content_untrusted':
    case 'redacted_fields_excluded':
      return null;
    case 'more_detail_requires_gate':
      // Routine explain/chat cards already show the answer; this warning
      // re-stated "更多详情需要你的批准" under every follow-up reply and made
      // the stack look like a HITL wall. Only surface for non-routine kinds
      // handled by the caller via response_kind — default: hide.
      return null;
    default:
      return humanizeMachineLabel(value, locale);
  }
}

function normalizedCitations(citations: string[], locale: Locale): string[] {
  const normalized = citations
    .map((citation) => normalizeCitation(citation, locale))
    .filter((citation): citation is string => Boolean(citation));
  return Array.from(new Set(normalized));
}

function normalizedSourceEvidence(sources: SourceEvidenceItem[] | undefined): SourceEvidenceItem[] {
  if (!Array.isArray(sources)) return [];
  const normalized: SourceEvidenceItem[] = [];
  const seen = new Set<string>();
  for (const source of sources) {
    const label = source.label?.trim();
    const summary = source.summary?.trim();
    const detail = source.detail?.trim();
    if (!label || !summary || !detail) continue;
    const ownerDetail = ownerFacingSourceEvidenceDetail(source.kind, label, detail);
    if (!ownerDetail) continue;
    const ownerSummary = ownerFacingSourceEvidenceSummary(summary, ownerDetail);
    const key = `${source.kind ?? ''}:${label}:${ownerSummary}:${ownerDetail}`;
    if (seen.has(key)) continue;
    seen.add(key);
    normalized.push({
      kind: source.kind,
      label,
      summary: ownerSummary,
      detail: ownerDetail,
      ...(source.jev ? { jev: parseJevProvenance(source.jev) } : {}),
    });
  }
  return normalized;
}

function summaryContainedInDetail(summary: string, detail: string): boolean {
  const norm = (text: string) => text.replace(/\s+/g, ' ').trim();
  const needle = norm(summary).replace(/(?:…|\.{3})$/, '').trim();
  return needle.length === 0 || norm(detail).includes(needle);
}

function ownerFacingSourceEvidenceSummary(summary: string, detail: string): string {
  const firstEvidence = detail
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map((line) => line.replace(/^\s*[-*]\s*/, '').trim())
    .find((line) => line && !/^Current evidence:?$/i.test(line));
  if (firstEvidence && isGenericSourceEvidenceSummary(summary)) {
    return firstEvidence.length > 120 ? `${firstEvidence.slice(0, 117).trim()}...` : firstEvidence;
  }
  return summary;
}

function ownerFacingSourceEvidenceDetail(kind: string | undefined, label: string, detail: string): string | null {
  if (isLowValueRuntimeSourceEvidence(kind, label, detail)) return null;
  const evidenceIndex = detail.search(/(^|\n)\s*Current evidence:\s*/i);
  if (evidenceIndex >= 0) {
    const evidence = detail.slice(evidenceIndex).trim();
    return evidence || null;
  }
  return detail;
}

function isGenericSourceEvidenceSummary(summary: string): boolean {
  return /^(Relevant saved context was used|Task or reminder records were used|Aegis used this tool as source data|.+ data or .+ tooling was used|External web information was requested)\.?$/i
    .test(summary.trim());
}

function isLowValueRuntimeSourceEvidence(kind: string | undefined, label: string, detail: string): boolean {
  const normalizedKind = (kind ?? '').trim().toLowerCase();
  const normalizedLabel = label.trim().toLowerCase();
  const normalizedDetail = detail.toLowerCase();
  if (
    normalizedKind === 'tool'
    && !normalizedDetail.includes('current evidence:')
    && (
      normalizedDetail.includes('this tool informed the answer')
      || normalizedDetail.includes('aegis used recalled memory as context')
      || normalizedDetail.includes('aegis used the task/reminder system')
      || normalizedDetail.includes('aegis used calendar availability')
      || normalizedDetail.includes('aegis used web search')
    )
  ) {
    return true;
  }
  if (
    normalizedKind === 'context'
    && normalizedLabel === 'visible webapp context'
    && (
      normalizedDetail.includes('boundary:')
      || normalizedDetail.includes('mode: general_assistant')
      || normalizedDetail.includes('safe actions:')
    )
  ) {
    return true;
  }
  if (
    normalizedKind === 'locked'
    && normalizedLabel === 'locked private detail'
    && normalizedDetail.includes('calendar private detail')
    && normalizedDetail.includes('raw memory')
  ) {
    return true;
  }
  return false;
}

function normalizeCitation(citation: string, locale: Locale): string | null {
  const value = citation.trim();
  if (!value) return null;
  if (/^https?:\/\//i.test(value)) return value;
  if (/^visible_fields\./.test(value)) return null;
  const tool = value.match(/^tool:(.+)$/);
  if (tool) return null;
  return humanizeMachineLabel(value, locale);
}

function runtimeSourceLabel(source: string, locale: Locale): string {
  const normalized = source.trim().replace(/^skill_/, '');
  const labelKeys: Record<string, string> = {
    manage_tasks: 'assistantResponse.runtimeSource.tasks',
    schedule_intent: 'assistantResponse.runtimeSource.reminderScheduler',
    commute_traffic_monitor: 'assistantResponse.runtimeSource.commute',
    recall_memory: 'assistantResponse.runtimeSource.memory',
    knowledge_base: 'assistantResponse.runtimeSource.knowledgeBase',
    calendar_reader: 'assistantResponse.runtimeSource.calendar',
    web_search: 'assistantResponse.runtimeSource.webSearch',
    check_api_usage: 'assistantResponse.runtimeSource.apiCost',
    server_monitor: 'assistantResponse.runtimeSource.serverMonitor',
  };
  const key = labelKeys[normalized];
  return key ? t(key, locale) : humanizeMachineLabel(normalized, locale);
}

function humanizeMachineLabel(value: string, locale: Locale): string {
  const normalized = value.trim().toLowerCase();
  const phraseLabelKeys: Record<string, string> = {
    data_access_gate: 'assistantResponse.machineLabel.dataAccessGate',
    tool_gate: 'assistantResponse.machineLabel.toolGate',
    mfa_required: 'assistantResponse.machineLabel.mfaRequired',
    consent_required: 'assistantResponse.machineLabel.consentRequired',
    provider_request_id: 'assistantResponse.machineLabel.providerRequestId',
    llm_provider: 'assistantResponse.machineLabel.llmProvider',
  };
  if (phraseLabelKeys[normalized]) return t(phraseLabelKeys[normalized], locale);
  const extraConfirmation = t('assistantResponse.machineWord.mfa', locale);
  return value
    .replace(/[_-]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/\b([A-Za-z])([A-Za-z]*)\b/g, (word: string, first: string, rest: string) => (
      machineWordLabel(word.toLowerCase(), locale) ?? `${first.toUpperCase()}${rest.toLowerCase()}`
    ))
    .replace(new RegExp(`\\b${extraConfirmation} Required\\b`, 'g'), t('assistantResponse.machineLabel.mfaRequired', locale));
}

/** Per-word normalization for {@link humanizeMachineLabel}. Acronyms are universal
 * (English in both locales); the semantic words come from the catalog. */
function machineWordLabel(word: string, locale: Locale): string | undefined {
  const literal: Record<string, string> = {
    api: 'API',
    html: 'HTML',
    pdf: 'PDF',
    sms: 'SMS',
    url: 'URL',
  };
  if (literal[word]) return literal[word];
  const catalogKeys: Record<string, string> = {
    hitl: 'assistantResponse.machineWord.hitl',
    kb: 'assistantResponse.machineWord.kb',
    llm: 'assistantResponse.machineWord.llm',
    mfa: 'assistantResponse.machineWord.mfa',
    oauth: 'assistantResponse.machineWord.oauth',
  };
  const key = catalogKeys[word];
  return key ? t(key, locale) : undefined;
}

function classNames(...values: Array<string | false | null | undefined>): string {
  return values.filter(Boolean).join(' ');
}

const EVIDENCE_REF_PATTERN = /\[(E\d+(?:\s*,\s*E\d+)*)\]/g;

type AnswerDecoration = {
  evidenceIds: ReadonlySet<string>;
  evidenceExcerpts: ReadonlyMap<string, string>;
  evidenceScope: string;
  sourcesOpen: boolean;
  unsupportedNumbers: string[];
  remainingEstimates?: string[];
  highlightEstimates: boolean;
  estimateTitle: string;
  onSelectEvidence: (id: string, fact?: string) => void;
  locale: Locale;
  userTimezone: string | null;
  citedFact: { id: string; fact: string } | null;
};

export function evidenceRowId(id: string, scope: string): string {
  return `vnext-evidence-${scope}-${id}`;
}

function isUnsupportedNumbersWarning(warning: string): boolean {
  return /^unsupported_numbers:\d+$/i.test(warning.trim());
}

function normalizedUnsupportedNumbers(values: string[] | undefined): string[] {
  if (!Array.isArray(values)) return [];
  const seen = new Set<string>();
  const out: string[] = [];
  for (const value of values) {
    if (typeof value !== 'string') continue;
    const text = value.trim();
    if (!text || seen.has(text)) continue;
    seen.add(text);
    out.push(text);
    if (out.length >= 32) break;
  }
  return out;
}

function normalizedClaimEvidence(items: ClaimEvidenceItem[] | undefined): ClaimEvidenceItem[] {
  if (!Array.isArray(items)) return [];
  const normalized: ClaimEvidenceItem[] = [];
  const seen = new Set<string>();
  for (const item of items) {
    const id = item.id?.trim();
    const tool = item.tool?.trim() ?? '';
    const label = item.label?.trim();
    const excerpt = item.excerpt?.trim();
    const fullLen = typeof item.full_len === 'number' && Number.isFinite(item.full_len) ? item.full_len : -1;
    if (!id || !/^E\d+$/.test(id) || !label || excerpt === undefined || fullLen < 0) continue;
    if (seen.has(id)) continue;
    seen.add(id);
    normalized.push({ id, tool, label, excerpt: excerpt || 'No source details returned.', full_len: fullLen });
    if (normalized.length >= 16) break;
  }
  return normalized.sort((left, right) => claimEvidenceOrder(left.id) - claimEvidenceOrder(right.id) || left.id.localeCompare(right.id));
}

function claimEvidenceOrder(id: string): number {
  const match = id.match(/^E(\d+)$/);
  return match ? Number(match[1]) : Number.MAX_SAFE_INTEGER;
}

function formatFullLength(value: number): string {
  return Math.trunc(value).toLocaleString('en-US');
}

function annotateAnswerText(text: string, decoration: AnswerDecoration): ReactNode {
  const remainingEstimates = decoration.remainingEstimates ?? decoration.unsupportedNumbers.slice();
  const nodes: ReactNode[] = [];
  const pattern = new RegExp(EVIDENCE_REF_PATTERN.source, 'g');
  let cursor = 0;
  let match = pattern.exec(text);
  let partIndex = 0;
  while (match) {
    const index = match.index;
    if (index > cursor) {
      nodes.push(...wrapEstimates(text.slice(cursor, index), remainingEstimates, decoration, `t${partIndex}`));
    }
    const ids = match[1].split(/\s*,\s*/);
    const sentence = sentenceBeforeChip(text, index);
    ids.forEach((id, idIndex) => {
      const fact = citedSourceFact(decoration.evidenceExcerpts.get(id) ?? '', sentence, decoration.locale, decoration.userTimezone);
      nodes.push(renderEvidenceRef(id, decoration, `${partIndex}-${idIndex}`, fact));
    });
    cursor = index + match[0].length;
    partIndex += 1;
    match = pattern.exec(text);
  }
  if (cursor < text.length) {
    nodes.push(...wrapEstimates(text.slice(cursor), remainingEstimates, decoration, `t${partIndex}`));
  }
  return nodes;
}

function sentenceBeforeChip(text: string, index: number): string {
  const before = text.slice(0, index);
  const parts = before.split(/[。！？!?\n]/);
  for (let i = parts.length - 1; i >= 0; i -= 1) {
    if (parts[i].trim()) return parts[i];
  }
  return '';
}

function renderEvidenceRef(id: string, decoration: AnswerDecoration, key: string, fact: string | null): ReactNode {
  if (!decoration.evidenceIds.has(id)) {
    return <span key={key} className="vnext-evidence-missing">{id}</span>;
  }
  const expanded = decoration.sourcesOpen;
  return (
    <button
      key={key}
      type="button"
      className="vnext-evidence-ref"
      data-evidence-id={id}
      data-evidence-fact={fact ?? undefined}
      aria-controls={evidenceRowId(id, decoration.evidenceScope)}
      aria-expanded={expanded}
      aria-label={tf('assistantResponse.evidence.ref', decoration.locale, { id })}
      onClick={() => decoration.onSelectEvidence(id, fact ?? undefined)}
    >
      {id}
    </button>
  );
}

function wrapEstimates(
  text: string,
  remainingEstimates: string[],
  decoration: AnswerDecoration,
  keyPrefix: string,
): ReactNode[] {
  if (!text) return [];
  let earliest = -1;
  let which = -1;
  for (let index = 0; index < remainingEstimates.length; index += 1) {
    const needle = remainingEstimates[index];
    if (!needle) continue;
    const at = text.indexOf(needle);
    if (at < 0) continue;
    if (earliest < 0 || at < earliest) {
      earliest = at;
      which = index;
    }
  }
  if (which < 0) return [text];
  const needle = remainingEstimates[which];
  remainingEstimates.splice(which, 1);
  return [
    ...wrapEstimates(text.slice(0, earliest), remainingEstimates, decoration, `${keyPrefix}b`),
    <span
      key={`${keyPrefix}est`}
      className={classNames('vnext-estimate', decoration.highlightEstimates && 'is-highlighted')}
      title={decoration.estimateTitle}
      aria-label={decoration.estimateTitle}
    >
      {needle}
    </span>,
    ...wrapEstimates(text.slice(earliest + needle.length), remainingEstimates, decoration, `${keyPrefix}a`),
  ];
}

/**
 * Localizes a section title for DISPLAY only. The English title string stays the
 * internal identity used for logic comparisons (`section.title === 'Answer'`),
 * LABEL_RULES matching, and dedupe keys — only what the owner sees is translated.
 * Titles not in the known set (e.g. owner-authored markdown headings) pass
 * through unchanged.
 */
function sectionTitleLabel(title: string, locale: Locale): string {
  const key = SECTION_TITLE_KEYS[title];
  return key ? t(key, locale) : title;
}

const SECTION_TITLE_KEYS: Record<string, string> = {
  Answer: 'assistantResponse.title.answer',
  'Next actions': 'assistantResponse.title.nextActions',
  Sources: 'assistantResponse.title.sources',
  Caveats: 'assistantResponse.title.caveats',
  Done: 'assistantResponse.title.done',
  Safety: 'assistantResponse.title.safety',
  Receipt: 'assistantResponse.title.receipt',
};

function textSections(content: string): AssistantResponseSection[] {
  const lines = content.replace(/\r\n?/g, '\n').split('\n');
  const sections: AssistantResponseSection[] = [];
  let current: SectionDraft | null = null;
  let inCodeFence = false;

  function start(title: string, tone: AssistantResponseTone, firstLine = '') {
    flush();
    current = { title, tone, lines: [] };
    if (firstLine.trim()) current.lines.push(firstLine.trim());
  }

  function flush() {
    if (!current) return;
    const text = current.lines.join('\n').trim();
    if (text) {
      sections.push({
        id: `${current.tone}-${sections.length}`,
        title: current.title,
        content: text,
        tone: current.tone,
      });
    }
    current = null;
  }

  for (const rawLine of lines) {
    const line = rawLine.trimEnd();
    const trimmed = line.trim();
    if (trimmed.startsWith('```') || inCodeFence) {
      if (!current) current = { title: 'Answer', tone: 'answer', lines: [] };
      current.lines.push(line);
      if (trimmed.startsWith('```')) inCodeFence = !inCodeFence;
      continue;
    }
    const heading = trimmed.match(/^(#{1,3})\s+(.+)$/);
    if (heading) {
      const title = normalizeSectionTitle(heading[2]);
      start(title, toneForTitle(title));
      continue;
    }

    const labeled = labeledSection(trimmed);
    if (labeled) {
      start(labeled.title, labeled.tone, labeled.rest);
      continue;
    }

    if (!current) current = { title: 'Answer', tone: 'answer', lines: [] };
    current.lines.push(line);
  }
  flush();
  return sections.length > 0 ? sections : [{
    id: 'answer-0',
    title: 'Answer',
    content: content.trim(),
    tone: 'answer',
  }];
}

function labeledSection(line: string): { title: string; tone: AssistantResponseTone; rest: string } | null {
  if (!line || line.startsWith('- ') || line.startsWith('* ')) return null;
  for (const rule of LABEL_RULES) {
    const match = line.match(rule.pattern);
    if (match) return { title: rule.title, tone: rule.tone, rest: match[2] ?? '' };
  }
  return null;
}

function normalizeSectionTitle(title: string): string {
  const cleaned = title.replace(/\s+/g, ' ').replace(/[:：]\s*$/, '').trim();
  const matched = LABEL_RULES.find((rule) => rule.pattern.test(`${cleaned}:`));
  return matched?.title ?? cleaned;
}

function toneForTitle(title: string): AssistantResponseTone {
  const matched = LABEL_RULES.find((rule) => rule.pattern.test(`${title}:`));
  return matched?.tone ?? 'neutral';
}

function dedupeSections(sections: AssistantResponseSection[]): AssistantResponseSection[] {
  const seen = new Set<string>();
  return sections.filter((section) => {
    const key = `${section.title}:${section.content}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}
