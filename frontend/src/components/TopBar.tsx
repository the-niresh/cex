import { cn } from "@/lib/utils";
import type { FeedStatus } from "../lib/feed";
import { decimalsForStep } from "../lib/num";
import type { DayStats, Market, Session } from "../lib/types";
import { Num } from "./format";

interface Props {
  markets: Market[];
  market: Market | null;
  symbol: string;
  onSelect(symbol: string): void;
  lastPrice: bigint | null;
  lastSide: "BUY" | "SELL" | null;
  status: FeedStatus;
  /**
   * The book cannot be trusted — socket down, or a sequence gap. The shell
   * decides this once and hands the same answer to every readout of it, so the
   * dot here, the strip at the bottom and the band over the ladder can never
   * disagree about the same condition the way they used to.
   */
  feedDegraded: boolean;
  day: DayStats | null;
  session: Session | null;
  onSignIn(): void;
  onSignOut(): void;
}

const STATUS_TEXT: Record<FeedStatus, string> = {
  connecting: "Connecting",
  live: "Live",
  reconnecting: "Reconnecting",
  closed: "Offline",
};

/** The caption over a readout in the ticker. */
function Key({ children }: { children: string }) {
  return <span className="font-sans text-micro text-ink-4">{children}</span>;
}

/**
 * brand · markets · ticker · connection · account, and the account block is the
 * one part that must never move: it is where you look to sign in or out, and a
 * control that changes position depending on which market you clicked is a
 * control you have to hunt for.
 *
 * So the row does not wrap. Wrapping would be the obvious way to stop the
 * account block being clipped off the end, but wrapping takes priority over
 * shrinking — the moment the ticker holds real numbers instead of em-dashes it
 * grows, and the account block is what gets pushed onto a second line. That is
 * exactly the jump you see switching from an untraded market to a traded one.
 * Instead the ticker gives ground: it shrinks and folds its own fields onto
 * more lines, growing the bar downward while everything keeps its place along
 * it. Only below 880px, where no arrangement fits five blocks across, does the
 * bar wrap — and then on a fixed seam rather than wherever the numbers push it.
 */
