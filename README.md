# cex

[![ci](https://github.com/the-niresh/cex/actions/workflows/ci.yml/badge.svg)](https://github.com/the-niresh/cex/actions/workflows/ci.yml)

A centralised exchange in Rust — order book, matching, settlement, live market data and a
trading screen. Spot is complete and tested end to end; perpetuals reuse the same engine and
are not started.

The exchange is a **deterministic state machine**: one function, `apply(state, command) -> events`,
running single-threaded over a durable command log. Everything else — HTTP, WebSocket, Postgres,
snapshots — is plumbing around that.

![The spot trading screen](docs/screen.png)

## Four things that are easy to get wrong

**Money never becomes a float.** Prices and quantities are integers end to end. `core` has no async
dependencies *by manifest*, so it cannot perform I/O and therefore cannot become non-deterministic.
Even the browser keeps it exact: `JSON.parse` rounds a 64-bit integer to a double before any reviver
can see it, so the frontend parses the source text and hands back `bigint`.

**The command log is the source of truth.** Everything that mutates state goes through one Redis
stream, so that stream is a complete, replayable record. Recovery loads the newest snapshot and
replays forward. Downstream consumers deduplicate on `seq`, because replay republishes.

**The engine never waits on a database.** `persist` is a separate process reading the event stream;
the engine publishes a batch and moves on in microseconds. Stopping `persist` costs history
freshness and nothing else.

**A gap in the feed is assumed, not hoped against.** `depth_seq` is monotonic per symbol; anything
other than exactly one past the last is refused, the book is marked stale, and nothing is applied
until a fresh snapshot arrives. Applying the next delta onto a book that is already wrong gives a
book that stays wrong and never looks wrong.

## Architecture

![Spot architecture](docs/architecture.svg)

Every arrow is a real network hop between separate processes: `engine` owns state, `api` speaks
HTTP, `ws` fans out market data, `persist` writes history.

## Status

| Component | State |
|---|---|
| `cex-core` — matching, ledger, settlement, snapshots, idempotency | Built · 132 tests |
| `cex-proto` — wire types | Built · 18 tests |
| `engine` — stream consumer, snapshots, crash recovery, boot lock, concurrent query serving | Built · 56 tests |
| `api` — loopback, auth, REST routes | Built · 110 tests |
| `persist` — Postgres history writer | Built · 43 tests |
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

Amounts are integers in atomic units — see [Scaling](docs/internals.md#scaling).

## The screen

One trading screen in `frontend/` — TypeScript, React and Vite, against the same four processes.
No mock backend anywhere, including in its tests: the Playwright suite registers, deposits and
trades through the real exchange.

```bash
cd frontend
npm install
npm run dev          # http://localhost:5173

npm run typecheck && npm run lint && npm test
npm run e2e          # playwright, against the running stack
```

Nothing asks for an account until something moves money — the book, tape, chart and ticket are all
usable signed out, and the sign-in panel opens on BUY, SELL or CREDIT. The layout holds from 320px
to 1920px, and every one of its 44 text styles is checked against WCAG AA by a script that walks
the rendered page.

## Known gaps

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

## More

[docs/internals.md](docs/internals.md) — the REST and WebSocket contract, integer scaling, the
design rules the engine is held to, idempotency, and how exactly one engine is guaranteed.
