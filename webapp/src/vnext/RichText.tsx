import { Fragment, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { ownerFacingUrlText } from './ownerUrlText';
import { useLocale, useT } from '../i18n/LocaleContext';

type RichTextProps = {
  content: string;
  className?: string;
  importanceSeverity?: NoticeSeverity | null;
  importanceDismissKey?: string;
  /** Render quoted/evidence text without promoting lines to alert callouts. */
  disableImportantDetection?: boolean;
  /**
   * Optional transform for visible text tokens only (skips `<a>` and `<code>`).
   * Used by the assistant answer view to turn `[E1]` into chips and mark
   * unsupported numbers without re-parsing markdown as HTML.
   */
  decorateText?: (text: string) => ReactNode;
};

type NoticeSeverity = 'info' | 'notice' | 'important' | 'warning' | 'critical';

type InlineToken =
  | { kind: 'text'; value: string }
  | { kind: 'image'; alt: string; src: string }
  | { kind: 'code'; value: string }
  | { kind: 'strong'; value: string }
  | { kind: 'emphasis'; value: string }
  | { kind: 'link'; label: string; href: string };

type ImportantCalloutStart = {
  title: string;
  item?: string;
  consumeHeading: boolean;
};

type RichTextParseOptions = {
  importanceDismissed: boolean;
  onToggleImportance?: () => void;
  disableImportantDetection?: boolean;
  /**
   * The localized generic "Important" label (catalog key `callout.important`).
   * Substituted for the default `'Important'` callout title so the label follows
   * the active locale instead of leaking English into a Chinese session.
   */
  importantLabel: string;
  reminderLabel: string;
  alertLabel: string;
  restartLabel: string;
  resourceLabel: string;
  /** Localized "Dismissed importance" callout title (catalog key `callout.dismissedImportance`). */
  dismissedImportanceLabel: string;
  /** Localized "Dismiss" button label (catalog key `callout.dismiss`). */
  dismissLabel: string;
  /** Localized "Restore" button label (catalog key `callout.restore`). */
  restoreLabel: string;
  /** Localized "Code" fence label (catalog key `richText.code`). */
  codeLabel: string;
  /**
   * Localized fallback alt text for images with empty alt (catalog key
   * `app.richText.generatedImageAlt`). Substituted for the default
   * `'Generated image'` so a Chinese session does not leak English alt text.
   */
  imageAltFallback: string;
  decorateText?: (text: string) => ReactNode;
};

const DISMISSED_IMPORTANCE_PREFIX = 'aegis:vnext:dismissed-importance:';

let richTextKey = 0;

function nextKey(prefix: string): string {
  richTextKey += 1;
  return `${prefix}-${richTextKey}`;
}

export function RichText({
  content,
  className,
  importanceSeverity,
  importanceDismissKey,
  disableImportantDetection = false,
  decorateText,
}: RichTextProps) {
  const effectiveImportanceDismissKey = useMemo(
    () => importanceDismissKey || stableImportanceDismissKey(content),
    [content, importanceDismissKey],
  );
  const [importanceDismissed, setImportanceDismissed] = useState(
    () => readImportanceDismissed(effectiveImportanceDismissKey),
  );
  useEffect(() => {
    setImportanceDismissed(readImportanceDismissed(effectiveImportanceDismissKey));
  }, [effectiveImportanceDismissKey]);

  function toggleImportance() {
    const next = !importanceDismissed;
    setImportanceDismissed(next);
    writeImportanceDismissed(effectiveImportanceDismissKey, next);
  }

  const tr = useT();
  const { locale } = useLocale();
  const shownContent = locale === 'zh' ? stripParallelEnglish(content) : content;
  const importantLabel = tr('callout.important');
  const reminderLabel = tr('callout.reminder');
  const alertLabel = tr('callout.alert');
  const restartLabel = tr('callout.systemRestart');
  const resourceLabel = tr('callout.resourcePressure');
  const dismissedImportanceLabel = tr('callout.dismissedImportance');
  const dismissLabel = tr('callout.dismiss');
  const restoreLabel = tr('callout.restore');
  const codeLabel = tr('richText.code');
  const imageAltFallback = tr('app.richText.generatedImageAlt');
  const forceImportantCallout = severityForcesImportantCallout(importanceSeverity)
    && !contentHasImportantCalloutStart(shownContent);
  const blocks = parseBlocks(shownContent, {
    importanceDismissed,
    onToggleImportance: toggleImportance,
    disableImportantDetection: forceImportantCallout || disableImportantDetection,
    importantLabel,
    reminderLabel,
    alertLabel,
    restartLabel,
    resourceLabel,
    dismissedImportanceLabel,
    dismissLabel,
    restoreLabel,
    codeLabel,
    imageAltFallback,
    decorateText,
  });
  const renderedBlocks = forceImportantCallout
    ? [
        renderImportantCallout({
          title: severityCalloutTitle(importanceSeverity, tr),
          key: 'severity-callout',
          options: {
            importanceDismissed,
            onToggleImportance: toggleImportance,
            importantLabel,
            reminderLabel,
            alertLabel,
            restartLabel,
            resourceLabel,
            dismissedImportanceLabel,
            dismissLabel,
            restoreLabel,
            codeLabel,
            imageAltFallback,
            decorateText,
          },
          children: blocks.length > 0 ? blocks : [<p key="severity-fallback">{shownContent.trim()}</p>],
        }),
      ]
    : blocks;
  return (
    <div className={['vnext-rich-text', className].filter(Boolean).join(' ')}>
      {renderedBlocks.length > 0 ? renderedBlocks : <p>{shownContent.trim()}</p>}
    </div>
  );
}

function parseBlocks(content: string, options: RichTextParseOptions): ReactNode[] {
  const lines = content.replace(/\r\n?/g, '\n').split('\n');
  const blocks: ReactNode[] = [];
  let paragraph: string[] = [];
  let listItems: string[] = [];
  let orderedListItems: string[] = [];
  let orderedListStart = 1;
  let quoteLines: string[] = [];
  let codeLines: string[] | null = null;
  let codeLanguage = '';

  function flushParagraph() {
    const text = paragraph.join(' ').trim();
    paragraph = [];
    if (!text) return;
    const image = imageMarkdown(text);
    if (image) {
      blocks.push(renderImageFigure(image.alt, image.src, nextKey('image'), options.imageAltFallback));
    } else {
      blocks.push(<p key={nextKey('p')}>{renderInline(text, options)}</p>);
    }
  }

  function flushList() {
    if (listItems.length > 0) {
      const items = listItems;
      listItems = [];
      blocks.push(<ul key={nextKey('ul')}>{items.map((item) => renderListItem(item, options))}</ul>);
    }
    if (orderedListItems.length > 0) {
      const items = orderedListItems;
      orderedListItems = [];
      const references = items.every(item => /^\[[^\]]+\]\(https?:\/\/[^\s)]+\)$/.test(item));
      blocks.push(<ol key={nextKey('ol')} start={orderedListStart === 1 ? undefined : orderedListStart} className={references ? 'vnext-reference-list' : undefined}
        style={orderedListStart === 1 ? undefined : { counterReset: `aegis-rich-ol ${orderedListStart - 1}` }}>{items.map((item) => <li key={nextKey('li')}>{renderInline(item, options)}</li>)}</ol>);
    }
  }

  function flushQuote() {
    const text = quoteLines.join(' ').trim();
    quoteLines = [];
    if (text) blocks.push(<blockquote key={nextKey('quote')}>{renderInline(text, options)}</blockquote>);
  }

  function flushOpenText() {
    flushParagraph();
    flushList();
    flushQuote();
  }

  for (let lineIndex = 0; lineIndex < lines.length; lineIndex += 1) {
    const rawLine = lines[lineIndex];
    const line = rawLine.trimEnd();
    const trimmed = line.trim();

    if (codeLines) {
      if (/^```/.test(trimmed)) {
        const code = codeLines.join('\n');
        const languageLabel = codeLanguage ? `${options.codeLabel} · ${codeLanguage}` : options.codeLabel;
        blocks.push(
          <figure className="vnext-rich-code" key={nextKey('code')}>
            <figcaption>{languageLabel}</figcaption>
            <pre><code>{code}</code></pre>
          </figure>,
        );
        codeLines = null;
        codeLanguage = '';
      } else {
        codeLines.push(rawLine);
      }
      continue;
    }

    const codeFence = trimmed.match(/^```([A-Za-z0-9_-]*)\s*$/);
    if (codeFence) {
      flushOpenText();
      codeLines = [];
      codeLanguage = codeFence[1] ?? '';
      continue;
    }

    if (!trimmed) {
      flushOpenText();
      continue;
    }

    const importantCallout = options.disableImportantDetection ? null : importantCalloutStart(trimmed);
    if (importantCallout) {
      flushOpenText();
      const items: string[] = [];
      if (importantCallout.item) items.push(importantCallout.item);
      while (lineIndex + 1 < lines.length) {
        const nextLine = lines[lineIndex + 1].trim();
        if (!nextLine || startsNewRichTextBlock(nextLine)) break;
        if (importantCallout.consumeHeading && isImportantCalloutHeading(nextLine)) {
          lineIndex += 1;
          continue;
        }
        items.push(stripListPrefix(nextLine));
        lineIndex += 1;
      }
      blocks.push(renderImportantCallout({ title: importantCallout.title, items, options, key: nextKey('callout') }));
      continue;
    }

    const nextTrimmed = lines[lineIndex + 1]?.trim() ?? '';
    if (looksLikeTableRow(trimmed) && isTableSeparator(nextTrimmed)) {
      flushOpenText();
      const headers = parseTableRow(trimmed);
      const rows: string[][] = [];
      lineIndex += 2;
      while (lineIndex < lines.length && looksLikeTableRow(lines[lineIndex].trim())) {
        rows.push(parseTableRow(lines[lineIndex].trim()));
        lineIndex += 1;
      }
      lineIndex -= 1;
      blocks.push(
        <div className="vnext-rich-table-scroll" key={nextKey('table')}>
          <table>
            <thead>
              <tr>{headers.map((header) => <th key={nextKey('th')}>{renderInline(header, options)}</th>)}</tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={nextKey('tr')}>
                  {headers.map((_, cellIndex) => (
                    <td key={nextKey('td')}>{renderInline(row[cellIndex] ?? '', options)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }

    const heading = trimmed.match(/^(#{1,3})\s+(.+)$/);
    if (heading) {
      flushOpenText();
      const level = heading[1].length;
      const text = heading[2].trim();
      if (level === 1) blocks.push(<h3 key={nextKey('h')}>{renderInline(text, options)}</h3>);
      else if (level === 2) blocks.push(<h4 key={nextKey('h')}>{renderInline(text, options)}</h4>);
      else blocks.push(<h5 key={nextKey('h')}>{renderInline(text, options)}</h5>);
      continue;
    }

    const quote = trimmed.match(/^>\s?(.+)$/);
    if (quote) {
      flushParagraph();
      flushList();
      quoteLines.push(quote[1]);
      continue;
    }

    const bullet = trimmed.match(/^(?:[-*]\s+|•\s*)(.+)$/);
    if (bullet) {
      flushParagraph();
      flushQuote();
      if (orderedListItems.length > 0) flushList();
      listItems.push(bullet[1]);
      continue;
    }

    // ASCII numeric-dot markers require whitespace: decimals, dates and
    // versions are source facts, not list numbering.
    const ordered = trimmed.match(/^(?:\d+\.\s+|(?:\d+[)、．]|[一二三四五六七八九十]+[.)、．]|[（(](?:\d+|[一二三四五六七八九十]+)[）)])\s*)(.+)$/);
    if (ordered) {
      flushParagraph();
      flushQuote();
      if (listItems.length > 0) flushList();
      if (orderedListItems.length === 0) orderedListStart = Number(trimmed.match(/^(\d+)/)?.[1] ?? 1);
      // Keep a reference title and its next-line URL together. This also
      // accommodates plain-text source lists, not only Markdown links.
      let next = lineIndex + 1;
      while (next < lines.length && !lines[next].trim()) next += 1;
      const url = lines[next]?.trim();
      if (url && /^https?:\/\/[^\s<>]+$/.test(url) && !ordered[1].includes('](')) {
        orderedListItems.push(`[${ordered[1].replace(/[\[\]]/g, '')}](${url})`);
        lineIndex = next;
      } else {
        orderedListItems.push(ordered[1]);
      }
      continue;
    }

    flushList();
    flushQuote();
    paragraph.push(trimmed);
  }

  if (codeLines) {
    blocks.push(
      <figure className="vnext-rich-code" key={nextKey('code')}>
        <figcaption>{codeLanguage ? `${options.codeLabel} · ${codeLanguage}` : options.codeLabel}</figcaption>
        <pre><code>{codeLines.join('\n')}</code></pre>
      </figure>,
    );
  }
  flushOpenText();
  return blocks;
}

function renderImportantCallout({
  title,
  items,
  options,
  key,
  children,
}: {
  title: string;
  items?: string[];
  options: Pick<
    RichTextParseOptions,
    'importanceDismissed' | 'onToggleImportance' | 'importantLabel'
    | 'reminderLabel' | 'alertLabel' | 'restartLabel' | 'resourceLabel'
    | 'dismissedImportanceLabel' | 'dismissLabel' | 'restoreLabel' | 'codeLabel'
    | 'imageAltFallback' | 'decorateText'
  >;
  key: string;
  children?: ReactNode[];
}): ReactNode {
  const displayedTitle = calloutTitle(title, options);
  return (
    <aside
      className={options.importanceDismissed ? 'vnext-rich-callout important is-dismissed' : 'vnext-rich-callout important'}
      key={key}
    >
      <div className="vnext-rich-callout-head">
        <strong>
          <span aria-hidden="true">{options.importanceDismissed ? '✓' : '‼️'}</span>
          {' '}
          {options.importanceDismissed ? options.dismissedImportanceLabel : displayedTitle}
        </strong>
        {options.onToggleImportance && (
          <button type="button" onClick={options.onToggleImportance}>
            {options.importanceDismissed ? options.restoreLabel : options.dismissLabel}
          </button>
        )}
      </div>
      {items && items.length > 0 && <ul>{items.map((item) => <li key={nextKey('callout-li')}>{renderInline(item, options)}</li>)}</ul>}
      {children && children.length > 0 && <div className="vnext-rich-callout-body">{children}</div>}
    </aside>
  );
}

function stableImportanceDismissKey(content: string): string {
  const normalized = content.trim().replace(/\s+/g, ' ');
  let hash = 2166136261;
  for (let i = 0; i < normalized.length; i += 1) {
    hash ^= normalized.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return hash.toString(36);
}

function readImportanceDismissed(key: string): boolean {
  if (typeof window === 'undefined') return false;
  return window.localStorage.getItem(`${DISMISSED_IMPORTANCE_PREFIX}${key}`) === '1';
}

function writeImportanceDismissed(key: string, dismissed: boolean) {
  if (typeof window === 'undefined') return;
  const storageKey = `${DISMISSED_IMPORTANCE_PREFIX}${key}`;
  if (dismissed) window.localStorage.setItem(storageKey, '1');
  else window.localStorage.removeItem(storageKey);
}

function severityForcesImportantCallout(severity?: NoticeSeverity | null): boolean {
  return severity === 'important' || severity === 'warning' || severity === 'critical';
}

function severityCalloutTitle(severity: NoticeSeverity | null | undefined, tr: (key: string) => string): string {
  if (severity === 'critical') return tr('callout.critical');
  if (severity === 'warning') return tr('callout.warning');
  return tr('callout.important');
}

function contentHasImportantCalloutStart(content: string): boolean {
  return content
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .some((line) => importantCalloutStart(line.trim()) !== null);
}

export const richTextImportanceTestHooks = {
  stableImportanceDismissKey,
  readImportanceDismissed,
  writeImportanceDismissed,
};

function calloutTitle(
  title: string,
  options: Pick<RichTextParseOptions, 'importantLabel' | 'reminderLabel' | 'alertLabel' | 'restartLabel' | 'resourceLabel'>,
): string {
  switch (title) {
    case 'Important':
      return options.importantLabel;
    case 'reminder':
      return options.reminderLabel;
    case 'alert':
      return options.alertLabel;
    case 'restart':
      return options.restartLabel;
    case 'resource':
      return options.resourceLabel;
    default:
      return title;
  }
}

function stripParallelEnglish(content: string): string {
  return content
    .replace(/\bHITL 待审批/g, '待确认')
    .split('\n')
    .map((line) => {
      const match = line.match(/^(.*[\u4e00-\u9fff][^/\n]*?)\s+\/\s+[A-Za-z][^:\n]*?(?::\s*(.*))?$/);
      if (!match) return line;
      const left = match[1].trim();
      const value = match[2]?.trim();
      if (!value || left.includes(value)) return left;
      return `${left}: ${value}`;
    })
    .join('\n');
}

function importantCalloutStart(line: string): ImportantCalloutStart | null {
  if (isImportantCalloutHeading(line)) {
    return { title: 'Important', consumeHeading: true };
  }
  if (/^⏰/.test(line) || /^(?:reminder|提醒)\b[:：]?/i.test(line)) {
    return { title: 'reminder', item: line, consumeHeading: false };
  }
  if (/系统已重启|system restarted/i.test(line)) {
    return { title: 'restart', item: line, consumeHeading: true };
  }
  if (/^🚨|^alerts?[:：]?$/i.test(line)) {
    return { title: 'alert', item: line, consumeHeading: false };
  }
  if (resourcePressureLine(line)) {
    return { title: 'resource', item: line, consumeHeading: false };
  }
  return null;
}

function isImportantCalloutHeading(line: string): boolean {
  return /^(?:当前)?重要信息[:：]?$/i.test(line)
    || /^important (?:information|info)[:：]?$/i.test(line)
    || /^current important (?:information|info)[:：]?$/i.test(line);
}

function resourcePressureLine(line: string): boolean {
  return /exceeds threshold/i.test(line)
    || /\b(?:disk|memory|cpu)\b.*\b(?:high|critical|exceeds|pressure|usage)\b/i.test(line)
    || /(?:磁盘|内存|CPU|资源).*(?:紧张|过高|告警|超过|不足)/i.test(line);
}

function startsNewRichTextBlock(line: string): boolean {
  return /^```/.test(line)
    || /^#{1,3}\s+/.test(line)
    || /^>\s?/.test(line)
    || looksLikeTableRow(line);
}

function stripListPrefix(line: string): string {
  return line.replace(/^(?:[-*]\s+|•\s*)/, '').trim();
}

function renderListItem(item: string, options: Pick<RichTextParseOptions, 'imageAltFallback' | 'decorateText'>): ReactNode {
  const checklist = item.match(/^\[([ xX])\]\s+(.+)$/);
  if (!checklist) return <li key={nextKey('li')}>{renderInline(item, options)}</li>;
  const checked = checklist[1].toLowerCase() === 'x';
  return (
    <li key={nextKey('li')} className={checked ? 'vnext-rich-check-item is-checked' : 'vnext-rich-check-item'}>
      <span className="vnext-rich-check" aria-hidden="true">{checked ? '✓' : ''}</span>
      <span>{renderInline(checklist[2], options)}</span>
    </li>
  );
}

function looksLikeTableRow(line: string): boolean {
  return line.includes('|') && parseTableRow(line).length >= 2;
}

function isTableSeparator(line: string): boolean {
  if (!looksLikeTableRow(line)) return false;
  return parseTableRow(line).every((cell) => /^:?-{3,}:?$/.test(cell.replace(/\s+/g, '')));
}

function parseTableRow(line: string): string[] {
  return line
    .replace(/^\s*\|/, '')
    .replace(/\|\s*$/, '')
    .split('|')
    .map((cell) => cell.trim());
}

function renderInline(
  text: string,
  options: Pick<RichTextParseOptions, 'imageAltFallback' | 'decorateText'>,
): ReactNode[] {
  const imageAltFallback = options.imageAltFallback ?? 'Generated image';
  return tokenizeInline(text).map((token) => {
    switch (token.kind) {
      case 'code':
        return <code key={nextKey('inline-code')}>{token.value}</code>;
      case 'strong':
        return <strong key={nextKey('strong')}>{decorateInlineText(token.value, options)}</strong>;
      case 'emphasis':
        return <em key={nextKey('em')}>{decorateInlineText(token.value, options)}</em>;
      case 'link': {
        const safeHref = safeLink(token.href);
        if (!safeHref) return <span key={nextKey('unsafe-link')}>{token.label}</span>;
        const isExternal = safeHref.startsWith('https://');
        return (
          <a
            key={nextKey('link')}
            href={safeHref}
            target={isExternal ? '_blank' : undefined}
            rel={isExternal ? 'noopener noreferrer' : undefined}
          >
            {ownerFacingUrlText(token.label)}
          </a>
        );
      }
      case 'image': {
        return renderImageFigure(token.alt, token.src, nextKey('image'), imageAltFallback) ?? <span key={nextKey('unsafe-image')}>{token.alt}</span>;
      }
      default:
        return decorateInlineText(token.value, options);
    }
  });
}

function decorateInlineText(
  text: string,
  options: Pick<RichTextParseOptions, 'decorateText'>,
): ReactNode {
  if (!options.decorateText) return text;
  return <Fragment key={nextKey('decorated')}>{options.decorateText(text)}</Fragment>;
}

function imageMarkdown(text: string): { alt: string; src: string } | null {
  const image = text.match(/^!\[([^\]]*)\]\(([^)]+)\)$/);
  if (!image) return null;
  return { alt: image[1], src: image[2] };
}

function renderImageFigure(alt: string, src: string, key: string, imageAltFallback = 'Generated image'): ReactNode | null {
  const safeSrc = safeImageSrc(src);
  if (!safeSrc) return null;
  return (
    <figure key={key} className="vnext-rich-image">
      <img src={safeSrc} alt={alt || imageAltFallback} loading="lazy" decoding="async" />
      {alt && <figcaption>{alt}</figcaption>}
    </figure>
  );
}

function tokenizeInline(text: string): InlineToken[] {
  const tokens: InlineToken[] = [];
  const pattern = /(!\[[^\]]*\]\([^)]+\)|`[^`]+`|\*\*[^*]+\*\*|\*[^*]+\*|\[[^\]]+\]\([^)]+\)|https:\/\/[^\s<>"`\[\]，。；！？]+)/g;
  let cursor = 0;
  for (const match of text.matchAll(pattern)) {
    const matched = match[0];
    const index = match.index ?? 0;
    if (index > cursor) tokens.push({ kind: 'text', value: text.slice(cursor, index) });
    if (matched.startsWith('![')) {
      const image = matched.match(/^!\[([^\]]*)\]\(([^)]+)\)$/);
      if (image) tokens.push({ kind: 'image', alt: image[1], src: image[2] });
      else tokens.push({ kind: 'text', value: matched });
    } else if (matched.startsWith('`')) {
      tokens.push({ kind: 'code', value: matched.slice(1, -1) });
    } else if (matched.startsWith('**')) {
      tokens.push({ kind: 'strong', value: matched.slice(2, -2) });
    } else if (matched.startsWith('*')) {
      tokens.push({ kind: 'emphasis', value: matched.slice(1, -1) });
    } else if (matched.startsWith('https://')) {
      let href = matched.replace(/[.,;:!?]+$/, '');
      while (href.endsWith(')') && (href.match(/\)/g)?.length ?? 0) > (href.match(/\(/g)?.length ?? 0)) href = href.slice(0, -1);
      tokens.push({kind: 'link', label: href, href});
      if (href.length < matched.length) tokens.push({kind: 'text', value: matched.slice(href.length)});
    } else {
      const link = matched.match(/^\[([^\]]+)\]\(([^)]+)\)$/);
      if (link) tokens.push({ kind: 'link', label: link[1], href: link[2] });
      else tokens.push({ kind: 'text', value: matched });
    }
    cursor = index + matched.length;
  }
  if (cursor < text.length) tokens.push({ kind: 'text', value: text.slice(cursor) });
  return tokens;
}

function safeLink(rawHref: string): string | null {
  const href = rawHref.trim();
  if (!href || href !== rawHref) return null;
  if (href.startsWith('/webapp/vnext/generated-image/')) return href;
  try {
    const url = new URL(href);
    if (!['https:', 'mailto:'].includes(url.protocol)) return null; // https + email links
    if (url.protocol === 'https:' && (!url.hostname || url.username || url.password)) return null;
    return url.href;
  } catch {
    return null;
  }
}

function safeImageSrc(rawSrc: string): string | null {
  const src = rawSrc.trim();
  if (!src || src !== rawSrc) return null;
  if (/^\/webapp\/vnext\/generated-image\/[a-z0-9_.-]+\.(?:png|jpg|jpeg|webp)$/i.test(src)) {
    return src;
  }
  return null;
}
