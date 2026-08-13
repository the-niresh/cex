export type BookTab = "book" | "tape";

/**
 * The book and the tape are two readings of the same market, so they share one
 * panel and one column rather than each holding a column open. The strip lives
 * in the panel head where the title used to be — each side still writes its
 * own `.meta` beside it, because what is worth saying differs.
 */
export function PanelTabs({ tab, onTab }: { tab: BookTab; onTab(next: BookTab): void }) {
  return (
    <div className="seg tabs" data-testid="book-tabs">
      <button type="button" aria-selected={tab === "book"} onClick={() => onTab("book")}>
        BOOK
      </button>
      <button type="button" aria-selected={tab === "tape"} onClick={() => onTab("tape")}>
        TRADES
      </button>
    </div>
  );
}
