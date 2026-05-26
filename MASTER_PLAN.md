# CEX Master Plan — Spot + Perps Exchange
> Target: Portfolio-ready by June 6, 2026

---

## 0. What Are We Building (And Why Each Piece Exists)

Imagine a real exchange like Binance. It has millions of users placing orders every second. If you built it as one big server, it would crash under load and be impossible to reason about. So real exchanges break it into specialised services that talk to each other.

Here is what each piece does:

| Service | Job | Analogy |
|---|---|---|
| **Backend** | Accept HTTP requests, check auth, forward orders | The bank teller who takes your slip |
| **Matching Engine** | Match buyers with sellers, the core brain | The trading floor that actually makes deals |
| **WebSocket Server** | Push real-time data to the browser | Live TV broadcast of prices |
| **DB Poller** | Write confirmed trades to the database | The record-keeper who writes everything in a ledger |
| **Frontend** | React UI for trading | The app you see on screen |

They communicate via **Redis Streams** — a fast message bus. Think of it like WhatsApp channels where each service subscribes to relevant channels.

**Spot vs Perps — same engine, same repo:**
Both live inside a single `apps/engine` process. Spot uses `orderBook.ts` + `matchingEngine.ts`. Perps adds `perpsOrderBook.ts` + `perpsEngine.ts` on top of that. The backend separates them by route prefix: `/order` for spot, `/perps/*` for perps. This avoids the complexity of syncing two separate engines while keeping the code well organised.

---

## 1. Final System Architecture

```
                        ┌─────────────────────────────────────────────┐
                        │           Redis Streams                      │
                        │                                              │
Frontend ──HTTP──► Backend ──XADD──► [orders:stream] ──► Engine       │
                        │                                              │
                        │  Engine ──XADD──► [events:stream]            │
                        │                    │                         │
                        │                    ├──► DB Poller ──► PostgreSQL
                        │                    ├──► WS Server ──► Frontend (live)
                        │                    └──► Backend (HTTP response correlation)
                        │                                              │
                        │  Engine ──────────────────► S3 Snapshots     │
                        │                                              │
                        │  Binance WS API ──► WS Server ──► Frontend   │
                        └─────────────────────────────────────────────┘
```

**Why Redis Streams instead of Redis Lists?**
- Your existing code uses `lPush`/`brPop` (Redis List). That's a simple queue — once Engine reads the message, it's gone.
- Redis Streams keep the message. Multiple consumers (DB Poller, WS Server) can read the SAME message via **consumer groups**. This is how real systems work.
- Think of it like: Redis List = a phone call (one-to-one), Redis Streams = a podcast (one-to-many, you can replay).

---

## 2. Final File Structure

Turborepo monorepo. All services live inside `apps/`. Turborepo lets you run all of them in parallel with one command and share configs.

```
cex/                                         <- Turborepo root
├── apps/
│   ├── backend/                             <- existing code moves here
│   │   ├── src/
│   │   │   ├── index.ts
│   │   │   ├── db.ts
│   │   │   ├── routes/
│   │   │   │   ├── index.ts
│   │   │   │   ├── auth-routes.ts
│   │   │   │   └── exchange-routes.ts       <- add perps routes here
│   │   │   ├── controllers/
│   │   │   │   ├── auth-controller.ts
│   │   │   │   └── exchange-controller.ts
│   │   │   ├── store/
│   │   │   │   └── pending-responses.ts
│   │   │   ├── types/
│   │   │   │   ├── auth-schema.ts
│   │   │   │   ├── exchange-schema.ts       <- add perps order types here
│   │   │   │   ├── engine.ts
│   │   │   │   └── express.d.ts
│   │   │   └── utils/
│   │   │       ├── env.ts
│   │   │       ├── auth.ts
│   │   │       ├── engine-client.ts         <- switch lPush -> XADD (Day 7)
│   │   │       ├── validation.ts
│   │   │       └── async-handler.ts
│   │   └── prisma/
│   │       └── schema.prisma                <- add Order, Fill, Position models
│   │
│   ├── engine/                              <- existing code moves here
│   │   ├── src/
│   │   │   ├── index.ts                     <- switch brPop -> xReadGroup (Day 7)
│   │   │   ├── modules/
│   │   │   │   ├── orderBook.ts             <- fix 3 bugs (Day 1)
│   │   │   │   ├── matchingEngine.ts        <- wire this up (Day 1)
│   │   │   │   ├── balance.ts
│   │   │   │   ├── perpsOrderBook.ts        <- NEW (Day 3)
│   │   │   │   ├── perpsEngine.ts           <- NEW (Day 3)
│   │   │   │   ├── margin.ts                <- NEW (Day 3)
│   │   │   │   ├── liquidator.ts            <- NEW (Day 3)
│   │   │   │   ├── fundingRate.ts           <- NEW (Day 5)
│   │   │   │   └── snapshotter.ts           <- NEW (Day 11)
│   │   │   ├── store/
│   │   │   │   └── exchange-store.ts        <- add POSITIONS (Day 3)
│   │   │   └── utils/
│   │   │       ├── env.ts
│   │   │       └── types.ts                 <- add perps types (Day 3)
│   │   └── package.json
│   │
│   ├── engine-rust/                         <- NEW (Day 11) — Rust matching engine
│   │   ├── src/
│   │   │   ├── main.rs                      <- entry point, Redis Streams loop
│   │   │   ├── order_book.rs                <- order book with BTreeMap
│   │   │   ├── matching_engine.rs           <- routes messages to market threads
│   │   │   └── types.rs                     <- shared Rust types
│   │   └── Cargo.toml                       <- Rust package file (like package.json)
│   │
│   ├── ws-server/                           <- NEW (Day 6)
│   │   ├── src/
│   │   │   ├── index.ts
│   │   │   ├── handlers/
│   │   │   │   ├── ticker.ts                <- Binance price feed
│   │   │   │   ├── orderBook.ts             <- push order book updates to clients
│   │   │   │   └── trades.ts
│   │   │   └── subscriptions/
│   │   │       └── manager.ts
│   │   └── package.json
│   │
│   ├── db-poller/                           <- NEW (Day 7)
│   │   ├── src/
│   │   │   ├── index.ts
│   │   │   └── handlers/
│   │   │       ├── tradeHandler.ts
│   │   │       ├── orderHandler.ts
│   │   │       └── balanceHandler.ts
│   │   └── package.json
│   │
│   └── frontend/                            <- NEW (Day 8)
│       ├── src/
│       │   ├── main.tsx
│       │   ├── App.tsx
│       │   ├── pages/
│       │   │   ├── SpotTrade.tsx
│       │   │   └── PerpsTrade.tsx
│       │   ├── components/
│       │   │   ├── OrderBook/
│       │   │   │   ├── OrderBook.tsx
│       │   │   │   └── OrderRow.tsx
│       │   │   ├── Chart/
│       │   │   │   └── TradingChart.tsx     <- lightweight-charts
│       │   │   ├── OrderForm/
│       │   │   │   ├── SpotOrderForm.tsx
│       │   │   │   └── PerpsOrderForm.tsx
│       │   │   ├── BalancePanel.tsx
│       │   │   ├── TradeHistory.tsx
│       │   │   └── PositionsPanel.tsx
│       │   ├── hooks/
│       │   │   ├── useWebSocket.ts
│       │   │   ├── useOrderBook.ts
│       │   │   └── useTrades.ts
│       │   ├── store/
│       │   │   └── tradingStore.ts          <- Zustand
│       │   └── api/
│       │       └── client.ts
│       ├── package.json
│       └── vite.config.ts
│
├── turbo.json                               <- Turborepo config
├── package.json                             <- root workspace config
├── docker-compose.yml
├── MASTER_PLAN.md
└── README.md
```

