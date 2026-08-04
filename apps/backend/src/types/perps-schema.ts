import { z } from "zod";

export const perpsDepositSchema = z.object({
  amount: z.number().positive("amount must be positive"),
});

export const perpsOpenSchema = z.object({
  symbol: z.string().trim().min(1, "symbol is required"),
  side: z.enum(["long", "short"]),
  size: z.number().positive("size must be positive"),
  leverage: z.number().positive().max(100, "leverage must be between 1 and 100"),
});

export const perpsMarkPriceSchema = z.object({
  symbol: z.string().trim().min(1, "symbol is required"),
  price: z.number().positive("price must be positive"),
});

export const perpsSymbolParamSchema = z.object({
  symbol: z.string().trim().min(1, "symbol is required"),
});
