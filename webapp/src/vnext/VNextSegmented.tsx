import React, { useRef } from 'react';

/*
 * VNextSegmented — single-select segmented control (SP1.5).
 *
 * A track holding 2+ segments; exactly one is selected. Shares the L3
 * "obsidian + champagne" --btn-* tokens with VNextButton (see
 * design/surfaces/controls.css). The SELECTED segment reads as a gold-edge
 * obsidian fill with gold text; UNSELECTED segments are ghost/transparent with
 * muted text.
 *
 * Two semantic modes (the audit's two call shapes):
 *  - role="tablist" (mode="tab", default): each segment is role="tab" +
 *    aria-selected. Use for view/folder switches.
 *  - role="radiogroup" (mode="radio"): each segment is role="radio" +
 *    aria-checked. Use for option pickers (settings language, work scan
 *    scope/sort).
 *
 * Keyboard: roving tabindex — only the selected segment is in the tab order;
 * ArrowLeft/Right (and Up/Down) move selection between segments, Home/End jump
 * to the ends. This matches the WAI-ARIA tabs / radiogroup patterns.
 *
 * Spec: docs/superpowers/specs/2026-06-27-button-design-system-design.md
 *       (§Out of scope → SP1.5) + …-webapp-chrome-controls-audit.md
 */

export type VNextSegmentedMode = 'tab' | 'radio';
export type VNextSegmentedSize = 'sm' | 'md';

export interface VNextSegmentedOption<V extends string = string> {
  value: V;
  label: React.ReactNode;
  /** Accessible name when `label` is an icon/glyph; falls back to nothing. */
  ariaLabel?: string;
  disabled?: boolean;
}

export interface VNextSegmentedProps<V extends string = string> {
  options: ReadonlyArray<VNextSegmentedOption<V>>;
  value: V;
  onChange: (value: V) => void;
  /** Visible caption above the track (owner-facing). */
  label?: React.ReactNode;
  /** Group accessible name (maps to aria-label on the track). */
  ariaLabel?: string;
  /** 'tab' → role=tablist/tab + aria-selected; 'radio' → radiogroup/radio + aria-checked. */
  mode?: VNextSegmentedMode;
  size?: VNextSegmentedSize;
  /** Stretch the track to fill its container; segments share width equally. */
  fullWidth?: boolean;
  className?: string;
}

function cx(...parts: Array<string | false | undefined>): string {
  return parts.filter(Boolean).join(' ');
}

export function VNextSegmented<V extends string = string>(props: VNextSegmentedProps<V>) {
  const {
    options, value, onChange, label, ariaLabel, mode = 'tab', size = 'md',
    fullWidth = false, className,
  } = props;

  const enabledValues = options.filter((o) => !o.disabled).map((o) => o.value);
  const trackRef = useRef<HTMLDivElement>(null);

  // Roving focus must FOLLOW selection on keyboard nav (WAI-ARIA): focus the
  // newly selected segment, else the SR/focus ring stays on the stale option and
  // a subsequent Space/Enter re-activates it (Codex). Match by data attribute to
  // avoid CSS.escape on arbitrary values.
  function focusValue(v: V) {
    trackRef.current
      ?.querySelectorAll<HTMLElement>('.vnext-seg-item')
      .forEach((el) => {
        if (el.getAttribute('data-seg-value') === v) el.focus();
      });
  }

  function select(v: V) {
    onChange(v);
    focusValue(v);
  }

  function move(delta: number) {
    if (enabledValues.length === 0) return;
    const current = enabledValues.indexOf(value);
    // If the active value is disabled/absent, start from the first enabled.
    const base = current === -1 ? 0 : current;
    const nextIdx = (base + delta + enabledValues.length) % enabledValues.length;
    select(enabledValues[nextIdx]);
  }

  function onKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    switch (event.key) {
      case 'ArrowRight':
      case 'ArrowDown':
        event.preventDefault();
        move(1);
        break;
      case 'ArrowLeft':
      case 'ArrowUp':
        event.preventDefault();
        move(-1);
        break;
      case 'Home':
        event.preventDefault();
        if (enabledValues.length) select(enabledValues[0]);
        break;
      case 'End':
        event.preventDefault();
        if (enabledValues.length) select(enabledValues[enabledValues.length - 1]);
        break;
      default:
        break;
    }
  }

  const isTab = mode === 'tab';
  const trackCls = cx(
    'vnext-seg',
    `vnext-seg-${size}`,
    fullWidth && 'vnext-seg-full',
    className,
  );

  const track = (
    <div
      ref={trackRef}
      className={trackCls}
      role={isTab ? 'tablist' : 'radiogroup'}
      aria-label={ariaLabel}
      onKeyDown={onKeyDown}
    >
      {options.map((option) => {
        const selected = option.value === value;
        const segCls = cx('vnext-seg-item', selected && 'is-selected');
        // Roving tabindex: only the selected segment is reachable by Tab; the
        // rest are focusable only via the arrow keys (WAI-ARIA pattern). A
        // disabled segment is never the tab stop.
        const tabIndex = option.disabled ? -1 : selected ? 0 : -1;
        const ariaSelectionProps = isTab
          ? { role: 'tab' as const, 'aria-selected': selected }
          : { role: 'radio' as const, 'aria-checked': selected };
        return (
          <button
            key={option.value}
            type="button"
            className={segCls}
            data-seg-value={option.value}
            tabIndex={tabIndex}
            disabled={option.disabled}
            aria-label={option.ariaLabel}
            {...ariaSelectionProps}
            onClick={() => { if (!option.disabled) onChange(option.value); }}
          >
            <span className="vnext-seg-label">{option.label}</span>
          </button>
        );
      })}
    </div>
  );

  if (label == null) return track;

  return (
    <div className="vnext-segmented-field">
      <span className="vnext-segmented-label">{label}</span>
      {track}
    </div>
  );
}
