import { cn } from "@/lib/utils";

export type BookTab = "book" | "tape";

/**
 * The book and the tape are two readings of the same market, so they share one
 * panel and one column rather than each holding a column open. The strip lives
 * in the panel head where the title used to be — each side still writes its
 * own meta beside it, because what is worth saying differs.
 */
export function PanelTabs({ tab, onTab }: { tab: BookTab; onTab(next: BookTab): void }) {
  return (
    // Pulled left by the head's own padding so the first tab starts on the same
    // line as every other panel title, rather than one indent further in.
    <div className="-ml-2.5 flex gap-1 self-stretch" data-testid="book-tabs">
      <Tab tab={tab} value="book" onTab={onTab}>
        Book
      </Tab>
      <Tab tab={tab} value="tape" onTab={onTab}>
        Trades
      </Tab>
    </div>
  );
}

function Tab({
  tab,
  value,
  onTab,
  children,
}: {
  tab: BookTab;
  value: BookTab;
  onTab(next: BookTab): void;
  children: string;
}) {
  const selected = tab === value;
  return (
    <button
      type="button"
      aria-selected={selected}
      onClick={() => onTab(value)}
      className={cn(
        "flex min-h-6 cursor-pointer items-center rounded-control px-2.5",
        "font-sans text-label transition-colors",
        selected
          ? // The same pill the order-type tabs carry: a field-toned surface
            // on the active choice, not an underline in the control hue.
            "bg-field text-ink"
          : "text-ink-4 hover:text-ink-2",
      )}
    >
      {children}
    </button>
  );
}
