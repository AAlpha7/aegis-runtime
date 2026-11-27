import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

// Reuses the compiled catalog module produced by the i18n-parity build step
// (tsc -> catalog.js). The App.tsx app-shell chrome (auth/loading screens,
// brand rail, nav buttons, header, work badge) renders these keys via
// t()/tf() with `errorLocale`, so verifying the catalog resolution here proves
// the shell switches zh/en without standing up the full VNextApp render tree.
const modulePath = process.env.I18N_CATALOG_MODULE;

if (!modulePath) {
  throw new Error('I18N_CATALOG_MODULE must point at the compiled catalog.js file');
}

const { t, tf, CATALOG, setAssistantName, assistantName } = await import(modulePath);

// Mirror of App.tsx's workBadgeLabel construction (lines ~578). Kept in sync
// with the component so the plural/locale branching is exercised end to end.
function workBadgeLabel(decisionCount, readyCount, locale) {
  if (decisionCount > 0) {
    return tf(
      decisionCount === 1
        ? 'app.shell.workBadge.decisionsSingular'
        : 'app.shell.workBadge.decisionsPlural',
      locale,
      { count: String(decisionCount) },
    );
  }
  if (readyCount > 0) {
    return tf(
      readyCount === 1
        ? 'app.shell.workBadge.readySingular'
        : 'app.shell.workBadge.readyPlural',
      locale,
      { count: String(readyCount) },
    );
  }
  return t('app.shell.workBadge.plain', locale);
}

