import { useState } from "react";
import { feesPaid, valueLocked } from "../lib/summary";
import type { Market, MyFill, Order } from "../lib/types";
import { MyFills } from "./MyFills";
import { OpenOrders } from "./OpenOrders";
import { Meta } from "./ui/panel";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./ui/tabs";

interface Props {
  orders: Order[];
  fills: MyFill[];
  markets: Market[];
  onCancel(orderId: bigint): void;
}

/**
 * Your resting orders and your fills, in one panel rather than two.
 *
 * They used to sit side by side, each with its own heading, and on a fresh
 * account both said nothing — two empty tables taking a quarter of the screen
 * to report that nothing had happened. One panel with two tabs gives the same
 * information the space it actually needs, and the counts ride on the tabs so
 * you can see there is nothing to look at without opening either.
 */
export function ActivityPanel({ orders, fills, markets, onCancel }: Props) {
  // Controlled rather than defaulted, because the head reports on whichever
  // table is showing and cannot do that without knowing which one it is.
  const [tab, setTab] = useState("orders");

  const summary = tab === "orders" ? valueLocked(orders, markets) : feesPaid(fills, markets);

  return (
    <Tabs
      value={tab}
      onValueChange={(next) => setTab(String(next))}
      className="col-span-full row-start-3 flex min-h-0 flex-col gap-0 overflow-hidden rounded-panel border border-rule bg-panel max-stack:row-auto max-stack:min-h-[190px]"
      data-testid="activity"
    >
      <div className="flex h-6 flex-none items-center gap-1 border-b border-rule bg-panel-hi px-2">
        <TabsList className="h-auto gap-1 border-0 bg-transparent p-0">
          <Trigger value="orders" label="Open orders" count={orders.length} />
          <Trigger value="fills" label="Fills" count={fills.length} />
        </TabsList>
        {/* What the open orders are holding, or what the fills cost. Both used
            to be computed inside the tables and rendered into a panel head that
            nothing mounted once they moved in here — the arithmetic ran and the
            answer went nowhere. */}
        {summary && (
          <Meta data-testid="activity-summary">
            {tab === "orders" ? "locked" : "fees"}{" "}
            <b className="tnum font-medium text-ink-2">{summary}</b>
          </Meta>
        )}
      </div>

      <TabsContent value="orders" className="flex min-h-0 flex-1 flex-col">
        <OpenOrders orders={orders} markets={markets} onCancel={onCancel} />
      </TabsContent>
      <TabsContent value="fills" className="flex min-h-0 flex-1 flex-col">
        <MyFills fills={fills} markets={markets} />
      </TabsContent>
    </Tabs>
  );
}

function Trigger({ value, label, count }: { value: string; label: string; count: number }) {
  return (
    <TabsTrigger
      value={value}
      data-testid={`activity-tab-${value}`}
      className="rounded-control px-2 py-0.5 font-sans text-label font-medium uppercase tracking-[0.12em] text-ink-4 transition-colors hover:text-ink-2 data-[selected]:bg-rule data-[selected]:text-ink"
    >
      {label}
      {/* The count sits on the tab so an empty panel does not need opening. */}
      <span className="tnum ml-1.5 text-ink-4">{count}</span>
    </TabsTrigger>
  );
}
