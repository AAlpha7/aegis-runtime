// Real HTTP/router/SQLCipher/WebAuthn/browser behavior, with a virtual CTAP2
// authenticator. No Tailscale, subscription, API key, or production data.
import { chromium } from 'playwright';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { randomBytes } from 'node:crypto';
import net from 'node:net';
import assert from 'node:assert/strict';
import { run as runJevScenario } from './ui-scenarios/settings-jev.mjs';
import { run as runChatNavigation } from './ui-scenarios/chat-sheet-flow.mjs';
import { run as runNavigationScroll } from './ui-scenarios/chat-sheet-scroll-lock.mjs';
import { checkProgressLanguage } from './check-progress-language.mjs';
import { checkModelRecommendation } from './check-model-recommendation.mjs';
import { checkSettingsPersistence } from './check-settings-persistence.mjs';

const root = resolve(import.meta.dirname, '../..');
const target = process.env.CARGO_TARGET_DIR || join(root, 'target');
const harness = join(target, 'debug/examples/remote_access_harness');
const helper = join(target, 'debug/aegis_runtime_access');
const dir = mkdtempSync(join(tmpdir(), 'aegis-access-test-'));
const db = join(dir, 'memory.db');
const env = { ...process.env, AEGIS_RUNTIME_DB_KEY_HEX: randomBytes(32).toString('hex'), AEGIS_WEBAPP_DIR: join(root, 'webapp/dist') };
const reserve = net.createServer();
await new Promise(resolve => reserve.listen(0, '127.0.0.1', resolve));
const port = reserve.address().port;
await new Promise(resolve => reserve.close(resolve));
const origin = `http://localhost:${port}`;
let child;
let browser;
let activePage;
let passed = 0;
const check = message => { console.log(`PASS ${++passed}: ${message}`); };
async function start() {
  child = spawn(harness, [db, String(port)], {env, cwd: root, stdio: ['ignore', 'pipe', 'pipe']});
  let stderr = '';
  child.stderr.on('data', chunk => {stderr = (stderr + chunk).slice(-3000);});
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('harness startup timed out')), 30_000);
    child.once('exit', code => { clearTimeout(timer); reject(new Error(`harness exit ${code}: ${stderr}`)); });
    child.stdout.on('data', chunk => { if (String(chunk).includes('ready')) {clearTimeout(timer); resolve();} });
  });
}
async function stop() {
  if (child && child.exitCode === null) { const done = new Promise(resolve => child.once('exit', resolve)); child.kill('SIGTERM'); await done; }
}
function pair(command = 'pair') {
  const output = execFileSync(helper, [command, db, origin, ...(command === 'recover' ? ['--confirm-reset'] : [])], {env, encoding: 'utf8'});
  return output.match(/#pair=([a-f0-9]{64})/)[1];
}
async function post(context, action, data = {}, suppliedOrigin = origin) {
  return context.request.post(`${origin}/auth/passkey/${action}`, {headers: {Origin: suppliedOrigin}, data});
}
async function statusText(page, pattern) {
  try {
    await page.locator('#status').filter({hasText: pattern}).waitFor({timeout: 15_000});
  } catch {
    throw new Error(`Access page did not reach expected state: ${await page.locator('#status').textContent()}`);
  }
}
try {
  await start();
  browser = await chromium.launch({headless: true});
  const context = await browser.newContext({baseURL: origin});
  const page = await context.newPage();
  activePage = page;
  const cdp = await context.newCDPSession(page);
  await cdp.send('WebAuthn.enable');
  const authenticatorOptions = {protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true};
  const firstAuthenticator = await cdp.send('WebAuthn.addVirtualAuthenticator', {options: authenticatorOptions});
  // Real unauthenticated UI: a disabled Google integration must not be offered.
  await page.goto(`${origin}/ui/`);
  await page.getByRole('link', {name: 'Passkey', exact: true}).waitFor();
  assert.equal(await page.locator('a[href="/auth/login/google"]').count(), 0);
  const loginMethods = await (await context.request.get(`${origin}/api/session`)).json();
  assert.equal(loginMethods.google_login_enabled, false);
  assert.equal(loginMethods.passkey_login_enabled, true);
  check('Google login is hidden by default; the unauthenticated console offers the configured Passkey path');
  await page.goto(`${origin}/access`);
  for (const [width, height, scheme] of [[1280, 900, 'light'], [390, 844, 'dark'], [320, 740, 'light']]) {
    await page.setViewportSize({width, height});
    await page.emulateMedia({colorScheme: scheme, reducedMotion: 'reduce'});
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    assert.equal(await page.locator('#login').evaluate(el => getComputedStyle(el).backgroundColor === 'rgba(0, 0, 0, 0)'), false);
    if (process.env.AEGIS_ACCESS_SCREENSHOTS) {
      mkdirSync(process.env.AEGIS_ACCESS_SCREENSHOTS, {recursive: true});
      await page.screenshot({path: join(process.env.AEGIS_ACCESS_SCREENSHOTS, `access-${width}-${scheme}.png`), fullPage: true});
    }
  }
  await page.locator('#language').click();
  assert.equal(await page.locator('html').getAttribute('lang'), 'zh-CN');
  await page.locator('#pair-panel summary').click();
  await page.locator('#pairing').fill('invalid');
  await page.locator('#register').click();
  await statusText(page, /配对码无效/);
  assert.equal(await page.locator('#register').isEnabled(), true);
  if (process.env.AEGIS_ACCESS_SCREENSHOTS) {
    await page.screenshot({path: join(process.env.AEGIS_ACCESS_SCREENSHOTS, 'access-320-zh-error.png'), fullPage: true});
  }
  await page.locator('#language').click();
  await statusText(page, /invalid, expired/);
  assert.equal(await page.locator('#pairing').inputValue(), '');
  // A local installation never sends a novice to a remote-only Passkey flow.
  await page.route('**/api/session', route => route.fulfill({json: {authenticated: false, google_login_enabled: false, passkey_login_enabled: false}}));
  await page.goto(`${origin}/ui/`);
  await page.getByText(/No Google account setup is needed|无需配置 Google/).waitFor();
  assert.equal(await page.locator('a[href="/access"], a[href="/auth/login/google"]').count(), 0);
  await page.goto(`${origin}/access`);
  await page.locator('#local-help').waitFor();
  assert.equal(await page.locator('#login').isVisible(), false);
  await page.unroute('**/api/session');
  await page.setViewportSize({width: 1280, height: 900});
  check('access page shares theme, fits desktop/320px screens, switches language and permits retry after invalid pairing');
  await page.goto(`${origin}/access`);
  await page.route('**/auth/passkey/login-start', route => route.abort('internetdisconnected'));
  await page.locator('#login').click();
  await statusText(page, /Could not reach Aegis/);
  assert.doesNotMatch(await page.locator('#status').textContent(), /access was reset/);
  assert.equal(await page.locator('#login').isEnabled(), true);
  await page.locator('#language').click();
  await statusText(page, /无法连接 Aegis/);
  await page.locator('#language').click();
  await page.unroute('**/auth/passkey/login-start');
  check('transport failure has a distinct bilingual connection hint and permits retry');
  assert.equal((await context.request.get(`${origin}/auth/login/local?token=test-local-token-must-not-work-remotely`)).status(), 404);
  assert.equal((await context.request.post(`${origin}/mfa/host-confirm`, {data: {}})).status(), 403);
  assert.equal((await post(context, 'register-start', {pairing_code: '0'.repeat(64)}, 'https://evil.invalid')).status(), 403);
  assert.equal((await context.request.post(`${origin}/auth/passkey/login-start`, {data: {}})).status(), 403);
  assert.equal((await post(context, 'register-start', {pairing_code: '0'.repeat(64)})).status(), 400);
  assert.equal((await context.request.get(`${origin}/api/session`)).status(), 200);
  assert.equal((await (await context.request.get(`${origin}/api/session`)).json()).authenticated, false);
  const protectedResponse = await context.request.get(`${origin}/webapp/vnext/status`);
  assert.ok([401, 403].includes(protectedResponse.status()));
  check('unauthorized, spoofed origin, host-login and host-confirm requests rejected');
  const code = pair();
  await page.goto(`${origin}/access#pair=${code}`);
  await page.waitForFunction(() => location.hash === '');
  await page.locator('#register').click();
  await statusText(page, /Passkey created/);
  assert.equal((await post(context, 'register-start', {pairing_code: code})).status(), 400);
  check('host pairing and real WebAuthn registration; secret removed from URL; pairing replay rejected');
  await page.locator('#login').click();
  await page.waitForURL('**/ui/');
  let session = await (await context.request.get(`${origin}/api/session`)).json();
  assert.equal(session.authenticated, true);
  assert.equal(session.mfa_level, 'webauthn');
  assert.ok(session.csrf_token);
  let cookies = await context.cookies();
  const cookie = cookies.find(c => c.name === '__Host-aegis_session');
  assert.ok(cookie?.httpOnly && cookie?.secure);
  check('real WebAuthn assertion creates a secure HttpOnly owner session with MFA');
  const voiceStatus = await (await context.request.get(`${origin}/webapp/vnext/voice/status`)).json();
  assert.equal(voiceStatus.realtime_available, false);
  assert.equal(voiceStatus.reason, 'deferred_to_next_release');
  const voiceMint = await context.request.post(`${origin}/webapp/vnext/voice/session`, {
    headers: {Origin: origin, 'x-aegis-csrf': session.csrf_token}, data: {},
  });
  assert.equal(voiceMint.status(), 501);
  await page.locator('.vnext-shell').waitFor({timeout: 15_000});
  assert.equal(await page.getByTestId('ceo-voice-agent').count(), 0);
  check('default release refuses realtime mint and does not mount the voice agent');
  const avatarShots = join(root, 'target/avatar-ui');
  mkdirSync(avatarShots, {recursive: true});
  for (const [width, colorScheme] of [[320, 'light'], [390, 'dark']]) {
    await page.setViewportSize({width, height: 844});
    await page.emulateMedia({colorScheme});
    const scenarioContext = {page,
      ok: () => {}, fail: (message) => assert.fail(message),
      shot: label => page.screenshot({path: join(avatarShots, `${width}-${colorScheme}-${label}.png`)}),
    };
    await runChatNavigation(scenarioContext);
    await runNavigationScroll(scenarioContext);
    const portrait = page.locator('.vnext-chat-topbar img').first();
    await portrait.waitFor({state: 'visible'});
    await portrait.evaluate(img => img.decode());
    assert.equal(await portrait.getAttribute('src'), '/ui/aegis-avatar-v2.jpg');
    assert(await portrait.evaluate(img => img.naturalWidth > 0));
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await page.screenshot({path: join(avatarShots, `${width}-${colorScheme}-avatar.png`)});
  }
  await page.setViewportSize({width: 1280, height: 900});
  await page.emulateMedia({colorScheme: 'light'});
  check('mobile default portrait loads; main chat retains draft; duplicate opener and stale scroll locks are absent');

  await checkProgressLanguage({page, context, origin, csrfToken: session.csrf_token});
  check('English and Chinese live progress follow the selected language; disabled Hercules is absent from settings');

  await checkSettingsPersistence({page, context, origin, csrfToken: session.csrf_token});
  check('failed paid-fallback disable retains saved state and permits a successful retry in both languages');

  for (const provider of ['codex_oauth', 'auto']) {
    const changed = await context.request.post(`${origin}/webapp/vnext/settings/web-search`, {
      headers: {Origin: origin, 'x-aegis-csrf': session.csrf_token},
      data: {schema_version: 1, web_search_provider: provider, idempotency_key: `search-${provider}`},
    });
    assert.equal(changed.status(), 200);
    const settings = await changed.json();
    assert.equal(settings.web_search_provider, provider);
    assert.equal(settings.web_search_model, '');
    assert.deepEqual(settings.web_search_model_options, []);
  }
  const retiredSearch = await context.request.post(`${origin}/webapp/vnext/settings/web-search`, {
    headers: {Origin: origin, 'x-aegis-csrf': session.csrf_token},
    data: {schema_version: 1, web_search_provider: 'duckduckgo', idempotency_key: 'search-retired'},
  });
  assert.equal(retiredSearch.status(), 400, 'retired search must not be saved');
  check('authenticated settings accept subscription/automatic search and reject the retired provider');
  await context.request.post(`${origin}/webapp/vnext/settings/display-language`, {
    headers: {Origin: origin, 'x-aegis-csrf': session.csrf_token},
    data: {schema_version: 1, display_language: 'en', idempotency_key: 'jev-language-en'},
  });
  await page.goto(`${origin}/ui/?view=settings`);
  await page.locator('[data-settings-tier="advanced"] > summary').click();
  const providerBefore = await (await context.request.get(`${origin}/webapp/vnext/settings`)).json();
  let providerWrites = 0;
  await page.route('**/webapp/vnext/settings/llm-provider', async route => {
    providerWrites++;
    assert.equal(route.request().postDataJSON().llm_provider, 'xai_subscription');
    if (providerWrites === 1) await route.fulfill({status: 422, body: 'Validation failed: authentication failed. Not saved; previous configuration is unchanged.'});
    else await route.continue();
  });
  await page.locator('.vnext-settings-provider-toggle [data-seg-value="xai_subscription"]').click();
  assert.equal(providerWrites, 0, 'staging provider must not save');
  const verifySave = page.getByRole('button', {name: 'Verify and save', exact: true});
  await verifySave.click();
  await page.getByRole('status').filter({hasText: 'Validation failed: authentication failed.'}).waitFor();
  const providerAfterFailure = await (await context.request.get(`${origin}/webapp/vnext/settings`)).json();
  assert.equal(providerAfterFailure.llm_provider, providerBefore.llm_provider);
  assert.equal(providerAfterFailure.llm_model, providerBefore.llm_model);
  await verifySave.click();
  await page.getByRole('status').filter({hasText: 'Verified and saved'}).waitFor();
  assert.equal(providerWrites, 2);
  const providerAfterSuccess = await (await context.request.get(`${origin}/webapp/vnext/settings`)).json();
  assert.equal(providerAfterSuccess.llm_provider, 'xai_subscription');
  await page.unroute('**/webapp/vnext/settings/llm-provider');
  check('provider and model are staged; failed verification preserves settings; explicit retry saves');
  await checkModelRecommendation({page, context, origin, csrfToken: session.csrf_token});
  check('DeepSeek recommendation is bilingual and catalog-bound; staging and rejected probes preserve chat and backup settings');
  const jev = page.locator('.vnext-jev-card');
  const jevToggle = jev.getByRole('switch');
  await jevToggle.waitFor();
  assert.equal(await jevToggle.isChecked(), false);
  assert.equal(await jev.getByRole('button', {name: 'Save', exact: true}).isEnabled(), false);
  await jevToggle.check();
  assert.equal(await jev.getByRole('button', {name: 'Save', exact: true}).isEnabled(), false);
  await jev.getByLabel('OpenRouter API key', {exact: true}).fill('sk-or-synthetic-browser-settings-key');
  assert.equal(await jev.getByRole('button', {name: 'Save', exact: true}).isEnabled(), false);
  await jev.getByRole('checkbox').check();
  await jev.getByLabel('OpenRouter API key', {exact: true}).fill('sk-or-synthetic-rejected-browser-key');
  await jev.getByRole('button', {name: 'Save', exact: true}).click();
  await jev.getByRole('status').filter({hasText: 'Validation failed:'}).waitFor();
  const rejectedJev = await (await context.request.get(`${origin}/webapp/vnext/settings/jev`)).json();
  assert.equal(rejectedJev.enabled, false);
  assert.equal(rejectedJev.saved_key, false);
  assert.equal(rejectedJev.verified, false);
  await jev.getByLabel('OpenRouter API key', {exact: true}).fill('sk-or-synthetic-browser-settings-key');
  await jev.getByRole('button', {name: 'Save', exact: true}).click();
  await jev.getByRole('status').filter({hasText: 'Saved.'}).waitFor();
  assert.equal(await jev.getByLabel('OpenRouter API key', {exact: true}).inputValue(), '');
  let jevStatus = await (await context.request.get(`${origin}/webapp/vnext/settings/jev`)).json();
  assert.equal(jevStatus.enabled, true);
  assert.equal(jevStatus.saved_key, true);
  assert.equal(jevStatus.verified, true);
  assert.ok(jevStatus.checked_at > 0);
  assert.ok(!JSON.stringify(jevStatus).includes('sk-or-'));
  await page.reload();
  await page.locator('[data-settings-tier="advanced"] > summary').click();
  await jevToggle.waitFor();
  assert.equal(await jevToggle.isChecked(), true);
  await jevToggle.uncheck();
  await jev.getByRole('button', {name: 'Cancel', exact: true}).click();
  assert.equal(await jevToggle.isChecked(), true);
  await jevToggle.uncheck();
  await page.route('**/webapp/vnext/settings/jev', async route => {
    if (route.request().method() === 'POST') await route.fulfill({status: 503, body: 'temporary failure'});
    else await route.continue();
  });
  await jev.getByRole('button', {name: 'Save', exact: true}).click();
  await jev.getByRole('status').filter({hasText: 'Could not confirm'}).waitFor();
  assert.match(await jev.locator('.vnext-jev-state').textContent(), /Enabled/);
  await page.unroute('**/webapp/vnext/settings/jev');
  await jev.getByRole('button', {name: 'Reload', exact: true}).click();
  await jev.getByRole('button', {name: 'Remove saved key', exact: true}).click();
  await jev.getByRole('button', {name: 'Save', exact: true}).click();
  await jev.getByRole('status').filter({hasText: 'Saved.'}).waitFor();
  jevStatus = await (await context.request.get(`${origin}/webapp/vnext/settings/jev`)).json();
  assert.equal(jevStatus.enabled, false);
  assert.equal(jevStatus.saved_key, false);
  await page.setViewportSize({width: 390, height: 844});
  await jev.scrollIntoViewIfNeeded();
  await page.screenshot({path: join(root, 'target', 'jev-settings-mobile-en.png')});
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth));
  await context.request.post(`${origin}/webapp/vnext/settings/display-language`, {
    headers: {Origin: origin, 'x-aegis-csrf': session.csrf_token},
    data: {schema_version: 1, display_language: 'zh', idempotency_key: 'jev-language-zh'},
  });
  await page.reload();
  await page.locator('[data-settings-tier="advanced"] > summary').click();
  await jevToggle.waitFor();
  assert.match(await jev.textContent(), /独立 API 计费/);
  await jev.scrollIntoViewIfNeeded();
  await page.screenshot({path: join(root, 'target', 'jev-settings-mobile-zh.png')});
  await page.setViewportSize({width: 1280, height: 960});
  check('Jev settings consent, encrypted save/reload, cancel, failure recovery, removal and bilingual mobile layout');
  await runJevScenario({page, ok: check, shot: name => page.screenshot({path: join(root, 'target', `jev-scenario-${name}.png`)})});
  assert.equal((await post(context, 'revoke-sessions')).status(), 403);
  check('session revocation requires CSRF proof');
  // Perform account-management operations on their real page, after the
  // console navigation has completed (the console can update its own URL).
  await page.goto(`${origin}/access`);
  const makeAssertion = () => page.evaluate(async () => {
    const s = await (await fetch('/auth/passkey/login-start', {method:'POST', headers:{'Content-Type':'application/json'}, body:'{}'})).json();
    const dec = s => Uint8Array.from(atob(s.replace(/-/g,'+').replace(/_/g,'/')), c=>c.charCodeAt(0));
    const enc = b => btoa(String.fromCharCode(...new Uint8Array(b))).replace(/\+/g,'-').replace(/\//g,'_').replace(/=+$/,'');
    s.options.publicKey.challenge = dec(s.options.publicKey.challenge);
    s.options.publicKey.allowCredentials.forEach(c=>c.id=dec(c.id));
    const c = await navigator.credentials.get(s.options);
    return {challenge_id:s.challenge_id, credential:{id:c.id,rawId:enc(c.rawId),type:c.type,extensions:c.getClientExtensionResults(),response:{clientDataJSON:enc(c.response.clientDataJSON),authenticatorData:enc(c.response.authenticatorData),signature:enc(c.response.signature),...(c.response.userHandle?{userHandle:enc(c.response.userHandle)}:{})}}};
  });
  const assertion = await makeAssertion();
  const tampered = structuredClone(assertion);
  tampered.credential.response.signature = 'AAAA';
  assert.equal((await post(context, 'login-finish', tampered)).status(), 400);
  assert.equal((await post(context, 'login-finish', assertion)).status(), 400);
  check('invalid signature rejected and consumed challenge cannot be replayed');
  const validAssertion = await makeAssertion();
  assert.equal((await post(context, 'login-finish', validAssertion)).status(), 200);
  assert.equal((await post(context, 'login-finish', validAssertion)).status(), 400);
  check('a valid signed assertion succeeds exactly once');
  await cdp.send('WebAuthn.removeVirtualAuthenticator', {authenticatorId: firstAuthenticator.authenticatorId});
  await cdp.send('WebAuthn.addVirtualAuthenticator', {options: authenticatorOptions});
  const secondCode = pair();
  await page.goto(`${origin}/access#pair=${secondCode}`);
  await page.locator('#register').click(); await statusText(page, /Passkey created/);
  assert.match(execFileSync(helper, ['status', db, origin], {env, encoding:'utf8'}), /registered_passkeys=2/);
  check('a second host-authorized Passkey can be enrolled for recovery');
  await stop(); await start();
  session = await (await context.request.get(`${origin}/api/session`)).json();
  assert.equal(session.authenticated, true);
  assert.notEqual(readFileSync(db).subarray(0,16).toString(), 'SQLite format 3\0');
  check('encrypted credentials and authenticated session survive a process restart');
  const persistedJev = await (await context.request.get(`${origin}/webapp/vnext/settings/jev`)).json();
  assert.equal(persistedJev.enabled, false);
  assert.equal(persistedJev.saved_key, false);
  assert.equal(persistedJev.source, 'settings');
  const logout = await context.request.post(`${origin}/auth/passkey/revoke-sessions`, {headers:{Origin:origin,'x-aegis-csrf':session.csrf_token},data:{}});
  assert.equal(logout.status(), 200);
  const oldContext = await browser.newContext(); await oldContext.addCookies(cookies);
  assert.equal((await (await oldContext.request.get(`${origin}/api/session`)).json()).authenticated, false);
  check('revocation invalidates previously copied session cookies');
  const recovered = pair('recover');
  assert.equal((await post(context, 'login-start')).status(), 400);
  assert.equal((await post(context, 'register-start', {pairing_code: code})).status(), 400);
  await page.goto(`${origin}/access`);
  await page.setViewportSize({width: 320, height: 844});
  await page.locator('#login').click();
  await statusText(page, /If access was reset on your computer/);
  assert.doesNotMatch(await page.locator('#status').textContent(), /Check your connection/);
  assert.equal(await page.locator('#login').isEnabled(), true);
  assert.equal(await page.locator('.access-card').getAttribute('aria-busy'), 'false');
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  if (process.env.AEGIS_ACCESS_SCREENSHOTS) {
    await page.screenshot({path: join(process.env.AEGIS_ACCESS_SCREENSHOTS, 'recovery-rejected-320-en.png'), fullPage: true});
  }
  await page.locator('#language').click();
  await statusText(page, /如果已在电脑上重置访问/);
  assert.doesNotMatch(await page.locator('#status').textContent(), /检查连接/);
  if (process.env.AEGIS_ACCESS_SCREENSHOTS) {
    await page.screenshot({path: join(process.env.AEGIS_ACCESS_SCREENSHOTS, 'recovery-rejected-320-zh.png'), fullPage: true});
  }
  await page.locator('#language').click();
  await page.goto(`${origin}/access#pair=${recovered}`);
  await page.locator('#register').click(); await statusText(page, /Passkey created/);
  await page.locator('#login').click(); await page.waitForURL('**/ui/');
  assert.equal((await (await context.request.get(`${origin}/api/session`)).json()).authenticated, true);
  check('host recovery revokes old credentials and supports a fresh pairing/login');
  console.log(`${passed} remote-access browser journeys passed; no real accounts used.`);
} catch (error) {
  if (process.env.AEGIS_ACCESS_SCREENSHOTS && activePage) {
    mkdirSync(process.env.AEGIS_ACCESS_SCREENSHOTS, {recursive: true});
    await activePage.screenshot({path: join(process.env.AEGIS_ACCESS_SCREENSHOTS, 'failure.png'), fullPage: true}).catch(() => {});
  }
  throw error;
} finally {
  await browser?.close(); await stop(); rmSync(dir, {recursive:true, force:true});
}
