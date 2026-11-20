import assert from "node:assert/strict";
import { mkdtemp, rm, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "target-values-"));
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body><div id="fixture"></div><script type="module">
import React from 'react';import{createRoot}from'react-dom/client';import{TargetValuations}from'/src/valuation/target-valuations.tsx';
const options=new Map([['gold',{kind:'model',id:'gold',name:'Gold'}],['blue',{kind:'backdrop',id:'blue',name:'Blue',palette:{center:'#446688',edge:'#223344',pattern:'#112233'}}]]);
createRoot(document.getElementById('fixture')).render(React.createElement(TargetValuations,{collection:{id:'gift',name:'Gift',slug:'Gift'},currency:'GRAM',options,filters:{models:['gold']}}));
</script></body></html>`;
const strategies = [
  ["ask_floor", "Ask floor", false],
  ["execution_mean", "Execution mean", true],
  ["execution_median", "Execution median", true],
].map(([id, name, requires_period]) => ({
  id,
  name,
  requires_period,
  description: `Calculation for ${name}`,
  targets: ["collection", "model", "backdrop", "model_backdrop", "gift"],
  documentation_url: "/guide/" + id,
  documentation_revision: "1",
}));
let metadataRequests = 0;
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "target-values-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url === "/__targets") {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
            return;
          }
          if (req.url === "/valuations/strategies") {
            metadataRequests++;
            res.setHeader("Content-Type", "application/json");
            res.end(JSON.stringify(strategies));
            return;
          }
          next();
        });
      },
    },
  ],
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({
    viewport: { width: 1200, height: 860 },
    reducedMotion: "reduce",
  });
  page.setDefaultTimeout(10000);
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const requests = [],
    cancelled = [];
  let socket;
  await page.routeWebSocket("**/ws", (ws) => {
    socket = ws;
    ws.send(JSON.stringify({ type: "market.status", status: "available" }));
    ws.onMessage((raw) => {
      const command = JSON.parse(String(raw));
      if (command.type === "cancel") {
        cancelled.push(command.id);
        return;
      }
      if (command.type !== "request") return;
      assert.equal(command.method, "valuation.query");
      requests.push(command);
      if (requests.length !== 1) return;
      const results = command.params.strategies.map((strategy, index) => ({
        strategy,
        data: {
          price:
            index === 2
              ? {
                  error: {
                    code: "not_enough_data",
                    message: "Three gifts required",
                  },
                }
              : { value: index === 0 ? "1.25" : "20" },
        },
        display: {
          fields: [
            {
              id: "price",
              label: "Value",
              primary: true,
              value:
                index === 2
                  ? {
                      kind: "unavailable",
                      label: "Not enough data",
                      message: "Three gifts required",
                      detail: "1 / 3 required samples",
                    }
                  : {
                      kind: "currency",
                      amount: index === 0 ? "1.25" : "20.00",
                      currency_id: "GRAM",
                    },
            },
          ],
          summary: "3 gifts",
          note: "Observed evidence only.",
        },
      }));
      ws.send(
        JSON.stringify({
          type: "result",
          id: command.id,
          method: "valuation.query",
          data: {
            target: command.params.target,
            currency_id: "GRAM",
            evaluated_at: new Date().toISOString(),
            stale: false,
            results,
          },
        }),
      );
    });
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__targets`,
  );
  assert.equal(metadataRequests, 0);
  assert.equal(requests.length, 0);
  await page.getByRole("button", { name: "Collection & trait values" }).click();
  const dialog = page.getByRole("dialog", {
    name: "Collection and trait values",
  });
  await dialog.getByText("Execution median", { exact: true }).waitFor();
  assert.equal(metadataRequests, 1);
  assert.equal(requests.length, 0);
  assert.equal(
    await dialog.getByLabel("Value", { exact: true }).inputValue(),
    "model",
  );
  await dialog.getByRole("button", { name: "Calculate values" }).click();
  await dialog.locator(".target-results article").last().waitFor();
  assert.equal(requests.length, 1);
  assert.deepEqual(requests[0].params.target, {
    kind: "model",
    collection_id: "gift",
    model_id: "gold",
  });
  assert.ok(requests[0].params.period.from);
  assert.ok(requests[0].params.period.until);
  await dialog.getByText("Not enough data", { exact: true }).waitFor();
  assert.equal(await dialog.locator(".target-results article").count(), 3);
  socket.send(JSON.stringify({ type: "market.status", status: "delayed" }));
  await dialog
    .getByText(
      "Market updates changed or are delayed; recalculate to refresh.",
      { exact: false },
    )
    .waitFor();
  assert.equal(requests.length, 1);
  await mkdir(join(root, "..", ".market", "run"), { recursive: true });
  await page.screenshot({
    path: join(root, "..", ".market", "run", "target-valuations-desktop.png"),
  });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.ok(await dialog.evaluate((el) => el.scrollWidth <= el.clientWidth));
  await page.screenshot({
    path: join(root, "..", ".market", "run", "target-valuations-mobile.png"),
  });
  await dialog.getByLabel("Value", { exact: true }).selectOption("backdrop");
  await dialog.getByLabel("Backdrop", { exact: true }).selectOption("blue");
  assert.equal(await dialog.locator(".target-results").count(), 0);
  await dialog.getByRole("button", { name: "Calculate values" }).click();
  await page.waitForFunction(
    () =>
      document.querySelector(".target-values button.primary")?.textContent ===
      "Calculating…",
  );
  await page
    .getByRole("button", { name: "Close valuations", exact: true })
    .click();
  await dialog.waitFor({ state: "detached" });
  for (let i = 0; i < 30 && !cancelled.length; i++)
    await page.waitForTimeout(10);
  assert.equal(requests.length, 2);
  assert.ok(
    cancelled.includes(requests[1].id),
    "Closing aborts the in-flight target report",
  );
  await page.getByRole("button", { name: "Collection & trait values" }).click();
  await dialog.getByText("Execution median", { exact: true }).waitFor();
  assert.equal(metadataRequests, 1, "Reopening reuses strategy metadata");
  assert.equal(requests.length, 2);
  assert.equal(await dialog.locator(".target-results").count(), 0);
  assert.deepEqual(errors, []);
  console.log(
    "Target valuation browser passed: intent-only requests, discovery, typed scopes, explicit periods, independent results, stale status, responsive layout, cancellation and metadata reuse.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
