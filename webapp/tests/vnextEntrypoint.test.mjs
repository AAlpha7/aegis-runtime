import assert from 'node:assert/strict';
import { readFileSync, statSync } from 'node:fs';
import { describe, it } from 'node:test';

const mainSource = readFileSync(new URL('../src/main.tsx', import.meta.url), 'utf8');
const indexSource = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const manifestSource = readFileSync(new URL('../public/manifest.webmanifest', import.meta.url), 'utf8');
const serviceWorkerSource = readFileSync(new URL('../public/service-worker.js', import.meta.url), 'utf8');
const serviceWorkerRegistrationSource = readFileSync(new URL('../src/vnext/serviceWorker.ts', import.meta.url), 'utf8');
const webPushSource = readFileSync(new URL('../src/vnext/webPush.ts', import.meta.url), 'utf8');
const manifest = JSON.parse(manifestSource);

describe('vNext entrypoint', () => {
  it('uses vNext as the only owner-facing entrypoint', () => {
    assert.match(mainSource, /render\(<VNextApp \/>/);
    assert.doesNotMatch(mainSource, /const useLegacy =/);
    assert.doesNotMatch(mainSource, /search\.get\('legacy'\) === '1'/);
    assert.doesNotMatch(mainSource, /window\.location\.pathname\.startsWith\('\/ui\/legacy'\)/);
    assert.doesNotMatch(mainSource, /<App \/>/);
    assert.doesNotMatch(mainSource, /search\.get\('vnext'\) === '1'/);
    assert.doesNotMatch(mainSource, /window\.Telegram\?\.WebApp\?\.initData/);
    assert.doesNotMatch(indexSource, /<script[^>]+telegram\.org\/js\/telegram-web-app\.js/);
  });

  it('ships the document language zh-first (re-stamped at runtime by the locale provider)', () => {
    assert.match(indexSource, /<html lang="zh-CN">/);
    assert.doesNotMatch(indexSource, /<html lang="en">/);
  });

  it('installs cleanly as the owner mobile home surface', () => {
    assert.match(indexSource, /<link rel="manifest" href="\/ui\/manifest\.webmanifest" \/>/);
    assert.match(indexSource, /<meta name="theme-color" content="#f3f4f6" media="\(prefers-color-scheme: light\)" \/>/);
    assert.match(indexSource, /<meta name="theme-color" content="#0d1017" media="\(prefers-color-scheme: dark\)" \/>/);
    assert.match(indexSource, /<meta name="apple-mobile-web-app-status-bar-style" content="default" \/>/);
    assert.doesNotMatch(indexSource, /black-translucent/);
    assert.equal(manifest.theme_color, '#f3f4f6');
    assert.equal(manifest.background_color, '#f3f4f6');
    assert.match(indexSource, /<meta name="apple-mobile-web-app-capable" content="yes" \/>/);
    assert.match(indexSource, /<meta name="apple-mobile-web-app-title" content="Aegis" \/>/);
    assert.match(indexSource, /manifest-src 'self'/);
    assert.match(indexSource, /worker-src 'self'/);
    assert.equal(manifest.short_name, 'Aegis');
    assert.equal(manifest.start_url, '/ui/');
    assert.equal(manifest.scope, '/ui/');
    assert.equal(manifest.display, 'standalone');
    assert.equal(manifest.orientation, 'portrait-primary');
    assert.ok(manifest.icons.some((icon) => icon.src === '/ui/icon-192.png' && icon.sizes === '192x192' && icon.purpose.includes('maskable')));
    assert.ok(manifest.icons.some((icon) => icon.src === '/ui/icon-512.png' && icon.sizes === '512x512' && icon.purpose.includes('maskable')));
    assert.ok(manifest.shortcuts.some((shortcut) => shortcut.url === '/ui/?view=work'));
    assert.ok(statSync(new URL('../public/icon-192.png', import.meta.url)).size > 1024);
    assert.ok(statSync(new URL('../public/icon-512.png', import.meta.url)).size > 4096);
    assert.ok(statSync(new URL('../public/apple-touch-icon.png', import.meta.url)).size > 1024);
  });

  it('registers a cache-purging service worker without intercepting owner data', () => {
    assert.match(mainSource, /registerOwnerServiceWorker\(\)/);
    assert.match(serviceWorkerRegistrationSource, /updateViaCache: 'none'/);
    assert.match(serviceWorkerRegistrationSource, /registration\.update\(\)/);
    assert.match(serviceWorkerSource, /CACHE_PREFIX = 'aegis-owner-shell-'/);
    assert.match(serviceWorkerSource, /CACHE_NAME = 'aegis-owner-shell-v2'/);
    assert.match(serviceWorkerSource, /key\.startsWith\(CACHE_PREFIX\)/);
    assert.doesNotMatch(serviceWorkerSource, /addEventListener\('fetch'/);
    assert.doesNotMatch(serviceWorkerSource, /cache\.addAll/);
    assert.doesNotMatch(serviceWorkerSource, /caches\.match/);
    assert.doesNotMatch(serviceWorkerSource, /\/webapp\/vnext/);
    assert.doesNotMatch(serviceWorkerSource, /\/api\/session/);
  });

  it('handles Web Push notifications without caching owner data', () => {
    assert.match(serviceWorkerSource, /addEventListener\('push'/);
    assert.match(serviceWorkerSource, /showNotification/);
    assert.match(serviceWorkerSource, /addEventListener\('notificationclick'/);
    assert.match(serviceWorkerSource, /url\.startsWith\('\/ui\/'\)/);
    assert.doesNotMatch(serviceWorkerSource, /fetch\('\/webapp\/vnext/);
  });

  it('subscribes Web Push against the registered owner service worker', () => {
    assert.match(webPushSource, /register\('\/ui\/service-worker\.js'/);
    assert.match(webPushSource, /waitForServiceWorkerActive\(registration\)/);
    assert.match(webPushSource, /registration\.active/);
    assert.match(webPushSource, /registration\.pushManager\.subscribe/);
    assert.match(webPushSource, /controllerchange/);
    assert.doesNotMatch(webPushSource, /navigator\.serviceWorker\.ready/);
    assert.match(webPushSource, /pushSubscriptionUsesPublicKey/);
    assert.match(webPushSource, /existing\.unsubscribe\(\)/);
  });
});
