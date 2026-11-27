import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

const chatHomeSource = readFileSync(new URL('../src/vnext/ChatHomeView.tsx', import.meta.url), 'utf8');
const appSource = readFileSync(new URL('../src/vnext/App.tsx', import.meta.url), 'utf8');
const chatWorkStateSource = readFileSync(new URL('../src/vnext/chatWorkState.ts', import.meta.url), 'utf8');
const errorSource = readFileSync(new URL('../src/userFacingErrors.ts', import.meta.url), 'utf8');
const settingsSource = readFileSync(new URL('../src/vnext/SettingsView.tsx', import.meta.url), 'utf8');
const stylesSource = readFileSync(new URL('../src/vnext/styles.css', import.meta.url), 'utf8');

function cssRuleBlock(source, selector) {
  const index = source.indexOf(`${selector} {`);
  assert.notEqual(index, -1, `missing CSS rule ${selector}`);
  return cssRuleBlockAt(source, selector, index);
}

function cssRuleBlockAt(source, selector, index) {
  const start = source.indexOf('{', index);
  let depth = 0;
  for (let cursor = start; cursor < source.length; cursor += 1) {
    if (source[cursor] === '{') depth += 1;
    if (source[cursor] === '}') {
      depth -= 1;
      if (depth === 0) return source.slice(start + 1, cursor);
    }
  }
  throw new Error(`unterminated CSS rule ${selector}`);
}

function mobileCssRuleBlock(source, selector) {
  const escapedSelector = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const selectorPattern = new RegExp(`(?:^|\\n)\\s*${escapedSelector}\\s*\\{`, 'g');
  let selectorIndex = -1;
  for (const match of source.matchAll(selectorPattern)) {
    selectorIndex = (match.index ?? 0) + match[0].indexOf(selector);
  }
  assert.notEqual(selectorIndex, -1, `missing mobile CSS rule ${selector}`);
  const mediaIndex = source.lastIndexOf('@media (max-width: 820px)', selectorIndex);
  assert.notEqual(mediaIndex, -1, 'missing mobile media query');
  return cssRuleBlockAt(source, selector, selectorIndex);
}