describe('vNext app-shell i18n', () => {
  it('localizes the auth + loading screens for both locales', () => {
    assert.equal(t('app.shell.loading.title', 'zh'), '正在加载 Aegis');
    assert.equal(t('app.shell.loading.title', 'en'), 'Loading Aegis');
    assert.equal(t('app.shell.error.title', 'zh'), '无法打开 Aegis');
    assert.equal(t('app.shell.error.title', 'en'), 'Could not open Aegis');
    assert.equal(t('app.shell.error.retry', 'zh'), '重试');
    assert.equal(t('app.shell.error.retry', 'en'), 'Retry');
    assert.equal(t('app.shell.auth.title', 'zh'), '登录 Aegis');
    assert.equal(t('app.shell.auth.title', 'en'), 'Sign in to Aegis');
    assert.equal(t('app.shell.auth.continueWithGoogle', 'zh'), '使用 Google 继续');
    assert.equal(t('app.shell.auth.continueWithGoogle', 'en'), 'Continue with Google');
  });

  it('localizes the nav buttons (desktop + mobile share these keys)', () => {
    assert.equal(t('app.shell.nav.aegis', 'zh'), '安琪');
    assert.equal(t('app.shell.nav.aegis', 'en'), 'Aegis');
    assert.equal(t('app.shell.nav.work', 'zh'), '工作');
    assert.equal(t('app.shell.nav.work', 'en'), 'Work');
    assert.equal(t('app.shell.nav.chat', 'en'), 'Aegis');
  });

  it('localizes the brand rail + owner card', () => {
    assert.equal(t('app.shell.tagline.secretary', 'zh'), '你的私人秘书。');
    assert.equal(t('app.shell.tagline.secretary', 'en'), 'Your private secretary.');
    assert.equal(t('app.shell.ownerCard.signedIn', 'zh'), '已登录');
    assert.equal(t('app.shell.ownerCard.signedIn', 'en'), 'Signed in');
    assert.equal(t('app.shell.ownerCard.youFallback', 'zh'), '你');
    assert.equal(t('app.shell.ownerCard.youFallback', 'en'), 'You');
    assert.equal(
      t('app.shell.brandCopy', 'en'),
      'Organizes what needs attention, asks before important changes, and keeps receipts.',
    );
  });

  it('localizes the header buttons', () => {
    assert.equal(t('app.shell.header.openStatus', 'zh'), '打开 Aegis 状态');
    assert.equal(t('app.shell.header.openStatus', 'en'), 'Open Aegis status');
    assert.equal(t('app.shell.header.saveSnapshot', 'en'), 'Save support snapshot');
    assert.equal(t('app.shell.header.saving', 'zh'), '保存中');
    assert.equal(t('app.shell.header.snapshot', 'zh'), '快照');
  });

  it('localizes header surface titles + subtitles per active view', () => {
    assert.equal(t('app.shell.surface.work.title', 'en'), 'Work');
    assert.equal(
      t('app.shell.surface.work.subtitle', 'en'),
      'Only what needs you.',
    );
    assert.equal(t('app.shell.surface.home.subtitle', 'zh'), '告诉安琪你需要什么。');
  });

  it('localizes the snapshot-toast status strings (avatar / cost-saving / support-debug)', () => {
    // en values must stay byte-identical to the prior hardcoded literals.
    assert.equal(t('app.avatarRefreshEnabled', 'en'), 'Aegis weekly avatar refresh enabled.');
    assert.equal(t('app.avatarRefreshPaused', 'en'), 'Aegis weekly avatar refresh paused.');
    assert.equal(t('app.checksResumed', 'en'), 'Automatic checks can run again under your saved settings.');
    assert.equal(t('app.snapshotNotEnabled', 'en'), 'Support snapshots are not enabled on this server.');
    assert.equal(t('app.debugLeaseNotEnabled', 'en'), 'Support debug leases are not enabled on this server.');
    assert.equal(t('app.debugJobNotEnabled', 'en'), 'Support debug jobs are not enabled on this server.');
    // zh actually switches.
    assert.equal(t('app.avatarRefreshEnabled', 'zh'), '安琪的每周头像刷新已开启。');
    assert.equal(t('app.checksResumed', 'zh'), '自动检查现在可以在你保存的设置下重新运行。');
    assert.equal(t('app.snapshotNotEnabled', 'zh'), '此服务器未启用支持快照。');
  });

  it('localizes the chatHomeNotice channel (plain + interpolated, en byte-identical)', () => {
    assert.equal(t('chatHome.notice.installed', 'en'), 'Aegis is installed on this device.');
    assert.equal(t('chatHome.notice.alreadyInstalled', 'en'), 'Aegis is already installed on this device.');
    assert.equal(t('chatHome.notice.queued', 'en'), 'Queued. Aegis will answer it next.');
    assert.equal(t('chatHome.notice.queuedWork', 'en'), 'Queued on that work item. Aegis will answer it next.');
    assert.equal(t('chatHome.notice.chatCleared', 'en'), 'Chat cleared on this browser.');
    // Interpolated queue-full notices keep the count and stay byte-identical in en.
    assert.equal(tf('chatHome.notice.queueFullCommands', 'en', { max: '5' }), 'Aegis already has 5 queued. This draft is still here.');
    assert.equal(
      tf('chatHome.notice.queueFullWork', 'en', { max: '3' }),
      'Aegis already has 3 item questions queued. Try again after the current answer appears.',
    );
    // zh switches and still embeds the count.
    assert.equal(t('chatHome.notice.queued', 'zh'), '已排队。安琪将接着回答它。');
    assert.equal(tf('chatHome.notice.queueFullCommands', 'zh', { max: '5' }), '安琪已有 5 条排队。此草稿仍保留在这里。');
  });

  it('composes the debug-lease toast with verbatim owner_message + localized label', () => {
    const owner = 'Owner authorized inspect access.';
    // en stays byte-identical to `${owner_message} Lease ${id}.`
    assert.equal(
      tf('app.debugLeaseOpened', 'en', { owner, id: 'abc12345' }),
      'Owner authorized inspect access. Lease abc12345.',
    );
    // server-provided owner_message is preserved verbatim even in zh; only the label localizes.
    assert.equal(
      tf('app.debugLeaseOpened', 'zh', { owner, id: 'abc12345' }),
      'Owner authorized inspect access. 租约 abc12345。',
    );
    assert.equal(tf('app.debugLeaseRevoked', 'en', { id: 'abc12345' }), 'Debug lease abc12345 revoked.');
    assert.equal(tf('app.debugJobStarted', 'en', { id: 'job123456789' }), 'Debug job job123456789 started.');
  });

  it('builds the work badge aria label with correct plural + count in both locales', () => {
    // English needs singular/plural agreement; Chinese embeds the bare count.
    assert.equal(workBadgeLabel(1, 0, 'en'), 'Work, 1 decision waiting');
    assert.equal(workBadgeLabel(3, 0, 'en'), 'Work, 3 decisions waiting');
    assert.equal(workBadgeLabel(1, 0, 'zh'), '工作，1 个决策待处理');
    assert.equal(workBadgeLabel(3, 0, 'zh'), '工作，3 个决策待处理');

    assert.equal(workBadgeLabel(0, 1, 'en'), 'Work, 1 item ready');
    assert.equal(workBadgeLabel(0, 5, 'en'), 'Work, 5 items ready');
    assert.equal(workBadgeLabel(0, 1, 'zh'), '工作，1 项已就绪');
    assert.equal(workBadgeLabel(0, 5, 'zh'), '工作，5 项已就绪');

    assert.equal(workBadgeLabel(0, 0, 'en'), 'Work');
    assert.equal(workBadgeLabel(0, 0, 'zh'), '工作');
  });

  // i18n receipt tail (2026-06-10): FE-authored receipt_expectation, action
  // {ok,message} results, and the sourceDetail unavailable payload. en values
  // MUST stay byte-identical to the removed App.tsx literals (other suites
  // assert the English output).
  it('localizes the receipt_expectation values (en byte-identical)', () => {
    assert.equal(
      t('app.receipt.workActionNoExternal', 'en'),
      'No additional external action ran unless this reply includes a receipt.',
    );
    assert.equal(t('app.receipt.noActionTaken', 'en'), 'No action was taken.');
    // zh actually switches.
    assert.equal(t('app.receipt.noActionTaken', 'zh'), '未执行任何操作。');
    assert.equal(t('app.receipt.workActionNoExternal', 'zh'), '除非此回复附带记录，否则没有执行任何额外的外部操作。');
  });

  it('zh copy has no system jargon', () => {
    const banned = /回执|事项|主人|幕僚长/;
    const hits = [];
    for (const [key, entry] of Object.entries(CATALOG || {})) {
      const zh = entry?.zh;
      if (typeof zh === 'string' && banned.test(zh)) hits.push(`${key}: ${zh}`);
    }
    assert.deepEqual(hits, [], `banned zh jargon: ${hits.join(' | ')}`);
  });

  it('localizes the action {ok,message} results + work-action context label', () => {
    assert.equal(t('app.action.signedOutBeforeFinished', 'en'), 'Signed out before this finished.');
    assert.equal(t('app.action.sentToAegis', 'en'), 'Sent to Aegis.');
    assert.equal(
      t('app.action.itemChangedRefreshedQueue', 'en'),
      'This item changed before the action ran. I refreshed the work queue; continue from the current available actions.',
    );
    assert.equal(tf('app.action.workContextLabel', 'en', { title: 'Renew passport' }), 'Work: Renew passport');
    assert.equal(t('app.action.workContextFallback', 'en'), 'Work action');
    // zh actually switches.
    assert.equal(t('app.action.signedOutBeforeFinished', 'zh'), '操作完成前已退出登录。');
    assert.equal(t('app.action.sentToAegis', 'zh'), '已发送给安琪。');
    assert.equal(
      t('app.action.itemChangedRefreshedQueue', 'zh'),
      '此条目在操作执行前已发生变化。我已刷新工作队列；请从当前可用操作继续。',
    );
    assert.equal(tf('app.action.workContextLabel', 'zh', { title: '续签护照' }), '工作：续签护照');
    assert.equal(t('app.action.workContextFallback', 'zh'), '工作操作');
  });

  it('localizes the source-unavailable detail payload strings', () => {
    assert.equal(t('app.sourceDetail.changedTitle', 'en'), 'This detail changed');
    assert.equal(
      t('app.sourceDetail.changedMessage', 'en'),
      'This work item changed or the underlying source is no longer available. Refresh Work to load the current version before deciding.',
    );
    assert.equal(t('app.sourceDetail.changedNextStep', 'en'), 'Refresh Work, then reopen the item.');
    // zh actually switches.
    assert.equal(t('app.sourceDetail.changedTitle', 'zh'), '此详情已变化');
    assert.equal(t('app.sourceDetail.changedNextStep', 'zh'), '刷新「工作」，然后重新打开该条目。');
  });
});

