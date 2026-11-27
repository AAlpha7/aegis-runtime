import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const progressPath = process.env.VNEXT_WORKING_PROGRESS_MODULE;
const jevPath = process.env.VNEXT_JEV_PROVENANCE_MODULE;
if (!progressPath) {
  throw new Error('VNEXT_WORKING_PROGRESS_MODULE must point at the compiled workingProgress module');
}
if (!jevPath) {
  throw new Error('VNEXT_JEV_PROVENANCE_MODULE must point at the compiled jevProvenance module');
}

const { appendProgressLine, composeWorkingText, workingPhraseForPhase, workingNotice } = await import(progressPath);
const { parseJevProvenance } = await import(jevPath);

describe('working progress composition', () => {
  it('accumulates live lines and keeps the page-open hint as a suffix', () => {
    const lines = appendProgressLine([], '正在调用 Grok Build…');
    const next = appendProgressLine(lines, 'Jev：允许，正在执行…');
    const keepOpen = '仍在处理。请保持此页面打开，结果会显示在这里。';
    const text = composeWorkingText(next, true, keepOpen, '正在处理…');
    assert.match(text, /正在调用 Grok Build/);
    assert.match(text, /Jev：允许/);
    assert.match(text, /保持此页面打开/);
    assert.ok(text.indexOf('正在调用 Grok Build') < text.indexOf('保持此页面打开'));
  });

  it('does not replace live phases with only the keep-open line', () => {
    const lines = appendProgressLine([], '正在调用 Codex…');
    const keepOpen = '仍在处理。请保持此页面打开，结果会显示在这里。';
    const text = composeWorkingText(lines, true, keepOpen, keepOpen);
    assert.notEqual(text, keepOpen);
    assert.match(text, /正在调用 Codex/);
  });

  it('maps action phases to owner-facing Chinese', () => {
    assert.equal(workingPhraseForPhase('jev_ask'), 'Jev：需要确认，正在等待你批准…');
    assert.equal(workingPhraseForPhase('jev_allow'), 'Jev：允许，正在执行…');
    assert.equal(workingPhraseForPhase('calling_executor', null, 'calling Grok Build'), '正在调用 Grok Build…');
    assert.equal(
      workingPhraseForPhase('calling_executor', 'run_external', 'Grok Build failed: spawn failed: grok missing; calling Codex'),
      'Grok Build 失败，改为调用 Codex…',
    );
    assert.equal(workingPhraseForPhase('executing', 'google_drive_upload'), '正在上传到 Drive…');
    assert.equal(workingPhraseForPhase('tool', 'web_search'), '正在搜索…');
    assert.equal(workingPhraseForPhase('executing', 'not_a_real_tool'), '正在处理…');
  });
});

describe('localized progress and disclosure', () => {
  it('keeps English phases English even when server details are Chinese', () => {
    for (const phase of ['calling_executor', 'tool', 'jev_allow', 'jev_ask', 'jev_deny', 'executing', 'done', 'approval_required', 'unknown']) {
      assert.doesNotMatch(workingPhraseForPhase(phase, 'web_search', '正在调用工具', 'en'), /[\u4e00-\u9fff]/);
    }
    assert.equal(workingPhraseForPhase('tool', 'web_search', null, 'en'), 'Searching…');
    assert.equal(workingPhraseForPhase('calling_executor', null, 'Grok Build failed: missing binary; calling Codex', 'en'), 'Grok Build failed. Switching to Codex…');
    assert.doesNotMatch(workingPhraseForPhase('approval_required', null, null, 'en'), /Jev/);
  });

  it('localizes protocol notices while retaining important owner disclosures', () => {
    assert.equal(workingNotice('[Executor Codex] Read file (in_progress)', 'system', 'zh'), 'Codex · 正在执行');
    assert.equal(workingNotice('[Executor Codex] 读取文件 (completed)', 'system', 'en'), 'Codex · Completed');
    assert.equal(workingNotice('[Executor Codex] Read file (error)', 'system', 'zh'), 'Codex · 执行失败');
    assert.equal(workingNotice('[Executor Codex] Read file (future_status)', 'system', 'en'), 'Codex · Status updated');
    assert.match(workingNotice('⚠️ 主模型 primary 不可用，已切换至备用模型 backup', 'model_fallback', 'en'), /primary.*backup/);
    const notice = 'External search failed; no paid fallback was authorized.';
    assert.equal(workingNotice(notice, 'system', 'zh'), notice, 'unknown notices must not lose cost/failure disclosure');
  });
});

describe('jev provenance parse', () => {
  it('extracts allow fields and never invents a verdict', () => {
    const jev = parseJevProvenance({
      verdict: 'allow',
      audit_tip: 'tip-abc',
      risk: 'high_risk=0.10',
      approval_id: 'jev-ask-skip',
    });
    assert.equal(jev?.verdict, 'allow');
    assert.equal(jev?.audit_tip, 'tip-abc');
    assert.equal(jev?.risk, 'high_risk=0.10');
    assert.equal(parseJevProvenance({ status: 'ok', title: 'remind me' }), undefined);
    assert.equal(parseJevProvenance({ verdict: 'maybe' }), undefined);
    assert.equal(parseJevProvenance(null), undefined);
  });

  it('keeps owner approved/rejected on ask', () => {
    const approved = parseJevProvenance({ verdict: 'ask', owner_decision: 'approved', approval_id: 'cu-ask-1' });
    assert.equal(approved?.owner_decision, 'approved');
    assert.equal(approved?.approval_id, 'cu-ask-1');
    const rejected = parseJevProvenance({ verdict: 'ask', owner_decision: 'rejected' });
    assert.equal(rejected?.owner_decision, 'rejected');
  });
});
