import { portfolioSnapshot, portfolioPage } from "./portfolio-fixture.mjs";
// Shared shell with the real market and portfolio pages; all transport data is synthetic.
import assert from "node:assert/strict";
import { readFile, mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
import { orderResult, RangeWatch } from "./range-fixture.mjs";
import {
  floorReport,
  fixtureDisplay,
  giftReport,
  bestBidResult,
} from "./valuation-fixture.mjs";
const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "market-portfolio-"));
const output = fileURLToPath(new URL("../../.market/run", import.meta.url));
const artwork = await readFile(
  new URL("../src/brand/liberty-figure.svg", import.meta.url),
);
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">
import React from 'react';import{createRoot}from'react-dom/client';import{AppShell}from'/src/app/shell.tsx';import{MarketPage}from'/src/market-page.tsx';import'/src/style.css';
window.inlineShares=[];window.websiteLinks=[];window.fixtureInstance=Math.random();window.nativeBack={visible:false,handlers:new Set()};window.tapNativeBack=()=>[...window.nativeBack.handlers].forEach(f=>f());
window.startupScreens=0;new MutationObserver(records=>{for(const record of records)for(const node of record.addedNodes)if(node.nodeType===1&&(node.matches('.startup-loading')||node.querySelector('.startup-loading')))window.startupScreens++;}).observe(document.getElementById('root'),{childList:true,subtree:true});
const context={initData:'fixture',app:{isVersionAtLeast(){return true},openLink(url){window.websiteLinks.push(url)},switchInlineQuery(query){window.inlineShares.push(query)},initData:'fixture',initDataUnsafe:{start_param:new URLSearchParams(location.search).get('tgWebAppStartParam')||'portfolio_7'},themeParams:{},colorScheme:'dark',onEvent(){},offEvent(){},BackButton:{show(){window.nativeBack.visible=true},hide(){window.nativeBack.visible=false},onClick(f){window.nativeBack.handlers.add(f)},offClick(f){window.nativeBack.handlers.delete(f)}}}};
createRoot(document.getElementById('root')).render(React.createElement(AppShell,{context,market:React.createElement(MarketPage,{context})}));</script></body></html>`;

const server = await createServer({
  root,
  cacheDir,
  define: { "import.meta.env.VITE_MARKET_MINIAPP": JSON.stringify("true") },
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "portfolio-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (
            ["/__navigation", "/", "/portfolio", "/portfolio/7"].includes(
              req.url.split("?")[0],
            )
          ) {
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
    let authRequests = 0;
    let preparationRequests = 0;
    await page.route(
      (url) => url.pathname === "/telegram/share",
      (r) => {
        preparationRequests++;
        return r.fulfill({ status: 404 });
      },
    );
    await page.route(
      (url) => url.pathname === "/telegram",
      (r) => {
        authRequests++;
        return r.fulfill({
          json: {
            auth_enabled: true,
            bot_id: "42",
            bot_username: "fixture_bot",
            public_url: "https://example.test",
          },
        });
      },
    );
    await page.route(
      (url) => url.pathname === "/auth/telegram",
      (r) =>
        r.fulfill({
          json: {
            token: "navigation-session-fixture",
            expires_at: new Date(Date.now() + 86400000).toISOString(),
            user: {
              id: "7",
              first_name: "Fixture",
              last_name: null,
              username: null,
            },
          },
        }),
    );
    await page.route(
      (url) => url.pathname.startsWith("/media/"),
      (r) =>
        r.request().url().endsWith("/animation")
          ? r.fulfill({ status: 404 })
          : r.fulfill({ contentType: "image/svg+xml", body: artwork }),
    );
    await page.route(
      (url) => url.pathname === "/auth/me",
      (route) =>
        route.fulfill({
          json: {
            user: {
              id: "7",
              first_name: "Fixture",
              last_name: null,
              username: null,
            },
          },
        }),
    );
    let catalogDetails = 0,
      catalogDirectories = 0;
    await page.route(
      (url) => url.pathname === "/catalog",
      (r) => {
        catalogDirectories++;
        return r.fulfill({
          headers: { etag: '"directory"' },
          json: {
            collections: [collection],
            currencies: [
              { id: "GRAM", name: "GRAM", decimals: 9 },
              { id: "XTR", name: "Stars", decimals: 0 },
            ],
            venues: [{ id: "venue", name: "MRKT" }],
            books: ["GRAM", "XTR"].map((currency_id) => ({
              collection_id: "gift",
              currency_id,
              venue_id: "venue",
            })),
          },
        });
      },
    );
    await page.route(
      (url) => url.pathname.startsWith("/collections/"),
      (r) => {
        catalogDetails++;
        return r.fulfill({
          headers: { etag: '"details"' },
          json: { collection, models, backdrops, patterns },
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
          {
            id: "best_bid",
            revision: "r1",
            purpose: "quote",
            name: "Best bid",
            description: "Eligible bid",
            input: {
              targets: ["gift", "gift_attributes", "selection"],
              period: "unsupported",
            },
            display: fixtureDisplay("best_bid"),
            documentation_url: "/guide",
            documentation_revision: "r1",
          },
        ],
      }),
    );
    const active = new Map();
    const marketQueries = [];
    const rows = (params) =>
      Array.from({ length: 260 }, (_, i) => ({
        id: String(i + 1),
        book: {
          collection_id: "gift",
          currency_id: params.filters.currency_id,
          venue_id: "venue",
        },
        kind: params.source === "bids" ? "bid" : "ask",
        price: String(i + 2),
        quantity: 1,
        observed_at: "2026-11-24T00:00:00Z",
        target: {
          kind: "gift",
          id: String(i + 1),
          number: i + 1,
          traits: { model: "model", backdrop: "back" },
        },
        market_url: "https://example.com",
      }));
    let report,
      peer,
      watchId,
      watchCount = 0,
      refreshCount = 0,
      cancellations = 0;
    let holdMarketSnapshots = false;
    const heldMarketSnapshots = [];
    let holdCatalogVersions = false;
    const heldCatalogVersions = [];
    const send = (socket, value) => socket.send(JSON.stringify(value));
    await page.routeWebSocket("**/ws", (socket) => {
      const ids = new Set();
      socket.onClose(() => {
        for (const id of ids) active.delete(id);
      });
      send(socket, { type: "market.status", status: "available" });
      socket.onMessage((raw) => {
        const frame = JSON.parse(String(raw));
        if (frame.type === "cancel") {
          active.delete(frame.id);
          if (frame.id === watchId) cancellations++;
        }
        if (
          [
            "portfolio.watch",
            "orders.watch",
            "valuation.watch",
            "catalog.watch",
          ].includes(frame.type)
        ) {
          active.set(frame.id, frame.type);
          ids.add(frame.id);
        }
        if (frame.type === "orders.watch") {
          marketQueries.push(frame.params);
          const watch = new RangeWatch(frame.params, rows(frame.params));
          const value = watch.frame(frame.id, rows(frame.params));
          if (holdMarketSnapshots)
            heldMarketSnapshots.push(() => send(socket, value));
          else send(socket, value);
        }
        if (frame.type === "request" && frame.method === "orders.query")
          send(socket, {
            type: "result",
            id: frame.id,
            method: frame.method,
            ...orderResult(frame.params, rows(frame.params)),
          });
        if (frame.type === "catalog.watch") {
          const deliver = () =>
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
          if (holdCatalogVersions) heldCatalogVersions.push(deliver);
          else deliver();
        }
        if (frame.type === "portfolio.watch") {
          peer = socket;
          watchId = frame.id;
          watchCount++;
          report = state(frame.params.user_id, 700);
          report.valuation.currency_id = frame.params.currency_id;
          send(socket, {
            type: "portfolio.snapshot",
            id: frame.id,
            data: portfolioSnapshot(report),
          });
        }
        if (frame.type === "request" && frame.method === "portfolio.query") {
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
                    id: String(params.target.number ?? 1),
                    number: params.target.number ?? 1,
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
          if (params.target.kind !== "catalog")
            data.results.push(bestBidResult());
          send(
            socket,
            frame.type === "request"
              ? { type: "result", method: frame.method, id: frame.id, data }
              : { type: "valuation.snapshot", id: frame.id, data },
          );
        }
      });
    });
    await page.goto(server.resolvedUrls.local[0] + "__navigation");
    await page.locator(".portfolio-card").first().waitFor();
    const instance = await page.evaluate(() => window.fixtureInstance);
    assert.equal(
      await page.evaluate(() => window.nativeBack.visible),
      false,
      "root launch has no artificial back history",
    );
    await page
      .getByRole("button", { name: "View Gift-1", exact: true })
      .click();
    await page
      .getByRole("dialog", { name: "Gift details", exact: true })
      .waitFor();
    assert.equal(await page.locator(".detail-venue").count(), 0);
    assert.equal(await page.locator(".listing-price").count(), 0);
    await page.waitForFunction(() => window.nativeBack.visible);
    await page.evaluate(() => window.tapNativeBack());
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    await page.waitForFunction(() => !window.nativeBack.visible);
    await page.locator(".portfolio-owner-trigger").click();
    await page
      .getByRole("dialog", { name: "Open portfolio", exact: true })
      .waitFor();
    await page.waitForFunction(() => window.nativeBack.visible);
    await page.evaluate(() => window.tapNativeBack());
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    await page.getByLabel("Gift #", { exact: true }).fill("10");
    if (viewport.width < 960) {
      await page.evaluate(() => {
        window.keyboardHeight = innerHeight - 300;
        Object.defineProperty(visualViewport, "height", {
          configurable: true,
          get: () => window.keyboardHeight,
        });
        visualViewport.dispatchEvent(new Event("resize"));
      });
      await page.waitForFunction(
        () =>
          getComputedStyle(document.querySelector(".app-navigation"))
            .display === "none",
      );
      await page.evaluate(() => {
        window.keyboardHeight = innerHeight;
        visualViewport.dispatchEvent(new Event("resize"));
      });
      await page.waitForFunction(
        () =>
          getComputedStyle(document.querySelector(".app-navigation"))
            .display !== "none",
      );
      await page.evaluate(() =>
        Reflect.deleteProperty(visualViewport, "height"),
      );
    }
    await page.waitForFunction(
      () =>
        document.querySelector(".portfolio-total")?.textContent.trim() === "2",
    );
    await page
      .getByRole("button", { name: "Share portfolio", exact: true })
      .click();
    await page.waitForFunction(() => window.inlineShares.length === 1);
    const portfolioShare = await page.evaluate(() => window.inlineShares[0]);
    assert.match(portfolioShare, /^portfolio user=7 currency=GRAM/);
    assert(!/version=|offset=/.test(portfolioShare));
    assert.equal(
      await page.locator(".header-actions .share-button").count(),
      1,
    );
    assert.equal(
      await page.locator(".portfolio-check .share-button").count(),
      0,
    );
    const portfolioWebsite = new URL(
      await page
        .getByRole("link", { name: "Open website", exact: true })
        .getAttribute("href"),
    );
    assert.equal(portfolioWebsite.origin, "https://example.test");
    assert.equal(portfolioWebsite.pathname, "/portfolio/7");
    assert.match(
      new URLSearchParams(portfolioWebsite.hash.slice(1)).get("f"),
      /n=10/,
    );
    await page.getByRole("link", { name: "Open website", exact: true }).click();
    assert.equal(
      await page.evaluate(() => window.websiteLinks[0]),
      portfolioWebsite.href,
    );
    assert.equal(refreshCount, 0, "sharing does not recheck inventory");
    assert.equal(preparationRequests, 0, "sharing makes no API request");
    await page
      .locator(".app-navigation")
      .getByRole("link", { name: "Market", exact: true })
      .click();
    await page.locator(".orders .gift-card").first().waitFor();
    assert.equal(
      await page.evaluate(() => window.fixtureInstance),
      instance,
      "tab navigation must not reload",
    );
    assert.equal(authRequests, 1, "one shared auth/config lifecycle");
    await page
      .getByRole("button", { name: "Share market selection", exact: true })
      .click();
    await page.waitForFunction(() => window.inlineShares.length === 2);
    const marketShare = await page.evaluate(() => window.inlineShares[1]);
    assert.match(marketShare, /^market f=c=gift /);
    assert.match(marketShare, /mode=ask/);
    assert.equal(
      await page.locator(".header-actions .share-button").count(),
      1,
    );
    assert.equal(await page.locator(".feed-status .share-button").count(), 0);
    assert.equal(
      await page
        .getByRole("link", { name: "Open in Telegram", exact: true })
        .count(),
      0,
    );
    const marketWebsite = new URL(
      await page
        .getByRole("link", { name: "Open website", exact: true })
        .getAttribute("href"),
    );
    assert.equal(marketWebsite.origin, "https://example.test");
    assert.equal(
      new URLSearchParams(marketWebsite.hash.slice(1)).get("f"),
      "c=gift",
    );
    assert(!/initData|tgWebAppStartParam|token=/.test(marketWebsite.href));
    assert(
      ![...active.values()].includes("portfolio.watch"),
      "inactive portfolio releases its lease",
    );
    assert([...active.values()].includes("orders.watch"));
    await page.evaluate(() => window.scrollTo(0, 700));
    await page.waitForTimeout(100);
    const marketScroll = await page.evaluate(() => scrollY);
    assert(marketScroll > 500);
    await page
      .locator(".app-navigation")
      .getByRole("link", { name: "Portfolio", exact: true })
      .click();
    await page.locator(".portfolio-card").waitFor();
    assert.equal(
      await page.getByLabel("Gift #", { exact: true }).inputValue(),
      "10",
    );
    assert(
      ![...active.values()].includes("orders.watch"),
      "inactive market releases its feeds",
    );
    // Reopening must retain rows/scroll even before the new watch initializes.
    holdMarketSnapshots = true;
    holdCatalogVersions = true;
    const cachedCatalogReads = [catalogDirectories, catalogDetails];
    const startupScreens = await page.evaluate(() => window.startupScreens);
    await page
      .locator(".app-navigation")
      .getByRole("link", { name: "Market", exact: true })
      .click();
    await page
      .locator(".orders .gift-card")
      .first()
      .waitFor({ state: "attached" });
    await page.waitForFunction(
      (expected) => Math.abs(scrollY - expected) < 3,
      marketScroll,
    );
    assert(
      heldMarketSnapshots.length > 0,
      "restored cards did not wait for a fresh snapshot",
    );
    if (viewport.width < 960) {
      assert.equal(
        await page.locator(".feed-status").isVisible(),
        false,
        "Connecting does not add a row above retained listings",
      );
      assert.match(
        await page
          .locator('.market-views [aria-pressed="true"]')
          .getAttribute("aria-description"),
        /Connecting|Reconnecting/,
      );
      assert.equal(
        await page
          .locator(".market-results > .orders")
          .evaluate((el) => getComputedStyle(el).marginTop),
        "12px",
      );
    } else {
      assert.equal(
        await page.locator(".feed-status").isVisible(),
        true,
        "Desktop retains its collection header status",
      );
    }
    assert.equal(
      await page.evaluate(() => window.startupScreens),
      startupScreens,
      "Returning to Market never flashes the startup logo",
    );
    assert.deepEqual(
      [catalogDirectories, catalogDetails],
      cachedCatalogReads,
      "Cached catalog renders without another download",
    );
    holdCatalogVersions = false;
    for (const deliver of heldCatalogVersions.splice(0)) deliver();
    holdMarketSnapshots = false;
    for (const deliver of heldMarketSnapshots.splice(0)) deliver();
    await page
      .locator(".app-navigation")
      .getByRole("link", { name: "Portfolio", exact: true })
      .click();
    await page.locator(".portfolio-card").waitFor();
    await page.getByLabel("Gift #", { exact: true }).fill("");
    await page.getByRole("button", { name: "Gift", exact: true }).click();
    await page.locator("#pick-models").click();
    await page.waitForFunction(() => window.nativeBack.visible);
    await page.evaluate(() => window.tapNativeBack());
    await page.waitForFunction(
      () =>
        document
          .querySelector("#pick-models")
          ?.getAttribute("aria-expanded") === "false",
    );
    await page.getByRole("combobox", { name: "Currency", exact: true }).click();
    await page
      .getByRole("option", { name: "Stars (XTR)", exact: true })
      .click();
    await page.waitForTimeout(80);
    await page
      .locator(".app-navigation")
      .getByRole("link", { name: "Market", exact: true })
      .click();
    await page.locator(".orders .gift-card").first().waitFor();
    assert.match(
      await page
        .getByRole("combobox", { name: "Currency", exact: true })
        .getAttribute("title"),
      /Stars/,
    );
    assert.equal(
      refreshCount,
      0,
      "navigation and currency do not recheck Telegram",
    );
    // Virtual rows can start before the visible anchor. Share the clicked order,
    // rather than assuming the first mounted card is the market's first order.
    await page.waitForFunction(() =>
      [...document.querySelectorAll(".orders .card-open")].some((button) => {
        const rect = button.getBoundingClientRect();
        return rect.top >= 0 && rect.bottom <= innerHeight - 80;
      }),
    );
    const sharedOrderId = await page
      .locator(".orders .card-open")
      .evaluateAll((buttons) =>
        buttons
          .find((button) => {
            const rect = button.getBoundingClientRect();
            return rect.top >= 0 && rect.bottom <= innerHeight - 80;
          })
          .getAttribute("data-order-id"),
      );
    await page
      .locator(`.orders .card-open[data-order-id="${sharedOrderId}"]`)
      .click();
    await page
      .getByRole("dialog", { name: "Order details", exact: true })
      .waitFor();
    assert.equal(await page.locator(".detail-venue").count(), 1);
    assert.equal(await page.locator(".listing-price").count(), 1);
    await page.getByRole("button", { name: "Share", exact: true }).click();
    await page.waitForFunction(() => window.inlineShares.length === 3);
    assert.equal(
      await page.evaluate(() => window.inlineShares[2]),
      `Gift-${sharedOrderId} XTR`,
    );
    assert.equal(preparationRequests, 0);

    await page.evaluate(() => window.tapNativeBack());
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    await page
      .locator(".app-navigation")
      .getByRole("link", { name: "Portfolio", exact: true })
      .click();
    await page.locator(".portfolio-card").first().waitFor();
    await page.screenshot({
      path: join(output, `portfolio-compact-${viewport.width}.png`),
      fullPage: false,
    });
    await page.locator(".brand").click();
    await page.locator(".orders .gift-card").first().waitFor();
    assert.equal(
      await page.evaluate(() => window.fixtureInstance),
      instance,
      "brand navigation shares the client-side router",
    );
    await page.goBack();
    await page.locator(".portfolio-card").first().waitFor();
    await page.goForward();
    await page.locator(".orders .gift-card").first().waitFor();
    assert.equal(authRequests, 1);
    const marketUrl = page.url();
    const refreshesBeforeProfile = refreshCount;
    await page.locator(".account-button").click();
    await page.getByRole("heading", { name: "Profile", exact: true }).waitFor();
    for (let i = 0; active.size && i < 100; i++) await page.waitForTimeout(10);
    assert.equal(
      active.size,
      0,
      "profile releases data watches and starts none",
    );
    assert.equal(
      refreshCount,
      refreshesBeforeProfile,
      "profile never rechecks inventory",
    );
    assert.equal(authRequests, 1, "profile shares the authenticated session");
    assert.match(
      await page
        .getByRole("link", { name: "My portfolio", exact: true })
        .getAttribute("href"),
      /\/portfolio\/7\?/,
    );
    await page.getByRole("link", { name: "My portfolio", exact: true }).click();
    await page.locator(".portfolio-card").first().waitFor();
    await page.evaluate(() => window.tapNativeBack());
    await page.getByRole("heading", { name: "Profile", exact: true }).waitFor();
    await page.evaluate(() => window.tapNativeBack());
    await page.locator(".orders .gift-card").first().waitFor();
    assert.equal(
      page.url(),
      marketUrl,
      "profile Back restores the market selection",
    );
    assert.equal(
      await page.evaluate(() => window.nativeBack.handlers.size),
      1,
      "native Back handlers are not accumulated",
    );
    await page.screenshot({
      path: join(output, `navigation-${viewport.width}.png`),
      fullPage: false,
    });
    await page.evaluate(() => sessionStorage.clear());
    const returnParams = new URLSearchParams({
      user: "7",
      currency: "XTR",
      f: "c=gift|m=model|b=back|n=1",
      sort: "number_desc",
    });
    const payload =
      "p_" + Buffer.from(returnParams.toString()).toString("base64url");
    await page.goto(
      server.resolvedUrls.local[0] +
        "__navigation?tgWebAppStartParam=" +
        payload,
    );
    await page.locator(".portfolio-card").first().waitFor();
    assert.equal(await page.locator(".portfolio-card").count(), 1);
    assert.equal(
      await page
        .getByRole("textbox", { name: "Gift #", exact: true })
        .inputValue(),
      "1",
    );
    assert.match(await page.locator(".portfolio-sort").innerText(), /Gift #/);
    assert.match(
      await page
        .getByRole("combobox", { name: "Currency", exact: true })
        .getAttribute("title"),
      /Stars/,
    );
    assert.equal(new URL(page.url()).pathname, "/portfolio/7");
    const returned = new URL(page.url()).searchParams;
    assert.equal(returned.get("sort"), "number_desc");
    assert.match(returned.get("f"), /m=model/);
    await page.getByRole("textbox", { name: "Gift #", exact: true }).fill("0");
    await page.getByRole("textbox", { name: "Gift #", exact: true }).fill("1");
    await page.locator(".portfolio-card").first().waitFor();
    await page.goto(server.resolvedUrls.local[0] + "__navigation");
    await page.locator('.gift-grid[data-order-count="100"]').waitFor();
    const linkInstance = await page.evaluate(() => window.fixtureInstance);
    assert.equal(
      await page.locator(".sidebar-heading .brand").count(),
      1,
      "main views retain the animated home logo",
    );
    assert.equal(await page.locator(".header-back").count(), 0);
    await page.evaluate(() => window.scrollTo(0, 900));
    await page.waitForTimeout(100);
    const selectedCard = await page
      .locator(".portfolio-card .card-open")
      .evaluateAll((buttons) =>
        buttons
          .find((button) => {
            const box = button.getBoundingClientRect();
            return box.top >= 0 && box.bottom <= innerHeight - 90;
          })
          ?.getAttribute("aria-label"),
      );
    assert(selectedCard);
    const selectedNumber = selectedCard.match(/Gift-(\d+)/)[1];
    // Price links open a separate market history entry and return to the exact preview.
    for (const [label, source, traits] of [
      ["View model asks", "asks", { models: ["model"], backdrops: [] }],
      [
        "View model & backdrop bids",
        "bids",
        { models: ["model"], backdrops: ["back"] },
      ],
    ]) {
      await page
        .getByRole("button", { name: selectedCard, exact: true })
        .click();
      const preview = page.getByRole("dialog", {
        name: "Gift details",
        exact: true,
      });
      await preview.waitFor();
      const link = preview.getByRole("link", { name: label, exact: true });
      await link.waitFor();
      await link.scrollIntoViewIfNeeded();
      const quoteButton = await link.evaluate((anchor) => {
        const bounds = anchor.getBoundingClientRect(),
          value = anchor.querySelector(".floor-value").getBoundingClientRect(),
          cell = anchor.closest("td").getBoundingClientRect();
        return {
          contained: bounds.left >= cell.left && bounds.right <= cell.right,
          padded:
            value.left - bounds.left >= 5 && bounds.right - value.right >= 5,
          compact: bounds.width <= Math.max(44, value.width + 16),
          height: bounds.height,
        };
      });
      assert(
        quoteButton.contained && quoteButton.padded,
        "price and gift count fit inside a padded button",
      );
      assert(
        quoteButton.compact,
        "hover targets follow price content rather than spanning the column",
      );
      assert(
        quoteButton.height >= (viewport.width < 960 ? 44 : 36),
        "compact quotes retain their touch/click target",
      );
      const before = await page.evaluate(() => ({
        url: location.pathname + location.search,
        y: scrollY,
        count: document.querySelector(".gift-grid").dataset.orderCount,
        sheet: document.querySelector(".detail").scrollTop,
        pane: document.querySelector(".detail-content").scrollTop,
      }));
      const href = await link.getAttribute("href");
      assert(href.includes("f="));
      await link.click();
      await page.locator(".orders .gift-card").first().waitFor();
      await page.waitForFunction(() => !document.querySelector("dialog[open]"));
      const query = marketQueries.at(-1);
      assert.equal(query.source, source);
      assert.equal(query.sort, source === "asks" ? "price_asc" : "price_desc");
      assert.deepEqual(
        query.filters.models,
        traits.models.map(() => "@name:Model"),
      );
      assert.deepEqual(
        query.filters.backdrops,
        traits.backdrops.map(() => "@name:Green"),
      );
      assert.deepEqual(query.filters.patterns, []);
      assert.equal(query.filters.number, null);
      assert.equal(query.filters.currency_id, "GRAM");
      if (source === "bids") assert.equal(query.bid_match, "any");
      assert.equal(
        await page.locator(".sidebar-heading > .header-back").count(),
        1,
        "contextual return replaces the top-left logo",
      );
      assert.equal(await page.locator(".sidebar-heading .brand").count(), 0);
      assert.equal(
        await page.locator(".page-return").count(),
        0,
        "no separate back row remains",
      );
      if (source === "asks") {
        for (const width of [280, 320, 390, 960, 1440]) {
          await page.setViewportSize({ width, height: viewport.height });
          const header = await page
            .locator(".sidebar-heading")
            .evaluate((heading) => {
              const bounds = heading.getBoundingClientRect(),
                back = heading
                  .querySelector(".header-back")
                  .getBoundingClientRect(),
                actions = heading
                  .querySelector(".header-actions")
                  .getBoundingClientRect();
              return {
                height: bounds.height,
                touch: back.width >= 44 && back.height >= 44,
                contained:
                  back.left >= bounds.left &&
                  actions.right <= bounds.right + 1 &&
                  back.right <= actions.left - 3,
                caption: getComputedStyle(
                  heading.querySelector(".header-back-label"),
                ).display,
                labelFits:
                  heading.querySelector(".header-back-label").scrollWidth <=
                  heading.querySelector(".header-back-label").clientWidth,
              };
            });
          assert.equal(header.height, 44, `one header row at ${width}px`);
          assert(
            header.touch && header.contained,
            `return and account actions fit at ${width}px`,
          );
          assert.equal(
            header.caption === "none",
            width < 360,
            "narrow headers use an accessible arrow-only button",
          );
          if (width >= 360)
            assert(header.labelFits, `return label fits at ${width}px`);
        }
        await page.setViewportSize(viewport);
        await page
          .locator(".sidebar-heading")
          .screenshot({
            path: join(output, `header-back-${viewport.width}.png`),
          });
      }
      if (source === "asks") {
        await page.locator(".orders .card-open").first().click();
        const marketPreview = page.getByRole("dialog", {
          name: "Order details",
          exact: true,
        });
        const collectionPrice = marketPreview.getByRole("link", {
          name: "View collection asks",
          exact: true,
        });
        await collectionPrice.waitFor();
        const priorMarketUrl = page.url();
        await collectionPrice.click();
        await page
          .getByRole("button", { name: "Back to preview", exact: true })
          .waitFor();
        await page.waitForFunction(
          () => !document.querySelector("dialog[open]"),
        );
        await page
          .getByRole("button", { name: "Back to preview", exact: true })
          .click();
        await marketPreview.waitFor();
        assert.equal(
          page.url(),
          priorMarketUrl,
          "market-to-market drill-down restores its prior filters and preview",
        );
        await page
          .getByRole("button", { name: "Close details", exact: true })
          .click();
        await marketPreview.waitFor({ state: "detached" });
      }
      assert.equal(
        await page.evaluate(() => window.fixtureInstance),
        linkInstance,
        "price navigation never reloads the app",
      );
      if (source === "asks")
        await page
          .getByRole("button", { name: "Back to portfolio", exact: true })
          .click();
      else await page.evaluate(() => window.tapNativeBack());
      await page
        .getByRole("dialog", { name: "Gift details", exact: true })
        .waitFor();
      await page.waitForFunction(
        (expected) => Math.abs(scrollY - expected) < 3,
        before.y,
      );
      assert.equal(
        new URL(page.url()).pathname + new URL(page.url()).search,
        before.url,
      );
      assert.equal(
        await page.locator(".gift-grid").getAttribute("data-order-count"),
        before.count,
      );
      const restored = await preview.evaluate((el) => ({
        sheet: el.scrollTop,
        pane: el.querySelector(".detail-content").scrollTop,
      }));
      assert(
        Math.abs(restored.sheet - before.sheet) < 3,
        "mobile preview scroll returns",
      );
      assert(
        Math.abs(restored.pane - before.pane) < 3,
        "desktop preview scroll returns",
      );
      assert.equal(
        await preview.locator("#detail-title").innerText(),
        `Gift #${selectedNumber}`,
      );
      await page
        .getByRole("button", { name: "Close details", exact: true })
        .click();
      await preview.waitFor({ state: "detached" });
    }
    assert.deepEqual(errors, []);
    await page.close();
  }
  console.log(
    "Navigation browser passed: shared cards/details, one auth lifecycle, native Back, launch routing, tab/brand links, browser history, preserved filters/currency and released inactive watches.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
