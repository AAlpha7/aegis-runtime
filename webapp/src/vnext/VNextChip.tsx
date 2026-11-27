import React from 'react';

/*
 * VNextChip — filter / toggle chip (SP1.5).
 *
 * A standalone pill toggle. Shares the L3 "obsidian + champagne" --btn-* tokens
 * with VNextButton (see design/surfaces/controls.css). SELECTED = gold-edge
 * obsidian / champagne-tinted with gold text; UNSELECTED = quiet surface +
 * hairline with muted text.
 *
 * Unlike VNextSegmented (one-of-N, exclusive), a chip is an independent
 * toggle: `selected` + `aria-pressed`. Use for non-exclusive filters
 * (work scan toggles). A row of chips where exactly one is
 * "all/none" is still modeled as independent chips driven by the parent's state.
 *
 * Spec: docs/superpowers/specs/2026-06-27-button-design-system-design.md
 *       (§Out of scope → SP1.5) + …-webapp-chrome-controls-audit.md
 */

export type VNextChipSize = 'sm' | 'md';

type SharedProps = {
  selected?: boolean;
  size?: VNextChipSize;
  leadingIcon?: React.ReactNode;
  className?: string;
  children?: React.ReactNode;
};

export type VNextChipProps = SharedProps &
  Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'className' | 'children'>;

function cx(...parts: Array<string | false | undefined>): string {
  return parts.filter(Boolean).join(' ');
}

export function VNextChip(props: VNextChipProps) {
  const {
    selected = false, size = 'md', leadingIcon, className, children,
    type, ...rest
  } = props;

  const cls = cx(
    'vnext-chip',
    `vnext-chip-${size}`,
    selected && 'is-selected',
    className,
  );

  return (
    <button
      type={type ?? 'button'}
      className={cls}
      aria-pressed={selected}
      {...rest}
    >
      {leadingIcon && <span className="vnext-chip-ico" aria-hidden="true">{leadingIcon}</span>}
      <span className="vnext-chip-label">{children}</span>
    </button>
  );
}
