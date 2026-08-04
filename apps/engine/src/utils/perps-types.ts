import type { MARKET_ASSETS } from "./types.ts";

// A perp position is directional: you are LONG (betting price up) or SHORT (betting price down).
// This is different from spot where you either own the asset or you don't.
export type PositionSide = "long" | "short";

// v1 is a SINGLE, ISOLATED-MARGIN, ONE-WAY system:
//  - single collateral asset (USD) backs every position
//  - isolated margin: each position has its own margin bucket; a loss can only wipe that
//    bucket, never the rest of your wallet
//  - one-way: at most one net position per market per user (no simultaneous long+short)
export interface Position {
  userId: string;
  symbol: MARKET_ASSETS;
  side: PositionSide;
  size: number; // number of contracts / units of the base asset (e.g. BTC)
  entryPrice: number; // volume-weighted average price you entered at
  leverage: number; // e.g. 10 => you control 10x your posted margin
  margin: number; // USD locked as isolated margin for THIS position
  liquidationPrice: number; // mark price at which the position gets force-closed
  createdAt: number;
}

// What the engine hands back when you ask for an account snapshot.
export interface AccountSnapshot {
  userId: string;
  freeCollateral: number; // USD not locked in any position
  positions: PositionView[];
  totalEquity: number; // freeCollateral + sum(margin + unrealizedPnl)
}

// A position enriched with live, mark-price-derived numbers.
export interface PositionView extends Position {
  markPrice: number;
  notional: number; // size * markPrice — the real economic exposure
  unrealizedPnl: number; // profit/loss if you closed right now at mark
  marginRatio: number; // equity / maintenanceMargin; <= 1 means liquidatable
  maintenanceMargin: number;
}

export interface LiquidationRecord {
  userId: string;
  symbol: MARKET_ASSETS;
  side: PositionSide;
  size: number;
  entryPrice: number;
  markPrice: number; // price at which liquidation fired
  marginSeized: number; // isolated margin lost to the (future) insurance fund
  at: number;
}

// Maintenance margin rate: the minimum equity-to-notional ratio before liquidation.
// Real exchanges tier this by position size; v1 uses a flat 0.5%.
export const MAINTENANCE_MARGIN_RATE = 0.005;

// Taker fee taken on notional when opening/closing. Kept tiny and flat for v1.
export const TAKER_FEE_RATE = 0.0004;
