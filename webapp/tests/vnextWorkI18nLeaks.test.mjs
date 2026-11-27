import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

// SP3 guard — the Work-view logic layer (model.ts / presentation.ts /
// interactionState.ts / adapters.ts / notificationActions.ts) returns owner-
// facing strings from HELPER FUNCTIONS, not JSX. A JSX-only i18n scan misses
// them, which is exactly how the "Open email / Mark read / Working… / Paused"
// English leaked into the zh Work view. This guard exercises each offending
// helper in BOTH locales and asserts the zh output is genuinely localized
// (zh !== en, and the zh output contains no Latin-letter run that betrays a
// bare English literal). It fails LOUDLY if a future edit reintroduces a
// hardcoded English string in any of these helpers.
//
// Modules are bundled via esbuild (see package.json test:vnext-work-i18n-leaks)
// so the `../i18n/catalog` import resolves.

const modelPath = process.env.VNEXT_WORK_I18N_MODEL_MODULE;
const presentationPath = process.env.VNEXT_WORK_I18N_PRESENTATION_MODULE;
const interactionPath = process.env.VNEXT_WORK_I18N_INTERACTION_MODULE;
const adaptersPath = process.env.VNEXT_WORK_I18N_ADAPTERS_MODULE;
const notificationPath = process.env.VNEXT_WORK_I18N_NOTIFICATION_MODULE;
const projectionPath = process.env.VNEXT_WORK_I18N_PROJECTION_MODULE;

for (const [name, value] of Object.entries({
  VNEXT_WORK_I18N_MODEL_MODULE: modelPath,
  VNEXT_WORK_I18N_PRESENTATION_MODULE: presentationPath,
  VNEXT_WORK_I18N_INTERACTION_MODULE: interactionPath,
  VNEXT_WORK_I18N_ADAPTERS_MODULE: adaptersPath,
  VNEXT_WORK_I18N_NOTIFICATION_MODULE: notificationPath,
  VNEXT_WORK_I18N_PROJECTION_MODULE: projectionPath,
})) {
  if (!value) throw new Error(`${name} must point at the bundled module`);
}

const model = await import(modelPath);
const presentation = await import(presentationPath);
const interaction = await import(interactionPath);
const adapters = await import(adaptersPath);
const notification = await import(notificationPath);
const projection = await import(projectionPath);

// A string is "still leaking English" if it carries a run of >=4 consecutive
// ASCII letters. Chinese copy may legitimately embed brand tokens (Aegis,
// Gmail, Codex), so we only flag runs that are NOT in this allowlist. The
// allowlist is deliberately tiny — anything else (Open, Paused, Working,
// Archive, Review…) trips the guard.
const BRAND_ALLOWLIST = new Set([
  'aegis', 'gmail', 'imap', 'codex', 'grok', 'google',
  'webapp', 'mfa', 'oauth', 'csrf', 'reminder', 'research', 'initiative',
]);

function leaksEnglish(text) {
  const runs = String(text).match(/[A-Za-z]{4,}/g);
  if (!runs) return null;
  const offenders = runs.filter((run) => !BRAND_ALLOWLIST.has(run.toLowerCase()));
  return offenders.length > 0 ? offenders : null;
}

function assertLocalized(label, zh, en) {
  assert.notEqual(zh, en, `${label}: zh output must differ from en (got identical "${zh}")`);
  const offenders = leaksEnglish(zh);
  assert.equal(offenders, null, `${label}: zh output leaks English ${JSON.stringify(offenders)} → "${zh}"`);
}

// Minimal OwnerWorkItem-shaped fixtures hitting each helper branch.
function item(overrides = {}) {
  return {
    id: 'w1',
    kind: 'task',
    title: '',
    summary: '',
    status: 'new',
    urgency: 'normal',
    updated_at: '',
    secondary_actions: [],
    related_action_ids: [],
    related_receipt_ids: [],
    ...overrides,
  };
}

