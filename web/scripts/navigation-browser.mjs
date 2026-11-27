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
} from "./valuation-fixture.mjs";
const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "market-portfolio-"));
const output = fileURLToPath(new URL("../../.market/run", import.meta.url));
const artwork = await readFile(
  new URL("../src/brand/liberty-figure.svg", import.meta.url),
);
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">
import React from 'react';import{createRoot}from'react-dom/client';import{AppShell}from'/src/app/shell.tsx';import{MarketPage}from'/src/market-page.tsx';import'/src/style.css';
window.shares=[];window.inlineShares=[];window.canPrepare=true;window.fixtureInstance=Math.random();window.nativeBack={visible:false,handlers:new Set()};window.tapNativeBack=()=>[...window.nativeBack.handlers].forEach(f=>f());
const context={initData:'fixture',app:{isVersionAtLeast(){return window.canPrepare},shareMessage(id,callback){window.shares.push(id);window.shareCallback=callback},switchInlineQuery(query){window.inlineShares.push(query)},initData:'fixture',initDataUnsafe:{start_param:new URLSearchParams(location.search).get('tgWebAppStartParam')||'portfolio_7'},themeParams:{},colorScheme:'dark',onEvent(){},offEvent(){},BackButton:{show(){window.nativeBack.visible=true},hide(){window.nativeBack.visible=false},onClick(f){window.nativeBack.handlers.add(f)},offClick(f){window.nativeBack.handlers.delete(f)}}}};
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
    const shareTargets = [];
    await page.route(
      (url) => url.pathname === "/telegram/share",
      (r) => {
        assert.equal(
          r.request().headers().authorization,
          "Bearer navigation-session-fixture",
        );
        shareTargets.push(r.request().postDataJSON());
        return r.fulfill({
          json: {
            prepared: {
              id: "prepared-" + shareTargets.length,
              expires_at: Math.floor(Date.now() / 1000) + 600,
            },
            fallback: {
              type: "inline",
              query:
                shareTargets.at(-1).type === "gift"
                  ? `${shareTargets.at(-1).slug} ${shareTargets.at(-1).currency_id}`
                  : "portfolio 7 GRAM",
            },
          },
        });
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
    let catalogDetails = 0;
    await page.route(
      (url) => url.pathname === "/catalog",
      (r) =>
        r.fulfill({
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
        }),
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
        ],
      }),
    );
    const active = new Map();
    const rows = (params) =>
      Array.from({ length: 260 }, (_, i) => ({
        id: String(i + 1),
        book: {
          collection_id: "gift",
          currency_id: params.filters.currency_id,
          venue_id: "venue",
        },
        kind: "ask",
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
    await page.waitForFunction(() => window.shares.length === 1);
    assert.equal(shareTargets[0].type, "portfolio");
    assert.equal(shareTargets[0].target.user_id, 7);
    assert.equal(shareTargets[0].target.version, undefined);
    assert.equal(shareTargets[0].target.offset, undefined);
    assert.equal(
      refreshCount,
      0,
      "sharing does not recheck Telegram inventory",
    );
    await page.evaluate(() => window.shareCallback(false));
    assert.deepEqual(await page.evaluate(() => window.inlineShares), []);

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
    await page.waitForFunction(() => window.shares.length === 2);
    assert.equal(shareTargets[1].type, "market");
    assert.equal(shareTargets[1].selection.source, "asks");
    assert.equal(shareTargets[1].selection.filters.collection_id, "gift");
    await page.evaluate(() => window.shareCallback(true));

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
    const sharedOrderId = await page
      .locator(".orders .card-open")
      .first()
      .getAttribute("data-order-id");
    await page
      .locator(`.orders .card-open[data-order-id="${sharedOrderId}"]`)
      .click();
    await page
      .getByRole("dialog", { name: "Order details", exact: true })
      .waitFor();
    assert.equal(await page.locator(".detail-venue").count(), 1);
    assert.equal(await page.locator(".listing-price").count(), 1);
    await page.evaluate(() => {
      window.canPrepare = false;
    });
    await page.getByRole("button", { name: "Share ↗", exact: true }).click();
    await page.waitForFunction(() => window.inlineShares.length === 1);
    assert.equal(shareTargets.at(-1).type, "gift");
    assert.equal(shareTargets.at(-1).slug, `Gift-${sharedOrderId}`);
    assert.equal(
      await page.evaluate(() => window.inlineShares[0]),
      `${shareTargets.at(-1).slug} ${shareTargets.at(-1).currency_id}`,
    );
    await page.evaluate(() => {
      window.canPrepare = true;
    });

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
      await page.getByRole("spinbutton", { name: "Gift #" }).inputValue(),
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
    await page.getByRole("spinbutton", { name: "Gift #" }).fill("0");
    await page.getByRole("spinbutton", { name: "Gift #" }).fill("1");
    await page.locator(".portfolio-card").first().waitFor();
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
