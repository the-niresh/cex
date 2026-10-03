import type { Session } from "./types";

/**
 * Every session is a guest session, kept in `localStorage`.
 *
 * There is no password to sign in with again - losing the token loses the
 * account - and all money here is demo money, so the token stays across tab
 * closes for 30 days.
 */
const KEY = "cex.session";

function read(raw: string | null): Session | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<Session>;
    if (typeof parsed.token !== "string" || typeof parsed.user_id !== "string") return null;
    return {
      token: parsed.token,
      user_id: parsed.user_id,
      name: typeof parsed.name === "string" ? parsed.name : null,
      is_guest: parsed.is_guest === true,
    };
  } catch {
    return null;
  }
}

export function loadSession(): Session | null {
  return read(localStorage.getItem(KEY));
}

export function saveSession(session: Session): void {
  localStorage.setItem(KEY, JSON.stringify(session));
}

export function clearSession(): void {
  localStorage.removeItem(KEY);
}