describe('assistant response evidence chrome', () => {
  it('localizes claim-evidence chips and estimate copy without banned zh words', () => {
    assert.equal(t('assistantResponse.evidence.ref', 'zh'), '来源 {id}');
    assert.equal(t('assistantResponse.evidence.ref', 'en'), 'Source {id}');
    assert.equal(t('assistantResponse.evidence.fullLength', 'zh'), '来源摘要 · 原文 {count} 字符');
    assert.equal(t('assistantResponse.evidence.fullLength', 'en'), 'Source excerpt · original {count} characters');
    assert.equal(t('assistantResponse.estimate.title', 'zh'), '推算，没有工具数据支持');
    assert.equal(t('assistantResponse.estimate.title', 'en'), 'Estimate — no tool data behind it');
    assert.equal(t('assistantResponse.estimate.count', 'zh'), '含 {count} 个推算数字');
    assert.equal(t('assistantResponse.estimate.count', 'en'), '{count} estimated number(s)');
    assert.equal(t('assistantResponse.estimate.toggle', 'zh'), '标出推算');
    assert.equal(t('assistantResponse.estimate.toggle', 'en'), 'Highlight estimates');
    for (const key of [
      'assistantResponse.evidence.ref',
      'assistantResponse.evidence.fullLength',
      'assistantResponse.estimate.title',
      'assistantResponse.estimate.count',
      'assistantResponse.estimate.toggle',
    ]) {
      assert.doesNotMatch(t(key, 'zh'), /回执|事项|主人|幕僚长/);
    }
  });
});

