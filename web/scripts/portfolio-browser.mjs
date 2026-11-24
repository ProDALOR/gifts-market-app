import { portfolioSnapshot, portfolioPage } from "./portfolio-fixture.mjs";
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
  patterns = [
    {
      kind: "pattern",
      id: "symbol",
      name: "Star",
      media: { thumbnail_url: "/media/patterns/symbol/thumbnail" },
    },
  ];
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
    relation: number === 2 ? "unverified" : number === 3 ? "hosted" : "owned",
  },
  resolved: {
    id: String(number),
    number,
    collection_id: "gift",
    traits: { model: "model", backdrop: "back", pattern: "symbol" },
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
    const page = await browser.newPage({
      viewport,
      reducedMotion: "reduce",
      ...(viewport.width < 960 ? { isMobile: true, hasTouch: true } : {}),
    });
    const symbol = await page.evaluate(() => {
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 96;
      const context = canvas.getContext("2d");
      context.beginPath();
      for (let i = 0; i < 10; i++) {
        const angle = (i * Math.PI) / 5 - Math.PI / 2;
        const radius = i % 2 ? 20 : 44;
        context.lineTo(
          48 + Math.cos(angle) * radius,
          48 + Math.sin(angle) * radius,
        );
      }
      context.closePath();
      context.fill();
      return canvas.toDataURL().split(",")[1];
    });
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
          : r.fulfill({
              contentType: r.request().url().includes("/patterns/")
                ? "image/png"
                : "image/svg+xml",
              body: r.request().url().includes("/patterns/")
                ? Buffer.from(symbol, "base64")
                : artwork,
            }),
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
      cancellations = 0,
      holdRefresh = false,
      holdSummary = false,
      releaseSummary,
      pendingRefreshId;
    const cancelledRequests = new Set();
    const pageReads = [];
    const send = (socket, value) => socket.send(JSON.stringify(value));
    await page.routeWebSocket("**/ws", (socket) => {
      send(socket, { type: "market.status", status: "available" });
      socket.onMessage((raw) => {
        const frame = JSON.parse(String(raw));
        if (frame.type === "cancel" && frame.id === watchId) cancellations++;
        if (frame.type === "cancel") cancelledRequests.add(frame.id);
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
          const snapshot = () =>
            send(socket, {
              type: "portfolio.snapshot",
              id: frame.id,
              data:
                report?.user_id === frame.params.user_id
                  ? portfolioSnapshot(report)
                  : {
                      user_id: frame.params.user_id,
                      refresh_id: 1,
                      progress: {
                        stage: "queued",
                        checked: 0,

                        valued: 0,
                        collectibles: 0,
                      },
                    },
            });
          if (holdSummary) releaseSummary = snapshot;
          else snapshot();
        }
        if (frame.type === "request" && frame.method === "portfolio.query") {
          pageReads.push(frame.params);
          try {
            send(socket, {
              type: "result",
              id: frame.id,
              method: frame.method,
              data: portfolioPage(report, frame.params),
            });
          } catch (error) {
            send(socket, { type: "error", id: frame.id, error });
          }
        }
        if (frame.type === "request" && frame.method === "portfolio.refresh") {
          refreshCount++;
          if (holdRefresh) {
            pendingRefreshId = frame.id;
            return;
          }
          const pending = {
            ...report,
            refresh_id: 2,
            progress: {
              ...report.progress,
              stage: "loading_inventory",
              checked: 100,
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
              data: portfolioSnapshot(report),
            });
            setTimeout(
              () =>
                send(socket, {
                  type: "result",
                  id: frame.id,
                  method: frame.method,
                  data: portfolioSnapshot(pending),
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
    assert.equal(
      await page.getByLabel("Estimate from", { exact: true }).count(),
      0,
    );
    assert.equal(
      await page.getByLabel("Estimate to", { exact: true }).count(),
      0,
    );

    send(peer, {
      type: "portfolio.progress",
      id: watchId,
      refresh_id: 1,
      progress: {
        stage: "loading_inventory",
        checked: 100,
        total: 700,

        valued: 0,
        collectibles: 0,
      },
    });
    await page
      .locator(".portfolio-progress")
      .filter({ hasText: "100 of 700" })
      .waitFor();
    report = state(7, 700);
    send(peer, {
      type: "portfolio.snapshot",
      id: watchId,
      data: portfolioSnapshot(report),
    });
    await page.locator('.gift-grid[data-order-count="100"]').waitFor();
    assert((await page.locator(".portfolio-card").count()) < 150);
    assert.equal(
      pageReads.length,
      1,
      "first load requests only one 100-gift page",
    );
    assert.equal(pageReads[0].limit, 100);
    await page
      .locator(".portfolio-meta")
      .filter({ hasText: "1 unverified" })
      .waitFor();
    assert.equal(
      (await page.locator(".portfolio-total").innerText()).trim(),
      "1,400",
      "visible unverified gifts contribute to the same total",
    );
    const unverified = page.locator(".portfolio-card").filter({
      has: page.getByRole("button", { name: "View Gift-2", exact: true }),
    });
    await unverified
      .getByLabel("Visible on this profile; ownership unverified.", {
        exact: true,
      })
      .waitFor();

    await page.getByLabel("Gift #", { exact: true }).fill("2");
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
      .locator(".portfolio-card .gift-model-image.loaded")
      .first()
      .waitFor();
    const composition = await page
      .locator(".portfolio-card")
      .first()
      .evaluate((card) => {
        const image = card
          .querySelector(".gift-model-image.loaded")
          .getBoundingClientRect();
        const copy = card
          .querySelector(".card-copy strong")
          .getBoundingClientRect();
        return {
          width: image.width,
          height: image.height,
          bottom: image.bottom,
          labelTop: copy.top,
          symbols: !!card.querySelector(".gift-backdrop"),
        };
      });
    assert.equal(composition.width, viewport.width < 960 ? 64 : 112);
    assert.equal(composition.width, composition.height);
    assert(
      composition.bottom <= composition.labelTop,
      "artwork leaves the model name readable",
    );
    assert(composition.symbols);
    await page
      .locator(".portfolio-card")
      .first()
      .evaluate((card) => {
        const label = card.querySelector(".card-copy strong");
        const original = label.textContent;
        label.textContent = "Very Long Collectible Model";
        const image = card
          .querySelector(".gift-model-image.loaded")
          .getBoundingClientRect();
        const text = label.getBoundingClientRect();
        label.textContent = original;
        if (image.bottom > text.top)
          throw new Error("two-line model labels overlap artwork");
      });
    assert.equal(
      await page.locator(".portfolio-relation").first().innerText(),
      "",
      "ownership does not crowd the artwork with a text badge",
    );
    await page.locator("#pick-models").click();
    await page
      .locator("#trait-panel")
      .getByRole("button", { name: "Model", exact: true })
      .click();
    await page
      .getByRole("figure", { name: "Virtual gift preview", exact: true })
      .waitFor();
    await page
      .getByRole("button", { name: "Reset traits", exact: true })
      .click();
    assert.equal(
      await page
        .getByRole("figure", { name: "Virtual gift preview", exact: true })
        .count(),
      0,
    );
    const numberBounds = await page
      .getByLabel("Gift #", { exact: true })
      .boundingBox();
    const sortBounds = await page.locator(".portfolio-sort").boundingBox();
    if (viewport.width < 960) {
      assert(
        Math.abs(numberBounds.y - sortBounds.y) < 1,
        "mobile gift number and sort share a row",
      );
      assert.equal(
        sortBounds.height,
        44,
        "mobile sorting has a comfortable touch target",
      );
      assert(numberBounds.x + numberBounds.width < sortBounds.x);
    }
    await page.getByRole("combobox", { name: "Sort by", exact: true }).click();
    const menuBounds = await page
      .getByRole("listbox", { name: "Sort by options", exact: true })
      .boundingBox();
    assert(
      menuBounds.x >= 0 && menuBounds.x + menuBounds.width <= viewport.width,
      "sort menu stays inside the viewport",
    );
    await page.screenshot({
      path: join(output, `portfolio-sort-${viewport.width}.png`),
      fullPage: false,
    });
    await page.getByRole("option", { name: "Position", exact: true }).click();
    assert.equal(
      (
        await page
          .getByRole("combobox", { name: "Sort by", exact: true })
          .innerText()
      ).trim(),
      "Position",
    );
    await page
      .getByRole("button", { name: "Sort descending", exact: true })
      .click();
    await page
      .getByRole("button", { name: "View Gift-700", exact: true })
      .waitFor();
    await page
      .getByRole("button", { name: "Sort ascending", exact: true })
      .click();
    await page
      .getByRole("button", { name: "View Gift-1", exact: true })
      .waitFor();

    await page
      .getByRole("button", { name: "Refresh gifts", exact: true })
      .click();
    await page
      .locator(".portfolio-meta")
      .filter({ hasText: "701 gifts" })
      .waitFor();
    assert.equal(
      await page.locator(".portfolio-progress").innerText(),
      "Inventory checked",
    );
    assert.equal(refreshCount, 1);
    const oldWatch = watchId;
    const beforeReconnect = pageReads.length;
    holdSummary = true;
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
    await page.evaluate(() => {
      window.scrollTo(0, document.documentElement.scrollHeight);
      window.dispatchEvent(new WheelEvent("wheel", { deltaY: 500 }));
    });
    await page.waitForTimeout(50);
    assert.equal(
      pageReads.length,
      beforeReconnect,
      "cached rows wait for a fresh summary before fetching more",
    );
    holdSummary = false;
    releaseSummary();
    await page.waitForTimeout(50);
    assert.equal(
      pageReads.length,
      beforeReconnect,
      "unchanged baseline reuses cached pages",
    );
    await page.evaluate(() => window.scrollTo(0, 0));
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
    assert.equal(await page.locator(".portfolio-ownership-note").count(), 0);
    await page
      .getByRole("button", { name: "View Gift-2", exact: true })
      .click();
    await page
      .getByRole("dialog")
      .getByText("Visible on this profile; ownership unverified.", {
        exact: true,
      })
      .waitFor();
    await page
      .getByRole("button", { name: "Close details", exact: true })
      .click();
    await page
      .getByRole("button", { name: "View Gift-3", exact: true })
      .click();
    await page
      .getByRole("dialog")
      .getByText("Hosted on this Telegram profile.", { exact: true })
      .waitFor();
    await page
      .getByRole("button", { name: "Close details", exact: true })
      .click();
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
    send(peer, {
      type: "portfolio.snapshot",
      id: watchId,
      data: portfolioSnapshot(report),
    });
    await page
      .locator(".portfolio-meta")
      .filter({ hasText: "6,000 gifts" })
      .waitFor();
    await page.locator('.gift-grid[data-order-count="100"]').waitFor();
    const other = page.locator(".portfolio-card").filter({
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
    let loaded = Number(
      await page.locator(".gift-grid").getAttribute("data-order-count"),
    );
    while (loaded < 6000) {
      await page.evaluate(() =>
        window.scrollTo(0, document.documentElement.scrollHeight),
      );
      await page.mouse.wheel(0, 500);
      await page
        .waitForFunction(
          (previous) =>
            Number(
              document
                .querySelector(".gift-grid")
                ?.getAttribute("data-order-count"),
            ) > previous,
          loaded,
        )
        .catch(async (error) => {
          console.log({
            loaded,
            scroll: await page.evaluate(() => ({
              y: window.scrollY,
              height: document.documentElement.scrollHeight,
              viewport: innerHeight,
              sentinel: document
                .querySelector(".scroll-loader")
                ?.getBoundingClientRect()
                .toJSON(),
            })),
            reads: pageReads.slice(-3),
            errors,
            alerts: await page.locator(".error").allTextContents(),
            loader: await page.locator(".scroll-loader").allTextContents(),
          });
          throw error;
        });
      loaded = Number(
        await page.locator(".gift-grid").getAttribute("data-order-count"),
      );
    }
    await page.evaluate(() =>
      window.scrollTo(0, document.documentElement.scrollHeight),
    );
    await page
      .getByRole("button", { name: "View Gift-6000", exact: true })
      .waitFor();
    assert(
      pageReads.every((query) => query.limit === 100),
      "all scroll reads remain bounded",
    );
    const bottomReads = catalogDetails;
    const bottomPageReads = pageReads.length;
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
    assert.equal(
      pageReads.length,
      bottomPageReads,
      "progress does not fetch portfolio pages",
    );
    await page.evaluate(() => window.scrollTo(0, 0));
    send(peer, {
      type: "portfolio.progress",
      id: watchId,
      refresh_id: 3,
      progress: report.progress,
    });
    await page
      .locator(".portfolio-progress")
      .filter({ hasText: "Inventory checked" })
      .waitFor();
    holdRefresh = true;
    await page.evaluate(() => {
      const refresh = document.querySelector('[aria-label="Refresh gifts"]');
      refresh.click();
      refresh.click();
    });
    await page.waitForTimeout(50);
    assert.equal(
      refreshCount,
      2,
      "repeated clicks share one pending refresh request",
    );
    assert(pendingRefreshId);
    await page.locator(".portfolio-owner-trigger").click();
    await page.getByLabel("Telegram user ID").fill("8");
    await page.getByRole("button", { name: "Open", exact: true }).click();
    await page.waitForFunction(() =>
      document.querySelector("h1")?.textContent?.includes("8"),
    );
    await page.waitForTimeout(50);
    assert(
      cancelledRequests.has(pendingRefreshId),
      "switching users cancels the pending refresh wait",
    );
    send(peer, {
      type: "error",
      id: pendingRefreshId,
      error: { code: "unavailable", message: "Old refresh failed" },
    });
    await page.waitForTimeout(30);
    assert.equal(
      await page.getByText("Old refresh failed", { exact: true }).count(),
      0,
    );
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
