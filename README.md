# cex

A centralised exchange in Rust. Spot markets first, perpetual futures on the same engine.

The exchange is a **deterministic state machine**: one function, `apply(state, command) -> events`,
running single-threaded over a durable command log. Everything else — HTTP, WebSocket, Postgres,
snapshots — is plumbing around that.

## Architecture

![Spot architecture](docs/architecture.svg)

Every arrow is a real network hop between separate processes. The write path runs left to right
along the top and loops back along the bottom; because everything that mutates state goes through
`cex:commands`, that one stream is a complete, replayable record of everything that ever happened.

Perpetual futures reuse all of it — same order book, same matching loop, same recovery — adding a
price feed and three command types:

![Perps architecture](docs/perps-architecture.jpeg)

## Status

| Component | State |
|---|---|
| `cex-core` — matching, ledger, settlement, snapshots | Built · 118 tests |
| `cex-proto` — wire types | Built · 18 tests |
| `engine` — stream consumer, snapshots, crash recovery | Built · 36 tests |
| `api` — loopback, auth, REST routes | Built · 58 tests |
| `persist` — Postgres history writer | Built · 27 tests |
| `ws` — market data fan-out | Built · 52 tests |
| Perpetuals | Not started |

**The spot exchange is complete.** Two users can register, deposit, place orders, match and settle
over HTTP; every order, fill and balance change lands in Postgres behind the engine; and the book,
the trades and each user's own orders stream live over WebSocket. What is left is perpetuals, and
the gaps listed below.

## Running it

```bash
docker compose up -d                     # redis on 6390, postgres on 5442
cargo build --release

./target/release/engine &                # consumes cex:commands
CEX_JWT_SECRET=$(openssl rand -hex 32) \
  ./target/release/api &                 # listens on :8080
./target/release/persist &               # cex:events → postgres
CEX_JWT_SECRET=$SECRET \
  ./target/release/ws &                  # cex:events → websocket, on :8081
```

`ws` must be given the **same** `CEX_JWT_SECRET` as `api`, or it cannot verify the tokens `api`
issues and every private subscription is refused. It exits at boot rather than serve a feed whose
private channels silently never work.

`persist` is optional to trade — the engine does not wait on it, and stopping it costs history
freshness and nothing else. Give each deployed instance its own stable `CEX_PERSIST_CONSUMER`
name: Redis holds unacknowledged entries against the name that received them, so a name that
changed on every boot would orphan its own backlog.

```bash
# register, fund, and trade
TOKEN=$(curl -s -XPOST localhost:8080/register \
  -H 'content-type: application/json' \
  -d '{"username":"alice","password":"a-good-password"}' | jq -r .token)

curl -s -XPOST localhost:8080/deposit -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' -d '{"asset":"USDT","amount":1000000000}'

curl -s -XPOST localhost:8080/orders -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"symbol":"BTC_USDT","side":"BUY","order_type":"LIMIT",
       "time_in_force":"GTC","price":50000000000,"qty":100000}'

curl -s localhost:8080/depth/BTC_USDT

# watch the book and the tape
cargo run -p cex-ws --example tail -- trades@BTC_USDT depth@BTC_USDT
# watch your own orders
cargo run -p cex-ws --example tail -- --token "$TOKEN" orders
```

