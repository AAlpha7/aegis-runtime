/** Facts on an egress-gate hold card. The runtime writes these lines; the card does not infer them. */

export type EgressStopped = 'sensitive' | 'untrusted_source' | 'jev_unavailable';
export type UntrustedSourceKind = 'web_page' | 'post' | 'file' | 'tool_result';

export interface EgressHoldCard {
  destination: string;
  wouldSend: string;
  stopped: EgressStopped;
  untrustedSource?: UntrustedSourceKind;
}

const STOPPED: readonly EgressStopped[] = ['sensitive', 'untrusted_source', 'jev_unavailable'];
const SOURCES: readonly UntrustedSourceKind[] = ['web_page', 'post', 'file', 'tool_result'];

function field(lines: string[], prefix: string): string | null {
  const line = lines.find((item) => item.startsWith(prefix));
  if (!line) return null;
  const value = line.slice(prefix.length).trim();
  return value.length > 0 ? value : null;
}

export function parseEgressHoldCard(summary: string | null | undefined): EgressHoldCard | null {
  if (!summary || !summary.trimStart().startsWith('destination:')) return null;
  const lines = summary.split('\n').map((line) => line.trim()).filter(Boolean);
  const destination = field(lines, 'destination:');
  const wouldSend = field(lines, 'would_send:');
  const stopped = field(lines, 'stopped:');
  if (!destination || !wouldSend || !stopped) return null;
  if (!STOPPED.includes(stopped as EgressStopped)) return null;
  const source = field(lines, 'untrusted_source:');
  const untrustedSource = source && SOURCES.includes(source as UntrustedSourceKind)
    ? source as UntrustedSourceKind
    : undefined;
  return {
    destination,
    wouldSend,
    stopped: stopped as EgressStopped,
    untrustedSource,
  };
}
