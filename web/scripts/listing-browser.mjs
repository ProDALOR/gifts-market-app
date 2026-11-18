// Live transitions and valuation lifecycle against isolated React components and a fake feed.
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "vite";
import { chromium } from "playwright";
const root = new URL("..", import.meta.url).pathname;
const html = `<!doctype html><html><body><div id="fixture"></div><script type="module">
import React,{useState,useEffect} from 'react';
import {createRoot} from 'react-dom/client';
import {useOrderFeed} from '/src/use-order-feed.ts';
import {GiftGrid} from '/src/orderbook/gift-grid.tsx';
import {OrderDetails} from '/src/orderbook/order-details.tsx';
import {orderKey} from '/src/orderbook/order-activity.ts';
import '/src/style.css';
const query={kind:'orders',query:{filters:{collection_id:'gift',currency_id:'GRAM',venue_id:null,models:[],backdrops:[],patterns:[],number:null,price:null},side:'ask',sort:'time_desc',consistency:'live',limit:50,page_token:''}};
const options=new Map([
['gold',{kind:'model',id:'gold',name:'Gold',media:{thumbnail_url:'/__gift.svg',animation_url:null}}],
['blue',{kind:'backdrop',id:'blue',name:'Sapphire',palette:{center:'#759fb4',edge:'#364e73',pattern:'#283d60'}}]
]);
const names=new Map([['gift','Gift'],['gold','Gold'],['blue','Sapphire'],['venue','MRKT']]);
function App(){const feed=useOrderFeed(query,false,0);const [selected,select]=useState();
window.selectListing=select;window.feedView=()=>feed;
useEffect(()=>{select(old=>old ? feed.displayOrders.find(row=>orderKey(row)===orderKey(old))||old:undefined)},[feed.displayOrders]);
return React.createElement('main',{className:'workspace'},React.createElement('div',{className:'orderbook'},
React.createElement('h2',{style:{margin:'14px 0'}},'Live gift market'),
React.createElement(GiftGrid,{orders:feed.displayOrders,busy:feed.loadingMore,visible:feed.setVisibleRange,card:order=>({options,collectionName:'Gift',venueName:'MRKT',selected:selected?.id===order.id,open:select,activity:feed.activity.get(orderKey(order))})})),
selected&&React.createElement(OrderDetails,{selected,options,names,collectionName:'Gift',removed:!feed.page.data.some(row=>orderKey(row)===orderKey(selected)),close:()=>select(undefined)}));}
const app=createRoot(document.getElementById('fixture'));app.render(React.createElement(App));window.unmount=()=>app.unmount();
</script></body></html>`;
const methodology = await readFile(
  new URL(
    "../../crates/market-valuation/src/gift_floor/methodology.md",
    import.meta.url,
  ),
  "utf8",
);
const cacheDir = await mkdtemp(join(tmpdir(), "market-listings-"));
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "listing-fixture",
      configureServer(server) {
        server.middlewares.use("/__listing_test", async (_, res, next) => {
          try {
            res.setHeader("Content-Type", "text/html");
            // Vite uses style elements in development; block style attributes as production does.
            res.setHeader(
              "Content-Security-Policy",
              "style-src-attr 'none'; style-src-elem 'self' 'unsafe-inline'",
            );
            res.end(await server.transformIndexHtml("/__listing_test", html));
          } catch (error) {
            next(error);
          }
        });
      },
    },
  ],
});
const order = (id, number) => ({
  id,
  book: { venue_id: "venue", collection_id: "gift", currency_id: "GRAM" },
  kind: "ask",
  target: {
    kind: "gift",
    gift: {
      id: `g${number}`,
      collection_id: "gift",
      number,
      traits: { model: "gold", backdrop: "blue", pattern: null },
    },
  },
  price: String(number * 10),
  quantity: 1,
  observed_at: "2026-11-18T12:00:00Z",
  executed_at: null,
  expires_at: null,
  market_url: "https://example.com/gift",
});
const sync = { ready: true, state: "live", updated_at: null };
const a = order("a", 1),
  b = order("b", 2),
  x = order("x", 3);
