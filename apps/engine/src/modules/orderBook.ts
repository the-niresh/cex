import { OrderedMap } from "js-sdsl";
import type { Kind, MARKET_ASSETS, Side, Status } from "../utils/types";

interface FillRecord {
  counterPartyUserId: string;
  quantity: number;
  price: number;
}

interface createOrderResponse {
  orderId: string;
  filledQuantity: number;
  totalQuantity: number;
  averagePrice: number;
  status: Status;
  fills: FillRecord[];
}

interface OrderBookOrders {
  orderId: string;
  userId: string;
  totalQuantity: number;
  filledQuantity: number;
  createdAt: Date;
}

interface PriceLevel {
  price: number;
  total: number;
  orders: OrderBookOrders[];
}

interface OrderDetails {
  orderId: string;
  symbol: MARKET_ASSETS;
  quantity: number;
  price: number;
  side: Side;
  kind: Kind;
  status: Status;
  createdAt: Date;
}

interface FillDetails {
  orderId: string;
  buyerId: string;
  sellerId: string;
  filledQuantity: number;
  totalQuantity: number;
  price: number;
  createdAt: Date;
}

interface ReturnCreateUserOrder {
  orderId: string;
  userId: string;
  quantity?: number;
  side: Side;
  kind: Kind;
  status: Status;
  createdAt: Date;
  price?: number;
  symbol: MARKET_ASSETS;
}

interface CancelResult {
  side: Side;
  price: number;
  remainingQuantity: number;
  symbol: MARKET_ASSETS;
}

export default class orderBook {
  private orderBook: Partial<
    Record<
      MARKET_ASSETS,
      {
        BIDS: OrderedMap<number, PriceLevel>;
        ASKS: OrderedMap<number, PriceLevel>;
      }
    >
  >;
  private fills: FillDetails[];
  private orders: Record<string, OrderDetails[]>;

  constructor() {
    this.orderBook = {};
    this.fills = [];
    this.orders = {};
  }

  createLimitOrder(
    userId: string,
    symbol: MARKET_ASSETS,
    quantity: number,
    price: number,
    side: Side,
  ): createOrderResponse & { symbol: MARKET_ASSETS; side: Side } {
    const currentOrder = this.createUserOrder(
      side,
      userId,
      "limit",
      symbol,
      price,
      undefined,
      quantity,
    );
    const assetMarket = this.getOrCreateMarket(symbol);
    const oppositeKey = side === "buy" ? "ASKS" : "BIDS";
    const oppositeSide = assetMarket[oppositeKey];

    let pendingQuantity = quantity;
    const fillRecords: FillRecord[] = [];

    while (pendingQuantity > 0 && !oppositeSide.empty()) {
      const topElement = oppositeSide.front();
      if (!topElement) break;
      const [fillPrice, priceLevel] = topElement;

      if (side === "buy" && fillPrice <= price) break;
      if (side === "sell" && fillPrice >= price) break;

      while (priceLevel.orders.length > 0 && pendingQuantity > 0) {
        const counterOrder = priceLevel.orders[0]!;
        const availableQuantity = counterOrder.totalQuantity - counterOrder.filledQuantity;
        const fillQuantity = Math.min(availableQuantity, pendingQuantity);

        counterOrder.filledQuantity += fillQuantity;
        pendingQuantity -= fillQuantity;
        priceLevel.total -= fillQuantity;

        fillRecords.push({
          counterPartyUserId: counterOrder.userId,
          quantity: fillQuantity,
          price: fillPrice,
        });

        const counterOrders = this.orders[counterOrder.userId];
        if (counterOrders) {
          const o = counterOrders.find(
            (x) => x.orderId === counterOrder.orderId,
          );
          if (o)
            o.status =
              counterOrder.filledQuantity === counterOrder.totalQuantity
                ? "FILLED"
                : "PARTIALLY_FILLED";
        }

        this.fills.push({
          orderId: currentOrder.orderId,
          buyerId: side === "buy" ? userId : counterOrder.userId,
          sellerId: side === "sell" ? userId : counterOrder.userId,
          filledQuantity: fillQuantity,
          totalQuantity: quantity,
          price: fillPrice,
          createdAt: new Date(),
        });

        if (counterOrder.filledQuantity === counterOrder.totalQuantity) {
          priceLevel.orders.shift();
        }
      }

      if (priceLevel.orders.length === 0) {
        oppositeSide.eraseElementByKey(fillPrice);
      }
    }

    if (pendingQuantity > 0) {
      const sameKey = side === "buy" ? "BIDS" : "ASKS";
      const sameSide = assetMarket[sameKey];
      let priceLevel = sameSide.getElementByKey(price);
      if (!priceLevel) {
        priceLevel = { price, orders: [] as OrderBookOrders[], total: 0 };
      }
      priceLevel.orders.push({
        orderId: currentOrder.orderId,
        userId,
        totalQuantity: quantity,
        filledQuantity: quantity - pendingQuantity,
        createdAt: currentOrder.createdAt,
      });
      priceLevel.total += pendingQuantity;
      sameSide.setElement(price, priceLevel);
    }

    const filledQty = quantity - pendingQuantity;
    const status: Status =
      filledQty === quantity
        ? "FILLED"
        : filledQty > 0
          ? "PARTIALLY_FILLED"
          : "PENDING";

    const userOrders = this.orders[userId];
    if (userOrders) {
      const o = userOrders.find((x) => x.orderId === currentOrder.orderId);
      if (o) o.status = status;
    }

    const averagePrice =
      fillRecords.length > 0
        ? fillRecords.reduce((sum, f) => sum + f.price * f.quantity, 0) /
          filledQty
        : price;

    return {
      orderId: currentOrder.orderId,
      status,
      filledQuantity: filledQty,
      totalQuantity: quantity,
      averagePrice,
      symbol,
      side,
      fills: fillRecords,
    };
  }

