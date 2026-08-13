import { Children, isValidElement, type ReactNode } from "react";
import { cn } from "@/lib/utils";
import { Button } from "./button";
import { Input } from "./input";

/**
 * The form vocabulary the ticket, the balances panel and the sign-in panel all
 * speak: a segmented choice, a labelled field, a field you type into, and the
 * one button that commits.
 *
 * They were three copies of the same six CSS rules, keyed off class names each
 * component opted into by hand — which is how the ticket ended up with a 24px
 * tap target and the deposit row with a 21px one. As components the floor is
 * decided once.
 */

/** Which hue a selected segment takes. Grey unless the choice *is* a side. */
type Tone = "control" | "buy" | "sell";

/**
 * The track's shape depends on what the choice means, and nothing else says
 * so: a side (BUY/SELL — `tone` is "buy" or "sell") gets the reference's 44px
 * pill. Anything ordinary — an order type, an auth mode, an asset chip —
 * gets the 32px control-radius tab track instead. Segmented reads this off
 * its children's own `tone` rather than asking every caller to say it twice;
 * they already set it, for the buy/sell case, and leave it at the default for
 * everything else.
 */
function hasSideTone(children: ReactNode): boolean {
  return Children.toArray(children).some((child) => {
    if (!isValidElement<{ tone?: Tone }>(child)) return false;
    return child.props.tone === "buy" || child.props.tone === "sell";
  });
}

/**
 * A row of mutually exclusive choices. The 1px gaps show the rule colour
 * underneath in the side-tone case, which is what gives hairline dividers
 * with no borders; the control case insets its selected segment instead.
 */
export function Segmented({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.HTMLAttributes<HTMLDivElement>) {
  const sided = hasSideTone(children);
  return (
    <div
      className={cn(
        "grid overflow-hidden",
        sided
          ? "h-11 gap-px rounded-pill bg-rule"
          : "h-8 gap-0.5 rounded-control bg-panel-hi p-0.5",
        className,
      )}
      {...rest}
    >
      {children}
    </div>
  );
}

const SELECTED: Record<Tone, string> = {
  control: "rounded-control bg-field text-ink",
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
        quiet ? "font-normal" : "font-medium",
        selected ? SELECTED[tone] : "bg-panel-hi text-ink-4 hover:bg-hover hover:text-ink-2",
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
  return <span className="text-micro text-ink-3">{children}</span>;
}

/** The rule the field enforces — tick, lot, minimum length. Pushed to the end. */
export function FieldRule({ children }: { children: ReactNode }) {
  return <span className="ml-auto text-micro text-ink-4">{children}</span>;
}

/**
 * A number you type into. Controls sit *above* the panel now, not sunk into
 * it, so this is built on shadcn's `Input` rather than a bare `<input>` — the
 * border and focus ring live on the wrapper, the same way the old sunken
 * version worked, just lighter than the panel instead of darker.
 */
export function FieldInput({
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
        "flex h-[46px] items-center rounded-control border bg-field",
        bad ? "border-sell" : "border-rule-hi",
        "focus-within:border-control focus-within:shadow-[inset_0_0_0_1px_var(--color-control-dim)]",
        className,
      )}
    >
      <Input
        // `min-w-0` because a flex item's default `min-width:auto` refuses to
        // go below an input's intrinsic width, which pushed the CREDIT button
        // off the side of a narrow ticket.
        className="h-full min-w-0 flex-1 rounded-control border-0 bg-transparent px-3 font-mono text-field tabular-nums text-ink shadow-none outline-none focus-visible:ring-0"
        {...rest}
      />
      {unit !== undefined && (
        <span className="flex items-center self-stretch border-l border-rule px-2.5 font-sans text-micro text-ink-4">
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
    <Button
      variant={side === "SELL" ? "sell" : "buy"}
      size="trade"
      className={cn(
        "w-full",
        // Dead in the same way for both reasons, so the user never has to work
        // out which of the two is stopping them.
        //
        // ⚠️ `degraded`, not `stale`. Stale only means nothing has printed
        // lately, which on a quiet market is normal — gating on it locked
        // people out of a book that was working perfectly well.
        "disabled:opacity-100 disabled:bg-panel-hi disabled:text-ink-4",
        "group-data-[degraded=true]/screen:pointer-events-none group-data-[degraded=true]/screen:bg-panel-hi",
        "group-data-[degraded=true]/screen:text-ink-4",
        className,
      )}
      {...rest}
    >
      {children}
    </Button>
  );
}

/** A secondary commit: outlined, not filled, so it never competes with BUY. */
export function GhostButton({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <Button
      variant="trade-ghost"
      size="trade"
      className={cn("disabled:opacity-100 disabled:text-ink-4 disabled:border-rule", className)}
      {...rest}
    >
      {children}
    </Button>
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
