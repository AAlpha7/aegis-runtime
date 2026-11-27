import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, it } from 'node:test';

async function load() {
  const outDir = mkdtempSync(join(tmpdir(), 'aegis-x-deeplink-'));
  const outfile = join(outDir, 'xDeepLink.mjs');
  await build({
    entryPoints: [new URL('../src/vnext/xDeepLink.ts', import.meta.url).pathname],
    bundle: true,
    platform: 'node',
    format: 'esm',
    outfile,
    logLevel: 'silent',
  });
  return import(`file://${outfile}`);
}

describe('xDeepLink helpers', () => {
  it('normalizes x/twitter hosts to https://x.com', async () => {
    const m = await load();
    assert.equal(
      m.normalizeXUrl('https://twitter.com/foo/status/1234567890'),
      'https://x.com/foo/status/1234567890',
    );
    assert.equal(m.normalizeXUrl('https://evil.com/x.com/foo'), null);
    assert.equal(m.extractXStatusId('https://x.com/a/status/999'), '999');
    assert.equal(m.extractXHandle('https://x.com/elonmusk'), 'elonmusk');
    assert.equal(m.extractXHandle('https://x.com/elonmusk/status/1'), null);
  });

  it('extracts X URLs from free text and work items', async () => {
    const m = await load();
    const urls = m.extractXUrlsFromText('See https://x.com/a/status/1 and https://twitter.com/b/status/2');
    assert.deepEqual(urls, ['https://x.com/a/status/1', 'https://x.com/b/status/2']);
    assert.equal(
      m.extractXUrlFromWorkItem({ summary: 'Track https://x.com/foo/status/42 for reply' }),
      'https://x.com/foo/status/42',
    );
  });

  it('collapses one post cited as host, tracking, and web-status aliases', async () => {
    const m = await load();
    const links = m.dedupeXLinks([
      'https://x.com/i/web/status/111?s=20',
      'https://twitter.com/alice/status/111',
      'https://x.com/alice/status/111?ref_src=twsrc',
      'https://x.com/bob/status/222',
      'https://x.com/bob',
    ]);
    assert.deepEqual(links, [
      'https://x.com/alice/status/111',
      'https://x.com/bob/status/222',
      'https://x.com/bob',
    ]);
    const handoff = m.xHandoffTargets(links);
    assert.equal(handoff.openHref, 'https://x.com/alice/status/111');
    assert.match(handoff.replyHref, /in_reply_to=111/);
    assert.deepEqual(handoff.extras.map((extra) => extra.caption), ['@bob · 222', '@bob']);
  });

  it('decodes percent-encoded UTF-8 in owner-facing link captions', async () => {
    const m = await load();
    assert.equal(m.ownerFacingUrlText('xianba%E2%80%A6'), 'xianba…');
    assert.equal(m.ownerFacingUrlText('plain'), 'plain');
    assert.equal(m.ownerFacingUrlText('100%'), '100%');
    const handoff = m.xHandoffTargets([
      'https://x.com/alice/status/111',
      'https://x.com/xianba%E2%80%A6',
    ]);
    assert.equal(handoff.openHref, 'https://x.com/alice/status/111');
    assert.deepEqual(handoff.extras.map((extra) => extra.caption), ['xianba…']);
    assert.equal(handoff.extras[0].caption.includes('%'), false);
  });

  it('disambiguates two posts from the same author and skips a reply for profiles', async () => {
    const m = await load();
    const posts = m.xHandoffTargets([
      'https://x.com/alice/status/1111',
      'https://x.com/alice/status/2222',
      'https://x.com/alice/status/3333',
    ]);
    assert.equal(posts.openHref, 'https://x.com/alice/status/1111');
    assert.deepEqual(posts.extras.map((extra) => extra.caption), ['@alice · 2222', '@alice · 3333']);
    const profile = m.xHandoffTargets(['https://twitter.com/alice?s=20']);
    assert.equal(profile.openHref, 'https://x.com/alice');
    assert.equal(profile.replyHref, null);
    assert.deepEqual(profile.extras, []);
    assert.equal(m.xHandoffTargets([]), null);
  });

  it('keeps the work dock handoff until an open answer can own it', async () => {
    const m = await load();
    const closed = m.mergeWorkAndAnswerXHandoff({
      assistantOpen: false,
      finishedAnswerVisible: true,
      workHref: 'https://x.com/alice/status/111',
      answerLinks: ['https://x.com/alice/status/111'],
    });
    assert.equal(closed.suppressDock, false);
    assert.deepEqual(closed.extraLinks, []);
    const typing = m.mergeWorkAndAnswerXHandoff({
      assistantOpen: true,
      finishedAnswerVisible: false,
      workHref: 'https://x.com/alice/status/111',
      answerLinks: [],
    });
    assert.equal(typing.suppressDock, false);
    const answered = m.mergeWorkAndAnswerXHandoff({
      assistantOpen: true,
      finishedAnswerVisible: true,
      workHref: 'https://twitter.com/alice/status/111?s=20',
      answerLinks: ['https://x.com/bob/status/222'],
    });
    assert.equal(answered.suppressDock, true);
    assert.deepEqual(answered.extraLinks, ['https://twitter.com/alice/status/111?s=20']);
    const quiet = m.mergeWorkAndAnswerXHandoff({
      assistantOpen: true,
      finishedAnswerVisible: true,
      workHref: null,
      answerLinks: [],
    });
    assert.equal(quiet.suppressDock, false);
  });

  it('builds compose and reply intents without silent post', async () => {
    const m = await load();
    const compose = m.buildXComposeUrl({ text: 'hello world' });
    assert.match(compose, /^https:\/\/x\.com\/intent\/post\?/);
    assert.match(compose, /text=hello(\+|%20)world/);
    const reply = m.buildXReplyComposeUrl('https://x.com/a/status/123', 'thanks');
    assert.match(reply, /in_reply_to=123/);
    assert.match(reply, /text=thanks/);
  });
});