  createUserOrder(
    side: Side,
    userId: string,
    kind: Kind,
    symbol: MARKET_ASSETS,
    price?: number,
    _budget?: number,
    quantity?: number,
  ): ReturnCreateUserOrder {
    if (!this.orders[userId]) this.orders[userId] = [];
    const orderDetails: OrderDetails = {
      orderId: crypto.randomUUID(),
      side,
      price: price || 0,
      kind,
      quantity: quantity || 0,
      createdAt: new Date(),
      status: "PENDING",
      symbol,
    };
    this.orders[userId]!.push(orderDetails);
    return { ...orderDetails, userId };
  }

  getOrCreateMarket(symbol: MARKET_ASSETS) {
    if (!this.orderBook[symbol]) {
      this.orderBook[symbol] = {
        BIDS: new OrderedMap([], (a: number, b: number) => b - a),
        ASKS: new OrderedMap([], (a: number, b: number) => a - b),
      };
    }
    return this.orderBook[symbol]!;
  }

  createMarketOrder(
    userId: string,
    symbol: MARKET_ASSETS,
    quantity: number,
    side: Side,
    budget: number,
  ): createOrderResponse {
    const assetMarket = this.getOrCreateMarket(symbol);
    const currentOrder = this.createUserOrder(
      side,
      userId,
      "market",
      symbol,
      undefined,
      budget,
      quantity,
    );
    const oppositeKey = side === "buy" ? "ASKS" : "BIDS";
    const fillRecords: FillRecord[] = [];

    if (side === "buy") {
      let remainingBudget = budget;
      let totalFilledQty = 0;
      let totalCost = 0;

      while (remainingBudget > 0 && !assetMarket[oppositeKey].empty()) {
        const topEle = assetMarket[oppositeKey].front()!;
        const [bestPrice, priceLevelOrders] = topEle;
        if (bestPrice > remainingBudget) break;

        const maxQtyAtPrice = Math.floor(remainingBudget / bestPrice);
        const qtyToFill = Math.min(maxQtyAtPrice, priceLevelOrders.total);
        if (qtyToFill <= 0) break;

        let filledAtLevel = 0;
        while (
          priceLevelOrders.orders.length > 0 &&
          filledAtLevel < qtyToFill
        ) {
          const sellerOrder = priceLevelOrders.orders[0]!;
          const sellerRemaining =
            sellerOrder.totalQuantity - sellerOrder.filledQuantity;
          const fillQty = Math.min(sellerRemaining, qtyToFill - filledAtLevel);

          sellerOrder.filledQuantity += fillQty;
          filledAtLevel += fillQty;

          fillRecords.push({
            counterPartyUserId: sellerOrder.userId,
            quantity: fillQty,
            price: bestPrice,
          });

          const sellerFullOrder = this.orders[sellerOrder.userId]?.find(
            (o) => o.orderId === sellerOrder.orderId,
          );
          if (sellerFullOrder) {
            sellerFullOrder.status =
              sellerOrder.filledQuantity === sellerOrder.totalQuantity
                ? "FILLED"
                : "PARTIALLY_FILLED";
          }

          this.fills.push({
            orderId: currentOrder.orderId,
            buyerId: userId,
            sellerId: sellerOrder.userId,
            filledQuantity: fillQty,
            price: bestPrice,
            totalQuantity: quantity,
            createdAt: new Date(),
          });

          if (sellerOrder.filledQuantity === sellerOrder.totalQuantity)
            priceLevelOrders.orders.shift();
        }

        priceLevelOrders.total -= filledAtLevel;
        totalFilledQty += filledAtLevel;
        totalCost += filledAtLevel * bestPrice;
        remainingBudget -= filledAtLevel * bestPrice;

        if (priceLevelOrders.total === 0)
          assetMarket[oppositeKey].eraseElementByKey(bestPrice);
      }

      const status: Status =
        totalFilledQty === quantity
          ? "FILLED"
          : totalFilledQty > 0
            ? "PARTIALLY_FILLED"
            : "CANCELLED";
      const o = this.orders[userId]?.find(
        (x) => x.orderId === currentOrder.orderId,
      );
      if (o) o.status = status;

      return {
        averagePrice: totalFilledQty > 0 ? totalCost / totalFilledQty : 0,
        filledQuantity: totalFilledQty,
        totalQuantity: quantity,
        orderId: currentOrder.orderId,
        status,
        fills: fillRecords,
      };
    }

    // Market sell
    let remainingQuantity = quantity;
    let totalFilledQty = 0;
    let totalRevenue = 0;

    while (remainingQuantity > 0 && !assetMarket[oppositeKey].empty()) {
      const topEle = assetMarket[oppositeKey].front()!;
      const [bestPrice, priceLevelOrders] = topEle;

      const qtyToFill = Math.min(remainingQuantity, priceLevelOrders.total);
      let filledAtLevel = 0;

      while (priceLevelOrders.orders.length > 0 && filledAtLevel < qtyToFill) {
        const buyerOrder = priceLevelOrders.orders[0]!;
        const buyerRemaining =
          buyerOrder.totalQuantity - buyerOrder.filledQuantity;
        const fillQty = Math.min(buyerRemaining, qtyToFill - filledAtLevel);

        buyerOrder.filledQuantity += fillQty;
        filledAtLevel += fillQty;

        fillRecords.push({
          counterPartyUserId: buyerOrder.userId,
          quantity: fillQty,
          price: bestPrice,
        });

        const buyerFullOrder = this.orders[buyerOrder.userId]?.find(
          (o) => o.orderId === buyerOrder.orderId,
        );
        if (buyerFullOrder) {
          buyerFullOrder.status =
            buyerOrder.filledQuantity === buyerOrder.totalQuantity
              ? "FILLED"
              : "PARTIALLY_FILLED";
        }

        this.fills.push({
          orderId: currentOrder.orderId,
          buyerId: buyerOrder.userId,
          sellerId: userId,
          filledQuantity: fillQty,
          price: bestPrice,
          totalQuantity: quantity,
          createdAt: new Date(),
        });

        if (buyerOrder.filledQuantity === buyerOrder.totalQuantity)
          priceLevelOrders.orders.shift();
      }

      priceLevelOrders.total -= filledAtLevel;
      totalFilledQty += filledAtLevel;
      totalRevenue += filledAtLevel * bestPrice;
      remainingQuantity -= filledAtLevel;

      if (priceLevelOrders.total === 0)
        assetMarket[oppositeKey].eraseElementByKey(bestPrice);
    }

    const status: Status =
      totalFilledQty === quantity
        ? "FILLED"
        : totalFilledQty > 0
          ? "PARTIALLY_FILLED"
          : "CANCELLED";
    const o = this.orders[userId]?.find(
      (x) => x.orderId === currentOrder.orderId,
    );
    if (o) o.status = status;

    return {
      averagePrice: totalFilledQty > 0 ? totalRevenue / totalFilledQty : 0,
      filledQuantity: totalFilledQty,
      totalQuantity: quantity,
      orderId: currentOrder.orderId,
      status,
      fills: fillRecords,
    };
  }