Amounts are integers in atomic units — see [Scaling](#scaling).

## Endpoints

| Method | Path | Auth | |
|---|---|---|---|
| `GET` | `/health` | — | liveness |
| `POST` | `/register` | — | create an account, returns a token |
| `POST` | `/login` | — | exchange credentials for a token |
| `GET` | `/markets` | — | tradable pairs and their tick, lot and fee rules |
| `GET` | `/depth/:symbol` | — | current order book |
| `POST` | `/deposit` | yes | credit an account |
| `GET` | `/balances` | yes | available and locked, per asset |
| `POST` | `/orders` | yes | place a limit or market order |
| `DELETE` | `/orders/:id` | yes | cancel a resting order |
| `GET` | `/orders/open` | yes | your live orders |

## Scaling

Prices and quantities are integers, never floats.

* A **quantity** counts `10^-base_decimals` units of the base asset. BTC has 8 decimals, so
  `qty: 100000` is 0.001 BTC.
* A **price** counts quote atomic units per *one whole* base unit. USDT has 6 decimals, so
  `price: 50000000000` is 50,000.00 USDT per BTC.
* Therefore `notional = price × qty / 10^base_decimals`, in quote atoms.

## Getting started

```bash
docker compose up -d      # redis on 6390, postgres on 5442
cargo test                # 309 tests; the engine, api, persist and ws suites need both containers
cargo clippy --all-targets -- -D warnings
```

The toolchain is pinned in `rust-toolchain.toml`; rustup will fetch the right compiler
automatically.

## Layout

```
crates/
├── proto/     # every message that crosses a process boundary
├── core/      # the engine. no tokio, no redis, no clock, no f64
├── engine/    # [bin] command stream in → core → event stream out
├── api/       # [bin] REST + auth + loopback to the engine
├── ws/        # [bin] market data fan-out
└── persist/   # [bin] event stream → Postgres
```

`core` has no async dependencies **on purpose**. A crate that cannot perform I/O cannot
accidentally become non-deterministic, and the constraint is enforced by the manifest rather
than by review.

## History

`persist` reads `cex:events` and writes four tables. It is a separate process because **the engine
must never wait on a database**: the engine publishes a batch and moves on in microseconds while
the persister catches up at whatever speed Postgres allows.

| Table | |
|---|---|
| `event_batches` | one row per applied command, keyed on `seq`. The dedupe guard. |
| `orders` | one row per order, updated in place as it fills or is cancelled |
| `fills` | one row per match, immutable, keyed `(seq, idx)` |
| `balance_changes` | append-only trail of every balance-affecting event |

Unlike the engine, it reads with `XREADGROUP`: it has no snapshot of its own, so it wants exactly
what a consumer group gives — Redis tracks the cursor, and anything unacknowledged comes back.

Delivery is therefore at-least-once, twice over: Redis redelivers what was never acknowledged, and
the engine republishes events whenever recovery replays the command log. Both are handled the same
way. A batch and the row recording that it was written commit in **one transaction**, so a crash
anywhere leaves history exactly as it was and the redelivery that follows re-does the work cleanly.

## Market data

`ws` reads `cex:events` through its own consumer group — separate from the persister's, so the two
read the same stream without competing — and fans every update out over a
`tokio::sync::broadcast` channel. The stream is read **once** no matter how many clients are
connected; one Redis reader per connection would multiply load on the stream by the number of
subscribers, which is exactly backwards for the one component whose job is to have a lot of them.

| Channel | |
|---|---|
| `depth@SYMBOL` | public. Incremental book updates, carrying the monotonic `depth_seq` |
| `trades@SYMBOL` | public. Trade prints |
| `orders` | **private.** Your own orders and your own fills. Requires a token |

```json
{"op": "auth", "token": "..."}
{"op": "subscribe", "channels": ["depth@BTC_USDT", "orders"]}
```

Unlike the persister, this group starts at the **tail** of the stream. History and live data want
opposite things: a batch `persist` never wrote is a hole in the record forever, whereas replaying
yesterday's depth deltas into a fresh connection would not be catching up, it would be lying about
the state of the book.

Two rules this crate exists to keep:

**A public channel never carries a user id.** `cex_proto::Fill` names both counterparties, so
forwarding one to `trades@SYMBOL` would tell everyone who traded with whom. The public message is a
separate type with no user fields, so the leak does not compile rather than relying on review.

**A slow subscriber is dropped, never allowed to stall the others.** Each connection has its own
cursor into a shared ring buffer, so one that stops reading falls behind alone. When it falls off
the end its connection is closed and it is told why — a client that carried on would be rebuilding
a book from a feed with a silent hole in it, which is wrong without ever looking wrong.

## Design rules

These are not preferences. Breaking any one of them breaks something that depends on it.

**1. The engine is pure.** No clock, no randomness, no sockets, no file reads inside `apply`.
Anything from the outside world — timestamps, mark prices, funding ticks — enters as a command
appended to the log first. This is what makes snapshot-and-replay recovery exact. Break it and
replay silently produces different state than the original run.

**2. Money is integers.** `i64` counts of atomic units, `i128` for intermediate products, one
`mul_div` helper with explicit rounding direction. `f64` must never appear in `core`.

**3. Reads are not logged.** State-changing requests are `Command` and go on the durable stream.
Read-only requests are `Query` and travel on a separate channel. Logging reads would bloat the
log and slow every replay for no benefit.

**4. Locked balances are real.** Funds backing a resting order move from `available` to `locked`
and are released exactly, never recomputed. `check_invariants()` asserts after every command that
supply is conserved and that every locked atom is backed by a live order.

**5. Fills print at the maker's price.** Price improvement belongs to the taker, and any
difference between the reservation and the actual cost is refunded immediately.

**6. Exactly one engine per command stream.** The engine reads with plain `XREAD`, not a consumer
group, so a second instance would read the same commands and apply everything twice. There is no
lock enforcing this yet — see Known gaps.

## Known gaps

Named rather than buried, because each is a real thing to fix:

* **Nothing prevents two engines running.** Both would consume the whole stream and double-apply
  every command. Needs a Redis lock at boot.
* **There is no `GET /trades/:symbol` yet.** The data is now there — `persist` writes it and
  `HistoryStore::fills_for_symbol` reads it — but no route serves it.
* **A `504` from the API is genuinely ambiguous.** The command is on the durable log and may still
  be applied, so a timeout is not proof that nothing happened. Re-read `/orders/open` to find out.
* **Replay republishes events.** Recovery re-applies commands after the snapshot, so downstream
  consumers see duplicates and must deduplicate on `seq`. `persist` does it against a table and
  `ws` against an in-memory high-water mark; anything new must do it too.
* **A batch `persist` cannot write stalls history rather than skipping it.** The entries stay
  unacknowledged and are retried forever. That is the right failure — better a stalled writer that
  pages you than one that quietly drops trades — but it does need someone watching for it.

## Naming conventions

Spot and perpetuals share this repository, this engine, and this order book. The conventions
below keep them distinguishable without duplicating anything.

### Market symbols

| Kind | Format | Example |
|---|---|---|
| Spot | `BASE_QUOTE` | `BTC_USDT` |
| Perpetual | `BASE_QUOTE_PERP` | `BTC_USDT_PERP` |

A `Market` carries a `kind` discriminator; the suffix is a human convenience, never the thing
the code branches on. Never parse a symbol string to decide behaviour — look up the market.

### Crates

Package names are prefixed `cex-`; directories are not. `crates/core` is the package `cex-core`.
Binary crates keep their bare directory name as the executable (`engine`, `api`, `ws`, `persist`),
because that is what gets typed on a server.

### Modules in `core`

```
math.rs       shared    fixed-point arithmetic
market.rs     shared    market definitions, tick/lot rules
book.rs       shared    the order book. identical for spot and perps
balances.rs   shared    the asset ledger
spot.rs       spot      spot settlement
positions.rs  perps     position ledger
perps.rs      perps     funding, liquidation, mark price
state.rs      shared    apply(), dispatching on market kind
```

Perpetuals are **additive**. They do not fork the order book, the matching loop, or the recovery
mechanism — they add command variants, an event or two, and a position ledger alongside the
balance ledger.

### Commands

Neutral verbs shared by both (`Deposit`, `Withdraw`, `PlaceOrder`, `CancelOrder`) stay unqualified.
Perpetual-only commands are named for what they do, not for the product: `SetMarkPrice`,
`SettleFunding`, `Liquidate`, `ClosePosition`.

## Terminology

| Term | Meaning |
|---|---|
| maker | The resting order. Provided liquidity, pays the lower fee. |
| taker | The incoming order that removed liquidity. Pays the higher fee. |
| bps | Basis point. 1 bps = 0.01%. |
| notional | Value of a trade in the quote asset: `price × qty`. |
| tick size | Smallest permitted price increment. |
| lot size | Smallest permitted quantity increment. |
| atom | The smallest indivisible unit of an asset. All money is counted in these. |
| base / quote | In `BTC_USDT`, BTC is the base (what you buy), USDT the quote (what you pay with). |