---

## 3. Concepts You Must Understand Before Coding Each Day

### What Is an Order Book?
An order book is a list of all buy orders (BIDS) and sell orders (ASKS) sorted by price.
- BIDS: sorted highest price first (someone willing to pay $100 is ahead of $99)
- ASKS: sorted lowest price first (someone willing to sell at $100 is ahead of $101)
- A **match** happens when the highest bid >= lowest ask

```
BIDS (buyers)      ASKS (sellers)
$101  <- 2 BTC      $102 -> 1 BTC
$100  <- 5 BTC      $103 -> 3 BTC
$99   <- 1 BTC      $105 -> 2 BTC
```

### What Is Leverage (Perps)?
In spot trading, if you have $1000, you can only buy $1000 of BTC.
With 10x leverage, you can control a $10,000 position with only $1,000 (your **margin**).
The exchange lends you the rest. If the position moves against you enough that you'd lose your margin, the exchange **liquidates** you (closes your position forcibly) before you go negative.

### What Is Funding Rate?
Perps don't expire (unlike futures). To keep the perp price close to spot price:
- Every 8 hours, longs pay shorts (if perp > spot) or shorts pay longs (if perp < spot)
- This fee is the **funding rate**

### What Is Cross vs Isolated Margin?
- **Isolated margin**: Each position has its own reserved margin. If BTC position gets liquidated, it doesn't touch your ETH position margin.
- **Cross margin**: All your margin is shared. One position can be saved by another's profit, but a big loss can liquidate everything.

### What Is Insurance Fund?
When a position is liquidated but can't be closed at a good enough price, the exchange would lose money. The insurance fund covers this gap. It's filled by liquidation fees.

### What Is ADL (Auto-Deleveraging)?
When the insurance fund runs out, the exchange forcibly reduces the most profitable opposing positions to cover the loss. This is the last resort.

### What Are Worker Threads?
JavaScript is single-threaded. If you have BTC, ETH, and SOL order books, they all share the same thread. Under heavy load, matching BTC orders could delay SOL order processing.
Worker threads let each order book run in its own thread (its own CPU core). They communicate via message passing.

---

## 4. Day-by-Day Implementation Plan

---

### DAY 1 — Monday May 25: Fix Spot Backend Bugs

**Goal**: The existing spot system actually works end-to-end.

**What's broken (in `engine/src/modules/orderBook.ts`):**

**Bug 1** — Line 115, wrong condition:
```typescript
// WRONG (current):
while (pendingQuantity > 0 && oppositeSide.empty()) {

// RIGHT (fix to):
while (pendingQuantity > 0 && !oppositeSide.empty()) {
```
Why: We want to keep matching while there ARE orders on the opposite side. `empty()` returns true when the list is empty — so the old code only ran when there was nothing to match against. Completely backwards.

**Bug 2** — Lines 179-180, wrong property names:
```typescript
// WRONG (current):
priceLevel.orders.push({
  orderId: currentOrder.orderId,
  userId,
  totalQty: quantity,
  filledQty: quantity - pendingQuantity,
  createdAt: currentOrder.createdAt,
});

// RIGHT (fix to):
priceLevel.orders.push({
  orderId: currentOrder.orderId,
  userId,
  totalQuantity: quantity,
  filledQuantity: quantity - pendingQuantity,
  createdAt: currentOrder.createdAt,
});
```
Why: The `OrderBookOrders` interface (line 19-25) uses `totalQuantity` and `filledQuantity`. The push was using wrong names so the data stored in the book has undefined quantity fields.

**Bug 3** — Market orders read wrong names too:
In `createMarketOrder`, when reading from order book entries you use `sellerOrder.totalQty` and `sellerOrder.filledQty` (lines ~293, 294, 373, 374). After fixing Bug 2, the stored values will be under `totalQuantity`/`filledQuantity`. Fix all reads to match.

**Bug 4** — Engine doesn't use MatchingEngine:
The existing `handleEngineRequest()` in `engine/src/index.ts` returns dummy data and doesn't use the `MatchingEngine` or `orderBook` classes. You need to:
1. Import and instantiate `MatchingEngine` once (singleton at top of file)
2. Route each command to the correct method

**Today's tasks:**
1. Fix Bug 1 in `orderBook.ts` (1 line change — add `!`)
2. Fix Bug 2 in `orderBook.ts` (rename 2 properties in the push)
3. Fix Bug 3 in `orderBook.ts` (rename 4 property reads in `createMarketOrder`)
4. Wire the MatchingEngine into `engine/src/index.ts` (implement all 5 operations)
5. Test with curl: place a sell order, place a buy order, check they match

**Files to touch:**
- `engine/src/modules/orderBook.ts` (bug fixes)
- `engine/src/index.ts` (wire MatchingEngine)

---

### DAY 2 — Tuesday May 26: Implement All 5 Engine Operations

