import { useState } from 'react';
import { DEFAULT_AEGIS_AVATAR_URL, ownerInitial } from './avatarPrefs';

export function AegisAvatar({
  src = DEFAULT_AEGIS_AVATAR_URL,
  className = '',
}: {
  src?: string;
  className?: string;
}) {
  const [broken, setBroken] = useState(false);
  return (
    <span className={['vnext-avatar', 'aegis', className].filter(Boolean).join(' ')} aria-hidden="true">
      {broken ? <span>A</span> : (
        <img
          src={src || DEFAULT_AEGIS_AVATAR_URL}
          alt=""
          loading="eager"
          decoding="async"
          onError={() => setBroken(true)}
        />
      )}
    </span>
  );
}

export function OwnerAvatar({
  src,
  email,
  className = '',
}: {
  src?: string;
  email?: string | null;
  className?: string;
}) {
  const [broken, setBroken] = useState(false);
  return (
    <span className={['vnext-avatar', 'owner', className].filter(Boolean).join(' ')} aria-hidden="true">
      {src && !broken ? (
        <img src={src} alt="" loading="eager" decoding="async" onError={() => setBroken(true)} />
      ) : (
        <span>{ownerInitial(email)}</span>
      )}
    </span>
  );
}
