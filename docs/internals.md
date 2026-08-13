# Internals

The detail behind [the README](../README.md): the wire contract, the invariants, and the
reasoning behind the parts that are easy to get wrong. Not required reading to run the thing.

## Endpoints


| Method | Path | Auth | |
|---|---|---|---|
| `GET` | `/health` | — | liveness |
| `POST` | `/register` | — | create an account (`username`, `password`, optional `name`), returns a token |
| `POST` | `/login` | — | exchange credentials for a token |
| `GET` | `/markets` | — | tradable pairs and their tick, lot and fee rules |
| `GET` | `/depth/:symbol` | — | current order book |
| `POST` | `/deposit` | yes | credit an account |
| `GET` | `/balances` | yes | available and locked, per asset |
| `POST` | `/orders` | yes | place a limit or market order |
| `DELETE` | `/orders/:id` | yes | cancel a resting order |
| `GET` | `/orders/open` | yes | your live orders |
| `GET` | `/orders/history` | yes | your own fills, newest first (`?limit=`, max 500) |
| `GET` | `/trades/:symbol` | — | recent trade prints, newest first (`?limit=`, max 500) |
| `GET` | `/candles/:symbol` | — | OHLCV bars, **oldest first** (`?interval=1m\|5m\|15m\|1h\|4h\|1d`, `?limit=`, max 500) |

`/trades/:symbol` is the public tape and names nobody. `/orders/history` is the same rows scoped
to the caller and told from their side — their order id, their side, their role, the fee they
paid — and still says nothing about who was on the other side of the trade.

`/candles/:symbol` aggregates the same fills into OHLCV bars, and is **a display projection and
nothing more**. It buckets on `created_at` — when the persister wrote the row, not when the trade
matched, because the engine owns no clock by design. That is good enough to draw a chart with and
wrong for anything that prices, values or settles, so nothing but a chart may read it. Open and
close come from `(seq, idx)`, the order the engine really matched the fills in, rather than from
the timestamp. Prices stay quote atoms and volume base atoms all the way to the wire; the chart
divides for display, and design rule 2 does not bend for a chart.

Bars come back oldest first — a chart draws left to right — while `limit` still keeps the *newest*
buckets. Intervals are a fixed set rather than a parsed duration, because the value becomes a
divisor inside the aggregate.

A browser may call the API from the origins named in `CEX_CORS_ORIGINS` (comma-separated;
unset means the Vite dev server). Listed explicitly, never `*`: a wildcard would hand a
token-bearing API to any page on the internet. `authorization` and `idempotency-key` are in the
allowed set, because a browser silently strips any header that is not.

## Scaling


Prices and quantities are integers, never floats.

* A **quantity** counts `10^-base_decimals` units of the base asset. BTC has 8 decimals, so
  `qty: 100000` is 0.001 BTC.
* A **price** counts quote atomic units per *one whole* base unit. USDT has 6 decimals, so
  `price: 50000000000` is 50,000.00 USDT per BTC.
* Therefore `notional = price × qty / 10^base_decimals`, in quote atoms.

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
log and slow every replay for no benefit. Queries are answered by a task of their own — its own
Redis connection, `BRPOP`ing that channel — running concurrently with the command loop's blocking
`XREAD`, so a read is never held up by the command stream being idle. The two share the engine's
state behind a lock, so a read is still exact: it always reflects every command already applied,
never a half-applied one.

**4. Locked balances are real.** Funds backing a resting order move from `available` to `locked`
and are released exactly, never recomputed. `check_invariants()` asserts after every command that
supply is conserved and that every locked atom is backed by a live order.

**5. Fills print at the maker's price.** Price improvement belongs to the taker, and any
difference between the reservation and the actual cost is refunded immediately.

**6. Exactly one engine per command stream.** The engine reads with plain `XREAD`, not a consumer
group, so a second instance would read the same commands and apply everything twice. A Redis lease
on `<commands-stream>:lock` enforces it: the engine takes it at boot, renews it while it runs, and
stops the moment it cannot. A second engine refuses to start and names the one holding the stream.

## Measured


What `State::apply` actually costs, in isolation, with no Redis and no HTTP anywhere near it.
`crates/core/benches/apply.rs` seeds a `BTC_USDT` book to a given depth, then times one call to
`apply` per iteration — a fresh `State` clone and a fresh `request_id` each time, so neither the
idempotency log nor a draining book skews the number (see `crates/core/tests/bench_guards.rs`,
which pins both of those assumptions).

Reproduce with:

```bash
cargo bench -p cex-core
```

