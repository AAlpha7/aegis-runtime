import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { pathToFileURL } from 'node:url';
import test from 'node:test';

const webappRoot = new URL('..', import.meta.url);
const repoRoot = new URL('../..', import.meta.url);

function walk(dir, acc = []) {
  for (const name of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, name.name);
    if (name.isDirectory()) walk(path, acc);
    else if (name.name.endsWith('.tsx')) acc.push(path);
  }
  return acc;
}

async function loadScenarios() {
  const dir = new URL('../scripts/ui-scenarios/', import.meta.url);
  const files = readdirSync(dir).filter((name) => name.endsWith('.mjs')).sort();
  const scenarios = [];
  for (const file of files) {
    const mod = await import(pathToFileURL(join(dir.pathname, file)).href);
    scenarios.push({ file, surfaces: Array.isArray(mod.surfaces) ? mod.surfaces : [] });
  }
  return scenarios;
}

test('live owner-turn scenario does not stub the command route', () => {
  const source = readFileSync(new URL('../scripts/ui-scenarios/chat-live-owner-turn.mjs', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /page\.route\s*\(/);
  assert.match(source, /\/webapp\/vnext\/command/);
  assert.match(source, /收到/);
  const runner = readFileSync(new URL('../scripts/hercules-ui-evidence.mjs', import.meta.url), 'utf8');
  const probeAt = runner.indexOf('probeLiveOwnerTurn(');
  const launchAt = runner.indexOf('chromium.launch');
  assert.ok(probeAt > 0 && launchAt > probeAt, 'live owner turn must run before scenario stubs');
  const probe = readFileSync(new URL('../scripts/live-owner-turn.mjs', import.meta.url), 'utf8');
  assert.match(probe, /visible_answer/);
  assert.doesNotMatch(probe, /page\.route\s*\(/);
});

test('a new owner-visible flow without a scenario fails', async () => {
  const vnext = new URL('../src/vnext/', import.meta.url);
  const files = walk(vnext.pathname)
    .map((path) => relative(repoRoot.pathname, path).split('\\').join('/'))
    .sort();
  const scenarios = await loadScenarios();
  const covered = new Set();
  for (const scenario of scenarios) {
    for (const surface of scenario.surfaces) {
      if (typeof surface === 'string' && surface.startsWith('webapp/')) covered.add(surface);
    }
  }
  const uncovered = files.filter((file) => !covered.has(file));
  const known = JSON.parse(readFileSync(new URL('../scripts/ui-scenarios/uncovered-flows.json', import.meta.url), 'utf8'));
  assert.deepEqual(
    uncovered,
    known,
    'Owner-visible vnext tsx files must be named in a ui-scenarios surfaces list. Add a scenario, or the shrink-only uncovered-flows.json entry is stale.',
  );
});
