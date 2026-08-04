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
| `ws` — market data fan-out | Not started |
| `persist` — Postgres history writer | Not started |
| Perpetuals | Not started |

**Spot trading works end to end.** Two users can register, deposit, place orders, match, and settle
over HTTP. What is missing is the live market-data feed and the historical record — you can trade,
but a UI has nothing to stream and `GET /trades` has no source yet.

## Running it

```bash
docker compose up -d                     # redis on 6390, postgres on 5442
cargo build --release

./target/release/engine &                # consumes cex:commands
CEX_JWT_SECRET=$(openssl rand -hex 32) \
  ./target/release/api &                 # listens on :8080
```

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
cargo test                # 230 tests; the engine and api suites need both containers
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
* **`ws` and `persist` do not exist.** No live market-data feed, and no queryable history.
* **A `504` from the API is genuinely ambiguous.** The command is on the durable log and may still
  be applied, so a timeout is not proof that nothing happened. Re-read `/orders/open` to find out.
* **Replay republishes events.** Recovery re-applies commands after the snapshot, so downstream
  consumers see duplicates and must deduplicate on `seq`.

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
