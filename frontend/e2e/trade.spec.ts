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
 * A price that rests *and* stays visible in the ladder.
 *
 * Derived from the live best bid rather than fixed, because both facts matter
 * and they pull against each other. It has to be below the best bid so it
 * rests instead of trading. It also has to be within the fourteen levels the
 * ladder renders, or a test asserting "my order shows in the book" fails on a
 * busy exchange while the order is resting perfectly well just out of view —
 * which is exactly what a fixed 30,000-35,000 range did once the book filled
 * up around 50,000.
 *
 * The random offset keeps runs off each other's prices; a shared level would
 * still mark as mine, but a distinct one makes a failure easier to read.
 */
async function restingBidPrice(page: Page): Promise<string> {
  const snapshot = await page.request.get(`${API}/depth/BTC_USDT`);
  const depth = (await snapshot.json()) as { bids: [number, number][] };
  const best = depth.bids[0]?.[0] ?? 50_000_000_000;
  const atoms = best - (1 + Math.floor(Math.random() * 8)) * TICK;
  return (atoms / 1e6).toFixed(2);
}

/**
 * Rest one bid and one ask on the book, through the API rather than the screen.
 *
 * The tests that read the ladder used to depend on levels left behind by
 * *earlier* tests. That made them order-dependent, and it meant they passed for
 * the wrong reason: while the depth-resync bug was live, the tests before them
 * failed early and abandoned their orders on the book, which is what gave these
 * something to read. Against a freshly reset exchange the ask side was always
 * empty — nothing in the suite ever leaves a resting ask — so "MID and BBO"
 * could not pass at all.
 *
 * Seeding through the API keeps them quick: they are about what the ladder
 * renders, not about order entry, which other tests already cover through the UI.
 */
async function seedBook(page: Page): Promise<void> {
  const username = freshUser("seed");
  const registered = await page.request.post(`${API}/register`, {
    data: { username, name: `Seed ${username}`, password: "a-good-password" },
  });
  const { token } = (await registered.json()) as { token: string };
  const headers = { authorization: `Bearer ${token}` };

  for (const asset of ["USDT", "BTC"]) {
    await page.request.post(`${API}/deposit`, {
      headers: { ...headers, "idempotency-key": crypto.randomUUID() },
      data: { asset, amount: 1_000_000_000_000_000 },
    });
  }

  // Far apart and on the tick, so neither crosses the other or anything an
  // earlier run left behind.
  const bid = 30_000_000_000 + Math.floor(Math.random() * 1_000) * TICK;
  const ask = 90_000_000_000 + Math.floor(Math.random() * 1_000) * TICK;
  for (const [side, price] of [
    ["BUY", bid],
    ["SELL", ask],
  ] as const) {
    const placed = await page.request.post(`${API}/orders`, {
      headers: { ...headers, "idempotency-key": crypto.randomUUID() },
      data: {
        symbol: "BTC_USDT",
        side,
        order_type: "LIMIT",
        time_in_force: "GTC",
        price,
        qty: 1_000_000,
      },
    });
    expect(placed.ok(), `seeding a resting ${side} must succeed`).toBeTruthy();
  }
}

/** Register through the UI and fund the account through the UI's own deposit. */
async function signUpAndFund(page: Page, asset: string, amount: string) {
  const username = freshUser("e2e");
  await page.goto("/");

  // Nothing asks for an account on arrival, so the panel has to be opened —
  // and it opens on LOG IN, so registering means switching tabs.
  await page.getByTestId("account-action").click();
  await page.getByRole("button", { name: "REGISTER" }).click();
  // Exact, or this also matches the "username" field.
  await page.getByLabel("name", { exact: true }).fill(`Trader ${username}`);
  await page.getByLabel("username").fill(username);
  await page.getByLabel("password").fill("a-good-password");
  await page.getByRole("button", { name: "CREATE ACCOUNT" }).click();

  // The overlay goes once a session exists.
  await expect(page.getByTestId("auth-panel")).toBeHidden();

  await page.getByTestId("deposit-assets").getByText(asset, { exact: true }).click();
  await page.getByLabel("deposit amount").fill(amount);
  await page.getByRole("button", { name: "CREDIT" }).click();

  await expect(page.getByTestId("balance-row").filter({ hasText: asset })).toBeVisible();
  return username;
}

