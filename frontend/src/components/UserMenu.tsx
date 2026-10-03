import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import type { Session } from "../lib/types";

interface Props {
  session: Session;
  onDeposit(): void;
  onSignOut(): void;
}

export function UserMenu({ session, onDeposit, onSignOut }: Props) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const initial = (session.name ?? session.user_id).charAt(0).toUpperCase();

  useEffect(() => {
    function onDoc(event: MouseEvent) {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    }
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, []);

  return (
    <div className="relative" ref={root} data-testid="user-menu">
      <button
        type="button"
        className="flex min-h-8 cursor-pointer items-center gap-2 rounded-control px-1.5 hover:bg-hover"
        onClick={() => setOpen((v) => !v)}
        data-testid="user-menu-trigger"
      >
        <span
          className="flex size-7 flex-none items-center justify-center rounded-full bg-field font-sans text-micro font-medium text-ink"
          aria-hidden="true"
        >
          {initial}
        </span>
        {session.name && (
          <span className="max-w-[14ch] truncate font-sans text-micro text-ink" data-testid="account-name">
            {session.name}
          </span>
        )}
      </button>
      {open && (
        <div
          className="absolute right-0 top-[calc(100%+4px)] z-50 min-w-[140px] rounded-panel border border-rule bg-panel-hi py-1 shadow-lg"
          data-testid="user-menu-panel"
        >
          <Link
            to="/portfolio"
            className="block px-3 py-1.5 font-sans text-micro text-ink-2 hover:bg-hover hover:text-ink"
            onClick={() => setOpen(false)}
          >
            Portfolio
          </Link>
          <button
            type="button"
            className="block w-full px-3 py-1.5 text-left font-sans text-micro text-ink-2 hover:bg-hover hover:text-ink"
            onClick={() => {
              setOpen(false);
              onDeposit();
            }}
          >
            Deposit
          </button>
          <button
            type="button"
            className="block w-full px-3 py-1.5 text-left font-sans text-micro text-ink-4 hover:bg-hover hover:text-ink-2"
            onClick={() => {
              setOpen(false);
              onSignOut();
            }}
            data-testid="account-action"
          >
            Log out
          </button>
        </div>
      )}
    </div>
  );
}
