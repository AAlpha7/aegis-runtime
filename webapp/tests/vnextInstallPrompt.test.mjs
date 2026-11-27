import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_INSTALL_PROMPT_MODULE;
assert.ok(modulePath, 'VNEXT_INSTALL_PROMPT_MODULE must be set');
const installPrompt = await import(modulePath);

describe('vNext owner install prompt helpers', () => {
  it('prioritizes already-installed standalone signals over browser prompts', () => {
    assert.equal(installPrompt.ownerInstallStatusFromSignals({
      promptAvailable: true,
      displayModeStandalone: true,
      navigatorStandalone: false,
    }), 'installed');
    assert.equal(installPrompt.ownerInstallStatusFromSignals({
      promptAvailable: true,
      displayModeStandalone: false,
      navigatorStandalone: true,
    }), 'installed');
  });

  it('uses the native prompt when the browser exposes one', () => {
    assert.equal(installPrompt.ownerInstallStatusFromSignals({
      promptAvailable: true,
      displayModeStandalone: false,
      navigatorStandalone: false,
    }), 'available');
    assert.equal(installPrompt.ownerInstallActionLabel('available'), 'Install app');
  });

  it('falls back to concise device instructions without exposing a new product page', () => {
    assert.equal(installPrompt.ownerInstallStatusFromSignals({
      promptAvailable: false,
      displayModeStandalone: false,
      navigatorStandalone: false,
    }), 'manual');
    assert.match(installPrompt.ownerInstallManualNotice('Mozilla/5.0 (iPhone)'), /Share, then Add to Home Screen/);
    assert.match(installPrompt.ownerInstallManualNotice('Mozilla/5.0 (X11; Linux x86_64)'), /browser menu/);
    assert.equal(installPrompt.ownerInstallActionLabel('manual'), 'Add to Home Screen');
  });

  it('reports prompt outcomes without implying external work completed', () => {
    assert.match(installPrompt.ownerInstallOutcomeNotice({ outcome: 'accepted' }), /install started/i);
    assert.match(installPrompt.ownerInstallOutcomeNotice({ outcome: 'dismissed' }), /dismissed/i);
  });

  it('localizes the labels/notices by locale (en byte-identical, zh switches)', () => {
    // en default stays byte-identical to the prior hardcoded strings.
    assert.equal(installPrompt.ownerInstallActionLabel('installed', 'en'), 'Installed on this device');
    assert.equal(installPrompt.ownerInstallActionLabel('available', 'en'), 'Install app');
    assert.equal(installPrompt.ownerInstallActionLabel('manual', 'en'), 'Add to Home Screen');
    assert.equal(
      installPrompt.ownerInstallManualNotice('Mozilla/5.0 (iPhone)', 'en'),
      'Use Share, then Add to Home Screen. Aegis will open like a private app on this device.',
    );
    assert.equal(
      installPrompt.ownerInstallOutcomeNotice({ outcome: 'dismissed' }, 'en'),
      'Install dismissed. You can add Aegis from this menu later.',
    );
    // zh actually switches.
    assert.equal(installPrompt.ownerInstallActionLabel('available', 'zh'), '安装应用');
    assert.match(installPrompt.ownerInstallManualNotice('Mozilla/5.0 (iPhone)', 'zh'), /分享/);
    assert.match(installPrompt.ownerInstallManualNotice('Mozilla/5.0 (X11; Linux)', 'zh'), /浏览器菜单/);
    assert.match(installPrompt.ownerInstallOutcomeNotice({ outcome: 'accepted' }, 'zh'), /安装已开始/);
  });
});
