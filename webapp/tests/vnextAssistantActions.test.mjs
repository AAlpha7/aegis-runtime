import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

const assistantActionsSource = readFileSync(new URL('../src/vnext/assistantActions.ts', import.meta.url), 'utf8');
const retryModulePath = process.env.VNEXT_ASSISTANT_RETRY_MODULE;
const assistantRetry = retryModulePath ? await import(retryModulePath) : null;
// Compiled by the `test:vnext-assistant-actions` npm script (tsc → /tmp). A
// missing module FAILS the test rather than skipping it.
const fallbackModulePath = process.env.VNEXT_ASSISTANT_FALLBACK_MODULE
  ?? 'file:///tmp/aegis-runtime-webapp-vnext-assistant-actions-test/assistantFallbackText.js';

describe('vNext assistant command fallback text (owner: voice note answered as "explain this")', () => {
  it('routes the empty-text case through the pure helper with the attachment flag', () => {
    assert.match(assistantActionsSource, /text \|\| assistantCommandFallbackText\(intent, attachments\.length > 0\)/);
    assert.doesNotMatch(assistantActionsSource, /请简明说明这件事是什么/);
  });

  it('never substitutes the item-page prompt for a chat turn or an attachment-only turn', async () => {
    const { assistantCommandFallbackText, ASSISTANT_EXPLAIN_FALLBACK_TEXT, ASSISTANT_DRAFT_FALLBACK_TEXT } = await import(fallbackModulePath);
    // A voice-only / image-only chat turn: the server derives the text from the attachment.
    assert.equal(assistantCommandFallbackText('chat', true), '');
    assert.equal(assistantCommandFallbackText('chat', false), '');
    assert.equal(assistantCommandFallbackText('explain', true), '');
    assert.equal(assistantCommandFallbackText('draft', true), '');
    // The item-page quick actions keep their canned question.
    assert.equal(assistantCommandFallbackText('explain', false), ASSISTANT_EXPLAIN_FALLBACK_TEXT);
    assert.equal(assistantCommandFallbackText('draft', false), ASSISTANT_DRAFT_FALLBACK_TEXT);
    assert.match(ASSISTANT_EXPLAIN_FALLBACK_TEXT, /不要执行任何外部动作/);
  });
});

describe('vNext assistant action resilience guards', () => {
  it('keeps long-running chat checks idempotent and owner-facing', () => {
    assert.match(assistantActionsSource, /requestWithStillWorkingAutoCheck/);
    assert.match(assistantActionsSource, /ASSISTANT_STILL_WORKING_RETRY_DELAYS_MS/);
    assert.match(assistantActionsSource, /checksUsed < ASSISTANT_STILL_WORKING_RETRY_DELAYS_MS\.length/);
    assert.match(assistantActionsSource, /STILL_WORKING_STATUS_CODES.*new Set\(\[408, 502, 504, 524\]\)/);
    assert.match(assistantActionsSource, /isStillWorkingError\(error\)/);
    assert.match(assistantActionsSource, /fetchVNextCommandStatus\(initData, csrfToken/);
    assert.match(assistantActionsSource, /checkStatus,\n  \}\);/);
    assert.match(assistantActionsSource, /shouldRetry: shouldAutoCheckAssistantResult/);
    assert.doesNotMatch(assistantActionsSource, /Checking the saved result/);
  });

  it('rechecks 504 command results with the same idempotency key before surfacing failure', async (context) => {
    if (!assistantRetry) {
      context.skip('compiled assistantRetry module was not provided');
      return;
    }
    const calls = [];
    const sleeps = [];
    const progress = [];
    const args = {
      initData: '',
      csrfToken: 'csrf',
      focusId: 'assistant:general',
      intent: 'chat',
      text: '通勤',
      scope: 'general',
      idempotencyKey: 'stable-command-key',
    };
    const result = await assistantRetry.requestWithStillWorkingAutoCheck({
      args,
      onStillWorking: (message) => progress.push(message),
      sleep: async (ms) => {
        sleeps.push(ms);
      },
      shouldRetry: (error, idempotencyKey, checksUsed) => (
        error instanceof Error
        && error.message === 'Still working'
        && idempotencyKey === 'stable-command-key'
        && checksUsed < assistantRetry.ASSISTANT_STILL_WORKING_RETRY_DELAYS_MS.length
      ),
      request: async (request) => {
        calls.push({ ...request });
        if (calls.length <= 3) {
          throw new Error('Still working');
        }
        return {
          response: {
            schema_version: 1,
            response_kind: 'orchestrated_response',
            visible_answer: '从Example City家到Worktown公司，预计44分钟。',
            receipt_expectation: 'Handled through the main assistant runtime.',
            citations: [],
            warnings: [],
          },
          workItem: null,
        };
      },
    });

    assert.equal(result.response.visible_answer, '从Example City家到Worktown公司，预计44分钟。');
    assert.deepEqual(sleeps, [2400, 5000, 10000]);
    assert.deepEqual(progress, [
      'Still working. Checking whether the answer is ready...',
      'Still working. Checking whether the answer is ready...',
      'Still working. Checking whether the answer is ready...',
    ]);
    assert.equal(calls.length, 4);
    assert.deepEqual(new Set(calls.map((call) => call.idempotencyKey)), new Set(['stable-command-key']));
    assert.deepEqual(new Set(calls.map((call) => call.text)), new Set(['通勤']));
  });
});


