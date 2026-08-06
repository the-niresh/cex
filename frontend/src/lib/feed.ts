import { parseExact } from "./json";
import type { DepthUpdate, OrderUpdate, TradeUpdate } from "./types";

// Same-origin by default, same reasoning as API_URL in ./api — Vite's dev
// proxy forwards /ws to the ws crate on 8081. Set VITE_WS_URL to override.
const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
export const WS_URL = import.meta.env.VITE_WS_URL ?? `${wsProtocol}//${window.location.host}/ws`;

export type FeedStatus = "connecting" | "live" | "reconnecting" | "closed";

export interface FeedHandlers {
  onDepth(update: DepthUpdate): void;
  onTrade(update: TradeUpdate): void;
  onOrder(update: OrderUpdate): void;
  onStatus(status: FeedStatus): void;
  /**
   * Everything in memory must be thrown away and refetched.
   *
   * Fires on every (re)connection, because updates published while
   * disconnected are never replayed — so the first delta after a reconnect
   * lands on a book that is already behind.
   */
  onResync(): void;
}

interface Envelope {
  channel?: string;
  seq?: bigint;
  data?: { type?: string; body?: unknown };
  op?: string;
  error?: string;
  channels?: string[];
}

const RECONNECT_MIN_MS = 500;
const RECONNECT_MAX_MS = 10_000;

/**
 * The market-data socket.
 *
 * Two rules from the server shape everything here:
 *
 * 1. **Nothing is replayed.** Updates published before you connected are gone.
 *    So a connection is only usable after a REST snapshot taken *after* the
 *    socket opened — which is why {@link FeedHandlers.onResync} fires on every
 *    connect, not only on an error.
 * 2. **A slow subscriber is dropped**, with `fell behind by N updates`. The
 *    answer is to reconnect and refetch, never to retry the socket blindly and
 *    carry on with a book that has a hole in it.
 */
export class Feed {
  private socket: WebSocket | null = null;
  private reconnectDelay = RECONNECT_MIN_MS;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private closedByUs = false;

  private token: string | null = null;
  private channels = new Set<string>();

  constructor(private readonly handlers: FeedHandlers) {}

  /** Channels to hold across reconnects. `orders` requires a token. */
  setSubscriptions(channels: string[], token: string | null): void {
    this.channels = new Set(channels);
    this.token = token;
    if (this.socket?.readyState === WebSocket.OPEN) this.handshake();
  }

  connect(): void {
    this.closedByUs = false;
    this.open();
  }

  close(): void {
    this.closedByUs = true;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    this.socket?.close();
    this.socket = null;
    this.handlers.onStatus("closed");
  }

  private open(): void {
    this.handlers.onStatus(this.reconnectDelay === RECONNECT_MIN_MS ? "connecting" : "reconnecting");

    const socket = new WebSocket(WS_URL);
    this.socket = socket;

    socket.onopen = () => {
      this.reconnectDelay = RECONNECT_MIN_MS;
      this.handshake();
      this.handlers.onStatus("live");
      // Anything published while we were away is gone for good. Whatever is
      // in memory is stale by definition, so refetch before applying a delta.
      this.handlers.onResync();
    };

    socket.onmessage = (event: MessageEvent<string>) => this.receive(event.data);

    socket.onclose = () => {
      if (this.socket === socket) this.socket = null;
      if (!this.closedByUs) this.scheduleReconnect();
    };

    // An error is always followed by a close, which is where reconnecting is
    // handled. Nothing to do here but keep it off the console.
    socket.onerror = () => {};
  }

  private handshake(): void {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) return;

    // `orders` is refused unless auth came first, so this order is load-bearing.
    if (this.token) this.send({ op: "auth", token: this.token });

    const channels = [...this.channels].filter((c) => c !== "orders" || this.token);
    if (channels.length > 0) this.send({ op: "subscribe", channels });
  }

  private send(message: unknown): void {
    this.socket?.send(JSON.stringify(message));
  }

  private receive(raw: string): void {
    let message: Envelope;
    try {
      message = parseExact(raw) as Envelope;
    } catch {
      console.error("feed: unparseable frame");
      return;
    }

    if (message.op === "error") {
      // "fell behind by N updates; reconnect and resync" is the server closing
      // us deliberately. Reconnecting is the fix; retrying the socket without
      // refetching would rebuild a book around the hole.
      console.warn(`feed: ${message.error ?? "unknown error"}`);
      return;
    }
    if (message.op) return; // subscribed / authenticated — nothing to route.

    const body = message.data?.body;
    if (!body) return;

    switch (message.data?.type) {
      case "depth":
        this.handlers.onDepth(body as DepthUpdate);
        break;
      case "trade":
        this.handlers.onTrade(body as TradeUpdate);
        break;
      case "order":
        this.handlers.onOrder(body as OrderUpdate);
        break;
    }
  }

  private scheduleReconnect(): void {
    this.handlers.onStatus("reconnecting");
    const delay = this.reconnectDelay;
    // Backing off keeps a restarting server from being hammered by every open
    // tab the moment it starts listening again.
    this.reconnectDelay = Math.min(this.reconnectDelay * 2, RECONNECT_MAX_MS);
    this.reconnectTimer = setTimeout(() => this.open(), delay);
  }
}