describe('assistant call-name', () => {
  it('defaults to 安琪 / Angie and resolves {assistant} in catalog strings', () => {
    setAssistantName(null);
    assert.equal(assistantName('zh'), '安琪');
    assert.equal(assistantName('en'), 'Angie');
    assert.equal(t('chatHome.composer.placeholder', 'zh'), '告诉安琪…');
    assert.equal(t('chatHome.composer.placeholder', 'en'), 'Tell Angie…');
    assert.equal(t('chatHome.role.aegis', 'zh'), '安琪');
  });

  it('honours a custom name from the session in every locale', () => {
    setAssistantName('小艾');
    assert.equal(t('chatHome.sheet.open', 'zh'), '和小艾说');
    assert.equal(t('chatHome.sheet.open', 'en'), 'Talk to 小艾');
    setAssistantName(null);
  });

  it('keeps Aegis as the system name in non-chat strings', () => {
    assert.equal(t('chatHome.account.aegisStatus', 'zh'), '运行状态');
    assert.equal(t('chatHome.account.aegisStatus', 'en'), 'Runtime status');
    assert.equal(t('app.shell.header.kicker', 'zh'), '私人秘书');
  });
});

describe('app-shell runtime rails i18n (Telegram banner, offline banner, nightly line)', () => {
  it('localizes the Telegram rail banners with the .env fix in both locales', () => {
    assert.equal(t('telegram.rail.invalidToken.title', 'zh'), 'Telegram 机器人的 token 已失效，手机端通知已停');
    assert.equal(t('telegram.rail.invalidToken.title', 'en'), 'The Telegram bot token is invalid; phone notifications have stopped');
    assert.match(t('telegram.rail.invalidToken.body', 'zh'), /\.env/);
    assert.match(t('telegram.rail.invalidToken.body', 'zh'), /TELOXIDE_TOKEN/);
    assert.match(t('telegram.rail.invalidToken.body', 'en'), /TELOXIDE_TOKEN in \.env/);
    assert.equal(tf('telegram.rail.unreachable.body', 'zh', { minutes: '12' }), '已持续 12 分钟，安琪会自动重试。');
    assert.equal(tf('telegram.rail.unreachable.body', 'en', { minutes: '12' }), 'For 12 minutes now; Aegis keeps retrying.');
    assert.equal(t('telegram.rail.dismiss', 'zh'), '忽略');
    assert.equal(t('telegram.rail.dismiss', 'en'), 'Dismiss');
  });

  it('localizes the offline banner', () => {
    assert.equal(t('app.offline.banner', 'zh'), '已离线，等待网络恢复');
    assert.equal(t('app.offline.banner', 'en'), 'Offline. Waiting for the network to come back.');
  });

  it('localizes every Telegram rail state the guard accepts, and the nightly line', () => {
    for (const state of ['unconfigured', 'starting', 'reachable', 'invalid_token', 'unreachable']) {
      assert.ok(CATALOG[`status.telegram.state.${state}`], `missing status.telegram.state.${state}`);
      assert.doesNotMatch(t(`status.telegram.state.${state}`, 'zh'), /MISSING/);
    }
    assert.equal(tf('status.nightly.red', 'zh', { verdict: 'RED', sha: '02c0d462', hours: '3' }), '夜间测试 RED（02c0d462，3 小时前）');
    assert.equal(tf('status.nightly.red', 'en', { verdict: 'RED', sha: '02c0d462', hours: '3' }), 'Nightly test run RED (02c0d462, 3 hours ago)');
  });
});