describe('vNext assistant still-working status polling', () => {
  it('collects the answer from /command/status after a 504 without re-posting the command', async (context) => {
    if (!assistantRetry) {
      context.skip('compiled assistantRetry module was not provided');
      return;
    }
    const calls = [];
    const sleeps = [];
    const progress = [];
    const statuses = [{ kind: 'pending' }, { kind: 'pending' }, { kind: 'done', value: { answer: 'ready' } }];
    const result = await assistantRetry.requestWithStillWorkingAutoCheck({
      args: { idempotencyKey: 'k1', text: 'hello' },
      request: async (request) => {
        calls.push(request);
        const error = new Error('gateway timeout');
        error.statusCode = 504;
        throw error;
      },
      sleep: async (ms) => { sleeps.push(ms); },
      shouldRetry: (error, key) => Boolean(key) && error.statusCode === 504,
      onStillWorking: (message) => progress.push(message),
      checkStatus: async () => statuses.shift(),
      pollDelaysMs: [3000, 5000, 5000, 5000],
    });
    assert.deepEqual(result, { answer: 'ready' });
    assert.equal(calls.length, 1, 'the command body is posted exactly once');
    assert.deepEqual(sleeps, [3000, 5000, 5000]);
    assert.equal(progress.length, 3);
  });

  it('surfaces the original failure when the server no longer knows the request', async (context) => {
    if (!assistantRetry) {
      context.skip('compiled assistantRetry module was not provided');
      return;
    }
    const error = new Error('edge timeout');
    error.statusCode = 524;
    await assert.rejects(
      assistantRetry.requestWithStillWorkingAutoCheck({
        args: { idempotencyKey: 'k2' },
        request: async () => { throw error; },
        sleep: async () => {},
        shouldRetry: () => true,
        checkStatus: async () => ({ kind: 'gone' }),
        pollDelaysMs: [1000, 1000],
      }),
      (err) => err === error,
    );
  });

  it('gives up with the original failure after the poll schedule is exhausted', async (context) => {
    if (!assistantRetry) {
      context.skip('compiled assistantRetry module was not provided');
      return;
    }
    const error = new Error('still working');
    error.statusCode = 504;
    let polls = 0;
    await assert.rejects(
      assistantRetry.requestWithStillWorkingAutoCheck({
        args: { idempotencyKey: 'k3' },
        request: async () => { throw error; },
        sleep: async () => {},
        shouldRetry: () => true,
        checkStatus: async () => { polls += 1; return { kind: 'pending' }; },
        pollDelaysMs: [1, 1, 1],
      }),
      (err) => err === error,
    );
    assert.equal(polls, 3);
  });
});
