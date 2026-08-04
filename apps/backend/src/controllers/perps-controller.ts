import type { Request, Response } from "express";
import {
  perpsDepositSchema,
  perpsMarkPriceSchema,
  perpsOpenSchema,
  perpsSymbolParamSchema,
} from "../types/perps-schema.js";
import { sendToEngine } from "../utils/engine-client.js";
import { sendValidationError } from "../utils/validation.js";

function getUserId(req: Request): string {
  if (!req.userId) throw new Error("Missing authenticated user");
  return req.userId;
}

export async function perpsDeposit(req: Request, res: Response): Promise<void> {
  const parsed = perpsDepositSchema.safeParse(req.body);
  if (!parsed.success) return sendValidationError(res, parsed.error);

  const engineResponse = await sendToEngine("perps_deposit", {
    userId: getUserId(req),
    amount: parsed.data.amount,
  });
  res
    .status(engineResponse.ok ? 200 : 400)
    .json(engineResponse.ok ? engineResponse.data : { error: engineResponse.error });
}

export async function perpsOpen(req: Request, res: Response): Promise<void> {
  const parsed = perpsOpenSchema.safeParse(req.body);
  if (!parsed.success) return sendValidationError(res, parsed.error);

  const { symbol, side, size, leverage } = parsed.data;
  const engineResponse = await sendToEngine("perps_open", {
    userId: getUserId(req),
    symbol,
    side,
    size,
    leverage,
  });
  res
    .status(engineResponse.ok ? 200 : 400)
    .json(engineResponse.ok ? engineResponse.data : { error: engineResponse.error });
}

export async function perpsClose(req: Request, res: Response): Promise<void> {
  const parsed = perpsSymbolParamSchema.safeParse(req.params);
  if (!parsed.success) return sendValidationError(res, parsed.error);

  const engineResponse = await sendToEngine("perps_close", {
    userId: getUserId(req),
    symbol: parsed.data.symbol,
  });
  res
    .status(engineResponse.ok ? 200 : 400)
    .json(engineResponse.ok ? engineResponse.data : { error: engineResponse.error });
}

// In production the mark price is pushed by the price feed (Binance WS), not a
// user. Exposed here so the flow is testable end-to-end without the feed wired up.
export async function perpsSetMarkPrice(req: Request, res: Response): Promise<void> {
  const parsed = perpsMarkPriceSchema.safeParse(req.body);
  if (!parsed.success) return sendValidationError(res, parsed.error);

  const engineResponse = await sendToEngine("perps_mark_price", {
    symbol: parsed.data.symbol,
    price: parsed.data.price,
  });
  res
    .status(engineResponse.ok ? 200 : 400)
    .json(engineResponse.ok ? engineResponse.data : { error: engineResponse.error });
}

export async function perpsGetPosition(req: Request, res: Response): Promise<void> {
  const parsed = perpsSymbolParamSchema.safeParse(req.params);
  if (!parsed.success) return sendValidationError(res, parsed.error);

  const engineResponse = await sendToEngine("perps_position", {
    userId: getUserId(req),
    symbol: parsed.data.symbol,
  });
  res
    .status(engineResponse.ok ? 200 : 404)
    .json(engineResponse.ok ? engineResponse.data : { error: engineResponse.error });
}

export async function perpsGetAccount(req: Request, res: Response): Promise<void> {
  const engineResponse = await sendToEngine("perps_account", {
    userId: getUserId(req),
  });
  res
    .status(engineResponse.ok ? 200 : 400)
    .json(engineResponse.ok ? engineResponse.data : { error: engineResponse.error });
}
