import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_PRESENTATION_MODULE
  ?? 'file:///tmp/aegis-runtime-webapp-vnext-presentation-test/presentation.js';
const { assistantResultTitle, isActionSuccessMessage, itemMeta, ownerTimeLabel, ownerUtilityMenuItemIds, isOwnerUtilityMenuId, ownerShellChromeClass } = await import(modulePath);

describe('ownerTimeLabel timezone handling', () => {
  it('renders an instant in the user timezone and treats SQLite no-offset as UTC', () => {
    // Both forms denote the same UTC instant; must render Jan 16 in Shanghai
    // regardless of process TZ (suite also runs under TZ=America/New_York).
    assert.match(ownerTimeLabel('2026-01-15T20:30:00Z', 'Asia/Shanghai'), /Jan 16/);
    assert.match(ownerTimeLabel('2026-01-15 20:30:00', 'Asia/Shanghai'), /Jan 16/);
  });

  it('passes a date-only value through unchanged (no day-shift)', () => {
    assert.equal(ownerTimeLabel('2026-05-15', 'America/New_York'), '2026-05-15');
  });
});

function result(responseKind, visibleAnswer = 'Done.') {
  return {
    response_kind: responseKind,
    visible_answer: visibleAnswer,
  };
}

describe('vNext presentation helpers', () => {
  it('keeps work item metadata owner-facing and avoids repeated machine status', () => {
    assert.equal(itemMeta({ kind: 'task', urgency: 'high', status: 'active' }), 'High priority · Task');
    assert.equal(itemMeta({ kind: 'task', urgency: 'normal', status: 'waiting' }), 'Task');
    assert.equal(itemMeta({ kind: 'email', urgency: 'normal', status: 'unread' }), 'Email · Unread');
    assert.equal(itemMeta({ kind: 'mail', urgency: 'high', status: 'archived' }), 'Mail · Archived');
    assert.equal(itemMeta({ kind: 'task', urgency: 'high', status: 'completed' }), 'Task · Completed');
    assert.equal(itemMeta({ kind: 'action_draft', urgency: 'normal', status: 'blocked_by_gate' }), 'Draft');
  });

  it('uses owner-facing assistant titles for common runtime outcomes', () => {
    assert.equal(assistantResultTitle(result('working')), 'Angie is working');
    assert.equal(assistantResultTitle(result('draft')), 'Draft ready');
    assert.equal(assistantResultTitle(result('explanation')), 'Answer');
    assert.equal(assistantResultTitle(result('request_clarification')), 'One detail needed');
    assert.equal(assistantResultTitle(result('action_proposal')), 'Review proposal');
    assert.equal(assistantResultTitle(result('orchestrated_response')), 'Answer');
    assert.equal(assistantResultTitle(result('action_receipt')), 'Done');
    assert.equal(assistantResultTitle(result('answer')), 'Answer');
  });

  it('keeps error titles specific without leaking raw failure language', () => {
    assert.equal(
      assistantResultTitle(result('error', "I couldn't finish that: I did not get a complete answer.")),
      'Aegis needs another try',
    );
    assert.equal(
      assistantResultTitle(result('error', "I couldn't finish that: temporarily unavailable. No action was taken.")),
      'Aegis needs another try',
    );
    assert.equal(
      assistantResultTitle(result('error', 'Model provider credits are exhausted.')),
      'Model credits needed',
    );
    assert.equal(
      assistantResultTitle(result('error', 'Model credits are exhausted.')),
      'Model credits needed',
    );
    assert.equal(assistantResultTitle(result('error', 'Something failed.')), "I couldn't finish that");
  });

  it('localizes assistant titles into the display_language (no English leak in zh)', () => {
    // Owner complaint: the chat header "I couldn't finish that" leaked English
    // into a zh session. Every title must resolve through the catalog.
    assert.equal(assistantResultTitle(result('working'), 'zh'), '安琪正在处理');
    assert.equal(assistantResultTitle(result('draft'), 'zh'), '草稿已就绪');
    assert.equal(assistantResultTitle(result('answer'), 'zh'), '回答');
    assert.equal(assistantResultTitle(result('request_clarification'), 'zh'), '还需要一个细节');
    assert.equal(assistantResultTitle(result('action_proposal'), 'zh'), '查看提案');
    assert.equal(assistantResultTitle(result('action_receipt'), 'zh'), '已完成');
    assert.equal(assistantResultTitle(result('error', 'Something failed.'), 'zh'), '我没能完成这件事');
    assert.equal(
      assistantResultTitle(result('error', 'I did not get a complete answer.'), 'zh'),
      '安琪需要再试一次',
    );
    assert.equal(
      assistantResultTitle(result('error', 'Model credits are exhausted.'), 'zh'),
      '需要模型额度',
    );
    // None of the English title phrases should survive in the zh renders.
    const englishTitlePhrases = /Draft ready|One detail needed|Review proposal|couldn't finish|needs another try|Model credits needed|Model setting needs attention/;
    for (const kind of ['working', 'draft', 'request_clarification', 'action_proposal', 'action_receipt']) {
      assert.doesNotMatch(assistantResultTitle(result(kind), 'zh'), englishTitlePhrases);
    }
    assert.doesNotMatch(assistantResultTitle(result('error', 'Something failed.'), 'zh'), englishTitlePhrases);
  });

  it('recognizes real receipt-backed action messages as success', () => {
    assert.equal(isActionSuccessMessage('Archived. Aegis recorded the receipt.'), true);
    assert.equal(isActionSuccessMessage('Approval decision sent through the existing approval path.'), true);
    assert.equal(isActionSuccessMessage('Task updated. Aegis recorded the receipt.'), true);
    assert.equal(isActionSuccessMessage('Action failed: refresh Google first'), false);
  });
});


describe('owner shell utility menu', () => {
  it('exposes only everyday status/settings peers (no dev)', () => {
    const ids = ownerUtilityMenuItemIds();
    assert.deepEqual(ids, ['status', 'settings']);
    assert.equal(isOwnerUtilityMenuId('status'), true);
    assert.equal(isOwnerUtilityMenuId('settings'), true);
    assert.equal(isOwnerUtilityMenuId('dev'), false);
    assert.equal(isOwnerUtilityMenuId('snapshot'), false);
    assert.match(ownerShellChromeClass('header'), /vnext-header/);
    assert.match(ownerShellChromeClass('chat-topbar'), /vnext-chat-topbar/);
    assert.match(ownerShellChromeClass('header'), /vnext-owner-shell-chrome/);
  });
});
