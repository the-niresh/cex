import { useEffect, useRef, useState, type ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * A 6px thumb on no track: present when you need it, invisible when you do not,
 * and it never steals width from a column.
 */
const THUMB =
  "[&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-thumb]:bg-rule-hi [&::-webkit-scrollbar-track]:bg-transparent";

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
        "shadow-[0_1px_1px_rgb(0_0_0/0.3)]",
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

export function PanelTitle({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.HTMLAttributes<HTMLHeadingElement>) {
  return (
    <h2 className={cn("font-sans text-micro font-medium text-ink", className)} {...rest}>
      {children}
    </h2>
  );
}

/**
 * Secondary detail in a panel head. Pushed to the far end.
 *
 * ⚠️ Forwards `...rest` like every other component in this file. It was the one
 * that did not, so a `data-testid` handed to it disappeared without a word.
 */
export function Meta({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.HTMLAttributes<HTMLSpanElement>) {
  return (
    <span className={cn("ml-auto font-sans text-micro text-ink-4", className)} {...rest}>
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
        "font-sans text-micro text-ink-4",
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
      className={cn("min-h-0 flex-1 overflow-y-auto overflow-x-hidden", THUMB, className)}
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
export function Table({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "flex min-h-0 flex-1 flex-col overflow-x-auto",
        "[&::-webkit-scrollbar]:h-1.5 [&::-webkit-scrollbar-thumb]:bg-rule-hi [&::-webkit-scrollbar-track]:bg-transparent",
        className,
      )}
      {...rest}
    >
      {children}
    </div>
  );
}

/**
 * A scroller that admits there is more below.
 *
 * The right rail runs ~250px past a 900px-tall viewport, and the clip lands
 * wherever it lands — through the middle of a table row, most of the time,
 * which reads as a rendering fault rather than as "keep going". A fade over the
 * bottom edge says the same thing a cut row cannot, and it clears the moment
 * you reach the end so it never lies about content that is not there.
 *
 * Measured, not assumed. CSS alone can do this with `scroll-timeline`, which
 * Safari does not have; an always-on fade is the version that lies.
 *
 * The state is recomputed after every render rather than by watching the
 * content for resizes: the shell re-renders on every book update and once a
 * second besides, the measurement is two property reads, and `setState` with an
 * unchanged value does not re-render. That covers content growing (balances
 * arriving) and shrinking (the ticket dropping its price field) without an
 * observer that has to be re-attached whenever the children change.
 */
export function ScrollShade({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.HTMLAttributes<HTMLDivElement>) {
  const scroller = useRef<HTMLDivElement>(null);
  const [more, setMore] = useState(false);

  function measure() {
    const el = scroller.current;
    if (!el) return;
    // A pixel of slack: fractional layout heights can leave scrollTop a hair
    // short of the true bottom, which would strand the fade on forever.
    setMore(el.scrollHeight - el.scrollTop - el.clientHeight > 1);
  }

  // No dependency array on purpose — see the note above.
  useEffect(measure);

  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    el.addEventListener("scroll", measure, { passive: true });
    // The panel changing height changes the answer without React rendering.
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => {
      el.removeEventListener("scroll", measure);
      observer.disconnect();
    };
  }, []);

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={scroller}
        className={cn("flex min-h-0 flex-1 flex-col overflow-y-auto", THUMB, className)}
        {...rest}
      >
        {children}
      </div>
      <div
        aria-hidden
        data-more={more ? "true" : "false"}
        className={cn(
          "pointer-events-none absolute inset-x-0 bottom-0 h-9",
          "bg-linear-to-t from-panel via-panel/80 to-transparent",
          "opacity-0 transition-opacity duration-150 data-[more=true]:opacity-100",
        )}
      />
    </div>
  );
}

/** A panel with nothing in it yet. Says so, rather than rendering a void. */
export function Empty({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={cn("p-4 text-center font-sans text-micro text-ink-4", className)} {...rest}>
      {children}
    </div>
  );
}
