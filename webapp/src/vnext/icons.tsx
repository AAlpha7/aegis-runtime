export type VNextIconName =
  | 'home'
  | 'work'
  | 'instructions'
  | 'status'
  | 'settings'
  | 'voice'
  | 'send'
  | 'close'
  | 'trash'
  | 'logout'
  | 'spark'
  | 'shield'
  | 'receipt'
  | 'check'
  | 'clock'
  | 'camera'
  | 'terminal'
  | 'paperclip'
  | 'expand'
  | 'collapse'
  | 'reply'
  | 'refresh'
  | 'search'
  | 'compose'
  | 'forward'
  | 'back'
  | 'overflow'
  | 'link'
  | 'add';

export function itemActionIconName(actionId: string, tone?: 'normal' | 'destructive'): VNextIconName {
  if (tone === 'destructive') return 'close';
  if (actionId === 'goal.abandon') return 'close';
  if (actionId === 'tracking.cancel') return 'close';
  if (actionId === 'task.skip_occurrence') return 'clock';
  if (actionId === 'task.complete') return 'check';
  if (actionId === 'task.snooze_tomorrow') return 'clock';
  if (actionId === 'knowledge.done') return 'check';
  if (actionId === 'knowledge.snooze_tomorrow') return 'clock';
  if (actionId === 'approval.approve') return 'check';
  if (actionId === 'memory.confirm') return 'check';
  if (actionId === 'memory.pin' || actionId === 'memory.unpin') return 'spark';
  return 'receipt';
}

