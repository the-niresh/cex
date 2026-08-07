/**
 * The wire contract, transcribed from the Rust.
 *
 * Every integer is a `bigint` because {@link parseExact} makes it one. Prices
 * and quantities are atomic units and never floats — see `num.ts` for what the
 * units mean and how they are formatted.
 */

export type Side = "BUY" | "SELL";
export type OrderType = "LIMIT" | "MARKET";
export type TimeInForce = "GTC" | "IOC" | "FOK";
export type Role = "MAKER" | "TAKER";

/** `GET /markets` */
export interface Market {
  symbol: string;
  base: string;
  quote: string;
  base_decimals: bigint;
  quote_decimals: bigint;
  /** Smallest permitted price increment, in quote atoms. */
  tick_size: bigint;
  /** Smallest permitted quantity increment, in base atoms. */
  lot_size: bigint;
  /** Smallest permitted order value, in quote atoms. */
  min_notional: bigint;
  maker_fee_bps: bigint;
  taker_fee_bps: bigint;
}

/** One rung: `[price, qty]`, both atomic. */
export type PriceLevel = [bigint, bigint];

/** `GET /depth/:symbol` */
export interface DepthSnapshot {
  symbol: string;
  depth_seq: bigint;
  /** Best first, descending. */
  bids: PriceLevel[];
  /** Best first, ascending. */
  asks: PriceLevel[];
}

/** A level change from `depth@SYMBOL`. `qty: 0` means remove the level. */
export interface DepthDelta {
  side: Side;
  price: bigint;
  qty: bigint;
}

export interface DepthUpdate {
  symbol: string;
  depth_seq: bigint;
  deltas: DepthDelta[];
}

/** A print on the public tape. Carries no user id, by design. */
export interface PublicTrade {
  seq: bigint;
  price: bigint;
  qty: bigint;
  taker_side: Side;
  timestamp_ms: bigint;
}

/** A print arriving live on `trades@SYMBOL`. No `seq`, no timestamp. */
export interface TradeUpdate {
  symbol: string;
  price: bigint;
  qty: bigint;
  taker_side: Side;
}

export interface Balance {
  asset: string;
  available: bigint;
  locked: bigint;
}

/** `GET /orders/open` */
export interface Order {
  order_id: bigint;
  user_id: string;
  symbol: string;
  side: Side;
  order_type: OrderType;
  /** Null for a market order. */
  price: bigint | null;
  qty: bigint;
  filled_qty: bigint;
  status: string;
}

/** `POST /orders` */
export interface PlacedOrder {
  order_id: bigint;
  status: string;
  filled_qty: bigint;
  qty: bigint;
  avg_price: bigint | null;
}

/** `GET /orders/history` — the caller's own fills, told from their side. */
export interface MyFill {
  seq: bigint;
  idx: bigint;
  symbol: string;
  /** The caller's own order. The counterparty's is never sent. */
  order_id: bigint;
  /** The caller's own side, not the aggressor's. */
  side: Side;
  role: Role;
  price: bigint;
  qty: bigint;
  notional: bigint;
  /**
   * What this caller paid, in atoms of whatever their side received — base on
   * a BUY, quote on a SELL. Use `feeAsset` rather than assuming either.
   */
  fee: bigint;
  timestamp_ms: bigint;
}

/** `GET /candles/:symbol` — a display projection, oldest first. */
export interface Candle {
  time_ms: bigint;
  open: bigint;
  high: bigint;
  low: bigint;
  close: bigint;
  /** Base atoms traded in the bucket. */
  volume: bigint;
  trades: bigint;
}

export type Interval = "1m" | "5m" | "15m" | "1h" | "4h" | "1d";

/**
 * The last 24 hours of a market, folded out of 24 hourly candles.
 *
 * Derived on the client rather than served: the exchange has no 24h stats
 * endpoint, and `GET /candles?interval=1h&limit=24` already carries every
 * number this needs exactly. Volume is in **base** atoms because that is what
 * a candle records — quoting it in the quote asset would mean multiplying each
 * bucket by its own close, which is an estimate, not a figure.
 */
export interface DayStats {
  open: bigint;
  close: bigint;
  high: bigint;
  low: bigint;
  /** Base atoms traded across the window. */
  volume: bigint;
  trades: bigint;
  /** `close - open`, in quote atoms. */
  change: bigint;
  /** Percent, or `null` when the window opened at zero. */
  changePct: number | null;
}

export type AuthMode = "login" | "register";

/** What the sign-in panel collects. `name` only matters when registering. */
export interface Credentials {
  username: string;
  name: string;
  password: string;
}

export interface Session {
  user_id: string;
  token: string;
  /**
   * What to call this person on screen. `null` for accounts registered before
   * the API asked for a name, so every reader has to cope without one.
   */
  name: string | null;
}

// ───────────────────────── the private feed ─────────────────────────

export type OrderEvent = "accepted" | "updated" | "cancelled" | "rejected" | "fill";

/**
 * A message on the private `orders` channel.
 *
 * The shape varies by `event`, so the fields beyond it are optional and must be
 * narrowed before use. Your own fills arrive here — with your side, your fee
 * and your role — and never on the public tape.
 */
export interface OrderUpdate {
  event: OrderEvent;
  order_id?: bigint;
  symbol?: string;
  side?: Side;
  order_type?: OrderType;
  price?: bigint;
  qty?: bigint;
  filled_qty?: bigint;
  status?: string;
  role?: Role;
  fee?: bigint;
  /**
   * Position of a `fill` within its batch. `seq` alone is not unique — one
   * command can produce several — so `(seq, idx)` is the identity, and it is
   * the same one `/orders/history` reports.
   */
  idx?: bigint;
  reason?: string;
}
