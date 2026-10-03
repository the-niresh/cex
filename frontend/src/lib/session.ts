import type { Session } from "./types";

/**
 * Registered users use `sessionStorage`; guests use `localStorage`.
 *
 * A registered account can sign in again with a password, so a token that dies
 * when the tab closes is the right trade-off. A guest has no password - losing
 * the token loses the account - and all money here is demo money, so guests
 * keep their token across tab closes for 30 days.
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
  return read(localStorage.getItem(KEY)) ?? read(sessionStorage.getItem(KEY));
}

export function saveSession(session: Session): void {
  const payload = JSON.stringify(session);
  if (session.is_guest) {
    localStorage.setItem(KEY, payload);
    sessionStorage.removeItem(KEY);
  } else {
    sessionStorage.setItem(KEY, payload);
    localStorage.removeItem(KEY);
  }
}

export function clearSession(): void {
  sessionStorage.removeItem(KEY);
  localStorage.removeItem(KEY);
}