export function VNextIcon({ name }: { name: VNextIconName }) {
  const common = {
    width: 18,
    height: 18,
    viewBox: '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth: 2,
    strokeLinecap: 'round' as const,
    strokeLinejoin: 'round' as const,
    'aria-hidden': true,
  };
  if (name === 'home') {
    return <svg {...common}><path d="M4 11.5 12 5l8 6.5" /><path d="M6.5 10.5V19h11v-8.5" /><path d="M10 19v-5h4v5" /></svg>;
  }
  if (name === 'work') {
    // Leftmost stroke is x=4, the same viewBox edge as the home icon, so the
    // selected and unselected nav glyphs share a left boundary.
    return <svg {...common}><path d="M4 6.5h12v11H4z" /><path d="M7 6.5V5h6v1.5" /><path d="M6.5 11h7" /><path d="M6.5 15h4" /></svg>;
  }
  if (name === 'instructions') {
    return <svg {...common}><path d="M7 5.25h10v13.5H7z" /><path d="M9.5 8.5h5" /><path d="M9.5 12h5" /><path d="M9.5 15.5h3" /></svg>;
  }
  if (name === 'status') {
    return <svg {...common}><path d="M4.75 13.5h3l2-5 3.5 8 2-4h4" /><path d="M12 4.75a7.25 7.25 0 1 0 0 14.5" /></svg>;
  }
  if (name === 'settings') {
    return <svg {...common}><path d="M12 8.25a3.75 3.75 0 1 1 0 7.5 3.75 3.75 0 0 1 0-7.5Z" /><path d="M4.75 12h2" /><path d="M17.25 12h2" /><path d="M12 4.75v2" /><path d="M12 17.25v2" /></svg>;
  }
  if (name === 'overflow') {
    // Horizontal ellipsis (round-capped zero-length strokes render as dots).
    return <svg {...common}><path d="M6 12h.01" /><path d="M12 12h.01" /><path d="M18 12h.01" /></svg>;
  }
  if (name === 'voice') {
    return <svg {...common}><path d="M12 4.75a2.75 2.75 0 0 0-2.75 2.75v4a2.75 2.75 0 1 0 5.5 0v-4A2.75 2.75 0 0 0 12 4.75Z" /><path d="M6.75 11.5a5.25 5.25 0 0 0 10.5 0" /><path d="M12 16.75v2.5" /></svg>;
  }
  if (name === 'spark') {
    return <svg {...common}><path d="M12 4.75 13.45 9 17.75 10.5 13.45 12 12 16.25 10.55 12 6.25 10.5 10.55 9 12 4.75Z" /><path d="M18 15.5 18.6 17.15 20.25 17.75 18.6 18.35 18 20 17.4 18.35 15.75 17.75 17.4 17.15 18 15.5Z" /></svg>;
  }
  if (name === 'shield') {
    return <svg {...common}><path d="M12 4.75 18.25 7v5.2c0 3.75-2.55 6.2-6.25 7.05-3.7-.85-6.25-3.3-6.25-7.05V7L12 4.75Z" /><path d="m9.25 12.25 1.75 1.75 3.75-4" /></svg>;
  }
  if (name === 'receipt') {
    return <svg {...common}><path d="M7 4.75h10v14.5l-2-1.2-2 1.2-2-1.2-2 1.2-2-1.2V4.75Z" /><path d="M9.5 8.75h5" /><path d="M9.5 12h5" /><path d="M9.5 15.25h3" /></svg>;
  }
  if (name === 'check') {
    return <svg {...common}><path d="m5.75 12.25 4.25 4 8.25-8.5" /></svg>;
  }
  if (name === 'clock') {
    return <svg {...common}><path d="M12 5.25a6.75 6.75 0 1 1 0 13.5 6.75 6.75 0 0 1 0-13.5Z" /><path d="M12 8.25v4.25l2.75 1.75" /></svg>;
  }
  if (name === 'camera') {
    return <svg {...common}><path d="M5.25 8.25h3l1.25-2h5l1.25 2h3v9.5H5.25z" /><path d="M12 10.25a3 3 0 1 1 0 6 3 3 0 0 1 0-6Z" /></svg>;
  }
  if (name === 'terminal') {
    return <svg {...common}><path d="M4.75 6.75h14.5v10.5H4.75z" /><path d="m8 10 2.25 2L8 14" /><path d="M12 14h4" /></svg>;
  }
  if (name === 'reply') {
    return <svg {...common}><path d="M9.25 8.25 5.25 12l4 3.75" /><path d="M5.75 12h7.75a5.25 5.25 0 0 1 5.25 5.25v1" /></svg>;
  }
  if (name === 'paperclip') {
    return <svg {...common}><path d="M17.5 9.5 10 17a3.25 3.25 0 0 1-4.6-4.6l7.35-7.35a2.25 2.25 0 0 1 3.18 3.18l-7.4 7.4a1.25 1.25 0 0 1-1.77-1.77l6.9-6.9" /></svg>;
  }
  if (name === 'expand') {
    return <svg {...common}><path d="M4 9V4h5" /><path d="M20 15v5h-5" /><path d="M4 4l6 6" /><path d="M20 20l-6-6" /></svg>;
  }
  if (name === 'collapse') {
    return <svg {...common}><path d="M10 4v6H4" /><path d="M14 20v-6h6" /><path d="M4 20l6-6" /><path d="M20 4l-6 6" /></svg>;
  }
  if (name === 'close') {
    return <svg {...common}><path d="m6.75 6.75 10.5 10.5" /><path d="m17.25 6.75-10.5 10.5" /></svg>;
  }
  if (name === 'trash') {
    return <svg {...common}><path d="M5.25 7h13.5" /><path d="M9.5 7V4.75h5V7" /><path d="M7 7l.8 12.25h8.4L17 7" /><path d="M10.25 10.5v5.5" /><path d="M13.75 10.5v5.5" /></svg>;
  }
  if (name === 'logout') {
    return <svg {...common}><path d="M10.5 4.75H6.25v14.5h4.25" /><path d="M14 8.25 17.75 12 14 15.75" /><path d="M17.5 12H9" /></svg>;
  }
  if (name === 'refresh') {
    return <svg {...common}><path d="M18.75 12a6.75 6.75 0 1 1-1.95-4.75" /><path d="M17.5 4.75v3h-3" /></svg>;
  }
  if (name === 'search') {
    return <svg {...common}><circle cx="10.75" cy="10.75" r="6" /><path d="m19.25 19.25-4.25-4.25" /></svg>;
  }
  if (name === 'compose') {
    return <svg {...common}><path d="M5 19h2l8.5-8.5-2-2L5 17v2Z" /><path d="m14 7.5 2.5 2.5" /></svg>;
  }
  if (name === 'send') {
    return <svg {...common}><path d="M4.75 12 19.25 5.75 13 20.25l-1.65-6.15L4.75 12Z" /><path d="m11.35 14.1 3.9-3.9" /></svg>;
  }
  if (name === 'forward') {
    return <svg {...common}><path d="M14.75 8.25 18.75 12l-4 3.75" /><path d="M18.25 12h-7.75a5.25 5.25 0 0 0-5.25 5.25v1" /></svg>;
  }
  if (name === 'back') {
    return <svg {...common}><path d="M10.5 6.75 5.25 12l5.25 5.25" /><path d="M5.5 12h13" /></svg>;
  }
  if (name === 'link') {
    // Two interlocked chain links — reads as "attach / link context".
    return <svg {...common}><path d="M10 13a4 4 0 0 0 5.66 0l2-2a4 4 0 0 0-5.66-5.66l-1.1 1.1" /><path d="M14 11a4 4 0 0 0-5.66 0l-2 2a4 4 0 0 0 5.66 5.66l1.1-1.1" /></svg>;
  }
  if (name === 'add') {
    // Plus glyph — "add a thing".
    return <svg {...common}><path d="M12 5.75v12.5" /><path d="M5.75 12h12.5" /></svg>;
  }
  return <svg {...common}><path d="M4.75 12 19.25 5.75 13 20.25l-1.65-6.15L4.75 12Z" /><path d="m11.35 14.1 3.9-3.9" /></svg>;
}
