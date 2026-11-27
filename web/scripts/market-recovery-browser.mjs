// Real market page with controlled catalog latency and transient valuation failures.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
import { RangeWatch } from "./range-fixture.mjs";
import { floorReport } from "./valuation-fixture.mjs";
const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "market-recovery-"));
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">
import React from 'react';import{createRoot}from'react-dom/client';import{AppShell}from'/src/app/shell.tsx';import{MarketPage}from'/src/market-page.tsx';import'/src/style.css';
const context={};createRoot(document.getElementById('root')).render(React.createElement(AppShell,{context,market:React.createElement(MarketPage,{context})}));</script></body></html>`;
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "market-recovery",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url.split("?")[0] !== "/") return next();
          res.setHeader("Content-Type", "text/html");
          res.end(await server.transformIndexHtml(req.url, html));
        });
      },
    },
  ],
});
const collections = [
  { id: "bag", name: "Swag Bag", slug: "SwagBag", preview: null },
  { id: "hat", name: "Witch Hat", slug: "WitchHat", preview: null },
];
const backdrop = (id, name) => ({
  id,
  name,
  palette: { center: "#ff3333", edge: "#992222", pattern: "#661111" },
});
const details = (id) => ({
  collection: collections.find((c) => c.id === id),
  models: [{ id: id + ".model", name: "Model", media: null }],
  backdrops: [
    backdrop(id + ".black", "Black"),
    ...(id === "bag" ? [backdrop("carmine", "Carmine")] : []),
  ],
  patterns: [{ id: id + ".sun", name: "Sun", media: null }],
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  for (const width of [390, 1440]) {
    const page = await browser.newPage({
      viewport: { width, height: 900 },
      reducedMotion: "reduce",
    });
    const errors = [],
      watches = [],
      valuationCalls = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.route("**/telegram", (r) =>
      r.fulfill({
        json: { auth_enabled: false, bot_id: null, bot_username: null },
      }),
    );
    await page.route("**/media/**", (r) => r.fulfill({ status: 404 }));
    await page.route("**/catalog", (r) =>
      r.fulfill({
        headers: { etag: '"dir"' },
        json: {
          collections,
          venues: [{ id: "venue", name: "MRKT" }],
          currencies: [{ id: "GRAM", name: "GRAM", decimals: 9 }],
          books: collections.map((c) => ({
            collection_id: c.id,
            currency_id: "GRAM",
            venue_id: "venue",
          })),
        },
      }),
    );
    const pending = new Map();
    const release = async (id) => {
      for (let i = 0; i < 200 && !pending.has(id); i++)
        await page.waitForTimeout(10);
      const deliver = pending.get(id);
      assert.ok(deliver, `Missing catalog request for ${id}`);
      pending.delete(id);
      await deliver();
    };
    await page.route(
      (url) => url.pathname.startsWith("/collections/"),
      (r) =>
        new Promise((resolve) => {
          const id = new URL(r.request().url()).pathname.split("/").at(-1);
          pending.set(id, async () => {
            await r.fulfill({
              headers: { etag: '"detail"' },
              json: details(id),
            });
            resolve();
          });
        }),
    );
    let failFloors = true;
    await page.routeWebSocket("**/ws", (socket) => {
      const send = (value) => socket.send(JSON.stringify(value));
      send({ type: "market.status", status: "available" });
      socket.onMessage((raw) => {
        const frame = JSON.parse(String(raw));
        if (frame.type === "catalog.watch")
          send({
            type: "catalog.versions",
            id: frame.id,
            versions: {
              directory: "dir",
              collections: Object.fromEntries(
                frame.collection_ids.map((id) => [id, "detail"]),
              ),
            },
          });
        if (frame.type === "orders.watch") {
          watches.push(frame.params);
          send(new RangeWatch(frame.params, []).frame(frame.id, []));
        }
        if (frame.type === "request" && frame.method === "valuation.query") {
          valuationCalls.push(frame.params);
          if (failFloors)
            send({
              type: "error",
              id: frame.id,
              error: { code: "timed_out", message: "Temporarily busy" },
            });
          else
            send({
              type: "result",
              id: frame.id,
              method: frame.method,
              data: floorReport(frame.params),
            });
        }
      });
    });
    await page.goto(
      server.resolvedUrls.local[0] +
        "?f=" +
        encodeURIComponent("c=bag|b=@name:Carmine,@name:Black|s=@name:Sun"),
      { waitUntil: "domcontentloaded" },
    );
    await page.getByRole("button", { name: "Swag Bag", exact: true }).waitFor();
    await page.waitForTimeout(150);
    assert.equal(
      watches.length,
      0,
      "No query before the selected catalog arrives",
    );
    assert.match(
      new URL(page.url()).searchParams.get("f"),
      /Carmine/,
      "Loading must not clear carried filters",
    );
    await release("bag");
    await page.waitForFunction(() =>
      document.querySelector("#pick-backdrops")?.textContent.includes("2"),
    );
    await page.waitForTimeout(100);
    assert.equal(watches.length, 1);
    assert.deepEqual(watches[0].filters.backdrops, [
      "@name:Carmine",
      "@name:Black",
    ]);
    await page
      .locator(".floors-error")
      .filter({ hasText: "Updating floors" })
      .waitFor();
    failFloors = false;
    await page.waitForFunction(
      () => !document.querySelector(".floors-error"),
      {},
      { timeout: 10000 },
    );
    assert(
      valuationCalls.length >= 3,
      "Failed floors recovered without a user reload",
    );
    const before = watches.length;
    await page.getByRole("button", { name: "Witch Hat", exact: true }).click();
    await page.waitForTimeout(150);
    assert.equal(
      watches.length,
      before,
      "Switch waits for that collection's own catalog",
    );
    await release("hat");
    await page.waitForTimeout(150);
    assert.equal(
      watches.length,
      before + 1,
      "Only the reconciled query is subscribed",
    );
    const latest = watches.at(-1).filters;
    assert.equal(latest.collection_id, "hat");
    assert.deepEqual(latest.backdrops, ["@name:Black"]);
    assert.deepEqual(latest.patterns, ["@name:Sun"]);
    assert(!new URL(page.url()).searchParams.get("f").includes("Carmine"));
    assert.equal(await page.getByRole("alert").count(), 0);
    await page.getByRole("button", { name: "Swag Bag", exact: true }).click();
    await page.waitForTimeout(150);
    assert(
      !watches.at(-1).filters.backdrops.includes("@name:Carmine"),
      "Dropped filters do not reappear when returning",
    );
    const request = page.waitForRequest("**/collections/hat");
    await page.goto(
      server.resolvedUrls.local[0] +
        "?f=" +
        encodeURIComponent("c=hat|b=@name:Carmine"),
      { waitUntil: "domcontentloaded" },
    );
    await request;
    await release("hat");
    await page.waitForFunction(
      () => !new URL(location.href).searchParams.get("f")?.includes("Carmine"),
    );
    assert.deepEqual(
      watches.at(-1).filters.backdrops,
      [],
      "An obsolete direct link also recovers without reload",
    );
    assert.deepEqual(errors, []);
    await page.close();
  }
  console.log(
    "Market recovery passed: mobile/desktop, delayed catalog, carried backdrop/symbol aliases, no invalid subscriptions, URL cleanup, cached switching and automatic floor retries.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
