import assert from "node:assert/strict";
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
const query={kind:'orders',query:{filters:{collection_id:'gift',currency_id:'GRAM',venue_id:null,models:[],backdrops:[],patterns:[],number:null,price:null},side:'ask',sort:'price_asc',consistency:'live',limit:50,page_token:''}};
const options=new Map(),open=()=>{};
const card=()=>({options,collectionName:'Gift',venueName:'Venue',selected:false,open});
function App(){const feed=useOrderFeed(query,false,0);window.feedView=()=>feed;
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
  // A bottom scroll should append one page without dragging the viewport along
  // with the loading sentinel and draining every continuation automatically.
  let requests = 0;
  const sync = { ready: true, state: "live", updated_at: null };
  const makePage = (offset) => ({
    data: Array.from({ length: 50 }, (_, i) => ({
      id: String(offset + i),
      book: { venue_id: "v", collection_id: "gift", currency_id: "GRAM" },
      kind: "ask",
      price: String(offset + i + 1),
      quantity: 1,
      target: {
        kind: "criteria",
        criteria: { models: [], backdrops: [], patterns: [], number: null },
      },
    })),
    next_page_token: offset < 950 ? String(offset + 50) : "",
    cursor: "view",
    sync,
  });
  await page.routeWebSocket("**/ws", (socket) => {
    socket.onMessage((message) => {
      if (JSON.parse(String(message)).type === "subscribe")
        socket.send(
          JSON.stringify({ type: "page", id: "offers", page: makePage(0) }),
        );
    });
  });
  await page.route("**/orders/query", async (route) => {
    requests++;
    await route.fulfill({
      json: makePage(Number(route.request().postDataJSON().page_token)),
    });
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__scroll_test`,
  );
  await page.waitForFunction(
    () => window.feedView?.().displayOrders.length === 50,
  );
  assert.equal(requests, 0);
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  const scrollTop = await page.evaluate(() => window.scrollY);
  await page.waitForFunction(
    () => window.feedView?.().displayOrders.length >= 100,
  );
  // Give observer/layout notifications enough time to expose a request loop.
  await page.waitForTimeout(500);
  assert.equal(requests, 1, "A single scroll must not load every continuation");
  assert.ok(
    Math.abs((await page.evaluate(() => window.scrollY)) - scrollTop) < 2,
    "Appending a page must preserve the viewport, rather than follow the footer",
  );
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  await page.waitForFunction(
    () => window.feedView?.().displayOrders.length >= 150,
  );
  await page.waitForTimeout(500);
  assert.equal(
    requests,
    2,
    "Scrolling to the new bottom should load the next page",
  );
  assert.ok((await page.locator(".gift-card").count()) < 60);
  await page.evaluate(() => window.unmount());
  assert.deepEqual(errors, []);
  console.log(
    "Grid browser passed: 10,000 rows, bounded DOM, scroll anchors, resize, final row, incremental loading and cleanup.",
  );
} finally {
  await browser?.close();
  await server.close();
}
