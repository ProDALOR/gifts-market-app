// Real startup artwork, virtual grid and picker options with delayed synthetic media.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "igifts-loading-"));
const requests = new Map();
let png;
const svg =
  '<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128"><rect width="128" height="128" fill="#eac2e1"/></svg>';
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">
import React from'react';import{createRoot}from'react-dom/client';import{LoadingScreen}from'/src/app/loading-screen.tsx';import{VirtualGiftGrid}from'/src/orderbook/virtual-gift-grid.tsx';import{GiftCardView}from'/src/previews/gift-card.tsx';import{OptionArt}from'/src/previews/option-art.tsx';import{setDisplayMode}from'/src/app/preferences.ts';import'/src/style.css';
const h=React.createElement,root=createRoot(document.getElementById('root')),options=new Map([['model',{kind:'model',id:'model',name:'Gold',media:{thumbnail_url:'/media/models/model/thumbnail',preview_url:'/media/models/model/preview',animation_url:'/media/models/model/animation'}}]]),empty=new Map(),key=item=>item.id,visible=()=>{};
window.startup=()=>root.render(h(LoadingScreen));
window.grid=(count,busy,ready=false)=>root.render(h('div',{className:'workspace'},h('main',{className:'orderbook market-shell'},h('aside',{className:'market-sidebar'},'Controls'),h('div',{className:'market-results'},h(VirtualGiftGrid,{orders:Array.from({length:count},(_,i)=>({id:String(i)})),keyOf:key,visible,busy,render:item=>h(GiftCardView,{subject:{target:{kind:'gift',id:item.id,number:1,traits:{model:'model'}}},options:ready?options:empty,collectionName:'Gift',identity:'#1',currency:'GRAM',price:'12',action:h('button',{className:'card-open','aria-label':'Open gift'})})})))));
window.pickers=mode=>{setDisplayMode(mode);root.render(h('main',{className:'workspace orderbook'},h('button',{id:'model-picker','aria-label':'Model'},h(OptionArt,{option:options.get('model')})),h('button',{id:'symbol-picker','aria-label':'Symbol'},h(OptionArt,{option:{kind:'pattern',id:'symbol',name:'Symbol',media:{thumbnail_url:'/media/patterns/symbol/thumbnail'}}}))))};
window.startup();
</script><style>#model-picker,#symbol-picker{width:120px;height:64px}.option-art{width:40px;height:40px}</style></body></html>`;
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "loading-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url === "/__loading") {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
            return;
          }
          if (!req.url.startsWith("/media/")) return next();
          requests.set(req.url, (requests.get(req.url) || 0) + 1);
          const thumbnail = req.url.endsWith("/thumbnail");
          res.setHeader(
            "Content-Type",
            thumbnail ? "image/png" : "image/svg+xml",
          );
          res.setHeader("Cache-Control", "public,max-age=86400");
          const timer = setTimeout(() => res.end(thumbnail ? png : svg), 500);
          res.on("close", () => clearTimeout(timer));
        });
      },
    },
  ],
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  const encoder = await browser.newPage();
  png = Buffer.from(
    await encoder.evaluate(() => {
      const c = document.createElement("canvas");
      c.width = c.height = 8;
      return c.toDataURL().split(",")[1];
    }),
    "base64",
  );
  await encoder.close();
  for (const width of [390, 1440]) {
    requests.clear();
    const page = await browser.newPage({ viewport: { width, height: 900 } }),
      errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(server.resolvedUrls.local[0] + "__loading");
    await page
      .getByRole("status", { name: "Loading app", exact: true })
      .waitFor();
    await page.locator(".startup-loading .model-animation.ready").waitFor();
    assert.equal(
      await page.locator(".startup-loading a").count(),
      0,
      "startup logo cannot trigger navigation",
    );
    assert.equal(requests.size, 0, "startup uses only bundled assets");
    await page.evaluate(() => window.grid(0, true));
    await page.locator(".gift-card-loading").first().waitFor();
    assert.equal(await page.locator(".gift-card-loading").count(), 12);
    assert.equal(
      await page
        .locator(".gift-card-loading .card-preview")
        .first()
        .evaluate((el) => parseFloat(getComputedStyle(el).borderRadius) > 0),
      true,
      "loading cards keep the real card corner shape",
    );
    assert.equal(
      await page.locator(".gift-grid").getAttribute("data-order-count"),
      "0",
    );
    assert.equal(
      await page
        .locator(".model-animation,.gift-grid img,.gift-placeholder-shimmer")
        .count(),
      0,
      "loading shells allocate no players or remote images",
    );
    assert.equal(requests.size, 0);
    await page.screenshot({
      path: "/tmp/igifts-loading-grid-" + width + ".png",
    });
    await page.evaluate(() => window.grid(6, true));
    await page.waitForFunction(
      () => document.querySelectorAll(".card-open").length === 6,
    );
    assert.equal(
      await page.locator(".gift-card-loading,.gift-preview-fallback").count(),
      0,
      "unknown model metadata shows outlines, never letters",
    );
    assert.equal(
      await page.locator(".gift-card .gift-placeholder-gift").count(),
      6,
    );
    await page.evaluate(() => {
      window.cardNode = document.querySelector(".gift-preview");
      window.grid(6, false);
    });
    await page.waitForFunction(
      () =>
        document.querySelector(".gift-grid").getAttribute("aria-busy") ===
        "false",
    );
    assert.equal(
      await page.evaluate(
        () => window.cardNode === document.querySelector(".gift-preview"),
      ),
      true,
      "refresh states retain existing artwork",
    );
    await page.evaluate(() => window.grid(0, false));
    await page.waitForFunction(() => !document.querySelector(".gift-card"));
    await page.evaluate(() => window.grid(6, true, true));
    await page.locator(".gift-card .gift-placeholder-gift").first().waitFor();
    await page.locator(".gift-model-image.loaded").first().waitFor();
    assert.equal(
      await page.locator(".gift-card-loading").count(),
      0,
      "real rows keep known prices while their images load",
    );
    await page.evaluate(() => window.pickers("performance"));
    await page.locator("#model-picker img.loaded").waitFor();
    await page.locator("#symbol-picker img.loaded").waitFor();
    assert.match(
      await page.locator("#model-picker .gift-model-image").getAttribute("src"),
      /\/thumbnail$/,
    );
    await page.locator("#model-picker").hover();
    await page.waitForTimeout(200);
    assert.equal(
      await page.locator(".model-animation").count(),
      0,
      "picker buttons stay static on hover",
    );
    assert.equal(requests.get("/media/models/model/animation") || 0, 0);
    assert.equal(requests.get("/media/models/model/preview") || 0, 0);
    await page.evaluate(() => window.pickers("quality"));
    await page.locator('#model-picker img.loaded[src$="/preview"]').waitFor();
    assert.equal(
      await page.locator(".model-animation").count(),
      0,
      "quality picker options are static too",
    );
    assert.deepEqual(errors, []);
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
      true,
    );
    await page.close();
  }
  console.log(
    "Loading UI passed: bundled animated startup, fixed static skeletons, loading/empty/refresh transitions, unknown-model outlines, raster performance picker options and static quality options.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
