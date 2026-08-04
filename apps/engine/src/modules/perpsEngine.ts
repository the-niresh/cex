import type { MARKET_ASSETS } from "../utils/types.ts";
import {
  MAINTENANCE_MARGIN_RATE,
  TAKER_FEE_RATE,
  type AccountSnapshot,
  type LiquidationRecord,
  type Position,
  type PositionSide,
  type PositionView,
} from "../utils/perps-types.ts";

export interface PerpsResult {
  ok: boolean;
  data?: unknown;
  error?: string;
}


export default class PerpsEngine {
  // userId -> free USD collateral not locked in a position
  private collateral: Record<string, number>;
  // userId -> symbol -> the single net position
  private positions: Record<string, Partial<Record<MARKET_ASSETS, Position>>>;
  // symbol -> latest mark price
  private markPrices: Partial<Record<MARKET_ASSETS, number>>;
  private liquidations: LiquidationRecord[];

  constructor() {
    this.collateral = {};
    this.positions = {};
    this.markPrices = {};
    this.liquidations = [];
  }


  deposit(userId: string, amount: number): PerpsResult {
    if (amount <= 0) return { ok: false, error: "Deposit must be positive" };
    this.collateral[userId] = (this.collateral[userId] ?? 0) + amount;
    return { ok: true, data: { userId, freeCollateral: this.collateral[userId] } };
  }

  private getFreeCollateral(userId: string): number {
    return this.collateral[userId] ?? 0;
  }


  setMarkPrice(symbol: MARKET_ASSETS, price: number): PerpsResult {
    if (price <= 0) return { ok: false, error: "Mark price must be positive" };
    this.markPrices[symbol] = price;
    const liquidated = this.runLiquidations(symbol);
    return { ok: true, data: { symbol, markPrice: price, liquidated } };
  }

  private getMarkPrice(symbol: MARKET_ASSETS): number | undefined {
    return this.markPrices[symbol];
  }

  openPosition(
    userId: string,
    symbol: MARKET_ASSETS,
    side: PositionSide,
    size: number,
    leverage: number,
  ): PerpsResult {
    if (size <= 0) return { ok: false, error: "Size must be positive" };
    if (leverage <= 0 || leverage > 100) {
      return { ok: false, error: "Leverage must be between 1 and 100" };
    }

    // WithOut a mark price we have no price to enter/liquidate against.
    const mark = this.getMarkPrice(symbol);
    if (mark === undefined) {
      return { ok: false, error: `No mark price for ${symbol} yet` };
    }

    const existing = this.positions[userId]?.[symbol];
    // v1 keeps flips simple: you must close a position before opening the other side.
    if (existing && existing.side !== side) {
      return {
        ok: false,
        error: "Opposite-side position open — close it first (flips are v1.5)",
      };
    }

    // Enter at the mark price for v1 (no perps order book yet).
    const entryPrice = mark;
    const notional = size * entryPrice;
    const requiredMargin = notional / leverage;
    const fee = notional * TAKER_FEE_RATE;
    const cost = requiredMargin + fee;

    if (this.getFreeCollateral(userId) < cost) {
      return {
        ok: false,
        error: `Insufficient collateral: need ${cost.toFixed(2)}, have ${this.getFreeCollateral(userId).toFixed(2)}`,
      };
    }

    this.collateral[userId] = this.getFreeCollateral(userId) - cost;
    if (!this.positions[userId]) this.positions[userId] = {};

    if (!existing) {
      const position: Position = {
        userId,
        symbol,
        side,
        size,
        entryPrice,
        leverage,
        margin: requiredMargin,
        liquidationPrice: 0,
        createdAt: Date.now(),
      };
      position.liquidationPrice = this.computeLiquidationPrice(position);
      this.positions[userId]![symbol] = position;
      return { ok: true, data: this.viewPosition(position, mark) };
    }

    // Adding to a same-side position: volume-weighted average entry, pooled margin.
    const totalSize = existing.size + size;
    existing.entryPrice =
      (existing.entryPrice * existing.size + entryPrice * size) / totalSize;
    existing.size = totalSize;
    existing.margin += requiredMargin;
    existing.leverage = leverage;
    existing.liquidationPrice = this.computeLiquidationPrice(existing);
    return { ok: true, data: this.viewPosition(existing, mark) };
  }

