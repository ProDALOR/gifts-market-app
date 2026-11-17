import assert from "node:assert/strict";
import { chromium } from "playwright";

const url = process.env.MARKET_API_ONLY_TEST_URL;
if (!url) throw Error("Run the Rust API-only browser test");
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage();
  const requests = [],
    errors = [];
  page.on("request", (request) => requests.push(request.url()));
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${url}/?item=tg.gift.123&currency=GRAM#tgWebAppData=unused`);
  await page.locator(".gift-grid .price").first().waitFor();
  await page.getByText("Live", { exact: true }).waitFor();
  assert.equal(await page.locator(".gift-card").count(), 1);
  assert.equal(
    await page
      .getByRole("button", { name: "Telegram sign-in", exact: true })
      .count(),
    0,
  );
  assert.equal(
    await page.getByRole("link", { name: "Telegram ↗", exact: true }).count(),
    0,
  );
  assert.ok(
    requests.every((url) => {
      const parsed = new URL(url);
      return (
        !parsed.hostname.includes("telegram.org") &&
        parsed.pathname !== "/telegram" &&
        !parsed.pathname.startsWith("/auth/")
      );
    }),
    "API-only browser attempted to initialize Telegram",
  );
  assert.deepEqual(errors, []);
  console.log(
    "API-only browsing passed without Telegram requests or account controls.",
  );
} finally {
  await browser.close();
}
