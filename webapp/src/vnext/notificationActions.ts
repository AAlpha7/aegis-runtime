import { t } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';

export type DeliveryNoticeActionTone = 'normal' | 'positive' | 'destructive';

export type DeliveryNoticeAction = {
  command: string;
  label: string;
  detail: string;
  tone: DeliveryNoticeActionTone;
  requiresConfirmation: boolean;
  directTaskAction?: {
    taskId: string;
    action: 'complete' | 'snooze' | 'delete';
    preset?: '10m' | '1h' | 'tomorrow';
  };
};

export type ParsedDeliveryNotice = {
  body: string;
  actions: DeliveryNoticeAction[];
};

export function parseDeliveryNotice(text: string, locale: Locale = 'en'): ParsedDeliveryNotice {
  const lines = text.replace(/\r\n?/g, '\n').split('\n');
  const actions: DeliveryNoticeAction[] = [];
  const bodyLines: string[] = [];

  for (const line of lines) {
    if (/^\s*(?:Web actions?|网页操作)\s*[:：]/i.test(line)) {
      for (const match of line.matchAll(/`([^`]{2,160})`/g)) {
        const action = deliveryNoticeActionFromCommand(match[1].trim(), locale);
        if (action && !actions.some((candidate) => candidate.command === action.command)) {
          actions.push(action);
        }
      }
      if (actions.length > 0) continue;
    }
    bodyLines.push(line);
  }

  return {
    body: bodyLines.join('\n').trim(),
    actions,
  };
}

export function deliveryNoticeActionFromCommand(command: string, locale: Locale = 'en'): DeliveryNoticeAction | null {
  const normalized = command.trim().replace(/\s+/g, ' ');
  if (!isDeliveryNoticeActionCommand(normalized)) return null;
  const [rawVerb = '', target = '', id = ''] = normalized.split(' ');
  const verb = rawVerb.toLowerCase().startsWith('trigger') ? 'trigger' : rawVerb;
  const lowerVerb = verb.toLowerCase();
  const directTaskAction = reminderTaskAction(lowerVerb, target, id);
  return {
    command: normalized,
    label: actionLabel(lowerVerb, locale),
    detail: actionDetail(lowerVerb, target, id, locale),
    tone: actionTone(lowerVerb),
    requiresConfirmation: actionRequiresConfirmation(lowerVerb),
    ...(directTaskAction ? { directTaskAction } : {}),
  };
}

export function isDeliveryNoticeActionCommand(command: string): boolean {
  return /^(approve|reject|retry|delete|later|dismiss|never|cancel|useful|useless|done|complete|snooze|tomorrow)\b|^trigger(?:\b|_)/i
    .test(command.trim());
}

function actionLabel(verb: string, locale: Locale): string {
  const labelKeys: Record<string, string> = {
    approve: 'work.delivery.approve',
    reject: 'work.delivery.reject',
    retry: 'work.delivery.retry',
    delete: 'work.delivery.delete',
    later: 'work.delivery.later',
    dismiss: 'work.delivery.dismiss',
    never: 'work.delivery.never',
    cancel: 'work.delivery.cancel',
    useful: 'work.delivery.useful',
    useless: 'work.delivery.useless',
    done: 'work.delivery.done',
    complete: 'work.delivery.done',
    snooze: 'work.delivery.snooze',
    tomorrow: 'work.delivery.tomorrow',
    trigger: 'work.delivery.run',
  };
  const key = labelKeys[verb];
  return key ? t(key, locale) : titleCase(verb);
}

function actionDetail(verb: string, target: string, id: string, locale: Locale): string {
  if (verb === 'trigger') return t('work.delivery.rebuildMemoryIndex', locale);
  const targetLabel = targetKindLabel(target, locale);
  if (!targetLabel) return t('work.delivery.aegisAction', locale);
  const lower = target.toLowerCase();
  const isReminder = lower === 'reminder' || lower === 'task';
  if (isReminder && verb === 'snooze') {
    const suffix = t('work.delivery.detail.hour1', locale);
    return id ? `${targetLabel} #${id} · ${suffix}` : `${targetLabel} · ${suffix}`;
  }
  if (isReminder && verb === 'tomorrow') {
    const suffix = t('work.delivery.detail.tomorrow', locale);
    return id ? `${targetLabel} #${id} · ${suffix}` : `${targetLabel} · ${suffix}`;
  }
  return id ? `${targetLabel} #${id}` : targetLabel;
}

// 'reminder'/'task' localize to 提醒; Initiative/Research are product names
// kept verbatim in both locales (historical notices still reference them).
function targetKindLabel(target: string, locale: Locale): string {
  const lower = target.toLowerCase();
  if (lower === 'reminder' || lower === 'task') return t('work.delivery.target.reminder', locale);
  const labels: Record<string, string> = {
    sysop: 'SysOp',
    initiative: 'Initiative',
    research: 'Research',
  };
  return labels[lower] ?? titleCase(target);
}

function actionTone(verb: string): DeliveryNoticeActionTone {
  if (verb === 'approve' || verb === 'retry' || verb === 'useful' || verb === 'trigger' || verb === 'done' || verb === 'complete') return 'positive';
  if (verb === 'reject' || verb === 'delete' || verb === 'never' || verb === 'cancel' || verb === 'useless') return 'destructive';
  return 'normal';
}

function actionRequiresConfirmation(verb: string): boolean {
  return ['approve', 'reject', 'delete', 'never', 'cancel', 'trigger'].includes(verb);
}

function reminderTaskAction(
  verb: string,
  target: string,
  id: string,
): DeliveryNoticeAction['directTaskAction'] | null {
  const targetKind = target.toLowerCase();
  if (targetKind !== 'reminder' && targetKind !== 'task') return null;
  if (!/^\d+$/.test(id)) return null;
  if (verb === 'done' || verb === 'complete') {
    return { taskId: id, action: 'complete' };
  }
  if (verb === 'snooze') {
    return { taskId: id, action: 'snooze', preset: '1h' };
  }
  if (verb === 'tomorrow') {
    return { taskId: id, action: 'snooze', preset: 'tomorrow' };
  }
  if (verb === 'delete') {
    return { taskId: id, action: 'delete' };
  }
  return null;
}

function titleCase(value: string): string {
  if (!value) return '';
  return `${value.slice(0, 1).toUpperCase()}${value.slice(1)}`;
}
