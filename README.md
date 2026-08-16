# cex

[![ci](https://github.com/the-niresh/cex/actions/workflows/ci.yml/badge.svg)](https://github.com/the-niresh/cex/actions/workflows/ci.yml)

A centralised exchange in Rust — order book, matching, settlement, live market data and a
trading screen. Complete and tested end to end.

The exchange is a **deterministic state machine**: one function, `apply(state, command) -> events`,
running single-threaded over a durable command log. Everything else — HTTP, WebSocket, Postgres,
snapshots — is plumbing around that.

![The spot trading screen](docs/image.png)

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
| `ws` — market data fan-out | Built · 53 tests |

Two users can register, deposit, place orders, match and settle over HTTP; every order, fill and
balance change lands in Postgres behind the engine; and the book, the trades and each user's own
orders stream live over WebSocket.

## Running it

The whole exchange is in `docker-compose.yml`, so it comes up with one command and comes back
on its own after a reboot:

```bash
cp .env.example .env                     # CEX_JWT_SECRET, CEX_DATABASE_URL, CEX_CORS_ORIGINS
docker compose up -d
```

`Dockerfile` builds all four binaries into **one** image. They share a workspace and nearly all
of their dependencies, so building an image each would compile the same crates four times over
to produce four images differing only in an argv; every service is that image with a different
`command`.

Postgres is behind a `local-db` profile rather than started by default, because a deployment
points `CEX_DATABASE_URL` at managed Postgres. Either way it is off the hot path — `persist` is
asynchronous and `api` touches it only for auth — so the extra hop costs nothing that matters.
Redis is not optional and stays next to the engine: a command and its reply are two round trips
on the matching path.

```bash
docker compose --profile local-db up -d  # redis on 6390, postgres on 5442
```

To run the binaries directly instead — the usual loop while working on them:

```bash
docker compose --profile local-db up -d redis postgres
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

`npm run e2e` needs `api` on `localhost:8080` and `ws` on `localhost:8081` — the dev server proxies
to both, and without them every spec fails on `ECONNREFUSED` rather than on anything it tested. A
deployed stack behind a reverse proxy does not count: those ports have to be reachable on this
box. Start `persist` before `api` on a fresh database, for the reason in the known limitations.

Nothing asks for an account until something moves money — the book, tape, chart and ticket are all
usable signed out, and the sign-in panel opens on BUY, SELL or CREDIT. The layout holds from 320px
to 1920px, and every one of its 44 text styles is checked against WCAG AA by a script that walks
the rendered page.

## More

[docs/internals.md](docs/internals.md) — the REST and WebSocket contract, integer scaling, the
design rules the engine is held to, idempotency, how exactly one engine is guaranteed, the history
read path and the outage that taught us how it fails, and the known limitations.
