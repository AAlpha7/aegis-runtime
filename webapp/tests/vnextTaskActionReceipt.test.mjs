import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_TASK_ACTION_RECEIPT_MODULE
  ?? 'file:///tmp/aegis-runtime-vnext-task-action-receipt-test/taskActionReceipt.mjs';
const { ownerTaskActionReceipt, ownerFacingWorkActionMessage } = await import(modulePath);

const ledger = {
  action: 'complete',
  resource_id: '2138',
  summary: 'Task #2138 marked complete',
  recurring: true,
};

describe('owner task action receipts follow the interface language', () => {
  it('does not show the English ledger sentence in Chinese', () => {
    const message = ownerTaskActionReceipt({
      action: ledger.action,
      recurring: ledger.recurring,
      dueAt: null,
    }, 'zh');
    assert.equal(message.includes('Task #'), false);
    assert.equal(message.includes('marked complete'), false);
    assert.equal(message.includes('2138'), false);
    assert.match(message, /还会继续/);
  });

  it('localizes snooze and delete from the action, not the summary', () => {
    const snoozed = ownerTaskActionReceipt({
      action: 'snooze',
      dueAt: '2026-09-25T05:00:00Z',
    }, 'zh', 'UTC');
    assert.equal(snoozed.includes('snoozed'), false);
    assert.match(snoozed, /已改到/);
    assert.equal(/\d{4}-\d{2}-\d{2}T/.test(snoozed), false);

    const archived = ownerTaskActionReceipt({ action: 'delete' }, 'zh');
    assert.equal(archived.includes('archived'), false);
    assert.match(archived, /已删除/);
  });

  it('prefers structured raw_result over an English owner_message', () => {
    const message = ownerFacingWorkActionMessage('task.complete', {
      owner_message: 'Task #2138 marked complete',
      raw_result: ledger,
    }, 'zh');
    assert.equal(message.includes('marked complete'), false);
    assert.match(message, /还会继续/);
  });

  it('keeps an already localized server receipt for other actions', () => {
    const message = ownerFacingWorkActionMessage('task.skip_occurrence', {
      owner_message: '已跳过这一次。',
      raw_result: { action: 'task.skip_occurrence', summary: 'skipped' },
    }, 'zh');
    assert.equal(message, '已跳过这一次。');
  });
});
