// Real socket protocol + React: gaps, queued reconnect reads, status and collection retirement.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "market-live-feed-"));
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body><div id="fixture"></div><script type="module">
import React,{useState}from'react';import{createRoot}from'react-dom/client';import{useOrderFeed}from'/src/use-order-feed.ts';import{GiftGrid}from'/src/orderbook/gift-grid.tsx';
function App(){const[id,select]=useState('a');const feed=useOrderFeed({source:'history',filters:{collection_id:id,currency_id:'GRAM'},sort:'time_desc',period:{from:'2026-11-19T00:00:00Z',until:'2026-11-21T00:00:00Z'}},false,0);window.more=feed.loadMore;window.select=select;window.view=()=>feed;
return React.createElement('main',{className:'orderbook'},React.createElement('p',{role:'status'},feed.status),feed.error&&React.createElement('p',{role:'alert'},feed.error),React.createElement(GiftGrid,{orders:feed.displayOrders,busy:feed.loadingMore,visible:feed.setVisibleRange,card:()=>({options:new Map(),collectionName:id,venueName:'Venue',selected:false,open:()=>{}})}));}
createRoot(document.getElementById('fixture')).render(React.createElement(App));</script></body></html>`;
const key = (n) => `K${String(n).padStart(7, "0")}`;
const row = (id, rank, collection) => ({
  id,
  key: key(rank),
  book: { collection_id: collection, currency_id: "GRAM", venue_id: "v" },
  kind: "executed",
  price: "1",
  quantity: 1,
  observed_at: "2026-11-20T00:00:00Z",
  executed_at: "2026-11-20T00:00:00Z",
  target: {
    kind: "gift",
    id: "gift-" + id,
    number: rank + 1,
    traits: { model: null, backdrop: null, pattern: null },
  },
});
const datasets = Object.fromEntries(
  ["a", "b", "c"].map((collection) => [
    collection,
    Array.from({ length: 250 }, (_, i) =>
      row("old-" + i, 1000 + i * 2, collection),
    ),
  ]),
);
function read(q) {
  assert.equal(q.source, "history");
  assert.equal(q.sort, "time_desc");
  assert.ok(q.period.from);
  assert.equal(q.view, undefined);
  const range = q.range ?? { start: null, end: null };
  const all = datasets[q.filters.collection_id].filter(
    (r) =>
      (range.start === null || r.key >= range.start) &&
      (range.end === null || r.key < range.end),
  );
  const data = all.slice(0, q.limit ?? 100);
  return {
    data,
    range: {
      start: range.start,
      end:
        all.length > data.length
          ? key(Number(data.at(-1).key.slice(1)) + 1)
          : range.end,
    },
  };
}
const sockets = new Set(),
  ops = new Map(),
  watches = [],
  queries = [],
  cancelled = [];
let holdBaseline = false,
  holdQuery = false;
const sendBaseline = (op) => {
  op.socket.send(
    JSON.stringify({
      type: "orders.snapshot",
      id: op.id,
      sequence: 0,
      ...op.baseline,
    }),
  );
  op.sent = true;
};
const sendQuery = (op) =>
  op.socket.send(
    JSON.stringify({
      type: "result",
      id: op.id,
      method: "orders.query",
      ...op.result,
    }),
  );
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "live-feed-fixture",
      configureServer(server) {
        server.middlewares.use("/__live_feed", async (_, res) => {
          res.setHeader("Content-Type", "text/html");
          res.end(await server.transformIndexHtml("/__live_feed", html));
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
  page.setDefaultTimeout(10000);
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.route("**/orders/query", () =>
    assert.fail("Live views must not create REST snapshot leases"),
  );
  await page.routeWebSocket("**/ws", (socket) => {
    sockets.add(socket);
    socket.send(JSON.stringify({ type: "market.status", status: "available" }));
    socket.onMessage((message) => {
      const cmd = JSON.parse(String(message));
      if (cmd.type === "cancel") {
        cancelled.push(cmd.id);
        ops.delete(cmd.id);
        return;
      }
      if (cmd.type === "orders.watch") {
        const op = {
          socket,
          id: cmd.id,
          query: cmd.params,
          baseline: read(cmd.params),
          sent: false,
        };
        ops.set(cmd.id, op);
        watches.push(op);
        if (!holdBaseline) sendBaseline(op);
      } else if (cmd.type === "request") {
        assert.equal(cmd.method, "orders.query");
        const op = {
          socket,
          id: cmd.id,
          query: cmd.params,
          result: read(cmd.params),
        };
        ops.set(cmd.id, op);
        queries.push(op);
        if (!holdQuery) sendQuery(op);
      }
    });
    socket.onClose(() => {
      sockets.delete(socket);
      for (const [id, op] of ops) if (op.socket === socket) ops.delete(id);
    });
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__live_feed`,
  );
  const count = (expected) =>
    page.waitForFunction(
      (n) => window.view?.().orders.length === n && !window.view().loadingMore,
      expected,
    );
  async function until(check) {
    for (let i = 0; i < 200; i++) {
      if (check()) return;
      await page.waitForTimeout(20);
    }
    assert.fail("Server operation did not arrive");
  }
  await count(80);
  assert.equal(queries.length, 0);
  await page.evaluate(() => window.more());
  await count(180);
  await page.evaluate(() => window.more());
  await count(250);
  datasets.a = [
    ...Array.from({ length: 200 }, (_, i) => row("fresh-" + i, i + 1, "a")),
    ...datasets.a,
  ];
  [...sockets][0].close();
  await count(330);
  assert.equal(
    await page.evaluate(() => window.view().hasMore),
    true,
    "The unread burst must reopen pagination",
  );
  const before = queries.length;
  holdBaseline = true;
  const beforeWatch = watches.length;
  [...sockets][0].close();
  await until(() => watches.length > beforeWatch);
  await page.evaluate(() => window.more());
  await page.waitForTimeout(80);
  assert.equal(
    queries.length,
    before,
    "No page is sent before the reconnect baseline",
  );
  sendBaseline(watches.at(-1));
  holdBaseline = false;
  await count(430);
  assert.equal(queries.length, before + 1);
  assert.ok(
    !cancelled.includes(queries.at(-1).id),
    "Fresh reconnect pages are not cancelled and repeated",
  );
  assert.deepEqual(queries.at(-1).query.range, {
    start: key(81),
    end: key(1000),
  });
  await page.evaluate(() => window.more());
  await count(450);
  assert.equal(await page.evaluate(() => window.view().hasMore), false);
  assert.equal(
    queries.length,
    before + 2,
    "Only the missing 120 executions are queried",
  );
  await page.evaluate(() => window.select("b"));
  await count(80);
  holdQuery = true;
  const oldQueries = queries.length;
  await page.evaluate(() => window.more());
  await until(() => queries.length > oldQueries);
  const stale = queries.at(-1);
  await page.evaluate(() => window.select("c"));
  await count(80);
  assert.ok(cancelled.includes(stale.id));
  sendQuery(stale);
  holdQuery = false;
  await page.waitForTimeout(100);
  assert.equal(
    await page.evaluate(() =>
      window.view().orders.every((row) => row.book.collection_id === "c"),
    ),
    true,
  );
  const current = [...ops.values()].find(
    (op) => op.baseline && op.query.filters.collection_id === "c",
  );
  holdBaseline = true;
  current.socket.send(
    JSON.stringify({
      type: "error",
      id: current.id,
      error: { code: "capacity_exceeded", message: "Narrow the range" },
    }),
  );
  await page.waitForFunction(() => window.view().error === "Narrow the range");
  current.socket.send(
    JSON.stringify({ type: "market.status", status: "delayed" }),
  );
  current.socket.send(
    JSON.stringify({ type: "market.status", status: "available" }),
  );
  await page.waitForTimeout(30);
  assert.notEqual(await page.evaluate(() => window.view().status), "Live");
  await until(() => watches.at(-1) !== current);
  assert.notEqual(await page.evaluate(() => window.view().status), "Live");
  sendBaseline(watches.at(-1));
  holdBaseline = false;
  await page.waitForFunction(() => window.view().status === "Live");
  assert.deepEqual(errors, []);
  console.log(
    "Live feed browser passed: stable-key burst recovery, bounded gap reads, queued reconnect pagination, honest status and late collection-response cancellation.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
