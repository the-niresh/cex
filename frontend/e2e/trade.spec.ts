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

  // Nothing asks for an account on arrival, so the panel has to be opened —
  // and it opens on LOG IN, so registering means switching tabs.
  await page.locator(".who button").click();
  await page.getByRole("button", { name: "REGISTER" }).click();
  // Exact, or this also matches the "username" field.
  await page.getByLabel("name", { exact: true }).fill(`Trader ${username}`);
  await page.getByLabel("username").fill(username);
  await page.getByLabel("password").fill("a-good-password");
  await page.getByRole("button", { name: "CREATE ACCOUNT" }).click();

  // The overlay goes once a session exists.
  await expect(page.locator(".auth")).toBeHidden();

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

  // Two independent accounts in two independent contexts. Run one after the
  // other they burn most of this test's budget before the cross it exists to
  // prove even starts — on a loaded machine, all of it. Nothing here is
  // ordered against anything there, so they overlap.
  await Promise.all([
    signUpAndFund(maker, "USDT", "500000"),
    signUpAndFund(taker, "BTC", "5.00000000"),
  ]);

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

test("the whole screen is usable before signing in, with nothing in the way", async ({ page }) => {
  await page.goto("/");

  // Nothing blocks the screen: no panel, and the exchange is already running.
  await expect(page.locator(".auth")).toHaveCount(0);
  await expect(page.locator(".book .chead")).toBeVisible();
  // API_URL is same-origin by default (Vite proxies /register, /depth, etc. to
  // 8080) so this reads the page's own origin, not the API's actual port —
  // see the comment on API_URL in src/lib/api.ts.
  await expect(page.locator(".statusbar")).toContainText("localhost:5173");

  const depth = await page.request.get(`${API}/depth/BTC_USDT`);
  expect(depth.ok()).toBeTruthy();
});

test("the book and the tape share one column, and the sweep card does the arithmetic", async ({
  page,
}) => {
  await page.goto("/");

  // The header states the day, derived from hourly candles rather than served.
  const ticker = page.locator(".ticker");
  await expect(ticker).toContainText("24h change");
  await expect(ticker).toContainText("24h high");
  await expect(ticker).toContainText("24h volume");

  // BOOK is the default; TRADES swaps the same column over, and back.
  await expect(page.locator(".book .chead")).toBeVisible();
  await expect(page.locator(".tape")).toHaveCount(0);
  await page.locator(".seg.tabs button", { hasText: "TRADES" }).click();
  await expect(page.locator(".tape .chead")).toBeVisible();
  await expect(page.locator(".book")).toHaveCount(0);
  await page.locator(".seg.tabs button", { hasText: "BOOK" }).click();
  await expect(page.locator(".book .chead")).toBeVisible();

  // Hovering a level answers "what would taking all of that cost".
  await expect(page.locator(".sweep")).toHaveCount(0);
  const level = page.locator(".book .bids .lvl").first();
  await level.hover();
  const sweep = page.locator(".sweep");
  await expect(sweep).toBeVisible();
  // The best bid alone, so the sweep's size is that level's own size and the
  // average is its own price — the one case checkable without re-deriving it.
  await expect(sweep).toContainText("avg price");
  const shownSize = await level.locator(".num.cum").innerText();
  await expect(sweep.locator(".r").nth(1).locator(".v")).toHaveText(shownSize);

  // And the imbalance bar reports both sides, adding to 100.
  const shares = await page.locator(".imbalance span").allInnerTexts();
  expect(shares).toHaveLength(2);
  const total = shares.reduce((sum, s) => sum + Number(s.replace("%", "")), 0);
  expect(total).toBe(100);
});

test("MID and BBO fill the price from the book", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".book .lvl").first()).toBeVisible();

  const price = page.getByLabel("price");
  await expect(price).toHaveValue("");

  // BBO on the buy side is the best bid — the top row of the bids ladder.
  await page.locator(".picks button", { hasText: "BBO" }).click();
  const bestBid = (await page.locator(".book .bids .lvl").first().locator(".price").innerText())
    .replace(/,/g, "");
  await expect(price).toHaveValue(bestBid);

  // MID lands between the two, and on a tick the ticket will accept.
  await page.locator(".picks button", { hasText: "MID" }).click();
  const mid = Number((await price.inputValue()).replace(/,/g, ""));
  const bestAsk = Number(
    (await page.locator(".book .asks .lvl").last().locator(".price").innerText()).replace(/,/g, ""),
  );
  expect(mid).toBeGreaterThanOrEqual(Number(bestBid));
  expect(mid).toBeLessThanOrEqual(bestAsk);
  await expect(page.locator(".bad-note")).toHaveCount(0);
});

test("pressing BUY while signed out asks for an account instead of doing nothing", async ({
  page,
}) => {
  await page.goto("/");

  // The button is live, not dead — a disabled control would teach a visitor
  // nothing about why nothing happened.
  const submit = page.locator(".ticket button.submit");
  await expect(submit).toHaveText("LOG IN TO BUY");
  await expect(submit).toBeEnabled();

  await submit.click();
  await expect(page.locator(".auth")).toBeVisible();

  // It opens on LOG IN, not REGISTER — most arrivals already have an account.
  await expect(page.locator(".auth .seg.kind button[aria-selected='true']")).toHaveText("LOG IN");

  // And it is dismissible: the book is public, so nobody is trapped here.
  await page.keyboard.press("Escape");
  await expect(page.locator(".auth")).toHaveCount(0);

  // SELL says so too.
  await page.locator('.seg.side button[data-side="sell"]').click();
  await expect(page.locator(".ticket button.submit")).toHaveText("LOG IN TO SELL");
});

test("registering asks for a name, and the name comes back on the next sign in", async ({
  page,
}) => {
  const username = freshUser("e2e");
  const displayName = `Ada ${username}`;

  await page.goto("/");
  await page.locator(".who button").click();
  await page.getByRole("button", { name: "REGISTER" }).click();

  // A name is required to register — the button stays dead without one.
  await page.getByLabel("username").fill(username);
  await page.getByLabel("password").fill("a-good-password");
  await expect(page.locator(".auth button.submit")).toBeDisabled();

  await page.getByLabel("name", { exact: true }).fill(displayName);
  await expect(page.locator(".auth button.submit")).toBeEnabled();
  await page.locator(".auth button.submit").click();

  await expect(page.locator(".auth")).toBeHidden();
  await expect(page.locator(".who .name")).toHaveText(displayName);

  // Sign out and back in: the name is the account's, not this session's.
  await page.locator(".who button").click();
  await expect(page.locator(".who .name")).toHaveCount(0);

  await page.locator(".who button").click();
  await page.getByLabel("username").fill(username);
  await page.getByLabel("password").fill("a-good-password");
  await page.locator(".auth button.submit").click();

  await expect(page.locator(".who .name")).toHaveText(displayName);
});
