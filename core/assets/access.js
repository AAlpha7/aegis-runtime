/* Browser-native WebAuthn. No third-party scripts, analytics or persisted secrets. */
const status = document.querySelector('#status');
let locale = navigator.language.toLowerCase().startsWith('zh') ? 'zh' : 'en';
const strings = {
  en: {
    eyebrow: 'YOUR PRIVATE WORKSPACE', title: 'Welcome back.',
    intro: "Use your device's face, fingerprint or screen lock to sign in with a passkey.",
    login: 'Sign in with a passkey', privacy: 'Your database key stays on your computer.',
    pairTitle: 'First time on this device?', pairLabel: 'Pairing code',
    pairHelp: 'Open a pairing link from the computer running Aegis, or enter its one-time code below. It expires in 10 minutes.',
    pairNote: 'Keep this code private. It is not your database key.', register: 'Create Passkey',
    console: 'Already signed in? Open your workspace', recoveryTitle: 'Need help signing in?',
    recoveryHelp: 'Lost your passkey? Recover access on the computer running Aegis, then pair this device again. A second passkey can help you avoid losing access.',
    revokeHelp: 'Signing out everywhere asks you to verify your passkey again. Your passkeys and data are kept.',
    revoke: 'Sign out all devices', footer: 'Your computer. Your workspace.',
    localHelp: 'This installation uses local sign-in. Open the private sign-in link from the Aegis launcher on your computer. No Google account setup is needed.',
    waiting: 'Follow the verification prompt on your device…', created: 'Passkey created. Select Sign in to continue.',
    revoked: 'All sessions signed out. Your passkeys and data are kept.', confirm: 'Sign out all devices? Your passkeys and data will be kept.',
    cancelled: 'Verification was cancelled or timed out. You can try signing in again. If pairing, request a new pairing link.',
    invalid: 'This pairing code is invalid, expired or already used. Request a new link from the computer running Aegis.',
    failed: 'Passkey verification failed. Try again. If access was reset on your computer, use a new pairing link to create a passkey.',
    network: 'Could not reach Aegis. Check your connection and try again. For private access, make sure Tailscale is connected.',
    unavailable: 'Passkeys require a supported browser and a secure connection. Open the HTTPS address from your Aegis installation.',
    slow: 'Too many attempts. Wait a minute, then try again.',
  },
  zh: {
    eyebrow: '你的私人工作区', title: '欢迎回来。', intro: '使用设备的面容、指纹或锁屏密码，通过通行密钥安全登录。',
    login: '使用通行密钥登录', privacy: '数据库密钥始终保留在你的电脑上。',
    pairTitle: '首次在这台设备上使用？', pairLabel: '配对码',
    pairHelp: '打开运行 Aegis 的电脑生成的配对链接，或在下方输入一次性配对码。配对码在 10 分钟后失效。',
    pairNote: '请妥善保管配对码，不要在此输入数据库密钥。', register: '创建通行密钥',
    console: '已经登录？进入工作区', recoveryTitle: '登录遇到困难？',
    recoveryHelp: '丢失通行密钥时，请在运行 Aegis 的电脑上恢复访问，然后重新配对这台设备。提前添加第二个通行密钥，可以避免失去访问权限。',
    revokeHelp: '退出所有设备前，需要再次验证通行密钥。通行密钥和数据将保留。',
    revoke: '退出所有设备', footer: '你的电脑，你的工作区。',
    localHelp: '当前安装使用本机登录。请从这台电脑上的 Aegis 启动入口打开私人登录链接，无需配置 Google 账户。',
    waiting: '请按设备提示完成验证…', created: '通行密钥已创建，请点击登录进入工作区。',
    revoked: '所有设备已退出登录，通行密钥和数据已保留。', confirm: '退出所有设备？通行密钥和数据将保留。',
    cancelled: '验证已取消或超时，可以重新尝试登录。如果正在配对，请获取新的配对链接。',
    invalid: '配对码无效、已过期或已使用，请在运行 Aegis 的电脑上获取新的配对链接。',
    failed: '通行密钥验证未通过，请重试。如果已在电脑上重置访问，请使用新的配对链接创建通行密钥。',
    network: '无法连接 Aegis，请检查连接后重试。使用私网访问时，请确认 Tailscale 已连接。',
    unavailable: '通行密钥需要受支持的浏览器和安全连接。请使用 Aegis 安装时提供的 HTTPS 地址。',
    slow: '尝试次数过多，请等待一分钟后重试。',
  },
};
let notice = '';
function announce(key, tone = '') {
  notice = key;
  status.textContent = key ? strings[locale][key] : '';
  status.dataset.tone = tone;
}
function renderLanguage() {
  document.documentElement.lang = locale === 'zh' ? 'zh-CN' : 'en';
  document.title = locale === 'zh' ? 'Aegis · 安全访问' : 'Aegis · Secure access';
  for (const element of document.querySelectorAll('[data-i18n]')) element.textContent = strings[locale][element.dataset.i18n];
  const toggle = document.querySelector('#language');
  toggle.textContent = locale === 'zh' ? 'EN' : '中文';
  toggle.setAttribute('aria-label', locale === 'zh' ? 'Switch to English' : '切换为中文');
  if (notice) status.textContent = strings[locale][notice];
}
document.querySelector('#language').onclick = () => { locale = locale === 'zh' ? 'en' : 'zh'; renderLanguage(); };
renderLanguage();
const decode = value => Uint8Array.from(atob(value.replace(/-/g, '+').replace(/_/g, '/')), c => c.charCodeAt(0));
const encode = value => btoa(String.fromCharCode(...new Uint8Array(value))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
async function api(action, body, csrf) {
  let response;
  try {
    response = await fetch(`/auth/passkey/${action}`, {method: 'POST', credentials: 'same-origin', headers: {'Content-Type': 'application/json', ...(csrf ? {'x-aegis-csrf': csrf} : {})}, body: JSON.stringify(body)});
  } catch {
    const error = new Error('Authentication service unreachable');
    error.notice = 'network';
    throw error;
  }
  if (!response.ok) {
    const error = new Error('Authentication request failed');
    error.notice = response.status === 429 ? 'slow' : response.status === 400 && action === 'register-start' ? 'invalid' : 'failed';
    throw error;
  }
  return response.json();
}
function options(json) {
  const key = json.publicKey;
  key.challenge = decode(key.challenge);
  if (key.user) key.user.id = decode(key.user.id);
  for (const item of key.allowCredentials || key.excludeCredentials || []) item.id = decode(item.id);
  return {publicKey: key};
}
function credentialJSON(credential) {
  const response = {clientDataJSON: encode(credential.response.clientDataJSON)};
  for (const key of ['attestationObject', 'authenticatorData', 'signature', 'userHandle']) {
    if (credential.response[key]) response[key] = encode(credential.response[key]);
  }
  if (credential.response.getTransports) response.transports = credential.response.getTransports();
  return {id: credential.id, rawId: encode(credential.rawId), type: credential.type, response, extensions: credential.getClientExtensionResults()};
}
async function login() {
  const start = await api('login-start', {});
  const credential = await navigator.credentials.get(options(start.options));
  if (!credential) throw new Error('Cancelled / 已取消');
  await api('login-finish', {challenge_id: start.challenge_id, credential: credentialJSON(credential)});
}
async function perform(action) {
  const buttons = [...document.querySelectorAll('button:not(#language)')];
  buttons.forEach(b => b.disabled = true);
  document.querySelector('.access-card').setAttribute('aria-busy', 'true');
  announce('waiting');
  try {
    if (!window.PublicKeyCredential || !window.isSecureContext) { const error = new Error(); error.notice = 'unavailable'; throw error; }
    await action();
  } catch (error) {
    announce(error.notice || (error.name === 'NotAllowedError' || /Cancelled/.test(error.message) ? 'cancelled' : 'failed'), 'error');
  } finally { buttons.forEach(b => b.disabled = false); document.querySelector('.access-card').setAttribute('aria-busy', 'false'); }
}
document.querySelector('#login').onclick = () => perform(async () => {await login(); location.assign('/ui/');});
document.querySelector('#register').onclick = () => perform(async () => {
  const input = document.querySelector('#pairing');
  const code = input.value.trim(); input.value = '';
  if (!/^[a-f0-9]{64}$/i.test(code)) { const error = new Error(); error.notice = 'invalid'; throw error; }
  const start = await api('register-start', {pairing_code: code});
  const credential = await navigator.credentials.create(options(start.options));
  if (!credential) throw new Error('Cancelled; generate a new pairing code / 已取消，请生成新的配对码');
  await api('register-finish', {challenge_id: start.challenge_id, credential: credentialJSON(credential)});
  announce('created', 'success');
});
document.querySelector('#revoke').onclick = () => perform(async () => {
  if (!confirm(strings[locale].confirm)) {announce(''); return;}
  await login();
  const response = await fetch('/api/session', {credentials: 'same-origin', cache: 'no-store'});
  const session = await response.json();
  await api('revoke-sessions', {}, session.csrf_token);
  announce('revoked', 'success');
});
function readPairingFragment() {
  if (!location.hash.startsWith('#pair=')) return;
  const code = location.hash.slice(6);
  history.replaceState(null, '', location.pathname);
  if (/^[a-f0-9]{64}$/.test(code)) { document.querySelector('#pairing').value = code; document.querySelector('#pair-panel').open = true; }
}
window.addEventListener('hashchange', () => { readPairingFragment(); refreshAccessMode(); });
readPairingFragment();
function refreshAccessMode() {
  fetch('/api/session', {credentials: 'same-origin', cache: 'no-store'}).then(response => response.json()).then(session => {
    const local = session.passkey_login_enabled === false;
    document.querySelector('#passkey-actions').hidden = local;
    document.querySelector('.recovery').hidden = local;
    document.querySelector('#local-help').hidden = !local;
    document.querySelector('.intro').hidden = local;
  }).catch(() => { /* The visible login action reports connection failures. */ });
}
refreshAccessMode();
