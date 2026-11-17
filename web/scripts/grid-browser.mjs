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
const make=(id)=>({id:String(id),book:{venue_id:'v',collection_id:'gift',currency_id:'GRAM'},mode:'ask',price:'1',quantity:1,target:{kind:'criteria',criteria:{models:[],backdrops:[],patterns:[],number:null}}});
let rows=Array.from({length:10000},(_,i)=>make(i));
const options=new Map();const open=()=>{};const visible=(start,end)=>{window.range=[start,end]};
const card=()=>({options,collectionName:'Gift',venueName:'Venue',selected:false,open});
const render=()=>root.render(React.createElement('div',{className:'workspace'},React.createElement('div',{style:{height:72}},'Header'),React.createElement(GiftGrid,{orders:rows,card,busy:false,visible})));
window.prepend=()=>{rows=[make(-3),make(-2),make(-1),...rows];render()};
window.unmount=()=>root.unmount();
render();
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
  assert.deepEqual(errors, []);
  console.log(
    "Grid browser passed: 10,000 rows, bounded DOM, scroll anchors, resize, final row and cleanup.",
  );
} finally {
  await browser?.close();
  await server.close();
}
