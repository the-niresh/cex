import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * The form vocabulary the ticket, the balances panel and the sign-in panel all
 * speak: a segmented choice, a labelled field, a sunken input, and the one
 * button that commits.
 *
 * They were three copies of the same six CSS rules, keyed off class names each
 * component opted into by hand — which is how the ticket ended up with a 24px
 * tap target and the deposit row with a 21px one. As components the floor is
 * decided once.
 */

/**
 * A row of mutually exclusive choices. The 1px gaps show the rule colour
 * underneath, which is what gives hairline dividers with no borders.
 */
export function Segmented({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={cn("grid gap-px bg-rule", className)} {...rest}>
      {children}
    </div>
  );
}

/** Which hue a selected segment takes. Grey unless the choice *is* a side. */
type Tone = "control" | "buy" | "sell";

const SELECTED: Record<Tone, string> = {
  control: "bg-panel text-ink shadow-[inset_0_0_0_1px_var(--color-control-dim)]",
  buy: "bg-buy-fill text-buy shadow-[inset_0_0_0_1px_var(--color-buy-line)]",
  sell: "bg-sell-fill text-sell shadow-[inset_0_0_0_1px_var(--color-sell-line)]",
};

export function Segment({
  selected,
  tone = "control",
  quiet = false,
  className,
  children,
  ...rest
}: {
  selected: boolean;
  tone?: Tone;
  /** Asset chips read as data, not as commands, so they drop the emphasis. */
  quiet?: boolean;
  className?: string;
  children: ReactNode;
} & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      aria-selected={selected}
      className={cn(
        // 24px is the floor for a pointer target (WCAG 2.2). Several of these
        // used to sit at 21–23px, one short in each case.
        "flex min-h-6 cursor-pointer items-center justify-center py-1.5 text-center",
        "font-sans text-micro transition-colors",
        quiet ? "tracking-[0.06em]" : "font-bold uppercase tracking-[0.16em]",
        selected ? SELECTED[tone] : "bg-panel-hi text-ink-4 hover:text-ink-2",
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}

/** A label, whatever rides beside it, and the control underneath. */
export function Field({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cn("flex flex-col gap-[3px]", className)}>{children}</div>;
}

export function FieldLabel({ children }: { children: ReactNode }) {
  return <div className="flex items-baseline font-sans">{children}</div>;
}

export function FieldName({ children }: { children: ReactNode }) {
  return (
    <span className="text-label font-medium uppercase tracking-[0.14em] text-ink-3">{children}</span>
  );
}

/** The rule the field enforces — tick, lot, minimum length. Pushed to the end. */
export function FieldRule({ children }: { children: ReactNode }) {
  return <span className="ml-auto text-micro text-ink-4">{children}</span>;
}

/**
 * A number you type into. Sunken rather than raised, and it takes the control
 * hue on focus — the one place azure is allowed next to the data.
 */
export function SunkenInput({
  unit,
  bad = false,
  className,
  ...rest
}: {
  unit?: string;
  /** The value cannot be sent. Said in the border before it is said in words. */
  bad?: boolean;
  className?: string;
} & React.InputHTMLAttributes<HTMLInputElement>) {
  return (
    <div
      className={cn(
        "flex items-center border bg-field",
        bad ? "border-sell" : "border-rule-hi",
        "focus-within:border-control focus-within:shadow-[inset_0_0_0_1px_var(--color-control-dim)]",
        className,
      )}
    >
      <input
        // `min-w-0` because a flex item's default `min-width:auto` refuses to
        // go below an input's intrinsic width, which pushed the CREDIT button
        // off the side of a narrow ticket.
        className="min-w-0 flex-1 bg-transparent px-2 py-1.5 font-mono text-[13px] tabular-nums text-ink outline-none"
        {...rest}
      />
      {unit !== undefined && (
        <span className="flex items-center self-stretch border-l border-rule px-2.5 font-sans text-[9.5px] tracking-[0.1em] text-ink-4">
          {unit}
        </span>
      )}
    </div>
  );
}

/**
 * The button that commits. It carries the side's hue, and it goes inert — not
 * hidden — when there is nothing valid to send or the feed has gone stale.
 */
export function SubmitButton({
  side = "BUY",
  className,
  children,
  ...rest
}: {
  side?: "BUY" | "SELL";
  className?: string;
  children: ReactNode;
} & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      className={cn(
        "cursor-pointer py-2.5 text-center font-sans text-micro font-bold tracking-[0.14em] transition-colors",
        side === "SELL"
          ? "bg-sell-fill text-sell shadow-[inset_0_0_0_1px_var(--color-sell-line)] hover:bg-sell-fill-hi"
          : "bg-buy-fill text-buy shadow-[inset_0_0_0_1px_var(--color-buy-line)] hover:bg-buy-fill-hi",
        // Dead in the same way for both reasons, so the user never has to work
        // out which of the two is stopping them.
        //
        // ⚠️ `degraded`, not `stale`. Stale only means nothing has printed
        // lately, which on a quiet market is normal — gating on it locked
        // people out of a book that was working perfectly well.
        "disabled:pointer-events-none disabled:bg-panel-hi disabled:text-ink-4 disabled:shadow-[inset_0_0_0_1px_var(--color-rule-hi)]",
        "group-data-[degraded=true]/screen:pointer-events-none group-data-[degraded=true]/screen:bg-panel-hi",
        "group-data-[degraded=true]/screen:text-ink-4 group-data-[degraded=true]/screen:shadow-[inset_0_0_0_1px_var(--color-rule-hi)]",
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}

/** A secondary commit: outlined, not filled, so it never competes with BUY. */
export function GhostButton({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      className={cn(
        "cursor-pointer py-1.5 text-center font-sans text-micro font-bold tracking-[0.14em]",
        "text-ink-2 shadow-[inset_0_0_0_1px_var(--color-rule-hi)] transition-colors",
        "hover:bg-panel-hi hover:text-ink",
        "disabled:pointer-events-none disabled:text-ink-4",
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}

/** `available … 12,000 USDT`. The figure the field above is measured against. */
export function AvailableLine({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex pt-px font-sans text-micro text-ink-4">
      <span>{label}</span>
      <b className="ml-auto font-mono font-normal text-ink-3">{children}</b>
    </div>
  );
}
