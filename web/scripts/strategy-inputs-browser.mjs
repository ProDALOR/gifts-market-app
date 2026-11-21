import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "strategy-inputs-"));
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body><div id="fixture"></div><script type="module">
import React,{useState}from'react';import{createRoot}from'react-dom/client';
import{useOrderValuation}from'/src/valuation/use-order-valuation.ts';import{OrderValuationSummary}from'/src/orderbook/order-valuation.tsx';
const gift=n=>({id:'order-'+n,kind:'ask',book:{collection_id:'collection',currency_id:'GRAM',venue_id:'venue'},price:'12',quantity:1,observed_at:'2026-01-01T00:00:00Z',target:{kind:'gift',id:'gift-'+n,number:n,traits:{}}});
function Details({order}){const state=useOrderValuation(order);return React.createElement('div',null,React.createElement('output',null,state.report?.target.kind==='gift'?state.report.target.gift.number:state.report?.target.kind),React.createElement(OrderValuationSummary,{state}));}
function App(){const[order,select]=useState();window.choose=n=>select(n==='criteria'?{...gift(1),kind:'bid',target:{kind:'criteria',models:[],backdrops:[],patterns:[]}}:n?gift(n):undefined);return order?React.createElement(Details,{order}):null;}
createRoot(document.getElementById('fixture')).render(React.createElement(App));
</script></body></html>`;
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "strategy-inputs-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url !== "/__strategy_inputs") return next();
          res.setHeader("Content-Type", "text/html");
          res.end(await server.transformIndexHtml(req.url, html));
        });
      },
    },
  ],
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  page.setDefaultTimeout(10000);
  await page.clock.install({ time: new Date() });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  let metadataRequests = 0,
    guideRequests = 0,
    failMetadata = false,
    emptyMetadata = false,
    holdDiscovery = false,
    releaseDiscovery;
  const rows = [
    {
      id: "future_selection",
      input: "selection",
      targets: ["gift", "criteria"],
      requires_period: false,
    },
    {
      id: "future_gift",
      input: "resolved_gift",
      targets: ["gift"],
      requires_period: false,
    },
    {
      id: "future_history",
      input: "selection_with_execution_period",
      targets: ["gift"],
      requires_period: true,
    },
  ].map((row) => ({
    ...row,
    name: row.id,
    description: row.id,
    documentation_url: "/guides/" + row.id,
    documentation_revision: "one",
    revision: "r1",
    display: {
      fields: [
        {
          id: "mark",
          label: "Value",
          primary: true,
          format: { kind: "currency", decimals: null },
        },
      ],
      evidence: [],
      note: row.id + " note",
    },
  }));
  await page.route("**/valuations/strategies", async (route) => {
    metadataRequests++;
    if (holdDiscovery)
      await new Promise((resolve) => (releaseDiscovery = resolve));
    return route.fulfill(
      failMetadata
        ? { status: 503, json: { error: { message: "Discovery unavailable" } } }
        : { json: emptyMetadata ? [] : rows },
    );
  });
  await page.route("**/guides/*", (route) => {
    guideRequests++;
    return route.fulfill({
      contentType: "text/markdown",
      body: "# Generic strategy guide",
    });
  });
  const queries = [],
    cancelled = [];
  let held,
    rejectRemoved = false,
    deployAfterDiscovery = false,
    incompatibleDefinition = false;
  await page.routeWebSocket("**/ws", (ws) => {
    ws.send(JSON.stringify({ type: "market.status", status: "available" }));
    ws.onMessage((raw) => {
      const command = JSON.parse(String(raw));
      if (command.type === "cancel") {
        cancelled.push(command.id);
        return;
      }
      if (command.type !== "request") return;
      assert.equal(command.method, "valuation.query");
      queries.push(command);
      if (rejectRemoved) {
        rejectRemoved = false;
        ws.send(
          JSON.stringify({
            type: "error",
            id: command.id,
            error: {
              code: "invalid_query",
              message: "unknown valuation strategy",
            },
          }),
        );
        return;
      }
      if (deployAfterDiscovery) {
        deployAfterDiscovery = false;
        for (const row of rows) row.revision = "r2";
        rows[0].display = {
          ...rows[0].display,
          fields: [
            {
              id: "mark",
              label: "Value",
              primary: true,
              format: { kind: "currency", decimals: 2 },
            },
          ],
        };
      }
      const params = command.params,
        n = params.target.gift?.number;
      const data = {
        target:
          params.target.kind === "gift"
            ? {
                kind: "gift",
                gift: {
                  id: "gift-" + n,
                  collection_id: "collection",
                  number: n,
                  traits: {},
                },
              }
            : params.target,
        currency_id: params.currency_id,
        evaluated_at: new Date().toISOString(),
        stale: false,
        results: [...params.strategies].reverse().map((strategy) => ({
          strategy,
          revision: incompatibleDefinition
            ? "unavailable_revision"
            : rows.find((row) => row.id === strategy).revision,
          data: {
            mark: {
              value: strategy === "future_selection" ? "10" : String(n * 20),
            },
          },
        })),
      };
      if (n === 4) {
        const failed = data.results.find(
          (row) => row.strategy !== "future_selection",
        );
        delete failed.data;
        failed.error = {
          code: "not_enough_data",
          message: "No comparable gifts",
        };
      }
      const send = () =>
        ws.send(
          JSON.stringify({
            type: "result",
            id: command.id,
            method: command.method,
            data,
          }),
        );
      if (n === 2) held = { send, id: command.id };
      else send();
    });
  });
  const url = `http://127.0.0.1:${server.httpServer.address().port}/__strategy_inputs`;
  await page.goto(url);
  assert.equal(metadataRequests, 0);
  await page.evaluate(() => window.choose(1));
  const strategyRows = page.locator(".valuation-strategy-row");
  const selectionRow = page.locator('[data-strategy="future_selection"]');
  const info = () =>
    selectionRow.getByRole("button", {
      name: "How is future_selection calculated?",
      exact: true,
    });
  const refresh = async () => {
    await page.clock.fastForward(60_000);
    await page.waitForTimeout(1);
  };
  await strategyRows.first().waitFor();
  assert.equal(
    await page.getByRole("combobox", { name: "Valuation strategy" }).count(),
    0,
  );
  assert.deepEqual(
    await strategyRows.evaluateAll((rows) =>
      rows.map((row) => row.dataset.strategy),
    ),
    ["future_gift", "future_selection"],
  );
  assert.deepEqual(queries[0].params.strategies, [
    "future_gift",
    "future_selection",
  ]);
  assert.equal(queries[0].params.period, undefined);
  assert.deepEqual(await page.locator(".quote-amount").allTextContents(), [
    "20",
    "10",
  ]);
  assert.equal(queries.length, 1);
  assert.equal(guideRequests, 0);
  await info().click();
  await page
    .getByRole("article", { name: "future_selection calculation guide" })
    .waitFor();
  assert.equal(metadataRequests, 1);
  assert.equal(guideRequests, 1);
  await page.evaluate(() => window.choose("criteria"));
  await page.waitForFunction(
    () => document.querySelector("output")?.textContent === "criteria",
  );
  assert.equal(
    await page.locator(".valuation-popup").count(),
    0,
    "A scope change closes the previous gift's popup",
  );
  await page.evaluate(() => window.choose(1));
  await selectionRow.waitFor();
  const beforePopup = queries.length;
  await info().click();
  await page
    .getByRole("article", { name: "future_selection calculation guide" })
    .waitFor();
  assert.equal(
    queries.length,
    beforePopup,
    "Opening a popup does not recalculate",
  );
  assert.equal(metadataRequests, 1);
  assert.equal(
    guideRequests,
    1,
    "Guides survive gift changes in the shared cache",
  );
  await page.getByRole("button", { name: "Close calculation guide" }).click();
  await page.locator(".valuation-popup").waitFor({ state: "detached" });
  rows[1] = { ...rows[1], id: "future_gift_v2", name: "New gift strategy" };
  rejectRemoved = true;
  await refresh();
  await page
    .getByRole("alert")
    .getByText("unknown valuation strategy", { exact: true })
    .waitFor();
  const metadataBeforeRetry = metadataRequests;
  await refresh();
  await page.locator('[data-strategy="future_gift_v2"]').waitFor();
  assert.equal(
    metadataRequests,
    metadataBeforeRetry + 1,
    "The next minute discovers updated capabilities",
  );
  await page.evaluate(() => window.choose(2));
  for (let i = 0; !held && i < 100; i++)
    await new Promise((resolve) => setTimeout(resolve, 10));
  assert.ok(held);
  await page.evaluate(() => window.choose(3));
  await page.waitForFunction(
    () => document.querySelector("output")?.textContent === "3",
  );
  held.send();
  assert.ok(cancelled.includes(held.id));
  assert.equal(await page.locator("output").textContent(), "3");
  const beforePartial = queries.length;
  await page.evaluate(() => window.choose(4));
  await page.getByText("Not enough data", { exact: true }).waitFor();
  assert.equal(
    await selectionRow.locator(".quote-amount").textContent(),
    "10",
    "One unavailable strategy does not hide another result",
  );
  assert.equal(
    queries.length,
    beforePartial + 1,
    "All strategies share one request",
  );
  for (const width of [320, 390, 1200]) {
    await page.setViewportSize({ width, height: 850 });
    assert.ok(
      await page.locator(".valuation-strategies").evaluate((list) => {
        const rows = [...list.children].map((row) =>
          row.getBoundingClientRect(),
        );
        return rows.every(
          (row, i) =>
            row.right <= innerWidth &&
            row.left >= 0 &&
            (!i || row.top >= rows[i - 1].bottom),
        );
      }),
      `Strategy results form one bounded column at ${width}px`,
    );
  }
  await page.evaluate(() => window.choose("criteria"));
  await page.waitForFunction(
    () => document.querySelector("output")?.textContent === "criteria",
  );
  assert.deepEqual(queries.at(-1).params.strategies, ["future_selection"]);
  assert.equal(await strategyRows.count(), 1);
  assert.equal(metadataRequests, metadataBeforeRetry + 1);
  const before = queries.length;
  failMetadata = true;
  await page.goto(url);
  await page.evaluate(() => window.choose(1));
  await page
    .getByRole("alert")
    .getByText("Discovery unavailable", { exact: true })
    .waitFor();
  assert.equal(queries.length, before);
  failMetadata = false;
  await refresh();
  await selectionRow.waitFor();
  assert.equal(queries.length, before + 1);
  emptyMetadata = true;
  await page.goto(url);
  await page.evaluate(() => window.choose(1));
  await page
    .getByRole("alert")
    .getByText("No compatible valuation strategies are available.", {
      exact: true,
    })
    .waitFor();
  assert.equal(queries.length, before + 1);
  emptyMetadata = false;
  await page.goto(url);
  await page.evaluate(() => window.choose(1));
  await strategyRows.first().waitFor();
  const initial = queries.length;
  assert.equal(
    await page.getByRole("button", { name: "Refresh valuation" }).count(),
    0,
  );
  assert.equal(
    await page.getByRole("button", { name: "Recalculate valuation" }).count(),
    0,
  );
  await page.clock.fastForward(59_999);
  assert.equal(queries.length, initial, "No refresh before a minute");
  await page.clock.fastForward(1);
  for (let i = 0; queries.length === initial && i < 100; i++)
    await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(
    queries.length,
    initial + 1,
    "One batched request at the minute boundary",
  );
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", {
      configurable: true,
      value: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.clock.fastForward(180_000);
  assert.equal(
    queries.length,
    initial + 1,
    "Hidden tabs do not poll valuations",
  );
  await page.evaluate(() => {
    delete document.hidden;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  for (let i = 0; queries.length === initial + 1 && i < 100; i++)
    await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(
    queries.length,
    initial + 2,
    "Visibility catches up once without replaying missed intervals",
  );
  await page.evaluate(() => window.choose(undefined));
  await page.locator(".valuation-summary").waitFor({ state: "detached" });
  await page.clock.fastForward(120_000);
  assert.equal(queries.length, initial + 2, "Unmount clears the refresh timer");
  holdDiscovery = true;
  await page.goto(url);
  await page.evaluate(() => window.choose(6));
  for (let i = 0; !releaseDiscovery && i < 100; i++)
    await new Promise((resolve) => setTimeout(resolve, 10));
  assert.ok(releaseDiscovery);
  const pendingDiscovery = metadataRequests;
  await page.clock.fastForward(180_000);
  assert.equal(
    metadataRequests,
    pendingDiscovery,
    "An in-flight calculation cannot overlap another refresh",
  );
  await page.evaluate(() => window.choose(undefined));
  holdDiscovery = false;
  releaseDiscovery();
  await page.evaluate(() => window.choose(7));
  await page.waitForFunction(
    () => document.querySelector("output")?.textContent === "7",
  );
  assert.equal(
    await page.getByText("Estimate may be outdated", { exact: true }).count(),
    0,
  );
  const beforeDeployment = queries.length;
  const beforeDeploymentMetadata = metadataRequests;
  deployAfterDiscovery = true;
  await page.evaluate(() => window.choose(8));
  await page.waitForFunction(
    () => document.querySelector("output")?.textContent === "8",
  );
  assert.equal(
    queries.length,
    beforeDeployment + 1,
    "Metadata refresh reuses the completed calculation",
  );
  assert.equal(
    metadataRequests,
    beforeDeploymentMetadata + 1,
    "Only one metadata reload after a contract mismatch",
  );
  assert.equal(
    await selectionRow.locator(".quote-amount").textContent(),
    "10.00",
    "New metadata precision renders the same typed value",
  );
  incompatibleDefinition = true;
  const beforeMismatch = queries.length;
  const beforeMismatchMetadata = metadataRequests;
  await page.evaluate(() => window.choose(9));
  await page
    .getByRole("alert")
    .getByText("Valuation strategy definition changed", { exact: true })
    .waitFor();
  assert.equal(queries.length, beforeMismatch + 1);
  assert.equal(
    metadataRequests,
    beforeMismatchMetadata + 1,
    "Persistent deployment mismatch cannot loop",
  );
  assert.equal(
    await page.locator("output").textContent(),
    "",
    "Never render values against mismatched definitions",
  );
  incompatibleDefinition = false;
  await page.evaluate(() => window.choose(undefined));
  assert.deepEqual(errors, []);
  console.log(
    "Strategy input browser passed: capability selection, reversed results, stacked results, guide reuse, cancellation, target changes, discovery recovery and minute refresh/visibility cleanup.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
