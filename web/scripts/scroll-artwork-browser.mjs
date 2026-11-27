// Slow HTTP responses and real native SVG decoding in the actual virtual grid.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "scroll-artwork-"));
const counts = new Map();
let png;
const svg =
  '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><path fill="#fcb731" d="M80 80H432V432H80Z"/></svg>';
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">
import React from 'react';import{createRoot}from'react-dom/client';import{flushSync}from'react-dom';import{VirtualGiftGrid}from'/src/orderbook/virtual-gift-grid.tsx';import{GiftArtwork}from'/src/previews/gift-artwork.tsx';import{GiftBackdrop}from'/src/previews/gift-backdrop.tsx';import'/src/style.css';
const draw=CanvasRenderingContext2D.prototype.drawImage;CanvasRenderingContext2D.prototype.drawImage=function(...args){if(this.canvas.matches('.gift-backdrop'))this.canvas.hasSymbols=true;return draw.apply(this,args)};
const root=createRoot(document.getElementById('root')),orders=Array.from({length:6000},(_,i)=>({id:String(i),model:i%96}));
const render=item=>React.createElement('li',{className:'gift-card','data-id':item.id},React.createElement(GiftArtwork,{className:'card-preview',preload:true,media:{preview_url:'/media/scroll/'+item.model+'/preview',animation_url:'/media/scroll/'+item.model+'/animation'},playback:'interaction',fallback:'G',palette:{center:'#a6b67f',edge:'#62904b',pattern:'#37552a'},patternUrls:['/media/scroll/symbol.png']}));
const show=()=>root.render(React.createElement('div',{className:'workspace'},React.createElement('main',{className:'orderbook market-shell'},React.createElement('aside',{className:'market-sidebar'},'Controls'),React.createElement('div',{className:'market-results'},React.createElement(VirtualGiftGrid,{orders,keyOf:item=>item.id,render,visible:()=>{},busy:false})))));
window.visibleState=()=>{const images=[...document.querySelectorAll('.gift-model-image')].filter(i=>{const r=i.getBoundingClientRect();return r.bottom>0&&r.top<innerHeight});return{images:images.length,ready:images.filter(i=>i.classList.contains('loaded')&&i.complete&&i.naturalWidth).length,placeholders:document.querySelectorAll('.gift-loading-placeholder').length,total:document.querySelectorAll('.gift-card').length}};
window.remount=()=>{flushSync(()=>root.render(null));window.scrollTo(0,0);flushSync(show);return window.visibleState();};
window.backdrop=(visible,clear=false)=>{if(clear)flushSync(()=>root.render(null));flushSync(()=>root.render(React.createElement('div',{style:{position:'relative',width:200,height:200}},React.createElement(GiftBackdrop,{palette:{center:'#a6b67f',edge:'#62904b',pattern:'#37552a'},patternUrls:['/media/scroll/warm-symbol.png'],preload:true,visible}))));const c=document.querySelector('.gift-backdrop');return{ready:c.width>1&&!!c.hasSymbols,background:c.style.background};};
show();window.fixtureReady=true;
</script></body></html>`;
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "scroll-artwork-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url === "/__scroll_artwork") {
            res.setHeader("content-type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
            return;
          }
          if (!req.url.startsWith("/media/scroll/")) return next();
          counts.set(req.url, (counts.get(req.url) || 0) + 1);
          res.setHeader(
            "content-type",
            req.url.endsWith(".png") ? "image/png" : "image/svg+xml",
          );
          res.setHeader("cache-control", "public,max-age=86400");
          const timer = setTimeout(
            () => res.end(req.url.endsWith(".png") ? png : svg),
            350,
          );
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
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 8;
      const ctx = canvas.getContext("2d");
      ctx.fillStyle = "white";
      ctx.fillRect(0, 0, 8, 8);
      return canvas.toDataURL("image/png").split(",")[1];
    }),
    "base64",
  );
  await encoder.close();
  for (const width of [390, 1440]) {
    const page = await browser.newPage({
      viewport: { width, height: 844 },
      reducedMotion: "no-preference",
    });
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto(server.resolvedUrls.local[0] + "__scroll_artwork");
    await page.waitForFunction(
      () =>
        window.fixtureReady &&
        document.querySelectorAll(".gift-model-image.loaded").length > 0,
    );
    await page.waitForFunction(() =>
      [...document.querySelectorAll(".gift-model-image")].every((i) =>
        i.classList.contains("loaded"),
      ),
    );
    await page.waitForFunction(() =>
      [...document.querySelectorAll(".gift-backdrop")].some((c) => c.width > 1),
    );
    assert.ok(
      await page.evaluate(() =>
        [...document.querySelectorAll(".gift-backdrop")].some(
          (c) => c.getBoundingClientRect().top >= innerHeight && c.width === 1,
        ),
      ),
      "overscan warms symbol data without painting offscreen canvases",
    );
    const initial = await page.evaluate(() => window.visibleState());
    assert.ok(initial.images > 0);
    assert.equal(initial.ready, initial.images);
    assert.ok(initial.total < 100);
    assert.equal(
      await page.locator('.gift-model-image[loading="lazy"]').count(),
      0,
      "virtual rows must start stills before entering the viewport",
    );
    assert.ok(
      (await page.locator('.gift-model-image[fetchpriority="auto"]').count()) >
        0,
      "overscan requests do not outrank visible gifts",
    );
    const high = await page
      .locator('.gift-model-image[fetchpriority="high"]')
      .count();
    assert.ok(high > 0);
    for (const y of [140, 280, 420, 560, 700]) {
      await page.evaluate((y) => scrollTo(0, y), y);
      await page.waitForTimeout(200);
    }
    const ahead = await page.evaluate(() => window.visibleState());
    assert.equal(
      ahead.ready,
      ahead.images,
      "prefetched gifts are ready on entry",
    );
    assert.ok(ahead.total < 100);
    const warm = await page.evaluate(() => window.remount());
    assert.equal(
      warm.placeholders,
      0,
      "cached artwork must not reset to shimmer during remount",
    );
    assert.equal(
      warm.ready,
      warm.images,
      "decoded native images appear in the same commit",
    );
    for (const y of [1300, 2600, 1300, 0]) {
      await page.evaluate((y) => scrollTo(0, y), y);
      await page.waitForTimeout(50);
    }
    assert.equal(
      await page.locator(".model-animation").count(),
      0,
      "offscreen preloading does not activate animation",
    );
    const start = counts.get("/media/scroll/warm-symbol.png") || 0;
    const cold = await page.evaluate(() => window.backdrop(false, true));
    assert.notEqual(
      cold.background,
      "",
      "backdrop palette appears before symbol/network completion",
    );
    while ((counts.get("/media/scroll/warm-symbol.png") || 0) === start)
      await page.waitForTimeout(10);
    await page.evaluate(() => window.backdrop(true));
    await page.waitForFunction(
      () => document.querySelector(".gift-backdrop")?.hasSymbols,
    );
    assert.equal(
      counts.get("/media/scroll/warm-symbol.png"),
      start + 1,
      "entering the viewport must not cancel and restart a prefetched symbol",
    );
    await page.evaluate(() => window.backdrop(false));
    assert.equal(
      (await page.evaluate(() => window.backdrop(true))).ready,
      true,
      "warm symbols paint in the visibility commit",
    );
    assert.equal(
      (await page.evaluate(() => window.backdrop(true, true))).ready,
      true,
      "remounted backdrops reuse decoded/tinted symbols synchronously",
    );
    assert.deepEqual(errors, []);
    await page.close();
  }
  assert.ok(
    [...counts.keys()].every(
      (url) =>
        url.endsWith("/preview") ||
        url.endsWith("/symbol.png") ||
        url.endsWith("/warm-symbol.png"),
    ),
    "no animation or thumbnail downloads while scrolling",
  );
  assert.ok(
    [...counts.values()].every((n) => n <= 2),
    "each context downloads each still once despite scrolling, clones and remounts",
  );
  console.log(
    "Scroll artwork browser passed: bounded overscan, eager SVG preloading, visible priority, immediate warm remounts, no repeated transfers or animation work.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
