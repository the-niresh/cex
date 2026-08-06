import { useState } from "react";
import { formatAtoms, parseAtoms } from "../lib/num";
import type { Balance, Market } from "../lib/types";
import { Num } from "./format";

interface Props {
  balances: Balance[];
  markets: Market[];
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

export function Balances({ balances, markets, onDeposit }: Props) {
  const assets = [...new Set(markets.flatMap((m) => [m.quote, m.base]))];
  const [asset, setAsset] = useState("USDT");
  const [amount, setAmount] = useState("10000");
  const [sending, setSending] = useState(false);

  const decimals = decimalsFor(asset, markets);
  const parsed = parseAtoms(amount, decimals);
  const ready = !sending && parsed !== null && parsed > 0n;

  async function credit() {
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
      <div className="phead">
        <h2>Balances</h2>
        <span className="meta">{balances.length} assets</span>
      </div>
      <div className="chead">
        <span>Asset</span>
        <span>Available</span>
        <span>Locked</span>
      </div>
      <div className="scroll">
        {balances.length === 0 ? (
          <div className="empty">no balances — deposit below</div>
        ) : (
          balances.map((balance) => {
            const dp = decimalsFor(balance.asset, markets);
            return (
              <div className="brow" key={balance.asset}>
                <span className="asset">{balance.asset}</span>
                <span className="num">
                  <Num atoms={balance.available} decimals={dp} />
                </span>
                <span className={`num lock${balance.locked === 0n ? " zero" : ""}`}>
                  <Num atoms={balance.locked} decimals={dp} />
                </span>
              </div>
            );
          })
        )}
      </div>

      <div className="phead">
        <h2>Deposit</h2>
        <span className="meta">test exchange</span>
      </div>
      <div className="deposit">
        <div className="assets">
          {assets.map((a) => (
            <button key={a} aria-selected={a === asset} onClick={() => setAsset(a)}>
              {a}
            </button>
          ))}
        </div>
        <div className="row">
          <div className={`input${parsed === null && amount !== "" ? " bad" : ""}`}>
            <input
              value={amount}
              inputMode="decimal"
              aria-label="deposit amount"
              onChange={(e) => setAmount(e.target.value)}
            />
          </div>
          <button className="ghost" disabled={!ready} onClick={() => void credit()}>
            {sending ? "…" : "CREDIT"}
          </button>
        </div>
        {parsed !== null && (
          <div className="avail">
            <span>credits</span>
            <b>
              {formatAtoms(parsed, decimals)} {asset}
            </b>
          </div>
        )}
      </div>
    </>
  );
}