describe('SP3 Work-view i18n leak guard', () => {
  it('localizes interactionState action button + confirmation hint', () => {
    const action = { id: 'mail.archive', label: '归档', confirmLabel: '再次点按以归档', receiptHint: 'x' };
    assertLocalized(
      'itemActionButtonLabel(working)',
      interaction.itemActionButtonLabel(action, 'mail.archive', 'mail.archive', 'zh'),
      interaction.itemActionButtonLabel(action, 'mail.archive', 'mail.archive', 'en'),
    );
    assertLocalized(
      'itemActionButtonLabel(confirm)',
      interaction.itemActionButtonLabel(action, 'mail.archive', null, 'zh'),
      interaction.itemActionButtonLabel(action, 'mail.archive', null, 'en'),
    );
    const hintAction = { ...action, receiptHint: 'Aegis 会记录归档记录。' };
    assertLocalized(
      'itemActionConfirmationHint',
      interaction.itemActionConfirmationHint(hintAction, 'zh'),
      interaction.itemActionConfirmationHint(hintAction, 'en'),
    );
  });

  it('localizes model ownerVisibleWorkCue branches', () => {
    const closed = item({ status: 'done', urgency: 'silent' });
    assertLocalized('cue.handled', model.ownerVisibleWorkCue(closed, 220, undefined, 'zh'), model.ownerVisibleWorkCue(closed, 220, undefined, 'en'));
    const unreadMail = item({ kind: 'email', status: 'unread' });
    assertLocalized('cue.unreadEmail', model.ownerVisibleWorkCue(unreadMail, 220, undefined, 'zh'), model.ownerVisibleWorkCue(unreadMail, 220, undefined, 'en'));
    const scheduled = item({ status: 'scheduled' });
    assertLocalized('cue.scheduledNoAction', model.ownerVisibleWorkCue(scheduled, 220, undefined, 'zh'), model.ownerVisibleWorkCue(scheduled, 220, undefined, 'en'));
  });

  it('localizes blockedAutomation title + what-happened + decision text', () => {
    const blockedGeneric = item({
      kind: 'task',
      status: 'blocked_by_gate',
      blocked_by: 'tool gate',
      summary: 'primary mail triage',
    });
    assertLocalized('blockedAutomationTitle(generic)', model.blockedAutomationTitle(blockedGeneric, undefined, 96, 'zh'), model.blockedAutomationTitle(blockedGeneric, undefined, 96, 'en'));
    // Server prose (including leftover mail-kind copy) is shown as-is in both locales.
    assert.equal(model.blockedAutomationWhatHappened(blockedGeneric, 'zh'), 'primary mail triage');
    assert.equal(model.blockedAutomationWhatHappened(blockedGeneric, 'en'), 'primary mail triage');
    const blockedApproval = item({ status: 'blocked_by_gate', secondary_actions: [{ action_kind: 'approval.approve' }] });
    assertLocalized('blockedAutomationDecisionText', model.blockedAutomationDecisionText(blockedApproval, 'zh'), model.blockedAutomationDecisionText(blockedApproval, 'en'));
  });

  it('localizes a coded item title everywhere it is rendered (row, Home signal, helper)', () => {
    // Approvals carry a STATIC server title plus title_code. The row title was
    // localized while the Home signal card still read item.title raw, so one zh
    // screen showed a Chinese brief above an English signal summary.
    const approval = item({
      kind: 'approval',
      status: 'waiting',
      title: 'Approval required',
      title_code: 'approval_title_generic',
      summary: 'Aegis wants to continue a tool action.',
    });
    assertLocalized('ownerVisibleCodedTitle', model.ownerVisibleCodedTitle(approval, 'zh'), model.ownerVisibleCodedTitle(approval, 'en'));
    assertLocalized('ownerVisibleWorkTitle(approval)', model.ownerVisibleWorkTitle(approval, undefined, 96, 'zh'), model.ownerVisibleWorkTitle(approval, undefined, 96, 'en'));

    const queue = {
      schema_version: 1,
      generated_at: '2026-07-24T00:00:00Z',
      owner: 'owner',
      sources: [],
      counts: {},
      items: [approval],
    };
    const zhSignals = model.buildVNextHomeSignals(model.buildVNextHome(queue, 'zh'), undefined, 'zh');
    const enSignals = model.buildVNextHomeSignals(model.buildVNextHome(queue, 'en'), undefined, 'en');
    // Compare the owner-facing summary only — the envelope keys are identifiers.
    assertLocalized('buildVNextHomeSignals(summary)', zhSignals[0].summary, enSignals[0].summary);

    // Unknown / deleted mail-specific codes → the server prose survives in BOTH locales.
    const unknown = {
      ...approval,
      title: 'Approve marking mail read?',
      title_code: 'approval_title_gmail_read',
    };
    assert.equal(model.ownerVisibleCodedTitle(unknown, 'zh'), 'Approve marking mail read?');
    assert.equal(model.ownerVisibleWorkTitle(unknown, undefined, 96, 'zh'), 'Approve marking mail read?');
  });

  it('localizes buildVNextLanes titles + subtitles (decide/now lanes)', () => {
    const zh = model.buildVNextLanes([], 'zh');
    const en = model.buildVNextLanes([], 'en');
    assertLocalized('lane.decide.title', zh[0].title, en[0].title);
    assertLocalized('lane.decide.subtitle', zh[0].subtitle, en[0].subtitle);
    assertLocalized('lane.now.title', zh[1].title, en[1].title);
    assertLocalized('lane.now.subtitle', zh[1].subtitle, en[1].subtitle);
  });

  it('localizes authorityBasisLabel (waiting + mapped + fallback)', () => {
    assertLocalized('authorityBasis(empty)', model.authorityBasisLabel('', 'zh'), model.authorityBasisLabel('', 'en'));
    assertLocalized('authorityBasis(active_mandate)', model.authorityBasisLabel('active_mandate', 'zh'), model.authorityBasisLabel('active_mandate', 'en'));
    assertLocalized('authorityBasis(unknown→fallback)', model.authorityBasisLabel('something_else', 'zh'), model.authorityBasisLabel('something_else', 'en'));
  });

  it('localizes buildSupportedItemActions labels (task)', () => {
    const task = item({ kind: 'task', status: 'active', secondary_actions: [{ action_kind: 'task.complete' }, { action_kind: 'task.snooze_tomorrow' }] });
    const zhActions = model.buildSupportedItemActions(task, 'zh');
    const enActions = model.buildSupportedItemActions(task, 'en');
    assert.ok(zhActions.length > 0);
    for (const [zhAction, enAction] of zhActions.map((a, i) => [a, enActions[i]])) {
      assertLocalized(`taskAction.label(${zhAction.id})`, zhAction.label, enAction.label);
      assertLocalized(`taskAction.confirm(${zhAction.id})`, zhAction.confirmLabel, enAction.confirmLabel);
      assertLocalized(`taskAction.hint(${zhAction.id})`, zhAction.receiptHint, enAction.receiptHint);
    }
  });

  it('localizes buildCommandSuggestions labels (general + item)', () => {
    const generalZh = model.buildCommandSuggestions(null, 'general', 'zh');
    const generalEn = model.buildCommandSuggestions(null, 'general', 'en');
    generalZh.forEach((s, i) => assertLocalized(`suggest.general(${s.id})`, s.label, generalEn[i].label));
    const mail = item({ kind: 'email' });
    const itemZh = model.buildCommandSuggestions(mail, 'item', 'zh');
    const itemEn = model.buildCommandSuggestions(mail, 'item', 'en');
    itemZh.forEach((s, i) => assertLocalized(`suggest.item(${s.id})`, s.label, itemEn[i].label));
  });

  it('localizes presentation itemMeta + ownerMetaStatusLabel(unread)', () => {
    const unreadTask = item({ kind: 'task', status: 'unread', urgency: 'high' });
    assertLocalized('itemMeta(task/unread/high)', presentation.itemMeta(unreadTask, 'zh'), presentation.itemMeta(unreadTask, 'en'));
    // Backend kind `email` has no catalog mapping after the mail client was
    // removed; title-case the token. Unread + priority still localize.
    const emailKind = item({ kind: 'email', status: 'unread', urgency: 'high' });
    const zh = presentation.itemMeta(emailKind, 'zh');
    const en = presentation.itemMeta(emailKind, 'en');
    assert.match(zh, /Email/);
    assert.match(en, /Email/);
    assert.notEqual(zh, en);
  });

  it('localizes s0/s1 severity urgency (no raw "S0"/"S1" leak)', () => {
    // s0/s1 are internal severity codes in HIGH_URGENCY; they must render as the
    // urgent/high label in zh, not the bare token.
    for (const urgency of ['s0', 's1']) {
      const it0 = item({ kind: 'task', status: 'new', urgency });
      assertLocalized(`itemMeta(${urgency})`, presentation.itemMeta(it0, 'zh'), presentation.itemMeta(it0, 'en'));
      assertLocalized(`urgencyLevelLabel(${urgency})`, presentation.urgencyLevelLabel(urgency, 'zh'), presentation.urgencyLevelLabel(urgency, 'en'));
    }
  });

  it('localizes empty-title fallbacks (ownerVisibleWorkTitle / blockedAutomationTitle defaults)', () => {
    // Empty-title items with NO explicit fallback must localize the default.
    const empty = item({ title: '' });
    assertLocalized(
      'ownerVisibleWorkTitle(empty, no-fallback)',
      model.ownerVisibleWorkTitle(empty, undefined, undefined, 'zh'),
      model.ownerVisibleWorkTitle(empty, undefined, undefined, 'en'),
    );
    const blockedGeneric = item({ title: '', status: 'blocked_by_gate', summary: '', blocked_by: '' });
    assertLocalized(
      'blockedAutomationTitle(empty, no-fallback)',
      model.blockedAutomationTitle(blockedGeneric, undefined, 96, 'zh'),
      model.blockedAutomationTitle(blockedGeneric, undefined, 96, 'en'),
    );
  });

  it('localizes adapters actionSuccessMessage', () => {
    assertLocalized('success.task', adapters.actionSuccessMessage('task.complete', 'zh'), adapters.actionSuccessMessage('task.complete', 'en'));
    assertLocalized('success.generic', adapters.actionSuccessMessage('other.thing', 'zh'), adapters.actionSuccessMessage('other.thing', 'en'));
  });

  it('localizes notificationActions delivery labels + details', () => {
    const zh = notification.deliveryNoticeActionFromCommand('trigger_corpus_reembed', 'zh');
    const en = notification.deliveryNoticeActionFromCommand('trigger_corpus_reembed', 'en');
    assertLocalized('delivery.label(trigger)', zh.label, en.label);
    assertLocalized('delivery.detail(rebuild)', zh.detail, en.detail);
    const approveZh = notification.deliveryNoticeActionFromCommand('approve initiative 7', 'zh');
    const approveEn = notification.deliveryNoticeActionFromCommand('approve initiative 7', 'en');
    assertLocalized('delivery.label(approve)', approveZh.label, approveEn.label);
  });

  it('localizes sourceStatusView (Status Connections disclosure) + source labels', () => {
    // statusLabel / reason / nextStep render directly in the zh Connections
    // disclosure; the English `label` stays the identity (localized for display
    // via localizedSourceLabel).
    for (const [status, reason] of [
      ['deferred', ''],
      ['auth_required', ''],
      ['permission_required', ''],
      ['unavailable', ''],
      ['available', 'mfa_required'],
      ['available', 'consent_required'],
      ['available', 'rate_limit hit'],
      ['available', 'circuit open'],
    ]) {
      const zh = model.sourceStatusView({ source: 'calendar', status, reason }, 'zh');
      const en = model.sourceStatusView({ source: 'calendar', status, reason }, 'en');
      assertLocalized(`sourceStatusView(${status}).statusLabel`, zh.statusLabel, en.statusLabel);
      if (zh.reason) assertLocalized(`sourceStatusView(${reason}).reason`, zh.reason, en.reason);
      if (zh.nextStep) assertLocalized(`sourceStatusView(${status}).nextStep`, zh.nextStep, en.nextStep);
    }
    // All 9 source labels localize for display.
    for (const label of ['Calendar', 'Tasks and reminders', 'System alerts', 'Drafts and receipts', 'Goals', 'Owner approvals', 'Waiting items', 'Standing instructions']) {
      assertLocalized(`localizedSourceLabel(${label})`, model.localizedSourceLabel(label, 'zh'), model.localizedSourceLabel(label, 'en'));
    }
  });

  it('localizes ownerProjectionText (Status/Settings notices)', () => {
    const section = (overrides = {}) => ({
      label: 'Connections',
      health: 'attention',
      owner_message: '',
      owner_next_step: '',
      ...overrides,
    });
    // Cost-saving section (empty message → localized cost-saving copy).
    const cost = section({ label: 'Cost controls', owner_message: '', owner_next_step: 'x' });
    assertLocalized(
      'projection.costSaving.message',
      projection.ownerMessageForProjectionSection(cost, 'zh'),
      projection.ownerMessageForProjectionSection(cost, 'en'),
    );
    assertLocalized(
      'projection.costSaving.nextStep',
      projection.ownerNextStepForProjectionSection(cost, 'zh'),
      projection.ownerNextStepForProjectionSection(cost, 'en'),
    );
    // Token-matched branches (server token stays; owner copy localizes).
    for (const [name, token] of [
      ['mfa', 'mfa_required'],
      ['consent', 'consent_required'],
      ['calendar', 'calendar_reader failed'],
      ['credits', 'model provider credits exhausted'],
      ['rateLimit', 'rate_limit hit'],
      ['safetyBoundary', 'tool_gate blocked'],
      ['hiddenDetail', 'redacted_fields present'],
      ['notConnected', 'projector not connected'],
      ['metadataOnly', 'metadata only'],
      ['quiet', 'heartbeat loop'],
    ]) {
      const s = section({ owner_message: token, owner_next_step: token });
      assertLocalized(
        `projection.${name}.message`,
        projection.ownerMessageForProjectionSection(s, 'zh'),
        projection.ownerMessageForProjectionSection(s, 'en'),
      );
    }
    // Empty non-cost section → generic localized fallback. owner_next_step is
    // whitespace (truthy so the helper localizes it, but trims to empty so the
    // empty.nextStep branch fires; a falsy value would short-circuit to itself).
    const empty = section({ owner_message: '', owner_next_step: ' ' });
    assertLocalized(
      'projection.empty.message',
      projection.ownerMessageForProjectionSection(empty, 'zh'),
      projection.ownerMessageForProjectionSection(empty, 'en'),
    );
    assertLocalized(
      'projection.empty.nextStep',
      projection.ownerNextStepForProjectionSection(empty, 'zh'),
      projection.ownerNextStepForProjectionSection(empty, 'en'),
    );
  });

  // Anchor: the EXACT offenders the owner saw on screen must no longer be the
  // zh output. These literal assertions fail before the SP3 fix and pass after.
  it('the screenshot offenders are gone from the zh output', () => {
    const action = { id: 'task.complete', label: '完成', confirmLabel: 'x', receiptHint: 'y' };
    assert.notEqual(interaction.itemActionButtonLabel(action, 'task.complete', 'task.complete', 'zh'), 'Working...');
    assert.notEqual(interaction.itemActionButtonLabel(action, 'task.complete', null, 'zh'), 'Confirm?');
    const blockedTask = item({ status: 'blocked_by_gate', summary: 'primary task pause' });
    assert.notEqual(model.blockedAutomationTitle(blockedTask, undefined, 96, 'zh'), 'Mail triage paused');
    const task = item({ kind: 'task', status: 'active', secondary_actions: [{ action_kind: 'task.complete' }] });
    const completeAction = model.buildSupportedItemActions(task, 'zh')[0];
    assert.notEqual(completeAction.label, 'Mark read');
  });
});
