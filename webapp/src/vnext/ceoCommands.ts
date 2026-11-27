import { t, tf, preferredLocale, type Locale } from '../i18n/catalog';
/**
 * CEO chief-of-staff command registry — single source of truth for:
 *   - Realtime tools (session.update)
 *   - Text-mode tool dispatch
 *   - Confirm-required high-risk verbs
 *
 * Handlers are supplied by App (closures over navigate / work actions / chat).
 * Pattern: spoken voice commands mapped to console actions.
 */

export type CeoCommandResult = {
  ok: boolean;
  message: string;
  /** When true, UI should show CEO Review Card before re-dispatch with confirmed. */
  needsConfirmation?: boolean;
};

export type CeoCommandHandler = (
  args: Record<string, unknown>,
) => CeoCommandResult | Promise<CeoCommandResult>;

export type CeoHandlers = Record<string, CeoCommandHandler>;

type ParamSpec = {
  type: string;
  description: string;
  enum?: readonly string[];
};

export type CeoCommandSpec = {
  name: string;
  description: string;
  confirmRequired?: boolean;
  parameters: {
    type: 'object';
    properties: Record<string, ParamSpec>;
    required?: readonly string[];
  };
};

const CONFIRMED: ParamSpec = {
  type: 'boolean',
  description:
    'Set true ONLY after the owner verbally or in the panel confirmed this exact action. Never invent confirmation.',
};

/** Canonical chief-of-staff verbs for tools + dispatcher. */
export const CEO_COMMANDS: readonly CeoCommandSpec[] = [
  {
    name: 'get_screen_context',
    description:
      'Get a fresh snapshot of what the owner is looking at: surface, focused item, needs-you list. Prefer this before acting on "this / that / current".',
    parameters: { type: 'object', properties: {} },
  },
  {
    name: 'what_needs_me',
    description: 'List the top items that need the CEO now (decisions + urgent now). Short owner language.',
    parameters: { type: 'object', properties: {} },
  },
  {
    name: 'open_surface',
    description:
      'REQUIRED for any pure UI navigation. Switch the owner WebApp to home, work, status, or settings. '
      + 'Use when the owner says 打开/去/看 + 工作/设置/首页, or open/show/go to work|settings|home. '
      + 'Never claim you opened a surface without calling this and getting ok=true. '
      + 'Do NOT use ask_chief for pure navigation — backend reading is not the same as opening the UI.',
    parameters: {
      type: 'object',
      properties: {
        surface: {
          type: 'string',
          enum: ['home', 'work', 'status', 'settings'],
          description: 'Target surface',
        },
        itemId: { type: 'string', description: 'Optional work item id when opening work.' },
      },
      required: ['surface'],
    },
  },
  {
    name: 'ask_chief',
    description:
      'Send free-text to the chief-of-staff runtime for research, commute, drafts — '
      + 'anything not covered by a more specific tool. '
      + 'Do NOT use this alone for pure "打开工作/open work" navigation (use open_surface). '
      + 'Do NOT use this alone for 批准/reject of a pending gate (use approve_pending / reject_pending). '
      + 'Current screen context is already in instructions.',
    parameters: {
      type: 'object',
      properties: {
        text: { type: 'string', description: 'Owner request in their language.' },
      },
      required: ['text'],
    },
  },
  {
    name: 'approve_pending',
    description:
      'Approve a pending action when the owner says 批 / 批准 / 就这样 / 好 / ok / approve. '
      + 'Settles (1) a runtime HITL gate from a prior tool, '
      + 'or (2) a focused Work decide/approval item. Prefer this over re-asking ask_chief.',
    // Owner saying 批准 is the confirmation; do not require a second confirmed=true hop
    // (that blocked voice "批准标记已读" after the gate was already raised).
    parameters: {
      type: 'object',
      properties: {
        itemId: { type: 'string', description: 'Work item id; omit for current focus / latest pending gate.' },
      },
    },
  },
  {
    name: 'reject_pending',
    description:
      'Reject a pending action when the owner says 不用 / 否 / 拒绝 / no / reject. '
      + 'Settles a runtime HITL gate or a focused Work decide/approval item.',
    parameters: {
      type: 'object',
      properties: {
        itemId: { type: 'string', description: 'Work item id; omit for current focus / latest pending gate.' },
      },
    },
  },
  {
    name: 'complete_task',
    description: 'Mark a task/knowledge item done.',
    confirmRequired: true,
    parameters: {
      type: 'object',
      properties: {
        itemId: { type: 'string', description: 'Work item id; omit for current focus.' },
        confirmed: CONFIRMED,
      },
    },
  },
  {
    name: 'draft_reply',
    description: 'Ask the chief of staff to draft a reply for the focused thread (does not send).',
    parameters: {
      type: 'object',
      properties: {
        itemId: { type: 'string', description: 'Optional focused item id.' },
        instruction: { type: 'string', description: 'Optional tone / points to include.' },
      },
    },
  },
];