**Goal**: Every REST endpoint works. You can place orders, cancel them, check depth, get a specific order.

**Operations to implement in `engine/src/index.ts`:**

```typescript
case "get_depth":
  // Call orderBookInstance.getDepth(symbol)
  // Return { bids: [[price, qty], ...], asks: [[price, qty], ...] }

case "get_order":
  // Call orderBookInstance.getUserOrder(userId, orderId)
  // Return the order details

case "cancel_order":
  // Call matchingEngine.cancelOrder(userId, orderId)
  // This must also refund the locked balance
```

**What `matchingEngine.ts` needs to do on cancel:**
When you place a limit buy order for 1 BTC at $100, the engine locks $100 from your USD balance. If you cancel, it must unlock that $100. Cancelling an order means:
1. Remove from order book
2. Calculate remaining unfilled quantity
3. Unlock `price x remaining_qty` of USD (for buy) or `remaining_qty` of the asset (for sell)

**Files to touch:**
- `engine/src/index.ts`
- `engine/src/modules/matchingEngine.ts` (add cancel with balance refund)

**Test checklist:**
- POST /order (limit buy) -> should lock balance
- POST /order (limit sell at same price) -> both should match
- GET /depth/:symbol -> should show bids/asks
- DELETE /order/:orderId -> should cancel and refund
- GET /order/:orderId -> should return order status

---

### DAY 3 — Wednesday May 27: Perps Engine v1

**Goal**: Users can open leveraged long/short positions. Liquidations work.

**New concepts this day:**
A perps position is fundamentally different from a spot order:
- Spot: you own the asset. Buy 1 BTC = you have 1 BTC.
- Perps: you have a **position**. Long 1 BTC at 10x leverage = you put in $X of margin, you're exposed to the price movement of 1 BTC.

**New data you need to store (add to `exchange-store.ts`):**
```typescript
interface Position {
  positionId: string;
  userId: string;
  symbol: string;
  side: "long" | "short";
  size: number;             // how many contracts (e.g., 1 BTC)
  entryPrice: number;       // the price when they opened
  margin: number;           // USD locked as collateral
  leverage: number;         // 1x to 100x
  liquidationPrice: number; // price at which they get liquidated
  marginType: "isolated" | "cross";
}

// Store: userId -> Map<positionId, Position>
export const POSITIONS = new Map<string, Map<string, Position>>();
```

**New file: `engine/src/modules/margin.ts`**
This calculates:
1. Required margin: `(size x price) / leverage`
2. Liquidation price (isolated margin):
   - Long:  `entryPrice x (1 - 1/leverage + maintenanceMarginRate)`
   - Short: `entryPrice x (1 + 1/leverage - maintenanceMarginRate)`
   - Maintenance margin rate = 0.5% (0.005) for most assets

**New file: `engine/src/modules/liquidator.ts`**
A loop (runs every second) that:
1. Gets the current mark price for each market
2. Checks all open positions
3. If `markPrice <= liquidationPrice` (long) or `markPrice >= liquidationPrice` (short): liquidate
4. To liquidate: close the position at mark price, collect liquidation fee (0.5%), send remainder to insurance fund

**New file: `engine/src/modules/perpsEngine.ts`**
Handles:
- `open_position`: validate margin available, create order in perps order book, record Position
- `close_position`: create opposite order, settle PnL to balance, release margin
- `adjust_leverage`: recalculate margin requirement and liquidation price

**New file: `engine/src/modules/perpsOrderBook.ts`**
Mostly identical to `orderBook.ts` but:
- Orders are settled in USD (no asset transfer)
- Tracks open interest (total size of all positions)

**Spot routes already in `exchange-routes.ts` (no change needed):**
```
POST   /order              -> create spot limit or market order
DELETE /order/:orderId     -> cancel spot order
GET    /order/:orderId     -> get spot order status
GET    /depth/:symbol      -> get spot order book depth
GET    /balance            -> get user balances
```

**New perps routes to add to backend (in `exchange-routes.ts`):**
```
POST   /perps/position           -> open or add to position
DELETE /perps/position/:id       -> close position
GET    /perps/positions          -> get all open positions
GET    /perps/mark-price/:symbol -> get current mark price
```

**Today's tasks:**
1. Write `engine/src/modules/margin.ts` (liquidation price formula)
2. Write `engine/src/modules/perpsOrderBook.ts` (copy + adapt from spot)
3. Write `engine/src/modules/perpsEngine.ts` (open/close position)
4. Write `engine/src/modules/liquidator.ts` (background liquidation loop)
5. Add POSITIONS to `exchange-store.ts`
6. Wire perps operations into `engine/src/index.ts`
7. Add perps routes to backend

---

### DAY 4 — Thursday May 28: Perps v2 — Cross Margin + Isolated Margin

**Goal**: Support both margin modes. Users can switch.

**Isolated Margin (default, simpler):**
- Each position has its own margin pool
- Max loss = initial margin for that position
- Liquidation only affects that one position

**Cross Margin (advanced):**
- All positions share your entire account balance as margin
- One position's profit protects another from liquidation
- But a huge loss can wipe out your whole account

**What changes in `margin.ts` for cross margin:**
For cross margin, the liquidation price depends on ALL your open positions combined, not just one. You need to recalculate whenever a new position opens or closes.

The key formula:
```
available_balance = total_wallet_balance - sum(all_position_initial_margins)
cross_liq_price(long) = entryPrice - (available_balance + position_margin) / size
```

**Changes to `exchange-store.ts`:**
Add per-user margin mode setting: `Map<userId, Map<symbol, "cross" | "isolated">>`
Users can only switch margin mode when they have no open position in that market.

**New route:**
```
POST /perps/margin-mode  -> { symbol, mode: "cross" | "isolated" }
```

**Files to touch:**
- `engine/src/modules/margin.ts` (add cross margin calculations)
- `engine/src/store/exchange-store.ts` (add margin mode store)
- `engine/src/modules/liquidator.ts` (handle cross margin differently)
- `backend/src/routes/exchange-routes.ts` (add margin-mode endpoint)

---

### DAY 5 — Friday May 29: Funding Rate + Insurance Fund + ADL + Trading Fees

**Goal**: The exchange is economically complete.

**New file: `engine/src/modules/fundingRate.ts`**

