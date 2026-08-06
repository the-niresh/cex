import { execSync } from "node:child_process";
import { expect, test, type Page } from "@playwright/test";

/**
 * End-to-end, against the four real processes.
 *
 * These need the stack up:
 *
 *   docker compose up -d
 *   SECRET=$(openssl rand -hex 32)
 *   CEX_SNAPSHOT_DIR=/tmp/snap CEX_BLOCK_MS=200 ./target/debug/engine &
 *   CEX_JWT_SECRET=$SECRET ./target/debug/api &
 *   ./target/debug/persist &
 *   CEX_JWT_SECRET=$SECRET ./target/debug/ws &
 *
 * `CEX_BLOCK_MS=200` matters: the engine only answers reads between blocking
 * stream reads, so on the 5s default every snapshot fetch takes 5s and these
 * time out. See the note in the README.
 */

const API = "http://localhost:8080";
const REPO = "/srv/claude/projects/cex";
/** BTC_USDT tick size, in quote atoms: 0.01 USDT. */
const TICK = 10_000;

/** A username no other run will pick. */
function freshUser(prefix: string): string {
  return `${prefix}${Date.now().toString(36)}${Math.floor(Math.random() * 1e4)}`;
}

/**
 * A price no other run has used, well below the market so it rests.
 *
 * The exchange these run against is persistent: orders left behind by earlier
 * runs stay on the book. Reusing a fixed price means a new order queues
 * *behind* an old one at the same price, and a test that expects its own order
 * to be hit watches the stale one get filled instead.
 */
function freshPrice(): string {
  const ticks = 3_000_000 + Math.floor(Math.random() * 500_000);
  return `${(ticks / 100).toFixed(2)}`;
}

/** Register through the UI and fund the account through the UI's own deposit. */
async function signUpAndFund(page: Page, asset: string, amount: string) {
  const username = freshUser("e2e");
  await page.goto("/");

  await page.getByLabel("username").fill(username);
  await page.getByLabel("password").fill("a-good-password");
  await page.getByRole("button", { name: "CREATE ACCOUNT" }).click();

  // The overlay goes once a session exists.
  await expect(page.getByRole("button", { name: "CREATE ACCOUNT" })).toBeHidden();

  await page.locator(".assets button", { hasText: asset }).click();
  await page.getByLabel("deposit amount").fill(amount);
  await page.getByRole("button", { name: "CREDIT" }).click();

  await expect(page.locator(".brow", { hasText: asset })).toBeVisible();
  return username;
}

test("registers, deposits, places an order, sees it in the book, cancels it", async ({ page }) => {
  await signUpAndFund(page, "USDT", "500000");

  // A bid far below the market, so it rests rather than trading.
  const price = freshPrice();
  await page.getByLabel("price").fill(price);
  await page.getByLabel("quantity").fill("0.01000");

  await expect(page.locator(".readout .r.total .v")).not.toHaveText("—");

  await page.locator("button.submit").click();

  // It is mine, it is open, and it is on the book.
  const row = page.locator(".oo").first();
  await expect(row).toBeVisible();
  await expect(row.locator(".side")).toHaveText("BUY");
  await expect(row.locator(".st")).toContainText("OPEN");

  await expect(page.locator(".lvl.has-mine")).toHaveCount(1);
  await expect(page.locator(".lvl.has-mine .price")).toContainText(price.split(".")[0]!.slice(0, 2));

  // The funds it is holding show as locked, not spent.
  await expect(page.locator(".brow", { hasText: "USDT" }).locator(".lock")).not.toHaveText(
    /^0\.0+$/,
  );

  await row.locator("button.x").click();

  await expect(page.locator(".oo")).toHaveCount(0);
  await expect(page.locator(".lvl.has-mine")).toHaveCount(0);
});

test("refuses a price off the tick and a quantity off the lot, before sending", async ({ page }) => {
  await signUpAndFund(page, "USDT", "500000");

  // BTC_USDT ticks at 0.01 and lots at 0.00001.
  await page.getByLabel("price").fill("30000.005");
  await page.getByLabel("quantity").fill("0.01000");

  await expect(page.locator(".bad-note")).toContainText("multiple of 0.01");
  await expect(page.locator("button.submit")).toBeDisabled();

  await page.getByLabel("price").fill("30000.00");
  await page.getByLabel("quantity").fill("0.000001");

  await expect(page.locator(".bad-note")).toContainText("multiple of 0.00001");
  await expect(page.locator("button.submit")).toBeDisabled();

  // Nothing was ever sent, so nothing is resting.
  await expect(page.locator(".oo")).toHaveCount(0);
});

test("clicking a level in the ladder loads its price into the ticket", async ({ page }) => {
  await signUpAndFund(page, "USDT", "500000");

  const price = freshPrice();
  await page.getByLabel("price").fill(price);
  await page.getByLabel("quantity").fill("0.01000");
  await page.locator("button.submit").click();
  await expect(page.locator(".lvl.has-mine")).toHaveCount(1);

  await page.getByLabel("price").fill("");
  await page.locator(".lvl.has-mine").click();

  await expect(page.getByLabel("price")).toHaveValue(price);

  await page.locator(".oo button.x").first().click();
  await expect(page.locator(".oo")).toHaveCount(0);
});

