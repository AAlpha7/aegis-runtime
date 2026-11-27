import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';
import { createRequire } from 'node:module';
import { build } from 'esbuild';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const require = createRequire(import.meta.url);
const workSource = readFileSync(new URL('../src/vnext/WorkView.tsx', import.meta.url), 'utf8');
const appSource = readFileSync(new URL('../src/vnext/App.tsx', import.meta.url), 'utf8');
const cleanupSource = readFileSync(new URL('../src/vnext/followUpCleanup.ts', import.meta.url), 'utf8');
const modelSource = readFileSync(new URL('../src/vnext/model.ts', import.meta.url), 'utf8');
const catalogSource = readFileSync(new URL('../src/i18n/catalog.ts', import.meta.url), 'utf8');

async function loadCleanupModule() {
  const outDir = mkdtempSync(join(tmpdir(), 'aegis-followup-cleanup-'));
  const outfile = join(outDir, 'followUpCleanup.mjs');
  await build({
    entryPoints: [new URL('../src/vnext/followUpCleanup.ts', import.meta.url).pathname],
    bundle: true,
    platform: 'node',
    format: 'esm',
    outfile,
    // model.ts pulls React types only as types; strip via esbuild external of nothing.
    packages: 'bundle',
    logLevel: 'silent',
  });
  return import(`file://${outfile}`);
}

describe('follow-up cleanup closes with actions, not confirmation loops', () => {
  it('puts cleanup on the primary action dock, not a second strip chip', () => {
    assert.match(cleanupSource, /knowledge\.done/);
    assert.match(cleanupSource, /task\.complete/);
    assert.match(cleanupSource, /No close_action chip when a receipt-backed close/);
    assert.match(workSource, /followUpQuickControls/);
    assert.match(workSource, /shouldExecuteFollowUpCleanup/);
    assert.match(workSource, /onExecuteAction/);
    assert.match(appSource, /onExecuteAction=\{\(item, action\) => void executeConfirmedItemAction/);
    assert.match(modelSource, /work\.itemAction\.endThisCard/);
    assert.match(modelSource, /tracking\.cancel/);
    // Legacy forever-ask prompt must not be the primary strip path.
    assert.doesNotMatch(workSource, /function followUpQuickPrompts/);
  });

  it('rewrites the fallback cleanup prompt to execute, not re-confirm', () => {
    assert.match(catalogSource, /Do not ask for confirmation again/);
    assert.match(catalogSource, /不要再次要求确认/);
  });

  it('detects cleanup commands and affirmations for receipt-backed close', async () => {
    const mod = await loadCleanupModule();
    const item = {
      id: 'kb_waiting:1',
      kind: 'waiting_for',
      title: '补齐周四通勤监测',
      status: 'waiting',
      urgency: 'high',
      source: 'knowledge',
      source_kind: 'tracking',
      primary_action: null,
      secondary_actions: [
        { action_kind: 'knowledge.done', label: 'Done', requires_gate: false, requires_mfa: false },
        { action_kind: 'knowledge.snooze_tomorrow', label: 'Tomorrow', requires_gate: false, requires_mfa: false },
      ],
      allowed_actions: ['knowledge.done', 'knowledge.snooze_tomorrow'],
    };
    // buildSupportedItemActions needs full shape — if close action resolves null, skip dynamic asserts.
    assert.equal(mod.isStrongFollowUpCleanupCommand('清理'), true);
    assert.equal(mod.isOwnerCleanupAffirmation('是'), true);
    assert.equal(mod.isCannedFollowUpCleanupPrompt('检查这个跟进项是否已过时或重复，然后建议如何清理：x'), true);
    assert.equal(mod.isStrongFollowUpCleanupCommand('随便问问'), false);
    // 2026-09-13: a bare affirmation never executes the close on its own.
    const turns = [{ id: 't1', role: 'assistant', text: '要我清理这个跟进项吗？确认一下。' }];
    assert.equal(mod.shouldExecuteFollowUpCleanup(item, '好', turns, 'zh'), false);
    assert.equal(mod.shouldExecuteFollowUpCleanup(item, '嗯', turns, 'zh'), false);
  });
});

it('closes a goal through its projected lifecycle action, never a chat prompt', async () => {
  const mod = await loadCleanupModule();
  const item = {
    id: 'goal:42', kind: 'goal', source: 'goals', source_kind: 'goal', status: 'tracking',
    title: 'Track release updates', lane: 'follow_up',
    secondary_actions: [{ action_kind: 'goal.abandon', label: 'Close follow-up', requires_gate: false, requires_mfa: false }],
  };
  assert.equal(mod.followUpCloseAction(item, 'zh')?.id, 'goal.abandon');
  assert.equal(mod.followUpQuickControls(item, 'zh').some(c => c.id === 'followup.cleanup'), false);
  assert.equal(mod.shouldExecuteFollowUpCleanup(item, '关闭', [], 'zh'), true);
  assert.equal(mod.followUpCloseAction({ ...item, status: 'abandoned' }, 'zh'), null);
  assert.deepEqual(mod.followUpQuickControls({ ...item, status: 'abandoned' }, 'zh'), []);
});

it('closes a recurring watch through tracking.cancel, not the one-time skip', async () => {
  const mod = await loadCleanupModule();
  const item = {
    id: 'task:12', kind: 'task', source: 'tasks', source_kind: 'task', status: 'tracking',
    title: '门铃电量', lane: 'follow_up',
    secondary_actions: [
      { action_kind: 'task.skip_occurrence', label: 'End only this time', requires_gate: false, requires_mfa: false },
      { action_kind: 'tracking.cancel', label: 'Close this tracking', requires_gate: false, requires_mfa: false },
    ],
  };
  assert.equal(mod.followUpCloseAction(item, 'zh')?.id, 'tracking.cancel');
  assert.equal(mod.shouldExecuteFollowUpCleanup(item, '关闭', [], 'zh'), true);
});