**The first published version of this table was wrong**, and worth saying plainly rather than
silently redrawing. The routine closure took its `(State, Command)` input by value and returned
only the `Result`, so the cloned `State` was dropped *inside* criterion's timed region —
`iter_batched`'s own docs say as much: setup is excluded from timing, but the input's `Drop`
is not. Dropping a depth-`d` `State` frees on the order of `2d` orders, `2d` price levels each
holding a `VecDeque`, and `2d` idempotency entries — an O(d) cost that dominated every one of the
twelve original numbers, including `market_sweep_half`, and produced growth that looked like a
matching-cost problem but was actually a benchmark bug. The fix returns the state alongside the
result so criterion drops it after the clock stops:

```rust
|(mut state, cmd)| {
    let r = state.apply(cmd);
    (state, black_box(r))
}
```

Point estimate of the mean and the median, from criterion's `estimates.json`, at three book
depths (resting price levels per side):

| Case | depth | mean | median |
|---|---|---|---|
| `limit_rest` — rests below best bid, no match | 10 | 3.81 µs | 0.90 µs |
| | 100 | 5.71 µs | 1.61 µs |
| | 1,000 | 7.21 µs | 4.59 µs |
| `limit_cross_one` — one match against best ask | 10 | 5.23 µs | 1.79 µs |
| | 100 | 16.45 µs | 3.00 µs |
| | 1,000 | 29.00 µs | 6.79 µs |
| `market_sweep_half` — market buy through half the asks | 10 | 9.91 µs | 4.22 µs |
| | 100 | 142.43 µs | 77.74 µs |
| | 1,000 | 1276.22 µs | 831.50 µs |
| `cancel` — lookup and removal, no matching | 10 | 4.52 µs | 0.88 µs |
| | 100 | 13.16 µs | 1.96 µs |
| | 1,000 | 38.28 µs | 4.05 µs |

`market_sweep_half` climbs with depth as expected — a sweep sized to half the resting asks does
more matching work the deeper the book is: roughly 14.4x (mean) from depth 10 to 100 and 9x from
100 to 1,000, tracking the growth in swept quantity at each step. `limit_rest`, which touches at
most one price level and never matches, stays close to flat by the median (0.90 → 1.61 → 4.59 µs)
— the shape the brief expected once the drop-time bug above is fixed.

**The mean sits 2–9x above the median in every case**, which means the box this ran on was noisy
— background load from other work on the shared machine, most likely, rather than anything in
the engine. `cancel`'s median stays close to flat (0.88 → 1.96 → 4.05 µs, matching its O(1)
tombstone design) while its mean grows 8.5x, which is exactly what heavy-tailed noise looks like:
a handful of slow outlier iterations pull the mean up without moving the typical case. Re-running
on a quieter box would likely tighten these numbers; the table above is what this machine actually
produced, reported honestly rather than smoothed over.

### Load driver: HTTP ack vs private-feed visibility

`crates/loadgen` (`cargo run -p cex-loadgen`) drives real orders through a running exchange and
times two things per order: `ack` is the HTTP response returning from `POST /orders`; `visible`
is the same order reaching this client's own `orders` channel on the private WebSocket feed —
the point at which a real trader would actually see the order took effect. A third histogram,
`engine`, is the server's own `x-cex-engine-us` response header: the exchange's own view of how
long it spent inside the engine, independent of where the load driver happens to be running.

**Correlation.** Several orders can land inside one depth delta on the public book feed, so a
depth delta cannot be attributed to any single order. The driver therefore does not use the
public depth path at all; it correlates on the private `orders` feed's
`OrderUpdate::Accepted { order_id, .. }` event, which carries the order's own id, so each
measurement belongs to exactly one order.

Measured against the isolated local stack (`.superpowers/sdd/2026-08-11-latency-measurement-and-readout/local-stack.sh up`
— redis db 3, `loadtest:*` streams, postgres schema `loadtest`, fully separated from the live
exchange behind api.niresh.tech), one maker seeding 2,000 resting asks and one taker sending
2,000 market buys against them, one order at a time — each order's HTTP round trip and feed wait
complete before the next order is sent, so this is serialized single-order latency, not
throughput under concurrent load:

```bash
cargo run -p cex-loadgen -- --host http://localhost:8080 --ws ws://localhost:8081 --count 2000
```

