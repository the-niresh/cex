import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as api from "./lib/api";
import { DepthBook, type Level } from "./lib/book";
import { Feed, type FeedStatus } from "./lib/feed";
import { liveFillFrom, mergeFills } from "./lib/fills";
import { nextResyncBackoffMs, resyncWaitMs } from "./lib/resync";
import { clearSession, loadSession, saveSession } from "./lib/session";
import type {
  AuthMode,
  Balance,
  Candle,
  Credentials,
  DayStats,
  Interval,
  Market,
  MyFill,
  Order,
  PublicTrade,
  Session,
  Side,
} from "./lib/types";

/** A print on the tape. Live ones have no seq, so the arrival time orders them. */
export interface TapePrint {
  key: string;
  price: bigint;
  qty: bigint;
  taker_side: Side;
  timestamp_ms: bigint;
  /** Just arrived, so the row can flash once. */
  fresh: boolean;
}

const TAPE_LIMIT = 60;

/**
 * A counter for tape row keys.
 *
 * `seq` alone is not unique: one command can produce several fills — a taker
 * sweeping two makers is one seq with two prints — and `GET /trades/:symbol`
 * does not return the index within the batch. Duplicate keys make React drop
 * or duplicate rows, which on a tape means prints that silently never appear.
 */
let nextPrintKey = 0;
const BOOK_DEPTH = 14;

/**
 * How many times a resync will re-ask for a snapshot that came back older than
 * the stream, and how long it waits between tries.
 *
 * The engine answers reads between blocking stream reads, so its `/depth` view
 * can trail the deltas it has already published. Bounded on purpose: if the
 * read path is still behind after this, leaving the book visibly stale is
 * better than retrying forever against an exchange that is not keeping up.
 */
const SNAPSHOT_RETRIES = 5;
const SNAPSHOT_RETRY_MS = 300;

/**
 * How often the chart asks for candles again.
 *
 * Candles are the one thing on the screen the socket does not carry: the feed
 * sends depth and prints, and nothing aggregates them into bars. So the chart
 * needs a clock of its own.
 *
 * It used to borrow the resync counter for this, which meant every depth gap
 * pulled two candle requests along behind it and made a book problem into three
 * times the database load. A market has to trade for a whole minute to add a
 * bar, so asking this often is already generous.
 */
const CHART_REFRESH_MS = 15_000;

/** The ladder as the screen renders it — plain data, not the live book. */
interface BookView {
  bids: Level[];
  asks: Level[];
  spread: bigint | null;
  depthSeq: bigint | null;
  stale: boolean;
}

const EMPTY_BOOK: BookView = { bids: [], asks: [], spread: null, depthSeq: null, stale: false };

export interface Exchange {
  session: Session | null;
  markets: Market[];
  market: Market | null;
  symbol: string;
  selectMarket(symbol: string): void;

  bids: Level[];
  asks: Level[];
  spread: bigint | null;
  depthSeq: bigint | null;
  bookStale: boolean;
  /** Your own resting quantity, by price, for the ladder's MINE column. */
  mine: Map<bigint, bigint>;

  tape: TapePrint[];
  candles: Candle[];
  interval: Interval;
  setInterval(interval: Interval): void;
  /** The last 24 hours, for the header strip. `null` until the first load. */
  day: DayStats | null;

  balances: Balance[];
  openOrders: Order[];
  fills: MyFill[];

  status: FeedStatus;
  resyncs: number;
  lastUpdateMs: number | null;
  error: string | null;
  clearError(): void;

  signIn(mode: AuthMode, credentials: Credentials): Promise<void>;
  signOut(): void;
  submitOrder(request: api.PlaceOrderRequest): Promise<void>;
  cancel(orderId: bigint): Promise<void>;
  credit(asset: string, amount: bigint): Promise<void>;
}

