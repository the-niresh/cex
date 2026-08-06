import type { Session } from "./types";

/**
 * `sessionStorage`, never `localStorage`.
 *
 * A token in `localStorage` outlives the tab, survives a reboot, and is
 * readable by any script that ever manages to run on this origin. Session
 * storage is cleared when the tab closes, which is the right lifetime for a
 * bearer token that can move money.
 */
const KEY = "cex.session";

export function loadSession(): Session | null {
  try {
    const raw = sessionStorage.getItem(KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<Session>;
    if (typeof parsed.token !== "string" || typeof parsed.user_id !== "string") return null;
    // The name is a label, so a stored session that predates it is still a
    // valid session — missing it must not sign anyone out.
    return {
      token: parsed.token,
      user_id: parsed.user_id,
      name: typeof parsed.name === "string" ? parsed.name : null,
    };
  } catch {
    return null;
  }
}

export function saveSession(session: Session): void {
  sessionStorage.setItem(KEY, JSON.stringify(session));
}

export function clearSession(): void {
  sessionStorage.removeItem(KEY);
}
