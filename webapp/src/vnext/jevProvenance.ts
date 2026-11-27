/**
 * Owner-visible Jev provenance for the「来源」panel.
 * Only a real allow|ask|deny verdict counts — never invent one.
 */

export type JevVerdict = 'allow' | 'ask' | 'deny';
export type JevOwnerDecision = 'approved' | 'rejected';

export type JevProvenanceView = {
  verdict: JevVerdict;
  owner_decision?: JevOwnerDecision;
  approval_id?: string;
  receipt?: string;
  audit_tip?: string;
  risk?: string;
  model?: string;
  reason?: string;
};

const VERDICTS = new Set<string>(['allow', 'ask', 'deny']);
const OWNER_DECISIONS = new Set<string>(['approved', 'rejected']);

function optionalText(value: unknown, max: number): string | undefined {
  if (typeof value !== 'string') return undefined;
  const text = value.trim();
  if (!text) return undefined;
  return text.slice(0, max);
}

/** Parse a backend `jev` object. Returns undefined unless verdict is real. */
export function parseJevProvenance(value: unknown): JevProvenanceView | undefined {
  if (!value || typeof value !== 'object') return undefined;
  const raw = value as Record<string, unknown>;
  const verdict = optionalText(raw.verdict, 16)?.toLowerCase();
  if (!verdict || !VERDICTS.has(verdict)) return undefined;
  const owner = optionalText(raw.owner_decision, 16)?.toLowerCase();
  return {
    verdict: verdict as JevVerdict,
    ...(owner && OWNER_DECISIONS.has(owner) ? { owner_decision: owner as JevOwnerDecision } : {}),
    ...(optionalText(raw.approval_id, 160) ? { approval_id: optionalText(raw.approval_id, 160) } : {}),
    ...(optionalText(raw.receipt, 160) ? { receipt: optionalText(raw.receipt, 160) } : {}),
    ...(optionalText(raw.audit_tip, 160) ? { audit_tip: optionalText(raw.audit_tip, 160) } : {}),
    ...(optionalText(raw.risk, 80) ? { risk: optionalText(raw.risk, 80) } : {}),
    ...(optionalText(raw.model, 80) ? { model: optionalText(raw.model, 80) } : {}),
    ...(optionalText(raw.reason, 240) ? { reason: optionalText(raw.reason, 240) } : {}),
  };
}

export function isJevProvenanceView(value: unknown): value is JevProvenanceView {
  return parseJevProvenance(value) !== undefined;
}
