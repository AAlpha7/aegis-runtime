/**
 * Mid-flight working-bubble composition.
 *
 * Live SSE frames accumulate as lines. The 25s "keep this page open" hint is
 * appended as a suffix and must never replace tool / Jev / executor phases.
 */

import { workingPhraseForTool } from './toolPhrases';

export const WORKING_PROGRESS_MAX_LINES = 8;

/** Append a live progress line, dropping an identical trailing duplicate. */
export function appendProgressLine(lines: string[], next: string, max = WORKING_PROGRESS_MAX_LINES): string[] {
  const text = (next || '').trim();
  if (!text) return lines;
  if (lines[lines.length - 1] === text) return lines;
  const nextLines = [...lines, text];
  return nextLines.length > max ? nextLines.slice(nextLines.length - max) : nextLines;
}

export function composeWorkingText(
  lines: string[],
  keepOpen: boolean,
  keepOpenText: string,
  fallback: string,
): string {
  const body = lines.length > 0 ? lines.join('\n') : fallback;
  const hint = (keepOpenText || '').trim();
  if (keepOpen && hint && !body.includes(hint)) {
    return `${body}\n${hint}`;
  }
  return body;
}

type ProgressLocale = 'zh' | 'en';
const PHASE_PHRASES: Record<string, [string, string]> = {
  calling_executor: ['正在调用执行者…', 'Calling the executor…'],
  tool: ['正在调用工具…', 'Calling a tool…'],
  jev_allow: ['Jev：允许，正在执行…', 'Jev approved. Executing…'],
  jev_ask: ['Jev：需要确认，正在等待你批准…', 'Jev requires your approval…'],
  jev_deny: ['Jev：已拒绝该操作。', 'Jev denied this action.'],
  approval_required: ['正在等待你确认…', 'Waiting for your approval…'],
  executing: ['正在执行…', 'Executing…'],
  done: ['好了，正在整理回复…', 'Preparing the reply…'],
};

/** Owner-facing line for an `action_phase` SSE frame. */
export function workingPhraseForPhase(
  phase: string,
  toolName?: string | null,
  detail?: string | null,
  locale: ProgressLocale = 'zh',
): string {
  const d = (detail || '').trim();
  const localizedDetail = d ? localizePhaseDetail(d, locale) : undefined;
  if (localizedDetail) return localizedDetail;
  const key = (phase || '').trim().toLowerCase();
  const base = PHASE_PHRASES[key]?.[locale === 'en' ? 1 : 0] ?? workingPhraseForTool('', locale);
  const tool = (toolName || '').trim();
  if (tool && (key === 'tool' || key === 'executing')) {
    return workingPhraseForTool(tool, locale);
  }
  return base;
}

function localizePhaseDetail(detail: string, locale: ProgressLocale): string | undefined {
  const t = detail.trim();
  if (/^calling (Grok Build|grok)$/i.test(t)) return workingPhraseForTool('grok', locale);
  if (/^calling Codex$/i.test(t)) return workingPhraseForTool('codex', locale);
  const fallback = t.match(/^(Grok Build|Codex) failed: (.*); calling (Grok Build|Codex)$/);
  if (fallback) {
    return locale === 'en'
      ? `${fallback[1]} failed. Switching to ${fallback[3]}…`
      : `${fallback[1]} 失败，改为调用 ${fallback[3]}…`;
  }
  if (/^Jev judging /i.test(t)) return locale === 'en' ? 'Jev is reviewing this action…' : 'Jev 正在判断这项操作…';
  if (/^Jev allow /i.test(t)) return workingPhraseForPhase('jev_allow', null, null, locale);
  if (/^Jev deny /i.test(t)) return workingPhraseForPhase('jev_deny', null, null, locale);
  if (/^Jev ask/i.test(t)) return workingPhraseForPhase('jev_ask', null, null, locale);
  if (/^executing /i.test(t)) return workingPhraseForTool(t.slice('executing '.length), locale);
  // Opaque provider diagnostics are not localized UI copy. Use the structured
  // phase instead; approval details remain on the approval card.
  return undefined;
}

/** Translate known protocol notices; preserve arbitrary owner notices verbatim. */
export function workingNotice(text: string, kind: string, locale: ProgressLocale): string {
  const fallback = kind === 'model_fallback' && text.match(/^⚠️ 主模型 (.+) 不可用，已切换至备用模型 (.+)$/);
  if (fallback) return locale === 'en'
    ? `⚠️ Primary model ${fallback[1]} is unavailable. Switched to ${fallback[2]}.`
    : text;
  const executor = kind === 'system' && text.match(/^\[Executor ([^\]]+)\] [\s\S]* \(([^()]*)\)$/);
  if (executor) {
    const status = executor[2].toLowerCase();
    const labels: Record<string, [string, string]> = {
      pending: ['等待执行', 'Pending'], in_progress: ['正在执行', 'Working'],
      completed: ['已完成', 'Completed'], failed: ['执行失败', 'Failed'],
      error: ['执行失败', 'Failed'], declined: ['已拒绝', 'Declined'],
      cancelled: ['已取消', 'Cancelled'], aborted: ['已中止', 'Aborted'],
    };
    const label = labels[status]?.[locale === 'en' ? 1 : 0] ?? (locale === 'en' ? 'Status updated' : '状态已更新');
    return `${executor[1]} · ${label}`;
  }
  return text;
}
