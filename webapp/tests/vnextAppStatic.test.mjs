import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

const appSource = readFileSync(new URL('../src/vnext/App.tsx', import.meta.url), 'utf8');
const chatHistorySource = readFileSync(new URL('../src/vnext/chatHistory.ts', import.meta.url), 'utf8');
const settingsSource = readFileSync(new URL('../src/vnext/SettingsView.tsx', import.meta.url), 'utf8');
const statusSource = readFileSync(new URL('../src/vnext/StatusView.tsx', import.meta.url), 'utf8');
const navigationSource = readFileSync(new URL('../src/vnext/navigation.ts', import.meta.url), 'utf8');
const workSource = readFileSync(new URL('../src/vnext/WorkView.tsx', import.meta.url), 'utf8');
// WorkView user-facing copy migrated to the i18n catalog (work.* namespace); the
// English strings now live there, so source-string guards assert against this.
const catalogSource = readFileSync(new URL('../src/i18n/catalog.ts', import.meta.url), 'utf8');
const presentationSource = readFileSync(new URL('../src/vnext/presentation.ts', import.meta.url), 'utf8');
const workItemRowSource = readFileSync(new URL('../src/vnext/WorkItemRow.tsx', import.meta.url), 'utf8');
const assistantActionsSource = readFileSync(new URL('../src/vnext/assistantActions.ts', import.meta.url), 'utf8');
const assistantFallbackSource = readFileSync(new URL('../src/vnext/assistantFallbackText.ts', import.meta.url), 'utf8');
const assistantResponseSource = readFileSync(new URL('../src/vnext/AssistantResponseView.tsx', import.meta.url), 'utf8');
const itemActionsSource = readFileSync(new URL('../src/vnext/itemActions.ts', import.meta.url), 'utf8');
const detailSource = readFileSync(new URL('../src/vnext/detail.ts', import.meta.url), 'utf8');
const chatHomeSource = readFileSync(new URL('../src/vnext/ChatHomeView.tsx', import.meta.url), 'utf8');
const chatWorkStateSource = readFileSync(new URL('../src/vnext/chatWorkState.ts', import.meta.url), 'utf8');
const installPromptSource = readFileSync(new URL('../src/vnext/installPrompt.ts', import.meta.url), 'utf8');
const inlineAssistantSource = readFileSync(new URL('../src/vnext/InlineAssistantPanel.tsx', import.meta.url), 'utf8');
const modelSource = readFileSync(new URL('../src/vnext/model.ts', import.meta.url), 'utf8');
const iconsSource = readFileSync(new URL('../src/vnext/icons.tsx', import.meta.url), 'utf8');
const ownerProjectionTextSource = readFileSync(new URL('../src/vnext/ownerProjectionText.ts', import.meta.url), 'utf8');
const voiceInputSource = readFileSync(new URL('../src/vnext/voiceInput.ts', import.meta.url), 'utf8');
const voiceNoteSource = readFileSync(new URL('../src/vnext/voiceNote.ts', import.meta.url), 'utf8');
const ceoVoiceAgentSource = readFileSync(new URL('../src/vnext/CeoVoiceAgent.tsx', import.meta.url), 'utf8');
const stylesSource = readFileSync(new URL('../src/vnext/styles.css', import.meta.url), 'utf8');
const localeContextSource = readFileSync(new URL('../src/i18n/LocaleContext.tsx', import.meta.url), 'utf8');
const indexHtmlSource = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
// The ACTIVE chat-home surface (Direction B) is imported AFTER styles.css and
// scopes .vnext-app-chat-home .vnext-chat-topbar with higher specificity, so
// topbar styling must live here, not in the (clobbered) styles.css base.
const chatHomeCss = readFileSync(new URL('../src/vnext/design/surfaces/chat-home.css', import.meta.url), 'utf8');
const shellSource = [appSource, settingsSource, statusSource, workSource, inlineAssistantSource, chatHomeSource].join('\n');

