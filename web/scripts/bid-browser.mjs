import { fixtureDisplay, bestBidResult } from "./valuation-fixture.mjs";
// Bid cards stay static; details share bounded playback with gift and filter previews.
import assert from "node:assert/strict";
import { gzipSync } from "node:zlib";
import { criteriaReport } from "./valuation-fixture.mjs";
import { readFile, mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const root = fileURLToPath(new URL("..", import.meta.url));
const output = fileURLToPath(new URL("../../.market/run", import.meta.url));
const artwork = await readFile(
  new URL("../src/brand/liberty-figure.svg", import.meta.url),
);
const animation = gzipSync(
  JSON.stringify({
    v: "5.7.4",
    w: 128,
    h: 128,
    fr: 30,
    ip: 0,
    op: 60,
    assets: [],
    layers: [
      {
        ddd: 0,
        ind: 1,
        ty: 1,
        nm: "Moving square",
        sr: 1,
        ks: {
          o: { a: 0, k: 100 },
          r: { a: 0, k: 0 },
          a: { a: 0, k: [8, 8, 0] },
          s: { a: 0, k: [100, 100, 100] },
          p: {
            a: 1,
            k: [
              {
                t: 0,
                s: [32, 64, 0],
                e: [96, 64, 0],
                o: { x: 0.33, y: 0 },
                i: { x: 0.67, y: 1 },
              },
              { t: 59, s: [96, 64, 0] },
            ],
          },
        },
        sw: 16,
        sh: 16,
        sc: "#ffffff",
        ao: 0,
        ip: 0,
        op: 60,
        st: 0,
        bm: 0,
      },
    ],
  }),
);
const cacheDir = await mkdtemp(join(tmpdir(), "market-bids-"));
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body><div id="fixture"></div><script type="module">
import React,{useState}from'react';import{createRoot}from'react-dom/client';
import{GiftGrid}from'/src/orderbook/gift-grid.tsx';import{OrderDetails}from'/src/orderbook/order-details.tsx';
const models=Array.from({length:8},(_,i)=>({kind:'model',id:'model-'+i,rarity:i===0?{kind:'legendary'}:null,name:i===0?'Artisan Brick':'Model '+i,media:{thumbnail_url:'/media/model-'+i+'/thumbnail',preview_url:'/media/model-'+i+'/preview',animation_url:'/media/model-'+i+'/animation'}}));
const backdrops=Array.from({length:3},(_,i)=>({kind:'backdrop',id:'backdrop-'+i,rarity:i===0?{kind:'percentage',permille:20}:null,name:['Sapphire','Lavender','Amber'][i],palette:{center:['#759fb4','#b789e4','#e2c585'][i],edge:'#364e73',pattern:'#283d60'}}));
const patterns=Array.from({length:4},(_,i)=>({kind:'pattern',id:'pattern-'+i,rarity:i===0?{kind:'percentage',permille:10}:null,name:'Star '+i,media:{thumbnail_url:'/media/pattern-'+i+'/thumbnail',animation_url:'/media/pattern-'+i+'/animation'}}));
const options=new Map([...models,...backdrops,...patterns].map(o=>[o.id,o]));
const names=new Map([...options.values()].map(o=>[o.id,o.name]));names.set('gift','Liberty Figure');names.set('venue','MRKT');
const base={id:'bid',book:{collection_id:'gift',venue_id:'venue',currency_id:'GRAM'},kind:'bid',price:'12.34',quantity:8,observed_at:'2026-11-20T00:00:00Z',market_url:'https://example.com/bid',target:{kind:'criteria',models:models.map(o=>o.id),backdrops:backdrops.map(o=>o.id),patterns:patterns.map(o=>o.id),number:{min:1,max:1000}}};
const any={...base,id:'any',quantity:1,target:{kind:'criteria',models:[],backdrops:[],patterns:[]}};
const missing={...base,id:'missing',quantity:2,target:{kind:'criteria',models:['missing','pattern-0'],backdrops:[],patterns:[]}};
const concrete={...base,id:'concrete',quantity:1,target:{kind:'gift',id:'gift-42',number:42,traits:{model:'model-0',backdrop:'backdrop-0',pattern:'pattern-0'}}};
const single={...base,id:'single',target:{kind:'criteria',models:['model-0'],backdrops:['backdrop-0'],patterns:['pattern-0']}};
const backdrop={...base,id:'backdrop',target:{kind:'criteria',models:[],backdrops:['backdrop-1'],patterns:['pattern-1']}};
const largeModels=Array.from({length:200},(_,i)=>({kind:'model',id:'large-model-'+i,name:'Large model '+i,media:{preview_url:'/media/large-model-'+i+'/preview',thumbnail_url:'/media/large-model-'+i+'/thumbnail',animation_url:'/media/large-model-'+i+'/animation'}}));
const largeBackdrops=Array.from({length:10},(_,i)=>({...backdrops[i%3],id:'large-backdrop-'+i,name:'Large backdrop '+i}));
const largePatterns=Array.from({length:10},(_,i)=>({...patterns[i%4],id:'large-pattern-'+i,name:'Large symbol '+i,media:{thumbnail_url:'/media/large-pattern-'+i+'/thumbnail'}}));
for(const o of [...largeModels,...largeBackdrops,...largePatterns]){options.set(o.id,o);names.set(o.id,o.name);}names.set('known-missing','Remembered model');
const large={...base,id:'large',target:{kind:'criteria',models:[...largeModels.map(o=>o.id),'known-missing'],backdrops:[...largeBackdrops.map(o=>o.id),'missing-backdrop'],patterns:[...largePatterns.map(o=>o.id),'missing-pattern'],number:base.target.number}};
function App(){const[selected,select]=useState();const[catalog,setCatalog]=useState(options);const[largeMode,setLargeMode]=useState(false);window.selectBid=select;window.showLargeCard=()=>setLargeMode(true);window.showModelOnlyCard=()=>setLargeMode("models");window.restoreCatalog=()=>setCatalog(previous=>new Map([...previous,['known-missing',{kind:'model',id:'known-missing',name:'Restored model',media:{preview_url:'/media/restored-model/preview',thumbnail_url:'/media/restored-model/thumbnail'}}]]));return React.createElement('main',{className:'orderbook'},React.createElement('section',{className:'market-results'},React.createElement('h1',null,'Bids'),React.createElement(GiftGrid,{orders:largeMode==="models"?[{...base,id:"model-only",target:{...base.target,backdrops:[],patterns:[]}}]:largeMode?[large]:[base,any,missing,concrete,single,backdrop],busy:false,visible:()=>{},card:()=>({options:catalog,collectionName:'Liberty Figure',venueName:'MRKT',selected:false,open:select})})),selected&&React.createElement(OrderDetails,{selected,options:catalog,names,collectionName:'Liberty Figure',close:()=>select(undefined)}));}
createRoot(document.getElementById('fixture')).render(React.createElement(App));
</script></body></html>`;
let symbol;
const requests = [];
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "bid-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url === "/__bids") {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
            return;
          }
          if (req.url === "/valuations/strategies") {
            res.setHeader("Content-Type", "application/json");
            res.end(
              JSON.stringify([
                {
                  purpose: "reference",
                  id: "ask_floor",
                  revision: "r1",
                  display: fixtureDisplay("ask_floor"),
                  name: "Matching asks",
                  description: "Fixture",
                  input: { targets: ["selection"], period: "unsupported" },
                  documentation_url: "/guide",
                  documentation_revision: "1",
                },
                {
                  purpose: "quote",
                  id: "best_bid",
                  revision: "r1",
                  display: fixtureDisplay("best_bid"),
                  name: "Best bid",
                  description: "Fixture",
                  input: {
                    targets: ["selection", "gift", "gift_attributes"],
                    period: "unsupported",
                  },
                  documentation_url: "/guide",
                  documentation_revision: "1",
                },
              ]),
            );
            return;
          }
          if (req.url === "/guide") {
            res.setHeader("Content-Type", "text/markdown");
            res.end("# Matching asks\nCurrent asks for the accepted gifts.");
            return;
          }
          if (!req.url.startsWith("/media/")) return next();
          requests.push(req.url);
          if (req.url.endsWith("/animation")) {
            assert.ok(!req.url.includes("pattern-"), "Symbols remain static");
            res.setHeader("Content-Type", "application/x-tgsticker");
            res.setHeader("Cache-Control", "public,max-age=86400");
            res.end(animation);
            return;
          }
          const pattern = req.url.includes("pattern-");
          res.setHeader(
            "Content-Type",
            pattern ? "image/png" : "image/svg+xml",
          );
          res.setHeader("Cache-Control", "public,max-age=86400");
          res.end(pattern ? symbol : artwork);
        });
      },
    },
  ],
});
let browser;
try {
  await server.listen();
  await mkdir(output, { recursive: true });
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
  await page.addInitScript(() =>
    localStorage.setItem("market.display-mode", "quality"),
  );
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "hardwareConcurrency", {
      configurable: true,
      get: () => 4,
    });
    Object.defineProperty(navigator, "deviceMemory", {
      configurable: true,
      get: () => 4,
    });
  });
  symbol = Buffer.from(
    await page.evaluate(() => {
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 64;
      const c = canvas.getContext("2d");
      c.fillStyle = "#302d36";
      c.beginPath();
      c.moveTo(32, 4);
      c.lineTo(40, 24);
      c.lineTo(60, 32);
      c.lineTo(40, 40);
      c.lineTo(32, 60);
      c.lineTo(24, 40);
      c.lineTo(4, 32);
      c.lineTo(24, 24);
      c.closePath();
      c.fill();
      return canvas.toDataURL().split(",")[1];
    }),
    "base64",
  );
  page.setDefaultTimeout(10000);
  await page.routeWebSocket("**/ws", (peer) => {
    peer.send(JSON.stringify({ type: "market.status", status: "available" }));
    peer.onMessage((message) => {
      const command = JSON.parse(String(message));
      if (command.type === "request" && command.method === "valuation.query")
        peer.send(
          JSON.stringify({
            type: "result",
            id: command.id,
            method: command.method,
            data: (() => {
              const report =
                command.params.target.kind === "selection"
                  ? criteriaReport(command.params)
                  : {
                      target: command.params.target,
                      currency_id: command.params.currency_id,
                      evaluated_at: new Date().toISOString(),
                      stale: false,
                      results: [],
                    };
              if (command.params.strategies.includes("best_bid"))
                report.results.push(bestBidResult());
              return report;
            })(),
          }),
        );
    });
  });
  const errors = [],
    unexpected = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("request", (req) => {
    const url = new URL(req.url());
    if (
      ["xhr", "fetch"].includes(req.resourceType()) &&
      !url.pathname.startsWith("/@") &&
      !url.pathname.startsWith("/node_modules/") &&
      !url.pathname.startsWith("/src/") &&
      !url.pathname.startsWith("/media/")
    )
      unexpected.push(url.pathname);
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__bids`,
  );
  const card = page
    .locator(".gift-card")
    .filter({ has: page.locator('[data-order-id="bid"]') });
  const checkLayout = async () => {
    assert.equal(
      await card.evaluate((node) => {
        const models = node
          .querySelector(".preview-models")
          .getBoundingClientRect();
        const summary = node
          .querySelector(".bid-preview-summary")
          .getBoundingClientRect();
        const number = node
          .querySelector(".bid-number")
          .getBoundingClientRect();
        return models.bottom <= summary.top && summary.bottom <= number.top;
      }),
      true,
      "Artwork, overflow and number must not overlap",
    );
  };
  await card.locator("img.loaded").first().waitFor();
  assert.equal(await card.locator(".preview-model").count(), 4);
  assert.equal(await card.locator(".preview-band").count(), 3);
  await card.locator(".preview-symbols canvas").waitFor();
  await page.waitForFunction(
    () =>
      document.querySelector(".bid-card .preview-symbols canvas")?.width > 1,
  );
  assert.deepEqual(
    await card.locator(".bid-preview-summary span").allTextContents(),
    ["+4 models"],
  );
  assert.equal(await card.locator(".condition-chip,.reset-traits").count(), 0);
  assert.match(
    await card.locator("figure").getAttribute("aria-label"),
    /Model 7/,
  );
  assert.equal(await card.locator(".serial").textContent(), "×8");
  assert.match(await card.locator(".bid-number").textContent(), /#1 – 1\s000/);
  const any = page
    .locator(".gift-card")
    .filter({ has: page.locator('[data-order-id="any"]') });
  await any.getByRole("img", { name: "Any model", exact: true }).waitFor();
  await any.getByRole("img", { name: "Any backdrop", exact: true }).waitFor();
  await any.getByRole("img", { name: "Any symbol", exact: true }).waitFor();
  assert.equal(await any.locator(".unrestricted-trait").count(), 3);
  assert.equal(
    await any.locator(".unrestricted-trait defs").count(),
    0,
    "Static icons do not allocate clipping paths or shimmer gradients",
  );
  assert.equal(
    await card.locator(".unrestricted-trait").count(),
    0,
    "Restricted traits do not show unrestricted icons",
  );
  assert.equal(
    await any.locator(".preview-model,.preview-band,.preview-symbols").count(),
    0,
  );
  assert.equal(
    await any
      .locator(
        ".trait-preview img:not(.shared-icon > img),.gift-preview-fallback",
      )
      .count(),
    0,
  );
  const missing = page
    .locator(".gift-card")
    .filter({ has: page.locator('[data-order-id="missing"]') });
  await missing.getByText("Unavailable model", { exact: true }).waitFor();
  assert.equal(await missing.locator(".preview-model").count(), 0);
  await missing.getByText("2 unavailable models", { exact: true }).waitFor();
  assert.equal(
    await missing.getByRole("img", { name: "Any model", exact: true }).count(),
    0,
    "Missing catalog entries cannot turn a restricted bid into Any",
  );
  const single = page
    .locator(".gift-card")
    .filter({ has: page.locator('[data-order-id="single"]') });
  assert.equal(await single.locator(".preview-model").count(), 1);
  assert.equal(await single.locator(".preview-band").count(), 1);
  assert.equal(await single.locator(".bid-preview-summary span").count(), 0);
  const backdrop = page
    .locator(".gift-card")
    .filter({ has: page.locator('[data-order-id="backdrop"]') });
  await backdrop.getByRole("img", { name: "Any model", exact: true }).waitFor();
  assert.equal(await backdrop.locator(".preview-band").count(), 1);
  assert.equal(await backdrop.locator(".preview-symbols").count(), 1);
  assert.equal(
    await backdrop
      .getByRole("img", { name: "Any backdrop", exact: true })
      .count(),
    0,
  );
  assert.equal(
    await backdrop
      .getByRole("img", { name: "Any symbol", exact: true })
      .count(),
    0,
  );
  assert.equal(
    await card.evaluate((c) => c.scrollWidth <= c.clientWidth),
    true,
  );
  assert.equal(
    await page
      .locator(".gift-grid")
      .evaluate((c) => getComputedStyle(c).gridAutoRows),
    "180px",
  );
  await card.hover();
  await checkLayout();
  await page.waitForTimeout(300);
  assert.equal(await page.locator(".model-animation").count(), 0);
  assert.equal(
    requests.filter((url) => url.endsWith("/animation")).length,
    0,
    "Bid cards never fetch TGS",
  );
  await page.screenshot({ path: join(output, "bid-cards-mobile.png") });
  await card.locator(".card-open").focus();
  await page.keyboard.press("Enter");
  await page.locator(".detail-criteria").waitFor();
  const details = page.locator(".detail-criteria");
  const detailPreview = details.locator(".bid-detail-preview");
  await details.getByLabel("Quantity: 8", { exact: true }).waitFor();
  await details
    .getByRole("img", { name: "Venue: MRKT", exact: true })
    .waitFor();
  assert.equal(await details.locator('[data-condition="quantity"]').count(), 0);
  const checkModelSizing = async () => {
    const sizes = await detailPreview
      .locator(".preview-model")
      .evaluateAll((models) =>
        models.flatMap((model) => {
          const artwork = model.querySelector(".gift-preview");
          const image = artwork?.querySelector(
            ".model-animation, .gift-loading-placeholder",
          );
          if (!image) return [];
          const host = artwork.getBoundingClientRect();
          const media = image.getBoundingClientRect();
          const name = model
            .querySelector("span[title]")
            .getBoundingClientRect();
          const title = model
            .closest("figure")
            .querySelector("#detail-title")
            .getBoundingClientRect();
          return [
            {
              expected: parseFloat(
                getComputedStyle(artwork).getPropertyValue(
                  "--preview-model-size",
                ),
              ),
              width: media.width,
              height: media.height,
              fits:
                Math.abs(media.width - host.width) < 1 &&
                Math.abs(media.height - host.height) < 1,
              nameBelow: name.top >= host.bottom - 1,
              titleBelow: title.top >= name.bottom - 1,
              clearsBadges:
                host.top >=
                model
                  .closest(".detail-art")
                  .querySelector(".detail-art-meta")
                  .getBoundingClientRect().bottom,
            },
          ];
        }),
      );
    assert.ok(
      sizes.length &&
        sizes.every(
          (size) =>
            size.fits &&
            size.nameBelow &&
            size.titleBelow &&
            size.clearsBadges &&
            Math.abs(size.width - size.expected) < 1 &&
            Math.abs(size.height - size.expected) < 1,
        ),
      `Bid artwork fills the shared picker-sized media box: ${JSON.stringify(sizes)}`,
    );
  };
  await detailPreview.locator(".model-animation.ready").first().waitFor();
  assert.equal(
    await detailPreview.locator("figure > figcaption #detail-title").count(),
    1,
    "The collection name belongs to the composition background",
  );
  assert.ok(
    await detailPreview.locator("figure").evaluate((figure) => {
      const title = figure
        .querySelector("#detail-title")
        .getBoundingClientRect();
      const backdrop = figure
        .querySelector(".preview-bands")
        .getBoundingClientRect();
      const symbols = figure
        .querySelector(".preview-symbols")
        .getBoundingClientRect();
      return backdrop.bottom >= title.bottom && symbols.bottom >= title.bottom;
    }),
    "Backdrops and symbols extend behind the collection title",
  );
  assert.equal(await detailPreview.locator(".preview-model").count(), 8);
  await checkModelSizing();
  assert.equal(
    await detailPreview.locator(".preview-model > span[title]").count(),
    8,
    "Detail models use the same names and artwork layout as the filter preview",
  );
  assert.equal(await detailPreview.locator(".preview-band").count(), 3);
  assert.equal(
    await detailPreview.locator(".preview-symbols canvas").count(),
    1,
  );
  assert.equal(
    await detailPreview.locator(".bid-preview-summary span").count(),
    0,
  );
  await detailPreview.hover();
  await details.evaluate((el) => {
    el.scrollTop = 0;
    el.querySelector(".detail-content").scrollTop = 0;
  });
  assert.equal(
    await details.locator(".detail-art #detail-title").isVisible(),
    true,
  );
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".bid-detail-preview .model-animation.ready")
        .length === 8,
  );
  assert.equal(
    await page
      .locator(".gift-card .model-animation, .condition-chip .model-animation")
      .count(),
    0,
    "Cards and condition chips remain static",
  );
  const frame = await detailPreview
    .locator(".model-animation.ready")
    .first()
    .innerHTML();
  await page.waitForFunction(
    (before) =>
      document.querySelector(".bid-detail-preview .model-animation.ready")
        ?.innerHTML !== before,
    frame,
  );
  assert.equal(
    await details.locator(".model-animation.ready").count(),
    8,
    "All eight opened bid models animate within the adaptive device budget",
  );
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.waitForFunction(
    () => !document.querySelector(".model-animation.ready"),
  );
  const reducedRequests = requests.length;
  await page.waitForTimeout(150);
  assert.equal(
    requests.length,
    reducedRequests,
    "Reduced motion stops animation downloads and players",
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".bid-detail-preview .model-animation.ready")
        .length === 8,
  );
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", {
      configurable: true,
      value: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForFunction(
    () => !document.querySelector(".model-animation.ready"),
  );
  await page.evaluate(() => {
    delete document.hidden;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".bid-detail-preview .model-animation.ready")
        .length === 8,
  );
  await details
    .getByRole("button", {
      name: "How is Matching asks calculated?",
      exact: true,
    })
    .click();
  await page
    .getByRole("dialog", { name: "Calculation guide", exact: true })
    .waitFor();
  await page.waitForFunction(
    () => !document.querySelector(".model-animation.ready"),
  );
  await page
    .getByRole("dialog", { name: "Calculation guide", exact: true })
    .getByText("Current asks for the accepted gifts.", { exact: true })
    .waitFor();
  await page
    .getByRole("button", { name: "Close calculation guide", exact: true })
    .click();
  await page.locator(".valuation-popup").waitFor({ state: "detached" });
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".bid-detail-preview .model-animation.ready")
        .length === 8,
  );
  assert.equal(await details.locator(".condition-chip").count(), 16);
  for (const [kind, key, id] of [
    ["model", "m", "model-0"],
    ["backdrop", "b", "backdrop-0"],
    ["pattern", "s", "pattern-0"],
    ["number", "n", "1..1000"],
  ]) {
    const chip = details
      .locator(`[data-condition="${kind}"] a.condition-chip`)
      .first();
    const url = new URL(await chip.getAttribute("href"), page.url());
    assert.equal(url.searchParams.get("f"), `c=gift|${key}=${id}`);
    assert.equal(url.searchParams.get("mode"), "ask");
    assert.equal(url.searchParams.get("sort"), "price_asc");
    assert.equal(url.searchParams.get("currency"), "GRAM");
  }
  assert.equal(
    await details
      .locator('[data-condition="model"] .attribute-rarity')
      .textContent(),
    "Rarity: Legendary",
  );
  assert.equal(
    await details
      .locator('[data-condition="backdrop"] .attribute-rarity')
      .textContent(),
    "Rarity: 2%",
  );
  assert.equal(
    await details
      .locator('[data-condition="pattern"] .attribute-rarity')
      .textContent(),
    "Rarity: 1%",
  );
  assert.equal(await details.locator(".condition-overflow").count(), 0);
  assert.equal(await details.locator(".detail-art").count(), 1);
  await details.getByText("/ gift", { exact: true }).waitFor();
  assert.equal(await details.locator(".detail-pricing > section").count(), 2);
  await details
    .getByRole("columnheader", { name: "Best asks", exact: true })
    .waitFor();
  await details
    .getByRole("columnheader", { name: "Best bids", exact: true })
    .waitFor();
  await details
    .getByLabel("Collection best bid: 1.5 GRAM · MRKT · 3 available", {
      exact: true,
    })
    .waitFor();
  assert.equal(await details.locator(".matching-floor time").count(), 0);
  await details.locator(".matching-floor .valuation-value").first().waitFor();
  assert.equal(await details.locator(".detail-quantity").textContent(), "×8");
  assert.equal(
    await details.locator('[data-condition="model"] .condition-chip').count(),
    8,
  );
  await details
    .locator(".condition-chip")
    .getByText("Model 7", { exact: true })
    .waitFor();
  assert.equal(
    await details.evaluate((c) => c.scrollWidth <= c.clientWidth),
    true,
  );
  await details.evaluate(async (el) => {
    await Promise.all(
      el.getAnimations().map((a) => a.finished.catch(() => {})),
    );
  });
  await page.screenshot({ path: join(output, "bid-details-mobile.png") });
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  await details.waitFor({ state: "detached" });
  assert.equal(
    await page.locator(".model-animation").count(),
    0,
    "Closing bid details releases all animation players",
  );
  const closedRequests = requests.length;
  await card.hover();
  await page.waitForTimeout(200);
  assert.equal(
    requests.length,
    closedRequests,
    "Hovering bid cards after closing details stays static",
  );
  await page.setViewportSize({ width: 1200, height: 850 });
  await card.locator("img.loaded").first().waitFor();
  assert.equal(
    await page
      .locator(".gift-grid")
      .evaluate((c) => getComputedStyle(c).gridAutoRows),
    "184px",
  );
  assert.equal(
    await card.evaluate((c) => c.scrollWidth <= c.clientWidth),
    true,
  );
  await page.screenshot({ path: join(output, "bid-cards-desktop.png") });
  await checkLayout();
  await card.locator(".card-open").click();
  await details.waitFor();
  assert.ok((await details.boundingBox()).width <= 1040);
  assert.ok(
    await details.evaluate((dialog) => {
      const art = dialog.querySelector(".detail-art").getBoundingClientRect();
      const content = dialog
        .querySelector(".detail-content")
        .getBoundingClientRect();
      const close = dialog.querySelector(".modal-close");
      const button = close.getBoundingClientRect();
      return (
        content.left >= art.right - 1 &&
        Math.abs(art.top - content.top) < 1 &&
        close.contains(
          document.elementFromPoint(
            button.x + button.width / 2,
            button.y + button.height / 2,
          ),
        )
      );
    }),
    "Bids share the ask layout and a reachable viewport close action",
  );
  assert.equal(await detailPreview.locator(".preview-model").count(), 8);
  await details.evaluate(async (el) => {
    await Promise.all(
      el.getAnimations().map((a) => a.finished.catch(() => {})),
    );
  });
  const dialogBox = await details.boundingBox();
  const actionsBox = await details.locator(".detail-actions").boundingBox();
  assert.ok(
    dialogBox.y + dialogBox.height - actionsBox.y - actionsBox.height < 4,
    "Actions stay at the bottom of the shared detail layout",
  );
  await page.screenshot({ path: join(output, "bid-details-desktop.png") });
  for (const [width, height] of [
    [320, 568],
    [390, 844],
    [1024, 400],
    [1200, 900],
  ]) {
    await page.setViewportSize({ width, height });
    await page.evaluate(
      () =>
        new Promise((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(resolve)),
        ),
    );
    await checkModelSizing();
    const layout = await details.evaluate((dialog) => {
      dialog.scrollTop = 0;
      dialog.querySelector(".detail-content").scrollTop = 0;
      const bounds = dialog.getBoundingClientRect();
      const art = dialog.querySelector(".detail-art").getBoundingClientRect();
      const caption = dialog
        .querySelector(".detail-caption")
        .getBoundingClientRect();
      const close = dialog.querySelector(".modal-close");
      const button = close.getBoundingClientRect();
      const glyph = close.querySelector("svg").getBoundingClientRect();
      return {
        contained:
          bounds.top >= 0 &&
          bounds.bottom <= innerHeight &&
          dialog.scrollWidth <= dialog.clientWidth,
        caption: caption.bottom <= art.bottom + 1,
        close:
          button.width === 44 &&
          button.height === 44 &&
          !close.querySelector("img") &&
          Math.abs(glyph.x + glyph.width / 2 - button.x - button.width / 2) <
            1 &&
          Math.abs(glyph.y + glyph.height / 2 - button.y - button.height / 2) <
            1 &&
          close.contains(
            document.elementFromPoint(
              button.x + button.width / 2,
              button.y + button.height / 2,
            ),
          ),
        artHeight: art.height,
        badges: [...dialog.querySelectorAll(".detail-art-meta > span")].every(
          (badge) => {
            const box = badge.getBoundingClientRect();
            return (
              box.left >= art.left &&
              box.top >= art.top &&
              box.right <= art.right &&
              box.bottom <= art.bottom &&
              !(
                box.right > button.left &&
                box.left < button.right &&
                box.bottom > button.top &&
                box.top < button.bottom
              )
            );
          },
        ),
      };
    });
    assert.ok(
      layout.contained && layout.caption && layout.close && layout.badges,
      `Bid at ${width}×${height}: ${JSON.stringify(layout)}`,
    );
    if (width === 390)
      assert.ok(
        layout.artHeight < 320,
        "A small bid preview does not reserve a full gift-sized cover",
      );
  }
  await page.setViewportSize({ width: 1200, height: 850 });
  await details.locator('[data-condition="number"]').scrollIntoViewIfNeeded();
  await details.locator(".detail-actions").scrollIntoViewIfNeeded();
  const visibleDialog = await details.boundingBox();
  const visibleActions = await details.locator(".detail-actions").boundingBox();
  assert.ok(
    visibleActions.y >= visibleDialog.y &&
      visibleActions.y + visibleActions.height <=
        visibleDialog.y + visibleDialog.height,
  );
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  await details.waitFor({ state: "detached" });
  await page.locator('[data-order-id="missing"]').click();
  await details.waitFor();
  assert.equal(
    await details
      .locator(".condition-chip")
      .getByText("Unavailable model", { exact: true })
      .count(),
    2,
    "Wrong trait kinds are unavailable, not displayed as accepted models",
  );
  await detailPreview.getByText("Unavailable model", { exact: true }).waitFor();
  assert.equal(await detailPreview.locator(".preview-model").count(), 0);
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  await details.waitFor({ state: "detached" });
  await page.locator('[data-order-id="any"]').click();
  await details.waitFor();
  await detailPreview
    .getByRole("img", { name: "Any model", exact: true })
    .waitFor();
  assert.equal(await details.locator(".condition-any").count(), 3);
  for (const [width, height] of [
    [320, 568],
    [390, 844],
    [1200, 850],
  ]) {
    await page.setViewportSize({ width, height });
    const closePosition = await details.locator(".modal-close").boundingBox();
    assert.ok(
      Math.abs(closePosition.y - 12) < 1 &&
        Math.abs(closePosition.x + closePosition.width - (width - 12)) < 1,
      "The close button stays at the viewport corner during opening and resizing",
    );
    assert.equal(
      await details.evaluate((dialog) => {
        dialog.scrollTop = 0;
        const header = dialog.querySelector(".bid-preview-unrestricted");
        const badges = [
          ...dialog.querySelectorAll(".detail-art-meta > span"),
        ].map((node) => node.getBoundingClientRect());
        const art = dialog.querySelector(".detail-art").getBoundingClientRect();
        return [...header.querySelectorAll(".unrestricted-trait")].every(
          (node) => {
            const box = node.getBoundingClientRect();
            return (
              box.left >= art.left &&
              box.right <= art.right &&
              box.top >= art.top &&
              box.bottom <= art.bottom &&
              badges.every(
                (badge) =>
                  box.right <= badge.left ||
                  box.left >= badge.right ||
                  box.bottom <= badge.top ||
                  box.top >= badge.bottom,
              )
            );
          },
        );
      }),
      true,
      "Unrestricted icons and quantity/venue badges stay separate at every viewport",
    );
    await page.screenshot({
      path: join(output, `bid-unrestricted-${width}.png`),
    });
  }
  await detailPreview
    .getByRole("img", { name: "Any backdrop", exact: true })
    .waitFor();
  await detailPreview
    .getByRole("img", { name: "Any symbol", exact: true })
    .waitFor();
  assert.equal(
    await details
      .locator(".unrestricted-trait .gift-placeholder-shimmer")
      .count(),
    0,
  );
  for (const kind of ["model", "backdrop", "pattern"]) {
    const condition = details.locator(
      `[data-condition="${kind}"] .condition-any`,
    );
    const label = kind === "pattern" ? "Any symbol" : `Any ${kind}`;
    await condition.getByRole("img", { name: label, exact: true }).waitFor();
    assert.equal((await condition.textContent()).trim(), "∞");
    const url = new URL(await condition.getAttribute("href"), page.url());
    assert.equal(
      url.searchParams.get("f"),
      "c=gift",
      "Any trait opens unrestricted collection asks",
    );
    assert.equal(url.searchParams.get("mode"), "ask");
    assert.equal(await condition.locator(".condition-any-label").count(), 0);
    assert.equal(await condition.locator("summary, button").count(), 0);
  }
  assert.equal(
    await details
      .locator(".bid-conditions button, .gift-attributes [aria-haspopup]")
      .count(),
    0,
  );
  assert.ok(
    await details
      .locator(".condition-chip")
      .evaluateAll((chips) =>
        chips.every((chip) => getComputedStyle(chip).borderTopWidth === "1px"),
      ),
  );
  assert.equal(
    await details
      .locator('[data-attribute="collection"] .attribute-name')
      .count(),
    1,
  );
  assert.equal(
    await details.locator(".model-animation").count(),
    0,
    "An unrestricted preview needs no animation or remote artwork",
  );
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  await details.waitFor({ state: "detached" });
  assert.equal(
    await any.getByRole("img", { name: "Any model", exact: true }).isVisible(),
    true,
    "Cards reuse the unrestricted model artwork",
  );
  await page.locator('[data-order-id="concrete"]').click();
  await page
    .locator(".detail-art > .gift-preview-large .model-animation.ready")
    .waitFor();
  assert.equal(
    await page.locator(".gift-card .model-animation").count(),
    0,
    "Numbered bid cards stay static while their opened artwork animates",
  );
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  await page.locator(".detail").waitFor({ state: "detached" });
  // A large bid must keep its summaries visible without downloading every condition icon.
  await page.evaluate(() => {
    window.showLargeCard();
    window.scrollTo(0, 0);
  });
  const largeCard = page
    .locator(".gift-card")
    .filter({ has: page.locator('[data-order-id="large"]') });
  await largeCard.locator(".preview-model").first().waitFor();
  assert.deepEqual(
    await largeCard.locator(".bid-preview-summary span").allTextContents(),
    ["+208 choices", "3 unavailable"],
  );
  assert.match(
    await largeCard
      .locator(".bid-preview-summary span")
      .first()
      .getAttribute("title"),
    /\+196 models, \+6 backdrops, \+6 symbols/,
  );
  for (const width of [390, 1200]) {
    await page.setViewportSize({ width, height: 844 });
    await page.evaluate(() => new Promise(requestAnimationFrame));
    assert.equal(
      await largeCard.evaluate((el) => {
        const models = el
          .querySelector(".preview-models")
          .getBoundingClientRect();
        const summary = el
          .querySelector(".bid-preview-summary")
          .getBoundingClientRect();
        const number = el.querySelector(".bid-number").getBoundingClientRect();
        return (
          models.bottom <= summary.top &&
          summary.bottom <= number.top &&
          [...el.querySelectorAll(".bid-preview-summary span")].every(
            (span) => {
              const r = span.getBoundingClientRect();
              return (
                r.top >= summary.top &&
                r.bottom <= summary.bottom &&
                span.scrollWidth <= span.clientWidth
              );
            },
          )
        );
      }),
      true,
      "All compact counts must fit without overlapping models or price",
    );
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: join(output, "bid-large-mobile.png") });
  await largeCard.locator(".card-open").click();
  await details.waitFor();
  assert.equal(await details.locator(".condition-chip").count(), 224);
  assert.equal(await detailPreview.locator(".preview-model").count(), 16);
  await detailPreview.locator(".model-animation.ready").first().waitFor();
  await checkModelSizing();
  await page.waitForTimeout(200);
  assert.equal(
    await details
      .locator(".condition-chip")
      .filter({ hasText: "Remembered model" })
      .getAttribute("title"),
    "Remembered model — unavailable",
  );
  assert.match(
    await detailPreview.locator("figure").getAttribute("aria-label"),
    /Remembered model \(unavailable\)/,
  );
  const mountedChipImages = await details
    .locator(".condition-chip img")
    .count();
  assert.ok(mountedChipImages < 50, "Offscreen chip images remain unmounted");
  assert.ok(
    !requests.some((url) => url.includes("/large-model-199/")),
    "The unvisited tail must not request images",
  );
  assert.ok(
    new Set(requests.filter((url) => url.includes("/large-model-"))).size < 50,
    "Opening details must not preload all 200 models",
  );
  assert.equal(
    await detailPreview.evaluate((el) => {
      const models = el
        .querySelector(".preview-models")
        .getBoundingClientRect();
      const summary = el
        .querySelector(".bid-preview-summary")
        .getBoundingClientRect();
      const figure = el.querySelector("figure").getBoundingClientRect();
      return (
        models.bottom <= summary.top &&
        summary.bottom <= figure.bottom &&
        el.scrollHeight <= el.clientHeight
      );
    }),
    true,
    "Expanded overflow follows the artwork in normal layout flow",
  );
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".bid-detail-preview .model-animation.ready")
        .length === 10,
  );
  await page.evaluate(async () => {
    const { playbackBudget } = await import("/src/previews/playback-budget.ts");
    playbackBudget.setLimit(18);
  });
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".bid-detail-preview .model-animation.ready")
        .length === 16,
  );
  await page.evaluate(async () => {
    const { playbackBudget } = await import("/src/previews/playback-budget.ts");
    playbackBudget.setLimit(10);
  });
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".bid-detail-preview .model-animation.ready")
        .length === 10,
  );
  const warmRequests = requests.length;
  await page.screenshot({ path: join(output, "bid-large-details-mobile.png") });
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  await details.waitFor({ state: "detached" });
  await largeCard.locator(".card-open").click();
  await detailPreview.locator(".model-animation.ready").first().waitFor();
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".bid-detail-preview .model-animation.ready")
        .length === 10,
  );
  await page.waitForTimeout(200);
  assert.equal(
    requests.length,
    warmRequests,
    `Reopening unchanged details reuses warm media: ${JSON.stringify(requests.slice(warmRequests))}`,
  );
  await details.evaluate((el) => {
    window.firstChipImage = el.querySelector(".condition-chip img");
  });
  const tail = details
    .locator(".condition-chip")
    .filter({ hasText: /^Large model 199$/ });
  await tail.scrollIntoViewIfNeeded();
  await tail.locator("img.loaded").waitFor();
  assert.equal(
    requests.filter((url) => url.includes("/large-model-199/")).length,
    1,
    "Scrolling admits the previously deferred image once",
  );
  await page.evaluate(() => window.restoreCatalog());
  const restored = details
    .locator(".condition-chip")
    .filter({ hasText: "Restored model" });
  await restored.locator("img.loaded").waitFor();
  assert.equal(
    await restored.getAttribute("title"),
    "View Restored model asks",
  );
  assert.equal(
    await restored.evaluate((el) => el.tagName),
    "A",
    "Resolved attributes become asks links",
  );
  assert.match(
    await detailPreview.locator("figure").getAttribute("aria-label"),
    /Restored model/,
  );
  await details.evaluate((el) => {
    el.querySelector(".detail-content").scrollTop = 0;
  });
  assert.equal(
    await details.evaluate(
      (el) => el.querySelector(".condition-chip img") === window.firstChipImage,
    ),
    true,
    "Previously admitted artwork survives scrolling and catalog updates",
  );
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  await details.waitFor({ state: "detached" });
  await page.evaluate(() => {
    window.showModelOnlyCard();
    window.scrollTo(0, 0);
  });
  const modelOnly = page
    .locator(".gift-card")
    .filter({ has: page.locator('[data-order-id="model-only"]') });
  await modelOnly
    .getByRole("img", { name: "Any symbol", exact: true })
    .waitFor();
  await modelOnly
    .getByRole("img", { name: "Any backdrop", exact: true })
    .waitFor();
  for (const width of [320, 390, 1200]) {
    await page.setViewportSize({ width, height: 850 });
    assert.equal(
      await modelOnly.evaluate((card) => {
        const header = card
          .querySelector(".bid-preview-unrestricted")
          .getBoundingClientRect();
        const summary = card
          .querySelector(".bid-preview-summary")
          .getBoundingClientRect();
        return (
          [...card.querySelectorAll(".preview-model")].every((model) => {
            const box = model.getBoundingClientRect();
            return (
              box.top >= header.bottom - 1 && box.bottom <= summary.top + 1
            );
          }) && card.scrollWidth <= card.clientWidth
        );
      }),
      true,
      "Unrestricted icons, four models and overflow fit inside a compact card",
    );
  }
  assert.ok(requests.some((url) => url.endsWith("/preview")));
  assert.ok(
    !requests.some((url) => /model-\d+\/thumbnail/.test(url)),
    "Model options prefer SVG previews",
  );
  assert.deepEqual(
    unexpected,
    ["/valuations/strategies", "/guide"],
    "Bid details share one strategy discovery request; the guide loads only on intent",
  );
  assert.deepEqual(errors, []);
  console.log(
    `Bid browser passed: matching current floor, normal-flow summaries, partial catalog recovery, ${mountedChipImages}/210 chip images admitted on opening a large bid, scroll admission, warm reopening and budgeted detail animations with static cards.`,
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
