import { Fragment, useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent, ReactElement } from 'react';
import { createPortal } from 'react-dom';
import { VNextIcon } from './icons';
import type { VNextIconName } from './icons';

export type VNextHeaderMenuItem = {
  id: string;
  label: string;
  icon?: VNextIconName;
  onSelect: () => void;
  disabled?: boolean;
  tone?: 'normal' | 'destructive';
  /** Hairline before this row. Used to separate everyday actions from session actions. */
  separatorBefore?: boolean;
};

type PanelCoords = {
  top?: number;
  bottom?: number;
  left?: number;
  right?: number;
};

// A reusable accessible overflow/utility menu for the shared header (SP2 chrome
// unification). Owner decision: Status/Settings + utilities live behind one gear
// menu instead of being "secret" full views or a drifting per-view button trio.
//
// Stacking: the panel is position:fixed + high z-index (not absolute inside the
// frosted header). Parent backdrop-filter creates a stacking context that
// otherwise lets later siblings (sticky chrome) paint over the open menu.
//
// Placement (owner 2026-09-12, IMG "菜单没对齐"): the panel is PORTALED to
// document.body. A `position: fixed` element is positioned against the nearest
// ancestor with a transform/filter/contain, not the viewport — and several
// views keep an identity transform after their entrance animation (see the
// `transform: none` end keyframes in styles.css / motion.css). Inside such an
// ancestor the viewport coords measured here landed the panel hundreds of px
// off (528px on desktop). Rendering into body removes every such ancestor.
// The portal root keeps the `.vnext-header-menu` class so menu.css scoping
// still applies.
//
// A11y: the trigger is aria-haspopup="menu" + aria-expanded; the panel is
// role="menu" with role="menuitem" buttons. Opening focuses the first enabled
// item; ArrowUp/Down/Home/End move a roving focus; Escape closes and returns
// focus to the trigger; a pointer-down outside closes without re-focusing.
export function VNextHeaderMenu({
  triggerLabel,
  items,
  align = 'end',
  direction = 'down',
  triggerClassName,
  triggerIcon = 'overflow',
  defaultOpen = false,
}: {
  triggerLabel: string;
  items: VNextHeaderMenuItem[];
  align?: 'start' | 'end';
  // 'up' opens the panel above the trigger (for triggers pinned to the bottom,
  // e.g. the chat composer, where a downward panel would fall off-screen).
  direction?: 'down' | 'up';
  triggerClassName?: string;
  triggerIcon?: VNextIconName;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const [coords, setCoords] = useState<PanelCoords>({});
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const menuId = useId();

  function measurePanel() {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const r = trigger.getBoundingClientRect();
    const gap = 10;
    const next: PanelCoords = {};
    if (direction === 'up') {
      next.bottom = Math.max(8, window.innerHeight - r.top + gap);
    } else {
      next.top = Math.min(window.innerHeight - 8, r.bottom + gap);
    }
    if (align === 'start') {
      next.left = Math.max(8, r.left);
    } else {
      next.right = Math.max(8, window.innerWidth - r.right);
    }
    setCoords(next);
  }

  useLayoutEffect(() => {
    if (!open) return;
    measurePanel();
  }, [open, align, direction, items.length]);

  // Reposition on viewport change while open (keyboard / rotate / scroll).
  useEffect(() => {
    if (!open) return undefined;
    const onReposition = () => measurePanel();
    window.addEventListener('resize', onReposition);
    // Capture scroll on any scroller so sticky chrome can't leave a ghost menu.
    window.addEventListener('scroll', onReposition, true);
    return () => {
      window.removeEventListener('resize', onReposition);
      window.removeEventListener('scroll', onReposition, true);
    };
  }, [open, align, direction]);

  // Close on outside pointer-down or Escape (Escape returns focus to the trigger).
  useEffect(() => {
    if (!open) return undefined;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && containerRef.current?.contains(target)) return;
      if (target instanceof Node && panelRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    window.addEventListener('pointerdown', onPointerDown);
    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('pointerdown', onPointerDown);
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  // On open, move focus to the first enabled item so keyboard users land inside.
  useEffect(() => {
    if (!open) return;
    const first = panelRef.current?.querySelector<HTMLButtonElement>('.vnext-header-menu-item:not([disabled])');
    first?.focus();
  }, [open]);

  function itemButtons(): HTMLButtonElement[] {
    return Array.from(
      panelRef.current?.querySelectorAll<HTMLButtonElement>('.vnext-header-menu-item:not([disabled])') ?? [],
    );
  }

  function moveFocus(current: HTMLButtonElement, delta: number) {
    const buttons = itemButtons();
    if (buttons.length === 0) return;
    const index = buttons.indexOf(current);
    const next = buttons[(index + delta + buttons.length) % buttons.length];
    next?.focus();
  }

  function onItemKeyDown(event: ReactKeyboardEvent<HTMLButtonElement>) {
    const current = event.currentTarget;
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      moveFocus(current, 1);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      moveFocus(current, -1);
    } else if (event.key === 'Home') {
      event.preventDefault();
      itemButtons()[0]?.focus();
    } else if (event.key === 'End') {
      event.preventDefault();
      const buttons = itemButtons();
      buttons[buttons.length - 1]?.focus();
    }
  }

  function select(item: VNextHeaderMenuItem) {
    if (item.disabled) return;
    setOpen(false);
    item.onSelect();
  }

  const panelStyle: CSSProperties = {
    position: 'fixed',
    top: coords.top !== undefined ? `${coords.top}px` : undefined,
    bottom: coords.bottom !== undefined ? `${coords.bottom}px` : undefined,
    left: coords.left !== undefined ? `${coords.left}px` : undefined,
    right: coords.right !== undefined ? `${coords.right}px` : undefined,
  };

  return (
    <div className={['vnext-header-menu', open && 'is-open'].filter(Boolean).join(' ')} ref={containerRef}>
      <button
        type="button"
        ref={triggerRef}
        className={['vnext-header-menu-trigger', triggerClassName].filter(Boolean).join(' ')}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-label={triggerLabel}
        title={triggerLabel}
        onClick={() => setOpen((value) => !value)}
      >
        <VNextIcon name={triggerIcon} />
      </button>
      {open && renderMenuPanel(
        triggerRef.current,
        <div
          ref={panelRef}
          id={menuId}
          className={`vnext-header-menu-panel vnext-header-menu-fixed vnext-header-menu-${align}${direction === 'up' ? ' vnext-header-menu-up' : ''}`}
          role="menu"
          aria-label={triggerLabel}
          style={panelStyle}
        >
          {items.map((item) => (
            <Fragment key={item.id}>
              {item.separatorBefore ? (
                <div className="vnext-header-menu-separator" role="separator" />
              ) : null}
              <button
                type="button"
                role="menuitem"
                className={['vnext-header-menu-item', item.tone === 'destructive' && 'vnext-header-menu-item-destructive']
                  .filter(Boolean)
                  .join(' ')}
                disabled={item.disabled}
                onKeyDown={onItemKeyDown}
                onClick={() => select(item)}
              >
                {item.icon && <VNextIcon name={item.icon} />}
                <span className="vnext-header-menu-item-label">{item.label}</span>
              </button>
            </Fragment>
          ))}
        </div>,
      )}
    </div>
  );
}

// Portal when a DOM is available (browser); inline for SSR/render tests,
// where `document` does not exist and the legacy absolute anchors apply.
// The host is the `.vnext-app` root (the design tokens — obsidian panel,
// gold hairline — are scoped to it, so a body-level portal renders an
// unstyled, transparent panel), falling back to body. The app root itself
// is never animated, so it is not a containing block for the fixed panel.
function renderMenuPanel(trigger: HTMLElement | null, panel: ReactElement): ReactElement {
  if (typeof document === 'undefined' || !document.body) return panel;
  const host = trigger?.closest<HTMLElement>('.vnext-app') ?? document.body;
  return createPortal(
    <div className="vnext-header-menu is-open vnext-header-menu-portal">{panel}</div>,
    host,
  );
}
