import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.API_GUARDS_MODULE;

if (!modulePath) {
  throw new Error('API_GUARDS_MODULE must point at the compiled apiGuards.js file');
}

const {
  readConsoleApprovalActionResponse,
  readConsoleMandateLifecycleResponse,
  readConsoleMemoryDeleteResponse,
  readConsoleMemoryUpdateResponse,
  readConsoleProfileUpdateResponse,
  readMfaStatusResponse,
    readOwnerWorkContextEnvelope,
    readOwnerWorkQueue,
    readOwnerWorkResponse,
  readSessionInfo,
  readStreamTokenInfo,
  readTotpSetupResponse,
  readVNextAvatarProfileProjection,
  readVNextChatHistoryProjection,
  readVNextCommandResponse,
  readVNextDebugSnapshotResponse,
  readVNextItemProjection,
  readVNextModelCatalog,
  readVNextSettingsProjection,
  readVNextStatusProjection,
  readVNextWorkActionResponse,
  readVNextWorkProjection,
  readWebAuthnCredentialsResponse,
  sanitizeNightlyStatus,
  sanitizeTelegramRail,
} = await import(modulePath);

describe('api response guards', () => {
  it('accepts expected session, owner work, and stream token payloads', () => {
    assert.equal(readSessionInfo({
      authenticated: true,
      principal_type: 'web_console',
      email: 'owner@example.com',
      mfa_level: 'passkey',
      mfa_verified_at: 1_800_000_000,
      requires_mfa_setup: false,
      csrf_token: 'csrf',
    })?.authenticated, true);
    const ownerWorkPayload = {
      schema_version: 1,
      generated_at: '2026-05-05T00:00:00Z',
      owner: 'did:aegis:telegram:12345678',
      sources: [{ source: 'gmail', status: 'available', reason: 'metadata only' }],
      counts: { total: 1, email: 1, sensitive: 1 },
      items: [{
        id: 'email:1',
        kind: 'email',
        source_kind: 'mail',
        title: 'Billing statement',
        summary: 'From sender. AI thinks this needs attention now. No attachments are known from the header.',
        source: 'gmail',
        source_id: '1',
        source_event_id: 'gmail_message:abc',
        status: 'unread',
        urgency: 'high',
        sensitivity: 'S3',
        privacy_level: 'owner_only',
        created_at: '2026-05-05T00:00:00Z',
        updated_at: '2026-05-05T00:00:00Z',
        due_at: null,
        owner_visible_reason: 'Email sender, subject, labels, or timing made this worth review.',
        owner_visible_reason_code: 'email_basics_review',
        next_safe_action: 'Read the exact email, then brief, draft, archive, or mark read.',
        next_safe_action_code: 'email_unread_review',
        blocked_by: null,
        blocked_by_code: null,
        primary_action: {
          label: 'Read email detail',
          label_code: 'open_email',
          action_kind: 'navigate',
          route: '/webapp/console/mail/events/1/body',
          requires_gate: true,
          requires_mfa: false,
        },
        secondary_actions: [],
        receipt_expectation: 'Mailbox changes keep a provider receipt.',
        authority: {
          item_id: 'email:1',
          authority_basis: 'manual_review',
          visible_fields: ['title', 'summary'],
          locked_fields: ['body'],
          allowed_now: ['explain', 'draft'],
          requires_data_gate: true,
          requires_mfa: false,
          requires_approval: false,
          expected_receipt: 'Mailbox changes keep a provider receipt.',
          source_mandate_id: null,
          audit_links: [],
        },
        detail_route: '/webapp/console/mail/events/1/body',
        external_url_policy: 'none',
        related_action_ids: [],
        related_receipt_ids: [],
        route_reason: 'metadata only',
        schema_version: 1,
      }],
    };
    assert.equal(readOwnerWorkQueue(ownerWorkPayload)?.items[0].receipt_expectation, 'Mailbox changes keep a provider receipt.');
    assert.equal(readOwnerWorkQueue(ownerWorkPayload)?.items[0].source_kind, 'mail');
    // Structural i18n codes: PRESENT codes pass through the guard…
    assert.equal(readOwnerWorkQueue(ownerWorkPayload)?.items[0].owner_visible_reason_code, 'email_basics_review');
    assert.equal(readOwnerWorkQueue(ownerWorkPayload)?.items[0].next_safe_action_code, 'email_unread_review');
    assert.equal(readOwnerWorkQueue(ownerWorkPayload)?.items[0].primary_action?.label_code, 'open_email');
    assert.equal(readOwnerWorkQueue({
      ...ownerWorkPayload,
      items: [{
        ...ownerWorkPayload.items[0],
        blocked_by: 'Waiting for your approve or reject decision.',
        blocked_by_code: 'approval_waiting_decision',
        decision_request: {
          id: 'decision:email:1',
          title: 'Approve marking mail read?',
          title_code: 'approval_title_gmail_read',
          what_aegis_was_doing: 'Safe summary',
          blocked_reason: 'Aegis needs your explicit approval before changing Gmail state.',
          blocked_reason_code: 'approval_gmail_state',
          requested_capability: 'Approve only if these are the messages you meant to mark read; otherwise reject.',
          capability_code: 'nextCode:approval_gmail_confirm',
          scope: 'This approval request only.',
          scope_code: 'scope_approval_request',
          choices: [],
          safe_details: ['No outside action runs until you choose.'],
          safe_detail_codes: ['safe_no_outside_action'],
          audit_refs: [],
          receipt_expectation: 'Mailbox changes keep a provider receipt.',
        },
      }],
    })?.items[0].decision_request?.blocked_reason_code, 'approval_gmail_state');
    // …ABSENT codes are fine (optional fields; older backend)…
    const {
      owner_visible_reason_code: _r,
      next_safe_action_code: _n,
      blocked_by_code: _b,
      ...itemNoCodes
    } = ownerWorkPayload.items[0];
    const primaryNoCode = { ...ownerWorkPayload.items[0].primary_action };
    delete primaryNoCode.label_code;
    assert.notEqual(readOwnerWorkQueue({
      ...ownerWorkPayload,
      items: [{ ...itemNoCodes, primary_action: primaryNoCode }],
    }), null);
    // …and WRONG-TYPE codes reject the payload (defensive boundary).
    assert.equal(readOwnerWorkQueue({
      ...ownerWorkPayload,
      items: [{ ...ownerWorkPayload.items[0], owner_visible_reason_code: 42 }],
    }), null);
    assert.equal(readOwnerWorkQueue({
      ...ownerWorkPayload,
      items: [{ ...ownerWorkPayload.items[0], next_safe_action_code: { nested: true } }],
    }), null);
    assert.equal(readOwnerWorkQueue({
      ...ownerWorkPayload,
      items: [{ ...ownerWorkPayload.items[0], blocked_by_code: { bad: true } }],
    }), null);
    assert.equal(readOwnerWorkQueue({
      ...ownerWorkPayload,
      items: [{
        ...ownerWorkPayload.items[0],
        blocked_by_code: 'y'.repeat(65),
      }],
    }), null);
    assert.equal(readOwnerWorkQueue({
      ...ownerWorkPayload,
      items: [{
        ...ownerWorkPayload.items[0],
        decision_request: {
          id: 'decision:email:1',
          title: 'Approve marking mail read?',
          what_aegis_was_doing: 'Safe summary',
          blocked_reason: 'Aegis needs your explicit approval before changing Gmail state.',
          blocked_reason_code: 'z'.repeat(65),
          requested_capability: 'Approve or reject.',
          scope: 'This approval request only.',
          choices: [],
          safe_details: ['No outside action runs until you choose.'],
          audit_refs: [],
          receipt_expectation: 'Mailbox changes keep a provider receipt.',
        },
      }],
    }), null);
    assert.equal(readOwnerWorkQueue({
      ...ownerWorkPayload,
      items: [{
        ...ownerWorkPayload.items[0],
        primary_action: { ...ownerWorkPayload.items[0].primary_action, label_code: ['open_email'] },
      }],
    }), null);
    const vnextItem = {
      id: 'email:1',
      lane: 'decide',
      source_kind: 'mail',
      source: 'gmail',
      source_id: '19de6aafd6479f82',
      source_event_id: 'gmail-event:19de6aafd6479f82',
      title: 'Billing statement',
      summary: 'Safe summary',
      why_it_matters: 'Aegis surfaced this for owner review.',
      why_it_matters_code: 'email_basics_review',
      state: 'unread',
      urgency: 'high',
      sensitivity: 'S3',
      created_at: '2026-05-05T00:00:00Z',
      updated_at: '2026-05-05T00:00:00Z',
      due_at: null,
      authority: {
        basis: 'manual_review',
        owner_visible_reason: 'Aegis surfaced this for owner review.',
        visible_data: ['title', 'summary'],
        locked_data: ['body'],
        allowed_now: ['explain', 'draft'],
        requires_mfa: false,
        requires_approval: false,
        requires_data_grant: true,
        expected_receipt: 'Mailbox changes keep a provider receipt.',
        source_mandate_id: null,
        audit_refs: [],
      },
      recommended_action: {
        id: 'navigate',
        label: 'Read email detail',
        label_code: 'open_email',
        kind: 'navigate',
        route: '/webapp/console/mail/events/19de6aafd6479f82/body',
        requires_confirmation: false,
        requires_mfa: false,
        requires_data_gate: true,
        expected_receipt: 'Mailbox changes keep a provider receipt.',
      },
      contextual_actions: [],
      next_safe_action: 'Read and decide.',
      next_safe_action_code: 'email_unread_review',
      blocked_by: null,
      detail_route: '/webapp/console/mail/events/19de6aafd6479f82/body',
      external_url_policy: 'internal_only',
      route_reason: 'test projection',
      related_work_item_ids: [],
      related_receipt_ids: [],
      receipt_expectation: 'Mailbox changes keep a provider receipt.',
    };
    assert.equal(readVNextWorkProjection({
      schema_version: 1,
      generated_at: '2026-05-05T00:00:00Z',
      sources: [{ source: 'tasks', status: 'available' }],
      counts: { total: 1 },
      lanes: [
        { id: 'now', title: 'Now', collapsed: true, items: [] },
        { id: 'decide', title: 'Decide', collapsed: false, items: [vnextItem] },
        { id: 'follow_up', title: 'Follow Up', collapsed: true, items: [] },
        { id: 'backlog', title: 'Backlog', collapsed: true, items: [] },
        { id: 'done', title: 'Done', collapsed: true, items: [] },
      ],
    })?.lanes[1].items[0].source_kind, 'mail');
    // Structural i18n codes flow through the vNext projection guard too.
    const codedProjection = readVNextWorkProjection({
      schema_version: 1,
      generated_at: '2026-05-05T00:00:00Z',
      sources: [{ source: 'tasks', status: 'available' }],
      counts: { total: 1 },
      lanes: [
        { id: 'now', title: 'Now', collapsed: true, items: [] },
        { id: 'decide', title: 'Decide', collapsed: false, items: [vnextItem] },
        { id: 'follow_up', title: 'Follow Up', collapsed: true, items: [] },
        { id: 'backlog', title: 'Backlog', collapsed: true, items: [] },
        { id: 'done', title: 'Done', collapsed: true, items: [] },
      ],
    });
    assert.equal(codedProjection?.lanes[1].items[0].why_it_matters_code, 'email_basics_review');
    assert.equal(codedProjection?.lanes[1].items[0].next_safe_action_code, 'email_unread_review');
    assert.equal(codedProjection?.lanes[1].items[0].recommended_action?.label_code, 'open_email');
    // Wrong-type code rejects the projection.
    assert.equal(readVNextWorkProjection({
      schema_version: 1,
      generated_at: '2026-05-05T00:00:00Z',
      sources: [{ source: 'tasks', status: 'available' }],
      counts: { total: 1 },
      lanes: [
        { id: 'now', title: 'Now', collapsed: true, items: [] },
        { id: 'decide', title: 'Decide', collapsed: false, items: [{ ...vnextItem, why_it_matters_code: 7 }] },
        { id: 'follow_up', title: 'Follow Up', collapsed: true, items: [] },
        { id: 'backlog', title: 'Backlog', collapsed: true, items: [] },
        { id: 'done', title: 'Done', collapsed: true, items: [] },
      ],
    }), null);
    // Extra mail_ref on a work item is ignored (still accepted).
    assert.ok(readVNextWorkProjection({
      schema_version: 1,
      generated_at: '2026-05-05T00:00:00Z',
      sources: [{ source: 'gmail', status: 'available' }],
      counts: { total: 1 },
      lanes: [
        { id: 'now', title: 'Now', collapsed: true, items: [] },
        {
          id: 'decide',
          title: 'Decide',
          collapsed: false,
          items: [{ ...vnextItem, mail_ref: { account: 'work', message_id: 'gmail-msg-1' } }],
        },
        { id: 'follow_up', title: 'Follow Up', collapsed: true, items: [] },
        { id: 'backlog', title: 'Backlog', collapsed: true, items: [] },
        { id: 'done', title: 'Done', collapsed: true, items: [] },
      ],
    }));
    assert.equal(readVNextWorkProjection({
      schema_version: 1,
      generated_at: '2026-05-05T00:00:00Z',
      sources: [{ source: 'tasks', status: 'available' }],
      counts: { total: 1 },
      lanes: [
        { id: 'now', title: 'Now', collapsed: true, items: [] },
        {
          id: 'decide',
          title: 'Decide',
          collapsed: false,
          items: [{ ...vnextItem, detail_route: 'https://evil.example/path' }],
        },
        { id: 'follow_up', title: 'Follow Up', collapsed: true, items: [] },
        { id: 'backlog', title: 'Backlog', collapsed: true, items: [] },
        { id: 'done', title: 'Done', collapsed: true, items: [] },
      ],
    }), null);
    assert.equal(readVNextItemProjection({
      schema_version: 1,
      generated_at: '2026-05-05T00:00:00Z',
      item: vnextItem,
      safe_source_detail: 'Safe summary',
      history: [],
      collaboration: {
        focus_id: 'email:1',
        placeholder: 'Ask Aegis anything about this item.',
        suggested_prompts: ['What should I do with this?'],
      },
    })?.collaboration.focus_id, 'email:1');
    const statusEntry = {
      label: 'System Health',
      health: 'ready',
      owner_message: 'Connected sources are ready or intentionally quiet.',
      owner_next_step: 'Open Settings if a connection needs owner action.',
    };
    assert.equal(readVNextStatusProjection({
      schema_version: 1,
      generated_at: '2026-05-05T00:00:00Z',
      sections: [statusEntry],
      system_events: [{
        id: 'system:restart:1779491535',
        kind: 'restart',
        severity: 'quiet',
        title: 'System restarted',
        summary: 'Aegis restarted and returned to service.',
        occurred_at: 1779491535,
        window_start_at: 1779486963,
        count: 3,
        details: ['系统已重启 / System restarted'],
      }],
      advanced_diagnostics_available: true,
    })?.sections[0].owner_next_step, 'Open Settings if a connection needs owner action.');
    assert.equal(readVNextSettingsProjection({
      schema_version: 1,
      signed_in_owner: 'owner@example.com',
      connected_services: [statusEntry],
      security: [statusEntry],
      notification_preferences: [statusEntry],
      cost_saving_paused: true,
      advanced_diagnostics_available: true,
    })?.signed_in_owner, 'owner@example.com');
    // display_language is exposed so the webapp learns the configured locale.
    assert.equal(readVNextSettingsProjection({
      schema_version: 1,
      signed_in_owner: 'owner@example.com',
      connected_services: [statusEntry],
      security: [statusEntry],
      notification_preferences: [statusEntry],
      cost_saving_paused: true,
      advanced_diagnostics_available: true,
      display_language: 'en',
    })?.display_language, 'en');
    // Absent display_language stays backward-safe: parse succeeds, field undefined.
    assert.equal(readVNextSettingsProjection({
      schema_version: 1,
      signed_in_owner: 'owner@example.com',
      connected_services: [statusEntry],
      security: [statusEntry],
      notification_preferences: [statusEntry],
      cost_saving_paused: true,
      advanced_diagnostics_available: true,
    })?.display_language, undefined);
    // A malformed display_language (wrong type) is rejected by the guard.
    assert.equal(readVNextSettingsProjection({
      schema_version: 1,
      signed_in_owner: 'owner@example.com',
      connected_services: [statusEntry],
      security: [statusEntry],
      notification_preferences: [statusEntry],
      cost_saving_paused: true,
      advanced_diagnostics_available: true,
      display_language: 42,
    }), null);
    // llm_provider + paid_api_fallback_enabled are exposed for the provider selector.
    const withProvider = readVNextSettingsProjection({
      schema_version: 1,
      signed_in_owner: 'owner@example.com',
      connected_services: [statusEntry],
      security: [statusEntry],
      notification_preferences: [statusEntry],
      cost_saving_paused: true,
      advanced_diagnostics_available: true,
      llm_provider: 'codex_oauth',
      paid_api_fallback_enabled: true,
    });
    assert.equal(withProvider?.llm_provider, 'codex_oauth');
    assert.equal(withProvider?.paid_api_fallback_enabled, true);
    const withSearch = readVNextSettingsProjection({
      schema_version: 1,
      signed_in_owner: 'owner@example.com',
      connected_services: [statusEntry],
      security: [statusEntry],
      notification_preferences: [statusEntry],
      cost_saving_paused: false,
      advanced_diagnostics_available: false,
      web_search_provider: 'xai_subscription',
      web_search_model: 'grok-4.7',
      web_search_model_options: ['grok-4.7', 'grok-4.6'],
    });
    assert.equal(withSearch?.web_search_provider, 'xai_subscription');
    assert.equal(withSearch?.web_search_model, 'grok-4.7');
    assert.deepEqual(withSearch?.web_search_model_options, ['grok-4.7', 'grok-4.6']);
    assert.equal(readVNextSettingsProjection({
      schema_version: 1,
      signed_in_owner: 'owner@example.com',
      connected_services: [statusEntry],
      security: [statusEntry],
      notification_preferences: [statusEntry],
      cost_saving_paused: false,
      advanced_diagnostics_available: false,
      web_search_provider: 'x'.repeat(40),
    }), null);
    // Absent provider fields stay backward-safe (parse succeeds, fields undefined).
    const noProvider = readVNextSettingsProjection({
      schema_version: 1,
      signed_in_owner: 'owner@example.com',
      connected_services: [statusEntry],
      security: [statusEntry],
      notification_preferences: [statusEntry],
      cost_saving_paused: true,
      advanced_diagnostics_available: true,
    });
    assert.equal(noProvider?.llm_provider, undefined);
    assert.equal(noProvider?.paid_api_fallback_enabled, undefined);
    // A malformed paid_api_fallback_enabled (wrong type) is rejected.
    assert.equal(readVNextSettingsProjection({
      schema_version: 1,
      signed_in_owner: 'owner@example.com',
      connected_services: [statusEntry],
      security: [statusEntry],
      notification_preferences: [statusEntry],
      cost_saving_paused: true,
      advanced_diagnostics_available: true,
      paid_api_fallback_enabled: 'yes',
    }), null);
    assert.equal(readVNextSettingsProjection({
      schema_version: 1,
      signed_in_owner: 'owner@example.com',
      connected_services: [statusEntry],
      security: [statusEntry],
      notification_preferences: [statusEntry],
      cost_saving_paused: true,
      advanced_diagnostics_available: true,
      llm_model: '',
    })?.llm_model, '');
    const catalogOk = readVNextModelCatalog({
      schema_version: 1,
      providers: [{
        id: 'openrouter',
        connected: true,
        source: 'openrouter_models_api',
        fetched_at_unix: 1,
        models: [{ id: 'deepseek/deepseek-v4-pro', display_name: 'DeepSeek V4 Pro' }],
      }],
      current: { provider: 'openrouter', model: 'deepseek/deepseek-v4-pro' },
      fallback: { provider: null, model: '' },
    });
    assert.equal(catalogOk?.providers[0].models[0].id, 'deepseek/deepseek-v4-pro');
    assert.equal(readVNextModelCatalog({...catalogOk, recommended_api_model: 'vendor/example'})?.recommended_api_model, 'vendor/example');
    assert.equal(readVNextModelCatalog({...catalogOk, recommended_api_model: {id: 'bad'}}), null);
    assert.ok(readVNextModelCatalog({...catalogOk, recommended_api_model: null}));
    assert.equal(readVNextModelCatalog({
      schema_version: 1,
      providers: new Array(11).fill({
        id: 'openrouter',
        connected: true,
        source: 'openrouter_models_api',
        models: [],
      }),
      current: { provider: 'openrouter', model: '' },
      fallback: { provider: null, model: '' },
    }), null);
    assert.equal(readVNextModelCatalog({
      schema_version: 1,
      providers: [{
        id: 'openrouter',
        connected: true,
        source: 'openrouter_models_api',
        models: [{ id: 'x'.repeat(201), display_name: 'too long' }],
      }],
      current: { provider: 'openrouter', model: '' },
      fallback: { provider: null, model: '' },
    }), null);
    // 加载更早: has_more is optional and must be a boolean when present.
    assert.ok(readVNextChatHistoryProjection({ schema_version: 1, has_more: true, messages: [] }));
    const holdCard = {
      approval_id: 'gate-aaaaaaaaaaaaaaaa',
      tool_name: 'http_api',
      summary: 'destination: lawyer@example.com\nwould_send: billing note\nstopped: sensitive',
      state: 'approved',
      timestamp: 1_758_000_100,
    };
    assert.equal(readVNextChatHistoryProjection({ schema_version: 1, messages: [], approvals: [holdCard] })?.approvals[0].state, 'approved');
    assert.equal(readVNextChatHistoryProjection({ schema_version: 1, messages: [], approvals: [{ ...holdCard, state: 'done' }] }), null);
    const identifiedTurn = { role: 'aegis', text: '晴', timestamp: 1_777_000_001, turn_id: 'turn-weather' };
    assert.equal(readVNextChatHistoryProjection({ schema_version: 1, messages: [identifiedTurn] })?.messages[0].turn_id, 'turn-weather');
    const failureHint = {
      role: 'aegis',
      text: '这轮没有生成回复。消息已留下，可以再发一次。',
      timestamp: 1_777_000_002,
      retry_text: '帮我看看这封信',
    };
    assert.equal(readVNextChatHistoryProjection({ schema_version: 1, messages: [failureHint] })?.messages[0].retry_text, '帮我看看这封信');
    assert.equal(readVNextChatHistoryProjection({ schema_version: 1, messages: [{ ...failureHint, retry_text: 12 }] }), null);
    assert.equal(readVNextChatHistoryProjection({ schema_version: 1, messages: [{ ...failureHint, retry_text: 'x'.repeat(4001) }] }), null);
    for (const turn_id of ['', 12, 'x'.repeat(257)]) {
      assert.equal(readVNextChatHistoryProjection({ schema_version: 1, messages: [{...identifiedTurn, turn_id}] }), null);
    }
    assert.equal(readVNextChatHistoryProjection({ schema_version: 1, has_more: 'yes', messages: [] }), null);
    assert.equal(readVNextChatHistoryProjection({
      schema_version: 1,
      messages: [
        { role: 'owner', text: '通勤', timestamp: 1_777_000_000 },
        { role: 'aegis', text: '预计 44 分钟。', timestamp: 1_777_000_001, severity: 'important' },
      ],
    })?.messages[1].severity, 'important');
    assert.equal(readVNextChatHistoryProjection({
      schema_version: 1,
      messages: [
        { role: 'aegis', text: 'bad severity', timestamp: 1_777_000_001, severity: 'panic' },
      ],
    }), null);
    assert.equal(readVNextChatHistoryProjection({
      schema_version: 1,
      messages: [
        {
          role: 'aegis',
          text: '约 44 分钟 [E1]',
          timestamp: 1_777_000_002,
          evidence: [{
            id: 'E1',
            tool: 'commute_traffic_monitor',
            label: '通勤检查',
            excerpt: '当前交通约 44 分钟。',
            full_len: 1234,
          }],
          unsupported_numbers: ['60–80'],
        },
      ],
    })?.messages[0].evidence[0].id, 'E1');
    assert.equal(readVNextChatHistoryProjection({
      schema_version: 1,
      messages: [
        {
          role: 'aegis',
          text: '约 44 分钟 [E1]',
          timestamp: 1_777_000_002,
          evidence: [{
            id: 'evidence-1',
            tool: 'commute_traffic_monitor',
            label: '通勤检查',
            excerpt: '当前交通约 44 分钟。',
            full_len: 1234,
          }],
        },
      ],
    }), null);
    assert.equal(readVNextAvatarProfileProjection({
      schema_version: 1,
      aegis_avatar_url: '/aegis-avatar.svg',
      owner_avatar_url: 'data:image/png;base64,b3duZXI=',
      updated_at_unix: 1_777_000_002,
    })?.owner_avatar_url, 'data:image/png;base64,b3duZXI=');
    assert.equal(readVNextAvatarProfileProjection({
      schema_version: 1,
      aegis_avatar_url: '/webapp/vnext/avatar-image/aegis-owner-1779559061649-abcd.png',
      owner_avatar_url: 'data:image/png;base64,b3duZXI=',
      updated_at_unix: 1_777_000_002,
      auto_refresh: {
        enabled: true,
        provider: 'xai_subscription',
        interval_days: 7,
        image_generation_available: true,
        status: 'ready',
        owner_message: 'Grok image capability is connected.',
        owner_next_step: 'Use Generate Aegis avatar.',
        last_refresh_at_unix: 1_777_000_001,
        next_refresh_at_unix: 1_777_604_801,
        prompt_hint: '',
      },
    })?.aegis_avatar_url, '/webapp/vnext/avatar-image/aegis-owner-1779559061649-abcd.png');
    assert.equal(readVNextCommandResponse({
      schema_version: 1,
      generated_at: '2026-05-05T00:00:00Z',
      outcome: 'answer_now',
      owner_message: 'Aegis responded.',
      assistant_response: { response_kind: 'answer' },
    })?.outcome, 'answer_now');
    assert.equal(readVNextWorkActionResponse({
      schema_version: 1,
      generated_at: '2026-05-05T00:00:00Z',
      status: 'ok',
      item_id: 'email:1',
      action_id: 'mail.mark_read',
      owner_message: 'Mail updated.',
      receipt: {
        id: 'action:1',
        kind: 'action_receipt',
        label: 'gmail_read_state:1',
        created_at: '2026-05-05T00:00:00Z',
      },
    })?.receipt?.kind, 'action_receipt');
    assert.equal(readVNextWorkActionResponse({
      schema_version: 1,
      generated_at: '2026-05-05T00:00:00Z',
      status: 'ok',
      item_id: 'approval:browser',
      action_id: 'approval.approve',
      owner_message: 'Aegis continued after approval. '.repeat(80),
    })?.action_id, 'approval.approve');
    assert.equal(readVNextDebugSnapshotResponse({
      schema_version: 1,
      id: 'snapshot-123',
      expires_at_unix: 1_800_000_060,
    })?.id, 'snapshot-123');
    const ownerWorkContextPayload = {
      schema_version: 1,
      focus_id: 'email:1',
      focus_kind: 'email',
      focus_source: 'gmail',
      visible_fields: {
        title: 'Billing statement',
        summary: 'From sender. AI thinks this needs attention now. No attachments are known from the header.',
        sensitivity: 'S3',
      },
      redacted_fields: ['body', 'html', 'raw_message_id'],
      authorized_sensitive_scopes: ['inbox_metadata'],
      denied_sensitive_scopes: ['email.body', 'email.raw_message_id'],
      allowed_intents: ['explain', 'draft', 'propose_action'],
      required_gate: {
        gate_kind: 'data_access_gate',
        reason: 'private fields omitted',
        route: '/webapp/console/mail/events/1/body',
        requires_mfa: false,
      },
      related_action_ids: [],
      related_receipt_ids: [],
      source_event_id: 'gmail_message:abc',
      trace_id: null,
      run_id: null,
    };
    assert.equal(readOwnerWorkContextEnvelope(ownerWorkContextPayload)?.required_gate?.gate_kind, 'data_access_gate');
    assert.equal(readOwnerWorkContextEnvelope(ownerWorkContextPayload)?.required_gate?.gate_kind, 'data_access_gate');
    const ownerWorkResponsePayload = {
      schema_version: 1,
      response_kind: 'draft',
      visible_answer: 'Draft only. No action taken.',
      draft: 'Draft only. No action taken.',
      action_draft: null,
      suggested_intent: {
        kind: 'mail_followup',
        label: 'Mail follow-up',
        summary: 'Aegis interpreted this as Mail follow-up; review before any durable work item or external action is created.',
        confidence: 0.68,
        next_action_kind: 'prepare_mail_next_step',
      },
      required_gate: null,
      receipt_expectation: 'No external action was executed.',
      citations: ['visible_fields.title'],
      severity: 'warning',
      source_evidence: [{
        kind: 'context',
        label: 'Visible mail context',
        summary: '3 safe fields visible.',
        detail: 'Aegis used only owner-visible context fields.',
      }],
      warnings: ['source_content_untrusted'],
      trace_id: null,
      run_id: null,
      provider_request_id: 'chatcmpl-test',
    };
    assert.equal(readOwnerWorkResponse(ownerWorkResponsePayload)?.response_kind, 'draft');
    assert.equal(readOwnerWorkResponse(ownerWorkResponsePayload)?.severity, 'warning');
    assert.equal(readOwnerWorkResponse({ ...ownerWorkResponsePayload, severity: 'panic' }), null);
    assert.equal(readOwnerWorkResponse(ownerWorkResponsePayload)?.source_evidence?.[0]?.label, 'Visible mail context');
    const jevPayload = {
      ...ownerWorkResponsePayload,
      source_evidence: [{
        kind: 'tool',
        label: 'Google Drive upload',
        summary: 'File uploaded to Google Drive successfully.',
        detail: 'Current evidence:\n- Tool returned ok: File uploaded.',
        jev: {
          verdict: 'allow',
          audit_tip: 'tip-abc',
          risk: 'high_risk=0.10',
          approval_id: 'jev-ask-skip',
        },
      }],
    };
    assert.equal(readOwnerWorkResponse(jevPayload)?.source_evidence?.[0]?.jev?.verdict, 'allow');
    assert.equal(readOwnerWorkResponse(jevPayload)?.source_evidence?.[0]?.jev?.audit_tip, 'tip-abc');
    assert.equal(readOwnerWorkResponse({
      ...jevPayload,
      source_evidence: [{ ...jevPayload.source_evidence[0], jev: { verdict: 'maybe' } }],
    }), null);
    assert.equal(readOwnerWorkResponse(ownerWorkResponsePayload)?.suggested_intent?.kind, 'mail_followup');
    assert.equal(readOwnerWorkResponse(ownerWorkResponsePayload)?.response_kind, 'draft');
    assert.equal(readOwnerWorkResponse(ownerWorkResponsePayload)?.evidence, undefined);
    assert.equal(readOwnerWorkResponse(ownerWorkResponsePayload)?.unsupported_numbers, undefined);
    const groundedPayload = {
      ...ownerWorkResponsePayload,
      evidence: [{
        id: 'E1',
        tool: 'commute_traffic_monitor',
        label: 'Commute check',
        excerpt: 'drive time 43.70 minutes',
        full_len: 25,
      }],
      unsupported_numbers: ['60–80'],
    };
    assert.equal(readOwnerWorkResponse(groundedPayload)?.evidence?.[0]?.id, 'E1');
    assert.deepEqual(readOwnerWorkResponse(groundedPayload)?.unsupported_numbers, ['60–80']);
    assert.equal(readOwnerWorkResponse({
      ...groundedPayload,
      evidence: Array.from({ length: 17 }, (_, index) => ({
        id: 'E1',
        tool: 'web_search',
        label: 'Web search',
        excerpt: `excerpt ${index}`,
        full_len: 8,
      })),
    }), null);
    assert.equal(readOwnerWorkResponse({
      ...groundedPayload,
      evidence: [{
        id: 'EVIDENCE1',
        tool: 'web_search',
        label: 'Web search',
        excerpt: 'ok',
        full_len: 2,
      }],
    }), null);
    assert.equal(readOwnerWorkResponse({
      ...groundedPayload,
      evidence: [{
        id: 'E1',
        tool: 'web_search',
        label: 'Web search',
        excerpt: 'ok',
        full_len: '25',
      }],
    }), null);
    assert.equal(readOwnerWorkResponse({
      ...groundedPayload,
      unsupported_numbers: Array.from({ length: 33 }, (_, index) => `n${index}`),
    }), null);
    assert.equal(readOwnerWorkResponse({
      ...groundedPayload,
      unsupported_numbers: [12],
    }), null);
    assert.deepEqual(readStreamTokenInfo({
      stream_token: 'stream-token',
      expires_at: 1_800_000_060,
      expires_in: 60,
    })?.stream_token, 'stream-token');
    const mfaStatus = readMfaStatusResponse({
      authenticated: true,
      mfa: {
        mfa_level: 'passkey',
        mfa_verified_at: null,
        requires_mfa_setup: false,
        webauthn_registered_count: 1,
        webauthn_available: false,
        webauthn_status: 'deferred',
        totp_configured: true,
        preferred_step_up_method: 'totp',
        recovery_codes_remaining: 8,
      },
    });
    assert.equal(mfaStatus?.mfa.totp_configured, true);
    assert.equal(mfaStatus?.mfa.webauthn_available, false);
    assert.equal(mfaStatus?.mfa.webauthn_status, 'deferred');
    assert.equal(mfaStatus?.mfa.preferred_step_up_method, 'totp');
    assert.equal(readMfaStatusResponse({
      authenticated: true,
      mfa: {
        mfa_level: 'passkey',
        mfa_verified_at: null,
        requires_mfa_setup: false,
        webauthn_registered_count: 1,
        totp_registered: false,
        recovery_codes_remaining: 8,
      },
    })?.mfa.totp_configured, false);
    assert.equal(readTotpSetupResponse({
      status: 'ok',
      method: 'totp',
      setup: {
        secret_base32: 'JBSWY3DPEHPK3PXP',
        otpauth_url: 'otpauth://totp/Aegis Runtime:owner@example.com?secret=JBSWY3DPEHPK3PXP',
        issuer: 'Aegis Runtime',
        account_name: 'owner@example.com',
        digits: 6,
        period_secs: 30,
        algorithm: 'SHA256',
      },
    })?.setup.algorithm, 'SHA256');
    assert.equal(readConsoleApprovalActionResponse({
      status: 'ok',
      approval_id: 'config-test',
      action: 'approve',
      risk: 'R3 High',
      approval_type: 'setting_change',
      message: 'approved',
    })?.action, 'approve');
    assert.equal(readConsoleMandateLifecycleResponse({
      status: 'ok',
      mandate: {
        schema_version: 1,
        mandate_id: 'mandate:abc123',
        status: 'active',
        title: 'Billing watch',
        description: 'Surface billing metadata only.',
        scope_kind: 'email_metadata',
        allowed_reads: ['email_metadata'],
        allowed_llm_uses: ['summarize_metadata'],
        allowed_actions: ['surface_card'],
        must_ask: ['email_body'],
        notification_policy: { max_notifications_per_day: 2 },
        budget_policy: { max_llm_calls_per_day: 3 },
        frequency_policy: { dedupe_window: 'PT6H' },
        expiry: null,
        review_after: null,
        created_at: '2026-05-05T00:00:00Z',
        updated_at: '2026-05-05T00:00:00Z',
        created_from: 'webapp',
        source_event_id: null,
        trace_id: null,
        run_id: null,
        matching_enabled: true,
        background_autonomy_enabled: false,
      },
      receipt_expectation: 'Lifecycle only.',
    })?.mandate.status, 'active');
    assert.equal(readConsoleMemoryDeleteResponse({
      status: 'ok',
      resource_type: 'memory_item',
      resource_id: '42',
      deleted: true,
      ledger_entry_id: 99,
      receipt: 'memory_delete:42:abc',
    })?.ledger_entry_id, 99);
    assert.equal(readConsoleMemoryUpdateResponse({
      status: 'ok',
      resource_type: 'memory_item',
      resource_id: '42',
      updated: true,
      ledger_entry_id: 101,
      receipt: 'memory_update:42:abc',
    })?.ledger_entry_id, 101);
    assert.equal(readConsoleProfileUpdateResponse({
      status: 'ok',
      resource_type: 'profile_slot',
      resource_id: 'preference.communication_style',
      updated: true,
      ledger_entry_id: 100,
      receipt: 'profile_update:preference.communication_style:abc',
    })?.ledger_entry_id, 100);
    assert.equal(readWebAuthnCredentialsResponse({
      credentials: [{
        id: 'cred-1',
        nickname: 'Laptop',
        transports: ['internal'],
        backup_eligible: true,
        backup_state: false,
        created_at: 1_800_000_000,
        last_used_at: null,
      }],
    })?.[0].id, 'cred-1');
  });

  it('handles llm_degraded on the status projection defensively (healthy when absent/malformed)', () => {
    const baseStatus = {
      schema_version: 1,
      generated_at: '2026-06-08T00:00:00Z',
      sections: [],
      system_events: [],
      advanced_diagnostics_available: false,
    };
    // Absent → undefined (healthy), projection still parses.
    assert.equal(readVNextStatusProjection({ ...baseStatus })?.llm_degraded, undefined);
    // Explicit null → null (healthy).
    assert.equal(readVNextStatusProjection({ ...baseStatus, llm_degraded: null })?.llm_degraded, null);
    // Well-formed degraded block is preserved.
    const degraded = readVNextStatusProjection({
      ...baseStatus,
      llm_degraded: { reason: 'OpenRouter 429', since: '2026-06-08T10:00:00Z', provider: 'openrouter' },
    })?.llm_degraded;
    assert.equal(degraded?.reason, 'OpenRouter 429');
    assert.equal(degraded?.since, '2026-06-08T10:00:00Z');
    assert.equal(degraded?.provider, 'openrouter');
    // Malformed block (missing fields / wrong type) is DROPPED to undefined, not
    // rejected — a degraded-banner field must never break the status fetch.
    assert.equal(readVNextStatusProjection({ ...baseStatus, llm_degraded: { reason: 'x' } })?.llm_degraded, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, llm_degraded: { reason: 1, since: 2, provider: 3 } })?.llm_degraded, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, llm_degraded: 'oops' })?.llm_degraded, undefined);
    // The surrounding projection still parses successfully despite the bad block.
    assert.ok(readVNextStatusProjection({ ...baseStatus, llm_degraded: 'oops' }));
  });

  it('handles embedder_degraded on the status projection defensively (S2 M1)', () => {
    const baseStatus = {
      schema_version: 1,
      generated_at: '2026-07-08T00:00:00Z',
      sections: [],
      system_events: [],
      advanced_diagnostics_available: false,
    };
    // Absent → undefined (healthy); explicit null → null (healthy).
    assert.equal(readVNextStatusProjection({ ...baseStatus })?.embedder_degraded, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, embedder_degraded: null })?.embedder_degraded, null);
    // Well-formed block with a backup model is preserved field-for-field.
    const withBackup = readVNextStatusProjection({
      ...baseStatus,
      embedder_degraded: {
        since: '2026-07-08T01:00:00Z',
        primary_model: 'text-embedding-primary',
        backup_model: 'text-embedding-backup',
      },
    })?.embedder_degraded;
    assert.equal(withBackup?.since, '2026-07-08T01:00:00Z');
    assert.equal(withBackup?.primary_model, 'text-embedding-primary');
    assert.equal(withBackup?.backup_model, 'text-embedding-backup');
    // backup_model null (no backup exists) and absent both normalize to null.
    assert.equal(readVNextStatusProjection({
      ...baseStatus,
      embedder_degraded: { since: '2026-07-08T01:00:00Z', primary_model: 'p', backup_model: null },
    })?.embedder_degraded?.backup_model, null);
    assert.equal(readVNextStatusProjection({
      ...baseStatus,
      embedder_degraded: { since: '2026-07-08T01:00:00Z', primary_model: 'p' },
    })?.embedder_degraded?.backup_model, null);
    // Malformed block is DROPPED to undefined (treated healthy), never rejected.
    assert.equal(readVNextStatusProjection({ ...baseStatus, embedder_degraded: { since: 'x' } })?.embedder_degraded, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, embedder_degraded: { since: 1, primary_model: 2 } })?.embedder_degraded, undefined);
    assert.equal(readVNextStatusProjection({
      ...baseStatus,
      embedder_degraded: { since: '2026-07-08T01:00:00Z', primary_model: 'p', backup_model: 42 },
    })?.embedder_degraded, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, embedder_degraded: 'oops' })?.embedder_degraded, undefined);
    // The surrounding projection still parses successfully despite the bad block.
    assert.ok(readVNextStatusProjection({ ...baseStatus, embedder_degraded: 'oops' }));
  });

  it('rejects malformed successful payloads before the app renders them', () => {
    assert.equal(readSessionInfo({ authenticated: true, principal_type: 'owner' }), null);
    assert.equal(readSessionInfo({ authenticated: false, google_login_enabled: 'true' }), null);
    assert.equal(readSessionInfo({ authenticated: false, passkey_login_enabled: 1 }), null);
    assert.equal(readSessionInfo({ authenticated: true, mfa_verified_at: Number.NaN }), null);
    assert.equal(readOwnerWorkQueue({
      schema_version: 1,
      generated_at: '2026-05-05T00:00:00Z',
      owner: 'did:aegis:telegram:12345678',
      sources: [],
      counts: { total: 1 },
      items: [{
        id: 'email:1',
        kind: 'email',
        title: 'x',
        summary: 'x',
        source: 'gmail',
        source_id: '1',
        status: 'unread',
        urgency: 'high',
        sensitivity: 'S3',
        privacy_level: 'owner_only',
        created_at: '2026-05-05T00:00:00Z',
        updated_at: '2026-05-05T00:00:00Z',
        primary_action: null,
        secondary_actions: [],
        receipt_expectation: 'x'.repeat(513),
        authority: null,
        detail_route: 'javascript:alert(1)',
        external_url_policy: 'none',
        related_action_ids: [],
        related_receipt_ids: [],
        route_reason: 'metadata only',
        schema_version: 1,
      }],
    }), null);
    assert.equal(readOwnerWorkContextEnvelope({
      schema_version: 1,
      focus_id: 'email:1',
      focus_kind: 'email',
      focus_source: 'gmail',
      visible_fields: {
        title: 'Billing statement',
      },
      redacted_fields: ['body'],
      authorized_sensitive_scopes: ['inbox_metadata'],
      denied_sensitive_scopes: ['email.body'],
      allowed_intents: ['explain'],
      required_gate: {
        gate_kind: 'data_access_gate',
        reason: 'private fields omitted',
        route: 'javascript:alert(1)',
        requires_mfa: false,
      },
      related_action_ids: [],
      related_receipt_ids: [],
    }), null);
    assert.equal(readOwnerWorkContextEnvelope({
      schema_version: 1,
      focus_id: 'email:1',
      focus_kind: 'email',
      focus_source: 'gmail',
      visible_fields: {
        title: 'x'.repeat(1025),
      },
      redacted_fields: ['body'],
      authorized_sensitive_scopes: ['inbox_metadata'],
      denied_sensitive_scopes: ['email.body'],
      allowed_intents: ['explain'],
      required_gate: null,
      related_action_ids: [],
      related_receipt_ids: [],
    }), null);
    assert.equal(readOwnerWorkResponse({
      schema_version: 1,
      response_kind: 'draft',
      visible_answer: 'x'.repeat(4097),
      draft: null,
      required_gate: null,
      receipt_expectation: 'No external action was executed.',
      citations: [],
      warnings: [],
    }), null);
    assert.equal(readOwnerWorkResponse({
      schema_version: 1,
      response_kind: 'action_proposal',
      visible_answer: 'Proposed action',
      draft: null,
      action_draft: { unsafeUntilPhase3: true },
      required_gate: null,
      receipt_expectation: 'No external action was executed.',
      citations: [],
      warnings: [],
    }), null);
    assert.equal(readOwnerWorkResponse({
      schema_version: 1,
      response_kind: 'orchestrated_response',
      visible_answer: 'Done',
      draft: null,
      action_draft: null,
      suggested_intent: {
        kind: 'reminder',
        label: 'Reminder',
        summary: 'Aegis interpreted this as Reminder.',
        confidence: 2,
        next_action_kind: 'draft_reminder',
      },
      required_gate: null,
      receipt_expectation: 'No external action was executed.',
      citations: [],
      warnings: [],
    }), null);
    assert.equal(readStreamTokenInfo({ stream_token: '', expires_at: 1, expires_in: 60 }), null);
    assert.equal(readMfaStatusResponse({
      authenticated: true,
      mfa: { mfa_level: 'passkey', requires_mfa_setup: false },
    }), null);
    assert.equal(readTotpSetupResponse({
      status: 'ok',
      method: 'totp',
      setup: { secret_base32: '', otpauth_url: 'otpauth://totp/x' },
    }), null);
    const safeTotpSetup = {
      secret_base32: 'JBSWY3DPEHPK3PXP',
      otpauth_url: 'otpauth://totp/Aegis Runtime:owner@example.com?secret=JBSWY3DPEHPK3PXP',
      issuer: 'Aegis Runtime',
      account_name: 'owner@example.com',
      digits: 6,
      period_secs: 30,
      algorithm: 'SHA256',
    };
    for (const setup of [
      { ...safeTotpSetup, otpauth_url: 'javascript:alert(1)' },
      { ...safeTotpSetup, otpauth_url: 'https://example.com/totp?secret=JBSWY3DPEHPK3PXP' },
      { ...safeTotpSetup, otpauth_url: ' otpauth://totp/Aegis Runtime:owner@example.com?secret=JBSWY3DPEHPK3PXP' },
      { ...safeTotpSetup, otpauth_url: 'otpauth://totp/Aegis Runtime:owner@example.com?secret=DIFFERENTSECRET22' },
      { ...safeTotpSetup, secret_base32: 'not-base32' },
      { ...safeTotpSetup, digits: 8 },
      { ...safeTotpSetup, period_secs: 60 },
      { ...safeTotpSetup, algorithm: 'SHA1' },
    ]) {
      assert.equal(readTotpSetupResponse({ status: 'ok', method: 'totp', setup }), null);
    }
    assert.equal(readConsoleApprovalActionResponse({
      status: 'ok',
      approval_id: 'config-test',
      action: 'maybe',
      risk: 'R3 High',
      approval_type: 'setting_change',
      message: 'approved',
    }), null);
    assert.equal(readConsoleMandateLifecycleResponse({
      status: 'ok',
      mandate: {
        schema_version: 1,
        mandate_id: 'mandate:abc123',
        status: 'active',
        title: 'Billing watch',
        description: 'Surface billing metadata only.',
        scope_kind: 'email_metadata',
        allowed_reads: ['email_metadata'],
        allowed_llm_uses: [],
        allowed_actions: ['surface_card'],
        must_ask: ['email_body'],
        notification_policy: {},
        budget_policy: {},
        frequency_policy: {},
        created_at: '2026-05-05T00:00:00Z',
        updated_at: '2026-05-05T00:00:00Z',
        created_from: 'webapp',
        matching_enabled: true,
        background_autonomy_enabled: true,
      },
      receipt_expectation: 'Lifecycle only.',
    }), null);
    assert.equal(readConsoleMemoryDeleteResponse({
      status: 'ok',
      resource_type: 'memory_item',
      resource_id: '42',
      deleted: false,
      ledger_entry_id: 99,
      receipt: 'memory_delete:42:abc',
    }), null);
    assert.equal(readConsoleMemoryUpdateResponse({
      status: 'ok',
      resource_type: 'profile_slot',
      resource_id: '42',
      updated: true,
      ledger_entry_id: 101,
      receipt: 'memory_update:42:abc',
    }), null);
    assert.equal(readConsoleMemoryUpdateResponse({
      status: 'ok',
      resource_type: 'memory_item',
      resource_id: '42',
      updated: false,
      ledger_entry_id: 101,
      receipt: 'memory_update:42:abc',
    }), null);
    assert.equal(readConsoleProfileUpdateResponse({
      status: 'ok',
      resource_type: 'memory_item',
      resource_id: 'preference.communication_style',
      updated: true,
      ledger_entry_id: 100,
      receipt: 'profile_update:preference.communication_style:abc',
    }), null);
    assert.equal(readConsoleProfileUpdateResponse({
      status: 'ok',
      resource_type: 'profile_slot',
      resource_id: 'preference.communication_style',
      updated: false,
      ledger_entry_id: 100,
      receipt: 'profile_update:preference.communication_style:abc',
    }), null);
    assert.equal(readWebAuthnCredentialsResponse({
      credentials: [{ id: 'cred-1', transports: [42], created_at: 1 }],
    }), null);
  });
});

