import { useMemo, useState } from "react";
import type { PlaceOrderRequest } from "../lib/api";
import { cn } from "../lib/utils";
import { decimalsForStep, feeOn, formatAtoms, isAligned, notional, parseAtoms } from "../lib/num";
import type { Balance, Market, Side } from "../lib/types";
import {
  AvailableLine,
  Field,
  FieldLabel,
  FieldName,
  FieldRule,
  Segment,
  Segmented,
  SubmitButton,
  SunkenInput,
} from "./ui/form";
import { Meta, PanelHead, PanelTitle } from "./ui/panel";

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
      <PanelHead>
        <PanelTitle>Order ticket</PanelTitle>
        {market && (
          <Meta>
            min{" "}
            <b className="tnum font-medium text-ink-2">
              {formatAtoms(market.min_notional, market.quote_decimals, { places: 2 })}
            </b>{" "}
            {market.quote}
          </Meta>
        )}
      </PanelHead>

      <div className="flex flex-none flex-col gap-2.5 p-2.5 [&>*]:flex-none">
        <Segmented className="grid-cols-2" data-testid="side-select">
          <Segment
            data-side="buy"
            tone="buy"
            selected={side === "BUY"}
            onClick={() => setSide("BUY")}
          >
            BUY
          </Segment>
          <Segment
            data-side="sell"
            tone="sell"
            selected={side === "SELL"}
            onClick={() => setSide("SELL")}
          >
            SELL
          </Segment>
        </Segmented>

        <Segmented className="grid-cols-2">
          <Segment selected={kind === "LIMIT"} onClick={() => setKind("LIMIT")}>
            LIMIT
          </Segment>
          <Segment selected={kind === "MARKET"} onClick={() => setKind("MARKET")}>
            MARKET
          </Segment>
        </Segmented>

        <Field>
          <FieldLabel>
            <FieldName>Price</FieldName>
            {market && kind === "LIMIT" && (bestBid !== null || bestAsk !== null) && (
              // Two prices worth one click each: the middle of the spread, and
              // the best price already showing on your own side of it. Both
              // come off the book already on screen.
              <span className="ml-2.5 flex gap-px" data-testid="price-picks">
                <Pick onClick={() => setPriceFrom(midPrice)} disabled={midPrice === null}>
                  MID
                </Pick>
                <Pick onClick={() => setPriceFrom(bboPrice)} disabled={bboPrice === null}>
                  BBO
                </Pick>
              </span>
            )}
            {market && (
              <FieldRule>
                tick {formatAtoms(market.tick_size, market.quote_decimals, { places: priceDp })}
              </FieldRule>
            )}
          </FieldLabel>
          <SunkenInput
            unit={market?.quote ?? ""}
            value={kind === "MARKET" ? "" : price}
            placeholder={kind === "MARKET" ? "market" : ""}
            disabled={kind === "MARKET"}
            inputMode="decimal"
            aria-label="price"
            onChange={(e) => onPriceChange(e.target.value)}
          />
        </Field>

        <Field>
          <FieldLabel>
            <FieldName>Quantity</FieldName>
            {market && (
              <FieldRule>
                lot {formatAtoms(market.lot_size, market.base_decimals, { places: qtyDp })}
              </FieldRule>
            )}
          </FieldLabel>
          <SunkenInput
            unit={market?.base ?? ""}
            bad={problem !== null}
            value={qty}
            inputMode="decimal"
            aria-label="quantity"
            onChange={(e) => setQty(e.target.value)}
          />
          <div className="grid grid-cols-4 gap-px bg-rule">
            {([25n, 50n, 75n, 100n] as const).map((percent) => (
              <button
                key={String(percent)}
                type="button"
                onClick={() => fillPercent(percent)}
                className="flex min-h-6 cursor-pointer items-center justify-center bg-panel-hi py-1 text-center font-sans text-micro text-ink-3 transition-colors hover:bg-hover hover:text-ink"
              >
                {percent === 100n ? "MAX" : `${percent}%`}
              </button>
            ))}
          </div>
        </Field>

        {problem && (
          <div className="font-sans text-micro leading-[1.4] text-sell" data-testid="ticket-problem">
            {problem}
          </div>
        )}

        <div className="border border-rule bg-field">
          <Readout label="Notional">
            {quote && market
              ? `${formatAtoms(quote.value, market.quote_decimals)} ${market.quote}`
              : "—"}
          </Readout>
          <Readout
            label="Fee"
            hint={`taker ${market ? String(market.taker_fee_bps) : "—"} bps`}
          >
            {quote && market
              ? `${formatAtoms(quote.fee, market.quote_decimals)} ${market.quote}`
              : "—"}
          </Readout>
          <Readout label={side === "BUY" ? "Total cost" : "Net proceeds"} total>
            <span data-testid="ticket-total">
              {quote && market
                ? `${formatAtoms(
                    side === "BUY" ? quote.value + quote.fee : quote.value - quote.fee,
                    market.quote_decimals,
                  )} ${market.quote}`
                : "—"}
            </span>
          </Readout>
        </div>

        <SubmitButton
          side={side}
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
        </SubmitButton>

        <AvailableLine label="available">
          {spendBalance
            ? `${formatAtoms(spendBalance.available, spendDecimals)} ${spendAsset}`
            : `0 ${spendAsset}`}
        </AvailableLine>
      </div>
    </>
  );
}

/** One price straight off the book. Outlined, small, and never the loudest thing. */
function Pick({
  children,
  ...rest
}: { children: string } & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      className={[
        "flex min-h-5 cursor-pointer items-center px-1.5 py-0.5",
        "font-sans text-[9.5px] font-bold tracking-[0.12em] text-ink-3",
        "shadow-[inset_0_0_0_1px_var(--color-rule-hi)] transition-colors",
        "hover:bg-panel-hi hover:text-ink",
        "disabled:cursor-default disabled:bg-transparent disabled:text-ink-4",
        "disabled:shadow-[inset_0_0_0_1px_var(--color-rule)]",
      ].join(" ")}
      {...rest}
    >
      {children}
    </button>
  );
}

/** One line of the cost estimate. The total is the one that gets the weight. */
function Readout({
  label,
  hint,
  total = false,
  children,
}: {
  label: string;
  hint?: string;
  total?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center border-b border-rule px-2 py-1 last:border-b-0">
      <span className="font-sans text-label font-medium uppercase tracking-[0.13em] text-ink-4">
        {label}
        {hint !== undefined && <span className="ml-1.5 text-[9.5px] normal-case">{hint}</span>}
      </span>
      <span className={cn("tnum ml-auto", total ? "text-data text-ink" : "text-ink-2")}>
        {children}
      </span>
    </div>
  );
}
