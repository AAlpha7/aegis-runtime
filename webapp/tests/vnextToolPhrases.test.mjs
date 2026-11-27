import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_TOOL_PHRASES_MODULE;
if (!modulePath) {
  throw new Error('VNEXT_TOOL_PHRASES_MODULE must point at the compiled toolPhrases module');
}

const {
  workingPhraseForTool,
  toolDonePhrase,
  executorFooterLabel,
  formatExecutorFooter,
} = await import(modulePath);

describe('vNext tool phrases', () => {
  it('maps calendar_reader to a human phrase', () => {
    assert.equal(workingPhraseForTool('calendar_reader'), '正在查日历…');
  });

  it('maps known tools and falls back for unknown ones', () => {
    assert.equal(workingPhraseForTool('web_search'), '正在搜索…');
    assert.equal(workingPhraseForTool('web_reader'), '正在读网页…');
    assert.equal(workingPhraseForTool('recall_memory'), '正在回忆…');
    assert.equal(workingPhraseForTool('knowledge_base'), '正在查资料…');
    assert.equal(workingPhraseForTool('manage_tasks'), '正在处理待办…');
    assert.equal(workingPhraseForTool('schedule_intent'), '正在安排提醒…');
    assert.equal(workingPhraseForTool('run_external'), '正在调用 Grok Build / Codex…');
    assert.equal(workingPhraseForTool('secure_computer_use'), '正在操作电脑…');
    assert.equal(workingPhraseForTool('google_drive_upload'), '正在上传到 Drive…');
    assert.equal(workingPhraseForTool('delegate_task'), '已交给 Hercules…');
    assert.equal(workingPhraseForTool('some_new_tool'), '正在处理…');
    assert.equal(workingPhraseForTool(''), '正在处理…');
  });

  it('uses a single tool_done phrase', () => {
    assert.equal(toolDonePhrase('zh'), '好了，正在整理回复…');
    assert.equal(toolDonePhrase('en'), 'Preparing the reply…');
  });

  it('labels executor footer from the stream tool only', () => {
    assert.equal(executorFooterLabel('delegate_task'), 'Hercules');
    assert.equal(executorFooterLabel('run_external'), '执行者');
    assert.equal(executorFooterLabel('web_search'), null);
    assert.equal(formatExecutorFooter('执行者', 24500), '经 执行者 · 25 秒');
    assert.equal(formatExecutorFooter('Hercules', 900), '经 Hercules · 1 秒');
  });

  it('uses the selected language for every tool, fallback, done and executor footer', () => {
    for (const tool of ['calendar_reader', 'web_search', 'web_reader', 'recall_memory', 'knowledge_base',
      'manage_tasks', 'schedule_intent', 'run_external', 'run_external:grok', 'run_external:codex',
      'delegate_task', 'secure_computer_use', 'google_drive_upload', 'unknown']) {
      assert.doesNotMatch(workingPhraseForTool(tool, 'en'), /[\u4e00-\u9fff]/);
      assert.match(workingPhraseForTool(tool, 'zh'), /[\u4e00-\u9fff]/);
    }
    assert.equal(workingPhraseForTool('web_search', 'en'), 'Searching…');
    assert.equal(executorFooterLabel('run_external', 'en'), 'Executor');
    assert.equal(formatExecutorFooter('Codex', 24500, 'en'), 'Via Codex · 25s');
    assert.doesNotMatch(workingPhraseForTool('secure_computer_use', 'en'), /Jev/);
  });
});
