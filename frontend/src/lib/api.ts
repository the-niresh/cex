import { parseExact, stringifyExact } from "./json";
import { LatencyWindow, type LatencyStats } from "./latency";
import type {
  Balance,
  Candle,
  DepthSnapshot,
  Interval,
  Market,
  MyFill,
  Order,
  PlacedOrder,
  PublicTrade,
  Session,
  Side,
  TimeInForce,
} from "./types";

// Same-origin by default so it always resolves against wherever the page was
// actually loaded from (Vite's dev proxy forwards these paths to the API on
// 8080) — "localhost:8080" would instead mean the browser's own machine,
// which is wrong the moment the dev server is reached through SSH port
// forwarding. Set VITE_API_URL to override, e.g. when the API is on a
// different origin in production.
export const API_URL = import.meta.env.VITE_API_URL ?? window.location.origin;

/** A response the exchange refused, carrying the status it refused it with. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }

  /**
   * A 504 is genuinely ambiguous: the command is already on the durable log
   * and may still apply. Retrying is only safe with the same idempotency key.
   */
  get isAmbiguous(): boolean {
    return this.status === 504;
  }
}

/**
 * A key for one user *intent*, not one HTTP attempt.
 *
 * Generate it once where the user decides to do something, and reuse it for
 * every retry of that decision. A fresh key per attempt is the same as having
 * no key at all — it is exactly how a retried order becomes two orders.
 */
export function newIdempotencyKey(): string {
  return crypto.randomUUID();
}

interface RequestOptions {
  method?: "GET" | "POST" | "DELETE";
  token?: string | undefined;
  body?: unknown;
  idempotencyKey?: string | undefined;
  signal?: AbortSignal | undefined;
}

const SERVER_US_HEADER = "x-cex-server-us";

// One window for the session. Every call goes through `request`, so this sees
// the whole surface without any caller opting in.
const latency = new LatencyWindow(50);
const listeners = new Set<(stats: LatencyStats) => void>();

export const latencyStats = (): LatencyStats => latency.stats();

export function onLatency(fn: (stats: LatencyStats) => void): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

function recordLatency(totalMs: number, header: string | null): void {
  // An absent header and one that is present but empty are the same failure:
  // no measurement arrived. `Number("")` is 0, which would sail past the
  // window's own guard and render as an impossibly fast response.
  const serverUs = header === null || header.trim() === "" ? null : Number(header);
  latency.add(totalMs, serverUs);

  const stats = latency.stats();
  for (const fn of listeners) fn(stats);
}

async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = "GET", token, body, idempotencyKey, signal } = options;

  const headers: Record<string, string> = {};
  if (token) headers["authorization"] = `Bearer ${token}`;
  if (body !== undefined) headers["content-type"] = "application/json";
  if (idempotencyKey) headers["idempotency-key"] = idempotencyKey;

  const init: RequestInit = { method, headers };
  if (body !== undefined) init.body = stringifyExact(body);
  if (signal) init.signal = signal;

  // Timed to the headers arriving, not to the body being read. Folding the
  // body read in would make a large history request look like a slow exchange.
  const startedAt = performance.now();
  const response = await fetch(`${API_URL}${path}`, init);
  recordLatency(performance.now() - startedAt, response.headers.get(SERVER_US_HEADER));

  const text = await response.text();

  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const parsed = parseExact(text) as { error?: string };
      if (parsed?.error) message = parsed.error;
    } catch {
      // A non-JSON error body is not worth failing over — the status carries
      // the useful part.
    }
    throw new ApiError(response.status, message);
  }

  return (text ? parseExact(text) : null) as T;
}

// ───────────────────────── public ─────────────────────────

export async function markets(): Promise<Market[]> {
  const { markets } = await request<{ markets: Market[] }>("/markets");
  return markets;
}

export function depth(symbol: string, signal?: AbortSignal): Promise<DepthSnapshot> {
  return request<DepthSnapshot>(`/depth/${symbol}`, { signal });
}

export async function trades(symbol: string, limit = 50, signal?: AbortSignal): Promise<PublicTrade[]> {
  const { trades } = await request<{ trades: PublicTrade[] }>(
    `/trades/${symbol}?limit=${limit}`,
    { signal },
  );
  return trades;
}

export async function candles(
  symbol: string,
  interval: Interval = "1m",
  limit = 200,
  signal?: AbortSignal,
): Promise<Candle[]> {
  const { candles } = await request<{ candles: Candle[] }>(
    `/candles/${symbol}?interval=${interval}&limit=${limit}`,
    { signal },
  );
  return candles;
}

// ───────────────────────── auth ─────────────────────────

export function register(username: string, name: string, password: string): Promise<Session> {
  return request<Session>("/register", { method: "POST", body: { username, name, password } });
}

export function login(username: string, password: string): Promise<Session> {
  return request<Session>("/login", { method: "POST", body: { username, password } });
}

// ───────────────────────── authenticated ─────────────────────────

export async function balances(token: string, signal?: AbortSignal): Promise<Balance[]> {
  const { balances } = await request<{ balances: Balance[] }>("/balances", { token, signal });
  return balances;
}

export async function openOrders(token: string, signal?: AbortSignal): Promise<Order[]> {
  const { orders } = await request<{ orders: Order[] }>("/orders/open", { token, signal });
  return orders;
}

export async function fillHistory(token: string, limit = 50, signal?: AbortSignal): Promise<MyFill[]> {
  const { fills } = await request<{ fills: MyFill[] }>(`/orders/history?limit=${limit}`, {
    token,
    signal,
  });
  return fills;
}

export function deposit(
  token: string,
  asset: string,
  amount: bigint,
  idempotencyKey: string,
): Promise<{ status: string }> {
  return request<{ status: string }>("/deposit", {
    method: "POST",
    token,
    idempotencyKey,
    body: { asset, amount },
  });
}

export interface PlaceOrderRequest {
  symbol: string;
  side: Side;
  order_type: "LIMIT" | "MARKET";
  time_in_force: TimeInForce;
  /** Null for a market order. */
  price: bigint | null;
  qty: bigint;
}

export function placeOrder(
  token: string,
  order: PlaceOrderRequest,
  idempotencyKey: string,
): Promise<PlacedOrder> {
  return request<PlacedOrder>("/orders", {
    method: "POST",
    token,
    idempotencyKey,
    body: order,
  });
}

export function cancelOrder(
  token: string,
  orderId: bigint,
  idempotencyKey: string,
): Promise<{ status: string; order_id: bigint }> {
  return request<{ status: string; order_id: bigint }>(`/orders/${orderId}`, {
    method: "DELETE",
    token,
    idempotencyKey,
  });
}
