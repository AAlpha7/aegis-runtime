/**
 * Plan C — tab signals: static guards for NavButton badge/dot, chat unseen-dot
 * guard logic, Work count wiring, and i18n parity.
 *
 * All tests read source text so they run without a build step. The render-level
 * check (badge/dot markup in rendered HTML) is in vnextChatHomeRender.test.mjs.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

const appSource = readFileSync(new URL('../src/vnext/App.tsx', import.meta.url), 'utf8');
const catalogSource = readFileSync(new URL('../src/i18n/catalog.ts', import.meta.url), 'utf8');
const stylesSource = readFileSync(new URL('../src/vnext/styles.css', import.meta.url), 'utf8');

describe('Plan C — NavButton badge/dot props', () => {
  it('NavButton accepts badgeCount, dot, and badgeAria props', () => {
    assert.match(appSource, /badgeCount\?: number/);
    assert.match(appSource, /dot\?: boolean/);
    assert.match(appSource, /badgeAria\?: string/);
  });

  it('NavButton renders count pill when badgeCount > 0', () => {
    assert.match(appSource, /badgeCount > 0/);
    // classNames helper is used: classNames('vnext-nav-badge', ...)
    assert.match(appSource, /classNames\('vnext-nav-badge/);
  });

  it('NavButton renders dot span when dot is true', () => {
    assert.match(appSource, /dot && <span className="vnext-nav-dot"/);
  });

  it('count pill has aria-hidden="true"', () => {
    // The badge is visual; the badgeAria on the button carries screen-reader info.
    assert.match(appSource, /classNames\('vnext-nav-badge[\s\S]{0,120}aria-hidden="true"/);
  });

  it('dot has aria-hidden="true"', () => {
    assert.match(appSource, /className="vnext-nav-dot"[\s\S]{0,60}aria-hidden="true"/);
  });

  it('CSS: .vnext-nav-badge rule exists', () => {
    assert.match(stylesSource, /\.vnext-nav-badge\s*\{/);
    assert.match(stylesSource, /\.vnext-nav-badge\s*\{[\s\S]*?border-radius: 999px/);
  });

  it('CSS: .vnext-nav-badge.warn rule for decision-heavy count', () => {
    assert.match(stylesSource, /\.vnext-nav-badge\.warn\s*\{/);
  });

  it('CSS: .vnext-nav-dot rule exists with calm accent color', () => {
    assert.match(stylesSource, /\.vnext-nav-dot\s*\{/);
    // Must use brand/accent color, not warning or error
    const dotBlock = stylesSource.slice(
      stylesSource.indexOf('.vnext-nav-dot {'),
      stylesSource.indexOf('}', stylesSource.indexOf('.vnext-nav-dot {')),
    );
    assert.match(dotBlock, /--aegis-accent|--aegis-brand/);
  });
});

describe('Plan C — Work count wiring', () => {
  it('Work NavButton gets badgeCount=workBadgeCount in bottom nav', () => {
    assert.match(appSource, /className="vnext-nav"[\s\S]*?view="work"[\s\S]*?badgeCount=\{workBadgeCount\}/);
  });

  it('Work NavButton gets badgeCount=workBadgeCount in rail nav', () => {
    assert.match(appSource, /className="vnext-rail-nav"[\s\S]*?view="work"[\s\S]*?badgeCount=\{workBadgeCount\}/);
  });

  it('Work NavButton gets badgeAria wired to workBadgeLabel in both navs', () => {
    // The ternary: workBadgeCount > 0 ? workBadgeLabel : undefined
    const matches = [...appSource.matchAll(/view="work"[\s\S]{0,500}?workBadgeLabel/g)];
    assert.ok(matches.length >= 2, `Expected at least 2 Work NavButton badgeAria wires, got ${matches.length}`);
  });

  it('Work NavButton badgeTone is warn when decisionCount > 0', () => {
    assert.match(appSource, /decisionCount > 0[\s\S]{0,60}?'warn'|'warn'[\s\S]{0,80}?decisionCount/);
  });
});

describe('Plan C — Mail client removed from shell', () => {
  it('does not keep mail unread state or a mail nav destination', () => {
    assert.doesNotMatch(appSource, /mailUnreadCount/);
    assert.doesNotMatch(appSource, /view="mail"/);
    assert.doesNotMatch(appSource, /MailView/);
  });
});

describe('Plan C — Chat unseen-dot', () => {
  it('App has unseenChatActivity state', () => {
    assert.match(appSource, /const \[unseenChatActivity, setUnseenChatActivity\] = useState\(false\)/);
  });

  it('unseenChatActivity uses a derived-signal effect keyed on chatHomeMessages and view', () => {
    // lastSeenChatAtRef is the high-water mark of aegis messages the owner has seen.
    assert.match(appSource, /lastSeenChatAtRef = useRef<number>/);
    // latestAegisAt is computed from chatHomeMessages role==='aegis' timestamps.
    assert.match(appSource, /latestAegisAt/);
    // Effect dependency array covers both chatHomeMessages and view.
    assert.match(appSource, /\[chatHomeMessages, view\]/);
    // When view === 'home': update the seen-mark and clear the dot.
    assert.match(appSource, /view === 'home'[\s\S]{0,400}?setUnseenChatActivity\(false\)/);
    // When view !== 'home': light the dot when a new aegis message arrived.
    assert.match(appSource, /setUnseenChatActivity\(latestAegisAt > lastSeenChatAtRef\.current\)/);
  });

  it('Chat NavButton gets dot=unseenChatActivity in bottom nav', () => {
    assert.match(appSource, /className="vnext-nav"[\s\S]*?view="home"[\s\S]*?dot=\{unseenChatActivity\}/);
  });

  it('Chat NavButton gets dot=unseenChatActivity in rail nav', () => {
    assert.match(appSource, /className="vnext-rail-nav"[\s\S]*?view="home"[\s\S]*?dot=\{unseenChatActivity\}/);
  });
});

describe('Plan C — i18n keys', () => {
  it('app.shell.nav.chatUnseenAria has zh and en translations', () => {
    assert.match(catalogSource, /'app\.shell\.nav\.chatUnseenAria':[\s\S]*?zh:.*有新消息[\s\S]*?en:.*new message/);
  });

  it('chatUnseenAria is used in App.tsx for the Chat NavButton badgeAria', () => {
    assert.match(appSource, /app\.shell\.nav\.chatUnseenAria/);
  });
});
