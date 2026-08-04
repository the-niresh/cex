/**
 * Runnable, Redis-free walkthrough of the perps v1 engine.
 *
 *   bun run apps/engine/src/perps-demo.ts
 *
 * It plays out the exact story you should be able to narrate in the meeting:
 *   deposit -> open a 10x long -> price rises (profit) -> price crashes ->
 *   liquidation fires automatically on the mark-price tick.
 */
import PerpsEngine from "./modules/perpsEngine.ts";

function line() {
  console.log("─".repeat(64));
}
function show(label: string, result: { ok: boolean; data?: unknown; error?: string }) {
  console.log(`\n▶ ${label}`);
  if (result.ok) console.log(JSON.stringify(result.data, null, 2));
  else console.log("  ✗ error:", result.error);
}

const engine = new PerpsEngine();
const USER = "alice";

line();
console.log("PERPS v1 DEMO — leverage, isolated margin, liquidation");
line();

// 1. Seed a mark price (in prod this comes from the Binance WS feed).
engine.setMarkPrice("BTC", 100);
console.log("\nMark price BTC = 100");

// 2. Alice deposits 1,000 USD of collateral.
show("deposit 1000 USD", engine.deposit(USER, 1000));

// 3. Open a 10x LONG on 10 BTC.
//    notional = 10 * 100 = 1000 ; required margin = 1000 / 10 = 100.
//    Expected liquidation price (long) = entry*(1 - 1/lev)/(1 - mmr)
//      = 100*0.9/0.995 ≈ 90.45
show("open 10x LONG, size 10 @ mark 100", engine.openPosition(USER, "BTC", "long", 10, 10));

// 4. Price rises to 110 -> unrealized PnL = (110-100)*10 = +100.
engine.setMarkPrice("BTC", 110);
show("mark -> 110, position now", engine.getPosition(USER, "BTC"));

// 5. Price falls to 95 -> still alive (above ~90.45 liq price).
engine.setMarkPrice("BTC", 95);
show("mark -> 95, position now", engine.getPosition(USER, "BTC"));

// 6. Price crashes to 90 -> below liq price -> auto-liquidated on this very tick.
const crash = engine.setMarkPrice("BTC", 90);
show("mark -> 90 (crash), liquidation sweep result", crash);

// 7. Position is gone; only the position's isolated margin (100) was lost.
//    Free collateral (900) is untouched — that is isolated margin working.
show("account after liquidation", engine.getAccount(USER));
show("liquidation log", engine.getLiquidations(USER));

line();
console.log("Second run: a WINNING trade closed manually");
line();

const bob = "bob";
engine.setMarkPrice("SOL", 200);
engine.deposit(bob, 500);
show("bob opens 5x SHORT, size 5 @ 200", engine.openPosition(bob, "SOL", "short", 5, 5));
engine.setMarkPrice("SOL", 180); // short profits when price falls: (200-180)*5 = +100
show("mark -> 180, bob position", engine.getPosition(bob, "SOL"));
show("bob closes SOL short (realizes +PnL)", engine.closePosition(bob, "SOL"));
show("bob account after close", engine.getAccount(bob));

line();
console.log("Demo complete.");
line();
