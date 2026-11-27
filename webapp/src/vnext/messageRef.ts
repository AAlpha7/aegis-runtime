// Short public cite for one owner/Aegis chat message.
//
// The DOM id (`aegis:<clock>:<random>` vs `server:<turn>:<role>`) changes when
// a live turn is replaced by history. The cite is derived from the turn id and
// role, so the same message keeps the same token after reload.

const ALPHABET = '0123456789abcdefghjkmnpqrstvwxyz';

export type MessageRefInput = {
  role: 'owner' | 'aegis';
  turnId?: string | null;
  timestamp?: number | null;
  text?: string | null;
  fallbackId?: string | null;
};

export function messageCiteToken(ref: string): string {
  return `#${ref}`;
}

export type LineageChatCite = {
  ref: string;
  token: string;
  turnId: string;
  role: 'owner' | 'aegis';
};

/** Cite for a work-card 「对话消息」 hop. Only a webapp `turn_id` plus role
 *  matches the chat button; an excerpt without that handle is not a jump. */
export function lineageChatCite(node: {
  kind?: string | null;
  turn_id?: string | null;
  message_role?: string | null;
}): LineageChatCite | null {
  if (node.kind !== 'message') return null;
  const turnId = node.turn_id?.trim() ?? '';
  const role = node.message_role;
  if (!turnId || turnId.length > 256 || (role !== 'owner' && role !== 'aegis')) return null;
  const ref = messagePublicRef({ role, turnId });
  return { ref, token: messageCiteToken(ref), turnId, role };
}

export function messagePublicRef(input: MessageRefInput): string {
  return encodeRef(fnv1a32(citeSeed(input)));
}

export async function copyMessageRef(token: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(token);
      return true;
    }
  } catch {
    // Fall through to the textarea path (insecure context, denied permission).
  }
  try {
    const area = document.createElement('textarea');
    area.value = token;
    area.setAttribute('readonly', '');
    area.style.position = 'fixed';
    area.style.left = '-9999px';
    document.body.appendChild(area);
    area.select();
    const ok = document.execCommand('copy');
    area.remove();
    return ok;
  } catch {
    return false;
  }
}

function citeSeed(input: MessageRefInput): string {
  const role = input.role;
  const turnId = input.turnId?.trim();
  if (turnId) return `turn\n${turnId}\n${role}`;
  const text = normalizeCiteText(input.text);
  if (typeof input.timestamp === 'number' && Number.isFinite(input.timestamp)) {
    return `legacy\n${input.timestamp}\n${role}\n${text}`;
  }
  return `local\n${input.fallbackId ?? ''}\n${role}\n${text}`;
}

function normalizeCiteText(text: string | null | undefined): string {
  return (text ?? '').trim().replace(/\s+/g, ' ').slice(0, 240);
}

function fnv1a32(text: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i += 1) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

function encodeRef(hash: number): string {
  let value = hash >>> 0;
  let out = '';
  for (let i = 0; i < 6; i += 1) {
    out += ALPHABET[value & 31];
    value >>>= 5;
  }
  return out;
}
