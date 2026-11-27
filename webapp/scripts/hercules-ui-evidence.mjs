#!/usr/bin/env node
// hercules-ui-evidence.mjs — generic Web Console evidence runner for Hercules'
// trusted CLI evidence step (WORKFLOW.md "web-console-flow"). Runs EVERY
// scenario in ./ui-scenarios/*.mjs against the isolated aegis_runtime_core
// instance in an iPhone-sized Chromium, records one video per scenario
// (<id>.webm) plus screenshots (<id>-<name>.png), and prints one line per
// assertion. Prints "ui evidence ok" only when every scenario passes.
//
// Owner rule (2026-09-06): flow changes ship a recorded operation as proof.
// A task that adds or changes an owner-visible flow adds/updates a scenario
// here; the trusted plan (argv only) never takes steps from the task body.
import { chromium, devices as playwrightDevices } from 'playwright';
import { createHmac } from 'node:crypto';
import { mkdirSync, readdirSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { probeLiveOwnerTurn } from './live-owner-turn.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const BASE = process.env.AEGIS_UI_BASE || 'http://127.0.0.1:9999';
const OUT = process.env.AEGIS_UI_OUT || 'target/hercules-ui-evidence';
const BOT_TOKEN = process.env.AEGIS_UI_BOT_TOKEN || '';
const OWNER_ID = Number(process.env.AEGIS_UI_OWNER_ID || '12345678');
const ONLY = (process.env.AEGIS_UI_SCENARIOS || '').split(',').map((s) => s.trim()).filter(Boolean);
const VIEWPORT = { width: 390, height: 844 };
// Device matrix: the primary pass keeps the historical 390×844 phone (plain
// scenario ids, what Hercules packets key on); extra passes use Playwright
// device descriptors and suffix their ids. Override with
// AEGIS_UI_DEVICES="iPhone SE,iPhone 15 Pro Max" (comma list; "none" = primary only).
const EXTRA_DEVICE_NAMES = (() => {
  const raw = process.env.AEGIS_UI_DEVICES;
  if (raw === undefined) return ['iPhone SE'];
  if (raw.trim().toLowerCase() === 'none') return [];
  return raw.split(',').map((s) => s.trim()).filter(Boolean);
})();
const DEVICE_PASSES = [
  { slug: '', label: 'phone 390x844', viewport: VIEWPORT, deviceScaleFactor: 2, userAgent: undefined },
  ...EXTRA_DEVICE_NAMES.map((name) => {
    const d = playwrightDevices[name];
    if (!d) { console.log(`FAIL: runner — unknown device "${name}"`); return null; }
    return { slug: '@' + name.toLowerCase().replace(/[^a-z0-9]+/g, '-'), label: name, viewport: d.viewport, deviceScaleFactor: d.deviceScaleFactor, userAgent: d.userAgent };
  }).filter(Boolean),
];

function initData() {
  const authDate = String(Math.floor(Date.now() / 1000));
  const user = JSON.stringify({ id: OWNER_ID, first_name: 'Evidence', username: 'hercules' });
  const dcs = `auth_date=${authDate}\nuser=${user}`;
  const secret = createHmac('sha256', 'WebAppData').update(BOT_TOKEN).digest();
  const hash = createHmac('sha256', secret).update(dcs).digest('hex');
  return `auth_date=${authDate}&user=${encodeURIComponent(user)}&hash=${hash}`;
}

async function loadScenarios() {
  const dir = join(HERE, 'ui-scenarios');
  const files = readdirSync(dir).filter((f) => f.endsWith('.mjs')).sort();
  const scenarios = [];
  for (const f of files) {
    const mod = await import(pathToFileURL(join(dir, f)).href);
    if (!mod.id || typeof mod.run !== 'function') throw new Error(`scenario ${f} must export id and run()`);
    if (ONLY.length && !ONLY.includes(mod.id)) continue;
    scenarios.push({ file: f, id: mod.id, title: mod.title || mod.id, run: mod.run, surfaces: Array.isArray(mod.surfaces) ? mod.surfaces : [] });
  }
  return scenarios;
}

mkdirSync(OUT, { recursive: true });
// Scenarios may stub /webapp/vnext/command. This probe always hits the
// booted server, so a broken owner turn fails the evidence run even when
// every scenario fulfills that route itself. AEGIS_UI_SCENARIOS cannot skip it.
const liveTurn = await probeLiveOwnerTurn(BASE, initData());
if (!liveTurn.ok) {
  console.log(`FAIL: [live-owner-turn] ${liveTurn.detail}`);
  console.log('ui evidence FAILED: live owner turn');
  process.exit(1);
}
console.log(`ok: [live-owner-turn] ${liveTurn.detail}`);

const browser = await chromium.launch();
const summary = { base: BASE, viewport: VIEWPORT, scenarios: [] };
let failedTotal = 0;
try {
  const scenarios = await loadScenarios();
  if (scenarios.length === 0) throw new Error('no scenarios found in ui-scenarios/');
  for (const pass of DEVICE_PASSES) for (const base of scenarios) {
    const scenario = { ...base, id: base.id + pass.slug };
    const VIEWPORT = pass.viewport;
    console.log(`scenario ${scenario.id}: ${scenario.title} [${pass.label} ${VIEWPORT.width}x${VIEWPORT.height}]`);
    // Structured line for Hercules' coverage receipt (repo-owned surface globs).
    console.log(`hercules-scenario: ${scenario.id} surfaces=${scenario.surfaces.join(',')}`);
    const checks = [];
    const ok = (name, detail = '') => { checks.push({ name, ok: true, detail }); console.log(`ok: [${scenario.id}] ${name}${detail ? ' — ' + detail : ''}`); };
    const fail = (name, detail = '') => { checks.push({ name, ok: false, detail }); console.log(`FAIL: [${scenario.id}] ${name}${detail ? ' — ' + detail : ''}`); };
    const context = await browser.newContext({
      baseURL: BASE,
      // Scenarios route synthetic API responses. A newly installed service
      // worker can bypass those routes after reload and use backend data instead.
      // Worker code/serving has separate entrypoint tests and production smoke.
      serviceWorkers: 'block',
      viewport: VIEWPORT,
      deviceScaleFactor: pass.deviceScaleFactor ?? 2,
      ...(pass.userAgent ? { userAgent: pass.userAgent } : {}),
      isMobile: true,
      hasTouch: true,
      recordVideo: { dir: OUT, size: VIEWPORT },
      extraHTTPHeaders: BOT_TOKEN ? { Authorization: `TelegramWebApp ${initData()}` } : {},
    });
    const page = await context.newPage();
    page.setDefaultTimeout(15000);
    const shot = async (name) => { await page.screenshot({ path: join(OUT, `${scenario.id}-${name}.png`) }); };
    const inViewport = async (locator) => {
      const box = await locator.boundingBox();
      if (!box) return { visible: false, box };
      const inside = box.y >= 0 && box.x >= 0 && box.y + box.height <= VIEWPORT.height + 1 && box.x + box.width <= VIEWPORT.width + 1;
      return { visible: inside, box };
    };
    try {
      await scenario.run({ page, context, ok, fail, shot, inViewport, viewport: VIEWPORT });
    } catch (err) {
      fail('unexpected error', String(err && err.message ? err.message : err));
    }
    const video = page.video();
    await context.close();
    let videoPath = video ? await video.path().catch(() => null) : null;
    if (videoPath) {
      const stable = join(OUT, `${scenario.id}.webm`);
      try { renameSync(videoPath, stable); videoPath = stable; } catch { /* keep generated name */ }
    }
    const failed = checks.filter((c) => !c.ok).length;
    failedTotal += failed;
    summary.scenarios.push({ id: scenario.id, title: scenario.title, file: scenario.file, checks, video: videoPath });
    console.log(`scenario ${scenario.id}: ${failed === 0 ? 'passed' : 'FAILED'} (${checks.length - failed}/${checks.length}) video=${videoPath ?? '-'}`);
  }
} catch (err) {
  failedTotal += 1;
  console.log(`FAIL: runner — ${String(err && err.message ? err.message : err)}`);
} finally {
  await browser.close();
  writeFileSync(join(OUT, 'summary.json'), JSON.stringify(summary, null, 2));
  const total = summary.scenarios.reduce((n, s) => n + s.checks.length, 0);
  if (failedTotal === 0) console.log(`ui evidence ok: ${summary.scenarios.length} scenarios, ${total} checks`);
  else console.log(`ui evidence FAILED: ${failedTotal} failed checks across ${summary.scenarios.length} scenarios`);
}
process.exit(failedTotal === 0 ? 0 : 1);
