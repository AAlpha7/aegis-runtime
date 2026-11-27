/**
 * Lightweight typed frontend i18n catalog.
 *
 * Mirrors the backend `core/src/i18n.rs` `t(key, locale)` philosophy: a plain
 * dictionary keyed by message id, each entry carrying every supported locale.
 * No heavy i18n library (consistent with the backend's deliberate no-`fluent`
 * choice). Keys are added as webapp strings are migrated (see the i18n plan
 * Tasks 7/8).
 *
 * Parity is guarded by `webapp/tests/i18nCatalogParity.test.mjs`, which fails
 * the suite if any key is missing a `zh` or `en` translation — a missing key
 * must surface as a test failure, never as a silent English leak.
 */

export type Locale = 'zh' | 'en';

type Entry = Record<Locale, string>;

export const CATALOG: Record<string, Entry> = {
  'settings.validation.notice': { zh: '调用配置需验证成功才保存。验证会向所选服务发送固定测试内容；API 路径可能产生少量费用。失败时保留原配置，不使用兜底代测。关闭或移除配置无需联网。', en: 'Call settings are saved only after a successful check using fixed test content. API checks may incur a small charge. Failures preserve the previous configuration; fallback services cannot pass the check for another service. Disabling or removing settings works offline.' },
  'settings.validation.save': { zh: '验证并保存', en: 'Verify and save' },
  'settings.validation.checking': { zh: '正在验证…', en: 'Verifying…' },
  'settings.validation.saved': { zh: '验证通过，已保存。', en: 'Verified and saved.' },
  'settings.validation.failed': { zh: '无法确认保存结果，请刷新设置核对；页面不会自动重发保存请求。', en: 'Could not confirm the save result. Reload settings to check; this page will not automatically resubmit the save request.' },
  'settings.jev.verified': { zh: '已开启 · 保存时验证通过', en: 'Enabled · checked at save' },
  'settings.jev.title': { zh: 'Jev 安全判断', en: 'Jev safety assessment' },
  'settings.jev.subtitle': { zh: '可选增强 · 默认关闭', en: 'Optional enhancement · off by default' },
  'settings.jev.disclosure': { zh: '开启后，待外发内容、目标和操作上下文会发送给 OpenRouter Decisions 判断。使用独立 API 计费，Grok／ChatGPT 订阅不包含此费用。', en: 'When enabled, proposed outbound content, destinations and action context are sent to OpenRouter Decisions for assessment. API charges are separate from Grok/ChatGPT subscriptions.' },
  'settings.jev.enable': { zh: '启用 Jev', en: 'Enable Jev' },
  'settings.jev.on': { zh: '已开启 · 未验证', en: 'Enabled · unverified' },
  'settings.jev.off': { zh: '已关闭', en: 'Off' },
  'settings.jev.loading': { zh: '状态待确认', en: 'Status pending' },
  'settings.jev.needsKey': { zh: '缺少密钥', en: 'Key required' },
  'settings.jev.key': { zh: 'OpenRouter API key', en: 'OpenRouter API key' },
  'settings.jev.keepKey': { zh: '留空保留已保存密钥', en: 'Leave blank to keep the saved key' },
  'settings.jev.enterKey': { zh: '输入 sk-or- 开头的密钥', en: 'Enter a key starting with sk-or-' },
  'settings.jev.stored': { zh: '密钥已加密保存，不会回显。', en: 'Key saved encrypted; it is never displayed again.' },
  'settings.jev.environment': { zh: '已有主机注入的凭据。可在此保存独立密钥覆盖它；主机凭据只能在主机上删除。', en: 'A host-injected credential is available. Save a dedicated key here to override it; host credentials can only be removed on the host.' },
  'settings.jev.missing': { zh: '尚未配置兼容凭据。普通 OpenAI key 无法用于 Jev。', en: 'No compatible credential configured. Generic OpenAI keys cannot be used for Jev.' },
  'settings.jev.invalidKey': { zh: '请输入有效格式的 OpenRouter key，不能含空格或示例占位符。', en: 'Enter an OpenRouter key without whitespace or placeholder text.' },
  'settings.jev.consent': { zh: '我同意上述数据发送范围和独立 API 费用。', en: 'I agree to the data sharing and separate API charges described above.' },
  'settings.jev.localRules': { zh: '关闭时继续使用本地检查和必要确认；它们不等同于 Jev 语义判断。', en: 'When off, local checks and required approvals remain; they are not equivalent to Jev semantic assessment.' },
  'settings.jev.scope': { zh: '开启或更换密钥时，先发送固定测试内容验证 Jev，可能产生少量 API 费用；成功才保存。关闭或移除无需验证。验证只代表当时可用。', en: 'Enabling Jev or changing its key first sends fixed test content and may incur a small API charge. Only a successful check is saved. Disabling or removal needs no check. Verification reflects availability at that time.' },
  'settings.jev.save': { zh: '保存', en: 'Save' },
  'settings.jev.cancel': { zh: '取消', en: 'Cancel' },
  'settings.jev.remove': { zh: '移除已保存密钥', en: 'Remove saved key' },
  'settings.jev.removing': { zh: '保存后移除密钥并关闭 Jev。主机凭据不受影响。', en: 'Saving will remove the stored key and turn Jev off. Host credentials remain unchanged.' },
  'settings.jev.reload': { zh: '重新加载', en: 'Reload' },
  'settings.jev.saved': { zh: '已保存，后续请求将使用此配置。', en: 'Saved. Subsequent requests will use this configuration.' },
  'settings.jev.loadFailed': { zh: '暂时无法读取 Jev 配置，请重试。主机需配置加密凭据库。', en: 'Could not load Jev configuration. Retry; the host needs an encrypted credential store.' },
  'settings.jev.saveFailed': { zh: '未能确认保存结果，请重新加载核对。密钥输入已清空，重试时需重新输入。', en: 'Could not confirm the save. Reload to check the stored state. The key field was cleared; re-enter it to retry.' },
  'callout.important': { zh: '重要', en: 'Important' },
  'callout.warning': { zh: '警告', en: 'Warning' },
  'callout.critical': { zh: '严重', en: 'Critical' },
  'callout.dismissedImportance': { zh: '已忽略重要标记', en: 'Dismissed importance' },
  'callout.dismiss': { zh: '忽略', en: 'Dismiss' },
  'callout.restore': { zh: '恢复', en: 'Restore' },
  'callout.reminder': { zh: '提醒', en: 'Reminder' },
  'callout.alert': { zh: '警报', en: 'Alert' },
  'callout.systemRestart': { zh: '系统重启', en: 'System restart' },
  'callout.resourcePressure': { zh: '资源紧张', en: 'Resource pressure' },
  'richText.code': { zh: '代码', en: 'Code' },

  // Settings — tier labels (essentials / advanced / developer).
  'settings.tier.advanced': { zh: '高级', en: 'Advanced' },
  'settings.tier.advancedHint': {
    zh: 'AI、花费与头像',
    en: 'AI, spending, and avatars',
  },
  'settings.tier.developer': { zh: '开发者', en: 'Developer' },
  'settings.tier.developerHint': {
    zh: '快照、E2E 与登录会话',
    en: 'Snapshots, E2E, and sign-in',
  },

  // Settings — language toggle (Task 6).
  'settings.language': { zh: '语言', en: 'Language' },
  'settings.language.description': {
    zh: '选择安琪界面与系统消息使用的语言。',
    en: 'Choose the language Aegis uses for the interface and system messages.',
  },
  'settings.provider.reasoning.low': { zh: '低', en: 'Low' },
  'settings.provider.reasoning.medium': { zh: '中', en: 'Medium' },
  'settings.provider.reasoning.high': { zh: '高', en: 'High' },
  'settings.provider.reasoning.xhigh': { zh: '极高', en: 'Extra high' },
  // Section heading is 语言; the control caption says what the choice changes.
  'settings.language.caption': { zh: '界面语言', en: 'Interface language' },
  'settings.language.zh': { zh: '中文', en: '中文' },
  'settings.language.en': { zh: 'English', en: 'English' },

  // Settings — hero + account.
  'settings.kicker': { zh: '设置', en: 'Settings' },
  'settings.preferences': { zh: '偏好设置', en: 'Preferences' },
  'settings.hero.description': {
    zh: '你、语言、通知、已连接的服务与隐私。',
    en: 'You, language, notifications, connected services, and privacy.',
  },
  'settings.account': { zh: '你', en: 'You' },
  'settings.account.label': { zh: '已登录账户', en: 'Signed-in account' },
  'settings.account.signedIn': { zh: '已登录', en: 'Signed in' },
  'settings.account.youFallback': { zh: '你', en: 'You' },
  'settings.logout': { zh: '退出登录', en: 'Log out' },
  'settings.loggingOut': { zh: '正在退出', en: 'Logging out' },
  'settings.services.headingShort': { zh: '已连接的服务', en: 'Connected services' },
  'settings.privacy.heading': { zh: '隐私', en: 'Privacy' },

  // Settings — Support / debug section.
  'settings.support.kicker': { zh: '支持', en: 'Support' },
  'settings.support.heading': { zh: '证据与调试访问', en: 'Evidence and debug access' },
  'settings.support.backendE2eTitle': { zh: '后端 E2E 套件', en: 'Backend E2E suite' },
  'settings.support.backendE2eDescription': {
    zh: '推荐用于生产检查。Aegis 会在后端运行器内创建受限授权、对日志脱敏，并清理冒烟任务。',
    en: 'Recommended for production checks. Aegis creates a scoped grant inside the backend runner, redacts logs, and cleans up the smoke task.',
  },
  'settings.support.snapshotDescription': {
    zh: '保存快照会捕获一个不含 cookie、令牌、消息正文、原始 HTML 或附件的隐私安全视图。',
    en: 'Save snapshot captures a privacy-safe view without cookies, tokens, message bodies, raw HTML, or attachments.',
  },
  'settings.support.terminalGrantPrefix': { zh: '终端授权：', en: 'Terminal grant: ' },
  'settings.support.activeLeaseSingular': { zh: '{count} 个有效授权', en: '{count} active lease' },
  'settings.support.activeLeasePlural': { zh: '{count} 个有效授权', en: '{count} active leases' },
  'settings.support.noneActive': { zh: '无有效授权', en: 'none active' },
  'settings.support.checking': { zh: '检查中', en: 'Checking' },
  'settings.support.runBackendE2e': { zh: '运行后端 E2E', en: 'Run backend E2E' },
  'settings.support.saving': { zh: '保存中', en: 'Saving' },
  'settings.support.saveSnapshot': { zh: '保存快照', en: 'Save snapshot' },

  // Settings — E2E evidence.
  'settings.e2e.latestResultLabel': { zh: '最新 E2E 结果', en: 'Latest E2E result' },
  'settings.e2e.latestPrefix': { zh: '最新 E2E：', en: 'Latest E2E: ' },
  'settings.e2e.phasesLabel': { zh: '最新 E2E 阶段', en: 'Latest E2E phases' },
  'settings.e2e.hideRawLog': { zh: '隐藏原始日志', en: 'Hide raw log' },
  'settings.e2e.showRawLog': { zh: '显示原始日志', en: 'Show raw log' },
  'settings.e2e.noLogYet': { zh: '暂无日志输出。', en: 'No log output yet.' },
  'settings.e2e.previousRuns': { zh: '历史 E2E 运行（{count}）', en: 'Previous E2E runs ({count})' },
  'settings.e2e.previousRunsLabel': { zh: '历史 E2E 运行', en: 'Previous E2E runs' },

  // Settings — E2E verdict + status labels.
  'settings.verdict.deployReady': { zh: '可部署', en: 'Deploy ready' },
  'settings.verdict.pathPassed': { zh: '路径通过', en: 'Path passed' },
  'settings.verdict.blocked': { zh: '被阻塞', en: 'Blocked' },
  'settings.verdict.running': { zh: '运行中', en: 'Running' },
  'settings.verdict.queued': { zh: '排队中', en: 'Queued' },
  'settings.status.passed': { zh: '已通过', en: 'Passed' },
  'settings.status.failed': { zh: '已失败', en: 'Failed' },
  'settings.status.running': { zh: '运行中', en: 'Running' },
  'settings.status.queued': { zh: '排队中', en: 'Queued' },
  'settings.phase.needsReview': { zh: '需要复核', en: 'Needs review' },

  // Settings — debug grant token.
  'settings.grant.tokenLabel': { zh: '一次性调试授权令牌', en: 'One-time debug grant token' },
  'settings.grant.tokenReady': { zh: '终端授权已就绪', en: 'Terminal grant ready' },
  'settings.grant.tokenDescription': {
    zh: '仅当终端代理需要运行实时冒烟测试时才复制此请求头。刷新后将不再显示。',
    en: 'Copy this header only if a terminal agent needs to run live smoke tests. It will not be shown again after refresh.',
  },
  'settings.grant.copied': { zh: '已复制', en: 'Copied' },
  'settings.grant.copy': { zh: '复制', en: 'Copy' },
  'settings.grant.tokenUnrecoverable': {
    zh: '终端授权处于有效状态，但其令牌仅显示过一次且无法恢复。请撤销它，然后在终端代理需要该请求头时重新开启一个终端授权。',
    en: 'A terminal grant is active, but its token was shown once and cannot be recovered. Revoke it, then open a new terminal grant if a terminal agent needs the header.',
  },

  // Settings — terminal grant section.
  'settings.terminal.summary': { zh: '终端授权', en: 'Terminal grant' },
  'settings.terminal.description': {
    zh: '面向仅限终端检查的高级路径。优先使用后端 E2E，因为其令牌保留在服务器运行器内部。',
    en: 'Advanced path for terminal-only checks. Prefer backend E2E because its token stays inside the server runner.',
  },
  'settings.terminal.durationLabel': { zh: '终端授权时长', en: 'Terminal grant duration' },
  'settings.terminal.duration': { zh: '时长', en: 'Duration' },
  'settings.terminal.chooseDuration': { zh: '选择终端授权时长', en: 'Choose terminal grant duration' },
  'settings.terminal.scope': { zh: '范围', en: 'Scope' },
  'settings.terminal.scopeLabel': { zh: '终端授权范围', en: 'Terminal grant scope' },
  'settings.terminal.opening': { zh: '开启中', en: 'Opening' },
  'settings.terminal.openGrant': { zh: '开启终端授权', en: 'Open terminal grant' },
  'settings.terminal.noneActive': { zh: '当前没有有效的终端授权。', en: 'No terminal grant is active.' },
  'settings.terminal.activeGrantsLabel': { zh: '有效的终端授权', en: 'Active terminal grants' },
  'settings.terminal.grantSuffix': { zh: ' 终端授权', en: ' terminal grant' },
  'settings.terminal.expiresPrefix': { zh: '到期于 ', en: 'Expires ' },
  'settings.terminal.revoke': { zh: '撤销', en: 'Revoke' },

  // Settings — Appearance / Avatars.
  'settings.appearance.kicker': { zh: '外观', en: 'Appearance' },
  'settings.appearance.heading': { zh: '头像', en: 'Avatars' },
  'settings.avatar.aegisLabel': { zh: '{assistant}', en: '{assistant}' },
  'settings.avatar.aegisDescription': {
    zh: '显示在聊天、回复和账户界面中。',
    en: 'Shown in chat, replies, and account surfaces.',
  },
  'settings.avatar.aegisPreviewLabel': { zh: '打开安琪头像预览', en: 'Open Aegis avatar preview' },
  'settings.avatar.ownerLabel': { zh: '你', en: 'You' },
  'settings.avatar.ownerDescription': {
    zh: '显示在此设备上你的消息旁边。',
    en: 'Shown next to your messages on this device.',
  },
  'settings.avatar.ownerPreviewLabel': { zh: '打开你的头像预览', en: 'Open your avatar preview' },
  'settings.avatar.aegisReset': { zh: '安琪头像已重置。', en: 'Aegis avatar reset.' },
  'settings.avatar.ownerReset': { zh: '你的头像已重置。', en: 'Your avatar reset.' },
  'settings.avatar.aegisUpdated': { zh: '安琪头像已在此设备上更新。', en: 'Aegis avatar updated on this device.' },
  'settings.avatar.ownerUpdated': { zh: '你的头像已在此设备上更新。', en: 'Your avatar updated on this device.' },
  'settings.avatar.generating': { zh: '正在生成新的安琪头像。这可能需要一点时间。', en: 'Generating a new Aegis avatar. This can take a moment.' },
  'settings.avatar.chooseImage': { zh: '选择图片', en: 'Choose image' },
  'settings.avatar.reset': { zh: '重置', en: 'Reset' },
  'settings.avatar.selfImageKicker': { zh: '安琪自我形象', en: 'Aegis self-image' },
  'settings.avatar.weeklyRefreshEnabled': { zh: '每周刷新已启用', en: 'Weekly refresh enabled' },
  'settings.avatar.weeklyRefreshOff': {
    zh: '打开后每周自动刷新自我形象。',
    en: 'Turn on to refresh the self-image automatically each week.',
  },
  'settings.avatar.selfImageDescription': {
    zh: '当连接了具备图像能力的订阅服务商时，安琪可以刷新她自己的头像。',
    en: 'Aegis can refresh her own avatar when an image-capable subscription provider is connected.',
  },
  'settings.avatar.styleHint': { zh: '头像描述', en: 'Avatar description' },
  'settings.avatar.styleHintPlaceholder': {
    zh: '默认：二十多岁、优雅的女性人形 AI 助手，柔和棚拍光，干净的中性背景',
    en: 'Default: an elegant female humanoid AI assistant in her 20s, soft studio lighting, clean neutral background',
  },
  'settings.avatar.styleHintCaption': {
    zh: '写什么就画什么，完全按你的描述生成；留空用默认形象。',
    en: 'Your description is the whole subject; leave it empty for the default look.',
  },
  'settings.avatar.generateAvailableTitle': { zh: '生成并保存新的安琪头像', en: 'Generate and save a new Aegis avatar' },
  'settings.avatar.generateUnavailableTitle': {
    zh: '立即尝试图像生成；如果所连接的服务商无法完成，安琪会作出说明。',
    en: 'Try image generation now; Aegis will explain if the connected provider cannot do it.',
  },
  'settings.avatar.generating.button': { zh: '生成中', en: 'Generating' },
  'settings.avatar.generateButton': { zh: '生成安琪头像', en: 'Generate Aegis avatar' },
  'settings.avatar.pauseWeekly': { zh: '暂停每周刷新', en: 'Pause weekly' },
  'settings.avatar.enableWeekly': { zh: '启用每周刷新', en: 'Enable weekly' },
  'settings.avatar.couldNotUseImage': { zh: '无法使用该图片。', en: 'Could not use that image.' },
  'settings.avatar.usePngJpegWebp': { zh: '请使用 PNG、JPEG 或 WebP。', en: 'Use PNG, JPEG, or WebP.' },
  'settings.avatar.under8mb': { zh: '请使用小于 8 MB 的图片。', en: 'Use an image under 8 MB.' },
  'settings.avatar.couldNotRead': { zh: '无法读取该图片。', en: 'Could not read that image.' },
  'settings.avatar.couldNotPrepare': { zh: '无法处理该图片。', en: 'Could not prepare that image.' },
  'settings.avatar.previewCloseLabel': { zh: '关闭头像预览', en: 'Close avatar preview' },
  'settings.avatar.close': { zh: '关闭', en: 'Close' },
  'settings.avatar.aegisPreviewDialogLabel': { zh: '安琪头像预览', en: 'Aegis avatar preview' },
  'settings.avatar.ownerPreviewDialogLabel': { zh: '你的头像预览', en: 'Your avatar preview' },

  // Settings — Spending / cost controls.
  'settings.spending.kicker': { zh: '支出', en: 'Spending' },
  'settings.runtime.heading': { zh: '运行', en: 'Runtime' },
  'settings.spending.saveMode': { zh: '省钱模式', en: 'Save mode' },
  'settings.spending.saveModeOn': { zh: '自动检查已暂停，只在你要求时工作。', en: 'Automatic checks paused — I only work when you ask' },
  'settings.spending.saveModeOff': {
    zh: '打开后暂停后台自动检查，只在你发消息时工作。',
    en: 'Turn on to pause background checks; I only work when you message me.',
  },
  'settings.spending.pausedTitle': { zh: '自动检查已暂停', en: 'Automatic checks paused' },
  'settings.spending.pauseTitle': { zh: '暂停自动检查', en: 'Pause automatic checks' },
  'settings.spending.pausedMessage': {
    zh: '自动检查已暂停，只在你要求时工作。',
    en: 'Automatic checks paused — I only work when you ask',
  },
  'settings.spending.working': { zh: '处理中', en: 'Working' },
  'settings.spending.resumeChecks': { zh: '恢复检查', en: 'Resume checks' },
  'settings.spending.pauseNow': { zh: '立即暂停', en: 'Pause now' },
  'settings.spending.updating': { zh: '正在更新自动检查。', en: 'Updating automatic checks.' },
  'settings.support.savingSnapshot': { zh: '正在保存支持快照。', en: 'Saving support snapshot.' },

  // Settings — Web push device notifications.
  'settings.push.deviceKicker': { zh: '设备通知', en: 'Device notifications' },
  'settings.push.enabledHere': { zh: '已在此设备启用推送通知', en: 'Push notifications enabled here' },
  'settings.push.enabledElsewhere': { zh: '已在其他设备启用推送通知', en: 'Push notifications enabled elsewhere' },
  'settings.push.enableTitle': { zh: '启用推送通知', en: 'Enable push notifications' },
  'settings.push.setupNeeded': { zh: '需要设置 Web 推送', en: 'Web Push setup needed' },
  'settings.push.serverKeysMissing': { zh: '服务器推送密钥尚未配置。', en: 'Server push keys are not configured yet.' },
  'settings.push.ownerMessage': {
    zh: '控制此设备的浏览器推送。WebApp 时间线保持开启，确保重要通知和记录不会丢失。',
    en: 'Controls browser push for this device. The WebApp timeline remains on so important notices and receipts are not lost.',
  },
  'settings.push.notRegistered': {
    zh: '此浏览器当前未注册。在此启用即可在该设备上接收通知。',
    en: 'This browser is not registered right now. Enable here to receive notifications on this device.',
  },
  'settings.push.checkingBrowser': { zh: '正在检查此浏览器注册情况。', en: 'Checking this browser registration.' },
  'settings.push.savedDeviceSingular': { zh: '此所有者已保存 {count} 台设备。', en: '{count} saved device for this owner.' },
  'settings.push.savedDevicePlural': { zh: '此所有者已保存 {count} 台设备。', en: '{count} saved devices for this owner.' },
  'settings.push.lastDeliverySingular': {
    zh: '上次发送：{status}，{delivered}/{attempted} 台设备，时间为 {time}{cleaned}。',
    en: 'Last delivery: {status}, {delivered}/{attempted} device at {time}{cleaned}.',
  },
  'settings.push.lastDeliveryPlural': {
    zh: '上次发送：{status}，{delivered}/{attempted} 台设备，时间为 {time}{cleaned}。',
    en: 'Last delivery: {status}, {delivered}/{attempted} devices at {time}{cleaned}.',
  },
  'settings.push.cleanedSuffix': { zh: '；已清理 {count} 个过期项', en: '; cleaned {count} expired' },
  'settings.push.working': { zh: '处理中', en: 'Working' },
  'settings.push.turnOff': { zh: '关闭推送', en: 'Turn off push' },
  'settings.push.checking': { zh: '检查中', en: 'Checking' },
  'settings.push.enable': { zh: '启用推送', en: 'Enable push' },
  'settings.push.sendTest': { zh: '发送测试', en: 'Send test' },

  // Settings — AI provider (Codex + Grok).
  'settings.provider.kicker': { zh: 'AI 服务商', en: 'AI Provider' },
  'settings.provider.codexConnected': { zh: 'Codex 已连接', en: 'Codex connected' },
  'settings.provider.codexSetupNeeded': { zh: '需要设置 Codex', en: 'Codex setup needed' },
  'settings.provider.codexInstall': { zh: '安装 Codex CLI', en: 'Install Codex CLI' },
  'settings.provider.codexOwnerMessage': {
    zh: 'Codex 还没登录。',
    en: 'Codex is not signed in yet.',
  },
  'settings.provider.modelLabel': { zh: '模型', en: 'Model' },
  'settings.provider.reasoningLabel': { zh: '主模型思考深度', en: 'Primary model thinking depth' },
  'settings.provider.reasoningHint': {
    zh: '只对主模型 Codex 生效，备用模型不受影响；越高越仔细，也更慢、更花钱。',
    en: 'Applies only to the primary Codex model, not the backup; higher is more careful, slower and costlier.',
  },
  'settings.provider.label': { zh: '服务商', en: 'Provider' },
  'settings.provider.codexControlsLabel': { zh: 'Codex 模型控制', en: 'Codex model controls' },
  'settings.provider.codeLabel': { zh: '验证码', en: 'Code' },
  'settings.provider.openOpenAI': { zh: '打开 OpenAI', en: 'Open OpenAI' },
  'settings.provider.working': { zh: '处理中', en: 'Working' },
  'settings.provider.stopUsingForChat': { zh: '停止用于聊天', en: 'Stop using for chat' },
  'settings.provider.useCodexForChat': { zh: '将 Codex 用于聊天', en: 'Use Codex for chat' },
  'settings.provider.checking': { zh: '检查中', en: 'Checking' },
  'settings.provider.iApprovedIt': { zh: '我已批准', en: 'I approved it' },
  'settings.provider.newCode': { zh: '新验证码', en: 'New code' },
  'settings.provider.starting': { zh: '启动中', en: 'Starting' },
  'settings.provider.connect': { zh: '连接', en: 'Connect' },
  'settings.provider.grokConnected': { zh: 'Grok 已连接', en: 'Grok connected' },
  'settings.provider.grokSignInPending': { zh: 'Grok 登录待处理', en: 'Grok sign-in pending' },
  'settings.provider.grokConnect': { zh: '连接 Grok', en: 'Connect Grok' },
  'settings.provider.grokOwnerMessage': {
    zh: 'Grok 还没连接。',
    en: 'Grok is not connected.',
  },
  'settings.provider.codexModelSummary': {
    zh: '模型：{model} · {reasoning} · {routing} · 仅前台 · 上限 {cap}/天',
    en: 'Model: {model} · {reasoning} · {routing} · foreground only · cap {cap}/day',
  },
  'settings.provider.grokModelSummary': {
    zh: '模型：{model} · {routing} · 仅前台 · 上限 {cap}/天',
    en: 'Model: {model} · {routing} · foreground only · cap {cap}/day',
  },
  'settings.provider.activeForChat': { zh: '已用于聊天', en: 'active for chat' },
  'settings.provider.notRoutingChat': { zh: '未用于聊天', en: 'not routing chat' },
  'settings.provider.authPrefix': { zh: '认证：', en: 'Auth: ' },
  'settings.provider.openXAI': { zh: '打开 xAI', en: 'Open xAI' },
  'settings.provider.useForChat': { zh: '用于聊天', en: 'Use for chat' },
  'settings.provider.disconnect': { zh: '断开连接', en: 'Disconnect' },

  // X (Twitter) integration — Phase A public search via Grok; Phase B OAuth later.
  'settings.x.kicker': { zh: '社交', en: 'Social' },
  'settings.x.heading': { zh: 'X（Twitter）', en: 'X (Twitter)' },
  'settings.x.description': {
    zh: '手机上的 X App 负责刷流与发送；Aegis 负责筛选、摘要、起草，并用「在 X 中打开」交给系统 App。公开搜索经 Grok（只读）。以你身份同步提及/代发，需之后「连接 X 账号」。',
    en: 'The X app on your phone handles the feed and sending; Aegis filters, summarizes, drafts, and hands off via Open in X. Public search uses Grok (read-only). Mentions/posting as you need Connect X later.',
  },
  'settings.x.publicSearchReady': {
    zh: '公开 X 搜索：可用（经 Grok / xAI）',
    en: 'Public X search: ready (via Grok / xAI)',
  },
  'settings.x.publicSearchNeedsGrok': {
    zh: '公开 X 搜索：请先连接 Grok，或使用已配置的 xAI / OpenRouter 密钥',
    en: 'Public X search: connect Grok first, or use a configured xAI / OpenRouter key',
  },
  'settings.x.connectAccountLater': {
    zh: '连接 X 账号（OAuth）即将推出 — 用于提及监控与确认后发帖。',
    en: 'Connect X account (OAuth) coming soon — for mentions and confirm-to-post.',
  },
  'settings.x.connectAccountSoon': { zh: '连接 X 账号（即将推出）', en: 'Connect X account (soon)' },
  'settings.x.connectGrokFirst': { zh: '先连接 Grok', en: 'Connect Grok first' },

  // X deep links — complement native X app (open / draft; never silent-post).
  'work.x.openInX': { zh: '在 X 中打开', en: 'Open in X' },
  'work.x.draftReplyInX': { zh: '在 X 中起草回复', en: 'Draft reply in X' },
  'work.x.complementHint': {
    zh: '在 X App 中继续 · 安琪不替代刷流',
    en: 'Continue in the X app · Aegis is not a second timeline',
  },

  // Settings — LLM provider selector + paid-API fallback (this change).
  'settings.provider.selectKicker': { zh: 'AI 服务', en: 'AI service' },
  'settings.provider.selectHeading': { zh: '选择 AI 服务商', en: 'Choose AI provider' },
  'settings.provider.selectDescription': {
    zh: '选择安琪用于聊天与思考的 AI 服务商。',
    en: 'Choose which AI provider Aegis uses for chat and reasoning.',
  },
  'settings.provider.selectLabel': { zh: 'AI 服务商', en: 'AI provider' },
  'settings.provider.option.openrouter': { zh: 'OpenRouter', en: 'OpenRouter' },
  'settings.provider.option.codex_oauth': { zh: 'Codex', en: 'Codex' },
  'settings.provider.option.xai_subscription': { zh: 'Grok', en: 'Grok' },
  'settings.connection.disconnected': { zh: '未连接', en: 'Not connected' },
  'settings.connection.pending': { zh: '待登录', en: 'Pending' },
  'settings.connection.connected': { zh: '已连接', en: 'Connected' },
  'settings.connection.activeForChat': { zh: '聊天中', en: 'In use for chat' },
  'settings.ai.heading': { zh: 'AI 与花费', en: 'AI and spending' },
  'settings.search.providerLabel': { zh: '搜索服务', en: 'Search provider' },
  'settings.search.modelLabel': { zh: '搜索模型', en: 'Search model' },
  'settings.search.option.xai_subscription': { zh: 'Grok 订阅', en: 'Grok subscription' },
  'settings.search.option.openrouter': { zh: 'OpenRouter', en: 'OpenRouter' },
  'settings.search.option.auto': { zh: '跟随对话订阅', en: 'Follow chat subscription' },
  'settings.search.option.codex_oauth': { zh: 'Codex 订阅', en: 'Codex subscription' },
  'settings.search.option.unsupported': { zh: '需重新选择', en: 'Select a service' },
  'settings.search.unsupported': { zh: '原搜索服务已不再支持。请选择并验证新的搜索服务；系统不会自动切换。', en: 'The saved search service is no longer supported. Select and verify a new service; no automatic switch was made.' },
  'settings.search.hint': {
    zh: '默认跟随对话订阅；失败时提示重试，不自动切换服务。API 对话需单独选择并验证搜索服务。选择 OpenRouter 会使用其付费搜索。',
    en: 'Search follows your chat subscription. Failures offer a retry without switching services. API chat requires a separately selected and verified search service. Selecting OpenRouter uses paid search.',
  },
  'settings.search.notSignedIn': {
    zh: 'Grok 订阅尚未登录，当前无法搜索。请在「服务」里登录 Grok 后重试。',
    en: 'Grok is not signed in; search is unavailable. Sign in under Services and retry.',
  },
  'settings.model.current': { zh: '当前模型：{model}', en: 'Current model: {model}' },
  'settings.avatar.weeklyRefresh': { zh: '自我形象每周刷新', en: 'Weekly self-image refresh' },
  'settings.avatar.needsGrok': { zh: '连接 Grok 后可生成。', en: 'Connect Grok to generate' },
  'settings.notifications.telegramStays': { zh: 'Telegram 照常提醒。', en: 'Telegram reminders stay on' },
  'settings.notifications.telegramOff': {
    zh: 'Telegram 未开启。提醒留在这个控制台。',
    en: 'Telegram is off. Reminders stay in this console.',
  },
  'settings.security.ownerSession.label': { zh: '登录会话', en: 'Sign-in' },
  'settings.provider.paidFallbackLabel': { zh: '付费 API 回退', en: 'Paid API fallback' },
  'settings.provider.paidFallbackDescription': {
    zh: '打开后，主模型失败时会改用按量计费的付费 API，以维持服务。',
    en: 'Turn on so that when the primary model fails, I fall back to the metered paid API to keep working.',
  },
  'settings.model.label': { zh: '模型', en: 'Model' },
  'settings.model.description': {
    zh: '从当前服务商的列表里选一个模型。修改即时生效，无需重启。',
    en: 'Pick a model from the current provider list. Takes effect immediately, no restart.',
  },
  'settings.model.filter': { zh: '筛选模型', en: 'Filter models' },
  'settings.model.emptyList': { zh: '这个服务商还没拿到模型列表', en: 'This provider has no model list yet' },
  'settings.model.providerDefault': { zh: '服务商默认', en: 'Provider default' },
  'settings.model.useRecommended': { zh: '使用推荐模型：{model}', en: 'Use recommended model: {model}' },
  'settings.fallback.useRecommended': { zh: '验证并启用推荐备用：{model}（API 计费）', en: 'Verify and enable recommended backup: {model} (paid API)' },
  'settings.model.providerDefaultNamed': {
    zh: '服务商默认 · {model}',
    en: 'Provider default · {model}',
  },
  'settings.fallback.label': { zh: '备用模型', en: 'Backup model' },
  'settings.fallback.validationHint': { zh: '选择模型后会验证并保存，启用主模型失败时的自动备用；API 模型会独立计费。选择“不用备用”会关闭。', en: 'Choosing a model verifies and saves it, enabling automatic backup when the primary fails. API models bill separately. Choose No backup to disable it.' },
  'settings.fallback.off': { zh: '不用备用', en: 'No backup' },
  'settings.fallback.none': { zh: '没有备用模型', en: 'No backup model' },
  'settings.fallback.chooseModel': { zh: '选择模型', en: 'Choose a model' },
  'settings.fallback.summary': { zh: '主模型失败时用：{provider} · {model}', en: 'If the primary model fails, use: {provider} · {model}' },
  'error.fallback.couldNotUpdateFallbackModel': {
    zh: '无法更新备用模型',
    en: 'Could not update backup model',
  },
  'error.fallback.couldNotUpdateWebSearch': {
    zh: '无法更新网页搜索',
    en: 'Could not update web search',
  },
  'error.fallback.couldNotLoadModelCatalog': {
    zh: '无法加载模型列表',
    en: 'Could not load the model list',
  },

  // App shell — sticky LLM-degraded banner (this change).
  'provider.degraded.title': { zh: 'AI 服务暂时不可用', en: 'AI service temporarily unavailable' },
  'provider.degraded.body': {
    zh: '⚠️ {reason}。可在设置里切换服务商，或等待恢复。',
    en: '⚠️ {reason}. You can switch provider in Settings, or wait for it to recover.',
  },
  'provider.degraded.dismiss': { zh: '忽略', en: 'Dismiss' },

  // App shell — sticky embedder-degraded banner (semantic dedup paused).
  'embedder.degraded.title': { zh: '嵌入服务降级', en: 'Embedding service degraded' },
  'embedder.degraded.body.backup': {
    zh: '⚠️ 主模型 {primary} 故障，已切换到备用模型 {backup}。语义去重暂停，可能出现重复提醒。',
    en: '⚠️ Primary model {primary} is down; serving from backup {backup}. Semantic dedup is paused, so duplicate reminders may appear.',
  },
  'embedder.degraded.body.unreachable': {
    zh: '⚠️ 主模型 {primary} 故障且无备用模型，嵌入服务不可用。语义去重暂停，可能出现重复提醒。',
    en: '⚠️ Primary model {primary} is down and no backup is configured; embeddings are unavailable. Semantic dedup is paused, so duplicate reminders may appear.',
  },
  'embedder.degraded.dismiss': { zh: '忽略', en: 'Dismiss' },

  // App shell — Telegram rail banners (incident 2026-09-15: the bot token was
  // revoked for hours and the console never said a word). The invalid-token
  // banner is an alert with the fix; the unreachable one is a softer status.
  'telegram.rail.invalidToken.title': {
    zh: 'Telegram 机器人的 token 已失效，手机端通知已停',
    en: 'The Telegram bot token is invalid; phone notifications have stopped',
  },
  'telegram.rail.invalidToken.body': {
    zh: '请在 .env 里更新 TELOXIDE_TOKEN 并重启安琪。',
    en: 'Set a new TELOXIDE_TOKEN in .env and restart Aegis.',
  },
  'telegram.rail.unreachable.title': {
    zh: 'Telegram 暂时连不上，手机端通知可能延迟',
    en: 'Telegram is unreachable; phone notifications may be delayed',
  },
  'telegram.rail.unreachable.body': {
    zh: '已持续 {minutes} 分钟，安琪会自动重试。',
    en: 'For {minutes} minutes now; Aegis keeps retrying.',
  },
  'telegram.rail.dismiss': { zh: '忽略', en: 'Dismiss' },

  // App shell — offline banner (window offline/online + navigator.onLine).
  'app.offline.banner': { zh: '已离线，等待网络恢复', en: 'Offline. Waiting for the network to come back.' },

  // Status — Telegram rail row (same grammar as the connection rows) and the
  // nightly verdict line.
  'status.telegram.label': { zh: 'Telegram 通知', en: 'Telegram notifications' },
  'status.telegram.state.unconfigured': { zh: '未配置', en: 'Not configured' },
  'status.telegram.state.starting': { zh: '正在连接', en: 'Connecting' },
  'status.telegram.state.reachable': { zh: '正常', en: 'Connected' },
  'status.telegram.state.invalid_token': { zh: 'token 已失效', en: 'Token invalid' },
  'status.telegram.state.unreachable': { zh: '连不上', en: 'Unreachable' },
  'status.telegram.reason.invalidToken': {
    zh: '手机端通知已停。请在 .env 里更新 TELOXIDE_TOKEN 并重启安琪。',
    en: 'Phone notifications have stopped. Set a new TELOXIDE_TOKEN in .env and restart Aegis.',
  },
  'status.telegram.reason.unreachable': {
    zh: '已持续 {minutes} 分钟，安琪会自动重试。',
    en: 'For {minutes} minutes now; Aegis keeps retrying.',
  },
  'status.nightly.red': {
    zh: '夜间测试 {verdict}（{sha}，{hours} 小时前）',
    en: 'Nightly test run {verdict} ({sha}, {hours} hours ago)',
  },
  'status.nightly.redRecent': {
    zh: '夜间测试 {verdict}（{sha}，不到 1 小时前）',
    en: 'Nightly test run {verdict} ({sha}, less than an hour ago)',
  },
  'status.nightly.unknownSha': { zh: '未知提交', en: 'unknown commit' },

  // Settings — summary cards + section headers.
  'settings.summary.label': { zh: '设置摘要', en: 'Settings summary' },
  'settings.summary.services': { zh: '服务', en: 'Services' },
  'settings.summary.security': { zh: '安全', en: 'Security' },
  'settings.summary.alerts': { zh: '提醒', en: 'Alerts' },
  'settings.summary.toCheck': { zh: '{count} 项待检查', en: '{count} to check' },
  'settings.summary.allSet': { zh: '已就绪', en: 'All set' },
  'settings.summary.notSet': { zh: '未设置', en: 'Not set' },
  'settings.services.kicker': { zh: '连接', en: 'Connections' },
  'settings.services.heading': { zh: '安琪可使用的服务', en: 'Services Aegis can use' },
  'settings.services.none': { zh: '暂无可用的已连接服务状态。', en: 'No connected service status is available yet.' },
  'settings.security.kicker': { zh: '安全', en: 'Security' },
  'settings.security.heading': { zh: '隐私与访问', en: 'Privacy and access' },
  'settings.notifications.kicker': { zh: '通知', en: 'Notifications' },
  'settings.notifications.heading': { zh: '安琪如何联系你', en: 'Where Aegis reaches you' },

  // Settings — projection row health labels.
  'settings.health.attention': { zh: '需要关注', en: 'Needs attention' },
  'settings.health.quiet': { zh: '安静', en: 'Quiet' },
  'settings.health.ready': { zh: '就绪', en: 'Ready' },

  // userFacingErrors — system-authored error copy (Task 7, Channel B).
  // Generic assistant failure fallback (assistantDisplayError); also the
  // {fallback} prefix that the more specific error templates interpolate.
  'error.couldNotFinish': { zh: '我没能完成这件事', en: "I couldn't finish that" },
  'error.signInAgain': {
    zh: '{fallback}：请重新登录。',
    en: '{fallback}: sign in again.',
  },
  'error.creditsExhausted': {
    zh: '{fallback}：模型额度已用尽。请充值或在设置中切换已批准的模型，然后重试。',
    en: '{fallback}: model credits are exhausted. Add credits or switch the approved model setting, then try again.',
  },
  'error.csrfExpired': {
    zh: '{fallback}：你的安全会话令牌已过期。请刷新页面后重试。',
    en: '{fallback}: your secure session token expired. Refresh the page, then retry.',
  },
  'error.mfaRequiredShort': {
    zh: '此高风险操作需要额外的所有者确认。',
    en: 'Extra owner confirmation is required for this high-risk action.',
  },
  'error.forbiddenGeneric': {
    zh: '{fallback}：当前条目或会话无法执行此操作。请刷新该条目后，重试所列操作之一。',
    en: '{fallback}: this action is not available for the current item or session. Refresh this item, then retry one of the shown actions.',
  },
  'error.staleItem': {
    zh: '此条目在操作执行前已发生变化。请重新打开它，然后选择一个可用操作。',
    en: 'This item changed before the action ran. Open it again, then choose one of the available actions.',
  },
  'error.timeout': {
    zh: '{fallback}：耗时过长。请稍后重试。',
    en: '{fallback}: this took too long. Try again in a moment.',
  },
  'error.busy': {
    zh: '{fallback}：安琪当前繁忙。请稍后重试。',
    en: '{fallback}: Aegis is busy right now. Try again shortly.',
  },
  'error.modelUnavailable': {
    zh: '{fallback}：AI 这会儿连不上，我会自动换备用模型，请过一会儿再发一次。',
    en: '{fallback}: the AI service is unreachable right now; I will switch to the backup model. Try again in a moment.',
  },
  'error.incompleteAnswer': {
    zh: '{fallback}：我没有得到完整的回复。请稍后重试。',
    en: '{fallback}: I did not get a complete answer. Try again in a moment.',
  },
  'error.tryAgain': {
    zh: '{fallback}。请重试。',
    en: '{fallback}. Try again.',
  },
  // Fetch threw / status 0: the request never reached Aegis (offline, DNS,
  // captive portal). Distinct from a 5xx, where Aegis answered but failed.
  'error.networkUnreachable': {
    zh: '{fallback}：网络连不上。请检查网络后重试。',
    en: '{fallback}: I could not connect. Check your network, then try again.',
  },
  'error.mfaRequiredBeforeContinue': {
    zh: '在安琪继续之前，需要你再确认一次。',
    en: 'Extra owner confirmation is required before Aegis can continue.',
  },
  'error.consentRequired': {
    zh: '在打开此选定详情之前，Google 需要授权。',
    en: 'Google needs permission before this selected detail can open.',
  },
  // displayError() fallback copy (Channel B). These are the localized {fallback}
  // values fed to inlineUserFacingError; the en value MUST stay byte-identical to
  // the prior hardcoded English literal so the en output is unchanged, while the
  // zh value lets the zh error templates interpolate fully-Chinese text instead
  // of mixing an English fragment into a Chinese sentence.
  'chatHome.notice.workLoadFailed': {
    zh: '工作列表暂时无法加载；聊天仍可使用。请刷新页面重试。',
    en: 'The Work list could not load. Chat is still available. Reload the page to retry.',
  },
  'error.fallback.couldNotLoadAegis': {
    zh: '无法加载 Aegis',
    en: 'Could not load Aegis',
  },
  'error.fallback.couldNotOpenDetail': {
    zh: '无法打开此详情',
    en: 'Could not open this detail',
  },
  'error.fallback.actionFailed': {
    zh: '操作失败',
    en: 'Action failed',
  },
  'error.fallback.reminderActionFailed': {
    zh: '提醒操作失败',
    en: 'Reminder action failed',
  },
  'error.fallback.avatarServerSyncFailed': {
    zh: '头像已保存到此设备，但服务器同步失败',
    en: 'Avatar saved on this device, but server sync failed',
  },
  'error.fallback.avatarAutoRefreshFailed': {
    zh: '头像自动刷新更新失败',
    en: 'Avatar auto-refresh update failed',
  },
  'error.fallback.avatarGenerationFailed': {
    zh: '安琪头像生成失败',
    en: 'Aegis avatar generation failed',
  },
  'error.fallback.couldNotPauseChecks': {
    zh: '无法暂停自动检查',
    en: 'Could not pause automatic checks',
  },
  'error.fallback.couldNotResumeChecks': {
    zh: '无法恢复自动检查',
    en: 'Could not resume automatic checks',
  },
  'error.fallback.couldNotUpdateLanguage': {
    zh: '无法更新语言',
    en: 'Could not update language',
  },
  'error.fallback.couldNotUpdateProvider': {
    zh: '无法更新 AI 提供方',
    en: 'Could not update AI provider',
  },
  'error.fallback.couldNotUpdateModel': {
    zh: '无法更新 AI 模型',
    en: 'Could not update AI model',
  },
  'error.fallback.couldNotUpdatePaidApiFallback': {
    zh: '无法更新付费 API 回退',
    en: 'Could not update paid API fallback',
  },
  'error.fallback.couldNotEnableWebPush': {
    zh: '无法启用 WebApp 通知',
    en: 'Could not enable WebApp notifications',
  },
  'error.fallback.couldNotDisableWebPush': {
    zh: '无法关闭 WebApp 通知',
    en: 'Could not turn off WebApp notifications',
  },
  'error.fallback.couldNotSendWebPushTest': {
    zh: '无法发送 WebApp 通知测试',
    en: 'Could not send WebApp notification test',
  },
  'error.fallback.couldNotStartCodexSignIn': {
    zh: '无法启动 Codex 登录',
    en: 'Could not start Codex sign-in',
  },
  'error.fallback.couldNotCheckCodexSignIn': {
    zh: '无法检查 Codex 登录',
    en: 'Could not check Codex sign-in',
  },
  'error.fallback.couldNotUseCodexForChat': {
    zh: '无法将 Codex 用于聊天',
    en: 'Could not use Codex for chat',
  },
  'error.fallback.couldNotUpdateCodexModel': {
    zh: '无法更新 Codex 模型',
    en: 'Could not update Codex model',
  },
  'error.fallback.couldNotStopUsingCodexForChat': {
    zh: '无法停止将 Codex 用于聊天',
    en: 'Could not stop using Codex for chat',
  },
  'error.fallback.couldNotStartGrokSignIn': {
    zh: '无法启动 Grok 登录',
    en: 'Could not start Grok sign-in',
  },
  'error.fallback.couldNotCheckGrokSignIn': {
    zh: '无法检查 Grok 登录',
    en: 'Could not check Grok sign-in',
  },
  'error.fallback.couldNotRevokeGrokConnection': {
    zh: '无法撤销 Grok 连接',
    en: 'Could not revoke Grok connection',
  },
  'error.fallback.couldNotUseGrokForChat': {
    zh: '无法将 Grok 用于聊天',
    en: 'Could not use Grok for chat',
  },
  'error.fallback.couldNotStopUsingGrokForChat': {
    zh: '无法停止将 Grok 用于聊天',
    en: 'Could not stop using Grok for chat',
  },
  'error.fallback.couldNotSaveSnapshot': {
    zh: '无法保存支持快照',
    en: 'Could not save support snapshot',
  },
  'error.fallback.couldNotOpenSupportDebugLease': {
    zh: '无法打开支持调试租约',
    en: 'Could not open support debug lease',
  },
  'error.fallback.couldNotRevokeSupportDebugLease': {
    zh: '无法撤销支持调试租约',
    en: 'Could not revoke support debug lease',
  },
  'error.fallback.couldNotStartBackendDebugJob': {
    zh: '无法启动后端调试任务',
    en: 'Could not start backend debug job',
  },
  // App.tsx — system-authored status messages (Channel B).
  'app.reminderUpdated': { zh: '提醒已更新。', en: 'Reminder updated.' },
  // Chat and work action receipts. Built from the action kind the API
  // returns (complete / snooze / delete), never from the English ledger summary.
  'app.taskAction.complete': { zh: '已完成。', en: 'Marked complete.' },
  'app.taskAction.completeLoop': {
    zh: '已完成这一次。这条提醒还会继续。',
    en: 'Marked this occurrence done. This reminder stays on the schedule.',
  },
  'app.taskAction.archived': { zh: '这条提醒已删除。', en: 'This reminder was deleted.' },
  'app.taskAction.snoozed': { zh: '已改到 {due}。', en: 'Snoozed until {due}.' },
  'app.checksPaused': {
    zh: '自动检查已暂停。聊天、工作操作以及你主动请求的检查仍会运行。',
    en: 'Automatic checks are paused. Chat, Work actions, and checks requested by you still run.',
  },
  'app.snapshotSaved': {
    zh: '已保存支持快照 {id}。它将自动过期。',
    en: 'Saved support snapshot {id}. It expires automatically.',
  },
  // App.tsx — owner-facing snapshot-toast status strings (Channel B). Set via
  // setSnapshotMessage and rendered in the vnext-snapshot-toast <p>. These are
  // the avatar/cost-saving/support-debug toasts the 2026-06-09 shell sweep
  // initially missed (they are setter-call literals, not JSX text).
  'app.avatarRefreshEnabled': {
    zh: '安琪的每周头像刷新已开启。',
    en: 'Aegis weekly avatar refresh enabled.',
  },
  'app.avatarRefreshPaused': {
    zh: '安琪的每周头像刷新已暂停。',
    en: 'Aegis weekly avatar refresh paused.',
  },
  'app.avatarGenerated': {
    zh: '安琪的新头像已生成并保存。',
    en: 'Aegis avatar was generated and saved to the server profile.',
  },
  'app.avatarGeneratedReconciled': {
    zh: '安琪头像已生成并保存。确认响应被中断，因此我刷新了服务器上的资料。',
    en: 'Aegis avatar was generated and saved. The confirmation response was interrupted, so I refreshed the server profile.',
  },
  'app.checksResumed': {
    zh: '自动检查现在可以在你保存的设置下重新运行。',
    en: 'Automatic checks can run again under your saved settings.',
  },
  'app.snapshotNotEnabled': {
    zh: '此服务器未启用支持快照。',
    en: 'Support snapshots are not enabled on this server.',
  },
  'app.debugLeaseNotEnabled': {
    zh: '此服务器未启用支持调试租约。',
    en: 'Support debug leases are not enabled on this server.',
  },
  'app.debugJobNotEnabled': {
    zh: '此服务器未启用支持调试任务。',
    en: 'Support debug jobs are not enabled on this server.',
  },
  // {owner} is a verbatim server-provided owner_message; only the lease label
  // is localized. Keep the leading "{owner} " so en stays byte-identical.
  'app.debugLeaseOpened': {
    zh: '{owner} 租约 {id}。',
    en: '{owner} Lease {id}.',
  },
  'app.debugLeaseRevoked': {
    zh: '调试租约 {id} 已撤销。',
    en: 'Debug lease {id} revoked.',
  },
  'app.debugJobStarted': {
    zh: '调试任务 {id} 已启动。',
    en: 'Debug job {id} started.',
  },

  // App.tsx — FE-authored receipt_expectation values (Channel B, i18n receipt
  // tail). Rendered by AssistantResponseView's receipt line. en values stay
  // byte-identical to the prior hardcoded literals.
  'app.receipt.workActionNoExternal': {
    zh: '除非此回复附带记录，否则没有执行任何额外的外部操作。',
    en: 'No additional external action ran unless this reply includes a receipt.',
  },
  'app.receipt.noActionTaken': {
    zh: '未执行任何操作。',
    en: 'No action was taken.',
  },

  // App.tsx — action `{ok, message}` results + work-action chat context label
  // (Channel B). Owner-facing messages surfaced inline on Work/Chat Home.
  'app.action.signedOutBeforeFinished': {
    zh: '操作完成前已退出登录。',
    en: 'Signed out before this finished.',
  },
  'app.action.sentToAegis': {
    zh: '已发送给安琪。',
    en: 'Sent to Aegis.',
  },
  // Also matched (en + zh, exact catalog values) by ChatHomeView's
  // isStaleWorkRefreshMessage() to pick the 'hint' presentation — keep that
  // predicate in sync when editing this entry.
  'app.action.itemChangedRefreshedQueue': {
    zh: '此条目在操作执行前已发生变化。我已刷新工作队列；请从当前可用操作继续。',
    en: 'This item changed before the action ran. I refreshed the work queue; continue from the current available actions.',
  },
  'app.action.workContextLabel': {
    zh: '工作：{title}',
    en: 'Work: {title}',
  },
  'app.action.workContextFallback': {
    zh: '工作操作',
    en: 'Work action',
  },
  'app.action.secretaryDone': {
    zh: '好，这件事我已按你的决定处理完毕。',
    en: 'Done — I handled that according to your decision.',
  },
  'app.action.secretaryDoneWithDetail': {
    zh: '好，已处理：{detail}',
    en: 'Done: {detail}',
  },
  // CEO voice assistant (composer + floating orb).
  'voice.notice.listening': { zh: '在听…', en: 'Listening…' },
  'voice.notice.captured': { zh: '已识别，可检查后发送。', en: 'Voice captured. Review or send.' },
  'voice.notice.capturedSending': { zh: '已识别并发送。', en: 'Voice captured — sending.' },
  'voice.notice.unavailable': {
    zh: '当前浏览器不支持语音输入。先打字即可，没有打开麦克风。',
    en: 'Voice input is not available in this browser. Type here for now; no microphone was opened.',
  },
  'voice.notice.error': {
    zh: '语音已停止{reason}。可再试或打字。',
    en: 'Voice input stopped{reason}. Type here or try again.',
  },
  'voice.notice.empty': {
    zh: '没有听清。再试一次或打字。',
    en: 'Voice ended without text. Try again or type here.',
  },
  'voice.notice.startFailed': {
    zh: '无法开始语音。先打字即可，没有发送任何指令。',
    en: 'Voice input could not start. Type here for now; no command was sent.',
  },
  'voice.orb.start': { zh: '召唤安琪', en: 'Summon Aegis' },
  'voice.orb.stop': { zh: '结束通话', en: 'End call' },
  'voice.orb.listening': { zh: '在听你说', en: 'Listening' },
  'voice.orb.hearingYou': { zh: '听到了…', en: 'Hearing you…' },
  'voice.orb.speaking': { zh: '安琪在说', en: 'Aegis speaking' },
  'voice.orb.connecting': { zh: '正在接通…', en: 'Connecting…' },
  'voice.orb.live': { zh: '全双工通话中 · 直接说', en: 'Live duplex — just speak' },
  'voice.orb.hint': {
    zh: '点安琪头像开始全双工语音。通话中再点一次结束。',
    en: 'Tap the Aegis avatar for full-duplex voice. Tap again to hang up.',
  },
  'voice.orb.sessionCap': { zh: '本轮语音已到上限，已挂断。', en: 'Voice session reached its cap and ended.' },
  'voice.orb.idleHangup': { zh: '一段时间没说话，已挂断省费用。', en: 'No speech for a while — hung up to save cost.' },
  'voice.orb.realtimeFailed': {
    zh: '语音没接通。再点一次头像重试，或先用文字。',
    en: 'Voice did not connect. Tap the avatar to retry, or use text for now.',
  },
  'voice.orb.realtimeFailedShort': { zh: '语音没接通', en: 'Voice did not connect' },
  'voice.orb.failMicDenied': {
    zh: '麦克风没有权限：在系统设置里允许这个网页使用麦克风，再点头像。',
    en: 'Microphone permission is off: allow it for this site in system settings, then tap the avatar.',
  },
  'voice.orb.failCsp': {
    zh: '浏览器拦截了语音通道，硬刷新页面后再试。',
    en: 'The browser blocked the voice channel; hard-refresh the page and retry.',
  },
  'voice.orb.realtimeSoon': {
    zh: '点头像进入全双工；文字请用「和安琪说」。',
    en: 'Tap the avatar for duplex; use Talk to Aegis for text.',
  },
  'voice.panel.title': { zh: '安琪', en: 'Aegis' },
  'voice.panel.close': { zh: '关闭面板', en: 'Close panel' },
  'voice.panel.tabVoice': { zh: '语音', en: 'Voice' },
  'voice.panel.empty': {
    zh: '直接说。我能处理当前屏幕上的内容。',
    en: 'Just speak. I can act on what’s on screen.',
  },
  'voice.panel.voiceHintRealtime': {
    zh: '点开始进入全双工对话（可打断）。',
    en: 'Start for full-duplex conversation (barge-in supported).',
  },
  'voice.panel.working': { zh: '安琪处理中…', en: 'Aegis is working…' },
  'voice.panel.queued': { zh: '已排队，当前对话结束后会处理。', en: 'Queued — will run after the current turn.' },
  'voice.panel.replyEmpty': { zh: '已处理，但没有可显示的回复。', en: 'Done, but no displayable reply.' },
  'voice.panel.replyFailed': { zh: '这次没能完成，请再试一次。', en: 'Could not finish that — try again.' },
  'voice.panel.needsConfirm': { zh: '需要你确认', en: 'Needs your confirmation' },
  'voice.panel.approve': { zh: '批准', en: 'Approve' },
  'voice.panel.reject': { zh: '先不要', en: 'Not now' },
  'voice.panel.approved': { zh: '已批准', en: 'Approved' },
  'voice.panel.rejected': { zh: '已取消', en: 'Cancelled' },
  // Live duplex captions (compact strip above the orb — not the full text panel).
  'voice.captions.label': { zh: '通话字幕', en: 'Call captions' },
  'voice.captions.you': { zh: '你', en: 'You' },
  'voice.captions.aegis': { zh: '安琪', en: 'Aegis' },

  // App.tsx — sourceDetailPayloadFromOpenError (module-level pure fn; locale is
  // threaded in as a param). Rendered via buildSourceDetailView's unavailable
  // branch: title / subtitle(message) / "Next" field(next_step).
  'app.sourceDetail.changedTitle': {
    zh: '此详情已变化',
    en: 'This detail changed',
  },
  'app.sourceDetail.changedMessage': {
    zh: '此工作项已发生变化，或其底层来源已不可用。请刷新「工作」以加载当前版本后再做决定。',
    en: 'This work item changed or the underlying source is no longer available. Refresh Work to load the current version before deciding.',
  },
  'app.sourceDetail.changedNextStep': {
    zh: '刷新「工作」，然后重新打开该条目。',
    en: 'Refresh Work, then reopen the item.',
  },

  // App.tsx — chatHomeNotice channel (Channel B). Owner-facing notices rendered
  // on the home view via `chatHomeNotice || snapshotMessage`. Set via
  // setChatHomeNotice. en values byte-identical to the prior literals.
  'chatHome.notice.needsApprovalDecision': {
    zh: '安琪需要你做决定。打开「工作」查看这次批准。',
    en: 'Aegis needs a decision. Open Work to review the approval.',
  },
  'chatHome.notice.installed': {
    zh: '安琪已安装到此设备。',
    en: 'Aegis is installed on this device.',
  },
  'chatHome.notice.alreadyInstalled': {
    zh: '安琪已经安装在此设备上。',
    en: 'Aegis is already installed on this device.',
  },
  'chatHome.notice.queued': {
    zh: '已排队。安琪将接着回答它。',
    en: 'Queued. Aegis will answer it next.',
  },
  'chatHome.notice.queuedWork': {
    zh: '已在该工作项上排队。安琪将接着回答它。',
    en: 'Queued on that work item. Aegis will answer it next.',
  },
  'chatHome.notice.queueFullCommands': {
    zh: '安琪已有 {max} 条排队。此草稿仍保留在这里。',
    en: 'Aegis already has {max} queued. This draft is still here.',
  },
  'chatHome.notice.queueFullWork': {
    zh: '安琪已有 {max} 个工作项提问排队。请在当前回答出现后再试。',
    en: 'Aegis already has {max} item questions queued. Try again after the current answer appears.',
  },
  'chatHome.notice.chatCleared': {
    zh: '已在此浏览器清除聊天。',
    en: 'Chat cleared on this browser.',
  },

  // installPrompt.ts — PWA install action label + manual/outcome notices
  // (Channel B). Localized via a `locale` param threaded into the pure helpers.
  'install.action.installed': { zh: '已安装到此设备', en: 'Installed on this device' },
  'install.action.installOnDevice': { zh: '安装到此设备', en: 'Install on this device' },
  'install.action.available': { zh: '安装应用', en: 'Install app' },
  'install.action.manual': { zh: '添加到主屏幕', en: 'Add to Home Screen' },
  'install.manual.ios': {
    zh: '使用「分享」，然后「添加到主屏幕」。Aegis 将像一个私人应用一样在此设备上打开。',
    en: 'Use Share, then Add to Home Screen. Aegis will open like a private app on this device.',
  },
  'install.manual.generic': {
    zh: '使用你的浏览器菜单来安装 Aegis 或将其添加到主屏幕。',
    en: 'Use your browser menu to install Aegis or add it to the home screen.',
  },
  'install.outcome.accepted': {
    zh: 'Aegis 安装已开始。浏览器完成后，它将从你的主屏幕打开。',
    en: 'Aegis install started. It will open from your home screen once the browser finishes.',
  },
  'install.outcome.dismissed': {
    zh: '已取消安装。你之后可以从此菜单添加 Aegis。',
    en: 'Install dismissed. You can add Aegis from this menu later.',
  },

  // App.tsx — app-shell / chrome layer (auth + loading screens, brand rail,
  // header, nav, badges). Final i18n tail (2026-06-09 audit). Rendered in the
  // VNextApp body via t()/tf() with `errorLocale`.
  // -- CenterState auth/loading screens --
  'app.shell.loading.kicker': { zh: '正在打开', en: 'Opening' },
  'app.shell.loading.title': { zh: '正在加载 Aegis', en: 'Loading Aegis' },
  'app.shell.loading.body': {
    zh: '正在检查你的会话并准备你的工作区。',
    en: 'Checking your session and preparing the owner workspace.',
  },
  'app.shell.error.kicker': { zh: '连接', en: 'Connection' },
  'app.shell.error.title': { zh: '无法打开 Aegis', en: 'Could not open Aegis' },
  'app.shell.error.retry': { zh: '重试', en: 'Retry' },
  'app.shell.auth.kicker': { zh: '私人秘书', en: 'Private secretary' },
  'app.shell.auth.title': { zh: '登录 Aegis', en: 'Sign in to Aegis' },
  'app.shell.auth.body': {
    zh: '使用已配对的通行密钥或你已启用的登录方式，进入私人工作区。',
    en: 'Use a paired passkey or a sign-in method you have enabled to enter your private workspace.',
  },
  'app.shell.auth.localBody': {
    zh: '请从这台电脑上的 Aegis 启动入口打开一次性登录链接。无需配置 Google 账户。链接已使用或失效时，可从启动入口重新获取。',
    en: 'Open the one-time sign-in link from Aegis on this computer. No Google account setup is needed. If the link was used or expired, get a fresh one from the launcher.',
  },
  'app.shell.auth.continueWithGoogle': { zh: '使用 Google 继续', en: 'Continue with Google' },
  // -- skip link + brand rail --
  'app.shell.skipToMain': { zh: '跳到主要内容', en: 'Skip to main content' },
  'app.shell.ownerNavigation': { zh: 'Aegis 导航', en: 'Aegis navigation' },
  'app.shell.brand': { zh: 'Aegis', en: 'Aegis' },
  'app.shell.tagline.secretary': { zh: '你的私人秘书。', en: 'Your private secretary.' },
  'app.shell.tagline.timeProtected': { zh: '守护你的时间。', en: 'Your time, protected.' },
  'app.shell.brandCopy': {
    zh: '整理需要关注的待办，在重要变更前先征求你的同意，并保留记录。',
    en: 'Organizes what needs attention, asks before important changes, and keeps receipts.',
  },
  'app.shell.nav.primaryDesktop': { zh: '主导航（桌面）', en: 'Primary desktop' },
  'app.shell.nav.primary': { zh: '主导航', en: 'Primary' },
  'app.shell.nav.aegis': { zh: '安琪', en: 'Aegis' },
  // Home rail label = Aegis (system persona), not a separate "Chat app".
  'app.shell.nav.chat': { zh: '安琪', en: 'Aegis' },
  'app.shell.nav.work': { zh: '工作', en: 'Work' },
  'app.shell.ownerCard.label': { zh: '已登录账户', en: 'Signed-in account' },
  'app.shell.ownerCard.youFallback': { zh: '你', en: 'You' },
  'app.shell.ownerCard.signedIn': { zh: '已登录', en: 'Signed in' },
  // -- header --
  'app.shell.header.kicker': { zh: '私人秘书', en: 'Private secretary' },
  'app.shell.header.menu': { zh: '打开菜单', en: 'Open menu' },
  'app.shell.header.openStatus': { zh: '打开 Aegis 状态', en: 'Open Aegis status' },
  'app.shell.header.status': { zh: '状态', en: 'Status' },
  'app.shell.header.openSettings': { zh: '打开设置', en: 'Open settings' },
  'app.shell.header.settings': { zh: '设置', en: 'Settings' },
  'app.shell.header.saveSnapshot': { zh: '保存支持快照', en: 'Save support snapshot' },
  'app.shell.header.saving': { zh: '保存中', en: 'Saving' },
  'app.shell.header.snapshot': { zh: '快照', en: 'Snapshot' },
  // -- Web Push device status (App.tsx owner-facing messages) --
  'app.webPush.notConfigured': {
    zh: '此设备尚未配置 Web 应用通知。',
    en: 'WebApp notifications are not configured yet.',
  },
  'app.webPush.enabled': {
    zh: '此设备已开启 Web 应用通知。',
    en: 'WebApp notifications are enabled on this device.',
  },
  'app.webPush.disabled': {
    zh: '此设备已关闭 Web 应用通知。',
    en: 'WebApp notifications are off on this device.',
  },
  // -- Work badge aria (pluralized). Chinese has no plural; just embed count. --
  'app.shell.workBadge.decisionsSingular': {
    zh: '工作，{count} 个决策待处理',
    en: 'Work, {count} decision waiting',
  },
  'app.shell.workBadge.decisionsPlural': {
    zh: '工作，{count} 个决策待处理',
    en: 'Work, {count} decisions waiting',
  },
  'app.shell.workBadge.readySingular': {
    zh: '工作，{count} 项已就绪',
    en: 'Work, {count} item ready',
  },
  'app.shell.workBadge.readyPlural': {
    zh: '工作，{count} 项已就绪',
    en: 'Work, {count} items ready',
  },
  'app.shell.workBadge.plain': { zh: '工作', en: 'Work' },
  // -- Plan C: tab-signal aria labels --
  /** Chat tab: aria-label override when the unseen-activity dot is lit. */
  'app.shell.nav.chatUnseenAria': { zh: '安琪，有新消息', en: 'Aegis — new message' },
  // -- header surface titles + subtitles (per active view) --
  'app.shell.surface.work.title': { zh: '工作', en: 'Work' },
  'app.shell.surface.status.title': { zh: '状态', en: 'Status' },
  'app.shell.surface.settings.title': { zh: '设置', en: 'Settings' },
  'app.shell.surface.home.title': { zh: '安琪', en: 'Aegis' },
  'app.shell.surface.work.subtitle': {
    zh: '只放需要你的事。',
    en: 'Only what needs you.',
  },
  'app.shell.surface.status.subtitle': {
    zh: '需要你时再看，其它我安静处理。',
    en: 'Only what needs you — I handle the rest quietly.',
  },
  'app.shell.surface.settings.subtitle': {
    zh: '你、语言、通知、连接。',
    en: 'You, language, notifications, connections.',
  },
  'app.shell.surface.home.subtitle': {
    zh: '告诉安琪你需要什么。',
    en: 'Tell Aegis what you need.',
  },
  // -- RichText inline/block image alt fallback (rendered via RichText's own tr) --
  'app.richText.generatedImageAlt': { zh: '生成的图片', en: 'Generated image' },

  // Pattern that re-emits the backend-provided message after the fallback.
  'error.withDetail': {
    zh: '{fallback}：{detail}',
    en: '{fallback}: {detail}',
  },

  // ChatHomeView — owner-facing chat home surface (Task 8).
  // Starter prompts (visible labels; the prompt text sent to Aegis stays raw).
  'chatHome.starter.commute': { zh: '通勤', en: 'Commute' },
  'chatHome.starter.tasks': { zh: '任务', en: 'Tasks' },
  'chatHome.starter.focus': { zh: '关注', en: 'Focus' },
  // Composer size labels.
  'chatHome.composer.singleLine': { zh: '单行', en: 'Single line' },
  'chatHome.composer.halfScreen': { zh: '半屏', en: 'Half screen' },
  'chatHome.composer.size': { zh: '输入框大小：{mode}', en: 'Composer size: {mode}' },
  // Action-oriented size labels (what the next tap does) — no cryptic 1L glyph.
  'chatHome.composer.expand': { zh: '展开输入框', en: 'Expand composer' },
  'chatHome.composer.collapse': { zh: '收起为单行', en: 'Collapse to one line' },
  // Roles / authors.
  'chatHome.role.aegis': { zh: '{assistant}', en: '{assistant}' },
  'chatHome.role.you': { zh: '你', en: 'You' },
  'chatHome.role.queued': { zh: '排队中', en: 'Queued' },
  'chatHome.role.earlier': { zh: '更早', en: 'Earlier' },
  // MessageTime accessibility.
  'chatHome.time.showFull': { zh: '显示完整时间 {time}', en: 'Show full time {time}' },
  'chatHome.date.today': { zh: '今天', en: 'Today' },
  'chatHome.date.yesterday': { zh: '昨天', en: 'Yesterday' },
  'chatHome.time.hideFull': { zh: '隐藏完整时间 {time}', en: 'Hide full time {time}' },
  'chatHome.latency.compact': { zh: '{seconds} 秒', en: '{seconds}s' },
  'chatHome.latency.full': { zh: '从收到到回复 {seconds} 秒', en: 'Received to reply in {seconds}s' },
  'chatHome.message.copyRef': { zh: '复制引用 {ref}', en: 'Copy reference {ref}' },
  'chatHome.message.copiedRef': { zh: '已复制 {ref}', en: 'Copied {ref}' },
  'chatHome.message.refCopied': { zh: '已复制', en: 'Copied' },
  'chatHome.citeFocus.outsideWindow': {
    zh: '已打开对话。引用 {ref} 不在当前加载的消息里。',
    en: 'Chat is open. {ref} is not in the messages loaded here.',
  },
  'chatHome.alert.repeated': { zh: '已重复 {count} 次', en: 'Repeated {count} times' },
  'chatHome.alert.repeatedSince': { zh: '已重复 {count} 次，首次 {first}', en: 'Repeated {count} times since {first}' },
  // Collapsed notice badge.
  'chatHome.collapsed.earlierLabel': { zh: '更早的{label}', en: 'Earlier {label}' },
  // Quote preview.
  'chatHome.quote.replyingTo': { zh: '回复 {role}', en: 'Replying to {role}' },
  // Topbar / account menu.
  'chatHome.kicker.privateSecretary': { zh: '私人秘书', en: 'Private secretary' },
  'chatHome.account.menu': { zh: '账户菜单', en: 'Account menu' },
  'chatHome.account.label': { zh: '账户', en: 'Account' },
  'chatHome.account.signedIn': { zh: '已登录', en: 'Signed in' },
  'chatHome.account.saving': { zh: '保存中', en: 'Saving' },
  'chatHome.account.savingSnapshot': { zh: '正在保存…', en: 'Saving…' },
  'chatHome.account.saveSnapshot': { zh: '保存支持快照', en: 'Save support snapshot' },
  'chatHome.account.updating': { zh: '更新中', en: 'Updating' },
  'chatHome.account.resumeChecks': { zh: '恢复自动检查', en: 'Resume automatic checks' },
  'chatHome.account.pauseChecks': { zh: '暂停自动检查', en: 'Pause automatic checks' },
  'chatHome.account.aegisStatus': { zh: '运行状态', en: 'Runtime status' },
  'chatHome.account.settings': { zh: '设置', en: 'Settings' },
  'chatHome.account.clearChat': { zh: '清空对话', en: 'Clear chat' },
  'chatHome.account.signingOut': { zh: '退出中', en: 'Signing out' },
  'chatHome.account.signOut': { zh: '退出登录', en: 'Sign out' },
  // Session row.
  'chatHome.session.working': { zh: '{assistant}正在处理', en: '{assistant} is working' },
  'chatHome.session.signedIn': { zh: '{name} 已登录', en: '{name} signed in' },
  'chatHome.session.snapshot': { zh: '快照', en: 'Snapshot' },
  'chatHome.session.clear': { zh: '清空', en: 'Clear' },
  'chatHome.session.work': { zh: '工作', en: 'Work' },
  // Section / shell.
  'chatHome.shell.chatHome': { zh: '安琪桌面', en: 'Aegis desk' },
  'chatHome.sheet.open': { zh: '和{assistant}说', en: 'Talk to {assistant}' },
  'chatHome.sheet.openAria': { zh: '打开和{assistant}的对话', en: 'Open chat with {assistant}' },
  'chatHome.sheet.closeAria': { zh: '关闭对话', en: 'Close chat' },
  'chatHome.sheet.handleAria': { zh: '拖动或点按以关闭对话', en: 'Drag or tap to close chat' },
  // Hero / empty stage.
  'chatHome.hero.ready': { zh: '就绪', en: 'Ready' },
  'chatHome.hero.tellAegis': { zh: '告诉{assistant}。', en: 'Tell {assistant}.' },
  'chatHome.hero.openBrief': { zh: '打开当前安琪简报', en: 'Open current Aegis brief' },
  'chatHome.hero.starterPrompts': { zh: '快捷提示', en: 'Starter prompts' },
  'chatHome.destinations.nav': { zh: '常用页面', en: 'Everyday destinations' },
  'chatHome.destinations.work': { zh: '工作', en: 'Work' },
  'chatHome.destinations.workHint': { zh: '查看需要你的待办', en: 'What needs you' },
  // CEO home: one elegant peer path is enough; empty hero elevates "needs you".
  'chatHome.hero.hint': {
    zh: '一句话就行。重要的事会先给你确认。',
    en: 'One sentence is enough. Important steps wait for your review.',
  },
  'chatHome.needsYou.kicker': { zh: '需要你', en: 'Needs you' },
  'chatHome.jumpLatest': { zh: '回到最新', en: 'Back to latest' },
  'chatHome.loadEarlier.label': { zh: '加载更早的对话', en: 'Load earlier messages' },
  'chatHome.loadEarlier.loading': { zh: '正在加载…', en: 'Loading…' },
  'chatHome.loadEarlier.failed': { zh: '更早的对话暂时未能加载，现有消息已保留。请再次点击“加载更早的对话”。', en: 'Earlier messages could not load. Your current messages are still here. Select “Load earlier messages” to retry.' },
  'chatHome.needsYou.title': { zh: '先看这些', en: 'Start here' },
  'chatHome.needsYou.cardsLabel': { zh: '可直接处理的待办', en: 'Items you can handle here' },
  'chatHome.needsYou.done': { zh: '已处理。', en: 'Done.' },
  'chatHome.needsYou.moreOne': { zh: '还有 1 项 — 打开工作', en: '1 more — open Work' },
  'chatHome.needsYou.moreMany': { zh: '还有 {count} 项 — 打开工作', en: '{count} more — open Work' },
  // Single dismissible banner above the composer (UX pass 2026-09-07).
  // The kicker renders as its own chip; the title no longer sits behind a
  // second colon (server titles like 没做成：通知 already carry one).
  'chatHome.needsYou.banner': {
    zh: '{title}',
    en: '{title}',
  },
  'chatHome.needsYou.handle': { zh: '去处理', en: 'Handle' },
  'chatHome.needsYou.later': { zh: '稍后', en: 'Later' },
  'chatHome.needsYou.moreOnWorkOne': {
    zh: '还有 1 件在工作页',
    en: '1 more on Work',
  },
  'chatHome.needsYou.moreOnWorkMany': {
    zh: '还有 {count} 件在工作页',
    en: '{count} more on Work',
  },
  'chatHome.needsYou.bannerAria': { zh: '需要你处理', en: 'Needs your attention' },
  'chatHome.needsYou.dismissAria': { zh: '稍后处理', en: 'Dismiss for later' },
  'chatHome.intent.understood': { zh: '我理解成：{label}', en: 'I take this as: {label}' },
  'chatHome.intent.cueAria': { zh: '意图理解', en: 'Understood intent' },
  'chatHome.intent.reviewHint': { zh: '重要步骤会先请你确认', en: 'Important steps will ask you first' },
  'chatHome.working.intent': { zh: '我理解成：{label}…', en: 'I take this as: {label}…' },
  'chatHome.working.intentReview': {
    zh: '我理解成：{label}（重要步骤会先请你确认）…',
    en: 'I take this as: {label} (important steps will ask you first)…',
  },
  // CEO shorthand labels + expansions (docs/design/2026-07-25-ceo-intent-contract.md).
  'ceoIntent.shorthand.commute.label': { zh: '查看上班路况与出发建议', en: 'Commute and leave-by advice' },
  'ceoIntent.shorthand.commute.expansion': {
    zh: '看一下我现在的通勤路况，并建议我最晚什么时候出发。',
    en: 'Check my commute conditions and recommend when I should leave.',
  },
  'ceoIntent.shorthand.now.label': { zh: '现在最需要我拍板的事', en: 'What needs me now' },
  'ceoIntent.shorthand.now.expansion': {
    zh: '现在最需要我关注或拍板的是什么？只列最重要的几件。',
    en: 'What needs my attention or decision right now? Only the most important items.',
  },
  'ceoIntent.shorthand.lawyer.label': { zh: '与律师/法务相关的待办', en: 'Lawyer / legal items' },
  'ceoIntent.shorthand.lawyer.expansion': {
    zh: '找出与律师或法务相关的待办，给我简要情况和下一步建议。',
    en: 'Find lawyer or legal-related work, then brief me with next steps.',
  },
  'ceoIntent.shorthand.billing.label': { zh: '账单与付款相关', en: 'Bills and payments' },
  'ceoIntent.shorthand.billing.expansion': {
    zh: '汇总需要我看的账单或付款待办，写清金额和截止日期；不要自动付款。',
    en: 'Summarize bills or payment items that need me, with amounts and due dates. Do not pay anything.',
  },
  'ceoIntent.shorthand.reply.label': { zh: '起草回复（先不发送）', en: 'Draft a reply (do not send)' },
  'ceoIntent.shorthand.reply.expansion': {
    zh: '针对当前焦点或最近需要回复的那封信，起草一封回复给我审阅，先不要发送。',
    en: 'Draft a reply for the focused or most recent message that needs a response. Do not send it yet.',
  },
  'ceoIntent.shorthand.approve.label': { zh: '批准当前待确认的待办', en: 'Approve the pending review' },
  'ceoIntent.shorthand.approve.expansion': {
    zh: '批准我当前待确认的那一项（若有多项，先说明是哪一项）。',
    en: 'Approve the item currently waiting for my review (if several, say which one first).',
  },
  'ceoIntent.shorthand.reject.label': { zh: '拒绝当前待确认的待办', en: 'Reject the pending review' },
  'ceoIntent.shorthand.reject.expansion': {
    zh: '拒绝或取消我当前待确认的那一项，不要继续执行。',
    en: 'Reject or cancel the item currently waiting for my review. Do not proceed.',
  },
  'ceoIntent.shorthand.draft.label': { zh: '查看或继续草稿', en: 'Show or continue draft' },
  'ceoIntent.shorthand.draft.expansion': {
    zh: '打开或继续当前相关的草稿，给我审阅，先不要发送。',
    en: 'Open or continue the relevant draft for my review. Do not send yet.',
  },
  'ceoIntent.shorthand.watch.label': { zh: '盯着当前这件事', en: 'Watch this for me' },
  'ceoIntent.shorthand.watch.expansion': {
    zh: '盯着当前这件事的后续变化，有需要我拍板时再叫我。',
    en: 'Watch the current item for follow-up changes and only pull me in when a decision is needed.',
  },
  'ceoIntent.shorthand.today.label': { zh: '今日简报', en: 'Today brief' },
  'ceoIntent.shorthand.today.expansion': {
    zh: '给我今天的简报：日程、需要我的事、以及你已安静处理的部分。',
    en: 'Give me today\'s brief: schedule, what needs me, and what you already handled quietly.',
  },
  'ceoIntent.shorthand.archive.label': { zh: '归档这件事', en: 'Archive the focused item' },
  'ceoIntent.shorthand.archive.expansion': {
    zh: '归档当前焦点或我刚提到的这件事，并留下记录。',
    en: 'Archive the focused or just-mentioned item and leave a receipt.',
  },
  'ceoIntent.shorthand.send.label': { zh: '发送（需确认）', en: 'Send (needs confirmation)' },
  'ceoIntent.shorthand.send.expansion': {
    zh: '准备发送当前草稿或回复；发送前必须先给我确认。',
    en: 'Prepare to send the current draft or reply. You must get my confirmation before sending.',
  },
  'ceoIntent.shorthand.delete.label': { zh: '删除（需确认）', en: 'Delete (needs confirmation)' },
  'ceoIntent.shorthand.delete.expansion': {
    zh: '准备删除当前焦点项；删除前必须先给我确认，说明是否可恢复。',
    en: 'Prepare to delete the focused item. Confirm with me first and say whether it is recoverable.',
  },
  'ceoIntent.shorthand.handle.label': { zh: '帮我处理待办', en: 'Handle open loops' },
  'ceoIntent.shorthand.handle.expansion': {
    zh: '帮我处理当前待办：能安静办的先办，需要我拍板的列出来。',
    en: 'Handle my open loops: do what you can quietly, and list only what needs my decision.',
  },
  'ceoIntent.shorthand.quiet.label': { zh: '减少主动打扰', en: 'Quiet mode' },
  'ceoIntent.shorthand.quiet.expansion': {
    zh: '暂时减少主动打扰和自启动检查；告诉我如何恢复。',
    en: 'Reduce proactive pings and self-started checks for now, and tell me how to resume.',
  },
  // Brief card labels (compactHomeBrief).
  'chatHome.brief.decisionWaiting': { zh: '等待决定', en: 'Decision waiting' },
  'chatHome.brief.startHere': { zh: '从这里开始', en: 'Start here' },
  'chatHome.brief.readyToReview': { zh: '可以查看', en: 'Ready to review' },
  'chatHome.brief.quiet': { zh: '平静', en: 'Quiet' },
  'chatHome.brief.quietRightNow': { zh: '当前很平静', en: 'Quiet right now' },
  'chatHome.brief.firstLook': { zh: '初看', en: 'First look' },
  // Context strip.
  'chatHome.context.attachable': { zh: '可附加的上下文', en: 'Attachable context' },
  'chatHome.context.active': { zh: '当前上下文', en: 'Active context' },
  'chatHome.context.openMoreOne': {
    zh: '打开另外 {count} 个工作项',
    en: 'Open {count} more work item',
  },
  'chatHome.context.openMoreMany': {
    zh: '打开另外 {count} 个工作项',
    en: 'Open {count} more work items',
  },
  'chatHome.context.workItemsOne': { zh: '{count} 个工作项', en: '{count} item' },
  'chatHome.context.workItemsMany': { zh: '{count} 个工作项', en: '{count} items' },
  'chatHome.context.desktop': { zh: '桌面端上下文', en: 'Desktop context' },
  'chatHome.context.contextKicker': { zh: '上下文', en: 'Context' },
  'chatHome.context.nearbyWork': { zh: '附近的工作', en: 'Nearby work' },
  'chatHome.context.attachKicker': { zh: '附加上下文', en: 'Attach context' },
  'chatHome.context.attachToChat': { zh: '为此对话附加上下文', en: 'Attach context to this chat' },
  'chatHome.context.chooseConsider': {
    zh: '选择安琪应当考虑的内容',
    en: 'Choose what Aegis should consider',
  },
  'chatHome.context.noneWaiting': {
    zh: '附近没有等待中的工作。你仍然可以直接询问安琪。',
    en: 'No nearby work is waiting. You can still ask Aegis directly.',
  },
  'chatHome.context.close': { zh: '关闭', en: 'Close' },
  // Messages.
  'chatHome.message.replyToMessage': { zh: '回复这条消息', en: 'Reply to this message' },
  'chatHome.message.using': { zh: '使用 {label}', en: 'Using {label}' },
  'chatHome.message.replyToAnswer': { zh: '回复{assistant}这条回答', en: 'Reply to this answer from {assistant}' },
  // Retry button.
  'chatHome.retry.queueCheck': { zh: '排队检查', en: 'Queue check' },
  'chatHome.retry.queueRetry': { zh: '排队重试', en: 'Queue retry' },
  'chatHome.retry.checkResult': { zh: '查看结果', en: 'Check result' },
  'chatHome.retry.tryAgain': { zh: '重试', en: 'Try again' },

  // Assistant message titles (response_kind → owner-facing header), shared by
  // ChatHomeView + InlineAssistantPanel via assistantResultTitle(result, locale).
  'assistantTitle.working': { zh: '{assistant}正在处理', en: '{assistant} is working' },
  'assistantTitle.draft': { zh: '草稿已就绪', en: 'Draft ready' },
  'assistantTitle.requestClarification': { zh: '还需要一个细节', en: 'One detail needed' },
  'assistantTitle.actionProposal': { zh: '查看提案', en: 'Review proposal' },
  'assistantTitle.actionReceipt': { zh: '已完成', en: 'Done' },
  'assistantTitle.answer': { zh: '回答', en: 'Answer' },
  'assistantTitle.error.creditsNeeded': { zh: '需要模型额度', en: 'Model credits needed' },
  'assistantTitle.error.modelSetting': { zh: '模型设置需要处理', en: 'Model setting needs attention' },
  'assistantTitle.error.anotherTry': { zh: '安琪需要再试一次', en: 'Aegis needs another try' },
  'assistantTitle.error.couldNotFinish': { zh: '我没能完成这件事', en: "I couldn't finish that" },

  // Working-state placeholders rendered as the visible turn text while Aegis is
  // still processing (owner-facing; App.tsx threads display_language locale).
  'chatHome.working.initial': {
    zh: '正在确认我已知的信息以及可以安全使用的内容…',
    en: 'Checking what I know and what I can safely use...',
  },
  'chatHome.working.slow': {
    zh: '仍在处理。正在查看可用的工具…',
    en: 'Still working. Looking through the allowed tools...',
  },
  'chatHome.working.long': {
    zh: '仍在处理。请保持此页面打开，结果会显示在这里。',
    en: 'Still working. Keep this page open; the answer will appear here.',
  },
  'chatHome.working.stillChecking': {
    zh: '仍在处理。正在确认回复是否已经准备好…',
    en: 'Still working. Checking whether the answer is ready...',
  },
  'chatHome.working.checkingContext': {
    zh: '正在结合可见的上下文确认{command}…',
    en: 'Checking {command} against the visible context...',
  },
  'chatHome.working.checkingContextGeneric': {
    zh: '正在结合可见的上下文进行确认…',
    en: 'Checking this against the visible context...',
  },
  // Live per-turn progress (Slice 3): replaces the static working text while a
  // tool runs / vision routing happens. `{tool}` is the backend tool name (not
  // localized). Single-pass `tf` interpolation only.
  // Live tool progress uses `toolPhrases.ts` for the human phrase; these keys
  // remain as generic fallbacks if a caller still interpolates `{tool}`.
  'chatHome.working.usingTool': {
    zh: '正在处理…',
    en: 'Working on it...',
  },
  'chatHome.working.toolDone': {
    zh: '好了，正在整理回复…',
    en: 'Done — finishing the reply...',
  },
  'chatHome.executorFooter': {
    zh: '经 {name} · {secs} 秒',
    en: 'via {name} · {secs}s',
  },
  'chatHome.working.processingImage': {
    zh: '正在处理图片…',
    en: 'Processing the image...',
  },
  // In-chat HITL approval card (Slice 4). The card shows the summary excerpt the
  // backend streams over the per-turn SSE need_approval frame (never raw
  // html/shell) plus Approve/Reject buttons that call the existing console
  // approve/reject endpoints. The same approval can also be actioned from
  // Telegram; the card degrades to a resolved chip if the backend reports it
  // already handled.
  // CEO Review Card shape (contract §4): 需要你确认 / 我准备做 / 为什么 / 批准·先不要
  'chatHome.approval.title': { zh: '需要你确认', en: 'Needs your review' },
  'chatHome.approval.whatLabel': { zh: '我准备做', en: 'I will' },
  'chatHome.approval.whyLabel': { zh: '为什么要你看', en: 'Why you' },
  'chatHome.approval.destinationLabel': { zh: '发往', en: 'Destination' },
  'chatHome.approval.wouldSendLabel': { zh: '将发送', en: 'Would send' },
  'chatHome.approval.stopped.sensitive': { zh: '内容敏感', en: 'Sensitive content' },
  'chatHome.approval.stopped.untrusted_source': { zh: '来源不可信', en: 'Untrusted source' },
  'chatHome.approval.stopped.jev_unavailable': { zh: 'Jev 不可用', en: 'Jev unavailable' },
  'chatHome.approval.untrusted.web_page': { zh: '网页', en: 'web page' },
  'chatHome.approval.untrusted.post': { zh: '帖子', en: 'post' },
  'chatHome.approval.untrusted.file': { zh: '文件', en: 'file' },
  'chatHome.approval.untrusted.tool_result': { zh: '工具结果', en: 'tool result' },
  'chatHome.approval.whatFallback': { zh: '一项需要你授权的操作', en: 'An action that needs your authorization' },
  'chatHome.approval.whatNamed': { zh: '执行：{action}', en: 'Run: {action}' },
  'chatHome.approval.whyFallback': { zh: '这一步会影响外部或不可轻易撤销，需你拍板。', en: 'This step has external or hard-to-undo effects and needs your call.' },
  'chatHome.approval.why.external': { zh: '会代表你对外发出内容。', en: 'This would send something outward in your name.' },
  'chatHome.approval.why.irreversible': { zh: '可能难以撤销。', en: 'This may be hard to undo.' },
  'chatHome.approval.why.money': { zh: '涉及付款或费用。', en: 'This involves payment or charges.' },
  'chatHome.approval.why.privacy': { zh: '会扩大可见的隐私内容。', en: 'This expands access to private content.' },
  'chatHome.approval.why.external_tool': { zh: '会调用外部工具或系统能力。', en: 'This uses an external tool or system capability.' },
  'chatHome.approval.tool.archive': { zh: '归档', en: 'Archive' },
  'chatHome.approval.tool.delete': { zh: '删除', en: 'Delete' },
  'chatHome.approval.tool.calendar': { zh: '更改日历', en: 'Change calendar' },
  'chatHome.approval.tool.external_tool': { zh: '使用外部工具', en: 'Use an external tool' },
  'chatHome.approval.tool.shell': { zh: '运行系统命令', en: 'Run a system command' },
  'chatHome.approval.tool.browser': { zh: '访问网页', en: 'Open the web' },
  'chatHome.approval.tool.read_private': { zh: '读取私密内容', en: 'Read private content' },
  'chatHome.approval.tool': { zh: '工具：{tool}', en: 'Tool: {tool}' },
  'chatHome.approval.summary': { zh: '{summary}', en: '{summary}' },
  'chatHome.approval.approve': { zh: '批准', en: 'Approve' },
  'chatHome.approval.reject': { zh: '先不要', en: 'Not now' },
  'chatHome.search.receipt': { zh: '搜索收据', en: 'Search receipt' },
  'chatHome.search.query': { zh: '查了', en: 'Looked up' },
  'chatHome.search.sources': { zh: '来源', en: 'Sources' },
  'chatHome.search.stop': { zh: '停止调研', en: 'Stop research' },
  'chatHome.search.stopped': { zh: '已停止，剩下的没有发出。', en: 'Stopped. The rest was not sent.' },
  'chatHome.approval.approved': { zh: '已批准', en: 'Approved' },
  'chatHome.approval.rejected': { zh: '已拒绝', en: 'Rejected' },
  'chatHome.approval.failed': { zh: '操作失败，请重试', en: 'Action failed, please try again' },
  'chatHome.approval.searchQuestion': { zh: '你问的', en: 'You asked' },
  'chatHome.approval.searchRunning': { zh: '正在继续这次搜索', en: 'Continuing this search' },
  'chatHome.approval.searchContinued': { zh: '搜索已继续', en: 'The search continued' },
  'chatHome.approval.searchFailed': { zh: '这次搜索没有完成', en: 'This search did not finish' },
  'chatHome.approval.alreadyDecided': { zh: '这次搜索已经批准', en: 'This search is already approved' },
  // Inline note.
  // Composer.
  'chatHome.composer.voiceInput': { zh: '语音输入', en: 'Voice input' },
  // Voice notes: the composer mic records AUDIO and the server transcribes it.
  // (It is not browser dictation — the owner asked for a real voice message.)
  'chatHome.voiceNote.record': { zh: '录制语音消息', en: 'Record a voice message' },
  'chatHome.voiceNote.recording': { zh: '正在录音…', en: 'Recording…' },
  'chatHome.voiceNote.transcribing': { zh: '正在转写…', en: 'Transcribing…' },
  'chatHome.voiceNote.stop': { zh: '发送', en: 'Send' },
  // Runtime notices persisted into chat history (were hardcoded English).
  'runtime.notice.toolUnavailable': { zh: '{tool}暂不可用或未授权。', en: '{tool} was unavailable or not authorized.' },
  'runtime.notice.moreDetailGate': { zh: '更多细节需要你的批准。', en: 'More detail needs your approval.' },
  'runtime.notice.dataAccessGate': { zh: '访问检查', en: 'Access check' },
  'runtime.notice.toolGate': { zh: '操作安全检查', en: 'Action safety check' },
  'runtime.notice.mfaRequired': { zh: '需要额外确认', en: 'Extra confirmation required' },
  'runtime.notice.consentRequired': { zh: '需要 Google 授权', en: 'Google permission required' },
  'runtime.notice.llmProvider': { zh: 'AI 服务', en: 'AI service' },
  'runtime.source.generic': { zh: '安琪的工具', en: 'Aegis tool' },
  'assistant.working.receipt': { zh: '安琪正在思考，尚未执行任何外部操作。', en: 'Aegis is reasoning now. No external action has been executed.' },
  'ceo.command.unknown': { zh: '未知指令：{name}', en: 'Unknown command: {name}' },
  'ceo.command.invalidArgs': { zh: '指令参数无效', en: 'Invalid command arguments' },
  'ceo.command.notWired': { zh: '指令尚未接入：{name}', en: 'Command not wired: {name}' },
  'ceo.command.failed': { zh: '指令执行失败', en: 'Command failed' },
  'ceo.command.needsConfirmation': {
    zh: '需要你确认后才会执行：{name}。请口头确认或在面板点批准。',
    en: 'This runs only after you confirm: {name}. Say so out loud or tap Approve in the panel.',
  },

  // Web Push (browser-side) — capability reasons + browser error names mapped
  // to owner copy. Unknown browser errors keep the raw name in parentheses.
  'webPush.capability.noBrowser': { zh: 'Web 应用通知只能在浏览器里使用。', en: 'Web Push is only available in a browser.' },
  'webPush.capability.noServiceWorker': { zh: '这个浏览器不支持 Service Worker。', en: 'This browser does not support service workers.' },
  'webPush.capability.noPushManager': { zh: '这个浏览器无法接收 Web 应用通知。', en: 'This browser cannot receive Web Push notifications.' },
  'webPush.capability.noNotification': { zh: '这个浏览器不提供通知权限。', en: 'This browser does not expose notification permission.' },
  'webPush.capability.supported': { zh: '这个浏览器可以申请 Web 应用通知。', en: 'This browser can request Web Push notifications.' },
  'webPush.error.permissionBlocked': { zh: '这个浏览器已屏蔽通知。请在浏览器设置里允许通知后重试。', en: 'Notifications are blocked for this browser. Allow them in the browser settings, then try again.' },
  'webPush.error.permissionNotGranted': { zh: '通知没有被允许。', en: 'Notifications were not enabled.' },
  'webPush.error.registerFailed': { zh: '通知服务注册失败', en: 'WebApp notification setup could not register the service worker' },
  'webPush.error.activateTimeout': { zh: '通知服务还没准备好。请关闭并重新打开已安装的应用，然后重试。', en: 'WebApp notification setup could not activate the service worker yet. Close and reopen the installed app, then try again.' },
  'webPush.error.subscribeFailed': { zh: '浏览器已授权，但订阅通知失败', en: 'WebApp notification subscription failed after browser permission was granted' },
  'webPush.error.subscribeDetail': { zh: '{second}；首次尝试：{first}', en: '{second}; first attempt: {first}' },
  'webPush.browserError.notAllowed': { zh: '浏览器拒绝了通知权限', en: 'the browser refused notification permission' },
  'webPush.browserError.abort': { zh: '推送服务没有完成订阅，请稍后重试', en: 'the push service did not complete the subscription; try again shortly' },
  'webPush.browserError.invalidState': { zh: '通知服务尚未就绪，请关闭并重新打开应用', en: 'the notification service is not ready yet; close and reopen the app' },
  'webPush.browserError.network': { zh: '连不上推送服务，请检查网络', en: 'the push service could not be reached; check your network' },
  'webPush.browserError.unknown': { zh: '浏览器返回了未知错误（{name}）', en: 'the browser returned an unknown error ({name})' },
  'chatHome.voiceNote.cancel': { zh: '取消', en: 'Cancel' },
  'chatHome.voiceNote.remove': { zh: '移除语音消息', en: 'Remove voice message' },
  'chatHome.voiceNote.sentAria': { zh: '语音消息', en: 'Voice message' },
  'chatHome.voiceNote.errorPermission': {
    zh: '无法使用麦克风。请在浏览器设置里允许本站访问麦克风。',
    en: 'No microphone access. Allow this site to use the microphone in your browser settings.',
  },
  'chatHome.voiceNote.errorUnsupported': {
    zh: '这台设备或服务器暂不支持语音消息。',
    en: 'Voice messages are not available on this device or server yet.',
  },
  'chatHome.voiceNote.errorRecord': {
    zh: '录音失败，请再试一次。',
    en: 'Recording failed. Try again.',
  },
  'chatHome.voiceNote.errorUpload': {
    zh: '语音消息发送失败，请再试一次。',
    en: 'Could not send the voice message. Try again.',
  },
  'chatHome.voiceNote.errorEmpty': {
    zh: '没有听清内容，请靠近麦克风重录。',
    en: 'No speech was recognized. Record again, closer to the microphone.',
  },
  'chatHome.message.imageAlt': { zh: '已发送的图片', en: 'Sent image' },
  'chatHome.message.attachmentOnly': { zh: '附件：{filename}', en: 'Attachment: {filename}' },
  'chatHome.composer.attachContext': { zh: '附加上下文', en: 'Attach context' },
  'chatHome.composer.attachFile': { zh: '添加附件', en: 'Attach file' },
  'chatHome.composer.fileAttached': { zh: '已附加：{filename}', en: 'Attached: {filename}' },
  'chatHome.composer.removeFile': { zh: '移除附件', en: 'Remove attachment' },
  'chatHome.composer.uploading': { zh: '上传中…', en: 'Uploading...' },
  'chatHome.composer.uploadFailed': { zh: '上传失败', en: 'Upload failed' },
  'chatHome.composer.removeAttached': { zh: '移除已附加的上下文', en: 'Remove attached context' },
  'chatHome.composer.removeQuoted': { zh: '移除引用的消息', en: 'Remove quoted message' },
  'chatHome.composer.busyPlaceholder': {
    zh: '安琪处理时可继续输入…',
    en: 'Keep typing while Aegis works...',
  },
  'chatHome.composer.placeholder': { zh: '告诉{assistant}…', en: 'Tell {assistant}…' },
  'chatHome.composer.tellAria': { zh: '告诉{assistant}任何事', en: 'Tell {assistant} anything' },
  'chatHome.composer.queueForAegis': { zh: '排队给{assistant}', en: 'Queue for {assistant}' },
  'chatHome.composer.sendToAegis': { zh: '发送给{assistant}', en: 'Send to {assistant}' },
  'chatHome.composer.queue': { zh: '排队', en: 'Queue' },
  'chatHome.composer.send': { zh: '发送', en: 'Send' },
  'chatHome.composer.needText': { zh: '先写一句再发送', en: 'Write a message to send' },
  'chatHome.composer.waitUpload': { zh: '等附件上传完再发送', en: 'Wait for the upload to finish' },
  // Work card next steps / proof.
  'chatHome.work.archivedNote': { zh: '已归档；如有需要再重新打开。', en: 'Archived; reopen only if needed.' },
  'chatHome.work.handledNote': {
    zh: '已处理。查看记录，或如有异常询问安琪。',
    en: 'Handled. Review the receipt or ask Aegis if anything looks wrong.',
  },
  'chatHome.work.openWhenUseful': { zh: '有需要时打开。', en: 'Open when useful.' },
  'chatHome.work.openMessageThenDecide': { zh: '打开该条目，然后决定。', en: 'Open the message, then decide.' },
  'chatHome.work.serviceReceiptKept': { zh: '已保留服务记录。', en: 'Service receipt kept.' },
  'chatHome.work.noOutsideAction': { zh: '尚未执行任何外部动作。', en: 'No outside action has run.' },
  'chatHome.work.open': { zh: '打开', en: 'Open' },
  // Work card UI.
  'chatHome.work.cardLabel': { zh: '已创建或引用的工作', en: 'Work created or referenced' },
  'chatHome.work.decision': { zh: '决定', en: 'Decision' },
  'chatHome.work.work': { zh: '工作', en: 'Work' },
  'chatHome.work.title': { zh: '工作项', en: 'Work item' },
  'chatHome.work.factsLabel': { zh: '工作下一步与记录', en: 'Work next step and proof' },
  'chatHome.work.next': { zh: '下一步', en: 'Next' },
  'chatHome.work.receipt': { zh: '记录', en: 'Receipt' },
  'chatHome.work.refreshedHidden': {
    zh: '安琪已刷新此项；旧操作已隐藏。',
    en: 'Aegis refreshed this; old actions are hidden.',
  },
  'chatHome.work.noNewAction': { zh: '没有执行新的动作。', en: 'No new action ran.' },
  'chatHome.work.opening': { zh: '打开中…', en: 'Opening...' },
  'chatHome.work.hideDetails': { zh: '隐藏详情', en: 'Hide details' },
  'chatHome.work.askAegis': { zh: '问{assistant}', en: 'Ask {assistant}' },
  'chatHome.work.couldNotOpenDetail': { zh: '无法打开此详情。', en: 'Could not open this detail.' },
  'chatHome.work.noWorkItem': { zh: '没有可用的工作项。', en: 'No work item is available.' },
  'chatHome.work.actions': { zh: '操作', en: 'Actions' },
  'chatHome.work.actionsLabel': { zh: '工作项操作', en: 'Work item actions' },
  'chatHome.work.quickActionsLabel': { zh: '快捷工作操作', en: 'Quick work actions' },
  'chatHome.work.working': { zh: '处理中…', en: 'Working...' },
  'chatHome.work.confirm': { zh: '确认？', en: 'Confirm?' },
  'chatHome.work.actionFailed': { zh: '操作失败。请重试。', en: 'Action failed. Try again.' },
  'chatHome.work.receiptLabel': { zh: '操作记录', en: 'Action receipt' },
  'chatHome.work.openedDetail': { zh: '已打开的工作详情', en: 'Opened work detail' },
  'chatHome.work.askPlaceholder': { zh: '询问关于这个…', en: 'Ask about this...' },
  'chatHome.work.askAboutItem': { zh: '问{assistant}这件事', en: 'Ask {assistant} about this item' },
  'chatHome.work.queueAboutItem': {
    zh: '为此工作项排队询问安琪',
    en: 'Queue for Aegis about this work item',
  },
  'chatHome.work.threadLabel': {
    zh: '安琪关于此工作项的回答',
    en: 'Aegis answers about this work item',
  },
  'chatHome.work.queuedNext': {
    zh: '已排队。安琪接下来会回答这个。',
    en: 'Queued. Aegis will answer this next.',
  },
  'chatHome.work.checkingItem': { zh: '{assistant}正在看这件事…', en: '{assistant} is checking this item…' },
  // Plain notice presentations.
  'chatHome.notice.aegisBrief': { zh: '安琪简报', en: 'Aegis brief' },
  'chatHome.notice.dailyBrief': { zh: '今日摘要', en: 'Daily brief' },
  'chatHome.notice.eveningReview': { zh: '晚间回顾', en: 'Evening review' },
  'chatHome.notice.aegisNotice': { zh: '安琪通知', en: 'Aegis notice' },
  'chatHome.notice.fullReview': { zh: '完整回顾', en: 'Full review' },
  'chatHome.notice.fullBrief': { zh: '完整简报', en: 'Full brief' },
  'chatHome.notice.fullMessage': { zh: '完整消息', en: 'Full message' },
  'chatHome.notice.lines': { zh: '{count} 行', en: '{count} lines' },
  'chatHome.notice.suggestedAttached': {
    zh: '附带建议的{kinds}',
    en: 'Suggested {kinds} attached',
  },
  'chatHome.notice.suggestedTask': { zh: '任务', en: 'task' },
  'chatHome.notice.suggestedReminder': { zh: '提醒', en: 'reminder' },
  'chatHome.notice.suggestedJoin': { zh: ' 和 ', en: ' + ' },
  // Count labels (brief meta).
  'chatHome.count.activeTaskOne': { zh: '{count} 个活跃任务', en: '{count} active task' },
  'chatHome.count.activeTaskMany': { zh: '{count} 个活跃任务', en: '{count} active tasks' },
  'chatHome.count.recentActionOne': { zh: '{count} 个最近动作', en: '{count} recent action' },
  'chatHome.count.recentActionMany': { zh: '{count} 个最近动作', en: '{count} recent actions' },
  'chatHome.count.pendingApprovalOne': { zh: '{count} 个待审批', en: '{count} pending approval' },
  'chatHome.count.pendingApprovalMany': { zh: '{count} 个待审批', en: '{count} pending approvals' },
  'chatHome.count.warningOne': { zh: '{count} 个警告', en: '{count} warning' },
  'chatHome.count.warningMany': { zh: '{count} 个警告', en: '{count} warnings' },
  // Severity labels.
  'chatHome.severity.critical': { zh: '严重', en: 'Critical' },
  'chatHome.severity.warning': { zh: '警告', en: 'Warning' },
  'chatHome.severity.important': { zh: '重要', en: 'Important' },
  // Collapsed notice rules.
  'chatHome.collapse.restart.label': { zh: '条重启通知', en: 'restart notices' },
  'chatHome.collapse.restart.detail': {
    zh: '更早的相邻重启通知已合并到这条最新的里。',
    en: 'earlier nearby restart notices were collapsed into this latest one.',
  },
  'chatHome.collapse.integrity.label': { zh: '份完整性报告', en: 'integrity reports' },
  'chatHome.collapse.integrity.detail': {
    zh: '更早的相邻完整性报告已合并到这条最新的里。',
    en: 'earlier nearby integrity reports were collapsed into this latest one.',
  },
  // Plain message extras.
  'chatHome.plain.sources': { zh: '来源', en: 'Sources' },
  'chatHome.plain.actions': { zh: '操作', en: 'Actions' },
  'chatHome.plain.aegisActions': { zh: '安琪操作', en: 'Aegis actions' },
  'chatHome.plain.sensitiveReview': {
    zh: '敏感项需要查看。准备好后在下方打开详情。',
    en: 'Sensitive item needs review. Open details below when you are ready.',
  },
  // WorkView — owner-facing Work surface chrome and labels (Task 8).
  // Empty-work + queue chrome.
  'work.kicker': { zh: '工作', en: 'Work' },
  'work.empty.headline': { zh: '没有等待你处理的待办。', en: 'Nothing is waiting for you.' },
  'work.empty.body': {
    zh: '去安琪那里说出你的需求。安琪可以从那里帮你查找内容、创建提醒，或开始跟踪一个话题。',
    en: 'Go to Aegis and say what you need. Aegis can look for something, create a reminder, or start tracking a topic from there.',
  },
  'work.startWith': { zh: '从这里开始', en: 'Start with' },
  'work.quietWatching.note': {
    zh: '{count} {loopWord}正在静默关注。在这里打开其中一个，看看安琪在关注什么、它是如何知道的。',
    en: '{count} {loopWord} watching quietly. Open one here to see what Aegis is watching and how it knows.',
  },
  // Owner-facing: 件事, not the internal "loop" (2026-09-13 UX pass).
  'work.loop.one': { zh: '件事', en: 'item' },
  'work.loop.other': { zh: '件事', en: 'items' },
  'work.next.kicker': { zh: '下一步', en: 'Next' },
  'work.next.heading': { zh: '接下来', en: 'After this' },
  'work.item.one': { zh: '条', en: 'item' },
  'work.item.other': { zh: '条', en: 'items' },
  'work.itemCount': { zh: '{count} {itemWord}', en: '{count} {itemWord}' },
  'work.itemCountHidden': { zh: '{count} {itemWord}已隐藏', en: '{count} {itemWord} hidden' },

  // Aria labels.
  'work.aria.queueBrief': { zh: '工作队列简报', en: 'Work queue brief' },
  'work.aria.lanes': { zh: '工作泳道', en: 'Work lanes' },
  'work.aria.otherItems': { zh: '其他待办', en: 'Other work items' },
  'work.aria.quietWatching': { zh: '静默关注', en: 'Watching quietly' },
  'work.aria.nextPreview': { zh: '下一项工作预览', en: 'Next work preview' },
  'work.aria.nearbyItems': { zh: '附近的待办', en: 'Nearby work items' },
  'work.aria.mobileInlineLanes': { zh: '带内联详情的待办', en: 'Work items with inline detail' },
  'work.aria.selectedItem': { zh: '选中的待办', en: 'Selected work item' },
  'work.aria.currentSummary': { zh: '当前工作摘要', en: 'Current work summary' },
  'work.aria.workWithAegis': { zh: '与安琪协作', en: 'Work with Aegis' },
  'work.aria.itemControls': { zh: '待办控制', en: 'Item controls' },
  'work.aria.receiptActions': { zh: '有记录支持的操作', en: 'Record-backed actions' },
  'work.aria.quickActions': { zh: '快捷待办操作', en: 'Quick item actions' },
  'work.aria.expandedItem': { zh: '展开的待办', en: 'Expanded work item' },
  'work.aria.inlineActions': { zh: '内联待办操作', en: 'Inline item actions' },
  'work.aria.followUpChoices': { zh: '后续选择', en: 'Follow-up choices' },
  'work.aria.actionReceipt': { zh: '操作记录', en: 'Action receipt' },
  'work.aria.scanControls': { zh: '工作扫描控制', en: 'Work scan controls' },
  'work.aria.range': { zh: '工作范围', en: 'Work range' },
  'work.aria.sort': { zh: '工作排序', en: 'Work sort' },
  'work.aria.laneNow': { zh: '现在可处理的泳道', en: 'Ready-now lane' },
  'work.aria.laneDecide': { zh: '需要拍板的泳道', en: 'Decide lane' },
  'work.aria.laneFollowUp': { zh: '跟进中的泳道', en: 'Follow-up lane' },
  'work.aria.laneBacklog': { zh: '待办泳道', en: 'Backlog lane' },
  'work.aria.laneDone': { zh: '已完成泳道', en: 'Done lane' },
  'work.aria.laneToggle': { zh: '切换泳道', en: 'Toggle lane' },
  'work.aria.matchingOther': { zh: '匹配的其他工作', en: 'Matching other work' },
  'work.aria.matching': { zh: '匹配的工作', en: 'Matching work' },
  'work.aria.pausedDecision': { zh: '已暂停的决策请求', en: 'Paused decision request' },
  'work.aria.pausedTrust': { zh: '已暂停待办的信任摘要', en: 'Paused item trust summary' },
  'work.aria.decisionChoices': { zh: '决策选择', en: 'Decision choices' },
  'work.aria.attachmentMetadata': { zh: '附件元数据', en: 'Attachment metadata' },
  'work.aria.taskSummary': { zh: '任务摘要', en: 'Task summary' },
  'work.aria.askThisItem': { zh: '向安琪询问这件事', en: 'Ask Aegis about this item' },
  'work.aria.askExpandedItem': { zh: '向安琪询问此展开的待办', en: 'Ask Aegis about this expanded item' },
  'work.aria.openDetails': { zh: '打开详情', en: 'Open details' },
  'work.aria.moreActions': { zh: '更多待办操作', en: 'More item actions' },
  'work.trust.receiptLabel': { zh: '完成后会留下', en: 'Proof when done' },
  'work.moreActions': { zh: '更多操作', en: 'More actions' },

  // Detail empty / Ask Aegis entry.
  'work.detail.selectPrompt': { zh: '选择一个待办以与安琪协作。', en: 'Select an item to work with Aegis.' },
  'work.askAegis': { zh: '询问安琪', en: 'Ask Aegis' },
  'work.askAegis.usingOpenedDetail': { zh: '同时使用已打开的详情。', en: 'Using opened detail too.' },
  'work.askAegis.usingItemContext': { zh: '使用这件事的上下文。', en: 'Uses this item context.' },
  'work.askAegis.usingExpandedItem': { zh: '使用此展开的待办，不会改变你的位置。', en: 'Uses this expanded item, without changing your place.' },
  'work.askAegis.placeholderItem': { zh: '询问关于这件事的问题……', en: 'Ask about this item...' },
  'work.askAegis.placeholderExpanded': { zh: '询问关于此展开待办的问题……', en: 'Ask about this expanded item...' },
  'work.askAegis.placeholderPaused': { zh: '询问关于此已暂停待办的问题，或告诉安琪如何修改规则。', en: 'Ask about this paused item, or tell Aegis how to revise the rule.' },

  // Intent row labels.
  'work.intent.when': { zh: '何时', en: 'When' },
  'work.intent.why': { zh: '为何', en: 'Why' },
  'work.intent.detail': { zh: '详情', en: 'Detail' },
  'work.intent.signal': { zh: '信号', en: 'Signal' },
  'work.intent.next': { zh: '下一步', en: 'Next' },

  // Primary / quick actions.
  'work.action.opening': { zh: '正在打开……', en: 'Opening...' },
  'work.action.openDetails': { zh: '打开详情', en: 'Open details' },
  'work.action.brief': { zh: '简报', en: 'Brief' },
  'work.action.explain': { zh: '说明', en: 'Explain' },
  'work.action.draft': { zh: '起草', en: 'Draft' },

  // Focus chip labels.
  'work.focus.decision': { zh: '决策', en: 'Decision' },
  'work.focus.needsDetail': { zh: '需要详情', en: 'Needs detail' },
  'work.focus.paused': { zh: '已暂停', en: 'Paused' },
  'work.focus.handled': { zh: '已处理', en: 'Handled' },
  'work.focus.tracking': { zh: '跟踪中', en: 'Tracking' },
  'work.focus.comingUp': { zh: '即将到来', en: 'Coming up' },
  'work.focus.ready': { zh: '就绪', en: 'Ready' },

  // Source note + timeline panels.
  'work.sourceNote.summary': { zh: '来源备注', en: 'Source note' },
  'work.sourceNote.title': { zh: '来自来源的补充上下文', en: 'Extra context from the source' },
  'work.receipt.label': { zh: '记录', en: 'Record' },
  'work.timeline.summary': { zh: '时间线', en: 'Timeline' },
  'work.timeline.available': { zh: '可查看历史', en: 'History available' },
  'work.timeline.due': { zh: '截止', en: 'Due' },
  'work.timeline.actions': { zh: '操作', en: 'Actions' },
  'work.timeline.receipts': { zh: '记录', en: 'Records' },
  'work.timeline.dueAt': { zh: '截止 {time}', en: 'Due {time}' },
  'work.action.one': { zh: '次操作', en: 'action' },
  'work.action.plural': { zh: '次操作', en: 'actions' },
  'work.receiptWord.one': { zh: '条记录', en: 'receipt' },
  'work.receiptWord.other': { zh: '条记录', en: 'receipts' },
  'work.update.one': { zh: '条更新', en: 'update' },
  'work.update.other': { zh: '条更新', en: 'updates' },
  'work.count.actions': { zh: '{count} {word}', en: '{count} {word}' },
  'work.count.receipts': { zh: '{count} {word}', en: '{count} {word}' },
  'work.count.updates': { zh: '{count} {word}', en: '{count} {word}' },

  // Scan controls.
  'work.scan.find': { zh: '查找', en: 'Find' },
  'work.scan.chromeHint': { zh: '需要时再搜索或筛选', en: 'Search or filter when you need it' },
  'work.scan.filters': { zh: '筛选与排序', en: 'Filters and sort' },
  'work.scan.searchPlaceholder': { zh: '搜索工作', en: 'Search work' },
  'work.scan.scope.all': { zh: '全部', en: 'All' },
  'work.scan.scope.needsMe': { zh: '需要我', en: 'Needs me' },
  'work.scan.scope.quiet': { zh: '安静', en: 'Quiet' },
  'work.scan.sort.lane': { zh: '泳道', en: 'Lane' },
  'work.scan.sort.priority': { zh: '优先级', en: 'Priority' },
  'work.scan.sort.updated': { zh: '更新时间', en: 'Updated' },
  'work.scan.total': { zh: '共 {count} 项', en: '{count} total' },
  'work.scan.ratio': { zh: '{shown}/{total}', en: '{shown}/{total}' },
  'work.scan.clear': { zh: '清除', en: 'Clear' },
  'work.scan.matchingOther': { zh: '匹配的其他工作', en: 'Matching other work' },
  'work.scan.matching': { zh: '匹配的工作', en: 'Matching work' },
  'work.scan.noMatches': { zh: '无匹配', en: 'No matches' },
  'work.scan.match.one': { zh: '项匹配', en: 'match' },
  'work.scan.match.other': { zh: '项匹配', en: 'matches' },
  'work.scan.matchCount': { zh: '{count} {word}', en: '{count} {word}' },
  'work.scan.emptyHint': { zh: '更改搜索或范围，让工作重新进入视野。', en: 'Change the search or range to bring work back into view.' },

  // Nearby quiet-tracking line.
  'work.nearby.quietNoAction': {
    zh: '现在无需任何操作。安琪只是在关注这件事，如果有变化或需要你，会再次提示。',
    en: 'No action is needed now. Aegis is only watching this item and will surface it again if something changes or needs you.',
  },

  // Item action group labels.
  'work.group.approveReject': { zh: '批准或拒绝', en: 'Approve or reject' },
  'work.group.pausedChoices': { zh: '已暂停的选择', en: 'Paused choices' },
  'work.group.decision': { zh: '决策操作', en: 'Decision actions' },
  'work.group.followUp': { zh: '后续操作', en: 'Follow-up actions' },
  'work.group.task': { zh: '任务操作', en: 'Task actions' },
  'work.group.actions': { zh: '操作', en: 'Actions' },

  // Item meta (kind + priority).
  'work.meta.task': { zh: '任务', en: 'Task' },
  'work.meta.goal': { zh: '目标', en: 'Goal' },
  // Timing line under a work row (was "watching · Sep 13, 8:09 PM" in zh).
  'work.timing.watching': { zh: '跟踪到', en: 'watching' },
  'work.timing.scheduled': { zh: '安排在', en: 'scheduled' },
  'work.timing.due': { zh: '截止', en: 'due' },
  'work.meta.draft': { zh: '草稿', en: 'Draft' },
  'work.meta.receipt': { zh: '记录', en: 'Record' },
  'detail.timeline.label.receipt': { zh: '记录', en: 'Receipt' },
  'detail.timeline.label.action': { zh: '操作', en: 'Action' },
  'detail.timeline.label.update': { zh: '更新', en: 'Update' },
  'work.meta.decision': { zh: '决定', en: 'Decision' },
  'work.meta.priority': { zh: '{level}优先级', en: '{level} priority' },

  // Source detail panel.
  'work.source.taskDetail': { zh: '任务详情', en: 'Task detail' },
  'work.source.sourceDetail': { zh: '来源详情', en: 'Source detail' },
  'work.source.draft': { zh: '草稿', en: 'Draft' },
  'work.source.detail': { zh: '详情', en: 'Detail' },
  'work.source.note': { zh: '备注', en: 'Note' },
  'work.source.taskHistory': { zh: '任务历史', en: 'Task history' },
  'work.source.attachments': { zh: '附件', en: 'Attachments' },
  'work.source.attachmentDownload': { zh: '下载', en: 'Download' },
  'work.source.attachmentPreview': { zh: '预览', en: 'Preview' },
  'work.source.attachmentHidePreview': { zh: '收起预览', en: 'Hide preview' },
  'work.source.taskDetailsHistory': { zh: '任务详情与历史', en: 'Task details and history' },
  'work.source.openTaskSource': { zh: '打开任务来源', en: 'Open task source' },
  'work.source.openReceiptSource': { zh: '打开记录来源', en: 'Open receipt source' },
  'work.source.openSource': { zh: '打开来源', en: 'Open source' },

  // Blocked-by-gate panel.
  'work.blocked.noActionRan': { zh: '还没有对外执行', en: 'Nothing ran outward' },
  // CEO Review Card labels (aligned with chatHome.approval.*).
  'work.blocked.ceoTitle': { zh: '需要你确认', en: 'Needs your review' },
  'work.blocked.whatLabel': { zh: '我准备做', en: 'I will' },
  'work.blocked.whyLabel': { zh: '为什么要你看', en: 'Why you' },
  'work.blocked.contextLabel': { zh: '当时在做什么', en: 'What was happening' },
  'work.blocked.whatAegisDoing': { zh: '安琪当时在做什么', en: 'What Aegis was doing' },
  'work.blocked.whyPaused': { zh: '为何暂停', en: 'Why it paused' },
  'work.blocked.requestedCapability': { zh: '请求的能力', en: 'Requested capability' },
  'work.blocked.safeDetails': { zh: '安全详情', en: 'Safe details' },
  'app.shell.header.signOut': { zh: '退出登录', en: 'Sign out' },
  'app.shell.header.signingOut': { zh: '正在退出…', en: 'Signing out…' },
  'work.blocked.choiceUnavailable': { zh: '此选择在当前待办状态下不可用。', en: 'This choice is not available in the current item state.' },
  'work.blocked.suggest.whyPaused.label': { zh: '为何暂停', en: 'Why paused' },
  'work.blocked.suggest.revise.label': { zh: '修改规则', en: 'Revise rule' },
  'work.blocked.suggest.canClose.label': { zh: '可以关闭吗？', en: 'Can close?' },

  // Decision-card choice copy keyed by stable server `choice.id` (FE-only
  // localization). Unknown ids fall back to the server `label`/`consequence`
  // strings. English matches server console_handlers prose byte-for-byte.
  'work.decide.choice.run_once.label': { zh: '批准本次运行', en: 'Approve this run' },
  'work.decide.choice.run_once.consequence': {
    zh: '允许安琪继续这次暂停的运行，并记录批准路径。',
    en: 'Aegis will continue this paused run once and record the approval path.',
  },
  'work.decide.choice.skip_once.label': { zh: '跳过本次运行', en: 'Skip this run' },
  'work.decide.choice.skip_once.consequence': {
    zh: '安琪跳过这次运行，重复规则保持不变。',
    en: 'Aegis will skip this paused run and leave the recurring rule unchanged.',
  },
  'work.decide.choice.approve_unavailable.label': { zh: '批准本次运行', en: 'Approve this run' },
  'work.decide.choice.approve_unavailable.consequence': {
    zh: '找不到有效的批准请求（可能已过期），无法从这里继续。',
    en: 'Aegis cannot continue from this card because the live approval request is missing or expired.',
  },
  'work.decide.choice.skip_unavailable.label': { zh: '跳过本次运行', en: 'Skip this run' },
  'work.decide.choice.skip_unavailable.consequence': {
    zh: '原批准请求已不在等待中，无法跳过。',
    en: 'Aegis cannot skip the original approval request because that request is no longer pending.',
  },
  'work.decide.choice.revise_rule.label': { zh: '修改规则', en: 'Revise rule' },
  'work.decide.choice.revise_rule.consequence': {
    zh: '先与安琪讨论一个更窄、更清晰的规则，再决定是否继续。',
    en: 'Discuss a narrower or clearer rule with Aegis before any run continues.',
  },
  'work.decide.choice.view_safe_details.label': { zh: '查看安全详情', en: 'View safe details' },
  'work.decide.choice.view_safe_details.consequence': {
    zh: '打开任务上下文与历史，不执行被拦截的步骤。',
    en: 'Open the task context and history without running the blocked step.',
  },
  // Additional finite choice ids on the same decide card (systemic sweep).
  'work.decide.choice.review_tomorrow.label': { zh: '明天再看', en: 'Review tomorrow' },
  'work.decide.choice.review_tomorrow.consequence': {
    zh: '安琪会保持暂停，明天再提醒你。',
    en: 'Aegis will keep this paused and surface it again tomorrow.',
  },
  'work.decide.choice.close_pause.label': { zh: '不运行并关闭', en: 'Close without running' },
  'work.decide.choice.close_pause.consequence': {
    zh: '安琪会把此暂停任务标为完成。不会执行被拦截步骤，也不会继续此任务。',
    en: 'Aegis will mark this paused task done. It will not run the blocked step, or continue this task.',
  },
  'work.approval.hostConfirm': { zh: '再确认一次', en: 'Confirm again' },
  'work.decide.choice.approve.label': { zh: '批准', en: 'Approve' },
  'work.decide.choice.approve.consequence': {
    zh: '安琪会通过现有审批流程发送此批准。',
    en: 'Aegis will send this approval through the existing approval path.',
  },
  'work.decide.choice.reject.label': { zh: '拒绝', en: 'Reject' },
  'work.decide.choice.reject.consequence': {
    zh: '安琪会通过现有审批流程拒绝此请求。',
    en: 'Aegis will reject this request through the existing approval path.',
  },
  // Risk / scope footers (finite server enums). Unknown values render raw.
  'work.decide.risk.low': { zh: '低风险', en: 'Low risk' },
  'work.decide.risk.medium': { zh: '中风险', en: 'Medium risk' },
  'work.decide.risk.high': { zh: '高风险', en: 'High risk' },
  'work.decide.scope.this_run': { zh: '本次运行', en: 'This run' },
  'work.decide.scope.rule': { zh: '规则', en: 'Rule' },
  'work.decide.scope.this_item': { zh: '这件事', en: 'This item' },
  'work.decide.scope.this_request': { zh: '此请求', en: 'This request' },
  // Prefix for freeform server `disabled_reason` strings.
  'work.decide.unavailable': { zh: '暂不可用：', en: 'Unavailable: ' },

  // Decision-card freeform-looking server TEMPLATES (title / what / capability /
  // scope / safe_details). Backend emits companion codes on VNextDecisionRequest;
  // FE overlays under non-en via resolveWorkCodeText('work.decide', code).
  // English matches server console_handlers prose byte-for-byte.
  'work.decide.title_web_search': { zh: '网页搜索需要你的选择', en: 'Web search needs your choice' },
  'work.decide.title_generic': { zh: '自动化需要你的选择', en: 'Automation needs your choice' },
  'work.decide.approval_title_browser_submit': { zh: '批准网页提交操作？', en: 'Approve browser submit?' },
  'work.decide.approval_title_tool_server': { zh: '批准连接工具服务？', en: 'Approve tool connection?' },
  'work.decide.approval_title_generic': { zh: '需要你的批准', en: 'Approval required' },
  'work.decide.what_web_search_scheduled': {
    zh: '运行计划中的网页搜索请求。',
    en: 'Run the scheduled web search request.',
  },
  'work.decide.what_generic_boundary': {
    zh: '继续一个到达受保护边界的自动化。',
    en: 'Continue an automation that reached a protected boundary.',
  },
  'work.decide.cap_web_search_tool': {
    zh: '为该计划请求使用网页搜索工具。',
    en: 'Use the web search tool for this scheduled request.',
  },
  'work.decide.cap_generic_boundary': {
    zh: '越过该自动化的受保护边界继续执行。',
    en: 'Continue past the protected boundary for this automation.',
  },
  'work.decide.scope_blocked_run': {
    zh: '批准或跳过仅作用于本次暂停的运行。选择关闭会把此暂停任务标为完成。只有你明确选择修改规则时，规则才会变化。',
    en: 'Approve or skip applies to this paused run. Closing marks this paused task done. Rule changes only happen if you explicitly choose a rule change.',
  },
  'work.decide.scope_approval_request': {
    zh: '仅限此批准请求。',
    en: 'This approval request only.',
  },
  'work.decide.safe_no_external_action': {
    zh: '尚未执行任何外部操作。',
    en: 'No external action has run.',
  },
  'work.decide.safe_visible_metadata': {
    zh: '可在不打开私密内容的情况下查看可见元数据。',
    en: 'Visible metadata can be reviewed without opening private content.',
  },
  'work.decide.safe_no_live_approval': {
    zh: '此暂停未挂接有效的批准请求；在安琪重新生成真实批准请求前，批准/跳过不可用。',
    en: 'No live approval request is attached to this pause, so approve/skip is unavailable until Aegis regenerates a real approval request.',
  },
  'work.decide.safe_no_outside_action': {
    zh: '在你做出选择前，不会执行任何外部操作。',
    en: 'No outside action runs until you choose.',
  },

  // Static fallback decision request (rendered when no server decision_request).
  'work.fallback.title': { zh: '自动化需要你的选择', en: 'Automation needs your choice' },
  'work.fallback.scope': { zh: '仅限此已暂停的待办。', en: 'This paused item only.' },
  'work.fallback.blockedReason': { zh: '安琪在继续之前已暂停。', en: 'Aegis paused before continuing.' },
  'work.fallback.requestedCapability': { zh: '继续此已暂停的自动化。', en: 'Continue this paused automation.' },
  'work.fallback.choice.label': { zh: '询问安琪', en: 'Ask Aegis' },
  'work.fallback.choice.consequence': { zh: '在做出选择前先讨论此次暂停。', en: 'Discuss this pause before choosing.' },
  'work.fallback.safeDetail': { zh: '没有外部操作执行。', en: 'No external action has run.' },
  'work.fallback.receipt': { zh: '安琪在声明完成前需要记录。', en: 'A receipt is required before Aegis claims completion.' },

  // Blocked trust chips.
  'work.trust.visibleMetadata': { zh: '可见元数据', en: 'Visible metadata' },
  'work.trust.safeSummaryOnly': { zh: '仅安全摘要', en: 'Safe summary only' },
  'work.trust.privateLocked': { zh: '私密详情已锁定', en: 'Private detail locked' },
  'work.trust.privateUnopened': { zh: '私密详情未打开', en: 'Private detail unopened' },
  'work.trust.receiptsRequired': { zh: '需要记录', en: 'Receipts required' },
  'work.trust.noActionRan': { zh: '没有操作执行', en: 'No action ran' },

  // Authority panel.
  'work.authority.access': { zh: '权限与依据', en: 'Permissions and basis' },
  'work.authority.open': { zh: '已打开', en: 'Open' },
  'work.authority.locked': { zh: '已锁定', en: 'Locked' },
  'work.authority.visible': { zh: '可见', en: 'Visible' },
  'work.authority.allowed': { zh: '允许', en: 'Allowed' },
  'work.authority.lockedCount': { zh: '{count} 项已锁定', en: '{count} locked' },
  'work.authority.noExtraLock': { zh: '无额外锁定', en: 'No extra lock' },
  'work.authority.askFirst': { zh: '先询问', en: 'Ask first' },
  'work.authority.visibleBasics': { zh: '可见基础信息', en: 'Visible basics' },
  'work.authority.why': { zh: '为何', en: 'Why' },
  'work.authority.contextUsed': { zh: '此处使用的上下文', en: 'Context used here' },
  'work.authority.record': { zh: '记录：{links}', en: 'Record: {links}' },
  'work.authority.safety.open': { zh: '所选详情已在此登录会话中打开。', en: 'Selected detail is open in this signed-in session.' },
  'work.authority.safety.locked': { zh: '在你打开来源详情之前，私密内容保持关闭。', en: 'Private content stays closed until you open the source detail.' },
  'work.authority.safety.approval': { zh: '安琪可以使用此处的可见上下文。最终仍由你决定。', en: 'Aegis can use the visible context here. You still make the call.' },
  'work.authority.safety.default': { zh: '安琪可以使用此处的可见上下文。如果它更改了任何内容，会保留记录。', en: 'Aegis can use the visible context here. If it changes anything, it will keep a receipt.' },

  // Queue headline / summary / kicker.
  'work.queue.oneDecision': { zh: '1 个决策需要你', en: '1 decision needs you' },
  'work.queue.decisionsNeedYou': { zh: '{count} 个决策需要你', en: '{count} decisions need you' },
  'work.queue.readyToHandle': { zh: '{count} {itemWord}可以处理', en: '{count} {itemWord} ready to handle' },
  'work.queue.loopsTracked': { zh: '正在跟踪 {count} {loopWord}', en: '{count} {loopWord} being tracked' },
  'work.queue.organizedQuietly': { zh: '一切都已悄然整理妥当', en: 'Everything is organized quietly' },
  'work.queue.bodyDefault': { zh: '{count} {itemWord}已按真正需要关注的程度整理。', en: '{count} {itemWord} organized by what actually needs attention.' },
  'work.queue.quietHeadline': { zh: '{count} {loopWord}正在静默关注', en: '{count} {loopWord} watching quietly' },
  'work.queue.quietBody': { zh: '现在没有其他待办需要你的操作。', en: 'No other item needs your action right now.' },
  'work.queue.onlyActive': { zh: '唯一的活动待办', en: 'Only active item' },
  'work.queue.onlyActiveBody': { zh: '在此处理。其他一切保持安静。', en: 'Handle it here. Everything else stays quiet.' },
  'work.queue.nearbyHeadline': { zh: '{count} 个附近{itemWord}', en: '{count} nearby {itemWord}' },
  'work.queue.nearbyBodyQuiet': { zh: '{count} {loopWord}正在静默关注，直到安琪需要你。', en: '{count} {loopWord} watching quietly until Aegis needs you.' },
  'work.queue.nearbyBodyDefault': { zh: '打开此面板即可切换而不离开当前待办。', en: 'Open this panel to switch without leaving the item.' },
  'work.queue.overviewKicker': { zh: '队列概览', en: 'Queue overview' },
  'work.queue.watchingKicker': { zh: '关注中', en: 'Watching' },
  'work.queue.moreWorkKicker': { zh: '更多工作', en: 'More work' },
  'work.nearbyItem.one': { zh: '待办', en: 'item' },
  'work.nearbyItem.other': { zh: '待办', en: 'items' },

  // Lane cue.
  'work.laneCue.quiet': { zh: '安静', en: 'quiet' },
  'work.laneCue.needsYou': { zh: '需要你', en: 'needs you' },
  'work.laneCue.readyNow': { zh: '现在就绪', en: 'ready now' },
  'work.laneCue.tracking': { zh: '跟踪中', en: 'tracking' },
  'work.laneCue.later': { zh: '稍后', en: 'later' },
  'work.laneCue.done': { zh: '完成', en: 'done' },
  'work.lane.decide.title': { zh: '需要决策', en: 'Decide' },
  'work.lane.decide.subtitle': { zh: '需要你拍板，安琪才能继续。', en: 'Needs your call before Aegis continues.' },
  'work.lane.now.title': { zh: '现在', en: 'Now' },
  'work.lane.now.subtitle': { zh: '现在就能处理。', en: 'Ready to handle now.' },
  'work.lane.followUp.title': { zh: '跟进', en: 'Follow Up' },
  'work.lane.followUp.subtitle': { zh: '等待、跟踪或已排期。', en: 'Waiting, tracking, or scheduled.' },
  'work.lane.backlog.title': { zh: '待办', en: 'Backlog' },
  'work.lane.backlog.subtitle': { zh: '稍后处理，默认安静。', en: 'Later, quiet by default.' },
  'work.lane.done.title': { zh: '已完成', en: 'Done' },
  'work.lane.done.subtitle': { zh: '已完成并留有记录。', en: 'Completed with receipts.' },

  // State cue.
  'work.stateCue.openDetailFirst': { zh: '先打开详情', en: 'Open detail first' },
  'work.stateCue.waitingForYou': { zh: '等待你', en: 'Waiting for you' },
  'work.stateCue.tracking': { zh: '跟踪中', en: 'Tracking' },
  'work.stateCue.scheduled': { zh: '已排程', en: 'Scheduled' },
  'work.stateCue.next': { zh: '下一步', en: 'Next' },

  // Next-move labels.
  'work.nextMove.openDetails': { zh: '打开详情', en: 'Open details' },
  'work.nextMove.handled': { zh: '已处理。查看记录，或在仍有疑问时询问安琪。', en: 'Handled. Review the receipt or ask Aegis if anything still looks wrong.' },
  'work.nextMove.askOrOpen': { zh: '询问安琪或打开来源详情。', en: 'Ask Aegis or open the source detail.' },
  'work.nextMove.reviewTask': { zh: '查看任务，然后选择。', en: 'Review the task, then choose.' },
  'work.nextMove.reviewBelow': { zh: '在下方查看或询问安琪。', en: 'Review below or ask Aegis.' },

  // Machine slugs on the authority line (title, brief, …).
  'work.field.title': { zh: '标题', en: 'Title' },
  'work.field.summary': { zh: '摘要', en: 'Summary' },
  'work.field.sender': { zh: '发件人', en: 'Sender' },
  'work.field.status': { zh: '状态', en: 'Status' },
  'work.field.body': { zh: '正文', en: 'Body' },
  'work.field.subject': { zh: '主题', en: 'Subject' },
  'work.field.due': { zh: '到期', en: 'Due' },
  'work.field.attachments': { zh: '附件', en: 'Attachments' },
  'work.slug.brief': { zh: '简报', en: 'Brief' },
  'work.slug.draft': { zh: '草稿', en: 'Draft' },
  'work.slug.explain': { zh: '说明', en: 'Explain' },

  // Owner-facing receipt copy.
  'work.receiptText.default': { zh: '如果安琪更改了任何内容，会保留记录。', en: 'If Aegis changes anything, it will keep a receipt.' },
  'work.receiptText.provider': { zh: '服务变更会保留服务商记录。仅阅读或起草不会更改安琪之外的任何内容。', en: 'Service changes keep a provider receipt. Reading or drafting alone does not change anything outside Aegis.' },
  'work.receiptText.task': { zh: '任务更改会以记录或明确的失败状态记录。', en: 'Task changes are recorded with a receipt or a clear failure state.' },
  'work.receiptText.externalComplete': { zh: '安琪无法在没有记录的情况下声明外部工作已完成。', en: 'Aegis cannot mark outside work done without a receipt.' },
  'work.receiptText.complete': { zh: '安琪无法在没有记录的情况下声明工作已完成。', en: 'Aegis cannot mark work done without a receipt.' },

  // Owner-visible signal cue (follow-up).
  'work.signal.savedTiming': { zh: '已保存时间：{time}。', en: 'Saved timing: {time}.' },
  'work.signal.hasReceipt': { zh: '安琪为此后续保留了记录或关联的工作记录。', en: 'Aegis has a receipt or linked work record for this follow-up.' },
  'work.signal.fromSavedState': { zh: '安琪从已保存的工作状态延续这件事；如果已过时，请让安琪清理。', en: 'Aegis is carrying this from saved work state; if that is stale, ask Aegis to clean it up.' },
  'work.signal.savedWorkState': { zh: '已保存的工作状态。', en: 'Saved work state.' },

  // Working-turn placeholder.
  'work.assistant.working': { zh: '{assistant}正在处理这件事……', en: '{assistant} is working inside this item...' },

  // Follow-up quick-prompt labels (prompts themselves stay LLM-facing English).
  'work.followUp.addUpdate': { zh: '添加更新', en: 'Add update' },
  'work.meta.followUpClosed': { zh: '已结束跟进', en: 'Follow-up closed' },
  'work.itemAction.hintCloseGoal': { zh: '停止跟踪这个目标，保留进度和历史，不将它标为已完成。', en: 'Stop tracking this goal, keeping its progress and history without marking it completed.' },
  'work.success.goalClosed': { zh: '已结束目标跟进，原有进度和历史记录已保留。', en: 'Goal follow-up closed. Existing progress and history have been kept.' },
  'work.followUp.cleanUp': { zh: '结束跟进', en: 'Close follow-up' },

  // ItemDetail title fallbacks.
  'work.title.pausedAutomation': { zh: '已暂停的自动化', en: 'Paused automation' },
  'work.title.selectedItem': { zh: '选中的待办', en: 'Selected item' },

  // InlineAssistantPanel — the item-scoped assistant collaboration panel.
  'inlineAssistant.label': { zh: '安琪', en: 'Aegis' },
  'inlineAssistant.turn.queued': { zh: '已排队', en: 'Queued' },
  'inlineAssistant.turn.you': { zh: '你', en: 'You' },
  'inlineAssistant.turn.aegis': { zh: '安琪', en: 'Aegis' },
  'inlineAssistant.turn.working': { zh: '{assistant}正在处理…', en: '{assistant} is working...' },
  'inlineAssistant.collapse': { zh: '收起', en: 'Collapse' },
  'inlineAssistant.retry.queueCheck': { zh: '排队查看结果', en: 'Queue check' },
  'inlineAssistant.retry.queueRetry': { zh: '排队重试', en: 'Queue retry' },
  'inlineAssistant.retry.checkResult': { zh: '查看结果', en: 'Check result' },
  'inlineAssistant.retry.tryAgain': { zh: '重试', en: 'Try again' },
  'inlineAssistant.send.queue': { zh: '排队', en: 'Queue' },
  'inlineAssistant.send.send': { zh: '发送', en: 'Send' },
  'inlineAssistant.aria.queueForAegis': { zh: '排队给安琪', en: 'Queue for Aegis' },
  'inlineAssistant.aria.sendToAegis': { zh: '发送给安琪', en: 'Send to Aegis' },
  'inlineAssistant.aria.conversation': { zh: '此条目的助手对话', en: 'Item assistant conversation' },
  'inlineAssistant.aria.suggestions': { zh: '建议的条目指令', en: 'Suggested item instructions' },
  'inlineAssistant.aria.ask': { zh: '就此条目询问安琪', en: 'Ask Aegis about this item' },
  'inlineAssistant.aria.showFullTime': { zh: '显示完整时间 {time}', en: 'Show full time {time}' },
  'inlineAssistant.aria.hideFullTime': { zh: '隐藏完整时间 {time}', en: 'Hide full time {time}' },
  // Assistant response view — section titles (display only; English string stays
  // the internal identity used for logic comparisons + dedupe).
  'assistantResponse.title.answer': { zh: '回答', en: 'Answer' },
  'assistantResponse.title.nextActions': { zh: '下一步', en: 'Next actions' },
  'assistantResponse.title.sources': { zh: '来源', en: 'Sources' },
  'assistantResponse.title.caveats': { zh: '注意点', en: 'Caveats' },
  'assistantResponse.title.done': { zh: '已完成', en: 'Done' },
  'assistantResponse.title.safety': { zh: '安全', en: 'Safety' },
  'assistantResponse.title.receipt': { zh: '记录', en: 'Receipt' },

  // Assistant response view — chrome.
  'assistantResponse.moreDetail': { zh: '更多详情', en: 'More detail' },

  // Assistant response view — runtime error rewrite (legacy connection failures).
  'assistantResponse.incompleteAnswer': {
    zh: '我没有得到完整的回答。',
    en: 'I did not get a complete answer.',
  },

  // Assistant response view — source evidence (external URL).
  'assistantResponse.source.externalLink': {
    zh: '外部链接：{host}',
    en: 'External link: {host}',
  },
  'assistantResponse.source.externalDetail': {
    zh: '安琪引用了这个外部网址：\n{url}\n\n请将外部页面视为不受信任的来源数据。在依据可能过时或敏感的信息采取行动前，请让安琪先核实重要细节。',
    en: 'Aegis cited this external URL:\n{url}\n\nTreat external pages as untrusted source data. Ask Aegis to verify important details before acting on stale or sensitive information.',
  },

  // Assistant response view — source evidence (memory).
  'assistantResponse.source.memory.summary': {
    zh: '使用了相关的已保存上下文。',
    en: 'Relevant saved context was used.',
  },
  'assistantResponse.source.memory.detail': {
    zh: '安琪将回忆的记忆作为回答的上下文。记忆本身不会执行操作；持久性更改仍需要明确的操作和记录。',
    en: 'Aegis used recalled memory as context for the answer. Memory does not execute actions by itself; durable changes still require an explicit action and receipt.',
  },

  // Assistant response view — source evidence (tasks / reminders).
  'assistantResponse.source.tasks.summary': {
    zh: '使用了任务或提醒记录。',
    en: 'Task or reminder records were used.',
  },
  'assistantResponse.source.tasks.detail': {
    zh: '安琪使用了任务/提醒系统。当创建或更改提醒或任务时，回答应包含任务 id、时间、频率以及记录或已保存的工作项。',
    en: 'Aegis used the task/reminder system. When a reminder or task is created or changed, the answer should include the task id, time, cadence, and receipt or saved work item.',
  },

  // Assistant response view — source evidence (calendar).
  'assistantResponse.source.calendar.summary': {
    zh: '涉及了日历数据或日历工具。',
    en: 'Calendar data or calendar tooling was involved.',
  },
  'assistantResponse.source.calendar.detail': {
    zh: '安琪使用了日历空闲信息或事件工具。日历写入在计为已完成前，需要明确的操作路径和记录。',
    en: 'Aegis used calendar availability or event tooling. Calendar writes require an explicit action path and receipt before they count as done.',
  },

  // Assistant response view — source evidence (web search).
  'assistantResponse.source.webSearch.summary': {
    zh: '请求了外部网络信息。',
    en: 'External web information was requested.',
  },
  'assistantResponse.source.webSearch.detail': {
    zh: '安琪将网络搜索结果作为依据。请将网络内容视为不受信任的来源数据；重要操作仍需要确认和记录。',
    en: 'Aegis used web search results as evidence. Treat web content as untrusted source data; important actions still need confirmation and receipt.',
  },

  // Assistant response view — source evidence (commute check).
  'assistantResponse.source.commute.summary': {
    zh: '使用了通勤上下文或交通工具。',
    en: 'Commute context or traffic tooling was used.',
  },
  'assistantResponse.source.commute.detail': {
    zh: '安琪使用了与通勤相关的上下文或工具。实时交通回答应包含时间、路线假设，以及结果是否来自实时检查。',
    en: 'Aegis used commute-related context or tooling. Live traffic answers should include timing, route assumptions, and whether the result came from a real-time check.',
  },

  // Assistant response view — source evidence (generic fallback).
  'assistantResponse.source.generic.summary': {
    zh: '安琪将此来源作为依据。',
    en: 'Aegis used this source as evidence.',
  },
  'assistantResponse.source.generic.detail': {
    zh: '此来源为回答提供了依据。如果它在安琪之外发生了变化，请在依据过时信息采取行动前刷新或让安琪核实。',
    en: 'This source informed the answer. If it changed outside Aegis, refresh or ask Aegis to verify before acting on stale information.',
  },

  // Assistant response view — claim evidence chips and excerpts.
  'assistantResponse.evidence.ref': { zh: '来源 {id}', en: 'Source {id}' },
  'assistantResponse.evidence.fullLength': { zh: '来源摘要 · 原文 {count} 字符', en: 'Source excerpt · original {count} characters' },
  'assistantResponse.evidence.gaps': { zh: '{ids} 没有带来可查看的来源', en: '{ids} did not return a source to open' },
  'assistantResponse.jev.verdict': { zh: 'Jev 判定', en: 'Jev verdict' },
  'assistantResponse.jev.verdict.allow': { zh: '允许', en: 'allow' },
  'assistantResponse.jev.verdict.ask': { zh: '需确认', en: 'ask' },
  'assistantResponse.jev.verdict.deny': { zh: '拒绝', en: 'deny' },
  'assistantResponse.jev.owner': { zh: '你的决定', en: 'Your decision' },
  'assistantResponse.jev.owner.approved': { zh: '已批准', en: 'approved' },
  'assistantResponse.jev.owner.rejected': { zh: '已拒绝', en: 'rejected' },
  'assistantResponse.jev.owner.pending': { zh: '等待确认', en: 'waiting' },
  'assistantResponse.jev.approvalId': { zh: '批准编号', en: 'Approval id' },
  'assistantResponse.jev.receipt': { zh: '记录', en: 'Receipt' },
  'assistantResponse.jev.auditTip': { zh: '审计提示', en: 'Audit tip' },
  'assistantResponse.jev.risk': { zh: '风险', en: 'Risk' },

  // Assistant response view — numbers the model stated without tool support.
  'assistantResponse.estimate.title': {
    zh: '推算，没有工具数据支持',
    en: 'Estimate — no tool data behind it',
  },
  'assistantResponse.estimate.count': {
    zh: '含 {count} 个推算数字',
    en: '{count} estimated number(s)',
  },
  'assistantResponse.estimate.toggle': { zh: '标出推算', en: 'Highlight estimates' },

  // Assistant response view — warnings.
  'assistantResponse.warning.toolUnavailable': {
    zh: '{tool}不可用或未获授权。',
    en: '{tool} was unavailable or not authorized.',
  },
  'assistantResponse.warning.moreDetailNeedsApproval': {
    zh: '更多详情需要你的批准。',
    en: 'More detail needs your approval.',
  },

  // Assistant response view — runtime source labels (humanized tool names).
  'assistantResponse.runtimeSource.tasks': { zh: '任务与提醒', en: 'Tasks and reminders' },
  'assistantResponse.runtimeSource.reminderScheduler': { zh: '提醒调度器', en: 'Reminder scheduler' },
  'assistantResponse.runtimeSource.commute': { zh: '通勤检查', en: 'Commute check' },
  'assistantResponse.runtimeSource.memory': { zh: '记忆', en: 'Memory' },
  'assistantResponse.runtimeSource.knowledgeBase': { zh: '知识库', en: 'Knowledge base' },
  'assistantResponse.runtimeSource.calendar': { zh: '日历', en: 'Calendar' },
  'assistantResponse.runtimeSource.webSearch': { zh: '网络搜索', en: 'Web search' },
  'assistantResponse.runtimeSource.apiCost': { zh: 'API 成本追踪', en: 'API cost tracker' },
  'assistantResponse.runtimeSource.serverMonitor': { zh: '服务器监控', en: 'Server monitor' },

  // Assistant response view — humanized machine labels (phrase map).
  'assistantResponse.machineLabel.dataAccessGate': { zh: '访问检查', en: 'Access check' },
  'assistantResponse.machineLabel.toolGate': { zh: '操作安全检查', en: 'Action safety check' },
  'assistantResponse.machineLabel.mfaRequired': { zh: '需要额外确认', en: 'Extra confirmation required' },
  'assistantResponse.machineLabel.consentRequired': { zh: '需要 Google 权限', en: 'Google permission required' },
  'assistantResponse.machineLabel.providerRequestId': { zh: '服务请求', en: 'Service request' },
  'assistantResponse.machineLabel.llmProvider': { zh: 'AI 服务', en: 'AI service' },
  'assistantResponse.machineWord.hitl': { zh: '你', en: 'Owner' },
  'assistantResponse.machineWord.llm': { zh: 'AI', en: 'AI' },
  'assistantResponse.machineWord.mfa': { zh: '额外确认', en: 'Extra confirmation' },
  'assistantResponse.machineWord.oauth': { zh: 'Google 登录', en: 'Google sign-in' },
  'assistantResponse.machineWord.kb': { zh: '知识库', en: 'Knowledge' },
  // StatusView — owner-facing health summary (status.* namespace).
  // Hero.
  'status.kicker': { zh: '系统健康', en: 'System health' },
  'status.hero.clear': { zh: 'Aegis 运行正常', en: 'Aegis is healthy' },
  // Status must not compete with Chat/Work as a second "needs you" desk.
  // These keys keep historical names for tests; copy is health-oriented.
  'status.hero.needsYouOne': {
    zh: '有 {count} 处系统需要关注',
    en: '{count} system area needs attention',
  },
  'status.hero.needsYouMany': {
    zh: '有 {count} 处系统需要关注',
    en: '{count} system areas need attention',
  },
  'status.trust.line': {
    zh: '要拍板的事请回对话或工作。',
    en: 'Decisions belong in Chat or Work.',
  },
  'status.hero.description': {
    zh: '连接、成本与运行状态。',
    en: 'Connections, cost, and runtime.',
  },
  // Metrics — one-word tile explanations.
  'status.metrics.label': { zh: '一眼看懂', en: 'At a glance' },
  'status.metric.needsYou': { zh: '需关注', en: 'Attention' },
  'status.metric.quiet': { zh: '正常', en: 'Quiet' },
  'status.metric.connections': { zh: '数据来源', en: 'Sources' },
  // Attention section.
  'status.attention.label': { zh: '系统关注点', en: 'System attention' },
  'status.attention.kicker': { zh: '健康', en: 'Health' },
  'status.attention.heading': { zh: '系统先看这些', en: 'System check first' },
  // Calm (nothing-needs-attention) panel.
  'status.calm.chip': { zh: '健康', en: 'Healthy' },
  'status.calm.heading': { zh: '系统无需关注。', en: 'Nothing in the system needs attention.' },
  'status.calm.description': {
    zh: '此页只显示 Aegis 是否能安全运行。',
    en: 'This page only shows whether Aegis can operate safely.',
  },
  // Attention-card action buttons (display only; the section label that picks the action stays English).
  'status.action.openWork': { zh: '打开工作', en: 'Open Work' },
  'status.action.openSettings': { zh: '打开设置', en: 'Open Settings' },
  // Operations disclosure ("working quietly").
  'status.operations.summary': { zh: '我在安静处理的事', en: 'I am handling quietly' },
  'status.operations.readyCount': { zh: '{count} 项就绪', en: '{count} ready' },
  'status.operations.quietCount': { zh: '{count} 项安静', en: '{count} quiet' },
  'status.operations.pillsLabel': { zh: '安静运行摘要', en: 'Quiet operation summary' },
  'status.operations.gridLabel': { zh: '安静处理的领域', en: 'Quietly handled areas' },
  // Health labels (display of section.health; the health value itself stays as data).
  'status.health.attention': { zh: '需要关注', en: 'Needs attention' },
  'status.health.quiet': { zh: '安静', en: 'Quiet' },
  'status.health.ready': { zh: '就绪', en: 'Ready' },
  // Connections (source) disclosure.
  'status.connections.summary': { zh: '连接', en: 'Connections' },
  'status.connections.ready': { zh: '就绪', en: 'Ready' },
  'status.connections.needsAttentionOne': { zh: '{count} 个需要关注', en: '{count} needs attention' },
  'status.connections.needsAttentionMany': { zh: '{count} 个需要关注', en: '{count} need attention' },
  // System events disclosure.
  'status.systemEvents.summary': { zh: 'Aegis 最近动静', en: 'What Aegis noticed' },
  'status.systemEvents.none': { zh: '最近没有新动静', en: 'Nothing new to report' },
  'status.systemEvents.noticeOne': { zh: '{count} 条提示', en: '{count} notice' },
  'status.systemEvents.noticeMany': { zh: '{count} 条提示', en: '{count} notices' },
  'status.systemEvents.quietCount': { zh: '{count} 项安静', en: '{count} quiet' },
  'status.systemEvents.emptyDetail': {
    zh: '常规系统提示保持安静。重要问题仍会显示在上方。',
    en: 'Routine system notices are quiet. Important problems still surface above.',
  },
  'status.systemEvents.repeated': { zh: '{count} 个事件 · {start} - {occurred}', en: '{count} events · {start} - {occurred}' },
  'status.systemEvents.eventLines': { zh: '详情', en: 'Details' },
  'status.systemEvents.recently': { zh: '最近', en: 'recently' },
  // Display mapping for backend-provided english section labels (identity stays English).
  'status.sectionLabel.Work': { zh: '工作', en: 'Work' },
  'status.sectionLabel.Decisions': { zh: '决定', en: 'Decisions' },
  'status.sectionLabel.Context': { zh: '我了解的情况', en: 'What I know' },
  'status.sectionLabel.CostControls': { zh: '花费与自动检查', en: 'Spending and checks' },
  'status.sectionLabel.Routines': { zh: '日常打理', en: 'Daily care' },
  'status.sectionLabel.Tracking': { zh: '跟进中', en: 'Following up' },
  'status.sectionLabel.Receipts': { zh: '已办记录', en: 'Proof of work' },
  'status.sectionLabel.Connections': { zh: '连接', en: 'Connections' },
  // Display mapping for backend-provided english source labels that appear in
  // the Status connection rows (identity stays English; only the display maps).
  'status.sourceLabel.Calendar': { zh: '日历', en: 'Calendar' },
  'status.sourceLabel.Memory': { zh: '记忆', en: 'Memory' },
  'status.sourceLabel.TasksAndReminders': { zh: '任务与提醒', en: 'Tasks and reminders' },
  'status.sourceLabel.SystemAlerts': { zh: '系统提醒', en: 'System alerts' },
  'status.sourceLabel.DraftsAndReceipts': { zh: '草稿与记录', en: 'Drafts and receipts' },
  'status.sourceLabel.Goals': { zh: '目标', en: 'Goals' },
  'status.sourceLabel.OwnerApprovals': { zh: '你的审批', en: 'Owner approvals' },
  'status.sourceLabel.WaitingItems': { zh: '等待中的待办', en: 'Waiting items' },
  'status.sourceLabel.StandingInstructions': { zh: '长期指令', en: 'Standing instructions' },
  'detail.field.reason': { zh: '原因', en: 'Reason' },
  'detail.field.looksLike': { zh: '看起来像', en: 'Looks like' },
  'detail.field.next': { zh: '下一步', en: 'Next' },
  'detail.field.state': { zh: '状态', en: 'State' },
  'detail.field.risk': { zh: '风险', en: 'Risk' },
  'detail.field.kind': { zh: '类型', en: 'Kind' },
  'detail.field.urgency': { zh: '紧急程度', en: 'Urgency' },
  'detail.field.primaryAction': { zh: '主要操作', en: 'Primary action' },
  'detail.field.availableActions': { zh: '可用操作', en: 'Available actions' },
  'detail.field.receipt': { zh: '记录', en: 'Receipt' },
  'detail.field.confidence': { zh: '置信度', en: 'Confidence' },
  'detail.field.pinned': { zh: '已固定', en: 'Pinned' },
  'detail.field.created': { zh: '创建时间', en: 'Created' },
  'detail.field.decision': { zh: '决定', en: 'Decision' },
  'detail.field.confirmation': { zh: '确认', en: 'Confirmation' },
  'detail.field.priority': { zh: '优先级', en: 'Priority' },
  'detail.field.due': { zh: '截止', en: 'Due' },
  'detail.field.mode': { zh: '模式', en: 'Mode' },
  'detail.field.focus': { zh: '焦点', en: 'Focus' },
  'detail.field.execution': { zh: '执行', en: 'Execution' },
  'detail.field.status': { zh: '状态', en: 'Status' },
  'detail.field.executed': { zh: '执行时间', en: 'Executed' },

  // Static display values / sentinels. 'Not set' is display-only: WorkView
  // filters the due placeholder via DetailField.placeholder, never by matching
  // this text. 'detail.value.unknown' (en 'Unknown') and
  // 'detail.value.unknownLower' (en 'unknown') keep the old en casing
  // byte-identical at their respective render sites.
  'detail.value.yes': { zh: '是', en: 'Yes' },
  'detail.value.no': { zh: '否', en: 'No' },
  'detail.value.notSet': { zh: '未设置', en: 'Not set' },
  'detail.value.unknown': { zh: '未知', en: 'Unknown' },
  'detail.value.unknownLower': { zh: '未知', en: 'unknown' },
  'detail.value.notExecuted': { zh: '未执行', en: 'Not executed' },
  'detail.value.receiptPresent': { zh: '已提供', en: 'Present' },
  'detail.value.receiptMissing': { zh: '缺失', en: 'Missing' },
  'detail.value.receiptNotApplicable': { zh: '执行前暂不适用', en: 'Not applicable until execution' },
  'detail.value.executionEnabled': { zh: '已启用', en: 'Enabled' },
  'detail.value.executionBlocked': {
    zh: '在批准并处理记录之前不会执行',
    en: 'Blocked until approval and receipt handling',
  },
  'detail.value.mfaMayBeRequired': { zh: '可能需要额外确认', en: 'Extra confirmation may be required' },
  'detail.value.sessionEnough': { zh: '你已登录的会话即可', en: 'Your signed-in session is enough' },

  // Locked detail (mfa_required / consent_required).
  'detail.locked.title': { zh: '详情已锁定', en: 'Detail locked' },
  'detail.locked.subtitleFallback': {
    zh: '请先在已登录会话中打开，安琪才能读取此详情。',
    en: 'Open this from your signed-in session before Aegis reads it.',
  },
  'detail.locked.warning': {
    zh: '此视图未读取任何私密正文内容。',
    en: 'No private body content was fetched for this view.',
  },
  'detail.locked.reasonMfa': {
    zh: '需要额外确认后，安琪才能打开此详情。',
    en: 'Extra confirmation is required before Aegis can open this detail.',
  },
  'detail.locked.reasonConsent': {
    zh: '需要 Google 授权后，安琪才能打开此详情。',
    en: 'Google permission is required before Aegis can open this detail.',
  },
  'detail.locked.reasonSensitive': {
    zh: '敏感详情需要确认。',
    en: 'Sensitive detail requires confirmation.',
  },

  // Generic fallback detail.
  'detail.generic.title': { zh: '来源详情', en: 'Source detail' },
  'detail.generic.subtitle': { zh: '安琪已打开所选来源。', en: 'Aegis opened the selected source.' },

  // Owner work item detail.
  'detail.workItem.titleFallback': { zh: '工作项', en: 'Work item' },
  'detail.workItem.summaryFallback': {
    zh: '请查看此条目并选择下一个安全操作。',
    en: 'Review this item and choose the next safe action.',
  },
  'detail.workItem.nextFallback': {
    zh: '询问安琪接下来应该怎么做。',
    en: 'Ask Aegis what should happen next.',
  },
  'detail.workItem.bodyLabel': { zh: '为什么重要', en: 'Why this matters' },
  'detail.workItem.warning': {
    zh: '这里只是安全的队列上下文。此视图未读取任何私密来源正文、附件或原始来源 id。',
    en: 'This is safe queue context only. No private source body, attachment, or raw source id was fetched for this view.',
  },

  // Source-unavailable detail. Title / next-step fallbacks REUSE
  // app.sourceDetail.changedTitle / app.sourceDetail.changedNextStep (en values
  // byte-identical, same render vector).
  'detail.unavailable.messageFallback': {
    zh: '底层来源已不可用。',
    en: 'The underlying source is no longer available.',
  },
  'detail.unavailable.warning': {
    zh: '此视图未读取任何私密详情。',
    en: 'No private detail was fetched for this view.',
  },

  // Memory item detail.
  'detail.memory.titleFallback': { zh: '记忆条目', en: 'Memory item' },
  'detail.memory.subtitle': {
    zh: '请确认安琪是否应继续依赖这条记忆。',
    en: 'Review whether Aegis should keep relying on this memory.',
  },
  'detail.memory.bodyLabel': { zh: '记忆', en: 'Memory' },
  'detail.memory.warning': {
    zh: '仅供查看。打开此详情不会更改、固定、删除或修正这条记忆。',
    en: 'Review only. Opening this detail does not change, pin, delete, or correct the memory.',
  },

  // Approval detail.
  'detail.approval.titleFallback': { zh: '需要批准', en: 'Approval required' },
  'detail.approval.summaryFallback': {
    zh: '安琪需要你的决定才能继续。',
    en: 'Aegis needs your decision before continuing.',
  },
  'detail.approval.nextFallback': {
    zh: '请批准或拒绝此请求。',
    en: 'Approve or reject this request.',
  },
  'detail.approval.receiptFallback': {
    zh: '安琪必须拿到记录后才能声称已完成。',
    en: 'A receipt is required before Aegis can claim completion.',
  },
  'detail.approval.bodyLabel': { zh: '为什么在等待', en: 'Why this is waiting' },
  'detail.approval.warning': {
    zh: '除非你选择「批准」，否则什么都不会执行。「拒绝」会停止此批准请求。',
    en: 'Nothing runs unless you choose Approve. Reject stops this approval request.',
  },

  // Task detail.
  'detail.task.titleFallback': { zh: '任务', en: 'Task' },
  'detail.task.subtitleFallback': { zh: '任务详情', en: 'Task detail' },

  // Contextual action draft detail. The `Review ${kind}` title stays English
  // (enum-arg-coupled: ownerLabel(action_kind) renders a wire enum — deferred
  // to the enum-map ticket to avoid zh-template + English-arg mixed output).
  'detail.actionDraft.subtitle': {
    zh: '已保存待审阅。尚未执行任何操作。',
    en: 'Saved for review. Nothing has been executed.',
  },
  'detail.actionDraft.bodyLabel': { zh: '草稿', en: 'Draft' },
  'detail.actionDraft.timelineSaved': { zh: '草稿已保存', en: 'Draft saved' },
  'detail.actionDraft.timelineStatus': { zh: '需要审阅', en: 'review needed' },
  'detail.actionDraft.warning': {
    zh: '仅供审阅。安琪尚未为此草稿发送、更改、删除或完成任何内容。',
    en: 'Review only. Aegis has not sent, changed, deleted, or completed anything for this draft.',
  },

  // Action receipt detail.
  'detail.receipt.summaryFallback': { zh: '操作记录', en: 'Action receipt' },
  'detail.receipt.bodyLabel': { zh: '记录证据', en: 'Receipt evidence' },
  'detail.receipt.warning': {
    zh: '在没有记录证据的情况下，安琪不应声称此操作已完成。',
    en: 'Aegis should not claim this action is complete without receipt evidence.',
  },

  // Timeline fallback label.
  'detail.timeline.eventFallback': { zh: '事件', en: 'Event' },

  // ── Chat work-state sync (vnext/chatWorkState.ts) ─────────────────────────
  // Shown on a chat work card whose item vanished from the refreshed queue.
  // The accompanying `status: 'done'` stays an English wire enum (identity:
  // CLOSED_CHAT_WORK_STATUSES and downstream status logic compare it).
  'chatWork.itemChangedSummary': {
    zh: '此条目在显示后已发生变化。安琪已刷新工作队列并隐藏旧操作。',
    en: 'This item changed after it was shown here. Aegis refreshed the work queue and hid the old actions.',
  },
  'chatWork.itemChangedNextAction': {
    zh: '如仍需要，请问安琪。',
    en: 'Ask Aegis if you still need it.',
  },
  // Store-wide header totals (NOT the paginated window). `{count}` interpolated.
    /** Select-mode row button label (tap toggles selection, does not open). */
  // SP3 — Work-view logic-layer strings (cues/titles/basis/actions/focus chips).
  // These render in the Work view but were string LITERALS returned from helper
  // functions in model.ts / interactionState.ts / presentation.ts / adapters.ts /
  // notificationActions.ts / ownerProjectionText.ts (they evaded a JSX-only scan,
  // mixing English into a zh UI). en values stay BYTE-IDENTICAL to the prior
  // hardcoded literals so existing English-output tests + internal English-match
  // predicates are unchanged; zh now switches.

  // interactionState.ts — per-action button + confirmation hint.
  'work.interaction.working': { zh: '处理中…', en: 'Working...' },
  'work.interaction.confirm': { zh: '确认？', en: 'Confirm?' },
  'work.interaction.confirmationHint': {
    zh: '尚未执行。点按「确认？」以确认。{hint}',
    en: 'Not run yet. Tap "Confirm?" to confirm. {hint}',
  },

  // WorkItemRow.tsx focusLabel() + WorkView detail focus chip share these.
  'work.focusChip.handled': { zh: '已处理', en: 'Handled' },
  'work.focusChip.paused': { zh: '已暂停', en: 'Paused' },
  'work.focusChip.decision': { zh: '需决策', en: 'Decision' },
  'work.focusChip.needsDetail': { zh: '需查看详情', en: 'Needs detail' },
  'work.focusChip.tracking': { zh: '跟踪中', en: 'Tracking' },
  'work.focusChip.comingUp': { zh: '即将到来', en: 'Coming up' },
  'work.focusChip.ready': { zh: '就绪', en: 'Ready' },

  // model.ts — ownerVisibleWorkCue / fallbacks / titles.
  'work.cue.handled': {
    zh: '已处理。除非你想回顾历史，否则无需操作。',
    en: 'Handled. No action is needed unless you want to review history.',
  },
  'work.cue.pausedFallback': { zh: '安琪在继续前已暂停。', en: 'Aegis paused before continuing.' },
  'work.cue.waitingForDecision': { zh: '等待你的决定。', en: 'Waiting for your decision.' },
  'work.cue.trackingUntil': {
    zh: '安琪会跟踪此事直到 {time}。',
    en: 'Aegis is tracking this until {time}.',
  },
  'work.cue.trackingQuietly': { zh: '安琪正在静默跟踪此事。', en: 'Aegis is tracking this quietly.' },
  'work.cue.scheduledFor': { zh: '已安排在 {time}。', en: 'Scheduled for {time}.' },
  'work.cue.scheduledNoAction': { zh: '已安排；暂时无需操作。', en: 'Scheduled; no action needed yet.' },
  'work.cue.openPrivateTask': {
    zh: '打开详情以在修改前查看任务标题和正文。',
    en: 'Open details to reveal the task title and body before changing it.',
  },
  'work.cue.openWhenUseful': { zh: '需要时再打开。', en: 'Open when useful.' },

  'work.title.thisItem': { zh: '此条目', en: 'this item' },
  'work.title.thisFollowUp': { zh: '此跟进项', en: 'this follow-up' },
  'work.title.theFirstItem': { zh: '第一个条目', en: 'the first item' },
  'work.title.privateTaskReview': { zh: '私密任务待审核', en: 'Private task needs review' },
  'work.title.pausedAutomationFallback': { zh: '已暂停的自动化', en: 'Paused automation' },
  'work.title.searchPaused': { zh: '搜索已暂停：{target}', en: 'Search paused: {target}' },
  'work.title.webSearchPaused': { zh: '网页搜索已暂停', en: 'Web search paused' },
  'work.blocked.searchWhatHappened': {
    zh: '安琪在搜索前暂停了：{target}。',
    en: 'Aegis paused before searching: {target}.',
  },
  'work.blocked.searchWhatHappenedNoTarget': {
    zh: '安琪正准备为一项已安排的请求运行网页搜索。',
    en: 'Aegis was about to run a web search for a scheduled request.',
  },
  'work.blocked.genericWhatHappened': {
    zh: '安琪在继续前暂停了这项自动化。',
    en: 'Aegis paused this automation before continuing.',
  },
  'work.blocked.decisionApproval': {
    zh: '批准此次运行、跳过它，或先让安琪修订规则。',
    en: 'Approve this run, skip it, or ask Aegis to revise the rule first.',
  },
  'work.blocked.decisionExpired': {
    zh: '实时审批请求缺失或已过期。你可以修订规则、查看安全详情，或关闭此暂停。',
    en: 'The live approval request is missing or expired. You can revise the rule, review safe details, or close the pause.',
  },
  'work.sensitiveStepFallback': {
    zh: '在安琪继续前，有一个敏感步骤需要你审核。',
    en: 'A sensitive step needs your review before Aegis continues.',
  },

  // ownerProjectionText.ts — Status/Settings owner-facing projection notices.
  // Server status tokens are matched against English keywords (kept), but the
  // owner-facing message/next-step text was hardcoded English; these localize
  // it. en values stay byte-identical to the prior literals. The card already
  // shows the localized section label as its title, so the empty-message branch
  // uses a generic "this area" phrasing instead of interpolating the label.
  'work.projection.costSaving.message': { zh: '暂停计划检查、你批准的例程、跟进项以及其他自动检查。', en: 'Pauses scheduled checks, owner-approved routines, follow-ups, and other automatic checks.' },
  'work.projection.costSaving.nextStep': { zh: '想让安琪恢复自动检查时，请到「设置」操作。', en: 'Use Settings when you want Aegis to resume automatic checks.' },
  'work.projection.empty.message': { zh: '此区域需要你关注。', en: 'This area needs attention.' },
  'work.projection.empty.nextStep': { zh: '如果此项仍需处理，请打开相关条目。', en: 'Open the related item if this still needs attention.' },
  'work.projection.mfa': { zh: '在安琪继续之前，需要你额外确认。', en: 'Extra owner confirmation is required before Aegis can continue.' },
  'work.projection.consent.message': { zh: 'Google 需要授权后才能打开私密详情。', en: 'Google needs permission before private details can open.' },
  'work.projection.consent.nextStep': { zh: '刷新此来源的 Google 授权。', en: 'Refresh the Google permission for this source.' },
  'work.projection.calendar.message': { zh: '日历同步需要你关注。', en: 'Calendar sync needs attention.' },
  'work.projection.calendar.nextStep': { zh: '如持续出现，请在「设置」刷新日历。', en: 'Refresh Calendar from Settings if this persists.' },
  'work.projection.credits.message': { zh: '模型额度已用尽。', en: 'Model credits are exhausted.' },
  'work.projection.credits.nextStep': { zh: '补充模型额度后重试。', en: 'Add model credits, then retry.' },
  'work.projection.rateLimit.message': { zh: '此区域暂时受到速率限制。', en: 'This area is temporarily rate limited.' },
  'work.projection.rateLimit.nextStep': { zh: '请稍后再试。', en: 'Try again later.' },
  'work.projection.safetyBoundary.message': { zh: '安琪在一个安全边界处暂停了。', en: 'Aegis paused at a safety boundary.' },
  'work.projection.safetyBoundary.nextStep': { zh: '打开所选条目并选择下一步。', en: 'Open the selected item and choose the next step.' },
  'work.projection.hiddenDetail.message': { zh: '部分私密详情已隐藏，打开所选条目后才会显示。', en: 'Some private detail stayed hidden until you open the selected item.' },
  'work.projection.hiddenDetail.nextStep': { zh: '打开所选条目以走安全的详情路径。', en: 'Open the selected item for the safe detail path.' },
  'work.projection.notConnected': { zh: '此信号尚未接入安琪。', en: 'This signal is not connected to Aegis yet.' },
  'work.projection.metadataOnly': { zh: '仅使用安全的基础信息；未打开任何私密来源内容。', en: 'Using safe basics only; no private source content was opened.' },
  'work.projection.quiet.message': { zh: '除非需要你关注，安琪会让此区域保持安静。', en: 'Aegis is keeping this area quiet unless it needs your attention.' },

  // model.ts — sourceStatusView (Status Connections disclosure + Settings
  // services). statusLabel/reason/nextStep render directly in a zh UI; server
  // status tokens stay English in the matching conditions, only owner copy
  // localizes. The unknown-status tail returns server content (not translated).
  'work.source.status.quiet': { zh: '暂时安静', en: 'Quiet for now' },
  'work.source.status.authRefresh': { zh: 'Google 登录需要刷新', en: 'Google sign-in needs refresh' },
  'work.source.status.permissionNeeded': { zh: '需要授权', en: 'Permission needed' },
  'work.source.status.unavailable': { zh: '当前不可用', en: 'Unavailable right now' },
  'work.source.status.ready': { zh: '就绪', en: 'Ready' },
  'work.source.status.delegated': { zh: '经 Codex 读取', en: 'Read via Codex' },
  // Reasons distinct from work.projection.* (these say "source", not "area");
  // byte-identical ones reuse the projection keys at the call site.
  'work.source.reason.consent': { zh: 'Google 需要授权后，此来源才能打开私密详情。', en: 'Google needs permission before this source can open private details.' },
  'work.source.reason.rateLimit': { zh: '此来源暂时受到速率限制。', en: 'This source is temporarily rate limited.' },
  'work.source.reason.circuit': { zh: '此来源在多次失败后已暂停。', en: 'This source paused after repeated failures.' },
  'work.source.reason.notConnectedYet': { zh: '此来源在连接之前会刻意保持安静。', en: 'This source is intentionally quiet until it is connected.' },
  'work.source.nextStep.authRefresh': { zh: '在期待新的私密更新前，请在「设置」中刷新 Google 连接。', en: 'Refresh the Google connection from Settings before expecting new private updates.' },
  'work.source.nextStep.permission': { zh: '授予缺失的权限后，安琪才能完整使用此来源。', en: 'Grant the missing permission before Aegis can use this source fully.' },
  'work.source.nextStep.unavailable': { zh: '请稍后再试；如持续出现，请在「设置」中检查服务连接。', en: 'Try again later; if it persists, check the service connection in Settings.' },
  'work.source.issueSummary.single': { zh: '{label}：{reason}。', en: '{label}: {reason}.' },
  'work.source.issueSummary.moreOne': { zh: '{label}：{reason}；另有 1 个连接受限。', en: '{label}: {reason}; 1 more connection limited.' },
  'work.source.issueSummary.moreMany': { zh: '{label}：{reason}；另有 {count} 个连接受限。', en: '{label}: {reason}; {count} more connections limited.' },

  // StatusView.fallbackStatusSections — client-built owner copy for the
  // operations grid when the server omits a projection. Section LABELS localize
  // separately (displaySectionLabel); these are the message/next-step strings.
  'work.statusFallback.work.organized': { zh: '当前工作已整理妥当；低价值的杂讯保持安静。', en: 'Current work is organized; low-value noise stays quiet.' },
  'work.statusFallback.work.quiet': { zh: '没有需要审核的进行中工作。', en: 'No active work needs review.' },
  'work.statusFallback.decisions.waitingOne': { zh: '工作区有 {count} 个决定待处理。', en: '{count} decision waiting in Work.' },
  'work.statusFallback.decisions.waitingMany': { zh: '工作区有 {count} 个决定待处理。', en: '{count} decisions waiting in Work.' },
  'work.statusFallback.decisions.none': { zh: '没有等待中的决定。', en: 'No decision is waiting.' },
  'work.statusFallback.decisions.nextStep': { zh: '打开工作区，先处理决定卡片。', en: 'Open Work and handle the decision cards first.' },
  'work.statusFallback.context': { zh: '安琪可以借助记忆中的上下文来解释事情，但仅凭上下文绝不会授权任何操作。', en: 'Aegis can use remembered context to explain things, but context alone never authorizes action.' },
  'work.statusFallback.costControls': { zh: '除非有事需要你，自动检查应保持安静且低成本。', en: 'Automatic checks should stay quiet and low-cost unless something needs you.' },
  'work.statusFallback.routines': { zh: '周期性职责在请求你关注之前，应说明它们为何浮现。', en: 'Recurring responsibilities should say why they surfaced before asking for your attention.' },
  'work.statusFallback.tracking.countOne': { zh: '正在跟踪 {count} 个待办。', en: '{count} item being tracked.' },
  'work.statusFallback.tracking.countMany': { zh: '正在跟踪 {count} 个待办。', en: '{count} items being tracked.' },
  'work.statusFallback.tracking.none': { zh: '没有等待中的跟进闭环。', en: 'No follow-up loop is waiting.' },
  'work.statusFallback.receipts': { zh: '没有记录，安琪不会声称外部工作已完成。', en: 'Aegis cannot claim external work is done without a receipt.' },
  'work.statusFallback.connections.attentionOne': { zh: '{count} 个连接需要关注。', en: '{count} connection needs attention.' },
  'work.statusFallback.connections.attentionMany': { zh: '{count} 个连接需要关注。', en: '{count} connections need attention.' },
  'work.statusFallback.connections.ready': { zh: '连接看起来已就绪。', en: 'Connections look ready.' },
  'work.statusFallback.connections.nextStep': { zh: '展开下方的连接详情，然后在「设置」中修复受影响的来源。', en: 'Open connection details below, then fix the affected source from Settings.' },

  // SettingsView — client-built security/notification fallback sections (label +
  // owner copy) when the server omits a projection. Telegram stays a brand name.
  'settings.security.signedInSession.label': { zh: '已登录会话', en: 'Signed-in session' },
  'settings.security.signedInSession.message': { zh: 'Google 会在打开私密工作前先核验你配置的账户。', en: 'Google verifies your configured account before private work opens.' },
  'settings.security.itemAccess.label': { zh: '条目访问', en: 'Item access' },
  'settings.security.itemAccess.message': { zh: '私密正文、附件以及对外写入都与所选条目和操作绑定。', en: 'Private bodies, attachments, and external writes stay tied to the selected item and action.' },
  'settings.notifications.telegram.message': { zh: 'Telegram 用于及时提醒，并链接回 WebApp——后者仍是主要的工作界面。', en: 'Telegram is for timely alerts and links back to the WebApp, which remains the main work surface.' },

  // WorkView.tsx — owner-action quick prompts. These are sent to the assistant
  // AND rendered as the owner's chat bubble (appendOwnerTurn), so they must
  // localize like buildCommandSuggestions.prompt. The safety constraints in the
  // blocked prompts are preserved in translation.
  'work.assistant.prompt.brief': { zh: '简要说明这个条目。', en: 'Brief this item.' },
  'work.assistant.prompt.draft': { zh: '起草下一步。', en: 'Draft a next step.' },
  'work.assistant.retry.checkResult': { zh: '检查结果。', en: 'Check result.' },
  'work.assistant.retry.tryAgain': { zh: '再试一次。', en: 'Try again.' },
  'work.followUp.update.prompt': { zh: '我有关于 {title} 的更新。帮我记录下来并决定下一步。', en: 'I have an update for {title}. Help me record it and decide the next step.' },
  // Fallback only when no knowledge.done/task.complete action exists. Prefer the
  // direct close action strip (execute on confirm) so cleanup never becomes an
  // infinite "确认清理吗?" assistant loop.
  'work.followUp.cleanup.prompt': {
    zh: '请立即关闭这个已过时的跟进项（保留历史）：{title}。不要再次要求确认——本句即为确认。完成后给出简短记录。',
    en: 'Close this stale follow-up now (keep history): {title}. Do not ask for confirmation again — this message is the confirmation. Reply with a short receipt when done.',
  },
  'work.blocked.suggest.whyPaused.prompt': { zh: '解释为什么这件事暂停了，并且只参考此卡片上显示的选项。', en: 'Explain why this paused and only refer to the choices shown on this card.' },
  'work.blocked.suggest.revise.prompt': { zh: '在任何运行继续之前，帮我修改这条已暂停的自动化规则。', en: 'Help me revise this paused automation rule before any run continues.' },
  'work.blocked.suggest.canClose.prompt': { zh: '解释关闭这个已暂停的条目是否安全，且不要执行任何外部操作。', en: 'Explain whether closing this paused item is safe, without executing any external action.' },
  'work.fallback.choice.assistantPrompt': { zh: '解释这个已暂停的条目以及最安全的可用选项。不要执行任何外部操作。', en: 'Explain this paused item and the safest available choices. Do not execute any external action.' },

  // model.ts — ownerVisiblePrimaryActionLabel default + review-request label.
  'work.primaryAction.open': { zh: '打开', en: 'Open' },
  'work.primaryAction.reviewRequest': { zh: '审核请求', en: 'Review request' },

  // model.ts — ownerVisibleReasonCue derived phrases.
  'work.reason.waitingDependency': { zh: '等待一项依赖。', en: 'Waiting on a dependency.' },
  'work.reason.needsApprovalAccess': { zh: '需要审批或访问权限。', en: 'Needs approval or access.' },
  'work.reason.fromMandate': { zh: '来自已批准的授权。', en: 'From an approved mandate.' },

  // Backend structural codes — owner_visible_reason_code (admin.rs
  // UnifiedInboxItem). One key per backend construction-site meaning; the en
  // value is byte-identical to the backend prose so en rendering is unchanged.
  // Unknown/absent codes fall back to the backend prose (never blank).
  'work.reasonCode.rule_matched_source': {
    zh: '一条启用的规则命中了该条目的基础来源信息。',
    en: "An active rule matched this item's basic source information.",
  },
  'work.reasonCode.stale_unread_kept_later': {
    zh: '这条较早的未读待办会先放在稍后处理，直到你打开、标记已读或归档它。',
    en: 'This older unread item is kept out of current work until you open, mark read, or archive it.',
  },
  'work.reasonCode.calendar_basics_changed': {
    zh: '日历基础信息有变化，可能影响你的安排。',
    en: 'Calendar basics changed or may affect planning.',
  },
  'work.reasonCode.approval_website_action': {
    zh: '安琪需要你的明确批准才能提交或更改网站内容。',
    en: 'Aegis needs your explicit approval before submitting or changing a website.',
  },
  'work.reasonCode.approval_tool_server': {
    zh: '安琪需要你的明确批准才能使用新的外部工具服务。',
    en: 'Aegis needs your explicit approval before using a new external tool server.',
  },
  'work.reasonCode.approval_generic': {
    zh: '安琪需要你的批准才能在应用之外做出任何更改。',
    en: 'Aegis needs your approval before changing anything outside the app.',
  },
  'work.reasonCode.approval_waiting_decision': {
    zh: '正在等待你选择批准或拒绝。',
    en: 'Waiting for your approve or reject decision.',
  },
  'work.reasonCode.approval_waiting_mfa': {
    zh: '执行前需要一次近期的多因素验证。',
    en: 'Approval requires a recent MFA check before execution.',
  },
  'work.reasonCode.approval_waiting_owner': {
    zh: '正在等待你做出决定。',
    en: 'Waiting for the owner decision.',
  },
  'work.reasonCode.task_closed_history': {
    zh: '保留用于查看历史与记录。',
    en: 'Kept for history and receipt review.',
  },
  'work.reasonCode.blocked_by_web_search_permission': {
    zh: '执行外部搜索这一步需要你的许可。',
    en: 'Running the external search step needs owner permission.',
  },
  'work.reasonCode.blocked_by_sensitive_step': {
    zh: '安琪在一个敏感步骤之前暂停了这个自动化。',
    en: 'Aegis paused this automation before a sensitive step.',
  },
  'work.reasonCode.blocked_by_source_details_locked': {
    zh: '在打开具体待办之前，私密来源详情保持锁定。',
    en: 'Private source details remain locked until an exact item is opened.',
  },
  'work.reasonCode.blocked_by_calendar_details_hidden': {
    zh: '私密的日程描述、地点与会议细节不在这一行队列信息里。',
    en: 'Private calendar description, location, and meeting details are not in this queue row.',
  },
  'work.reasonCode.blocked_by_memory_detail_gate': {
    zh: '原始记忆详情仍在记忆详情关卡之后。',
    en: 'Raw memory detail stays behind the memory detail gate.',
  },
  'work.reasonCode.blocked_by_draft_execution_disabled': {
    zh: '在接上批准路径与记录路径之前，这份草稿的执行被有意禁用。',
    en: 'Execution is intentionally disabled for this draft until an approval route and receipt path are connected.',
  },
  'work.reasonCode.blocked_by_receipt_missing': {
    zh: '在出现类似记录的证据之前，完成状态不可信。',
    en: 'Completion is not trusted until receipt-like evidence exists.',
  },
  'work.reasonCode.blocked_web_search_pause': {
    zh: '这一步可能调用外部搜索工具，安琪已先暂停。',
    en: 'This may call an external search tool, so Aegis paused before continuing.',
  },
  'work.reasonCode.blocked_generic_pause': {
    zh: '有一个敏感步骤需要你先确认，安琪已暂停。',
    en: 'A sensitive step needs owner review before Aegis continues.',
  },
  'work.reasonCode.task_tracking_loop': {
    zh: '安琪正在跟进此事，它还没有了结。',
    en: 'Aegis is tracking this because it is part of an open follow-up loop.',
  },
  'work.reasonCode.task_scheduled_hold': {
    zh: '安琪会在计划的检查或提醒时间处理此事。',
    en: 'Aegis is holding this for its scheduled check or notification time.',
  },
  'work.reasonCode.task_backlog_quiet': {
    zh: '此事有意保持安静，直到你主动推进。',
    en: 'This is intentionally quiet until you pull it forward.',
  },
  'work.reasonCode.task_decision_needed': {
    zh: '安琪保留此事，因为它需要你做出选择。',
    en: 'Aegis is preserving this because it needs an owner choice.',
  },
  'work.reasonCode.task_owner_review': {
    zh: '安琪将此任务提交给你查看。',
    en: 'Aegis surfaced this task for owner review.',
  },
  'work.reasonCode.task_follow_up_visible': {
    zh: '安琪把它保持可见作为跟进待办，不需要立即决定。',
    en: 'Aegis is keeping this visible as a follow-up, not as an immediate decision.',
  },
  'work.reasonCode.task_needs_recovery': {
    zh: '该任务需要你协助恢复。',
    en: 'Aegis needs owner help to recover this task.',
  },
  'work.reasonCode.task_active_owner_work': {
    zh: '这是你正在进行的工作，安琪将它列出。',
    en: 'Aegis surfaced this task because it is active owner work.',
  },
  'work.reasonCode.pinned_memory_review': {
    zh: '一条置顶的长期记忆已有一段时间未核对，值得偶尔复查。',
    en: 'A pinned durable memory is old enough to deserve occasional owner review.',
  },
  'work.reasonCode.active_goal_ongoing': {
    zh: '这个进行中的目标属于你正在推进的工作。',
    en: "This active goal is part of the owner's ongoing work.",
  },
  'work.reasonCode.waiting_due_date_visible': {
    zh: '此事设有截止日期，安琪保持它可见。',
    en: 'Aegis is keeping this visible because it has a due date.',
  },
  'work.reasonCode.draft_needs_confirmation': {
    zh: '安琪根据你的指示起草了此内容，保存或执行前需要你确认。',
    en: 'Aegis drafted this from your instruction and needs your confirmation before saving or executing anything.',
  },
  'work.reasonCode.action_state_inspect': {
    zh: '有一个动作处于执行中、已完成、失败或有记录的状态，可查看。',
    en: 'An action has a running, completed, failed, or receipt state to inspect.',
  },
  'work.reasonCode.system_health_changed': {
    zh: '系统健康状况或数据源可用性发生了变化。',
    en: 'System health or source availability changed.',
  },

  // Backend structural codes — next_safe_action_code (admin.rs
  // UnifiedInboxItem). Same contract as work.reasonCode.*.
  'work.nextCode.open_source_inspect': {
    zh: '只有当你希望安琪检查时才打开对应来源。',
    en: 'Open the exact source only if you want Aegis to inspect it.',
  },
  'work.nextCode.stale_unread_later': {
    zh: '清理、归档，或留待以后处理。',
    en: 'Clean up, archive, or leave for later.',
  },
  'work.nextCode.calendar_basics_review': {
    zh: '先查看日程基础信息，再让安琪处理私人细节。',
    en: 'Review the schedule basics before asking Aegis to reason over private details.',
  },
  'work.nextCode.approval_website_confirm': {
    zh: '确认这正是你想执行的网站操作后再批准；否则请拒绝。',
    en: 'Approve only if this is the exact website action you intended; otherwise reject.',
  },
  'work.nextCode.approval_tool_server_confirm': {
    zh: '只有当你信任该工具服务用于本次会话时才批准；否则请拒绝。',
    en: 'Approve only if you trust this tool server for the current session; otherwise reject.',
  },
  'work.nextCode.approval_high_risk_review': {
    zh: '查看这个高风险审批，如选择批准请完成验证。',
    en: 'Review the high-risk approval and verify if you choose to approve.',
  },
  'work.nextCode.approval_review_decide': {
    zh: '查看该审批，然后批准或拒绝。',
    en: 'Review the approval, then approve or reject.',
  },
  'work.nextCode.blocked_web_search_choice': {
    zh: '批准这次搜索，跳过它，或让安琪修改规则。',
    en: 'Approve this search run, skip it, or ask Aegis to revise the rule.',
  },
  'work.nextCode.blocked_generic_choice': {
    zh: '查看这次暂停，让安琪修改该自动化，或将其关闭。',
    en: 'Review this pause, ask Aegis to revise the automation, or close it.',
  },
  'work.nextCode.task_closed_receipts': {
    zh: '只有需要记录或历史时才打开详情。',
    en: 'Open details only if you need receipts or history.',
  },
  'work.nextCode.task_open_or_act': {
    zh: '打开详情查看私人任务，或在已确认的情况下选择一个安全操作。',
    en: 'Open details to reveal the private task, or choose a safe action if you already recognize it.',
  },
  'work.nextCode.task_private_review': {
    zh: '先打开详情，看完标题和正文再完成或改期。',
    en: 'Open the details and read the title and body before completing or snoozing.',
  },
  'work.nextCode.memory_review_choices': {
    zh: '继续使用、置顶、标记有误或忘记这条记忆。需要改写时可以问安琪。',
    en: 'Keep using, pin, mark wrong, or forget this memory. Ask Aegis if it needs rewriting.',
  },
  'work.nextCode.goal_next_step': {
    zh: '让安琪给出下一步具体行动，或把它拆成任务。',
    en: 'Ask Aegis for the next concrete step or to turn this into tasks.',
  },
  'work.nextCode.waiting_review_due': {
    zh: '检查这个有截止日期的跟进是否仍然需要，然后延后或关闭它。',
    en: 'Review whether this dated follow-up is still needed, then defer or close it.',
  },
  'work.nextCode.draft_reminder_review': {
    zh: '检查提醒内容。时间和措辞正确再保存；否则取消或让安琪修改。',
    en: 'Review the reminder text. Save it only if the timing and wording are right; otherwise cancel or ask Aegis to revise it.',
  },
  'work.nextCode.draft_task_review': {
    zh: '检查任务内容。确认它属于你的工作队列再保存；否则取消或让安琪修改。',
    en: 'Review the task text. Save it only if it belongs in your work queue; otherwise cancel or ask Aegis to revise it.',
  },
  'work.nextCode.draft_generic_review': {
    zh: '检查这份草稿。尚未执行任何操作，在审批与记录路径接通前也不会执行。',
    en: 'Review the draft. Nothing has been executed, and execution remains blocked until approval and receipt handling are connected.',
  },
  'work.nextCode.receipt_missing_verify': {
    zh: '打开该动作记录，确认记录证据缺失的原因。',
    en: 'Open the action record and verify why receipt evidence is missing.',
  },
  'work.nextCode.receipt_verify': {
    zh: '打开记录，核实发生了什么。',
    en: 'Open the receipt to verify what happened.',
  },
  'work.nextCode.system_review_if_needed': {
    zh: '仅当数据源或系统状态需要处理时再查看。',
    en: 'Review only if the source or system state needs attention.',
  },

  // Backend structural codes — UnifiedInboxAction.label_code (admin.rs).
  'work.actionLabel.open_calendar_item': { zh: '打开日历条目', en: 'Open calendar item' },
  'work.actionLabel.open_source_item': { zh: '打开来源条目', en: 'Open source item' },
  'work.actionLabel.mark_read': { zh: '标记已读', en: 'Mark read' },
  'work.actionLabel.mark_unread': { zh: '标记未读', en: 'Mark unread' },
  'work.actionLabel.archive': { zh: '归档', en: 'Archive' },
  'work.actionLabel.review_calendar_basics': { zh: '查看日程基础信息', en: 'Review calendar basics' },
  'work.actionLabel.review_approval': { zh: '查看审批请求', en: 'Review approval' },
  'work.actionLabel.approve': { zh: '批准', en: 'Approve' },
  'work.actionLabel.reject': { zh: '拒绝', en: 'Reject' },
  'work.actionLabel.open_details': { zh: '打开详情', en: 'Open details' },
  'work.actionLabel.close_without_running': { zh: '直接关闭（不执行）', en: 'Close without running' },
  'work.actionLabel.review_tomorrow': { zh: '明天再看', en: 'Review tomorrow' },
  'work.actionLabel.complete': { zh: '完成', en: 'Complete' },
  'work.actionLabel.snooze': { zh: '稍后提醒', en: 'Snooze' },
  'work.actionLabel.keep_using': { zh: '继续使用', en: 'Keep using' },
  'work.actionLabel.pin': { zh: '置顶', en: 'Pin' },
  'work.actionLabel.unpin': { zh: '取消置顶', en: 'Unpin' },
  'work.actionLabel.mark_wrong': { zh: '标记有误', en: 'Mark wrong' },
  'work.actionLabel.forget': { zh: '忘记', en: 'Forget' },
  'work.actionLabel.review_memory_detail': { zh: '查看记忆详情', en: 'Review memory detail' },
  'work.actionLabel.done': { zh: '完成', en: 'Done' },
  'work.actionLabel.review_draft': { zh: '查看草稿', en: 'Review draft' },
  'work.actionLabel.open_receipt': { zh: '打开记录', en: 'Open receipt' },

  // model.ts — buildVNextHomeSignals + signalSummaryForItem.
  'work.signalSummary.needsDecision': { zh: '需决策', en: 'Needs decision' },
  'work.signalSummary.needsDetail': { zh: '需查看详情', en: 'Needs detail' },
  'work.signalSummary.readyNow': { zh: '现在就绪', en: 'Ready now' },
  'work.signalSummary.followingUp': { zh: '跟进中', en: 'Following up' },
  'work.signalSummary.connectionIssue': { zh: '连接问题', en: 'Connection issue' },
  'work.signalSummary.quiet': { zh: '安静', en: 'Quiet' },
  'work.signalSummary.oneConnection': { zh: '一个连接需要关注。', en: 'One connection needs attention.' },
  'work.signalSummary.connectionsLimited': {
    zh: '{count} 个连接受限。',
    en: '{count} connections are limited.',
  },
  'work.signalSummary.openWorkHidden': {
    zh: '打开「工作」以查看隐藏的详情',
    en: 'Open Work to review hidden details',
  },
  'work.signalSummary.pausedForReview': { zh: '已暂停待审核', en: 'Paused for review' },
  'work.signalSummary.waitingForChoice': { zh: '等待你的选择', en: 'Waiting for your choice' },
  'work.signalSummary.trackingUntil': { zh: '跟踪至 {time}', en: 'Tracking until {time}' },
  'work.signalSummary.trackingQuietly': { zh: '静默跟踪', en: 'Tracking quietly' },
  'work.signalSummary.scheduledAt': { zh: '已安排 {time}', en: 'Scheduled {time}' },
  'work.signalSummary.scheduled': { zh: '已安排', en: 'Scheduled' },
  'work.signalFallback.thisDecision': { zh: '此决策', en: 'this decision' },
  'work.signalFallback.thisWork': { zh: '此项工作', en: 'this work' },
  'work.signalFallback.thisFollowUp': { zh: '此跟进', en: 'this follow-up' },
  'work.quietHomeBrief': {
    zh: '目前没有紧急待办需要你处理。',
    en: 'Nothing urgent is asking for you right now.',
  },

  // model.ts — homeBriefForItem.
  'work.homeBrief.needsAttention': { zh: '需要关注：{title}。', en: 'Needs attention: {title}.' },
  'work.homeBrief.needsDecision': { zh: '需要你决策：{title}。', en: 'Needs your decision: {title}.' },
  'work.homeBrief.comingUp': { zh: '即将到来：{title}。', en: 'Coming up: {title}.' },
  'work.homeBrief.readyToReview': { zh: '可以查看：{title}。', en: 'Ready to review: {title}.' },
  'work.homeBrief.followingUp': { zh: '正在跟进：{title}。', en: 'Following up: {title}.' },
  'work.homeBrief.savedForLater': { zh: '已静默保存以备后用：{title}。', en: 'Quietly saved for later: {title}.' },
  'work.homeBrief.startWith': { zh: '从 {title} 开始。', en: 'Start with {title}.' },

  // model.ts — authorityBasisLabel (the basis-text variants).
  'work.basis.waitingReview': { zh: '安琪正在等待你的审核。', en: 'Aegis is waiting for your review.' },
  'work.basis.ownerReviewFallback': {
    zh: '安琪提出此项以供你审核。',
    en: 'Aegis surfaced this for owner review.',
  },
  'work.basis.activeMandate': {
    zh: '你批准的一项常设指令触发了此项。',
    en: 'A standing instruction you approved surfaced this.',
  },
  'work.basis.directRequest': { zh: '你请求安琪处理此事。', en: 'You asked Aegis to work on this.' },
  'work.basis.manualReview': {
    zh: '安琪在进一步处理前请求你的审核。',
    en: 'Aegis is asking for your review before going further.',
  },
  'work.basis.systemHealth': { zh: '系统健康状况需要关注。', en: 'System health needs attention.' },

  // model.ts — buildSupportedItemActions labels / confirm labels / receipt hints.
  'work.itemAction.approveRun': { zh: '批准此次运行', en: 'Approve this run' },
  'work.itemAction.approve': { zh: '批准', en: 'Approve' },
  'work.itemAction.confirmApproveRun': { zh: '再次点按以批准此次运行', en: 'Tap again to approve this run' },
  'work.itemAction.confirmApprove': { zh: '再次点按以批准', en: 'Tap again to approve' },
  'work.itemAction.hintApprove': {
    zh: '安琪会通过现有的审批流程处理此项。高风险审批可能需要额外确认。',
    en: 'Aegis will route this through the existing approval path. High-risk approvals may ask for extra confirmation.',
  },
  'work.itemAction.skipRun': { zh: '跳过此次运行', en: 'Skip this run' },
  'work.itemAction.reject': { zh: '拒绝', en: 'Reject' },
  'work.itemAction.confirmSkipRun': { zh: '再次点按以跳过此次运行', en: 'Tap again to skip this run' },
  'work.itemAction.confirmReject': { zh: '再次点按以拒绝', en: 'Tap again to reject' },
  'work.itemAction.hintReject': {
    zh: '安琪会通过现有的审批流程处理此项，并停止此审批请求。',
    en: 'Aegis will route this through the existing approval path and stop this approval request.',
  },
  'work.itemAction.keepUsing': { zh: '继续使用', en: 'Keep using' },
  'work.itemAction.confirmKeepUsing': { zh: '再次点按以继续使用', en: 'Tap again to keep using' },
  'work.itemAction.hintKeepUsing': {
    zh: '安琪会刷新此记忆审核并留下记录。',
    en: 'Aegis will refresh this memory review and record a receipt.',
  },
  'work.itemAction.unpin': { zh: '取消置顶', en: 'Unpin' },
  'work.itemAction.pin': { zh: '置顶', en: 'Pin' },
  'work.itemAction.confirmUnpin': { zh: '再次点按以取消置顶', en: 'Tap again to unpin' },
  'work.itemAction.confirmPin': { zh: '再次点按以置顶', en: 'Tap again to pin' },
  'work.itemAction.hintPin': {
    zh: '安琪会更新记忆置顶状态并留下记录。',
    en: 'Aegis will update the memory pin and record a receipt.',
  },
  'work.itemAction.markWrong': { zh: '标为错误', en: 'Mark wrong' },
  'work.itemAction.confirmMarkWrong': { zh: '再次点按以标为错误', en: 'Tap again to mark wrong' },
  'work.itemAction.hintMarkWrong': {
    zh: '安琪会停止依赖此记忆并留下记录。',
    en: 'Aegis will stop relying on this memory and record a receipt.',
  },
  'work.itemAction.forget': { zh: '忘记', en: 'Forget' },
  'work.itemAction.confirmForget': { zh: '再次点按以忘记', en: 'Tap again to forget' },
  'work.itemAction.hintForget': {
    zh: '安琪会删除此记忆并留下记录。',
    en: 'Aegis will delete this memory and record a receipt.',
  },
  'work.itemAction.done': { zh: '完成', en: 'Done' },
  'work.itemAction.confirmCloseFollowUp': { zh: '再次点按以关闭此跟进', en: 'Tap again to close this follow-up' },
  'work.itemAction.hintCloseWaitingLoop': {
    zh: '安琪会结束这件等待中的事并留下记录。',
    en: 'Aegis will close this waiting loop and record a receipt.',
  },
  'work.itemAction.reviewTomorrow': { zh: '明天再看', en: 'Review tomorrow' },
  'work.itemAction.confirmTomorrow': { zh: '再次点按以推到明天', en: 'Tap again for tomorrow' },
  'work.itemAction.hintKeepVisibleTomorrow': {
    zh: '安琪会让此项明天保持可见，并记录此变更。',
    en: 'Aegis will keep this visible tomorrow and record the change.',
  },
  'work.itemAction.closeWithoutRunning': { zh: '关闭且不运行', en: 'Close without running' },
  'work.itemAction.confirmCloseWithoutRunning': {
    zh: '再次点按以关闭且不运行',
    en: 'Tap again to close without running',
  },
  'work.itemAction.hintCloseWithoutRunning': {
    zh: '安琪会将此已暂停的任务标记为完成，且不运行被阻止的步骤。',
    en: 'Aegis will mark this paused task done without running the blocked step.',
  },
  'work.itemAction.hintKeepPausedTomorrow': {
    zh: '安琪会让此项保持暂停，并在明天带着记录将其带回。',
    en: 'Aegis will keep this paused and bring it back tomorrow with a receipt.',
  },
  'work.itemAction.complete': { zh: '完成', en: 'Complete' },
  'work.itemAction.endThisOnce': { zh: '只结束这一次', en: 'End only this time' },
  'work.itemAction.confirmEndThisOnce': { zh: '再次点按，只结束这一次', en: 'Tap again to end only this time' },
  'work.itemAction.hintSkipOccurrence': {
    zh: '这次先跳过。跟踪保持开启，下次到点还会出现。安琪会留下记录。',
    en: 'Skip this time. Tracking stays on and returns at the next fire. Aegis records a receipt.',
  },
  'work.itemAction.endThisCard': { zh: '结束这一次', en: 'End this card' },
  'work.itemAction.confirmEndThisCard': { zh: '再次点按，结束这一次', en: 'Tap again to end this card' },
  'work.itemAction.closeTracking': { zh: '关闭整条跟踪', en: 'Close this tracking' },
  'work.itemAction.confirmCloseTracking': { zh: '再次点按，关闭整条跟踪', en: 'Tap again to close this tracking' },
  'work.itemAction.hintCloseTracking': {
    zh: '之后不会再新建卡片。已有记录会留下。',
    en: 'Later fires will not create new cards. History stays.',
  },
  'work.lineage.heading': { zh: '来源', en: 'Sources' },
  'work.lineage.layer.occurrence': { zh: '这一次', en: 'This time' },
  'work.lineage.layer.intent': { zh: '整条跟踪', en: 'Whole tracking' },
  'work.lineage.layer.origin': { zh: '更早的来源', en: 'Earlier source' },
  'work.lineage.intent.recurring': { zh: '循环跟踪', en: 'Recurring tracking' },
  'work.lineage.intent.watch': { zh: '持续守望', en: 'Standing watch' },
  'work.lineage.intent.oneshot': { zh: '一次性安排', en: 'One-time intent' },
  'work.lineage.intent.parent_goal': { zh: '上层目标', en: 'Parent goal' },
  'work.lineage.occurrence.card': { zh: '这张卡片', en: 'This card' },
  'work.lineage.occurrence.waiting': { zh: '这条等待', en: 'This waiting note' },
  'work.lineage.occurrence.goal': { zh: '这个目标', en: 'This goal' },
  'work.lineage.origin.context': { zh: '当时的说明', en: 'Original instruction' },
  'work.lineage.origin.message': { zh: '对话消息', en: 'Chat message' },
  'work.lineage.openMessage': { zh: '打开对话里的 {ref}', en: 'Open {ref} in chat' },
  'work.lineage.origin.record': { zh: '记录', en: 'Record' },
  'work.success.trackingClosed': { zh: '整条跟踪已关闭。记录还在。', en: 'Tracking is closed. History stays.' },
  'work.success.occurrenceSkipped': {
    zh: '这一次已跳过。跟踪还在，下次还会出现。',
    en: 'Skipped this time. Tracking stays and returns at the next fire.',
  },
  'work.actionLabel.end_this_once': { zh: '结束这一次', en: 'End this card' },
  'work.actionLabel.close_tracking': { zh: '关闭整条跟踪', en: 'Close this tracking' },
  'work.actionLabel.skip_occurrence': { zh: '只结束这一次', en: 'End only this time' },
  'work.itemAction.confirmComplete': { zh: '再次点按以完成', en: 'Tap again to complete' },
  'work.itemAction.hintTaskCompletion': { zh: '安琪会记录任务完成记录。', en: 'Aegis will record a task completion receipt.' },
  'work.itemAction.snooze': { zh: '稍后提醒', en: 'Snooze' },
  'work.itemAction.hintSnoozeTomorrow': {
    zh: '安琪会记录推迟到明天的记录。',
    en: 'Aegis will record a snooze-to-tomorrow receipt.',
  },

  // model.ts — buildCommandSuggestions (general + per-item quick prompts).
  'work.suggest.general.missing.label': { zh: '我遗漏了什么？', en: 'What am I missing?' },
  'work.suggest.general.missing.prompt': { zh: '我现在遗漏了什么？', en: 'What am I missing right now?' },
  'work.suggest.general.today.label': { zh: '回顾今天', en: 'Review today' },
  'work.suggest.general.today.prompt': {
    zh: '回顾今天，并告诉我哪些值得关注。',
    en: 'Review today and tell me what deserves attention.',
  },
  'work.suggest.general.followups.label': { zh: '检查跟进', en: 'Check follow-ups' },
  'work.suggest.general.followups.prompt': {
    zh: '检查我的跟进待办，并告诉我哪些已陈旧或在等待。',
    en: 'Check my follow-ups and tell me what is stale or waiting.',
  },
  'work.suggest.item.next.label': { zh: '我该做什么？', en: 'What should I do?' },
  'work.suggest.item.next.prompt': { zh: '我该如何处理这件事？', en: 'What should I do with this?' },
  'work.suggest.item.draft.label': { zh: '起草回复', en: 'Draft reply' },
  'work.suggest.item.draft.prompt': {
    zh: '为此事起草一封简洁、温暖的回复。',
    en: 'Draft a concise, warm reply for this.',
  },
  'work.suggest.item.track.label': { zh: '跟踪回复', en: 'Track response' },
  'work.suggest.item.track.prompt': {
    zh: '跟踪对方是否回复，如果没有则提醒我。',
    en: 'Track whether they respond and remind me if they do not.',
  },
  'work.suggest.item.tomorrow.label': { zh: '明天提醒', en: 'Remind tomorrow' },
  'work.suggest.item.tomorrow.prompt': {
    zh: '如果这件事明天还没处理，提醒我一下。',
    en: 'Remind me tomorrow if this is not handled.',
  },
  'work.suggest.item.breakdown.label': { zh: '拆解步骤', en: 'Break it down' },
  'work.suggest.item.breakdown.prompt': {
    zh: '把这件事拆解成接下来的具体步骤。',
    en: 'Break this into the next concrete steps.',
  },
  'work.suggest.item.trackThis.label': { zh: '跟踪此事', en: 'Track this' },
  'work.suggest.item.trackThis.prompt': { zh: '跟踪此事直到解决。', en: 'Track this until it is resolved.' },
  'work.suggest.item.moveLater.label': { zh: '移到稍后', en: 'Move later' },
  'work.suggest.item.moveLater.prompt': {
    zh: '把这件事移到待办积压，除非现在需要我处理。',
    en: 'Move this to backlog unless it needs me now.',
  },

  // presentation.ts — ownerMetaStatusLabel + actionLabel + compactOwnerList.
  'work.meta.completed': { zh: '已完成', en: 'Completed' },
  'work.meta.failed': { zh: '没做成', en: "Didn't finish" },
  'work.meta.error': { zh: '出错了', en: 'Error' },
  'work.meta.unread': { zh: '未读', en: 'Unread' },
  'work.list.andMore': { zh: '{shown}，以及另外 {count} 项', en: '{shown} and {count} more' },
  // Priority {level} word — server urgency is English; localize the level so
  // the zh "{level}优先级" template does not embed "High"/"Urgent".
  'work.meta.level.high': { zh: '高', en: 'High' },
  'work.meta.level.urgent': { zh: '紧急', en: 'Urgent' },
  'work.meta.level.low': { zh: '低', en: 'Low' },

  // adapters.ts — actionSuccessMessage.
  'work.success.task': { zh: '任务已更新。安琪已记录。', en: 'Task updated. Aegis recorded the receipt.' },
  'work.success.approval': {
    zh: '审批决定已通过现有审批流程发送。',
    en: 'Approval decision sent through the existing approval path.',
  },
  'work.success.memory': { zh: '记忆已更新。安琪已记录。', en: 'Memory updated. Aegis recorded the receipt.' },
  'work.success.generic': { zh: '完成。安琪已记录。', en: 'Done. Aegis recorded the receipt.' },

  // notificationActions.ts — deliveryNotice action labels + detail strings.
  'work.delivery.approve': { zh: '批准', en: 'Approve' },
  'work.delivery.reject': { zh: '拒绝', en: 'Reject' },
  'work.delivery.retry': { zh: '重试', en: 'Retry' },
  'work.delivery.delete': { zh: '删除', en: 'Delete' },
  'work.delivery.later': { zh: '稍后', en: 'Later' },
  'work.delivery.dismiss': { zh: '忽略', en: 'Dismiss' },
  'work.delivery.never': { zh: '永不', en: 'Never' },
  'work.delivery.cancel': { zh: '取消', en: 'Cancel' },
  'work.delivery.useful': { zh: '有用', en: 'Useful' },
  'work.delivery.useless': { zh: '没用', en: 'Not useful' },
  'work.delivery.done': { zh: '完成', en: 'Done' },
  'work.delivery.snooze': { zh: '稍后提醒', en: 'Snooze' },
  'work.delivery.tomorrow': { zh: '明天', en: 'Tomorrow' },
  'work.delivery.run': { zh: '运行', en: 'Run' },
  'work.delivery.rebuildMemoryIndex': { zh: '重建记忆索引', en: 'Rebuild memory index' },
  'work.delivery.target.reminder': { zh: '提醒', en: 'Reminder' },
  'work.delivery.detail.hour1': { zh: '1 小时', en: '1h' },
  'work.delivery.detail.tomorrow': { zh: '明天', en: 'tomorrow' },
  'work.delivery.aegisAction': { zh: '安琪操作', en: 'Aegis action' },
};

