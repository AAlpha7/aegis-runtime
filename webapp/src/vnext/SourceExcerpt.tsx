import type { ReactNode } from 'react';
import { ownerFacingUrlText } from './ownerUrlText';
import type { Locale } from '../i18n/catalog';
import { RichText } from './RichText';
import {
  emptyMessage,
  incompleteLong,
  localizeField,
  localizeNote,
  formatOwnerFacingInstants,
  localizeProse,
  readSourceEvidence,
} from './sourceEvidenceText';
import type { SourceNode } from './sourceEvidenceText';
import { useUserTimezone } from './UserTimezoneContext';

const URL_PATTERN = /https:\/\/[^\s<>"`[\]]+/g;

function safeHttps(raw: string): string | null {
  try {
    const url = new URL(raw);
    if (url.protocol !== 'https:' || !url.hostname || url.username || url.password) return null;
    return url.href;
  } catch {
    return null;
  }
}

function EvidenceText({ text }: { text: string }) {
  const parts: ReactNode[] = [];
  let last = 0;
  for (const match of text.matchAll(URL_PATTERN)) {
    const start = match.index ?? 0;
    if (start > last) parts.push(text.slice(last, start));
    const href = safeHttps(match[0]);
    if (href) parts.push(<a key={start} href={href}>{ownerFacingUrlText(match[0])}</a>);
    else parts.push(match[0]);
    last = start + match[0].length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return <>{parts}</>;
}

function factMark(value: string, highlightFact?: string | null): { 'data-source-fact'?: string; className?: string } {
  const fact = value.trim();
  if (fact.length < 2) return {};
  return {
    'data-source-fact': fact,
    className: highlightFact === fact ? 'is-cited-fact' : undefined,
  };
}

function Leaf({ node, locale, zone, highlightFact }: { node: SourceNode; locale: Locale; zone: string | null; highlightFact?: string | null }) {
  if (node.kind === 'note') return <p>{localizeNote(node.text, locale)}</p>;
  if (node.kind === 'text') {
    const text = formatOwnerFacingInstants(node.text, zone);
    return <p {...factMark(text, highlightFact)}><EvidenceText text={text} /></p>;
  }
  if (node.kind === 'field') return <FieldView node={node} locale={locale} zone={zone} highlightFact={highlightFact} />;
  return <ItemChildren nodes={node.children} locale={locale} zone={zone} highlightFact={highlightFact} />;
}

function FieldView({ node, locale, zone, highlightFact }: { node: Extract<SourceNode, { kind: 'field' }>; locale: Locale; zone: string | null; highlightFact?: string | null }) {
  const shown = localizeField(node.label, node.value, locale, zone);
  return (
    <>
      <p {...factMark(shown.value, highlightFact)}>
        {shown.label}{shown.value ? <>: <EvidenceText text={shown.value} /></> : ':'}
      </p>
      {node.children.length > 0 && (
        <ul className="vnext-source-tree">
          {node.children.map((child, index) => (
            <li key={index}><Leaf node={child} locale={locale} zone={zone} highlightFact={highlightFact} /></li>
          ))}
        </ul>
      )}
    </>
  );
}

function ItemChildren({ nodes, locale, zone, highlightFact }: { nodes: SourceNode[]; locale: Locale; zone: string | null; highlightFact?: string | null }) {
  const first = nodes[0];
  const titled = first !== undefined && (first.kind === 'text' || first.kind === 'note');
  const body = titled ? nodes.slice(1) : nodes;
  return (
    <>
      {titled && first ? <Leaf node={first} locale={locale} zone={zone} highlightFact={highlightFact} /> : null}
      {body.length > 0 && (
        <ul className="vnext-source-tree">
          {body.map((node, index) => (
            <li key={index}><Leaf node={node} locale={locale} zone={zone} highlightFact={highlightFact} /></li>
          ))}
        </ul>
      )}
    </>
  );
}

function SourceNodes({ nodes, locale, zone, highlightFact }: { nodes: SourceNode[]; locale: Locale; zone: string | null; highlightFact?: string | null }) {
  const chunks: ReactNode[] = [];
  let index = 0;
  while (index < nodes.length) {
    const node = nodes[index];
    if (node.kind === 'item') {
      const start = index;
      const items: Array<Extract<SourceNode, { kind: 'item' }>> = [];
      while (index < nodes.length && nodes[index].kind === 'item') {
        items.push(nodes[index] as Extract<SourceNode, { kind: 'item' }>);
        index += 1;
      }
      chunks.push(
        <ul className="vnext-source-tree" key={`items-${start}`}>
          {items.map((item, itemIndex) => (
            <li key={itemIndex}><ItemChildren nodes={item.children} locale={locale} zone={zone} highlightFact={highlightFact} /></li>
          ))}
        </ul>,
      );
      continue;
    }
    chunks.push(<Leaf key={`node-${index}`} node={node} locale={locale} zone={zone} highlightFact={highlightFact} />);
    index += 1;
  }
  return <>{chunks}</>;
}

export function SourceExcerpt({ excerpt, locale, highlightFact }: { excerpt: string; locale: Locale; highlightFact?: string | null }) {
  const zone = useUserTimezone();
  const body = readSourceEvidence(excerpt);
  if (body.kind === 'incomplete') return <p>{incompleteLong(locale)}</p>;
  if (body.kind === 'prose') {
    return <RichText content={localizeProse(body.text, locale, zone)} disableImportantDetection />;
  }
  if (body.nodes.length === 0) return <p>{emptyMessage(locale)}</p>;
  return <SourceNodes nodes={body.nodes} locale={locale} zone={zone} highlightFact={highlightFact} />;
}
