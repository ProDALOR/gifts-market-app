import assert from "node:assert/strict";
import { chromium } from "playwright";
import { writeFile, mkdir, access } from "node:fs/promises";
import { join } from "node:path";
const url = process.env.MARKET_APP_TEST_URL;
const control = process.env.MARKET_APP_TEST_CONTROL;
const output = process.env.MARKET_APP_TEST_SCREENSHOTS;
if (!url || !control || !output)
  throw Error("Set local test URL, control and screenshot paths");
await mkdir(output, { recursive: true });
const browser = await chromium.launch({ headless: true });
try {
  const context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
  });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const rows = (count) =>
    page.waitForFunction(
      (n) => document.querySelectorAll("tbody tr").length === n,
      count,
    );
  const price = (expected) =>
    page.waitForFunction(
      (v) => document.querySelector("tbody .price")?.textContent?.startsWith(v),
      expected,
    );
  const button = (name) => page.getByRole("button", { name, exact: true });
  const live = () => page.getByText("Live", { exact: true }).waitFor();
  await page.goto(url);
  await page
    .getByRole("heading", { name: "Velvet Heart (test data)", exact: true })
    .waitFor();
  await live();
  await rows(30);
  await price("1.25");
  await page.getByLabel("Rows per page").selectOption("25");
  await rows(25);
  await button("Next →").click();
  await page.getByText("Page 2", { exact: true }).waitFor();
  await rows(5);
  await button("← Previous").click();
  await page.getByText("Page 1", { exact: true }).waitFor();
  await rows(25);
  await page.getByLabel("Venue", { exact: true }).selectOption("telegram");
  await price("16.25");
  await rows(15);
  await page.getByLabel("Venue", { exact: true }).selectOption("");
  await page.getByLabel("Currency", { exact: true }).selectOption("XTR");
  await price("1,250");
  await rows(25);
  assert.ok(
    (await page.locator("tbody .price").allTextContents()).every((v) =>
      v.includes("XTR"),
    ),
  );
  await page.getByLabel("Currency", { exact: true }).selectOption("GRAM");
  await price("1.25");
  await page.getByRole("button", { name: /Filters/ }).click();
  await page
    .getByLabel("Model", { exact: true })
    .selectOption("tg.gift.123.model.gold");
  await page.getByLabel("Maximum price").fill("3");
  await button("Apply filters →").click();
  await rows(1);
  await price("1.25");
  await page.reload();
  await live();
  await rows(1);
  assert.ok(page.url().includes("filters="));
  await page.getByRole("button", { name: /Filters/ }).click();
  await button("Clear").click();
  await button("Apply filters →").click();
  await rows(30);
  await page.getByLabel("Rows per page").selectOption("25");
  await rows(25);
  await live();
  await button("Ⅱ Pause").click();
  await page.getByText("Paused", { exact: true }).waitFor();
  await rows(25);
  await writeFile(join(control, "update.ready"), "ready");
  for (let n = 0; ; n++) {
    try {
      await access(join(control, "update.done"));
      break;
    } catch {}
    assert.ok(n < 200, "fixture update timed out");
    await new Promise((r) => setTimeout(r, 50));
  }
  assert.match(
    await page.locator("tbody .price").first().innerText(),
    /^1\.25/,
  );
  await button("▶ Resume").click();
  await price("0.500000001");
  await live();
  assert.equal(await page.getByLabel("Rows per page").inputValue(), "25");
  await button("Inspect partner ask-15:GRAM").click();
  await page.getByRole("dialog", { name: "Order details" }).waitFor();
  await page.screenshot({ path: join(output, "desktop.png") });
  await button("Close details").click();
  await button("Bids").click();
  await rows(2);
  await live();
  await button("Executions").click();
  await page.getByText("Snapshot", { exact: true }).waitFor();
  await rows(1);
  await price("0.75");
  await button("Offers").click();
  await rows(25);
  await live();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: join(output, "mobile.png"), fullPage: true });
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    ),
    false,
  );
  assert.deepEqual(errors, []);
  console.log(
    "Public GUI passed price ranking, currency isolation, venue filtering, typed filters, URL restore, pagination, pause/resume, bids, executions and mobile layout.",
  );
} catch (error) {
  for (const page of browser.contexts().flatMap((c) => c.pages())) {
    console.error((await page.locator("body").innerText()).slice(0, 5000));
    await page.screenshot({
      path: join(output, "failure.png"),
      fullPage: true,
    });
  }
  throw error;
} finally {
  await browser.close();
}
