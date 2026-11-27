import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_AVATAR_PREFS_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-avatar-prefs-test/avatarPrefs.js', import.meta.url).href;
const {
  DEFAULT_AEGIS_AVATAR_URL,
  avatarProfileOwnerKey,
  avatarProfileFromServer,
  avatarProfileToServer,
  defaultAvatarProfile,
  isSafeAvatarUrl,
  normalizeAvatarProfile,
  ownerInitial,
  readAvatarProfile,
  writeAvatarProfile,
} = await import(modulePath);

function memoryStorage(initial = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (key) => map.get(key) ?? null,
    setItem: (key, value) => map.set(key, value),
    removeItem: (key) => map.delete(key),
  };
}

describe('vNext avatar preferences', () => {
  it('keeps a trusted generated Aegis avatar as the default', () => {
    // Served under the /ui/ mount; the bare root path 404s in production.
    assert.equal(DEFAULT_AEGIS_AVATAR_URL, '/ui/aegis-avatar-v2.jpg');
    assert.deepEqual(defaultAvatarProfile(), {
      aegisAvatarUrl: '/ui/aegis-avatar-v2.jpg',
      ownerAvatarUrl: '',
    });
  });

  it('scopes avatar settings by signed-in owner email', () => {
    assert.equal(avatarProfileOwnerKey('L@example.com'), 'aegis-runtime:vnext:avatar-profile:owner:l@example.com');
    assert.equal(avatarProfileOwnerKey(' weird owner '), 'aegis-runtime:vnext:avatar-profile:owner:weird_owner');
  });

  it('accepts only local image data urls or the built-in Aegis asset', () => {
    assert.equal(isSafeAvatarUrl('/ui/aegis-avatar-v2.jpg'), true);
    assert.equal(isSafeAvatarUrl('/aegis-avatar.svg'), true);
    assert.equal(isSafeAvatarUrl('/webapp/vnext/avatar-image/aegis-owner-123.png'), true);
    assert.equal(isSafeAvatarUrl('data:image/png;base64,abcd='), true);
    assert.equal(isSafeAvatarUrl('https://tracker.example/avatar.png'), false);
    assert.equal(isSafeAvatarUrl('/webapp/vnext/avatar-image/../secret.png'), false);
    assert.equal(isSafeAvatarUrl('data:image/svg+xml;base64,abcd='), false);
    assert.equal(isSafeAvatarUrl('data:image/gif;base64,abcd='), false);
    assert.equal(isSafeAvatarUrl('javascript:alert(1)'), false);
  });

  it('normalizes corrupt profiles without preserving unsafe urls', () => {
    assert.deepEqual(normalizeAvatarProfile({
      aegisAvatarUrl: 'https://example.com/a.png',
      ownerAvatarUrl: 'data:image/webp;base64,abcd=',
    }), {
      aegisAvatarUrl: '/ui/aegis-avatar-v2.jpg',
      ownerAvatarUrl: 'data:image/webp;base64,abcd=',
    });
    assert.equal(normalizeAvatarProfile({ aegisAvatarUrl: '/ui/aegis-avatar.svg' }).aegisAvatarUrl, DEFAULT_AEGIS_AVATAR_URL);
    // A profile saved before 2026-09-11 carries the old root path.
    assert.deepEqual(normalizeAvatarProfile({ aegisAvatarUrl: '/aegis-avatar.svg', ownerAvatarUrl: '' }), {
      aegisAvatarUrl: '/ui/aegis-avatar-v2.jpg',
      ownerAvatarUrl: '',
    });
  });

  it('round-trips safe local preferences through storage', () => {
    const storage = memoryStorage();
    const key = avatarProfileOwnerKey('owner@example.com');
    const profile = {
      aegisAvatarUrl: 'data:image/jpeg;base64,abcd=',
      ownerAvatarUrl: 'data:image/png;base64,efgh=',
    };
    writeAvatarProfile(storage, key, profile);
    assert.deepEqual(readAvatarProfile(storage, key), profile);
  });

  it('maps server snake_case profiles through the same safety normalization', () => {
    const profile = avatarProfileFromServer({
      aegis_avatar_url: 'data:image/jpeg;base64,YWVnaXM=',
      owner_avatar_url: 'data:image/png;base64,b3duZXI=',
    });
    assert.deepEqual(profile, {
      aegisAvatarUrl: 'data:image/jpeg;base64,YWVnaXM=',
      ownerAvatarUrl: 'data:image/png;base64,b3duZXI=',
    });
    assert.deepEqual(avatarProfileToServer(profile), {
      aegis_avatar_url: 'data:image/jpeg;base64,YWVnaXM=',
      owner_avatar_url: 'data:image/png;base64,b3duZXI=',
    });
    assert.deepEqual(avatarProfileFromServer({
      aegis_avatar_url: 'https://tracker.example/a.png',
      owner_avatar_url: 'javascript:alert(1)',
    }), defaultAvatarProfile());
  });

  it('uses a stable owner initial fallback', () => {
    assert.equal(ownerInitial('owner@example.com'), 'O');
    assert.equal(ownerInitial(''), 'Y');
  });
});