test("registers, deposits, places an order, sees it in the book, cancels it", async ({ page }) => {
  await signUpAndFund(page, "USDT", "500000");

  // Below the best bid, so it rests rather than trading — and close enough to
  // it that the ladder actually renders the level.
  const price = await restingBidPrice(page);
  await page.getByLabel("price").fill(price);
  await page.getByLabel("quantity").fill("0.01000");

  await expect(page.getByTestId("ticket-total")).not.toHaveText("—");

  await page.getByTestId("ticket-submit").click();

  // It is mine, it is open, and it is on the book.
  const row = page.getByTestId("open-order").first();
  await expect(row).toBeVisible();
  await expect(row.getByTestId("order-side")).toHaveText("BUY");
  await expect(row.getByTestId("order-status")).toContainText("OPEN");

  await expect(page.locator('[data-testid="ladder-level"][data-mine="true"]')).toHaveCount(1);
  await expect(page.locator('[data-testid="ladder-level"][data-mine="true"] [data-testid="level-price"]')).toContainText(price.split(".")[0]!.slice(0, 2));

  // The funds it is holding show as locked, not spent.
  await expect(page.getByTestId("balance-row").filter({ hasText: "USDT" }).getByTestId("balance-locked")).not.toHaveText(
    /^0\.0+$/,
  );

  await row.getByTestId("cancel-order").click();

  await expect(page.getByTestId("open-order")).toHaveCount(0);
  await expect(page.locator('[data-testid="ladder-level"][data-mine="true"]')).toHaveCount(0);
});

test("refuses a price off the tick and a quantity off the lot, before sending", async ({ page }) => {
  await signUpAndFund(page, "USDT", "500000");

  // BTC_USDT ticks at 0.01 and lots at 0.00001.
  await page.getByLabel("price").fill("30000.005");
  await page.getByLabel("quantity").fill("0.01000");

  await expect(page.getByTestId("ticket-problem")).toContainText("multiple of 0.01");
  await expect(page.getByTestId("ticket-submit")).toBeDisabled();

  await page.getByLabel("price").fill("30000.00");
  await page.getByLabel("quantity").fill("0.000001");

  await expect(page.getByTestId("ticket-problem")).toContainText("multiple of 0.00001");
  await expect(page.getByTestId("ticket-submit")).toBeDisabled();

  // Nothing was ever sent, so nothing is resting.
  await expect(page.getByTestId("open-order")).toHaveCount(0);
});