| | count | p50 | p90 | p99 | p999 | max |
|---|---|---|---|---|---|---|
| `ack` — HTTP response | 2000 | 1014 µs | 4615 µs | 9599 µs | 14943 µs | 17167 µs |
| `visible` — private feed | 2000 | 1082 µs | 4939 µs | 11159 µs | 40671 µs | 43871 µs |
| `server` — `x-cex-server-us` | 2000 | 772 µs | 3845 µs | 8583 µs | 12191 µs | 16591 µs |
| `engine` — `x-cex-engine-us` | 2000 | 738 µs | 3799 µs | 8407 µs | 12135 µs | 16527 µs |

All four come from one run against a freshly reset stack, so they are directly comparable. The
ordering `visible ≥ ack ≥ server ≥ engine` holds at every percentile, which is the ordering the
construction requires: each is measured strictly inside the next one out.

⚠️ **`server` minus `engine` is not the Redis command hop**, though it looks like the obvious
place to read one off. `x-cex-engine-us` is recorded around the entire `Loopback` round trip
(`crates/api/src/loopback.rs`), which already contains the `XADD` out, the engine's apply, and
the reply coming back. The gap between the two headers is what the API process does *outside*
that round trip: routing, JWT verification, deserialising the request, serialising the response.
At ~34 µs (p50) it is the right order of magnitude for either, which is exactly why the
mislabelling would have been easy to publish and hard to notice.

`visible` is slower than `ack` at every percentile, as expected: the private-feed message is
strictly downstream of the HTTP response, so `visible` pays for the full HTTP round trip and
then also whatever it takes the acceptance event to reach this WebSocket connection through the
engine → ws pipeline. `engine` sits below all of them, as it should. The difference between the
`visible` and `ack` numbers at the same rank (~68 µs at p50,
growing to ~25.7ms at p999) is a rough sense of what that event path costs, not a measured
latency of any one thing — it is a difference of percentiles, not a percentile of per-order
differences.

**`visible ≥ ack` on every sample is guaranteed by construction, not something this run
discovered.** Both are timed from the same `Instant` (`ack.record(started.elapsed()...)` runs
right after the HTTP response, `visible.record(started.elapsed()...)` runs later in the same
iteration, after `wait_for_order` returns), and `Instant::elapsed` is monotonic — so the
ordering holds pointwise for every single sample regardless of whether the event being waited
on is actually *this* order's. A correlation bug that matched some other order's `Accepted`
event would still show `visible ≥ ack`, as long as that event happened to arrive after the HTTP
response, which it almost always would. What actually establishes the correlation is right is
`is_order_accepted` in `crates/loadgen/src/main.rs`, which compares the private feed's own
`order_id` field rather than matching on event type alone, together with its mutation-tested
unit test `does_not_match_the_accepted_event_for_a_different_order_id`: deleting the id
comparison reintroduces exactly the "matches a previous order's event" bug, and that test is the
one that catches it. The size of the `visible`/`ack` gap above is weak corroboration at best — a
bug that matched a very recently stale event would still show a small positive gap — not proof.

Raw per-bucket CSVs, from which every percentile above can be recomputed, land in
`target/latency/{ack,visible,engine,server}.csv`.

A run that fails to collect either header on every single order aborts with a non-zero exit
rather than publishing a percentile drawn from a set nobody knows is short. Both headers are
load-bearing: `server`'s p99 is where the trading screen's degraded threshold comes from, and
both appear in the budget below.

### The latency budget

Every segment below is measured. None is estimated, and each row names the command that
produced it.

| Segment | p50 | Where it comes from |
|---|---|---|
| matching (`apply`), depth 100, one match | 3.00 µs (median) | `cargo bench -p cex-core`, `limit_cross_one/100` |
| engine round trip, incl. both Redis hops | 738 µs | `x-cex-engine-us`, the `engine` row above |
| API's own work outside that round trip | ~34 µs | `server` minus `engine` at the same rank |
| whole request, server side | 772 µs | `x-cex-server-us`, the `server` row above |
| event path, order accepted → private feed | ~68 µs | `visible` minus `ack` at the same rank |
| network | not measured | needs a run from off-box; see below |

Three limitations, stated rather than buried:

* **The engine holds no clock, by design.** That is why chart candles bucket on persister write
  time. Every end-to-end number here is measured at the edges by `loadgen`; none is derived
  from event timestamps inside the system.
* **The two "minus" rows are differences of percentiles, not percentiles of per-order
  differences.** They give the order of magnitude of a segment, not its distribution.
* **The Redis command hop is not separately measured.** It sits inside the engine round trip
  and cannot be split out without giving the engine a clock — see the warning above about what
  `server` minus `engine` actually is.