export const CEO_PERSONA_INSTRUCTIONS = `You are Aegis, the sole chief-of-staff for one very busy CEO.
You are the owner’s primary communication interface and the system’s representative — not a side feature of a “Chat” page.
Work, Home desk, and other views are expressive context (what is on the table); talk through you while they browse.
Speak concise Simplified Chinese unless the owner uses English.
You can see structured SCREEN CONTEXT (surface, focus, needs-you) — use it for "这个/当前".

UI truth rules (critical):
- "打开/去/看 工作|设置|首页" means switch the WebApp view via open_surface.
- Never say you opened work/settings unless the corresponding tool returned ok=true.
- Prefer open_surface for surfaces; ask_chief for open-ended research/summary after the view is open if needed.
- Prefer tools for navigation and decisions; use ask_chief for open-ended work.
- When a tool result or on-screen card says needs confirmation / 需要你确认, and the owner says 批准/批/好/就这样, call approve_pending immediately (not ask_chief again).
- When the owner says 不用/否/拒绝 after a pending gate, call reject_pending.

NEVER send, pay, delete permanently, or approve high-risk actions without confirmed=true after the owner agrees.
Important / irreversible / external actions: restate what you will do, ask for confirmation, then call the tool with confirmed=true.
One clarifying question max. Finish with a short result + receipt.`;

export function realtimeCeoTools(specs: readonly CeoCommandSpec[] = CEO_COMMANDS) {
  return specs.map((spec) => ({
    type: 'function' as const,
    name: spec.name,
    description: spec.description,
    parameters: spec.parameters,
  }));
}

export type DispatchCeoCommandOptions = {
  specs?: readonly CeoCommandSpec[];
  /**
   * Display locale for dispatcher-authored messages (unknown command, bad
   * args, confirmation prompt). Callers inside the console pass the context
   * locale; the default follows the `<html lang>` the LocaleProvider stamps.
   */
  locale?: Locale;
};

export async function dispatchCeoCommand(
  handlers: CeoHandlers,
  name: string,
  argsJson: string,
  options: DispatchCeoCommandOptions = {},
): Promise<CeoCommandResult> {
  const specs = options.specs ?? CEO_COMMANDS;
  const locale = options.locale ?? preferredLocale();
  const spec = specs.find((s) => s.name === name);
  if (!spec) {
    return { ok: false, message: tf('ceo.command.unknown', locale, { name }) };
  }
  let args: Record<string, unknown> = {};
  try {
    args = argsJson ? (JSON.parse(argsJson) as Record<string, unknown>) : {};
  } catch {
    return { ok: false, message: t('ceo.command.invalidArgs', locale) };
  }
  if (spec.confirmRequired && args.confirmed !== true) {
    return {
      ok: false,
      needsConfirmation: true,
      message: tf('ceo.command.needsConfirmation', locale, { name: spec.name }),
    };
  }
  const handler = handlers[name];
  if (!handler) {
    return { ok: false, message: tf('ceo.command.notWired', locale, { name }) };
  }
  try {
    return await handler(args);
  } catch (err) {
    return {
      ok: false,
      message: err instanceof Error ? err.message : t('ceo.command.failed', locale),
    };
  }
}
