import { useState } from "react";
import { formatAtoms, parseAtoms } from "../lib/num";
import type { Balance, Market } from "../lib/types";
import { Num } from "./format";
import { AvailableLine, GhostButton, Segment, Segmented, SunkenInput } from "./ui/form";
import { ColumnHeads, Empty, Meta, PanelHead, PanelTitle, Scroll } from "./ui/panel";

/** Headings and rows share one column template, so they cannot drift apart. */
const COLS = "grid-cols-[44px_1fr_104px] gap-x-2.5 [&>span:not(:first-child)]:text-right";

interface Props {
  balances: Balance[];
  markets: Market[];
  signedIn: boolean;
  /** Same reasoning as the ticket's: crediting an account needs an account. */
  onRequireSignIn(): void;
  onDeposit(asset: string, amount: bigint): Promise<void>;
}

/** Decimals for an asset, taken from whichever market names it. */
function decimalsFor(asset: string, markets: Market[]): bigint {
  for (const market of markets) {
    if (market.base === asset) return market.base_decimals;
    if (market.quote === asset) return market.quote_decimals;
  }
  return 8n;
}

export function Balances({ balances, markets, signedIn, onRequireSignIn, onDeposit }: Props) {
  const assets = [...new Set(markets.flatMap((m) => [m.quote, m.base]))];
  const [asset, setAsset] = useState("USDT");
  const [amount, setAmount] = useState("10000");
  const [sending, setSending] = useState(false);

  const decimals = decimalsFor(asset, markets);
  const parsed = parseAtoms(amount, decimals);
  const ready = !sending && parsed !== null && parsed > 0n;

  async function credit() {
    if (!signedIn) {
      onRequireSignIn();
      return;
    }
    if (!ready || parsed === null) return;
    setSending(true);
    try {
      await onDeposit(asset, parsed);
    } finally {
      setSending(false);
    }
  }

  return (
    <>
      <PanelHead className="border-t border-rule">
        <PanelTitle>Balances</PanelTitle>
        <Meta>{balances.length} assets</Meta>
      </PanelHead>
      <ColumnHeads className={COLS}>
        <span>Asset</span>
        <span>Available</span>
        <span>Locked</span>
      </ColumnHeads>
      {/* Yields height to the panels around it, but never down to a sliver. */}
      <Scroll className="min-h-14">
        {balances.length === 0 ? (
          <Empty>{signedIn ? "no balances — deposit below" : "sign in to hold a balance"}</Empty>
        ) : (
          balances.map((balance) => {
            const dp = decimalsFor(balance.asset, markets);
            return (
              <div
                key={balance.asset}
                className={`tnum grid h-5 items-center px-2.5 hover:bg-row-hover ${COLS}`}
                data-testid="balance-row"
                data-asset={balance.asset}
              >
                <span className="font-sans tracking-[0.06em] text-ink-2">{balance.asset}</span>
                <span>
                  <Num atoms={balance.available} decimals={dp} />
                </span>
                <span
                  className={balance.locked === 0n ? "text-ink-4" : "text-ink-3"}
                  data-testid="balance-locked"
                >
                  <Num atoms={balance.locked} decimals={dp} />
                </span>
              </div>
            );
          })
        )}
      </Scroll>

      <PanelHead className="border-t border-rule">
        <PanelTitle>Deposit</PanelTitle>
        <Meta>test exchange</Meta>
      </PanelHead>
      <div className="flex flex-col gap-[7px] p-2.5">
        <Segmented className="grid-cols-4" data-testid="deposit-assets">
          {assets.map((a) => (
            <Segment key={a} quiet selected={a === asset} onClick={() => setAsset(a)}>
              {a}
            </Segment>
          ))}
        </Segmented>
        {/* `minmax(0,1fr)`, not `1fr`. A bare `1fr` floors at the column's
            min-content width, and this column holds a text input whose
            intrinsic width is ~180px — so on a narrow ticket the row grew past
            the panel and pushed the CREDIT button off the side. */}
        <div className="grid grid-cols-[minmax(0,1fr)_118px] gap-[7px]">
          <SunkenInput
            bad={parsed === null && amount !== ""}
            value={amount}
            inputMode="decimal"
            aria-label="deposit amount"
            onChange={(e) => setAmount(e.target.value)}
          />
          <GhostButton disabled={signedIn && !ready} onClick={() => void credit()}>
            {!signedIn ? "LOG IN" : sending ? "…" : "CREDIT"}
          </GhostButton>
        </div>
        {parsed !== null && (
          <AvailableLine label="credits">
            {formatAtoms(parsed, decimals)} {asset}
          </AvailableLine>
        )}
      </div>
    </>
  );
}
