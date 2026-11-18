import assert from "node:assert/strict";
import { orderWindow } from "./window-fixture.mjs";
import { createServer } from "vite";
import { chromium } from "playwright";
const root = new URL("..", import.meta.url).pathname;
const html = `<!doctype html><html><body style="margin:0"><div id="fixture"></div><script type="module">
import React from 'react';
import {createRoot} from 'react-dom/client';
import {GiftGrid} from '/src/orderbook/gift-grid.tsx';
import '/src/style.css';
const root=createRoot(document.getElementById('fixture'));
const make=(id)=>({id:String(id),book:{venue_id:'v',collection_id:'gift',currency_id:'GRAM'},kind:'ask',price:'1',quantity:1,target:{kind:'criteria',criteria:{models:[],backdrops:[],patterns:[],number:null}}});
let rows=Array.from({length:10000},(_,i)=>make(i));
const options=new Map();const open=()=>{};const visible=(start,end)=>{window.range=[start,end]};
const card=()=>({options,collectionName:'Gift',venueName:'Venue',selected:false,open});
const render=()=>root.render(React.createElement('div',{className:'workspace'},React.createElement('div',{style:{height:72}},'Header'),React.createElement(GiftGrid,{orders:rows,card,busy:false,visible})));
window.prepend=()=>{rows=[make(-3),make(-2),make(-1),...rows];render()};
window.unmount=()=>root.unmount();
render();
</script></body></html>`;
const feedHtml = `<!doctype html><html><body style="margin:0"><div id="fixture"></div><script type="module">
import React from 'react';
import {createRoot} from 'react-dom/client';
import {useOrderFeed} from '/src/use-order-feed.ts';
import {GiftGrid} from '/src/orderbook/gift-grid.tsx';
import {ScrollLoader} from '/src/orderbook/scroll-loader.tsx';
import '/src/style.css';
const query={kind:'orders',query:{filters:{collection_id:'gift',currency_id:'GRAM',venue_id:null,models:[],backdrops:[],patterns:[],number:null,price:null},side:'ask',sort:'price_asc'}};
const options=new Map(),open=()=>{};
const card=()=>({options,collectionName:'Gift',venueName:'Venue',selected:false,open});
function App(){const [paused,setPaused]=React.useState(false);window.setPaused=setPaused;const feed=useOrderFeed(query,paused,0);window.feedView=()=>feed;
return React.createElement('main',{className:'workspace'},React.createElement('div',{className:'orderbook'},
React.createElement('header',{style:{height:72}},'Market'),
React.createElement('section',{className:'orders'},
React.createElement(GiftGrid,{orders:feed.displayOrders,card,busy:feed.loadingMore,visible:feed.setVisibleRange}),
React.createElement(ScrollLoader,{loading:feed.loadingMore,hasMore:feed.hasMore,disabled:!!feed.error,loadMore:feed.loadMore})),
React.createElement('footer',{className:'app-footer'},'Footer')));}
const root=createRoot(document.getElementById('fixture'));root.render(React.createElement(App));window.unmount=()=>root.unmount();
</script></body></html>`;
const server = await createServer({
  root,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "grid-fixture",
      configureServer(server) {
        server.middlewares.use("/__grid_test", async (_req, res, next) => {
          try {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml("/__grid_test", html));
          } catch (error) {
            next(error);
          }
        });
        server.middlewares.use("/__scroll_test", async (_req, res, next) => {
          try {
            res.setHeader("Content-Type", "text/html");
            res.end(
              await server.transformIndexHtml("/__scroll_test", feedHtml),
            );
          } catch (error) {
            next(error);
          }
        });
      },
    },
  ],
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 480, height: 700 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript(() => {
    const add = window.addEventListener.bind(window),
      remove = window.removeEventListener.bind(window);
    window.scrollListeners = new Set();
    window.addEventListener = (type, listener, options) => {
      if (type === "scroll") window.scrollListeners.add(listener);
      add(type, listener, options);
    };
    window.removeEventListener = (type, listener, options) => {
      if (type === "scroll") window.scrollListeners.delete(listener);
      remove(type, listener, options);
    };
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__grid_test`,
  );
  await page.waitForFunction(
    () => document.querySelector(".gift-grid")?.dataset.orderCount === "10000",
  );
  assert.ok((await page.locator(".gift-card").count()) < 60);
  assert.ok((await page.evaluate(() => document.body.scrollHeight)) > 390000);
  await page.evaluate(() => window.scrollTo(0, 50000));
  await page.waitForFunction(() => window.range?.[0] > 1000);
  const before = await page.locator(".card-open").evaluateAll((nodes) => {
    const node = nodes.find((node) => node.getBoundingClientRect().top >= 0);
    return { id: node.dataset.orderId, top: node.getBoundingClientRect().top };
  });
  await page.evaluate(() => window.prepend());
  await page.waitForFunction(
    () => document.querySelector(".gift-grid")?.dataset.orderCount === "10003",
  );
  const after = await page
    .locator(`[data-order-id="${before.id}"]`)
    .evaluate((node) => node.getBoundingClientRect().top);
  assert.ok(
    Math.abs(after - before.top) < 2,
    `Scroll anchor moved by ${after - before.top}px`,
  );
  await page.setViewportSize({ width: 360, height: 500 });
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  await page.waitForFunction(
    () => !!document.querySelector('[data-order-id="9999"]'),
  );
  assert.ok((await page.locator(".gift-card").count()) < 60);
  assert.equal(await page.evaluate(() => window.scrollListeners.size), 1);
  await page.evaluate(() => window.unmount());
  assert.equal(await page.locator(".gift-card").count(), 0);
  assert.equal(await page.evaluate(() => window.scrollListeners.size), 0);
  // A live window slides atomically in both directions without any HTTP pages.
  let requests = 0,
    subscriptions = 0;
  const sync = { ready: true, state: "live", updated_at: null };
  const rows = Array.from({ length: 1000 }, (_, i) => ({
    id: String(i),
    book: { venue_id: "v", collection_id: "gift", currency_id: "GRAM" },
    kind: "ask",
    price: String(i + 1),
    quantity: 1,
    observed_at: "2026-11-18T12:00:00Z",
    target: {
      kind: "criteria",
      criteria: { models: [], backdrops: [], patterns: [], number: null },
    },
  }));
  const anchorErrors = [],
    checks = [];
  let anchorChecks = 0,
    nearTopChecks = 0;
  await page.routeWebSocket("**/ws", (socket) => {
    let current;
    socket.onMessage((message) => {
      const incoming = JSON.parse(String(message));
      if (incoming.type === "cancel") {
        if (current === incoming.id) current = undefined;
        return;
      }
      if (incoming.type === "subscribe_window") current = incoming.id;
      const work = (async () => {
        const command = incoming;
        if (command.type !== "subscribe_window") return;
        subscriptions++;
        const marker = await page.locator(".card-open").evaluateAll((cards) => {
          const row = cards.find((el) => {
            const r = el.getBoundingClientRect();
            return r.bottom > 0 && r.top < innerHeight;
          });
          return row
            ? { id: row.dataset.orderId, top: row.getBoundingClientRect().top }
            : null;
        });
        if (current !== command.id) return;
        const window = orderWindow(command.query, rows);
        socket.send(
          JSON.stringify({
            type: "window",
            id: command.id,
            window,
            sync,
            revision: "view",
          }),
        );
        if (marker && window.data.some((row) => row.id === marker.id)) {
          await page.waitForFunction(
            (first) =>
              window.feedView?.().orders[0]?.id === first &&
              !window.feedView().loadingMore,
            window.data[0].id,
          );
          await page.evaluate(() => new Promise(requestAnimationFrame));
          if (current !== command.id) return;
          const actual = await page.evaluate((id) => {
            const row = document.querySelector(`[data-order-id="${id}"]`);
            return {
              top: row?.getBoundingClientRect().top,
              first: window.feedView().orders[0]?.id,
              loading: window.feedView().loadingMore,
              scroll: scrollY,
              rendered: [...document.querySelectorAll(".card-open")]
                .slice(0, 4)
                .map((el) => el.dataset.orderId),
            };
          }, marker.id);
          assert.notEqual(
            actual.top,
            undefined,
            `Missing active anchor: ${JSON.stringify({ marker, actual, query: command.query })}`,
          );
          const top = actual.top;
          anchorChecks++;
          if (marker.top >= 0) nearTopChecks++;
          assert.ok(
            Math.abs(top - marker.top) < 2,
            `Window replacement moved anchor ${marker.id} by ${top - marker.top}px`,
          );
        }
      })().catch((error) => anchorErrors.push(error.message));
      checks.push(work);
    });
  });
  await page.route("**/orders/query", () => {
    requests++;
    assert.fail("Live windows must not fetch HTTP pages");
  });
  const frozen = new Map();
  let nextSnapshot = 0;
  const released = [];
  const frozenRequests = [];
  await page.route("**/orders/window", (route) => {
    const body = route.request().postDataJSON();
    if (route.request().method() === "DELETE") {
      released.push(body.snapshot);
      frozen.delete(body.snapshot);
      return route.fulfill({ status: 204 });
    }
    frozenRequests.push(body);
    const snapshot = body.snapshot ?? `snapshot.${++nextSnapshot}`;
    if (!frozen.has(snapshot)) frozen.set(snapshot, rows.slice());
    return route.fulfill({
      json: {
        window: orderWindow(body.query, frozen.get(snapshot)),
        snapshot,
        sync,
        revision: "frozen",
      },
    });
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__scroll_test`,
  );
  await page.waitForFunction(() => window.feedView?.().orders.length === 150);
  assert.equal(requests, 0);
  assert.equal(subscriptions, 1);
  const anchor = async () =>
    page.locator(".card-open").evaluateAll((cards) => {
      const row = cards.find((el) => {
        const r = el.getBoundingClientRect();
        return r.top >= 0 && r.bottom < innerHeight;
      });
      return row
        ? { id: row.dataset.orderId, top: row.getBoundingClientRect().top }
        : null;
    });
  await page.evaluate(() =>
    window.scrollTo(0, document.body.scrollHeight - 600),
  );
  await page.waitForFunction(
    () => Number(window.feedView?.().orders[0]?.id) > 0,
  );
  await page.waitForTimeout(200);
  const count = subscriptions;
  await page.waitForTimeout(500);
  assert.equal(
    subscriptions,
    count,
    "One scroll must settle without draining the book",
  );
  await Promise.all(checks);
  const liveAnchor = await anchor();
  assert.ok(liveAnchor);
  assert.equal(await page.evaluate(() => window.feedView().orders.length), 150);
  assert.ok((await page.locator(".gift-card").count()) < 60);
  assert.equal(requests, 0);
  const anchorBeforePause = await anchor();
  const liveSubscriptions = subscriptions;
  await page.evaluate(() => window.setPaused(true));
  await page.waitForFunction(() => window.feedView().status === "Paused");
  await page.evaluate(() => new Promise(requestAnimationFrame));
  const pausedTop = await page
    .locator(`[data-order-id="${anchorBeforePause.id}"]`)
    .evaluate((el) => el.getBoundingClientRect().top);
  assert.ok(
    Math.abs(pausedTop - anchorBeforePause.top) < 2,
    "Pausing preserves the visible anchor instead of restarting at the head",
  );
  assert.ok(
    Number(await page.evaluate(() => window.feedView().orders[0].id)) > 0,
  );
  assert.equal(
    subscriptions,
    liveSubscriptions,
    "Paused navigation uses a frozen HTTP view",
  );
  assert.equal(frozenRequests.length, 1);
  await page.evaluate(() => window.setPaused(false));
  await page.waitForFunction(() => window.feedView().status === "Live");
  await page.waitForTimeout(100);
  assert.equal(
    released.length,
    1,
    "Resuming releases the frozen snapshot lease",
  );
  // Scrolling back toward the first buffered row asks for preceding orders.
  const previousStart = await page.evaluate(() =>
    Number(window.feedView().orders[0].id),
  );
  await page.evaluate(() => window.scrollTo(0, 80));
  await page.waitForFunction(
    (start) => Number(window.feedView().orders[0].id) < start,
    previousStart,
  );
  await page.waitForTimeout(400);
  assert.equal(requests, 0);
  await Promise.all(checks);
  assert.deepEqual(anchorErrors, []);
  assert.ok(
    anchorChecks > 0,
    "At least one real anchor must be checked across window replacements",
  );
  assert.ok(
    nearTopChecks > 0,
    "Preserve a visible anchor below the header when loading preceding rows",
  );
  await page.evaluate(() => window.unmount());
  assert.deepEqual(errors, []);
  console.log(
    "Grid browser passed: 10,000 rows, bounded DOM, scroll anchors, resize, final row, incremental loading and cleanup.",
  );
} finally {
  await browser?.close();
  await server.close();
}
