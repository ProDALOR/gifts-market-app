// Bid alternatives use the picker composition, bounded static art and complete details.
import assert from "node:assert/strict";
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
const cacheDir = await mkdtemp(join(tmpdir(), "market-bids-"));
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body><div id="fixture"></div><script type="module">
import React,{useState}from'react';import{createRoot}from'react-dom/client';
import{GiftGrid}from'/src/orderbook/gift-grid.tsx';import{OrderDetails}from'/src/orderbook/order-details.tsx';
const models=Array.from({length:8},(_,i)=>({kind:'model',id:'model-'+i,name:i===0?'Artisan Brick':'Model '+i,media:{thumbnail_url:'/media/model-'+i+'/thumbnail',preview_url:'/media/model-'+i+'/preview',animation_url:'/media/model-'+i+'/animation'}}));
const backdrops=Array.from({length:3},(_,i)=>({kind:'backdrop',id:'backdrop-'+i,name:['Sapphire','Lavender','Amber'][i],palette:{center:['#759fb4','#b789e4','#e2c585'][i],edge:'#364e73',pattern:'#283d60'}}));
const patterns=Array.from({length:4},(_,i)=>({kind:'pattern',id:'pattern-'+i,name:'Star '+i,media:{thumbnail_url:'/media/pattern-'+i+'/thumbnail',animation_url:'/media/pattern-'+i+'/animation'}}));
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
function App(){const[selected,select]=useState();const[catalog,setCatalog]=useState(options);const[largeMode,setLargeMode]=useState(false);window.selectBid=select;window.showLargeCard=()=>setLargeMode(true);window.restoreCatalog=()=>setCatalog(previous=>new Map([...previous,['known-missing',{kind:'model',id:'known-missing',name:'Restored model',media:{preview_url:'/media/restored-model/preview',thumbnail_url:'/media/restored-model/thumbnail'}}]]));return React.createElement('main',{className:'orderbook'},React.createElement('section',{className:'market-results'},React.createElement('h1',null,'Bids'),React.createElement(GiftGrid,{orders:largeMode?[large]:[base,any,missing,concrete,single,backdrop],busy:false,visible:()=>{},card:()=>({options:catalog,collectionName:'Liberty Figure',venueName:'MRKT',selected:false,open:select})})),selected&&React.createElement(OrderDetails,{selected,options:catalog,names,collectionName:'Liberty Figure',close:()=>select(undefined)}));}
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
          if (!req.url.startsWith("/media/")) return next();
          requests.push(req.url);
          assert.ok(
            !req.url.endsWith("/animation"),
            "Bids must not load animations",
          );
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
            data: criteriaReport(command.params),
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
  assert.equal(await card.locator(".serial").textContent(), "Bid · ×8");
  assert.match(await card.locator(".bid-number").textContent(), /#1 – 1000/);
  const any = page
    .locator(".gift-card")
    .filter({ has: page.locator('[data-order-id="any"]') });
  await any.getByText("Any model", { exact: true }).waitFor();
  assert.equal(
    await any.locator(".preview-model,.preview-band,.preview-symbols").count(),
    0,
  );
  assert.equal(
    await any.locator(".trait-preview img,.gift-preview-fallback").count(),
    0,
  );
  const missing = page
    .locator(".gift-card")
    .filter({ has: page.locator('[data-order-id="missing"]') });
  await missing.getByText("Unavailable model", { exact: true }).waitFor();
  assert.equal(await missing.locator(".preview-model").count(), 0);
  await missing.getByText("2 unavailable models", { exact: true }).waitFor();
  assert.equal(
    await missing.getByText("Any model", { exact: true }).count(),
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
  await backdrop.getByText("Any model", { exact: true }).waitFor();
  assert.equal(await backdrop.locator(".preview-band").count(), 1);
  assert.equal(await backdrop.locator(".preview-symbols").count(), 1);
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
  await page.screenshot({ path: join(output, "bid-cards-mobile.png") });
  await card.locator(".card-open").focus();
  await page.keyboard.press("Enter");
  await page.locator(".detail-criteria").waitFor();
  const details = page.locator(".detail-criteria");
  const detailPreview = details.locator(".bid-detail-preview");
  await detailPreview.locator("img.loaded").first().waitFor();
  assert.equal(await detailPreview.locator(".preview-model").count(), 8);
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
  assert.equal(await details.locator(".bid-detail-heading").isVisible(), true);
  assert.equal(await details.locator(".model-animation").count(), 0);
  assert.equal(await details.locator(".condition-chip").count(), 15);
  assert.equal(await details.locator(".condition-overflow").count(), 0);
  assert.equal(
    await details.locator(".detail-art,.valuation-summary").count(),
    0,
  );
  await details.getByText("Price per gift", { exact: true }).waitFor();
  await details.locator(".matching-floor .valuation-value").waitFor();
  assert.equal(
    await details.locator('[data-condition="quantity"] dd').textContent(),
    "8",
  );
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
  assert.ok((await details.boundingBox()).width <= 640);
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
    "Criteria details fit their content instead of keeping an empty artwork-sized pane",
  );
  await page.screenshot({ path: join(output, "bid-details-desktop.png") });
  await details.locator('[data-condition="quantity"]').scrollIntoViewIfNeeded();
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
  assert.equal(await details.locator(".condition-chip").count(), 223);
  assert.equal(await detailPreview.locator(".preview-model").count(), 16);
  await detailPreview.locator("img.loaded").first().waitFor();
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
  const warmRequests = requests.length;
  await page.screenshot({ path: join(output, "bid-large-details-mobile.png") });
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  await details.waitFor({ state: "detached" });
  await largeCard.locator(".card-open").click();
  await detailPreview.locator("img.loaded").first().waitFor();
  await page.waitForTimeout(200);
  assert.equal(
    requests.length,
    warmRequests,
    "Reopening unchanged details reuses warm media",
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
  assert.equal(await restored.getAttribute("title"), "Restored model");
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
  assert.ok(requests.some((url) => url.endsWith("/preview")));
  assert.ok(
    !requests.some((url) => /model-\d+\/thumbnail/.test(url)),
    "Model options prefer SVG previews",
  );
  assert.deepEqual(
    unexpected,
    [],
    "Criteria bid previews need no HTTP API requests",
  );
  assert.deepEqual(errors, []);
  console.log(
    `Bid browser passed: matching current floor, normal-flow summaries, partial catalog recovery, ${mountedChipImages}/210 chip images admitted on opening a large bid, scroll admission, warm reopening and no animation requests.`,
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