Funding rate keeps the perp price close to spot price:
```
fundingRate = clamp((perpPrice - spotPrice) / spotPrice, -0.05%, +0.05%)
```
Applied every 8 hours:
- If rate > 0: longs pay shorts `position_size x markPrice x fundingRate`
- If rate < 0: shorts pay longs

```typescript
export class FundingRateEngine {
  private readonly INTERVAL_MS = 8 * 60 * 60 * 1000; // 8 hours

  calculateRate(markPrice: number, indexPrice: number): number {
    const premium = (markPrice - indexPrice) / indexPrice;
    return Math.max(-0.0005, Math.min(0.0005, premium));
  }

  applyFunding(positions: Position[], markPrice: number, rate: number): void {
    // For each position, deduct or credit funding payment from their margin
  }
}
```

**Insurance Fund (add to `exchange-store.ts`):**
```typescript
export const INSURANCE_FUND = new Map<string, number>(); // symbol -> USD amount
```
- When a position is liquidated: liquidation fee = `size x markPrice x 0.5%`
- If closing price is better than liquidation price: surplus goes to insurance fund
- If worse: insurance fund covers the shortfall

**ADL (Auto-Deleveraging) in `liquidator.ts`:**
When insurance fund can't cover losses:
1. Find the most profitable position on the opposite side (sorted by PnL%)
2. Force-close it at the bankruptcy price of the losing position
3. The winning trader gets a notification (in your system: just an event published to the stream)

**Trading Fees:**
```typescript
export const FEES = {
  spot:  { maker: 0.001,  taker: 0.001  }, // 0.1% both
  perps: { maker: 0.0002, taker: 0.0005 }, // 0.02% maker, 0.05% taker
};
// Maker = your limit order sat in the book waiting (you ADD liquidity)
// Taker = your order matched immediately (you REMOVE liquidity)
```

**Better In-Memory Data Structure:**
Currently orders are stored as `Record<string, OrderDetails[]>` — finding one order is O(n) linear search.
Replace with `Map<userId, Map<orderId, OrderDetails>>` — now it's O(1):
```typescript
// Old: this.orders[userId].find(o => o.orderId === id)  <- O(n)
// New: this.orders.get(userId)?.get(orderId)            <- O(1)
```

**Today's tasks:**
1. Write `fundingRate.ts` + wire into engine loop (setInterval for 8h, but test with 30s)
2. Add insurance fund to `exchange-store.ts`
3. Update `liquidator.ts` with insurance fund coverage + ADL fallback
4. Add trading fees to `matchingEngine.ts` and `perpsEngine.ts`
5. Refactor order storage from array to nested Map

---

### DAY 6 — Saturday May 30: WebSocket Server + Binance Price Feed

**Goal**: Real-time data flows to the frontend. Mark prices come from Binance.

**Why a separate WebSocket server?**
The backend handles REST (request -> response pattern). WebSockets are long-lived connections — a client stays connected and receives updates pushed from the server. Mixing them would complicate the backend. The WS server is read-only — it only subscribes to Redis Streams and pushes to clients.

**New package: `apps/ws-server/`**
Init: `bun init` in that folder, install `ws` and `redis`.

**`apps/ws-server/src/index.ts`:**
```typescript
import { WebSocketServer, WebSocket } from 'ws';
import { createClient } from 'redis';

const wss = new WebSocketServer({ port: 8080 });
const redis = createClient({ url: process.env.REDIS_URL });
await redis.connect();

// Subscribe to events stream from engine
// For each event (trade, orderbook_update, ticker): find subscribed clients and push
```

**WebSocket subscription protocol:**
Client sends to subscribe:
```json
{ "type": "subscribe", "channel": "orderbook", "symbol": "BTC" }
{ "type": "subscribe", "channel": "trades",    "symbol": "BTC" }
{ "type": "subscribe", "channel": "ticker",    "symbol": "BTC" }
```
Server pushes back:
```json
{ "type": "orderbook_update", "symbol": "BTC", "bids": [[50000, 2.5]], "asks": [[50001, 1.5]] }
{ "type": "trade", "symbol": "BTC", "price": 50000, "quantity": 0.5, "side": "buy" }
{ "type": "ticker", "symbol": "BTC", "price": 50000, "change24h": 2.3 }
```

**Binance Price Feed (`ws-server/src/handlers/ticker.ts`):**
Binance offers free public WebSocket streams. Connect to:
`wss://stream.binance.com:9443/ws/btcusdt@miniTicker`
This pushes a price update every second. Use this as the **index price** for funding rate calculations.

**`apps/ws-server/src/subscriptions/manager.ts`:**
```typescript
// Map<channel_key, Set<WebSocket>>
// channel_key = "orderbook:BTC", "trades:BTC", "ticker:BTC"
const subscriptions = new Map<string, Set<WebSocket>>();

export function subscribe(ws: WebSocket, channel: string, symbol: string) {
  const key = `${channel}:${symbol}`;
  if (!subscriptions.has(key)) subscriptions.set(key, new Set());
  subscriptions.get(key)!.add(ws);
}

export function broadcast(channel: string, symbol: string, data: unknown) {
  const key = `${channel}:${symbol}`;
  subscriptions.get(key)?.forEach(ws => {
    if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(data));
  });
}
```

**Engine must publish after every match:**
Add to end of `create_order` handling in engine:
```typescript
await redisClient.xAdd('events:stream', '*', {
  type: 'trade',
  symbol: message.payload.symbol,
  price: String(fill.price),
  quantity: String(fill.quantity),
  side: message.payload.side,
  timestamp: String(Date.now()),
});
```

**Today's tasks:**
1. Create `apps/ws-server/` and init package
2. Write `subscriptions/manager.ts`
3. Write `handlers/ticker.ts` (Binance WS integration)
4. Write `index.ts` (WS server + Redis Streams consumer)
5. Update engine to publish to `events:stream` after each match

---

### DAY 7 — Sunday May 31: DB Poller + Redis Streams Migration

**Goal**: Architecture matches the diagram. All trades persist to DB.

**Why DB Poller with consumer groups?**
When a trade happens:
- Engine writes to memory immediately (microseconds)
- DB Poller reads from the stream and writes to PostgreSQL (milliseconds — acceptable lag)
- Multiple DB Pollers can run simultaneously — consumer groups ensure each message is handled exactly once, never duplicated

