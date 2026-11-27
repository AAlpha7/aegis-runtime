import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_DETAIL_MODULE
  ?? 'file:///tmp/aegis-runtime-webapp-vnext-detail-test/vnext/detail.js';
const { buildSourceDetailView, taskDetailClosedStatus } = await import(modulePath);

describe('vNext source detail formatter', () => {
  it('renders leftover mail body payloads through the generic source path', () => {
    const view = buildSourceDetailView({
      status: 'ok',
      resource_type: 'gmail_message_body',
      detail: {
        sender: 'Law Office <law@example.com>',
        subject: 'Retainer receipt',
        read_state: 'unread',
        mailbox_state: 'inbox',
        body: 'Payment received. No reply needed.\n[HIDDEN CONTENT STRIPPED — email may contain manipulation attempts]',
        body_html_sanitized: '<p><strong>Payment received.</strong></p>',
        attachments: [
          {
            filename: 'statement.pdf',
            mime_type: 'application/pdf',
            size_bytes: 1536,
            attachment_id_present: true,
            attachment_id: 'hidden-id',
          },
        ],
        hidden_content_stripped: true,
        remote_images_proxied: true,
      },
    }, 'en');

    assert.equal(view.kind, 'generic');
    assert.doesNotMatch(JSON.stringify(view), /hidden-id/);
  });

  it('treats leftover IMAP mail_message_body payloads as generic source detail', () => {
    // Provider-honest backend envelope for IMAP reads (mirrors the serialized
    // shape asserted by webapp_vnext_mail_message_imap_maps_proxy_body_to_mail_body_detail).
    const view = buildSourceDetailView({
      status: 'ok',
      resource_type: 'mail_message_body',
      schema_version: 1,
      account: 'imapacct',
      provider: 'imap',
      actions: false,
      detail: {
        status: 'renderable',
        account: 'imapacct',
        sender: 'carol@imap.example',
        subject: 'imap middle',
        message_id: '7',
        read_state: 'unknown',
        mailbox_state: 'inbox',
        body_status: 'complete',
        html_status: 'not_present',
        attachments_status: 'none',
        attachments: [],
        body: 'sanitized plain text',
        body_truncated: false,
        body_warning: 'IMAP body is sanitized plain text (HTML stripped by the proxy); attachment metadata is not available over this transport.',
        body_html_warning: 'No HTML body was present.',
        hidden_content_stripped: false,
        remote_images_proxied: false,
      },
    }, 'en');

    assert.equal(view.kind, 'generic');
  });

  it('keeps long opened email bodies instead of clipping decision-critical detail', () => {
    const longBody = `${'Payment line.\n'.repeat(700)}Final instruction: no reply needed.`;
    const view = buildSourceDetailView({
      status: 'ok',
      resource_type: 'gmail_message_body',
      detail: {
        sender: 'Billing <billing@example.com>',
        subject: 'Long billing notice',
        read_state: 'unread',
        mailbox_state: 'inbox',
        body_status: 'complete',
        body: longBody,
        body_truncated: false,
      },
    }, 'en');

    assert.equal(view.kind, 'generic');
  });

  it('treats leftover truncated body payloads as generic source detail', () => {
    const view = buildSourceDetailView({
      status: 'ok',
      resource_type: 'gmail_message_body',
      detail: {
        sender: 'Billing <billing@example.com>',
        subject: 'Huge billing notice',
        read_state: 'unread',
        mailbox_state: 'inbox',
        body_status: 'complete',
        body: 'Visible body',
        body_truncated: true,
      },
    }, 'en');

    assert.equal(view.kind, 'generic');
  });

  it('formats locked details as a privacy-preserving gate state', () => {
    const view = buildSourceDetailView({
      status: 'mfa_required',
      reason: 'data_access_gate mfa_required provider_request_id=req_123',
      detail: { message: 'Continue with Google before Aegis reads this selected email.' },
    }, 'en');

    assert.equal(view.kind, 'locked');
    assert.equal(view.body, undefined);
    assert.match(view.subtitle, /Continue with Google/);
    assert.equal(
      view.fields.find((field) => field.label === 'Reason')?.value,
      'Extra confirmation is required before Aegis can open this detail.',
    );
    assert.match(view.warning ?? '', /No private body/);
    assert.doesNotMatch(JSON.stringify(view), /data_access_gate|mfa_required|provider_request_id|req_123/i);
  });

  it('formats Google permission gates without leaking raw consent internals', () => {
    const view = buildSourceDetailView({
      status: 'consent_required',
      reason: 'calendar_reader consent_required',
      detail: { message: 'Refresh with Google before opening this calendar detail.' },
    }, 'en');

    assert.equal(view.kind, 'locked');
    assert.equal(
      view.fields.find((field) => field.label === 'Reason')?.value,
      'Google permission is required before Aegis can open this detail.',
    );
    assert.doesNotMatch(JSON.stringify(view), /calendar_reader|consent_required/i);
  });

  it('formats task timeline as owner-readable fields and events', () => {
    const view = buildSourceDetailView({
      status: 'ok',
      resource_type: 'task_timeline',
      detail: {
        task: {
          title: 'Pay school bill',
          body: 'Due Friday',
          state: 'active',
          priority: 'high',
          due_date: '2026-05-15',
          execution_mode: 'llm_autonomous',
        },
        timeline: [
          { label: 'Task created', status: 'mfa_required', at: '2026-05-09T00:00:00Z' },
        ],
      },
    }, 'en');

    assert.equal(view.kind, 'task');
    assert.equal(view.title, 'Pay school bill');
    assert.equal(view.fields.find((field) => field.label === 'State')?.value, 'Active');
    assert.equal(view.fields.find((field) => field.label === 'Priority')?.value, 'High');
    assert.equal(view.fields.find((field) => field.label === 'Mode')?.value, 'AI Autonomous');
    assert.deepEqual(view.timeline.map((event) => event.label), ['Task created']);
    assert.deepEqual(view.timeline.map((event) => event.status), ['Extra confirmation required']);
    assert.doesNotMatch(JSON.stringify(view), /Llm|Mfa/i);
  });

  it('detects closed task timeline states for stale Work reconciliation', () => {
    assert.equal(taskDetailClosedStatus({
      status: 'ok',
      resource_type: 'task_timeline',
      detail: { task: { state: 'archived' } },
    }), 'archived');
    assert.equal(taskDetailClosedStatus({
      status: 'ok',
      resource_type: 'task_timeline',
      detail: { task: { state: 'active' } },
    }), null);
    assert.equal(taskDetailClosedStatus({
      status: 'ok',
      resource_type: 'gmail_message_body',
      detail: { state: 'archived' },
    }), null);
  });

  it('formats approval review details from safe queue projection fields', () => {
    const view = buildSourceDetailView({
      status: 'ok',
      resource_type: 'approval_review',
      detail: {
        title: 'Approval required',
        summary: 'Aegis needs your approval before changing anything outside the app.',
        status: 'waiting',
        owner_visible_reason: 'Aegis needs your approval before changing anything outside the app.',
        next_safe_action: 'Review the approval, then approve or reject.',
        blocked_by: 'Waiting for the owner decision.',
        receipt_expectation: 'If approved, execution must create an Action Ledger receipt before Aegis can claim completion.',
        requires_mfa: false,
        provider_request_id: 'req-secret',
      },
    }, 'en');

    assert.equal(view.kind, 'approval');
    assert.equal(view.title, 'Approval required');
    assert.equal(view.subtitle, 'Aegis needs your approval before changing anything outside the app.');
    assert.equal(view.fields.find((field) => field.label === 'Decision')?.value, 'Review the approval, then approve or reject.');
    assert.equal(view.fields.find((field) => field.label === 'Confirmation')?.value, 'Your signed-in session is enough');
    assert.match(view.fields.find((field) => field.label === 'Receipt')?.value ?? '', /Action Ledger/);
    assert.match(view.body ?? '', /Waiting for the owner decision/);
    assert.match(view.warning ?? '', /Nothing runs unless you choose Approve/);
    assert.doesNotMatch(JSON.stringify(view), /req-secret|provider_request_id/i);
  });

  it('formats receipts and warns when evidence is missing', () => {
    const view = buildSourceDetailView({
      status: 'ok',
      resource_type: 'action_ledger_entry',
      detail: {
        receipt_status: 'Missing',
        entry: {
          user_visible_summary: 'Archived message',
          action_kind: 'Email update',
          action_status: 'executed',
          created_at: '2026-05-09T00:00:00Z',
        },
      },
    }, 'en');

    assert.equal(view.kind, 'receipt');
    assert.equal(view.title, 'Archived message');
    assert.equal(view.subtitle, 'Email Update');
    assert.equal(view.fields.find((field) => field.label === 'Status')?.value, 'Executed');
    assert.equal(view.fields.find((field) => field.label === 'Receipt')?.value, 'Missing');
    assert.match(view.warning ?? '', /should not claim/);
  });

  it('formats contextual action drafts as reviewable work without execution claims', () => {
    const view = buildSourceDetailView({
      status: 'ok',
      resource_type: 'action_ledger_entry',
      detail: {
        receipt_status: 'Missing',
        entry: {
          action_kind: 'email_draft',
          action_status: 'needs_decision',
          raw_status: 'proposed',
          created_at: '2026-05-09T00:00:00Z',
          proposed_at: '2026-05-09T00:00:00Z',
          metadata_json: JSON.stringify({
            resource_type: 'contextual_action_draft',
            focus_kind: 'email',
            draft_text: 'Hello, I can review the statement tomorrow.',
            execution_enabled: false,
          }),
        },
      },
    }, 'en');

    assert.equal(view.kind, 'action_draft');
    assert.equal(view.title, 'Review Email Draft');
    assert.equal(view.body, 'Hello, I can review the statement tomorrow.');
    assert.equal(view.fields.find((field) => field.label === 'State')?.value, 'Needs Decision');
    assert.equal(view.fields.find((field) => field.label === 'Focus')?.value, 'Email');
    assert.match(view.fields.find((field) => field.label === 'Execution')?.value ?? '', /Blocked/);
    assert.match(view.warning ?? '', /has not sent/);
  });

  it('formats memory review details as owner-readable memory facts', () => {
    const view = buildSourceDetailView({
      status: 'ok',
      resource_type: 'memory_item',
      resource_id: '123',
      grant_id: 'grant-secret',
      detail: {
        status: 'available',
        id: 123,
        did: 'did:aegis:telegram:secret',
        topic: 'learned:lesson',
        fact_summary: 'Prefer concrete, action-ready UI copy over vague system labels.',
        confidence: 0.82,
        is_pinned: false,
        created_at: '2026-03-20T00:00:00Z',
      },
    }, 'en');

    assert.equal(view.kind, 'memory');
    assert.equal(view.title, 'learned:lesson');
    assert.equal(view.subtitle, 'Review whether Aegis should keep relying on this memory.');
    assert.equal(view.body, 'Prefer concrete, action-ready UI copy over vague system labels.');
    assert.equal(view.fields.find((field) => field.label === 'Confidence')?.value, '0.82');
    assert.equal(view.fields.find((field) => field.label === 'Pinned')?.value, 'No');
    assert.match(view.warning ?? '', /does not change/);
    assert.doesNotMatch(JSON.stringify(view), /grant-secret|did:aegis|resource_id/i);
  });

  it('renders memory Created timestamp in the user timezone (no raw UTC Z leak)', () => {
    const payload = {
      status: 'ok',
      resource_type: 'memory_item',
      detail: {
        status: 'available',
        topic: 'learned:lesson',
        fact_summary: 'Body.',
        created_at: '2026-01-15T20:30:00Z',
      },
    };
    const shanghai = buildSourceDetailView(payload, 'en', 'Asia/Shanghai');
    const created = shanghai.fields.find((field) => field.label === 'Created')?.value;
    assert.equal(created, '2026-01-16 04:30 UTC+08:00');
    // The raw RFC3339 wire string must NOT appear anywhere in the view.
    assert.doesNotMatch(JSON.stringify(shanghai), /2026-01-15T20:30:00Z/);

    const newYork = buildSourceDetailView(payload, 'en', 'America/New_York');
    assert.equal(
      newYork.fields.find((field) => field.label === 'Created')?.value,
      '2026-01-15 15:30 UTC-05:00',
    );
  });

  it('parses SQLite-style (no-offset) timestamps as UTC, not browser-local', () => {
    // `2026-01-15 20:30:00` (SQLite datetime('now'), space sep, no offset) is
    // UTC. It must render identically regardless of the process/browser TZ
    // (this suite also runs under TZ=America/New_York to prove it).
    const view = buildSourceDetailView(
      {
        status: 'ok',
        resource_type: 'memory_item',
        detail: { topic: 'T', fact_summary: 'B', created_at: '2026-01-15 20:30:00' },
      },
      'en',
      'Asia/Shanghai',
    );
    assert.equal(view.fields.find((field) => field.label === 'Created')?.value, '2026-01-16 04:30 UTC+08:00');
  });

  it('localizes timestamp-like scalars in a generic source detail', () => {
    const view = buildSourceDetailView(
      {
        status: 'ok',
        resource_type: 'something_unknown',
        created_at: '2026-01-15T20:30:00Z',
        note: 'plain text stays as-is',
      },
      'en',
      'Asia/Shanghai',
    );
    assert.equal(view.kind, 'generic');
    const created = view.fields.find((field) => /created/i.test(field.label))?.value;
    assert.equal(created, '2026-01-16 04:30 UTC+08:00');
    assert.doesNotMatch(JSON.stringify(view), /2026-01-15T20:30:00Z/);
    // Non-timestamp scalar passes through unchanged.
    assert.equal(view.fields.find((field) => /note/i.test(field.label))?.value, 'plain text stays as-is');
  });

  it('localizes an RFC3339 task Due but leaves a date-only Due raw', () => {
    const rfc = buildSourceDetailView(
      {
        status: 'ok',
        resource_type: 'task_timeline',
        detail: { task: { title: 'T', state: 'scheduled', due_date: '2026-01-15T20:30:00Z' }, timeline: [] },
      },
      'en',
      'Asia/Shanghai',
    );
    assert.equal(rfc.fields.find((field) => field.label === 'Due')?.value, '2026-01-16 04:30 UTC+08:00');
    assert.doesNotMatch(JSON.stringify(rfc), /2026-01-15T20:30:00Z/);

    const dateOnly = buildSourceDetailView(
      {
        status: 'ok',
        resource_type: 'task_timeline',
        detail: { task: { title: 'T', state: 'scheduled', due_date: '2026-05-15' }, timeline: [] },
      },
      'en',
      'America/New_York',
    );
    // Date-only must NOT be shifted into the prior day by the timezone.
    assert.equal(dateOnly.fields.find((field) => field.label === 'Due')?.value, '2026-05-15');
  });

  it('renders receipt Created/Executed timestamps in the user timezone', () => {
    const view = buildSourceDetailView(
      {
        status: 'ok',
        resource_type: 'action_ledger_entry',
        detail: {
          receipt_status: 'Present',
          entry: {
            action_kind: 'send_email',
            action_status: 'executed',
            external_receipt: 'msg-123',
            user_visible_summary: 'Sent the reply',
            created_at: '2026-01-15T20:30:00Z',
            executed_at: '2026-01-15T20:31:00Z',
          },
        },
      },
      'en',
      'Asia/Shanghai',
    );
    assert.equal(view.fields.find((field) => field.label === 'Created')?.value, '2026-01-16 04:30 UTC+08:00');
    assert.equal(view.fields.find((field) => field.label === 'Executed')?.value, '2026-01-16 04:31 UTC+08:00');
    assert.doesNotMatch(JSON.stringify(view), /2026-01-15T20:3[01]:00Z/);
  });

  it('formats generic scalar fields without leaking raw ids or internal metadata', () => {
    const view = buildSourceDetailView({
      status: 'ok',
      resource_type: 'custom_source',
      provider_request_id: 'req_123',
      trace_id: 'trace-secret',
      source_event_id: 'gmail-event-secret',
      schema_version: 1,
      safe_summary: 'No private content.',
    }, 'en');

    assert.deepEqual(view.fields.map((field) => field.label), ['Status', 'Safe Summary']);
    assert.doesNotMatch(JSON.stringify(view), /req_123|trace-secret|gmail-event-secret|custom_source/);
  });

  it('formats generic owner work items as useful safe detail instead of a dead end', () => {
    const view = buildSourceDetailView({
      status: 'ok',
      resource_type: 'owner_work_item',
      source_id: 'raw-source-secret',
      detail: {
        title: 'McCarthy Electric 电话报价脚本',
        summary: 'Waiting on owner phone call feedback.',
        kind: 'waiting_for',
        status: 'tracking',
        urgency: 'high',
        owner_visible_reason: 'Waiting on 用户电话联系McCarthy后反馈报价.',
        next_safe_action: 'Follow up, defer, or ask Aegis to clean up this loop.',
        blocked_by: 'Owner call result',
        receipt_expectation: 'A task update receipt is required before this is marked handled.',
        primary_action: 'Ask Aegis',
        available_actions: ['Still waiting', 'Clean up'],
      },
    }, 'en');

    assert.equal(view.kind, 'generic');
    assert.equal(view.title, 'McCarthy Electric 电话报价脚本');
    assert.equal(view.subtitle, 'Waiting on owner phone call feedback.');
    assert.match(JSON.stringify(view.fields), /Follow up, defer, or ask Aegis/);
    assert.match(JSON.stringify(view.fields), /A task update receipt is required before this is marked handled/);
    assert.doesNotMatch(JSON.stringify(view.fields), /A Task Update Receipt Is Required/);
    assert.match(view.body ?? '', /McCarthy/);
    assert.match(view.warning ?? '', /safe queue context/);
    assert.doesNotMatch(JSON.stringify(view), /raw-source-secret|source_id/);
  });

  it('shows changed source detail inline instead of a dead open error', () => {
    const view = buildSourceDetailView({
      status: 'unavailable',
      resource_type: 'source_unavailable',
      resource_id: 'memory_review:missing',
      detail: {
        status: 'changed',
        title: 'Memory review: learned:lesson',
        message: 'This work item changed or the underlying source is no longer available.',
        next_step: 'Refresh Work, then reopen the item.',
      },
    }, 'en');

    assert.equal(view.kind, 'generic');
    assert.equal(view.title, 'Memory review: learned:lesson');
    assert.match(view.subtitle, /underlying source is no longer available/);
    assert.match(JSON.stringify(view.fields), /Refresh Work/);
    assert.doesNotMatch(JSON.stringify(view), /memory_review:missing/);
  });
});