const frame = (rows) => ({
  type: "page",
  id: "offers",
  revision: JSON.stringify(rows.map((row) => row.id)),
  sync,
  page: { data: rows, next_page_token: "", cursor: "view", sync },
});
function report(body) {
  const number = body.gift.number;
  const missing = {
    error: {
      code: "not_enough_data",
      message: "More backdrop listings are needed.",
      required_samples: 3,
      available_samples: 1,
    },
  };
  return {
    strategy: "fixture_floor",
    revision: "view",
    evaluated_at: "2026-11-18T12:00:00Z",
    slug: null,
    gift: order("gift", number).target.gift,
    currency_id: body.currency_id,
    sync,
    collection_floor: { value: "8" },
    model_floor: { value: String(number * 10) },
    backdrop_floor: { value: "9" },
    model_backdrop_floor: { value: "12" },
    model_multiplier: { value: "1.25" },
    backdrop_multiplier: missing,
    final_mark: missing,
    display: {
      fields: [
        {
          id: "confidence",
          label: "Fixture confidence",
          primary: false,
          value: { kind: "number", amount: "87.500", suffix: "%" },
        },
        {
          id: "decision",
          label: "Fixture decision",
          primary: true,
          value: {
            kind: "unavailable",
            label: "Not enough data",
            message: missing.error.message,
            detail: "1 of 3 fixture observations",
          },
        },
        {
          id: "model_floor",
          label: "Fixture price",
          primary: false,
          value: {
            kind: "currency",
            amount: String(number * 10),
            currency_id: body.currency_id,
          },
        },
        {
          id: "model_multiplier",
          label: "Fixture score",
          primary: false,
          value: { kind: "number", amount: "1.2500", suffix: " points" },
        },
      ],
      summary: "5 fixture observations",
      note: "A note owned by the fixture strategy.",
    },
    evidence: {
      collection_samples: 5,
      model_samples: 3,
      backdrop_samples: 1,
      model_backdrop_samples: 1,
      minimum_multiplier_samples: 3,
    },
  };
}
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 440, height: 880 } });
  const errors = [];
  let guideModuleRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/src/orderbook/valuation-methodology.tsx"))
      guideModuleRequests++;
  });
  page.on("pageerror", (error) => {
    errors.push(error.message);
    console.error(error);
  });
  await page.clock.install({ time: new Date("2026-11-18T12:00:00Z") });
  await page.route("**/__gift.svg", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: `<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128"><defs><linearGradient id="g" x2="1" y2="1"><stop stop-color="#fff0a2"/><stop offset="1" stop-color="#e4a536"/></linearGradient></defs><path d="M23 49h82v62H23z" fill="url(#g)"/><path d="M17 37h94v20H17z" fill="#f3d273"/><path d="M57 38h15v73H57z" fill="#fdf5cf"/><path d="M63 36c-38 0-31-33-11-18 8 5 11 18 11 18Zm2 0c38 0 31-33 11-18-8 5-11 18-11 18Z" fill="none" stroke="#ffefb0" stroke-width="8" stroke-linejoin="round"/></svg>`,
    }),
  );
  let metadataFails = true;
  let metadataRequests = 0;
  const strategyMetadata = {
    id: "fixture_floor",
    name: "Fixture floor",
    description: "Discovered strategy description.",
    documentation_url: "/__fixture_strategy.md",
    documentation_revision: "r1",
  };
  await page.route("**/valuations/strategies", (route) => {
    metadataRequests++;
    return route.fulfill(
      metadataFails
        ? { status: 503, json: { error: { message: "Unavailable" } } }
        : { json: [strategyMetadata] },
    );
  });
  let guideRequests = 0;
  let guideFails = true;
  await page.route("**/__fixture_strategy.md", async (route) => {
    guideRequests++;
    await route.fulfill(
      guideFails
        ? { status: 503, body: "Unavailable" }
        : {
            contentType: "text/markdown; charset=utf-8",
            body:
              methodology +
              `\n\nGuide revision ${strategyMetadata.documentation_revision}.\n` +
              "\n<script>window.guideExecuted=true</script>\n\n[Unsafe](javascript:alert(1))",
          },
    );
  });
  let socket;
  await page.routeWebSocket("**/ws", (peer) => {
    socket = peer;
    peer.onMessage((message) => {
      if (JSON.parse(String(message)).type === "subscribe")
        peer.send(JSON.stringify(frame([a, b])));
    });
  });
  let mode = "ok",
    release,
    requests = [];
  await page.route("**/valuations/gift", async (route) => {
    const body = route.request().postDataJSON();
    requests.push(body);
    if (mode === "hold")
      await new Promise((resolve) => {
        release = resolve;
      });
    try {
      await route.fulfill(
        mode === "error"
          ? {
              status: 503,
              json: {
                error: { code: "unavailable", message: "Please try again" },
              },
            }
          : { json: report(body) },
      );
    } catch {
      /* Request may have been cancelled when the detail sheet closed. */
    }
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__listing_test`,
  );
  await page.locator(".gift-card").nth(1).waitFor();
  assert.equal(requests.length, 0);
  assert.equal(await page.locator(".activity-ribbon").count(), 0);
  socket.send(JSON.stringify(frame([x, a, b])));
  await page.locator('[data-order-id="x"]').waitFor();
  assert.equal(
    await page.locator(".is-new .activity-ribbon").textContent(),
    "New",
  );
  const clock = page.locator(".is-new .card-clock");
  assert.equal(
    await clock.evaluate((el) => getComputedStyle(el).animationDuration),
    "60s",
  );
  await clock.evaluate((el) => {
    el.getAnimations()[0].currentTime = 15000;
  });
  assert.ok(
    await clock.evaluate(
      (el) =>
        parseFloat(getComputedStyle(el).getPropertyValue("--clock-angle")) > 60,
    ),
  );
  await mkdir("/tmp/gifts-market-ui", { recursive: true });
  await page.screenshot({ path: "/tmp/gifts-market-ui/fresh-listings.png" });
  await page.locator('[data-order-id="a"]').click();
  await page.locator('[data-metric="model_floor"] .valuation-value').waitFor();
  assert.equal(requests.length, 1);
  assert.equal(guideRequests, 0, "Guide is not requested while collapsed");
  assert.deepEqual(requests[0], {
    gift: { collection_id: "gift", number: 1 },
    currency_id: "GRAM",
  });
  assert.equal(await page.locator(".valuation-metrics > li").count(), 4);
  assert.deepEqual(
    await page.locator(".valuation-metrics > li h4").allTextContents(),
    [
      "Fixture confidence",
      "Fixture decision",
      "Fixture price",
      "Fixture score",
    ],
  );
  assert.equal(
    await page
      .locator('[data-metric="confidence"] .valuation-value')
      .textContent(),
    "87.500%",
  );
  assert.equal(
    await page.locator('[data-metric="decision"]').getAttribute("class"),
    "valuation-primary",
  );
  await page.getByText("5 fixture observations", { exact: false }).waitFor();
  await page
    .getByText("A note owned by the fixture strategy.", { exact: true })
    .waitFor();
  assert.equal(
    await page
      .locator('[data-metric="model_multiplier"] .valuation-value')
      .textContent(),
    "1.2500 points",
  );
  await page
    .locator(".gift-valuations")
    .getByText("Not enough data", { exact: true })
    .first()
    .waitFor();
  for (const label of [
    "Listing information",
    "Last observed",
    "Venue",
    "Expires",
    "Order ID",
    "Gift ID",
  ])
    assert.equal(
      await page.locator(".detail").getByText(label, { exact: true }).count(),
      0,
    );
  socket.send(JSON.stringify(frame([x, b])));
  await page.locator(".is-removed").waitFor();
  assert.equal(
    await page.locator(".is-removed .activity-ribbon").textContent(),
    "Gone",
  );
  assert.ok(
    (
      await page
        .locator(".is-removed .card-preview")
        .evaluate((el) => getComputedStyle(el).filter)
    ).includes("grayscale(1)"),
  );
  await page.locator(".listing-gone").waitFor();
  assert.equal(await page.locator(".detail-market").count(), 0);
  assert.equal(requests.length, 1, "Feed changes do not poll valuations");
  await page.screenshot({ path: "/tmp/gifts-market-ui/valuation-preview.png" });
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.waitForFunction(() => !document.querySelector(".card-clock"));
  assert.equal(await page.locator(".card-clock").count(), 0);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.clock.runFor(61000);
  await page.waitForFunction(() => !document.querySelector(".activity-ribbon"));
  assert.equal(await page.locator('[data-order-id="a"]').count(), 0);
  assert.equal(requests.length, 1);
  // A slow response must not populate the next gift's panel, even without unmounting.
  mode = "hold";
  socket.send(JSON.stringify(frame([a, b])));
  await page.locator('[data-order-id="a"]').waitFor();
  await page.locator('[data-order-id="a"]').click();
  await page.waitForFunction(
    () =>
      document.querySelector(".gift-valuations")?.getAttribute("aria-busy") ===
      "true",
  );
  for (let tries = 0; !release && tries < 100; tries++)
    await new Promise((resolve) => setTimeout(resolve, 10));
  assert.ok(release, "The delayed valuation request started");
  mode = "ok";
  await page.evaluate((order) => window.selectListing(order), b);
  await page.locator('[data-metric="model_floor"] .valuation-value').waitFor();
  release();
  assert.ok((await page.locator("#detail-title").textContent()).includes("#2"));
  assert.equal(
    await page
      .locator('[data-metric="model_floor"] .quote-amount')
      .textContent(),
    "20",
  );
  mode = "error";
  await page
    .getByRole("button", { name: "Refresh valuation", exact: true })
    .click();
  await page
    .locator('.gift-valuations [role="alert"]')
    .getByText("Please try again", { exact: true })
    .waitFor();
  mode = "ok";
  await page
    .getByRole("button", { name: "Refresh valuation", exact: true })
    .click();
  await page.locator('[data-metric="model_floor"] .valuation-value').waitFor();
  assert.equal(
    guideModuleRequests,
    0,
    "The Markdown and math renderer stay unloaded until the guide opens",
  );
  const guide = page.locator(".valuation-guide");
  const summary = guide.locator("summary");
  await summary.focus();
  await page.keyboard.press("Enter");
  await guide
    .getByRole("button", { name: "Retry strategy information" })
    .waitFor();
  assert.equal(
    guideRequests,
    0,
    "No hardcoded documentation fallback while discovery fails",
  );
  const previousMetadataRequests = metadataRequests;
  metadataFails = false;
  await guide
    .getByRole("button", { name: "Retry strategy information" })
    .click();
  await page
    .locator(".valuation-heading")
    .getByRole("heading", { name: strategyMetadata.name, exact: true })
    .waitFor();
  assert.equal(metadataRequests, previousMetadataRequests + 1);
  await guide.getByRole("button", { name: "Retry guide" }).waitFor();
  guideFails = false;
  await guide.getByRole("button", { name: "Retry guide" }).click();
  const content = guide.getByRole("article", {
    name: `${strategyMetadata.name} calculation guide`,
  });
  await content
    .getByRole("heading", { name: "Gift floor valuation", exact: true })
    .waitFor();
  assert.equal(guideRequests, 2, "Guide retries after an error");
  assert.ok(
    guideModuleRequests > 0,
    "The renderer loads only after discovery and expansion",
  );
  assert.ok(
    (await content.locator(".katex").count()) >= 3,
    "Inline and block LaTeX formulas render",
  );
  assert.ok(
    (await content.locator("math").count()) >= 3,
    "MathML remains available for accessibility",
  );
  assert.equal(
    await content.locator("[style]").count(),
    0,
    "Native MathML needs no inline styles under the production content policy",
  );
  assert.equal(
    await content.locator(".katex-error").count(),
    0,
    "The strategy guide has valid formulas",
  );
  await page.evaluate(() => document.fonts.ready);
  assert.ok((await content.textContent()).includes("midpoint-to-even"));
  assert.equal(await content.locator("script").count(), 0);
  assert.equal(await page.evaluate(() => window.guideExecuted), undefined);
  assert.equal(
    await content
      .getByRole("link", { name: "Unsafe", exact: true })
      .getAttribute("href"),
    "",
  );
  await summary.click();
  await summary.click();
  assert.equal(
    guideRequests,
    2,
    "Collapsing and reopening reuses the mounted guide",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  await content
    .getByRole("heading", { name: "Final mark", exact: true })
    .scrollIntoViewIfNeeded();
  assert.equal(
    await page
      .locator(".detail")
      .evaluate((el) => el.scrollWidth > el.clientWidth),
    false,
    "Guide formulas fit mobile details",
  );
  await page.screenshot({
    path: "/tmp/gifts-market-ui/valuation-methodology.png",
  });
  const cachedMetadataRequests = metadataRequests;
  const cachedGuideRequests = guideRequests;
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  await page.evaluate((order) => window.selectListing(order), b);
  await page.locator('[data-metric="model_floor"] .valuation-value').waitFor();
  await page.locator(".valuation-guide summary").click();
  await page
    .getByRole("article", {
      name: `${strategyMetadata.name} calculation guide`,
    })
    .waitFor();
  assert.equal(
    metadataRequests,
    cachedMetadataRequests,
    "Reopening details reuses fresh metadata",
  );
  assert.equal(
    guideRequests,
    cachedGuideRequests,
    "Reopening details reuses rendered math without a new download",
  );
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  strategyMetadata.documentation_revision = "r2";
  strategyMetadata.name = "Updated fixture floor";
  await page.clock.runFor(60_001);
  await page.evaluate((order) => window.selectListing(order), b);
  await page
    .locator(".valuation-heading")
    .getByRole("heading", { name: strategyMetadata.name, exact: true })
    .waitFor();
  await page.locator(".valuation-guide summary").click();
  await page
    .getByRole("article", {
      name: `${strategyMetadata.name} calculation guide`,
    })
    .getByText("Guide revision r2.", { exact: true })
    .waitFor();
  assert.equal(
    metadataRequests,
    cachedMetadataRequests + 1,
    "Expired metadata revalidates on use",
  );
  assert.equal(
    guideRequests,
    cachedGuideRequests + 1,
    "New documentation revision bypasses the old rendered guide",
  );
  await page.evaluate(() => window.unmount());
  assert.deepEqual(errors, []);
  console.log(
    "Listing browser passed: 60-second clock, new/grey ribbons, bounded expiry, unified report, exact units, cancelled requests, retry, strategy discovery and lazy Markdown/LaTeX calculation guide.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
