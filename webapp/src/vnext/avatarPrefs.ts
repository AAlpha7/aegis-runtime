export type AvatarRole = 'aegis' | 'owner';

export type AvatarProfile = {
  aegisAvatarUrl: string;
  ownerAvatarUrl: string;
};

// The console is served under /ui/ (vite `base`), so the bundled asset lives
// there too; the bare root path was a 404 in production and every avatar fell
// back to the letter glyph. Profiles saved with the old path map to the default.
export const DEFAULT_AEGIS_AVATAR_URL = '/ui/aegis-avatar-v2.jpg';
export const LEGACY_AEGIS_AVATAR_URLS: readonly string[] = ['/aegis-avatar.svg', '/ui/aegis-avatar.svg'];
export const AVATAR_PREFS_BASE_KEY = 'aegis-runtime:vnext:avatar-profile';
export const MAX_AVATAR_DATA_URL_LENGTH = 700_000;

const ALLOWED_AVATAR_DATA_URL = /^data:image\/(?:png|jpeg|jpg|webp);base64,[a-z0-9+/=]+$/i;
const ALLOWED_GENERATED_AVATAR_URL = /^\/webapp\/vnext\/avatar-image\/[a-z0-9_.-]+\.(?:png|jpg|jpeg|webp)$/i;

export function defaultAvatarProfile(): AvatarProfile {
  return {
    aegisAvatarUrl: DEFAULT_AEGIS_AVATAR_URL,
    ownerAvatarUrl: '',
  };
}

export function avatarProfileOwnerKey(email?: string | null): string {
  const normalized = (email ?? '').trim().toLowerCase();
  if (!normalized) return `${AVATAR_PREFS_BASE_KEY}:owner`;
  const safeEmail = normalized.replace(/[^a-z0-9@._+-]/g, '_');
  return `${AVATAR_PREFS_BASE_KEY}:owner:${safeEmail}`;
}

export function isSafeAvatarUrl(value: unknown): value is string {
  if (typeof value !== 'string') return false;
  const trimmed = value.trim();
  if (!trimmed) return true;
  if (trimmed === DEFAULT_AEGIS_AVATAR_URL || LEGACY_AEGIS_AVATAR_URLS.includes(trimmed)) return true;
  if (ALLOWED_GENERATED_AVATAR_URL.test(trimmed)) return true;
  return trimmed.length <= MAX_AVATAR_DATA_URL_LENGTH && ALLOWED_AVATAR_DATA_URL.test(trimmed);
}

export function normalizeAvatarProfile(value: unknown): AvatarProfile {
  const fallback = defaultAvatarProfile();
  if (!value || typeof value !== 'object') return fallback;
  const raw = value as Partial<Record<keyof AvatarProfile, unknown>>;
  const rawAegis = isSafeAvatarUrl(raw.aegisAvatarUrl) ? raw.aegisAvatarUrl.trim() : '';
  const aegisAvatarUrl = rawAegis && !LEGACY_AEGIS_AVATAR_URLS.includes(rawAegis)
    ? rawAegis
    : fallback.aegisAvatarUrl;
  const ownerAvatarUrl = isSafeAvatarUrl(raw.ownerAvatarUrl) ? raw.ownerAvatarUrl.trim() : '';
  return { aegisAvatarUrl, ownerAvatarUrl };
}

export function avatarProfileFromServer(value: {
  aegis_avatar_url?: unknown;
  owner_avatar_url?: unknown;
}): AvatarProfile {
  return normalizeAvatarProfile({
    aegisAvatarUrl: value.aegis_avatar_url,
    ownerAvatarUrl: value.owner_avatar_url,
  });
}

export function avatarProfileToServer(profile: AvatarProfile): {
  aegis_avatar_url: string;
  owner_avatar_url: string;
} {
  const normalized = normalizeAvatarProfile(profile);
  return {
    aegis_avatar_url: normalized.aegisAvatarUrl,
    owner_avatar_url: normalized.ownerAvatarUrl,
  };
}

export function readAvatarProfile(storage: Storage, key: string): AvatarProfile {
  try {
    const raw = storage.getItem(key);
    if (!raw) return defaultAvatarProfile();
    return normalizeAvatarProfile(JSON.parse(raw));
  } catch {
    return defaultAvatarProfile();
  }
}

export function writeAvatarProfile(storage: Storage, key: string, profile: AvatarProfile) {
  const normalized = normalizeAvatarProfile(profile);
  try {
    storage.setItem(key, JSON.stringify(normalized));
  } catch {
    // Avatar customization is best-effort local display state.
  }
}

export function ownerInitial(email?: string | null): string {
  const normalized = (email ?? '').trim();
  if (!normalized) return 'Y';
  return normalized.charAt(0).toUpperCase();
}
