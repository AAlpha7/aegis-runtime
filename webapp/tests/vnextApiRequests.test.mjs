import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

const apiSource = readFileSync(new URL('../src/api.ts', import.meta.url), 'utf8');
const apiRetrySource = readFileSync(new URL('../src/apiRetry.ts', import.meta.url), 'utf8');

describe('vNext API request guards', () => {
  it('sends idempotency keys for command and item action mutations', () => {
    assert.match(apiSource, /function idempotencyKey\(prefix: string\): string/);
    assert.match(apiSource, /globalThis\.crypto\?\.randomUUID\?\.\(\)/);
    assert.match(apiSource, /commandIdempotencyKey\?: string/);
    assert.match(apiSource, /conversationContext: VNextConversationContextTurn\[\] = \[\]/);
    assert.match(apiSource, /kind\?: 'recent' \| 'quoted_reply';/);
    assert.match(apiSource, /source_id\?: string;/);
    assert.match(apiSource, /idempotency_key: commandIdempotencyKey \|\| idempotencyKey\('vnext-command'\)/);
    assert.match(apiSource, /\.\.\.\(conversationContext\.length > 0 \? \{ conversation_context: conversationContext \} : \{\}\)/);
    assert.match(apiSource, /idempotency_key: idempotencyKey\(`vnext-action:\$\{itemId\}:\$\{actionId\}`\)/);
    assert.match(apiSource, /idempotency_key: idempotencyKey\('vnext-cost-saving-pause'\)/);
    assert.match(apiSource, /idempotency_key: idempotencyKey\('vnext-cost-saving-resume'\)/);
  });

  it('uploads multimodal attachments as multipart and threads them into the command body (Slice 5)', () => {
    // The upload helper posts multipart/form-data to the dedicated endpoint and
    // forwards auth + CSRF (but NOT a JSON Content-Type — the browser sets the
    // multipart boundary).
    assert.match(apiSource, /export async function uploadVNextAttachment\(/);
    assert.match(apiSource, /const form = new FormData\(\);/);
    assert.match(apiSource, /form\.append\('file', file, file\.name\);/);
    assert.match(apiSource, /\$\{API_BASE\}\/webapp\/vnext\/upload/);
    assert.match(apiSource, /headers\.Authorization = `TelegramWebApp \$\{initData\}`;/);
    assert.match(apiSource, /headers\['X-Aegis-CSRF'\] = csrfToken;/);
    // The command body includes attachments only when present (no body change for
    // text-only clients).
    assert.match(apiSource, /attachments: VNextCommandAttachment\[\] = \[\]/);
    assert.match(apiSource, /\.\.\.\(attachments\.length > 0 \? \{ attachments \} : \{\}\)/);
    // The wire type is a tagged union mirroring the backend enum.
    assert.match(apiSource, /\{ type: 'image'; data_url: string \}/);
    assert.match(apiSource, /\{ type: 'voice'; transcript: string; duration_secs\?: number \}/);
    assert.match(apiSource, /\{ type: 'document'; data_url: string; filename: string; mime_type: string \}/);
  });

  it('retries transient network failures once without changing the idempotency body', () => {
    assert.match(apiSource, /import \{ fetchAssistantCommandWithTransientRetry, fetchJsonWithOneNetworkRetry \} from '\.\/apiRetry';/);
    assert.match(apiSource, /const body = JSON\.stringify\(\{[\s\S]*?idempotency_key: idempotencyKey\(`vnext-action:\$\{itemId\}:\$\{actionId\}`\),[\s\S]*?\}\);[\s\S]*?fetchJsonWithOneNetworkRetry\(`\$\{API_BASE\}\/webapp\/vnext\/work/);
    assert.match(apiSource, /pauseVNextCostSaving[\s\S]*?fetchJsonWithOneNetworkRetry\(`\$\{API_BASE\}\/webapp\/vnext\/settings\/cost-saving\/pause/);
    assert.match(apiSource, /resumeVNextCostSaving[\s\S]*?fetchJsonWithOneNetworkRetry\(`\$\{API_BASE\}\/webapp\/vnext\/settings\/cost-saving\/resume/);
  });

  it('loads server-backed chat history as an authenticated read projection', () => {
    assert.match(apiSource, /export interface VNextChatHistoryProjection/);
    assert.match(apiSource, /export async function fetchVNextChatHistory\(\s*initData: string,\s*options: \{ before\?: number; turn\?: string \} = \{\},\s*\): Promise<VNextChatHistoryProjection>/);
    // Older pages: ?before=<unix seconds of the oldest shown message>.
    assert.match(apiSource, /fetch\(`\$\{API_BASE\}\/webapp\/vnext\/chat-history\$\{query\}`, \{\s*headers: authHeaders\(initData\),\s*credentials: 'include',\s*\}\)/);
    assert.match(apiSource, /has_more\?: boolean/);
    assert.match(apiSource, /readVNextChatHistoryProjection\(json\)/);
  });

  it('syncs avatar profiles through authenticated server endpoints', () => {
    assert.match(apiSource, /export interface VNextAvatarProfileProjection/);
    assert.match(apiSource, /export async function fetchVNextAvatarProfile\(initData: string\): Promise<VNextAvatarProfileProjection>/);
    assert.match(apiSource, /fetch\(`\$\{API_BASE\}\/webapp\/vnext\/avatar-profile`, \{\s*headers: authHeaders\(initData\),\s*credentials: 'include',\s*\}\)/);
    assert.match(apiSource, /export async function updateVNextAvatarProfile\(/);
    assert.match(apiSource, /method: 'PUT'/);
    assert.match(apiSource, /headers: authHeaders\(initData, csrfToken\)/);
    assert.match(apiSource, /readVNextAvatarProfileProjection\(json\)/);
  });

  it('gives side-effect-free vNext assistant commands stronger transient retry coverage', () => {
    assert.match(apiSource, /mode: 'text' \| 'voice' = 'text'/);
    assert.match(apiRetrySource, /function isRetryableAssistantStatus\(status: number\): boolean/);
    assert.match(apiRetrySource, /status === 408 \|\| status === 429 \|\| status === 503 \|\| status === 504/);
    assert.match(apiRetrySource, /export async function fetchAssistantCommandWithTransientRetry/);
    assert.match(apiRetrySource, /const delaysMs = \[500, 1200, 2400\];/);
    assert.match(apiRetrySource, /if \(!isRetryableAssistantStatus\(response\.status\) \|\| attempt === delaysMs\.length\)/);
    assert.match(apiSource, /const body = JSON\.stringify\(\{[\s\S]*?idempotency_key: commandIdempotencyKey \|\| idempotencyKey\('vnext-command'\),[\s\S]*?\}\);[\s\S]*?fetchAssistantCommandWithTransientRetry\(`\$\{API_BASE\}\/webapp\/vnext\/command/);
  });

  it('keeps every vNext POST on cookie plus CSRF-authenticated requests', () => {
    for (const functionName of [
      'pauseVNextCostSaving',
      'resumeVNextCostSaving',
      'subscribeVNextWebPush',
      'unsubscribeVNextWebPush',
      'testVNextWebPush',
      'submitVNextCommand',
      'submitVNextWorkAction',
      'submitVNextDebugSnapshot',
      'fetchOwnerWorkContext',
      'createVNextDebugLease',
      'createVNextDebugJob',
      'markVNextPresence',
    ]) {
      const start = apiSource.indexOf(`export async function ${functionName}`);
      assert.ok(start >= 0, `missing ${functionName}`);
      const nextExport = apiSource.indexOf('\nexport async function ', start + 1);
      const body = apiSource.slice(start, nextExport > start ? nextExport : undefined);
      assert.match(body, /method: 'POST'/, `${functionName} must remain a POST contract`);
      assert.match(body, /headers: authHeaders\(initData, csrfToken\)/, `${functionName} must send CSRF-aware auth headers`);
      assert.match(body, /credentials: 'include'/, `${functionName} must include the signed-in owner cookie`);
    }
    const avatarStart = apiSource.indexOf('export async function updateVNextAvatarProfile');
    assert.ok(avatarStart >= 0, 'missing updateVNextAvatarProfile');
    const avatarNextExport = apiSource.indexOf('\nexport async function ', avatarStart + 1);
    const avatarBody = apiSource.slice(avatarStart, avatarNextExport > avatarStart ? avatarNextExport : undefined);
    assert.match(avatarBody, /method: 'PUT'/);
    assert.match(avatarBody, /headers: authHeaders\(initData, csrfToken\)/);
    assert.match(avatarBody, /credentials: 'include'/);
  });

  it('registers Web Push devices through authenticated owner endpoints', () => {
    assert.match(apiSource, /export interface VNextWebPushStatus/);
    assert.match(apiSource, /export interface VNextWebPushDeliverySummary/);
    assert.match(apiSource, /last_delivery\?: VNextWebPushDeliverySummary \| null/);
    assert.match(apiSource, /export async function fetchVNextWebPushStatus\(initData: string\): Promise<VNextWebPushStatus>/);
    assert.match(apiSource, /\/webapp\/vnext\/settings\/web-push\/status/);
    assert.match(apiSource, /export async function subscribeVNextWebPush/);
    assert.match(apiSource, /\/webapp\/vnext\/settings\/web-push\/subscribe/);
    assert.match(apiSource, /subscription\.toJSON\(\)/);
    assert.match(apiSource, /user_agent: navigator\.userAgent/);
    assert.match(apiSource, /export async function unsubscribeVNextWebPush/);
    assert.match(apiSource, /\/webapp\/vnext\/settings\/web-push\/unsubscribe/);
    assert.match(apiSource, /export async function testVNextWebPush/);
    assert.match(apiSource, /\/webapp\/vnext\/settings\/web-push\/test/);
    assert.match(apiSource, /export interface VNextWebPushTestResponse/);
  });

  it('parses Web Push mutation responses before validating their projection', () => {
    for (const functionName of [
      'subscribeVNextWebPush',
      'unsubscribeVNextWebPush',
      'testVNextWebPush',
    ]) {
      const start = apiSource.indexOf(`export async function ${functionName}`);
      assert.ok(start >= 0, `missing ${functionName}`);
      const nextExport = apiSource.indexOf('\nexport async function ', start + 1);
      const body = apiSource.slice(start, nextExport > start ? nextExport : undefined);
      assert.match(body, /const json = await expectJson\(res, /, `${functionName} must parse the Response body`);
      assert.doesNotMatch(body, /readVNext(WebPushStatus|WebPushTestResponse)\(res\)/, `${functionName} must not validate the raw Response object`);
    }
  });

  it('supports owner-scoped debug leases with selectable duration and revoke', () => {
    assert.match(apiSource, /export interface VNextDebugLeasesResponse/);
    assert.match(apiSource, /export async function createVNextDebugLease/);
    assert.match(apiSource, /duration_minutes: durationMinutes/);
    assert.match(apiSource, /export async function fetchVNextDebugLeases\(initData: string\): Promise<VNextDebugLeasesResponse>/);
    assert.match(apiSource, /\/webapp\/vnext\/debug-leases/);
    assert.match(apiSource, /export async function revokeVNextDebugLease/);
    assert.match(apiSource, /method: 'DELETE'/);
    assert.match(apiSource, /headers: authHeaders\(initData, csrfToken\)/);
    assert.match(apiSource, /encodeURIComponent\(leaseId\)/);
  });

  it('does not expose the retired owner-work assistant response endpoint in the client', () => {
    assert.doesNotMatch(apiSource, /requestOwnerWorkResponse/);
    assert.doesNotMatch(apiSource, /\/webapp\/owner-work\/respond/);
  });

  it('uses Telegram initData for session checks when the WebApp is opened from Telegram', () => {
    assert.match(apiSource, /export async function fetchSession\(initData = ''\): Promise<SessionInfo>/);
    assert.match(apiSource, /fetch\(`\$\{API_BASE\}\/api\/session`, \{\s*headers: authHeaders\(initData\),\s*credentials: 'include',\s*\}\)/);
    assert.match(readFileSync(new URL('../src/vnext/App.tsx', import.meta.url), 'utf8'), /fetchSession\(initData\)/);
  });

  it('renders locked detail payloads instead of treating normal gates as open failures', () => {
    assert.match(apiSource, /async function expectDetailJson\(res: Response, fallback: string\): Promise<unknown>/);
    assert.match(apiSource, /async function expectJson[\s\S]*?if \(isSensitiveAccessProblem\(parsed\.json\)\) \{[\s\S]*?throw new ApiResponseError/);

    for (const functionName of [
      'fetchConsoleMemoryItem',
      'fetchConsoleProfileSlot',
      'fetchConsoleTaskTimeline',
      'fetchConsoleLedgerEntry',
    ]) {
      const start = apiSource.indexOf(`export async function ${functionName}`);
      assert.ok(start >= 0, `missing ${functionName}`);
      const nextExport = apiSource.indexOf('\nexport async function ', start + 1);
      const body = apiSource.slice(start, nextExport > start ? nextExport : undefined);
      assert.match(body, /return expectDetailJson\(res, '[^']+ failed'\);/, `${functionName} must return mfa_required/consent_required detail payloads`);
      assert.doesNotMatch(body, /return expectJson\(res, '[^']+ failed'\);/, `${functionName} must not throw on normal locked detail payloads`);
    }
  });
});
