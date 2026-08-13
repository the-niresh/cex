import { useEffect, useState } from "react";
import type { AuthMode, Credentials } from "../lib/types";
import {
  Field,
  FieldLabel,
  FieldName,
  FieldRule,
  Segment,
  Segmented,
  SubmitButton,
  SunkenInput,
} from "./ui/form";
import { PanelHead, PanelTitle } from "./ui/panel";

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
    // Translucent on purpose: this only opens when someone tries to trade, and
    // the book behind it is public and still streaming while it is up.
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-bg/80 backdrop-blur-[2px]"
      data-testid="auth-panel"
    >
      {/* Clicking the dimmed area is the other thing people already try. The
          form sits above this, so its own clicks never reach it. */}
      <div className="absolute inset-0" onClick={onClose} aria-hidden="true" />
      <form
        className="relative flex w-80 flex-col rounded-panel border border-rule bg-panel"
        onSubmit={(e) => void submit(e)}
      >
        <PanelHead>
          <PanelTitle>cex · spot</PanelTitle>
          <button
            type="button"
            className="ml-auto cursor-pointer px-0.5 text-[11px] leading-none text-ink-4 hover:text-ink"
            aria-label="close"
            onClick={onClose}
          >
            ✕
          </button>
        </PanelHead>
        <div className="flex flex-col gap-2.5 p-3.5">
          <Segmented className="grid-cols-2" data-testid="auth-mode">
            <Segment selected={mode === "login"} onClick={() => setMode("login")}>
              LOG IN
            </Segment>
            <Segment selected={mode === "register"} onClick={() => setMode("register")}>
              REGISTER
            </Segment>
          </Segmented>

          {mode === "register" && (
            <Field>
              <FieldLabel>
                <FieldName>Name</FieldName>
                <FieldRule>what we call you</FieldRule>
              </FieldLabel>
              <SunkenInput
                value={name}
                autoComplete="name"
                aria-label="name"
                onChange={(e) => setName(e.target.value)}
              />
            </Field>
          )}

          <Field>
            <FieldLabel>
              <FieldName>Username</FieldName>
            </FieldLabel>
            <SunkenInput
              value={username}
              autoComplete="username"
              aria-label="username"
              onChange={(e) => setUsername(e.target.value)}
            />
          </Field>

          <Field>
            <FieldLabel>
              <FieldName>Password</FieldName>
              {/* What the API actually enforces (MIN_PASSWORD_LEN in
                  crates/api/src/users.rs). Stating a stricter rule than
                  anything checks just teaches people the labels here lie. */}
              <FieldRule>8+ characters</FieldRule>
            </FieldLabel>
            <SunkenInput
              type="password"
              value={password}
              autoComplete={mode === "register" ? "new-password" : "current-password"}
              aria-label="password"
              onChange={(e) => setPassword(e.target.value)}
            />
          </Field>

          {failure && (
            <div className="bg-sell/10 px-2 py-1.5 text-[10.5px] leading-[1.4] text-sell shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--color-sell)_35%,transparent)]">
              {failure}
            </div>
          )}

          <SubmitButton
            type="submit"
            data-testid="auth-submit"
            disabled={busy || !complete}
          >
            {busy ? "…" : mode === "register" ? "CREATE ACCOUNT" : "LOG IN"}
          </SubmitButton>

          <div className="font-sans text-micro leading-[1.5] text-ink-4">
            The token is held in memory and <code>sessionStorage</code> — it is gone when this tab
            closes. The book and the tape are public: close this and watch without an account.
          </div>
        </div>
      </form>
    </div>
  );
}
