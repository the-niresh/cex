import { decimalsForStep } from "../lib/num";
import type { Market } from "../lib/types";
import type { TapePrint } from "../useExchange";
import { Num, clockMs } from "./format";
import { PanelTabs, type BookTab } from "./PanelTabs";

interface Props {
  market: Market | null;
  prints: TapePrint[];
  tab: BookTab;
  onTab(next: BookTab): void;
}

export function Tape({ market, prints, tab, onTab }: Props) {
  return (
    <section className="panel tape" data-testid="tape-panel">
      <div className="phead">
        <PanelTabs tab={tab} onTab={onTab} />
        <span className="meta">
          last <b>{prints.length}</b>
        </span>
      </div>
      <div className="chead" data-testid="tape-heads">
        <span>Time</span>
        <span>Price</span>
        <span>Size</span>
      </div>
      <div className="scroll">
        {!market || prints.length === 0 ? (
          <div className="empty">no prints yet</div>
        ) : (
          prints.map((print) => {
            const { time, millis } = clockMs(print.timestamp_ms);
            return (
              <div
                key={print.key}
                className={`print ${print.taker_side === "BUY" ? "buy" : "sell"}${
                  print.fresh ? " fresh" : ""
                }`}
              >
                <span className="t">
                  {time}
                  <span className="d">{millis}</span>
                </span>
                <span className="p">
                  <Num
                    atoms={print.price}
                    decimals={market.quote_decimals}
                    places={decimalsForStep(market.tick_size, market.quote_decimals)}
                  />
                </span>
                <span className="q">
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
      </div>
    </section>
  );
}
