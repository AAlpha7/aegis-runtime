export type VNextViewId = 'home' | 'work' | 'status' | 'settings';

export type VNextLocationState = {
  view: VNextViewId;
  itemId: string | null;
};

/**
 * Views whose detail focus participates in the URL (`?item=`).
 */
const ITEM_VIEWS: ReadonlyArray<VNextViewId> = ['work'];

function viewSupportsItem(view: VNextViewId): boolean {
  return ITEM_VIEWS.includes(view);
}

export type VNextHistoryMode = 'push' | 'replace';

export const VNEXT_VIEW_IDS: VNextViewId[] = ['home', 'work', 'status', 'settings'];

export function parseVNextLocationState(search: string | URLSearchParams): VNextLocationState {
  const params = typeof search === 'string' ? new URLSearchParams(search) : search;
  const rawView = params.get('view');
  const view = VNEXT_VIEW_IDS.includes(rawView as VNextViewId) ? rawView as VNextViewId : 'home';
  const itemId = viewSupportsItem(view) ? params.get('item')?.trim() || null : null;
  return { view, itemId };
}

export function buildVNextLocationPath(currentHref: string, state: VNextLocationState): string {
  const url = new URL(currentHref, 'https://aegis.local');
  if (state.view === 'home') {
    url.searchParams.delete('view');
  } else {
    url.searchParams.set('view', state.view);
  }
  if (viewSupportsItem(state.view) && state.itemId) {
    url.searchParams.set('item', state.itemId);
  } else {
    url.searchParams.delete('item');
  }
  if (state.view !== 'settings' && url.hash.startsWith('#vnext-settings-')) {
    url.hash = '';
  }
  return `${url.pathname}${url.search}${url.hash}`;
}

export function readVNextLocationState(location: Pick<Location, 'search'>): VNextLocationState {
  return parseVNextLocationState(location.search);
}

export function writeVNextLocationState(
  windowRef: Pick<Window, 'history' | 'location'>,
  state: VNextLocationState,
  mode: VNextHistoryMode = 'push',
) {
  const next = buildVNextLocationPath(windowRef.location.href, state);
  const current = `${windowRef.location.pathname}${windowRef.location.search}${windowRef.location.hash}`;
  if (next === current) return;
  if (mode === 'replace') {
    windowRef.history.replaceState(null, '', next);
  } else {
    windowRef.history.pushState(null, '', next);
  }
}
