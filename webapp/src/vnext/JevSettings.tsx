import { useEffect, useRef, useState } from 'react';
import { requestJevSettings } from '../api';
import type { JevStatus } from '../api';
import { useT } from '../i18n/LocaleContext';
import { VNextButton } from './VNextButton';
import './JevSettings.css';

export function JevSettings({ initData, csrfToken }: { initData: string; csrfToken?: string }) {
  const tr = useT();
  const [status, setStatus] = useState<JevStatus | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [key, setKey] = useState('');
  const [removeKey, setRemoveKey] = useState(false);
  const [consent, setConsent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const generation = useRef(0);
  function reset(next: JevStatus) {
    setStatus(next); setEnabled(next.enabled); setKey(''); setRemoveKey(false); setConsent(false);
  }
  async function load() {
    const current = ++generation.current;
    setBusy(true); setMessage('');
    try {
      const next = await requestJevSettings(initData, csrfToken);
      if (generation.current === current) reset(next);
    } catch {
      if (generation.current === current) setMessage('settings.jev.loadFailed');
    } finally {
      if (generation.current === current) setBusy(false);
    }
  }
  useEffect(() => {
    setStatus(null); setKey('');
    void load();
    return () => { generation.current += 1; };
    // A session change invalidates pending private responses and secret drafts.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initData, csrfToken]);
  const validKey = /^sk-or-\S{15,}$/.test(key.trim()) && !key.includes('xxxxxxxx') && key.trim().length <= 4096;
  const dirty = !!status && (enabled !== status.enabled || !!key || removeKey);
  const canSave = (dirty || (enabled && !status?.verified)) && !busy && (!key || validKey) && (!(enabled || key) || consent) && (!enabled || (validKey || (status?.key_present && !removeKey)));
  async function save() {
    if (!canSave) return;
    const current = generation.current;
    setBusy(true); setMessage('');
    try {
      const next = await requestJevSettings(initData, csrfToken, {
        enabled, consent, ...(key.trim() ? { api_key: key.trim() } : {}), remove_key: removeKey,
      });
      if (generation.current === current) { reset(next); setMessage('settings.jev.saved'); }
    } catch (error) {
      if (generation.current === current) { setKey(''); setMessage(error instanceof Error && /^(Validation failed:|验证失败：)/.test(error.message) ? error.message : 'settings.jev.saveFailed'); }
    } finally {
      if (generation.current === current) setBusy(false);
    }
  }
  return <section className="vnext-jev-card" aria-labelledby="jev-heading">
    <header><div><h3 id="jev-heading">{tr('settings.jev.title')}</h3><p>{tr('settings.jev.subtitle')}</p></div>
      <span className="vnext-jev-state">{tr(!status ? 'settings.jev.loading' : status.enabled && status.key_present ? (status.verified ? 'settings.jev.verified' : 'settings.jev.on') : status.enabled ? 'settings.jev.needsKey' : 'settings.jev.off')}</span>
    </header>
    <p className="vnext-settings-inline-note">{tr('settings.jev.disclosure')}</p>
    {!status ? <VNextButton variant="secondary" loading={busy} onClick={() => void load()}>{tr('settings.jev.reload')}</VNextButton> : <>
      <label className="vnext-jev-toggle"><input type="checkbox" role="switch" checked={enabled} disabled={busy} onChange={e => { setEnabled(e.target.checked); setConsent(false); setMessage(''); }} />{tr('settings.jev.enable')}</label>
      <p className="vnext-settings-inline-note">{tr('settings.jev.localRules')}</p>
      <label className="vnext-jev-key">{tr('settings.jev.key')}<input type="password" autoComplete="new-password" spellCheck={false} maxLength={4096} value={key} disabled={busy || removeKey} placeholder={tr(status.saved_key ? 'settings.jev.keepKey' : 'settings.jev.enterKey')} onChange={e => { setKey(e.target.value); setMessage(''); }} /></label>
      <p className="vnext-settings-inline-note">{tr(removeKey ? 'settings.jev.removing' : status.key_source === 'settings' ? 'settings.jev.stored' : status.key_source === 'environment' ? 'settings.jev.environment' : 'settings.jev.missing')}</p>
      {!!key && !validKey && <p role="alert">{tr('settings.jev.invalidKey')}</p>}
      {(enabled || key) && <label className="vnext-jev-consent"><input type="checkbox" checked={consent} disabled={busy} onChange={e => setConsent(e.target.checked)} />{tr('settings.jev.consent')}</label>}
      <div className="vnext-jev-actions">
        <VNextButton disabled={!canSave} loading={busy} onClick={() => void save()}>{tr(busy ? 'settings.validation.checking' : 'settings.jev.save')}</VNextButton>
        <VNextButton variant="ghost" disabled={!dirty || busy} onClick={() => { reset(status); setMessage(''); }}>{tr('settings.jev.cancel')}</VNextButton>
        <VNextButton variant="ghost" disabled={busy} onClick={() => void load()}>{tr('settings.jev.reload')}</VNextButton>
        {status.saved_key && <VNextButton variant="ghost" disabled={busy || removeKey} onClick={() => { setRemoveKey(true); setKey(''); setEnabled(false); setConsent(false); setMessage(''); }}>{tr('settings.jev.remove')}</VNextButton>}
      </div>
      <p className="vnext-settings-inline-note">{tr('settings.jev.scope')}</p>
    </>}
    <p role="status" aria-live="polite">{message ? tr(message) : ''}</p>
  </section>;
}