The network row is deliberately blank rather than estimated. Filling it needs the same driver
run against the deployed exchange and subtracted from the localhost run, which requires the
deployed build to be emitting these headers in the first place.

## Retrying safely


A `504` from the API is genuinely ambiguous: the command is already on the durable log, so a
timeout is not proof that nothing happened. Send an `Idempotency-Key` and a retry becomes safe.

```bash
curl -XPOST localhost:8080/orders -H "authorization: Bearer $TOKEN" \
  -H 'idempotency-key: my-order-1' -H 'content-type: application/json' \
  -d '{"symbol":"BTC_USDT","side":"BUY","order_type":"LIMIT",
       "time_in_force":"GTC","price":50000000000,"qty":100000}'
```

Send it again with the same key and you get the same `order_id` back, not a second order. Accepted
on `POST /deposit`, `POST /orders` and `DELETE /orders/:id`.

The API turns the key into the command's `request_id` — a UUIDv5 derived from the key **and the
caller**, so two users may pick `"1"` without either receiving the other's answer. The engine
remembers ids it has applied along with what each one returned, so a repeat is answered from that
record: no state changes, no events are published, and `seq` does not advance.

Three things worth knowing:

* **It is opt-in.** Without the header every request gets a fresh id, because two deliberate
  identical orders are a normal thing to want.
* **Rejected commands are not remembered.** A command that failed changed nothing, so re-running it
  is harmless — and remembering the failure would leave you unable to retry a request that never
  happened.
* **The window is 50,000 commands.** Beyond that the id is forgotten and a retry applies as new.

Because every applied command is recorded, the command log itself is exactly-once: appending the
same command twice, however that happens, applies it once.

## Running exactly one engine


The engine takes a lease on `<commands-stream>:lock` before it reads anything, renews it once a
third of the lease has gone, and **stops** if a renewal finds the lock is no longer its own. A
second engine on the same stream refuses to start:

```
Error: the command stream cex:commands is already owned by engine engine-ab5bb596-...
```

Two engines on two different command streams is a legitimate deployment, so the key is derived from
the stream name rather than being global.

Stopping matters as much as starting. On `SIGTERM` the engine snapshots, releases the lease and
exits, so a replacement boots immediately — a deploy costs nothing. After a `kill -9` nothing is
released and the replacement waits out the lease instead, which is the right trade: an engine that
stopped answering is not necessarily an engine that stopped running.

`CEX_LOCK_TTL_MS` sets the lease (default 30s). `CEX_BLOCK_MS` must stay below a third of it, or
the loop could sit in `XREAD` for longer than its own lease; the engine refuses to start rather
than let that happen quietly.

## Naming conventions


> Perpetuals are **not built** — spot is the whole of the working system. What follows is the
> naming reserved for them, kept so that adding them later does not mean renaming what exists.

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

## Known limitations

Named rather than buried, because each is a real thing to fix:

* **The engine lock is a lease, not a hard guarantee.** It reliably stops the accidental second
  start, which is the thing that actually happens. It cannot make double-application impossible: a
  process paused past its lease may not notice until it wakes, and another engine can legitimately
  hold the stream by then. Closing that window completely needs a fencing token checked on every
  write to the command log.
* **Idempotency only reaches back as far as the log.** The engine remembers the last 50,000
  command ids; a retry that arrives after its command has been pushed out of that window is applied
  as a new one. Ample for a client retry, not a substitute for reconciliation after a long outage.
* **Replay republishes events.** Recovery re-applies commands after the snapshot, so downstream
  consumers see duplicates and must deduplicate on `seq`. `persist` does it against a table and
  `ws` against an in-memory high-water mark; anything new must do it too.
* **A batch `persist` cannot write stalls history rather than skipping it.** The entries stay
  unacknowledged and are retried forever. That is the right failure — better a stalled writer that
  pages you than one that quietly drops trades — but it does need someone watching for it.
* **A graceful restart leaves the old engine answering reads.** `main.rs` races `Runner::run`
  against SIGTERM in a `tokio::select!`. When the signal wins, `run` is *cancelled* — so the
  `handle.abort()` at the end of it never executes, and the query task is only stopped by
  `Drop for Runner`, which fires after `snapshot()` and `shutdown()` have already run. The task
  is a competing consumer on the shared queries queue, so once the lock is released a client's
  read can be answered by the outgoing engine from state the incoming one has already moved past,
  and two consecutive reads can show `seq` going backwards. The fix is to pass the stop signal
  into `run` so its own cleanup always executes, rather than adding another call site to forget.
