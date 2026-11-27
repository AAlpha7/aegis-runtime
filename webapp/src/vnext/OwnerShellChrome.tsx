/**
 * Shared owner shell header for Chat home and non-home surfaces.
 * CEO Intent Contract / shell unification: one frost chrome, one menu system,
 * one Aegis summon control (top-bar avatar = realtime voice only).
 * Text chat lives in ChatHomeView (full tab or bottom sheet), not here.
 */
import type { ReactNode } from 'react';
import { AegisAvatar } from './Avatar';
import { VNextHeaderMenu } from './VNextHeaderMenu';
import type { VNextHeaderMenuItem } from './VNextHeaderMenu';
import { classNames, ownerShellChromeClass } from './presentation';

/** Live duplex / summon state for the chrome avatar control. */
export type OwnerShellSummon = {
  avatarUrl?: string;
  /** Voice session active (connecting / live). */
  live: boolean;
  agentSpeaking?: boolean;
  userSpeaking?: boolean;
  fault?: boolean;
  /** Short status only while live or in fault — never idle noise. */
  statusLine?: string;
  voiceLabel: string;
  voiceHint: string;
  /** Tap avatar → duplex voice. */
  onToggleVoice: () => void;
};

export type OwnerShellChromeProps = {
  /** Chat uses sticky pill chrome; console uses surface title chrome. */
  variant: 'chat' | 'console';
  kicker: string;
  title: string;
  subtitle?: string;
  /**
   * When set, the Aegis avatar is the global voice summon (replaces floating
   * orb). Titles stay non-interactive — text chat opens via the sheet FAB.
   */
  summon?: OwnerShellSummon | null;
  /** Chat: custom identity block when summon is absent. Console: unused. */
  leading?: ReactNode;
  /** Optional center peers (e.g. mobile Work). Hidden on desktop via CSS. */
  middle?: ReactNode;
  /** Right-side status text (session / busy). */
  statusText?: string;
  /** Console: an action rendered first in the trailing cluster (和安琪说). */
  trailing?: ReactNode;
  menuTriggerLabel: string;
  menuItems: VNextHeaderMenuItem[];
  /** Chat scroll-collapse modifier. */
  collapsed?: boolean;
  className?: string;
};

export function OwnerShellChrome({
  variant,
  kicker,
  title,
  subtitle,
  summon,
  leading,
  middle,
  trailing,
  statusText,
  menuTriggerLabel,
  menuItems,
  collapsed = false,
  className,
}: OwnerShellChromeProps) {
  const chromeClass = ownerShellChromeClass(variant === 'chat' ? 'chat-topbar' : 'header');

  const titles = (
    <div className="vnext-owner-shell-titles">
      <p className="vnext-kicker">{kicker}</p>
      <h1>{title}</h1>
      {subtitle ? <p className="vnext-header-subtitle">{subtitle}</p> : null}
    </div>
  );

  const summonLeading = summon ? (
    <div className="vnext-owner-shell-summon-block">
      <button
        type="button"
        className={classNames(
          'vnext-owner-shell-summon',
          summon.live && 'is-live',
          summon.agentSpeaking && 'is-agent-speaking',
          summon.userSpeaking && 'is-user-speaking',
          summon.fault && 'is-fault',
        )}
        aria-label={summon.voiceLabel}
        title={summon.voiceHint}
        aria-pressed={summon.live}
        data-testid="ceo-voice-summon"
        onClick={summon.onToggleVoice}
      >
        <span className="vnext-owner-shell-summon-ring" aria-hidden="true" />
        <AegisAvatar src={summon.avatarUrl} className="vnext-owner-shell-summon-avatar" />
      </button>
      <div className="vnext-owner-shell-summon-meta">
        {titles}
        {summon.statusLine ? (
          <p className="vnext-owner-shell-summon-status" role="status" data-testid="ceo-voice-summon-status">
            {summon.statusLine}
          </p>
        ) : null}
      </div>
    </div>
  ) : null;

  return (
    <header
      className={classNames(
        chromeClass,
        variant === 'chat' && collapsed && 'is-collapsed',
        summon && 'has-summon',
        className,
      )}
    >
      <div className="vnext-owner-shell-leading">
        {summonLeading ?? leading ?? titles}
      </div>
      {middle ? <div className="vnext-owner-shell-middle">{middle}</div> : null}
      <div className="vnext-owner-shell-trailing vnext-header-actions">
        {trailing}
        {statusText ? (
          <span className="vnext-owner-shell-status">
            <span className="vnext-session-dot" aria-hidden="true" />
            <span className="vnext-owner-shell-status-text">{statusText}</span>
          </span>
        ) : null}
        <VNextHeaderMenu triggerLabel={menuTriggerLabel} items={menuItems} />
      </div>
    </header>
  );
}