describe('vNext chat home view', () => {
  it('positions the browser surface as the primary Aegis home, not a Telegram comparison', () => {
    assert.match(chatHomeSource, /chatHome\.hero\.tellAegis/);
    assert.match(chatHomeSource, /chatHome\.hero\.ready/);
    assert.match(chatHomeSource, /brief\?: string/);
    assert.match(chatHomeSource, /compactHomeBrief\(brief, tr\)/);
    assert.match(chatHomeSource, /className="vnext-chat-brief-card"/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.hero\.openBrief'\)\}/);
    assert.match(chatHomeSource, /chatHome\.brief\.decisionWaiting/);
    assert.match(chatHomeSource, /chatHome\.brief\.startHere/);
    assert.match(chatHomeSource, /handleSignal\(leadSignal\)/);
    assert.match(chatHomeSource, /onSuggestion\('我现在有什么需要注意的吗？'\)/);
    assert.match(chatHomeSource, /function compactHomeBrief/);
    assert.match(chatHomeSource, /chatHome\.brief\.quietRightNow/);
    assert.match(chatHomeSource, /<VNextIcon name=\{prompt\.icon\} \/>/);
    assert.match(chatHomeSource, /workBadgeCount/);
    assert.match(chatHomeSource, /workBadgeLabel/);
    assert.doesNotMatch(chatHomeSource, /onOpenMail/);
    assert.doesNotMatch(chatHomeSource, /mailUnreadCount/);
    // Peers live in bottom nav / brand rail only (not duplicated in chat topbar).
    assert.doesNotMatch(chatHomeSource, /vnext-chat-surface-nav/);
    assert.match(chatHomeSource, /vnext-chat-needs-you/);
    assert.match(chatHomeSource, /resolveCeoShorthand/);
    assert.match(chatHomeSource, /className="vnext-chat-starters"/);
    assert.match(chatHomeSource, /onOpenWorkDetail/);
    assert.match(chatHomeSource, /contextSignals = \[\]/);
    assert.match(chatHomeSource, /showHeroBrief/);
    assert.match(chatHomeSource, /leadContextSignal = !hasConversation && showHeroBrief \? contextSignals\[0\] : null/);
    assert.match(chatHomeSource, /visibleContextSignals = leadContextSignal \? contextSignals\.slice\(1\) : contextSignals/);
    assert.match(chatHomeSource, /visibleSignals = visibleContextSignals\.slice\(0, 3\)/);
    assert.match(chatHomeSource, /hiddenSignalCount = Math\.max\(visibleContextSignals\.length - visibleSignals\.length, 0\)/);
    assert.doesNotMatch(chatHomeSource, /function compactSignalSummary/);
    assert.match(chatHomeSource, /<strong>\{signal\.summary\}<\/strong>/);
    assert.doesNotMatch(chatHomeSource, /signal\.summary \+.*decision/);
    assert.match(chatHomeSource, /const signals = showAll \? visibleContextSignals : visibleSignals/);
    assert.match(chatHomeSource, /const overflowCount = showAll \? 0 : hiddenSignalCount/);
    assert.match(chatHomeSource, /overflowCount > 0/);
    assert.match(chatHomeSource, /overflowCount === 1 \? 'chatHome\.context\.openMoreOne' : 'chatHome\.context\.openMoreMany'/);
    assert.match(chatHomeSource, /className="vnext-chat-context-more"/);
    assert.match(chatHomeSource, /overflowCount === 1 \? 'chatHome\.context\.workItemsOne' : 'chatHome\.context\.workItemsMany'/);
    assert.doesNotMatch(chatHomeSource, /\{overflowCount\} more item/);
    assert.match(chatHomeSource, /onClick=\{\(\) => onOpenWork\(\)\}/);
    assert.match(chatHomeSource, /aria-label=\{mode === 'attach' \? tr\.t\('chatHome\.context\.attachable'\) : tr\.t\('chatHome\.context\.active'\)\}/);
    assert.doesNotMatch(chatHomeSource, /hasConversation && renderContextSignals\(true\)/);
    assert.doesNotMatch(chatHomeSource, /vnext-chat-starters compact/);
    assert.match(chatHomeSource, /className="vnext-chat-context-rail"/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.context\.desktop'\)\}/);
    assert.match(chatHomeSource, /chatHome\.context\.nearbyWork/);
    assert.match(chatHomeSource, /hasConversation && visibleSignals\.length > 0/);
    assert.match(chatHomeSource, /handleNeedsYouAction|dismissNeedsYouBanner/);
    assert.match(chatHomeSource, /vnext-chat-needs-you-banner/);
    assert.match(chatHomeSource, /renderNeedsYouBanner/);
    assert.match(chatHomeSource, /renderContextSignals\('attach', true\)/);
    assert.match(chatHomeSource, /onClick=\{\(\) => onOpenWork\(\)\}/);
    assert.match(chatHomeSource, /workBadgeTone/);
    // The Dev Harness is fully removed: no harness prop or menu entry may return.
    assert.doesNotMatch(chatHomeSource, /onOpenHarness/);
    assert.doesNotMatch(chatHomeSource, /harnessActiveCount/);
    assert.match(chatHomeSource, /chatHome\.composer\.attachContext/);
    assert.doesNotMatch(chatHomeSource, /costSavingPaused/);
    // The composer mic records a VOICE NOTE now (audio -> server-side STT);
    // the old dictation label is gone on purpose.
    assert.doesNotMatch(chatHomeSource, /chatHome\.composer\.voiceInput/);
    assert.match(chatHomeSource, /chatHome\.voiceNote\.record/);
    assert.match(chatHomeSource, /chatHome\.composer\.placeholder/);
    assert.match(chatHomeSource, /statusText=\{busy \? tr\.t\('chatHome\.session\.working'\)/);
    assert.doesNotMatch(chatHomeSource, /className="vnext-chat-status-button"/);
    assert.doesNotMatch(chatHomeSource, /className="vnext-chat-settings-button"/);
    // Shared OwnerShellChrome owns the topbar + overflow menu (Status/Settings items).
    assert.match(chatHomeSource, /OwnerShellChrome/);
    assert.match(chatHomeSource, /menuItems=\{homeMenuItems\}/);
    assert.match(chatHomeSource, /menuTriggerLabel=\{tr\.t\('chatHome\.account\.menu'\)\}/);
    // Utility peers come from ownerUtilityMenuItemIds(); status/settings keep the same handlers.
    assert.match(chatHomeSource, /id === 'status'[\s\S]*?label: tr\.t\('chatHome\.account\.aegisStatus'\)[\s\S]*?onSelect: onOpenStatus/);
    assert.match(chatHomeSource, /label: tr\.t\('chatHome\.account\.settings'\)[\s\S]*?onSelect: onOpenSettings/);
    assert.doesNotMatch(chatHomeSource, /vnext-chat-account-menu/);
    assert.doesNotMatch(stylesSource, /vnext-chat-status-button/);
    assert.doesNotMatch(stylesSource, /vnext-chat-settings-button/);
    // Identity is a static label (no dropdown trigger).
    assert.match(chatHomeSource, /className="vnext-chat-identity-label"/);
    assert.doesNotMatch(chatHomeSource, /aria-expanded=\{accountOpen\}/);
    assert.match(chatHomeSource, /avatarProfile: AvatarProfile/);
    assert.match(chatHomeSource, /<AegisAvatar src=\{avatarProfile\.aegisAvatarUrl\} className="vnext-chat-mark" \/>/);
    assert.match(chatHomeSource, /<OwnerAvatar src=\{avatarProfile\.ownerAvatarUrl\} email=\{email\} className="vnext-chat-avatar" \/>/);
    assert.match(chatHomeSource, /vnext-chat-message-author/);
    assert.match(chatHomeSource, /timestamp\?: number/);
    assert.match(chatHomeSource, /responseTimestamp\?: number/);
    assert.match(chatHomeSource, /function MessageTime/);
    assert.match(chatHomeSource, /chatFullTimestampLabel/);
    assert.match(chatHomeSource, /const \[expanded, setExpanded\] = useState\(false\)/);
    assert.match(chatHomeSource, /className="vnext-chat-time"/);
    assert.match(chatHomeSource, /role="button"/);
    assert.match(chatHomeSource, /tabIndex=\{0\}/);
    assert.match(chatHomeSource, /onClick=\{\(\) => setExpanded\(\(current\) => !current\)\}/);
    assert.match(chatHomeSource, /event\.key !== 'Enter' && event\.key !== ' '/);
    assert.match(chatHomeSource, /<MessageTime timestamp=\{message\.timestamp\} \/>/);
    assert.match(chatHomeSource, /<MessageTime timestamp=\{entry\.timestamp\} \/>/);
    assert.match(chatHomeSource, /<MessageTime timestamp=\{entry\.responseTimestamp\} \/>/);
    assert.match(chatHomeSource, /function MessageCite/);
    assert.match(chatHomeSource, /data-message-ref=\{ref\}/);
    assert.match(chatHomeSource, /copyMessageRef\(token\)/);
    assert.match(chatHomeSource, /hiddenNotices: CollapsedNoticeItem\[\]/);
    assert.match(chatHomeSource, /function collapsedNoticeItem/);
    assert.match(chatHomeSource, /className="vnext-chat-repeat-list"/);
    assert.match(chatHomeSource, /aria-label=\{tr\.f\('chatHome\.collapsed\.earlierLabel', \{ label \}\)\}/);
    assert.match(chatHomeSource, /notice\.hiddenNotices\.map/);
    assert.match(chatHomeSource, /<RichText content=\{item\.text\} importanceSeverity=\{item\.severity\} importanceDismissKey=\{item\.id\} \/>/);
    assert.match(stylesSource, /\.vnext-chat-message-author \.vnext-chat-time/);
    assert.match(stylesSource, /\.vnext-chat-answer-heading \.vnext-chat-time/);
    assert.match(stylesSource, /\.vnext-chat-work-response-label \.vnext-chat-time,\s*\.vnext-chat-work-question \.vnext-chat-time/);
    assert.match(stylesSource, /\.vnext-chat-repeat-list/);
    assert.match(stylesSource, /\.vnext-chat-repeat-item/);
    assert.match(stylesSource, /\.vnext-rich-callout\.important/);
    // Appearance/Avatars copy and the avatar image-validation messages were
    // migrated to the i18n catalog (settings.appearance.* / settings.avatar.*);
    // assert the catalog-key wiring rather than the now-removed raw literals.
    assert.match(settingsSource, /tr\('settings\.appearance\.heading'\)/);
    assert.match(settingsSource, /tr\('settings\.avatar\.chooseImage'\)/);
    assert.match(settingsSource, /accept="image\/png,image\/jpeg,image\/webp"/);
    assert.match(settingsSource, /tr\('settings\.avatar\.usePngJpegWebp'\)/);
    assert.match(settingsSource, /tr\('settings\.avatar\.under8mb'\)/);
    assert.match(settingsSource, /canvas\.toDataURL\('image\/jpeg', 0\.88\)/);
    assert.match(stylesSource, /\.vnext-avatar-settings-grid/);
    assert.match(stylesSource, /\.vnext-chat-message-author/);
    // SP2: the kicker rule is now a grouped selector (.vnext-chat-identity +
    // .vnext-chat-identity-label), so tolerate the comma group before the brace.
    assert.match(stylesSource, /\.vnext-chat-identity \.vnext-kicker[\s\S]*?\{[^}]*white-space: nowrap;[^}]*text-overflow: ellipsis;/s);
    // SP2: the mobile kicker-hide rule is now a grouped selector (.vnext-chat-identity
    // + .vnext-chat-identity-label), so tolerate the comma group before the brace.
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-chat-identity \.vnext-kicker[\s\S]*?\{[^}]*display: none;/s);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.voiceNote\.record'\)\}/);
    assert.match(chatHomeSource, /title=\{tr\.t\('chatHome\.voiceNote\.record'\)\}/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.composer\.attachContext'\)\}/);
    assert.match(chatHomeSource, /title=\{tr\.t\('chatHome\.composer\.attachContext'\)\}/);
    assert.doesNotMatch(chatHomeSource, /className="vnext-chat-voice"[\s\S]{0,220}disabled=\{busy\}/);
    assert.doesNotMatch(chatHomeSource, /className="vnext-chat-context-button"[\s\S]{0,220}disabled=\{busy\}/);
    assert.match(chatHomeSource, /className="vnext-chat-context-button"/);
    assert.match(chatHomeSource, /aria-expanded=\{contextPickerOpen\}/);
    assert.match(chatHomeSource, /setContextPickerOpen\(\(open\) => !open\)/);
    assert.match(chatHomeSource, /className="vnext-chat-context-picker"/);
    assert.match(chatHomeSource, /chatHome\.context\.chooseConsider/);
    assert.match(chatHomeSource, /onAttachContext/);
    assert.match(chatHomeSource, /attachedContext/);
    assert.match(chatHomeSource, /chatHome\.composer\.removeAttached/);
    assert.match(chatHomeSource, /tr\.f\('chatHome\.message\.using', \{ label: attachedContext\.label \}\)/);
    assert.match(chatHomeSource, /renderContextSignals\('attach', true\)/);
    assert.doesNotMatch(chatHomeSource, /Open full Work/);
    // Snapshot and pause-checks moved into Settings. Clear stays in the menu.
    assert.doesNotMatch(chatHomeSource, /className="vnext-chat-snapshot"/);
    assert.doesNotMatch(chatHomeSource, /id: 'snapshot'/);
    assert.doesNotMatch(chatHomeSource, /chatHome\.account\.pauseChecks/);
    assert.match(settingsSource, /data-settings-action="support-snapshot"/);
    assert.match(settingsSource, /chatHome\.account\.saveSnapshot/);
    assert.match(settingsSource, /onClick=\{onCreateSnapshot\}/);
    assert.match(settingsSource, /tr\('settings\.spending\.pauseTitle'\)/);
    assert.match(chatHomeSource, /installOffered/);
    assert.match(chatHomeSource, /onInstallOwnerApp/);
    assert.doesNotMatch(chatHomeSource, /disabled=\{busy\}[\s\S]{0,120}onInstallOwnerApp\(\);/);
    assert.match(chatHomeSource, /id: 'install',\s*label: tr\.t\('install\.action\.installOnDevice'\)/);
    assert.match(chatHomeSource, /chatHome\.account\.aegisStatus/);
    assert.match(chatHomeSource, /chatHome\.account\.settings/);
    assert.match(chatHomeSource, /chatHome\.account\.signOut/);
    // SP2: the open/dismiss behavior (toggle + click-outside + Escape) is now
    // owned by the shared VNextHeaderMenu, not hand-rolled in ChatHomeView.
    assert.match(chatHomeSource, /import \{ OwnerShellChrome(?:, type OwnerShellSummon)? \} from '\.\/OwnerShellChrome'/);
    assert.match(chatHomeSource, /const homeMenuItems: VNextHeaderMenuItem\[\] =/);
    assert.doesNotMatch(chatHomeSource, /accountRef/);
    assert.match(appSource, /onOpenWork=\{\(itemId\) => navigate\('work', itemId \?\? selectedId\)\}/);
    assert.match(chatHomeSource, /onSuggestion: \(prompt: string, displayText\?: string\) => void;/);
    assert.match(chatHomeSource, /onSuggestion\(prompt\.prompt, promptLabel\)/);
    assert.doesNotMatch(chatHomeSource, /STARTER_PROMPTS\.map[\s\S]*?disabled=\{busy\}[\s\S]*?onSuggestion\(prompt\.prompt, promptLabel\)/);
    assert.doesNotMatch(chatHomeSource, /signals\.map[\s\S]*?disabled=\{busy\}[\s\S]*?handleSignal/);
    assert.match(appSource, /displayTextOverride\?: string/);
    assert.match(appSource, /const displayText = displayTextOverride\?\.trim\(\)\n\s*\|\| text/);
    assert.match(appSource, /text: displayText/);
    assert.match(appSource, /deskTalkToAegis/);
    assert.match(appSource, /onSuggestion=\{\(prompt, displayText\) => \{[\s\S]{0,400}?submitChatHomeCommand\(runtime, undefined, undefined, 'text', displayText\?\.trim\(\) \|\| undefined\);/);
    assert.match(appSource, /void submitChatHomeCommand\(trimmed \|\| undefined, undefined, undefined, mode\)/);
    assert.doesNotMatch(appSource, /openAndAsk/);
    assert.match(appSource, /const decisionCount = lanes\.find/);
    assert.match(appSource, /const readyCount = lanes\.find/);
    assert.match(appSource, /workBadgeCount=\{workBadgeCount\}/);
    assert.match(appSource, /workBadgeTone=\{decisionCount > 0 \? 'warn' : 'neutral'\}/);
    assert.doesNotMatch(appSource, /harnessActiveCount/);
    assert.doesNotMatch(appSource, /onOpenHarness/);
    assert.match(appSource, /onOpenWorkDetail=\{\(item\) => openChatHomeWorkDetail\(item\)\}/);
    assert.match(appSource, /brief=\{home\.brief\}/);
    assert.match(appSource, /contextSignals=\{home\.signals\}/);
    assert.match(appSource, /onOpenStatus=\{\(\) => navigate\('status', null\)\}/);
    assert.match(appSource, /onCreateSnapshot=\{\(\) => void createDebugSnapshot\(\)\}/);
    assert.match(appSource, /installOffered=\{installStatus !== 'installed'\}/);
    assert.match(appSource, /onInstallOwnerApp=\{\(\) => void installOwnerApp\(\)\}/);
    assert.doesNotMatch(appSource, /onPauseSelfStartedWork=/);
    assert.match(appSource, /onPauseCostSaving=\{\(\) => void pauseCostSavingControls\(\)\}/);
    assert.match(appSource, /onResumeCostSaving=\{\(\) => void resumeCostSavingControls\(\)\}/);
    assert.match(appSource, /resumeVNextCostSaving/);
    assert.match(settingsSource, /onResumeCostSaving/);
    assert.match(settingsSource, /if \(checked\) onPauseCostSaving\(\);\s*else onResumeCostSaving\(\);/);
    assert.match(settingsSource, /tr\('settings\.spending\.pauseTitle'\)/);
    assert.match(appSource, /onLogout=\{\(\) => void logoutNow\(\)\}/);
    assert.doesNotMatch(chatHomeSource, /TRUST_SIGNALS/);
    assert.doesNotMatch(chatHomeSource, /vnext-chat-trust-strip/);
    assert.doesNotMatch(chatHomeSource, /Context-aware\. Permissioned\. Receipt-backed\./);
    assert.doesNotMatch(chatHomeSource, /OPERATING_FLOW/);
    assert.doesNotMatch(chatHomeSource, /vnext-chat-operating-rail/);
    assert.doesNotMatch(chatHomeSource, /Aegis request path/);
    assert.doesNotMatch(chatHomeSource, /Same Aegis as Telegram/);
    assert.doesNotMatch(chatHomeSource, /runtime as Telegram/);
    assert.match(stylesSource, /\.vnext-chat-brief-card\s*\{/);
    assert.match(stylesSource, /\.vnext-chat-brief-card strong\s*\{[^}]*text-overflow: ellipsis;[^}]*white-space: nowrap;/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-chat-brief-card strong\s*\{[^}]*font-size: 13px;/s);
  });

  it('keeps context signals actionable instead of rendering inert count tiles', () => {
    assert.match(chatHomeSource, /function handleSignal\(signal: VNextHomeSignal\)/);
    assert.match(chatHomeSource, /signal\.id === 'sources'[\s\S]*onOpenSettings\(\)/);
    assert.match(chatHomeSource, /signal\.id === 'quiet'[\s\S]*onSuggestion\('我现在有什么需要注意的吗？'\)/);
    assert.match(chatHomeSource, /onOpenWork\(signal\.targetItemId\)/);
    assert.match(chatHomeSource, /function handleAttachSignal\(signal: VNextHomeSignal\)/);
    assert.match(chatHomeSource, /mode === 'attach' \? handleAttachSignal\(signal\) : handleSignal\(signal\)/);
    assert.match(chatHomeSource, /role: 'owner' \| 'aegis' \| 'working' \| 'queued'/);
    assert.match(chatHomeSource, /message\.role === 'queued'/);
    assert.match(chatHomeSource, /vnext-chat-message-state/);
    assert.match(stylesSource, /\.vnext-chat-context-strip/);
    assert.doesNotMatch(stylesSource, /\.vnext-chat-context-strip\.compact/);
    assert.doesNotMatch(stylesSource, /\.vnext-chat-starters\.compact/);
    assert.match(stylesSource, /\.vnext-chat-context-strip button\.tone-warn/);
    assert.match(stylesSource, /\.vnext-chat-context-strip strong/);
    assert.match(stylesSource, /\.vnext-chat-message\.owner,\s*\.vnext-chat-message\.queued\s*\{[^}]*min-width: 96px;[^}]*overflow-wrap: break-word;[^}]*word-break: normal;/s);
    assert.match(stylesSource, /\.vnext-chat-message\.owner p,\s*\.vnext-chat-message\.queued p\s*\{[^}]*white-space: pre-wrap;/s);
    assert.match(stylesSource, /\.vnext-rich-text\s*\{[^}]*max-width: 100%;[^}]*overflow-wrap: anywhere;[^}]*word-break: normal;/s);
    assert.match(stylesSource, /\.vnext-rich-callout\s*\{[^}]*min-width: 0;/s);
    assert.match(stylesSource, /\.vnext-chat-answer-card\s*\{[^}]*min-width: 0;[^}]*max-width: 100%;/s);
    assert.match(stylesSource, /\.vnext-chat-message\.queued/);
    assert.match(stylesSource, /\.vnext-chat-message-state/);
    assert.match(stylesSource, /\.vnext-chat-context-strip button::after/);
    assert.match(stylesSource, /\.vnext-chat-context-strip button\.vnext-chat-context-more/);
    const moreContextButton = cssRuleBlock(stylesSource, '.vnext-chat-context-strip button.vnext-chat-context-more');
    assert.doesNotMatch(moreContextButton, /border-style: dashed;/);
    assert.match(stylesSource, /\.vnext-chat-context-rail/);
    assert.match(stylesSource, /\.vnext-chat-context-picker/);
    assert.match(stylesSource, /\.vnext-chat-home\.has-conversation\.has-context-picker/);
    assert.match(stylesSource, /\.vnext-chat-home\.has-conversation/);
    assert.match(stylesSource, /\.vnext-chat-home\.has-conversation\s*\{[\s\S]*grid-template-columns: minmax\(0, 1fr\) minmax\(280px, 330px\);/);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-chat-context-rail\s*\{[\s\S]*display: none;/);
    assert.match(stylesSource, /border-style: dashed;/);
    assert.match(stylesSource, /\.vnext-chat-context-strip button::after\s*\{[\s\S]*content: "";[\s\S]*border-right: 2px solid currentColor;[\s\S]*border-bottom: 2px solid currentColor;[\s\S]*transform: rotate\(-45deg\);/);
    assert.match(stylesSource, /\.vnext-chat-context-strip button:hover,\s*\.vnext-chat-context-strip button:focus-visible/);
    assert.match(stylesSource, /cursor: pointer;/);
  });

  it('keeps long source URLs in collapsible evidence instead of overflowing chat text', () => {
    assert.match(chatHomeSource, /if \(!label\) return false;/);
    assert.match(chatHomeSource, /if \(\/\^https\?:\\\/\\\/\/i\.test\(label\)\) return label\.length <= 2048;/);
    assert.match(chatHomeSource, /if \(label\.length > 80\) return false;/);
    assert.match(stylesSource, /\.vnext-answer-card\s*\{[^}]*min-width: 0;[^}]*max-width: 100%;/s);
    assert.match(stylesSource, /\.vnext-source-evidence-row summary\s*\{[^}]*min-width: 0;[^}]*max-width: 100%;/s);
    assert.match(stylesSource, /\.vnext-source-evidence-row strong\s*\{[^}]*overflow-wrap: anywhere;[^}]*word-break: break-word;/s);
    assert.match(stylesSource, /\.vnext-source-evidence-row em\s*\{[^}]*overflow-wrap: anywhere;[^}]*word-break: break-word;/s);
  });

  it('uses a multiline composer for real assistant instructions', () => {
    assert.match(chatHomeSource, /useRef<HTMLTextAreaElement/);
    assert.match(chatHomeSource, /<textarea/);
    assert.match(chatHomeSource, /rows=\{1\}/);
    assert.match(chatHomeSource, /function resizeComposerInput\(input: HTMLTextAreaElement \| null, mode: ComposerMode = 'line'\)/);
    assert.match(chatHomeSource, /input\.style\.height = 'auto'/);
    assert.match(chatHomeSource, /Math\.min\(input\.scrollHeight, maxHeight\)/);
    assert.match(chatHomeSource, /resizeComposerInput\(event\.currentTarget, composerMode\);/);
    assert.match(chatHomeSource, /resizeComposerInput\(inputRef\.current, composerMode\);/);
    assert.match(chatHomeSource, /function isImeEnter\(event: KeyboardEvent\): boolean/);
    assert.match(chatHomeSource, /event\.isComposing \|\| event\.keyCode === 229/);
    assert.match(chatHomeSource, /if \(event\.key !== 'Enter' \|\| event\.shiftKey \|\| isImeEnter\(event\.nativeEvent\)\) return;/);
    assert.match(chatHomeSource, /chatHome\.composer\.busyPlaceholder/);
    assert.match(chatHomeSource, /function submitComposer\(\)/);
    assert.match(chatHomeSource, /submitComposer\(\);/);
    // Send stays live for an attachment-only turn (voice note / image / PDF
    // with no typed text) — that is the fix for "attaching an image does nothing".
    // An empty field with nothing attached is disabled, with a visible reason.
    assert.match(chatHomeSource, /const sendDisabled = attachmentBlockingSend \|\| \(!value\.trim\(\) && !canSendAttachmentOnly\)/);
    assert.match(chatHomeSource, /disabled=\{sendDisabled\}/);
    assert.match(chatHomeSource, /chatHome\.composer\.needText/);
    assert.match(chatHomeSource, /className="vnext-chat-composer-send-reason"/);
    assert.match(chatHomeSource, /aria-label=\{busy \? tr\.t\('chatHome\.composer\.queueForAegis'\) : tr\.t\('chatHome\.composer\.sendToAegis'\)\}/);
    assert.match(chatHomeSource, /\{busy \? tr\.t\('chatHome\.composer\.queue'\) : tr\.t\('chatHome\.composer\.send'\)\}/);
    assert.match(appSource, /type QueuedChatHomeCommand =/);
    assert.match(appSource, /type QueuedChatHomeWorkPrompt =/);
    assert.match(appSource, /queuedChatHomeCommands/);
    assert.match(appSource, /queuedChatHomeWorkPrompts/);
    assert.match(appSource, /const MAX_CHAT_HOME_QUEUED_COMMANDS = 4;/);
    assert.match(appSource, /const MAX_CHAT_HOME_QUEUED_WORK_PROMPTS = 4;/);
    assert.match(appSource, /queuedChatHomeCommands\.length >= MAX_CHAT_HOME_QUEUED_COMMANDS/);
    assert.match(appSource, /tf\('chatHome\.notice\.queueFullCommands', errorLocale/);
    assert.doesNotMatch(appSource, /\.slice\(-4\)/);
    assert.match(appSource, /queuedMessageId/);
    assert.match(appSource, /function queueChatHomeCommand/);
    assert.match(appSource, /setChatHomeNotice\(t\('chatHome\.notice\.queued', errorLocale\)\)/);
    assert.match(appSource, /if \(busy && !queuedOwnerMessageId\) \{[\s\S]*queueChatHomeCommand\(prompt, contextOverride, modeOverride, displayTextOverride, retryIdempotencyKey, quoteOverride, queueAttachments\)/);
    assert.match(appSource, /role: 'queued'/);
    assert.match(appSource, /return \{ \.\.\.message, role: 'owner' as const, turnId: commandIdempotencyKey \};/);
    assert.match(appSource, /setQueuedChatHomeCommands\(remaining\)/);
    assert.match(chatHomeSource, /function isNearStreamBottom\(stream: HTMLElement\): boolean/);
    assert.match(chatHomeSource, /streamNearBottomRef = useRef\(true\)/);
    assert.match(chatHomeSource, /streamNearBottomRef\.current = isNearStreamBottom\(event\.currentTarget\)/);
    assert.match(chatHomeSource, /const shouldAutoScroll =[\s\S]*streamNearBottomRef\.current[\s\S]*latestRole === 'owner'[\s\S]*latestRole === 'working'/);
    assert.match(chatHomeSource, /if \(!shouldAutoScroll\) return;/);
    assert.match(chatHomeSource, /focus\(\{ preventScroll: true \}\)/);
    assert.match(stylesSource, /grid-template-columns: auto auto auto auto minmax\(0, 1fr\) auto;/);
    assert.match(chatHomeSource, /type ComposerMode = 'line' \| 'half'/);
    assert.match(chatHomeSource, /nextComposerMode/);
    assert.match(chatHomeSource, /composerSizeActionLabel/);
    assert.match(chatHomeSource, /chatHome\.composer\.expand/);
    assert.match(chatHomeSource, /chatHome\.composer\.collapse/);
    assert.doesNotMatch(chatHomeSource, /Full screen/);
    assert.match(chatHomeSource, /bottomRef\.current\?\.scrollIntoView\(\{ block: 'end', behavior \}\)/);
    const mainComposerTextarea = chatHomeSource.match(/<textarea[\s\S]*?aria-label=\{tr\.t\('chatHome\.composer\.tellAria'\)\}[\s\S]*?\/>/)?.[0] ?? '';
    assert.match(mainComposerTextarea, /aria-label=\{tr\.t\('chatHome\.composer\.tellAria'\)\}/);
    assert.doesNotMatch(mainComposerTextarea, /disabled=\{busy\}/);
    // The tellAria field must be a <textarea>, never an <input>. Scope the match
    // to a single tag (no '>' between <input and the attr) so the unrelated Slice 5
    // file-picker <input type="file"> earlier in the form is not a false positive.
    assert.doesNotMatch(chatHomeSource, /<input[^>]*aria-label=\{tr\.t\('chatHome\.composer\.tellAria'\)\}/);
  });

  it('keeps failed runtime turns recoverable inside the chat surface', () => {
    assert.match(chatHomeSource, /retryText\?: string/);
    assert.match(chatHomeSource, /retryIdempotencyKey\?: string/);
    assert.match(chatHomeSource, /retryLabel\?: string/);
    assert.match(chatHomeSource, /retryContext\?: ChatHomeAttachedContext/);
    assert.match(chatHomeSource, /contextLabel\?: string/);
    assert.match(chatHomeSource, /message\.result\.response_kind === 'error'/);
    assert.match(chatHomeSource, /onRetry\(message\.retryText \|\| '', message\.retryIdempotencyKey, message\.retryContext, message\.retryQuote\)/);
    assert.match(appSource, /onRetry=\{\(text, idempotencyKey, context, quote\) => \{[\s\S]*?submitChatHomeCommand\(text, idempotencyKey, context, 'text'/);
    // retryLabel is the dual-use English logic key; the displayed text is
    // localized via retryDisplayLabel so a zh session never sees raw English.
    assert.match(chatHomeSource, /retryDisplayLabel\(message\.retryLabel, tr\)/);
    assert.match(chatHomeSource, /retryLabel === 'Check result'\) return tr\.t\('chatHome\.retry\.checkResult'\)/);
    assert.match(chatHomeSource, /className="vnext-chat-message-context"/);
    assert.match(chatHomeSource, /tr\.f\('chatHome\.message\.using', \{ label: message\.contextLabel \}\)/);
    assert.match(chatHomeSource, /workItem\?: OwnerWorkItem \| null/);
    assert.match(chatHomeSource, /workItemUnavailable\?: boolean/);
    assert.match(chatHomeSource, /<ChatWorkCard/);
    assert.match(chatHomeSource, /unavailable=\{Boolean\(message\.workItemUnavailable\)\}/);
    assert.match(chatHomeSource, /ownerVisibleReasonCue/);
    assert.match(chatHomeSource, /const reasonCue = ownerVisibleReasonCue\(visibleItem, locale\)/);
    assert.match(chatHomeSource, /function workCardNextStep\(item: OwnerWorkItem, tr: Translator\)/);
    assert.match(chatHomeSource, /isClosedItem\(item\)/);
    assert.match(chatHomeSource, /chatHome\.work\.handledNote/);
    assert.match(chatHomeSource, /ownerVisiblePrimaryActionLabel\(item, openWhenUseful, 116, tr\.locale\)/);
    assert.doesNotMatch(chatHomeSource, /isMailLikeItem/);
    assert.doesNotMatch(chatHomeSource, /isGenericMailNextStep/);
    assert.match(chatHomeSource, /function workCardProofText\(item: OwnerWorkItem, tr: Translator\)/);
    assert.match(chatHomeSource, /item\.authority\?\.expected_receipt \|\| item\.receipt_expectation/);
    assert.doesNotMatch(chatHomeSource, /chatHome\.work\.gmailReceiptKept/);
    assert.match(chatHomeSource, /chatHome\.work\.noOutsideAction/);
    assert.match(chatHomeSource, /function workCardPrimaryActionLabel\(item: OwnerWorkItem, tr: Translator\)/);
    assert.match(chatHomeSource, /ownerVisiblePrimaryActionLabel\(item, tr\.t\('chatHome\.work\.open'\), 40, tr\.locale\)/);
    assert.match(chatHomeSource, /const primaryActionLabel = workCardPrimaryActionLabel\(visibleItem, tr\)/);
    assert.match(chatHomeSource, /type ChatHomeWorkDetailResult =/);
    assert.match(chatHomeSource, /workItemAnchorId\?: string \| null/);
    assert.match(chatHomeSource, /onOpenDetail: \(item: OwnerWorkItem\) => Promise<ChatHomeWorkDetailResult>/);
    assert.match(chatHomeSource, /const \[detailOpen, setDetailOpen\] = useState\(false\)/);
    assert.match(chatHomeSource, /const \[sourceDetail, setSourceDetail\] = useState<SourceDetailView \| null>\(null\)/);
    assert.match(chatHomeSource, /function toggleDetail\(\)/);
    assert.match(chatHomeSource, /void onOpenDetail\(visibleItem\)/);
    assert.match(chatHomeSource, /<SourceDetailPanel sourceDetail=\{sourceDetail\} collapsed=\{sourceDetail\.kind === 'task'\} \/>/);
    assert.match(chatHomeSource, /function webFirstPlainNoticeText/);
    assert.match(chatHomeSource, /chatHome\.plain\.sensitiveReview/);
    assert.match(chatHomeSource, /const \[localUnavailable, setLocalUnavailable\] = useState\(false\)/);
    assert.match(chatHomeSource, /const cardUnavailable = unavailable \|\| localUnavailable/);
    assert.match(chatHomeSource, /const nextStep = cardUnavailable \? tr\.t\('chatHome\.work\.refreshedHidden'\) : workCardNextStep\(visibleItem, tr\)/);
    assert.match(chatHomeSource, /const proofText = cardUnavailable \? tr\.t\('chatHome\.work\.noNewAction'\) : workCardProofText\(visibleItem, tr\)/);
    assert.match(chatHomeSource, /className="vnext-chat-work-facts"/);
    assert.match(chatHomeSource, /<em>\{tr\.t\('chatHome\.work\.next'\)\}<\/em>/);
    assert.match(chatHomeSource, /<em>\{tr\.t\('chatHome\.work\.receipt'\)\}<\/em>/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.work\.factsLabel'\)\}/);
    assert.match(chatHomeSource, /!\s*cardUnavailable && \(/);
    assert.match(chatHomeSource, /aria-expanded=\{detailOpen\}/);
    assert.match(chatHomeSource, /disabled=\{detailBusy\}/);
    // Detail toggle + Ask Aegis migrated to <VNextButton variant="secondary">
    // (leadingIcon for the glyph, label as children).
    assert.match(chatHomeSource, /<VNextButton variant="secondary" aria-expanded=\{detailOpen\} disabled=\{detailBusy\}/);
    assert.match(chatHomeSource, /\{detailActionLabel\}\s*<\/VNextButton>/);
    assert.match(chatHomeSource, /<VNextButton variant="secondary" leadingIcon=\{<VNextIcon name="spark" \/>\} onClick=\{\(\) => setAskOpen/);
    assert.match(chatHomeSource, /\{tr\.t\('chatHome\.work\.askAegis'\)\}\s*<\/VNextButton>/);
    assert.match(chatHomeSource, /const detailActionLabel = detailBusy \? tr\.t\('chatHome\.work\.opening'\) : detailOpen \? tr\.t\('chatHome\.work\.hideDetails'\) : primaryActionLabel/);
    assert.match(chatHomeSource, /const inlineActions = cardUnavailable \? \[\] : buildSupportedItemActions\(visibleItem, locale\)/);
    assert.match(chatHomeSource, /itemUnavailable\?: boolean/);
    assert.match(chatHomeSource, /isStaleWorkRefreshMessage\(result\.message\) && result\.itemUnavailable/);
    assert.match(chatHomeSource, /onAction\(action\)\.then\(\(result\) => \{[\s\S]*setArmedAction\(null\);[\s\S]*if \(result\.ok\) \{[\s\S]*setOptimisticStatus\(result\.nextStatus\);[\s\S]*\}/);
    assert.match(chatHomeSource, /const visibleItem = optimisticStatus \? \{ \.\.\.item, status: optimisticStatus \} : item/);
    assert.match(chatHomeSource, /className="vnext-chat-action-disclosure vnext-chat-work-action-disclosure vnext-disclosure"/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.work\.actionsLabel'\)\}/);
    assert.match(chatHomeSource, /<summary className="vnext-disclosure-summary">[\s\S]*chatHome\.work\.actions[\s\S]*\{inlineActions\.length\}[\s\S]*<\/summary>/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.work\.quickActionsLabel'\)\}/);
    assert.match(chatHomeSource, /itemActionIconName\(action\.id, action\.tone\)/);
    assert.doesNotMatch(chatHomeSource, /<VNextIcon name=\{action\.tone === 'destructive' \? 'close' : 'receipt'\} \/>/);
    assert.doesNotMatch(chatHomeSource, /chatHome\.starter\.mail/);
    assert.doesNotMatch(chatHomeSource, /id: 'mail'/);
    // Inline quick-actions migrated to <VNextButton variant=…/destructive size="sm">
    // with the glyph via leadingIcon and the label as children.
    assert.match(chatHomeSource, /variant=\{action\.tone === 'destructive' \? 'destructive' : 'secondary'\}/);
    assert.match(chatHomeSource, /\{itemActionButtonLabel\(action, armedAction, executingAction, locale\)\}\s*<\/VNextButton>/);
    assert.match(chatHomeSource, /itemActionButtonLabel\(action, armedAction, executingAction, locale\)/);
    assert.match(chatHomeSource, /itemActionConfirmationHint\(action, locale\)/);
    assert.match(chatHomeSource, /setActionMessage\(itemActionConfirmationHint\(action, locale\)\)/);
    assert.doesNotMatch(chatHomeSource, /setActionMessage\(action\.receiptHint\)/);
    assert.match(chatHomeSource, /onAction\(action\)\.then/);
    assert.match(chatHomeSource, /setLocalUnavailable\(true\)/);
    assert.match(chatHomeSource, /setOptimisticStatus\(null\)/);
    assert.match(chatHomeSource, /setActionMessageKind\(result\.ok \? 'receipt' : isStaleWorkRefreshMessage\(result\.message\) \? 'hint' : 'error'\)/);
    assert.match(chatHomeSource, /function isStaleWorkRefreshMessage\(value: string\)/);
    assert.match(chatHomeSource, /className=\{actionMessageKind === 'error' \? 'vnext-chat-work-action-note error' : 'vnext-chat-work-action-note'\}/);
    assert.match(chatHomeSource, /className="vnext-chat-work-receipt"/);
    assert.match(chatHomeSource, /itemStatus\?: string \| null/);
    assert.match(appSource, /itemStatus: item\.status/);
    assert.match(appSource, /function chatHomeWorkConversationContext\(messageId: string, itemStatus\?: string \| null, excludeThreadId\?: string\)/);
    assert.match(appSource, /entry\.id !== excludeThreadId && \(!entry\.itemStatus \|\| entry\.itemStatus === itemStatus\)/);
    assert.match(appSource, /chatHomeWorkConversationContext\(messageId, item\.status, queuedThreadId\)/);
    assert.match(chatHomeSource, /const visibleThread = thread\.filter\(\(entry\) => !entry\.itemStatus \|\| entry\.itemStatus === visibleItem\.status\)/);
    assert.match(chatHomeSource, /\{visibleThread\.length > 0 && \(/);
    assert.match(chatHomeSource, /\{visibleThread\.map\(\(entry\) => \(/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.work\.receiptLabel'\)\}/);
    assert.match(chatHomeSource, /<em>\{tr\.t\('chatHome\.work\.receipt'\)\}<\/em>/);
    assert.match(chatHomeSource, /onWorkItemAction\(message\.workItem, action\)/);
    assert.match(appSource, /type VNextActionExecutionResult = \{[\s\S]*ok: boolean;[\s\S]*message: string;[\s\S]*nextStatus: string \| null;[\s\S]*itemUnavailable\?: boolean;[\s\S]*\}/);
    assert.match(appSource, /async function executeConfirmedItemAction\(target: OwnerWorkItem, action: VNextItemAction\): Promise<VNextActionExecutionResult>/);
    assert.match(appSource, /executeSelectedItemAction\(\{[\s\S]*item: target,/);
    assert.match(appSource, /optimisticStatusForItemAction\(action\.id,\s*target\)/);
    assert.match(appSource, /updateQueueItemStatus\(target\.id, nextStatus\)/);
    assert.match(appSource, /syncChatWorkMessagesWithQueue/);
    assert.match(appSource, /updateChatWorkMessagesAfterItemStatus/);
    assert.match(appSource, /function updateChatHomeWorkItemStatus\(itemId: string, status: string\)/);
    assert.match(appSource, /setChatHomeMessages\(\(current\) => updateChatWorkMessagesAfterItemStatus\(current, itemId, status\)\)/);
    assert.match(appSource, /function workActionResultShouldSurfaceInChat\(actionId: VNextItemActionId\): boolean/);
    assert.match(appSource, /actionId === 'approval\.approve'/);
    assert.match(appSource, /actionId === 'task\.complete'/);
    assert.match(appSource, /function secretaryConfirmationForWorkAction/);
    assert.match(appSource, /function appendWorkActionResultToChat\(target: OwnerWorkItem, action: VNextItemAction, message: string\)/);
    assert.match(appSource, /response_kind: 'work_action_result'/);
    assert.match(appSource, /appendWorkActionResultToChat\(target, action, successMessage\)/);
    assert.match(appSource, /secretaryConfirmationForWorkAction\(message, errorLocale\)/);
    assert.match(appSource, /function syncChatHomeWorkItemsFromQueue\(nextQueue: OwnerWorkQueue\)/);
    assert.match(appSource, /setChatHomeMessages\(\(current\) => syncChatWorkMessagesWithQueue\(current, nextQueue, errorLocale\)\)/);
    assert.match(chatWorkStateSource, /message\.workItem\?\.id === itemId/);
    assert.match(chatWorkStateSource, /workItem: \{ \.\.\.message\.workItem, status \}/);
    assert.match(chatWorkStateSource, /workItemUnavailable: false/);
    assert.match(chatWorkStateSource, /if \(!nextItem\) \{/);
    // The stale-card strings now come from the i18n catalog (locale threaded in
    // as an explicit param); status stays the 'done' wire enum.
    assert.match(chatWorkStateSource, /summary: t\('chatWork\.itemChangedSummary', locale\)/);
    assert.match(chatWorkStateSource, /next_safe_action: t\('chatWork\.itemChangedNextAction', locale\)/);
    assert.match(chatWorkStateSource, /workItemUnavailable: true/);
    assert.match(chatWorkStateSource, /const statusChanged = nextItem\.status !== message\.workItem\.status/);
    assert.match(chatWorkStateSource, /workItemThread: statusChanged \? \[\] : message\.workItemThread/);
    assert.match(appSource, /updateChatHomeWorkItemStatus\(target\.id, nextStatus\)/);
    assert.match(appSource, /return \{ ok: true, message: successMessage, nextStatus \}/);
    assert.match(appSource, /function isStaleItemActionError\(error: unknown\): boolean/);
    assert.match(appSource, /const candidate = error as \{ statusCode\?: unknown; message\?: unknown \}/);
    // Migrated stale guard: the message now comes from the i18n catalog
    // (errorLocale) and ChatHomeView matches the exact catalog value per locale.
    assert.match(appSource, /const refreshedMessage = t\('app\.action\.itemChangedRefreshedQueue', errorLocale\)/);
    assert.match(chatHomeSource, /t\('app\.action\.itemChangedRefreshedQueue', 'en'\)/);
    assert.match(chatHomeSource, /t\('app\.action\.itemChangedRefreshedQueue', 'zh'\)/);
    assert.doesNotMatch(appSource, /preserveActionMessageOnNextSelectionRef/);
    assert.match(appSource, /await refreshQueue\(target\.id, optimisticStatusByItemId, sessionEpoch\);[\s\S]*setItemActionMessage\(target\.id, refreshedMessage\)/);
    assert.doesNotMatch(appSource, /setActionMessage\(/);
    assert.match(appSource, /await refreshQueue\(target\.id, optimisticStatusByItemId, sessionEpoch\)/);
    assert.match(appSource, /return \{ ok: false, message, nextStatus: null \}/);
    assert.match(appSource, /onWorkItemAction=\{\(item, action\) => executeConfirmedItemAction\(item, action\)\}/);
    assert.match(chatHomeSource, /className="vnext-chat-work-reason"/);
    assert.match(stylesSource, /\.vnext-chat-work-card p\.vnext-chat-work-reason/);
    assert.match(stylesSource, /\.vnext-chat-work-facts/);
    assert.match(stylesSource, /\.vnext-chat-work-facts span:only-child\s*\{[\s\S]*grid-column: 1 \/ -1;/);
    assert.match(stylesSource, /\.vnext-chat-work-facts strong/);
    assert.match(stylesSource, /\.vnext-chat-work-quick-actions/);
    assert.match(stylesSource, /\.vnext-chat-work-quick-actions button\s*\{[\s\S]*display: inline-flex;[\s\S]*gap: 7px;/);
    assert.match(stylesSource, /\.vnext-chat-work-quick-actions button svg/);
    assert.match(stylesSource, /\.vnext-chat-work-quick-actions button span/);
    assert.match(stylesSource, /\.vnext-chat-work-action-note/);
    assert.match(stylesSource, /\.vnext-chat-work-action-note\.error/);
    assert.match(stylesSource, /\.vnext-chat-work-receipt/);
    assert.match(chatHomeSource, /onAskWorkItem\(message\.id, message\.workItem, text\)/);
    assert.match(chatHomeSource, /function chatHomeResultTitle\(result: ContextualAssistantResponse, tr: Translator\)/);
    assert.match(chatHomeSource, /kind === 'orchestrated_response' \|\| kind === 'explanation' \|\| kind === 'answer'/);
    assert.match(chatHomeSource, /return tr\.t\('chatHome\.role\.aegis'\);/);
    assert.match(chatHomeSource, /<strong>\{chatHomeResultTitle\(message\.result, tr\)\}<\/strong>/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.work\.askAboutItem'\)\}/);
    assert.match(chatHomeSource, /placeholder=\{tr\.t\('chatHome\.work\.askPlaceholder'\)\}/);
    assert.match(chatHomeSource, /function submitAsk\(\)/);
    assert.match(chatHomeSource, /resizeComposerInput\(askInputRef\.current\);/);
    assert.match(chatHomeSource, /rows=\{1\}/);
    assert.match(chatHomeSource, /resizeComposerInput\(event\.currentTarget\);[\s\S]*setAskText\(event\.currentTarget\.value\);/);
    assert.match(chatHomeSource, /event\.key !== 'Enter' \|\| event\.shiftKey \|\| isImeEnter\(event\.nativeEvent\)/);
    assert.match(chatHomeSource, /event\.preventDefault\(\);[\s\S]*submitAsk\(\);/);
    assert.match(stylesSource, /\.vnext-chat-work-ask textarea\s*\{[^}]*min-height: 52px;[^}]*max-height: 128px;[^}]*overflow: hidden;[^}]*resize: none;/s);
    assert.doesNotMatch(chatHomeSource, /aria-label="Ask Aegis about this work item"[\s\S]{0,180}disabled=\{busy\}/);
    assert.doesNotMatch(chatHomeSource, /if \(!trimmedAsk \|\| busy\) return;/);
    assert.match(chatHomeSource, /disabled=\{busy && !trimmedAsk\}/);
    assert.match(chatHomeSource, /aria-label=\{busy \? tr\.t\('chatHome\.work\.queueAboutItem'\) : tr\.t\('chatHome\.work\.askAboutItem'\)\}/);
    assert.match(chatHomeSource, /<VNextIcon name="send" \/>[\s\S]*<span>\{busy \? tr\.t\('chatHome\.composer\.queue'\) : tr\.t\('chatHome\.composer\.send'\)\}<\/span>/);
    assert.match(chatHomeSource, /className="vnext-chat-work-ask"/);
    assert.match(chatHomeSource, /workItemThread\?: ChatHomeWorkThreadEntry\[\]/);
    assert.match(chatHomeSource, /state\?: 'queued' \| 'working'/);
    assert.match(chatHomeSource, /entry\.state === 'queued'/);
    assert.match(chatHomeSource, /chatHome\.work\.queuedNext/);
    assert.match(stylesSource, /\.vnext-chat-work-card \.vnext-chat-work-working,\s*\.vnext-chat-work-card \.vnext-chat-work-queued/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.work\.threadLabel'\)\}/);
    assert.match(stylesSource, /\.vnext-chat-work-thread/);
    assert.match(chatHomeSource, /className="vnext-chat-work-response-label"[\s\S]*<span>\{tr\.t\('chatHome\.role\.aegis'\)\}<\/span>[\s\S]*<MessageTime timestamp=\{entry\.responseTimestamp\} \/>/);
    assert.match(chatHomeSource, /<span>\{tr\.t\('chatHome\.role\.you'\)\}<\/span>/);
    assert.match(stylesSource, /\.vnext-chat-work-response-label/);
    assert.match(stylesSource, /border-radius: 14px 14px 4px 14px;/);
    assert.match(chatHomeSource, /const threadActivityKey = messages/);
    assert.match(chatHomeSource, /\}, \[messages\.length, busy, threadActivityKey\]\);/);
    assert.match(chatHomeSource, /const askInputRef = useRef<HTMLTextAreaElement \| null>\(null\);/);
    assert.match(chatHomeSource, /askInputRef\.current\?\.focus\(\);/);
    assert.match(appSource, /requestAssistantCommandWithAutoCheck/);
    assert.match(appSource, /workItem: command\.workItem/);
    assert.match(appSource, /focusId: item\.id/);
    assert.match(appSource, /focusId: item\.id,[\s\S]*intent: 'chat',/);
    assert.match(chatHomeSource, /const streamClassName = \[/);
    assert.match(chatHomeSource, /const homeClassName = \[/);
    assert.match(chatHomeSource, /busy \? 'is-working' : ''/);
    assert.match(chatHomeSource, /hasConversation \? '' : 'is-empty'/);
    assert.match(chatHomeSource, /className=\{streamClassName\}/);
    assert.match(chatHomeSource, /className=\{homeClassName\}/);
    assert.match(appSource, /retryText: text/);
    assert.match(appSource, /contextOverride\?: ChatHomeAttachedContext/);
    assert.match(appSource, /const requestContext = contextOverride \?\? chatHomeAttachedContext/);
    assert.match(appSource, /setChatHomeAttachedContext\(null\)/);
    assert.match(appSource, /contextLabel: requestContext\.label/);
    assert.match(appSource, /focusId: requestContext\?\.itemId \?\? GENERAL_ASSISTANT_FOCUS_ID/);
    assert.match(appSource, /scope: requestContext \? 'item' : 'general'/);
    assert.match(appSource, /retryContext: requestContext/);
    assert.match(appSource, /retryIdempotencyKey: retryKey/);
    assert.match(appSource, /retryLabel: retryKey \? 'Check result' : 'Try again'/);
    assert.match(appSource, /assistantError instanceof ApiResponseError && assistantError\.statusCode === 504/);
    assert.match(appSource, /onRetry=\{\(text, idempotencyKey, context, quote\) => \{[\s\S]*?submitChatHomeCommand\(text, idempotencyKey, context, 'text'/);
    assert.match(appSource, /function compactConversationTurnText\(text: string\): string/);
    assert.match(appSource, /function chatHomeConversationContext\(quote\?: ChatHomeQuote \| null\): VNextConversationContextTurn\[\]/);
    assert.match(appSource, /message\.role === 'owner'/);
    assert.match(appSource, /message\.result\?\.visible_answer \|\| message\.text/);
    assert.match(appSource, /chatHomeConversationContext\(requestQuote\)/);
    assert.match(appSource, /kind: 'quoted_reply'/);
    assert.match(appSource, /source_id: quote\.sourceId/);
    assert.doesNotMatch(appSource, /Aegis answer being quoted/);
    assert.doesNotMatch(appSource, /Owner message being quoted/);
    assert.match(appSource, /async function askChatHomeWorkItem\([\s\S]*messageId: string,[\s\S]*item: OwnerWorkItem,[\s\S]*ownerText: string,[\s\S]*queuedThreadId\?: string,[\s\S]*\)/);
    assert.match(appSource, /function queueChatHomeWorkItemAsk\(messageId: string, item: OwnerWorkItem, text: string\)/);
    assert.match(appSource, /if \(busy && !queuedThreadId\) \{[\s\S]*queueChatHomeWorkItemAsk\(messageId, item, text\);[\s\S]*return;/);
    assert.match(appSource, /queuedThreadId\?: string/);
    assert.match(appSource, /state: 'queued' as const/);
    assert.match(appSource, /state: 'working' as const/);
    assert.match(appSource, /chatHomeWorkConversationContext\(messageId, item\.status, queuedThreadId\)/);
    assert.match(appSource, /entry\.id !== excludeThreadId && \(!entry\.itemStatus \|\| entry\.itemStatus === itemStatus\)/);
    assert.match(appSource, /if \(entry\.state === 'queued'\) return \[\];/);
    assert.match(appSource, /setQueuedChatHomeWorkPrompts\(remaining\)/);
    assert.match(appSource, /nextPrompt\.queuedThreadId/);
    assert.match(appSource, /const text = ownerText\.trim\(\);/);
    assert.match(appSource, /function chatHomeWorkConversationContext\(messageId: string, itemStatus\?: string \| null, excludeThreadId\?: string\): VNextConversationContextTurn\[\]/);
    assert.match(appSource, /message\.workItemThread\.filter/);
    assert.match(appSource, /role: 'aegis', text: compactConversationTurnText\(entry\.result\.visible_answer\)/);
    assert.match(appSource, /const conversationContext = chatHomeWorkConversationContext\(messageId, item\.status, queuedThreadId\)/);
    assert.match(appSource, /conversationContext,/);
    assert.match(appSource, /updateChatHomeWorkThread\(messageId, threadId/);
    assert.doesNotMatch(appSource, /askChatHomeWorkItem[\s\S]*role: 'aegis'[\s\S]*workItem: item/);
    assert.doesNotMatch(appSource, /async function askChatHomeWorkItem[\s\S]{0,420}请结合/);
    assert.doesNotMatch(appSource, /async function askChatHomeWorkItem[\s\S]{0,420}不要执行任何外部动作/);
    assert.doesNotMatch(appSource, /Explain what I should do about/);
    assert.doesNotMatch(errorSource, /Refresh and try again/);
    assert.match(appSource, /retryIdempotencyKey\?: string/);
    assert.match(appSource, /queueChatHomeCommand\(prompt, contextOverride, modeOverride, displayTextOverride, retryIdempotencyKey, quoteOverride, queueAttachments\)/);
    assert.match(appSource, /nextCommand\.retryIdempotencyKey/);
    assert.doesNotMatch(chatHomeSource, /message\.result\.response_kind === 'error'[\s\S]{0,260}disabled=\{busy\}/);
    assert.match(chatHomeSource, /message\.retryLabel === 'Check result' \? tr\.t\('chatHome\.retry\.queueCheck'\) : tr\.t\('chatHome\.retry\.queueRetry'\)/);
    assert.match(chatHomeSource, /data-testid="aegis-chat-stream"/);
    assert.match(appSource, /case 'assistant_delta':/);
    assert.match(appSource, /case 'assistant_draft_reset':/);
    assert.match(appSource, /appendAssistantDelta\(message\.streamText \|\| '', event\.text\)/);
    assert.match(appSource, /const withoutWorking = current\.filter\(\(message\) => message\.id !== workingId\)/);
  });

  it('keeps long-running chat requests visibly alive without a separate toast', () => {
    // Working-state placeholders are localized: the constants hold catalog keys
    // and each render site resolves them against the owner's display_language.
    // Chat send and the work-item ask each paint through paintWorkingProgress:
    // 2.5s adds the slow line only when nothing live has arrived, and 25s sets
    // the keep-open hint (LONG key) without replacing those lines.
    assert.match(appSource, /CHAT_HOME_WORKING_INITIAL_KEY = 'chatHome\.working\.initial'/);
    assert.match(appSource, /CHAT_HOME_WORKING_SLOW_KEY = 'chatHome\.working\.slow'/);
    assert.match(appSource, /CHAT_HOME_WORKING_LONG_KEY = 'chatHome\.working\.long'/);
    assert.match(appSource, /const updateWorkingMessage = \(nextText: string\) =>/);
    assert.match(appSource, /updateWorkingMessage\(composeWorkingText\(\s*liveProgressLines,\s*keepPageOpenHint,\s*keepPageOpenText,/);
    const slowTimers = appSource.match(/const slowTimer = window\.setTimeout\(\(\) => \{\s*if \(liveProgressLines\.length === 0\) \{\s*paintWorkingProgress\(t\(CHAT_HOME_WORKING_SLOW_KEY, errorLocale\)\);\s*\}\s*\}, 2500\);/g) ?? [];
    const longTimers = appSource.match(/const longTimer = window\.setTimeout\(\(\) => \{\s*keepPageOpenHint = true;\s*paintWorkingProgress\(\);\s*\}, 25000\);/g) ?? [];
    const longKeyBindings = appSource.match(/const keepPageOpenText = t\(CHAT_HOME_WORKING_LONG_KEY, errorLocale\);/g) ?? [];
    assert.equal(slowTimers.length, 2);
    assert.equal(longTimers.length, 2);
    assert.equal(longKeyBindings.length, 2);
    assert.match(appSource, /window\.clearTimeout\(slowTimer\)/);
    assert.match(appSource, /window\.clearTimeout\(longTimer\)/);
    assert.doesNotMatch(chatHomeSource, /Sent: "/);
  });

  it('keeps the empty chat home from looking like a blank console canvas', () => {
    const chatStyles = stylesSource.slice(stylesSource.indexOf('.vnext-chat-home {'));
    assert.doesNotMatch(chatStyles, /rgba\(231, 185, 95,/);
    assert.doesNotMatch(chatStyles, /#ffdda0|#ffd9a0|#e7b95f/i);
    assert.match(chatStyles, /#0a84ff/);
    const emptyStream = cssRuleBlock(stylesSource, '.vnext-chat-stream.is-empty');
    assert.match(emptyStream, /min-height: 0;/);
    assert.match(emptyStream, /align-self: stretch;/);
    assert.match(emptyStream, /align-items: start;/);
    assert.doesNotMatch(emptyStream, /align-items: center;/);
    const emptyStage = cssRuleBlock(stylesSource, '.vnext-chat-empty-stage');
    assert.match(emptyStage, /min-height: 100%;/);
    assert.match(emptyStage, /align-items: start;/);
    assert.match(emptyStage, /padding-top: clamp\(12px, 5vh, 52px\);/);
    assert.match(emptyStage, /padding-bottom: clamp\(14px, 5vh, 48px\);/);
    assert.doesNotMatch(emptyStage, /align-items: center;/);
    const hero = cssRuleBlock(stylesSource, '.vnext-chat-hero');
    assert.match(hero, /min-height: 440px;/);
    assert.match(hero, /border: 1px solid rgba\(255, 255, 255, 0\.09\);/);
    assert.match(hero, /backdrop-filter: blur\(24px\) saturate\(132%\);/);
    assert.match(stylesSource, /\.vnext-chat-presence\s*\{/);
    const startersButton = cssRuleBlock(stylesSource, '.vnext-chat-starters button');
    assert.match(startersButton, /background:[\s\S]*rgba\(9, 22, 36, 0\.5\);/);
    assert.match(startersButton, /box-shadow: 0 16px 42px rgba\(0, 0, 0, 0\.2\);/);
    const mobileEmptyStage = mobileCssRuleBlock(stylesSource, '.vnext-chat-empty-stage');
    assert.match(mobileEmptyStage, /align-items: start;/);
    assert.match(mobileEmptyStage, /padding-top: 8px;/);
    assert.match(mobileEmptyStage, /padding-bottom: 12px;/);
    assert.doesNotMatch(mobileEmptyStage, /align-items: center;/);
    const emptyHome = cssRuleBlock(stylesSource, '.vnext-chat-home.is-empty');
    assert.match(emptyHome, /grid-template-rows: auto minmax\(0, 1fr\) auto;/);
    assert.match(emptyHome, /align-content: stretch;/);
    const mobileEmptyHome = mobileCssRuleBlock(stylesSource, '.vnext-chat-home.is-empty');
    assert.match(mobileEmptyHome, /grid-template-rows: auto minmax\(0, 1fr\) auto;/);
    assert.match(mobileEmptyHome, /align-content: stretch;/);
    const mobileComposer = mobileCssRuleBlock(stylesSource, '.vnext-chat-composer');
    assert.doesNotMatch(mobileComposer, /position: sticky;/);
    const mobileConversationComposer = mobileCssRuleBlock(stylesSource, '.vnext-chat-home.has-conversation .vnext-chat-composer');
    assert.doesNotMatch(mobileConversationComposer, /position: sticky;/);
    const mobileConversationStream = mobileCssRuleBlock(stylesSource, '.vnext-chat-home.has-conversation .vnext-chat-stream');
    assert.match(mobileConversationStream, /padding: 4px 0 0;/);
    assert.match(mobileConversationStream, /border: 0;/);
    assert.match(mobileConversationStream, /border-radius: 0;/);
    assert.match(mobileConversationStream, /background: transparent;/);
    assert.match(mobileConversationStream, /box-shadow: none;/);
    assert.match(stylesSource, /\.vnext-chat-brief-card/);
    assert.match(stylesSource, /\.vnext-chat-brief-card:hover/);
    assert.match(stylesSource, /\.vnext-chat-home\.composer-half \.vnext-chat-composer/);
    assert.doesNotMatch(stylesSource, /composer-full/);
    assert.doesNotMatch(stylesSource, /mode-full/);
    // Composer: paperclip | textarea | context | size-toggle | mic | submit(auto).
    // Attach is the paperclip alone; the size toggle is a direct button (the
    // "more" menu is gone, owner 2026-09-12). Mobile drops only the attach-context trigger.
    assert.match(stylesSource, /\.vnext-chat-composer\.mode-line\s*\{[\s\S]*grid-template-columns: auto minmax\(0, 1fr\) auto auto auto auto;/);
    assert.match(stylesSource, /\.vnext-chat-composer\.mode-line textarea\s*\{[\s\S]*max-height: 52px;/);
    assert.match(stylesSource, /\.vnext-chat-composer\.mode-line textarea\s*\{[\s\S]*grid-column: 2;/);
    assert.match(stylesSource, /\.vnext-chat-composer\.mode-line \.vnext-chat-attach-button\s*\{[\s\S]*grid-column: 1;/);
    assert.match(stylesSource, /\.vnext-chat-composer\.mode-line \.vnext-chat-context-button\s*\{[\s\S]*grid-column: 3;/);
    assert.match(stylesSource, /\.vnext-chat-composer\.mode-line \.vnext-chat-size-button\s*\{[\s\S]*grid-column: 4;/);
    assert.doesNotMatch(stylesSource, /\.vnext-chat-composer[^\n]*\.vnext-header-menu/);
    assert.match(stylesSource, /\.vnext-chat-composer\.mode-line \.vnext-chat-voice\s*\{[\s\S]*grid-column: 5;/);
    // Recording replaces the control row and spans the full composer width.
    assert.match(stylesSource, /\.vnext-chat-recording\s*\{[\s\S]*grid-column: 1 \/ -1;/);
    assert.match(stylesSource, /\.vnext-chat-recording-dot\s*\{[\s\S]*border-radius: 999px;/);
    // Slice 5: pending-attachment pill mirrors attached-context (full-width chip)
    // + neutral/uploading + danger/error variants; hidden file input takes no space.
    assert.match(stylesSource, /\.vnext-chat-attached-file\s*\{[\s\S]*grid-column: 1 \/ -1;[\s\S]*border-radius: 999px;/);
    assert.match(stylesSource, /\.vnext-chat-attached-file\.is-uploading\s*\{[\s\S]*var\(--aegis-muted\)/);
    assert.match(stylesSource, /\.vnext-chat-attached-file\.is-error\s*\{[\s\S]*var\(--aegis-danger\)/);
    assert.match(stylesSource, /\.vnext-chat-file-input\s*\{[\s\S]*display: none;/);
    assert.match(stylesSource, /\.vnext-chat-composer\.mode-line:has\(\.vnext-chat-composer-meta\) textarea[\s\S]*grid-row: 2;/);
    assert.match(chatHomeSource, /quotedMessage \? 'has-reply-context' : ''/);
    assert.match(chatHomeSource, /className="vnext-chat-composer-meta"/);
    assert.match(chatHomeSource, /className="vnext-chat-composer-tools"/);
    assert.match(stylesSource, /\.vnext-chat-composer\.mode-line \.vnext-chat-composer-tools\s*\{[\s\S]*display: contents;/);
    assert.match(stylesSource, /\.vnext-chat-composer\.mode-half \.vnext-chat-composer-tools\s*\{[\s\S]*display: flex;/);
    assert.match(stylesSource, /\.vnext-chat-home\.is-empty\.composer-half/);
    assert.match(stylesSource, /\.vnext-chat-home\.is-sheet\.composer-half \.vnext-chat-composer/);
    assert.doesNotMatch(stylesSource, /\.vnext-chat-composer\.is-composing \.vnext-chat-size-button/);
    assert.match(stylesSource, /\.vnext-chat-composer\.mode-half textarea/);
    // Mobile mode-line: paperclip | textarea | size-toggle | mic | submit → 5 cols.
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-chat-composer\.mode-line\s*\{[\s\S]*grid-template-columns: auto minmax\(0, 1fr\) auto auto auto;/);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-chat-composer\.mode-line \.vnext-chat-size-button\s*\{[\s\S]*grid-column: 3;/);
    // Attach and the voice-note mic are PRIMARY on mobile now — neither may be
    // hidden there (both used to be unreachable: attach buried, mic display:none).
    const mobileVoice = mobileCssRuleBlock(stylesSource, '.vnext-chat-composer.mode-line .vnext-chat-voice');
    assert.doesNotMatch(mobileVoice, /display: none;/);
    assert.match(mobileVoice, /grid-column: 4;/);
    const mobileAttach = mobileCssRuleBlock(stylesSource, '.vnext-chat-composer.mode-line .vnext-chat-attach-button');
    assert.match(mobileAttach, /grid-column: 1;/);
    // The attach-context trigger stays hidden in ALL mobile composer modes (so
    // .has-context-picker stays unreachable on mobile).
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-chat-composer \.vnext-chat-context-button\s*\{[\s\S]*display: none;/);
    // 44px is the minimum comfortable touch target on the composer row.
    const mobileControls = mobileCssRuleBlock(stylesSource, '.vnext-chat-composer button');
    assert.match(mobileControls, /height: 44px;/);
    assert.match(mobileControls, /min-width: 44px;/);
    assert.match(stylesSource, /\.vnext-chat-context-strip/);
    assert.match(stylesSource, /\.vnext-chat-session-text/);
    assert.match(stylesSource, /\.vnext-chat-session button \.vnext-chat-action-label/);
    assert.doesNotMatch(stylesSource, /span:not\(\.vnext-session-dot\)/);
  });
});