test("clicking a level in the ladder loads its price into the ticket", async ({ page }) => {
  await signUpAndFund(page, "USDT", "500000");

  const price = await restingBidPrice(page);
  await page.getByLabel("price").fill(price);
  await page.getByLabel("quantity").fill("0.01000");
  await page.getByTestId("ticket-submit").click();
  await expect(page.locator('[data-testid="ladder-level"][data-mine="true"]')).toHaveCount(1);

  await page.getByLabel("price").fill("");
  await page.locator('[data-testid="ladder-level"][data-mine="true"]').click();

  await expect(page.getByLabel("price")).toHaveValue(price);

  await page.getByTestId("cancel-order").first().click();
  await expect(page.getByTestId("open-order")).toHaveCount(0);
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
  await maker.getByTestId("ticket-submit").click();
  await expect(maker.locator('[data-testid="ladder-level"][data-mine="true"]')).toHaveCount(1);

  // The other window sees the level appear on its own feed, without a reload.
  await expect(taker.locator('[data-testid="ladder-level"] [data-testid="level-price"]', { hasText: shown })).toBeVisible();

  // The taker lifts it.
  await taker.getByLabel("price").fill(price);
  await taker.getByLabel("quantity").fill("0.02000");
  await taker.locator('[data-testid="side-select"] button[data-side="sell"]').click();
  await taker.getByTestId("ticket-submit").click();

  // Fills share a panel with open orders now, so ask for them the way a user
  // would. This is a real layout change rather than a restyle, which is why
  // the test changed with it — the testids alone cannot absorb a new tab.
  await maker.getByTestId("activity-tab-fills").click();
  await taker.getByTestId("activity-tab-fills").click();

  // Both see their own fill on their own private feed.
  await expect(maker.getByTestId("fill-row").first()).toBeVisible();
  await expect(taker.getByTestId("fill-row").first()).toBeVisible();
  await expect(maker.getByTestId("fill-row").first().getByTestId("fill-side")).toContainText("BUY");
  await expect(taker.getByTestId("fill-row").first().getByTestId("fill-side")).toContainText("SELL");

  // The maker rested, the taker aggressed, and each is told which they were.
  await expect(maker.getByTestId("fill-row").first().getByTestId("fill-role")).toHaveText("M");
  await expect(taker.getByTestId("fill-row").first().getByTestId("fill-role")).toHaveText("T");

  // Neither page contains the other's user id anywhere.
  const makerId = await maker.getByTestId("account-id").textContent();
  const takerId = await taker.getByTestId("account-id").textContent();
  const makerBody = (await maker.locator("body").textContent()) ?? "";
  const takerBody = (await taker.locator("body").textContent()) ?? "";
  const trim = (v: string | null) => (v ?? "").replace("…", "").trim();

  expect(makerBody).not.toContain(trim(takerId));
  expect(takerBody).not.toContain(trim(makerId));

  // The order traded in full, so nothing of the maker's is left resting.
  await expect(maker.locator('[data-testid="ladder-level"][data-mine="true"]')).toHaveCount(0);

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
  await expect(page.getByTestId("statusbar")).toContainText("live");

  const resyncsBefore = Number((await page.getByTestId("status-resyncs").textContent()) ?? "0");

  // Kill the real market-data process and bring it back, exactly as a deploy
  // would. Nothing touches the page — no reload, no navigation.
  execSync("pkill -f 'debug/ws$' || true");
  await page.waitForTimeout(1_500);
  await expect(page.getByTestId("screen")).toHaveAttribute("data-stale", "true");

  execSync(
    `setsid env CEX_JWT_SECRET='${secret}' ${REPO}/target/debug/ws > /dev/null 2>&1 < /dev/null &`,
    { shell: "/bin/bash" },
  );

  // It comes back on its own, and refetches rather than resuming mid-stream.
  await expect(page.getByTestId("statusbar")).toContainText("live", { timeout: 30_000 });
  await expect(page.getByTestId("screen")).toHaveAttribute("data-stale", "false", {
    timeout: 30_000,
  });

  const resyncsAfter = Number((await page.getByTestId("status-resyncs").textContent()) ?? "0");
  expect(resyncsAfter).toBeGreaterThan(resyncsBefore);

  // And it still trades, which is the only proof that the recovery was real.
  await page.getByLabel("price").fill(await restingBidPrice(page));
  await page.getByLabel("quantity").fill("0.01000");
  await page.getByTestId("ticket-submit").click();
  await expect(page.getByTestId("open-order")).toHaveCount(1);

  await page.getByTestId("cancel-order").first().click();
  await expect(page.getByTestId("open-order")).toHaveCount(0);
});

test("the whole screen is usable before signing in, with nothing in the way", async ({ page }) => {
  await page.goto("/");

  // Nothing blocks the screen: no panel, and the exchange is already running.
  await expect(page.getByTestId("auth-panel")).toHaveCount(0);
  await expect(page.getByTestId("ladder-heads")).toBeVisible();
  // API_URL is same-origin by default (Vite proxies /register, /depth, etc. to
  // 8080) so this reads the page's own origin, not the API's actual port —
  // see the comment on API_URL in src/lib/api.ts.
  await expect(page.getByTestId("statusbar")).toContainText("localhost:5173");

  const depth = await page.request.get(`${API}/depth/BTC_USDT`);
  expect(depth.ok()).toBeTruthy();
});

test("the book and the tape share one column, and the sweep card does the arithmetic", async ({
  page,
}) => {
  await seedBook(page);
  await page.goto("/");

  // The header states the day, derived from hourly candles rather than served.
  const ticker = page.getByTestId("ticker");
  await expect(ticker).toContainText("24h change");
  await expect(ticker).toContainText("24h high");
  await expect(ticker).toContainText("24h volume");

  // BOOK is the default; TRADES swaps the same column over, and back.
  await expect(page.getByTestId("ladder-heads")).toBeVisible();
  await expect(page.getByTestId("tape-panel")).toHaveCount(0);
  await page.getByTestId("book-tabs").getByText("TRADES").click();
  await expect(page.getByTestId("tape-heads")).toBeVisible();
  await expect(page.getByTestId("book-panel")).toHaveCount(0);
  await page.getByTestId("book-tabs").getByText("BOOK").click();
  await expect(page.getByTestId("ladder-heads")).toBeVisible();

  // Hovering a level answers "what would taking all of that cost".
  await expect(page.getByTestId("sweep")).toHaveCount(0);
  const level = page.locator('[data-testid="ladder-level"][data-side="bids"]').first();
  await level.hover();
  const sweep = page.getByTestId("sweep");
  await expect(sweep).toBeVisible();
  // The best bid alone, so the sweep's size is that level's own size and the
  // average is its own price — the one case checkable without re-deriving it.
  await expect(sweep).toContainText("avg price");
  const shownSize = await level.getByTestId("level-total").innerText();
  await expect(sweep.getByTestId("sweep-sum-base")).toHaveText(shownSize);

  // And the imbalance bar reports both sides, adding to 100.
  const shares = await page.getByTestId("imbalance").locator("span").allInnerTexts();
  expect(shares).toHaveLength(2);
  const total = shares.reduce((sum, s) => sum + Number(s.replace("%", "")), 0);
  expect(total).toBe(100);
});

