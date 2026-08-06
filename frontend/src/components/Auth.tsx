import { useEffect, useState } from "react";
import type { AuthMode, Credentials } from "../lib/types";

interface Props {
  /** Which tab to open on. The caller decides, so a prompt can open on LOG IN. */
  initialMode?: AuthMode;
  onSubmit(mode: AuthMode, credentials: Credentials): Promise<void>;
  onClose(): void;
}

export function Auth({ initialMode = "login", onSubmit, onClose }: Props) {
  const [mode, setMode] = useState<AuthMode>(initialMode);
  const [username, setUsername] = useState("");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [failure, setFailure] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // This panel is dismissible: the book, the tape and the chart are public, so
  // nobody has to sign in to look. Escape is the shortcut people already try.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setFailure(null);
    setBusy(true);
    try {
      await onSubmit(mode, { username: username.trim(), name: name.trim(), password });
    } catch (e) {
      setFailure(e instanceof Error ? e.message : "could not sign in");
    } finally {
      setBusy(false);
    }
  }

  // Registering asks for a name; signing in does not, and requiring one there
  // would block anyone whose account predates the field.
  const complete = username !== "" && password !== "" && (mode === "login" || name.trim() !== "");

  return (
    <div className="auth">
      {/* Clicking the dimmed area is the other thing people already try. */}
      <div className="scrim" onClick={onClose} aria-hidden="true" />
      <form onSubmit={(e) => void submit(e)}>
        <div className="phead">
          <h2>cex · spot</h2>
          <button type="button" className="close" aria-label="close" onClick={onClose}>
            ✕
          </button>
        </div>
        <div className="body">
          <div className="seg kind">
            <button type="button" aria-selected={mode === "login"} onClick={() => setMode("login")}>
              LOG IN
            </button>
            <button
              type="button"
              aria-selected={mode === "register"}
              onClick={() => setMode("register")}
            >
              REGISTER
            </button>
          </div>

          {mode === "register" && (
            <div className="field">
              <div className="flabel">
                <span className="k">Name</span>
                <span className="rule">what we call you</span>
              </div>
              <div className="input">
                <input
                  value={name}
                  autoComplete="name"
                  aria-label="name"
                  onChange={(e) => setName(e.target.value)}
                />
              </div>
            </div>
          )}

          <div className="field">
            <div className="flabel">
              <span className="k">Username</span>
            </div>
            <div className="input">
              <input
                value={username}
                autoComplete="username"
                aria-label="username"
                onChange={(e) => setUsername(e.target.value)}
              />
            </div>
          </div>

          <div className="field">
            <div className="flabel">
              <span className="k">Password</span>
              {/* What the API actually enforces (MIN_PASSWORD_LEN in
                  crates/api/src/users.rs). Stating a stricter rule than
                  anything checks just teaches people the labels here lie. */}
              <span className="rule">8+ characters</span>
            </div>
            <div className="input">
              <input
                type="password"
                value={password}
                autoComplete={mode === "register" ? "new-password" : "current-password"}
                aria-label="password"
                onChange={(e) => setPassword(e.target.value)}
              />
            </div>
          </div>

          {failure && <div className="fail">{failure}</div>}

          <button className="submit" type="submit" disabled={busy || !complete}>
            {busy ? "…" : mode === "register" ? "CREATE ACCOUNT" : "LOG IN"}
          </button>

          <div className="note">
            The token is held in memory and <code>sessionStorage</code> — it is gone when this tab
            closes. The book and the tape are public: close this and watch without an account.
          </div>
        </div>
      </form>
    </div>
  );
}
