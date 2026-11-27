import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';

const here = dirname(fileURLToPath(import.meta.url));
const webappRoot = join(here, '..');
const outDir = join(webappRoot, 'node_modules/.cache/aegis-runtime-vnext-ceo-intent-test');
mkdirSync(outDir, { recursive: true });
const outFile = join(outDir, 'ceoIntent.mjs');

const build = spawnSync(
  'npx',
  [
    'esbuild',
    'src/vnext/ceoIntent.ts',
    '--bundle',
    '--platform=node',
    '--format=esm',
    `--outfile=${outFile}`,
  ],
  { cwd: webappRoot, encoding: 'utf8' },
);
assert.equal(build.status, 0, build.stderr || build.stdout);

const {
  resolveCeoShorthand,
  resolveUiNavigationIntent,
  splitUiNavigationPrefix,
  isShortCeoUtterance,
  ownerFacingApprovalAction,
  ownerFacingApprovalWhy,
  riskClassNeedsReview,
  voicePendingDecision,
} = await import(`file://${outFile}`);

describe('ceoIntent shorthand (CEO Intent Contract)', () => {
  it('resolves one-word zh commute and expands for the runtime', () => {
    const hit = resolveCeoShorthand('通勤', 'zh');
    assert.ok(hit);
    assert.equal(hit.intentId, 'commute_status');
    assert.match(hit.label, /路况|出发/);
    assert.match(hit.expansion, /通勤|出发/);
    assert.equal(hit.needsReviewHint, false);
  });

  it('resolves send as high-risk review hint', () => {
    const hit = resolveCeoShorthand('发送', 'zh');
    assert.ok(hit);
    assert.equal(hit.intentId, 'send_draft');
    assert.equal(hit.needsReviewHint, true);
    assert.equal(riskClassNeedsReview(hit.riskClass), true);
  });

  it('treats 批准。 and 先不要 as the pending voice decision', () => {
    assert.equal(voicePendingDecision('批准。'), 'approve');
    assert.equal(voicePendingDecision('批准'), 'approve');
    assert.equal(voicePendingDecision('先不要'), 'reject');
    assert.equal(voicePendingDecision('先不要。'), 'reject');
    assert.equal(voicePendingDecision('我帮你查一下'), null);
    assert.equal(resolveCeoShorthand('先不要', 'zh')?.intentId, 'reject_pending');
  });

  it('ignores long free text (no false shorthand)', () => {
    assert.equal(resolveCeoShorthand('请帮我仔细分析一下这周所有的法务往来邮件并写纪要', 'zh'), null);
    assert.equal(isShortCeoUtterance('请帮我仔细分析一下这周所有的法务往来邮件并写纪要'), false);
  });

});

describe('ceoIntent UI navigation', () => {
  it('maps other pure surface opens', () => {
    assert.equal(resolveUiNavigationIntent('打开工作'), 'work');
    assert.equal(resolveUiNavigationIntent('打开设置'), 'settings');
    assert.equal(resolveUiNavigationIntent('回到首页'), 'home');
    assert.equal(resolveUiNavigationIntent('open settings'), 'settings');
  });

  it('does not treat mail phrases as a surface', () => {
    assert.equal(resolveUiNavigationIntent('打开邮件'), null);
    assert.equal(resolveUiNavigationIntent('open mail'), null);
    assert.equal(splitUiNavigationPrefix('打开邮件，看看 OpenRouter 最新那封'), null);
  });
});