  closePosition(userId: string, symbol: MARKET_ASSETS): PerpsResult {
    const position = this.positions[userId]?.[symbol];
    if (!position) return { ok: false, error: "No open position" };

    const mark = this.getMarkPrice(symbol);
    if (mark === undefined) return { ok: false, error: `No mark price for ${symbol}` };

    const pnl = this.unrealizedPnl(position, mark);
    const fee = position.size * mark * TAKER_FEE_RATE;
    const returned = Math.max(0, position.margin + pnl - fee);

    this.collateral[userId] = this.getFreeCollateral(userId) + returned;
    delete this.positions[userId]![symbol];

    return {
      ok: true,
      data: {
        symbol,
        realizedPnl: pnl,
        fee,
        returnedToCollateral: returned,
        freeCollateral: this.getFreeCollateral(userId),
      },
    };
  }

  // ---- PnL / margin math ------------------------------------------------

  /**
   * Unrealized PnL at a given mark price.
   *   long:  (mark - entry) * size   (you profit when price rises)
   *   short: (entry - mark) * size   (you profit when price falls)
   */
  private unrealizedPnl(position: Position, mark: number): number {
    const diff =
      position.side === "long"
        ? mark - position.entryPrice
        : position.entryPrice - mark;
    return diff * position.size;
  }

  /** Maintenance margin = current notional * maintenance rate. */
  private maintenanceMargin(position: Position, mark: number): number {
    return position.size * mark * MAINTENANCE_MARGIN_RATE;
  }

    const m = position.margin / position.size;
    const mmr = MAINTENANCE_MARGIN_RATE;
    if (position.side === "long") {
      return (position.entryPrice - m) / (1 - mmr);
    }
    return (position.entryPrice + m) / (1 + mmr);
  }

  // ---- liquidations -----------------------------------------------------

  /** Sweep one market and force-close every position whose equity <= maintenance. */
  private runLiquidations(symbol: MARKET_ASSETS): LiquidationRecord[] {
    const mark = this.getMarkPrice(symbol);
    if (mark === undefined) return [];

    const fired: LiquidationRecord[] = [];
    for (const userId of Object.keys(this.positions)) {
      const position = this.positions[userId]?.[symbol];
      if (!position) continue;

      const equity = position.margin + this.unrealizedPnl(position, mark);
      const maintenance = this.maintenanceMargin(position, mark);
      if (equity > maintenance) continue; // still healthy

      // Liquidate: seize the remaining margin (in v2 this feeds the insurance fund).
      const record: LiquidationRecord = {
        userId,
        symbol,
        side: position.side,
        size: position.size,
        entryPrice: position.entryPrice,
        markPrice: mark,
        marginSeized: Math.max(0, equity),
        at: Date.now(),
      };
      this.liquidations.push(record);
      fired.push(record);
      delete this.positions[userId]![symbol];
      // Note: with isolated margin the user's free collateral is untouched —
      // only this position's margin is lost. That is the whole point of isolation.
    }
    return fired;
  }

  // ---- views / getters --------------------------------------------------

  private viewPosition(position: Position, mark: number): PositionView {
    const unrealizedPnl = this.unrealizedPnl(position, mark);
    const maintenance = this.maintenanceMargin(position, mark);
    const equity = position.margin + unrealizedPnl;
    return {
      ...position,
      markPrice: mark,
      notional: position.size * mark,
      unrealizedPnl,
      maintenanceMargin: maintenance,
      marginRatio: maintenance > 0 ? equity / maintenance : Infinity,
    };
  }

  getPosition(userId: string, symbol: MARKET_ASSETS): PerpsResult {
    const position = this.positions[userId]?.[symbol];
    if (!position) return { ok: false, error: "No open position" };
    const mark = this.getMarkPrice(symbol);
    if (mark === undefined) return { ok: false, error: `No mark price for ${symbol}` };
    return { ok: true, data: this.viewPosition(position, mark) };
  }

  getAccount(userId: string): PerpsResult {
    const positions: PositionView[] = [];
    const userPositions = this.positions[userId] ?? {};
    for (const symbol of Object.keys(userPositions) as MARKET_ASSETS[]) {
      const position = userPositions[symbol];
      const mark = this.getMarkPrice(symbol);
      if (position && mark !== undefined) {
        positions.push(this.viewPosition(position, mark));
      }
    }
    const lockedEquity = positions.reduce(
      (sum, p) => sum + p.margin + p.unrealizedPnl,
      0,
    );
    const snapshot: AccountSnapshot = {
      userId,
      freeCollateral: this.getFreeCollateral(userId),
      positions,
      totalEquity: this.getFreeCollateral(userId) + lockedEquity,
    };
    return { ok: true, data: snapshot };
  }

  getLiquidations(userId?: string): PerpsResult {
    const data = userId
      ? this.liquidations.filter((l) => l.userId === userId)
      : this.liquidations;
    return { ok: true, data };
  }
}