/**
 * Resolves a catalog key for the given locale.
 *
 * Fallback chain: requested locale → `en` → an explicit `[MISSING:<key>]`
 * sentinel (so an un-migrated key is visible in the UI rather than rendering as
 * an empty string).
 */
/**
 * The assistant's call-name (owner decision 2026-09-08: 安琪 / Angie by
 * default; the `assistant_display_name` setting overrides both). Catalog
 * strings address her through the `{assistant}` placeholder, which `t()`
 * resolves for every caller. "Aegis" remains the system name.
 */
const DEFAULT_ASSISTANT_NAME: Record<Locale, string> = { zh: '安琪', en: 'Angie' };
let assistantNameOverride: string | null = null;

export function setAssistantName(name: string | null | undefined): void {
  const trimmed = (name ?? '').trim();
  assistantNameOverride = trimmed ? trimmed : null;
}

export function assistantName(locale: Locale): string {
  if (assistantNameOverride && assistantNameOverride !== DEFAULT_ASSISTANT_NAME.zh) {
    return assistantNameOverride;
  }
  return DEFAULT_ASSISTANT_NAME[locale] ?? DEFAULT_ASSISTANT_NAME.zh;
}

export function t(key: string, locale: Locale): string {
  const raw = CATALOG[key]?.[locale] ?? CATALOG[key]?.en ?? `[MISSING:${key}]`;
  return raw.includes('{assistant}') ? raw.split('{assistant}').join(assistantName(locale)) : raw;
}