  getPriceAfterSweepSimulation(
    quantity: number,
    symbol: MARKET_ASSETS,
    side: Side,
  ): number {
    const assetMarket = this.getOrCreateMarket(symbol);
    const oppositeKey: "ASKS" | "BIDS" = side === "buy" ? "ASKS" : "BIDS";
    let remainingQuantity = quantity;
    let totalCost = 0;

    assetMarket[oppositeKey].forEach(([price, level]: [number, PriceLevel]) => {
      if (remainingQuantity <= 0) return;
      const fillQty = Math.min(remainingQuantity, level.total);
      totalCost += fillQty * price;
      remainingQuantity -= fillQty;
    });

    return quantity > 0 ? totalCost / quantity : 0;
  }

  cancelOrder(userId: string, orderId: string): CancelResult | null {
    const userOrders = this.orders[userId];
    if (!userOrders) return null;

    const orderDetail = userOrders.find((o) => o.orderId === orderId);
    if (!orderDetail) return null;
    if (orderDetail.status === "FILLED" || orderDetail.status === "CANCELLED")
      return null;

    const orderForSymbol = this.orderBook[orderDetail.symbol];
    if (!orderForSymbol || orderDetail.price === undefined) {
      orderDetail.status = "CANCELLED";
      return {
        side: orderDetail.side,
        price: orderDetail.price ?? 0,
        symbol: orderDetail.symbol,
        remainingQuantity: 0,
      };
    }

    const sideKey = orderDetail.side === "buy" ? "BIDS" : "ASKS";
    const priceLevel = orderForSymbol[sideKey].getElementByKey(
      orderDetail.price,
    );

    if (!priceLevel) {
      orderDetail.status = "CANCELLED";
      return {
        side: orderDetail.side,
        price: orderDetail.price,
        symbol: orderDetail.symbol,
        remainingQuantity: 0,
      };
    }

    const obEntry = priceLevel.orders.find(
      (o: OrderBookOrders) => o.orderId === orderId,
    );
    const remainingQuantity = obEntry
      ? obEntry.totalQuantity - obEntry.filledQuantity
      : 0;

    priceLevel.orders = priceLevel.orders.filter(
      (o: OrderBookOrders) => o.orderId !== orderId,
    );
    priceLevel.total -= remainingQuantity;

    if (priceLevel.orders.length === 0) {
      orderForSymbol[sideKey].eraseElementByKey(orderDetail.price);
    }

    orderDetail.status = "CANCELLED";
    return {
      side: orderDetail.side,
      price: orderDetail.price,
      symbol: orderDetail.symbol,
      remainingQuantity,
    };
  }

  getDepth(symbol: MARKET_ASSETS) {
    const assetMarket = this.getOrCreateMarket(symbol);
    const askDepth: [number, number][] = [];
    const bidDepth: [number, number][] = [];

    assetMarket.ASKS.forEach(([price, { total }]: [number, PriceLevel]) =>
      askDepth.push([price, total]),
    );
    assetMarket.BIDS.forEach(([price, { total }]: [number, PriceLevel]) =>
      bidDepth.push([price, total]),
    );

    return { bidDepth, askDepth };
  }

  getUserOrder(userId: string, orderId: string) {
    return this.orders[userId]?.find((o) => o.orderId === orderId) ?? null;
  }

  getFillsOfUser(userId: string) {
    return this.fills.filter(
      (f) => f.buyerId === userId || f.sellerId === userId,
    );
  }
}