test("MID and BBO fill the price from the book", async ({ page }) => {
  await seedBook(page);
  await page.goto("/");
  await expect(page.locator('[data-testid="ladder-level"]').first()).toBeVisible();

  const price = page.getByLabel("price");
  await expect(price).toHaveValue("");

  // BBO on the buy side is the best bid — the top row of the bids ladder.
  await page.getByTestId("price-picks").getByText("BBO").click();
  const bestBid = (await page.locator('[data-testid="ladder-level"][data-side="bids"]').first().getByTestId("level-price").innerText())
    .replace(/,/g, "");
  await expect(price).toHaveValue(bestBid);

  // MID lands between the two, and on a tick the ticket will accept.
  await page.getByTestId("price-picks").getByText("MID").click();
  const mid = Number((await price.inputValue()).replace(/,/g, ""));
  const bestAsk = Number(
    (await page.locator('[data-testid="ladder-level"][data-side="asks"]').last().getByTestId("level-price").innerText()).replace(/,/g, ""),
  );
  expect(mid).toBeGreaterThanOrEqual(Number(bestBid));
  expect(mid).toBeLessThanOrEqual(bestAsk);
  await expect(page.getByTestId("ticket-problem")).toHaveCount(0);
});

test("pressing BUY while signed out asks for an account instead of doing nothing", async ({
  page,
}) => {
  await page.goto("/");

  // The button is live, not dead — a disabled control would teach a visitor
  // nothing about why nothing happened.
  const submit = page.getByTestId("ticket-submit");
  await expect(submit).toHaveText("LOG IN TO BUY");
  await expect(submit).toBeEnabled();

  await submit.click();
  await expect(page.getByTestId("auth-panel")).toBeVisible();

  // It opens on LOG IN, not REGISTER — most arrivals already have an account.
  await expect(page.locator('[data-testid="auth-mode"] button[aria-selected="true"]')).toHaveText("LOG IN");

  // And it is dismissible: the book is public, so nobody is trapped here.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("auth-panel")).toHaveCount(0);

  // SELL says so too.
  await page.locator('[data-testid="side-select"] button[data-side="sell"]').click();
  await expect(page.getByTestId("ticket-submit")).toHaveText("LOG IN TO SELL");
});

test("registering asks for a name, and the name comes back on the next sign in", async ({
  page,
}) => {
  const username = freshUser("e2e");
  const displayName = `Ada ${username}`;

  await page.goto("/");
  await page.getByTestId("account-action").click();
  await page.getByRole("button", { name: "REGISTER" }).click();

  // A name is required to register — the button stays dead without one.
  await page.getByLabel("username").fill(username);
  await page.getByLabel("password").fill("a-good-password");
  await expect(page.getByTestId("auth-submit")).toBeDisabled();

  await page.getByLabel("name", { exact: true }).fill(displayName);
  await expect(page.getByTestId("auth-submit")).toBeEnabled();
  await page.getByTestId("auth-submit").click();

  await expect(page.getByTestId("auth-panel")).toBeHidden();
  await expect(page.getByTestId("account-name")).toHaveText(displayName);

  // Sign out and back in: the name is the account's, not this session's.
  await page.getByTestId("account-action").click();
  await expect(page.getByTestId("account-name")).toHaveCount(0);

  await page.getByTestId("account-action").click();
  await page.getByLabel("username").fill(username);
  await page.getByLabel("password").fill("a-good-password");
  await page.getByTestId("auth-submit").click();

  await expect(page.getByTestId("account-name")).toHaveText(displayName);
});