export function TopBar({
  markets,
  market,
  symbol,
  onSelect,
  lastPrice,
  lastSide,
  status,
  feedDegraded,
  day,
  session,
  onSignIn,
  onSignOut,
}: Props) {
  const priceDp = market ? decimalsForStep(market.tick_size, market.quote_decimals) : 2;
  const qtyDp = market ? decimalsForStep(market.lot_size, market.base_decimals) : 5;
  const down = lastSide === "SELL";

  return (
    <header
      className={[
        "col-span-full flex min-w-0 flex-row flex-nowrap items-stretch overflow-hidden",
        "min-h-14 rounded-panel border border-rule bg-panel-hi [&>*]:min-h-14",
        "max-stack:flex-wrap",
      ].join(" ")}
    >
      <div className="flex flex-none items-center gap-2 border-r border-rule pl-3 pr-3.5">
        {/* Sized in CSS and given its dimensions in the markup, so the row does
            not jump a pixel taller the moment the image finishes loading. */}
        <img
          src="/logo.png"
          alt=""
          width={25}
          height={32}
          className="h-8 w-auto flex-none select-none"
          draggable={false}
          data-testid="brand-logo"
        />
        <span className="font-sans text-[11px] font-bold tracking-tight text-ink">CEX</span>
        <span className="font-sans text-micro tracking-[0.06em] text-ink-4">spot</span>
        {/* Says what this venue is, where someone actually looks.
            The order flow here is a market-making bot trading with itself, and
            a reader who works that out for themselves will assume the latency
            figures are staged too. They are not — they are the exchange timing
            its own work — so the cheap thing to protect is the real number, by
            being first to say which part is not real. */}
        <span
          className="rounded-pill bg-field px-1.5 py-0.5 font-sans text-micro text-ink-3"
          title="Demo venue. Deposits are free and the order flow is a market-making bot. The latency figures are real measurements of real matching."
          data-testid="demo-badge"
        >
          Demo
        </span>
      </div>

      {/* The tabs take their own scrollbar rather than pushing what follows off
          the row — but they keep a floor. With `min-width:0` they collapsed to
          nothing at narrow widths and their buttons spilled out underneath the
          account block, which then swallowed the clicks meant for them. */}
      <nav
        className={[
          "flex min-w-[150px] flex-[0_1_auto] items-center gap-1 overflow-x-auto px-2 py-1",
          "[scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
          "max-stack:flex-[1_1_auto]",
        ].join(" ")}
        data-testid="markets"
      >
        {markets.map((m) => {
          const selected = m.symbol === symbol;
          return (
            <button
              key={m.symbol}
              type="button"
              aria-selected={selected}
              onClick={() => onSelect(m.symbol)}
              className={cn(
                "flex min-w-[104px] cursor-pointer flex-col justify-center gap-px rounded-control px-2.5 py-1",
                "transition-colors",
                selected ? "bg-field" : "hover:bg-hover",
              )}
            >
              <span className={cn("font-sans text-data", selected ? "text-ink" : "text-ink-3")}>
                {m.symbol}
              </span>
              <span className={cn("tnum text-micro", selected ? "text-ink-3" : "text-ink-4")}>
                {selected && lastPrice !== null ? (
                  <span className={down ? "text-sell" : "text-buy"}>
                    <Num
                      atoms={lastPrice}
                      decimals={m.quote_decimals}
                      places={decimalsForStep(m.tick_size, m.quote_decimals)}
                    />
                  </span>
                ) : (
                  "—"
                )}
              </span>
            </button>
          );
        })}
      </nav>

      {market && (
        /* The day, not the rule book. Tick, lot and fees used to live here and
           are all still stated where they bite — beside the price and quantity
           inputs, and in the ticket's fee readout — whereas nothing on the
           screen said what the market had actually done since yesterday.

           This is the only item that gives. It takes the slack when there is
           room, and folds its own readouts onto more lines when there is not. */
        <div
          className={[
            "flex min-w-0 flex-[1_1_auto] flex-wrap items-center gap-x-[22px] gap-y-0.5",
            "ml-1 px-4.5 py-1.5",
            "max-stack:order-1 max-stack:ml-0 max-stack:flex-[0_0_100%]",
          ].join(" ")}
          data-testid="ticker"
        >
          <div className="flex flex-col gap-px leading-[1.15]">
            <Key>last</Key>
            <span
              className={`tnum text-[22px] font-medium leading-[1.1] tracking-[-0.015em] ${
                down ? "text-sell" : "text-buy"
              }`}
            >
              {lastPrice === null ? (
                "—"
              ) : (
                <>
                  <Num atoms={lastPrice} decimals={market.quote_decimals} places={priceDp} />
                  <span className="align-[3px] text-[11px]">{down ? " ▼" : " ▲"}</span>
                </>
              )}
            </span>
          </div>

          <div className="flex flex-col gap-px leading-[1.15]">
            <Key>24h change</Key>
            {/* The day's move is the one number here that carries a direction,
                so it is the only other one that carries a colour. */}
            <span
              className={`tnum text-micro ${
                day === null ? "text-ink-2" : day.change < 0n ? "text-sell" : "text-buy"
              }`}
            >
              {day === null ? (
                "—"
              ) : (
                <>
                  {day.change < 0n ? "−" : "+"}
                  <Num
                    atoms={day.change < 0n ? -day.change : day.change}
                    decimals={market.quote_decimals}
                    places={priceDp}
                  />
                  {day.changePct !== null && (
                    <span className="ml-1.5 opacity-75">
                      {day.changePct < 0 ? "" : "+"}
                      {day.changePct.toFixed(2)}%
                    </span>
                  )}
                </>
              )}
            </span>
          </div>

          <div className="flex flex-col gap-px leading-[1.15]">
            <Key>24h high</Key>
            <span className="tnum text-micro text-ink-2">
              {day === null ? (
                "—"
              ) : (
                <Num atoms={day.high} decimals={market.quote_decimals} places={priceDp} />
              )}
            </span>
          </div>

          <div className="flex flex-col gap-px leading-[1.15]">
            <Key>24h low</Key>
            <span className="tnum text-micro text-ink-2">
              {day === null ? (
                "—"
              ) : (
                <Num atoms={day.low} decimals={market.quote_decimals} places={priceDp} />
              )}
            </span>
          </div>

          <div className="flex flex-col gap-px leading-[1.15]">
            <Key>24h volume</Key>
            <span className="tnum text-micro text-ink-2">
              {day === null ? (
                "—"
              ) : (
                <>
                  <Num atoms={day.volume} decimals={market.base_decimals} places={qtyDp} />{" "}
                  <span className="text-ink-3">{market.base}</span>
                </>
              )}
            </span>
          </div>
        </div>
      )}

      <div
        className={[
          "ml-auto flex flex-none items-center gap-[9px] border-l border-rule px-3",
          "max-stack:order-2",
        ].join(" ")}
      >
        <i
          className={`size-1.5 rounded-full ${feedDegraded ? "bg-warn" : "animate-[dot-pulse_2.4s_ease-in-out_infinite] bg-buy"}`}
        />
        <span className={`font-sans text-micro ${feedDegraded ? "text-warn" : "text-ink-2"}`}>
          {feedDegraded && status === "live" ? "Resyncing" : STATUS_TEXT[status]}
        </span>
      </div>

      <div
        className={[
          "flex flex-none items-center gap-2.5 border-l border-rule px-3",
          "max-stack:order-2",
        ].join(" ")}
      >
        {session?.name && (
          <span
            className="max-w-[14ch] truncate font-sans text-micro text-ink"
            data-testid="account-name"
          >
            {session.name}
          </span>
        )}
        <span className="tnum text-micro text-ink-4" data-testid="account-id">
          {session ? `${session.user_id.slice(0, 8)}…` : "signed out"}
        </span>
        {/* 24px is the floor for a pointer target (WCAG 2.2). This one was 11px
            tall — the single hardest thing on the screen to click, and it is
            the control that signs you out. */}
        <button
          type="button"
          onClick={session ? onSignOut : onSignIn}
          data-testid="account-action"
          className="flex min-h-6 cursor-pointer items-center px-1 font-sans text-micro font-medium text-ink-4 transition-colors hover:text-ink-2"
        >
          {session ? "Log out" : "Log in"}
        </button>
      </div>
    </header>
  );
}
