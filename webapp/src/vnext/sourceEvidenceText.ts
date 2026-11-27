import type { Locale } from '../i18n/catalog';
import { looksLikeModelInstruction, ownerFacingExcerpt, ownerFacingQuery, stripMarkdownMarkers } from './searchReceipt';
import { formatInUserTz, timezoneOffsetLabel } from './time';

// Source keys and values are untrusted; inherited Object properties are not translations.
function lookup<T>(dictionary: Record<string, T>, key: string): T | undefined {
  return Object.prototype.hasOwnProperty.call(dictionary, key) ? dictionary[key] : undefined;
}

function own(record: object, key: string): unknown {
  if (!Object.prototype.hasOwnProperty.call(record, key)) return undefined;
  return Object.getOwnPropertyDescriptor(record, key)?.value;
}

const labels: Record<string, string> = {
  Count: '条目数', Status: '状态', Due: '到期时间', Scheduled: '计划时间',
  Tasks: '待办和提醒', Items: '条目', Results: '结果', Tags: '标签',
  Error: '错误', 'Error kind': '错误类型', Hint: '处理建议', Retryable: '可原样重试',
  Warning: '注意', Warnings: '注意',
  Title: '标题', Name: '名称', Body: '内容', Summary: '摘要',
  Id: '编号', 'Group key': '分组',
};
const statuses: Record<string, [string, string]> = {
  error: ['未成功', 'Failed'],
  partial: ['部分结果', 'Partial results'],
  ok: ['查询成功', 'Query succeeded'], fired: ['已触发', 'Triggered'],
  active: ['进行中', 'Active'], pending: ['待处理', 'Pending'],
  completed: ['已完成', 'Completed'], done: ['已完成', 'Done'],
  cancelled: ['已取消', 'Cancelled'], canceled: ['已取消', 'Cancelled'],
  paused: ['已暂停', 'Paused'], failed: ['失败', 'Failed'],
};
const errors: Record<string, string> = {
  'Invalid schedule parameters / 定时参数无效': '定时参数无效，任务未创建。',
  'Schedule start time has passed / 开始时间已过': '开始时间已过，任务未创建。',
  validation_error: '参数校验失败',
  'Check trigger_type, trigger_value, and interval format.': '请确认开始时间、重复间隔和时区，再重新设置。',
  'Check trigger_type, trigger_at or delay_seconds, interval_seconds, group_key, and recurrence window/timezone against the tool schema.': '请确认开始时间、重复间隔、重复任务标识，以及运行时段和时区，再重新设置。',
  'Choose a new future trigger_at or use delay_seconds >= 10 for a relative start. Do not retry the same expired timestamp.': '请重新选择未来的开始时间，或从至少 10 秒后开始；不要沿用已过期的时间。',
};
const omitted = new Set(['index', 'status_filter', 'execution_mode', 'trigger_type', 'schema_version']);
const primary: Array<[string, string]> = [
  ['status', 'Status'], ['error', 'Error'], ['error_kind', 'Error kind'],
  ['warning', 'Warning'], ['warnings', 'Warnings'], ['hint', 'Hint'],
  ['title', ''], ['name', ''], ['count', 'Count'], ['due_date', 'Due'],
  ['trigger_at_unix', 'Scheduled'], ['url', 'URL'],
  ['summary', ''], ['answer', ''], ['result', ''], ['content', ''], ['text', ''],
  ['message', ''], ['output', ''], ['detail', ''], ['body', ''],
];
const nestedLead: Array<[string, string]> = [
  ['title', ''], ['name', ''], ['summary', ''], ['text', ''], ['message', ''],
  ['answer', ''], ['result', ''], ['output', ''], ['detail', ''], ['body', ''],
];
const primaryKeys = new Set(primary.map(([key]) => key));
const unlabeledArrays = new Set(['tasks', 'items', 'results', 'entries']);
const fieldLabels: Record<string, string> = { due_date: 'Due', trigger_at_unix: 'Scheduled', url: 'URL' };
const NOTE = /^(?:Further nested source details are omitted\.|Further source entries are omitted\.|No source details returned\.|This saved source is incomplete\.|Showing \d+ of \d+ source entries\.)$/;
const ERROR_SENTENCES: Array<[string, string]> = [
  ['Check trigger_type, trigger_at or delay_seconds, interval_seconds, group_key, and recurrence window/timezone against the tool schema.', '请确认开始时间、重复间隔、重复任务标识，以及运行时段和时区，再重新设置。'],
  ['Choose a new future trigger_at or use delay_seconds >= 10 for a relative start. Do not retry the same expired timestamp.', '请重新选择未来的开始时间，或从至少 10 秒后开始；不要沿用已过期的时间。'],
  ['Check trigger_type, trigger_value, and interval format.', '请确认开始时间、重复间隔和时区，再重新设置。'],
  ['Invalid schedule parameters / 定时参数无效', '定时参数无效，任务未创建。'],
  ['Schedule start time has passed / 开始时间已过', '开始时间已过，任务未创建。'],
  ['validation_error', '参数校验失败'],
];
const ERROR_TOKENS: Array<[RegExp, string]> = [
  [/trigger_at_unix/g, '开始时间'],
  [/delay_seconds/g, '延迟秒数'],
  [/interval_seconds/g, '间隔秒数'],
  [/trigger_type/g, '触发方式'],
  [/trigger_value/g, '触发值'],
  [/trigger_at/g, '开始时间'],
  [/group_key/g, '分组'],
];

