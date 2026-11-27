/**
 * Human working-bubble phrases for live tool progress.
 * Maps backend tool names to owner-facing copy; unknown tools get a generic line.
 */

type ProgressLocale = 'zh' | 'en';

const TOOL_WORKING_PHRASES: Record<string, string> = {
  calendar_reader: '正在查日历…',
  web_search: '正在搜索…',
  web_reader: '正在读网页…',
  recall_memory: '正在回忆…',
  knowledge_base: '正在查资料…',
  manage_tasks: '正在处理待办…',
  schedule_intent: '正在安排提醒…',
  run_external: '正在调用 Grok Build / Codex…',
  run_external_grok: '正在调用 Grok Build…',
  'run_external:grok': '正在调用 Grok Build…',
  grok: '正在调用 Grok Build…',
  grok_build: '正在调用 Grok Build…',
  'grok build': '正在调用 Grok Build…',
  run_external_codex: '正在调用 Codex…',
  'run_external:codex': '正在调用 Codex…',
  codex: '正在调用 Codex…',
  delegate_task: '已交给 Hercules…',
  secure_computer_use: '正在操作电脑…',
  google_drive_upload: '正在上传到 Drive…',
};

const EN_TOOL_WORKING_PHRASES: Record<string, string> = {
  calendar_reader: 'Checking your calendar…',
  web_search: 'Searching…',
  web_reader: 'Reading the page…',
  recall_memory: 'Recalling…',
  knowledge_base: 'Looking up information…',
  manage_tasks: 'Updating tasks…',
  schedule_intent: 'Scheduling a reminder…',
  run_external: 'Calling Grok Build / Codex…',
  run_external_grok: 'Calling Grok Build…',
  'run_external:grok': 'Calling Grok Build…',
  grok: 'Calling Grok Build…',
  grok_build: 'Calling Grok Build…',
  'grok build': 'Calling Grok Build…',
  run_external_codex: 'Calling Codex…',
  'run_external:codex': 'Calling Codex…',
  codex: 'Calling Codex…',
  delegate_task: 'Delegating to Hercules…',
  secure_computer_use: 'Operating the computer…',
  google_drive_upload: 'Uploading to Drive…',
};

export function toolDonePhrase(locale: ProgressLocale = 'zh'): string {
  return locale === 'en' ? 'Preparing the reply…' : '好了，正在整理回复…';
}

/** Owner-facing working line for a `tool_start` frame. */
export function workingPhraseForTool(toolName: string, locale: ProgressLocale = 'zh'): string {
  const key = (toolName || '').trim().toLowerCase();
  return locale === 'en'
    ? EN_TOOL_WORKING_PHRASES[key] ?? 'Working…'
    : TOOL_WORKING_PHRASES[key] ?? '正在处理…';
}

/**
 * Quiet reply-footer executor label for turns that used an executor tool.
 * Only returns a name the stream actually implied via tool_name — never invents
 * grok/codex when the client only saw `run_external`.
 */
export function executorFooterLabel(toolName: string, locale: ProgressLocale = 'zh'): string | null {
  const key = (toolName || '').trim().toLowerCase();
  if (key === 'delegate_task') return 'Hercules';
  if (key === 'run_external') return locale === 'en' ? 'Executor' : '执行者';
  // Optional future: stream may carry "run_external:grok" / "run_external:codex".
  if (key === 'run_external:grok' || key === 'grok' || key === 'grok_build' || key === 'grok build') {
    return 'grok build';
  }
  if (key === 'run_external:codex' || key === 'codex') return 'codex';
  return null;
}

/** Format "经 {name} · {n} 秒" for an assistant reply footer. */
export function formatExecutorFooter(executorName: string, elapsedMs: number, locale: ProgressLocale = 'zh'): string {
  const secs = Math.max(1, Math.round(Math.max(0, elapsedMs) / 1000));
  return locale === 'en' ? `Via ${executorName} · ${secs}s` : `经 ${executorName} · ${secs} 秒`;
}