describe('vNext reset static guards', () => {
  it('keeps the main browser surface narrow: Aegis, Work, with system actions in the header', () => {
    assert.match(navigationSource, /'home' \| 'work' \| 'status' \| 'settings'/);
    assert.doesNotMatch(appSource, /label=\{t\('app\.shell\.nav\.mail', errorLocale\)\}/);
    assert.doesNotMatch(catalogSource, /'app\.shell\.nav\.mail'/);
    assert.doesNotMatch(appSource, /view === 'mail'/);
    // App-shell chrome migrated to the i18n catalog (app.shell.* namespace,
    // 2026-06-09 final i18n tail). The narrow-surface invariant is now asserted
    // via the catalog-key wiring in App.tsx plus the byte-identical English
    // values living in the catalog, not via raw English literals in App source.
    // Task 2: desktop rail home button relabeled from nav.aegis → nav.chat;
    // nav.aegis must no longer appear as a NavButton label in App source.
    assert.doesNotMatch(appSource, /label=\{t\('app\.shell\.nav\.aegis', errorLocale\)\}/);
    assert.match(appSource, /<strong>\{t\('app\.shell\.brand', errorLocale\)\}<\/strong>/);
    assert.match(appSource, /label=\{t\('app\.shell\.nav\.work', errorLocale\)\}/);
    // Task 1+2: nav.chat is used by both the mobile bottom nav AND the desktop rail
    // home button. The desktop rail has Chat/Work only.
    assert.match(appSource, /label=\{t\('app\.shell\.nav\.chat', errorLocale\)\}/);
    // Home rail = the 安琪 persona (Aegis in English), not a separate "Chat" product.
    assert.match(catalogSource, /'app\.shell\.nav\.chat':.*zh: '安琪'.*en: 'Aegis'/s);
    // The mobile nav must NOT be guarded by view !== 'home'.
    assert.doesNotMatch(appSource, /view !== 'home' && <nav className="vnext-nav"/);
    // Shared OwnerShellChrome + overflow menu carry Status/Settings (not peer tabs).
    assert.match(appSource, /OwnerShellChrome/);
    assert.match(appSource, /menuTriggerLabel=\{t\('app\.shell\.header\.menu', errorLocale\)\}/);
    assert.match(appSource, /ownerUtilityMenuItemIds\(\)/);
    assert.match(appSource, /navigate\('status', null\)/);
    assert.match(appSource, /navigate\('settings', null\)/);
    // settings wired via ownerUtilityMenuItemIds map (asserted above)
    assert.match(appSource, /<span>\{t\('app\.shell\.ownerCard\.signedIn', errorLocale\)\}<\/span>/);
    assert.match(appSource, /\{t\('app\.shell\.brandCopy', errorLocale\)\}/);
    assert.match(catalogSource, /'app\.shell\.ownerCard\.signedIn':.*en: 'Signed in'/);
    assert.match(catalogSource, /'app\.shell\.brandCopy':[\s\S]*?en: 'Organizes what needs attention, asks before important changes, and keeps receipts\.'/);
    assert.match(appSource, /function surfaceTitle/);
    assert.match(appSource, /function surfaceSubtitle/);
    assert.match(catalogSource, /'app\.shell\.surface\.work\.subtitle':[\s\S]*?en: 'Only what needs you\.'/);
    assert.match(catalogSource, /'app\.shell\.surface\.status\.subtitle':[\s\S]*?en: 'Only what needs you — I handle the rest quietly\.'/);
    assert.match(catalogSource, /'app\.shell\.surface\.settings\.subtitle':[\s\S]*?en: 'You, language, notifications, connections\.'/);
    assert.match(appSource, /subtitle=\{surfaceSubtitle\(view, errorLocale\)\}/);
    assert.match(stylesSource, /\.vnext-header-subtitle\s*\{/);
    assert.match(stylesSource, /grid-template-columns:\s*repeat\(2, minmax\(0, 1fr\)\);/);
    assert.doesNotMatch(appSource, /Aegis Status/);
    assert.doesNotMatch(shellSource, />Aegis Runtime</);
    assert.doesNotMatch(appSource, /ProofView/);
    assert.doesNotMatch(appSource, /InstructionsView/);
    assert.doesNotMatch(appSource, /label="Proof"/);
    assert.doesNotMatch(appSource, /label="Today"/);
    assert.doesNotMatch(appSource, /Standing Instructions/);
    assert.doesNotMatch(appSource, />Private access</);
    assert.doesNotMatch(appSource, /keeps proof before claiming work is done/);
    // The Dev Harness surface is fully removed: no 'dev' route, view id, nav
    // entry, catalog key, or Settings entry point may reappear.
    assert.doesNotMatch(navigationSource, /'dev'/);
    assert.doesNotMatch(appSource, /navigate\('dev'/);
    assert.doesNotMatch(appSource, /onOpenDev/);
    assert.doesNotMatch(settingsSource, /onOpenDev/);
    assert.doesNotMatch(catalogSource, /'app\.shell\.menu\.dev'/);
    assert.doesNotMatch(catalogSource, /'settings\.developer\.openHarness'/);
  });


  it('shares owner utility menu ids (status/settings) across home and non-home chrome', () => {
    assert.match(presentationSource, /export function ownerUtilityMenuItemIds/);
    assert.match(presentationSource, /export function ownerShellChromeClass/);
    assert.match(appSource, /variant="console"/);
    assert.match(chatHomeSource, /variant="chat"/);
    assert.match(presentationSource, /'status'/);
    assert.match(presentationSource, /'settings'/);
    assert.doesNotMatch(presentationSource, /OWNER_UTILITY_MENU_IDS = \[[^\]]*dev/);
    assert.match(appSource, /ownerUtilityMenuItemIds\(\)/);
    assert.match(chatHomeSource, /ownerUtilityMenuItemIds\(\)/);
    // Shared chrome for header + chat topbar — one geometry across views.
    // Owner 2026-09-16 pass 2: BOTH are the same opaque 52px bar (--b-bar,
    // bottom hairline, no radius, no blur, no shadow); the chat topbar is no
    // longer a 56–64px frosted rounded card.
    const chromeCss = readFileSync(new URL('../src/vnext/design/surfaces/chrome.css', import.meta.url), 'utf8');
    assert.match(chromeCss, /Shared owner shell chrome/);
    assert.match(chromeCss, /\.vnext-header\.vnext-owner-shell-chrome/);
    assert.match(chromeCss, /\.vnext-chat-topbar\.vnext-owner-shell-chrome/);
    assert.match(chromeCss, /\.vnext-chat-topbar\.vnext-owner-shell-chrome\s*\{[^}]*min-height:\s*var\(--console-header-h/s);
    assert.match(chromeCss, /\.vnext-chat-topbar\.vnext-owner-shell-chrome\s*\{[^}]*border-radius:\s*0/s);
    assert.match(chromeCss, /\.vnext-chat-topbar\.vnext-owner-shell-chrome\s*\{[^}]*background:\s*var\(--b-bar\)/s);
    assert.match(chromeCss, /\.vnext-chat-topbar\.vnext-owner-shell-chrome\s*\{[^}]*backdrop-filter:\s*none/s);
    assert.doesNotMatch(chromeCss, /\.vnext-chat-topbar\.vnext-owner-shell-chrome\s*\{[^}]*frost/s);
    // Unified title size (chat h1 used to float at browser default ~32px);
    // the bar title is 17/600 on both chrome variants.
    assert.match(chromeCss, /\.vnext-owner-shell-titles h1\s*\{[^}]*font-size:\s*18px/s);
    assert.match(chromeCss, /\.vnext-chat-topbar\.vnext-owner-shell-chrome \.vnext-owner-shell-titles h1\s*\{[^}]*font-size:\s*17px/s);
    // Summon avatar footprint ~38px.
    assert.match(chromeCss, /\.vnext-owner-shell-summon\s*\{[^}]*width:\s*2\.5rem/s);
    // Console viewport shell: app is fixed viewport; shell scrolls; header sticky top:0
    // (body-scroll + overflow-x:hidden made sticky inert — bar scrolled away).
    assert.match(chromeCss, /Console viewport shell/);
    assert.match(chromeCss, /\.vnext-shell\s*\{[^}]*overflow-y:\s*auto/s);
    assert.match(chromeCss, /\.vnext-header\.vnext-owner-shell-chrome\s*\{[^}]*position:\s*sticky/s);
    assert.match(chromeCss, /\.vnext-header\.vnext-owner-shell-chrome\s*\{[^}]*top:\s*0/s);
  });

  it('marks the active primary nav item for assistive tech and names the model select explicitly', () => {
    // aria-current="page" on the active tab (desktop rail + mobile bottom nav
    // share NavButton), not just a CSS class.
    assert.match(appSource, /aria-current=\{isActive \? 'page' : undefined\}/);
    assert.match(appSource, /className=\{classNames\(isActive && 'active'\)\}/);
    // Model picker: label ↔ select linked explicitly so the filter <input> can
    // no longer pollute / drop the select's accessible name.
    assert.match(settingsSource, /<label id="vnext-settings-model-label" htmlFor="vnext-settings-model-select">/);
    assert.match(settingsSource, /id="vnext-settings-model-select"/);
    assert.match(settingsSource, /aria-labelledby="vnext-settings-model-label"/);
    assert.match(settingsSource, /className="vnext-model-filter"[\s\S]*?aria-label=\{tr\('settings\.model\.filter'\)\}/);
    assert.doesNotMatch(settingsSource, /<label className="vnext-model-field">/);
  });

  it('ships zh-first and re-stamps <html lang> from the locale provider', () => {
    // The shipped document language is zh-CN (preferredLocale() reads it for
    // text produced outside React renders) and the LocaleProvider re-stamps it
    // on every locale change, so restored history / voice command errors
    // follow the console locale instead of the shipped default.
    assert.match(indexHtmlSource, /<html lang="zh-CN">/);
    assert.match(localeContextSource, /document\.documentElement\.lang = next/);
    assert.match(localeContextSource, /useEffect\(\(\) => \{\n\s+stampDocumentLanguage\(locale\);\n\s+\}, \[locale\]\);/);
    // Console call sites pass the context locale explicitly rather than
    // relying on the stamped default.
    assert.match(appSource, /readChatHomeHistory\(window\.localStorage, ownerHistoryKey, errorLocale\)/);
    assert.match(appSource, /writeChatHomeHistory\(window\.sessionStorage, CHAT_HOME_HISTORY_KEY, chatHomeMessages, errorLocale\)/);
    assert.match(ceoVoiceAgentSource, /dispatchCeoCommand\(handlersRef\.current, name, argsJson, \{ locale \}\)/);
    assert.match(appSource, /subscribeOwnerWebPush\(status\.public_key, errorLocale\)/);
    assert.match(appSource, /unsubscribeOwnerWebPush\(errorLocale\)/);
    assert.doesNotMatch(chatHistorySource, /locale: Locale = preferredLocale\(\)\): string \| null/);
  });

  it('keeps primary Chat/Work nav identical on home and non-home', () => {
    assert.match(presentationSource, /OWNER_PRIMARY_NAV_IDS = \['home', 'work'\]/);
    assert.match(presentationSource, /export function ownerPrimaryNavItemIds/);
    // Brand rail is always rendered (not gated by view !== 'home').
    assert.doesNotMatch(appSource, /view !== 'home' && <aside className="vnext-brand-rail"/);
    assert.match(appSource, /<aside className="vnext-brand-rail"/);
    assert.match(appSource, /ownerPrimaryNavItemIds\(\)\.map/);
    // Chat home: peers only in bottom/rail; topbar no surface-nav duplicate.
    assert.doesNotMatch(chatHomeSource, /vnext-chat-surface-nav/);
    assert.match(chatHomeSource, /vnext-chat-needs-you/);
    assert.match(chatHomeSource, /needsYouItems/);
    assert.match(chatHomeSource, /vnext-chat-needs-you-cards/);
    assert.match(chatHomeSource, /resolveCeoShorthand/);
    assert.match(appSource, /needsYouItems=/);
    assert.doesNotMatch(chatHomeSource, /onOpenMail/);
    assert.doesNotMatch(appSource, /onOpenMail=/);
    assert.match(catalogSource, /'chatHome\.destinations\.work'/);
    assert.doesNotMatch(catalogSource, /'chatHome\.destinations\.mail'/);
    assert.match(catalogSource, /'chatHome\.intent\.understood'/);
    assert.match(catalogSource, /'chatHome\.working\.intent'/);
  });

  it('uses shared OwnerShellChrome for chat and console headers', () => {
    assert.match(appSource, /OwnerShellChrome/);
    assert.match(chatHomeSource, /OwnerShellChrome/);
    assert.match(appSource, /variant="console"/);
    assert.match(chatHomeSource, /variant="chat"/);
  });

  it('turns locked source errors into inline detail cards across Work and Chat Home', () => {
    assert.match(appSource, /function sensitiveAccessStatus\(problem: ApiResponseError\['problem'\]\)/);
    assert.match(appSource, /problem\.status \|\| problem\.error \|\| problem\.error_kind/);
    assert.match(appSource, /return \{ \.\.\.error\.problem, status \};/);
    assert.match(appSource, /const fallbackDetail = sourceDetailPayloadFromOpenError\(detailError, target, errorLocale\)/);
    assert.match(chatHomeSource, /<SourceDetailPanel sourceDetail=\{sourceDetail\}/);
    assert.match(chatHomeSource, /chatHome\.plain\.sensitiveReview/);
  });

  it('uses Chat Home as the default owner landing surface instead of the old Today dashboard', () => {
    assert.match(appSource, /ChatHomeView/);
    assert.match(chatHomeSource, /chatHome\.hero\.tellAegis/);
    assert.match(chatHomeSource, /vnext-chat-presence/);
    assert.match(chatHomeSource, /vnext-chat-presence-avatar/);
    assert.match(chatHomeSource, /chatHome\.hero\.ready/);
    // OwnerShellChrome receives collapsed prop (scroll-driven iOS large-title).
    assert.match(chatHomeSource, /OwnerShellChrome/);
    assert.match(chatHomeSource, /collapsed=\{hasConversation && topbarCollapsed\}/);
    assert.match(chatHomeSource, /topbarCollapsed/);
    // The collapse write is gated on hasConversation (empty hero scroll must not
    // poison the next conversation) AND reset when leaving conversation state.
    assert.match(chatHomeSource, /setTopbarCollapsed\(hasConversation && event\.currentTarget\.scrollTop > 24\)/);
    assert.match(chatHomeSource, /if \(!hasConversation\) setTopbarCollapsed\(false\)/);
    // The topbar collapse + opaque-fill styling must live in the ACTIVE chat-home
    // surface (loaded after + higher specificity than styles.css). Guards against
    // regressing the fix back into the clobbered base layer.
    assert.match(chatHomeCss, /\.vnext-app-chat-home \.vnext-chat-topbar\.is-collapsed\s*\{/);
    // Owner 2026-09-16 pass 2: the chat topbar is an OPAQUE bar (--b-bar) with
    // no blur — the frosted, shadowed card let the stream bleed through and read
    // as the "渐变阴影" the owner asked to remove. NOT transparent either.
    assert.match(chatHomeCss, /\.vnext-app-chat-home \.vnext-chat-topbar\s*\{[^}]*background:\s*var\(--b-bar\)/s);
    assert.match(chatHomeCss, /\.vnext-app-chat-home \.vnext-chat-topbar\s*\{[^}]*backdrop-filter:\s*none/s);
    assert.doesNotMatch(chatHomeCss, /\.vnext-app-chat-home \.vnext-chat-topbar\s*\{[^}]*frost/s);
    assert.doesNotMatch(chatHomeCss, /\.vnext-app-chat-home \.vnext-chat-topbar\s*\{[^}]*background:\s*transparent;/s);
    // Whitespace fix: the sticky topbar sits at top:0 (NOT max(0px,--safe-top)) —
    // the app padding already insets by the safe area, so safe-top here double-counted
    // it and left a phantom band above the header (WebKit-measured 118→59).
    assert.match(stylesSource, /\.vnext-chat-topbar\s*\{[\s\S]*?\n\s*top: 0;/);
    assert.match(chatHomeSource, /vnext-chat-stream/);
    assert.match(chatHomeSource, /vnext-chat-home/);
    assert.doesNotMatch(chatHomeSource, /Active context while chatting/);
    assert.match(chatHomeSource, /chatHome\.context\.attachable/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.context\.desktop'\)\}/);
    assert.match(chatHomeSource, /chatHome\.context\.nearbyWork/);
    assert.match(chatHomeSource, /brief\?: string/);
    assert.match(chatHomeSource, /className="vnext-chat-brief-card"/);
    assert.match(chatHomeSource, /showHeroBrief/);
    assert.match(chatHomeSource, /leadContextSignal = !hasConversation && showHeroBrief \? contextSignals\[0\] : null/);
    assert.match(chatHomeSource, /visibleContextSignals = leadContextSignal \? contextSignals\.slice\(1\) : contextSignals/);
    assert.match(chatHomeSource, /const signals = showAll \? visibleContextSignals : visibleSignals/);
    assert.match(chatHomeSource, /const overflowCount = showAll \? 0 : hiddenSignalCount/);
    assert.match(chatHomeSource, /overflowCount > 0/);
    assert.match(chatHomeSource, /\{tr\.t\('chatHome\.session\.work'\)\}/);
    assert.match(chatHomeSource, /onClick=\{\(\) => onOpenWork\(\)\}/);
    assert.match(chatHomeSource, /<VNextIcon name=\{prompt\.icon\} \/>/);
    assert.doesNotMatch(chatHomeSource, /vnext-chat-starters compact/);
    assert.match(appSource, /brief=\{home\.brief\}/);
    assert.match(appSource, /contextSignals=\{home\.signals\}/);
    assert.match(chatHomeSource, /chatHome\.account\.aegisStatus/);
    assert.match(workSource, /vnext-work-desktop-nearby/);
    assert.match(stylesSource, /\.vnext-work-desktop-nearby\s*\{[^}]*display: none;/s);
    assert.match(stylesSource, /@media \(min-width: 821px\)[\s\S]*\.vnext-work-desktop-nearby\s*\{[^}]*display: grid;/s);
    assert.doesNotMatch(chatHomeSource, /className="vnext-chat-status-button"/);
    assert.doesNotMatch(chatHomeSource, /className="vnext-chat-settings-button"/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.composer\.tellAria'\)\}/);
    assert.match(chatHomeSource, /<textarea/);
    // Composer size toggle (owner 2026-09-12): the "…" menu is gone — Attach is
    // the paperclip alone, and the size toggle is a direct button that flips
    // the mode with the same helper the menu item used.
    assert.doesNotMatch(chatHomeSource, /vnext-chat-more-button/);
    assert.doesNotMatch(chatHomeSource, /composerMoreMenuItems/);
    assert.match(chatHomeSource, /className="vnext-chat-size-button"/);
    assert.match(chatHomeSource, /onClick=\{\(\) => setComposerMode\(\(mode\) => nextComposerMode\(mode\)\)\}/);
    // Attach is ALSO a visible paperclip in the composer row (the owner never
    // found it inside "more"). iOS only opens the picker when `.click()` runs
    // synchronously in the tap handler, so both entries call it directly.
    assert.match(chatHomeSource, /className="vnext-chat-attach-button"/);
    assert.match(chatHomeSource, /onClick=\{\(\) => fileInputRef\.current\?\.click\(\)\}/);
    assert.doesNotMatch(chatHomeSource, /vnext-chat-size-toggle/);
    // Scope to a single tag so Slice 5's <input type="file"> is not a false match.
    assert.doesNotMatch(chatHomeSource, /<input[^>]*aria-label=\{tr\.t\('chatHome\.composer\.tellAria'\)\}/);
    // Clear/snapshot are menu-only (label via menu item, not topbar aria-label).
    assert.match(chatHomeSource, /chatHome\.account\.clearChat/);
    assert.match(chatHomeSource, /id: 'clear'/);
    assert.doesNotMatch(chatHomeSource, /className="vnext-chat-clear"/);
    assert.match(stylesSource, /\.vnext-app\.vnext-app-chat-home::after/);
    assert.match(stylesSource, /@keyframes vnext-chat-aurora/);
    assert.match(stylesSource, /@keyframes vnext-chat-presence-breathe/);
    assert.match(stylesSource, /@media \(prefers-reduced-motion: reduce\)/);
    assert.match(appSource, /const chatHomeGenerationRef = useRef\(0\)/);
    assert.match(appSource, /chatHomeGenerationRef\.current \+= 1/);
    assert.match(appSource, /chatHomeGenerationRef\.current !== chatHomeGeneration/);
    // Stuck-busy fix: the per-request finally releases busy only when the
    // session is unchanged (via shouldReleaseBusyOnRequestSettle), and a
    // session-reset failsafe effect force-releases busy on any teardown so an
    // in-flight request whose epoch/generation advanced can never wedge busy.
    assert.match(appSource, /shouldReleaseBusyOnRequestSettle\(/);
    assert.match(appSource, /shouldForceReleaseBusyOnSessionReset\(sessionResetTick\)/);
    assert.match(appSource, /const \[sessionResetTick, setSessionResetTick\] = useState\(0\)/);
    assert.match(appSource, /function invalidatePrivateSessionEpoch\(\)[\s\S]*setSessionResetTick\(\(tick\) => tick \+ 1\)/);
    assert.match(appSource, /function clearChatHomeConversation\(\)[\s\S]*setChatHomeCommand\(''\);[\s\S]*setQueuedChatHomeCommands\(\[\]\);[\s\S]*setQueuedChatHomeWorkPrompts\(\[\]\);[\s\S]*setChatHomeAttachedContext\(null\);[\s\S]*writeSessionDraft\(CHAT_HOME_DRAFT_KEY, ''\);/);
    assert.doesNotMatch(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.account\.clearChat'\)\}[\s\S]{0,180}disabled=\{busy\}/);
    assert.doesNotMatch(chatHomeSource, /role="menuitem"[\s\S]{0,160}disabled=\{busy\}[\s\S]{0,120}chatHome\.account\.clearChat/);
    assert.doesNotMatch(chatHomeSource, /chatHome\.account\.pauseChecks/);
    assert.match(settingsSource, /tr\('settings\.spending\.pauseTitle'\)/);
    assert.match(appSource, /onPauseCostSaving=\{\(\) => void pauseCostSavingControls\(\)\}/);
    assert.match(appSource, /onResumeCostSaving=\{\(\) => void resumeCostSavingControls\(\)\}/);
    assert.match(appSource, /const \[costSavingBusy, setCostSavingBusy\] = useState\(false\)/);
    assert.match(appSource, /costSavingBusy=\{costSavingBusy\}/);
    assert.match(appSource, /previousAegisAvatarUrl = avatarProfile\.aegisAvatarUrl/);
    assert.match(appSource, /const reconciledProfile = await fetchVNextAvatarProfile\(initData\)/);
    assert.match(appSource, /syncedProfile\.aegisAvatarUrl !== previousAegisAvatarUrl/);
    assert.match(appSource, /syncedProfile\.aegisAvatarUrl\.startsWith\('\/webapp\/vnext\/avatar-image\/'\)/);
    assert.match(appSource, /setSnapshotMessage\(t\('app\.avatarGeneratedReconciled', errorLocale\)\)/);
    assert.match(appSource, /snapshotMessage=\{workLoadNotice \|\| chatHomeNotice \|\| snapshotMessage\}/);
    assert.match(appSource, /currentOwnerInstallStatus\(false\)/);
    assert.match(appSource, /window\.addEventListener\('beforeinstallprompt', handleBeforeInstallPrompt\)/);
    assert.match(appSource, /window\.addEventListener\('appinstalled', handleAppInstalled\)/);
    assert.match(appSource, /window\.addEventListener\('popstate', onPopState\)/);
    assert.match(appSource, /window\.removeEventListener\('popstate', onPopState\)/);
    assert.match(appSource, /const next = readVNextLocationState\(window\.location\)/);
    assert.match(appSource, /setView\(next\.view\)/);
    assert.match(appSource, /setSelectedId\(next\.itemId\)/);
    assert.match(appSource, /writeVNextLocationState\(window, \{ view, itemId: selectedId \}, 'replace'\)/);
    assert.doesNotMatch(appSource, /view: 'mail'/);
    assert.match(appSource, /const requestedSelectedId = nextView === 'work' \? nextSelectedId : null;/);
    assert.match(appSource, /ownerInstallManualNotice\(navigator\.userAgent, errorLocale\)/);
    assert.match(appSource, /ownerInstallOutcomeNotice\(choice, errorLocale\)/);
    assert.match(chatHomeSource, /installOffered/);
    assert.match(chatHomeSource, /install\.action\.installOnDevice/);
    assert.match(chatHomeSource, /onInstallOwnerApp/);
    assert.doesNotMatch(chatHomeSource, /disabled=\{busy\}[\s\S]{0,120}onInstallOwnerApp\(\);/);
    assert.match(appSource, /setChatHomeCommand\(''\);\s*setChatHomeInputMode\('text'\);\s*if \(!contextOverride\) \{[\s\S]*setChatHomeAttachedContext\(null\);[\s\S]*setChatHomeNotice\(''\);\s*setSnapshotMessage\(''\);/);
    assert.match(appSource, /function clearChatHomeConversation\(\)[\s\S]*setChatHomeNotice\(t\('chatHome\.notice\.chatCleared', errorLocale\)\)/);
    assert.match(appSource, /async function pauseCostSavingControls\(\)[\s\S]*setSnapshotMessage\(''\);\s*setChatHomeNotice\(''\);/);
    assert.match(appSource, /async function resumeCostSavingControls\(\)[\s\S]*setSnapshotMessage\(''\);\s*setChatHomeNotice\(''\);/);
    assert.match(appSource, /async function pauseCostSavingControls\(\)[\s\S]*setCostSavingBusy\(true\);[\s\S]*setCostSavingBusy\(false\);/);
    assert.match(appSource, /async function resumeCostSavingControls\(\)[\s\S]*setCostSavingBusy\(true\);[\s\S]*setCostSavingBusy\(false\);/);
    assert.doesNotMatch(appSource, /async function pauseCostSavingControls\(\)[\s\S]*?setBusy\(true\);[\s\S]*?async function resumeCostSavingControls/);
    assert.doesNotMatch(appSource, /async function resumeCostSavingControls\(\)[\s\S]*?setBusy\(true\);[\s\S]*?async function logoutNow/);
    assert.match(appSource, /async function createDebugSnapshot\(\)[\s\S]*setSnapshotMessage\(''\);\s*setChatHomeNotice\(''\);/);
    assert.match(appSource, /resumeVNextCostSaving/);
    assert.match(settingsSource, /onResumeCostSaving/);
    assert.match(settingsSource, /if \(checked\) onPauseCostSaving\(\);\s*else onResumeCostSaving\(\);/);
    assert.match(settingsSource, /disabled=\{costSavingBusy\}/);
    assert.doesNotMatch(settingsSource, /disabled=\{busy\}/);
    // Pause-checks moved out of the home menu into the Settings runtime toggle.
    assert.doesNotMatch(chatHomeSource, /disabled: costSavingBusy/);
    assert.match(settingsSource, /disabled=\{costSavingBusy\}/);
    assert.match(chatHomeSource, /onRetry/);
    assert.match(chatHomeSource, /chatHome\.retry\.tryAgain/);
    assert.match(chatHomeSource, /AssistantResponseView/);
    assert.match(chatHomeSource, /function ChatWorkCard/);
    assert.match(chatHomeSource, /message\.workItem/);
    assert.match(chatHomeSource, /chatHome\.work\.cardLabel/);
    assert.match(chatHomeSource, /chatHome\.work\.askAegis/);
    assert.match(appSource, /requestAssistantCommandWithAutoCheck/);
    assert.match(appSource, /workItem: command\.workItem/);
    assert.match(appSource, /function askChatHomeWorkItem/);
    assert.match(appSource, /async function askChatHomeWorkItem\([\s\S]*messageId: string,[\s\S]*item: OwnerWorkItem,[\s\S]*ownerText: string,[\s\S]*queuedThreadId\?: string,[\s\S]*\)/);
    assert.match(appSource, /function chatHomeWorkConversationContext\(messageId: string, itemStatus\?: string \| null, excludeThreadId\?: string\): VNextConversationContextTurn\[\]/);
    assert.match(appSource, /const conversationContext = chatHomeWorkConversationContext\(messageId, item\.status, queuedThreadId\)/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.work\.askAboutItem'\)\}/);
    assert.match(stylesSource, /\.vnext-chat-work-ask/);
    assert.match(stylesSource, /\.vnext-chat-work-thread/);
    assert.match(chatHomeSource, /thread=\{message\.workItemThread \?\? \[\]\}/);
    assert.match(chatHomeSource, /threadActivityKey/);
    assert.match(chatHomeSource, /askInputRef\.current\?\.focus\(\);/);
    assert.match(chatHomeSource, /entry\.result\.response_kind === 'error'/);
    assert.match(chatHomeSource, /onClick=\{\(\) => onAsk\(entry\.question\)\}/);
    assert.match(appSource, /focusId: item\.id/);
    assert.match(appSource, /conversationContext,/);
    assert.match(appSource, /submitChatHomeCommand/);
    assert.match(appSource, /retryText: text/);
    assert.match(appSource, /idempotencyKey: retryKey/);
    assert.match(appSource, /retryLabel: retryKey \? 'Check result' : 'Try again'/);
    assert.match(appSource, /contextOverride\?: ChatHomeAttachedContext/);
    assert.match(appSource, /const requestContext = contextOverride \?\? chatHomeAttachedContext/);
    assert.match(appSource, /setChatHomeAttachedContext\(null\)/);
    assert.match(appSource, /contextLabel: requestContext\.label/);
    assert.match(appSource, /focusId: requestContext\?\.itemId \?\? GENERAL_ASSISTANT_FOCUS_ID/);
    assert.match(appSource, /scope: requestContext \? 'item' : 'general'/);
    assert.match(appSource, /retryContext: requestContext/);
    assert.match(chatHomeSource, /className="vnext-chat-message-context"/);
    assert.match(chatHomeSource, /onRetry\(message\.retryText \|\| '', message\.retryIdempotencyKey, message\.retryContext, message\.retryQuote\)/);
    assert.match(appSource, /vnext-app-chat-home/);
    assert.match(stylesSource, /\.vnext-chat-home\.is-empty\s*\{[^}]*grid-template-rows: auto minmax\(0, 1fr\) auto;[^}]*align-content: stretch;/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*?\.vnext-chat-home\.is-empty\s*\{[^}]*grid-template-rows: auto minmax\(0, 1fr\) auto;[^}]*align-content: stretch;/s);
    assert.match(stylesSource, /\.vnext-chat-stream\.is-empty\s*\{[^}]*min-height: 0;[^}]*display: grid;[^}]*align-items: start;/s);
    assert.match(stylesSource, /\.vnext-chat-empty-stage\s*\{[^}]*min-height: 100%;[^}]*align-items: start;[^}]*padding-top: clamp\(12px, 5vh, 52px\);/s);
    assert.doesNotMatch(chatHomeSource, /Context-aware\. Permissioned\. Receipt-backed\./);
    assert.doesNotMatch(chatHomeSource, /OPERATING_FLOW/);
    assert.doesNotMatch(chatHomeSource, /vnext-chat-operating-rail/);
    assert.doesNotMatch(chatHomeSource, /Aegis request path/);
    // Brand-rail is ALWAYS mounted (including chat-home) so desktop owners have
    // the same Chat/Work map on every surface. Mobile still hides the rail
    // via the shared max-width:820px rule and uses bottom nav.
    assert.doesNotMatch(stylesSource, /\.vnext-app-chat-home \.vnext-brand-rail\s*\{[^}]*display: none;/s);
    assert.doesNotMatch(stylesSource, /\.vnext-app-chat-home \.vnext-nav\s*\{[^}]*display: none;/s);
    assert.match(appSource, /ownerPrimaryNavItemIds\(\)/);
    assert.match(presentationSource, /export function ownerPrimaryNavItemIds/);
    // Surface header (title/kicker) stays non-home only; chat keeps its topbar.
    assert.match(appSource, /view !== 'home' && \(/);
    assert.match(stylesSource, /\.vnext-chat-work-card\s*\{/);
    assert.match(installPromptSource, /ownerInstallStatusFromSignals/);
    assert.match(installPromptSource, /ownerInstallActionLabel/);
    assert.match(installPromptSource, /ownerInstallManualNotice/);
    assert.doesNotMatch(appSource, /Notification\.requestPermission/);
    assert.doesNotMatch(chatHomeSource, /Useful now/);
    assert.doesNotMatch(chatHomeSource, /vnext-chat-trust-strip/);
    assert.doesNotMatch(chatHomeSource, /Same Aegis as Telegram/);
    assert.doesNotMatch(appSource, /import \{ HomeView \}/);
    assert.doesNotMatch(appSource, /<HomeView/);
    assert.doesNotMatch(workSource, /from '.\/HomeView'/);
    assert.match(workSource, /from '.\/WorkItemRow'/);
    assert.doesNotMatch(stylesSource, /vnext-today-/);
    assert.doesNotMatch(stylesSource, /vnext-primary-work/);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*?\.vnext-chat-empty-stage\s*\{[^}]*align-items: start;[^}]*padding-top: 8px;/s);
  });

  it('keeps owner-triggered UX snapshots available without adding a navigation surface', () => {
    assert.match(appSource, /submitVNextDebugSnapshot/);
    // SP2: the snapshot affordance is now a gear-menu ITEM (camera icon), gated on
    // snapshotAvailable, calling createDebugSnapshot — not a direct header button.
    assert.match(appSource, /id: 'snapshot'[\s\S]*?icon: 'camera' as const[\s\S]*?onSelect: \(\) => void createDebugSnapshot\(\)/);
    assert.match(appSource, /label: snapshotBusy \? t\('app\.shell\.header\.saving', errorLocale\) : t\('app\.shell\.header\.snapshot', errorLocale\)/);
    assert.match(catalogSource, /'app\.shell\.header\.snapshot':.*en: 'Snapshot'/);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*?\.vnext-header\s*\{[^}]*margin-bottom: 12px;/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*?\.vnext-header \.vnext-kicker\s*\{[^}]*display: none;/s);
    assert.match(appSource, /setSnapshotMessage\(tf\('app\.snapshotSaved'/);
    assert.match(appSource, /const snapshotAvailable = Boolean\(/);
    assert.match(appSource, /statusProjection\?\.advanced_diagnostics_available/);
    assert.match(appSource, /settingsProjection\?\.advanced_diagnostics_available/);
    assert.match(appSource, /if \(!snapshotAvailable\) \{/);
    assert.match(appSource, /setSnapshotMessage\(t\('app\.snapshotNotEnabled', errorLocale\)\)/);
    assert.match(appSource, /menuTriggerLabel=\{t\('app\.shell\.header\.menu', errorLocale\)\}/);
    assert.match(appSource, /snapshotAvailable=\{snapshotAvailable\}/);
    // Chat home no longer carries the snapshot row; Settings keeps the same action.
    assert.doesNotMatch(chatHomeSource, /\.\.\.\(snapshotAvailable \? \[\{/);
    assert.doesNotMatch(chatHomeSource, /className="vnext-chat-snapshot"/);
    assert.doesNotMatch(chatHomeSource, /id: 'snapshot'/);
    assert.match(settingsSource, /data-settings-action="support-snapshot"/);
    assert.match(settingsSource, /data-settings-section="runtime"/);
    assert.match(settingsSource, /const supportSection = snapshotAvailable \? \(/);
    assert.match(appSource, /submitVNextDebugSnapshot\(\s*initData,\s*session\?\.csrf_token,\s*view,/);
    // Support/debug copy was migrated to the i18n catalog (settings.support.* /
    // settings.terminal.*); assert the catalog-key wiring, not raw literals.
    assert.match(settingsSource, /tr\('settings\.support\.saveSnapshot'\)/);
    assert.match(settingsSource, /tr\('settings\.support\.backendE2eTitle'\)/);
    assert.match(settingsSource, /tr\('settings\.support\.runBackendE2e'\)/);
    assert.match(settingsSource, /tr\('settings\.support\.saveSnapshot'\)/);
    assert.match(settingsSource, /tr\('settings\.terminal\.summary'\)/);
    assert.doesNotMatch(appSource, /className="vnext-secondary vnext-header-snapshot"[\s\S]{0,260}disabled=\{busy \|\| snapshotBusy\}/);
    assert.doesNotMatch(settingsSource, /Save snapshot[\s\S]{0,260}disabled=\{busy \|\| snapshotBusy\}/);
    assert.doesNotMatch(chatHomeSource, /Save support snapshot[\s\S]{0,260}disabled=\{busy \|\| snapshotBusy\}/);
    // The snapshot menu item carries the busy state as `disabled: snapshotBusy`.
    assert.match(appSource, /disabled: snapshotBusy/);
    assert.doesNotMatch(settingsSource, /for debugging/);
    assert.doesNotMatch(appSource, /label="Snapshot"/);
    assert.doesNotMatch(shellSource, /Advanced diagnostics/);
  });

  it('refreshes all provider cards after any chat-provider routing mutation', () => {
    assert.match(appSource, /async function refreshProviderStatuses\(\)/);
    assert.match(appSource, /fetchVNextCodexStatus\(initData\)\.catch\(\(\) => null\)/);
    assert.match(appSource, /fetchVNextGrokStatus\(initData\)\.catch\(\(\) => null\)/);

    for (const functionName of [
      'useCodexForChat',
      'stopUsingCodexForChat',
      'revokeGrokConnection',
      'useGrokForChat',
      'stopUsingGrokForChat',
    ]) {
      const start = appSource.indexOf(`async function ${functionName}`);
      assert.ok(start >= 0, `missing ${functionName}`);
      const nextFunction = appSource.indexOf('\n  async function ', start + 1);
      const body = appSource.slice(start, nextFunction > start ? nextFunction : undefined);
      assert.match(body, /await refreshProviderStatuses\(\);/, `${functionName} must refresh both provider status cards`);
      assert.doesNotMatch(body, /await refreshGrokStatus\(\)\.catch\(\(\) => null\);/, `${functionName} must not refresh only Grok`);
      assert.doesNotMatch(body, /await refreshCodexStatus\(\)\.catch\(\(\) => null\);/, `${functionName} must not refresh only Codex`);
    }
  });

  it('does not route ordinary assistant suggestions into action drafts', () => {
    assert.doesNotMatch(appSource, /prepareSuggestedIntent/);
    assert.doesNotMatch(appSource, /saveSuggestedIntentDraft/);
    assert.doesNotMatch(appSource, /actionDraftKindForSuggestedIntent/);
    assert.doesNotMatch(assistantActionsSource, /createOwnerWorkActionDraft/);
    assert.doesNotMatch(workSource, /Prepare next step/);
  });

  it('does not offer draft create or draft cancel as item actions', () => {
    assert.doesNotMatch(modelSource, /draft\.create_task/);
    assert.doesNotMatch(modelSource, /draft\.save_reminder_task/);
    assert.doesNotMatch(modelSource, /draft\.cancel/);
    assert.doesNotMatch(itemActionsSource, /createTaskFromOwnerWorkActionDraft/);
    assert.doesNotMatch(itemActionsSource, /cancelOwnerWorkActionDraft/);
  });

  it('renders the shell header only in full mode, never inside the sheet', () => {
    assert.match(chatHomeSource, /\{presentationMode === 'full' \? \(\s*<OwnerShellChrome/);
  });

  it('every command context goes through the capped merge (server accepts 8 turns)', () => {
    assert.doesNotMatch(appSource, /\.slice\(-12\)/);
    assert.equal((appSource.match(/mergeConversationContext\(/g) || []).length, 2);
  });

  it('keeps general commands on the canonical chat home runtime path', () => {
    assert.match(assistantActionsSource, /submitVNextCommand/);
    assert.match(assistantActionsSource, /requestAssistantResultWithAutoCheck/);
    assert.match(assistantActionsSource, /STILL_WORKING_STATUS_CODES.*new Set\(\[408, 502, 504, 524\]\)/);
    assert.match(assistantActionsSource, /checkStatus,\n  \}\);/);
    assert.match(assistantActionsSource, /requestWithStillWorkingAutoCheck/);
    assert.match(assistantActionsSource, /checksUsed < ASSISTANT_STILL_WORKING_RETRY_DELAYS_MS\.length/);
    assert.match(assistantActionsSource, /shouldRetry: shouldAutoCheckAssistantResult/);
    // The canned explain/draft questions live in the pure fallback module and
    // are only used for item-page quick actions (never for a chat turn or an
    // attachment-only send — the 2026-09-13 voice-note defect).
    assert.match(assistantActionsSource, /text \|\| assistantCommandFallbackText\(intent, attachments\.length > 0\)/);
    assert.match(assistantFallbackSource, /请简明说明这件事是什么/);
    assert.match(assistantFallbackSource, /请基于这件事起草一个有用的下一步方案/);
    assert.match(assistantFallbackSource, /if \(hasAttachments \|\| intent === 'chat'\) return '';/);
    // briefPrompt/draftPrompt now localize via the catalog (rendered as owner bubbles).
    assert.match(workSource, /t\('work\.assistant\.prompt\.brief', locale\)/);
    assert.match(workSource, /t\('work\.assistant\.prompt\.draft', locale\)/);
    assert.match(catalogSource, /Brief this item\./);
    assert.match(catalogSource, /Draft a next step\./);
    assert.doesNotMatch(workSource, /不要执行任何外部动作/);
    assert.doesNotMatch(assistantActionsSource, /requestOwnerWorkResponse/);
    assert.doesNotMatch(assistantActionsSource, /isMissingVNextEndpoint/);
    assert.match(appSource, /requestAssistantResultWithAutoCheck/);
    // The still-working message is localized: the callback substitutes the
    // display_language version instead of forwarding the util's English default.
    assert.match(appSource, /paintWorkingProgress\(t\('chatHome\.working\.stillChecking', errorLocale\)\)/);
    assert.match(appSource, /focusId: requestContext\?\.itemId \?\? GENERAL_ASSISTANT_FOCUS_ID/);
    assert.match(appSource, /scope: requestContext \? 'item' : 'general'/);
    assert.doesNotMatch(appSource, /setView\('work'\);\n\s+await requestAssistantResult/);
    assert.doesNotMatch(appSource, /<CommandBar/);
  });

  it('keeps item actions on the canonical vNext action endpoint', () => {
    assert.match(itemActionsSource, /submitVNextWorkAction/);
    assert.doesNotMatch(itemActionsSource, /executeLegacyItemActionFallback/);
    assert.doesNotMatch(itemActionsSource, /setConsoleMailReadState/);
    assert.doesNotMatch(itemActionsSource, /setConsoleMailMailboxState/);
    assert.doesNotMatch(itemActionsSource, /updateConsoleTaskAction/);
    assert.doesNotMatch(itemActionsSource, /isMissingVNextEndpoint/);
  });

  it('submits the current DOM input value for every assistant composer', () => {
    assert.match(chatHomeSource, /const raw = \(inputRef\.current\?\.value \?\? value\)\.trim\(\);/);
    assert.match(chatHomeSource, /onSubmit\(raw\);/);
    assert.match(inlineAssistantSource, /onSend\(inputRef\.current\?\.value \?\? text\);/);
    assert.match(appSource, /async function submitChatHomeCommand\([\s\S]*prompt\?: string,[\s\S]*retryIdempotencyKey\?: string,[\s\S]*contextOverride\?: ChatHomeAttachedContext,[\s\S]*\)/);
    assert.match(appSource, /const text = \(prompt \?\? chatHomeCommand\)\.trim\(\);/);
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.composer\.attachContext'\)\}/);
    assert.match(chatHomeSource, /aria-expanded=\{contextPickerOpen\}/);
    assert.match(appSource, /requestContext\?\.itemId \?\? GENERAL_ASSISTANT_FOCUS_ID/);
    assert.match(appSource, /scope: requestContext \? 'item' : 'general'/);
    assert.match(chatHomeSource, /chatHome\.composer\.removeAttached/);
    assert.match(chatHomeSource, /renderContextSignals\('attach', true\)/);
    assert.match(chatHomeSource, /placeholder=\{busy \? tr\.t\('chatHome\.composer\.busyPlaceholder'\) : tr\.t\('chatHome\.composer\.placeholder'\)\}/);
    assert.doesNotMatch(chatHomeSource, /className="vnext-chat-voice"[\s\S]{0,220}disabled=\{busy\}/);
    assert.doesNotMatch(chatHomeSource, /className="vnext-chat-context-button"[\s\S]{0,220}disabled=\{busy\}/);
    assert.doesNotMatch(chatHomeSource, /STARTER_PROMPTS\.map[\s\S]*?disabled=\{busy\}[\s\S]*?onSuggestion\(prompt\.prompt, promptLabel\)/);
    assert.doesNotMatch(chatHomeSource, /signals\.map[\s\S]*?disabled=\{busy\}[\s\S]*?handleSignal/);
    assert.doesNotMatch(chatHomeSource, /Open full Work/);
    // Send stays live for an attachment-only turn (voice note / image / PDF
    // with no typed text) — that is the fix for "attaching an image does nothing".
    assert.match(chatHomeSource, /const sendDisabled = attachmentBlockingSend \|\| \(!value\.trim\(\) && !canSendAttachmentOnly\);/);
    assert.match(chatHomeSource, /disabled=\{sendDisabled\}/);
    assert.match(chatHomeSource, /aria-label=\{busy \? tr\.t\('chatHome\.composer\.queueForAegis'\) : tr\.t\('chatHome\.composer\.sendToAegis'\)\}/);
    assert.match(chatHomeSource, /\{busy \? tr\.t\('chatHome\.composer\.queue'\) : tr\.t\('chatHome\.composer\.send'\)\}/);
    assert.match(chatHomeSource, /role: 'owner' \| 'aegis' \| 'working' \| 'queued'/);
    assert.match(chatHomeSource, /vnext-chat-message-state/);
    assert.match(stylesSource, /\.vnext-chat-message\.queued/);
    assert.match(appSource, /type QueuedChatHomeCommand =/);
    assert.match(appSource, /const MAX_CHAT_HOME_QUEUED_COMMANDS = 4;/);
    assert.match(appSource, /queuedChatHomeCommands\.length >= MAX_CHAT_HOME_QUEUED_COMMANDS/);
    assert.match(appSource, /tf\('chatHome\.notice\.queueFullCommands', errorLocale/);
    assert.doesNotMatch(appSource, /\.slice\(-4\)/);
    assert.match(appSource, /queuedMessageId/);
    assert.match(appSource, /retryIdempotencyKey\?: string/);
    assert.match(appSource, /function queueChatHomeCommand/);
    assert.match(appSource, /queueChatHomeCommand\(prompt, contextOverride, modeOverride, displayTextOverride, retryIdempotencyKey, quoteOverride, queueAttachments\)/);
    assert.match(appSource, /nextCommand\.retryIdempotencyKey/);
    assert.match(appSource, /role: 'queued'/);
    assert.match(appSource, /return \{ \.\.\.message, role: 'owner' as const, turnId: commandIdempotencyKey \};/);
    assert.match(appSource, /setQueuedChatHomeCommands\(remaining\)/);
    assert.match(appSource, /setChatHomeNotice\(t\('chatHome\.notice\.queued', errorLocale\)\)/);
    assert.doesNotMatch(chatHomeSource, /disabled=\{busy \|\| !value\.trim\(\)\}/);
    assert.doesNotMatch(chatHomeSource, /message\.result\.response_kind === 'error'[\s\S]{0,260}disabled=\{busy\}/);
    assert.match(chatHomeSource, /chatHome\.retry\.queueRetry/);
    const mainComposerTextarea = chatHomeSource.match(/<textarea[\s\S]*?aria-label=\{tr\.t\('chatHome\.composer\.tellAria'\)\}[\s\S]*?\/>/)?.[0] ?? '';
    assert.match(mainComposerTextarea, /aria-label=\{tr\.t\('chatHome\.composer\.tellAria'\)\}/);
    assert.doesNotMatch(mainComposerTextarea, /disabled=\{busy\}/);
    assert.match(stylesSource, /\.vnext-chat-message\.owner[\s\S]*min-width:\s*96px/);
    assert.match(stylesSource, /\.vnext-chat-message\.owner p[\s\S]*white-space:\s*pre-wrap/);
    assert.match(appSource, /import type \{ DeliveryNoticeAction \} from '\.\/notificationActions';/);
    assert.match(appSource, /async function executeChatNoticeAction\(action: DeliveryNoticeAction\)/);
    assert.match(appSource, /if \(!action\.directTaskAction\)/);
    assert.match(appSource, /submitChatHomeCommand\(action\.command, undefined, undefined, 'text', action\.label\)/);
    assert.match(appSource, /updateConsoleTaskAction\(/);
    assert.match(appSource, /action\.directTaskAction\?\.preset/);
    assert.match(chatHomeSource, /onNoticeAction: \(action: DeliveryNoticeAction\) => Promise<ChatHomeNoticeActionResult>/);
    assert.match(chatHomeSource, /disabled=\{executingCommand !== ''\}/);
  });

  it('refreshes the owner work queue after assistant turns without failing the reply', () => {
    assert.match(appSource, /async function refreshQueueAfterAssistant/);
    assert.match(appSource, /catch\s*\{\s*\/\/ Assistant answers remain useful/s);
    assert.match(appSource, /void refreshQueueAfterAssistant\(focusId\);/);
    assert.match(appSource, /void refreshQueueAfterAssistant\(command\.workItem\?\.id \?\? requestContext\?\.itemId\);/);
    assert.match(appSource, /\[initData, selected\?\.id, session\?\.authenticated, session\?\.csrf_token\]/);
    assert.doesNotMatch(appSource, /await refreshQueueAfterAssistant/);
    assert.doesNotMatch(appSource, /\[initData, selected, session\]/);
  });

  it('self-heals a stale CSRF/session on the home chat command, like the work action path', () => {
    // Production outage guard: webapp chat returned "I couldn't finish that"
    // for every message while Telegram + mail kept working. The mail/work
    // action path (`executeConfirmedItemAction`) recovers from a stale or
    // missing per-session CSRF token by refreshing the session and retrying;
    // the home chat command path did NOT, so any 403 (e.g. after a backend
    // restart rotated the session) surfaced the generic failure with no
    // recovery. The home chat command must reuse the SAME self-heal helpers.
    //
    // 1. The command is invoked through a reusable closure parameterized by the
    //    CSRF token (so the same call can be retried with a refreshed token).
    assert.match(appSource, /const runCommand = \(token: string \| undefined\) => requestAssistantCommandWithAutoCheck\(/);
    assert.match(appSource, /csrfToken: token,/);
    // 2. A missing token is refreshed pre-emptively (mirrors the mail path),
    //    gated to web-console sessions (Telegram needs no CSRF) and guarded so a
    //    cleared conversation can't still POST after the in-flight refresh.
    assert.match(appSource, /let commandCsrfToken = session\?\.csrf_token;/);
    assert.match(appSource, /if \(!commandCsrfToken && session\?\.authenticated && session\?\.principal_type === 'web_console'\) \{[\s\S]*?refreshSessionForAction\(sessionEpoch\)[\s\S]*?chatHomeGenerationRef\.current !== chatHomeGeneration[\s\S]*?commandCsrfToken = refreshed\?\.csrf_token;/);
    // 3. A refreshable 403 triggers exactly one session refresh + retry.
    assert.match(appSource, /command = await runCommand\(commandCsrfToken\);/);
    assert.match(appSource, /if \(!isRefreshableSessionActionError\(firstCommandError\)\) throw firstCommandError;/);
    assert.match(appSource, /command = await runCommand\(refreshed\?\.csrf_token\);/);
    // The pre-existing direct-call shape (no retry) must be gone.
    assert.doesNotMatch(appSource, /const command = await requestAssistantCommandWithAutoCheck\(\{\s*initData,\s*csrfToken: session\?\.csrf_token,/);
  });

  it('keeps chat work cards in sync with refreshed queue state', () => {
    assert.match(appSource, /syncChatWorkMessagesWithQueue/);
    assert.match(appSource, /updateChatWorkMessagesAfterItemStatus/);
    assert.match(appSource, /syncChatHomeWorkItemsFromQueue\(projectedQueue\)/);
    assert.match(chatWorkStateSource, /const itemsById = new Map\(nextQueue\.items\.map\(\(item\) => \[item\.id, item\]\)\)/);
    assert.match(chatWorkStateSource, /const statusChanged = nextItem\.status !== message\.workItem\.status;/);
    assert.match(chatWorkStateSource, /workItemThread: statusChanged \? \[\] : message\.workItemThread/);
  });

  it('keeps normal Work action feedback bound to the selected item', () => {
    assert.match(appSource, /const \[actionMessagesByItemId, setActionMessagesByItemId\] = useState<Record<string, string>>\(\{\}\)/);
    assert.match(appSource, /const \[retiredActionItemIds, setRetiredActionItemIds\] = useState<Record<string, boolean>>\(\{\}\)/);
    assert.match(appSource, /const \[armedActionItemId, setArmedActionItemId\] = useState<string \| null>\(null\)/);
    assert.match(appSource, /function setItemActionMessage\(itemId: string, message: string\)/);
    assert.match(appSource, /const selectedActionMessage = selected\?\.id \? actionMessagesByItemId\[selected\.id\] \|\| '' : '';/);
    assert.match(appSource, /const selectedActionsRetired = selected\?\.id \? Boolean\(retiredActionItemIds\[selected\.id\]\) : false;/);
    assert.match(appSource, /actionMessage=\{selectedActionMessage\}/);
    assert.match(appSource, /workNotice=\{workNotice\}/);
    assert.match(appSource, /actionsRetired=\{selectedActionsRetired\}/);
    assert.match(appSource, /setItemActionMessage\(target\.id, successMessage\)/);
    assert.match(appSource, /setWorkNotice\(successMessage\)/);
    assert.match(appSource, /setRetiredActionItemIds\(\(current\) => \(\{ \.\.\.current, \[target\.id\]: true \}\)\)/);
    assert.match(appSource, /for \(const item of projectedQueue\.items\) \{[\s\S]*delete next\[item\.id\];/);
    assert.match(appSource, /itemActionConfirmationHint\(action, errorLocale\)/);
    assert.match(appSource, /armedAction !== action\.id \|\| armedActionItemId !== target\.id/);
    assert.match(appSource, /setArmedActionItemId\(target\.id\)/);
    assert.match(appSource, /setItemActionMessage\(target\.id, itemActionConfirmationHint\(action, errorLocale\)\)/);
    assert.doesNotMatch(appSource, /It left the active queue after refresh/);
    assert.doesNotMatch(appSource, /setActionMessage\(/);
  });

  it('keeps the Aegis command entry near the top of the product shell', () => {
    const commandIndex = appSource.indexOf('<ChatHomeView');
    const mainIndex = appSource.indexOf('<main className="vnext-main"');
    const mobilePadding = stylesSource.match(/padding-bottom:\s*calc\((\d+)px \+ var\(--safe-bottom, 0px\)\)/);
    assert.ok(commandIndex > 0, 'ChatHomeView should render in the product shell');
    assert.ok(mainIndex > 0, 'main content should render in the product shell');
    assert.ok(commandIndex > mainIndex, 'Chat Home should own the first page inside main content');
    assert.ok(mobilePadding, 'mobile shell should reserve bottom nav space explicitly');
    assert.ok(Number(mobilePadding[1]) <= 100, 'mobile shell should not reserve obsolete bottom composer space');
  });

  it('syncs avatar customization through the signed-in owner profile, with local cache only as fallback', () => {
    assert.match(appSource, /fetchVNextAvatarProfile/);
    assert.match(appSource, /updateVNextAvatarProfile/);
    assert.match(appSource, /avatarProfileFromServer/);
    assert.match(appSource, /avatarProfileToServer/);
    assert.match(appSource, /fetchVNextAvatarProfile\(initData\)/);
    assert.match(appSource, /updateVNextAvatarProfile\(initData, session\.csrf_token, avatarProfileToServer\(next\)\)/);
    assert.match(appSource, /writeAvatarProfile\(window\.localStorage, ownerKey, syncedProfile\)/);
    // The avatar server-sync failure copy is now catalog-backed (localized) so it
    // no longer appears as an inline English literal in App.tsx; it lives in the
    // catalog so the zh error template interpolates a Chinese fallback.
    assert.match(appSource, /displayError\(syncError, t\('error\.fallback\.avatarServerSyncFailed', errorLocale\), errorLocale\)/);
    assert.match(catalogSource, /'error\.fallback\.avatarServerSyncFailed':[\s\S]*?en: 'Avatar saved on this device, but server sync failed'/);
  });

  it('keeps Settings operation results local to the relevant controls', () => {
    // 2026-09-13: a settings message that no inline slot claims (e.g. a failed
    // language/provider write) falls back to the shared toast instead of vanishing.
    assert.match(appSource, /view !== 'home' && \(view !== 'settings' \|\| routeSettingsSnapshotMessage\(snapshotMessage\) === null\) && snapshotMessage/);
    assert.match(settingsSource, /export function routeSettingsSnapshotMessage/);
    assert.match(settingsSource, /routeSettingsSnapshotMessage\(snapshotMessage\)/);
    assert.match(settingsSource, /avatarInlineMessage/);
    assert.match(settingsSource, /costInlineMessage/);
    assert.match(settingsSource, /supportInlineMessage/);
    assert.match(settingsSource, /vnext-settings-inline-note/);
    assert.match(settingsSource, /setPreviewAvatarRole\('aegis'\)/);
    assert.match(settingsSource, /setPreviewAvatarRole\('owner'\)/);
    assert.match(settingsSource, /AvatarPreviewDialog/);
    assert.match(settingsSource, /role="dialog"/);
    assert.match(settingsSource, /event\.key === 'Escape'/);
    assert.match(stylesSource, /\.vnext-avatar-preview-backdrop/);
    assert.match(stylesSource, /\.vnext-avatar-preview-large\.vnext-avatar/);
  });

  it('uses safe-area fallbacks for normal browser rendering', () => {
    assert.match(stylesSource, /var\(--safe-top, 0px\)/);
    assert.match(stylesSource, /var\(--safe-bottom, 0px\)/);
    assert.match(stylesSource, /var\(--safe-left, 0px\)/);
    assert.match(stylesSource, /var\(--safe-right, 0px\)/);
    assert.doesNotMatch(stylesSource, /var\(--safe-(top|bottom|left|right)\)(?![,\\w-])/);
  });

  it('keeps useful product primitives: assistant progress, logout, and safe rich rendering', () => {
    assert.match(assistantActionsSource, /response_kind: 'working'/);
    assert.match(appSource, /await logout\(csrfToken\);/);
    assert.match(appSource, /privateSessionEpochRef = useRef\(0\)/);
    assert.match(appSource, /function invalidatePrivateSessionEpoch\(\)/);
    assert.match(appSource, /function isPrivateSessionEpochCurrent\(epoch: number\)/);
    assert.match(appSource, /clearPrivateBrowserSessionState\(\)[\s\S]*invalidatePrivateSessionEpoch\(\)/);
    assert.match(appSource, /async function logoutNow\(\)[\s\S]*setSession\(\{ authenticated: false \}\);[\s\S]*clearPrivateBrowserSessionState\(\);/);
    assert.match(appSource, /if \(!isPrivateSessionEpochCurrent\(sessionEpoch\)\) return;/);
    assert.match(appSource, /logoutBusy=\{logoutBusy\}/);
    // SP2: the home logout is now a destructive gear-menu item (disabled: logoutBusy).
    assert.match(chatHomeSource, /id: 'logout'[\s\S]*?tone: 'destructive'[\s\S]*?disabled: logoutBusy/);
    assert.match(settingsSource, /disabled=\{logoutBusy\}/);
    assert.doesNotMatch(chatHomeSource, /vnext-chat-account-danger" role="menuitem" disabled=\{busy\}/);
    assert.doesNotMatch(settingsSource, /className="vnext-danger" disabled=\{busy\}/);
    // Composer dictation is gone; voice agent is realtime-only (no browser STT panel).
    assert.doesNotMatch(appSource, /startBrowserVoiceInput/);
    assert.doesNotMatch(ceoVoiceAgentSource, /startBrowserVoiceInput/);
    assert.match(appSource, /chatHomeInputMode/);
    assert.match(appSource, /mode: modeOverride/);
    assert.match(appSource, /setChatHomeInputMode\('voice'\)/);
    assert.match(appSource, /setChatHomeInputMode\('text'\)/);
    assert.match(assistantActionsSource, /mode\?: 'text' \| 'voice'/);
    assert.match(voiceInputSource, /webkitSpeechRecognition/);
    assert.match(voiceInputSource, /onTranscriptCaptured\?\.\(\)/);
    assert.match(voiceInputSource, /voice\.notice\.captured/);
    assert.match(voiceInputSource, /onAutoSend/);
    assert.match(appSource, /CeoVoiceAgent/);
    // Shell unification: top-bar summon replaces floating orb dock.
    assert.match(appSource, /chromeMode/);
    assert.match(appSource, /onChromeState|voiceChrome|voiceSummon|toggleVoice/);
    assert.match(ceoVoiceAgentSource, /chromeMode/);
    assert.match(ceoVoiceAgentSource, /onChromeState/);
    assert.match(ceoVoiceAgentSource, /toggleVoice/);
    assert.match(appSource, /mintVNextVoiceSession/);
    assert.match(appSource, /secretaryHandlersRef/);
    assert.match(appSource, /buildCeoScreenContext/);
    assert.match(appSource, /aegisAvatarUrl=\{avatarProfile\.aegisAvatarUrl\}/);
    // Design B: one ChatHomeView; desk sends go to submitChatHomeCommand (no floating text panel).
    assert.doesNotMatch(appSource, /onAskSecretary=/);
    assert.doesNotMatch(appSource, /openAndAsk/);
    assert.doesNotMatch(appSource, /onOpenText/);
    assert.doesNotMatch(ceoVoiceAgentSource, /openPanel/);
    assert.doesNotMatch(ceoVoiceAgentSource, /openAndAsk/);
    assert.doesNotMatch(ceoVoiceAgentSource, /voice\.panel\.textPlaceholder/);
    assert.doesNotMatch(ceoVoiceAgentSource, /voice\.panel\.backToVoice/);
    assert.doesNotMatch(ceoVoiceAgentSource, /voice\.panel\.send/);
    assert.doesNotMatch(ceoVoiceAgentSource, /data-testid="ceo-voice-panel"/);
    assert.doesNotMatch(ceoVoiceAgentSource, /<textarea/);
    assert.match(appSource, /deskTalkToAegis/);
    assert.match(appSource, /ceoVoiceAgentRef/);
    assert.match(appSource, /toggleVoice/);
    assert.match(appSource, /chatSheetOpen/);
    assert.doesNotMatch(appSource, /aegis-chat-sheet-open/);
    assert.match(appSource, /presentationMode=\{view === 'home' \? 'full' : 'sheet'\}/);
    assert.match(appSource, /vnext-chat-panel-host/);
    assert.match(appSource, /Promise<SecretaryAskResult>/);
    assert.match(appSource, /kind: 'replied'/);
    assert.match(appSource, /buildPanelConversationContext/);
    assert.match(appSource, /mapViewToClientContext/);
    assert.match(appSource, /mintVNextTurnStreamToken/);
    assert.match(appSource, /need_approval/);
    assert.doesNotMatch(
      appSource,
      /async function secretaryAsk[\s\S]{0,400}navigate\('home'/,
    );
    assert.match(ceoVoiceAgentSource, /useImperativeHandle/);
    assert.match(ceoVoiceAgentSource, /type: 'approval'/);
    assert.doesNotMatch(appSource, /Voice is not enabled in the WebApp yet/);
    assert.doesNotMatch(workSource, /vnext-mail-html-frame/);
    assert.doesNotMatch(workSource, /vnext-mail-html-skeleton/);
    assert.doesNotMatch(workSource, /sandbox=/);
    assert.doesNotMatch(workSource, /MailBodyContent/);
    assert.doesNotMatch(workSource, /showMailBodyFirst/);
    assert.match(workSource, /function SourceMetadataContent/);
    assert.doesNotMatch(stylesSource, /\.vnext-mail-metadata/);
    assert.doesNotMatch(stylesSource, /\.vnext-detail-fields\.is-secondary/);
    assert.doesNotMatch(shellSource, /dangerouslySetInnerHTML/);
    assert.match(stylesSource, /vnext-working-pulse/);
    assert.doesNotMatch(appSource, /window\.location\.assign/);
    assert.doesNotMatch(itemActionsSource, /kind: 'navigate'/);
    assert.match(itemActionsSource, /isApprovalItem\(item\)/);
    assert.match(itemActionsSource, /resource_type: 'approval_review'/);
    assert.match(itemActionsSource, /fetchConsoleMemoryItem/);
    assert.match(itemActionsSource, /item\.kind === 'memory_review'/);
    assert.match(detailSource, /resource_type === 'approval_review'/);
    assert.match(detailSource, /resource_type === 'memory_item'/);
    assert.match(detailSource, /function buildMemoryDetail/);
    assert.match(detailSource, /function buildApprovalDetail/);
    assert.match(itemActionsSource, /resource_type: 'owner_work_item'/);
    assert.match(detailSource, /resource_type === 'owner_work_item'/);
    assert.match(detailSource, /function buildOwnerWorkItemDetail/);
    assert.doesNotMatch(itemActionsSource, /This item does not have an inline detail view in Aegis yet/);
    assert.doesNotMatch(itemActionsSource, /No source detail is available for this item yet/);
  });

  it('keeps non-chat vNext failures on the shared owner-facing error path', () => {
    // Error copy is Channel B (spec §2/§6): each call threads the active locale
    // so the message follows the display_language setting. The fallback is now a
    // catalog-backed localized string (t('error.fallback.*', errorLocale)) rather
    // than an English literal, so the zh error templates interpolate a Chinese
    // fallback instead of mixing English into Chinese output.
    assert.match(appSource, /displayError\(loadError, t\('error\.fallback\.couldNotLoadAegis', errorLocale\), errorLocale\)/);
    assert.match(appSource, /displayError\(detailError, t\('error\.fallback\.couldNotOpenDetail', errorLocale\), errorLocale\)/);
    assert.match(appSource, /displayError\(actionError, t\('error\.fallback\.actionFailed', errorLocale\), errorLocale\)/);
    assert.match(appSource, /displayError\(pauseError, t\('error\.fallback\.couldNotPauseChecks', errorLocale\), errorLocale\)/);
    assert.match(appSource, /displayError\(resumeError, t\('error\.fallback\.couldNotResumeChecks', errorLocale\), errorLocale\)/);
    assert.match(appSource, /displayError\(snapshotError, t\('error\.fallback\.couldNotSaveSnapshot', errorLocale\), errorLocale\)/);
    // en parity: the catalog English values are byte-identical to the former literals.
    assert.match(catalogSource, /'error\.fallback\.couldNotLoadAegis':[\s\S]*?en: 'Could not load Aegis'/);
    assert.match(catalogSource, /'error\.fallback\.couldNotSaveSnapshot':[\s\S]*?en: 'Could not save support snapshot'/);
    assert.doesNotMatch(appSource, /displayError\([A-Za-z]+Error\)\)/);
  });

  it('keeps assistant replies compact and avoids duplicated no-op receipt text', () => {
    assert.match(chatHomeSource, /<AssistantResponseView result=\{message\.result\} ownerQuestion=\{precedingOwnerQuestion\(displayMessages, messageIndex\)\} \/>/);
    assert.doesNotMatch(chatHomeSource, /message\.result\.receipt_expectation/);
    assert.match(assistantResponseSource, /function shouldCollapseSection\(section: AssistantResponseSection\)/);
    assert.match(assistantResponseSource, /section\.tone === 'evidence' \|\| section\.id === 'receipt'/);
    assert.match(assistantResponseSource, /<details[\s\S]*className=\{classNames\('vnext-answer-card', 'vnext-disclosure', 'is-collapsible'/);
    assert.match(assistantResponseSource, /<summary className="vnext-disclosure-summary">[\s\S]*<span>\{sectionTitleLabel\(section\.title, locale\)\}<\/span>[\s\S]*<\/summary>/);
    assert.match(stylesSource, /\.vnext-answer-card\.is-collapsible/);
    assert.match(assistantResponseSource, /function AssistantSectionContent/);
    assert.match(assistantResponseSource, /function longAnswerPreview/);
    assert.match(assistantResponseSource, /<details className="vnext-answer-more vnext-disclosure">/);
    assert.match(stylesSource, /\.vnext-answer-more/);
    assert.match(stylesSource, /\.vnext-answer-card summary::-webkit-details-marker/);
    assert.match(stylesSource, /\.vnext-answer-card\[open\] summary::after/);
    assert.match(stylesSource, /\.vnext-chat-answer-card > \.vnext-answer-stack > \.vnext-answer-card\s*\{[\s\S]*background: transparent;/);
    assert.match(stylesSource, /\.vnext-chat-answer-card > \.vnext-answer-stack > \.vnext-answer-card \+ \.vnext-answer-card/);
    assert.match(workSource, /ownerTimeLabel\(item\.due_at(, userTimezone(, locale)?)?\)/);
    assert.match(workSource, /ownerTimeLabel\(entry\.created_at(, userTimezone(, locale)?)?\)/);
  });

  it('settings writes never gate on a CSRF token (Telegram sessions have none)', () => {
    // 2026-09-11: five persist* functions returned early on !session.csrf_token,
    // so language/provider/model/fallback/paid-API changes from the Telegram
    // WebApp were silently dropped. The server skips CSRF for Telegram principals.
    assert.doesNotMatch(appSource, /if \(!session\?\.csrf_token\) return;/);
    assert.match(appSource, /async function persistDisplayLanguage\(value: Locale\) \{\s*\/\/[^\n]*\n(?:\s*\/\/[^\n]*\n)*\s*if \(!session\?\.authenticated\) return;/);
  });

  it('keeps Work detail compact by default and folds raw trust details', () => {
    assert.match(catalogSource, /'work\.queue\.oneDecision':.*en: '1 decision needs you'/);
    assert.match(catalogSource, /en: '\{count\} decisions need you'/);
    assert.match(catalogSource, /en: '\{count\} nearby \{itemWord\}'/);
    assert.match(workSource, /function itemShouldAppearInNearbyWork\(item: OwnerWorkItem\)/);
    assert.match(workSource, /quietNearbyCount/);
    assert.match(stylesSource, /\.vnext-quiet-nearby-note/);
    assert.match(stylesSource, /\.vnext-quiet-nearby-list/);
    assert.match(workSource, /ownerVisibleSignalCue/);
    assert.match(catalogSource, /en: 'Open this panel to switch without leaving the item\.'/);
    assert.doesNotMatch(workSource, /vnext-summary-affordance/);
    // 2026-09-16 console pass: one plain CSS chevron (no circled "›" puck),
    // always on the RIGHT of the summary row.
    assert.match(stylesSource, /\.vnext-disclosure > \.vnext-disclosure-summary::after\s*\{[^}]*content: "";[^}]*order: 99;[^}]*border-right: 1\.5px solid currentColor;/s);
    assert.match(workSource, /firstItem && !selected/);
    assert.match(workSource, /<span>\{t\('work\.startWith', locale\)\}<\/span>/);
    assert.match(catalogSource, /'work\.startWith':.*en: 'Start with'/);
    assert.doesNotMatch(workSource, /Current focus/);
    assert.match(workSource, /const activeAttentionLanes = attentionLanes\.filter\(\(lane\) => lane\.items\.length > 0\)/);
    assert.match(workSource, /const activeQuietLanes = quietLanes\.filter\(\(lane\) => lane\.items\.length > 0\)/);
    assert.match(workSource, /const selectedQuietLane = selected/);
    assert.match(workSource, /const switcherLanes = selected/);
    assert.match(workSource, /\?\s*\[\.\.\.activeAttentionLanes,[\s\S]*selectedQuietLane \? \[selectedQuietLane\] : \[\][\s\S]*\]/);
    assert.match(workSource, /:\s*\[\.\.\.activeAttentionLanes,\s*\.\.\.activeQuietLanes\]/);
    assert.match(workSource, /\{switcherLanes\.map\(\(lane\) => \(/);
    assert.match(workSource, /selected && 'is-secondary'/);
    assert.match(workSource, /<details className=\{classNames\('vnext-work-brief', 'vnext-disclosure', selected && 'is-secondary'\)\} aria-label=\{t\('work\.aria\.queueBrief', locale\)\} open=\{!selected\}>/);
    assert.match(workSource, /<summary className="vnext-disclosure-summary">/);
    assert.match(workSource, /workQueueKicker\(selected, nearbyItems\.length, quietNearbyCount, locale\)/);
    assert.match(workSource, /function workQueueKicker/);
    assert.match(workSource, /vnext-disclosure-copy/);
    assert.match(workSource, /\{hasItems && !workScanActive && visibleLanes\.map\(\(lane\) => \{/);
    assert.match(workSource, /\{hasItems && selected && \(/);
    assert.match(workSource, /const \[workScanQuery, setWorkScanQuery\] = useState\(''\)/);
    assert.match(workSource, /function WorkScanControls/);
    assert.match(workSource, /function WorkScanResults/);
    assert.match(workSource, /filterWorkForScan\(workScanPool/);
    assert.match(stylesSource, /\.vnext-work-scan\s*\{/);
    assert.match(stylesSource, /\.vnext-work-scan-results\s*\{/);
    assert.match(workSource, /className="vnext-work-mobile-inline-lanes"/);
    assert.match(workSource, /function renderMobileLaneItem\(item: OwnerWorkItem\)/);
    assert.match(workSource, /renderItemDetail\('vnext-work-inline-selected'\)/);
    assert.match(workSource, /renderItemDetail\('vnext-detail-desktop'\)/);
    assert.match(stylesSource, /\.vnext-detail-desktop\s*\{[\s\S]*display: none;/);
    assert.match(stylesSource, /\.vnext-work-mobile-inline-lanes\s*\{[\s\S]*display: grid;/);
    assert.match(workSource, /allNearbyCandidates = selected[\s\S]*switcherLanes\.flatMap/);
    assert.match(workSource, /actionableNearbyItems = allNearbyCandidates\.filter\(itemShouldAppearInNearbyWork\)/);
    assert.match(workSource, /nearbyItems = actionableNearbyItems\.slice\(0, 8\)/);
    assert.match(workSource, /className="vnext-work-switcher-list"/);
    const renderNearbyBody = workSource.match(/function renderNearbyItem\(item: OwnerWorkItem\)[\s\S]*?\n  function focusLane/)?.[0] ?? '';
    assert.ok(renderNearbyBody, 'renderNearbyItem should stay inspectable');
    assert.match(workSource, /function toggleNearbyDetail\(id: string\)/);
    assert.match(workSource, /function renderNearbyItem\(item: OwnerWorkItem\)/);
    assert.match(renderNearbyBody, /onClick=\{\(event\) => \{\s*tappedNearbyTopRef\.current = event\.currentTarget\.getBoundingClientRect\(\)\.top;\s*toggleNearbyDetail\(item\.id\);\s*\}\}/);
    assert.match(workSource, /<NearbyInlineDetail/);
    assert.match(workSource, /function NearbyInlineDetail/);
    assert.match(workSource, /onAction\(item, action\)/);
    assert.match(workSource, /assistantComposerItemId === item\.id/);
    assert.match(workSource, /assistantResultForItem\(item\)/);
    assert.match(workSource, /onAskText=\{\(text\) => submitAssistant\(item, text\)\}/);
    assert.match(catalogSource, /en: 'Uses this expanded item, without changing your place\.'/);
    assert.match(workSource, /followUpQuickControls/);
    assert.match(workSource, /function FollowUpPromptStrip/);
    assert.match(workSource, /aria-label=\{t\('work\.aria\.followUpChoices', locale\)\}/);
    assert.match(catalogSource, /'work\.followUp\.addUpdate':.*en: 'Add update'/);
    // Cleanup is a receipt-backed close action (double-tap), not an assistant chat loop.
    assert.match(workSource, /onCloseAction=\{\(action\) => onAction\(item, action\)\}/);
    assert.match(workSource, /onPrompt=\{\(prompt\) => onOpenAssistant\(prompt\)\}/);
    assert.match(workSource, /shouldExecuteFollowUpCleanup/);
    assert.doesNotMatch(workSource, /Still waiting/);
    assert.match(stylesSource, /\.vnext-followup-strip\s*\{/);
    assert.doesNotMatch(workSource, /lastAssistantResultRef\.current = null;/);
    assert.match(workSource, /function itemIsQuietTrackingOnly\(item: OwnerWorkItem\)/);
    assert.match(catalogSource, /en: 'No action is needed now\. Aegis is only watching this item/);
    // 2026-09-14: owner-facing copy says 件事 / item, never the internal "loop".
    assert.doesNotMatch(catalogSource, /个\{loopWord\}/);
    assert.doesNotMatch(catalogSource, /watching this loop/);
    // Owner rule 2026-09-16: a tapped row expands DOWNWARD with its top edge
    // fixed — WorkView never calls scrollIntoView; the only scroll is the
    // corrective keepRowTopVisible (row hidden under the sticky header).
    assert.doesNotMatch(workSource, /scrollIntoView\(/);
    assert.match(workSource, /export function keepRowTopInPlace\(host: HTMLElement, tappedTop: number \| null\)/);
    assert.match(workSource, /keepRowTopInPlace\(host, tappedTop\)/);
    assert.match(workSource, /tappedRowTopRef\.current = event\.currentTarget\.getBoundingClientRect\(\)\.top;/);
    assert.match(workSource, /const desired = Math\.max\(tappedTop \?\? rowTop, minTop\);/);
    const chromeCssScroller = readFileSync(new URL('../src/vnext/design/surfaces/chrome.css', import.meta.url), 'utf8');
    assert.match(chromeCssScroller, /\.vnext-shell\s*\{[^}]*overflow-anchor:\s*none/s);
    assert.doesNotMatch(workSource, /Open full detail/);
    assert.doesNotMatch(renderNearbyBody, /onClick=\{\(\) => selectItem\(item\.id\)\}/);
    assert.match(stylesSource, /\.vnext-work-brief > summary/);
    assert.match(stylesSource, /\.vnext-disclosure > \.vnext-disclosure-summary/);
    assert.match(stylesSource, /\.vnext-disclosure\[open\] > \.vnext-disclosure-summary::after/);
    assert.match(stylesSource, /\.vnext-work-switcher-list/);
    assert.match(stylesSource, /\.vnext-nearby-inline-detail/);
    assert.doesNotMatch(stylesSource, /\.vnext-nearby-preview/);
    assert.match(stylesSource, /\.vnext-work-brief\.is-secondary:not\(\[open\]\)/);
    assert.match(stylesSource, /vnext-work-brief\.is-secondary/);
    assert.match(stylesSource, /vnext-work-brief\.is-secondary p:not\(\.vnext-kicker\)/);
    assert.match(stylesSource, /vnext-work-brief\.is-secondary \.vnext-work-lane-strip button/);
    assert.doesNotMatch(workSource, /Decision selected/);
    assert.doesNotMatch(workSource, /Tracking selected/);
    assert.doesNotMatch(workSource, /Secretary desk/);
    assert.doesNotMatch(workSource, /Work left/);
    assert.doesNotMatch(workSource, /decision\$\{decisionLane\.items\.length === 1 \? '' : 's'\} need your call/);
    assert.doesNotMatch(workSource, /still on your desk/);
    assert.doesNotMatch(workSource, /Switch piles below/);
    assert.match(catalogSource, /'work\.authority\.access':.*en: 'Permissions and basis'/);
    assert.match(workSource, /detailOpen=\{Boolean\(sourceDetail\)\}/);
    assert.match(catalogSource, /en: 'Selected detail is open in this signed-in session\.'/);
    assert.match(workSource, /vnext-authority-panel-collapsed/);
    assert.doesNotMatch(workSource, /Show access details/);
    assert.match(workSource, /detailAccessStillClosed = !sourceDetail && \(Boolean\(authority\?\.requires_mfa\) \|\| lockedData\.length > 0\)/);
    assert.match(workSource, /!\s*blockedByGate && !privateDetailStillClosed && !detailAccessStillClosed && \(\s*<AuthorityPanel/);
    assert.match(catalogSource, /en: 'Aegis can use the visible context here\. If it changes anything, it will keep a receipt\.'/);
    assert.match(workSource, /function ownerFacingReceiptText/);
    assert.doesNotMatch(catalogSource, /Mailbox changes keep a Gmail receipt/);
    assert.doesNotMatch(workSource, /Mailbox changes must keep Gmail proof/);
    assert.doesNotMatch(workSource, /without proof/);
    assert.match(workSource, /function itemActionGroupLabel/);
    assert.match(workSource, /function itemHasApprovalActions/);
    assert.doesNotMatch(catalogSource, /'work\.group\.mailbox'/);
    assert.match(catalogSource, /'work\.group\.approveReject':.*en: 'Approve or reject'/);
    assert.match(catalogSource, /'work\.group\.decision':.*en: 'Decision actions'/);
    assert.match(catalogSource, /'work\.group\.followUp':.*en: 'Follow-up actions'/);
    assert.doesNotMatch(workSource, />Safe actions</);
    assert.match(workSource, /aria-label=\{t\('work\.aria\.workWithAegis', locale\)\}/);
    assert.match(workSource, /className="vnext-item-receipt-actions"/);
    assert.match(workSource, /actionsRetired = false/);
    assert.match(workSource, /delayReceiptActionsUntilDetail \|\| actionsRetired \? \[\] : itemActions/);
    assert.match(workSource, /function WorkActionMessage/);
    assert.match(workSource, /className="vnext-work-action-receipt"/);
    assert.match(workSource, /aria-label=\{t\('work\.aria\.actionReceipt', locale\)\}/);
    assert.match(workSource, /<em>\{t\('work\.receipt\.label', locale\)\}<\/em>/);
    assert.match(appSource, /const \[workNotice, setWorkNotice\] = useState\(''\)/);
    assert.match(appSource, /setWorkNotice\(successMessage\)/);
    assert.match(workSource, /workNotice: string/);
    assert.match(workSource, /<WorkActionMessage message=\{workNotice\} \/>/);
    assert.match(workSource, /className="vnext-host-step-up"/);
    assert.match(workSource, /t\('work\.approval\.hostConfirm', locale\)/);
    assert.match(catalogSource, /'work\.approval\.hostConfirm': \{ zh: '再确认一次', en: 'Confirm again' \}/);
    assert.match(appSource, /offerLocalHostStepUp\(session\?\.email/);
    assert.match(appSource, /confirmLocalHostStepUp\(session\.csrf_token\)/);
    assert.match(stylesSource, /\.vnext-chat-work-receipt,\s*\.vnext-work-action-receipt/);
    assert.doesNotMatch(workSource, /vnext-receipt-action-panel/);
    assert.doesNotMatch(inlineAssistantSource, /'vnext-mini-panel', 'vnext-inline-assistant'/);
    assert.match(workSource, /fallbackBlockedDecisionRequest/);
    assert.match(catalogSource, /'work\.fallback\.title':.*en: 'Automation needs your choice'/);
    assert.match(workSource, /blockedAutomationWhatHappened/);
    assert.doesNotMatch(workSource, /blockedAutomationDecisionText/);
    assert.match(workSource, /t\('work\.blocked\.suggest\.whyPaused\.prompt', locale\)/);
    assert.match(catalogSource, /Explain why this paused and only refer to the choices shown on this card/);
    assert.doesNotMatch(workSource, /Aegis paused this before using a protected action/);
    assert.doesNotMatch(workSource, /A protected action needs your review/);
    assert.match(workSource, /item\.decision_request \?\? fallbackBlockedDecisionRequest\(item, locale\)/);
    assert.match(catalogSource, /'work\.blocked\.noActionRan':.*en: 'Nothing ran outward'/);
    assert.match(workSource, /aria-label=\{t\('work\.aria\.pausedDecision', locale\)\}/);
    assert.match(catalogSource, /'work\.blocked\.whatAegisDoing':.*en: 'What Aegis was doing'/);
    assert.match(catalogSource, /'work\.blocked\.ceoTitle'/);
    assert.match(catalogSource, /'work\.blocked\.whatLabel'/);
    assert.match(appSource, /id: 'logout'/);
    assert.match(workSource, /label: t\('work\.blocked\.suggest\.whyPaused\.label', locale\)/);
    assert.match(catalogSource, /'work\.blocked\.requestedCapability':.*en: 'Requested capability'/);
    assert.match(workSource, /aria-label=\{t\('work\.aria\.decisionChoices', locale\)\}/);
    assert.match(workSource, /decision\.choices\.map/);
    assert.match(workSource, /choice\.action_kind/);
    assert.match(workSource, /actionUnavailable/);
    assert.match(catalogSource, /en: 'This choice is not available in the current item state\.'/);
    assert.doesNotMatch(workSource, /Review approval/);
    assert.match(workSource, /label: t\('work\.blocked\.suggest\.revise\.label', locale\)/);
    assert.match(workSource, /t\('work\.blocked\.suggest\.revise\.prompt', locale\)/);
    assert.match(catalogSource, /'work\.blocked\.safeDetails':.*en: 'Safe details'/);
    assert.match(catalogSource, /'work\.fallback\.safeDetail':.*en: 'No external action has run\.'/);
    assert.doesNotMatch(workSource, /vnext-paused-decision-actions/);
    assert.match(workSource, /aria-label=\{t\('work\.aria\.pausedTrust', locale\)\}/);
    assert.match(catalogSource, /'work\.trust\.visibleMetadata':.*en: 'Visible metadata'/);
    assert.match(catalogSource, /'work\.trust\.privateLocked':.*en: 'Private detail locked'/);
    assert.match(catalogSource, /'work\.trust\.receiptsRequired':.*en: 'Receipts required'/);
    assert.doesNotMatch(workSource, /!\s*assistantOpen && itemActions\.length > 0/);
    assert.doesNotMatch(workSource, /label=\{closeExecuting \? 'Working\.\.\.' : closeArmed \? 'Tap again' : 'Close item'\}/);
    assert.doesNotMatch(workSource, /detail="Stop tracking this pause"/);
    assert.doesNotMatch(workSource, />Find decision</);
    assert.match(workSource, /assistantOpen=\{assistantOpen\}/);
    assert.match(catalogSource, /en: 'Ask about this paused item, or tell Aegis how to revise the rule\.'/);
    assert.doesNotMatch(workSource, /label="Needed choice"/);
    assert.doesNotMatch(workSource, /detail="Why it stopped"/);
    assert.match(workSource, /onChange=\{onAskTextChange\}/);
    assert.match(workSource, /name=\{control\.id === 'draft' \? 'instructions' : 'spark'\}/);
    assert.match(workSource, /'instructions'/);
    assert.match(workSource, /itemActionIconName\(action\.id, action\.tone\)/);
    assert.doesNotMatch(workSource, /<VNextIcon name=\{action\.tone === 'destructive' \? 'close' : 'receipt'\} \/>/);
    assert.doesNotMatch(iconsSource, /actionId === 'mail\.mark_read'/);
    assert.doesNotMatch(iconsSource, /actionId === 'mail\.archive'/);
    assert.match(iconsSource, /actionId === 'task\.snooze_tomorrow'[\s\S]*return 'clock'/);
    assert.match(workSource, /function inlineAssistantConversationContext\(turns: InlineAssistantTurn\[\]\): VNextConversationContextTurn\[\]/);
    assert.match(workSource, /onAskText\(item, prompt, conversationContext\)/);
    assert.match(catalogSource, /en: 'Private content stays closed until you open the source detail\.'/);
    assert.doesNotMatch(workSource, /Protected detail/);
    assert.doesNotMatch(workSource, /Private details stay protected until opened from your signed-in owner session/);
    assert.match(workSource, /historyKindLabel\(entry\.kind, locale\)/);
    assert.doesNotMatch(workSource, /<strong>\{entry\.kind\}<\/strong>/);
    assert.doesNotMatch(workSource, /Sensitive access requires a fresh owner session/);
    assert.match(workSource, /t\('work\.fallback\.choice\.assistantPrompt', locale\)/);
    assert.match(catalogSource, /Do not execute any external action/);
    assert.match(workSource, /t\('work\.blocked\.suggest\.revise\.prompt', locale\)/);
    assert.doesNotMatch(appSource, /setCommand\('Revise this paused automation before I approve/);
    assert.doesNotMatch(modelSource, /status === 'blocked_by_gate'\) return \[\];/);
    assert.match(modelSource, /const serverActionIds = Array\.isArray\(item\.secondary_actions\)/);
    assert.match(stylesSource, /vnext-trust-summary/);
    assert.match(stylesSource, /vnext-authority-panel-collapsed/);
    assert.match(stylesSource, /vnext-blocked-panel/);
    assert.match(stylesSource, /vnext-blocked-summary/);
    assert.match(stylesSource, /vnext-decision-choice-list/);
    assert.match(stylesSource, /\.vnext-decision-choice-main strong\s*\{/);
    assert.match(stylesSource, /\.vnext-decision-choice-main em\s*\{/);
    assert.doesNotMatch(stylesSource, /vnext-blocked-choice-grid/);
    assert.match(stylesSource, /\.vnext-blocked-summary span\s*\{[^}]*display: -webkit-box;[^}]*-webkit-line-clamp: 2;/s);
    assert.doesNotMatch(stylesSource, /vnext-blocked-state-rail/);
    assert.doesNotMatch(stylesSource, /vnext-blocked-more/);
    assert.match(workSource, /<SourceDetailPanel sourceDetail=\{sourceDetail\} collapsed=\{sourceDetail\.kind === 'task'\} \/>/);
    assert.match(workSource, /summaryKicker = sourceDetail\.kind === 'task' \? t\('work\.source\.taskDetail', locale\)/);
    assert.match(workSource, /taskSummarySubtitle\(sourceDetail, locale\)/);
    assert.match(workSource, /function TaskDetailContent/);
    assert.match(workSource, /className="vnext-task-summary-card"/);
    assert.match(workSource, /className="vnext-inline-disclosure vnext-task-history vnext-disclosure"/);
    assert.match(stylesSource, /\.vnext-task-summary-card\s*\{/);
    assert.doesNotMatch(workSource, /State, priority, due date, and history/);
    assert.match(workSource, /vnext-source-panel-collapsed/);
    assert.match(workSource, /function ExternalSourceLink/);
    assert.match(workSource, /<ExternalSourceLink sourceDetail=\{sourceDetail\} \/>/);
    assert.match(stylesSource, /\.vnext-source-actions\s*\{/);
    assert.match(stylesSource, /\.vnext-source-actions a\s*\{/);
    assert.match(workSource, /function SourceDetailContent/);
    assert.match(workSource, /import \{ RichText \} from '\.\/RichText';/);
    assert.doesNotMatch(workSource, /<h3>Message<\/h3>/);
    assert.doesNotMatch(workSource, /Email layout/);
    assert.match(workSource, /sourceDetail\.kind === 'action_draft'/);
    assert.match(workSource, /<RichText content=\{sourceDetail\.body\} className="vnext-rich-text-draft" \/>/);
    assert.match(workSource, /<RichText content=\{sourceDetail\.body\} className="vnext-rich-text-detail" \/>/);
    assert.match(stylesSource, /\.vnext-source-panel-collapsed > summary/);
    assert.match(stylesSource, /\.vnext-source-panel-collapsed > summary::after\s*\{[\s\S]*content: "";[\s\S]*border-right: 2px solid currentColor;[\s\S]*transform: rotate\(-45deg\);/);
    assert.match(stylesSource, /\.vnext-authority-panel-collapsed > summary/);
    assert.match(stylesSource, /\.vnext-authority-panel-collapsed > summary::after/);
    assert.match(stylesSource, /\.vnext-supporting-panel > summary::after\s*\{[\s\S]*content: "";[\s\S]*border-right: 2px solid currentColor;[\s\S]*transform: rotate\(-45deg\);/);
    assert.doesNotMatch(stylesSource, /vnext-blocked-steps/);
    assert.match(stylesSource, /vnext-detail-fields span/);
    assert.match(stylesSource, /\.vnext-action-row button\.vnext-secondary/);
    assert.match(stylesSource, /\.vnext-action-row button\.vnext-danger/);
    assert.doesNotMatch(workSource, /Permission and proof/);
    assert.doesNotMatch(workSource, /Review visible context and audit trail/);
    assert.doesNotMatch(workSource, /Do this now/);
  });

  it('keeps receipt actions and the item assistant together inside the selected item', () => {
    const actionPanelStart = workSource.indexOf("aria-label={t('work.aria.workWithAegis', locale)}");
    const surfaceStart = workSource.indexOf('presentOwnerWorkSurfaceActions', actionPanelStart);
    const receiptActionsStart = workSource.indexOf("aria-label={t('work.aria.receiptActions', locale)}", actionPanelStart);
    const assistantStart = workSource.indexOf('InlineAssistantPanel', actionPanelStart);
    assert.ok(actionPanelStart > 0, 'missing item action panel');
    assert.ok(surfaceStart > actionPanelStart, 'missing owner work surface presentation inside item action panel');
    assert.ok(receiptActionsStart > surfaceStart, 'missing receipt actions label inside progressive disclosure');
    assert.ok(assistantStart > actionPanelStart, 'item assistant should remain inside the action panel');
    // Window covers surface actions + receipt row + inline assistant (grew with X open/draft controls).
    const actionPanelSource = workSource.slice(actionPanelStart, actionPanelStart + 6000);
    assert.match(actionPanelSource, /InlineAssistantPanel/);
    assert.match(actionPanelSource, /itemActions: dockItemActions/);
    assert.match(actionPanelSource, /vnext-work-more-actions/);
    assert.match(workSource, /BlockedByGatePanel[\s\S]*assistantOpen=\{assistantOpen\}/);
    assert.doesNotMatch(workSource, /!\s*assistantOpen && \(\s*<div className="vnext-blocked-choice-grid"/);
    assert.match(workSource, /<div className="vnext-decision-choice-list" aria-label=\{t\('work\.aria\.decisionChoices', locale\)\}>/);
  });

  it('makes work cards scannable with status chips before secondary text', () => {
    assert.match(appSource + workSource, /<WorkItemRow/);
    assert.match(modelSource, /function ownerVisibleWorkCue/);
    assert.match(modelSource, /function ownerVisibleWorkTitle/);
    assert.match(modelSource, /function isGenericWorkSummary/);
    assert.match(workSource, /closedItem[\s\S]*ownerVisibleWorkCue\(item, 220, userTimezone, locale\)[\s\S]*openedTaskDetail\?\.subtitle \|\| ownerVisibleWorkCue\(item, 220, userTimezone, locale\)/);
    assert.match(workSource, /openedTaskDetail\?\.title \|\| ownerVisibleWorkTitle\(item, t\('work\.title\.selectedItem', locale\), 220, locale\)/);
    assert.match(workSource, /function safeNextMoveLabel/);
    assert.match(modelSource, /export function itemNeedsPrivateTaskAccess/);
    assert.match(workSource, /itemNeedsPrivateTaskAccess\(item\)/);
    assert.match(workItemRowSource, /itemNeedsPrivateTaskAccess\(item\)/);
    assert.match(workSource, /privateDetailStillClosed\s*\?\s*t\('work\.focus\.needsDetail', locale\)/);
    assert.match(catalogSource, /'work\.askAegis\.usingItemContext':.*en: 'Uses this item context\.'/);
    assert.match(workSource, /className="vnext-item-primary-step"/);
    assert.match(workSource, /aria-label=\{t\('work\.aria\.openDetails', locale\)\}/);
    assert.doesNotMatch(workSource, /Open the private source first so Aegis can work with the real item\./);
    assert.match(stylesSource, /\.vnext-item-primary-step/);
    assert.match(stylesSource, /\.vnext-work-more-actions/);
    assert.match(workSource, /presentOwnerWorkSurfaceActions\(/);
    assert.match(workSource, /className="vnext-work-more-actions vnext-disclosure"/);
    assert.match(workSource, /privateDetailStillClosed = needsPrivateDetail && !sourceDetail/);
    assert.match(workSource, /delayReceiptActionsUntilDetail = privateDetailStillClosed/);
    assert.match(workSource, /visibleItemActions = delayReceiptActionsUntilDetail \|\| actionsRetired \? \[\] : itemActions/);
    assert.match(workSource, /itemActions: dockItemActions/);
    assert.match(workSource, /includeOpenDetail: offerOpenDetail/);
    assert.match(workSource, /offerOpenDetail = !sourceDetail && \(!followUpItem \|\| privateDetailStillClosed\)/);
    assert.match(modelSource, /private task title\\\/body/);
    assert.match(workSource, /return t\('work\.nextMove\.openDetails', locale\);/);
    assert.doesNotMatch(workSource, /isMailLikeItem/);
    assert.doesNotMatch(catalogSource, /'work\.nextMove\.openMessageDecide'/);
    assert.match(workSource, /includeAssistantChips: !privateDetailStillClosed && !closedItem && !followUpItem/);
    assert.match(workItemRowSource, /ownerVisibleWorkCue\(item, undefined, userTimezone, locale\)/);
    assert.match(workItemRowSource, /ownerVisibleWorkTitle\(item, undefined, undefined, locale\)/);
    assert.match(stylesSource, /vnext-item-top/);
    assert.match(stylesSource, /vnext-item-kind/);
    assert.match(stylesSource, /vnext-item-timing/);
    assert.match(stylesSource, /\.vnext-item-row\.compact \.vnext-item-summary\s*\{[^}]*-webkit-line-clamp: 1;/s);
    assert.doesNotMatch(workItemRowSource, /item\.summary \|\| actionLabel\(item\), actionLabel\(item\), 180/);
  });

  it('uses the owner-facing work priority model for default selection', () => {
    assert.match(appSource, /function firstPriorityItemId/);
    assert.match(appSource, /const activeItems = items\.filter\(\(item\) => !isClosedItem\(item\)\);/);
    assert.match(appSource, /buildVNextLanes<OwnerWorkItem>\(activeItems, errorLocale\)/);
    assert.doesNotMatch(appSource, /nextQueue\.items\[0\]\?\.id/);
  });

  it('removes closed optimistic work instead of rendering a handled item in the active queue', () => {
    assert.match(appSource, /items: nextQueue\.items\.map\(\(item\) => \{/);
    assert.match(appSource, /\}\)\.filter\(\(item\) => !isClosedItem\(item\)\),/);
  });

  it('clears hidden item focus when leaving Work', () => {
    // 2026-07-10: requestedSelectedId now derives from carriedSelectedId (the
    // mail→Work cross-namespace guard) instead of nextSelectedId directly —
    // same "only Work keeps an item focus" guarantee, plus mail keys can
    // never ride the carry default into Work.
    assert.match(appSource, /const requestedSelectedId = nextView === 'work' \? nextSelectedId : null;/);
    assert.match(appSource, /const resolvedSelectedId = nextView === 'work' && !requestedSelectedId && queue/);
    assert.match(appSource, /writeVNextLocationState\(window, \{ view: nextView, itemId: resolvedSelectedId \}\);/);
  });

  it('builds the owner queue model once and reuses it across Aegis and Work', () => {
    assert.match(appSource, /const nextHome = buildVNextHome<OwnerWorkItem>\(queue, errorLocale\);/);
    assert.doesNotMatch(appSource, /fetchVNextHome/);
    assert.doesNotMatch(appSource, /homeProjection/);
    assert.doesNotMatch(appSource, /top_now\.map/);
    assert.doesNotMatch(appSource, /decision_previews\.map/);
  });

  it('keeps mobile Work detail actions visible before diagnostics', () => {
    assert.match(stylesSource, /@media \(max-width: 820px\)/);
    assert.match(stylesSource, /\.vnext-detail\s*\{[^}]*order: -1;[^}]*position: static;[^}]*gap: 10px;[^}]*padding: 12px;/s);
    assert.match(stylesSource, /\.vnext-work-lane-strip\s*\{[^}]*grid-template-columns: repeat\(2, minmax\(0, 1fr\)\);/s);
    assert.match(stylesSource, /\.vnext-header-snapshot\s*\{[^}]*width: 40px;[^}]*border-radius: 999px;/s);
    assert.match(stylesSource, /\.vnext-header-snapshot span\s*\{[^}]*clip: rect\(0 0 0 0\);/s);
    assert.match(stylesSource, /\.vnext-work-brief\s*\{[^}]*grid-template-columns: 1fr;/s);
    assert.match(stylesSource, /-webkit-line-clamp: 2;/);
    assert.match(workSource, /function focusLane\(lane: LaneProjection\)/);
    assert.match(workSource, /onClick=\{\(\) => focusLane\(lane\)\}/);
    assert.match(workSource, /aria-pressed=\{lane\.items\.some\(\(item\) => item\.id === selected\?\.id\)\}/);
    assert.match(stylesSource, /\.vnext-work-lane-strip button\.active/);
  });

  it('keeps mobile selected item actions above diagnostics', () => {
    assert.match(stylesSource, /\.vnext-detail\s*\{[^}]*order: -1;[^}]*position: static;[^}]*gap: 10px;[^}]*padding: 12px;/s);
    assert.match(stylesSource, /\.vnext-detail-hero\s*\{[^}]*gap: 8px;[^}]*padding: 12px;/s);
    assert.match(workSource, /<h2 title=\{title\}>\{title\}<\/h2>/);
    // Selected/detail surfaces keep full title + cue (no clamp / ellipsis).
    assert.match(stylesSource, /\.vnext-detail \.vnext-detail-hero h2[\s\S]*?-webkit-line-clamp:\s*unset/);
    assert.match(stylesSource, /\.vnext-detail \.vnext-detail-hero > p[\s\S]*?-webkit-line-clamp:\s*unset/);
    assert.match(stylesSource, /\.vnext-detail \.vnext-item-meta[\s\S]*?text-overflow:\s*unset/);
    assert.doesNotMatch(stylesSource, /\.vnext-detail-hero h2\s*\{[^}]*-webkit-line-clamp:\s*[1-9]/s);
    assert.doesNotMatch(stylesSource, /\.vnext-detail-hero > p\s*\{[^}]*-webkit-line-clamp:\s*[1-9]/s);
    assert.doesNotMatch(stylesSource, /\.vnext-detail-title-row \.vnext-item-meta\s*\{[^}]*text-overflow:\s*ellipsis;/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-detail-hero h2\s*\{[^}]*font-size: 22px;/s);
    assert.match(workSource, /function stateCueLabel\(item: OwnerWorkItem, needsPrivateDetail: boolean, locale: Locale\)/);
    assert.match(workSource, /className="vnext-work-intent-row"/);
    assert.match(workSource, /aria-label=\{t\('work\.aria\.currentSummary', locale\)\}/);
    assert.match(workSource, /sourceDetail \? t\('work\.intent\.detail', locale\) : stateCueLabel/);
    assert.match(workSource, /function detailOpenNextMoveLabel/);
    assert.doesNotMatch(catalogSource, /'work\.nextMove\.reviewMessage'/);
    assert.match(catalogSource, /'work\.nextMove\.reviewTask':.*en: 'Review the task, then choose\.'/);
    assert.doesNotMatch(workSource, /Waiting for<\/strong>/);
    assert.doesNotMatch(workSource, /Next safe move<\/strong>/);
    assert.match(workSource, /function detailItemMeta\(item: OwnerWorkItem, locale: Locale\)/);
    assert.match(workSource, /<span className=\{classNames\('vnext-item-meta', itemHasPriority\(item\) && 'is-priority'\)\}>\{detailItemMeta\(item, locale\)\}<\/span>/);
    assert.match(workSource, /\(relatedWorkCount > 0 \|\| history\.length > 0\)/);
    assert.match(stylesSource, /\.vnext-work-intent-row\s*\{/);
    assert.match(stylesSource, /\.vnext-work-intent-row strong\s*\{[^}]*-webkit-line-clamp: 2;/s);
    assert.match(stylesSource, /\.vnext-detail \.vnext-work-intent-row strong[\s\S]*?-webkit-line-clamp:\s*unset/);
    assert.match(stylesSource, /\.vnext-next-step\s*\{[^}]*grid-template-columns: auto minmax\(0, 1fr\) auto;/s);
    assert.match(stylesSource, /\.vnext-status-row \.vnext-next-step\s*\{[^}]*display: block;/s);
    assert.doesNotMatch(stylesSource, /\n\.vnext-next-step\s*\{[^}]*display: block;/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-next-step\s*\{[^}]*grid-template-columns: minmax\(0, 1fr\);/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-next-step strong\s*\{[^}]*-webkit-line-clamp: 2;/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-work-intent-row\s*\{[^}]*grid-template-columns: minmax\(0, 1fr\);/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-decision-choice\s*\{[^}]*grid-template-columns: minmax\(0, 1fr\);/s);
    // Ask Aegis is compact secondary (not full-width vnext-item-ai-entry peer wall).
    assert.doesNotMatch(workSource, /className="vnext-item-ai-entry"/);
    assert.match(workSource, /className="vnext-work-ask-compact"/);
    assert.match(workSource, /aria-label=\{t\('work\.aria\.askThisItem', locale\)\}/);
    assert.match(workSource, /workDetailAskPlacement/);
    assert.match(catalogSource, /'work\.askAegis':.*en: 'Ask Aegis'/);
    assert.match(catalogSource, /'work\.askAegis\.usingOpenedDetail':.*en: 'Using opened detail too\.'/);
    assert.match(catalogSource, /'work\.askAegis\.usingItemContext':.*en: 'Uses this item context\.'/);
    assert.match(workSource, /vnext-action-primary-row-stacked/);
    assert.match(workSource, /className="vnext-action-dock-grid" aria-label=\{t\('work\.aria\.itemControls', locale\)\}/);
    assert.match(stylesSource, /\.vnext-action-primary-row\s*\{/);
    assert.match(stylesSource, /\.vnext-work-ask-compact\s*\{/);
    assert.match(stylesSource, /\.vnext-action-dock-grid\s*\{[\s\S]*grid-template-columns: minmax\(180px, 0\.72fr\) minmax\(240px, 1fr\);/);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-action-dock-grid\s*\{[^}]*grid-template-columns: minmax\(0, 1fr\);/s);
    // Mobile: scan above lanes (order -4 vs lanes -3).
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-work-mobile-inline-lanes\s*\{[\s\S]*order: -3;/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-work-scan\s*\{[\s\S]*order: -4;/s);
    // FIND scope+sort progressive disclosure.
    assert.match(workSource, /vnext-work-scan-filters/);
    assert.match(workSource, /workScanFiltersDefaultExpanded/);
    assert.match(catalogSource, /'work\.scan\.filters':/);
    assert.match(stylesSource, /\.vnext-action-primary-row-stacked\s*\{/);
    // Hierarchy flatten: second-level mini-panels inside inline selected detail.
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-work-inline-selected\.vnext-detail > \.vnext-mini-panel[\s\S]*border-color: transparent;/s);
    assert.match(stylesSource, /\.vnext-decision-choice\.is-primary\s*\{/);
    assert.match(stylesSource, /\.vnext-decision-choice\.is-secondary\s*\{/);
    assert.match(stylesSource, /\.vnext-decision-choice\.is-detail\s*\{/);
    assert.doesNotMatch(workSource, /Open details for full context/);
    assert.doesNotMatch(workSource, /Explain, revise, plan, or handle the item in place/);
    assert.match(stylesSource, /\.vnext-work-ask-compact\s*\{/);
    assert.match(stylesSource, /\.vnext-item-receipt-actions\s*\{[\s\S]*grid-template-columns: auto minmax\(0, 1fr\);/);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-item-receipt-actions\s*\{[\s\S]*grid-template-columns: minmax\(0, 1fr\);/);
    assert.match(stylesSource, /\.vnext-item-action-chips/);
  });

  it('shows assistant progress visually without adding noisy sent banners', () => {
    assert.match(chatHomeSource, /aria-busy=\{busy\}/);
    assert.match(chatHomeSource, /streamNearBottomRef = useRef\(true\)/);
    assert.match(chatHomeSource, /if \(!shouldAutoScroll\) return;/);
    assert.match(chatHomeSource, /focus\(\{ preventScroll: true \}\)/);
    assert.match(chatHomeSource, /stream\.scrollTo\(\{ top: stream\.scrollHeight, behavior \}\);/);
    assert.match(chatHomeSource, /bottomRef\.current\?\.scrollIntoView\(\{ block: 'end', behavior \}\)/);
    // The initial working placeholder is now a localized catalog key resolved
    // against the owner's display_language at the render site.
    assert.match(appSource, /CHAT_HOME_WORKING_INITIAL_KEY = 'chatHome\.working\.initial'/);
    assert.match(appSource, /text: t\(CHAT_HOME_WORKING_INITIAL_KEY, errorLocale\)/);
    assert.match(chatHomeSource, /'vnext-chat-stream'/);
    assert.match(chatHomeSource, /'is-working'/);
    assert.match(chatHomeSource, /'is-empty'/);
    // The slow/long working escalations are localized catalog keys.
    assert.match(appSource, /paintWorkingProgress\(t\(CHAT_HOME_WORKING_SLOW_KEY, errorLocale\)\)/);
    assert.match(appSource, /from '\.\/workingProgress'/);
    assert.match(appSource, /keepPageOpenHint/);
    assert.match(stylesSource, /vnext-chat-working-dot/);
    assert.match(stylesSource, /vnext-chat-stream-working/);
    assert.match(stylesSource, /\.vnext-chat-stream\s*\{[^}]*overflow-y: auto;/s);
    assert.doesNotMatch(shellSource, /Sent: /);
  });

  it('keeps Status and Settings owner-facing instead of console-like', () => {
    // Status chrome copy was migrated to the i18n catalog (status.*); assert
    // catalog-key wiring rather than raw literals. The english section labels
    // remain raw in the source because they are identity/comparison/key values
    // (the localized DISPLAY goes through displaySectionLabel).
    assert.match(statusSource, /t\('status\.kicker', locale\)/);
    assert.doesNotMatch(statusSource, /Aegis health/);
    assert.doesNotMatch(statusSource, /<p className="vnext-kicker">System<\/p>/);
    assert.doesNotMatch(statusSource, /System check/);
    assert.match(statusSource, /label: 'Context'/); // identity label kept raw
    // The fallback owner_message copy moved to the catalog (work.statusFallback.*);
    // the english literal now lives in catalog.ts, not the StatusView source.
    assert.match(statusSource, /t\('work\.statusFallback\.context', locale\)/);
    assert.match(catalogSource, /remembered context to explain things/);
    // Headline pluralization moved to status.hero.needsYouOne/Many catalog keys.
    assert.match(statusSource, /status\.hero\.needsYouOne/);
    assert.match(statusSource, /status\.hero\.needsYouMany/);
    assert.match(statusSource, /t\('status\.hero\.description', locale\)/);
    assert.match(statusSource, /t\('status\.attention\.heading', locale\)/); // "Check first"
    assert.match(statusSource, /t\('status\.calm\.heading', locale\)/); // "Nothing needs attention"
    assert.match(statusSource, /ownerStatusSections/);
    assert.match(statusSource, /onOpenWork/);
    assert.match(statusSource, /onOpenSettings/);
    assert.match(statusSource, /const openAttention = attentionMetric > 0/);
    assert.match(statusSource, /<StatusMetric label=\{t\('status\.metric\.needsYou', locale\)\}[\s\S]*onClick=\{openAttention\}/);
    assert.match(statusSource, /<StatusMetric label=\{t\('status\.metric\.connections', locale\)\}[\s\S]*onClick=\{onOpenSettings\}/);
    assert.match(statusSource, /button type="button" className=\{classNames\('vnext-status-metric', tone, 'actionable'\)\}/);
    assert.match(statusSource, /<em aria-hidden="true">&gt;<\/em>/);
    assert.match(statusSource, /function statusAttentionAction/);
    // Attention-card action labels are localized via the catalog; the section
    // label that SELECTS each action stays the raw english comparison.
    assert.match(statusSource, /section\.label === 'Decisions'/);
    assert.match(statusSource, /section\.label === 'Connections'/);
    assert.match(statusSource, /section\.label === 'Cost controls'/);
    assert.match(statusSource, /t\('status\.action\.openWork', locale\)/);
    assert.match(statusSource, /t\('status\.action\.openSettings', locale\)/);
    assert.doesNotMatch(statusSource, /<p className="vnext-status-next-step"/);
    assert.match(statusSource, /label: 'Receipts'/); // identity label kept raw
    assert.match(statusSource, /t\('work\.statusFallback\.receipts', locale\)/);
    assert.match(catalogSource, /without a receipt/);
    assert.doesNotMatch(statusSource, /label: 'Evidence'/);
    assert.match(statusSource, /label: 'Routines'/); // identity label kept raw
    assert.match(statusSource, /t\('work\.statusFallback\.routines', locale\)/);
    assert.match(catalogSource, /Recurring responsibilities should say why they surfaced/);
    assert.match(statusSource, /function StatusOperationsDisclosure/);
    assert.match(statusSource, /t\('status\.operations\.summary', locale\)/); // "Working quietly"
    assert.match(statusSource, /aria-label=\{t\('status\.operations\.gridLabel', locale\)\}/);
    assert.doesNotMatch(statusSource, /Quiet operations/);
    assert.doesNotMatch(statusSource, /aria-label="Assistant operations"/);
    assert.match(statusSource, /vnext-status-operation-pills/);
    // Display lookups exist; CRITICAL they are not used in comparisons/keys.
    // displaySectionLabel stays local to StatusView; source-label display
    // localization is the shared model.ts localizedSourceLabel (one source of
    // truth for the Status disclosure + Settings services cards).
    assert.match(statusSource, /function displaySectionLabel/);
    assert.match(modelSource, /export function localizedSourceLabel/);
    assert.match(statusSource, /localizedSourceLabel\(view\.label, locale\)/);
    assert.doesNotMatch(statusSource, /displaySectionLabel\([^)]*\) ===/);
    assert.doesNotMatch(statusSource, /key=\{displaySectionLabel/);
    assert.doesNotMatch(statusSource, /key=\{localizedSourceLabel/);
    // The English label stays the identity used for filtering, never the display value.
    assert.match(settingsSource, /CONNECTION_SOURCE_LABELS/);
    assert.match(settingsSource, /'Calendar', 'Hercules'/);
    assert.match(statusSource, /ownerMessageForProjectionSection\(section, locale\)/);
    assert.match(statusSource, /ownerNextStepForProjectionSection\(section, locale\)/);
    assert.match(stylesSource, /\.vnext-status-operations:not\(\[open\]\) \.vnext-status-grid\s*\{[^}]*display: none;/s);
    assert.match(stylesSource, /\.vnext-status-operation-pill\s*\{[^}]*border-radius: 999px;/s);
    assert.doesNotMatch(statusSource, /Recommendations/);
    assert.doesNotMatch(statusSource, /Work watch/);
    // Settings chrome copy was migrated to the i18n catalog (settings.*);
    // assert catalog-key wiring rather than raw literals.
    assert.match(settingsSource, /tr\('settings\.account'\)/);
    assert.match(settingsSource, /tr\('settings\.ai\.heading'\)/);
    assert.doesNotMatch(settingsSource, /tr\('settings\.hero\.description'\)/);
    assert.doesNotMatch(settingsSource, /Control Aegis/);
    // Jump-link summary cards were removed; section anchors remain for deep links.
    assert.doesNotMatch(settingsSource, /targetId="vnext-settings-(services|security|alerts)"/);
    assert.doesNotMatch(settingsSource, /href=\{`\#\$\{targetId\}`\}/);
    assert.match(settingsSource, /id="vnext-settings-services"/);
    assert.match(settingsSource, /id="vnext-settings-security"/);
    assert.match(settingsSource, /id="vnext-settings-alerts"/);
    // Collapsed 高级 block holds advanced + nested developer (debug/evidence).
    // Logout lives on the 你 card in essentials.
    assert.match(settingsSource, /data-settings-tier="advanced"/);
    assert.match(settingsSource, /tr\('settings\.tier\.advanced'\)/);
    assert.match(settingsSource, /data-settings-section="logout"/);
    assert.match(settingsSource, /developer: \['support', 'security'\]/);
    assert.match(settingsSource, /tr\('settings\.spending\.pauseTitle'\)/);
    assert.match(settingsSource, /ownerMessageForProjectionSection\(source, locale\)/);
    assert.match(settingsSource, /ownerNextStepForProjectionSection\(section, locale\)/);
    assert.match(ownerProjectionTextSource, /containsInternalOperationTerms/);
    // Cost-saving copy + projection messages are now localized via the catalog.
    assert.match(ownerProjectionTextSource, /costSavingOwnerMessage/);
    assert.match(ownerProjectionTextSource, /work\.projection\.costSaving\.message/);
    assert.match(settingsSource, /tr\('settings\.services\.kicker'\)/);
    assert.match(settingsSource, /tr\('settings\.notifications\.kicker'\)/);
    assert.match(settingsSource, /tr\('settings\.spending\.saveModeOn'\)/);
    assert.match(settingsSource, /tr\('settings\.spending\.saveModeOff'\)/);
    assert.match(catalogSource, /Pauses scheduled checks, owner-approved routines, follow-ups/);
    assert.match(settingsSource, /settingsTelegramNote\(/);
    assert.match(catalogSource, /'settings\.notifications\.telegramOff'/);
    assert.match(catalogSource, /'settings\.notifications\.telegramStays'/);
    assert.doesNotMatch(settingsSource, /heartbeat/i);
    assert.doesNotMatch(settingsSource, /memory consolidation/i);
    assert.doesNotMatch(shellSource, /owner session|owner-requested|owner approval|owner attention|owner review|owner decision|owner settings|configured owner|owner surface|Owner-only access/i);
    assert.doesNotMatch(appSource, /<CommandBar/);
    assert.doesNotMatch(stylesSource, /vnext-command-wrap/);
    assert.match(stylesSource, /vnext-health-dot/);
    assert.match(stylesSource, /\.vnext-status-metrics\s*\{[^}]*grid-template-columns: repeat\(3, minmax\(0, 1fr\)\);[^}]*min-width: 0;/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-status-metrics\s*\{[^}]*width: 100%;[^}]*max-width: 100%;[^}]*overflow: hidden;/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-status-metric\s*\{[^}]*width: 100%;[^}]*min-width: 0;[^}]*min-height: 48px;[^}]*padding: 6px 3px;/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-status-hero button\.vnext-status-metric\s*\{[^}]*min-width: 0;[^}]*justify-self: stretch;/s);
    assert.match(stylesSource, /@media \(max-width: 820px\)[\s\S]*\.vnext-status-metric span\s*\{[^}]*white-space: nowrap;/s);
    assert.match(stylesSource, /button\.vnext-status-metric:hover,\s*button\.vnext-status-metric:focus-visible/);
    assert.match(stylesSource, /button\.vnext-status-metric em/);
    assert.match(stylesSource, /\.vnext-settings-summary-card:hover,\s*\.vnext-settings-summary-card:focus-visible/);
    assert.match(stylesSource, /\.vnext-settings-summary-card em/);
    assert.match(stylesSource, /\.vnext-status-card strong\s*\{[^}]*border-radius: 999px;[^}]*font-size: 11px;[^}]*text-transform: uppercase;/s);
    assert.doesNotMatch(stylesSource, /\.vnext-status-card strong\s*\{[^}]*font-family: Georgia/s);
    assert.doesNotMatch(statusSource, /background loops/i);
    assert.doesNotMatch(statusSource, /server projection/i);
    assert.doesNotMatch(statusSource, /hasServerProjection/);
    assert.doesNotMatch(statusSource, /label: 'Proof'/);
    assert.doesNotMatch(statusSource, /label: 'Instructions'/);
    assert.doesNotMatch(statusSource, /label: 'Memory'/);
    assert.doesNotMatch(statusSource, /Standing instructions/i);
    assert.doesNotMatch(statusSource, /Background Review/);
    assert.doesNotMatch(settingsSource, /handoff/i);
    assert.doesNotMatch(statusSource, /<summary>Details<\/summary>/);
    assert.doesNotMatch(settingsSource, /<summary>Details<\/summary>/);
  });

  it('uses modern system typography instead of old serif display styling', () => {
    assert.match(stylesSource, /--font-display: -apple-system, BlinkMacSystemFont, "SF Pro Display"/);
    assert.doesNotMatch(stylesSource, /Georgia|Times New Roman|Noto Serif|Source Han Serif/);
    assert.match(stylesSource, /\.vnext-chat-hero h2\s*\{[\s\S]*font-family: var\(--font-display\);/);
    assert.match(stylesSource, /\.vnext-detail-hero h2\s*\{[\s\S]*font-family: var\(--font-display\);/);
  });

  it('splits voice: browser dictation stays in the panel, the composer records a real voice note', () => {
    // Design B: floating panel text/dictation is gone; Realtime is duplex-only.
    // Browser STT helpers remain available as a module for other callers/tests.
    assert.doesNotMatch(ceoVoiceAgentSource, /browserSpeechRecognitionConstructor/);
    assert.doesNotMatch(ceoVoiceAgentSource, /startBrowserVoiceInput/);
    assert.match(voiceInputSource, /SpeechRecognition/);
    assert.match(voiceInputSource, /webkitSpeechRecognition/);
    assert.match(voiceInputSource, /voice\.notice\.captured/);
    assert.match(voiceInputSource, /resolveCeoShorthand/);
    // The COMPOSER mic records audio and the SERVER transcribes it.
    assert.doesNotMatch(chatHomeSource, /chatHome\.composer\.voiceInput/);
    assert.match(chatHomeSource, /chatHome\.voiceNote\.record/);
    assert.match(appSource, /startVoiceNoteRecording/);
    assert.match(appSource, /navigator\.mediaDevices\.getUserMedia\(\{ audio: true \}\)/);
    // 2026-09-13 voice pass: one mic grant serves consecutive notes, dropped when
    // the page hides; Stop flips the composer at once and the transcript is
    // sent as the turn's text (never the item-page "explain this" prompt).
    assert.match(appSource, /createVoiceMicSession\(\{/);
    assert.match(appSource, /getUserMedia: \(\) => micSession\.acquire\(\)/);
    assert.match(appSource, /releaseStream: \(stream\) => micSession\.release\(stream\)/);
    assert.match(appSource, /if \(document\.visibilityState === 'hidden'\) voiceMicSessionRef\.current\?\.dispose\(\);/);
    assert.match(appSource, /applyVoiceNoteEvent\(\{ type: 'stop' \}\);\n\s*handle\.stop\(\);/);
    assert.match(appSource, /const runtimeText = text\n\s*\|\| \(sentAttachment\?\.kind === 'voice' \? sentAttachment\.transcript\.trim\(\) : ''\);/);
    assert.match(appSource, /intent: 'chat',\n\s*text: runtimeText,/);
    assert.match(appSource, /if \(!typedBeforeRecording\) \{[\s\S]{0,400}?setChatHomeVoiceNote\(IDLE_VOICE_NOTE\);\n\s*void submitChatHomeCommand\(/);
    assert.match(appSource, /if \(generation !== voiceUploadGenerationRef\.current\) return;/);
    // Media capture is confined to the composer's voice-note path in App.tsx;
    // no other owner surface touches the microphone directly.
    const surfacesWithoutCapture = [settingsSource, statusSource, workSource, inlineAssistantSource, chatHomeSource].join('\n');
    assert.doesNotMatch(surfacesWithoutCapture, /getUserMedia/);
    assert.doesNotMatch(surfacesWithoutCapture, /mediaDevices/);
    // The recorder state machine is a pure module (unit-tested without a browser).
    assert.match(voiceNoteSource, /export function voiceNoteReducer/);
    assert.match(voiceNoteSource, /export function pickVoiceNoteMimeType/);
    // No browser global is REACHED here (doc comments may name the APIs the
    // injected primitives stand in for) — the module is testable in plain Node.
    const voiceNoteCode = voiceNoteSource.replace(/\/\*[\s\S]*?\*\/|\/\/[^\n]*/g, '');
    assert.doesNotMatch(voiceNoteCode, /\bwindow\./);
    assert.doesNotMatch(voiceNoteCode, /\bnavigator\./);
    assert.doesNotMatch(voiceNoteCode, /\bnew MediaRecorder\b/);
  });

  it('persists owner-scoped chat history across refresh without clearing it on transient session loss', () => {
    assert.match(appSource, /sessionStorage\.getItem/);
    assert.match(appSource, /sessionStorage\.setItem/);
    assert.match(appSource, /sessionStorage\.removeItem/);
    assert.match(appSource, /localStorage/);
    assert.match(appSource, /CHAT_HOME_DRAFT_KEY/);
    assert.match(appSource, /CHAT_HOME_HISTORY_KEY/);
    assert.doesNotMatch(appSource, /COMMAND_DRAFT_KEY/);
    assert.match(appSource, /writeSessionDraft\(CHAT_HOME_DRAFT_KEY, ''\)/);
    assert.match(appSource, /readChatHomeHistory\(window\.sessionStorage, CHAT_HOME_HISTORY_KEY\)/);
    // Console call sites pass the display locale so restored runtime notices
    // render in the console language (see the aria-current / <html lang> guard).
    assert.match(appSource, /writeChatHomeHistory\(window\.sessionStorage, CHAT_HOME_HISTORY_KEY, chatHomeMessages, errorLocale\)/);
    assert.match(appSource, /chatHomeHistoryOwnerKey\(CHAT_HOME_HISTORY_KEY, session\.email\)/);
    assert.match(appSource, /readChatHomeHistory\(window\.localStorage, ownerHistoryKey, errorLocale\)/);
    assert.match(appSource, /writeChatHomeHistory\(window\.localStorage, ownerHistoryKey, chatHomeMessages, errorLocale\)/);
    assert.match(appSource, /if \(chatHomeMessages\.length === 0 && existingOwnerHistory\.length > 0\) return;/);
    assert.match(appSource, /fetchVNextChatHistory\(initData\)/);
    assert.match(appSource, /chatHomeMessagesFromServerHistory\(history\)/);
    assert.match(appSource, /const CHAT_HOME_SERVER_RECOVERY_INTERVAL_MS = 5000;/);
    assert.match(appSource, /chatHomeHasTransientMessages/);
    assert.match(appSource, /function syncChatHomeHistoryFromServer/);
    assert.match(appSource, /setInterval\(recover, CHAT_HOME_SERVER_RECOVERY_INTERVAL_MS\)/);
    assert.match(appSource, /document\.addEventListener\('visibilitychange', onVisibilityChange\)/);
    assert.match(chatHistorySource, /import \{ normalizeMessageTimestamp \} from '\.\/time'/);
    assert.match(chatHistorySource, /timestamp: normalizeMessageTimestamp\(message\.timestamp\)/);
    assert.match(chatHistorySource, /export function chatHomeMessagesFromServerHistory/);
    assert.match(appSource, /const ownerTimestamp = Date\.now\(\)/);
    assert.match(appSource, /role: 'owner',[\s\S]{0,260}timestamp: ownerTimestamp/);
    assert.match(appSource, /role: 'working',[\s\S]{0,180}timestamp: Date\.now\(\)/);
    assert.match(appSource, /role: 'aegis',[\s\S]{0,320}timestamp: Date\.now\(\)/);
    assert.match(appSource, /mergeServerChatHistory\(serverMessages, current\)/);
    assert.match(appSource, /const CHAT_HOME_RECOVERED_REPLY_DUPLICATE_WINDOW_MS = 2 \* 60 \* 1000;/);
    assert.match(appSource, /function hasChatHomeAegisAfterOwner/);
    assert.match(appSource, /completeChatHomeTurn\(withoutWorking, displayText, ownerTimestamp, \{/);
    assert.match(appSource, /hasChatHomeAegisAfterOwner\(withoutWorking, displayText, ownerTimestamp\)/);
    assert.match(appSource, /function clearChatHomeConversation/);
    assert.match(appSource, /function clearChatHomeConversation\(\)[\s\S]*clearChatHomeHistory\(window\.sessionStorage, CHAT_HOME_HISTORY_KEY\);[\s\S]*clearChatHomeHistory\(window\.localStorage, ownerHistoryKey\);/);
    assert.match(appSource, /function clearPrivateBrowserSessionState\(\)/);
    assert.match(appSource, /function clearPrivateBrowserSessionState\(\)[\s\S]*setQueue\(null\);[\s\S]*setContext\(null\);[\s\S]*setSourceDetail\(null\);[\s\S]*setAssistantResult\(null\);[\s\S]*setOptimisticStatusByItemId\(\{\}\);/);
    const clearPrivateBody = appSource.match(/function clearPrivateBrowserSessionState\(\)[\s\S]*?\n  async function pauseCostSavingControls/)?.[0] ?? '';
    assert.doesNotMatch(clearPrivateBody, /clearChatHomeHistory/);
    assert.match(appSource, /if \(!nextSession\.authenticated\) \{[\s\S]*clearPrivateBrowserSessionState\(\);[\s\S]*setState\('ready'\);[\s\S]*return;[\s\S]*\}/);
    assert.match(appSource, /isLocalStaticPreviewSessionMiss\(loadError\)[\s\S]*clearPrivateBrowserSessionState\(\);[\s\S]*setSession\(\{ authenticated: false \}\)/);
    assert.match(appSource, /async function logoutNow\(\)[\s\S]*clearChatHomeHistory\(window\.sessionStorage, CHAT_HOME_HISTORY_KEY\);[\s\S]*clearChatHomeHistory\(window\.localStorage, ownerHistoryKey\);/);
    assert.match(appSource, /function chatHomeConversationContext\(quote\?: ChatHomeQuote \| null\): VNextConversationContextTurn\[\]/);
    assert.match(appSource, /message\.result\?\.visible_answer \|\| message\.text/);
    assert.match(appSource, /chatHomeConversationContext\(requestQuote\)/);
    assert.match(appSource, /quotedConversationTurn\(quote\)/);
    assert.match(appSource, /kind: 'quoted_reply'/);
    assert.match(appSource, /source_id: quote\.sourceId/);
    assert.doesNotMatch(appSource, /Aegis answer being quoted/);
    assert.doesNotMatch(appSource, /Owner message being quoted/);
    assert.match(appSource, /conversationContext,/);
    assert.match(appSource, /function compactConversationTurnText\(text: string\): string/);
    assert.match(appSource, /setChatHomeNotice\(t\('chatHome\.notice\.chatCleared', errorLocale\)\)/);
  });

  it('keeps item-scoped AI inside the current item instead of jumping to the global command bar', () => {
    const itemAssistantBody = appSource.match(/async function runAssistant\([\s\S]*?\n  async function submitChatHomeCommand/)?.[0] ?? '';
    const chatWorkAskBody = appSource.match(/async function askChatHomeWorkItem\([\s\S]*?\n  function clearChatHomeConversation/)?.[0] ?? '';
    const itemActionBody = appSource.match(/async function executeConfirmedItemAction[\s\S]*?\n  function isStaleItemActionError/)?.[0] ?? '';
    assert.ok(itemAssistantBody, 'runAssistant should remain inspectable');
    assert.ok(chatWorkAskBody, 'askChatHomeWorkItem should remain inspectable');
    assert.ok(itemActionBody, 'executeConfirmedItemAction should remain inspectable');
    assert.doesNotMatch(itemAssistantBody, /navigate\(|setView\(/);
    assert.doesNotMatch(chatWorkAskBody, /navigate\(|setView\(|onOpenWork/);
    assert.doesNotMatch(itemActionBody, /navigate\(|setView\(/);
    assert.match(workSource, /InlineAssistantPanel/);
    assert.match(inlineAssistantSource, /InlineAssistantTurn/);
    assert.match(inlineAssistantSource, /vnext-inline-assistant-thread/);
    // The conversation aria-label is i18n-migrated; assert the catalog key wiring
    // (render-level copy is covered by tests/vnextInlineAssistantPanel.test.mjs).
    assert.match(inlineAssistantSource, /aria-label=\{t\('inlineAssistant\.aria\.conversation'\)\}/);
    assert.match(workSource, /function briefPrompt\(locale: Locale\): string/);
    assert.match(workSource, /function draftPrompt\(locale: Locale\): string/);
    assert.match(workSource, /function runDecisionPrompt\(prompt: string, label: string\)/);
    assert.match(workSource, /setAssistantTurns/);
    assert.match(workSource, /role: 'owner'/);
    assert.match(workSource, /role: 'working'/);
    assert.match(workSource, /role: 'aegis'/);
    assert.match(workSource, /role: 'queued'/);
    assert.match(workSource, /role: 'working',[\s\S]{0,180}timestamp: Date\.now\(\)/);
    assert.match(workSource, /role: 'aegis',[\s\S]{0,220}timestamp: Date\.now\(\)/);
    assert.match(workSource, /role: 'owner',[\s\S]{0,180}timestamp: Date\.now\(\)/);
    assert.match(workSource, /role: 'queued',[\s\S]{0,180}timestamp: Date\.now\(\)/);
    assert.match(workSource, /type QueuedInlineAssistantPrompt =/);
    assert.match(workSource, /queuedAssistantPrompts/);
    assert.match(workSource, /if \(busy\) \{[\s\S]*setQueuedAssistantPrompts/);
    assert.match(workSource, /function runBrief\(\) \{[\s\S]*if \(!selected\) return;[\s\S]*if \(busy\) \{[\s\S]*submitAssistant\(selected, briefPrompt\(locale\)\);[\s\S]*return;/);
    assert.match(workSource, /function runDraft\(\) \{[\s\S]*if \(!selected\) return;[\s\S]*if \(busy\) \{[\s\S]*submitAssistant\(selected, draftPrompt\(locale\)\);[\s\S]*return;/);
    assert.match(workSource, /function runDecisionPrompt\(prompt: string, label: string\) \{[\s\S]*if \(!selected\) return;[\s\S]*if \(busy\) \{[\s\S]*submitAssistant\(selected, prompt\);[\s\S]*return;/);
    assert.match(appSource, /type QueuedChatHomeWorkPrompt =/);
    assert.match(appSource, /queuedChatHomeWorkPrompts/);
    assert.match(appSource, /const \[sourceDetailBusy, setSourceDetailBusy\] = useState\(false\)/);
    assert.match(appSource, /sourceDetailBusy=\{sourceDetailBusy\}/);
    assert.match(appSource, /async function openSourceDetail\(\)[\s\S]*setSourceDetailBusy\(true\);[\s\S]*setSourceDetailBusy\(false\);/);
    assert.doesNotMatch(appSource, /async function openSourceDetail\(\)[\s\S]*?setBusy\(true\);[\s\S]*?async function executeConfirmedItemAction/);
    assert.doesNotMatch(appSource, /async function executeConfirmedItemAction[\s\S]*?setBusy\(true\);[\s\S]*?async function executeItemAction/);
    assert.match(appSource, /function queueChatHomeWorkItemAsk\(messageId: string, item: OwnerWorkItem, text: string\)/);
    assert.match(appSource, /responseTimestamp\?: number/);
    assert.match(appSource, /responseTimestamp: Date\.now\(\)/);
    assert.match(appSource, /question: text, timestamp: Date\.now\(\), itemStatus: item\.status, state: 'queued'/);
    assert.match(appSource, /question: text, timestamp: Date\.now\(\), itemStatus: item\.status, state: 'working'/);
    assert.match(appSource, /if \(busy && !queuedThreadId\) \{[\s\S]*queueChatHomeWorkItemAsk\(messageId, item, text\);[\s\S]*return;/);
    assert.match(appSource, /focusId: item\.id,[\s\S]*intent: 'chat',/);
    assert.match(appSource, /onAskText=\{\(item, text, conversationContext\) => void runAssistant\('chat', text, item\.id, undefined, conversationContext\)\}/);
    assert.match(appSource, /chatHomeWorkConversationContext\(messageId, item\.status, queuedThreadId\)/);
    assert.match(appSource, /if \(entry\.state === 'queued'\) return \[\];/);
    assert.match(appSource, /setQueuedChatHomeWorkPrompts\(remaining\)/);
    assert.match(chatHomeSource, /state\?: 'queued' \| 'working'/);
    assert.match(chatHomeSource, /entry\.state === 'queued'/);
    assert.match(chatHomeSource, /chatHome\.work\.queuedNext/);
    assert.match(workSource, /inlineAssistantConversationContextBeforeQueuedTurn/);
    // i18n-migrated; assert catalog-key wiring (render copy covered by tests/vnextInlineAssistantPanel.test.mjs).
    assert.match(inlineAssistantSource, /aria-label=\{t\('inlineAssistant\.aria\.ask'\)\}/);
    assert.match(chatHomeSource, /aria-label=\{busy \? tr\.t\('chatHome\.work\.queueAboutItem'\) : tr\.t\('chatHome\.work\.askAboutItem'\)\}/);
    assert.doesNotMatch(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.work\.askAboutItem'\)\}[\s\S]{0,180}disabled=\{busy\}/);
    assert.doesNotMatch(chatHomeSource, /disabled=\{busy \|\| executingAction !== null\}/);
    assert.doesNotMatch(chatHomeSource, /if \(!trimmedAsk \|\| busy\) return;/);
    assert.doesNotMatch(workSource, /aria-label="Ask Aegis about this item"[\s\S]{0,180}disabled=\{busy\}/);
    assert.match(workSource, /disabled=\{sourceDetailBusy\}/);
    assert.doesNotMatch(workSource, /onOpenSource\} disabled=\{busy\}/);
    assert.match(workSource, /disabled=\{executingAction !== null\}/);
    assert.doesNotMatch(workSource, /disabled=\{busy\}[\s\S]{0,220}onClick=\{\(\) => onAction\(action\)\}/);
    assert.doesNotMatch(workSource, /vnext-blocked-primary" onClick=\{onAsk\} disabled=\{busy\}/);
    assert.match(inlineAssistantSource, /vnext-inline-turn-label/);
    assert.match(inlineAssistantSource, /focus\(\{ preventScroll: true \}\)/);
    assert.doesNotMatch(inlineAssistantSource, /scrollIntoView/);
    assert.match(stylesSource, /\.vnext-inline-turn\.owner,\s*\.vnext-inline-turn\.queued/);
    assert.match(inlineAssistantSource, /disabled=\{busy && !text\.trim\(\)\}/);
    assert.match(inlineAssistantSource, /aria-label=\{busy \? t\('inlineAssistant\.aria\.queueForAegis'\) : t\('inlineAssistant\.aria\.sendToAegis'\)\}/);
    assert.doesNotMatch(inlineAssistantSource, /disabled=\{busy\}[\s\S]{0,220}onClick=\{\(\) => onSend\(suggestion\.prompt\)\}/);
    assert.doesNotMatch(inlineAssistantSource, /result\?\.response_kind === 'error'[\s\S]{0,260}disabled=\{busy\}/);
    assert.doesNotMatch(inlineAssistantSource, /result\.response_kind === 'error'[\s\S]{0,260}disabled=\{busy\}/);
    assert.match(inlineAssistantSource, /t\('inlineAssistant\.retry\.queueRetry'\)/);
    assert.match(workSource, /kind: 'prompt' \| 'retry'/);
    assert.match(workSource, /function retryAssistant\(\)/);
    assert.match(workSource, /onRetryAssistant\?\.\(\)/);
    assert.match(workSource, /onRetryAssistantLabel/);
    assert.match(appSource, /assistantResultItemId/);
    const sourceDetailPanelCall = "<SourceDetailPanel sourceDetail={sourceDetail} collapsed={sourceDetail.kind === 'task'} />";
    assert.ok(
      workSource.indexOf('aria-label="Recommended actions"') < workSource.indexOf(sourceDetailPanelCall),
      'Work detail should put Ask/Brief/Draft/Open controls before long source detail content',
    );
    assert.ok(
      workSource.indexOf("aria-label={t('work.aria.receiptActions', locale)}") >= 0
        && workSource.indexOf("aria-label={t('work.aria.receiptActions', locale)}") < workSource.indexOf(sourceDetailPanelCall),
      'Work detail should keep provider actions like archive/mark-read before long source detail content',
    );
    assert.match(appSource, /assistantResult=\{assistantResult\}/);
    assert.match(appSource, /assistantResultItemId=\{assistantResultItemId\}/);
    assert.doesNotMatch(appSource, /const selectedAssistantResult = selected\?\.id && selected\.id === assistantResultItemId/);
    assert.match(appSource, /runAssistant\(\s*lastItemCommand\.intent,[\s\S]*lastItemCommand\.text,[\s\S]*lastItemCommand\.itemId,[\s\S]*lastItemCommand\.idempotencyKey,[\s\S]*lastItemCommand\.conversationContext \?\? \[\],[\s\S]*\)/);
    // Item-scoped still-working callback ignores the util's English default and
    // sets the localized visible_answer (display_language) instead.
    assert.match(appSource, /onStillWorking: \(\) => \{[\s\S]*visible_answer: t\('chatHome\.working\.stillChecking', errorLocale\)/);
    assert.match(inlineAssistantSource, /t\('inlineAssistant\.collapse'\)/);
    assert.doesNotMatch(inlineAssistantSource, /disabled=\{busy \|\| !text\.trim\(\)\}/);
    assert.match(appSource, /focusIdOverride/);
    assert.match(workSource, /t\('work\.blocked\.suggest\.revise\.prompt', locale\)/);
    assert.doesNotMatch(appSource, /setCommand\('Revise this paused automation before I approve/);
    assert.doesNotMatch(workSource, /onFocusItemCommand/);
    assert.doesNotMatch(appSource, /navigate\('work', item\.id\);\n\s+window\.setTimeout\(\(\) => commandInputRef\.current\?\.focus\(\), 0\);/);
    assert.doesNotMatch(appSource, /commandInputRef/);
  });
});

describe('Work↔Mailbox FE unification wiring (A3 + A4)', () => {
  it('does not keep mailbox cross-refresh or MailView open-target wiring', () => {
    assert.doesNotMatch(appSource, /mailRefreshNonce/);
    assert.doesNotMatch(appSource, /mailOpenTarget/);
    assert.doesNotMatch(appSource, /inQueueMailRefs/);
    assert.doesNotMatch(appSource, /MailView/);
    assert.doesNotMatch(workSource, /onViewInMailbox/);
  });
});

// ---- IMG_3937/3938 batch: status-bar mask, kicker contrast, snippet decode ----
const chromeCss = readFileSync(new URL('../src/vnext/design/surfaces/chrome.css', import.meta.url), 'utf8');
const consoleCss = readFileSync(new URL('../src/vnext/design/surfaces/console.css', import.meta.url), 'utf8');

describe('vnext screenshot fixes (IMG_3937/3938)', () => {
  it('frosts only the status-bar inset and keeps the header below it', () => {
    // The status frost is exactly the device inset. A sticky top offset, or a
    // frost as tall as the header, washed out the first card (owner 2026-09-24).
    assert.match(chromeCss, /body::before\s*\{[^}]*height:\s*env\(safe-area-inset-top, 0px\)/s);
    assert.match(chromeCss, /body::before\s*\{[^}]*backdrop-filter:\s*none/s);
    assert.doesNotMatch(chromeCss, /body::before\s*\{[^}]*backdrop-filter:\s*blur/s);
    assert.doesNotMatch(chromeCss, /body::before\s*\{[^}]*console-header-h|body::before\s*\{[^}]*52px/s);
    assert.doesNotMatch(chromeCss, /--safe-chrome-blur/);
    assert.doesNotMatch(chromeCss, /:is\(\.vnext-app-work[^)]*\)::after/);
    assert.match(chromeCss, /vnext-chat-topbar\.vnext-owner-shell-chrome[\s\S]*top:\s*0;/);
    assert.doesNotMatch(chromeCss, /vnext-chat-topbar\.vnext-owner-shell-chrome[\s\S]*top:\s*env\(safe-area-inset-top/);
    assert.match(chromeCss, /\.vnext-nav\s*\{[^}]*position:\s*fixed/s);
    assert.match(chromeCss, /\.vnext-nav\s*\{[^}]*bottom:\s*calc\(-1 \* var\(--layout-shortfall, 0px\)\)/s);
    assert.match(chromeCss, /\.vnext-nav\s*\{[^}]*padding:[^;]*var\(--layout-shortfall, 0px\)/s);
    assert.match(chromeCss, /body::after\s*\{[^}]*height:\s*var\(--layout-shortfall, 0px\)/s);
    assert.doesNotMatch(chromeCss, /body::after\s*\{[^}]*height:\s*calc\(var\(--phone-tab-h/);
    assert.match(chromeCss, /\.vnext-nav button\s*\{[^}]*font-size:\s*16px/s);
    assert.match(chatHomeCss, /\.vnext-app\.vnext-app-chat-home\s*\{[^}]*overflow:\s*visible/s);
    const menuCss = readFileSync(new URL('../src/vnext/design/surfaces/menu.css', import.meta.url), 'utf8');
    assert.match(menuCss, /\.vnext-header-menu\.vnext-header-menu-portal\s*\{[^}]*position:\s*fixed/s);
    assert.match(menuCss, /\.vnext-header-menu\.vnext-header-menu-portal\s*\{[^}]*height:\s*0/s);
    assert.doesNotMatch(chatHomeCss, /height:\s*calc\(100dvh \+ var\(--layout-shortfall/);
    assert.doesNotMatch(readFileSync(new URL('../index.html', import.meta.url), 'utf8'), /name="viewport"[^>]*interactive-widget/);
    // Degraded banner desktop sticky pins BELOW the safe strip (not top:0).
    const banner = stylesSource.match(/\.vnext-llm-degraded-banner\s*\{[^}]*\}/s);
    assert.ok(banner, 'degraded banner rule exists');
    assert.match(banner[0], /top:\s*var\(--safe-top, 0px\);/);
    assert.doesNotMatch(banner[0], /top:\s*0;/);
    // Full z>42 fixed/sticky audit (Codex): the only overlays above the mask
    // must be full-screen modals (80/90) or offset below the strip. The
    // focused skip link (z 60) renders at top — keep it out of the notch band.
    const feedbackCss = readFileSync(new URL('../src/vnext/design/surfaces/feedback.css', import.meta.url), 'utf8');
    const skip = feedbackCss.match(/\.vnext-app \.vnext-skip-link\s*\{[^}]*\}/s);
    assert.ok(skip, 'skip-link rule exists');
    assert.match(skip[0], /top:\s*max\(10px, var\(--safe-top, 0px\)\);/);
    // Hidden state must clear its own height PLUS the top offset (a
    // height-relative -160% left the link painted inside the strip once top
    // grew to safe-top). With this calc the hidden bottom edge = 0 for ANY
    // safe-top (WebKit-measured: hidden bottom 0 at both 0px and 59px).
    assert.match(skip[0], /transform:\s*translateY\(calc\(-100% - max\(10px, var\(--safe-top, 0px\)\)\)\);/);
  });

  it('remaps the work-card kicker literals (#b6ddff/#a9f0c8) onto adaptive tokens', () => {
    // Dark-era literals are invisible on the light page (IMG_3937 为何/下一步/邮箱操作).
    // 2026-09-16 console pass: the facts are a plain label/value list — muted
    // label, hairline row, no accent-tinted tile — still on adaptive tokens.
    assert.match(consoleCss, /\.vnext-app-work \.vnext-work-intent-row em\s*\{[^}]*color:\s*var\(--b-text-3\)/s);
    assert.match(consoleCss, /\.vnext-app-work \.vnext-item-receipt-actions > span\s*\{[^}]*color:\s*var\(--b-text-3\)/s);
    assert.match(consoleCss, /\.vnext-app-work \.vnext-work-intent-row span\s*\{[^}]*border-top:\s*1px solid var\(--b-hairline\)/s);
    assert.doesNotMatch(consoleCss, /\.vnext-app-work \.vnext-work-intent-row span\s*\{[^}]*color-mix\(in srgb, var\(--b-accent\)/s);
  });

  it('frosted surfaces share ONE material recipe via tokens (owner: 透明度高一点 + consistency)', () => {
    const themeCss = readFileSync(new URL('../src/vnext/design/theme.css', import.meta.url), 'utf8');
    // Tokens: bars 60% (more transparent per owner), nav 80% strong, one blur.
    assert.match(themeCss, /--frost-fill:\s*color-mix\(in srgb, var\(--b-bg\) 60%, transparent\)/);
    assert.match(themeCss, /--frost-fill-strong:\s*color-mix\(in srgb, var\(--b-bg\) 80%, transparent\)/);
    assert.match(themeCss, /--frost-blur:\s*blur\(18px\) saturate\(140%\)/);
    // The phone header and the home-indicator strip are the page ground.
    // Chrome must not paint a frosted band over either one.
    assert.doesNotMatch(chromeCss, /background:\s*var\(--frost-fill\)/);
    assert.doesNotMatch(chromeCss, /backdrop-filter:\s*var\(--frost-blur\)/);
    // 2026-09-16 console pass: the bottom nav, the console header AND (pass 2)
    // the chat topbar are OPAQUE bars (--b-bar) with a hairline — no frost, no
    // blur — so scrolled content never bleeds through the chrome.
    assert.match(chatHomeCss, /\.vnext-app-chat-home \.vnext-chat-topbar\s*\{[^}]*background:\s*var\(--b-bar\)/s);
    assert.doesNotMatch(chatHomeCss, /\.vnext-app-chat-home \.vnext-chat-topbar\s*\{[^}]*var\(--frost-fill\)/s);
    assert.match(chromeCss, /\.vnext-nav\s*\{[^}]*background:\s*var\(--b-bar\)[^}]*backdrop-filter:\s*none/s);
    assert.doesNotMatch(chromeCss, /\.vnext-nav\s*\{[^}]*backdrop-filter:\s*var\(--frost-blur\)/s);
    assert.match(chromeCss, /\.vnext-header\.vnext-owner-shell-chrome\s*\{[^}]*background:\s*var\(--b-bar\)/s);
    assert.match(chromeCss, /\.vnext-header\.vnext-owner-shell-chrome\s*\{[^}]*border-radius:\s*0/s);
  });

  it('lifts the phone composer by the measured keyboard inset', () => {
    const lifted = chatHomeCss.match(/html\.is-keyboard-open \.vnext-app\.vnext-app-chat-home\s*\{[^}]*\}/s);
    assert.ok(lifted, 'keyboard-open app shell rule exists');
    assert.match(lifted[0], /height:\s*var\(--keyboard-visible, 100dvh\)/);
    assert.doesNotMatch(lifted[0], /safe-area-inset-bottom|874px|852px|393px|keyboard-inset/);
    assert.match(chatHomeCss, /html\.is-keyboard-open \.vnext-app\.vnext-app-chat-home\s*\{[^}]*padding-bottom:\s*0/s);
    assert.doesNotMatch(chatHomeCss, /is-composing\) \.vnext-chat-home\s*\{[^}]*100dvh/s);
    assert.match(chromeCss, /html\.is-keyboard-open \.vnext-app-chat-home \.vnext-nav\s*\{[^}]*display:\s*none/s);
    assert.doesNotMatch(chromeCss, /is-composing\) \.vnext-nav/);
    assert.match(appSource, /syncKeyboardInset\(window\)/);
  });

  it('chat consistency pass: inline-note + work-facts adaptive; dead account-menu CSS removed', () => {
    // #9be7c4 (.ok) and rgba(7,17,29,.28) facts wash were dark-era literals —
    // invisible / off-palette on the light page.
    assert.match(chatHomeCss, /\.vnext-app-chat-home \.vnext-chat-inline-note\.ok\s*\{[^}]*color:\s*var\(--b-ok\)/s);
    assert.match(chatHomeCss, /\.vnext-app-chat-home \.vnext-chat-work-facts span\s*\{[^}]*background:\s*var\(--b-surface\)/s);
    // .vnext-chat-account-menu is rendered by NO component (grep-verified) and
    // carried a hardcoded dark card that broke light theme — must stay deleted.
    assert.doesNotMatch(stylesSource, /vnext-chat-account-menu/);
    // Codex sibling-catch: the same pale-green/amber literal class also lived
    // in receipts, the snapshot toast, the status ok-metric and the degraded
    // banner — all remapped in their ACTIVE surfaces.
    assert.match(chatHomeCss, /\.vnext-chat-work-receipt svg[\s\S]{0,220}color:\s*var\(--b-ok\)/);
    assert.match(consoleCss, /\.vnext-work-action-receipt svg[\s\S]{0,320}color:\s*var\(--b-ok\)/);
    // Owner 2026-09-16 pass 2: metric digits are INK (never big green
    // numbers); only the 需关注 > 0 cell is amber.
    assert.doesNotMatch(consoleCss, /\.vnext-app-status \.vnext-status-metric\.ok strong\s*\{[^}]*var\(--b-ok\)/s);
    assert.match(consoleCss, /\.vnext-app-status \.vnext-status-metric\.warn strong[^{]*\{[^}]*var\(--b-decide\)/s);
    assert.match(chromeCss, /\.vnext-snapshot-toast\.ok\s*\{[^}]*var\(--b-ok\)/s);
    assert.match(chromeCss, /\.vnext-llm-degraded-banner\s*\{[^}]*var\(--b-decide\)/s);
    // The opaque bar needs no frost legibility backstop (pass 2 removed the
    // theme-bg glyph shadow along with the frost).
    assert.doesNotMatch(chatHomeCss, /\.vnext-chat-topbar \.vnext-chat-identity-label[\s\S]{0,200}text-shadow:/);
    // RichText renders Markdown across the B view roots: the checklist
    // checkmark must resolve through the adaptive scope group, not fall
    // through to the base #9be7c4 literal (Codex).
    const richTextCss = readFileSync(new URL('../src/vnext/design/surfaces/rich-text.css', import.meta.url), 'utf8');
    assert.match(richTextCss, /:is\(\.vnext-app-chat-home, \.vnext-app-work, \.vnext-app-status, \.vnext-app-settings\)[^{]*\.vnext-rich-check-item\.is-checked/);
    assert.match(
      richTextCss,
      /:is\(\.vnext-app-chat-home, \.vnext-app-work, \.vnext-app-status, \.vnext-app-settings\) \.vnext-rich-text a,\s*:is\(\.vnext-app-chat-home, \.vnext-app-work, \.vnext-app-status, \.vnext-app-settings\) \.vnext-chat-answer-card a\s*\{[^}]*overflow-wrap:\s*anywhere;[^}]*word-break:\s*break-all;/s,
    );
  });

  it('phone thread starts below the bar so the status frost cannot cover the card', () => {
    const phoneRows = chatHomeCss.match(/Phone thread sits BELOW the bar[\s\S]*?@media \(max-width: 820px\)\s*\{[\s\S]*?\n\}/);
    assert.ok(phoneRows, 'phone row block exists in the ACTIVE chat-home surface');
    assert.match(phoneRows[0], /\.vnext-chat-home\.has-conversation \.vnext-chat-topbar\s*\{[^}]*grid-row:\s*1/s);
    assert.match(phoneRows[0], /\.vnext-chat-home\.has-conversation \.vnext-chat-stream\s*\{[^}]*grid-row:\s*2/s);
    assert.match(phoneRows[0], /grid-template-rows:\s*auto minmax\(0, 1fr\) auto/);
    assert.doesNotMatch(phoneRows[0], /safe-area-inset-top \+ 8px|console-header-h, 52px\) \+ env/);
    assert.match(phoneRows[0], /has-context-picker \.vnext-chat-composer\s*\{[^}]*grid-row:\s*3/s);
    assert.match(phoneRows[0], /\.vnext-chat-home\.has-conversation\.composer-half\s*\{[^}]*minmax\(230px, 46dvh\)/s);
  });

  it('styles the chat date divider in the ACTIVE chat-home surface (IMG_3939)', () => {
    // Dead-layer lesson: chat-home.css (loaded after styles.css) is the live
    // layer for chat-home chrome; the divider must be styled there with
    // adaptive tokens or it ships unstyled.
    assert.match(chatHomeCss, /\.vnext-app-chat-home \.vnext-chat-date-divider\s*\{/);
    assert.match(chatHomeCss, /\.vnext-app-chat-home \.vnext-chat-date-divider span\s*\{[^}]*var\(--b-hairline\)/s);
    const chatHomeSource2 = readFileSync(new URL('../src/vnext/ChatHomeView.tsx', import.meta.url), 'utf8');
    assert.match(chatHomeSource2, /chatDateDividerLabels\(displayMessages, userTimezone, locale, dividerNowMs\)/);
    assert.match(chatHomeSource2, /vnext-chat-date-divider/);
    // Midnight rollover (Codex): a minutely day-key check drives dividerNowMs;
    // the same-value setState bails out except at an actual day change.
    // (Effects are not SSR-observable in this harness — source guard + the
    // pure helper's injectable-now tests cover the behavior.)
    assert.match(chatHomeSource2, /setInterval\(/);
    assert.match(chatHomeSource2, /chatDayKey\(Date\.now\(\), userTimezone\) === chatDayKey\(current, userTimezone\)/);
  });

  it('keeps HTML-entity decoding as a generic helper', () => {
    const htmlEntitiesSource = readFileSync(new URL('../src/vnext/htmlEntities.ts', import.meta.url), 'utf8');
    assert.match(htmlEntitiesSource, /export function decodeHtmlEntities/);
  });
});

describe('S2 — degraded banners occupy a single reserved slot', () => {
  it('the embedder banner yields when the LLM banner is visible (mobile reserves ONE banner row)', () => {
    // The chat-home mobile layout carves out exactly one banner row via
    // :has(.vnext-llm-degraded-banner); both banners share that class, so two
    // visible banners would clip the composer. Pin the App wiring: the
    // EmbedderDegradedBanner render is gated on the LLM banner NOT being
    // visible (healthy or dismissed-this-outage).
    assert.match(
      appSource,
      /\{!\(statusProjection\?\.llm_degraded\s*&& llmDegradedDismissedSince !== statusProjection\.llm_degraded\.since\) && \(/,
      'EmbedderDegradedBanner must be gated on LLM-banner visibility',
    );
    // And the embedder banner reuses the same styled banner surface (the
    // :has() carve-out keys on this class).
    const embedderSource = readFileSync(new URL('../src/vnext/EmbedderDegradedBanner.tsx', import.meta.url), 'utf8');
    assert.match(embedderSource, /className="vnext-llm-degraded-banner"/);
  });
});

describe('Composer attachment-only send (owner: "attaching an image does nothing")', () => {
  it('opens every empty-text gate on the send path when an attachment is held', () => {
    // The bug: the upload succeeded ("vNext upload accepted ... kind=image") but
    // the send path had `if (!text) return` gates, so the turn vanished with
    // no error, no bubble, nothing. Each remaining gate consults the held attachment.
    assert.match(appSource, /function chatHomeHasSendableAttachment\(\): boolean/);
    // 1. submitChatHomeCommand
    assert.match(appSource, /if \(!text && !chatHomeHasSendableAttachment\(\) && !voiceNoteOverride\) return \{ kind: 'empty' \};/);
    // 1b. busy → the attachment rides the queue entry instead of being dropped.
    assert.match(appSource, /queueChatHomeCommand\(prompt, contextOverride, modeOverride, displayTextOverride, retryIdempotencyKey, quoteOverride, queueAttachments\)/);
    assert.match(appSource, /if \(!text && attachments\.length === 0\) return;/);
    // 2. deskTalkToAegis → submitChatHomeCommand (no floating panel openAndAsk)
    assert.match(appSource, /if \(!trimmed && !chatHomeHasSendableAttachment\(\)\) return;/);
    assert.match(appSource, /void submitChatHomeCommand\(trimmed \|\| undefined, undefined, undefined, mode\)/);
    // 3. secretaryAsk — the attachment is captured BEFORE the gate (kept for voice tools).
    assert.match(appSource, /if \(!trimmed && requestAttachments\.length === 0\) return \{ kind: 'empty' \};/);
    // Floating panel submitOwnerTurn is gone — one composer only.
    assert.doesNotMatch(ceoVoiceAgentSource, /submitOwnerTurn/);
    // The ChatHomeView form submits an attachment-only turn rather than bailing.
    assert.match(chatHomeSource, /if \(canSendAttachmentOnly\) onSubmit\(''\)/);
  });

  it('keeps exactly one chat composer and one ChatHomeView for full + sheet modes', () => {
    // One ChatHomeView mount site in App; host switches full / sheet / dormant.
    const chatHomeMounts = appSource.match(/<ChatHomeView\b/g) || [];
    assert.equal(chatHomeMounts.length, 1, 'App must render ChatHomeView exactly once');
    assert.match(appSource, /vnext-chat-panel-host/);
    assert.match(appSource, /presentationMode=\{view === 'home' \? 'full' : 'sheet'\}/);
    assert.match(chatHomeSource, /presentationMode === 'sheet' \? 'is-sheet' : 'is-full'/);
    assert.match(chatHomeSource, /data-testid=\{presentationMode === 'sheet' \? 'aegis-chat-sheet' : 'aegis-chat-full'\}/);
    assert.match(chatHomeSource, /aegis-chat-sheet-handle/);
    assert.match(chatHomeSource, /aegis-chat-sheet-close/);
    // Exactly one composer textarea (ChatHomeView); no floating panel textarea.
    assert.match(chatHomeSource, /aria-label=\{tr\.t\('chatHome\.composer\.tellAria'\)\}/);
    assert.doesNotMatch(ceoVoiceAgentSource, /<textarea/);
    assert.doesNotMatch(ceoVoiceAgentSource, /voice\.panel\.textPlaceholder/);
    assert.doesNotMatch(appSource, /onOpenText/);
    assert.doesNotMatch(appSource, /voice\.chrome\.openText/);
    assert.doesNotMatch(catalogSource, /'voice\.chrome\.openText'/);
    assert.doesNotMatch(catalogSource, /'voice\.panel\.textPlaceholder'/);
    assert.doesNotMatch(catalogSource, /'voice\.panel\.backToVoice'/);
    assert.doesNotMatch(catalogSource, /'voice\.panel\.send'/);
    assert.doesNotMatch(appSource, /aegis-chat-sheet-open/);
    assert.doesNotMatch(stylesSource, /\.vnext-aegis-chat-fab/);
    assert.doesNotMatch(appSource, /chatFabHiddenByScroll/);
    assert.doesNotMatch(appSource, /vnext-aegis-chat-fab/);
    assert.match(appSource, /workingPhraseForTool/);
    assert.match(appSource, /from '\.\/toolPhrases'/);
    assert.match(stylesSource, /\.vnext-chat-panel-host\.is-sheet/);
  });

  it('the bottom nav has two fixed tabs without a duplicate header chat opener', () => {
    const navBlock = appSource.match(/<nav className="vnext-nav"[\s\S]*?<\/nav>/);
    assert.ok(navBlock, 'bottom nav block missing');
    assert.equal((navBlock[0].match(/<NavButton/g) || []).length, 2, 'exactly two nav tabs');
    assert.doesNotMatch(navBlock[0], /view === 'home' \?/);
    assert.doesNotMatch(appSource, /vnext-nav-chat-open-slot/);
    assert.match(stylesSource, /\.vnext-nav \{[\s\S]*?grid-template-columns: repeat\(2, minmax\(0, 1fr\)\)/);
    const chrome = appSource.match(/<OwnerShellChrome[\s\S]*?variant="console"[\s\S]*?\/>/);
    assert.ok(chrome, 'console chrome missing');
    assert.doesNotMatch(chrome[0], /aegis-chat-sheet-open|vnext-shell-chat-open/);
    const chromeSource = readFileSync(new URL('../src/vnext/OwnerShellChrome.tsx', import.meta.url), 'utf8');
    assert.match(chromeSource, /\{trailing\}\n\s*\{statusText \? \(/);
  });

  it('the main chat tab remains without duplicate global openers', () => {
    // V1 keeps two fixed tabs and item-scoped questions, without a global duplicate.
    const navBlock = appSource.match(/<nav className="vnext-nav"[\s\S]*?<\/nav>/);
    assert.ok(navBlock, 'bottom nav block missing');
    assert.doesNotMatch(navBlock[0], /aegis-chat-sheet-open/);
    assert.doesNotMatch(navBlock[0], /vnext-nav-chat-open/);
    assert.doesNotMatch(appSource, /aegis-chat-sheet-open/);
    assert.doesNotMatch(appSource, /vnext-aegis-chat-fab/);
    assert.doesNotMatch(stylesSource, /vnext-aegis-chat-fab/);
    const srcTree = [
      appSource,
      chatHomeSource,
      settingsSource,
      statusSource,
      workSource,
      stylesSource,
    ].join('\n');
    assert.doesNotMatch(srcTree, /vnext-aegis-chat-fab/);
  });

  it('locks page scroll behind the chat sheet with a backdrop and helper', () => {
    assert.match(appSource, /aegis-chat-sheet-backdrop/);
    assert.match(appSource, /applyChatSheetScrollLock/);
    assert.match(appSource, /releaseChatSheetScrollLock/);
    assert.match(appSource, /findChatSheetPageScroller/);
    assert.match(appSource, /from '\.\/chatSheetScrollLock'/);
    assert.match(appSource, /createPortal/);
    assert.match(chatHomeSource, /aegis-chat-sheet-stream/);
    assert.match(stylesSource, /\.vnext-chat-sheet-backdrop/);
    assert.match(stylesSource, /overscroll-behavior:\s*contain/);
  });

  it('builds page-aware CEO screen context for Work, Status, and Settings', () => {
    assert.match(appSource, /function buildCeoScreenContext\(\): string/);
    assert.match(appSource, /surface=\$\{view\}/);
    assert.match(appSource, /next_safe_action=/);
    assert.match(appSource, /lane=\$\{laneForItem\(selected\)\}/);
    assert.doesNotMatch(appSource, /mail_body=/);
    assert.doesNotMatch(appSource, /mail_list_subjects:/);
    assert.match(appSource, /status_sections:/);
    assert.match(appSource, /settings\.display_language=/);
    assert.match(appSource, /settings\.llm_model=/);
    assert.match(appSource, /truncateScreenContext/);
    assert.doesNotMatch(appSource, /setMailScreenContext/);
    // Chat asks carry the screen snapshot.
    assert.match(appSource, /buildPanelConversationContext\(\{[\s\S]*screenContext,/);
    assert.match(appSource, /clientContext,/);
  });

  it('an upload or voice failure is shown, never swallowed', () => {
    // Attachment upload errors carry the server's own reason into the pill.
    assert.match(appSource, /setChatHomeAttachmentError\(\s*displayError\(error/);
    // Voice failures map the transport status onto a specific owner message:
    // 503 = speech-to-text not configured on the host, 422 = no speech heard.
    assert.match(appSource, /status === 422 \? 'empty' : status === 503 \? 'unsupported' : 'upload'/);
    assert.match(appSource, /function voiceNoteErrorMessage\(code: VoiceNoteErrorCode\): string/);
    for (const key of [
      'chatHome.voiceNote.errorPermission',
      'chatHome.voiceNote.errorUnsupported',
      'chatHome.voiceNote.errorRecord',
      'chatHome.voiceNote.errorUpload',
      'chatHome.voiceNote.errorEmpty',
    ]) {
      assert.ok(catalogSource.includes(`'${key}'`), `missing i18n key ${key}`);
    }
  });

  it('sends a voice note as a voice attachment and shows it in the sent bubble', () => {
    assert.match(appSource, /type: 'voice',\s*transcript: voice\.transcript,/);
    assert.match(appSource, /duration_secs: Math\.max\(0, Math\.round\(voice\.durationSecs\)\)/);
    // Empty typed text sends the transcript as the message text.
    assert.match(appSource, /const trimmed = typed \|\| voiceTranscript;/);
    // The owner bubble carries the attachment descriptor for both send paths.
    assert.match(appSource, /function chatHomeSentAttachment\(/);
    assert.match(appSource, /\.\.\.\(sentAttachment \? \{ attachment: sentAttachment \} : \{\}\)/);
    assert.match(chatHomeSource, /<ChatMessageAttachment attachment=\{message\.attachment\} tr=\{tr\} \/>/);
  });
});

describe('chat home composer survives a queued-turn drain', () => {
  it('only clears composer state for a fresh send, not when draining a queued turn', () => {
    const start = appSource.indexOf('async function submitChatHomeCommand');
    assert.ok(start > 0, 'submitChatHomeCommand exists');
    const body = appSource.slice(start, start + 12000);
    const guard = body.indexOf('if (!queuedOwnerMessageId) {');
    const clear = body.indexOf("setChatHomeCommand('');");
    assert.ok(guard > 0 && clear > guard, 'composer clear is guarded by !queuedOwnerMessageId');
    // A drained queued turn carries ONLY what was queued with it (never an
    // attachment added to the composer afterwards).
    assert.match(body, /const requestAttachments = queuedOwnerMessageId\s*\n\s*\? \(queuedAttachments \?\? \[\]\)\s*\n\s*: !requestContext && !retryIdempotencyKey\s*\n\s*\? chatHomeCommandAttachments/, 'a drained queued turn never picks up a newly added attachment');
  });
});

describe('runtime rails wiring (Telegram banner, offline banner) — incident 2026-09-15', () => {
  it('mounts the offline banner first and hides the runtime banners while offline', () => {
    assert.match(appSource, /const online = useOnlineStatus\(\);/);
    assert.match(appSource, /<OfflineBanner offline=\{!online\} \/>/);
    // The LLM/Telegram/embedder chain only renders while online: their data
    // cannot refresh without a network, so the offline notice owns the slot.
    assert.match(appSource, /\{online && \(\s*<>\s*<LlmDegradedBanner/);
  });

  it('gives the Telegram rail the single banner slot ahead of the embedder, behind the LLM banner', () => {
    assert.match(
      appSource,
      /telegramRailBannerState\(telegramRail, telegramRailDismissedSince, nowUnixSeconds\(\)\) \? \(\s*<TelegramRailBanner/,
    );
    const telegramSource = readFileSync(new URL('../src/vnext/TelegramRailBanner.tsx', import.meta.url), 'utf8');
    // Same reserved-row surface as the other banners (the :has() carve-out).
    assert.match(telegramSource, /vnext-llm-degraded-banner vnext-telegram-rail-banner/);
    const offlineSource = readFileSync(new URL('../src/vnext/OfflineBanner.tsx', import.meta.url), 'utf8');
    assert.match(offlineSource, /vnext-llm-degraded-banner vnext-offline-banner/);
  });

  it('keys the Telegram dismissal on since_unix and persists it like the LLM banner', () => {
    assert.match(appSource, /useState<number \| null>\(\s*\(\) => readTelegramRailDismissedSince\(\),\s*\)/);
    assert.match(appSource, /function dismissTelegramRailBanner\(sinceUnix: number\) \{\s*setTelegramRailDismissedSince\(sinceUnix\);\s*writeTelegramRailDismissedSince\(sinceUnix\);/);
    const railsSource = readFileSync(new URL('../src/vnext/statusRails.ts', import.meta.url), 'utf8');
    assert.match(railsSource, /dismissedSinceUnix === rail\.since_unix\) return null;/);
    const telegramSource = readFileSync(new URL('../src/vnext/TelegramRailBanner.tsx', import.meta.url), 'utf8');
    assert.match(telegramSource, /onClick=\{\(\) => onDismiss\(banner\.sinceUnix\)\}/);
  });

  it('Status renders the Telegram rail row in the Connections list and the nightly line as muted text', () => {
    assert.match(statusSource, /data-telegram-rail-row="true"/);
    assert.match(statusSource, /'vnext-status-row', 'vnext-status-row-telegram', !telegramRow\.healthy && 'warn'/);
    assert.match(statusSource, /className="vnext-muted vnext-status-nightly" data-nightly-line="true"/);
  });
});
