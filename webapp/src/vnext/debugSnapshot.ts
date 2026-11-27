type VNextCommandScope = 'item' | 'general';
type VNextViewId = 'home' | 'work' | 'status' | 'settings';

interface DebugSnapshotWorkItem {
  id: string;
  kind: string;
  title: string;
  status: string;
  urgency: string;
  sensitivity: string;
  blocked_by?: string | null;
  route_reason?: string | null;
}

interface DebugSnapshotQueue {
  counts?: Record<string, unknown>;
  sources?: Array<{ source: string; status: string; reason?: string | null }>;
  items: DebugSnapshotWorkItem[];
}

interface DebugSnapshotDiagnosticsProjection {
  advanced_diagnostics_available?: boolean;
}

export interface VNextDebugLayoutMetric {
  selector: string;
  top: number;
  left: number;
  width: number;
  height: number;
  overflow: string;
  display: string;
}

export interface VNextDebugControlMetric {
  selector: string;
  buttons: string[];
}

export interface VNextDebugViewport {
  width: number;
  height: number;
  device_pixel_ratio: number;
  scroll_y: number;
}

export interface VNextDebugSnapshotPayload {
  schema_version: 1;
  captured_at: string;
  view: VNextViewId;
  selected_id: string | null;
  selected_item: {
    id: string;
    kind: string;
    status: string;
    urgency: string;
    sensitivity: string;
    blocked_by: string | null;
    route_reason: string | null;
    title: string;
  } | null;
  command_scope: VNextCommandScope;
  viewport: VNextDebugViewport;
  queue: {
    counts: Record<string, unknown>;
    item_count: number;
    source_statuses: Array<{ source: string; status: string; reason: string | null }>;
  };
  today: {
    decisions: number;
    ready: number;
    follow_ups: number;
    source_issues: number;
  } | null;
  diagnostics: {
    status_available: boolean;
    settings_available: boolean;
  };
  layout: VNextDebugLayoutMetric[];
  controls: VNextDebugControlMetric[];
}

export function buildVNextDebugSnapshotPayload({
  view,
  selectedId,
  queue,
  home,
  statusProjection,
  settingsProjection,
  commandScope,
  viewport,
  layout,
  controls = [],
  capturedAt = new Date().toISOString(),
}: {
  view: VNextViewId;
  selectedId: string | null;
  queue: DebugSnapshotQueue | null;
  home: { decisions: unknown[]; now: unknown[]; followUps: unknown[]; sourceIssueCount: number } | null;
  statusProjection: DebugSnapshotDiagnosticsProjection | null;
  settingsProjection: DebugSnapshotDiagnosticsProjection | null;
  commandScope: VNextCommandScope;
  viewport: VNextDebugViewport;
  layout: VNextDebugLayoutMetric[];
  controls?: VNextDebugControlMetric[];
  capturedAt?: string;
}): VNextDebugSnapshotPayload {
  const selectedItem = selectedId
    ? queue?.items.find((item) => item.id === selectedId) ?? null
    : null;
  return {
    schema_version: 1,
    captured_at: capturedAt,
    view,
    selected_id: selectedId,
    selected_item: selectedItem ? {
      id: selectedItem.id,
      kind: selectedItem.kind,
      status: selectedItem.status,
      urgency: selectedItem.urgency,
      sensitivity: selectedItem.sensitivity,
      blocked_by: selectedItem.blocked_by ?? null,
      route_reason: selectedItem.route_reason ?? null,
      title: selectedItem.title,
    } : null,
    command_scope: commandScope,
    viewport,
    queue: {
      counts: queue?.counts ?? {},
      item_count: queue?.items.length ?? 0,
      source_statuses: queue?.sources?.map((source) => ({
        source: source.source,
        status: source.status,
        reason: source.reason ?? null,
      })) ?? [],
    },
    today: home ? {
      decisions: home.decisions.length,
      ready: home.now.length,
      follow_ups: home.followUps.length,
      source_issues: home.sourceIssueCount,
    } : null,
    diagnostics: {
      status_available: Boolean(statusProjection?.advanced_diagnostics_available),
      settings_available: Boolean(settingsProjection?.advanced_diagnostics_available),
    },
    layout,
    controls,
  };
}

export function reviewVNextDebugSnapshot(snapshot: VNextDebugSnapshotPayload): {
  ok: boolean;
  issues: string[];
} {
  const issues: string[] = [];
  if (snapshot.schema_version !== 1) issues.push('unsupported_schema_version');
  if (snapshot.selected_item && snapshot.selected_id !== snapshot.selected_item.id) {
    issues.push('selected_item_mismatch');
  }
  if (snapshot.selected_item && snapshot.command_scope !== 'item') {
    issues.push('selected_item_without_item_command_scope');
  }
  if (!snapshot.selected_item && snapshot.queue.item_count > 0 && snapshot.command_scope === 'item') {
    issues.push('item_command_scope_without_selected_item');
  }

  const workDetail = snapshot.layout.find((entry) => entry.selector === '.vnext-detail');
  const workDetailTooTall = Boolean(workDetail && snapshot.viewport.height > 0 && workDetail.height > snapshot.viewport.height * 1.5);
  if (workDetailTooTall) {
    issues.push('work_detail_taller_than_viewport');
  }
  if (workDetailTooTall && snapshot.selected_item?.status === 'blocked_by_gate') {
    issues.push('blocked_item_flow_too_tall');
  }
  if (workDetail && workDetail.overflow === 'hidden' && snapshot.viewport.height > 0 && workDetail.height > snapshot.viewport.height) {
    issues.push('work_detail_may_clip_content');
  }

  const selectedDetailControls = (snapshot.controls ?? []).find((entry) => entry.selector === '.vnext-detail');
  const selectedDetailButtons = selectedDetailControls?.buttons ?? [];
  const redirectsElsewhere = selectedDetailButtons.some((label) => (
    /^(go to approvals|open full work|open work)$/i.test(label.trim())
  ));
  if (snapshot.selected_item && redirectsElsewhere) {
    issues.push('selected_item_requires_manual_redirect');
  }
  if (snapshot.selected_item && selectedDetailButtons.length === 0) {
    issues.push('selected_item_controls_missing');
  }
  const hasInlineNextStep = selectedDetailButtons.some((label) => (
    /(ask aegis|explain pause|needed choice|revise|open details?|close pause|review tomorrow|brief|draft|mark read|archive|approve|reject|complete|snooze|open message|open source|try again)/i.test(label)
  ));
  if (snapshot.selected_item && selectedDetailButtons.length > 0 && !hasInlineNextStep) {
    issues.push('selected_item_without_inline_next_step');
  }

  return { ok: issues.length === 0, issues };
}