// Dual-locale (zh + en) assertions for every localized static display vector,
// through the REAL exported builder entry path. en values must stay
// byte-identical to the pre-i18n literals (asserted throughout the suite
// above); zh is the app default locale.
describe('vNext source detail i18n (zh/en dual-locale)', () => {
  it('localizes the locked gate (title, reason, warning) per locale', () => {
    const payload = {
      status: 'mfa_required',
      reason: 'data_access_gate mfa_required provider_request_id=req_123',
      detail: {},
    };
    const zh = buildSourceDetailView(payload, 'zh');
    assert.equal(zh.kind, 'locked');
    assert.equal(zh.title, '详情已锁定');
    assert.equal(zh.subtitle, '请先在已登录会话中打开，安琪才能读取此详情。');
    assert.deepEqual(zh.fields, [{ label: '原因', value: '需要额外确认后，安琪才能打开此详情。' }]);
    assert.equal(zh.warning, '此视图未读取任何私密正文内容。');
    assert.doesNotMatch(JSON.stringify(zh), /data_access_gate|provider_request_id|req_123/i);

    const en = buildSourceDetailView(payload, 'en');
    assert.equal(en.title, 'Detail locked');
    assert.equal(en.warning, 'No private body content was fetched for this view.');

    const consentZh = buildSourceDetailView({ status: 'consent_required', detail: {} }, 'zh');
    assert.equal(consentZh.fields[0].value, '需要 Google 授权后，安琪才能打开此详情。');
  });

  it('localizes leftover generic body payloads per locale', () => {
    const payload = {
      status: 'ok',
      resource_type: 'gmail_message_body',
      detail: {
        sender: 'Billing <billing@example.com>',
        subject: 'Huge billing notice',
        read_state: 'unread',
        mailbox_state: 'inbox',
        body_status: 'complete',
        body: 'Visible body',
        body_truncated: true,
      },
    };
    const zh = buildSourceDetailView(payload, 'zh');
    assert.equal(zh.kind, 'generic');
    const en = buildSourceDetailView(payload, 'en');
    assert.equal(en.kind, 'generic');
  });

  it('localizes generic fallback title and subtitle per locale', () => {
    const zh = buildSourceDetailView({ status: 'ok', resource_type: 'custom_source', safe_summary: 'No private content.' }, 'zh');
    assert.equal(zh.title, '来源详情');
    assert.equal(zh.subtitle, '安琪已打开所选来源。');
    const en = buildSourceDetailView({ status: 'ok', resource_type: 'custom_source', safe_summary: 'No private content.' }, 'en');
    assert.equal(en.title, 'Source detail');
    assert.equal(en.subtitle, 'Aegis opened the selected source.');
  });

  it('localizes owner work item fallbacks, labels, and warning per locale', () => {
    const zh = buildSourceDetailView({ status: 'ok', resource_type: 'owner_work_item', detail: {} }, 'zh');
    assert.equal(zh.title, '工作项');
    assert.equal(zh.subtitle, '请查看此条目并选择下一个安全操作。');
    assert.equal(zh.fields.find((field) => field.label === '下一步')?.value, '询问安琪接下来应该怎么做。');
    assert.equal(zh.warning, '这里只是安全的队列上下文。此视图未读取任何私密来源正文、附件或原始来源 id。');

    const en = buildSourceDetailView({ status: 'ok', resource_type: 'owner_work_item', detail: {} }, 'en');
    assert.equal(en.title, 'Work item');
    assert.equal(en.fields.find((field) => field.label === 'Next')?.value, 'Ask Aegis what should happen next.');
    const bodyZh = buildSourceDetailView({
      status: 'ok',
      resource_type: 'owner_work_item',
      detail: { owner_visible_reason: 'Reason text.' },
    }, 'zh');
    assert.equal(bodyZh.bodyLabel, '为什么重要');
  });

  it('resolves structural codes in owner work item details per locale with prose fallback', () => {
    const detail = {
      title: 'Waiting item',
      summary: 'Waiting on a call.',
      status: 'waiting',
      kind: 'waiting_for',
      urgency: 'normal',
      owner_visible_reason: 'Aegis is keeping this visible because it has a due date.',
      owner_visible_reason_code: 'waiting_due_date_visible',
      next_safe_action: 'Review whether this dated follow-up is still needed, then defer or close it.',
      next_safe_action_code: 'waiting_review_due',
      primary_action: 'Open details',
      primary_action_code: 'open_details',
      available_actions: ['Done', 'Review tomorrow'],
      available_action_codes: ['done', 'review_tomorrow'],
    };
    const zh = buildSourceDetailView({ status: 'ok', resource_type: 'owner_work_item', detail }, 'zh');
    assert.equal(zh.body, '此事设有截止日期，安琪保持它可见。');
    assert.equal(zh.fields.find((field) => field.label === '下一步')?.value, '检查这个有截止日期的跟进是否仍然需要，然后延后或关闭它。');
    assert.match(JSON.stringify(zh.fields), /打开详情/);
    assert.match(JSON.stringify(zh.fields), /完成 · 明天再看/);

    const en = buildSourceDetailView({ status: 'ok', resource_type: 'owner_work_item', detail }, 'en');
    assert.equal(en.body, 'Aegis is keeping this visible because it has a due date.');
    assert.match(JSON.stringify(en.fields), /Done · Review tomorrow/);

    // Unknown codes fall back to the prose (never blank, never MISSING).
    const unknown = buildSourceDetailView({
      status: 'ok',
      resource_type: 'owner_work_item',
      detail: { ...detail, owner_visible_reason_code: 'v9_new_code', next_safe_action_code: 'v9_next', primary_action_code: 'v9_action' },
    }, 'zh');
    assert.equal(unknown.body, 'Aegis is keeping this visible because it has a due date.');
    assert.doesNotMatch(JSON.stringify(unknown), /MISSING/);
  });

  it('resolves structural codes in approval details per locale', () => {
    const zh = buildSourceDetailView({
      status: 'ok',
      resource_type: 'approval_review',
      detail: {
        title: 'Approve marking mail read?',
        owner_visible_reason: 'Aegis needs your explicit approval before changing Gmail state.',
        owner_visible_reason_code: 'approval_gmail_state',
        next_safe_action: 'Approve only if these are the messages you meant to mark read; otherwise reject.',
        next_safe_action_code: 'approval_gmail_confirm',
      },
    }, 'zh');
    assert.equal(zh.fields.find((field) => field.label === '决定')?.value, 'Approve only if these are the messages you meant to mark read; otherwise reject.');
    assert.equal(zh.body, 'Aegis needs your explicit approval before changing Gmail state.');
  });

  it('localizes the approval title and blocked_by through their own codes', () => {
    // Regression: the detail body used to render an ENGLISH blocked_by
    // paragraph directly above a Chinese reason paragraph, one tap from the
    // decide card. Both must be localized through the code of the sentence
    // they show, and unknown codes must keep the server prose.
    const detail = {
      title: 'Approve marking mail read?',
      title_code: 'approval_title_gmail_read',
      owner_visible_reason: 'Aegis needs your explicit approval before changing Gmail state.',
      owner_visible_reason_code: 'approval_gmail_state',
      blocked_by: 'Waiting for your approve or reject decision.',
      blocked_by_code: 'approval_waiting_decision',
      next_safe_action: 'Approve only if these are the messages you meant to mark read; otherwise reject.',
      next_safe_action_code: 'approval_gmail_confirm',
    };
    const zh = buildSourceDetailView({ status: 'ok', resource_type: 'approval_review', detail }, 'zh');
    assert.equal(zh.title, 'Approve marking mail read?');
    assert.match(zh.body, /正在等待你选择批准或拒绝。/);
    assert.doesNotMatch(zh.body, /Waiting for your approve or reject decision\./);

    const en = buildSourceDetailView({ status: 'ok', resource_type: 'approval_review', detail }, 'en');
    assert.equal(en.title, 'Approve marking mail read?');
    assert.match(en.body, /Waiting for your approve or reject decision\./);

    // Unknown codes keep the server prose verbatim (never blank, never MISSING).
    const unknown = buildSourceDetailView({
      status: 'ok',
      resource_type: 'approval_review',
      detail: { ...detail, title_code: 'v9_title', blocked_by_code: 'v9_blocked' },
    }, 'zh');
    assert.equal(unknown.title, 'Approve marking mail read?');
    assert.match(unknown.body, /Waiting for your approve or reject decision\./);
    assert.doesNotMatch(JSON.stringify(unknown), /MISSING/);
  });

  it('localizes unavailable-source fallbacks via the shared app.sourceDetail keys', () => {
    const payload = { status: 'unavailable', resource_type: 'source_unavailable', detail: {} };
    const zh = buildSourceDetailView(payload, 'zh');
    assert.equal(zh.title, '此详情已变化');
    assert.equal(zh.subtitle, '底层来源已不可用。');
    assert.equal(zh.fields.find((field) => field.label === '下一步')?.value, '刷新「工作」，然后重新打开该条目。');
    assert.equal(zh.warning, '此视图未读取任何私密详情。');

    const en = buildSourceDetailView(payload, 'en');
    assert.equal(en.title, 'This detail changed');
    assert.equal(en.subtitle, 'The underlying source is no longer available.');
    assert.equal(en.fields.find((field) => field.label === 'Next')?.value, 'Refresh Work, then reopen the item.');
  });

  it('localizes memory review subtitle, boolean values, sentinels, and warning', () => {
    const payload = {
      status: 'ok',
      resource_type: 'memory_item',
      detail: { topic: 'learned:lesson', fact_summary: 'Fact.', is_pinned: false },
    };
    const zh = buildSourceDetailView(payload, 'zh');
    assert.equal(zh.subtitle, '请确认安琪是否应继续依赖这条记忆。');
    assert.equal(zh.fields.find((field) => field.label === '已固定')?.value, '否');
    assert.equal(zh.fields.find((field) => field.label === '创建时间')?.value, '未知');
    assert.equal(zh.bodyLabel, '记忆');
    assert.equal(zh.warning, '仅供查看。打开此详情不会更改、固定、删除或修正这条记忆。');

    const en = buildSourceDetailView(payload, 'en');
    assert.equal(en.fields.find((field) => field.label === 'Pinned')?.value, 'No');
    assert.equal(en.fields.find((field) => field.label === 'Created')?.value, 'Unknown');

    const pinnedZh = buildSourceDetailView({
      status: 'ok',
      resource_type: 'memory_item',
      detail: { topic: 't', fact_summary: 'F.', is_pinned: true },
    }, 'zh');
    assert.equal(pinnedZh.fields.find((field) => field.label === '已固定')?.value, '是');
  });

  it('localizes approval fallbacks, confirmation values, and warning', () => {
    const zh = buildSourceDetailView({ status: 'ok', resource_type: 'approval_review', detail: { requires_mfa: true } }, 'zh');
    assert.equal(zh.title, '需要批准');
    assert.equal(zh.subtitle, '安琪需要你的决定才能继续。');
    assert.equal(zh.fields.find((field) => field.label === '决定')?.value, '请批准或拒绝此请求。');
    assert.equal(zh.fields.find((field) => field.label === '确认')?.value, '可能需要额外确认');
    assert.equal(zh.fields.find((field) => field.label === '记录')?.value, '安琪必须拿到记录后才能声称已完成。');
    assert.equal(zh.warning, '除非你选择「批准」，否则什么都不会执行。「拒绝」会停止此批准请求。');

    const en = buildSourceDetailView({ status: 'ok', resource_type: 'approval_review', detail: {} }, 'en');
    assert.equal(en.title, 'Approval required');
    assert.equal(en.fields.find((field) => field.label === 'Confirmation')?.value, 'Your signed-in session is enough');

    const bodyZh = buildSourceDetailView({
      status: 'ok',
      resource_type: 'approval_review',
      detail: { blocked_by: 'Waiting.' },
    }, 'zh');
    assert.equal(bodyZh.bodyLabel, '为什么在等待');
  });

  it('localizes task labels behind stable ids and flags the due placeholder', () => {
    const payload = {
      status: 'ok',
      resource_type: 'task_timeline',
      detail: { task: { title: 'T', state: 'active' }, timeline: [{}] },
    };
    const zh = buildSourceDetailView(payload, 'zh');
    // Stable machine ids are locale-invariant; labels localize.
    assert.deepEqual(zh.fields.map((field) => field.id), ['state', 'priority', 'due', 'mode']);
    assert.deepEqual(zh.fields.map((field) => field.label), ['状态', '优先级', '截止', '模式']);
    const dueZh = zh.fields.find((field) => field.id === 'due');
    assert.equal(dueZh.value, '未设置');
    assert.equal(dueZh.placeholder, true);
    // Timeline entry without label/kind falls back to the localized Event.
    assert.equal(zh.timeline[0].label, '事件');

    const en = buildSourceDetailView(payload, 'en');
    assert.deepEqual(en.fields.map((field) => field.id), ['state', 'priority', 'due', 'mode']);
    const dueEn = en.fields.find((field) => field.id === 'due');
    assert.equal(dueEn.value, 'Not set');
    assert.equal(dueEn.placeholder, true);
    assert.equal(en.timeline[0].label, 'Event');

    // A real due is NOT flagged as placeholder in either locale.
    const withDue = buildSourceDetailView({
      status: 'ok',
      resource_type: 'task_timeline',
      detail: { task: { title: 'T', state: 'active', due_date: '2026-05-15' }, timeline: [] },
    }, 'zh');
    const realDue = withDue.fields.find((field) => field.id === 'due');
    assert.equal(realDue.value, '2026-05-15');
    assert.equal(realDue.placeholder, undefined);
    assert.equal(withDue.subtitle, '任务详情');
    assert.equal(withDue.title, 'T');
  });

  it('localizes receipt labels, presence sentinels, and missing-evidence warning', () => {
    const missingZh = buildSourceDetailView({
      status: 'ok',
      resource_type: 'action_ledger_entry',
      detail: { entry: { user_visible_summary: 'Archived message', action_kind: 'email_update', action_status: 'executed' } },
    }, 'zh');
    assert.equal(missingZh.fields.find((field) => field.label === '记录')?.value, '缺失');
    assert.equal(missingZh.fields.find((field) => field.label === '创建时间')?.value, '未知');
    assert.equal(missingZh.fields.find((field) => field.label === '执行时间')?.value, '未执行');
    assert.equal(missingZh.warning, '在没有记录证据的情况下，安琪不应声称此操作已完成。');

    const missingEn = buildSourceDetailView({
      status: 'ok',
      resource_type: 'action_ledger_entry',
      detail: { entry: { user_visible_summary: 'Archived message', action_kind: 'email_update', action_status: 'executed' } },
    }, 'en');
    assert.equal(missingEn.fields.find((field) => field.label === 'Receipt')?.value, 'Missing');
    assert.equal(missingEn.fields.find((field) => field.label === 'Created')?.value, 'unknown');
    assert.equal(missingEn.fields.find((field) => field.label === 'Executed')?.value, 'Not executed');

    const presentZh = buildSourceDetailView({
      status: 'ok',
      resource_type: 'action_ledger_entry',
      detail: { entry: { action_kind: 'send_email', external_receipt: 'msg-123' } },
    }, 'zh');
    assert.equal(presentZh.fields.find((field) => field.label === '记录')?.value, '已提供');
    assert.equal(presentZh.bodyLabel, '记录证据');
    assert.equal(presentZh.warning, undefined);

    const fallbackTitleZh = buildSourceDetailView({
      status: 'ok',
      resource_type: 'action_ledger_entry',
      detail: { entry: {} },
    }, 'zh');
    assert.equal(fallbackTitleZh.title, '操作记录');
  });

  it('localizes action draft static strings while the enum-coupled title stays English (deferred)', () => {
    const payload = {
      status: 'ok',
      resource_type: 'action_ledger_entry',
      detail: {
        entry: {
          action_kind: 'email_draft',
          action_status: 'needs_decision',
          proposed_at: '2026-05-09T00:00:00Z',
          metadata_json: JSON.stringify({
            resource_type: 'contextual_action_draft',
            focus_kind: 'email',
            draft_text: 'Hello.',
            execution_enabled: false,
          }),
        },
      },
    };
    const zh = buildSourceDetailView(payload, 'zh');
    assert.equal(zh.subtitle, '已保存待审阅。尚未执行任何操作。');
    assert.equal(zh.fields.find((field) => field.label === '记录')?.value, '执行前暂不适用');
    assert.equal(zh.fields.find((field) => field.label === '执行')?.value, '在批准并处理记录之前不会执行');
    assert.equal(zh.bodyLabel, '草稿');
    assert.deepEqual(zh.timeline.map((event) => [event.label, event.status])[0], ['草稿已保存', '需要审阅']);
    assert.equal(zh.warning, '仅供审阅。安琪尚未为此草稿发送、更改、删除或完成任何内容。');
    // ownerLabel(action_kind) renders a wire enum — the Review title is
    // deliberately deferred to the enum-map ticket and stays English.
    assert.equal(zh.title, 'Review Email Draft');

    const en = buildSourceDetailView(payload, 'en');
    assert.equal(en.subtitle, 'Saved for review. Nothing has been executed.');
    assert.equal(en.fields.find((field) => field.label === 'Execution')?.value, 'Blocked until approval and receipt handling');
    assert.deepEqual(en.timeline.map((event) => [event.label, event.status])[0], ['Draft saved', 'review needed']);
  });
});
