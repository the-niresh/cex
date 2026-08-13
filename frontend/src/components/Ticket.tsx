import { useMemo, useState } from "react";
import type { PlaceOrderRequest } from "../lib/api";
import { decimalsForStep, feeOn, formatAtoms, isAligned, notional, parseAtoms } from "../lib/num";
import type { Balance, Market, Side } from "../lib/types";

interface Props {
  market: Market | null;
  balances: Balance[];
  price: string;
  onPriceChange(price: string): void;
  bestBid: bigint | null;
  bestAsk: bigint | null;
  signedIn: boolean;
  /**
   * Called instead of submitting when nobody is signed in. Trading is the
   * first thing that actually needs an account — looking never does — so this
   * is where the sign-in panel gets opened, not on the way into the app.
   */
  onRequireSignIn(): void;
  onSubmit(request: PlaceOrderRequest): Promise<void>;
}

type Kind = "LIMIT" | "MARKET";

export function Ticket({
  market,
  balances,
  price,
  onPriceChange,
  bestBid,
  bestAsk,
  signedIn,
  onRequireSignIn,
  onSubmit,
}: Props) {
  const [side, setSide] = useState<Side>("BUY");
  const [kind, setKind] = useState<Kind>("LIMIT");
  const [qty, setQty] = useState("");
  const [sending, setSending] = useState(false);

  const priceDp = market ? decimalsForStep(market.tick_size, market.quote_decimals) : 2;
  const qtyDp = market ? decimalsForStep(market.lot_size, market.base_decimals) : 5;

  const priceAtoms = market && kind === "LIMIT" ? parseAtoms(price, market.quote_decimals) : null;
  const qtyAtoms = market ? parseAtoms(qty, market.base_decimals) : null;

  /**
   * The price a market order would actually pay, for the estimate only.
   * The engine decides the real one; this must never be sent as a limit.
   */
  const referencePrice = kind === "LIMIT" ? priceAtoms : side === "BUY" ? bestAsk : bestBid;

  /** The middle of the spread, snapped down to a tick the engine will accept. */
  const midPrice = useMemo(() => {
    if (!market || bestBid === null || bestAsk === null) return null;
    const mid = (bestBid + bestAsk) / 2n;
    return mid - (mid % market.tick_size);
  }, [market, bestBid, bestAsk]);

  /** The best price on the side you are trading — join the queue, don't cross. */
  const bboPrice = side === "BUY" ? bestBid : bestAsk;

  /** Put an exact price into the field the user edits, in the field's own format. */
  function setPriceFrom(atoms: bigint | null) {
    if (!market || atoms === null) return;
    onPriceChange(formatAtoms(atoms, market.quote_decimals, { places: priceDp, group: false }));
  }

  const quote = useMemo(() => {
    if (!market || qtyAtoms === null || qtyAtoms <= 0n || referencePrice === null) return null;
    const value = notional(referencePrice, qtyAtoms, market.base_decimals, "up");
    // A resting limit order may end up a maker; a market order never does.
    const bps = kind === "MARKET" ? market.taker_fee_bps : market.taker_fee_bps;
    return { value, fee: feeOn(value, bps), bps };
  }, [market, qtyAtoms, referencePrice, kind]);

  // Validated here so the user reads the rule rather than a 400 from the engine.
  const problem = useMemo(() => {
    if (!market) return null;
    if (qty.trim() === "") return null;
    if (qtyAtoms === null) return `quantity takes at most ${Number(market.base_decimals)} decimals`;
    if (qtyAtoms <= 0n) return "quantity must be greater than zero";
    if (!isAligned(qtyAtoms, market.lot_size))
      return `quantity must be a multiple of ${formatAtoms(market.lot_size, market.base_decimals, { places: qtyDp })}`;

    if (kind === "LIMIT") {
      if (price.trim() === "") return null;
      if (priceAtoms === null) return `price takes at most ${Number(market.quote_decimals)} decimals`;
      if (priceAtoms <= 0n) return "price must be greater than zero";
      if (!isAligned(priceAtoms, market.tick_size))
        return `price must be a multiple of ${formatAtoms(market.tick_size, market.quote_decimals, { places: priceDp })}`;
    }

    if (quote && quote.value < market.min_notional)
      return `order is below the ${formatAtoms(market.min_notional, market.quote_decimals, { places: 2 })} ${market.quote} minimum`;

    return null;
  }, [market, qty, qtyAtoms, price, priceAtoms, kind, quote, qtyDp, priceDp]);

  const spendAsset = market ? (side === "BUY" ? market.quote : market.base) : "";
  const spendBalance = balances.find((b) => b.asset === spendAsset);
  const spendDecimals = market ? (side === "BUY" ? market.quote_decimals : market.base_decimals) : 8n;

  const ready =
    !sending &&
    market !== null &&
    problem === null &&
    qtyAtoms !== null &&
    qtyAtoms > 0n &&
    (kind === "MARKET" || (priceAtoms !== null && priceAtoms > 0n));

  async function submit() {
    // The gate is here rather than on a disabled button on purpose: a signed
    // out visitor has to be able to *press* BUY to find out an account is
    // needed. A dead button teaches them nothing.
    if (!signedIn) {
      onRequireSignIn();
      return;
    }
    if (!market || !ready || qtyAtoms === null) return;
    setSending(true);
    try {
      await onSubmit({
        symbol: market.symbol,
        side,
        order_type: kind,
        // A market order that carried a price would be a limit order wearing
        // the wrong label.
        time_in_force: kind === "MARKET" ? "IOC" : "GTC",
        price: kind === "LIMIT" ? priceAtoms : null,
        qty: qtyAtoms,
      });
      setQty("");
    } catch {
      // Already surfaced by the caller; the ticket keeps what was typed so it
      // can be corrected rather than retyped.
    } finally {
      setSending(false);
    }
  }

  function fillPercent(percent: bigint) {
    if (!market || !spendBalance) return;
    if (side === "SELL") {
      const amount = (spendBalance.available * percent) / 100n;
      setQty(formatAtoms(amount - (amount % market.lot_size), market.base_decimals, { places: qtyDp, group: false }));
      return;
    }
    if (referencePrice === null || referencePrice <= 0n) return;
    const budget = (spendBalance.available * percent) / 100n;
    const affordable = (budget * 10n ** market.base_decimals) / referencePrice;
    setQty(
      formatAtoms(affordable - (affordable % market.lot_size), market.base_decimals, {
        places: qtyDp,
        group: false,
      }),
    );
  }

  return (
    <>
      <div className="phead">
        <h2>Order ticket</h2>
        {market && (
          <span className="meta">
            min <b>{formatAtoms(market.min_notional, market.quote_decimals, { places: 2 })}</b>{" "}
            {market.quote}
          </span>
        )}
      </div>

      <div className="body">
        <div className="seg side" data-testid="side-select">
          <button data-side="buy" aria-selected={side === "BUY"} onClick={() => setSide("BUY")}>
            BUY
          </button>
          <button data-side="sell" aria-selected={side === "SELL"} onClick={() => setSide("SELL")}>
            SELL
          </button>
        </div>

        <div className="seg kind">
          <button aria-selected={kind === "LIMIT"} onClick={() => setKind("LIMIT")}>
            LIMIT
          </button>
          <button aria-selected={kind === "MARKET"} onClick={() => setKind("MARKET")}>
            MARKET
          </button>
        </div>

        <div className="field">
          <div className="flabel">
            <span className="k">Price</span>
            {market && kind === "LIMIT" && (bestBid !== null || bestAsk !== null) && (
              // Two prices worth one click each: the middle of the spread, and
              // the best price already showing on your own side of it. Both
              // come off the book already on screen.
              <span className="picks" data-testid="price-picks">
                <button type="button" onClick={() => setPriceFrom(midPrice)} disabled={midPrice === null}>
                  MID
                </button>
                <button type="button" onClick={() => setPriceFrom(bboPrice)} disabled={bboPrice === null}>
                  BBO
                </button>
              </span>
            )}
            {market && (
              <span className="rule">
                tick {formatAtoms(market.tick_size, market.quote_decimals, { places: priceDp })}
              </span>
            )}
          </div>
          <div className="input">
            <input
              value={kind === "MARKET" ? "" : price}
              placeholder={kind === "MARKET" ? "market" : ""}
              disabled={kind === "MARKET"}
              inputMode="decimal"
              aria-label="price"
              onChange={(e) => onPriceChange(e.target.value)}
            />
            <span className="unit">{market?.quote ?? ""}</span>
          </div>
        </div>

        <div className="field">
          <div className="flabel">
            <span className="k">Quantity</span>
            {market && (
              <span className="rule">
                lot {formatAtoms(market.lot_size, market.base_decimals, { places: qtyDp })}
              </span>
            )}
          </div>
          <div className={`input${problem ? " bad" : ""}`}>
            <input
              value={qty}
              inputMode="decimal"
              aria-label="quantity"
              onChange={(e) => setQty(e.target.value)}
            />
            <span className="unit">{market?.base ?? ""}</span>
          </div>
          <div className="pcts">
            <button onClick={() => fillPercent(25n)}>25%</button>
            <button onClick={() => fillPercent(50n)}>50%</button>
            <button onClick={() => fillPercent(75n)}>75%</button>
            <button onClick={() => fillPercent(100n)}>MAX</button>
          </div>
        </div>

        {problem && (
          <div className="bad-note" data-testid="ticket-problem">
            {problem}
          </div>
        )}

        <div className="readout">
          <div className="r">
            <span className="k">Notional</span>
            <span className="v">
              {quote && market
                ? `${formatAtoms(quote.value, market.quote_decimals)} ${market.quote}`
                : "—"}
            </span>
          </div>
          <div className="r">
            <span className="k">
              Fee<span className="hint">taker {market ? String(market.taker_fee_bps) : "—"} bps</span>
            </span>
            <span className="v">
              {quote && market
                ? `${formatAtoms(quote.fee, market.quote_decimals)} ${market.quote}`
                : "—"}
            </span>
          </div>
          <div className="r total">
            <span className="k">{side === "BUY" ? "Total cost" : "Net proceeds"}</span>
            <span className="v" data-testid="ticket-total">
              {quote && market
                ? `${formatAtoms(
                    side === "BUY" ? quote.value + quote.fee : quote.value - quote.fee,
                    market.quote_decimals,
                  )} ${market.quote}`
                : "—"}
            </span>
          </div>
        </div>

        <button
          className={`submit${side === "SELL" ? " sell" : ""}`}
          data-testid="ticket-submit"
          disabled={signedIn && !ready}
          onClick={() => void submit()}
        >
          {!signedIn
            ? `LOG IN TO ${side}`
            : sending
              ? "SENDING…"
              : qty
                ? `${side} ${qty} ${market?.base ?? ""}`.trim()
                : side}
        </button>

        <div className="avail">
          <span>available</span>
          <b>
            {spendBalance
              ? `${formatAtoms(spendBalance.available, spendDecimals)} ${spendAsset}`
              : `0 ${spendAsset}`}
          </b>
        </div>
      </div>
    </>
  );
}
