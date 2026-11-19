import assert from "node:assert/strict";
import { readFile, mkdir } from "node:fs/promises";
import { chromium } from "playwright";
const base = process.env.MARKET_DOCS_TEST_URL;
if (!base) throw Error("Run the Rust developer-documentation browser test");
const manifest = JSON.parse(
  await readFile(
    new URL("../dist/.vite/manifest.json", import.meta.url),
    "utf8",
  ),
);
const dependencies = (key, result = new Set()) => {
  if (result.has(key)) return result;
  result.add(key);
  for (const child of manifest[key]?.imports || []) dependencies(child, result);
  return result;
};
const marketFiles = new Set(
  [...dependencies("index.html")].map((key) => manifest[key].file),
);
const docsEntries = [
  "docs/index.html",
  "docs/ws/index.html",
  "src/docs/asyncapi.ts",
];
for (const entry of docsEntries)
  assert.ok(
    !marketFiles.has(manifest[entry].file),
    `${entry} leaked into the market entry`,
  );
const browser = await chromium.launch({ headless: true });
let page;
try {
  page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  page.setDefaultTimeout(30000);
  const errors = [],
    violations = [],
    requests = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => requests.push(request.url()));
  await page.exposeFunction("recordCsp", (violation) =>
    violations.push(violation),
  );
  await page.addInitScript(() =>
    document.addEventListener("securitypolicyviolation", (event) =>
      window.recordCsp(`${event.effectiveDirective}: ${event.blockedURI}`),
    ),
  );
  await page.goto(`${base}/?collection=tg.gift.123&currency=GRAM`);
  await page.locator(".gift-card").first().waitFor();
  assert.ok(
    requests.every(
      (url) => !docsEntries.some((entry) => url.endsWith(manifest[entry].file)),
    ),
    "Market page downloaded documentation code",
  );
  await page.goto(`${base}/docs`);
  await page.getByText("Market App", { exact: true }).first().waitFor();
  await mkdir(new URL("../test-artifacts", import.meta.url), {
    recursive: true,
  });
  await page.screenshot({
    path: new URL("../test-artifacts/docs-rest.png", import.meta.url).pathname,
    fullPage: false,
  });
  await page
    .getByRole("button", { name: /Test Request/i })
    .first()
    .waitFor();
  // Exercise Scalar on a public market endpoint; probes stay out of consumer docs.
  const openCatalog = page.getByRole("button", {
    name: "Open Group - Catalog",
    exact: true,
  });
  if (await openCatalog.isVisible()) await openCatalog.click();
  await page.getByText("/catalog", { exact: true }).first().click();
  await page.getByRole("button", { name: /Test Request.*catalog/i }).click();
  const send = page.getByRole("button", { name: /^Send( Request)?/i }).last();
  await Promise.all([
    page.waitForResponse(
      (response) =>
        response.url() === `${base}/catalog` && response.status() === 200,
    ),
    send.click(),
  ]);
  await page.goto(`${base}/docs/ws`);
  await page
    .locator("#collection option[value='tg.gift.123']")
    .waitFor({ state: "attached" });
  assert.equal(
    await page.locator("#connection-state").textContent(),
    "Disconnected",
  );
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await page.getByText("Connected", { exact: true }).waitFor();
  await page.getByRole("button", { name: "Subscribe", exact: true }).click();
  await page
    .locator("#messages")
    .getByText(/1\.000000001/)
    .first()
    .waitFor({ state: "attached" });
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  assert.match(await page.locator("#command").inputValue(), /cancel/);
  await page.getByText("Custom JSON command", { exact: true }).click();
  assert.ok(
    await page
      .locator("#command")
      .evaluate((node) => node.getBoundingClientRect().width > 600),
    "Command editor must fill the console",
  );
  await page.locator("#command").fill("{");
  await page.getByRole("button", { name: "Send JSON", exact: true }).click();
  await page.locator("#console-error").waitFor();
  await page
    .locator("#command")
    .fill(JSON.stringify({ type: "subscribe", id: "bad", query: {} }));
  await page.getByRole("button", { name: "Send JSON", exact: true }).click();
  await page
    .locator("#messages")
    .getByText(/invalid_query/)
    .first()
    .waitFor({ state: "attached" });
  await page.locator("#command").fill(
    JSON.stringify({
      type: "request",
      id: "typed-value",
      method: "valuation.evaluate",
      params: {
        gift: { collection_id: "unknown", number: 1 },
        currency_id: "GRAM",
      },
    }),
  );
  await page.getByRole("button", { name: "Send JSON", exact: true }).click();
  await page
    .locator("#messages pre")
    .filter({ hasText: '"code": "not_found"' })
    .filter({ hasText: '"id": "typed-value"' })
    .waitFor({ state: "attached" });
  await page.getByRole("button", { name: "Disconnect", exact: true }).click();
  await page.getByText("Disconnected", { exact: true }).waitFor();
  await page.locator("#protocol-reference summary").click();
  await page
    .locator("#asyncapi-reference")
    .getByText("Gift market WebSocket API", { exact: true })
    .first()
    .waitFor();
  await page
    .locator("#asyncapi-reference")
    .getByText(/Watch a fixed ordered range/)
    .first()
    .waitFor();
  await page
    .locator('[id="ws-message-orders.watch"] td code')
    .filter({ hasText: /^params$/ })
    .waitFor();
  await page
    .locator('[id="ws-message-orders.watch"] a[data-schema]')
    .filter({ hasText: "OrderWatchQuery" })
    .click();
  assert.equal(
    await page
      .locator("#ws-schema-OrderWatchQuery")
      .evaluate((node) => node.open),
    true,
  );
  await page
    .locator("#ws-message-watch_catalog")
    .getByText(/Watch catalog content revisions/)
    .waitFor();
  await page
    .locator("#ws-message-catalog td code")
    .filter({ hasText: /^versions$/ })
    .waitFor();
  for (const [message, field] of [
    ["request", "params"],
    ["result", "data"],
    ["cancel", "id"],
    ["orders.watch", "params"],
    ["orders.snapshot", "data"],
    ["market.status", "status"],
  ]) {
    await page
      .locator(`[id="ws-message-${message}"] td code`)
      .filter({ hasText: new RegExp(`^${field}$`) })
      .first()
      .waitFor();
  }
  await page
    .locator("#ws-message-request td code")
    .filter({ hasText: '"valuation.evaluate"' })
    .waitFor();
  await mkdir(new URL("../test-artifacts", import.meta.url), {
    recursive: true,
  });
  await page.screenshot({
    path: new URL("../test-artifacts/docs-websocket.png", import.meta.url)
      .pathname,
    fullPage: true,
  });
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await page.getByText("Connected", { exact: true }).waitFor();
  await page.goto(`${base}/docs`);
  await page.goBack();
  await page
    .locator("#connection-state")
    .getByText("Disconnected", { exact: true })
    .waitFor();
  await page.setViewportSize({ width: 390, height: 844 });
  assert.ok(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth + 1,
    ),
    "Docs overflow the mobile viewport",
  );
  assert.deepEqual(errors, []);
  assert.deepEqual(violations, []);
  assert.ok(
    requests.every((url) => new URL(url).origin === new URL(base).origin),
    `External docs requests: ${requests.filter((url) => new URL(url).origin !== new URL(base).origin)}`,
  );
  console.log(
    "Developer docs passed: isolated GUI assets, Scalar requests, AsyncAPI schemas, WS commands/errors, cleanup, mobile layout and local-only resources.",
  );
} catch (error) {
  console.error("Docs page:", await page?.locator("body").innerText());
  await mkdir(new URL("../test-artifacts", import.meta.url), {
    recursive: true,
  });
  await page?.screenshot({
    path: new URL("../test-artifacts/docs-failure.png", import.meta.url)
      .pathname,
    fullPage: true,
  });
  throw error;
} finally {
  await browser.close();
}