export function useExchange(): Exchange {
  const [session, setSession] = useState<Session | null>(() => loadSession());
  const [markets, setMarkets] = useState<Market[]>([]);
  const [symbol, setSymbol] = useState("BTC_USDT");
  const [interval, setIntervalState] = useState<Interval>("1m");

  const bookRef = useRef(new DepthBook());
  const [book, setBook] = useState<BookView>(EMPTY_BOOK);
  const [tape, setTape] = useState<TapePrint[]>([]);
  const [candles, setCandles] = useState<Candle[]>([]);
  // Kept apart from `candles`: those follow whichever interval the chart is
  // showing, and the header strip always means the last 24 hours.
  const [dayCandles, setDayCandles] = useState<Candle[]>([]);
  const [balances, setBalances] = useState<Balance[]>([]);
  const [openOrders, setOpenOrders] = useState<Order[]>([]);
  // What `/orders/history` last returned, and the fills the feed has reported
  // that it does not contain yet. They are kept apart because the history is
  // the authority: a live one is only shown while the row is still catching up.
  const [history, setHistory] = useState<MyFill[]>([]);
  const [liveFills, setLiveFills] = useState<MyFill[]>([]);

  const [status, setStatus] = useState<FeedStatus>("connecting");
  const [resyncs, setResyncs] = useState(0);
  // Bumped only when the socket comes back, never on a depth gap. The chart is
  // worth refetching after a real disconnection, because the feed carries no
  // candles and we may have missed minutes of them; a gap in the depth stream
  // says nothing about the chart at all.
  const [reconnects, setReconnects] = useState(0);
  const [lastUpdateMs, setLastUpdateMs] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  // The feed's callbacks live for the life of the socket, so anything they
  // read has to come from a ref or they close over the first render forever.
  // Written in an effect, never during render: a ref that changes mid-render
  // is exactly what tears under concurrent rendering.
  const symbolRef = useRef(symbol);
  const tokenRef = useRef<string | null>(session?.token ?? null);
  const marketsRef = useRef<Market[]>(markets);

  useEffect(() => {
    symbolRef.current = symbol;
  }, [symbol]);

  useEffect(() => {
    marketsRef.current = markets;
  }, [markets]);

  useEffect(() => {
    tokenRef.current = session?.token ?? null;
  }, [session]);

  /** Snapshot the mutable book into the state the screen renders from. */
  const publishBook = useCallback(() => {
    const b = bookRef.current;
    setBook({
      bids: b.bids(BOOK_DEPTH),
      asks: b.asks(BOOK_DEPTH),
      spread: b.spread(),
      depthSeq: b.depthSeq,
      stale: b.stale,
    });
  }, []);

  const failed = useCallback((e: unknown) => {
    if (e instanceof api.ApiError) setError(e.message);
    else if (e instanceof Error && e.name !== "AbortError") setError(e.message);
  }, []);

  // ── refetch everything ────────────────────────────────────────────────
  //
  // The one path back to a trustworthy screen. Called on connect, on a depth
  // gap, and whenever the market changes.
  //
  // Only ever one at a time. Gaps arrive faster than a round trip can answer
  // them, and firing a fresh refetch at each one produced far more requests
  // than the API could serve — which made the refetches slower, which caused
  // more gaps. A second request while one is already running is remembered
  // rather than sent, and runs once when the first finishes; that keeps the
  // "something changed, look again" guarantee without the pile-up.
  const resyncRunning = useRef(false);
  const resyncWanted = useRef(false);
  const resyncBackoffMs = useRef(0);

  const resyncOnce = useCallback(async () => {
    const sym = symbolRef.current;
    const token = tokenRef.current;
    try {
      // Snapshot first, then the stream applies on top of it. The other way
      // round leaves a window whose updates are silently lost.
      const [snapshot, recent] = await Promise.all([api.depth(sym), api.trades(sym, TAPE_LIMIT)]);

      // `reset` refuses a snapshot older than what the stream has already
      // applied — see DepthBook.reset for why that rollback is silent and
      // permanent. A refusal is fine for a healthy book: our own state is
      // simply newer. A *stale* book is different, because only a snapshot can
      // clear its gap, so give the read path a moment to catch up and ask
      // again rather than leaving the ladder frozen.
      let took = bookRef.current.reset(snapshot);
      for (let attempt = 0; !took && bookRef.current.stale && attempt < SNAPSHOT_RETRIES; attempt++) {
        await new Promise((resolve) => setTimeout(resolve, SNAPSHOT_RETRY_MS));
        took = bookRef.current.reset(await api.depth(sym));
      }
      publishBook();
      setTape(
        recent.map((t: PublicTrade) => ({
          key: `s${t.seq}-${nextPrintKey++}`,
          price: t.price,
          qty: t.qty,
          taker_side: t.taker_side,
          timestamp_ms: t.timestamp_ms,
          fresh: false,
        })),
      );
      setLastUpdateMs(Date.now());

      if (token) {
        const [b, o, f] = await Promise.all([
          api.balances(token),
          api.openOrders(token),
          api.fillHistory(token, TAPE_LIMIT),
        ]);
        setBalances(b);
        setOpenOrders(o);
        setHistory(f);
      }
    } catch (e) {
      failed(e);
    }
  }, [failed, publishBook]);

  /**
   * Ask for a resync. Runs one now, or notes that another is wanted.
   *
   * The wait between rounds only grows when the previous round left the book
   * still stale — see `RESYNC_BACKOFF_MIN_MS`. A round that fixed the book
   * clears it, so the next gap is answered as quickly as the first.
   */
  const resync = useCallback(async () => {
    if (resyncRunning.current) {
      resyncWanted.current = true;
      return;
    }

    resyncRunning.current = true;
    try {
      do {
        resyncWanted.current = false;

        const wait = resyncWaitMs(resyncBackoffMs.current, Math.random());
        if (wait > 0) {
          await new Promise((resolve) => setTimeout(resolve, wait));
        }

        await resyncOnce();

        resyncBackoffMs.current = nextResyncBackoffMs(
          resyncBackoffMs.current,
          bookRef.current.stale,
        );
      } while (resyncWanted.current);
    } finally {
      resyncRunning.current = false;
    }
  }, [resyncOnce]);

  const refreshAccount = useCallback(async () => {
    const token = tokenRef.current;
    if (!token) return;
    try {
      const [b, o, f] = await Promise.all([
        api.balances(token),
        api.openOrders(token),
        api.fillHistory(token, TAPE_LIMIT),
      ]);
      setBalances(b);
      setOpenOrders(o);
      setHistory(f);
    } catch (e) {
      failed(e);
    }
  }, [failed]);

  // ── the socket ────────────────────────────────────────────────────────

  const feedRef = useRef<Feed | null>(null);

  useEffect(() => {
    const feed = new Feed({
      onDepth(update) {
        if (update.symbol !== symbolRef.current) return;
        const result = bookRef.current.apply(update);
        if (result === "gap") {
          // The server never announces this. Refetch rather than apply a
          // delta onto a book that is already wrong.
          publishBook();
          setResyncs((n) => n + 1);
          void resync();
          return;
        }
        if (result === "applied") {
          publishBook();
          setLastUpdateMs(Date.now());
        }
      },
      onTrade(update) {
        if (update.symbol !== symbolRef.current) return;
        setTape((current) => {
          const print: TapePrint = {
            key: `live-${nextPrintKey++}`,
            price: update.price,
            qty: update.qty,
            taker_side: update.taker_side,
            timestamp_ms: BigInt(Date.now()),
            fresh: true,
          };
          return [print, ...current].slice(0, TAPE_LIMIT);
        });
        setLastUpdateMs(Date.now());
      },
      onOrder(update, seq) {
        // The private feed says *that* something changed; the REST views are
        // the authority on what it changed to. Refetching is a round trip the
        // screen can afford and keeps one source of truth.
        //
        // Fills are the exception, and only because of *when*. Balances and
        // open orders come back from the engine, so a refetch here already
        // reflects this event. `/orders/history` reads a table `persist`
        // writes asynchronously, so it truthfully does not have this fill yet
        // — and nothing would refetch again once it did. Hold the live one
        // until the row lands, or your own trade is invisible until you
        // reload. `mergeFills` drops it again on `(seq, idx)`.
        const live = liveFillFrom(update, seq, marketsRef.current);
        if (live) setLiveFills((current) => [live, ...current].slice(0, TAPE_LIMIT));
        void refreshAccount();
      },
      onStatus: setStatus,
      onResync() {
        // A fresh socket. Unlike a depth gap this really can mean the chart
        // missed something, so it is the one place that asks for candles again.
        setResyncs((n) => n + 1);
        setReconnects((n) => n + 1);
        void resync();
      },
    });

    feedRef.current = feed;
    feed.connect();
    return () => {
      feed.close();
      feedRef.current = null;
    };
  }, [resync, refreshAccount, publishBook]);

  // Channels follow the selected market and whether anyone is signed in.
  useEffect(() => {
    feedRef.current?.setSubscriptions(
      [`depth@${symbol}`, `trades@${symbol}`, "orders"],
      session?.token ?? null,
    );
  }, [symbol, session]);

  // ── one-off and per-market loads ──────────────────────────────────────

  useEffect(() => {
    api.markets().then(setMarkets).catch(failed);
  }, [failed]);

  useEffect(() => {
    void resync();
  }, [symbol, session, resync]);

  // The chart's own clock. Nothing on the feed carries candles, so without this
  // the bars would sit at whatever they were when the page loaded.
  useEffect(() => {
    const controller = new AbortController();
    const load = () =>
      api
        .candles(symbol, interval, 200, controller.signal)
        .then(setCandles)
        .catch((e: unknown) => {
          if (!controller.signal.aborted) failed(e);
        });

    void load();
    const timer = setInterval(() => void load(), CHART_REFRESH_MS);
    return () => {
      clearInterval(timer);
      controller.abort();
    };
  }, [symbol, interval, failed, reconnects]);

  // 24 hourly buckets, whatever the chart happens to be showing.
  useEffect(() => {
    const controller = new AbortController();
    const load = () =>
      api
        .candles(symbol, "1h", 24, controller.signal)
        .then(setDayCandles)
        .catch((e: unknown) => {
          if (!controller.signal.aborted) failed(e);
        });

    void load();
    const timer = setInterval(() => void load(), CHART_REFRESH_MS);
    return () => {
      clearInterval(timer);
      controller.abort();
    };
  }, [symbol, failed, reconnects]);

  // ── derived ───────────────────────────────────────────────────────────

  /** Fold the day's buckets into the one line the header reads from. */
  const day = useMemo<DayStats | null>(() => {
    const first = dayCandles[0];
    const last = dayCandles[dayCandles.length - 1];
    if (!first || !last) return null;

    let high = first.high;
    let low = first.low;
    let volume = 0n;
    let trades = 0n;
    for (const candle of dayCandles) {
      if (candle.high > high) high = candle.high;
      if (candle.low < low) low = candle.low;
      volume += candle.volume;
      trades += candle.trades;
    }

    const change = last.close - first.open;
    return {
      open: first.open,
      close: last.close,
      high,
      low,
      volume,
      trades,
      change,
      // Basis points first, so the division stays in integers until the very
      // last step — the same reason nothing else here touches a float.
      changePct: first.open > 0n ? Number((change * 10_000n) / first.open) / 100 : null,
    };
  }, [dayCandles]);

  /** History, plus any fill it has not caught up to yet. */
  const fills = useMemo(
    () => mergeFills(history, liveFills, TAPE_LIMIT),
    [history, liveFills],
  );

  const market = useMemo(
    () => markets.find((m) => m.symbol === symbol) ?? null,
    [markets, symbol],
  );

  /** Resting quantity of your own orders, per price, for this market. */
  const mine = useMemo(() => {
    const byPrice = new Map<bigint, bigint>();
    for (const order of openOrders) {
      if (order.symbol !== symbol || order.price === null) continue;
      const remaining = order.qty - order.filled_qty;
      if (remaining <= 0n) continue;
      byPrice.set(order.price, (byPrice.get(order.price) ?? 0n) + remaining);
    }
    return byPrice;
  }, [openOrders, symbol]);

  // ── actions ───────────────────────────────────────────────────────────

  const signIn = useCallback(async (mode: AuthMode, credentials: Credentials) => {
    const { username, name, password } = credentials;
    const next =
      mode === "register"
        ? await api.register(username, name, password)
        : await api.login(username, password);
    saveSession(next);
    setSession(next);
  }, []);

  const signOut = useCallback(() => {
    clearSession();
    setSession(null);
    setBalances([]);
    setOpenOrders([]);
    setHistory([]);
    setLiveFills([]);
  }, []);

  const submitOrder = useCallback(
    async (request: api.PlaceOrderRequest) => {
      const token = tokenRef.current;
      if (!token) return;
      // One key for this intent. A retry of *this* order reuses it and cannot
      // become a second order.
      const key = api.newIdempotencyKey();
      try {
        await api.placeOrder(token, request, key);
        await refreshAccount();
      } catch (e) {
        failed(e);
        throw e;
      }
    },
    [failed, refreshAccount],
  );

  const cancel = useCallback(
    async (orderId: bigint) => {
      const token = tokenRef.current;
      if (!token) return;
      try {
        await api.cancelOrder(token, orderId, api.newIdempotencyKey());
        await refreshAccount();
      } catch (e) {
        failed(e);
      }
    },
    [failed, refreshAccount],
  );

  const credit = useCallback(
    async (asset: string, amount: bigint) => {
      const token = tokenRef.current;
      if (!token) return;
      try {
        await api.deposit(token, asset, amount, api.newIdempotencyKey());
        await refreshAccount();
      } catch (e) {
        failed(e);
      }
    },
    [failed, refreshAccount],
  );

  const selectMarket = useCallback((next: string) => {
    setSymbol(next);
    // A fresh book, not a reset one: the old market's depth_seq has nothing to
    // do with the new market's, and carrying it would fake continuity.
    bookRef.current = new DepthBook();
    setBook(EMPTY_BOOK);
    setTape([]);
  }, []);

  return {
    session,
    markets,
    market,
    symbol,
    selectMarket,
    bids: book.bids,
    asks: book.asks,
    spread: book.spread,
    depthSeq: book.depthSeq,
    bookStale: book.stale,
    mine,
    tape,
    candles,
    day,
    interval,
    setInterval: setIntervalState,
    balances,
    openOrders,
    fills,
    status,
    resyncs,
    lastUpdateMs,
    error,
    clearError: useCallback(() => setError(null), []),
    signIn,
    signOut,
    submitOrder,
    cancel,
    credit,
  };
}