**Consumer Groups Explained:**
```
Stream: events:stream
  Message 1: trade BTC  (id: 1000-0)
  Message 2: trade ETH  (id: 1001-0)

Consumer Group: "db-poller-group"
  Poller-1 reads message 1 -> ACKs it (marks as processed)
  Poller-2 reads message 2 -> ACKs it
  If Poller-1 crashes before ACK -> message stays "pending" -> can be re-claimed
```

**Migrating Backend from Redis List to Redis Streams:**
Current (`engine-client.ts`):
```typescript
await redisClient.lPush('backend-to-engine-broker', JSON.stringify(message));
```
New:
```typescript
await redisClient.xAdd('orders:stream', '*', {
  payload: JSON.stringify(message),
});
```

Engine current (`index.ts`):
```typescript
const item = await brokerClient.brPop(env.incomingQueue, 0);
```
New:
```typescript
// Create consumer group first (once on startup)
await redisClient.xGroupCreate('orders:stream', 'engine-group', '0', { MKSTREAM: true });

// Then read in loop
const messages = await redisClient.xReadGroup(
  'engine-group', 'engine-worker-1',
  [{ key: 'orders:stream', id: '>' }],
  { COUNT: 1, BLOCK: 0 }
);
// Process message -> ACK it
await redisClient.xAck('orders:stream', 'engine-group', messageId);
```

**New `prisma/schema.prisma` models (add to existing):**
```prisma
model Order {
  id        String   @id @default(cuid())
  userId    String
  symbol    String
  type      String   // "spot" | "perps"
  side      String   // "buy" | "sell" | "long" | "short"
  kind      String   // "limit" | "market"
  price     Float?
  quantity  Float
  filledQty Float    @default(0)
  status    String   // "PENDING" | "FILLED" | "PARTIALLY_FILLED" | "CANCELLED"
  createdAt DateTime @default(now())
  user      User     @relation(fields: [userId], references: [id])
}

model Fill {
  id        String   @id @default(cuid())
  orderId   String
  buyerId   String
  sellerId  String
  symbol    String
  price     Float
  quantity  Float
  fee       Float
  createdAt DateTime @default(now())
}

model Position {
  id               String    @id @default(cuid())
  userId           String
  symbol           String
  side             String    // "long" | "short"
  size             Float
  entryPrice       Float
  margin           Float
  leverage         Int
  liquidationPrice Float
  status           String    // "open" | "closed" | "liquidated"
  createdAt        DateTime  @default(now())
  closedAt         DateTime?
  user             User      @relation(fields: [userId], references: [id])
}
```

**New `apps/db-poller/src/index.ts`:**
```typescript
await redis.xGroupCreate('events:stream', 'db-poller-group', '0', { MKSTREAM: true })
  .catch(() => {}); // ignore "group already exists" error

while (true) {
  const result = await redis.xReadGroup(
    'db-poller-group', 'poller-1',
    [{ key: 'events:stream', id: '>' }],
    { COUNT: 10, BLOCK: 1000 }
  );
  if (!result) continue;

  for (const { id, message } of result[0].messages) {
    if (message.type === 'trade')          await handleTrade(message);
    if (message.type === 'order_created')  await handleOrder(message);
    if (message.type === 'balance_update') await handleBalance(message);
    await redis.xAck('events:stream', 'db-poller-group', id);
  }
}
```

**Today's tasks:**
1. Update `prisma/schema.prisma` with Order, Fill, Position models
2. Run `bun prisma migrate dev` to apply to DB
3. Migrate `engine-client.ts` from `lPush` to `xAdd`
4. Migrate `engine/src/index.ts` from `brPop` to `xReadGroup`
5. Create `apps/db-poller/` package
6. Write `index.ts`, `handlers/tradeHandler.ts`, `handlers/orderHandler.ts`

---

### DAY 8 — Monday June 1: Frontend Setup + Spot Trading UI

**Goal**: You can trade spot on a working UI.

**Setup commands:**
```bash
cd apps
bun create vite@latest frontend -- --template react-ts
cd frontend
bun add zustand @tanstack/react-query axios lightweight-charts ws
```

**Why Zustand?**
Global state manager — simpler than Redux. Your trading data (order book, balances, positions) lives here and any component can read it without prop drilling.

**Why `lightweight-charts`?**
TradingView's open-source charting library. The same technology Binance uses for their charts.

**Spot trading page layout:**
```
+---------------------------------------------+
|  [BTC/USD v]    $50,000 (+2.3%)             |
+---------------+-----------------------------+
|               |         Chart               |
|  Order Book   |                             |
|               +-----------------------------+
|  ASKS         |  Buy/Sell Form              |
|  $50001 1.5   |  [Limit] [Market]           |
|  $50000 2.0   |  Price: [_______]           |
|  -----------  |  Amount:[_______]           |
|  BIDS         |  [Buy BTC]   [Sell BTC]     |
|  $49999 0.5   |                             |
+---------------+-----------------------------+
|  Open Orders | Trade History | Balances     |
+---------------------------------------------+
```

**Key hook — `hooks/useOrderBook.ts`:**
```typescript
export function useOrderBook(symbol: string) {
  const [orderBook, setOrderBook] = useState<{ bids: [number,number][], asks: [number,number][] }>
    ({ bids: [], asks: [] });

  useEffect(() => {
    const ws = new WebSocket('ws://localhost:8080');
    ws.onopen = () => ws.send(JSON.stringify({
      type: 'subscribe', channel: 'orderbook', symbol
    }));
    ws.onmessage = (e) => {
      const data = JSON.parse(e.data);
      if (data.type === 'orderbook_update') setOrderBook(data);
    };
    return () => ws.close();
  }, [symbol]);

  return orderBook;
}
```

**Today's tasks:**
1. Init Vite React app in `apps/frontend/`
2. Set up Zustand store in `store/tradingStore.ts`
3. Build `OrderBook.tsx` (connected to WS via `useOrderBook`)
4. Build `SpotOrderForm.tsx` (calls POST /order via axios)
5. Build `BalancePanel.tsx` (calls GET /balance)
6. Build `TradingChart.tsx` (lightweight-charts with candlestick data from Binance)
7. Wire everything in `SpotTrade.tsx`

---

### DAY 9 — Tuesday June 2: Frontend Perps Trading UI