test("two users cross, and neither sees the other's id", async ({ browser }) => {
  // Two full sign-ups plus a cross, against a real engine and database. It
  // does roughly twice the work of any other test here.
  test.setTimeout(300_000);

  const makerContext = await browser.newContext();
  const takerContext = await browser.newContext();
  const maker = await makerContext.newPage();
  const taker = await takerContext.newPage();

  await signUpAndFund(maker, "USDT", "500000");
  await signUpAndFund(taker, "BTC", "5.00000000");

  // The maker's bid has to be the *best* bid, or the taker's sell sweeps the
  // better ones first and never reaches it. One tick above the current best
  // does that, and is self-avoiding across runs: each run leaves the book a
  // tick higher, so the next run picks a price no earlier run has used.
  const snapshot = await maker.request.get(`${API}/depth/BTC_USDT`);
  const depth = (await snapshot.json()) as { bids: [number, number][]; asks: [number, number][] };
  const bestBid = depth.bids[0]?.[0] ?? 50_000_000_000;
  const bestAsk = depth.asks[0]?.[0] ?? bestBid + 10_000_000;
  const atoms = bestBid + TICK;
  expect(atoms, "no room between the best bid and the best ask to rest at").toBeLessThan(bestAsk);
  const price = (atoms / 1e6).toFixed(2);
  // What the ladder actually renders, separators and all.
  const shown = (atoms / 1e6).toLocaleString("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });

  await maker.getByLabel("price").fill(price);
  await maker.getByLabel("quantity").fill("0.02000");
  await maker.locator("button.submit").click();
  await expect(maker.locator(".lvl.has-mine")).toHaveCount(1);

  // The other window sees the level appear on its own feed, without a reload.
  await expect(taker.locator(".book .lvl .price", { hasText: shown })).toBeVisible();

  // The taker lifts it.
  await taker.getByLabel("price").fill(price);
  await taker.getByLabel("quantity").fill("0.02000");
  await taker.locator('.seg.side button[data-side="sell"]').click();
  await taker.locator("button.submit").click();

  // Both see their own fill on their own private feed.
  await expect(maker.locator(".fl").first()).toBeVisible();
  await expect(taker.locator(".fl").first()).toBeVisible();
  await expect(maker.locator(".fl").first().locator(".side")).toContainText("BUY");
  await expect(taker.locator(".fl").first().locator(".side")).toContainText("SELL");

  // The maker rested, the taker aggressed, and each is told which they were.
  await expect(maker.locator(".fl").first().locator(".role")).toHaveText("M");
  await expect(taker.locator(".fl").first().locator(".role")).toHaveText("T");

  // Neither page contains the other's user id anywhere.
  const makerId = await maker.locator(".who .id").textContent();
  const takerId = await taker.locator(".who .id").textContent();
  const makerBody = (await maker.locator("body").textContent()) ?? "";
  const takerBody = (await taker.locator("body").textContent()) ?? "";
  const trim = (v: string | null) => (v ?? "").replace("…", "").trim();

  expect(makerBody).not.toContain(trim(takerId));
  expect(takerBody).not.toContain(trim(makerId));

  // The order traded in full, so nothing of the maker's is left resting.
  await expect(maker.locator(".lvl.has-mine")).toHaveCount(0);

  await makerContext.close();
  await takerContext.close();
});

test("reconnects and resyncs when ws is restarted, without a reload", async ({ page }) => {
  const secret = process.env.CEX_JWT_SECRET;
  test.skip(
    !secret,
    "set CEX_JWT_SECRET to the running stack's secret so this test can restart ws",
  );

  await signUpAndFund(page, "USDT", "500000");
  await expect(page.locator(".statusbar")).toContainText("live");

  const resyncsBefore = Number((await page.locator(".right b").first().textContent()) ?? "0");

  // Kill the real market-data process and bring it back, exactly as a deploy
  // would. Nothing touches the page — no reload, no navigation.
  execSync("pkill -f 'debug/ws$' || true");
  await page.waitForTimeout(1_500);
  await expect(page.locator(".screen")).toHaveClass(/stale/);

  execSync(
    `setsid env CEX_JWT_SECRET='${secret}' ${REPO}/target/debug/ws > /dev/null 2>&1 < /dev/null &`,
    { shell: "/bin/bash" },
  );

  // It comes back on its own, and refetches rather than resuming mid-stream.
  await expect(page.locator(".statusbar")).toContainText("live", { timeout: 30_000 });
  await expect(page.locator(".screen")).not.toHaveClass(/stale/, { timeout: 30_000 });

  const resyncsAfter = Number((await page.locator(".right b").first().textContent()) ?? "0");
  expect(resyncsAfter).toBeGreaterThan(resyncsBefore);

  // And it still trades, which is the only proof that the recovery was real.
  await page.getByLabel("price").fill(freshPrice());
  await page.getByLabel("quantity").fill("0.01000");
  await page.locator("button.submit").click();
  await expect(page.locator(".oo")).toHaveCount(1);

  await page.locator(".oo button.x").first().click();
  await expect(page.locator(".oo")).toHaveCount(0);
});

test("the public tape and depth are visible before signing in", async ({ page }) => {
  await page.goto("/");

  // The auth panel is up, and the exchange is visibly already running behind it.
  await expect(page.getByRole("button", { name: "CREATE ACCOUNT" })).toBeVisible();
  await expect(page.locator(".book .chead")).toBeVisible();
  await expect(page.locator(".statusbar")).toContainText("localhost:8080");

  const depth = await page.request.get(`${API}/depth/BTC_USDT`);
  expect(depth.ok()).toBeTruthy();
});
