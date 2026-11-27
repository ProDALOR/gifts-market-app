import assert from "node:assert/strict";
import { chromium } from "playwright";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";
const base = process.env.MARKET_TELEGRAM_TEST_URL;
const proof = process.env.MARKET_TELEGRAM_TEST_PROOF;
const screenshots = process.env.MARKET_TELEGRAM_SCREENSHOTS;
if (!base || !proof || !screenshots)
  throw Error("Run the Rust Telegram browser test");
await mkdir(screenshots, { recursive: true });
const browser = await chromium.launch({ headless: true });
try {
  const context = await browser.newContext({
    viewport: { width: 390, height: 844 },
  });
  const page = await context.newPage();
  const errors = [];
  let signIns = 0,
    restored = 0,
    sdkLoads = 0;
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => {
    if (request.url().endsWith("/auth/telegram")) signIns++;
    if (request.url().endsWith("/auth/me")) restored++;
  });
  await context.route(
    "https://telegram.org/js/telegram-web-app.js",
    (route) => {
      sdkLoads++;
      return route.fulfill({
        contentType: "text/javascript",
        body: `
      (() => {
        const fields = new URLSearchParams(location.hash.slice(1));
        const raw = fields.get('tgWebAppData') || sessionStorage.getItem('test.launch');
        sessionStorage.setItem('test.launch', raw);
        const events = new Map();
        const back = new Set();
        const state = { ready: false, expanded: false, visible: false };
        const app = {
          initData: raw, initDataUnsafe: { user: { id: 1, first_name: 'FORGED' }, start_param: new URLSearchParams(raw).get('start_param') },
          colorScheme: 'light', themeParams: { bg_color: '#ffffff', secondary_bg_color: '#f3f5f7', text_color: '#18212b', hint_color: '#65717e', button_color: '#2481cc' },
          safeAreaInset: { top: 24, bottom: 12, left: 0, right: 0 }, contentSafeAreaInset: { top: 24, bottom: 0, left: 0, right: 0 },
          ready() { state.ready = true; }, expand() { state.expanded = true; },
          onEvent(name, fn) { if (!events.has(name)) events.set(name, new Set()); events.get(name).add(fn); },
          offEvent(name, fn) { events.get(name)?.delete(fn); },
          BackButton: { show() { state.visible = true; }, hide() { state.visible = false; }, onClick(fn) { back.add(fn); }, offClick(fn) { back.delete(fn); } },
        };
        window.Telegram = { WebApp: app };
        window.telegramTest = { state, back() { [...back].forEach(fn => fn()); }, dark() { app.colorScheme = 'dark'; app.themeParams = { bg_color: '#0b1116', text_color: '#c8d4de', secondary_bg_color: '#141e26', hint_color: '#81919d' }; events.get('themeChanged')?.forEach(fn => fn()); } };
      })();`,
      });
    },
  );
  await page.goto(
    `${base}/#tgWebAppData=${encodeURIComponent(proof)}&tgWebAppVersion=8.0`,
  );
  await page
    .getByRole("button", { name: "Account: Anna", exact: true })
    .waitFor();
  await page.getByRole("heading", { name: "Gift", exact: true }).waitFor();
  await page.getByText("Live", { exact: true }).waitFor();
  assert.equal(
    await page
      .getByRole("combobox", { name: "Sort by", exact: true })
      .innerText(),
    "Price",
  );
  assert.ok(page.url().includes("max=2"));
  assert.ok(!page.url().includes("tgWebAppData"));
  assert.equal(
    await page.locator("html").getAttribute("data-telegram-theme"),
    "light",
  );
  assert.equal(
    await page.evaluate(() =>
      getComputedStyle(document.documentElement).getPropertyValue(
        "--tg-safe-top",
      ),
    ),
    "48px",
  );
  assert.ok(
    await page.evaluate(
      () =>
        window.telegramTest.state.ready && window.telegramTest.state.expanded,
    ),
  );
  assert.equal(signIns, 1);
  assert.equal(sdkLoads, 1);
  assert.equal(await page.getByText("FORGED", { exact: true }).count(), 0);
  await page
    .locator('.card-open[data-order-id="order"][data-venue-id="test"]')
    .click();
  await page.getByRole("dialog", { name: "Order details" }).waitFor();
  await page.waitForFunction(() => window.telegramTest.state.visible);
  await page.evaluate(() => window.telegramTest.back());
  await page.waitForFunction(
    () => !document.querySelector('[aria-label="Order details"]'),
  );
  await page
    .getByRole("button", { name: "Account: Anna", exact: true })
    .click();
  await page.getByText("ID 9007199254740991", { exact: true }).waitFor();
  await page.screenshot({
    path: join(screenshots, "mini-app-light.png"),
    fullPage: true,
  });
  await page.evaluate(() => window.telegramTest.back());
  await page.waitForFunction(
    () => !document.querySelector('[aria-label="Telegram account"]'),
  );
  await page.reload();
  await page
    .getByRole("button", { name: "Account: Anna", exact: true })
    .waitFor();
  assert.equal(signIns, 1, "reload should reuse the application session");
  assert.ok(restored >= 1);
  await page.evaluate(() => window.telegramTest.dark());
  await page.waitForFunction(
    () => document.documentElement.dataset.telegramTheme === "dark",
  );
  await page.screenshot({
    path: join(screenshots, "mini-app-dark.png"),
    fullPage: true,
  });
  await page
    .getByRole("button", { name: "Account: Anna", exact: true })
    .click();
  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await page.reload();
  await page
    .getByRole("button", { name: "Telegram sign-in", exact: true })
    .waitFor();
  assert.equal(
    signIns,
    1,
    "sign-out should survive a reload within this launch",
  );
  await page
    .getByRole("button", { name: "Telegram sign-in", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Telegram account" })
    .getByRole("button", { name: "Sign in", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Account: Anna", exact: true })
    .waitFor();
  assert.equal(signIns, 2);
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
    false,
  );
  assert.deepEqual(errors, []);
  // A normal website visit never loads the bridge or requires Telegram authentication.
  const standalone = await browser.newContext();
  const site = await standalone.newPage();
  let external = 0;
  await standalone.route("https://telegram.org/**", (route) => {
    external++;
    return route.abort();
  });
  await site.goto(base);
  await site.getByRole("heading", { name: "Gift", exact: true }).waitFor();
  await site.getByText("Live", { exact: true }).waitFor();
  assert.equal(external, 0);
  const link = await site
    .getByRole("link", { name: "Telegram ↗", exact: true })
    .getAttribute("href");
  assert.ok(link.startsWith("https://t.me/test_market_bot?startapp=t_"));
  console.log(
    "Telegram bridge, real signed login, session restore, safe areas, themes, launch selection, Back button, sign-out and standalone browsing passed.",
  );
} finally {
  await browser.close();
}
