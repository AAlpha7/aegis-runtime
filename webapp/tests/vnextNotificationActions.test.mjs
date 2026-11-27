import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_NOTIFICATION_ACTIONS_MODULE
  ?? 'file:///tmp/aegis-runtime-webapp-vnext-notification-actions-test/notificationActions.js';
const {
  deliveryNoticeActionFromCommand,
  isDeliveryNoticeActionCommand,
  parseDeliveryNotice,
} = await import(modulePath);

describe('vNext notification actions', () => {
  it('extracts Web action commands into safe native action metadata', () => {
    const parsed = parseDeliveryNotice([
      'SysOp change #42 needs review.',
      '',
      'Web actions: `approve sysop 42` or `reject sysop 42`.',
    ].join('\n'));

    assert.equal(parsed.body, 'SysOp change #42 needs review.');
    assert.deepEqual(parsed.actions, [
      {
        command: 'approve sysop 42',
        label: 'Approve',
        detail: 'SysOp #42',
        tone: 'positive',
        requiresConfirmation: true,
      },
      {
        command: 'reject sysop 42',
        label: 'Reject',
        detail: 'SysOp #42',
        tone: 'destructive',
        requiresConfirmation: true,
      },
    ]);
  });

  it('does not treat incidental code spans as actions', () => {
    const parsed = parseDeliveryNotice(
      'Circuit breaker is open for `gmail_reader`.\n\nWeb actions: ask Aegis to fix skill `gmail_reader`.',
    );

    assert.equal(parsed.actions.length, 0);
    assert.match(parsed.body, /gmail_reader/);
  });

  it('classifies low-risk and high-risk commands differently', () => {
    assert.deepEqual(deliveryNoticeActionFromCommand('later initiative 7'), {
      command: 'later initiative 7',
      label: 'Later',
      detail: 'Initiative #7',
      tone: 'normal',
      requiresConfirmation: false,
    });
    assert.deepEqual(deliveryNoticeActionFromCommand('cancel research 9'), {
      command: 'cancel research 9',
      label: 'Cancel',
      detail: 'Research #9',
      tone: 'destructive',
      requiresConfirmation: true,
    });
  });

  it('recognizes the corpus rebuild action without exposing command syntax as the label', () => {
    const action = deliveryNoticeActionFromCommand('trigger_corpus_reembed');
    assert.equal(action.label, 'Run');
    assert.equal(action.detail, 'Rebuild memory index');
    assert.equal(action.requiresConfirmation, true);
    assert.equal(isDeliveryNoticeActionCommand('trigger_corpus_reembed'), true);
  });

  it('extracts 网页操作 the same way as Web actions', () => {
    const parsed = parseDeliveryNotice([
      '提醒 · 未完成',
      '',
      '网页操作：`done reminder 1894`、`snooze reminder 1894`、`tomorrow reminder 1894` 或 `delete reminder 1894`。',
    ].join('\n'), 'zh');

    assert.equal(parsed.body, '提醒 · 未完成');
    assert.deepEqual(parsed.actions.map((action) => action.label), ['完成', '稍后提醒', '明天', '删除']);
    assert.equal(parsed.actions[0].command, 'done reminder 1894');
  });

  it('maps reminder Web action commands to direct task actions', () => {
    assert.deepEqual(deliveryNoticeActionFromCommand('done reminder 1894'), {
      command: 'done reminder 1894',
      label: 'Done',
      detail: 'Reminder #1894',
      tone: 'positive',
      requiresConfirmation: false,
      directTaskAction: {
        taskId: '1894',
        action: 'complete',
      },
    });
    assert.deepEqual(deliveryNoticeActionFromCommand('snooze reminder 1894'), {
      command: 'snooze reminder 1894',
      label: 'Snooze',
      detail: 'Reminder #1894 · 1h',
      tone: 'normal',
      requiresConfirmation: false,
      directTaskAction: {
        taskId: '1894',
        action: 'snooze',
        preset: '1h',
      },
    });
    assert.deepEqual(deliveryNoticeActionFromCommand('tomorrow reminder 1894')?.directTaskAction, {
      taskId: '1894',
      action: 'snooze',
      preset: 'tomorrow',
    });
    assert.deepEqual(deliveryNoticeActionFromCommand('delete reminder 1894')?.directTaskAction, {
      taskId: '1894',
      action: 'delete',
    });
  });
});