**Goal**: Perps trading UI with positions panel, leverage slider, live PnL.

**Perps trading page layout:**
```
+---------------------------------------------+
|  [BTC-PERP v]  Mark: $50,000  Idx: $49,998 |
|                Funding: +0.01% in 3h        |
+---------------+-----------------------------+
|               |         Chart               |
|  Order Book   |                             |
|               +-----------------------------+
|  ASKS         |  [LONG]        [SHORT]      |
|               |  Leverage: [===O===] 10x    |
|  -----------  |  Mode: [Isolated][Cross]    |
|  BIDS         |  Amount: [_______] USD      |
|               |  Cost: $500 (10x)           |
|               |  Liq Price: ~$45,454        |
+---------------+-----------------------------+
|  Positions                                  |
|  BTC Long 10x  Size:1  Entry:$50k           |
|  PnL: +$120 (+2.4%)  Liq:$45,454  [Close] |
+---------------------------------------------+
```

**`PerpsOrderForm.tsx` key feature — live liquidation price preview:**
As user moves the leverage slider, recalculate the liquidation price in real time and show it:
```typescript
const liqPrice = useMemo(() => {
  if (!markPrice || !leverage || !amount) return null;
  const margin = amount / leverage;
  const maintenanceRate = 0.005;
  if (side === 'long') {
    return markPrice * (1 - 1/leverage + maintenanceRate);
  } else {
    return markPrice * (1 + 1/leverage - maintenanceRate);
  }
}, [markPrice, leverage, amount, side]);
```

**`PositionsPanel.tsx` — real-time PnL:**
PnL updates every time a new ticker price arrives via WebSocket:
```typescript
const unrealizedPnL = useMemo(() => {
  if (position.side === 'long') {
    return (currentPrice - position.entryPrice) * position.size;
  } else {
    return (position.entryPrice - currentPrice) * position.size;
  }
}, [currentPrice, position]);
```

**Today's tasks:**
1. Build `PerpsOrderForm.tsx` with leverage slider (1x-100x) and liquidation price preview
2. Build `PositionsPanel.tsx` with live PnL
3. Build `PerpsTrade.tsx` page
4. Add React Router: `/spot` -> SpotTrade, `/perps` -> PerpsTrade
5. Add navigation header with market switcher

---

### DAY 10 — Wednesday June 3: Learn Rust Syntax

**Goal**: Get comfortable enough with Rust to write the matching engine tomorrow.

This is a self-study day. No new CEX code. Just Rust fundamentals.

**What to study (in this order):**

1. **Ownership & borrowing** — the most important concept in Rust. Every value has one owner. When the owner goes out of scope, the value is dropped (freed). You can borrow a reference (`&T`) without taking ownership.
   ```rust
   let s = String::from("hello"); // s owns the string
   let r = &s;                    // r borrows it (read-only)
   println!("{}", r);             // fine
   println!("{}", s);             // also fine — s still owns it
   ```

2. **Structs** — Rust's equivalent of TypeScript interfaces + classes combined:
   ```rust
   struct Order {
       order_id: String,
       user_id: String,
       price: f64,
       quantity: f64,
   }
   ```

3. **Enums** — far more powerful than TypeScript enums:
   ```rust
   enum Side { Buy, Sell }
   enum OrderKind { Limit, Market }
   ```

4. **`impl` blocks** — how you add methods to a struct:
   ```rust
   impl Order {
       fn new(user_id: &str, price: f64) -> Self {
           Order { order_id: uuid(), user_id: user_id.to_string(), price, quantity: 0.0 }
       }
   }
   ```

5. **`std::collections::BTreeMap`** — Rust's built-in sorted map. This is what replaces `js-sdsl OrderedMap` for price levels. It keeps keys sorted automatically.
   ```rust
   use std::collections::BTreeMap;
   let mut bids: BTreeMap<u64, Vec<Order>> = BTreeMap::new();
   // Keys are always sorted ascending. For bids (highest first) you iterate in reverse.
   ```

6. **Threads** — Rust's threads are real OS threads (not green threads):
   ```rust
   use std::thread;
   let handle = thread::spawn(|| {
       println!("I run on my own OS thread");
   });
   handle.join().unwrap();
   ```

7. **`Arc<Mutex<T>>`** — how you share data between threads safely:
   - `Arc` = Atomic Reference Counted (shared ownership across threads)
   - `Mutex` = Mutual Exclusion (only one thread can access the data at a time)
   ```rust
   use std::sync::{Arc, Mutex};
   let shared_book = Arc::new(Mutex::new(OrderBook::new()));
   let book_clone = Arc::clone(&shared_book);
   thread::spawn(move || {
       let mut book = book_clone.lock().unwrap(); // lock before accessing
       book.add_order(/* ... */);
   }); // lock released automatically when `book` goes out of scope
   ```

8. **Channels** — how threads send messages to each other (like `postMessage` in Node Worker Threads):
   ```rust
   use std::sync::mpsc; // multi-producer, single-consumer
   let (tx, rx) = mpsc::channel::<String>();
   thread::spawn(move || {
       tx.send("hello from thread".to_string()).unwrap();
   });
   let msg = rx.recv().unwrap(); // blocks until message arrives
   ```

**Resources:**
- https://doc.rust-lang.org/book/ (The Book — chapters 1-16 cover everything above)
- Focus on: ch 4 (ownership), ch 8 (collections), ch 16 (concurrency)

**Install Rust if not already:**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# or on Windows: download rustup-init.exe from rustup.rs
rustc --version  # verify
cargo --version  # cargo = Rust's package manager (like bun/npm)
```

---

### DAY 11 — Thursday June 4: Rust Multithreaded Matching Engine

**Goal**: Replace the TypeScript engine with a Rust binary. Each market gets its own OS thread.

**Why Rust here?**
- Rust is genuinely multi-threaded — no GIL, no event loop limitations
- `BTreeMap` is built-in and sorted — perfect for price levels
- Zero-cost abstractions — the matching loop compiles to machine code, not interpreted
- Great portfolio signal: most CEX engineers use Rust or C++ for the hot path

**Architecture:**
```
Main thread: connects to Redis, reads orders:stream
             routes each message to the correct market thread via channel

