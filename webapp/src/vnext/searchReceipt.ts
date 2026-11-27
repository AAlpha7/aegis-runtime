export const SEARCH_RECEIPT_BEGIN = '@@search-receipt';
export const SEARCH_RECEIPT_END = '@@end-search-receipt';
export const RESEARCH_PROGRESS_PREFIX = '正在核对：';

export type SearchReceiptSource = {
  query: string;
  title: string;
  url: string;
  excerpt: string;
};

export type SearchReceipt = {
  queries: string[];
  sources: SearchReceiptSource[];
  services?: { source: string; fallback_used: boolean }[];
};

export function splitSearchReceipt(text: string): { body: string; receipt: SearchReceipt | null } {
  const begin = text.indexOf(SEARCH_RECEIPT_BEGIN);
  const end = text.indexOf(SEARCH_RECEIPT_END);
  if (begin < 0 || end < begin) {
    return { body: text, receipt: null };
  }
  const raw = text.slice(begin + SEARCH_RECEIPT_BEGIN.length, end).trim();
  const receipt = parseReceipt(raw);
  let body = `${text.slice(0, begin)}${text.slice(end + SEARCH_RECEIPT_END.length)}`;
  body = body.replace(/\n*搜索收据\n[\s\S]*$/, '').trim();
  return { body, receipt };
}

export function isResearchProgress(text: string): boolean {
  return text.includes(RESEARCH_PROGRESS_PREFIX);
}

const MODEL_INSTRUCTION = /use only these excerpts|do not fill a gap|do not call web_search|return the exact|grounding rule|do not invent/i;

export function looksLikeModelInstruction(text: string): boolean {
  return MODEL_INSTRUCTION.test(text);
}

export function stripMarkdownMarkers(text: string): string {
  const withoutLinks = text.replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, '$1');
  return withoutLinks.replace(/\*\*|__/g, '').replace(/[ \t]+/g, ' ').replace(/[ \t]*\n[ \t]*/g, '\n').trim();
}

function splitSentences(text: string): string[] {
  const sentences: string[] = [];
  let buf = '';
  for (const ch of text) {
    buf += ch;
    if ('。.!?！？\n'.includes(ch)) {
      const trimmed = buf.trim();
      if (trimmed) sentences.push(trimmed);
      buf = '';
    }
  }
  const rest = buf.trim();
  if (rest) sentences.push(rest);
  return sentences;
}

function instructionFree(text: string): string {
  const plain = stripMarkdownMarkers(text);
  return splitSentences(plain)
    .filter((sentence) => !looksLikeModelInstruction(sentence))
    .join(' ')
    .replace(/\s+/g, ' ')
    .trim();
}

function clipText(text: string, max: number): string {
  const chars = [...text];
  if (chars.length <= max) return text;
  return `${chars.slice(0, max).join('')}…`;
}

export function ownerFacingQuery(query: string): string {
  return clipText(instructionFree(query), 160);
}

export function ownerFacingExcerpt(text: string): string {
  return clipText(instructionFree(text), 180);
}

function urlHost(url: string): string {
  try {
    const parsed = new URL(url);
    return parsed.hostname.replace(/^www\./, '');
  } catch {
    return '';
  }
}

function ownerSourceTitle(title: string, url: string): string {
  const plain = stripMarkdownMarkers(title);
  if (plain && plain !== '来源' && plain.toLowerCase() !== 'source') return clipText(plain, 120);
  return urlHost(url);
}

/** Receipt the owner reads: their question, a source name, a short plain excerpt. */
export function ownerFacingSearchReceipt(receipt: SearchReceipt, ownerQuestion = ''): SearchReceipt {
  const question = ownerQuestion.trim();
  const queries = question
    ? [clipText(question, 160)]
    : receipt.queries.map((query) => ownerFacingQuery(query)).filter((query, index, all) => query && all.indexOf(query) === index);
  const shownQuery = queries[0] ?? '';
  return {
    queries,
    services: receipt.services,
    sources: receipt.sources.filter((source, index, all) => {
      try {
        const url = new URL(source.url);
        return ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password
          && all.findIndex(item => item.url === source.url) === index;
      } catch { return false; }
    }).map((source) => ({
      query: shownQuery || ownerFacingQuery(source.query),
      title: ownerSourceTitle(source.title, source.url),
      url: source.url,
      excerpt: ownerFacingExcerpt(source.excerpt),
    })),
  };
}

function parseReceipt(raw: string): SearchReceipt | null {
  try {
    const parsed = JSON.parse(raw) as {
      queries?: unknown;
      sources?: unknown;
      services?: unknown;
    };
    const queries = Array.isArray(parsed.queries)
      ? parsed.queries.filter((query): query is string => typeof query === 'string')
      : [];
    const sources = Array.isArray(parsed.sources)
      ? parsed.sources.flatMap((source) => {
        if (!source || typeof source !== 'object') return [];
        const row = source as Record<string, unknown>;
        return [{
          query: typeof row.query === 'string' ? row.query : '',
          title: typeof row.title === 'string' ? row.title : '',
          url: typeof row.url === 'string' ? row.url : '',
          excerpt: typeof row.excerpt === 'string' ? row.excerpt : '',
        }];
      })
      : [];
    if (queries.length === 0 && sources.length === 0) return null;
    const services = Array.isArray(parsed.services) ? parsed.services.flatMap(service => {
      if (!service || typeof service !== 'object' || typeof service.source !== 'string') return [];
      return [{ source: service.source, fallback_used: service.fallback_used === true }];
    }) : [];
    return { queries, sources, ...(services.length ? { services } : {}) };
  } catch {
    return null;
  }
}
