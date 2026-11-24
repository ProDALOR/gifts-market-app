import assert from "node:assert/strict";
import { access, mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { chromium } from "playwright";

const base = process.env.MARKET_APP_TEST_URL;
const control = process.env.MARKET_APP_TEST_CONTROL;
const screenshots = process.env.MARKET_APP_TEST_SCREENSHOTS;
if (!base || !control || !screenshots)
  throw Error("Run scripts/smoke.py to start the disposable hub fixture");
await mkdir(screenshots, { recursive: true });
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({
    viewport: { width: 1440, height: 1000 },
  });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(
    `${base}/?collection=tg.gift.123&currency=GRAM&mode=ask&sort=price_asc`,
  );
  await page.locator(".gift-card").first().waitFor();
  await page.getByText("Live", { exact: true }).waitFor();
  await page.waitForFunction(
    () =>
      document.querySelector(".gift-grid .quote-amount")?.textContent ===
      "1.25",
  );
  await page.screenshot({
    path: join(screenshots, "orderbook.png"),
    fullPage: true,
  });
  await writeFile(join(control, "update.ready"), "ready");
  const deadline = Date.now() + 30000;
  for (;;) {
    try {
      await access(join(control, "update.done"));
      break;
    } catch {
      if (Date.now() >= deadline) throw Error("Fixture update timed out");
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  await page.waitForFunction(
    () =>
      document.querySelector(".gift-grid .quote-amount")?.textContent ===
      "0.50.." &&
      document.querySelector(".gift-grid .quote-amount")?.getAttribute("aria-label") ===
      "0.500000001 GRAM",
  );
  assert.deepEqual(errors, []);
  console.log("Real-hub browser catalog and live order updates passed.");
} finally {
  await browser.close();
}
