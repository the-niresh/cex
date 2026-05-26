import Balance from "./balance";
import OrderBook from "./orderBook";
import type { MARKET_ASSETS, Side, Kind, CURRENCY_TYPE } from "../utils/types";

interface EngineResponse {
  correlationId: string;
  ok: boolean;
  data?: unknown;
  error?: string;
}

export default class MatchingEngine {
  private orderBook: OrderBook;
  private balance: Balance;

  constructor() {
    this.orderBook = new OrderBook();
    this.balance = new Balance();
  }

  createOrder(
    correlationId: string,
    userId: string,
    symbol: MARKET_ASSETS,
    quantity: number,
    kind: Kind,
    side: Side,
    price?: number,
  ): EngineResponse {
    if (!userId || !symbol || !quantity || !kind || !side) {
      return { correlationId, ok: false, error: "Missing required fields" };
    }

    if (kind === "limit") {
      if (!price) {
        return {
          correlationId,
          ok: false,
          error: "Limit order requires a price",
        };
      }

      if (side === "buy") {
        const totalCost = quantity * price;
        if (this.balance.getUSDBalance(userId) < totalCost) {
          return {
            correlationId,
            ok: false,
            error: "Insufficient USD balance",
          };
        }
        this.balance.deductAssetBalance(userId, totalCost, "USD");

        const orderDetails = this.orderBook.createLimitOrder(
          userId,
          symbol,
          quantity,
          price,
          "buy",
        );

        if (orderDetails.filledQuantity > 0) {
          this.balance.addAssetBalance(
            userId,
            orderDetails.filledQuantity,
            symbol,
          );
        }
        for (const fill of orderDetails.fills) {
          this.balance.addAssetBalance(
            fill.counterPartyUserId,
            fill.quantity * fill.price,
            "USD",
          );
        }

        return { correlationId, ok: true, data: orderDetails };
      }

      // Limit order sell
      const userAssetQty = this.balance.getAssetBalance(userId, symbol);
      if (userAssetQty < quantity) {
        return {
          correlationId,
          ok: false,
          error: `Insufficient ${symbol} balance`,
        };
      }
      this.balance.deductAssetBalance(userId, quantity, symbol);

      const orderDetails = this.orderBook.createLimitOrder(
        userId,
        symbol,
        quantity,
        price,
        "sell",
      );

      if (orderDetails.filledQuantity > 0) {
        const usdEarned = orderDetails.fills.reduce(
          (sum, f) => sum + f.quantity * f.price,
          0,
        );
        this.balance.addAssetBalance(userId, usdEarned, "USD");
      }
      for (const fill of orderDetails.fills) {
        this.balance.addAssetBalance(
          fill.counterPartyUserId,
          fill.quantity,
          symbol,
        );
      }

      return { correlationId, ok: true, data: orderDetails };
    }

    // Market buy
    if (side === "buy") {
      const estimatedAvgPrice = this.orderBook.getPriceAfterSweepSimulation(
        quantity,
        symbol,
        "buy",
      );
      const estimatedCost = quantity * estimatedAvgPrice;
      const usdBalance = this.balance.getUSDBalance(userId);

      if (estimatedCost > usdBalance) {
        return {
          correlationId,
          ok: false,
          error: "Insufficient USD for market buy",
        };
      }

      const budget = estimatedCost > 0 ? estimatedCost : usdBalance;
      this.balance.deductAssetBalance(userId, budget, "USD");

      const orderDetails = this.orderBook.createMarketOrder(
        userId,
        symbol,
        quantity,
        "buy",
        budget,
      );

      if (orderDetails.filledQuantity > 0) {
        this.balance.addAssetBalance(
          userId,
          orderDetails.filledQuantity,
          symbol,
        );
      }

      const actualCost = orderDetails.fills.reduce(
        (sum, f) => sum + f.quantity * f.price,
        0,
      );
      const refund = budget - actualCost;
      if (refund > 0) {
        this.balance.addAssetBalance(userId, refund, "USD");
      }

      for (const fill of orderDetails.fills) {
        this.balance.addAssetBalance(
          fill.counterPartyUserId,
          fill.quantity * fill.price,
          "USD",
        );
      }

      return { correlationId, ok: true, data: orderDetails };
    }

    // Market sell
    const userAssetQty = this.balance.getAssetBalance(userId, symbol);
    if (userAssetQty < quantity) {
      return {
        correlationId,
        ok: false,
        error: `Insufficient ${symbol} for market order`,
      };
    }
    this.balance.deductAssetBalance(userId, quantity, symbol);

    const orderDetails = this.orderBook.createMarketOrder(
      userId,
      symbol,
      quantity,
      "sell",
      0,
    );

    if (orderDetails.filledQuantity > 0) {
      const usdEarned = orderDetails.fills.reduce(
        (sum, f) => sum + f.quantity * f.price,
        0,
      );
      this.balance.addAssetBalance(userId, usdEarned, "USD");
    }

    const unfilled = quantity - orderDetails.filledQuantity;
    if (unfilled > 0) {
      this.balance.addAssetBalance(userId, unfilled, symbol);
    }

    for (const fill of orderDetails.fills) {
      this.balance.addAssetBalance(
        fill.counterPartyUserId,
        fill.quantity,
        symbol,
      );
    }

    return { correlationId, ok: true, data: orderDetails };
  }

  cancelOrder(
    correlationId: string,
    userId: string,
    orderId: string,
  ): EngineResponse {
    const result = this.orderBook.cancelOrder(userId, orderId);

    if (!result) {
      return {
        correlationId,
        ok: false,
        error: "Order not found or already completed",
      };
    }

    if (result.remainingQuantity > 0) {
      if (result.side === "buy") {
        this.balance.addAssetBalance(
          userId,
          result.remainingQuantity * result.price,
          "USD",
        );
      } else {
        this.balance.addAssetBalance(
          userId,
          result.remainingQuantity,
          result.symbol,
        );
      }
    }

    return {
      correlationId,
      ok: true,
      data: { message: `Order ${orderId} cancelled`, orderId },
    };
  }

  getOrderBookDepth(
    correlationId: string,
    symbol: MARKET_ASSETS,
  ): EngineResponse {
    return { correlationId, ok: true, data: this.orderBook.getDepth(symbol) };
  }

  getOrderOfUser(
    correlationId: string,
    userId: string,
    orderId: string,
  ): EngineResponse {
    const order = this.orderBook.getUserOrder(userId, orderId);
    if (!order) {
      return { correlationId, ok: false, error: "Order not found" };
    }
    return { correlationId, ok: true, data: order };
  }

  getUserBalance(correlationId: string, userId: string): EngineResponse {
    const assets = this.balance.getAllAssets(userId);
    const balances = Object.fromEntries(
      assets.map(([asset, total]) => [asset, { available: total, locked: 0 }]),
    );
    return { correlationId, ok: true, data: { userId, balances } };
  }

  getAssetBalance(
    correlationId: string,
    userId: string,
    currencyType: CURRENCY_TYPE = "USD",
  ): EngineResponse {
    const amount =
      currencyType === "USD"
        ? this.balance.getUSDBalance(userId)
        : this.balance.getAssetBalance(userId, currencyType);
    return {
      correlationId,
      ok: true,
      data: { currency: currencyType, available: amount },
    };
  }
}
