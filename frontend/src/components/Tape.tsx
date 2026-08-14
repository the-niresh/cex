import { decimalsForStep } from "../lib/num";
import type { Market } from "../lib/types";
import type { TapePrint } from "../useExchange";
import { Num, clockMs } from "./format";
import { PanelTabs, type BookTab } from "./PanelTabs";
import { ColumnHeads, Empty, Meta, Panel, PanelHead, Scroll } from "./ui/panel";

interface Props {
  market: Market | null;
  prints: TapePrint[];
  tab: BookTab;
  onTab(next: BookTab): void;
}

/** Headings and rows share one column template, so they cannot drift apart. */
const COLS = "grid-cols-[76px_1fr_58px] gap-x-1.5";

export function Tape({ market, prints, tab, onTab }: Props) {
  return (
    <Panel className="max-stack:min-h-[240px]" data-testid="tape-panel">
      <PanelHead>
        <PanelTabs tab={tab} onTab={onTab} />
        <Meta>
          last <b className="tnum font-medium text-ink-2">{prints.length}</b>
        </Meta>
      </PanelHead>

      <ColumnHeads className={`${COLS} [&>span:not(:first-child)]:text-right`} data-testid="tape-heads">
        <span>Time</span>
        <span>Price{market ? ` (${market.quote})` : ""}</span>
        <span>Size{market ? ` (${market.base})` : ""}</span>
      </ColumnHeads>

      {/* The tape is data too: it goes flat and grey the moment the feed does. */}
      {/* Dims when the feed cannot be trusted, not when the market is merely
          quiet: these prints happened, and they stay true however long ago the
          last one was. */}
      <Scroll className="group-data-[degraded=true]/screen:saturate-[.15] group-data-[degraded=true]/screen:brightness-[.62]">
        {!market || prints.length === 0 ? (
          <Empty>no prints yet</Empty>
        ) : (
          prints.map((print) => {
            const { time, millis } = clockMs(print.timestamp_ms);
            const buy = print.taker_side === "BUY";
            return (
              <div
                key={print.key}
                className={[
                  "grid h-[20px] items-center border-l-2 px-2.5 tnum text-micro",
                  COLS,
                  "[&>span:not(:first-child)]:text-right",
                  buy ? "border-l-buy/60" : "border-l-sell/60",
                  // One flash on arrival, then it settles into the tape.
                  print.fresh ? "animate-[tape-flash_1.1s_ease-out]" : "",
                ].join(" ")}
              >
                <span className="text-ink-4">
                  {time}
                  <span className="text-ink-3">{millis}</span>
                </span>
                <span className={buy ? "text-buy" : "text-sell"}>
                  <Num
                    atoms={print.price}
                    decimals={market.quote_decimals}
                    places={decimalsForStep(market.tick_size, market.quote_decimals)}
                  />
                </span>
                <span className="text-ink-2">
                  <Num
                    atoms={print.qty}
                    decimals={market.base_decimals}
                    places={decimalsForStep(market.lot_size, market.base_decimals)}
                  />
                </span>
              </div>
            );
          })
        )}
      </Scroll>
    </Panel>
  );
}