/**
 * Membership check for structural-code resolution (backend `*_code` fields).
 * Callers use this to distinguish "known code → localized text" from "unknown
 * code → fall back to backend prose" WITHOUT ever surfacing the `[MISSING:…]`
 * sentinel in production render paths.
 */
export function catalogHas(key: string): boolean {
  return Object.prototype.hasOwnProperty.call(CATALOG, key);
}

/**
 * Resolves a catalog key and interpolates `{name}` placeholders, mirroring the
 * backend `tf(key, locale, params)`. Unknown placeholders are left untouched so
 * a missing param is visible rather than silently blanked.
 */
export function tf(key: string, locale: Locale, params: Record<string, string>): string {
  const template = t(key, locale);
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    Object.prototype.hasOwnProperty.call(params, name) ? params[name] : match,
  );
}

/**
 * Normalizes a raw locale string (e.g. the backend `display_language` value, or
 * a Telegram `language_code`) into a supported {@link Locale}. Anything starting
 * with `en` (case-insensitive) is English; everything else defaults to `zh`,
 * matching the backend default.
 */
export function normalizeLocale(raw?: string): Locale {
  return raw && raw.toLowerCase().startsWith('en') ? 'en' : 'zh';
}

/**
 * Locale for text produced outside a React render (persisted history,
 * command dispatch): the document language the app stamps on switch, else
 * the browser language. Chinese-first, like the console.
 */
export function preferredLocale(): Locale {
  try {
    const lang = (typeof document !== 'undefined' && document.documentElement.lang) || '';
    if (lang) return lang.toLowerCase().startsWith('en') ? 'en' : 'zh';
    const nav = (typeof navigator !== 'undefined' && navigator.language) || '';
    return nav.toLowerCase().startsWith('en') ? 'en' : 'zh';
  } catch {
    return 'zh';
  }
}
