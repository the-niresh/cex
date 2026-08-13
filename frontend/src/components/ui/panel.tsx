import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * The chrome every data panel shares: a surface, a head that names it, column
 * headings, and a scroller.
 *
 * These were eight near-identical CSS rules that each component opted into by
 * class name. As components they carry their own contract, so a panel cannot
 * quietly end up with a head and no border, and the hierarchy below is decided
 * in one place rather than eight.
 */

export function Panel({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.HTMLAttributes<HTMLElement>) {
  return (
    <section
      className={cn(
        "flex min-w-0 flex-col overflow-hidden rounded-panel border border-rule bg-panel",
        className,
      )}
      {...rest}
    >
      {children}
    </section>
  );
}

/**
 * Names the panel. It has to win against the meta beside it and the column
 * headings below it, so the eye can find the edges of a region before it
 * starts reading numbers — those three used to sit within one step of each
 * other and the hierarchy read flat.
 */
export function PanelHead({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      {...rest}
      className={cn(
        "flex h-6 flex-none items-center gap-2.5 border-b border-rule bg-panel-hi px-2.5",
        className,
      )}
    >
      {children}
    </div>
  );
}

export function PanelTitle({ children }: { children: ReactNode }) {
  return (
    <h2 className="font-sans text-label font-semibold uppercase tracking-[0.14em] text-ink">{children}</h2>
  );
}

/** Secondary detail in a panel head. Pushed to the far end. */
export function Meta({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <span className={cn("ml-auto font-sans text-micro tracking-[0.04em] text-ink-4", className)}>
      {children}
    </span>
  );
}

/**
 * Column headings for a table panel. The grid columns belong to the table, not
 * to the chrome, so each caller passes its own — and passes the *same* string
 * to its rows, which is what stops headings and data drifting apart.
 */
export function ColumnHeads({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      {...rest}
      className={cn(
        "grid h-5 flex-none items-center border-b border-rule px-2.5",
        "font-sans text-micro font-medium uppercase tracking-[0.12em] text-ink-4",
        className,
      )}
    >
      {children}
    </div>
  );
}

export function Scroll({ className, children, ...rest }: { className?: string; children: ReactNode } & React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "min-h-0 flex-1 overflow-y-auto overflow-x-hidden",
        // A 6px thumb on no track: present when you need it, invisible when
        // you do not, and it never steals width from a column.
        "[&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-thumb]:bg-rule-hi [&::-webkit-scrollbar-track]:bg-transparent",
        className,
      )}
      {...rest}
    >
      {children}
    </div>
  );
}

/**
 * Headings and rows in one horizontal scroller, so they cannot drift out of
 * alignment and a panel narrower than the table scrolls sideways instead of
 * quietly cutting the last columns off the end.
 *
 * The caller puts the same `minWidth` on its `ColumnHeads` and its `Scroll` —
 * and drops it when the table is empty, because an empty panel has no columns
 * to hold in alignment and should centre its message in the width the user can
 * actually see rather than one they would have to scroll to reach.
 */
export function Table({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <div
      className={cn(
        "flex min-h-0 flex-1 flex-col overflow-x-auto",
        "[&::-webkit-scrollbar]:h-1.5 [&::-webkit-scrollbar-thumb]:bg-rule-hi [&::-webkit-scrollbar-track]:bg-transparent",
        className,
      )}
    >
      {children}
    </div>
  );
}

/** A panel with nothing in it yet. Says so, rather than rendering a void. */
export function Empty({ children }: { children: ReactNode }) {
  return <div className="p-4 text-center font-sans text-label text-ink-4">{children}</div>;
}
