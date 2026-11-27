import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.API_RETRY_MODULE;

if (!modulePath) {
  throw new Error('API_RETRY_MODULE must point at the compiled apiRetry.js file');
}

const {
  fetchAssistantCommandWithTransientRetry,
  fetchJsonWithOneNetworkRetry,
} = await import(modulePath);

function response(status, body = '{}') {
  return new Response(body, {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('API retry helpers', () => {
  it('retries assistant transient statuses with the exact same request body', async () => {
    const calls = [];
    const init = {
      method: 'POST',
      headers: { 'X-Aegis-CSRF': 'csrf-token' },
      body: JSON.stringify({ idempotency_key: 'stable-key', input: '通勤' }),
    };
    const sleeps = [];
    const result = await fetchAssistantCommandWithTransientRetry(
      '/webapp/vnext/command',
      init,
      async (url, requestInit) => {
        calls.push({ url, body: requestInit.body, csrf: requestInit.headers['X-Aegis-CSRF'] });
        return calls.length < 3 ? response(504) : response(200, '{"ok":true}');
      },
      async (ms) => {
        sleeps.push(ms);
      },
    );

    assert.equal(result.status, 200);
    assert.deepEqual(sleeps, [500, 1200]);
    assert.equal(calls.length, 3);
    assert.deepEqual(new Set(calls.map((call) => call.body)), new Set([init.body]));
    assert.deepEqual(new Set(calls.map((call) => call.csrf)), new Set(['csrf-token']));
  });

  it('retries one likely network failure for JSON mutations', async () => {
    const calls = [];
    const init = {
      method: 'POST',
      body: JSON.stringify({ idempotency_key: 'same-action-key' }),
    };
    const sleeps = [];
    const result = await fetchJsonWithOneNetworkRetry(
      '/webapp/vnext/work/item/actions',
      init,
      async (url, requestInit) => {
        calls.push({ url, body: requestInit.body });
        if (calls.length === 1) throw new TypeError('Load failed');
        return response(200, '{"ok":true}');
      },
      async (ms) => {
        sleeps.push(ms);
      },
    );

    assert.equal(result.status, 200);
    assert.deepEqual(sleeps, [600]);
    assert.equal(calls.length, 2);
    assert.deepEqual(calls.map((call) => call.body), [init.body, init.body]);
  });
});