describe('status projection runtime rails (telegram_rail + nightly, 2026-09-16)', () => {
  const baseStatus = {
    schema_version: 1,
    generated_at: '2026-09-16T00:00:00Z',
    sections: [],
    system_events: [],
    advanced_diagnostics_available: false,
  };
  const rail = { state: 'invalid_token', since_unix: 1789700000, last_error: 'A Telegram\'s error: Invalid bot token', degraded: true };
  const nightly = { verdict: 'RED', sha: '02c0d462', finished_at_unix: 1789690000, failed_steps: ['cargo test'], report: '/opt/aegis-runtime/shared/nightly/2026-09-16.md' };

  it('keeps a well-formed telegram_rail field-for-field and normalizes the optional ones', () => {
    const parsed = readVNextStatusProjection({ ...baseStatus, telegram_rail: rail })?.telegram_rail;
    assert.deepEqual(parsed, rail);
    // last_error absent / degraded absent → null / false (an old server that
    // reports the state but not the verdict still renders a row).
    assert.deepEqual(
      readVNextStatusProjection({ ...baseStatus, telegram_rail: { state: 'reachable', since_unix: 10.9 } })?.telegram_rail,
      { state: 'reachable', since_unix: 10, last_error: null, degraded: false },
    );
    for (const state of ['unconfigured', 'starting', 'reachable', 'invalid_token', 'unreachable']) {
      assert.equal(readVNextStatusProjection({ ...baseStatus, telegram_rail: { state, since_unix: 1 } })?.telegram_rail?.state, state);
    }
  });

  it('treats an absent telegram_rail as "old server" and a malformed one as unreported, never as an outage', () => {
    assert.equal(readVNextStatusProjection({ ...baseStatus })?.telegram_rail, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, telegram_rail: null })?.telegram_rail, null);
    // Unknown state: the console has no copy for it and must not guess.
    assert.equal(readVNextStatusProjection({ ...baseStatus, telegram_rail: { state: 'exploded', since_unix: 1 } })?.telegram_rail, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, telegram_rail: { state: 'invalid_token' } })?.telegram_rail, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, telegram_rail: { state: 'invalid_token', since_unix: 'soon' } })?.telegram_rail, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, telegram_rail: { state: 'invalid_token', since_unix: -1 } })?.telegram_rail, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, telegram_rail: { state: 'invalid_token', since_unix: 1, degraded: 'yes' } })?.telegram_rail, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, telegram_rail: { state: 'invalid_token', since_unix: 1, last_error: 7 } })?.telegram_rail, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, telegram_rail: 'oops' })?.telegram_rail, undefined);
    // The surrounding projection still parses despite the bad block.
    assert.ok(readVNextStatusProjection({ ...baseStatus, telegram_rail: 'oops' }));
    // Direct sanitizer contract mirrors the projection-level one.
    assert.equal(sanitizeTelegramRail(undefined), undefined);
    assert.equal(sanitizeTelegramRail(null), null);
    assert.deepEqual(sanitizeTelegramRail(rail), rail);
  });

  it('keeps a well-formed nightly verdict and normalizes its optional fields', () => {
    assert.deepEqual(readVNextStatusProjection({ ...baseStatus, nightly })?.nightly, nightly);
    assert.deepEqual(
      readVNextStatusProjection({ ...baseStatus, nightly: { verdict: 'BROWSER_RED', finished_at_unix: 5 } })?.nightly,
      { verdict: 'BROWSER_RED', sha: '', finished_at_unix: 5, failed_steps: [], report: null },
    );
    assert.equal(readVNextStatusProjection({ ...baseStatus })?.nightly, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, nightly: null })?.nightly, null);
  });

  it('drops a malformed nightly block without failing the status fetch', () => {
    assert.equal(readVNextStatusProjection({ ...baseStatus, nightly: { verdict: 'red', finished_at_unix: 1 } })?.nightly, undefined, 'verdicts are upper-case tokens');
    assert.equal(readVNextStatusProjection({ ...baseStatus, nightly: { verdict: 'RED' } })?.nightly, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, nightly: { verdict: 'RED', finished_at_unix: 'today' } })?.nightly, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, nightly: { verdict: 'RED', finished_at_unix: 1, failed_steps: 'cargo' } })?.nightly, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, nightly: { verdict: 'RED', finished_at_unix: 1, report: 9 } })?.nightly, undefined);
    assert.equal(readVNextStatusProjection({ ...baseStatus, nightly: 'oops' })?.nightly, undefined);
    assert.ok(readVNextStatusProjection({ ...baseStatus, nightly: 'oops' }));
    assert.equal(sanitizeNightlyStatus(undefined), undefined);
    assert.equal(sanitizeNightlyStatus(null), null);
    assert.deepEqual(sanitizeNightlyStatus(nightly), nightly);
  });
});
