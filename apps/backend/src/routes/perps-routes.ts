import { Router } from "express";
import {
  perpsClose,
  perpsDeposit,
  perpsGetAccount,
  perpsGetPosition,
  perpsOpen,
  perpsSetMarkPrice,
} from "../controllers/perps-controller.js";
import { requireAuth } from "../utils/auth.js";
import { asyncHandler } from "../utils/async-handler.js";

export const perpsRouter = Router();

perpsRouter.post("/perps/deposit", requireAuth, asyncHandler(perpsDeposit));
perpsRouter.post("/perps/position", requireAuth, asyncHandler(perpsOpen));
perpsRouter.delete("/perps/position/:symbol", requireAuth, asyncHandler(perpsClose));
perpsRouter.get("/perps/position/:symbol", requireAuth, asyncHandler(perpsGetPosition));
perpsRouter.get("/perps/account", requireAuth, asyncHandler(perpsGetAccount));
// Mark-price feed hook (public in v1 for testing; the real feed is Binance WS).
perpsRouter.post("/perps/mark-price", asyncHandler(perpsSetMarkPrice));
