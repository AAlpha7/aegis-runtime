import { parseEgressHoldCard } from './egressHoldCard';
import { voicePendingDecision } from './ceoIntent';

/**
 * The ReAct loop stores this sentence when a tool returns
 * `pending_human_approval`. It is not a second decision. A search hold
 * already has one card.
 *
 * Stored search outcomes use a `result` or `failure` prefix so a reload can
 * tell a finished lookup from a failed one without parsing tool JSON again.
 */
export function isGatePendingOwnerAck(text: string | null | undefined): boolean {
  const normalized = (text ?? '').trim().replace(/\s+/g, ' ');
  if (!normalized) return false;
  if (normalized === '等待你批准这次搜索。' || normalized === '等待你批准这次搜索') return true;
  const body = normalized.replace(/^⏳\s*/u, '');
  return body.includes('·')
    && body.includes('等待你批准')
    && body.includes('awaiting your approval')
    && body.length <= 180;
}

export function isSearchHold(summary?: string | null, toolName?: string | null): boolean {
  const tool = (toolName ?? '').trim().toLowerCase();
  if (tool === 'web_search' || tool === 'x_search') return true;
  return parseEgressHoldCard(summary)?.destination === 'search-provider';
}

export function ownerQuestionFromSummary(summary?: string | null): string {
  if (!summary) return '';
  const line = summary
    .split('\n')
    .map((item) => item.trim())
    .find((item) => item.startsWith('owner_question:'));
  if (!line) return '';
  return line.slice('owner_question:'.length).trim();
}

/** Same outbound facts the server uses, plus the tool name. Email holds return null. */
export function searchHoldIdentity(summary?: string | null, toolName?: string | null): string | null {
  if (!isSearchHold(summary, toolName)) return null;
  const card = parseEgressHoldCard(summary);
  const tool = (toolName ?? '').trim().toLowerCase();
  if (!card) return tool;
  return [tool, card.destination, card.wouldSend, card.stopped, card.untrustedSource ?? ''].join('\n');
}

export function searchOwnerQuestion(
  message: { summary?: string | null; toolName?: string | null },
  messages: ReadonlyArray<{ role: string; text?: string }>,
  index: number,
): string {
  const stored = ownerQuestionFromSummary(message.summary);
  if (stored) return stored;
  if (!isSearchHold(message.summary, message.toolName)) return '';
  for (let cursor = index - 1; cursor >= 0; cursor -= 1) {
    const candidate = messages[cursor];
    if (!candidate || candidate.role !== 'owner') continue;
    const text = (candidate.text ?? '').trim();
    if (!text || voicePendingDecision(text)) continue;
    return Array.from(text).slice(0, 160).join('');
  }
  return '';
}

export function omitGatePendingOwnerAcks<T extends { role: string; text: string }>(messages: T[]): T[] {
  if (!messages.some((message) => message.role === 'approval')) return messages;
  return messages.filter((message) => !(message.role === 'aegis' && isGatePendingOwnerAck(message.text)));
}

export function readSearchOutcome(raw: string | null | undefined): { phase: 'done' | 'failed'; text: string } | null {
  const trimmed = (raw ?? '').trim();
  if (!trimmed) return null;
  if (trimmed === 'failure') return { phase: 'failed', text: '' };
  if (trimmed.startsWith('failure\n')) {
    return { phase: 'failed', text: trimmed.slice('failure\n'.length).trim() };
  }
  if (trimmed.startsWith('result\n')) {
    return { phase: 'done', text: trimmed.slice('result\n'.length).trim() };
  }
  return ownerVisibleSearchOutcome(trimmed);
}

/** Shape a replayed tool body into the sentence the search card shows. */
export function ownerVisibleSearchOutcome(raw: string | null | undefined): { phase: 'done' | 'failed'; text: string } {
  const trimmed = (raw ?? '').trim();
  if (!trimmed) return { phase: 'failed', text: '' };
  if (trimmed === 'failure' || trimmed.startsWith('failure\n') || trimmed.startsWith('result\n')) {
    return readSearchOutcome(trimmed) ?? { phase: 'failed', text: '' };
  }
  if (trimmed.startsWith('{')) {
    try {
      const parsed = JSON.parse(trimmed) as { status?: unknown; error?: unknown; summary?: unknown };
      const status = typeof parsed.status === 'string' ? parsed.status : '';
      const error = typeof parsed.error === 'string' ? parsed.error.trim() : '';
      if (status === 'error' || error) return { phase: 'failed', text: error.slice(0, 500) };
      const summary = typeof parsed.summary === 'string' ? parsed.summary.trim() : '';
      if (summary) return { phase: 'done', text: summary.slice(0, 500) };
    } catch {
      // Plain text that happens to start with a brace.
    }
  }
  if (trimmed.startsWith('审批失败') || trimmed.startsWith('⏰')) {
    return { phase: 'failed', text: trimmed.slice(0, 500) };
  }
  return { phase: 'done', text: trimmed.slice(0, 500) };
}
