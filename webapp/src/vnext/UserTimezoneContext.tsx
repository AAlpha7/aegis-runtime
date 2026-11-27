import { createContext, useContext } from 'react';
import type { ReactNode } from 'react';

/**
 * The owner's configured IANA timezone (e.g. `Asia/Shanghai`), sourced from
 * the settings projection's `user_timezone` field. `null` means "not known
 * yet / not configured" — consumers fall back to the browser timezone (the
 * historical behavior). This is the single thread by which all user-facing
 * time formatting picks up the owner's timezone instead of the browser's.
 */
const UserTimezoneContext = createContext<string | null>(null);

export function UserTimezoneProvider({
  value,
  children,
}: {
  value: string | null | undefined;
  children: ReactNode;
}) {
  return (
    <UserTimezoneContext.Provider value={value ?? null}>{children}</UserTimezoneContext.Provider>
  );
}

/**
 * Returns the owner's configured IANA timezone, or `null` when unknown.
 * Pass the result into the `formatInUserTz`-backed helpers; `null`/invalid
 * values transparently fall back to the browser timezone.
 */
export function useUserTimezone(): string | null {
  return useContext(UserTimezoneContext);
}
