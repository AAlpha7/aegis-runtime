import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_CONTRACTS_MODULE
  ?? 'file:///tmp/aegis-runtime-webapp-vnext-contracts-test/contracts.js';
const {
  VNEXT_API_PROJECTIONS,
  VNEXT_PHASE0_SURFACES,
  VNEXT_WORK_LANES,
} = await import(modulePath);
const contractsSource = await import('node:fs').then(({ readFileSync }) =>
  readFileSync(new URL('../src/vnext/contracts.ts', import.meta.url), 'utf8')
);

describe('vNext product contracts', () => {
  it('names every Phase 0 owner surface as a first-class contract target', () => {
    assert.deepEqual(VNEXT_PHASE0_SURFACES, ['home', 'work', 'item', 'command', 'status', 'settings']);
  });

  it('keeps the unified work lanes as the owner-facing queue contract', () => {
    assert.deepEqual(VNEXT_WORK_LANES, ['decide', 'now', 'follow_up', 'backlog', 'done']);
  });

  it('documents the planned server-owned vNext API projection boundary', () => {
    assert.deepEqual(VNEXT_API_PROJECTIONS, [
      'GET /webapp/vnext/home',
      'GET /webapp/vnext/work',
      'GET /webapp/vnext/work/:id',
      'POST /webapp/vnext/command',
      'POST /webapp/vnext/work/:id/actions',
      'GET /webapp/vnext/status',
      'GET /webapp/vnext/settings',
      'POST /webapp/vnext/settings/cost-saving/pause',
      'POST /webapp/vnext/settings/cost-saving/resume',
      'GET /webapp/vnext/settings/web-push/status',
      'POST /webapp/vnext/settings/web-push/subscribe',
      'POST /webapp/vnext/settings/web-push/unsubscribe',
      'POST /webapp/vnext/settings/web-push/test',
      'GET /webapp/vnext/debug-leases',
      'POST /webapp/vnext/debug-leases',
      'DELETE /webapp/vnext/debug-leases/:id',
      'GET /webapp/vnext/debug-jobs',
      'POST /webapp/vnext/debug-jobs',
      'GET /webapp/vnext/debug-jobs/:id',
    ]);
  });

  it('keeps command and action request contracts aligned with server-owned validation fields', () => {
    assert.match(contractsSource, /export type SecretaryCommandIntentKind = 'chat' \| 'explain' \| 'draft';/);
    assert.match(contractsSource, /intent_kind\?: SecretaryCommandIntentKind;/);
    assert.match(contractsSource, /idempotency_key\?: string \| null;/);
    assert.match(contractsSource, /export interface SecretaryWorkActionRequest/);
    assert.match(contractsSource, /action_id: string;/);
  });

  it('keeps status summaries actionable instead of status-only', () => {
    assert.match(contractsSource, /export interface SecretaryStatusSummary/);
    assert.match(contractsSource, /owner_message: string;/);
    assert.match(contractsSource, /owner_next_step\?: string \| null;/);
  });
});
