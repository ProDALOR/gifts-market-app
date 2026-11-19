// Bid alternatives use shared static picker art, bounded card summaries and complete details.
import assert from "node:assert/strict";
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
function App(){const[selected,select]=useState();window.selectBid=select;return React.createElement('main',{className:'orderbook'},React.createElement('section',{className:'market-results'},React.createElement('h1',null,'Bids'),React.createElement(GiftGrid,{orders:[base,any,missing,concrete],busy:false,visible:()=>{},card:()=>({options,collectionName:'Liberty Figure',venueName:'MRKT',selected:false,open:select})})),selected&&React.createElement(OrderDetails,{selected,options,names,collectionName:'Liberty Figure',close:()=>select(undefined)}));}
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
          const pattern = req.url.includes("/pattern-");
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
  await card.locator("img.loaded").first().waitFor();
  assert.equal(await card.locator(".condition-chip").count(), 3);
  assert.deepEqual(
    await card.locator(".condition-overflow").allTextContents(),
    ["+7", "+2", "+3"],
  );
  assert.equal(await card.locator(".serial").textContent(), "Bid · ×8");
  assert.match(
    await card.locator('[data-condition="number"]').textContent(),
    /#1 – 1000/,
  );
  const any = page
    .locator(".gift-card")
    .filter({ has: page.locator('[data-order-id="any"]') });
  await any.getByText("Any model", { exact: true }).waitFor();
  assert.equal(await any.locator("[data-condition]").count(), 1);
  assert.equal(
    await any.locator(".bid-conditions img,.gift-preview-fallback").count(),
    0,
  );
  const missing = page
    .locator(".gift-card")
    .filter({ has: page.locator('[data-order-id="missing"]') });
  await missing.getByText("Unavailable model", { exact: true }).waitFor();
  assert.equal(
    await missing.locator(".condition-any").count(),
    0,
    "Missing catalog entries cannot turn a restricted bid into Any",
  );
  assert.equal(
    await card.evaluate((c) => c.scrollWidth <= c.clientWidth),
    true,
  );
  assert.equal(
    await page
      .locator(".gift-grid")
      .evaluate((c) => getComputedStyle(c).gridAutoRows),
    "156px",
  );
  await card.hover();
  await page.waitForTimeout(300);
  assert.equal(await page.locator(".model-animation").count(), 0);
  await page.screenshot({ path: join(output, "bid-cards-mobile.png") });
  await card.locator(".card-open").focus();
  await page.keyboard.press("Enter");
  await page.locator(".detail-criteria").waitFor();
  const details = page.locator(".detail-criteria");
  assert.equal(await details.locator(".condition-chip").count(), 15);
  assert.equal(await details.locator(".condition-overflow").count(), 0);
  assert.equal(
    await details.locator(".detail-art,.valuation-summary").count(),
    0,
  );
  await details.getByText("Price per gift", { exact: true }).waitFor();
  assert.equal(
    await details.locator('[data-condition="quantity"] dd').textContent(),
    "8",
  );
  assert.equal(
    await details.locator('[data-condition="model"] .condition-chip').count(),
    8,
  );
  await details.getByText("Model 7", { exact: true }).waitFor();
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
  await card.locator(".card-open").click();
  await details.waitFor();
  assert.ok((await details.boundingBox()).width <= 640);
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
  await page
    .getByRole("button", { name: "Close details", exact: true })
    .click();
  await details.waitFor({ state: "detached" });
  await page.locator('[data-order-id="missing"]').click();
  await details.waitFor();
  assert.equal(
    await details.getByText("Unavailable model", { exact: true }).count(),
    2,
    "Wrong trait kinds are unavailable, not displayed as accepted models",
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
    "Criteria bids need no additional API requests",
  );
  assert.deepEqual(errors, []);
  console.log(
    "Bid browser passed: shared static SVG art, compact overflow, complete conditions, unknown traits, keyboard details, responsive layout and no valuation/animation requests.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
