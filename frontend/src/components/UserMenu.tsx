import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
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
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const [panelPos, setPanelPos] = useState({ top: 0, left: 0 });
  const initial = (session.name ?? session.user_id).charAt(0).toUpperCase();

  function placePanel() {
    const el = trigger.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setPanelPos({ top: rect.bottom + 4, left: rect.right });
  }

  useLayoutEffect(() => {
    if (!open) return;
    placePanel();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    function onDoc(event: MouseEvent) {
      const target = event.target as Node;
      if (root.current?.contains(target) || panel.current?.contains(target)) return;
      setOpen(false);
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    function onReposition() {
      placePanel();
    }
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    window.addEventListener("resize", onReposition);
    window.addEventListener("scroll", onReposition, true);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", onReposition);
      window.removeEventListener("scroll", onReposition, true);
    };
  }, [open]);

  return (
    <div className="relative" ref={root} data-testid="user-menu">
      <button
        ref={trigger}
        type="button"
        className="flex min-h-8 cursor-pointer items-center gap-2 rounded-control px-1.5 hover:bg-hover focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-control"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-haspopup="menu"
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
      {open &&
        createPortal(
          <div
            ref={panel}
            role="menu"
            className="fixed z-[100] min-w-[140px] rounded-panel border border-rule bg-panel-hi py-1 shadow-lg"
            style={{ top: panelPos.top, left: panelPos.left, transform: "translateX(-100%)" }}
            data-testid="user-menu-panel"
          >
            <Link
              to="/portfolio"
              role="menuitem"
              className="block px-3 py-1.5 font-sans text-micro text-ink-2 hover:bg-hover hover:text-ink focus-visible:bg-hover focus-visible:text-ink focus-visible:outline-none"
              onClick={() => setOpen(false)}
            >
              Portfolio
            </Link>
            <button
              type="button"
              role="menuitem"
              className="block w-full px-3 py-1.5 text-left font-sans text-micro text-ink-2 hover:bg-hover hover:text-ink focus-visible:bg-hover focus-visible:text-ink focus-visible:outline-none"
              onClick={() => {
                setOpen(false);
                onDeposit();
              }}
            >
              Deposit
            </button>
            <button
              type="button"
              role="menuitem"
              className="block w-full px-3 py-1.5 text-left font-sans text-micro text-ink-4 hover:bg-hover hover:text-ink-2 focus-visible:bg-hover focus-visible:text-ink-2 focus-visible:outline-none"
              onClick={() => {
                setOpen(false);
                onSignOut();
              }}
              data-testid="account-action"
            >
              Log out
            </button>
          </div>,
          document.body,
        )}
    </div>
  );
}
