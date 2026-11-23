import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
import { floorReport } from "./valuation-fixture.mjs";
const cacheDir = await mkdtemp(join(tmpdir(), "slug-preview-"));
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/src/style.css"></head><body><div id="fixture"></div><script type="module">
import React,{useState} from 'react';import{createRoot}from'react-dom/client';
import{SlugDetails}from'/src/orderbook/slug-details.tsx';import{FloorsProvider}from'/src/valuation/floors.tsx';
const options=new Map([['m',{id:'m',kind:'model',name:'Model',media:null}],['b',{id:'b',kind:'backdrop',name:'Backdrop',palette:{center:'#70a0bb',edge:'#527e9c',pattern:'#345678'}}]]);const names=new Map([['c','Gift']]);
function App(){const[slug,setSlug]=useState('Gift-1'),[subject,onGift]=useState();window.choose=setSlug;return React.createElement(FloorsProvider,{currency:'GRAM',collection:'c',collections:['c'],order:subject},slug&&React.createElement(SlugDetails,{slug,collection:'c',currency:'GRAM',options,names,onGift,close:()=>setSlug('')}));}
createRoot(document.getElementById('fixture')).render(React.createElement(App));</script></body></html>`;
const server = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "slug-preview",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url !== "/__slug") return next();
          res.setHeader("Content-Type", "text/html");
          res.end(await server.transformIndexHtml(req.url, html));
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
    viewport: { width: 390, height: 844 },
    reducedMotion: "reduce",
  });
  page.setDefaultTimeout(10000);
  const errors = [],
    queries = [],
    cancelled = [];
  let held;
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/valuations/strategies", (route) =>
    route.fulfill({
      json: [
        {
          id: "gift_floor",
          name: "Gift floor",
          description: "Estimate",
          purpose: "estimate",
          revision: "r1",
          documentation_revision: "d1",
          documentation_url: "/guide",
          input: {
            targets: ["gift", "gift_attributes"],
            period: "unsupported",
          },
          display: {
            fields: [
              {
                id: "final_mark",
                label: "Estimate",
                primary: true,
                format: { kind: "currency", decimals: 2 },
              },
            ],
            evidence: [],
            note: "",
          },
        },
      ],
    }),
  );
  await page.route("**/guide", (route) =>
    route.fulfill({
      body: "# Calculation\n\nThe shared calculation guide.",
      contentType: "text/markdown",
    }),
  );
  await page.routeWebSocket("**/ws", (socket) => {
    socket.send(JSON.stringify({ type: "market.status", status: "available" }));
    socket.onMessage((message) => {
      const command = JSON.parse(String(message));
      if (command.type === "cancel") {
        cancelled.push(command.id);
        return;
      }
      if (command.type === "valuation.watch") {
        socket.send(
          JSON.stringify({
            type: "valuation.snapshot",
            id: command.id,
            data: floorReport(command.params, ["m"], ["b"]),
          }),
        );
        return;
      }
      if (command.type !== "request" || command.method !== "valuation.query")
        return;
      if (command.params.target.kind === "catalog") {
        socket.send(
          JSON.stringify({
            type: "result",
            id: command.id,
            method: command.method,
            data: floorReport(command.params, ["m"], ["b"]),
          }),
        );
        return;
      }
      queries.push(command);
      const slug = command.params.target.gift;
      const number = Number(slug.split("-")[1]);
      const send = () =>
        socket.send(
          JSON.stringify({
            type: "result",
            id: command.id,
            method: command.method,
            data: {
              target: {
                kind: "gift",
                slug,
                gift: {
                  id: "g" + number,
                  collection_id: "c",
                  number,
                  traits: { model: "m", backdrop: "b" },
                },
              },
              currency_id: "GRAM",
              evaluated_at: new Date().toISOString(),
              stale: false,
              results: [
                {
                  strategy: "gift_floor",
                  revision: "r1",
                  data: {
                    final_mark: { value: String(number * 10) },
                    collection_floor: { value: "8", count: 7 },
                    model_floor: { value: "10.000000001", count: 2 },
                    backdrop_floor: { value: "9", count: 1 },
                    model_backdrop_floor: { value: "12.5", count: 1 },
                  },
                },
              ],
            },
          }),
        );
      if (number === 1) held = { send, id: command.id };
      else send();
    });
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__slug`,
  );
  await page.waitForFunction(() =>
    document.querySelector('[role="dialog"],dialog'),
  );
  for (let n = 0; n < 100 && !held; n++) await page.waitForTimeout(10);
  assert.ok(held);
  await page.evaluate(() => window.choose("Gift-2"));
  await page.getByText("#2", { exact: true }).waitFor();
  assert.ok(
    cancelled.includes(held.id),
    "switching gift cancels the old request",
  );
  held.send();
  await page.waitForTimeout(100);
  assert.equal(await page.getByText("#1", { exact: true }).count(), 0);
  assert.equal(queries.length, 2, "one valuation request per selected slug");
  for (const viewport of [
    { width: 390, height: 844 },
    { width: 1440, height: 900 },
  ]) {
    await page.setViewportSize(viewport);
    await page.waitForTimeout(200);
    const dialog = page.locator("dialog");
    assert.ok(await dialog.isVisible());
    assert.ok(
      await page
        .getByRole("button", { name: "Close details", exact: true })
        .isVisible(),
    );
    assert.equal(
      await page.getByText("Ask price", { exact: true }).count(),
      0,
      "unlisted gifts never fabricate an asking price",
    );
    assert.equal(await page.locator(".valuation-primary-result").count(), 1);
    await page.screenshot({ path: `/tmp/slug-preview-${viewport.width}.png` });
  }
  await page
    .getByRole("button", { name: "How is this calculated?", exact: true })
    .click();
  await page.getByText("The shared calculation guide.").waitFor();
  await page
    .getByRole("button", { name: "Close calculation guide", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  await page.waitForTimeout(200);
  assert.equal(await page.locator("dialog").count(), 0);
  assert.deepEqual(errors, []);
  console.log(
    "Slug preview: mobile, desktop, superseded requests, calculation popup and close passed",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
