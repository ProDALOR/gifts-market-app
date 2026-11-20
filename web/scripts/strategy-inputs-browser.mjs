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
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  let metadataRequests = 0,
    guideRequests = 0,
    failMetadata = false,
    emptyMetadata = false;
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
  }));
  await page.route("**/valuations/strategies", (route) => {
    metadataRequests++;
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
    rejectRemoved = false;
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
      if (rejectRemoved && command.params.strategies.includes("future_gift")) {
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
        results: [...params.strategies]
          .reverse()
          .map((strategy) => ({
            strategy,
            data: {},
            display: {
              fields: [
                {
                  id: "mark",
                  label: "Value",
                  primary: true,
                  value: {
                    kind: "currency",
                    amount:
                      strategy === "future_selection" ? "10" : String(n * 20),
                    currency_id: params.currency_id,
                  },
                },
              ],
              note: strategy + " note",
            },
          })),
      };
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
  const picker = page.getByRole("combobox", { name: "Valuation strategy" });
  await picker.waitFor();
  assert.equal(await picker.inputValue(), "future_gift");
  assert.deepEqual(queries[0].params.strategies, [
    "future_gift",
    "future_selection",
  ]);
  assert.equal(queries[0].params.period, undefined);
  assert.equal(await page.locator(".quote-amount").textContent(), "20");
  await picker.selectOption("future_selection");
  assert.equal(await page.locator(".quote-amount").textContent(), "10");
  assert.equal(queries.length, 1);
  assert.equal(guideRequests, 0);
  await page.getByRole("button", { name: "How is this calculated?" }).click();
  await page
    .getByRole("article", { name: "future_selection calculation guide" })
    .waitFor();
  assert.equal(metadataRequests, 1);
  assert.equal(guideRequests, 1);
  await page.getByRole("button", { name: "Close calculation guide" }).click();
  rows[1] = { ...rows[1], id: "future_gift_v2", name: "New gift strategy" };
  rejectRemoved = true;
  await page.getByRole("button", { name: "Refresh valuation" }).click();
  await page
    .getByRole("alert")
    .getByText("unknown valuation strategy", { exact: true })
    .waitFor();
  assert.equal(metadataRequests, 1);
  await page.getByRole("button", { name: "Refresh valuation" }).click();
  await picker
    .locator('option[value="future_gift_v2"]')
    .waitFor({ state: "attached" });
  assert.equal(
    metadataRequests,
    2,
    "Retry after a deployment refreshes obsolete capabilities",
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
  await page.evaluate(() => window.choose("criteria"));
  await page.waitForFunction(
    () => document.querySelector("output")?.textContent === "criteria",
  );
  assert.deepEqual(queries.at(-1).params.strategies, ["future_selection"]);
  assert.equal(await picker.count(), 0);
  assert.equal(metadataRequests, 2);
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
  await page.getByRole("button", { name: "Refresh valuation" }).click();
  await picker.waitFor();
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
  assert.deepEqual(errors, []);
  console.log(
    "Strategy input browser passed: capability selection, reversed results, cached switching, guide reuse, cancellation, target changes and discovery failure recovery.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
