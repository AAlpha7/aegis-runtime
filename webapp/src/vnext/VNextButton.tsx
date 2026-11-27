import React from 'react';

export type VNextButtonVariant = 'primary' | 'cta' | 'secondary' | 'ghost' | 'destructive';
export type VNextButtonSize = 'sm' | 'md' | 'lg' | 'icon';

type SharedProps = {
  variant?: VNextButtonVariant;
  size?: VNextButtonSize;
  leadingIcon?: React.ReactNode;
  trailingIcon?: React.ReactNode;
  loading?: boolean;
  fullWidth?: boolean;
  className?: string;
  children?: React.ReactNode;
};

// iconOnly requires an aria-label; normal buttons may omit it.
type IconOnly = { iconOnly: true; 'aria-label': string };
type NotIconOnly = { iconOnly?: false };

type ButtonEl = SharedProps & (IconOnly | NotIconOnly) &
  Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'className' | 'children'> & { as?: 'button' };
type AnchorEl = SharedProps & (IconOnly | NotIconOnly) &
  Omit<React.AnchorHTMLAttributes<HTMLAnchorElement>, 'className' | 'children'> & { as: 'a'; href: string };

export type VNextButtonProps = ButtonEl | AnchorEl;

function cx(...parts: Array<string | false | undefined>): string {
  return parts.filter(Boolean).join(' ');
}

export function VNextButton(props: VNextButtonProps) {
  const {
    variant = 'primary', size = 'md', leadingIcon, trailingIcon, loading = false,
    fullWidth = false, iconOnly = false, className, children, as = 'button', ...rest
  } = props as SharedProps & { iconOnly?: boolean; as?: 'button' | 'a' } & Record<string, unknown>;

  const cls = cx(
    'vnext-btn',
    `vnext-btn-${variant}`,
    `vnext-btn-${size}`,
    iconOnly && 'vnext-btn-icononly',
    fullWidth && 'vnext-btn-full',
    loading && 'vnext-btn-loading',
    className,
  );

  const content = (
    <>
      {loading && <span className="vnext-btn-spinner" aria-hidden="true" />}
      {!loading && leadingIcon && <span className="vnext-btn-ico">{leadingIcon}</span>}
      {!iconOnly && <span className="vnext-btn-label">{children}</span>}
      {iconOnly && !loading && <span className="vnext-btn-ico">{children}</span>}
      {!loading && trailingIcon && <span className="vnext-btn-ico">{trailingIcon}</span>}
    </>
  );

  if (as === 'a') {
    const { href, ...aRest } = rest as { href: string };
    // While loading, drop the href so neither mouse nor KEYBOARD activation can
    // follow the link (CSS pointer-events only blocks the mouse), and mark it
    // disabled for AT.
    return (
      <a
        className={cls}
        {...aRest}
        href={loading ? undefined : href}
        aria-busy={loading || undefined}
        aria-disabled={loading || undefined}
      >
        {content}
      </a>
    );
  }
  const { disabled: bDisabled, type: bType, ...bRest } =
    rest as React.ButtonHTMLAttributes<HTMLButtonElement>;
  return (
    <button
      className={cls}
      type={bType ?? 'button'}
      aria-busy={loading || undefined}
      {...bRest}
      disabled={(bDisabled ?? false) || loading}
    >
      {content}
    </button>
  );
}
