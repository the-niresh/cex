import "dotenv/config";
import { createClient } from "redis";
import { env } from "./utils/env.js";
import type { MARKET_ASSETS } from "./utils/types.ts";
import type { PositionSide } from "./utils/perps-types.ts";
import  MatchingEngine  from "./modules/mathchingEngine.ts";
import PerpsEngine from "./modules/perpsEngine.ts";

const matchingEngine = new MatchingEngine();
const perpsEngine = new PerpsEngine();

export type EngineCommandType =
  | "create_order"
  | "get_depth"
  | "get_user_balance"
  | "get_order"
  | "cancel_order"
  | "perps_deposit"
  | "perps_open"
  | "perps_close"
  | "perps_mark_price"
  | "perps_position"
  | "perps_account";

export interface EngineRequest {
  correlationId: string;
  responseQueue: string;
  type: EngineCommandType;
  payload: Record<string, unknown>;
}

export interface EngineResponse {
  correlationId: string;
  ok: boolean;
  data?: unknown;
  error?: string;
}

const brokerClient = createClient({ url: env.redisUrl }).on("error", (error) => {
  console.error("Redis broker client error", error);
});

const responseClient = createClient({ url: env.redisUrl }).on("error", (error) => {
  console.error("Redis response client error", error);
});

await Promise.all([brokerClient.connect(), responseClient.connect()]);



async function sendResponse(responseQueue: string, response: EngineResponse): Promise<void> {
  await responseClient.lPush(responseQueue, JSON.stringify(response));
}

// DONE: get_user_balance, create_order
function handleEngineRequest(message: EngineRequest): unknown {

  switch (message.type) {

    case "get_user_balance": {
      const userId = message.payload.userId as string;
      if (!userId) throw new Error("Missing userId in payload");
      const result = matchingEngine.getUserBalance(message.correlationId, userId);
      if (!result.ok) throw new Error(result.error);
      return result.data;
    }

    case "create_order": {
      const p = message.payload;
      const result = matchingEngine.createOrder(
        message.correlationId,
        p.userId as string,
        p.symbol as MARKET_ASSETS,
        p.quantity as number,
        p.kind as "limit" | "market",
        p.side as "buy" | "sell",
        p.price as number | undefined,
      );
      if (!result.ok) throw new Error(result.error);
      return result.data;
    }

    case "get_depth": {
      const result = matchingEngine.getOrderBookDepth(
        message.correlationId,
        message.payload.symbol as MARKET_ASSETS,
      );
      if (!result.ok) throw new Error(result.error);
      return result.data;
    }

    case "get_order": {
      const result = matchingEngine.getOrderOfUser(
        message.correlationId,
        message.payload.userId as string,
        message.payload.orderId as string,
      );
      if (!result.ok) throw new Error(result.error);
      return result.data;
    }

    case "cancel_order": {
      const result = matchingEngine.cancelOrder(
        message.correlationId,
        message.payload.userId as string,
        message.payload.orderId as string,
      );
      if (!result.ok) throw new Error(result.error);
      return result.data;
    }

    case "perps_deposit": {
      const p = message.payload;
      const result = perpsEngine.deposit(p.userId as string, p.amount as number);
      if (!result.ok) throw new Error(result.error);
      return result.data;
    }

    case "perps_open": {
      const p = message.payload;
      const result = perpsEngine.openPosition(
        p.userId as string,
        p.symbol as MARKET_ASSETS,
        p.side as PositionSide,
        p.size as number,
        p.leverage as number,
      );
      if (!result.ok) throw new Error(result.error);
      return result.data;
    }

    case "perps_close": {
      const p = message.payload;
      const result = perpsEngine.closePosition(
        p.userId as string,
        p.symbol as MARKET_ASSETS,
      );
      if (!result.ok) throw new Error(result.error);
      return result.data;
    }

    case "perps_mark_price": {
      const p = message.payload;
      const result = perpsEngine.setMarkPrice(
        p.symbol as MARKET_ASSETS,
        p.price as number,
      );
      if (!result.ok) throw new Error(result.error);
      return result.data;
    }

    case "perps_position": {
      const p = message.payload;
      const result = perpsEngine.getPosition(
        p.userId as string,
        p.symbol as MARKET_ASSETS,
      );
      if (!result.ok) throw new Error(result.error);
      return result.data;
    }

    case "perps_account": {
      const result = perpsEngine.getAccount(message.payload.userId as string);
      if (!result.ok) throw new Error(result.error);
      return result.data;
    }

    default:
      throw new Error(`Unknown command: ${message.type}`);
  }
}

console.log(`Engine listening on Redis queue: ${env.incomingQueue}`);

for (;;) {
  const item = await brokerClient.brPop(env.incomingQueue, 0);
  if (!item) continue;

  let message: EngineRequest;

  try {
    message = JSON.parse(item.element) as EngineRequest;
  } catch {
    console.error("Skipping invalid broker message");
    continue;
  }

  try {
    const data = handleEngineRequest(message);
    await sendResponse(message.responseQueue, {
      correlationId: message.correlationId,
      ok: true,
      data,
    });
  } catch (error) {
    await sendResponse(message.responseQueue, {
      correlationId: message.correlationId,
      ok: false,
      error: error instanceof Error ? error.message : "engine_error",
    });
  }
}