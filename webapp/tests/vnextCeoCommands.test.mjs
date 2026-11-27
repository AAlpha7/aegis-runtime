import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_CEO_CMDS_MODULE;
assert.ok(modulePath, 'VNEXT_CEO_CMDS_MODULE is required');

const {
  dispatchCeoCommand,
  realtimeCeoTools,
  CEO_COMMANDS,
} = await import(modulePath);

describe('ceo command registry', () => {
  it('exports tools for realtime session.update', () => {
    const tools = realtimeCeoTools();
    assert.ok(tools.length >= 5);
    assert.equal(tools[0].type, 'function');
    assert.ok(CEO_COMMANDS.some((c) => c.name === 'ask_chief'));
    assert.ok(CEO_COMMANDS.some((c) => c.name === 'open_surface'));
    assert.ok(CEO_COMMANDS.some((c) => c.confirmRequired));
    assert.ok(!CEO_COMMANDS.some((c) => c.name === 'open_mail'));
  });

  it('blocks confirmRequired tools until confirmed=true', async () => {
    let called = false;
    const result = await dispatchCeoCommand(
      {
        complete_task: async () => {
          called = true;
          return { ok: true, message: 'done' };
        },
      },
      'complete_task',
      '{}',
    );
    assert.equal(result.needsConfirmation, true);
    assert.equal(called, false);

    const ok = await dispatchCeoCommand(
      {
        complete_task: async () => {
          called = true;
          return { ok: true, message: 'done' };
        },
      },
      'complete_task',
      JSON.stringify({ confirmed: true }),
    );
    assert.equal(ok.ok, true);
    assert.equal(called, true);
  });

  it('approve_pending runs without a second confirmed hop (voice 批准)', async () => {
    let called = false;
    const result = await dispatchCeoCommand(
      {
        approve_pending: async () => {
          called = true;
          return { ok: true, message: 'approved gate' };
        },
      },
      'approve_pending',
      '{}',
    );
    assert.equal(result.ok, true);
    assert.equal(called, true);
    assert.ok(CEO_COMMANDS.some((c) => c.name === 'complete_task' && c.confirmRequired));
  });

  it('localizes dispatcher-authored messages from the explicit locale option', async () => {
    const zhUnknown = await dispatchCeoCommand({}, 'nope', '{}', { locale: 'zh' });
    assert.equal(zhUnknown.ok, false);
    assert.equal(zhUnknown.message, '未知指令：nope');
    const enUnknown = await dispatchCeoCommand({}, 'nope', '{}', { locale: 'en' });
    assert.equal(enUnknown.message, 'Unknown command: nope');

    const zhArgs = await dispatchCeoCommand({}, 'ask_chief', '{not json', { locale: 'zh' });
    assert.equal(zhArgs.message, '指令参数无效');
    const enArgs = await dispatchCeoCommand({}, 'ask_chief', '{not json', { locale: 'en' });
    assert.equal(enArgs.message, 'Invalid command arguments');

    const zhConfirm = await dispatchCeoCommand({ complete_task: async () => ({ ok: true, message: 'done' }) }, 'complete_task', '{}', { locale: 'zh' });
    assert.equal(zhConfirm.needsConfirmation, true);
    assert.equal(zhConfirm.message, '需要你确认后才会执行：complete_task。请口头确认或在面板点批准。');
    const enConfirm = await dispatchCeoCommand({ complete_task: async () => ({ ok: true, message: 'done' }) }, 'complete_task', '{}', { locale: 'en' });
    assert.equal(enConfirm.needsConfirmation, true);
    assert.equal(enConfirm.message, 'This runs only after you confirm: complete_task. Say so out loud or tap Approve in the panel.');

    const enNotWired = await dispatchCeoCommand({}, 'ask_chief', '{}', { locale: 'en' });
    assert.equal(enNotWired.message, 'Command not wired: ask_chief');
  });

  it('falls back to the stamped <html lang> when no locale option is given', async () => {
    const hadDocument = Object.prototype.hasOwnProperty.call(globalThis, 'document');
    const previousDocument = globalThis.document;
    try {
      globalThis.document = { documentElement: { lang: 'zh-CN' } };
      assert.equal((await dispatchCeoCommand({}, 'nope', '{}')).message, '未知指令：nope');
      globalThis.document = { documentElement: { lang: 'en-US' } };
      assert.equal((await dispatchCeoCommand({}, 'nope', '{}')).message, 'Unknown command: nope');
    } finally {
      if (hadDocument) globalThis.document = previousDocument;
      else delete globalThis.document;
    }
  });
});
