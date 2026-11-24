// Isolated UI workflow: no live Telegram sessions or hub connections.
import assert from "node:assert/strict";
import { readFile, mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
import {
  floorReport,
  fixtureDisplay,
  giftReport,
} from "./valuation-fixture.mjs";
const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "market-portfolio-"));
const output = fileURLToPath(new URL("../../.market/run", import.meta.url));
const artwork = await readFile(
  new URL("../src/brand/liberty-figure.svg", import.meta.url),
);
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">
import React from 'react';import{createRoot}from'react-dom/client';import{PortfolioPage}from'/src/portfolio/page.tsx';import'/src/style.css';
createRoot(document.getElementById('root')).render(React.createElement(PortfolioPage,{initialUser:7,context:{initData:''}}));</script></body></html>`;
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "portfolio-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url === "/__portfolio") {
            res.setHeader("content-type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
          } else next();
        });
      },
    },
  ],
});
let browser;
const collection = {
  id: "gift",
  name: "Gift",
  slug: "Gift",
  preview: {
    thumbnail_url: "/media/models/model/thumbnail",
    preview_url: "/media/models/model/preview",
  },
};
const models = [
    { kind: "model", id: "model", name: "Model", media: collection.preview },
  ],
  backdrops = [
    {
      kind: "backdrop",
      id: "back",
      name: "Green",
      palette: { center: "#a6b67f", edge: "#62904b", pattern: "#37552a" },
    },
  ],
  patterns = [];
const item = (number) => ({
  gift: {
    gift: {
      id: String(number),
      slug: `Gift-${number}`,
      number,
      collection: "1",
      model: "2",
      backdrop: 3,
    },
    relation: "owned",
  },
  resolved: {
    id: String(number),
    number,
    collection_id: "gift",
    traits: { model: "model", backdrop: "back" },
  },
  estimate: "2",
});
const state = (user, count, refresh = 1) => ({
  user_id: user,
  refresh_id: refresh,
  refresh_after: new Date(Date.now() - 1000).toISOString(),
  progress: {
    stage: "ready",
    checked: count,
    total: count,
    pages: Math.ceil(count / 100),
    valued: count,
    collectibles: count,
  },
  inventory: {
    checked_at: new Date().toISOString(),
    count,
    scanned: count,
    regular: 0,
    skipped: 0,
    complete: true,
  },
  valuation: {
    currency_id: "GRAM",
    evaluated_at: new Date().toISOString(),
    stale: false,
    total: String(count * 2),
    priced: count,
    unpriced: 0,
    items: Array.from({ length: count }, (_, i) => item(i + 1)),
  },
});
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  await mkdir(output, { recursive: true });
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 390, height: 844 },
  ]) {
    const page = await browser.newPage({ viewport, reducedMotion: "reduce" });
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    page.on("console", (m) => {
      if (m.type() === "error") console.log("console", m.text());
    });
    page.on("requestfailed", (r) =>
      console.log("failed", r.url(), r.failure()),
    );
    page.on("response", (r) => {
      if (r.status() >= 400) console.log("http", r.status(), r.url());
    });
    await page.route(
      (url) => url.pathname === "/telegram",
      (r) => r.fulfill({ json: { auth_enabled: false } }),
    );
    await page.route(
      (url) => url.pathname.startsWith("/media/"),
      (r) =>
        r.request().url().endsWith("/animation")
          ? r.fulfill({ status: 404 })
          : r.fulfill({ contentType: "image/svg+xml", body: artwork }),
    );
    let catalogDetails = 0;
    await page.route(
      (url) => url.pathname === "/catalog",
      (r) =>
        r.fulfill({
          headers: { etag: '"directory"' },
          json: {
            collections: [
              collection,
              { ...collection, id: "other", name: "Other Gift" },
            ],
            currencies: [
              { id: "GRAM", name: "GRAM", decimals: 9 },
              { id: "XTR", name: "Stars", decimals: 0 },
            ],
            venues: [],
            books: [],
          },
        }),
    );
    await page.route(
      (url) => url.pathname.startsWith("/collections/"),
      (r) => {
        catalogDetails++;
        return r.fulfill({
          headers: { etag: '"details"' },
          json: r.request().url().endsWith("/other")
            ? {
                collection: { ...collection, id: "other", name: "Other Gift" },
                models: [
                  { ...models[0], id: "other-model", name: "Other Model" },
                ],
                backdrops: [
                  {
                    ...backdrops[0],
                    id: "other-back",
                    name: "Blue",
                    palette: {
                      center: "#123456",
                      edge: "#234567",
                      pattern: "#345678",
                    },
                  },
                ],
                patterns: [],
              }
            : { collection, models, backdrops, patterns },
        });
      },
    );
    await page.route("**/valuations/strategies", (r) =>
      r.fulfill({
        json: [
          {
            id: "gift_floor",
            revision: "r1",
            purpose: "estimate",
            name: "Gift floor",
            description: "Estimate",
            input: {
              targets: ["gift", "gift_attributes"],
              period: "unsupported",
            },
            display: fixtureDisplay("gift_floor"),
            documentation_url: "/guide",
            documentation_revision: "r1",
          },
        ],
      }),
    );
    let report,
      peer,
      watchId,
      watchCount = 0,
      refreshCount = 0,
      cancellations = 0;
    const send = (socket, value) => socket.send(JSON.stringify(value));
    await page.routeWebSocket("**/ws", (socket) => {
      send(socket, { type: "market.status", status: "available" });
      socket.onMessage((raw) => {
        const frame = JSON.parse(String(raw));
        if (frame.type === "cancel" && frame.id === watchId) cancellations++;
        if (frame.type === "catalog.watch")
          send(socket, {
            type: "catalog.versions",
            id: frame.id,
            versions: {
              directory: "directory",
              collections: Object.fromEntries(
                frame.collection_ids.map((id) => [id, "details"]),
              ),
            },
          });
        if (frame.type === "portfolio.watch") {
          peer = socket;
          watchId = frame.id;
          watchCount++;
          send(socket, {
            type: "portfolio.snapshot",
            id: frame.id,
            data:
              report?.user_id === frame.params.user_id
                ? report
                : {
                    user_id: frame.params.user_id,
                    refresh_id: 1,
                    progress: {
                      stage: "queued",
                      checked: 0,
                      pages: 0,
                      valued: 0,
                      collectibles: 0,
                    },
                  },
          });
        }
        if (frame.type === "request" && frame.method === "portfolio.refresh") {
          refreshCount++;
          const pending = {
            ...report,
            refresh_id: 2,
            progress: {
              ...report.progress,
              stage: "loading_inventory",
              checked: 100,
              pages: 1,
            },
          };
          send(socket, {
            type: "portfolio.progress",
            id: watchId,
            refresh_id: 2,
            progress: pending.progress,
          });
          setTimeout(() => {
            report = state(7, 701, 2);
            send(socket, {
              type: "portfolio.snapshot",
              id: watchId,
              data: report,
            });
            setTimeout(
              () =>
                send(socket, {
                  type: "result",
                  id: frame.id,
                  method: frame.method,
                  data: pending,
                }),
              20,
            );
          }, 100);
        }
        if (
          (frame.type === "request" && frame.method === "valuation.query") ||
          frame.type === "valuation.watch"
        ) {
          const params = frame.params;
          const data =
            params.target.kind === "catalog"
              ? floorReport(params, ["model"], ["back"], [])
              : giftReport(params, {
                  strategy: "gift_floor",
                  currency_id: params.currency_id,
                  stale: false,
                  evaluated_at: new Date().toISOString(),
                  gift: {
                    id: "1",
                    number: 1,
                    collection_id: "gift",
                    traits: { model: "model", backdrop: "back" },
                  },
                  final_mark: { value: "2" },
                  collection_floor: { value: "1", count: 700 },
                  model_floor: { value: "2", count: 700 },
                  backdrop_floor: { value: "1", count: 700 },
                  model_backdrop_floor: { value: "2", count: 700 },
                  additive_estimate: { value: "2" },
                  basis: "pair_capped",
                  model_multiplier: { value: "2" },
                  backdrop_multiplier: { value: "1" },
                });
          send(
            socket,
            frame.type === "request"
              ? { type: "result", method: frame.method, id: frame.id, data }
              : { type: "valuation.snapshot", id: frame.id, data },
          );
        }
      });
    });
    await page.goto(server.resolvedUrls.local[0] + "__portfolio");
    await page
      .locator(".portfolio-progress")
      .filter({ hasText: "Waiting to check gifts" })
      .waitFor({ timeout: 10000 })
      .catch(async (error) => {
        console.log({ errors, watchCount, body: await page.content() });
        throw error;
      });
    assert.equal(watchCount, 1);
    assert.equal(catalogDetails, 0);
    send(peer, {
      type: "portfolio.progress",
      id: watchId,
      refresh_id: 1,
      progress: {
        stage: "loading_inventory",
        checked: 100,
        total: 700,
        pages: 1,
        valued: 0,
        collectibles: 0,
      },
    });
    await page
      .locator(".portfolio-progress")
      .filter({ hasText: "100 of 700" })
      .waitFor();
    report = state(7, 700);
    send(peer, { type: "portfolio.snapshot", id: watchId, data: report });
    await page.locator('.gift-grid[data-order-count="700"]').waitFor();
    assert((await page.locator(".portfolio-card").count()) < 150);
    await page.getByLabel("Gift #", { exact: true }).fill("10");
    await page.locator('.gift-grid[data-order-count="1"]').waitFor();
    assert.equal(watchCount, 1);
    assert.equal(refreshCount, 0);
    assert.equal(
      (await page.locator(".portfolio-total").innerText()).trim(),
      "2",
    );
    assert.match(
      await page.locator(".portfolio-meta").innerText(),
      /1 \/ 700 gifts/,
    );
    assert.match(
      await page.locator(".portfolio-full-total").innerText(),
      /1,400/,
    );
    await page.getByLabel("Gift #", { exact: true }).fill("");
    await page.getByRole("button", { name: "Gift", exact: true }).click();
    await page.locator("#pick-models").waitFor();
    assert(catalogDetails >= 1);
    await page
      .getByRole("button", { name: "Refresh gifts", exact: true })
      .click();
    await page.locator('.gift-grid[data-order-count="701"]').waitFor();
    await page.waitForTimeout(80);
    assert.equal(
      await page.locator(".portfolio-progress").innerText(),
      "Inventory checked",
    );
    assert.equal(refreshCount, 1);
    const oldWatch = watchId;
    await page.evaluate(() => {
      Object.defineProperty(document, "hidden", {
        configurable: true,
        get: () => window.testHidden,
      });
      window.testHidden = true;
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await page.waitForTimeout(100);
    assert(cancellations > 0, "hidden tabs release portfolio watches");
    await page.evaluate(() => {
      window.testHidden = false;
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await page.waitForTimeout(150);
    assert.equal(watchCount, 2, "visible tabs restore one cached watch");
    send(peer, {
      type: "portfolio.progress",
      id: oldWatch,
      refresh_id: 0,
      progress: { ...report.progress, stage: "failed" },
    });
    await page.waitForTimeout(30);
    assert.equal(
      await page.locator(".portfolio-progress").innerText(),
      "Inventory checked",
    );
    assert.equal(refreshCount, 1, "resume does not recheck Telegram");
    await page.screenshot({
      path: join(output, `portfolio-${viewport.width}.png`),
      fullPage: false,
    });
    await page
      .getByRole("button", { name: "View Gift-1", exact: true })
      .click();
    await page.getByRole("dialog").waitFor();
    await page
      .getByRole("button", { name: "Close details", exact: true })
      .click();
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    // All gifts must use the same resolved artwork and model labels as market asks.
    await page.getByRole("button", { name: /^All gifts/ }).click();
    report = state(7, 6000, 3);
    report.valuation.items[1] = {
      ...item(2),
      gift: {
        ...item(2).gift,
        gift: { ...item(2).gift.gift, slug: "OtherGift-2", collection: "2" },
      },
      resolved: {
        ...item(2).resolved,
        collection_id: "other",
        traits: { model: "other-model", backdrop: "other-back" },
      },
    };
    send(peer, { type: "portfolio.snapshot", id: watchId, data: report });
    await page.locator('.gift-grid[data-order-count="6000"]').waitFor();
    const other = page
      .locator(".portfolio-card")
      .filter({
        has: page.getByRole("button", {
          name: "View OtherGift-2",
          exact: true,
        }),
      });
    await other.getByText("Other Model", { exact: true }).waitFor();
    assert(
      (await other.locator(".gift-backdrop").count()) > 0,
      "mixed collection card has its backdrop",
    );
    const reads = catalogDetails;
    await page
      .getByRole("button", { name: "View OtherGift-2", exact: true })
      .click();
    await page
      .getByRole("dialog")
      .getByText("Other Model", { exact: true })
      .first()
      .waitFor();
    assert.equal(
      catalogDetails,
      reads,
      "opening details reuses visible card metadata",
    );
    assert.equal(
      await page
        .getByRole("dialog")
        .getByText(/Unavailable (model|backdrop|symbol)/)
        .count(),
      0,
    );
    await page
      .getByRole("button", { name: "Close details", exact: true })
      .click();
    assert((await page.locator(".portfolio-card").count()) < 150);
    await page.evaluate(() =>
      window.scrollTo(0, document.documentElement.scrollHeight),
    );
    await page.waitForTimeout(100);
    assert(
      (await page
        .getByRole("button", { name: "View Gift-6000", exact: true })
        .count()) > 0,
    );
    const bottomReads = catalogDetails;
    send(peer, {
      type: "portfolio.progress",
      id: watchId,
      refresh_id: 3,
      progress: { ...report.progress, stage: "valuing", valued: 25 },
    });
    await page.waitForTimeout(50);
    assert.equal(
      catalogDetails,
      bottomReads,
      "progress does not refetch card catalogs",
    );
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.locator(".portfolio-owner-trigger").click();
    await page.getByLabel("Telegram user ID").fill("8");
    await page.getByRole("button", { name: "Open", exact: true }).click();
    await page.waitForFunction(() =>
      document.querySelector("h1")?.textContent?.includes("8"),
    );
    await page.waitForTimeout(50);
    assert.equal(await page.locator(".portfolio-card").count(), 0);
    assert.deepEqual(errors, []);
    await page.close();
  }
  console.log(
    "Portfolio browser checks passed: desktop/mobile progress, filters, virtualization, refresh races, details and subject switching.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