BTC thread:  has its own OrderBook, receives orders via channel, sends results back
ETH thread:  same, completely independent
SOL thread:  same
```

**New package: `apps/engine-rust/`**

Init with: `cargo new engine-rust` inside `apps/`

**`Cargo.toml`** (Rust's package.json):
```toml
[package]
name = "engine-rust"
version = "0.1.0"
edition = "2021"

[dependencies]
redis = { version = "0.25", features = ["tokio-comp"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
```

**`src/types.rs`** — shared types:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Side { Buy, Sell }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrderKind { Limit, Market }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingOrder {
    pub correlation_id: String,
    pub response_queue: String,
    pub symbol: String,
    pub user_id: String,
    pub side: Side,
    pub kind: OrderKind,
    pub price: Option<f64>,
    pub quantity: f64,
}

#[derive(Debug, Serialize)]
pub struct FillRecord {
    pub price: f64,
    pub quantity: f64,
    pub counter_party_user_id: String,
}
```

**`src/order_book.rs`** — the matching engine per market:
```rust
use std::collections::BTreeMap;
use crate::types::{Side, FillRecord};

pub struct PriceLevel {
    pub total: f64,
    pub orders: Vec<LevelOrder>,
}

pub struct LevelOrder {
    pub order_id: String,
    pub user_id: String,
    pub total_quantity: f64,
    pub filled_quantity: f64,
}

pub struct OrderBook {
    bids: BTreeMap<u64, PriceLevel>, // key = price * 1_000_000 (integer for exact sort)
    asks: BTreeMap<u64, PriceLevel>,
}

impl OrderBook {
    pub fn new() -> Self {
        OrderBook { bids: BTreeMap::new(), asks: BTreeMap::new() }
    }

    pub fn create_limit_order(
        &mut self, user_id: &str, price: f64, quantity: f64, side: &Side
    ) -> Vec<FillRecord> {
        let price_key = (price * 1_000_000.0) as u64;
        let mut pending = quantity;
        let mut fills = Vec::new();

        match side {
            Side::Buy => {
                // Match against asks (lowest ask first = ascending iteration)
                let keys: Vec<u64> = self.asks.keys()
                    .filter(|&&k| k <= price_key) // only asks at or below our bid price
                    .cloned().collect();
                for key in keys {
                    if pending <= 0.0 { break; }
                    // ... fill logic
                }
                // If still pending, add to bids
                if pending > 0.0 {
                    self.bids.entry(price_key).or_insert(PriceLevel { total: 0.0, orders: vec![] });
                    // add order to level
                }
            }
            Side::Sell => { /* mirror of buy, iterating bids in reverse */ }
        }
        fills
    }

    pub fn get_depth(&self) -> (Vec<(f64, f64)>, Vec<(f64, f64)>) {
        let bids: Vec<(f64, f64)> = self.bids.iter().rev() // highest first
            .map(|(k, v)| (*k as f64 / 1_000_000.0, v.total))
            .collect();
        let asks: Vec<(f64, f64)> = self.asks.iter()      // lowest first
            .map(|(k, v)| (*k as f64 / 1_000_000.0, v.total))
            .collect();
        (bids, asks)
    }
}
```

**`src/main.rs`** — main thread + market threads:
```rust
use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;

mod order_book;
mod types;
use types::IncomingOrder;

fn main() {
    // One channel sender per market symbol
    let mut senders: HashMap<String, mpsc::Sender<IncomingOrder>> = HashMap::new();

    // Spawn a thread for each market
    for symbol in ["BTC", "ETH", "SOL"] {
        let (tx, rx) = mpsc::channel::<IncomingOrder>();
        senders.insert(symbol.to_string(), tx);

        thread::spawn(move || {
            let mut book = order_book::OrderBook::new();
            while let Ok(order) = rx.recv() {
                let fills = book.create_limit_order(
                    &order.user_id, order.price.unwrap_or(0.0),
                    order.quantity, &order.side
                );
                // Send result back via Redis response queue
                // (use a separate Redis client per thread)
                println!("[{}] Filled {} orders", symbol, fills.len());
            }
        });
    }

    // Main thread: read from Redis orders:stream and route to correct market thread
    loop {
        // redis XREADGROUP -> parse -> senders[symbol].send(order)
    }
}
```

**Today's tasks:**
1. `cargo new engine-rust` inside `apps/`
2. Write `src/types.rs` with all shared structs
3. Write `src/order_book.rs` with `BTreeMap`-based limit order matching
4. Write `src/main.rs` — spawn one thread per market, route via channels
5. Connect Redis using the `redis` crate (read from `orders:stream`, write responses)
6. Test: the Rust engine processes orders from the same Redis stream as the old TypeScript engine

---

### DAY 12 — Friday June 4: S3 Snapshotting

**Goal**: Engine state is periodically saved so data isn't lost on restart.

**Why S3?**
The engine holds all state in RAM. Server crash = all open orders and positions gone.
Snapshots: every 10 minutes, serialize everything to JSON and upload to S3.
On startup: load the latest snapshot to restore state.

**New file: `engine/src/modules/snapshotter.ts`:**
```typescript
import { S3Client, PutObjectCommand, ListObjectsV2Command, GetObjectCommand } from '@aws-sdk/client-s3';

export class Snapshotter {
  private s3 = new S3Client({ region: process.env.AWS_REGION! });
  private bucket = process.env.SNAPSHOT_BUCKET!;

  async save(state: { balances: any; orderbooks: any; positions: any }): Promise<void> {
    const key = `snapshots/${Date.now()}.json`;
    await this.s3.send(new PutObjectCommand({
      Bucket: this.bucket,
      Key: key,
      Body: JSON.stringify({ timestamp: Date.now(), ...state }),
      ContentType: 'application/json',
    }));
    console.log(`Snapshot saved: ${key}`);
  }

  async loadLatest(): Promise<any | null> {
    const list = await this.s3.send(new ListObjectsV2Command({
      Bucket: this.bucket,
      Prefix: 'snapshots/',
    }));
    if (!list.Contents?.length) return null;
    // Sort by key (keys are timestamps) and get latest
    const latest = list.Contents.sort((a, b) =>
      (b.Key ?? '').localeCompare(a.Key ?? '')
    )[0];
    const obj = await this.s3.send(new GetObjectCommand({
      Bucket: this.bucket, Key: latest.Key!
    }));
    return JSON.parse(await streamToString(obj.Body));
  }
}

// Snapshot every 10 minutes
const snapshotter = new Snapshotter();
setInterval(async () => {
  await snapshotter.save(serializeState());
}, 10 * 60 * 1000);
```

**The tricky part — serializing `OrderedMap`:**
`OrderedMap` from `js-sdsl` can't be JSON stringified directly.
Write a helper:
```typescript
function serializeOrderBooks() {
  const result: Record<string, { bids: any[]; asks: any[] }> = {};
  for (const [symbol, market] of Object.entries(ORDERBOOKS)) {
    result[symbol] = {
      bids: [...market.BIDS].map(([price, level]) => ({ price, level })),
      asks: [...market.ASKS].map(([price, level]) => ({ price, level })),
    };
  }
  return result;
}
```

For local dev without real AWS, use **LocalStack**:
Add to `docker-compose.yml`:
```yaml
  localstack:
    image: localstack/localstack
    ports:
      - "4566:4566"
    environment:
      - SERVICES=s3
```
Then use endpoint: `http://localhost:4566` in your S3 client.

**Today's tasks:**
1. Install `@aws-sdk/client-s3` in engine package
2. Write `snapshotter.ts` (save + loadLatest)
3. Write `serializeState()` and `deserializeState()` helpers
4. Call `loadLatest()` on engine startup before starting to process messages
5. Add LocalStack to `docker-compose.yml` for local dev

---

### DAY 12 — Friday June 5-6: Polish + Portfolio Prep

**Goal**: Everything works together, looks good, can be demoed convincingly.

**Checklist:**
- [ ] Full end-to-end test: login -> spot trade -> perps position -> liquidation
- [ ] Fix any bugs found during testing
- [ ] Add meaningful error messages to frontend (balance too low, order rejected, etc.)
- [ ] Write `docker-compose.yml` so the whole system starts with `docker compose up`
- [ ] Write a strong `README.md` with:
  - Architecture diagram (copy from this doc)
  - "How to run" section (docker compose up, then visit localhost:5173)
  - Features list (spot, perps, leverage up to 100x, funding rates, liquidations, multithreaded, snapshotting)
  - Tech stack table
- [ ] Record a 2-minute demo:
  1. Sign up + see default balances
  2. Place a limit sell order (BTC at some price)
  3. Place a matching limit buy order -> watch them fill in the UI
  4. Check balance updated
  5. Open a 10x long perps position
  6. Watch liquidation price and PnL update in real time

---

## 5. Quick Reference: Key Technologies

| Technology | Why We Use It |
|---|---|
| **Bun** | Fast JS runtime + package manager |
| **Express** | HTTP server framework |
| **Prisma** | Type-safe database ORM |
| **PostgreSQL** | Relational DB for persistent storage |
| **Redis Streams** | Fast message bus between services |
| **js-sdsl `OrderedMap`** | Sorted structure for price levels (O(log n) insert) |
| **Worker Threads** | OS-level parallelism for order books |
| **WebSocket (`ws`)** | Real-time bidirectional communication |
| **React + Vite** | Fast frontend with hot reload |
| **Zustand** | Lightweight React state management |
| **lightweight-charts** | TradingView-compatible candlestick charts |
| **AWS S3 / LocalStack** | Object storage for state snapshots |
| **Zod** | Runtime type validation |
| **JWT** | Stateless authentication tokens |
| **bcryptjs** | Password hashing |

---

## 6. docker-compose.yml (Final)

```yaml
version: '3.8'
services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: cex
      POSTGRES_USER: cex
      POSTGRES_PASSWORD: secret
    ports:
      - "5432:5432"
    volumes:
      - postgres_data:/var/lib/postgresql/data

  redis:
    image: redis:7-alpine
    ports:
      - "6379:6379"

  localstack:
    image: localstack/localstack
    ports:
      - "4566:4566"
    environment:
      - SERVICES=s3

  backend:
    build: ./apps/backend
    ports:
      - "3000:3000"
    env_file: ./apps/backend/.env
    depends_on: [postgres, redis]

  engine:
    build: ./apps/engine
    env_file: ./apps/engine/.env
    depends_on: [redis, localstack]

  ws-server:
    build: ./apps/ws-server
    ports:
      - "8080:8080"
    env_file: ./apps/ws-server/.env
    depends_on: [redis]

  db-poller:
    build: ./apps/db-poller
    env_file: ./apps/db-poller/.env
    depends_on: [postgres, redis]

  frontend:
    build: ./apps/frontend
    ports:
      - "5173:80"
    depends_on: [backend, ws-server]

volumes:
  postgres_data:
```

Start everything: `docker compose up --build`

---

## 7. Known Bugs in Existing Code (Fix These First)

| File | Line | Bug | Fix |
|---|---|---|---|
| `engine/src/modules/orderBook.ts` | 115 | `oppositeSide.empty()` — should be `!oppositeSide.empty()` | Add `!` |
| `engine/src/modules/orderBook.ts` | 179 | `totalQty` — should be `totalQuantity` | Rename |
| `engine/src/modules/orderBook.ts` | 180 | `filledQty` — should be `filledQuantity` | Rename |
| `engine/src/modules/orderBook.ts` | ~293 | `sellerOrder.totalQty` — should be `sellerOrder.totalQuantity` | Rename read |
| `engine/src/modules/orderBook.ts` | ~294 | `sellerOrder.filledQty` — should be `sellerOrder.filledQuantity` | Rename read |
| `engine/src/modules/orderBook.ts` | ~373 | `buyerOrder.totalQty` — should be `buyerOrder.totalQuantity` | Rename read |
| `engine/src/modules/orderBook.ts` | ~374 | `buyerOrder.filledQty` — should be `buyerOrder.filledQuantity` | Rename read |
| `engine/src/index.ts` | 78-93 | `create_order` returns dummy — doesn't use MatchingEngine | Wire real engine |
| `engine/src/index.ts` | 96 | `get_depth`, `get_order`, `cancel_order` throw TODO | Implement |

---

## 8. Daily Checklist

Before ending each day:
- [ ] TypeScript build passes (`bun run build` — no errors)
- [ ] Manual test of today's feature with curl or Postman
- [ ] Git commit with meaningful message describing what and why
- [ ] Update this plan if anything changed

---

*Last updated: May 25, 2026*