export type SourceNode =
  | { kind: 'text'; text: string }
  | { kind: 'note'; text: string }
  | { kind: 'field'; label: string; value: string; children: SourceNode[] }
  | { kind: 'item'; children: SourceNode[] };

export type SourceBody =
  | { kind: 'incomplete' }
  | { kind: 'prose'; text: string }
  | { kind: 'tree'; nodes: SourceNode[] };

function humanize(key: string): string {
  const text = key.replace(/[_-]/g, ' ');
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function collapseWs(value: string): string {
  return value.split(/\s+/).filter(Boolean).join(' ');
}

function collapseLine(line: string): string {
  const indent = /^[ \t]*/.exec(line)?.[0] ?? '';
  const rest = line.slice(indent.length).replace(/[ \t]{2,}/g, ' ').replace(/[ \t]+$/g, '');
  return `${indent}${rest}`;
}

function leafText(value: string): string {
  if (!value.includes('\n')) return collapseWs(value);
  return value.split('\n').map(collapseLine).join('\n').replace(/^\n+|\n+$/g, '');
}

function looksLikeJson(text: string): boolean {
  return /^(\{|\[\s*(?:\{|\[|"|\]|-?\d|true\b|false\b|null\b))/.test(text);
}

// A clipped JSON fragment starts like a document and contains a JSON token.
// `{not a record` is a note the owner wrote, not a broken object.
function looksLikeTruncatedJson(text: string): boolean {
  return /^[\[{]/.test(text) && /["':]/.test(text);
}

function fromJson(value: unknown, depth: number): SourceNode[] {
  if (depth > 6) return [{ kind: 'note', text: 'Further nested source details are omitted.' }];
  if (typeof value === 'string') {
    const trimmed = value.trim();
    if (looksLikeJson(trimmed)) {
      try {
        const nested = JSON.parse(trimmed) as unknown;
        if (nested !== null && typeof nested === 'object') return fromJson(nested, depth);
      } catch {
        if (looksLikeTruncatedJson(trimmed)) {
          return [{ kind: 'note', text: 'This saved source is incomplete.' }];
        }
      }
    }
    const text = leafText(value);
    return text.trim() ? [{ kind: 'text', text }] : [];
  }
  if (Array.isArray(value)) {
    const nodes: SourceNode[] = [];
    for (const item of value) {
      const children = fromJson(item, depth + 1);
      if (children.length > 0) nodes.push({ kind: 'item', children });
    }
    return nodes;
  }
  if (value !== null && typeof value === 'object') return objectNodes(value, depth);
  if (value == null) return [];
  return [{ kind: 'text', text: String(value) }];
}

function pushField(nodes: SourceNode[], label: string, value: unknown, depth: number) {
  const children = fromJson(value, depth + 1);
  if (children.length === 0) return;
  if (!label) {
    nodes.push(...children);
    return;
  }
  const [first, ...rest] = children;
  if (rest.length === 0 && first.kind === 'text') {
    nodes.push({ kind: 'field', label, value: first.text, children: [] });
    return;
  }
  nodes.push({ kind: 'field', label, value: '', children });
}

function appendKnown(nodes: SourceNode[], record: object, depth: number, keys: Array<[string, string]>, seen: Set<string>) {
  for (const [key, label] of keys) {
    if (seen.has(key)) continue;
    seen.add(key);
    if (!Object.prototype.hasOwnProperty.call(record, key)) continue;
    const raw = own(record, key);
    if (raw == null) continue;
    if (key === 'trigger_at_unix' && typeof raw === 'number') {
      const date = new Date(raw * 1000);
      if (!Number.isNaN(date.getTime())) nodes.push({ kind: 'field', label: 'Scheduled', value: date.toISOString(), children: [] });
      continue;
    }
    pushField(nodes, label, raw, depth);
  }
}

function searchHost(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return '';
  }
}

function projectSearchPayload(record: object): SourceNode[] | null {
  const note = typeof own(record, 'note') === 'string' ? own(record, 'note') as string : '';
  const prompt = typeof own(record, 'search_prompt') === 'string' ? own(record, 'search_prompt') as string : '';
  const citations = own(record, 'verified_citations');
  const hasCitations = Array.isArray(citations);
  if (!prompt && !hasCitations && !looksLikeModelInstruction(note)) return null;
  if (own(record, 'status') === 'error') return null;
  const nodes: SourceNode[] = [];
  const owner = typeof own(record, 'owner_query') === 'string' ? (own(record, 'owner_query') as string).trim() : '';
  const question = owner || ownerFacingQuery(prompt);
  if (question && !looksLikeModelInstruction(question)) nodes.push({ kind: 'text', text: question });
  if (hasCitations) {
    for (const citation of (citations as unknown[]).slice(0, 3)) {
      if (!citation || typeof citation !== 'object') continue;
      const row = citation as Record<string, unknown>;
      const title = typeof row.title === 'string' ? row.title : '';
      const url = typeof row.url === 'string' ? row.url : '';
      const snippet = typeof row.snippet === 'string' ? row.snippet : typeof row.content === 'string' ? row.content : '';
      const name = stripMarkdownMarkers(title) || searchHost(url);
      const excerpt = ownerFacingExcerpt(snippet);
      const children: SourceNode[] = [];
      if (name && name !== '来源') children.push({ kind: 'text', text: name });
      if (excerpt) children.push({ kind: 'text', text: excerpt });
      if (url) children.push({ kind: 'field', label: 'URL', value: url, children: [] });
      if (children.length > 0) nodes.push({ kind: 'item', children });
    }
  }
  if (!nodes.some((node) => node.kind === 'item')) {
    const summary = typeof own(record, 'summary') === 'string' ? own(record, 'summary') as string : '';
    const excerpt = ownerFacingExcerpt(summary);
    if (excerpt) nodes.push({ kind: 'text', text: excerpt });
  }
  return nodes.length > 0 ? nodes : null;
}

function isSearchDump(text: string): boolean {
  return /use only these excerpts|do not call web_search|search prompt:|search_prompt/i.test(text);
}

function scrubSearchProse(text: string): string {
  const lines = text.split('\n');
  const statusOk = lines.some((line) => /^status:\s*ok$/i.test(line.trim()));
  const kept = lines.flatMap((line) => {
    const body = line.trim();
    if (!body) return [];
    if (isModelInstructionLine(body)) return [];
    if (statusOk && /^retryable:\s*false$/i.test(body)) return [];
    if (/^status:\s*ok$/i.test(body)) return [];
    const unlabeled = body.replace(/^(?:search prompt|note|snippet|title):\s*/i, '');
    const plain = stripMarkdownMarkers(unlabeled);
    return plain && !looksLikeModelInstruction(plain) ? [plain] : [];
  });
  return ownerFacingExcerpt(kept.join(' '));
}

function isModelInstructionLine(text: string): boolean {
  return looksLikeModelInstruction(text) || /^note:\s*/i.test(text) && looksLikeModelInstruction(text);
}

function objectNodes(record: object, depth: number): SourceNode[] {
  if (depth === 0) {
    const projected = projectSearchPayload(record);
    if (projected) return projected;
  }
  const nodes: SourceNode[] = [];
  const seen = new Set<string>();
  if (depth > 0) appendKnown(nodes, record, depth, nestedLead, seen);
  appendKnown(nodes, record, depth, primary, seen);
  for (const key of Object.keys(record)) {
    if (primaryKeys.has(key) || omitted.has(key)) continue;
    const raw = own(record, key);
    if (raw == null) continue;
    if (key === 'type' && raw === 'text' && typeof own(record, 'text') === 'string') continue;
    if (key === 'note' && typeof raw === 'string' && looksLikeModelInstruction(raw)) continue;
    if (key === 'retryable' && own(record, 'status') === 'ok') continue;
    if (key === 'search_prompt' && typeof raw === 'string' && looksLikeModelInstruction(raw)) continue;
    const label = unlabeledArrays.has(key) ? '' : (lookup(fieldLabels, key) || humanize(key));
    pushField(nodes, label, raw, depth);
  }
  return nodes;
}

function leadNode(content: string): SourceNode {
  if (NOTE.test(content)) return { kind: 'note', text: content };
  const field = content.match(/^([^:\n]+): (.*)$/);
  if (field) return { kind: 'field', label: field[1], value: field[2], children: [] };
  const bare = content.match(/^([^:\n]+):\s*$/);
  if (bare) return { kind: 'field', label: bare[1], value: '', children: [] };
  return { kind: 'text', text: content };
}

function isStructuredExcerpt(text: string): boolean {
  const lines = text.split('\n');
  let inFence = false;
  let sawFence = false;
  const structural: string[] = [];
  for (const line of lines) {
    if (line.trim() === '') continue;
    const body = line.trim();
    if (body === '```') {
      inFence = !inFence;
      sawFence = true;
      continue;
    }
    if (inFence) continue;
    if (/^\s*\d+\.\s/.test(line)) return false;
    structural.push(line);
  }
  if (inFence) return false;
  // Indent, or a fence around a multiline value, distinguishes a server tree
  // from a markdown list. A flat `- fact` list stays prose.
  const sawIndent = lines.some((line) => /^[ ]{2,}\S/.test(line));
  if (!sawIndent && !sawFence) return false;
  return structural.every((line) => {
    const body = line.trim();
    return NOTE.test(body) || /^- /.test(body) || /^[^:\n]+:(?:\s+\S.*)?$/.test(body);
  });
}

function parseStructured(text: string): SourceNode[] {
  const lines = text.split('\n');
  let index = 0;
  const parseBlock = (minIndent: number): SourceNode[] => {
    const nodes: SourceNode[] = [];
    while (index < lines.length) {
      const line = lines[index];
      if (line.trim() === '') {
        index += 1;
        continue;
      }
      const indent = /^[ ]*/.exec(line)?.[0].length ?? 0;
      if (indent < minIndent) break;
      if (line.trim() === '```') {
        const inner: string[] = [];
        index += 1;
        while (index < lines.length && lines[index].trim() !== '```') {
          const raw = lines[index];
          const pad = ' '.repeat(indent);
          inner.push(raw.startsWith(pad) ? raw.slice(pad.length) : raw.trimStart());
          index += 1;
        }
        if (index < lines.length && lines[index].trim() === '```') index += 1;
        const text = inner.join('\n').replace(/^\n+|\n+$/g, '');
        if (text.trim()) nodes.push({ kind: 'text', text });
        continue;
      }
      const body = line.slice(indent);
      const bullet = body.startsWith('- ');
      const content = bullet ? body.slice(2) : body;
      index += 1;
      const children = parseBlock(indent + 1);
      const lead = leadNode(content);
      if (bullet) nodes.push({ kind: 'item', children: [lead, ...children] });
      else if (lead.kind === 'field') nodes.push({ ...lead, children });
      else if (children.length > 0) nodes.push({ kind: 'item', children: [lead, ...children] });
      else nodes.push(lead);
    }
    return nodes;
  };
  return parseBlock(0);
}

export function readSourceEvidence(excerpt: string): SourceBody {
  const trimmed = excerpt.trim();
  const fenced = trimmed.match(/^```(?:json)?\s*\n?([\s\S]*?)\n?```$/i);
  const jsonText = (fenced ? fenced[1] : trimmed).trim();
  if (jsonText === 'null' || looksLikeJson(jsonText)) {
    try {
      return { kind: 'tree', nodes: fromJson(JSON.parse(jsonText) as unknown, 0) };
    } catch {
      return { kind: 'incomplete' };
    }
  }
  if (isSearchDump(trimmed)) {
    const plain = scrubSearchProse(trimmed);
    return plain ? { kind: 'prose', text: plain } : { kind: 'prose', text: '' };
  }
  if (isStructuredExcerpt(trimmed)) return { kind: 'tree', nodes: parseStructured(trimmed) };
  return { kind: 'prose', text: trimmed };
}

export function incompleteLong(locale: Locale): string {
  return locale === 'zh'
    ? '这条历史来源的内容不完整，暂时无法展开。请让安琪重新查询以查看详情。'
    : 'This saved source is incomplete. Ask Aegis to query it again to view the details.';
}

export function emptyMessage(locale: Locale): string {
  return locale === 'zh' ? '来源未返回详细内容。' : 'No source details returned.';
}

export function localizeNote(text: string, locale: Locale): string {
  if (locale !== 'zh') return text;
  return text
    .replace('This saved source is incomplete.', '这条历史来源的内容不完整。')
    .replace('Further nested source details are omitted.', '更深层的来源内容已省略。')
    .replace('Further source entries are omitted.', '其余来源条目已省略。')
    .replace(/Showing (\d+) of (\d+) source entries\./g, '共 $2 条来源记录，展示前 $1 条。')
    .replace('No source details returned.', '来源未返回详细内容。');
}

function localizeStatus(value: string, locale: Locale): string {
  const exact = lookup(statuses, value);
  if (exact) return exact[locale === 'zh' ? 0 : 1];
  return value.replace(/\b(error|partial|ok|fired|active|pending|completed|done|cancelled|canceled|paused|failed)\b/g, (token) => {
    const mapped = lookup(statuses, token);
    return mapped ? mapped[locale === 'zh' ? 0 : 1] : token;
  });
}

function localizeError(value: string): string {
  const exact = lookup(errors, value);
  if (exact) return exact;
  const sentences = [...ERROR_SENTENCES].sort((left, right) => right[0].length - left[0].length);
  let text = value;
  for (const [from, to] of sentences) {
    if (text.includes(from)) text = text.split(from).join(to);
  }
  for (const [pattern, to] of ERROR_TOKENS) {
    pattern.lastIndex = 0;
    text = text.replace(pattern, to);
  }
  return text;
}

function formatSourceTimestamp(value: string, userTimezone?: string | null): string {
  if (!/^\d{4}-\d{2}-\d{2}T/.test(value)) return value;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const wall = formatInUserTz(
    date,
    userTimezone,
    { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' },
    'en-CA',
  ).replace(',', '');
  return wall ? `${wall} ${timezoneOffsetLabel(date, userTimezone)}` : value;
}

// Owner-facing source text. A wire instant (the source row's
// `2026-09-24T05:11:11.427Z`) is the same class of raw text as a
// percent-encoded ellipsis: show the wall clock, leave hrefs alone.
// The lookbehind skips instants glued to a URL or a query (`t=2026-...`).
const OWNER_INSTANT = /(?<![=/\w])(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2}))/g;

export function formatOwnerFacingInstants(value: string, userTimezone?: string | null): string {
  if (!value || !value.includes('T')) return value;
  return value.replace(OWNER_INSTANT, (token) => {
    const shown = formatSourceTimestamp(token, userTimezone);
    return shown === token ? token : shown;
  });
}

export function localizeField(
  label: string,
  value: string,
  locale: Locale,
  userTimezone?: string | null,
): { label: string; value: string } {
  const known = lookup(labels, label);
  const displayLabel = locale === 'zh' && known ? known : label;
  let displayValue = value;
  if (label === 'Status') displayValue = localizeStatus(value, locale);
  if (locale === 'zh') {
    if (label === 'Error' || label === 'Error kind' || label === 'Hint') displayValue = localizeError(value);
    if (label === 'Retryable') displayValue = value === 'false' ? '否，需先修正设置' : value === 'true' ? '是' : value;
  }
  displayValue = formatOwnerFacingInstants(displayValue, userTimezone);
  return { label: displayLabel, value: displayValue };
}

function flattenNode(node: SourceNode, locale: Locale, userTimezone?: string | null): string {
  if (node.kind === 'text') return formatOwnerFacingInstants(node.text, userTimezone);
  if (node.kind === 'note') return localizeNote(node.text, locale);
  if (node.kind === 'item') return node.children.map((child) => flattenNode(child, locale, userTimezone)).filter(Boolean).join('\n');
  const shown = localizeField(node.label, node.value, locale, userTimezone);
  const head = shown.value ? `${shown.label}: ${shown.value}` : shown.label;
  const rest = node.children.map((child) => flattenNode(child, locale, userTimezone)).filter(Boolean).join('\n');
  return rest ? `${head}\n${rest}` : head;
}

export function localizeProse(text: string, locale: Locale, userTimezone?: string | null): string {
  return text.split('\n').map((line) => {
    const noted = localizeNote(line, locale);
    const match = noted.match(/^(\s*(?:- )?)([^:\n]+): (.*)$/);
    if (!match) {
      const plain = noted === 'No source details returned.' && locale === 'zh' ? emptyMessage(locale) : noted;
      return formatOwnerFacingInstants(plain, userTimezone);
    }
    const [, prefix, key, value] = match;
    const shown = localizeField(key, value, locale, userTimezone);
    if (shown.label === key && shown.value === value) return formatOwnerFacingInstants(noted, userTimezone);
    return formatOwnerFacingInstants(`${prefix}${shown.label}: ${shown.value}`, userTimezone);
  }).join('\n');
}

/** Flat text for comparisons. Display uses the tree from {@link readSourceEvidence}. */
export function sourceEvidenceText(excerpt: string, locale: Locale, userTimezone?: string | null): string {
  const body = readSourceEvidence(excerpt);
  if (body.kind === 'incomplete') return incompleteLong(locale);
  if (body.kind === 'prose') return localizeProse(body.text, locale, userTimezone);
  return body.nodes.map((node) => flattenNode(node, locale, userTimezone)).filter(Boolean).join('\n') || emptyMessage(locale);
}

function addFact(counts: Map<string, number>, value: string) {
  const fact = value.trim();
  if (fact.length < 2) return;
  counts.set(fact, (counts.get(fact) ?? 0) + 1);
}

function collectFacts(nodes: SourceNode[], locale: Locale, userTimezone: string | null, counts: Map<string, number>) {
  for (const node of nodes) {
    if (node.kind === 'text') addFact(counts, node.text);
    else if (node.kind === 'field') {
      const shown = localizeField(node.label, node.value, locale, userTimezone);
      if (shown.value) addFact(counts, shown.value);
      collectFacts(node.children, locale, userTimezone, counts);
    } else if (node.kind === 'item') collectFacts(node.children, locale, userTimezone, counts);
  }
}

/** The one displayed fact named by the sentence before an evidence chip. */
export function citedSourceFact(
  excerpt: string,
  sentence: string,
  locale: Locale,
  userTimezone?: string | null,
): string | null {
  const body = readSourceEvidence(excerpt);
  if (body.kind !== 'tree' || !sentence.trim()) return null;
  const counts = new Map<string, number>();
  collectFacts(body.nodes, locale, userTimezone ?? null, counts);
  const hits = [...counts.entries()]
    .filter(([fact, count]) => count === 1 && sentence.includes(fact))
    .map(([fact]) => fact)
    .sort((left, right) => right.length - left.length);
  if (hits.length === 0) return null;
  const [best, ...rest] = hits;
  for (const other of rest) {
    if (other.length === best.length || !best.includes(other)) return null;
  }
  return best;
}

/** The length banner is for a real clip, not for JSON that got shorter when formatted. */
export function sourceExcerptWasCut(excerpt: string, fullLen: number): boolean {
  // A long model payload is not a clipped source passage.
  if (isSearchDump(excerpt)) return false;
  if (!(Number.isFinite(fullLen) && fullLen > excerpt.length)) return false;
  if (/Further source entries are omitted\.|Further nested source details are omitted\.|Showing \d+ of \d+ source entries\./.test(excerpt)) {
    return true;
  }
  const trimmed = excerpt.trim();
  return trimmed.length >= 600 && !looksLikeJson(trimmed) && !isStructuredExcerpt(trimmed);
}

export function missingEvidenceIds(ids: string[]): string[] {
  const present = new Set<number>();
  for (const id of ids) {
    const match = /^E(\d+)$/.exec(id);
    if (match) present.add(Number(match[1]));
  }
  if (present.size === 0) return [];
  const max = Math.max(...present);
  const missing: string[] = [];
  for (let n = 1; n <= max; n += 1) {
    if (!present.has(n)) missing.push(`E${n}`);
  }
  return missing;
}
