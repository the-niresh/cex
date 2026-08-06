import { useState } from "react";

interface Props {
  onSubmit(username: string, password: string, mode: "login" | "register"): Promise<void>;
}

export function Auth({ onSubmit }: Props) {
  const [mode, setMode] = useState<"register" | "login">("register");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [failure, setFailure] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setFailure(null);
    setBusy(true);
    try {
      await onSubmit(username.trim(), password, mode);
    } catch (e) {
      setFailure(e instanceof Error ? e.message : "could not sign in");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="auth">
      <form onSubmit={(e) => void submit(e)}>
        <div className="phead">
          <h2>cex · spot</h2>
          <span className="meta">test exchange</span>
        </div>
        <div className="body">
          <div className="seg kind">
            <button type="button" aria-selected={mode === "register"} onClick={() => setMode("register")}>
              REGISTER
            </button>
            <button type="button" aria-selected={mode === "login"} onClick={() => setMode("login")}>
              LOG IN
            </button>
          </div>

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
              <span className="rule">12+ characters</span>
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

          <button className="submit" type="submit" disabled={busy || !username || !password}>
            {busy ? "…" : mode === "register" ? "CREATE ACCOUNT" : "LOG IN"}
          </button>

          <div className="note">
            The token is held in memory and <code>sessionStorage</code> — it is gone when this tab
            closes. The book and the tape are public and are already streaming behind this.
          </div>
        </div>
      </form>
    </div>
  );
}
